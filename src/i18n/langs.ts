/**
 * 界面语言登记表 —— 「启动器支持哪几种界面语言」的**唯一事实来源**。
 *
 * 语言不再以字面量联合的形式散落在各处（过去的 `type Localized = { chinese; english }`
 * 就是把语言数量写进了数据结构，加第三门语言必然对不齐）。现在数量只由本表决定：
 *
 * 加一门语言（例如波兰语）只需三步，且每一步漏了都会编译失败：
 * 1. 本表加 `"polish"`；
 * 2. `src/i18n/index.ts` 的 `dicts` 加一条 `polish: pl`（`Record<UiLang, LangDict>` 会要求补齐）；
 * 3. 写 `src/i18n/pl.ts`：因为 `LangDict` 的文案段是 `Record<SettingItemId, …>` 之类的**穷举映射**，
 *    编译器会把缺译的每一条列成错误清单，不存在「运行时才发现空白/undefined」。
 *
 * 与「VCMI 游戏语言标识」同源（对照 `references/vcmi/lib/texts/Languages.h`），
 * 完整语言名单见 `src/lib/vcmiLangs.ts` —— 那里是**数据**（各语言的本地化名），
 * 本表是**本启动器已备好译本的语言**，两者刻意分开。
 */
export const UI_LANG_IDS = ["chinese", "english", "russian", "polish", "german"] as const;

/** 启动器界面语言标识（VCMI 语言标识的子集）。 */
export type UiLang = (typeof UI_LANG_IDS)[number];

/** 无对应译本时的兜底界面语言。 */
export const UI_LANG_FALLBACK: UiLang = "english";
