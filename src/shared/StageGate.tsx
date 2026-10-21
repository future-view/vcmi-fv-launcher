import { type ReactNode } from "react";
import { useI18n } from "@/i18n";
import {
  stageReadyHint,
  StageRows,
  useWikiPreparing,
  useWikiStagesDone,
  type BuildStage,
} from "@/shared/WikiBuildOverlay";

/**
 * 通用「等待缓存阶段」容器 —— 在**需要等待的区块**上包一层，
 * 而不是在页面根容器挂蒙层。
 *
 * ## 为什么不用"页面根蒙层"
 *
 * 根蒙层会**误伤**：把整页锁住，包括压根不依赖缓存的区块，以及页面自己的出口
 * （返回 / 关闭）。`RealmEditor` 就踩过 —— 6 个 Tab 里只有「阵营巡礼」依赖 `data`，
 * 却被整页遮住，而它没有外部页头返回，用户被困住。
 *
 * ## 后续改版怎么用
 *
 * 新增 / 调整 Tab 或区块时，**只需要在那一块包一层**，不必碰 z-index、不必改页面根容器：
 *
 *     <StageGate deps={["data"]}> … 该区块内容 … </StageGate>
 *
 * - 依赖组合变了（例如巡礼后来也需要 `assets`）→ 只改 `deps`；
 * - 文案与进度条**自动跟随**（模板拼接，见 `stageReadyHint`），无需新增任何文案；
 * - 未就绪时是**内容替换**（原地占位卡），不是覆盖 → 出口天然保留。
 *
 * 语义：`deps` **全部完成**才渲染 `children`。
 */
export function StageGate({
  deps,
  children,
  hint,
}: {
  /** 依赖的阶段：全部完成才渲染 `children` */
  deps: readonly BuildStage[];
  /** 就绪后的内容 */
  children: ReactNode;
  /** 自定义提示（默认按 `deps` 自动生成） */
  hint?: string;
}) {
  const { t } = useI18n();
  const done = useWikiStagesDone(deps);
  const preparing = useWikiPreparing();
  if (done) return <>{children}</>;
  return (
    <div className="flex min-h-[200px] w-full flex-col items-center justify-center gap-3 rounded-xl border border-slate-800/60 bg-slate-900/40 p-6">
      {/* 预检期无真实阶段进度（`stages` 全 0/0）→ 不渲染进度行，只留提示文案 */}
      {!preparing && <StageRows stages={deps} />}
      <p className="text-center text-xs text-slate-400">{hint ?? stageReadyHint(t, deps)}</p>
    </div>
  );
}
