/**
 * 阵营巡礼纯逻辑层（设计 docs/features/faction-showcase.zh-CN.md）。
 *
 * 禁 import react（仅纯 TS）；供 `RealmEditor` 配置与 `ShowcaseOverlay` 播放共用。
 */

import { readTownCards, readWikiIndex, type ShowcaseEntry, type WikiReadResult } from "@/lib/tauri";
import { modIdKey, pickName, type WikiIndex } from "@/lib/modcache";
import type { Realm, RealmShowcase, ShowcaseDeckEntry } from "@/lib/realm/types";

/** 节奏默认值（§6.3 / §7.4 / §7.6，与文档钉死值一致） */
export const FACTION_DURATION_MS = 26000;
/**
 * 焦点切换间隔默认值。
 * ⚠️ 2026-09-21 由 3200 改 **3000**（拍板：默认取整秒）。步进仍是 200ms，
 * 作者手调过的值不受影响（只看配置里有没有 `focusStepMs`）。
 */
export const FOCUS_STEP_MS = 3000;
export const HERO_ROTATION_MS = 6000;
export const MAX_FACTION_DURATION_MS = 45000;
export const MUSIC_FADE_MS = 800;
/** 自动切到下一阵营前，提前多久开始降低音量（0 = 不提前淡出） */
export const MUSIC_PRE_FADE_MS = 2000;
export const DEFAULT_MAX_SLIDES = 12;
/** 兵种行的格子上限（§5.3：按 level 去重后取前 8 格 + 「+N」尾标）。播放器与编辑器估算共用 */
export const MAX_CREATURE_SLOTS = 8;
export const DEFAULT_HERO_COUNT = 2;

/**
 * 候选城镇 id（编辑器「可添加阵营」与运行时「自动牌组」**共用同一口径**）。
 *
 * 三条规则：
 * 1. 取 wiki 索引里 `type === "Town"` 的 mod（`vcmi` 伪模组除外）；
 * 2. **非**原版城镇：按「本领域启用集合」过滤；
 * 3. 原版 9 城镇 `core.*` **不在启用集合里**（引擎自带），只由 `includeCore` 单独控制，
 *    因此必须**旁路**启用过滤，否则该开关永远不生效。
 *
 * ⚠️ `enabledIds` 的三态语义（**空集合 ≠ 未知**）：
 * - `null` / `undefined` = **读不到**启用组合（如未激活领域）→ 不过滤，退化为全量以保证可演示；
 * - `Set`（**即使是空集**）= 权威值 → 严格按它过滤。空集表示「本组合没启用任何第三方城镇」，
 *   结果就应当为空（只剩 `includeCore` 的原版），而不是把 96 个城镇全倒进来。
 *
 * ⚠️ 大小写：wiki 索引沿用磁盘原大小写，预设里可能是小写（如 `fairytale` vs `Fairytale`），
 * 必须按 `modIdKey` 小写比较，否则会漏掉已启用的城镇。
 */
export function resolveTownIds(
  wikiIndex: WikiReadResult | null,
  includeCore: boolean,
  enabledIds?: Set<string> | null,
): string[] {
  const mods = (wikiIndex?.data as WikiIndex | undefined)?.mods ?? [];
  const enabled = enabledIds ? new Set([...enabledIds].map(modIdKey)) : null;

  const towns = mods
    .filter((m) => m.type === "Town" && m.modId !== "vcmi")
    .map((m) => m.modId)
    .filter(Boolean);

  const kept = towns.filter((id) => {
    if (id.startsWith("core.")) return includeCore;
    return enabled ? enabled.has(modIdKey(id)) : true;
  });

  // 原版优先（与领域信息页一致），同组内按 id 排序。
  // ⚠️ 不依赖「wiki 索引顺序」：返回顺序会经 `buildShowcaseDeck` 的 `order` 映射 → `shuffleBySeed`
  //    洗牌 → 截断，若上游顺序漂移，同一 seed 下"展示哪几个城镇"就会变，
  //    破坏"编辑器预览 = 实际播放"。读侧自足排序，不假设任何外部顺序。
  return kept.sort(
    (a, b) => Number(b.startsWith("core.")) - Number(a.startsWith("core.")) || a.localeCompare(b),
  );
}

/** 自动生成 deck：候选城镇 → TownCard → 截断到 maxSlides。
 * 手工牌组优先（`mode` 为 manual 且牌组非空）。
 *
 * ⚠️ 无老配置兼容层：字段只有 `mode` / `auto.*` / `manual.deck` 这一种写法。
 *    旧的扁平 `deck` / `maxSlides` / `autoSeed` 已废弃（不再读取），
 *    升级后需要重配一次 —— 取舍是"配置结构单一、没有两套口径"优先于"免迁移"。
 */
export async function buildShowcaseDeck(
  realm: Realm | null,
  wikiIndex: WikiReadResult | null,
  modPathMap: Map<string, string>,
  enabledIds?: Set<string> | null,
  /**
   * **种子覆盖**（"每次进入巡礼都换一批"用）：给了就以它洗牌，**不落盘**。
   * 未给 → 走 `cfg.auto.autoSeed` → 再退回 `fallbackSeed(realm.id)`。
   */
  seedOverride?: number | null,
): Promise<ShowcaseDeckEntry[]> {
  const cfg: RealmShowcase | undefined = realm?.showcase;

  // 手写牌组优先（作者已明确 deck 项）。
  // ⚠️ `realm.showcase` 可能整体缺省（仓库安装的旧 schema 领域，如 vcmi-realm 就没有该字段）
  // —— 此前直接 `cfg.manual` 会在老领域上抛 TypeError，巡礼永远卡在 loading。
  const deck = cfg?.manual?.deck;
  if (cfg?.mode === "manual" && deck && deck.length > 0) {
    const list = deck.map((d) => ({ ...d }));
    // ⚠️ 「包含原版 9 城镇」是**公共选项**，手工牌组同样受它约束：
    //    关闭时 `core.*` 条目不参与（条目本身保留在配置里，开关一开即恢复）。
    return cfg.includeCore === true
      ? list
      : list.filter((d) => !d.modId.startsWith("core."));
  }

  const townIds = resolveTownIds(wikiIndex, cfg?.includeCore === true, enabledIds);
  const cards = await readTownCards(townIds);
  const order = new Map(townIds.map((id, i) => [modIdKey(id), i] as const));
  const entries: ShowcaseDeckEntry[] = cards
    .filter((c) => c.modId && c.factionId)
    .map((c) => ({ modId: c.modId, factionId: c.factionId }));
  entries.sort(
    (a, b) => (order.get(modIdKey(a.modId)) ?? 0) - (order.get(modIdKey(b.modId)) ?? 0),
  );

  // ⚠️ 自动专属字段一律走 `cfg.auto`（老配置的扁平字段已由 `normalizeShowcase` 归一）
  const maxSlides = cfg?.auto?.maxSlides;
  const autoSeed = cfg?.auto?.autoSeed;

  // 自动编排 → **随机顺序**。
  // ⚠️ 必须**先洗牌再截断**：先截断的话每次只会在这前 N 个里换顺序，"展示哪 N 个"就不随机了。
  // 种子与编辑器预览共用（见 `shuffleBySeed` 注释），保证"预览 = 实播"。
  const shuffled = shuffleBySeed(
    entries,
    seedOverride ?? autoSeed ?? fallbackSeed(realm?.id ?? ""),
  );
  const max = maxSlides ?? DEFAULT_MAX_SLIDES;
  return max > 0 ? shuffled.slice(0, max) : shuffled;
}

/**
 * 兵种所属**巢穴建筑**的候选 id（§5.6）。
 *
 * ⚠️ 真值来源：`src-tauri/data/h3-core/chinese.buildings.json`
 * ```
 * "necropolis.dwellingLvl1"   → 邪庙      （招募骷髅兵）
 * "necropolis.dwellingUpLvl1" → 高级邪庙  （招募骷髅勇士）
 * ```
 * 即 VCMI 的命名是 **前缀 `dwellingUpLvl<N>`**，**不是** `dwellingLvl<N>up`。
 * 之前按 `dwellingLvl<N>up` 找 → 找不到 →回退到基础巢穴 →
 * 表现就是"选了骷髅勇士却联动到邪庙"。
 * 保留 `up`/`b` 后缀两种写法兜底第三方模组，最后才回退基础巢穴。
 */
export function dwellingIdCandidates(level: number, upgraded: boolean): string[] {
  return upgraded
    ? [`dwellingUpLvl${level}`, `dwellingLvl${level}up`, `dwellingLvl${level}b`, `dwellingLvl${level}`]
    : [`dwellingLvl${level}`];
}

/** 解析巢穴所需的最小生物形状（`ShowcaseSlide` 的 creatures 项结构兼容） */
export type DwellingCreature = {
  id: string;
  level?: number | null;
  upgradedFrom?: string[];
  /** 缓存带来的巢穴建筑名（多语言 → 名称数组），见 `tauri.ts` 的同名字段 */
  dwellingNames?: Record<string, string[]>;
};

/** 解析巢穴所需的最小建筑形状 */
export type DwellingBuilding = { id: string; name?: Record<string, string> | string | null };

/**
 * 解析**一个兵种的招募建筑**（返回建筑 id，找不到给 null）。
 *
 * 顺序（**名称优先**）：
 * 1. **名称匹配**：`creature.dwellingNames[lang]` 是缓存里"城镇配置 `building.creatures` 反查"
 *    出的巢穴名 —— 与 id 命名约定无关，第三方模组也准；
 * 2. **id 候选**：`dwellingUpLvl<N>` → `dwellingLvl<N>up` → `dwellingLvl<N>b` → `dwellingLvl<N>`，
 *    先精确匹配，再允许模块前缀（`castle.dwellingUpLvl2`）。
 *
 * ⚠️ **播放器与编辑器必须共用本函数**：从前播放器侧压根没有这段逻辑（建筑巡礼恒为空），
 * 编辑器又只按 id 猜（第三方模组常猜空 → "选了兵种但没有招募建筑"）。
 */
export function resolveDwellingId(
  buildings: DwellingBuilding[],
  creature: DwellingCreature,
  lang?: string,
): string | null {
  const names = (lang ? creature.dwellingNames?.[lang] : undefined) ?? [];
  if (names.length > 0) {
    const hit = buildings.find((b) => {
      // 建筑名可能是多语言对象，也可能已是字符串（第三方模组偶见）
      const nm = typeof b.name === "string" ? b.name : pickName(b.name ?? undefined, lang);
      return !!nm && names.includes(nm);
    });
    if (hit) return hit.id;
  }
  const ids = creature.level != null ? dwellingIdCandidates(creature.level, !!(creature.upgradedFrom?.length)) : [];
  for (const id of ids) {
    const hit =
      buildings.find((b) => b.id === id) ?? buildings.find((b) => b.id.endsWith(`.${id}`));
    if (hit) return hit.id;
  }
  return null;
}

/**
 * 按**给定兵种顺序**解析巢穴建筑 id 列表（去重、保序）—— 顺序即"兵种巡礼顺序"，两者一一对应。
 * 播放器的建筑巡礼清单、编辑器的自动置顶与停留估算**全部**走它（同源才不会各说各话）。
 */
export function resolveDwellingIds(
  buildings: DwellingBuilding[],
  creatures: DwellingCreature[],
  lang?: string,
): string[] {
  const out: string[] = [];
  for (const c of creatures) {
    const id = resolveDwellingId(buildings, c, lang);
    if (id && !out.includes(id)) out.push(id);
  }
  return out;
}

/**
 * 按**给定兵种顺序**解析出巢穴建筑 id（去重、保序）。
 *
 * 编辑器「巡礼建筑」的自动项与播放器的联动都走这个函数 —— 两边同源，才能保证
 * "编辑器里看到的置顶顺序 = 播放时的联动顺序"。
 */
export function dwellingsOfCreatures(
  buildings: { id: string }[],
  creatures: { id: string; level?: number | null; upgradedFrom?: string[] }[],
): string[] {
  const has = new Set(buildings.map((b) => b.id));
  const out: string[] = [];
  for (const c of creatures) {
    if (c.level == null) continue;
    const hit = dwellingIdCandidates(c.level, !!(c.upgradedFrom?.length)).find((id) => has.has(id));
    if (hit && !out.includes(hit)) out.push(hit);
  }
  return out;
}

/**
 * 单张阵营的**实际停留时长**（**编辑器估算与播放器调度共用同一函数**）。
 *
 * 为什么必须共用：停留时长是"作者在编辑器里看到的数字"与"播放器真正切走的时刻"，
 * 两处各算一遍必然漂移（历史上就出现过：注释写 110、实算 117.5 的同类问题）。
 *
 * 口径（与播放器调度器逐条对应）：
 * ```
 * cycleMs = 焦点格数 × 焦点间隔            // 「轮完一轮」的天然节奏
 * autoMs  = untilFocus && cycleMs > 0 ? cycleMs : 默认 26s
 * stayMs  = durationMs（每张覆盖，界面未暴露） ?? autoMs
 * ```
 * ⚠️ **不再有「最大停留上限」**（2026-09-21 拍板删除）：停留时长本来就是从巡礼清单
 * 推算出来的（兵种 + 建筑 + 英雄 的格数 × 间隔），"防止单张过长"的保险丝没有意义 ——
 * 真要短就减格子或调间隔，那些都是**作者可见的**数字。
 */
export function estimateStayMs(o: {
  /** 参与焦点的格数（兵种 + 入序列的建筑 + 英雄） */
  focusSlots: number;
  focusStepMs: number;
  /** 「轮完一轮才切」（默认 true） */
  untilFocus?: boolean;
  /** 每张覆盖（进阶用法，界面未暴露） */
  durationMs?: number;
}): { stayMs: number; cycleMs: number; autoMs: number } {
  const cycleMs = Math.max(0, o.focusSlots) * Math.max(1, o.focusStepMs);
  const untilFocus = o.untilFocus ?? true;
  const autoMs = untilFocus && cycleMs > 0 ? cycleMs : FACTION_DURATION_MS;
  const stayMs = o.durationMs ?? autoMs;
  return { stayMs, cycleMs, autoMs };
}

/** 把 deck 项转成后端请求项（补 modPath 供音乐解析） */
export function deckToEntries(
  deck: ShowcaseDeckEntry[],
  modPathMap: Map<string, string>,
): ShowcaseEntry[] {
  return deck.map((d) => ({
    modId: d.modId,
    factionId: d.factionId,
    modPath: modPathMap.get(d.modId),
  }));
}

/**
 * 职业分组键：`className` 在缓存里是 VCMI 多语言对象（与 name 同构），
 * 个别模组可能是纯字符串 —— 统一归一成可比较的字符串。
 */
export function heroClassKey(v: unknown): string {
  if (typeof v === "string") return v;
  if (v && typeof v === "object") return JSON.stringify(v);
  return "";
}

/**
 * 英雄的**默认巡礼清单**：每个职业各取 1 个（`2026-09-21` 拍板，取代旧的"随机轮换池"）。
 *
 * 为什么这样定：
 * - 旧模型的"随机洗牌 + 按间隔轮换"是纯粹的视觉噪音（用户看不到规律、也无法挑选）；
 * - 现在英雄是巡礼序列里的**第三个分类**（兵种 → 建筑 → 英雄），清单由作者勾选，
 *   没勾时用这个默认值：**一个职业一个**，既保证"两类英雄都露脸"，又让时长可预期
 *   （典型阵营 2 个英雄 ≈ 6.4s）；
 * - 顺序 = 职业在缓存里首次出现的顺序 → **确定性**（同一阵营每次一致，不用种子）。
 */
export function defaultHeroIds<T extends { id: string; className?: unknown }>(heroes: T[]): T[] {
  const seen = new Set<string>();
  const out: T[] = [];
  for (const h of heroes) {
    const k = heroClassKey(h.className);
    if (seen.has(k)) continue;
    seen.add(k);
    out.push(h);
  }
  return out;
}

/**
 * 自动编排的**随机顺序**用的一组工具。
 *
 * 关键点：必须是**可复现**的随机 —— 编辑器预览与运行时播放共用同一个种子，
 * 否则"编辑器里看到的顺序"和"实际播出来的顺序"不一致（预览就失去意义）。
 *
 * 种子来源优先级：`cfg.auto.autoSeed`（显式随机过）→ `fallbackSeed(realm.id)`（未随机过的兜底，
 * 同样是"随机但稳定"）。重新排列 = 写入一个新的 `autoSeed`。
 */

/** 生成新的随机种子（重新排列时用） */
export function randomSeed(): number {
  return Math.floor(Math.random() * 0xffffffff) + 1;
}

/** 缺省种子：由领域 id 派生，保证"未随机过的领域"也有稳定顺序 */
export function fallbackSeed(realmId: string): number {
  let h = 2166136261;
  for (let i = 0; i < realmId.length; i++) {
    h ^= realmId.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return (Math.abs(h) % 0x7fffffff) || 1;
}

/** 稳定洗牌（公开版）：同一 seed + 同一输入 → 同一结果 */
export function shuffleBySeed<T>(arr: T[], seed: number): T[] {
  return stableShuffle(arr, `s${seed}`);
}

/** 稳定洗牌：同一 seed + 同一输入 → 同一结果（避免每帧重排导致顺序跳动）。 */
function stableShuffle<T>(arr: T[], seed?: string): T[] {
  let h = 2166136261;
  const s = seed ?? "";
  for (let i = 0; i < s.length; i++) {
    h ^= s.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  const out = [...arr];
  for (let i = out.length - 1; i > 0; i--) {
    h ^= h << 13;
    h ^= h >>> 17;
    h ^= h << 5;
    const j = Math.abs(h) % (i + 1);
    [out[i], out[j]] = [out[j], out[i]];
  }
  return out;
}
