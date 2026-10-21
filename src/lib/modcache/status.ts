/**
 * Wiki 缓存后台构建状态（轻量事件总线）。
 * App 初始化后后台触发构建；WikiView 订阅状态显示进度条。
 */
import { buildModsWikiBatch, cancelWikiBuild, fetchWikiBuildProgress, isTauri, onWindowFocusChanged, probeWikiCacheState, readLauncherConfig, releaseWikiCaches, scanRealMods } from "@/lib/tauri";
import { logInfo } from "@/lib/log";
import { clearHdMissing, getHdPhys, getHdRate } from "@/lib/hd";
import { loadWikiIndex } from "./index";
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
   *
   * `prep`（扫描模组目录）是**所有页面共用的前置阶段**：排序前要对每个 mod 目录
   * 递归枚举估成本，冷缓存下是分钟级（HDD 实测 63 s）且发生在第一个 mod 之前。
   * 它不属任何页面的 `deps`（不参与撤除判定），但必须始终可见，否则那段
   * 分钟级工作在 UI 上就是"0% 静止一分钟"（用户读作卡死）。
   */
  stages?: {
    prep: { done: number; total: number };
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
  /**
   * **启动预检中**（`readLauncherConfig` + `scanRealMods` + `probeWikiCacheState`）。
   *
   * 预检期**刻意不置 `busy`**（既定设计：全部命中时页面用现有 `index.json` 立即渲染，
   * 不被"生成模组缓存"的重型蒙层整页阻塞）。但**无索引可渲染**时（首次运行 /
   * 缓存被清空）不能放行页面：那会先渲染一段"分类全 0 + 未找到匹配条目"的空百科
   * （用户读作"没有数据"），直到预检结束才跳进蒙层 —— 观感是"卡在空页，过一会
   * 才显示正在生成"。
   *
   * 故预检期由 `useWikiStagesDone` 结合 `indexReady` 决定是否挡页面；
   * 预检结束（无需构建 / 置 `busy` / 异常）时本标志与 `indexReady` 一并清除。
   */
  preparing?: boolean;
  /**
   * 预检期读取的**本地索引是否可用**（`index.json` 存在且 `mods` 非空）。
   *
   * 与后端 `wiki_cache_state` 出参的 `indexReady` 同口径，但**预检开始时就已知**
   * （前端直读一次，~ms 级，无需等后端）—— 正是"无索引 → 预检期即挡页面"的判据。
   */
  indexReady?: boolean;
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

/**
 * 预检中标志（模块级）。
 *
 * 预检阶段**不置 `busy`**（好让页面立即渲染），故 `busy` 不能再兼任"已有任务在跑"的互斥锁 ——
 * 否则并发调用（如领域流程的 `ensureWikiCacheBuilt`）会重复跑预检并各自启动一次构建。
 */
let probing = false;

/**
 * 全局构建是否**活跃**（预检中 `probing` 或构建中 `busy`）。
 *
 * 用途：领域 bundled 补建与全局构建**共用** `build_mods_cache_batch` 命令和
 * `wiki-build-progress` 事件通道（无批次标识），并发会互相打乱对方的进度账本
 * （2026-10-21 真机实测：全局蒙层 data 行停在中途值而 assets 已在跑），
 * 且两份 rayon 并发争抢 CPU。外部排队逻辑（`App.tsx` 的 bundled 补建）用它
 * 判断"该不该等"。
 */
export function isWikiBuildActive(): boolean {
  return status.busy || probing;
}

/** 请求暂停后台构建（已完成的 mod 缓存保留，下次续跑） */
export async function pauseWikiBuild(): Promise<void> {
  cancelRequested = true;
  try {
    await cancelWikiBuild();
  } catch {
    // 非 Tauri 环境 / 构建未启动，忽略（取消是尽力而为）
  }
}

/**
 * 窗口焦点跟踪：构建期轮询兜底在**失焦时暂停**。
 *
 * 为什么值得单独做（2026-10-21 真机反馈：构建期把鼠标移出启动器窗口，
 * 光标会不定期转圈）：WebView 对 `WM_SETCURSOR` 的响应被主线程的周期性工作
 * 延迟时，Windows 会临时显示忙碌光标。构建期主线程被 ~10 次/秒的进度事件
 * + 2.5 次/秒的轮询 IPC 持续唤醒 —— 用户把注意力移出窗口后，本不该再有
 * 任何周期性唤醒。失焦暂停轮询零损失（用户不看进度，恢复焦点后最多
 * 400ms 即追上；进度事件与焦点无关，照常到达）。
 */
let windowFocused = true;
let focusBound = false;
function bindWindowFocusOnce(): void {
  if (focusBound || !isTauri()) return;
  focusBound = true;
  void onWindowFocusChanged((focused) => {
    windowFocused = focused;
  });
}

/** 后台预构建全部 mod 缓存（多线程批量，增量：hash 命中跳过）。可重复调用，busy 时不重复。 */
export async function ensureWikiCacheBuilt(): Promise<void> {
  // 浏览器自测模式：无 Tauri 后端，构建不可用，直接消费 mock 缓存目录数据
  if (!isTauri()) return;
  if (status.busy || probing) return;
  probing = true;
  cancelRequested = false;
  let lastEmit = 0;
  try {
    // 预检开始即广播 `preparing`：让"无索引可渲染"的宿主（首次运行的百科页等）
    // 立刻进入等待态，而不是先渲染一段空列表（详见字段注释）。
    // ⚠️ 必须放在 try 内：`loadWikiIndex` 若抛异常，靠下方 finally 清 `probing`，
    // 否则并发入口（`if (status.busy || probing) return`）会被永久挡住。
    status = { ...status, preparing: true };
    emit();
    // 预检期先直读一次索引（一次 IPC，~ms 级）：有索引 → 老用户的增量重建照常
    // 放行页面（既定设计 C）；无索引 → 宿主显示等待蒙层，直到预检结束置 `busy`
    // 无缝接力，不再出现"先空列表、后跳蒙层"的闪断。
    const idx = await loadWikiIndex();
    status = { ...status, indexReady: !!(idx && idx.mods.length > 0) };
    emit();
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
    if (all.length === 0) return;

    // ── B/C：启动预检（**刻意不置 `busy`**）─────────────────────────────
    // 与真实构建**同源判定**（后端 `wiki_cache_state`，二者共用 `probe_mod_cache`）：
    // "全部命中 + 索引已就绪" → 本次无需任何构建，直接返回。
    // 因未置 `busy`：预检期间 wiki/mods/领域页用现有 `index.json` **立即渲染**（C，
    // 不再被"生成模组缓存"的重型蒙层整页阻塞）；全部命中时更不会出现进度条（B）。
    // 预检返回 `null`（命令缺失/异常）或有活要干 → 落入下方既有两阶段构建
    // （兜底：预检失败绝不漏建）。
    const probe = await probeWikiCacheState(gameDir, all);
    if (probe?.ok && probe.indexReady && probe.needData.length === 0 && probe.needAssets.length === 0) {
      return;
    }
    // 有活要干：**此刻才置 `busy`**（蒙层才出现），随后进入既有两阶段构建。
    // `preparing` / `indexReady` 到此使命结束，显式清除（预检 → 构建无缝接力）。
    status = { busy: true, preparing: false, progress: { done: 0, total: 0 } };
    emit();
    // 前端侧唯一的构建起点日志（后端只有批次汇总行）：与 `[boot]` 一起构成
    // "启动 → 预检 → 准备 → 构建" 的完整时间线，排查"卡在 0%"时不用猜。
    logInfo("wiki", `缓存构建开始：${all.length} 个 mod`);

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
      // 200ms 节流（同 setStage，理由见那里）
      if (now - lastEmit > 200 || done >= total) {
        lastEmit = now;
        status = { ...status, progress: { done: doneBase + done, total: grandTotal } };
        emit();
      }
    };

    // 进度轮询兜底的定时器句柄（在 try 内创建、finally 内清理，见下方 `poll`）
    let poll: number | undefined;

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
        prep: { done: 0, total: totalData },
        data: { done: 0, total: totalData },
        assets: { done: 0, total: assetsTotal },
        hd: { done: 0, total: assetsTotal },
      };
      let stageEmit = 0;
      const setStage = (k: "prep" | "data" | "assets" | "hd", done: number) => {
        // **单调保护**（done 只增不减）：`wiki-build-progress` 事件通道没有批次标识，
        // 并发的其它批次（领域 bundled 补建，走同一个 `build_mods_cache_batch` 命令）
        // 会把本阶段的进度打回旧值 —— 2026-10-21 真机实测：全局 data 行停在 18/124
        // 而 assets 已在 21/124（bundled 批次只有十几个 mod，其 done 覆盖了全局的）。
        const d = Math.max(st[k].done, done);
        const now = Date.now();
        // 200ms 节流（原 120ms）：进度事件本身已是 ~10 次/秒，再往上加密 emit
        // 只会让主线程更频繁唤醒（光标转圈反馈的来源之一），而 200ms 的
        // 显示延迟肉眼不可感知（StageRows 还有 1% 量化兜底）。
        if (now - stageEmit > 200 || d >= st[k].total) {
          stageEmit = now;
          st[k] = { ...st[k], done: d };
          status = { ...status, stages: { ...st } };
          emit();
        }
      };
      /**
       * 阶段**成功完成**后显式标满（`done = total`）。
       *
       * 两个必要性：① 事件可能丢失（末帧没到前端）→ 进度永远差一截；
       * ② 并发批次污染后本阶段停在中途值。而**蒙层撤除判据就是 `done >= total`** ——
       * 不到 100% 蒙层永驻，用户被锁在"正在生成缓存"里。
       * 仅在 invoke **成功返回**（`!== null`）时调用：失败不谎报完成。
       */
      const markStageFull = (k: "data" | "assets" | "hd") => {
        if (st[k].done >= st[k].total) return;
        st[k] = { done: st[k].total, total: st[k].total };
        status = { ...status, stages: { ...st } };
        emit();
      };
      /**
       * 准备阶段（成本估算）进度。
       *
       * ⚠️ 取**历史最大** total/done：阶段二/三会各自再走一次成本估算预热，
       * 而它的 total 是本批数量（town/rest 批远小于 572）——直接覆盖会让
       * "扫描模组目录"从 572/572 倒退成 20/20。
       */
      let prepLogged = false;
      const applyPrep = (done: number, total: number) => {
        const t = Math.max(st.prep.total, total);
        const d = Math.min(t, Math.max(st.prep.done, done));
        if (d === st.prep.done && t === st.prep.total) return;
        st.prep = { done: d, total: t };
        status = { ...status, stages: { ...st } };
        emit();
        // 准备阶段收尾打一行（每轮至多一条）：它是"第一个 mod 开工前"那段
        // 分钟级目录估算的唯一日志痕迹，排查"首屏前卡住"必备。
        if (!prepLogged && d >= t && t > 0) {
          prepLogged = true;
          logInfo("wiki", `准备阶段完成：扫描 ${t} 个 mod 目录（成本估算）`);
        }
      };
      /** 某一段的进度回调：`base` 用于把多批（town/rest）拼成一条连续进度 */
      const mkStage = (k: "data" | "assets" | "hd", base: number) => (done: number) =>
        setStage(k, base + done);
      /** 带准备阶段分流的进度回调：`prep=true` 走"扫描模组目录"行，否则走本阶段行 */
      const mkBatch =
        (k: "data" | "assets" | "hd", base: number) =>
        (done: number, total: number, prep?: boolean) => {
          if (prep) applyPrep(done, total);
          else mkStage(k, base)(done);
        };

      // ── 阶段一：全部 mod，仅数据（phase=0）──
      // 整批一次提交：后端在独立线程完成全部（不依赖前端 JS 存活，窗口最小化/失焦也不会停）
      // **优先级最高**：跑完这一批，模组列表/百科文本就全部可用了。
      // ⚠️ 用 **turbo（2 → cpus/2）**：这是"可用性关键路径"，用户此刻就在等列表；
      // 后台静默构建（ECO）会把这几分钟拖成十几分钟。
      status = { ...status, stage: "data", hdEnabled: !!hdOpt, hdProgress: null, stages: { ...st } };
      emit();

      // 进度轮询兜底（事件可能不达；400ms 拉取一次）。
      // ⚠️ 必须推进**可见的**阶段条：此前只写 `status.progress`，而蒙层渲染的是
      // `status.stages` → 事件真丢失时进度条仍停在 0%，文档承诺的"不会卡死"落空
      // （见 docs/features/modcache.zh-CN.md）。
      // ⚠️ 只在 `stage === "data"` 时写 data 行：阶段二/三的批计数是 town/rest
      // **子集**（0..本批数），直接写会把已完成的 data 行打回 0 → 蒙层重新弹出。
      bindWindowFocusOnce();
      poll = window.setInterval(() => {
        // 失焦时跳过（见 `windowFocused` 注释）—— 用户不看进度，主线程不该
        // 再被 400ms 的 IPC 唤醒；恢复焦点后下一个 tick 即追上。
        if (!windowFocused) return;
        void (async () => {
          const p = await fetchWikiBuildProgress();
          if (p && p.total > 0) {
            if (p.prepTotal > 0) applyPrep(p.prepDone, p.prepTotal);
            if (status.stage === "data") setStage("data", p.done);
            status = { ...status, progress: { done: doneBase + p.done, total: grandTotal } };
            emit();
          }
        })();
      }, 400);

      const rData = await buildModsWikiBatch(gameDir, all, mkBatch("data", 0), DATA_BUILD_THROTTLE, 0);
      if (rData) markStageFull("data");
      doneBase = totalData;

      // ── 阶段二：标清素材（**刻意不带 hd**）──
      // 与高清拆开：标清跑完即"所有页面图片可用"，不再被最慢的高清拖住
      // （此前两者同批，用户的'可用时刻'被推迟到高清完成）。
      status = { ...status, stage: "assets" };
      emit();
      const rTown = await buildModsWikiBatch(
        gameDir,
        townBatch,
        mkBatch("assets", 0),
        WIKI_BUILD_THROTTLE,
        1,
      );
      doneBase += townBatch.length;
      const rRest = await buildModsWikiBatch(
        gameDir,
        restBatch,
        mkBatch("assets", townBatch.length),
        WIKI_BUILD_THROTTLE,
        1,
      );
      // 两批都成功才算阶段二完成（任一批失败不标满，保留真实进度）
      if (rTown && rRest) markStageFull("assets");

      // ── 阶段三：高清（增强项；独立进度 + 独立 CPU 档；**接着跑**）──
      // 阶段二已完成 → 后端命中缓存跳过提取、`generate_for_mod` 幂等跳过已完成的
      // → 这一趟**只做高清**，几乎不重复劳动（拆分的成本≈0）。
      if (hdOpt) {
        const hdTotal = townBatch.length + restBatch.length;
        status = { ...status, stage: "hd", hdProgress: { done: 0, total: hdTotal } };
        emit();
        let hdLastEmit = 0;
        const mkHdProgress =
          (base: number) => (done: number, total: number, prep?: boolean) => {
            // 准备阶段（成本估算预热）只推进"扫描模组目录"行，不碰高清进度
            if (prep) {
              applyPrep(done, total);
              return;
            }
            const now = Date.now();
            // 单调保护（同 setStage）：防并发批次把高清进度打回
            const d = Math.max(st.hd.done, base + done);
            if (now - hdLastEmit > 120 || d >= total) {
              hdLastEmit = now;
              st.hd = { ...st.hd, done: d };
              status = { ...status, hdProgress: { done: d, total: hdTotal }, stages: { ...st } };
              emit();
            }
          };
        const rHdTown = await buildModsWikiBatch(
          gameDir,
          townBatch,
          mkHdProgress(0),
          HD_BUILD_THROTTLE,
          1,
          hdOpt,
        );
        const rHdRest = await buildModsWikiBatch(
          gameDir,
          restBatch,
          mkHdProgress(townBatch.length),
          HD_BUILD_THROTTLE,
          1,
          hdOpt,
        );
        if (rHdTown && rHdRest) {
          markStageFull("hd");
          status = { ...status, hdProgress: { done: hdTotal, total: hdTotal } };
          emit();
        }
        // 高清就绪 → 清掉"已知无高清"的记忆，让页面自动改用高清变体（渐进增强）
        clearHdMissing();
      }
    } finally {
      if (poll !== undefined) window.clearInterval(poll);
      // 整轮构建（阶段一/二/三，含被暂停提前结束）走完 → 释放后端构建期缓存，让内存回落。
      // 只能在这里收尾：后端单看一批不知道后面还有没有下一批，批间清空会白付一次全量扫描。
      void releaseWikiCaches();
    }
    // 用户暂停时不谎报完成，保留实际进度（下次调用增量续跑）
    if (!cancelRequested) {
      status = { ...status, progress: { done: grandTotal, total: grandTotal } };
    }
  } catch {
    status = { ...status, progress: null };
  } finally {
    probing = false;
    // 无论"无需构建直接返回 / 构建完成 / 异常"，预检标志都必须清除：
    // 残留 `preparing` + `indexReady=false` 会让宿主永远判为未就绪（蒙层永驻）。
    status = { ...status, busy: false, preparing: false };
    emit();
  }
}
