import { useEffect, useMemo, useRef, useState } from "react";
import { X } from "lucide-react";
import { useI18n } from "@/i18n";
import { Input } from "@/shared/ui/input";
import {
  builtinPresetLabel,
  isBuiltinPreset,
  getActiveBuiltinPresets,
} from "@/lib/builtinPresets";
import { cn } from "@/lib/utils";
import { backendErrorText } from "@/lib/backendError";

export interface PresetData {
  mods: string[];
  settings: Record<string, Record<string, boolean>>;
}
export type PresetMap = Record<string, PresetData>;

/** 新建预设「起始内容」的哨兵值：不复制任何现有预设（仅基础模组）。
    它是 PresetBar 与 ModView onCreate 之间的协议字符串，不可随意改动。 */
const INHERIT_BLANK = "__pure__";

/** 预设导出载荷：PresetBarProps.onExport 返回与导出状态共用的结构。 */
type PresetExport = { name: string; mods: string[]; settings: any };

interface PresetBarProps {
  presetName: string;
  presets: PresetMap;
  dirty?: boolean;
  /** 初始化加载中（真实 activePreset 尚未读出）：显示占位并禁用全部操作，避免误切/误存。 */
  loading?: boolean;
  lang?: string;
  onSave: () => void;
  onSwitch: (name: string) => void;
  onCreate: (name: string, inheritFrom: string) => void;
  onRename: (old: string, next: string) => void;
  onDelete: (name: string) => void;
  onImport: (json: string) => void;
  onExport: () => PresetExport | null;
  /** 打开报告（页内覆盖层由 ModView 渲染；内容生成也在 ModView） */
  onReportOpen: () => void;
  /** 复制/粘贴等动作的轻提示出口（父级用横幅呈现）；不新增布局元素，仅反馈剪贴板结果 */
  onNotify?: (msg: string, kind?: "success" | "error") => void;
  /** 模组管理页入口（P5/T3）：库维护全局动作（检查更新/强制重新校验等）已迁至管理页；徽章仅自动检查开启且存在可更新项时显示 */
  onManage?: () => void;
  manageBadge?: number;
}

/** 预设选项列表：内置预设置顶 + 内置标签 + 搜索。
    下拉浮层（PresetSelect）与新建弹窗的内联列表共用同一份渲染，避免两处漂移。 */
export function PresetOptionList({
  presets,
  current,
  onSelect,
  errorName,
  listClassName = "max-h-60",
  autoFocusSearch = false,
}: {
  presets: PresetMap;
  current: string;
  onSelect: (name: string) => void;
  errorName?: string | null;
  /** 选项列表容器的高度类（内联场景可传更矮的 max-h，避免在最小窗口下撑破卡片） */
  listClassName?: string;
  /** 是否自动聚焦搜索框（浮层模式聚焦；内联模式不抢外层输入框的焦点） */
  autoFocusSearch?: boolean;
}) {
  const { t, lang } = useI18n();
  const [q, setQ] = useState("");
  const selectedRef = useRef<HTMLButtonElement>(null);
  const builtin = getActiveBuiltinPresets();
  const customNames = useMemo(
    () => Object.keys(presets).filter((n) => !isBuiltinPreset(n)),
    [presets],
  );

  const filter = (n: string) => !q || n.toLowerCase().includes(q.toLowerCase());

  // 预设多时默认选中项可能落在滚动区外（内联列表尤其常见）：
  // 挂载时把它带入视野，用户才知道默认要复制的是哪个（block: "nearest" 只动最近的滚动容器）。
  useEffect(() => {
    selectedRef.current?.scrollIntoView({ block: "nearest" });
  }, []);

  return (
    <>
      <div className="border-b border-slate-800 p-2">
        <input
          autoFocus={autoFocusSearch}
          value={q}
          onChange={(e) => setQ(e.target.value)}
          placeholder={t.preset.search}
          className="w-full rounded-md border border-slate-700 bg-slate-800 px-2 py-1 text-xs text-slate-100 outline-none focus:border-indigo-500/60"
        />
      </div>
      <div className={cn("overflow-y-auto py-1", listClassName)}>
        {builtin.map((p) => {
          const name = p.id;
          if (!filter(name)) return null;
          return (
            <button
              key={name}
              type="button"
              ref={current === name ? selectedRef : undefined}
              onClick={() => onSelect(name)}
              className={cn(
                "flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm hover:bg-slate-800",
                current === name && "bg-slate-800/70",
              )}
            >
              <span className="flex-none rounded border border-blue-400/50 px-1 text-[10px] text-blue-300">
                {t.builtinPreset.builtinTag}
              </span>
              <span className="truncate">{builtinPresetLabel(name, lang)}</span>
              {current === name && <span className="ml-auto text-[11px] text-blue-300">●</span>}
              {errorName === name && <span className="ml-auto text-[11px] text-amber-400">⚠</span>}
            </button>
          );
        })}
        {customNames.length > 0 && <div className="mx-3 my-1 border-t border-slate-800" />}
        {customNames.map((name) => {
          if (!filter(name)) return null;
          return (
            <button
              key={name}
              type="button"
              ref={current === name ? selectedRef : undefined}
              onClick={() => onSelect(name)}
              className={cn(
                "flex w-full items-center gap-2 px-3 py-1.5 text-left text-sm hover:bg-slate-800",
                current === name && "bg-slate-800/70",
              )}
            >
              <span className="truncate">{name}</span>
              {current === name && <span className="ml-auto text-[11px] text-blue-300">●</span>}
              {errorName === name && <span className="ml-auto text-[11px] text-amber-400">⚠</span>}
            </button>
          );
        })}
      </div>
    </>
  );
}

/** 共享的预设下拉选择器：内置预设置顶 + 内置标签 + 搜索，复用于顶部栏 */
export function PresetSelect({
  presets,
  current,
  onSelect,
  placeholder,
  errorName,
  disabled = false,
}: {
  presets: PresetMap;
  current: string;
  onSelect: (name: string) => void;
  placeholder?: string;
  errorName?: string | null;
  /** 数据未就绪时禁用展开（避免切到还不存在的预设） */
  disabled?: boolean;
}) {
  const { t, lang } = useI18n();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open]);

  return (
    <div className="relative" ref={ref}>
      <button
        type="button"
        disabled={disabled}
        onClick={() => setOpen((o) => !o)}
        className={cn(
          "flex w-full items-center justify-between gap-2 rounded-lg border border-slate-700/60 bg-slate-800/60 px-3 py-1.5 text-sm text-slate-100 transition hover:border-slate-600 disabled:cursor-not-allowed disabled:opacity-40",
          open && "border-indigo-500/60",
        )}
      >
        <span className="flex min-w-0 items-center gap-2">
          {current && isBuiltinPreset(current) && (
            <span className="flex-none rounded border border-blue-400/50 px-1 text-[10px] text-blue-300">
              {t.builtinPreset.builtinTag}
            </span>
          )}
          <span className="truncate">
            {current ? displayName(current, lang) : placeholder ?? t.preset.search}
          </span>
        </span>
        <span className="text-slate-400">▾</span>
      </button>
      {open && (
        <div className="absolute z-[60] mt-1 w-full overflow-hidden rounded-lg border border-slate-700 bg-slate-900 shadow-xl">
          <PresetOptionList
            presets={presets}
            current={current}
            onSelect={(name) => {
              onSelect(name);
              setOpen(false);
            }}
            errorName={errorName}
            autoFocusSearch
          />
        </div>
      )}
    </div>
  );
}

function displayName(n: string, lang: string): string {
  if (isBuiltinPreset(n)) return builtinPresetLabel(n, lang);
  return n;
}

/** 预设导出序列化：导出弹窗文本域与「复制到剪贴板」共用（缩进 2 的格式化 JSON），
    避免两处格式漂移；导入侧走 JSON.parse 解析，对缩进不敏感，两种格式均可导入。 */
function serializePreset(data: PresetExport): string {
  return JSON.stringify(data, null, 2);
}

export function PresetBar({
  presetName,
  presets,
  dirty,
  loading = false,
  onSave,
  onSwitch,
  onCreate,
  onRename,
  onDelete,
  onImport,
  onExport,
  onReportOpen,
  onNotify,
  onManage,
  manageBadge = 0,
}: PresetBarProps) {
  const { t, lang } = useI18n();
  const [menuOpen, setMenuOpen] = useState(false);
  const [renaming, setRenaming] = useState(false);
  const [newName, setNewName] = useState("");
  const menuRef = useRef<HTMLDivElement>(null);

  const [showNew, setShowNew] = useState(false);
  const [newPresetName, setNewPresetName] = useState("");
  /** 新建预设的起始内容：INHERIT_BLANK=空白（仅基础模组）；否则为要复制的预设名 */
  const [inheritPreset, setInheritPreset] = useState(INHERIT_BLANK);
  const [showImport, setShowImport] = useState(false);
  const [importJson, setImportJson] = useState("");
  const [showExport, setShowExport] = useState(false);
  const [exportData, setExportData] = useState<PresetExport | null>(null);

  const builtin = isBuiltinPreset(presetName);

  useEffect(() => {
    const handler = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) setMenuOpen(false);
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, []);

  const openRename = () => {
    setNewName(presetName);
    setRenaming(true);
    setMenuOpen(false);
  };
  const confirmRename = () => {
    const n = newName.trim();
    if (n && n !== presetName && !presets[n]) onRename(presetName, n);
    setRenaming(false);
  };

  /** 起始内容是否为「空白」（false=复制现有预设）。 */
  const inheritIsBlank = inheritPreset === INHERIT_BLANK;
  const trimmedNewName = newPresetName.trim();
  /** 就地校验：空名 / 与现有预设重名时禁用「新建」按钮（不再用 alert 打断）。 */
  const newNameTaken = trimmedNewName !== "" && presets[trimmedNewName] !== undefined;
  const newDisabled = trimmedNewName === "" || newNameTaken;

  /** 打开弹窗：重置为默认「空白」起手，避免沿用上次输入。 */
  const openNew = () => {
    setNewPresetName("");
    setInheritPreset(INHERIT_BLANK);
    setShowNew(true);
  };

  const closeNew = () => setShowNew(false);

  const confirmNew = () => {
    if (newDisabled) return;
    onCreate(trimmedNewName, inheritPreset);
    setShowNew(false);
    setNewPresetName("");
    setInheritPreset(INHERIT_BLANK);
  };

  /** 切到「复制现有预设」：默认选中当前预设（多数场景是先复制当前组合再调整）。 */
  const startCopyFromPreset = () => {
    setInheritPreset((prev) =>
      prev === INHERIT_BLANK ? presetName || getActiveBuiltinPresets()[0]?.id || "" : prev,
    );
  };

  const doExport = () => {
    const data = onExport();
    if (data) {
      setExportData(data);
      setShowExport(true);
    }
  };

  const doImport = () => {
    onImport(importJson);
    setShowImport(false);
    setImportJson("");
  };

  /** 一键复制当前预设到剪贴板（与导出弹窗文本域共用 serializePreset，内容所见即所得）：
      不打开任何弹层，不动现有布局。 */
  const copyToClipboard = async () => {
    const data = onExport();
    if (!data) {
      onNotify?.(t.preset.importErr, "error");
      return;
    }
    try {
      await navigator.clipboard.writeText(serializePreset(data));
      onNotify?.(t.preset.copiedOk, "success");
    } catch (e) {
      onNotify?.(t.preset.clipboardFail.replace("{msg}", backendErrorText(e)), "error");
    }
  };

  /** 从剪贴板读回预设 JSON 填入导入文本框（浏览器拒绝授权时降级为手动粘贴提示）。 */
  const pasteFromClipboard = async () => {
    try {
      const text = await navigator.clipboard.readText();
      if (text.trim()) setImportJson(text);
      else onNotify?.(t.preset.clipboardEmpty, "error");
    } catch (e) {
      onNotify?.(t.preset.clipboardFail.replace("{msg}", backendErrorText(e)), "error");
    }
  };

  return (
    <div className="relative flex h-[58px] flex-none items-center justify-between gap-4 border-b border-slate-800/70 px-5">
      {/* 左侧：当前预设名称（恢复原始行内布局）。
          初始化未完成（真实 activePreset 尚未读出）时只显示「加载中」，
          避免先把内置预设（原版）渲染成当前预设、扫完再跳变。 */}
      <div className="flex min-w-0 items-center gap-2">
        {loading ? (
          <span className="text-sm text-slate-500">{t.common.loading}</span>
        ) : (
          <>
            {builtin ? (
              <span className="flex-none rounded border border-blue-400/50 px-1 text-[10px] text-blue-300">
                {t.builtinPreset.builtinTag}
              </span>
            ) : null}
            <span className="truncate text-base font-semibold text-slate-100" title={presetName}>
              {displayName(presetName, lang)}
            </span>
          </>
        )}
      </div>

      {/* 右侧：下拉、保存、新建、更多 */}
      <div className="flex flex-none items-center gap-2">
        <div className="w-44">
          <PresetSelect
            presets={presets}
            current={presetName}
            onSelect={onSwitch}
            disabled={loading}
          />
        </div>
        <button
          onClick={onSave}
          disabled={loading}
          className={cn(
            "rounded-lg border border-emerald-500/50 bg-emerald-500/15 px-3 py-1.5 text-sm font-medium text-emerald-300 hover:bg-emerald-500/25 disabled:cursor-not-allowed disabled:opacity-40",
            dirty && !loading && "animate-pulse",
          )}
        >
          {t.preset.save}
        </button>
        {dirty && <span className="rounded bg-amber-500/20 px-2 py-0.5 text-[11px] text-amber-300">●</span>}
        <button
          onClick={openNew}
          disabled={loading}
          className="rounded-lg bg-gradient-to-br from-blue-500 to-indigo-500 px-3 py-1.5 text-sm font-semibold text-white shadow-lg shadow-blue-600/30 disabled:cursor-not-allowed disabled:opacity-40"
        >
          {t.preset.new}
        </button>
        <div className="relative" ref={menuRef}>
          <button
            onClick={() => setMenuOpen((o) => !o)}
            disabled={loading}
            className="rounded-lg border border-slate-700 px-2.5 py-1.5 text-sm text-slate-300 hover:border-slate-600 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {t.preset.more}
          </button>
          {menuOpen && (
            <div className="absolute right-0 z-[60] mt-1 w-36 overflow-hidden rounded-lg border border-slate-700 bg-slate-900 shadow-xl">
              <button
                className="w-full px-3 py-2 text-left text-sm text-slate-200 hover:bg-slate-800 disabled:opacity-40"
                disabled={builtin}
                title={builtin ? t.preset.builtinNoRename : undefined}
                onClick={openRename}
              >
                {t.preset.rename}
              </button>
              <button
                className="w-full px-3 py-2 text-left text-sm text-slate-200 hover:bg-slate-800 disabled:opacity-40"
                disabled={builtin}
                title={builtin ? t.preset.builtinNoDelete : undefined}
                onClick={() => {
                  onDelete(presetName);
                  setMenuOpen(false);
                }}
              >
                {t.preset.delete}
              </button>
              <div className="my-1 border-t border-slate-800" />
              <button
                className="w-full px-3 py-2 text-left text-sm text-slate-200 hover:bg-slate-800"
                onClick={() => {
                  setShowImport(true);
                  setMenuOpen(false);
                }}
              >
                {t.preset.import}
              </button>
              <button
                className="w-full px-3 py-2 text-left text-sm text-slate-200 hover:bg-slate-800"
                onClick={() => {
                  doExport();
                  setMenuOpen(false);
                }}
              >
                {t.preset.export}
              </button>
              <button
                className="w-full px-3 py-2 text-left text-sm text-slate-200 hover:bg-slate-800"
                onClick={() => {
                  onReportOpen();
                  setMenuOpen(false);
                }}
              >
                {t.preset.report}
              </button>
            </div>
          )}
        </div>
        {/* 模组管理页入口（P5）：库维护语境与预设编排解耦；徽章只在自动检查开启时有意义（T3） */}
        {onManage && (
          <button
            onClick={onManage}
            disabled={loading}
            className="flex items-center gap-1.5 rounded-lg border border-slate-700 px-2.5 py-1.5 text-sm text-slate-300 hover:border-slate-600 disabled:cursor-not-allowed disabled:opacity-40"
          >
            {t.modsManage.title}
            {manageBadge > 0 && (
              <span
                title={t.modsManage.badgeHint.replace("{n}", String(manageBadge))}
                className="rounded-full bg-amber-500/90 px-1.5 py-0.5 text-xs font-semibold leading-none text-slate-950"
              >
                {manageBadge}
              </span>
            )}
          </button>
        )}
      </div>

      {/* 重命名（不撑开条高，绝对定位在条下方） */}
      {renaming && (
        <div className="absolute left-0 right-0 top-full z-50 mt-1 rounded-lg border border-slate-700 bg-slate-900 p-3 shadow-xl">
          <div className="flex items-center gap-2">
            <input
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && confirmRename()}
              className="flex-1 rounded-md border border-slate-700 bg-slate-800 px-2 py-1 text-sm text-slate-100 outline-none focus:border-indigo-500/60"
            />
            <button onClick={confirmRename} className="rounded-md bg-indigo-500 px-3 py-1 text-sm text-white">
              {t.common.ok}
            </button>
            <button onClick={() => setRenaming(false)} className="rounded-md border border-slate-700 px-3 py-1 text-sm text-slate-300">
              {t.common.cancel}
            </button>
          </div>
        </div>
      )}

      {/* 新建预设弹窗：起始内容二选一（空白 / 复制现有预设）+ 名称。
          外壳与 ConfirmDialog 同范式（居中卡片 + 遮罩）；遮罩不响应点击，防误触丢失输入。 */}
      {showNew && (
        <div className="fixed inset-0 z-[100] flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm">
          <div className="w-full max-w-md rounded-xl border border-slate-700/60 bg-slate-900 p-5 shadow-2xl">
            <div className="mb-3 flex items-start justify-between">
              <h3 className="text-base font-semibold text-slate-100">{t.preset.new}</h3>
              <button
                onClick={closeNew}
                className="rounded p-1 text-slate-500 hover:bg-slate-800 hover:text-slate-200"
              >
                <X className="h-4 w-4" />
              </button>
            </div>

            <div className="mb-4 space-y-3">
              <div>
                <label className="mb-1 block text-xs text-slate-500">{t.preset.newName}</label>
                <Input
                  value={newPresetName}
                  onChange={(e) => setNewPresetName(e.target.value)}
                  placeholder={t.preset.newName}
                  autoFocus
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && !newDisabled) confirmNew();
                  }}
                />
                {newNameTaken && <p className="mt-1 text-xs text-red-400">{t.preset.nameExists}</p>}
              </div>

              <div>
                <div className="mb-1 text-xs text-slate-500">{t.preset.startFrom}</div>
                <div className="space-y-2 rounded-lg border border-slate-700/50 bg-slate-800/50 p-3">
                  <label className="flex cursor-pointer items-center gap-2 text-sm text-slate-200">
                    <input
                      type="radio"
                      name="preset-start-from"
                      checked={inheritIsBlank}
                      onChange={() => setInheritPreset(INHERIT_BLANK)}
                      className="h-3.5 w-3.5 accent-blue-500"
                    />
                    {t.preset.blank}
                  </label>
                  <label className="flex cursor-pointer items-center gap-2 text-sm text-slate-200">
                    <input
                      type="radio"
                      name="preset-start-from"
                      checked={!inheritIsBlank}
                      onChange={startCopyFromPreset}
                      className="h-3.5 w-3.5 accent-blue-500"
                    />
                    {t.preset.copyExisting}
                  </label>
                  {!inheritIsBlank && (
                    /* 内联列表而非浮层：避免下拉撑出卡片、压在底部按钮上（最小窗口 960×640 的硬基准） */
                    <div className="ml-5 overflow-hidden rounded-lg border border-slate-700/60 bg-slate-900">
                      <PresetOptionList
                        presets={presets}
                        current={inheritPreset}
                        onSelect={setInheritPreset}
                        listClassName="max-h-40"
                      />
                    </div>
                  )}
                </div>
                <p className="mt-1 text-[11px] leading-relaxed text-slate-500">
                  {inheritIsBlank
                    ? t.preset.blankNote
                    : t.preset.copyNote
                        .replace("{name}", displayName(inheritPreset, lang))
                        .replace("{n}", String(presets[inheritPreset]?.mods.length ?? 0))}
                </p>
              </div>
            </div>

            <div className="flex justify-end gap-2">
              <button
                onClick={closeNew}
                className="rounded-lg border border-slate-700/60 bg-slate-800/60 px-4 py-2 text-sm font-medium text-slate-300 hover:bg-slate-800"
              >
                {t.common.cancel}
              </button>
              <button
                onClick={confirmNew}
                disabled={newDisabled}
                className={cn(
                  "rounded-lg px-4 py-2 text-sm font-medium",
                  newDisabled
                    ? "cursor-not-allowed bg-slate-700 text-slate-500"
                    : "bg-indigo-500/90 text-white hover:bg-indigo-500",
                )}
              >
                {t.preset.new}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* 导入弹窗 */}
      {showImport && (
        <div className="absolute left-0 right-0 top-full z-50 mt-1 rounded-lg border border-slate-700 bg-slate-900 p-3 shadow-xl">
          <div className="mb-2 text-xs text-slate-500">{t.preset.importHint}</div>
          <textarea
            value={importJson}
            onChange={(e) => setImportJson(e.target.value)}
            rows={20}
            className="w-full rounded-md border border-slate-700 bg-slate-800 px-2 py-1 text-xs text-slate-100 outline-none focus:border-indigo-500/60"
          />
          <div className="mt-2 flex items-center justify-between gap-2">
            <button
              onClick={() => void pasteFromClipboard()}
              className="rounded-md border border-slate-700 px-3 py-1 text-sm text-slate-300 hover:border-slate-600"
            >
              {t.preset.pasteFromClipboard}
            </button>
            <div className="flex gap-2">
              <button onClick={() => setShowImport(false)} className="rounded-md border border-slate-700 px-3 py-1 text-sm text-slate-300">
                {t.common.cancel}
              </button>
              <button onClick={doImport} className="rounded-md bg-indigo-500 px-3 py-1 text-sm text-white">
                {t.preset.import}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* 导出弹窗 */}
      {showExport && exportData && (
        <div className="absolute left-0 right-0 top-full z-50 mt-1 rounded-lg border border-slate-700 bg-slate-900 p-3 shadow-xl">
          <div className="mb-2 text-xs text-slate-500">{t.preset.exportHint}</div>
          <textarea
            readOnly
            value={serializePreset(exportData)}
            rows={20}
            className="w-full rounded-md border border-slate-700 bg-slate-800 px-2 py-1 text-xs text-slate-100"
          />
          <div className="mt-2 flex justify-end gap-2">
            <button
              onClick={() => void copyToClipboard()}
              className="rounded-md border border-slate-700 px-3 py-1 text-sm text-slate-300 hover:border-slate-600"
            >
              {t.preset.copyClipboard}
            </button>
            <button onClick={() => setShowExport(false)} className="rounded-md border border-slate-700 px-3 py-1 text-sm text-slate-300">
              {t.common.close}
            </button>
          </div>
        </div>
      )}

    </div>
  );
}
