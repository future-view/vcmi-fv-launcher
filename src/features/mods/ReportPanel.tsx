import { Maximize2, Minimize2 } from "lucide-react";
import { useI18n } from "@/i18n";
import { Markdown } from "@/shared/Markdown";
import { cn } from "@/lib/utils";
import type { ReportMode } from "@/lib/presetReport";

interface ReportPanelProps {
  markdown: string;
  mode: ReportMode;
  onModeChange: (m: ReportMode) => void;
  /** 阅读宽度：false=居中阅读（max-w-3xl），true=全宽 */
  fullWidth: boolean;
  onToggleWidth: () => void;
  onCopy: () => void;
  onClose: () => void;
}

/**
 * 模组预设报告（页内覆盖层）。
 *
 * 覆盖范围 = ModView 自身区域（顶部导航栏与底部工具栏之间，`absolute inset-0`）：
 * 顶/底栏保持可见可按（切页签会卸载 ModView，报告随之消失）；预设栏与模组列表
 * 被完全盖住——面板不透明，因此**不需要遮罩层**（无暗化、无点击拦截）。
 *
 * 阅读层次：面板底 = slate-950（最暗）；居中阅读栏 = slate-900（略亮，"纸"），
 * 两侧露出面板底（"背景 + 纸"的电子书感）；切全宽时阅读栏铺满、两侧消失。
 */
export function ReportPanel({
  markdown,
  mode,
  onModeChange,
  fullWidth,
  onToggleWidth,
  onCopy,
  onClose,
}: ReportPanelProps) {
  const { t } = useI18n();

  return (
    <div className="absolute inset-0 z-[130] flex flex-col bg-slate-950">
      <header className="flex flex-none items-center gap-3 border-b border-slate-800 px-5 py-2.5">
        <div className="text-sm font-semibold text-slate-100">{t.preset.reportTitle}</div>
        <div className="flex gap-0.5 rounded-lg border border-slate-700 bg-slate-800/60 p-0.5">
          {(["brief", "full"] as const).map((m) => (
            <button
              key={m}
              type="button"
              onClick={() => onModeChange(m)}
              className={cn(
                "rounded-md px-2.5 py-1 text-xs font-medium transition-colors",
                mode === m ? "bg-blue-600/80 text-white" : "text-slate-400 hover:text-slate-200",
              )}
            >
              {m === "brief" ? t.preset.reportModeBrief : t.preset.reportModeFull}
            </button>
          ))}
        </div>
        <div className="ml-auto flex items-center gap-2">
          <button
            type="button"
            onClick={onToggleWidth}
            title={fullWidth ? t.preset.reportWidthFull : t.preset.reportWidthCentered}
            className="rounded-lg border border-slate-700 p-1.5 text-slate-300 hover:border-slate-600 hover:text-slate-100"
          >
            {fullWidth ? <Minimize2 className="h-4 w-4" /> : <Maximize2 className="h-4 w-4" />}
          </button>
          <button
            type="button"
            onClick={onCopy}
            className="rounded-lg border border-slate-700/60 bg-slate-800/60 px-3 py-1.5 text-xs font-medium text-slate-300 hover:bg-slate-800"
          >
            {t.preset.reportCopy}
          </button>
          <button
            type="button"
            onClick={onClose}
            className="rounded-lg border border-slate-700 px-3 py-1.5 text-xs font-medium text-slate-300 hover:border-slate-600"
          >
            {t.common.close}
          </button>
        </div>
      </header>

      <div className="min-h-0 flex-1 overflow-y-auto">
        {/* min-h-full：内容不足时阅读栏也铺满可视高度，避免下方露出面板底色形成"截断" */}
        <div className={cn("min-h-full bg-slate-900 px-8 py-6", !fullWidth && "mx-auto max-w-3xl")}>
          <Markdown source={markdown} className="leading-7" />
        </div>
      </div>
    </div>
  );
}
