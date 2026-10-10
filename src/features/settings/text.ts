/**
 * 设置面板的**唯一取词入口**。
 *
 * 背景：设置项文本曾经直接写在 `src/lib/settingsSchema.ts` 里（`{ chinese, english }` 双字段），
 * 那等于在 `src/i18n` 之外另立了一套多语言，语言数量被写死进数据结构。现在结构（schema）
 * 与文案（词典）分离，本文件负责把两者接起来，并固化取值显示名的兜底顺序：
 *
 *   专属 `settings.enums[`${设置项id}.${原始值}`]`
 *     → 通用 `settings.valueWords[原始值]`（auto / off / hardware …）
 *     → 语言名 `vcmiLangName(原始值)`（仅 `general.language` 等语言选项类，语言无关）
 *     → 原始值本身（技术标识如 opengl33、xbrz4，与各官方界面一致原样显示）
 *
 * 新增语言不需要改本文件：只要 `src/i18n/<lang>.ts` 实现了 `LangDict`，
 * 缺译的条目会在编译期被 `Record<SettingItemId, …>` 穷举暴露出来。
 */
import { useMemo } from "react";
import { useI18n } from "@/i18n";
import type { LangDict, SettingsWords } from "@/i18n/types";
import type { SettingEnumKey, SettingItemId, SettingTagId, SettingValueWord } from "@/generated/settingKeys";
import { SETTING_LANG_OPTION_KEYS } from "@/generated/settingKeys";
import { vcmiLangName } from "@/lib/vcmiLangs";
import { SETTING_SCHEMA_ENUM_OPTIONS, type SettingItem } from "@/lib/settingsSchema";

/** 候选（含运行时动态候选）：`v` 是写入配置的原始值，`text` 是按当前界面语言解析好的显示名 */
export interface SettingOption {
  v: string;
  text: string;
  /** 分组标签（如分辨率按屏幕比分组：16:9 / 16:10 / 4:3 …）；无则不分组 */
  group?: string;
  /** 是否为当前显示器原生分辨率（前端据此挂「当前」徽标 / 优先排序） */
  primary?: boolean;
}

const LANG_OPTION_KEYS = new Set<string>(SETTING_LANG_OPTION_KEYS);

/** 把 `{name}` 之类的占位符替换进模板（与 `saveError: "…{msg}"` 同一套路） */
function fillTemplate(template: string, vars: Record<string, string>): string {
  return template.replace(/\{(\w+)\}/g, (whole, key: string) => vars[key] ?? whole);
}

export interface SettingTextApi {
  /** 左侧标签名 */
  tagLabel(id: SettingTagId): string;
  /** 设置项标题 */
  itemLabel(id: SettingItemId): string;
  /** 设置项详细描述（勾选「详细描述」才显示） */
  itemDesc(id: SettingItemId): string;
  /** 某设置项候选值的显示名（兜底顺序见文件头） */
  valueLabel(itemId: string, raw: unknown): string;
  /** 候选清单：运行时覆盖优先，否则取 `candidates ?? enabledO ?? o ?? schema 枚举` */
  optionsFor(item: SettingItem, override?: SettingOption[]): SettingOption[];
  /** 切换类控件的固定档位（如全屏三态、生物显示三态） */
  modesFor(itemId: SettingItemId, modes: readonly string[]): SettingOption[];
  /** 控件局部文案（「不使用」/「（默认）」/「{name}（推荐）」等） */
  word(name: keyof SettingsWords, vars?: Record<string, string>): string;
  /** 搜索用文本球（当前界面语言的标签 + 描述 + 设置键），已小写 */
  searchBlobOf(item: SettingItem): string;
}

/** 由词典构造取词 API（纯函数，便于在非组件环境复用与单测） */
export function createSettingText(t: LangDict): SettingTextApi {
  const valueLabel = (itemId: string, raw: unknown): string => {
    const v = raw === null || raw === undefined ? "" : String(raw);
    if (v === "") return t.settings.words.defaultOption;
    if (LANG_OPTION_KEYS.has(itemId)) {
      // 语言选项：`auto` 用通用词，其余一律显示该语言的本地化名（不随界面语言变化）
      return v === "auto" ? (t.settings.valueWords.auto ?? v) : vcmiLangName(v);
    }
    return (
      t.settings.enums[`${itemId}.${v}` as SettingEnumKey] ??
      t.settings.valueWords[v as SettingValueWord] ??
      v
    );
  };

  const optionsFor = (item: SettingItem, override?: SettingOption[]): SettingOption[] => {
    if (override?.length) return override;
    const values = item.candidates?.length
      ? item.candidates
      : item.enabledO?.length
        ? item.enabledO
        : item.o?.length
          ? item.o
          : (SETTING_SCHEMA_ENUM_OPTIONS[item.k] ?? []);
    return values.map((v) => ({ v, text: valueLabel(item.id, v) }));
  };

  return {
    tagLabel: (id) => t.settings.tags[id],
    itemLabel: (id) => t.settings.items[id].l,
    itemDesc: (id) => t.settings.items[id].d,
    valueLabel,
    optionsFor,
    modesFor: (itemId, modes) => modes.map((v) => ({ v, text: valueLabel(itemId, v) })),
    word: (name, vars) => (vars ? fillTemplate(t.settings.words[name], vars) : t.settings.words[name]),
    searchBlobOf: (item) =>
      `${item.k} ${item.id} ${t.settings.items[item.id].l} ${t.settings.items[item.id].d}`.toLowerCase(),
  };
}

/** 组件内取词：随界面语言切换自动重渲染 */
export function useSettingText(): SettingTextApi {
  const { t } = useI18n();
  return useMemo(() => createSettingText(t), [t]);
}
