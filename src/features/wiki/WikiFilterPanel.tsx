import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { ChevronDown, ListFilter, Search } from "lucide-react";
import { cn } from "@/lib/utils";
import type { LangDict } from "@/i18n/types";
import type { WikiEntry } from "@/lib/modcache";
import type { WikiFilterContext, WikiFilterOption, WikiFilterSpec } from "./filters";
import { PRIMARY_DIMS, dimLabel, hasSortSwitch, normalizeSort, secondaryDims } from "./sort";
import type { SortSpec } from "./sort";
import type { WikiCategory } from "@/lib/modcache";

/** 浮层宽度：固定值，便于按触发按钮右边界左对齐定位 */
const PANEL_W = 272;
/** 选项数超过此值改用「搜索 + 单选列表」（chip 行在窄栏里会折成很多行） */
const CHIP_MAX = 6;

export interface WikiFilterPanelProps {
  /** 当前类目的筛选维度（空数组 = 该类目无维度，按钮禁用但仍占位） */
  specs: WikiFilterSpec[];
  /**
   * 该类目**经过搜索、但未经维度筛选**的条目（选项与计数都基于它）。
   *
   * 传搜索结果而非全量：否则「全部 (12)」指的是全量 12 条，而列表实际只剩搜索命中的 3 条，
   * 数字与列表对不上。搜索被视为「先定范围」，维度在其之上联动。
   */
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
  /** 当前类目的排序配置（双槽）；仅生物/英雄/建筑三类提供切换 */
  sortSpec: SortSpec;
  /** 排序类目（决定维度池与说明文案；与 `specs` 同类目） */
  sortCategory: WikiCategory;
  onSortChange: (spec: SortSpec) => void;
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
  sortSpec,
  sortCategory,
  onSortChange,
  t,
}: WikiFilterPanelProps) {
  // 兜底规范化：保证「组内排序」**永远有且仅有一个选中项**。
  // 旧缓存 / 手改 localStorage 可能留下非法组合（如 primary=level 而 secondary=level），
  // 不兜底就会出现两个按钮都不高亮的状态真空 —— 用户看不懂当前到底怎么排。
  const sortSpecNorm = normalizeSort(sortCategory, sortSpec);
  const [open, setOpen] = useState(false);
  // 排序区块展开态：**默认折叠**（低频操作不该常驻），摘要行始终显示当前排序。
  // 组件由调用方以 `key={category}` 挂载 → 切类目时自动重置为折叠。
  const [sortOpen, setSortOpen] = useState(false);
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

  /**
   * 维度联动（faceted search）：每个维度的选项与计数，基于**其它维度已选值的交集**。
   *
   * 关键点是**排除自身**：若把自身选值也算进基准，选中「城堡」后再看「城堡」维度，
   * 其余阵营的计数会全部变成 0 —— 用户就没法改选了（这是分面筛选的标准做法）。
   *
   * 由此得到一条硬性质：**选不出 0 条结果**。计数 0 的选项会被置灰不可点，
   * 而每个维度给出的可选项都是"在当前其它条件下还有货"的，故任意组合的结果集非空。
   * 代价是当前展开维度的计数不再反映全量分布（要看全量分布把其它维度清掉即可）。
   *
   * 复杂度 O(N×D)（生物：1400 条 × 4 维 ≈ 数千次判定 + 能力数组展开），无感，无需 memo 之外的优化。
   */
  const subsetByKey = useMemo(() => {
    const out: Record<string, WikiEntry[]> = {};
    for (const s of specs) {
      const others = specs.filter((x) => x.key !== s.key && filters[x.key]);
      out[s.key] = others.length
        ? entries.filter((e) => others.every((x) => x.match(e, filters[x.key])))
        : entries;
    }
    return out;
  }, [specs, entries, filters]);

  const optionsByKey = useMemo(
    () => Object.fromEntries(specs.map((s) => [s.key, s.options(subsetByKey[s.key], ctx)])),
    // ctx 由下列各项构成；t 随语言变化，一并列入以避免取词落后一拍
    [specs, subsetByKey, contentLang, factionNames, modNames, t],
  );

  // 维度 Tab：**一次只展开一个维度** —— 否则多个大列表（阵营 12 / 级别 10 / 能力 67）各带一个
  // 搜索框与独立滚动区，会叠成"面板滚动套列表滚动"的嵌套滚动（实测观感很乱）。
  // 只有 1 个维度时不渲染 Tab 行，直接显示该维度。
  const [activeKey, setActiveKey] = useState("");
  const activeSpec = specs.find((s) => s.key === activeKey) ?? specs[0];
  const activeOpts = activeSpec ? (optionsByKey[activeSpec.key] ?? []) : [];
  const activeValue = activeSpec ? (filters[activeSpec.key] ?? "") : "";
  // 「全部」的计数 = 当前维度基准集大小（其它维度已生效、自身未生效）
  const activeSubset = activeSpec ? (subsetByKey[activeSpec.key] ?? entries) : entries;

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

            {/* 排序（双槽：主排序 + 次排序）。与筛选**正交** —— 筛选是收窄（只看一个
                阵营），排序是分组（全部条目按维度分段），所以「按等级排序」不是
                「按等级筛选」的重复：筛选只能看单个等级，排序能看 T1(16 城) → T2(16 城) → …
                仅生物/英雄/建筑三类提供；其余类目**整块隐藏**而非禁用 —— 它们没有城镇维度，
                显示一个禁用的开关会让人以为"设置失效了"，隐藏则直接表达"这类没有排序选项"。 */}
            {hasSortSwitch(sortCategory) && (
              <div className="mb-2 flex-none rounded-md border border-slate-800/70 bg-slate-900/50">
                {/* 摘要行（折叠态只占一行）：**始终显示当前排序**，状态不隐藏；
                    点整行展开完整选项。排序是低频操作（设一次就好），不该常驻展开 ——
                    否则会把下面的筛选维度挤出可视区。 */}
                <button
                  type="button"
                  onClick={() => setSortOpen((v) => !v)}
                  aria-expanded={sortOpen}
                  className="flex w-full items-center gap-2 px-2 py-1.5 text-left transition-colors hover:bg-slate-800/40"
                >
                  <span className="flex-none text-[11px] font-medium text-slate-400">
                    {t.wiki.sort.title}
                  </span>
                  <span className="min-w-0 flex-1 truncate text-[11px] text-slate-200">
                    {sortSpecNorm.primary === "name"
                      ? dimLabel("name", t)
                      : `${dimLabel(sortSpecNorm.primary, t)} → ${
                          // 次维度是名称时用短标签：chip 上"按名称"是 radio 语义，
                          // 摘要链里再出现一个「按」会读成「宝物等级 → 按名称」
                          sortSpecNorm.secondary === "name"
                            ? t.wiki.sort.nameShort
                            : dimLabel(sortSpecNorm.secondary, t)
                        }`}
                  </span>
                  <ChevronDown
                    className={cn(
                      "h-3.5 w-3.5 flex-none text-slate-500 transition-transform",
                      sortOpen && "rotate-180",
                    )}
                  />
                </button>
                {sortOpen && (
                  <div className="px-2 pb-2">
                    {/* 分组维度（radio 形态）：**组内选项作为缩进子行紧随其下**，
                        仅当前选中项展开 —— 位置即从属，不需要"组内排序"这种额外标题
                        （那个词本身就是理解障碍）。 */}
                    <div className="space-y-1">
                      {PRIMARY_DIMS[sortCategory]
                        .filter((d) => d !== "name")
                        .map((d) => (
                          <div key={d}>
                            <SpecTab
                              label={dimLabel(d, t)}
                              active={sortSpecNorm.primary === d}
                              hasValue={false}
                              onClick={() => {
                                // 切换主排序：次排序**联动到合法值**（旧值仍可用则保留，
                                // 否则取第一个）。否则会出现"组内按钮都不高亮"的状态真空。
                                const opts = secondaryDims(sortCategory, d);
                                const keep = opts.includes(sortSpecNorm.secondary)
                                  ? sortSpecNorm.secondary
                                  : opts[0] ?? "name";
                                onSortChange({ primary: d, secondary: keep });
                              }}
                            />
                            {/* 子行：仅当前选中项展开，且**组内选项多于一项时才渲染** ——
                                宝物/法术的组内只有"名称"一项，渲染出来点了也没变化。 */}
                            {sortSpecNorm.primary === d &&
                              secondaryDims(sortCategory, d).length > 1 && (
                                <div className="ml-4 mt-1 flex flex-wrap gap-1">
                                  {secondaryDims(sortCategory, d).map((s) => (
                                    <SpecTab
                                      key={s}
                                      label={dimLabel(s, t)}
                                      active={sortSpecNorm.secondary === s}
                                      hasValue={false}
                                      onClick={() =>
                                        onSortChange({ primary: d, secondary: s })
                                      }
                                    />
                                  ))}
                                </div>
                              )}
                          </div>
                        ))}
                      {/* 不分组（按名称）：独立项，没有"组"就没有组内选项 → 子行整块消失 */}
                      <SpecTab
                        label={t.wiki.sort.noGroup}
                        active={sortSpecNorm.primary === "name"}
                        hasValue={false}
                        onClick={() => onSortChange({ primary: "name", secondary: "name" })}
                      />
                    </div>
                    <p className="mt-1.5 text-[10px] leading-tight text-slate-500">
                      <span className="text-slate-600">{t.wiki.sort.noInfoLast}</span>
                    </p>
                  </div>
                )}
              </div>
            )}

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
                        // 「全部」永远可点：它正是把本维度从联动条件里摘掉的那一步
                        disabled={false}
                        label={t.wiki.filter.all}
                        count={activeSubset.length}
                        onClick={() => onChange(activeSpec.key, "")}
                      />
                      {activeOpts.map((o) => (
                        <FilterChip
                          key={o.value}
                          active={activeValue === o.value}
                          // 0 条 = 与当前其它维度的选择无交集 → 置灰不可点（不隐藏：
                          // 隐藏会让选项列表随选择跳动，"刚才还在的选项不见了"更难理解）
                          disabled={o.count === 0 && activeValue !== o.value}
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
                      allCount={activeSubset.length}
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
  disabled,
  label,
  count,
  onClick,
}: {
  active: boolean;
  disabled: boolean;
  label: string;
  count: number;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={cn(
        "rounded-full border px-2 py-0.5 text-[11px] transition-colors",
        active
          ? "border-blue-500/60 bg-blue-500/15 text-white"
          : disabled
            ? "cursor-not-allowed border-slate-800/60 text-slate-600"
            : "border-slate-700/60 text-slate-400 hover:text-white",
      )}
    >
      {label}
      <span className={cn("ml-1", disabled ? "text-slate-700" : "text-slate-500")}>{count}</span>
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
        <OptionRow active={!active} disabled={false} label={allLabel} count={allCount} onClick={() => onPick("")} />
        {list.map((o) => (
          <OptionRow
            key={o.value}
            active={active === o.value}
            // 0 条 = 与当前其它维度的选择无交集 → 置灰不可点（不隐藏，理由同 FilterChip）
            disabled={o.count === 0 && active !== o.value}
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
  disabled,
  label,
  count,
  onClick,
}: {
  active: boolean;
  disabled: boolean;
  label: string;
  count: number;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={cn(
        "flex w-full items-center justify-between gap-2 px-2 py-1.5 text-left text-[11px] transition-colors",
        active
          ? "bg-blue-500/15 text-white"
          : disabled
            ? "cursor-not-allowed text-slate-600"
            : "text-slate-300 hover:bg-slate-800",
      )}
    >
      <span className="truncate">{label}</span>
      <span className={cn("flex-none", disabled ? "text-slate-700" : "text-slate-500")}>{count}</span>
    </button>
  );
}
