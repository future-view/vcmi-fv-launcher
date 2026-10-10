/**
 * 与 VCMI mod.json schema 中 modType 枚举对齐的类型系统。
 * VCMI 官方枚举值（按字母序）：
 *   AI, Artifacts, Campaigns, Compatibility, Creatures, Demo, Expansion,
 *   Graphical, Heroes, Interface, Maps, Mechanics, Music, Objects, Other,
 *   Resources, Skills, Sounds, Spells, Templates, Test, Town, Translation
 */

/**
 * 用于左侧筛选标签与右侧分组的显示顺序。
 * 顺序按产品约定排列：城镇、生物、英雄、宝物、法术、技能、物件、模板、地图、战役、
 * 图形、界面、音乐、扩展、机制、AI、资源、兼容、翻译、其他；未列出的类型（Sounds/Test/Demo/Core）
 * 追加在末尾，比较时 `indexOf === -1` 会被排到最后（见 ModBrowser 标签排序）。
 */
export const TYPE_ORDER: string[] = [
  "Town",
  "Creatures",
  "Heroes",
  "Artifacts",
  "Spells",
  "Skills",
  "Objects",
  "Templates",
  "Maps",
  "Campaigns",
  "Graphical",
  "Interface",
  "Music",
  "Sounds",
  "Expansion",
  "Mechanics",
  "AI",
  "Resources",
  "Compatibility",
  "Translation",
  "Other",
  "Test",
  "Demo",
  "Core", // 内部保留类型，用于 vcmi/core
];

export const TYPE_COLORS: Record<string, string> = {
  Town: "#818CF8",
  Expansion: "#34D399",
  Heroes: "#FBBF24",
  Creatures: "#F87171",
  Artifacts: "#A78BFA",
  Spells: "#22D3EE",
  Skills: "#60A5FA",
  Objects: "#FB923C",
  Mechanics: "#94A3B8",
  AI: "#F472B6",
  Interface: "#2DD4BF",
  Graphical: "#C084FC",
  Music: "#E879F9",
  Sounds: "#38BDF8",
  Resources: "#A3E635",
  Maps: "#FCD34D",
  Templates: "#67E8F9",
  Campaigns: "#FDA4AF",
  Translation: "#34D399",
  Compatibility: "#94A3B8",
  Test: "#94A3B8",
  Demo: "#94A3B8",
  Other: "#64748B",
  Core: "#4ADE80",
};

/**
 * 分类逻辑：严格优先 mod.json 的 modType；缺失时才安全推测。
 * 翻译类关键词（ID / 名称 / 描述）优先级高于 factions，
 * 避免含 factions 配置的翻译 mod（如简体中文翻译）被误判为 Town。
 */
export function classifyMod(
  id: string,
  modType: string | undefined,
  name = "",
  description = "",
  factions?: string[],
): string {
  if (modType && modType.trim().length > 0) {
    return modType.trim();
  }

  const id_l = id.toLowerCase();
  const name_l = name.toLowerCase();
  const desc_l = description.toLowerCase();

  if (id === "core" || id === "vcmi") return "Core";
  if (id === "h3e-main-menu") return "Interface";

  // 翻译类优先于 factions，覆盖中文常见写法
  const isTranslation =
    id_l.includes("translation") ||
    id_l.includes("chinese") ||
    id_l.includes("language") ||
    name_l.includes("translation") ||
    name_l.includes("chinese") ||
    name_l.includes("language") ||
    name_l.includes("简体") ||
    name_l.includes("繁体") ||
    name_l.includes("中文") ||
    name_l.includes("汉化") ||
    desc_l.includes("translation") ||
    desc_l.includes("translate") ||
    desc_l.includes("简体") ||
    desc_l.includes("繁体") ||
    desc_l.includes("中文") ||
    desc_l.includes("汉化");
  if (isTranslation) return "Translation";

  if (factions && factions.length > 0) return "Town";
  if (id_l.includes("town") || id_l.includes("faction")) return "Town";
  if (id_l.includes("hero")) return "Heroes";
  if (id_l.includes("spell")) return "Spells";
  if (id_l.includes("artifact")) return "Artifacts";
  if (id_l.includes("creature")) return "Creatures";
  if (id_l.includes("skill")) return "Skills";
  if (id_l.includes("object")) return "Objects";
  if (id_l.includes("mechanic")) return "Mechanics";
  if (id_l.includes("ai") || id_l.includes("mmai")) return "AI";
  if (id_l.includes("interface") || id_l.includes("menu")) return "Interface";
  if (id_l.includes("graphic") || id_l.includes("gfx") || id_l.includes("hd"))
    return "Graphical";
  if (id_l.includes("music")) return "Music";
  if (id_l.includes("sound")) return "Sounds";
  if (id_l.includes("resource")) return "Resources";
  if (id_l.includes("map")) return "Maps";
  if (id_l.includes("template")) return "Templates";
  if (id_l.includes("campaign")) return "Campaigns";
  if (id_l.includes("compat")) return "Compatibility";
  if (id_l.includes("test")) return "Test";
  if (id_l.includes("demo")) return "Demo";

  return "Other";
}
