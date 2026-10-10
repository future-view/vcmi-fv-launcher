import { Globe } from "lucide-react";
import { UI_LANG_OPTIONS, useI18n } from "@/i18n";
import { Select, SelectContent, SelectItem, SelectTrigger } from "@/shared/ui/select";

/**
 * 「多语言」图标按钮（§9.2.0）：与窗口控件同排的语言下拉入口。
 *
 * 抽成独立组件而不是塞进 `WindowControls`：语言是**业务状态**（写 `launcher.json` 的 `ui.lang`，
 * 并决定 VCMI 的 `general.language` 与语言模组注入），而 `WindowControls` 只该负责窗口动作。
 * 由 `TopBar` / `RealmShell` 各自通过 `extra` 插槽决定是否挂载。
 *
 * 历史原因：这个下拉原先只存在于底部区（`BottomBar`），而领域界面不渲染底部区，
 * 导致领域内没有任何切语言入口——故上提到右上角控件区，两处共用同一个按钮。
 */
export function LanguageMenuButton() {
  const { t, vcmiLang, setVcmiLang } = useI18n();
  return (
    <Select value={vcmiLang} onValueChange={setVcmiLang}>
      <SelectTrigger
        title={t.app.language}
        aria-label={t.app.language}
        // 图标按钮形态：覆盖 SelectTrigger 的默认输入框样式与右侧 chevron
        // （[&>svg:last-child] 命中的是 ui/select 内建的 ChevronDown，不是本处 Globe）
        className="h-8 w-8 flex-none cursor-pointer justify-center border-0 bg-transparent px-0 text-slate-400 shadow-none hover:bg-slate-700/60 hover:text-white focus:ring-0 focus:ring-offset-0 [&>svg:last-child]:hidden"
      >
        <Globe className="h-4 w-4" />
      </SelectTrigger>
      {/* SelectContent 走 Portal 渲染到 body：RealmShell 控件层是 z-[140]、模态层 z-[145]，
          故此处必须用 z-[200] 才能盖在覆盖层之上（与 BottomBar 原有取值一致） */}
      <SelectContent className="z-[200] border-slate-700 bg-slate-900 text-slate-200">
        {UI_LANG_OPTIONS.map(({ value, label }) => (
          <SelectItem key={value} value={value}>
            {label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  );
}
