import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { ListFilter, Search } from "lucide-react";
import { cn } from "@/lib/utils";
import type { LangDict } from "@/i18n/types";
import type { WikiEntry } from "@/lib/modcache";
import type { WikiFilterContext, WikiFilterOption, WikiFilterSpec } from "./filters";

/** 浮层宽度：固定值，便于按触发按钮右边界左对齐定位 */
const PANEL_W = 272;
/** 选项数超过此值改用「搜索 + 单选列表」（chip 行在窄栏里会折成很多行） */
const CHIP_MAX = 6;

export interface WikiFilterPanelProps {
  /** 当前类目的筛选维度（空数组 = 该类目无维度，按钮禁用但仍占位） */
  specs: WikiFilterSpec[];
  /** 该类目**未经筛选**的全量条目（选项与计数都基于它） */
  entries: WikiEntry[];
  /** 当前生效筛选：维度 key → 选中值（缺省或空串 = 全部） */
  filters: Record<string, string>;
  onChange: (key: string, value: string) => void;
  onReset: () => void;
  contentLang: string;
  /** 阵营 id → 多语言名（来自 cache.json 顶层 factionNames，跨 mod 合并） */
  factionNames: Record<string, Record<string, string>>;
  /** modId（小写归一）→ 多语言名（来自 index.json） */
  modNames: Record<string, Record<string, string>>;
  t: LangDict;
}

/**
 * 中栏「筛选」入口：固宽图标按钮 + 浮层。
 *
 * 为什么是浮层而不是常驻控件：9 个类目的筛选维度数是 0~5、选项数是 2~70（阵营名 / 能力），
 * 任何常驻形态（chip 行 / 下拉行）都会让列表上沿随类目与筛选状态移动。
 * 把「按类目变化的部分」全部收进浮层，中栏头部高度与输入框宽度就恒定不变。
 *
 * 浮层经 Portal 渲染到 body：中栏 `<section>` 是 `overflow-hidden`（圆角裁剪），
 * 浮层留在其内部会被裁掉。位置按触发按钮的 `getBoundingClientRect()` 计算（右对齐）；
 * 自身限高 70vh 并内部滚动 —— 维度数与选项数都可能很大（生物有 5 个维度、能力有 60+ 项）。
 */
export function WikiFilterPanel({
  specs,
  entries,
  filters,
  onChange,
  onReset,
  contentLang,
  factionNames,
  modNames,
  t,
}: WikiFilterPanelProps) {
  const [open, setOpen] = useState(false);
  const [pos, setPos] = useState<{ top: number; left: number } | null>(null);
  const btnRef = useRef<HTMLButtonElement | null>(null);
  const panelRef = useRef<HTMLDivElement | null>(null);

  const disabled = specs.length === 0;
  const activeCount = specs.filter((s) => filters[s.key]).length;

  // 失焦自动关闭（对齐 ModManagerPanel 的弹层范式）：面板与按钮之外的任意处按下即收起。
  // 按钮与面板分处两棵 DOM 子树（Portal），故分别判断。
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      const target = e.target as Node;
      if (panelRef.current?.contains(target) || btnRef.current?.contains(target)) return;
      setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [open]);

  const toggle = () => {
    if (open) {
      setOpen(false);
      return;
    }
    const r = btnRef.current?.getBoundingClientRect();
    if (!r) return;
    setPos({
      top: r.bottom + 6,
      left: Math.max(8, Math.min(r.right - PANEL_W, window.innerWidth - PANEL_W - 8)),
    });
    setOpen(true);
  };

  const ctx: WikiFilterContext = { contentLang, t, factionNames, modNames };
  const optionsByKey = useMemo(
    () => Object.fromEntries(specs.map((s) => [s.key, s.options(entries, ctx)])),
    // ctx 由下列各项构成；t 随语言变化，一并列入以避免取词落后一拍
    [specs, entries, contentLang, factionNames, modNames, t],
  );

  // 维度 Tab：**一次只展开一个维度** —— 否则多个大列表（阵营 12 / 级别 10 / 能力 67）各带一个
  // 搜索框与独立滚动区，会叠成"面板滚动套列表滚动"的嵌套滚动（实测观感很乱）。
  // 只有 1 个维度时不渲染 Tab 行，直接显示该维度。
  const [activeKey, setActiveKey] = useState("");
  const activeSpec = specs.find((s) => s.key === activeKey) ?? specs[0];
  const activeOpts = activeSpec ? (optionsByKey[activeSpec.key] ?? []) : [];
  const activeValue = activeSpec ? (filters[activeSpec.key] ?? "") : "";

  return (
    <>
      <button
        ref={btnRef}
        type="button"
        onClick={toggle}
        disabled={disabled}
        title={disabled ? t.wiki.filter.none : t.wiki.filter.title}
        aria-label={t.wiki.filter.title}
        aria-expanded={open}
        className={cn(
          "relative flex h-9 w-9 flex-none items-center justify-center rounded-lg border transition-colors",
          disabled
            ? "cursor-not-allowed border-slate-800 text-slate-700"
            : open
              ? "border-blue-500/60 bg-blue-500/15 text-white"
              : "border-slate-700 text-slate-300 hover:border-slate-500 hover:text-white",
        )}
      >
        <ListFilter className="h-4 w-4" />
        {activeCount > 0 && (
          <span className="absolute -right-1 -top-1 flex h-4 min-w-4 items-center justify-center rounded-full bg-blue-500 px-1 text-[10px] font-medium text-white">
            {activeCount}
          </span>
        )}
      </button>

      {open &&
        pos &&
        createPortal(
          <div
            ref={panelRef}
            style={{ top: pos.top, left: pos.left, width: PANEL_W }}
            className="fixed z-[70] flex max-h-[70vh] flex-col rounded-lg border border-slate-700 bg-slate-900 p-3 shadow-xl"
          >
            <div className="mb-2 flex flex-none items-center justify-between">
              <p className="text-[11px] font-semibold uppercase tracking-wider text-slate-500">
                {t.wiki.filter.title}
              </p>
              {activeCount > 0 && (
                <button
                  type="button"
                  onClick={onReset}
                  className="text-[11px] text-blue-400 transition-colors hover:text-blue-300"
                >
                  {t.wiki.filter.reset}
                </button>
              )}
            </div>

            {specs.length > 1 && (
              <div className="mb-2 flex flex-none flex-wrap gap-1 border-b border-slate-800 pb-2">
                {specs.map((s) => (
                  <SpecTab
                    key={s.key}
                    label={s.label(t)}
                    active={s.key === activeSpec?.key}
                    hasValue={!!filters[s.key]}
                    onClick={() => setActiveKey(s.key)}
                  />
                ))}
              </div>
            )}

            <div className="min-h-0 flex-1 overflow-y-auto pr-0.5">
              {activeSpec && (
                <div>
                  {/* 单维度时不渲染 Tab，标题行仍需保留 */}
                  {specs.length === 1 && (
                    <p className="mb-1.5 text-[11px] font-medium text-slate-400">{activeSpec.label(t)}</p>
                  )}
                  {activeOpts.length <= CHIP_MAX ? (
                    <div className="flex flex-wrap gap-1">
                      <FilterChip
                        active={!activeValue}
                        label={t.wiki.filter.all}
                        count={entries.length}
                        onClick={() => onChange(activeSpec.key, "")}
                      />
                      {activeOpts.map((o) => (
                        <FilterChip
                          key={o.value}
                          active={activeValue === o.value}
                          label={o.label}
                          count={o.count}
                          onClick={() => onChange(activeSpec.key, o.value)}
                        />
                      ))}
                    </div>
                  ) : (
                    <OptionList
                      // key：切 Tab 时重建实例，清掉上一个维度的搜索词（否则会过滤空新维度的列表）
                      key={activeSpec.key}
                      options={activeOpts}
                      active={activeValue}
                      allLabel={t.wiki.filter.all}
                      allCount={entries.length}
                      searchPlaceholder={t.wiki.searchPlaceholder}
                      emptyLabel={t.wiki.filter.noOption}
                      onPick={(v) => onChange(activeSpec.key, v)}
                    />
                  )}
                </div>
              )}
            </div>
          </div>,
          document.body,
        )}
    </>
  );
}

/** 维度 Tab：切换当前展开的维度；已有选择值的维度右上角带一个小圆点 */
function SpecTab({
  label,
  active,
  hasValue,
  onClick,
}: {
  label: string;
  active: boolean;
  hasValue: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "relative rounded-md border px-2 py-1 text-[11px] transition-colors",
        active
          ? "border-blue-500/60 bg-blue-500/15 text-white"
          : "border-slate-700/60 text-slate-400 hover:text-white",
      )}
    >
      {label}
      {hasValue && (
        <span className="absolute -right-0.5 -top-0.5 h-1.5 w-1.5 rounded-full bg-blue-400" />
      )}
    </button>
  );
}

/** 单选 chip：样式对齐模组页的分类筛选 chip */
function FilterChip({
  active,
  label,
  count,
  onClick,
}: {
  active: boolean;
  label: string;
  count: number;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "rounded-full border px-2 py-0.5 text-[11px] transition-colors",
        active
          ? "border-blue-500/60 bg-blue-500/15 text-white"
          : "border-slate-700/60 text-slate-400 hover:text-white",
      )}
    >
      {label}
      <span className="ml-1 text-slate-500">{count}</span>
    </button>
  );
}

/**
 * 选项较多时的形态：搜索框 + 单选列表。
 *
 * 不用下拉（如 Radix Select）的原因：它的 Content 是 Portal 且不内置搜索，
 * 无法在 60+ 选项（生物能力）里快速定位；内联列表还能顺带显示每项的条目计数。
 * 高度由外层维度区滚动 + 内层 `max-h` 双层兜住，不会把浮层撑破。
 */
function OptionList({
  options,
  active,
  allLabel,
  allCount,
  searchPlaceholder,
  emptyLabel,
  onPick,
}: {
  options: WikiFilterOption[];
  active: string;
  allLabel: string;
  allCount: number;
  searchPlaceholder: string;
  emptyLabel: string;
  onPick: (value: string) => void;
}) {
  const [q, setQ] = useState("");
  const list = useMemo(() => {
    const k = q.trim().toLowerCase();
    if (!k) return options;
    return options.filter((o) => o.label.toLowerCase().includes(k));
  }, [options, q]);

  return (
    <>
      <div className="relative">
        <Search className="pointer-events-none absolute left-2 top-1.5 h-3.5 w-3.5 text-slate-500" />
        <input
          value={q}
          onChange={(e) => setQ(e.target.value)}
          placeholder={searchPlaceholder}
          className="w-full rounded border border-slate-700 bg-slate-800/70 py-1 pl-7 pr-2 text-[11px] text-slate-100 outline-none placeholder:text-slate-500 focus:border-blue-500"
        />
      </div>
      <div className="mt-1 max-h-[45vh] overflow-y-auto rounded border border-slate-800">
        <OptionRow active={!active} label={allLabel} count={allCount} onClick={() => onPick("")} />
        {list.map((o) => (
          <OptionRow
            key={o.value}
            active={active === o.value}
            label={o.label}
            count={o.count}
            onClick={() => onPick(o.value)}
          />
        ))}
        {list.length === 0 && <p className="px-2 py-2 text-[11px] text-slate-500">{emptyLabel}</p>}
      </div>
    </>
  );
}

/** 列表中的一行（单选） */
function OptionRow({
  active,
  label,
  count,
  onClick,
}: {
  active: boolean;
  label: string;
  count: number;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "flex w-full items-center justify-between gap-2 px-2 py-1.5 text-left text-[11px] transition-colors",
        active ? "bg-blue-500/15 text-white" : "text-slate-300 hover:bg-slate-800",
      )}
    >
      <span className="truncate">{label}</span>
      <span className="flex-none text-slate-500">{count}</span>
    </button>
  );
}
