/**
 * 百科中栏的**排序**。
 *
 * 与 `filters.ts` 的筛选正交：筛选是**收窄**（只看一个阵营），排序是**分组**
 * （全部条目按阵营分段展示）——所以「按等级排序」不是「按等级筛选」的重复：
 * 筛选只能看单个等级，排序能看到 T1(16 城) → T2(16 城) → … 的横向对比。
 *
 * ## 双槽模型
 *
 * - **主排序**（必选）：`name` / `faction` / 类目专属维度（生物 `level`、英雄 `class`、建筑 `bid`）
 * - **次排序**（可选）：仅当主排序不是 `name` 时出现，从剩余维度里选
 *
 * 主排序为 `name` 时不设次排序：单键排序没有"组内"可言，给了也是空操作。
 *
 * ## 无值处理（统一规则）
 *
 * 任一维度取不到值（无城镇 / 无等级 / 无 bid）→ **排在最后**，不参与分组。
 * 当前安装实测：生物无城镇 2 条、英雄无城镇 26 条、建筑无城镇 0 条（防御性规则）。
 * 等级是数值维度，`0`（战争机器 / 中立生物）天然排在最前，符合游戏内 T0 的心智。
 *
 * ## 持久化
 *
 * 按类目各记一组 `(primary, secondary)`（`uiPrefs` 的 `wikiSort` 键）。
 * 之所以不全局单记：维度池随类目变化（生物有等级、英雄有职业、建筑有类型），
 * 全局记会频繁遇到"在生物选的等级切到英雄不存在"，按类目记才与真实需求对齐。
 */
import type { LangDict } from "@/i18n/types";
import { pickName } from "@/lib/modcache";
import type { WikiCategory } from "@/lib/modcache";
import { artifactClassLabel } from "./filters";

/**
 * 排序项：条目 id + 原始 data。
 *
 * 刻意不绑 `SortItem`：`ModDetail` 排的是模组缓存里的**裸 obj**（没有
 * `category` / `modId` / `name` 等派生字段），包一层就能共用同一套排序。
 * `SortItem` 结构上兼容（有 `id` 与 `data`），可直接传入。
 */
export interface SortItem {
  id: string;
  data: Record<string, unknown>;
}

/**
 * VCMI 原版城镇固定序（H3 玩家的记忆顺序，**不是字典序**）。
 *
 * 用字典序会把「堡垒」排到「墓园」前面，打破肌肉记忆；原版序对齐游戏内
 * 城镇选择界面与设定集的排列。
 */
export const NATIVE_FACTION_ORDER = [
  "castle",
  "rampart",
  "necropolis",
  "tower",
  "inferno",
  "dungeon",
  "fortress",
  "conflux",
];

/**
 * 排序维度标识。
 *
 * ⚠️ `class`（英雄职业）与 `artifactClass`（宝物等级）**刻意分开**：两者虽然都读
 * `data.class`，但取值域完全不同（职业 id `knight` vs 等级 `TREASURE`），
 * 一个无固定序按名称、一个有固定序按档次 —— 合并会让其中一类排错。
 *
 * ⚠️ `level`（生物等级）与 `spellLevel`（法术等级）同理分开：生物的 `0` 是**战争机器**，
 * 是有效值、应排最前；法术的 `0` 在实测里占 210/273 条且全是 `core:*Trigger` 类
 * （投石 / 护城河触发器，没有等级概念），应按「无值」排最后。
 */
export type SortDim =
  | "name"
  | "faction"
  | "level"
  | "class"
  | "bid"
  | "artifactClass"
  | "spellLevel";

/** 一组排序配置（主 + 次） */
export interface SortSpec {
  primary: SortDim;
  secondary: SortDim;
}

/**
 * 各类目的默认排序。只有三类有城镇维度，其余类目恒为名称序。
 *
 * 默认值即"最常用的视角"：三类都是**城镇分组**；组内次键按类目最有信息量的维度取 ——
 * 生物用等级（兵种列表习惯）、英雄用职业、建筑用名称（bid 粒度太细，见下）。
 */
export const DEFAULT_SORT: Record<WikiCategory, SortSpec> = {
  factions: { primary: "name", secondary: "name" },
  creatures: { primary: "faction", secondary: "level" },
  heroes: { primary: "faction", secondary: "class" },
  buildings: { primary: "faction", secondary: "name" },
  // 宝物 / 法术：只有单值维度可用来分组（装备位 / 学派是多值数组，作排序键会误导，
  // 见 `spellLevel` 与 `slots` 相关注释），故组内次键固定为名称 —— 子行只有一项时
  // UI 不渲染（见 `WikiFilterPanel` 的 `secondaryDims(...).length > 1` 判断）。
  artifacts: { primary: "artifactClass", secondary: "name" },
  spells: { primary: "spellLevel", secondary: "name" },
  skills: { primary: "name", secondary: "name" },
  terrains: { primary: "name", secondary: "name" },
  glossary: { primary: "name", secondary: "name" },
  mods: { primary: "name", secondary: "name" },
};

/**
 * 各类目可选的**主排序**维度。`name` 恒在表内（表示「不分组」），UI 渲染时会被过滤掉、
 * 改作独立的「不分组（按名称）」项。
 *
 * 空数组 / 只有 `name` = 该类目不提供排序切换（浮层隐藏排序区块）。
 */
export const PRIMARY_DIMS: Record<WikiCategory, SortDim[]> = {
  factions: ["name"],
  creatures: ["name", "faction", "level"],
  heroes: ["name", "faction", "class"],
  buildings: ["name", "faction", "bid"],
  artifacts: ["name", "artifactClass"],
  spells: ["name", "spellLevel"],
  skills: ["name"],
  terrains: ["name"],
  glossary: ["name"],
  mods: ["name"],
};

/** 是否提供排序切换（有**非 name** 的分组维度才显示排序区块） */
export function hasSortSwitch(cat: WikiCategory): boolean {
  return (PRIMARY_DIMS[cat] ?? ["name"]).some((d) => d !== "name");
}

/** 次排序可选项：主排序用掉的维度不再出现（「城镇 + 城镇」是空操作） */
export function secondaryDims(cat: WikiCategory, primary: SortDim): SortDim[] {
  return (PRIMARY_DIMS[cat] ?? ["name"]).filter((d) => d !== primary);
}

/** 规范化：裁剪掉不在该类目维度池里的值，避免坏数据（旧版本 / 手改 localStorage）导致排序失效 */
export function normalizeSort(cat: WikiCategory, raw: Partial<SortSpec> | undefined): SortSpec {
  const pool = PRIMARY_DIMS[cat] ?? ["name"];
  const fallback = DEFAULT_SORT[cat] ?? { primary: "name", secondary: "name" };
  const pick = (v: string | undefined, dflt: SortDim): SortDim =>
    (pool as string[]).includes(v ?? "") ? (v as SortDim) : dflt;
  const primary = pick(raw?.primary, fallback.primary);
  // 主排序为名称时没有"组内"可言，次排序强制同名
  if (primary === "name") return { primary, secondary: "name" };
  //
  // 次排序必须合法且**不能与主排序相同**（「城镇 + 城镇」是空操作，UI 上会出现
  // "两个按钮都不高亮"的状态真空 —— 用户看不懂当前到底怎么排）。
  // 旧值不可用时退到「城镇」：三类共有的分组维度，也是最有用的组内视角
  // （「按等级 + 按城镇」= 跨城镇看同等级、组内按城镇归拢）。
  let secondary = pick(raw?.secondary, fallback.secondary);
  if (secondary === primary) {
    secondary = (pool as SortDim[]).includes("faction")
      ? "faction"
      : (pool as SortDim[]).find((d) => d !== primary && d !== "name") ?? "name";
  }
  return { primary, secondary };
}

/* ------------------------------ 维度定义 ------------------------------ */

/** 排序上下文：多语言取词 + 各类 id→名映射 */
export interface SortContext {
  lang: string;
  t: LangDict;
  /** 阵营 id → 多语言名（cache.json 顶层 `factionNames`，跨 mod 合并） */
  factionNames: Record<string, Record<string, string>>;
  /** 职业 id → 多语言名（英雄条目自带的 `className`，装载时收集） */
  classNames: Record<string, Record<string, string>>;
  /** 建筑 bid → 多语言名（从当前列表推导：同 bid 任取一条的建筑名） */
  bidNames: Record<string, Record<string, string>>;
}

interface DimDef {
  /** 取排序键；空串 = 该维度不适用（排最后） */
  keyOf: (e: SortItem, ctx: SortContext) => string;
  /** 固定序（原版城镇）；命中不了的归入"按显示名"的尾组 */
  order?: readonly string[];
  /** 数值维度（等级）：直接比大小，不走 Collator */
  numeric?: boolean;
  /** 显示名（下拉选项标签 / 兜底排序键） */
  labelOf: (key: string, ctx: SortContext) => string;
}

/** 条目 data 的字段值（统一转字符串，空值为 ""） */
function fieldOf(e: SortItem, field: string): string {
  const v = (e.data as Record<string, unknown>)[field];
  return v === undefined || v === null ? "" : String(v);
}

/** 名称：当前语言 → english → id（与列表显示同口径，避免英文界面排序键全空） */
function nameKeyOf(e: SortItem, ctx: SortContext): string {
  return pickName((e.data as Record<string, unknown>).name as Record<string, string> | undefined, ctx.lang) || e.id;
}

const DIMS: Record<SortDim, DimDef> = {
  name: {
    keyOf: nameKeyOf,
    labelOf: (k) => k,
  },
  faction: {
    // 字段名按类目不同：heroes/buildings → `factionId`，creatures → `faction`
    keyOf: (e) => fieldOf(e, "factionId") || fieldOf(e, "faction"),
    order: NATIVE_FACTION_ORDER,
    labelOf: (k, ctx) => pickName(ctx.factionNames[k], ctx.lang) || k,
  },
  level: {
    keyOf: (e) => fieldOf(e, "level"),
    numeric: true,
    labelOf: (k) => k,
  },
  class: {
    keyOf: (e) => fieldOf(e, "class"),
    labelOf: (k, ctx) => pickName(ctx.classNames[k], ctx.lang) || k,
  },
  bid: {
    keyOf: (e) => fieldOf(e, "bid"),
    numeric: true,
    labelOf: (k, ctx) => pickName(ctx.bidNames[k], ctx.lang) || `#${k}`,
  },
  /**
   * 宝物等级。与英雄的 `class` 分开：这里有**固定序**（宝藏→小型→大型→圣器→特殊），
   * 且标签来自 `artifactClassLabel`（同时吃下数字 `0-4` 与字符串两种写法）。
   */
  artifactClass: {
    keyOf: (e) => fieldOf(e, "class"),
    order: ["TREASURE", "MINOR", "MAJOR", "RELIC", "SPECIAL"],
    labelOf: (k, ctx) => artifactClassLabel(k, ctx.t) || k,
  },
  /**
   * 法术等级。与生物的 `level` 分开：`0` 在这里表示**无等级**（实测 210/273 条
   * `core:*Trigger` 类 —— 投石 / 护城河触发器，没有等级概念），按「无值」处理排最后；
   * 若按数值排，这 210 条会顶到最前，把真正 1-5 级的 63 条战斗法术全挤到后面。
   */
  spellLevel: {
    keyOf: (e) => {
      const v = fieldOf(e, "level");
      return v === "" || v === "0" ? "" : v;
    },
    numeric: true,
    labelOf: (k) => k,
  },
};

/* ------------------------------ 比较器 ------------------------------ */

/**
 * 单维度比较。规则：
 * 1. 双方都无值 → 相等（交给下一键）；
 * 2. 一方无值 → **排最后**；
 * 3. 数值维度 → 比大小（等级 0 天然最前）；
 * 4. 有固定序 → 按固定序；不在固定序里的（自定义城镇）按显示名归入尾组；
 * 5. 其余 → 按显示名（调用方传入的 Collator）。
 */
function compareDim(
  a: SortItem,
  b: SortItem,
  dim: SortDim,
  ctx: SortContext,
  collator: Intl.Collator,
): number {
  const def = DIMS[dim];
  const ka = def.keyOf(a, ctx);
  const kb = def.keyOf(b, ctx);
  if (!ka && !kb) return 0;
  if (!ka) return 1;
  if (!kb) return -1;
  if (def.numeric) {
    const na = Number(ka);
    const nb = Number(kb);
    if (Number.isFinite(na) && Number.isFinite(nb)) return na - nb;
  }
  if (def.order) {
    const ia = def.order.indexOf(ka);
    const ib = def.order.indexOf(kb);
    // 未登记的值（自定义城镇）统一归到尾组，组内按显示名
    const ra = ia < 0 ? Number.MAX_SAFE_INTEGER : ia;
    const rb = ib < 0 ? Number.MAX_SAFE_INTEGER : ib;
    if (ra !== rb) return ra - rb;
    if (ia < 0 && ib < 0) return collator.compare(def.labelOf(ka, ctx), def.labelOf(kb, ctx));
    return 0;
  }
  return collator.compare(def.labelOf(ka, ctx), def.labelOf(kb, ctx));
}

/**
 * 按 `spec` 排序（**原地**）。主键相等时才用次键；次键仍相等则保持相对稳定
 * （`Array.prototype.sort` 在现代引擎里稳定，无需再加 id 决胜）。
 */
export function sortEntries(
  list: SortItem[],
  spec: SortSpec,
  ctx: SortContext,
  collator: Intl.Collator,
): void {
  const { primary, secondary } = spec;
  list.sort((a, b) => {
    const c = compareDim(a, b, primary, ctx, collator);
    if (c !== 0 || primary === "name" || secondary === primary) return c;
    return compareDim(a, b, secondary, ctx, collator);
  });
}

/**
 * 从当前列表推导 **建筑 bid → 多语言名**（同 bid 任取一条的建筑名）。
 *
 * 建筑条目不带 bid 的显示名，而下拉选项需要可读标签（`#12` 对用户无意义）。
 * 取当前语言优先；同一 bid 的多条建筑名理应一致（都是"魔法行会1"之类），
 * 本地化差异取先到者。
 */
export function collectBidNames(list: SortItem[]): Record<string, Record<string, string>> {
  const out: Record<string, Record<string, string>> = {};
  for (const e of list) {
    const bid = fieldOf(e, "bid");
    if (!bid) continue;
    const nm = (e.data as Record<string, unknown>).name;
    if (nm && typeof nm === "object" && !out[bid]) out[bid] = nm as Record<string, string>;
  }
  return out;
}

/** 职业 id → 多语言名（英雄条目自带的 `className`） */
export function collectClassNames(
  list: SortItem[],
): Record<string, Record<string, string>> {
  const out: Record<string, Record<string, string>> = {};
  for (const e of list) {
    const c = fieldOf(e, "class");
    const n = (e.data as Record<string, unknown>).className;
    if (c && n && typeof n === "object" && !out[c]) out[c] = n as Record<string, string>;
  }
  return out;
}

/** 维度标签（浮层里 radio 与子行共用）。宝物/法术等级复用筛选维度已有的翻译键。 */
export function dimLabel(dim: SortDim, t: LangDict): string {
  const s = t.wiki.sort;
  switch (dim) {
    case "name":
      return s.byName;
    case "faction":
      return s.byFaction;
    case "level":
      return s.byLevel;
    case "class":
      return s.byClass;
    case "bid":
      return s.byType;
    case "artifactClass":
      return t.wiki.filter.byArtifactClass;
    case "spellLevel":
      return t.wiki.filter.bySpellLevel;
  }
}
