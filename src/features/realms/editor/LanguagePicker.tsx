import { useState } from "react";
import { Globe, ChevronDown, Plus, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n, UI_LANG_OPTIONS } from "@/i18n";

/** 多语言下拉（§9.6.1）：分区式交互，已启用语言 / 添加语言两个独立区域。
 *
 * **不再使用 checkbox**：此前 checkbox 的 `<label>` 包住整行，点击行内空白处就会
 * 命中 label 而 toggle 勾选，导致该语言被移除、已填写内容被清空（误触）。
 * 现在改为：整行点击 = 切换「当前编辑语言」；★ = 设为默认；× = 移除（带二次确认）。 */
export function LanguagePicker({
  selected,
  currentLang,
  defaultLang,
  onToggle,
  onSelect,
  onSetDefault,
}: {
  selected: string[];
  currentLang: string;
  defaultLang: string;
  onToggle: (v: string) => void;
  onSelect: (v: string) => void;
  onSetDefault: (v: string) => void;
}) {
  const { t } = useI18n();
  const [open, setOpen] = useState(false);
  const [adding, setAdding] = useState(false);
  /** 待确认移除的语言：非 null 时该行就地变成确认条，避免手滑丢内容 */
  const [pendingRemove, setPendingRemove] = useState<string | null>(null);
  const labelOf = (v: string) => UI_LANG_OPTIONS.find((l) => l.value === v)?.label ?? v;
  const unused = UI_LANG_OPTIONS.filter((l) => !selected.includes(l.value));

  const close = () => {
    setOpen(false);
    setAdding(false);
    setPendingRemove(null);
  };

  return (
    <div className="relative">
      <button
        type="button"
        onClick={() => (open ? close() : setOpen(true))}
        className="flex items-center gap-2 rounded-lg border border-slate-700 bg-slate-900 px-3 py-2 text-sm text-foreground hover:bg-slate-800/50"
      >
        <Globe className="h-4 w-4 text-slate-400" />
        <span className="font-medium">{t.realm.multiLang}</span>
        {selected.length === 0 ? (
          <span className="text-xs text-muted">（{t.realm.multiLangOff}）</span>
        ) : (
          <span className="text-xs text-muted">
            {t.realm.editingNow.replace("{lang}", labelOf(currentLang))}
          </span>
        )}
        <ChevronDown className="ml-auto h-4 w-4 text-slate-400" />
      </button>
      {open && (
        <>
          <div className="fixed inset-0 z-[60]" onClick={close} />
          <div className="absolute right-0 z-[61] mt-1 w-72 rounded-lg border border-slate-700 bg-slate-900 p-1.5 shadow-xl">
            <div className="px-2 py-1 text-xs text-muted">{t.realm.multiLangHint}</div>

            {/* 已启用语言：整行点击 = 切换编辑语言；★ 设默认；× 移除（就地二次确认） */}
            <div className="mt-1 space-y-0.5">
              {selected.map((v) => {
                const isDefault = defaultLang === v;
                const isCurrent = currentLang === v;
                if (pendingRemove === v) {
                  return (
                    <div key={v} className="rounded-md bg-slate-800/70 px-2 py-2">
                      <div className="text-xs leading-relaxed text-slate-300">
                        {t.realm.langRemoveConfirm.replace("{lang}", labelOf(v))}
                      </div>
                      <div className="mt-2 flex justify-end gap-2">
                        <button
                          type="button"
                          onClick={() => setPendingRemove(null)}
                          className="rounded border border-slate-600 px-2 py-0.5 text-xs text-slate-300 hover:bg-slate-700"
                        >
                          {t.common.cancel}
                        </button>
                        <button
                          type="button"
                          onClick={() => {
                            onToggle(v);
                            setPendingRemove(null);
                          }}
                          className="rounded bg-red-500/90 px-2 py-0.5 text-xs text-white hover:bg-red-500"
                        >
                          {t.realm.langRemove}
                        </button>
                      </div>
                    </div>
                  );
                }
                return (
                  <div
                    key={v}
                    className={cn("flex items-center rounded-md", isCurrent && "bg-blue-500/10")}
                  >
                    <button
                      type="button"
                      onClick={() => onSelect(v)}
                      title={t.realm.editingLang.replace("{lang}", labelOf(v))}
                      className={cn(
                        "flex min-w-0 flex-1 items-center gap-2 rounded-md px-2 py-1.5 text-left text-sm text-foreground hover:bg-slate-800/60",
                        isCurrent && "font-medium text-blue-300",
                      )}
                    >
                      <span className="truncate">{labelOf(v)}</span>
                      {isCurrent && (
                        <span className="shrink-0 rounded bg-blue-500/20 px-1.5 py-0.5 text-[10px] text-blue-300">
                          {t.realm.editingNowShort}
                        </span>
                      )}
                    </button>
                    <button
                      type="button"
                      onClick={() => onSetDefault(v)}
                      title={t.realm.setDefault}
                      className={cn(
                        "shrink-0 rounded px-1.5 py-1 text-xs",
                        isDefault ? "text-blue-300" : "text-muted hover:bg-slate-800 hover:text-white",
                      )}
                    >
                      {isDefault ? "★" : "☆"}
                    </button>
                    {/* 默认语言不可移除（`toggleLang` 内已拦截），故不显示移除按钮 */}
                    {!isDefault && (
                      <button
                        type="button"
                        onClick={() => setPendingRemove(v)}
                        title={t.realm.langRemove}
                        className="mr-1 shrink-0 rounded p-1 text-muted hover:bg-red-500/20 hover:text-red-300"
                      >
                        <X className="h-3.5 w-3.5" />
                      </button>
                    )}
                  </div>
                );
              })}
              {selected.length === 0 && (
                <div className="px-2 py-1.5 text-xs text-muted">{t.realm.multiLangOff}</div>
              )}
            </div>

            {/* 添加语言：独立底部区域，只列未启用的语言，点击即添加并进入编辑 */}
            <div className="mt-1 border-t border-slate-800 pt-1">
              {unused.length === 0 ? (
                <div className="px-2 py-1.5 text-xs text-muted">{t.realm.langAllAdded}</div>
              ) : (
                <>
                  <button
                    type="button"
                    onClick={() => setAdding((a) => !a)}
                    className="flex w-full items-center gap-1.5 rounded-md px-2 py-1.5 text-sm text-slate-300 hover:bg-slate-800/60"
                  >
                    <Plus className="h-3.5 w-3.5" />
                    {t.realm.langAdd}
                    <ChevronDown
                      className={cn("ml-auto h-3.5 w-3.5 transition-transform", adding && "rotate-180")}
                    />
                  </button>
                  {adding && (
                    <div className="space-y-0.5 pb-1">
                      {unused.map((l) => (
                        <button
                          key={l.value}
                          type="button"
                          onClick={() => {
                            onToggle(l.value);
                            onSelect(l.value);
                            setAdding(false);
                          }}
                          className="block w-full truncate rounded-md py-1.5 pl-7 pr-2 text-left text-sm text-slate-300 hover:bg-slate-800/60"
                        >
                          {l.label}
                        </button>
                      ))}
                    </div>
                  )}
                </>
              )}
            </div>
          </div>
        </>
      )}
    </div>
  );
}
