/**
 * VCMI 语言标识展示表（对照 `references/vcmi/lib/texts/Languages.h` 的 getLanguageList）。
 * identifier 与 VCMI 官方完全一致，地图语言、启动器 UI 语言共用同一套标识。
 * 仅取 nativeName / englishName / tag 用于展示与资源定位，其余属性（编码/日期格式等）由游戏端负责。
 *
 * `tag` 为 BCP-47 风格标签（如 zh-CN / en / ru），是「VCMI 语言 id → 文档文件名 / `<html lang>`」
 * 的**唯一事实来源**：界面词典的 `<html lang>`（`src/i18n/index.ts`）与内置帮助文档的文件名
 * （`src/lib/helpDoc.ts` 的 `help.<tag>.md`）都经 `vcmiLangTag` 取它，避免两处各维护一份映射、
 * 新增语言时漏改（旧版 `HTML_LANG` / `HELP_FILE_TAG` 即是分叉的两份）。
 */

export interface VcmiLangOption {
  id: string;
  /** 英文名（Languages.h 第 2 列） */
  en: string;
  /** 本地化名（Languages.h 第 3 列） */
  native: string;
  /** BCP-47 标签，用于 `<html lang>` 与帮助文档文件名（`help.<tag>.md`） */
  tag: string;
}

/** VCMI 官方默认游戏语言（引擎 `ModDescription::getBaseLanguage` 与 settings 默认值均为 english）；各处「缺省语言」兜底统一引用它，避免散落字面量。 */
export const VCMI_DEFAULT_LANG = "english" as const;

export const VCMI_LANGS: VcmiLangOption[] = [
  { id: "belarusian", en: "Belarusian", native: "Беларускі", tag: "be" },
  { id: "bulgarian", en: "Bulgarian", native: "Български", tag: "bg" },
  { id: "czech", en: "Czech", native: "Čeština", tag: "cs" },
  { id: "chinese", en: "Simplified Chinese", native: "简体中文", tag: "zh-CN" },
  { id: "tchinese", en: "Traditional Chinese", native: "正體字", tag: "zh-TW" },
  { id: "dutch", en: "Dutch", native: "Nederlands", tag: "nl" },
  { id: "english", en: "English", native: "English", tag: "en" },
  { id: "filipino", en: "Filipino", native: "Pilipino", tag: "fil" },
  { id: "finnish", en: "Finnish", native: "Suomi", tag: "fi" },
  { id: "french", en: "French", native: "Français", tag: "fr" },
  { id: "german", en: "German", native: "Deutsch", tag: "de" },
  { id: "greek", en: "Greek", native: "ελληνικά", tag: "el" },
  { id: "hungarian", en: "Hungarian", native: "Magyar", tag: "hu" },
  { id: "italian", en: "Italian", native: "Italiano", tag: "it" },
  { id: "japanese", en: "Japanese", native: "日本語", tag: "ja" },
  { id: "korean", en: "Korean", native: "한국어", tag: "ko" },
  { id: "latvian", en: "Latvian", native: "Latviešu", tag: "lv" },
  { id: "lithuanian", en: "Lithuanian", native: "Lietuvių", tag: "lt" },
  { id: "norwegian", en: "Norwegian", native: "Norsk Bokmål", tag: "nb" },
  { id: "polish", en: "Polish", native: "Polski", tag: "pl" },
  { id: "portuguese", en: "Portuguese", native: "Português", tag: "pt" },
  { id: "romanian", en: "Romanian", native: "Română", tag: "ro" },
  { id: "russian", en: "Russian", native: "Русский", tag: "ru" },
  { id: "serbian", en: "Serbian", native: "Српски", tag: "sr" },
  { id: "spanish", en: "Spanish", native: "Español", tag: "es" },
  { id: "swedish", en: "Swedish", native: "Svenska", tag: "sv" },
  { id: "turkish", en: "Turkish", native: "Türkçe", tag: "tr" },
  { id: "ukrainian", en: "Ukrainian", native: "Українська", tag: "uk" },
  { id: "vietnamese", en: "Vietnamese", native: "Tiếng Việt", tag: "vi" },
];

const BY_ID = new Map(VCMI_LANGS.map((l) => [l.id, l]));

/** 取语言的本地化展示名；未识别（unknown / 空 / 未知 id）回退 identifier 本身。 */
export function vcmiLangName(id?: string | null): string {
  if (!id) return "unknown";
  return BY_ID.get(id)?.native ?? id;
}

/** 取语言的 BCP-47 标签（`<html lang>` 与帮助文档文件名 `help.<tag>.md` 用）；未识别或未知 id 回退 undefined（调用方自行兜底）。 */
export function vcmiLangTag(id?: string | null): string | undefined {
  if (!id) return undefined;
  return BY_ID.get(id)?.tag;
}

/** 过滤目标语言下拉的选项集（native 名排序，语言无关展示）。 */
export function vcmiLangOptions(): VcmiLangOption[] {
  return [...VCMI_LANGS].sort((a, b) => a.native.localeCompare(b.native));
}
