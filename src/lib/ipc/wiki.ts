/**
 * Wiki 缓存构建与模组扫描（`wiki::*` / `modcache::*` 命令对侧）。
 *
 * 拆分自原 `@/lib/tauri.ts` 单体；对外仍由该 barrel 统一导出，调用方无需改动。
 */
import { invoke } from "@tauri-apps/api/core";
import { readLauncherConfig } from "./config";
import { isTauri } from "./core";
import { realmDir } from "./realm-maps";
import { onWindowFocusChanged } from "./window";

/**
 * 轮询批量构建进度（事件兜底：busy 时每 400ms 拉取一次）。
 *
 * `prepDone`/`prepTotal` = 准备阶段（成本估算）进度，与 `done`/`total` 分开记账
 * （两者计数域同为 `0..mods.len()`，混用会让进度条"冲满后归零"）。
 */
export async function fetchWikiBuildProgress(): Promise<{
  done: number;
  total: number;
  prepDone: number;
  prepTotal: number;
} | null> {
  if (!isTauri()) return null;
  try {
    return await invoke("cache_build_progress");
  } catch {
    return null;
  }
}

/** 请求取消正在进行的批量构建（已分发到线程的 mod 仍会跑完，之后不再构建新 mod） */
export async function cancelWikiBuild(): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("cancel_cache_build");
  } catch {
    // 命令不可用或构建已结束，忽略
  }
}

/* --------------------------- 领域加急队列（S1） --------------------------- */

/** 领域加急构建的返回（与 `buildModsWikiBatch` 同形） */
export interface RealmUrgentResult {
  ok: boolean;
  total: number;
  failed: string[];
  hd?: { generated: number; skipped: number; failed: number };
}

/**
 * **领域加急构建**：只为「当前领域所需素材」构建，进度走**独立通道**。
 *
 * 与 `buildModsWikiBatch` 的三点刻意差异（服务端实现）：
 * 1. 独立进度/取消计数器（`BuildSlot::realm`）—— 不会被启动时的全局构建覆盖；
 * 2. turbo 档 —— 用户在进度页等着，不再"空闲才提档"；
 * 3. 两阶段一次跑完 —— 省一次 IPC 往返。
 */
export async function buildRealmCacheUrgent(
  gameDir: string,
  mods: { modId: string; modPath: string }[],
  onProgress?: (done: number, total: number) => void,
  hd?: HdBuildOptions,
): Promise<RealmUrgentResult | null> {
  if (!isTauri() || mods.length === 0) return null;
  const hdArgs =
    hd && hd.rate >= 2
      ? hd.upscaler
        ? { hdRate: hd.rate, hdPhys: hd.phys, hdAi: hd.ai, hdUpscaler: hd.upscaler }
        : { hdRate: hd.rate, hdPhys: hd.phys, hdAi: hd.ai }
      : {};
  const run = () =>
    invoke<RealmUrgentResult>("build_realm_cache_urgent", { gameDir, mods, ...hdArgs });
  if (!onProgress) {
    try {
      return await run();
    } catch {
      return null;
    }
  }
  try {
    const { listen } = await import("@tauri-apps/api/event");
    const un = await listen<{ done: number; total: number }>("realm-cache-build-progress", (e) => {
      onProgress(e.payload.done, e.payload.total);
    });
    try {
      return await run();
    } finally {
      un();
    }
  } catch {
    return null;
  }
}

/** 领域加急构建进度（轮询兜底：事件在部分环境可能丢失） */
export async function fetchRealmBuildProgress(): Promise<{ done: number; total: number } | null> {
  if (!isTauri()) return null;
  try {
    return await invoke<{ done: number; total: number }>("realm_cache_build_progress");
  } catch {
    return null;
  }
}

/** 取消领域加急构建（进度页「返回」按钮） */
export async function cancelRealmCacheBuild(): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("cancel_realm_cache_build");
  } catch {
    // 命令不可用或构建已结束，忽略
  }
}

/**
 * 领域所需素材是否**已就绪**（领域信息 / 阵营巡礼入口的守门人）。
 *
 * 检查失败时返回 `ready: true`：宁可让用户进去看占位图，也不要卡在入口。
 */
export async function checkRealmAssets(
  ids: string[],
): Promise<{ ready: boolean; missing: string[] }> {
  if (!isTauri() || ids.length === 0) return { ready: true, missing: [] };
  try {
    const r = await invoke<{ ready: boolean; missing: string[] }>("check_realm_assets", { ids });
    return { ready: r?.ready !== false, missing: r?.missing ?? [] };
  } catch {
    return { ready: true, missing: [] };
  }
}

/**
 * 模组扫描的**进程内记忆**（模块级 promise）。
 *
 * 理由（见 `docs/features/startup-responsiveness.zh-CN.md`）：
 * - 视图互斥卸载：切走再切回「模组」页会重挂载 ModView 并重跑全量扫描（R5）；
 * - 启动期 `ensureWikiCacheBuilt` 与领域侧 `realmAssets` / `buildTargets` 也各扫一次 ——
 *   全是同一份磁盘状态。
 *
 * ⚠️ 安装 / 删除 / 更新模组后**必须**调用 [`invalidateModsScan`]，否则会用到陈旧列表。
 */
let modsScanPromise: Promise<unknown[]> | null = null;

/** 扫描真实 Mods 目录（进程内只真正扫一次，见 `modsScanPromise`） */
export async function scanRealMods(): Promise<unknown[]> {
  if (!isTauri()) return [];
  if (!modsScanPromise) {
    modsScanPromise = invoke<unknown[]>("scan_mods").catch((e) => {
      // 失败不缓存：一次瞬时失败不该让整个会话卡在空列表（下次调用可重试）
      modsScanPromise = null;
      throw e;
    });
  }
  return modsScanPromise;
}

/** 使模组扫描的进程内记忆失效（安装 / 删除 / 更新模组后必须调用） */
export function invalidateModsScan(): void {
  modsScanPromise = null;
}

/** 重扫：先失效进程内记忆，再取最新列表（安装 / 删除 / 更新后的唯一正确入口） */
export async function rescanRealMods(): Promise<unknown[]> {
  invalidateModsScan();
  return scanRealMods();
}

/**
 * 多线程批量构建 Wiki 缓存（进度经 wiki-build-progress 事件推送）。
 *
 * `throttle` 为构建档位，后端据此限制 rayon 线程数（并发度即限速）：
 * 0=eco（默认，后台静默构建）/ 1=normal / 2=turbo（用户主动触发或长时间空闲）。
 *
 * `phase` 为构建阶段：0=仅数据（阶段一，廉价）/ 1=仅素材（阶段二，重活）/ 2=两阶段。
 * 首次启动按「全部 mod 阶段一 → 全部 mod 阶段二」分批调用，使 wiki/realm 页面
 * 先拿到全部文本数据（无图可接受），图片再由阶段二在后台补齐。
 */
export interface HdBuildOptions {
  /** 倍率**上限**（**≥2 才生成**；1 = 只用原素材）。由显示器物理宽推导，见 `lib/hd.ts` */
  rate: number;
  /**
   * 基准物理倍率 = 最大显示器物理宽 / 1280（见 `lib/hd.ts::computeHdPhys`）。
   *
   * 后端用它**逐图按用途分级**算倍率（`需求物理 = 目标CSS × phys`）：
   * 兵种/英雄拿高倍率、城镇幕布只拿够用的倍率、源图已够大的跳过。
   * 缺省时后端按 `rate` 反推上界（钳位情形会偏保守）。
   */
  phys?: number;
  /** 放大算法 id；缺省 = 后端默认算法（xBRZ） */
  upscaler?: string;
  /**
   * **AI 高清增强**（`launcher.json` 的 `aiHdEnhance`，默认关）。
   * 开启后插画档走 Real-ESRGAN 超分（**仅 4x**），失败静默回退 Lanczos3；像素画仍走 xBRZ。
   */
  ai?: boolean;
}

export async function buildModsWikiBatch(
  gameDir: string,
  mods: { modId: string; modPath: string }[],
  onProgress?: (done: number, total: number, prep?: boolean) => void,
  throttle: 0 | 1 | 2 = 0,
  phase: 0 | 1 | 2 = 2,
  /** 高清素材生成选项（缺省 = 不生成）。只在素材阶段（phase 1/2）生效 */
  hd?: HdBuildOptions
): Promise<{ ok: boolean; total: number; failed: string[]; hd?: { generated: number; skipped: number; failed: number } } | null> {
  if (!isTauri()) return null;
  // `rate < 2` 时**不带任何 hd 参数**（而不是传 1）：后端据此完全跳过高清分支
  const hdArgs =
    hd && hd.rate >= 2
      ? hd.upscaler
        ? { hdRate: hd.rate, hdPhys: hd.phys, hdAi: hd.ai, hdUpscaler: hd.upscaler }
        : { hdRate: hd.rate, hdPhys: hd.phys, hdAi: hd.ai }
      : {};
  if (onProgress) {
    try {
      const { listen } = await import("@tauri-apps/api/event");
      // `prep=true` = 准备阶段（排序前的 mod 成本估算，UI 的"扫描模组目录"行）。
      // 冷缓存下它是分钟级且发生在第一个 mod 之前，不透传就会表现为"0% 静止一分钟"。
      let un: (() => void) | null = null;
      const startListen = async () => {
        if (un) return;
        un = await listen<{ done: number; total: number; prep?: boolean }>(
          "wiki-build-progress",
          (e) => {
            onProgress(e.payload.done, e.payload.total, e.payload.prep === true);
          },
        );
      };
      const stopListen = () => {
        un?.();
        un = null;
      };
      await startListen();
      // 窗口失焦时**退订事件**（不只停轮询）：Tauri 事件是 push 进 JS 的，
      // 失焦后每秒 ~10 次的回调 + 后续 status 处理仍是主线程的持续唤醒源。
      // 后端构建与窗口焦点无关，照常推进；聚焦后立即重订，后端的下一个事件
      // 即把进度追上（完成事件同样会到，蒙层不会卡在旧值）。
      const unFocus = await onWindowFocusChanged((focused) => {
        if (focused) void startListen();
        else stopListen();
      });
      try {
        return await invoke("build_mods_cache_batch", { gameDir, mods, throttle, phase, ...hdArgs });
      } finally {
        stopListen();
        void unFocus?.();
      }
    } catch {
      return null;
    }
  }
  try {
    return await invoke("build_mods_cache_batch", { gameDir, mods, throttle, phase, ...hdArgs });
  } catch {
    return null;
  }
}

/** 预检返回：哪些 mod 需要重建（数据 / 素材），以及索引是否已可用。 */
export interface WikiCacheState {
  ok: boolean;
  total: number;
  indexReady: boolean;
  needData: string[];
  needAssets: string[];
}

/**
 * **启动预检**：本次是否需要重建缓存（与真实构建**同源判定**，见后端 `wiki_cache_state`）。
 *
 * 用途：`ensureWikiCacheBuilt` 据此在"全部命中"时**完全跳过构建**（不置 `busy` → 不弹进度蒙层），
 * 让 wiki / mods / 领域页直接用现有 `index.json` 渲染 —— 修复"每次启动都弹重建进度条"。
 * 返回 `null`（命令不可用 / 异常）时调用方须退化为"照常构建"，**绝不因预检失败而漏建**。
 */
export async function probeWikiCacheState(
  gameDir: string,
  mods: { modId: string; modPath: string }[]
): Promise<WikiCacheState | null> {
  if (!isTauri()) return null;
  try {
    return await invoke<WikiCacheState>("wiki_cache_state", { gameDir, mods });
  } catch {
    return null;
  }
}

/** 把领域**整合模组（bundled）**的资料分析进百科缓存 `.vcmi-fv-launcher/mods`。
 *
 * 这些模组随领域导入解包到 `{realm-dir}/vcmi/Mods/<modId>`，**不在全局已安装模组列表**里，
 * 因此 `ensureWikiCacheBuilt()`（只扫全局用户目录与游戏目录）永远不会构建它们 ——
 * 领域信息页会拿不到模组描述、整合的城镇也没有预览图。导入后调用本函数补齐。 */
export async function buildRealmBundledWiki(
  bundledMods: { modId: string; modPath: string }[],
): Promise<void> {
  if (!isTauri() || bundledMods.length === 0) return;
  try {
    const cfg = await readLauncherConfig();
    await buildModsWikiBatch(cfg?.gameDir ?? "", bundledMods);
  } catch {
    /* 缓存构建失败不影响导入结果，静默处理 */
  }
}

/**
 * 释放**整轮缓存构建**留在后端进程里的加速缓存，让内存回落。
 *
 * 时机很关键：一轮构建是「阶段一 → 阶段二（城镇批 / 其余批）→ 阶段三（高清）」共 3~5 次
 * 独立的 `build_mods_cache_batch`，后端单看一批无法判断"后面还有没有"，所以**批间不能清**
 * （清了下一批要重新扫一遍目录/解析一遍配置）。只有前端知道整轮是否结束，故由本函数收尾。
 *
 * 只清可重建的派生缓存（目录索引 / 配置解析结果 / 编码产物 / 翻译表 / 去重集合），
 * 不影响已落盘的任何产物；构建前调用也无害（幂等）。
 */
export async function releaseWikiCaches(): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("release_wiki_caches");
  } catch {
    // 命令不可用（旧后端）时忽略：不释放只是内存不回落，不影响功能
  }
}

/**
 * bundled 补建的**串行链**（模块级单链）。
 *
 * 为什么必须串行：bundled 补建与全局后台构建**共用** `build_mods_cache_batch`
 * 命令和 `wiki-build-progress` 事件通道（payload 无批次标识）—— 并发的两个
 * 批次会把对方的进度账本互相打乱（2026-10-21 真机实测：全局蒙层 data 行停在
 * 中途值 18/124 而 assets 已在 21/124），且两份 rayon 并发争抢 CPU
 * （光标忙碌的诱因之一）。连续打开多个领域时，两次 bundled 之间同样会互踩。
 */
let bundledWikiChain: Promise<void> = Promise.resolve();

/** 按「领域 id + 整合模组 id 列表」补建百科缓存，用于**激活领域后**。
 *
 * 编辑器里标记「整合」只是往 realm.json 记 id，模组文件要等激活时
 * （`activate_realm` 步骤 5）才复制到 `{realm-dir}/vcmi/Mods/<modId>`；
 * 这条路径不走导入，也不在全局构建的扫描范围内，故在此补齐。
 * 构建是增量的（源 hash 命中即跳过），重复调用开销很小。
 *
 * ⚠️ **排队执行**（模块级串行链）：与全局构建并发会互相污染进度 + 争抢 CPU，
 * 见 `bundledWikiChain` 注释。与全局构建的排队等待由调用方（`App.tsx`）
 * 用 `isWikiBuildActive()` 处理 —— tauri 层不反向依赖 modcache/status。
 */
export async function buildRealmBundledWikiByIds(
  realmId: string,
  modIds: string[],
): Promise<void> {
  if (!isTauri() || modIds.length === 0) return;
  const run = async () => {
    const dir = await realmDir(realmId);
    if (!dir) return;
    await buildRealmBundledWiki(
      modIds.map((id) => ({ modId: id, modPath: `${dir}/vcmi/Mods/${id}` })),
    );
  };
  // 串到链尾（前面的失败不影响后面的入链）
  bundledWikiChain = bundledWikiChain.then(run).catch(() => {});
  return bundledWikiChain;
}
