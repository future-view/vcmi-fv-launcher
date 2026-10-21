import type { ReactNode } from "react";
import { Loader2 } from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";

/**
 * 通用加载蒙层（纯展示壳，无语义）。
 *
 * ## 用途
 *
 * 「页面 / 区块**未就绪时只渲染蒙层**（早返回）」的既定范式，见 `WikiView` 与
 * `docs/features/startup-responsiveness.zh-CN.md`：
 * 被蒙层遮住的**内容照样渲染、照样跑 effect、照样发 IPC**（实测浪费），
 * 所以未就绪就**不要渲染内容** —— 只渲染本蒙层。
 *
 * ## 形态
 *
 * - `title` 必填；`hint` / `children`（进度区）可选；
 * - **无进度可报时不要传 `children`**：只显示 spinner 即可（"正在加载中"的感知足够，
 *   不要用假进度条 —— 进度条应当表达真实进度）；
 * - `onExit` 可选：给了就渲染出口按钮，用户任何时候都能离开（不依赖 z-index 博弈）。
 *
 * ⚠️ 不要用"非阻塞蒙层"盖在**可用内容**上：与背后文字互相干扰、观感差（已被实测否决，
 * 见 `WikiBuildOverlay` 注释）。
 *
 * ## 为什么**不用 backdrop-blur**（2026-10-21 真机反馈："进度条一动整个窗口就闪"）
 *
 * 本组件的三个使用点（`WikiBuildOverlay` / `WikiView` boot / `ModView` boot）
 * **全部是"早返回只渲染蒙层"** —— 蒙层背后永远是空背景，模糊没有任何视觉收益。
 * 而 WebView2 下 `backdrop-filter` 元素的内容每次重绘都会触发整个 render surface
 * 重新采样：构建进度更新（即使已量化到 ~1.4 次/秒）时，占窗口 80%+ 的蒙层
 * 就整体闪一次。故改用**更高不透明度的实色底**（`/80` → `/90`）替代：
 * 遮挡效果等价，重绘只剩蒙层自身的小区域（文字 + 进度条）。
 */
export function LoadingOverlay({
  title,
  hint,
  children,
  onExit,
  exitLabel,
  blocking = true,
}: {
  /** 主标题（如「正在分析模组列表…」） */
  title: string;
  /** 辅助提示（可选） */
  hint?: string;
  /** 进度区（可选）。不传 = 只有 spinner（无进度可报时的正确形态） */
  children?: ReactNode;
  /** 显式出口（可选） */
  onExit?: () => void;
  /** 出口按钮文案（默认「返回」） */
  exitLabel?: string;
  /** `true`（默认）挡操作；`false` 不挡（`pointer-events-none`，层级压在抽屉 / 弹层之下） */
  blocking?: boolean;
}) {
  const { t } = useI18n();
  return (
    <div
      className={cn(
        "absolute inset-0 z-40 flex flex-col items-center justify-center gap-4 rounded-xl",
        blocking
          ? /* 阻塞：实色高不透明底（**不用 backdrop-blur**，见组件注释） */ "bg-slate-950/90"
          : /* 非阻塞：半透明且**不挡操作**，层级压在抽屉 / 弹层之下（z-20） */
            "pointer-events-none z-20 bg-slate-950/40",
      )}
    >
      <Loader2 className="h-7 w-7 animate-spin text-slate-300" aria-hidden />
      <p className="text-base font-semibold text-slate-100">{title}</p>
      {children}
      {hint && <p className="text-center text-xs text-slate-400">{hint}</p>}
      {onExit && (
        <button
          type="button"
          onClick={onExit}
          className="rounded-lg border border-slate-700 bg-slate-900/80 px-4 py-1.5 text-sm text-slate-300 transition-colors hover:border-slate-500 hover:text-slate-100"
        >
          {exitLabel ?? t.common.back}
        </button>
      )}
    </div>
  );
}
