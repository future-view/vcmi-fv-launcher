/**
 * 百科中栏「筛选」维度的声明表（+ 域内共用的展示工具）。
 *
 * 设计目标：**新增一个筛选维度 = 往表里加一项**，UI 与布局零改动。
 * 每个维度只描述三件事 —— 取哪个字段、选项从哪来、怎么匹配；
 * 控件形态（chip 行 / 可搜索列表）由选项数量自动决定（见 `WikiFilterPanel`）。
 *
 * 布局约定：筛选控件一律放在工具槽弹出的浮层内，**列表上方不新增任何行** ——
 * 否则切换类目（各分类维度数不同，0~5 个）或启用筛选时，列表上沿会移动。
 */
import { modIdKey, pickName } from "@/lib/modcache";
import type { WikiCategory, WikiEntry } from "@/lib/modcache";
import type { LangDict } from "@/i18n/types";

/** 一个筛选选项 */
export interface WikiFilterOption {
  value: string;
  label: string;
  count: number;
}

/** 选项生成所需的上下文（多语言取词 + 各类 id→名映射） */
export interface WikiFilterContext {
  contentLang: string;
  t: LangDict;
  /** 阵营 id → 多语言名（跨 mod 合并，来自 cache.json 顶层 `factionNames`） */
  factionNames: Record<string, Record<string, string>>;
  /** modId（**小写归一**，同 `modIdKey`）→ 多语言名（来自 index.json 的条目名） */
  modNames: Record<string, Record<string, string>>;
}

/** 一个筛选维度的声明 */
export interface WikiFilterSpec {
  /** 维度键：同一类目内唯一，同时用作筛选状态的 key */
  key: string;
  /** 浮层里的分区标题 */
  label: (t: LangDict) => string;
  /** 选项列表；入参是**未经筛选**的该类目全量条目（计数才反映真实分布） */
  options: (entries: WikiEntry[], ctx: WikiFilterContext) => WikiFilterOption[];
  /** 条目是否命中该值 */
  match: (e: WikiEntry, value: string) => boolean;
}

/** 阵营类型固定顺序（善/中立/恶），不随语言变化 */
const ALIGNMENT_ORDER = ["good", "neutral", "evil"];
/** 法术学派固定顺序（土/气/火/水，对齐 VCMI SpellSchool 枚举习惯） */
const SCHOOL_ORDER = ["earth", "air", "fire", "water"];
/** 宝物等级固定顺序（由低到高） */
const ARTIFACT_CLASS_ORDER = ["TREASURE", "MINOR", "MAJOR", "RELIC", "SPECIAL"];
/** 装备位固定顺序（头 → 肩 → 颈 → 躯干 → 手持 → 戒指 → 脚 → 杂物 → 战争机器 → 法术书） */
const SLOT_ORDER = [
  "HEAD",
  "SHOULDERS",
  "NECK",
  "TORSO",
  "RIGHT_HAND",
  "LEFT_HAND",
  "RIGHT_RING",
  "LEFT_RING",
  "FEET",
  "MISC",
  "WAR_MACHINE",
  "SPELLBOOK",
];

/* ------------------------------ 共用工具 ------------------------------ */

/** 按提取函数统计出现次数（跳过空值） */
function countBy(entries: WikiEntry[], pick: (e: WikiEntry) => string): Map<string, number> {
  const m = new Map<string, number>();
  for (const e of entries) {
    const v = pick(e);
    if (!v) continue;
    m.set(v, (m.get(v) ?? 0) + 1);
  }
  return m;
}

/** 条目 data 里某字段的字符串值（`isCombined` 这类布尔也会变成 "true" / "false"） */
function fieldOf(e: WikiEntry, field: string): string {
  const v = (e.data as Record<string, unknown>)[field];
  return v === undefined || v === null ? "" : String(v);
}

/** 多语言对象取词（与列表显示同口径：当前语言 → english） */
function textOf(v: unknown, lang: string): string {
  return pickName(v as Record<string, string> | undefined, lang);
}

/** 选项排序：按显示名（与列表排序同一把尺子） */
function byLabel(a: WikiFilterOption, b: WikiFilterOption): number {
  return a.label.localeCompare(b.label);
}

/**
 * 宝物等级标签：列表副标题与「宝物等级」筛选**共用一份实现**。
 *
 * 两种写法都要吃下：数字 `class`（0-4，H3 原生）与字符串（`TREASURE` / `MINOR` / …，
 * VCMI 现代写法）；字符串经 `class` + PascalCase 映射到 `t.wiki.artifact.class*`，
 * 未登记的等级回退原文。
 */
export function artifactClassLabel(raw: unknown, t: LangDict): string {
  const a = t.wiki.artifact;
  const s = String(raw ?? "");
  if (s === "") return a.classOther;
  const cnum = Number(raw);
  if (Number.isFinite(cnum)) {
    return (
      [a.classTreasure, a.classMinor, a.classMajor, a.classRelic, a.classScroll][cnum] ??
      a.classOther
    );
  }
  const key = "class" + s.charAt(0).toUpperCase() + s.slice(1).toLowerCase();
  return (a as unknown as Record<string, string>)[key] ?? s;
}

/**
 * 装备位归并：VCMI 用编号槽位表达「可放任一」（实测 `spellScroll` 的 `slots` 是
 * `["MISC5","MISC4","MISC3","MISC2","MISC1"]`）——直接展开会出现 5 个同义选项。
 * 故 `MISC1`~`MISC5` → 杂物、`MACH1`~`MACH4` → 战争机器，其余（`NECK`/`RIGHT_HAND`/…）原样。
 */
export function slotKeyOf(slot: string): string {
  const s = String(slot).toUpperCase();
  if (s.startsWith("MISC")) return "MISC";
  if (s.startsWith("MACH")) return "WAR_MACHINE";
  return s;
}

/**
 * 装备位标签：`RIGHT_HAND` → `slotRightHand`、`MISC` → `slotMisc`、`WAR_MACHINE` → `slotWarMachine`。
 * `t.wiki.artifact.slot*` 已覆盖全部槽位（含 warMachine / spellbook），无需新增翻译键。
 */
function slotLabel(slot: string, t: LangDict): string {
  const key =
    "slot" +
    slot
      .split("_")
      .map((p) => p.charAt(0).toUpperCase() + p.slice(1).toLowerCase())
      .join("");
  return (t.wiki.artifact as unknown as Record<string, string>)[key] ?? slot;
}

/** 条目 data 里某数组字段（用于多值维度） */
function arrayOf(e: WikiEntry, field: string): Record<string, unknown>[] {
  const v = (e.data as Record<string, unknown>)[field];
  return Array.isArray(v) ? (v as Record<string, unknown>[]) : [];
}

/** 生物是否「升级形态」（由某基础生物升级而来） */
function isUpgraded(e: WikiEntry): boolean {
  const v = (e.data as Record<string, unknown>).upgradedFrom;
  return Array.isArray(v) && v.length > 0;
}

/* ------------------------------ 维度定义 ------------------------------ */

/** 阵营类型（`factions.alignment`）：faction schema 必填且枚举闭合，无空值 */
const alignment: WikiFilterSpec = {
  key: "alignment",
  label: (t) => t.wiki.filter.byAlignment,
  options: (entries, ctx) => {
    const counts = countBy(entries, (e) => fieldOf(e, "alignment"));
    return ALIGNMENT_ORDER.filter((v) => counts.has(v)).map((v) => ({
      value: v,
      label: ctx.t.wiki.alignment[v as keyof LangDict["wiki"]["alignment"]] ?? v,
      count: counts.get(v) ?? 0,
    }));
  },
  match: (e, v) => fieldOf(e, "alignment") === v,
};

/**
 * 阵营名称。字段名按类目不同：
 * `heroes` → `factionId`（由 heroClass.faction 推导，见 build/data/hero.rs）、
 * `creatures` → `faction`、`buildings` → `factionId`。
 */
function factionFilter(field: string): WikiFilterSpec {
  return {
    key: "faction",
    label: (t) => t.wiki.filter.byFaction,
    options: (entries, ctx) =>
      [...countBy(entries, (e) => fieldOf(e, field)).entries()]
        .map(([v, n]) => ({
          value: v,
          // 显示名取 cache.json 顶层 `factionNames`；缺失（mod 未提供阵营数据）时回退 id 本身
          label: textOf(ctx.factionNames[v], ctx.contentLang) || v,
          count: n,
        }))
        .sort(byLabel),
    match: (e, v) => fieldOf(e, field) === v,
  };
}

/**
 * 英雄职业（`heroes.class`）。取 18 种职业，标签用条目自带的 `className`
 * （多语言对象，含 mod 职业的内嵌名），与列表副标题同源。
 */
const heroClass: WikiFilterSpec = {
  key: "class",
  label: (t) => t.wiki.filter.byHeroClass,
  options: (entries, ctx) => {
    const counts = countBy(entries, (e) => fieldOf(e, "class"));
    const names = new Map<string, Record<string, string>>();
    for (const e of entries) {
      const id = fieldOf(e, "class");
      const n = (e.data as Record<string, unknown>).className;
      if (id && !names.has(id) && n && typeof n === "object") {
        names.set(id, n as Record<string, string>);
      }
    }
    return [...counts.entries()]
      .map(([v, n]) => ({ value: v, label: textOf(names.get(v), ctx.contentLang) || v, count: n }))
      .sort(byLabel);
  },
  match: (e, v) => fieldOf(e, "class") === v,
};

/** 生物级别：0 = 战争机器，1-7 常规（对齐列表副标题 `T<n>`），更高为 mod 自定等级 */
const creatureLevel: WikiFilterSpec = {
  key: "level",
  label: (t) => t.wiki.filter.byCreatureLevel,
  options: (entries, ctx) => {
    const counts = countBy(entries, (e) => fieldOf(e, "level"));
    return [...counts.keys()]
      .map(Number)
      .filter((n) => Number.isFinite(n))
      .sort((a, b) => a - b)
      .map((n) => ({
        value: String(n),
        label: n === 0 ? ctx.t.wiki.filter.levelWarMachine : n <= 7 ? `T${n}` : String(n),
        count: counts.get(String(n)) ?? 0,
      }));
  },
  match: (e, v) => fieldOf(e, "level") === v,
};

/** 生物形态：基础形态 / 升级形态（由某生物升级而来） */
const creatureTier: WikiFilterSpec = {
  key: "upgraded",
  label: (t) => t.wiki.filter.byCreatureTier,
  options: (entries, ctx) => {
    const up = entries.filter(isUpgraded).length;
    return [
      { value: "base", label: ctx.t.wiki.filter.tierBase, count: entries.length - up },
      { value: "upgraded", label: ctx.t.wiki.filter.tierUpgraded, count: up },
    ];
  },
  match: (e, v) => (v === "upgraded") === isUpgraded(e),
};

/**
 * 生物能力。`abilities` 是数组（每项 `{type, name, description}`），**多值**：命中即拥有该能力。
 * 同一生物内的同 `type` 去重计数（实测平均 2.8 个能力，选项可达 60+）。
 */
const creatureAbility: WikiFilterSpec = {
  key: "ability",
  label: (t) => t.wiki.filter.byAbility,
  options: (entries, ctx) => {
    const counts = new Map<string, number>();
    const names = new Map<string, Record<string, string>>();
    for (const e of entries) {
      const abs = (e.data as Record<string, unknown>).abilities;
      if (!Array.isArray(abs)) continue;
      const seen = new Set<string>();
      for (const a of abs as Record<string, unknown>[]) {
        const ty = typeof a?.type === "string" ? a.type : "";
        if (!ty || seen.has(ty)) continue;
        seen.add(ty);
        counts.set(ty, (counts.get(ty) ?? 0) + 1);
        if (!names.has(ty) && a.name && typeof a.name === "object") {
          names.set(ty, a.name as Record<string, string>);
        }
      }
    }
    return [...counts.entries()]
      .map(([v, n]) => ({ value: v, label: textOf(names.get(v), ctx.contentLang) || v, count: n }))
      .sort(byLabel);
  },
  match: (e, v) => {
    const abs = (e.data as Record<string, unknown>).abilities;
    return Array.isArray(abs) && (abs as Record<string, unknown>[]).some((a) => a?.type === v);
  },
};

/** 法术等级 */
const spellLevel: WikiFilterSpec = {
  key: "level",
  label: (t) => t.wiki.filter.bySpellLevel,
  options: (entries) => {
    const counts = countBy(entries, (e) => fieldOf(e, "level"));
    return [...counts.keys()]
      .map(Number)
      .filter((n) => Number.isFinite(n))
      .sort((a, b) => a - b)
      .map((n) => ({ value: String(n), label: String(n), count: counts.get(String(n)) ?? 0 }));
  },
  match: (e, v) => fieldOf(e, "level") === v,
};

/**
 * 法术学派。`schools` 是**多值数组** —— 命中即「属于该学派」，因此「全系」法术
 * （实测原版有 2 条 `["earth","water","fire","air"]`）会同时出现在四个学派下。
 * 名称取条目自带的 `schoolNames`（与 `schools` 同下标、覆盖 25 种语言），不再维护静态翻译表。
 */
const spellSchool: WikiFilterSpec = {
  key: "schools",
  label: (t) => t.wiki.filter.bySchool,
  options: (entries, ctx) => {
    const counts = new Map<string, number>();
    const names = new Map<string, Record<string, string>>();
    for (const e of entries) {
      const d = e.data as Record<string, unknown>;
      const ids = Array.isArray(d.schools) ? (d.schools as string[]) : [];
      const nms = Array.isArray(d.schoolNames) ? (d.schoolNames as Record<string, string>[]) : [];
      ids.forEach((id, i) => {
        counts.set(id, (counts.get(id) ?? 0) + 1);
        if (!names.has(id) && nms[i]) names.set(id, nms[i]);
      });
    }
    const rest = [...counts.keys()].filter((k) => !SCHOOL_ORDER.includes(k)).sort();
    return [...SCHOOL_ORDER.filter((v) => counts.has(v)), ...rest].map((id) => ({
      value: id,
      label: textOf(names.get(id), ctx.contentLang) || id,
      count: counts.get(id) ?? 0,
    }));
  },
  match: (e, v) => {
    const ids = (e.data as Record<string, unknown>).schools;
    return Array.isArray(ids) && (ids as string[]).includes(v);
  },
};

/** 宝物等级（`artifacts.class`）：标签与列表副标题共用 `artifactClassLabel` */
const artifactClass: WikiFilterSpec = {
  key: "class",
  label: (t) => t.wiki.filter.byArtifactClass,
  options: (entries, ctx) => {
    const counts = countBy(entries, (e) => fieldOf(e, "class"));
    return ARTIFACT_CLASS_ORDER.filter((v) => counts.has(v)).map((v) => ({
      value: v,
      label: artifactClassLabel(v, ctx.t),
      count: counts.get(v) ?? 0,
    }));
  },
  match: (e, v) => fieldOf(e, "class") === v,
};

/** 是否组合宝物（`artifacts.isCombined`：该宝物本身由组件合成） */
const artifactCombined: WikiFilterSpec = {
  key: "isCombined",
  label: (t) => t.wiki.filter.byCombined,
  options: (entries, ctx) => {
    const counts = countBy(entries, (e) => fieldOf(e, "isCombined"));
    return [
      { value: "true", label: ctx.t.wiki.filter.combinedYes, count: counts.get("true") ?? 0 },
      { value: "false", label: ctx.t.wiki.filter.combinedNo, count: counts.get("false") ?? 0 },
    ];
  },
  match: (e, v) => fieldOf(e, "isCombined") === v,
};

/** 模组类型（`mods.type`）：标签复用 `t.mods.typeLabels` 的 VCMI modType 枚举 */
const modType: WikiFilterSpec = {
  key: "type",
  label: (t) => t.wiki.filter.byModType,
  options: (entries, ctx) =>
    [...countBy(entries, (e) => fieldOf(e, "type")).entries()]
      .map(([v, n]) => ({
        value: v,
        label: (ctx.t.mods.typeLabels as Record<string, string>)[v] ?? v,
        count: n,
      }))
      .sort(byLabel),
  match: (e, v) => fieldOf(e, "type") === v,
};

/**
 * 英雄副技能（`heroes.secondarySkills[].skill`）。**多值**：命中即「会该技能」。
 * 同一英雄内同一技能去重计数（实测覆盖 **156/156 = 100%**、28 种技能、291 个实例）。
 * 标签取条目自带的 `name` 多语言对象，与英雄详情页同源。
 */
const heroSkill: WikiFilterSpec = {
  key: "skill",
  label: (t) => t.wiki.filter.byHeroSkill,
  options: (entries, ctx) => {
    const counts = new Map<string, number>();
    const names = new Map<string, Record<string, string>>();
    for (const e of entries) {
      const seen = new Set<string>();
      for (const s of arrayOf(e, "secondarySkills")) {
        const id = typeof s.skill === "string" ? s.skill : "";
        if (!id || seen.has(id)) continue;
        seen.add(id);
        counts.set(id, (counts.get(id) ?? 0) + 1);
        if (!names.has(id) && s.name && typeof s.name === "object") {
          names.set(id, s.name as Record<string, string>);
        }
      }
    }
    return [...counts.entries()]
      .map(([v, n]) => ({ value: v, label: textOf(names.get(v), ctx.contentLang) || v, count: n }))
      .sort(byLabel);
  },
  match: (e, v) => arrayOf(e, "secondarySkills").some((s) => s.skill === v),
};

/**
 * 宝物装备位（`artifacts.slots`）。**多值**：命中即「可装备在该位」。
 * 编号槽位先经 `slotKeyOf` 归并（`MISC1-5` → 杂物、`MACH1-4` → 战争机器），
 * 故选项是归并后的 12 个，而不是原始 19 个。
 */
const artifactSlot: WikiFilterSpec = {
  key: "slot",
  label: (t) => t.wiki.filter.byArtifactSlot,
  options: (entries, ctx) => {
    const counts = new Map<string, number>();
    for (const e of entries) {
      const slots = (e.data as Record<string, unknown>).slots;
      if (!Array.isArray(slots)) continue;
      const seen = new Set<string>();
      for (const s of slots as string[]) {
        const k = slotKeyOf(s);
        if (seen.has(k)) continue;
        seen.add(k);
        counts.set(k, (counts.get(k) ?? 0) + 1);
      }
    }
    const rest = [...counts.keys()].filter((k) => !SLOT_ORDER.includes(k)).sort();
    return [...SLOT_ORDER.filter((v) => counts.has(v)), ...rest].map((k) => ({
      value: k,
      label: slotLabel(k, ctx.t),
      count: counts.get(k) ?? 0,
    }));
  },
  match: (e, v) => {
    const slots = (e.data as Record<string, unknown>).slots;
    return Array.isArray(slots) && (slots as string[]).some((s) => slotKeyOf(s) === v);
  },
};

/** 建筑是否为**招募建筑**（生物巢穴）：按 id 短名前缀判定 —— VCMI 建筑 id 跨阵营统一命名，无歧义 */
function isDwelling(e: WikiEntry): boolean {
  const id = String((e.data as Record<string, unknown>).id ?? "");
  const short = id.includes(".") ? id.slice(id.indexOf(".") + 1) : id;
  return /^dwelling/i.test(short);
}

/**
 * 建筑分类：是否为招募建筑（生物巢穴）。实测 140 / 402。
 * 更细的「建筑种类」（按引擎级统一编号 `bid` 分组，得 45 组）暂不做 —— 收益偏「对比研究」。
 */
const buildingDwelling: WikiFilterSpec = {
  key: "dwelling",
  label: (t) => t.wiki.filter.byBuildingDwelling,
  options: (entries, ctx) => {
    const n = entries.filter(isDwelling).length;
    return [
      { value: "dwelling", label: ctx.t.wiki.filter.dwellingYes, count: n },
      { value: "other", label: ctx.t.wiki.filter.dwellingNo, count: entries.length - n },
    ];
  },
  match: (e, v) => isDwelling(e) === (v === "dwelling"),
};

/**
 * 来源模组（条目自身的 `modId`）。
 *
 * 名称为多语言（`modMeta.name`，经 index.json 汇总 → 见 `WikiView` 的 `modNames`），
 * 与模组页显示一致；缺失时回退 modId。
 *
 * ⚠️ **只挂在 `artifacts` / `spells`**：这两个类目的原版内容全部收敛到 `core`，
 * 故该维度实际回答「原版 vs 某个 mod」，有独立价值。其余类目一律不挂：
 * - `factions`：1 个城镇 mod 恰好提供 1 个阵营 → 筛选结果与列表一一对应，计数恒为 1、零信息量；
 * - `heroes` / `creatures`：条目按 `core.<faction>` 伪模组分散，实测每个伪模组的条目阵营完全一致
 *   （`core.castle` 的 16 个英雄 `factionId` 全是 `castle`）→ 与「阵营名称」维度重复；
 * - `buildings`：已由「阵营」等价覆盖；`skills`：原版技能全部收敛到 core，无区分度；
 * - `terrains`：游戏级数据、跨 mod 去重后不归属任何单一模组；`mods`：本身即模组清单。
 */
function modFilter(): WikiFilterSpec {
  return {
    key: "mod",
    label: (t) => t.wiki.filter.byMod,
    options: (entries, ctx) =>
      [...countBy(entries, (e) => e.modId).entries()]
        .map(([v, n]) => ({
          value: v,
          label: textOf(ctx.modNames[modIdKey(v)], ctx.contentLang) || v,
          count: n,
        }))
        .sort(byLabel),
    match: (e, v) => e.modId === v,
  };
}

/**
 * 类目 → 筛选维度。**空数组 = 该类目暂无可筛选维度**（`skills` / `terrains` 只有名称搜索）：
 * 此时浮层按钮保持**禁用态但仍占位**，以保证中栏头部布局在任何类目下完全一致。
 */
export const WIKI_FILTERS: Record<WikiCategory, WikiFilterSpec[]> = {
  factions: [alignment],
  heroes: [heroClass, heroSkill, factionFilter("factionId")],
  creatures: [factionFilter("faction"), creatureLevel, creatureTier, creatureAbility],
  artifacts: [artifactClass, artifactSlot, artifactCombined, modFilter()],
  spells: [spellSchool, spellLevel, modFilter()],
  skills: [],
  buildings: [factionFilter("factionId"), buildingDwelling],
  terrains: [],
  mods: [modType],
};
