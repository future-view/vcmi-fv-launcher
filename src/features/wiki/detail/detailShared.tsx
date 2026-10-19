import { useState, type ReactNode } from "react";
import { cn } from "@/lib/utils";
import type { LangDict } from "@/i18n/types";
import { imgCandidates } from "@/lib/hd";
import type { WikiCategory, WikiEntry, WikiLinkTarget } from "@/lib/modcache";
import { fullImgUrl, pickName } from "@/lib/modcache";
import { getVcmiLanguageSnapshot } from "@/lib/vcmiLanguage";
import { HdIcon } from "@/shared/HdIcon";
import { HdImg } from "@/shared/HdImg";
import { useHdRate } from "@/shared/useHdRate";

/**
 * VCMI 生物动画组号（client/battle/BattleConstants.h:61-91 ECreatureAnimType）
 * 与 DEF 文件的 group 索引一一对应。
 */
export const ANIM = {
  MOVING: 0,
  MOUSEON: 1,
  HOLDING: 2, // 基础待机
  HITTED: 3, // 受击
  DEFENCE: 4, // 防御
  DEATH: 5,
  DEATH_RANGED: 6,
  TURN_L: 7,
  TURN_R: 8,
  ATTACK_UP: 11,
  ATTACK_FRONT: 12,
  ATTACK_DOWN: 13,
  SHOOT_UP: 14,
  SHOOT_FRONT: 15,
  SHOOT_DOWN: 16,
  SPECIAL_UP: 17,
  SPECIAL_FRONT: 18,
  SPECIAL_DOWN: 19,
  MOVE_START: 20,
  MOVE_END: 21,
} as const;

/** 预览循环动作表（client/widgets/Images.cpp:492-505 loopPreview） */
export const CREA_PREVIEW = [ANIM.HOLDING, ANIM.HITTED, ANIM.DEFENCE, ANIM.ATTACK_FRONT, ANIM.SPECIAL_FRONT];

export const MACH_PREVIEW = [ANIM.HOLDING, ANIM.MOVING, ANIM.SHOOT_UP, ANIM.SHOOT_FRONT, ANIM.SHOOT_DOWN];

/**
 * 安全图片：加载失败时显示占位，避免 broken 图标。
 *
 * **已内置高清素材支持**：按显示器倍率自动尝试 `@Nx` 变体，读不到就回退原图
 * （`lib/hd.ts::imgCandidates`）。因此**所有调用点无需改动**即可享受高清，
 * 高清缺失的记忆与回退由 `HdImg` 统一处理（详见其头注）。
 *
 * `pixelArt` 在这里是安全的：`HdImg` 会**按素材实际像素与物理显示尺寸动态判定** ——
 * 素材充足（缩小/1:1）时自动用平滑插值，只有真被放大时才切最近邻。
 * 百科里两种情形都有（建筑 150×70 → 80×56 是缩小；4K 下 32×32 小图标、
 * `EntryDetail` 的 320px 大图则是放大），动态判定各自正确，不会出现最近邻缩小的摩尔纹。
 */
export function SafeImg({
  src,
  alt,
  className,
  placeholderClassName,
}: {
  src?: string | null;
  alt?: string;
  className?: string;
  placeholderClassName?: string;
}) {
  const rate = useHdRate();
  return (
    <HdImg
      src={imgCandidates(src ?? "", rate)}
      alt={alt}
      className={className}
      placeholderClassName={placeholderClassName}
      pixelArt
    />
  );
}

export interface DetailProps {
  entry: WikiEntry;
  contentLang: string;
  t: LangDict;
  cacheDir?: string | null;
  modCache?: Record<string, unknown>;
  onNavigate?: (target: WikiLinkTarget) => void;
  // compact=true 用于右键弹窗：去掉顶部大图 banner，由各子组件按 VCMI 弹窗布局自渲染标题
  compact?: boolean;
  /**
   * **跨模组**按 (类目, id) 查条目（升级链 / 组件等跨 mod 引用需要）。
   *
   * 详情组件手上的 `modCache` 只是「当前条目所属模组」的那一份缓存，而 VCMI 的升级链
   * 常指向**别的模组**里的生物（实测 8,011 条引用里 301 条跨模组：mod 生物升级 core
   * 生物最典型，如 `tears-of-ashan` 的 `bowman` → `core.castle` 的 `marksman`）。
   * 没有它时这些条目只能显示裸 id、且没有图标。
   *
   * 可选：调用方没有全局索引时（如领域信息页只装了城镇）缺省即退化为"只查本 mod"，
   * 与改动前一致，不会更差。
   */
  getEntry?: (category: WikiCategory, id: string) => WikiEntry | undefined;
  /**
   * 聚合态（模组页）里每个条目的**来源子模组**：键 `${category}:${id}`。
   *
   * 只在「父模组页」出现——那一页合并了全部后代子模组的内容，行尾要能看出每样东西
   * 来自哪个子模组，点击跳转也要指向来源 mod（指向父模组会指错对象）。
   * 缺省（其它类目 / 未聚合）时所有条目都属于当前 mod，行尾不显示来源。
   */
  sourceLabels?: Record<string, { modId: string; label: string }>;
}

/** `makeRefResolver` 的解析结果 */
export interface RelatedRef {
  /** 命中记录：本 mod 表里的原始对象，或全局条目的 `data`；未命中为 undefined */
  record?: Record<string, unknown>;
  /** 显示名（已按 lang 取词；取不到回退 id） */
  name: string;
  /** 图片路径（本 mod 记录的 image / 全局条目的 image）；取不到 undefined */
  image?: string;
  /** 命中所在 mod —— 跳转/弹窗必须用它，否则会落到本 mod 的同名空位 */
  modId: string;
  /** 全局索引命中时的条目（弹窗需要 `data` + `modId`） */
  entry?: WikiEntry;
}

/**
 * 构造「按 id 解析关联条目」的函数：**本 mod 缓存表优先 → 全类目索引兜底**。
 *
 * 为什么需要兜底：`modCache` 只是**当前条目所属模组**那一份缓存，而关联引用常跨模组 ——
 * 实测该发行版 259 条宝物组件引用里 **123 条（47%）只存在于 core**（mod 的组合宝物引用
 * 原版组件），英雄初始部队 7,397 个单位里 126 个（战争机器 `ballista` / `firstAidTent` /
 * `catapult`）同样只在 core。没有兜底时这些行只能显示裸 id 且没有图标（组件行还会因为
 * 拿不到 `warMachine` 而把战争机器错分到"初始部队"里）。
 *
 * 调用方缺省不传 `getEntry`（如领域信息页只装了城镇）时自动退化为"只查本 mod"，
 * 与改动前一致，不会更差。
 */
export function makeRefResolver(opts: {
  category: WikiCategory;
  /** 当前条目所属 mod（本地命中即它） */
  modId: string;
  lang: string;
  modCache?: Record<string, unknown>;
  getEntry?: (category: WikiCategory, id: string) => WikiEntry | undefined;
}): (id: string) => RelatedRef {
  const local = (opts.modCache?.[opts.category] as Record<string, unknown>[] | undefined) ?? [];
  const byId = new Map(local.map((x) => [String(x.id ?? ""), x]));
  return (id: string): RelatedRef => {
    const hit = byId.get(id);
    if (hit) {
      return {
        record: hit,
        name: pickName((hit.name as Record<string, string>) ?? {}, opts.lang) || id,
        image: (hit.image as string | undefined) || undefined,
        modId: opts.modId,
      };
    }
    const entry = opts.getEntry?.(opts.category, id);
    if (entry) {
      return {
        record: entry.data as Record<string, unknown>,
        name: pickName(entry.name, opts.lang) || id,
        image: entry.image || undefined,
        modId: entry.modId,
        entry,
      };
    }
    return { name: id, modId: opts.modId };
  };
}

// 信息窗标题栏（对齐 VCMI CInfoWindow：图标 + 名称 + 副标题/类型徽标）
export function DetailHeader({
  icon,
  name,
  subtitle,
  badge,
}: {
  icon?: string;
  name: string;
  subtitle?: React.ReactNode;
  badge?: string;
}) {
  return (
    <div className="flex items-center gap-3 rounded-xl border border-slate-700/50 bg-gradient-to-r from-slate-800/60 to-slate-900/30 p-3">
      {icon ? (
        <HdIcon src={icon} alt="" className="h-14 w-14 flex-none rounded-lg border border-slate-600/50 object-contain" />
      ) : (
        <div className="h-14 w-14 flex-none rounded-lg border border-slate-600/50 bg-slate-800/60" />
      )}
      <div className="min-w-0 flex-1">
        <h2 className="truncate text-lg font-bold text-slate-100">{name || "—"}</h2>
        {subtitle ? <p className="truncate text-xs text-slate-400">{subtitle}</p> : null}
      </div>
      {badge ? (
        <span className="flex-none rounded-full bg-blue-500/20 px-2.5 py-1 text-[11px] font-medium uppercase tracking-wide text-blue-300">
          {badge}
        </span>
      ) : null}
    </div>
  );
}

export function StatRow({ k, v }: { k: string; v?: unknown }) {
  if (v === undefined || v === null || v === "" || v === false) return null;
  return (
    <div className="flex items-center justify-between py-1">
      <span className="text-sm text-slate-400">{k}</span>
      <span className="text-sm font-medium text-slate-100">{v as ReactNode}</span>
    </div>
  );
}

/**
 * 统计表的一行：**固定 144px 标签列 + 值列左对齐**。
 *
 * 两个刻意的偏离（相对 VCMI 与相对本文件里的 `StatRow`）：
 * 1. 值列**不贴右边界**：VCMI `WikiCreatureContent.cpp:177` 的值列起点是
 *    `colLbl = tableW / 2`（表格中线），但那是因为它的 viewport 只有约 470px；
 *    我们的右栏是自适应宽度（可到 900+），照抄"两端对齐"会把标签与值拉到数百 px 外。
 * 2. 标签列**用固定宽度而非 50%**：144px 的依据是 5 语言字段名实测 —— 最长
 *    pl `Podwójna szerokość`（19 字符 ≈ 110px），加 padding 与余量；值列全是数字/短词，
 *    最宽是地图部队 `12 – 20`（≈60px）与 5 位 AI 价值，不会挤。
 */
export function StatTableRow({ k, v }: { k: string; v: string }) {
  return (
    <div className="grid grid-cols-[144px_minmax(0,1fr)] items-baseline gap-x-2 border-b border-slate-800/40 px-1 py-1 last:border-b-0">
      <span className="truncate text-[11px] text-slate-400" title={k}>
        {k}
      </span>
      <span className="text-[11px] font-medium text-amber-200/90">{v}</span>
    </div>
  );
}

/**
 * 关联条目的**整行框**：图标 + 名称，整行可点（跳转）/ 可右键（名称气泡）。
 *
 * 对齐 VCMI `WikiCreatureContent.cpp:215-275` 的升级链与 `:411-450` 的招募建筑：
 * 那里用 `GraphicalPrimitiveCanvas` 画一个占满行宽的框，图标 36px / 42px，
 * 名称紧随其后，整行是 `WikiClickable`。做成整行框而不是行内 chip，是因为
 * 这类条目是"这条目的兄弟实体"，需要一个明确的落点，而不是一段文字里的高亮。
 */
export function RelatedRow({
  icon,
  label,
  iconSize = 36,
  onClick,
  onContextMenu,
}: {
  icon?: string | null;
  label: string;
  iconSize?: number;
  onClick?: () => void;
  onContextMenu?: (e: React.MouseEvent) => void;
}) {
  const body = (
    <>
      <span
        className="flex flex-none items-center justify-center"
        style={{ height: iconSize, width: iconSize }}
      >
        {icon ? <HdIcon src={icon} alt="" className="h-full w-full object-contain" /> : null}
      </span>
      <span className="min-w-0 flex-1 truncate text-[11px] text-slate-200">{label}</span>
    </>
  );
  const cls = "flex w-full items-center gap-2 px-2 py-1 text-left";
  return (
    <div
      className="rounded-lg border border-slate-700/50 bg-slate-800/30"
      onContextMenu={onContextMenu}
    >
      {onClick ? (
        <button
          type="button"
          onClick={onClick}
          className={cn(cls, "transition-colors hover:border-blue-500/40 hover:bg-slate-700/40")}
        >
          {body}
        </button>
      ) : (
        <div className={cls}>{body}</div>
      )}
    </div>
  );
}

// 资源 key -> VCMI SMALRES 帧索引（与 extract_resource_icons 一致）
const RESOURCE_INDEX: Record<string, number> = {
  wood: 0,
  mercury: 1,
  ore: 2,
  sulfur: 3,
  crystal: 4,
  gem: 5,
  gems: 5,
  gold: 6,
  mitril: 7,
  mithril: 7,
};

/** 后端 `resources` 段条目（cache.json.resources，v79 起）：多语言名 + 帧索引 + 图标 */
export type ResourceInfo = {
  id?: string;
  index?: number;
  name?: Record<string, string>;
  image?: string | null;
};

/** 资源键归一：`gem`/`gems`、`mithril`/`mitril` 等价（配置与 resources 段的写法差异） */
function normResKey(k: string): string {
  const s = k.toLowerCase();
  return s === "gem" ? "gems" : s === "mithril" ? "mitril" : s;
}

/** 按 key 在 resources 段中查条目（容错单复数/别名/大小写） */
function findResource(key: string, resources?: ResourceInfo[]): ResourceInfo | undefined {
  if (!resources?.length) return undefined;
  const n = normResKey(key);
  return resources.find((r) => normResKey(String(r.id ?? "")) === n);
}

/** 资源视觉（图标 + 多语言名）：图标优先 resources 段 image（阶段二），回退帧索引约定 */
function resourceVisual(
  key: string,
  cacheDir?: string | null,
  resources?: ResourceInfo[],
  lang = getVcmiLanguageSnapshot() ?? "english",
): { icon: string; name: string } {
  const r = findResource(key, resources);
  const idx = r?.index ?? RESOURCE_INDEX[normResKey(key)];
  const icon = r?.image
    ? fullImgUrl(cacheDir ?? null, r.image)
    : idx !== undefined
      ? fullImgUrl(cacheDir ?? null, `core/images/resources/${idx}.webp`)
      : "";
  const name = r?.name ? pickName(r.name, lang) : "";
  return { icon, name };
}

/** 把 cost 对象（{resource: amount}）格式化为带资源图标/名称的节点 */
export function formatCost(
  cost: unknown,
  cacheDir?: string | null,
  modId?: string,
  resources?: ResourceInfo[],
  lang = getVcmiLanguageSnapshot() ?? "english",
): ReactNode {
  if (!cost || typeof cost !== "object") return "—";
  const entries = Object.entries(cost as Record<string, unknown>).filter(([, v]) => v);
  if (entries.length === 0) return "—";
  return (
    <span className="flex flex-wrap items-center justify-end gap-1.5">
      {entries.map(([k, v]) => {
        const { icon, name } = resourceVisual(k, cacheDir, resources, lang);
        return (
          <span key={k} className="inline-flex items-center gap-0.5" title={name || undefined}>
            {icon ? <HdIcon src={icon} alt={name || k} className="h-4 w-4 object-contain" /> : <span className="text-[11px] text-slate-400">{name || k}</span>}
            <span className="text-amber-300">{String(v)}</span>
          </span>
        );
      })}
    </span>
  );
}

/**
 * cost 对象的**纯文本**形式（`金 8000 + 宝石 4`），供 tooltip 用。
 *
 * `formatCost` 出的是带图标的节点，无法进 `title`；而"全价 vs 本阶价"的差额说明
 * 必须放在 tooltip 里（否则用户不知道 `8000` 为什么比生物自己的 `5000` 大）。
 */
export function costText(
  cost: unknown,
  cacheDir?: string | null,
  resources?: ResourceInfo[],
  lang = getVcmiLanguageSnapshot() ?? "english",
): string {
  if (!cost || typeof cost !== "object") return "";
  const entries = Object.entries(cost as Record<string, unknown>).filter(([, v]) => Number(v) > 0);
  if (entries.length === 0) return "";
  return entries
    .map(([k, v]) => {
      const { name } = resourceVisual(k, cacheDir ?? null, resources, lang);
      return `${name || k} ${v}`;
    })
    .join(" + ");
}

/** 单值资源显示（如宝物价格 = 金币数）：图标 + 数量（名称作 title；无图标时补显示名） */
export function formatResourceAmount(
  amount: unknown,
  key: string,
  cacheDir?: string | null,
  resources?: ResourceInfo[],
  lang = getVcmiLanguageSnapshot() ?? "english",
): ReactNode {
  if (amount === undefined || amount === null || amount === "") return undefined;
  const { icon, name } = resourceVisual(key, cacheDir, resources, lang);
  return (
    <span className="inline-flex items-center gap-1" title={name || undefined}>
      {icon ? <HdIcon src={icon} alt={name || key} className="h-4 w-4 object-contain" /> : null}
      <span>{String(amount)}</span>
      {!icon && name ? <span className="text-xs text-slate-400">{name}</span> : null}
    </span>
  );
}

/** 可点击的 wiki 关联链接（跳转到其他条目） */
export function WikiLink({
  modId,
  category,
  id,
  label,
  onNavigate,
  onContextMenu,
}: {
  modId: string;
  category: WikiLinkTarget["category"];
  id: string;
  label: React.ReactNode;
  onNavigate?: (target: WikiLinkTarget) => void;
  onContextMenu?: (e: React.MouseEvent, target: WikiLinkTarget) => void;
}) {
  if (!onNavigate && !onContextMenu) {
    return <span className="rounded bg-slate-800 px-2 py-0.5 text-[11px] text-slate-300">{label}</span>;
  }
  return (
    <button
      type="button"
      onClick={() => onNavigate?.({ modId, category, id })}
      onContextMenu={(e) => onContextMenu?.(e, { modId, category, id })}
      className="rounded bg-blue-500/15 px-2 py-0.5 text-[11px] text-blue-300 transition-colors hover:bg-blue-500/30 hover:text-blue-200"
    >
      {label}
    </button>
  );
}
