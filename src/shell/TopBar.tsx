import { useI18n } from "@/i18n";
import { cn } from "@/lib/utils";
import { AppBrand } from "@/shared/AppBrand";
import { WindowControls } from "@/shared/WindowControls";
import { LanguageMenuButton } from "@/shared/LanguageMenuButton";

interface TopBarProps {
  view: "mods" | "wiki" | "settings";
  onViewChange: (v: "mods" | "wiki" | "settings") => void;
  /**
   * 点右上角 ⚙：**切换**设置视图（已在设置页则回到进入前的视图）。
   * 「设置」不是页签，故不开 toggle 就没有明确的"退出"手势（§9.0.1 v1.10）。
   */
  onToggleSettings: () => void;
  /** 「领域」页签：点击打开领域全屏覆盖层（§9.2.0），不是切换内容区视图 */
  onOpenRealm: () => void;
  /** 覆盖层是否打开（用于高亮「领域」页签） */
  realmOpen: boolean;
  /** 有新版本可用：⚙ 红点角标（共享 WindowControls 渲染，纯装饰不影响点击） */
  settingsBadge?: boolean;
  /** 红点 hover 说明 */
  settingsBadgeTitle?: string;
}

export function TopBar({
  view,
  onViewChange,
  onToggleSettings,
  onOpenRealm,
  realmOpen,
  settingsBadge,
  settingsBadgeTitle,
}: TopBarProps) {
  const { t } = useI18n();
  return (
    <header className="fixed top-0 left-0 right-0 z-50 flex h-[60px] items-center justify-between border-b border-slate-800/70 bg-slate-950/85 px-5 backdrop-blur-xl select-none">
      {/* 可拖拽区域（品牌 + 中部留白）；品牌块与首启标题栏共享（`AppBrand`） */}
      <div
        data-tauri-drag-region
        style={{ WebkitAppRegion: "drag" } as React.CSSProperties}
        className="flex h-full min-w-0 flex-1 cursor-move items-center gap-3"
      >
        <AppBrand />
      </div>

      <nav
        style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
        className="flex flex-none items-center gap-2"
      >
        {/* 页签顺序：领域 / 模组 / 百科（§9.0.1，领域第一位）。
            「设置」不是页签，见下方窗口控件；「领域」点击后打开全屏覆盖层，不是切换内容区视图（§9.2.0）。 */}
        <button
          onClick={onOpenRealm}
          className={cn(
            "rounded-lg px-4 py-2 text-sm font-medium transition-colors",
            realmOpen
              ? "bg-gradient-to-br from-blue-500 to-indigo-500 text-white shadow-md shadow-blue-600/40"
              : "text-slate-400 hover:bg-slate-800/70 hover:text-white",
          )}
        >
          {t.app.tabRealms}
        </button>
        <button
          onClick={() => onViewChange("mods")}
          className={cn(
            "rounded-lg px-4 py-2 text-sm font-medium transition-colors",
            view === "mods"
              ? "bg-gradient-to-br from-blue-500 to-indigo-500 text-white shadow-md shadow-blue-600/40"
              : "text-slate-400 hover:bg-slate-800/70 hover:text-white",
          )}
        >
          {t.app.tabMods}
        </button>
        <button
          onClick={() => onViewChange("wiki")}
          className={cn(
            "rounded-lg px-4 py-2 text-sm font-medium transition-colors",
            view === "wiki"
              ? "bg-gradient-to-br from-blue-500 to-indigo-500 text-white shadow-md shadow-blue-600/40"
              : "text-slate-400 hover:bg-slate-800/70 hover:text-white",
          )}
        >
          {t.app.tabWiki}
        </button>
        {/* 窗口控件：设置 / 多语言 / 最小化 / 最大化 / 关闭（§9.2.0 复用组件）。
            「设置」不是页签：⚙ 为 toggle（再点一次离开），激活态高亮表示当前在设置页。 */}
        <div className="ml-2">
          <WindowControls
            onSettings={onToggleSettings}
            settingsTitle={t.app.tabSettings}
            settingsActive={view === "settings"}
            settingsBadge={settingsBadge}
            settingsBadgeTitle={settingsBadgeTitle}
            extra={<LanguageMenuButton />}
          />
        </div>
      </nav>
    </header>
  );
}
