import { uiLangFor } from "@/i18n";

export interface BuiltinPreset {
  id: string;
  /** 启动器界面展示名，键为 VCMI 语言标识（与 i18n 体系同源：chinese/english/...） */
  label: Record<string, string>;
  /** 默认开启的模组（通用名单） */
  mods: string[];
  /** 按 VCMI 语言标识启用的不同模组名单，键例如 chinese / english / russian 等 */
  modsByLang?: Partial<Record<string, string[]>>;
  /** 是否锁定（仅展示用；强制启用始终由 launcher.json 的 requiredMods 决定） */
  locked?: boolean;
}

export const BUILTIN_PRESETS: BuiltinPreset[] = [
  {
    id: "__official__",
    label: { chinese: "原版", english: "Official" },
    mods: ["vcmi", "core"],
    // 不再在此写死任何「按语言的模组名字」：发行版碎片化导致中文包名字无规范
    // （增强版 simplified-chinese-translation / Expansion ChineseTrueTypeFonts / 官方
    // chinese-translation-vcmi）。语言包改为由 langMods.ts 按「已装模组的
    // modType==Translation && language==当前语言」分类发现，与模组 id 无关。
    // `modsByLang` 字段保留，仅供 launcher.json 按 id 字段级合并覆盖（不推荐常规使用）。
  },
];

// 当前生效的内置预设列表：默认即源码常量；ModView 加载 launcher.json 后会调用
// setActiveBuiltinPresets 将其替换为「源码常量 ∪ 配置追加」的合并结果。
// 这样预设相关的判断（isBuiltinPreset / builtinPresetMods / builtinPresetLabel）无需改签名即可感知配置。
let ACTIVE_BUILTINS: BuiltinPreset[] = BUILTIN_PRESETS;

export function setActiveBuiltinPresets(list: BuiltinPreset[]): void {
  ACTIVE_BUILTINS = list;
}

export function getActiveBuiltinPresets(): BuiltinPreset[] {
  return ACTIVE_BUILTINS;
}

export function isBuiltinPreset(name: string): boolean {
  return ACTIVE_BUILTINS.some((p) => p.id === name);
}

export function builtinPresetLabel(id: string, vcmiLang: string): string {
  const p = ACTIVE_BUILTINS.find((x) => x.id === id);
  if (!p) return id;
  const key = uiLangFor(vcmiLang);
  return p.label[key] || p.label.english || id;
}

/**
 * 按 VCMI 语言标识返回内置预设应启用的模组名单。
 * 键名即 VCMI mod.json language 枚举（chinese / english / ...），无需任何转换。
 */
export function builtinPresetMods(id: string, vcmiLang: string): string[] {
  const p = ACTIVE_BUILTINS.find((x) => x.id === id);
  if (!p) return [];
  if (p.modsByLang?.[vcmiLang]) {
    return p.modsByLang[vcmiLang]!;
  }
  return p.mods;
}

/**
 * 将 launcher.json 的 `builtin_presets` 与源码内置 `__official__` 合并追加：
 * - 新增 id → 直接追加；
 * - 同名 id → 字段级合并：`mods` 并集去重；`modsByLang` 按语言键并集去重（配置优先）；`label` 合并。
 * 这样发行版可不改源码地追加语言版/新预设，源码常量作为兜底默认。
 */
export function mergeBuiltinPresets(
  source: BuiltinPreset[],
  config?: BuiltinPreset[],
): BuiltinPreset[] {
  if (!config || config.length === 0) return source;
  const byId = new Map<string, BuiltinPreset>();
  for (const p of source) {
    byId.set(p.id, {
      ...p,
      label: { ...p.label },
      mods: [...p.mods],
      modsByLang: { ...p.modsByLang },
    });
  }
  for (const c of config) {
    const base = byId.get(c.id);
    if (!base) {
      byId.set(c.id, {
        id: c.id,
        label: { ...(c.label ?? {}) },
        mods: [...(c.mods ?? [])],
        modsByLang: { ...(c.modsByLang ?? {}) },
        locked: c.locked ?? false,
      });
    } else {
      const mods = Array.from(new Set([...base.mods, ...(c.mods ?? [])]));
      const modsByLang: Partial<Record<string, string[]>> = { ...base.modsByLang };
      for (const [lang, arr] of Object.entries(c.modsByLang ?? {})) {
        modsByLang[lang] = Array.from(new Set([...(modsByLang[lang] ?? []), ...(arr ?? [])]));
      }
      const label = { ...base.label, ...(c.label ?? {}) };
      byId.set(c.id, { ...base, mods, modsByLang, label, locked: c.locked ?? base.locked });
    }
  }
  return [...byId.values()];
}
