import { ArrowLeft } from "lucide-react";
import { useI18n } from "@/i18n";

/**
 * 二级页面页头（§9.2.4 / §9.6.1）：返回 + 标题。
 *
 * 绝对定位在左上角，与右上角窗口控件（`WindowControls`）处于同一行，
 * 省下原先「控件一行 + 页头一行」中多余的一行内容空间。
 * `max-w` 必须≥右上角五个按钮（设置/多语言/最小化/最大化/关闭 ≈ 12rem）的宽度，
 * 否则长标题会压到控件上。
 *
 * 作用域徽标（`ScopeBadge`）**不在这里**：它已下沉进 `SettingsView`，与标题同属设置页自身。
 */
export function PageHeaderBar({
  title,
  onBack,
  backLabel,
}: {
  title: string;
  onBack?: () => void;
  /** 返回按钮文案（默认「返回」）；从启动器进入「切换领域」时用「返回启动器」更明确 */
  backLabel?: string;
}) {
  const { t } = useI18n();
  return (
    <div
      style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
      className="absolute left-4 top-3 z-[140] flex h-9 max-w-[calc(100%_-_14rem)] min-w-0 items-center gap-1.5 rounded-xl bg-black/30 px-1.5 backdrop-blur-sm"
    >
      {onBack && (
        <button
          type="button"
          onClick={onBack}
          className="flex flex-none cursor-pointer items-center gap-1.5 rounded-lg px-2 py-1 text-sm text-slate-300 transition-colors hover:bg-slate-700/60 hover:text-white"
        >
          <ArrowLeft className="h-4 w-4" />
          {backLabel ?? t.common.back}
        </button>
      )}
      <h2 className="min-w-0 truncate text-sm font-semibold text-foreground">{title}</h2>
    </div>
  );
}
