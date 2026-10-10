/**
 * Wiki 缓存后台构建状态（轻量事件总线）。
 * App 初始化后后台触发构建；WikiView 订阅状态显示进度条。
 */
import { buildModsWikiBatch, cancelWikiBuild, fetchWikiBuildProgress, isTauri, readLauncherConfig, scanRealMods } from "@/lib/tauri";
import { clearHdMissing, getHdPhys, getHdRate } from "@/lib/hd";
import { buildCoreEntries, flattenTree, toBatchPayload } from "./buildTargets";

interface WikiBuildStatus {
  busy: boolean;
  progress: { done: number; total: number } | null;
  /**
   * 当前构建阶段（进度条**分段显示**用）：
   * `data`=游戏数据（最高优先，完成即"模组列表可用"）/ `assets`=标清图片 / `hd`=高清（增强项）。
   */
  stage?: "data" | "assets" | "hd";
  /**
   * **高清阶段的独立进度**（分段双条的第二条）。
   * 未开始或高清关闭时为 `null`（UI 据此不显示该条）。
   */
  hdProgress?: { done: number; total: number } | null;
  /**
   * **三条独立进度**（分段进度条用；核对文档见 `docs/features/modcache.zh-CN.md`）。
   *
   * 与 `progress`（总进度）并存：总进度给"还要多久"的整体感知，三段给"哪一段在跑/还剩几段"。
   * 未开始的段保持 `done=0`，UI 据此显示 `0%`（让用户一开始就知道总共几段）。
   * `hd` 段在开关关闭时始终为 0，UI 按 `hdEnabled` 决定是否渲染该条。
   */
  stages?: {
    data: { done: number; total: number };
    assets: { done: number; total: number };
    hd: { done: number; total: number };
  };
  /**
   * 本次运行**是否会做高清**（= 总开关开 且倍率 ≥2）。
   *
   * 与 `hdProgress` 的区别：它在阶段一开始就已知，UI 据此**预先显示**高清进度条（0%、待开始），
   * 用户一开始就知道总共要经历几段，而不是等到高清阶段突然多出一条。
   */
  hdEnabled?: boolean;
}

/**
 * 高清阶段的 CPU 档位：**eco（空闲感知，后端档位 0）**。
 *
 * 高清是**纯增强项**（永不出现在页面蒙层，前端候选链自动回退标清），故“低影响”优先于
 * “尽快完成”：用户在用电脑时 = `cpus/8`；长时间无操作（≥30s → `cpus/4`，≥5min →
 * `cpus/2`）时自动提速。后端按**块（8 个 mod）**重估档位 ⇒ 用户回来最多一块即回落 ECO。
 *
 * （曾用 normal=1 固定 `cpus/4`：实测用户在场时也被持续占用、观感是“整台电脑被吃满”。）
 */
const HD_BUILD_THROTTLE: 0 | 1 | 2 = 0;

/**
 * `data` 阶段的 CPU 档位：**turbo(2) → `cpus/2`**。
 *
 * data 是**可用性关键路径**（mods 列表文本、wiki 条目、巡礼脚本都等它），用户此刻就在等，
 * 不能沿用后台静默的 ECO（`cpus/8`，会把几分钟拖成十几分钟）。
 * 不用 `3`（全核）：那会明显拖慢用户同时使用电脑。
 */
const DATA_BUILD_THROTTLE: 0 | 1 | 2 = 2;

let status: WikiBuildStatus = { busy: false, progress: null };
const listeners = new Set<() => void>();

export function subscribeWikiStatus(fn: () => void): () => void {
  listeners.add(fn);
  return () => listeners.delete(fn);
}

function emit() {
  listeners.forEach((fn) => fn());
}

export function getWikiStatus(): WikiBuildStatus {
  return status;
}

// 构建目标清单的工具（原版伪模组 / 子模组展开 / 载荷字段）**与领域加急队列共用**，
// 见 `./buildTargets`。两套通道各写一份必然出现"一个建过、另一个不认"的错位。

/**
 * 构建档位：**固定传 0（空闲感知自动档）**。
 *
 * 后端 `wiki_threads()` 里 `0` 与 `None` 都落入空闲感知分支：
 * 空闲 ≥5min → `cpus/2`；≥30s → `cpus/4`；否则 → `cpus/8`（ECO）。
 * 即：用户在用电脑时安静构建，长时间无操作时自动提档，无需前端干预。
 * 原 `setWikiBuildThrottle`（手动 1/2 档）已被自动档取代，**已移除**。
 */
const WIKI_BUILD_THROTTLE = 0;

/** 暂停标志：由 pauseWikiBuild 置位，后端 job 在每个 mod 开始前检查（最多等已分发的少量 mod 跑完） */
let cancelRequested = false;

/** 请求暂停后台构建（已完成的 mod 缓存保留，下次续跑） */
export async function pauseWikiBuild(): Promise<void> {
  cancelRequested = true;
  try {
    await cancelWikiBuild();
  } catch {
    // 非 Tauri 环境 / 构建未启动，忽略（取消是尽力而为）
  }
}

/** 后台预构建全部 mod 缓存（多线程批量，增量：hash 命中跳过）。可重复调用，busy 时不重复。 */
export async function ensureWikiCacheBuilt(): Promise<void> {
  // 浏览器自测模式：无 Tauri 后端，构建不可用，直接消费 mock 缓存目录数据
  if (!isTauri()) return;
  if (status.busy) return;
  cancelRequested = false;
  status = { busy: true, progress: { done: 0, total: 0 } };
  let lastEmit = 0;
  emit();
  try {
    const cfg = await readLauncherConfig();
    const gameDir = cfg?.gameDir ?? "";
    const mods = (await scanRealMods()) as Record<string, unknown>[];
    const scanned = toBatchPayload(flattenTree(mods));

    // 注入原版伪模组：由「单个 core」改为「父 core + 9 个 core.<faction> 子模组」。
    // 原因：原版 9 城镇/生物/英雄只存在于游戏根 config/factions、creatures、heroes，
    // 而 scan_mods 因 RESERVED_SCOPES(["core","vcmi"]) 跳过了基础 mod，scan 结果里没有它们。
    // 拆成子模组后，每个 `core.<faction>` 的缓存形态与第三方 town mod 完全一致
    // （factions/creatures/heroes），前端可按普通 town mod 渲染；
    // 父 core 承载全局与无归属实体（法术/技能/宝物/地形/中立生物等），不持有城镇。
    // 后端由 modId 本身识别作用域（见 parse_core_mod），无需额外参数。
    // 条目构造统一走 `buildCoreEntries`（原版伪模组的显示名由后端派生，前端不携带文案）。
    const all = [...buildCoreEntries(gameDir), ...scanned];

    // 分两批：城镇类型优先 —— 模组页的城镇卡片依赖 preview 合成图，
    // 先构建城镇可让模组页尽快可用；其余类型后置，由后台逐步补充
    // wiki 页面的建筑/生物/英雄等完整数据。
    const townBatch = all.filter((m) => m.type === "Town");
    const restBatch = all.filter((m) => m.type !== "Town");
    // 需携带 diskName/parentId/type/name：后端重建 index.json 时从 serde flatten 的
    // extra 读取这些字段（commands.rs 6061+），只传 modId/modPath 会导致索引字段全空。
    // parentId 尤为关键——子模组缓存平铺存放，父子关系完全依赖 index.json 的该字段表达。
    // `all` / `townBatch` / `restBatch` 已是 `BatchPayload`（由 `toBatchPayload` 与
    // `buildCoreEntries` 产出，字段与后端 flatten extra 对齐），无需再转换一次。
    // 两阶段分批：**阶段一先让全部 mod 的文本数据整体就绪**（廉价，不解码 DEF/不编码 webp/
    // 不合成预览），wiki/realm 页面即可显示（无图可接受）；后端每批结束会重建 index.json，
    // 故阶段一完成后页面马上能列出全部模组。
    // 阶段二再分批补图：城镇类型优先（模组页尽快出大图），其余类型后置。
    // 若把「数据+素材」串在每个 mod 内部，后台 eco 档仅 1~2 线程时，
    // 队尾 mod 的数据会被前面所有 mod 的素材提取长时间阻塞——这正是要避免的。
    const totalData = all.length;
    const grandTotal = totalData + townBatch.length + restBatch.length;
    let doneBase = 0;
    status = { ...status, progress: { done: 0, total: grandTotal } };
    emit();

    const mkProgress = () => (done: number, total: number) => {
      const now = Date.now();
      if (now - lastEmit > 120 || done >= total) {
        lastEmit = now;
        status = { ...status, progress: { done: doneBase + done, total: grandTotal } };
        emit();
      }
    };

    // 进度轮询兜底（事件可能不达；400ms 拉取一次，UI 刷新平滑且必然推进）
    const poll = window.setInterval(async () => {
      const p = await fetchWikiBuildProgress();
      if (p && p.total > 0) {
        status = { ...status, progress: { done: doneBase + p.done, total: grandTotal } };
        emit();
      }
    }, 400);

    try {
      // 高清选项：倍率上限 + 基准物理倍率按**显示器物理宽**推导（取全部显示器最大值，
      // 窗口可拖到副屏），见 `lib/hd.ts`；`phys` 交给后端按用途分级算倍率。
      // **总开关 `hdEnhance` 默认关**：关闭时 `hdOpt` 为 undefined → 阶段二/三都不传 hd 参数
      // → 后端直接跳过高清分支。`aiHdEnhance` 决定高清时插画档是否走 AI。
      // ⚠️ 必须在阶段一**之前**算出来：UI 要一开始就显示"高清段（0%，待开始）"。
      const rate = await getHdRate();
      const hdOpt =
        cfg?.hdEnhance === true && rate >= 2
          ? { rate, phys: await getHdPhys(), ai: cfg?.aiHdEnhance === true }
          : undefined;

      // ── 三条独立进度的账本（分段进度条用）──
      const assetsTotal = townBatch.length + restBatch.length;
      const st = {
        data: { done: 0, total: totalData },
        assets: { done: 0, total: assetsTotal },
        hd: { done: 0, total: assetsTotal },
      };
      let stageEmit = 0;
      const setStage = (k: "data" | "assets" | "hd", done: number) => {
        const now = Date.now();
        if (now - stageEmit > 120 || done >= st[k].total) {
          stageEmit = now;
          st[k] = { ...st[k], done };
          status = { ...status, stages: { ...st } };
          emit();
        }
      };
      /** 某一段的进度回调：`base` 用于把多批（town/rest）拼成一条连续进度 */
      const mkStage = (k: "data" | "assets" | "hd", base: number) => (done: number) =>
        setStage(k, base + done);

      // ── 阶段一：全部 mod，仅数据（phase=0）──
      // 整批一次提交：后端在独立线程完成全部（不依赖前端 JS 存活，窗口最小化/失焦也不会停）
      // **优先级最高**：跑完这一批，模组列表/百科文本就全部可用了。
      // ⚠️ 用 **turbo（2 → cpus/2）**：这是"可用性关键路径"，用户此刻就在等列表；
      // 后台静默构建（ECO）会把这几分钟拖成十几分钟。
      status = { ...status, stage: "data", hdEnabled: !!hdOpt, hdProgress: null, stages: { ...st } };
      emit();
      await buildModsWikiBatch(gameDir, all, mkStage("data", 0), DATA_BUILD_THROTTLE, 0);
      doneBase = totalData;

      // ── 阶段二：标清素材（**刻意不带 hd**）──
      // 与高清拆开：标清跑完即"所有页面图片可用"，不再被最慢的高清拖住
      // （此前两者同批，用户的'可用时刻'被推迟到高清完成）。
      status = { ...status, stage: "assets" };
      emit();
      await buildModsWikiBatch(gameDir, townBatch, mkStage("assets", 0), WIKI_BUILD_THROTTLE, 1);
      doneBase += townBatch.length;
      await buildModsWikiBatch(
        gameDir,
        restBatch,
        mkStage("assets", townBatch.length),
        WIKI_BUILD_THROTTLE,
        1,
      );

      // ── 阶段三：高清（增强项；独立进度 + 独立 CPU 档；**接着跑**）──
      // 阶段二已完成 → 后端命中缓存跳过提取、`generate_for_mod` 幂等跳过已完成的
      // → 这一趟**只做高清**，几乎不重复劳动（拆分的成本≈0）。
      if (hdOpt) {
        const hdTotal = townBatch.length + restBatch.length;
        status = { ...status, stage: "hd", hdProgress: { done: 0, total: hdTotal } };
        emit();
        let hdLastEmit = 0;
        const mkHdProgress = (base: number) => (done: number, total: number) => {
          const now = Date.now();
          if (now - hdLastEmit > 120 || done >= total) {
            hdLastEmit = now;
            const d = base + done;
            st.hd = { ...st.hd, done: d };
            status = { ...status, hdProgress: { done: d, total: hdTotal }, stages: { ...st } };
            emit();
          }
        };
        await buildModsWikiBatch(
          gameDir,
          townBatch,
          mkHdProgress(0),
          HD_BUILD_THROTTLE,
          1,
          hdOpt,
        );
        await buildModsWikiBatch(
          gameDir,
          restBatch,
          mkHdProgress(townBatch.length),
          HD_BUILD_THROTTLE,
          1,
          hdOpt,
        );
        // 高清就绪 → 清掉"已知无高清"的记忆，让页面自动改用高清变体（渐进增强）
        clearHdMissing();
      }
    } finally {
      window.clearInterval(poll);
    }
    // 用户暂停时不谎报完成，保留实际进度（下次调用增量续跑）
    if (!cancelRequested) {
      status = { ...status, progress: { done: grandTotal, total: grandTotal } };
    }
  } catch {
    status = { ...status, progress: null };
  } finally {
    status = { ...status, busy: false };
    emit();
  }
}
