/**
 * 高清素材（@2x / @3x …）的**倍率推导**与**URL 选取 + 回退**。
 *
 * 纯 TS 逻辑层（**禁止 import react**）。
 *
 * 背景：启动器按 1280×720 设计稿整页缩放，于是
 * `素材需要的倍率 ≈ 屏幕物理宽 / 1280`（与系统缩放比例无关，见设计分析）。
 * 缓存里始终保留**原素材**（1x），高清变体是同名 + `@Nx` 后缀的**可选产物**：
 * 有就显示高清，没有（或加载失败）就回退原素材 —— 见 `imgCandidates`。
 */

import { DESIGN_CANVAS_W, isTauri, listMonitors } from "./tauri";

/**
 * 倍率上限 = **6**（与放大算法上限对齐）。
 *
 * xBRZ 原生支持 1..=6（`infra::upscale::XbrzUpscaler::max_factor`），后端 `upscale_with`
 * 会按算法上限钳位；这里取 6 让 4K@150% 这类极端组合也能拿到最大可用素材。
 *
 * 代价已评估（bench 实测）：xBRZ 的耗时由**源像素数**决定、放大只做块填充，
 * 所以 6x 相对 2x 不是线性变慢；产物体积因重复块多、webp 压缩率高，也远小于面积比。
 */
export const MAX_HD_RATE = 6;

/**
 * **最苛刻的显示放大比**（CSS 层面）：巡礼「放大展示区」把兵种/英雄图放大到
 * 148 CSS px 高，而 `core.*` 的 `imageLarge` 源图只有 64 px 高 → `148 / 64 ≈ 2.31`。
 *
 * ⚠️ 素材倍率必须**同时满足屏幕缩放与这个显示放大**，否则会出现
 * "按屏幕算够了、但在放大区仍被拉伸"的模糊（这正是 1080P 下 2x 仍糊的原因）。
 *
 * 来源：`features/realms/showcase/ShowcaseOverlay.tsx` 的 `DETAIL_IMG_H = 148`
 * 与 `core.*` 兵种大图 58×64。**若该尺寸调整，这里必须同步。**
 */
const WORST_DISPLAY_ZOOM = 148 / 64;

/** 显示器信息（与 `tauri.ts` 的 MonitorInfo 同构，此处只取用得到的最小字段） */
export interface HdMonitorInfo {
  /** **物理**像素宽（未扣任务栏） */
  width: number;
  height: number;
  scale?: number;
}

/**
 * 由**显示器物理宽 + 启动器缩放 + 显示放大比**推导素材倍率，钳 `[1, MAX_HD_RATE]`。
 *
 * 两步（缺一不可，第二步正是 1080P 下 2x 仍糊的原因）：
 *
 * 1. **基准物理倍率** = `物理宽 / 1280`。
 *    启动器整页 `setZoom`（`main.tsx` + `computeWindowScale`）使逻辑视口恒为设计稿
 *    1280×720，于是 `1 CSS px = 物理宽 / 1280` 物理像素（系统缩放比例会被约掉，
 *    见 `lib/tauri.ts::computeWindowScale` 注释）。
 * 2. 再乘 [`WORST_DISPLAY_ZOOM`]（≈2.31）：巡礼放大区会把 58×64 的源图显示到 148 CSS 高，
 *    所以"屏幕够用"不等于"放大区够用"。
 *
 * **多显示器取最大值**：窗口可被拖到任意一块屏，便携包换机器后也会变化；
 * 只按主显示器算会让"拖到副屏"的高分屏用户拿到不够用的倍率。
 *
 * | 屏幕 | 物理宽 | 基准 | ×2.31 后 | 最终倍率 |
 * |---|---|---|---|---|
 * | 1080p | 1920 | 1.50 | 3.47 | **4** |
 * | 2K | 2560 | 2.00 | 4.63 | **5** |
 * | 4K | 3840 | 3.00 | 6.94 | **6**（钳位） |
 *
 * 无显示器信息（浏览器 mock / 命令失败）时返回 1 = 只用原素材。
 */
export function computeHdPhys(monitors: HdMonitorInfo[] | null): number {
  if (!monitors || monitors.length === 0) return 1;
  let maxW = 0;
  for (const m of monitors) {
    if (m && typeof m.width === "number" && m.width > maxW) maxW = m.width;
  }
  if (maxW <= 0) return 1;
  return maxW / DESIGN_CANVAS_W;
}

/**
 * 倍率**上限**（给前端候选链用，也是后端逐图分级的钳位上限）。
 *
 * ⚠️ 后端还会在这个上限之下**按用途分级**（`hd.rs::hd_target_box`）：
 * 大图（城镇幕布）通常只生成 2x，小图（兵种/英雄）才拿满倍率。
 * 所以前端的候选链必须**逐个档位降序尝试**，不能只试这一个倍率。
 */
export function computeHdRate(monitors: HdMonitorInfo[] | null): number {
  const phys = computeHdPhys(monitors);
  if (phys <= 1) return 1;
  return Math.max(1, Math.min(MAX_HD_RATE, Math.ceil(phys * WORST_DISPLAY_ZOOM)));
}

/** 进程内倍率缓存（启动算一次；显示器热插拔不追，换机器后重跑素材阶段会补） */
let cachedRate: number | null = null;

/**
 * 基准物理倍率缓存（= 最大显示器物理宽 / 1280）。
 *
 * 后端**按用途分级**算倍率时必需（`需求物理 = 目标CSS × phys`）：只给一个钳位后的整数
 * 倍率无法反推（钳位时尤其失真，见 `hd::HdOptions::phys` 注释）。
 */
let cachedPhys: number | null = null;

/** 探测中的 promise（并发调用只探测一次显示器） */
let probing: Promise<void> | null = null;

/** 倍率变化订阅者（`useSyncExternalStore` 模式） */
const rateListeners = new Set<() => void>();

/** 订阅倍率变化，返回取消订阅函数 */
export function subscribeHdRate(fn: () => void): () => void {
  rateListeners.add(fn);
  return () => rateListeners.delete(fn);
}

function emitRate() {
  rateListeners.forEach((fn) => fn());
}

/** 当前倍率（未计算过返回 null）。组件侧请用 `shared/useHdRate` 的钩子以自动重渲染。 */
export function peekHdRate(): number | null {
  return cachedRate;
}

/** 显式设置倍率（设置项 / 调试覆盖用） */
export function setHdRate(rate: number): void {
  cachedRate = Math.max(1, Math.min(MAX_HD_RATE, Math.floor(rate) || 1));
  emitRate();
}

/**
 * 取当前倍率（首次调用会探测显示器并缓存；此后幂等）。
 *
 * 非 Tauri（浏览器 mock）恒为 1 —— mock 缓存没有高清变体。
 *
 * ⚠️ 探测是**异步**的：不要在渲染里"同步读一次就完事"（首帧会定格在 1x），
 * 用 `useHdRate()` 钩子订阅，探测完成会自动重渲染。
 */
export async function getHdRate(): Promise<number> {
  await probe();
  return cachedRate ?? 1;
}

/** 取基准物理倍率（与 `getHdRate` 共用一次探测）。后端分级算倍率用。 */
export async function getHdPhys(): Promise<number> {
  await probe();
  return cachedPhys ?? 1;
}

/** 探测显示器并缓存 `phys` / `rate`（幂等；并发安全） */
function probe(): Promise<void> {
  if (cachedPhys != null) return Promise.resolve();
  if (!probing) {
    probing = (async () => {
      const monitors = isTauri() ? await listMonitors() : null;
      cachedPhys = computeHdPhys(monitors);
      cachedRate = computeHdRate(monitors);
      emitRate();
    })().finally(() => {
      probing = null;
    });
  }
  return probing;
}

/** 已是高清变体的 URL（`…@2x.webp`，允许 query） */
const HD_VARIANT_RE = /@\d+x\.webp(\?.*)?$/i;

/**
 * 高清变体 URL：`images/a.webp` → `images/a@2x.webp`。
 *
 * - `rate <= 1` 或空 URL → 原样返回；
 * - **已是高清变体**（`…@2x.webp`）→ 原样返回，见下；
 * - 只认 `.webp`（缓存素材的唯一格式），其它扩展名不动，避免误改外链/领域自备图。
 *
 * ⚠️ 幂等性不是洁癖：`HdIcon` 会对传进来的 src **再算一次候选链**，而调用方常常已经
 * 传了候选链里的某一项（如 `fullImgCandidates(...)[0]`）。没有这条护栏时
 * `a@4x.webp` 会被再次叠加成 `a@4x@4x.webp` / `a@4x@3x.webp` …，而
 * `imgCandidates` 的**兜底最后一项恰好是这个叠加后的 URL** —— 于是"兜底"兜到的
 * 是一个永不存在的文件，图片必然空白（实测：生物详情「升级自/可升级为」整行无图，
 * 而同一份缓存里原图文件是好的）。已有的 `@Nx` 一律视为"调用方已经选好了档位"。
 */
export function hdVariantUrl(url: string, rate: number): string {
  if (!url || rate <= 1) return url;
  if (HD_VARIANT_RE.test(url)) return url;
  return url.replace(/\.webp(\?.*)?$/i, (_m, q) => `@${rate}x.webp${q ?? ""}`);
}

/**
 * 「已知该高清变体不存在」记忆。
 *
 * 素材是**按需/增量**生成的：大量素材在高分屏机器上也可能暂时没有 @Nx。
 * 每个缺失 URL 若每次渲染都重试会持续触发 404 请求，故在此记住，
 * 直到素材阶段补生成后由 `clearHdMissing()` 清空。
 */
const missingHd = new Set<string>();

export function markHdMissing(url: string): void {
  if (url) missingHd.add(url);
}

export function isHdMissing(url: string): boolean {
  return missingHd.has(url);
}

/**
 * 「高清可用性版本号」：清空缺失记忆时自增，**驱动所有图片重新尝试高清**。
 *
 * 为什么不能只清 `missingHd`：图片组件的候选链是在渲染时算出来的，
 * 清掉记忆不会让已经渲染的 `<img>` 重新取源（它们的 `src` 还是原图）。
 * 订阅本版本号 + 在变化时回到候选链首项，才能实现"高清生成完成后自动切高清"。
 */
let hdEpoch = 0;
const epochListeners = new Set<() => void>();

export function subscribeHdEpoch(fn: () => void): () => void {
  epochListeners.add(fn);
  return () => epochListeners.delete(fn);
}

export function peekHdEpoch(): number {
  return hdEpoch;
}

/**
 * 高清阶段完成后调用：清掉"已知无高清"记忆并**通知所有图片重试**（渐进增强）。
 *
 * 调用点：`modcache/status.ts` 的阶段三结束时（此时 `@Nx` 已落盘）。
 */
export function clearHdMissing(): void {
  missingHd.clear();
  hdEpoch++;
  epochListeners.forEach((fn) => fn());
}

/**
 * 候选 src 列表：**高清优先，原图兜底**。
 *
 * 调用方（组件）从第一个开始试，`onError` 时切下一个（见 `SafeImg` 的回退用法）。
 * `rate <= 1` 或该高清变体已知缺失时，只有原图一项。
 */
export function imgCandidates(url: string, rate: number): string[] {
  if (!url) return [];
  if (rate <= 1) return [url];
  const out: string[] = [];
  // ⚠️ 必须**逐档降序全试**（4x → 3x → 2x）：后端按用途分级生成，
  // 大图（城镇幕布）常常只到 2x，而小图（兵种/英雄）拿满倍率 ——
  // 前端不知道每张图实际拿了几倍，只试最高档会让大图白白退回原图。
  // 未命中的档位由 `markHdMissing` 记住，同一路径只付一次 404。
  //
  // ⚠️ "404 很便宜"**只对前端成立**：每次未命中还会经 Tauri 资产协议，
  // 由 `tauri::protocol::asset` 以 ERROR 级写一行应用日志（见 `infra/logging.rs` 的
  // `NOISY_TARGETS`）。故这里的记忆逻辑必须**每一档都记**，不能只记首档
  // （`HdImg` 的 `onError` 同理）。
  for (let r = Math.min(rate, MAX_HD_RATE); r >= 2; r--) {
    const hd = hdVariantUrl(url, r);
    if (hd === url || isHdMissing(hd)) continue;
    out.push(hd);
  }
  out.push(url);
  return out;
}
