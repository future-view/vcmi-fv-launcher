/**
 * 巡礼配置页的**通用表单控件**（滑杆 + 数值框 / 多选列表 / 作用域徽标）。
 *
 * 从 `ShowcaseTab.tsx` 抽出：这组控件不看巡礼数据结构，只认自己的入参与回调，
 * 与配置页的装配逻辑无关；抽成独立模块后主组件只剩「哪块配置用哪个控件」。
 */
import { useMemo, useState } from "react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";

/** ms → 人读文案（<60 秒显示秒，否则「N 分」/「N 分 M 秒」）；单位词随启动器界面语言，不硬编码 */
export function fmtSec(ms: number, sec: string, min: string): string {
  const s = ms / 1000;
  if (s < 60) return `${Number.isInteger(s) ? s : s.toFixed(1)} ${sec}`;
  const m = Math.floor(s / 60);
  const rest = Math.round(s % 60);
  return rest ? `${m} ${min} ${rest} ${sec}` : `${m} ${min}`;
}

/**
 * 行内滑杆 + **数值输入框**（§10）。
 *
 * 滑条适合粗调，但「120 分钟」「10 秒」这类区间上滑条分辨率太差（一像素几十秒），
 * 所以并排给一个输入框直接敲数值；`value` 全程用 **ms**，输入框按**秒**编辑。
 * 输入过程允许临时空串/越界，失焦或回车时才夹进 `[min, max]`。
 */
export function Slider({
  label,
  hint,
  min,
  max,
  step,
  value,
  unit,
  onChange,
}: {
  label: string;
  hint?: string;
  min: number;
  max: number;
  step: number;
  value: number;
  unit?: string;
  onChange: (v: number) => void;
}) {
  const { t } = useI18n();
  // 未显式指定单位时按「秒」（所有调用点皆巡礼时长，单位词随界面语言）
  const unitText = unit ?? t.realm.showcaseSeconds;
  const [draft, setDraft] = useState<string | null>(null);
  /**
   * 秒值显示：**保留小数**（最多两位，去掉多余的 0）。
   *
   * ⚠️ 此前是 `String(Math.round(value / 1000))` → 3200ms 显示成 `3`，
   * 而下方滑条标注写的是 `3.2 秒`，同一行给出两个答案（"到底是 3 还是 3.2"）。
   * 步进本来就是 200ms，秒必须是小数才有意义。
   */
  const fmtVal = (ms: number) => String(Number((ms / 1000).toFixed(2)));
  const commit = () => {
    if (draft === null) return;
    const n = Number(draft);
    setDraft(null);
    if (Number.isFinite(n)) {
      // ⚠️ 不要 `Math.round(n)`：那会把 3.2 秒吞成 3 秒（步进 200ms 的值只能靠小数表达）
      onChange(Math.max(min, Math.min(max, Math.round(n * 1000))));
    }
  };
  return (
    <div className="space-y-1.5">
      <div className="flex items-center justify-between gap-2">
        <span className="min-w-0 truncate text-sm text-slate-200">
          {label}
          {hint && <span className="ml-1 text-xs text-slate-500">{hint}</span>}
        </span>
        <div className="flex shrink-0 items-center gap-1">
          <input
            type="number"
            min={fmtVal(min)}
            max={fmtVal(max)}
            // 步进也用秒（0.2 / 0.5 / 1），否则微调按钮会按 1 秒跳，与滑条粒度不一致
            step={fmtVal(step)}
            value={draft ?? fmtVal(value)}
            onChange={(e) => setDraft(e.target.value)}
            onBlur={commit}
            onKeyDown={(e) => {
              if (e.key === "Enter") e.currentTarget.blur();
            }}
            className="w-20 rounded-lg border border-slate-700 bg-slate-950 px-2 py-0.5 text-right text-sm tabular-nums text-slate-100"
          />
          <span className="text-xs text-slate-500">{unitText}</span>
        </div>
      </div>
      <input
        type="range"
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(e) => onChange(Number(e.target.value))}
        className="w-full accent-blue-500"
      />
      <div className="flex items-center justify-between text-[11px] text-slate-500">
        <span>{fmtSec(min, t.realm.showcaseSeconds, t.realm.showcaseMinutes)}</span>
        <span className="tabular-nums text-slate-300">{fmtSec(value, t.realm.showcaseSeconds, t.realm.showcaseMinutes)}</span>
        <span>{fmtSec(max, t.realm.showcaseSeconds, t.realm.showcaseMinutes)}</span>
      </div>
    </div>
  );
}

/**
 * 固定选项多选（替代手打 id）：搜索 + 复选框列表 + 批量按钮。
 *
 * `sub` 是条目的副文本（英雄用职业名——只看名字分不清谁是谁）；
 * `onBulk` 提供「全选 / 随机 / 清空」三个批量操作（长列表逐个点太累）。
 */
export function Picker({
  items,
  selected,
  lockedIds,
  markIds,
  onToggle,
  onMark,
  onBulk,
  placeholder,
  emptyText,
}: {
  items: { id: string; label: string; sub?: string }[];
  selected: Set<string>;
  /** 自动项：已勾选且**不可取消**（巡礼兵种带出的巢穴建筑）；批量操作也不动它们 */
  lockedIds?: Set<string>;
  /** 次要列（右侧 `★`）：与主勾选**相互独立**，用于"重点兵种"这类额外标记 */
  markIds?: Set<string>;
  onToggle: (id: string) => void;
  onMark?: (id: string) => void;
  onBulk?: (ids: string[]) => void;
  placeholder: string;
  emptyText: string;
}) {
  const { t } = useI18n();
  const [q, setQ] = useState("");
  const list = useMemo(() => {
    const kw = q.trim().toLowerCase();
    if (!kw) return items;
    return items.filter(
      (i) =>
        i.label.toLowerCase().includes(kw) ||
        i.id.toLowerCase().includes(kw) ||
        (i.sub ?? "").toLowerCase().includes(kw),
    );
  }, [items, q]);
  /**
   * 随机挑一批。
   *
   * ⚠️ **数量也必须随机**：原先固定按 `len/3` 取，14 项的列表永远都是 5 个，
   * 连点几次结果个数完全一样，观感上就是"随机没生效"。现改为在 `1 ~ ceil(len/2)` 之间随机取个数。
   */
  const pickRandom = () => {
    const pool = [...items];
    const max = Math.max(1, Math.ceil(pool.length / 2));
    const n = 1 + Math.floor(Math.random() * max);
    const out: string[] = [];
    for (let k = 0; k < n && pool.length > 0; k++) {
      out.push(pool.splice(Math.floor(Math.random() * pool.length), 1)[0].id);
    }
    return out;
  };
  return (
    <div className="flex w-full flex-col gap-1.5">
      <input
        value={q}
        onChange={(e) => setQ(e.target.value)}
        placeholder={placeholder}
        className="w-full rounded-lg border border-slate-700 bg-slate-950 px-2 py-1 text-sm text-slate-100"
      />
      {onBulk && items.length > 0 && (
        <div className="flex items-center gap-3 px-1 text-[11px]">
          <button
            type="button"
            onClick={() => onBulk(items.map((i) => i.id))}
            className="text-blue-300 underline hover:text-blue-200"
          >
            {t.realm.showcaseSelectAll}
          </button>
          <button
            type="button"
            onClick={() => onBulk(pickRandom())}
            className="text-blue-300 underline hover:text-blue-200"
          >
            {t.realm.showcaseRandomPick}
          </button>
          <button
            type="button"
            onClick={() => onBulk([])}
            className="text-slate-400 underline hover:text-slate-200"
          >
            {t.realm.showcaseClearAll}
          </button>
          <span className="ml-auto text-slate-500">
            {selected.size}/{items.length}
          </span>
        </div>
      )}
      <div className="max-h-40 space-y-0.5 overflow-y-auto rounded-lg border border-slate-800 bg-slate-950/60 p-1">
        {list.map((i) => {
          const locked = lockedIds?.has(i.id) ?? false;
          const marked = markIds?.has(i.id) ?? false;
          return (
            <div
              key={i.id}
              className={cn(
                "flex items-center gap-2 rounded-md px-2 py-1 text-xs text-slate-200",
                locked ? "bg-slate-800/60" : "hover:bg-slate-800/60",
                selected.has(i.id) && !locked && "bg-amber-500/12",
              )}
            >
              {/* 主勾选（最左）＝ **是否参与巡礼**；名称整块可点 */}
              <label
                className={cn(
                  "flex min-w-0 flex-1 items-center gap-2",
                  locked ? "cursor-default" : "cursor-pointer",
                )}
              >
                <input
                  type="checkbox"
                  checked={selected.has(i.id)}
                  disabled={locked}
                  onChange={() => onToggle(i.id)}
                  className="h-3.5 w-3.5 shrink-0 accent-amber-500 disabled:opacity-60"
                />
                <span className="min-w-0 flex-1">
                  <span className="block truncate">
                    {i.label}
                    {locked && (
                      <span className="ml-1.5 rounded bg-slate-700/70 px-1 py-px text-[10px] text-slate-300">
                        {t.realm.showcaseAutoTag}
                      </span>
                    )}
                  </span>
                  {i.sub && (
                    <span className="block truncate text-[10px] text-slate-500">{i.sub}</span>
                  )}
                </span>
              </label>
              {/* id 列：固定宽度 + 右对齐 —— id 长短不一（skeleton / skeletonWarrior），
                  不定宽的话右侧 `★` 列会参差不齐。 */}
              <span className="w-[104px] shrink-0 truncate text-right font-mono text-[10px] text-slate-500">
                {i.id}
              </span>
              {/* `★` 列**固定在整行最右侧**，与「参与」勾选相互独立。
                  刻意用 button 而非嵌套 checkbox：整行是 label，
                  嵌套的 labelable 元素会连带触发外层勾选（点一下跳两个状态）。 */}
              {onMark && (
                <button
                  type="button"
                  onClick={() => onMark(i.id)}
                  disabled={locked}
                  title={t.realm.showcaseKeyMarkTitle}
                  className={cn(
                    "grid h-5 w-5 shrink-0 place-items-center rounded border text-[11px] leading-none transition-colors",
                    marked
                      ? "border-amber-500/70 bg-amber-500/20 text-amber-300"
                      : "border-slate-700 text-slate-600 hover:border-slate-500 hover:text-slate-300",
                    locked && "cursor-not-allowed opacity-50",
                  )}
                >
                  ★
                </button>
              )}
            </div>
          );
        })}
        {list.length === 0 && <div className="px-2 py-1 text-xs text-slate-500">{emptyText}</div>}
      </div>
    </div>
  );
}

/**
 * 配置项的**作用域徽标**（方案 C 的界面部分）。
 *
 * 作用域从此是**数据结构**的属性（共用的扁平在 `cfg`、自动专属在 `cfg.auto`、手工专属在 `cfg.manual`），
 * 但作者看不到类型定义 —— 所以每个块都要**显式标出**它属于哪一类，
 * 否则"我切了模式，这项还有没有效"只能靠猜。
 */
export function ScopeBadge({ scope }: { scope: "shared" | "auto" | "manual" }) {
  const { t } = useI18n();
  const map = {
    shared: { text: t.realm.showcaseScopeShared, cls: "bg-slate-700/50 text-slate-300" },
    auto: { text: t.realm.showcaseScopeAuto, cls: "bg-blue-500/15 text-blue-300" },
    manual: { text: t.realm.showcaseScopeManual, cls: "bg-amber-500/15 text-amber-300" },
  } as const;
  const m = map[scope];
  return (
    <span className={cn("shrink-0 rounded px-1.5 py-0.5 text-[10px]", m.cls)}>{m.text}</span>
  );
}
