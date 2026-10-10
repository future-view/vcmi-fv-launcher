import { useState, type ReactNode } from "react";
import type { LangDict } from "@/i18n/types";
import { imgCandidates } from "@/lib/hd";
import type { WikiEntry, WikiLinkTarget } from "@/lib/modcache";
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
