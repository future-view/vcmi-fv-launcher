import { useSyncExternalStore } from "react";
import { useI18n } from "@/i18n";
import { getWikiStatus, subscribeWikiStatus } from "@/lib/modcache/status";
import { LoadingOverlay } from "@/shared/LoadingOverlay";

/** 缓存构建阶段（与 `status.stages` 的键一致） */
export type BuildStage = "prep" | "data" | "assets" | "hd";

/**
 * 三条进度条：`labelKey` 是 `t.wiki` 的键，`bar` 是配色。
 *
 * ⚠️ **文案一律走 i18n**（`src/i18n/{types,zh-CN,en,ru,pl,de}.ts`）—— 这里绝不硬编码中文/英文，
 * 否则某一语言界面下会漏出另一种语言。
 */
const ROWS = [
  { key: "prep", labelKey: "stagePrep", bar: "bg-slate-400" },
  { key: "data", labelKey: "stageData", bar: "bg-blue-500" },
  { key: "assets", labelKey: "stageAssets", bar: "bg-sky-500" },
  { key: "hd", labelKey: "stageHd", bar: "bg-emerald-500" },
] as const;

/** 阶段 → i18n 键（与 `ROWS` 的 `labelKey` 同一套） */
const STAGE_LABEL_KEY = {
  prep: "stagePrep",
  data: "stageData",
  assets: "stageAssets",
  hd: "stageHd",
} as const;

/**
 * 「何时可用」说明 —— **模板 + 依赖阶段名列表**拼接（文案全部取自 i18n）。
 *
 * ⚠️ **不要为每种依赖组合各写一句**：`data` / `data+assets` / `data+assets+hd` / 仅 `assets`…
 * 组合会越写越多，新增组合时必然漏改。模板化后**任意组合自动正确**。
 * ⚠️ 也不要把"完成即可用"塞进阶段标签里：同一阶段在不同页面含义不同
 * （`data` 对领域信息页够用，对百科 / 巡礼则必须连 `assets` 一起完成），写在标签里必然说谎。
 */
export function stageReadyHint(
  t: {
    wiki: {
      readyHintTemplate: string;
      readyHintRest: string;
      stageSeparator: string;
      stageData: string;
      stageAssets: string;
      stageHd: string;
    };
  },
  deps: readonly BuildStage[],
): string {
  const names = deps.map((d) => t.wiki[STAGE_LABEL_KEY[d]]).join(t.wiki.stageSeparator);
  return `${t.wiki.readyHintTemplate.replace("{stages}", names)}${t.wiki.readyHintRest}`;
}

/**
 * 只订阅「某阶段**是否已完成**」这一个布尔量。
 *
 * ## 为什么必须这样订阅
 *
 * 构建期间进度每 ~120ms 就 emit 一次。如果页面**顶层**订阅整个 `WikiBuildStatus`，
 * 每次 emit 都会重渲染整棵子树（含长列表）—— 而列表内容在阶段完成前根本不会变，
 * 这些渲染纯属浪费（正是"进度条没跑完就没必要实时刷新列表"的问题）。
 *
 * `useSyncExternalStore` 的 `getSnapshot` 返回 **boolean**，值不变时 React 会跳过重渲染，
 * 于是页面只在**完成跃迁的那一刻**刷新一次 ✓
 *
 * 语义：
 * - 未在构建（`busy=false`）→ `true`（空闲即视为完成，不阻塞页面）；
 * - 构建中但该段未开始（`total=0`）→ `false`（蒙层要出现，否则一开始就"完成"了）。
 */
export function useWikiStageDone(stage: BuildStage): boolean {
  return useSyncExternalStore(
    subscribeWikiStatus,
    () => {
      const s = getWikiStatus();
      if (!s.busy) return true;
      const p = s.stages?.[stage];
      return !!p && p.total > 0 && p.done >= p.total;
    },
    () => false,
  );
}

/**
 * 只订阅「**一组**阶段是否全部完成」—— `WikiBuildOverlay` 用它决定撤除时机。
 *
 * 与 [`useWikiStageDone`] 一样返回 boolean 快照（进度数字变化不触发重渲染）。
 * 空数组 = 无依赖 → 视为完成（不阻塞）。
 *
 * 除"构建中按阶段完成度"外，还有一类**未就绪**：**预检中且本地无可用索引**
 * （首次运行 / 缓存被清空，见 `status.ts` 的 `preparing` / `indexReady` 注释）。
 * 该场景若不挡住，宿主会先渲染一段空列表（百科 = "分类全 0 + 未找到匹配条目"，
 * 用户读作"没有数据"），直到预检结束置 `busy` 才跳进蒙层 —— 观感即
 * "卡在空页，过一会才显示正在生成"。反之**有索引**时放行：老用户的增量
 * 重建不被蒙层整页阻塞（既定设计，见 `status.ts` 预检段注释）。
 */
export function useWikiStagesDone(deps: readonly BuildStage[]): boolean {
  return useSyncExternalStore(
    subscribeWikiStatus,
    () => {
      const s = getWikiStatus();
      if (s.busy) {
        return deps.every((d) => {
          const p = s.stages?.[d];
          return !!p && p.total > 0 && p.done >= p.total;
        });
      }
      // 空数组 = 无依赖 → 始终不阻塞（先于预检分支，勿让预检挡住"无依赖"宿主）
      if (deps.length === 0) return true;
      // 预检中且本地无可用索引 → 未就绪（宿主应显示等待态，而非空列表）
      if (s.preparing === true && !s.indexReady) return false;
      return true;
    },
    () => false,
  );
}

/** 只订阅「启动预检中」（`ensureWikiCacheBuilt` 的 scan/probe 阶段，详见 `status.ts`） */
export function useWikiPreparing(): boolean {
  return useSyncExternalStore(
    subscribeWikiStatus,
    () => getWikiStatus().preparing === true,
    () => false,
  );
}

/**
 * 三条进度条。**这里是唯一高频重渲染的地方**（~120ms 一次），
 * 它只包含几个 div，代价可忽略；页面主体/列表不受影响。
 */
/**
 * 阶段进度条列表（**唯一高频重渲染的地方**）。
 * 导出给 `StageGate` 复用 —— 内联等待与蒙层等待显示同样的进度。
 */
export function StageRows({ stages }: { stages: readonly BuildStage[] }) {
  const { t } = useI18n();
  const s = useSyncExternalStore(subscribeWikiStatus, getWikiStatus, getWikiStatus);
  // `prep`（扫描模组目录）是**所有页面共用的前置阶段**：排序前要对每个 mod 目录递归
  // 枚举估成本，冷缓存下是分钟级且发生在第一个 mod 开工之前。它不属于任何页面的
  // `deps`（不参与撤除判定），但只要后端报过它就始终显示 —— 否则那段分钟级工作
  // 在 UI 上就是"0% 静止一分钟"（2026-10-16 实测 63 s，是"卡在 0%"投诉的来源）。
  const prep = s.stages?.prep;
  const rows = ROWS.filter(
    (r) =>
      (r.key === "prep"
        ? !!prep && prep.total > 0
        : stages.includes(r.key) && (r.key !== "hd" || s.hdEnabled)),
  );
  return (
    <div className="w-[60%] max-w-md space-y-1.5">
      {rows.map((r) => {
        const p = s.stages?.[r.key] ?? { done: 0, total: 0 };
        const pct = p.total > 0 ? Math.round((p.done / p.total) * 100) : 0;
        return (
          <div key={r.key} className="space-y-1">
            <div className="flex items-baseline justify-between text-[11px]">
              <span className="text-slate-400">{t.wiki[r.labelKey]}</span>
              <span className="tabular-nums text-slate-300">
                {p.done} / {p.total}（{pct}%）
              </span>
            </div>
            <div className="h-2.5 w-full overflow-hidden rounded-full bg-slate-800">
              <div
                className={`h-full rounded-full transition-all ${r.bar}`}
                style={{ width: `${Math.max(0, Math.min(100, pct))}%` }}
              />
            </div>
          </div>
        );
      })}
    </div>
  );
}

/**
 * 缓存构建蒙层（三条进度），wiki 页与 mods 页共用。
 *
 * ## 撤除条件 = **该页真正需要的阶段**完成，而不是"全部构建完成"
 *
 * 依赖矩阵见 `docs/features/modcache.zh-CN.md`：
 * - **wiki 页**：需 **data + assets**（百科缺图根本没法阅读）→ 等 `assets` 完成才撤蒙层；
 * - **mods 页**：**刻意不加蒙层** —— 它的列表字段（名称/依赖/冲突/子模组层级）全部来自
 *   `scan_mods`，不依赖缓存；`cache.json` 只提供城镇图片路径（缺图有占位，assets 完成后自动补齐）。
 *
 * ⚠️ `hd` 阶段**永不**作为 `requiredStage`（纯增强，前端候选链自动回退标清）。
 * ⚠️ 不要用半透明"不阻塞"蒙层：它与背后列表文字**互相干扰**，观感很差（已被实测否决）。
 */
export function WikiBuildOverlay({
  deps = ["data"],
  blocking = true,
  onExit,
}: {
  /**
   * 本页**依赖的阶段**（顺序 = 展示顺序）。
   *
   * - **全部完成才撤蒙层**（阶段是顺序跑的，"最后一个"即全齐）；
   * - 进度条**按此列表渲染** —— 不依赖的阶段不显示，避免"要等图片才能用"的误导。
   *
   * 例：`["data"]` → 领域信息 / 编辑领域；`["data","assets"]` → 百科 / 巡礼。
   * 将来若有"高清必需"的页面，直接写 `["data","assets","hd"]` 即可，**本组件无需改动**
   * （说明文案会由模板自动拼接三段的名称）。
   */
  deps?: readonly BuildStage[];
  /** `true`（默认）挡操作：内容未就绪；`false` 不挡：列表可用，仅提示 */
  blocking?: boolean;
  /**
   * **显式出口回调**。给了就渲染一个"返回"按钮。
   *
   * ⚠️ 不要用 z-index 去"避开"蒙层：蒙层会盖住整个容器，而容器里的控件
   * （返回按钮、Tab 栏…）往往处于更深的堆叠上下文，抬高它们内部的 z **是无效的**
   * （祖先整体被压）。正确做法是**在这里给出口**。
   */
  onExit?: () => void;
}) {
  const { t } = useI18n();
  const preparing = useWikiPreparing();
  const done = useWikiStagesDone(deps);
  // 显示条件 = `done` 取反（而非 `busy && !done`）：预检期无索引时 `busy` 仍为 false，
  // 但 `useWikiStagesDone` 已把该场景判为未就绪 —— 蒙层在预检期即接管，
  // 预检结束置 `busy` 后无缝接力，不出现"先空列表、后跳蒙层"的闪断。
  if (done) return null;
  return (
    <LoadingOverlay
      title={t.wiki.generating}
      hint={stageReadyHint(t, deps)}
      /* 显式出口：蒙层会盖住容器内的原有控件，若宿主在这里提供 onExit，
         用户任何时候都能离开（**不依赖** z-index 谁高谁低）。 */
      onExit={onExit}
      exitLabel={t.common.back}
      blocking={blocking}
    >
      {/* 预检期没有真实阶段进度（`stages` 全 0/0）→ 只留 spinner，
          不渲染三条 0%（那会读作"卡死在 0%"）。预检结束即有真实进度。 */}
      {!preparing && <StageRows stages={deps} />}
      {/* ⚠️ 刻意**不提供「暂停构建」**：构建是整批后台任务，"暂停"只会让本页需要的那一步
          停在原地（缓存保留但进度不前进），对用户没有收益 —— 各页面都在**自己所需的阶段**
          完成时就放行，用户不需要也不应该操心后台任务。
          改为按本页实际需求给出"何时可用"的准确说明（见上方 stageReadyHint 注入的 hint）。 */}
    </LoadingOverlay>
  );
}
