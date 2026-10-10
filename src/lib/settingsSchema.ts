/**
 * VCMI 设置面板 schema —— 严格以 `docs/plan/设置面板_设计方案.md`(第五章) 为唯一事实来源逐行录入。
 *
 * ⚠️ **本文件只描述结构，不存放任何界面文案**：标签名、设置项标题与详细描述、
 * 枚举取值的显示名全部在 i18n 词典里（`src/i18n/settingsText.<lang>.ts`），
 * 取词入口是 `src/features/settings/text.ts`。新增设置项的流程：
 *  1. 在此登记一行结构（`k` / `id` / `ctl` / `level` / `val` …）；
 *  2. `npm run gen:setting-keys` 重生成键域 `src/generated/settingKeys.ts`；
 *  3. 给每本词典的 `settings.items` 补上 `{ l, d }` —— 漏了会被 `tsc` 直接报错（穷举映射）。
 *
 * 约定：
 * - `k` 直接等于 VCMI settings.json 的真实 key（1:1 映射）；`id` 与 `k` 同值，兼作文案键。
 * - `val` 取设计文档默认值；UI 打开即显示真实进度。
 * - `level`：可见级别 common(常用)/advanced(进阶)/all(全部)，严格按文档。
 * - `ctl`：控件类型（第四章）。其中 `fullscreenGroup` / `creatureDisplay` 为跨键互斥控件（见下方说明）。
 * - `o` / `candidates`：候选的**原始值**（写入配置用的字面量）；显示名由词典 `enums` / `valueWords` 供给，
 *   语言选项类（`general.language` 等）则由 `src/lib/vcmiLangs.ts` 的本地化名供给，不进词典。
 *
 * 运行态字段（文档标注"不渲染"）不进入设置列表，单独列于 RUN_STATE_KEYS 供"运行信息"弹窗读取。
 */
import type { SettingItemId, SettingTagId } from "@/generated/settingKeys";
import { VCMI_LANGS, VCMI_DEFAULT_LANG } from "./vcmiLangs";
import { UI_LANG_IDS } from "@/i18n/langs";

export type SettingLevel = "common" | "advanced" | "all";

export type SettingCtl =
  | "switch"
  | "select"
  | "slider"
  | "sliderPct"
  | "number"
  | "text"
  | "btnList"
  | "toggleGroup"
  | "resolutionCombo" // 分辨率组合下拉：选中 WxH 后拆写 width/height 两键
  | "scalingGroup" // 界面缩放联动组：自动(checkbox) + 预设下拉 + 自定义输入
  | "json"
  | "fullscreenGroup" // 跨键：窗口化/无边框/独占 ↔ fullscreen(realFullscreen=false) / realFullscreen
  | "creatureDisplay"; // 跨键：none/增长/可招募 ↔ creatureGrowthAsDwellingLabel / availableCreaturesAsDwellingLabel

export interface SettingItem {
  k: string;
  /** 文案键（与 `k` 同值）：词典 `settings.items[id]` 即标签与详细描述 */
  id: SettingItemId;
  ctl: SettingCtl;
  level: SettingLevel;
  val: unknown;
  o?: string[]; // select 选项（原始值）
  candidates?: string[]; // btnList / toggleGroup 候选（原始值）
  min?: number;
  max?: number;
  step?: number;
  /** 跨键控件：联动的另一个 key */
  altKey?: string;
  /** 仅显示的枚举子集（其余保留待后续开放）；省略=全部启用 */
  enabledO?: string[];
  /** 依赖禁用：当依赖键满足条件时禁用本项。eq/neq 取依赖键的当前值比较 */
  disabledWhen?: { key: string; eq?: unknown; neq?: unknown };
  /** 动态候选：运行时从后端拉取（scaling=按主显示器算上限；resolution=支持的分辨率；
   * aiModels=组件目录扫描到的 AI 模型文件名） */
  dynamic?: "scaling" | "resolution" | "aiModels";
}

export interface SettingTag {
  id: SettingTagId;
  icon: string;
  items: SettingItem[];
}

/**
 * 完整语言枚举 —— **派生自 `src/lib/vcmiLangs.ts`**（该表逐条对照
 * `references/vcmi/lib/texts/Languages.h` 的 getLanguageList()，与 VCMI 官方启动器
 * 同一套 identifier 与顺序）。不再手抄：手抄清单曾混入官方没有的 `slovak`，
 * 与官方的漂移正是这么发生的。
 * - INTERFACE_LANGS 共 29 种（无 auto），用于 general.language。
 * - DATA_LANGS 共 30 种（含 auto），用于 general.gameDataLanguage。
 * `general.language` / `general.gameDataLanguage` / `fv.uiLang` 的**启用范围**由
 * `src/i18n/langs.ts` 的 `UI_LANG_IDS`（已备译本的语言）与 `auto` 决定。
 * 候选项的显示名统一取 `src/lib/vcmiLangs.ts` 的本地化名（语言无关，不需逐语言重译）。
 */
const INTERFACE_LANGS = VCMI_LANGS.map((l) => l.id);
const DATA_LANGS = ["auto", ...INTERFACE_LANGS];

/** 运行态字段（文档标注"不渲染"，归入运行信息弹窗） */
export const RUN_STATE_KEYS = {
  general: ["general.lastMap", "general.lastSave", "general.lastSettingsTab", "general.lastCampaign", "general.lastDifficulty", "general.lastKindomInterface"],
  launcher: ["launcher.setupCompleted", "launcher.mainWindow.geometry.x", "launcher.mainWindow.geometry.y", "launcher.mainWindow.geometry.width", "launcher.mainWindow.geometry.height"],
} as const;

/* ===================== 通用 General ===================== */
const GENERAL: SettingItem[] = [
  { k: "general.playerName", id: "general.playerName", ctl: "text", level: "common", val: "Player" },
  { k: "general.gameDataLanguage", id: "general.gameDataLanguage", ctl: "select", o: DATA_LANGS, enabledO: ["auto", ...UI_LANG_IDS], level: "common", val: "auto" },
  { k: "general.music", id: "general.music", ctl: "sliderPct", level: "common", min: 0, max: 100, val: 88 },
  { k: "general.sound", id: "general.sound", ctl: "sliderPct", level: "common", min: 0, max: 100, val: 88 },
  { k: "general.saveBeforeVisit", id: "general.saveBeforeVisit", ctl: "switch", level: "common", val: false },
  { k: "general.autosaveCountLimit", id: "general.autosaveCountLimit", ctl: "number", min: 0, max: 50, val: 5, level: "common" },
  { k: "general.audioMuteFocus", id: "general.audioMuteFocus", ctl: "switch", level: "advanced", val: false },
  { k: "general.enableUiEnhancements", id: "general.enableUiEnhancements", ctl: "switch", level: "advanced", val: true },
  { k: "general.enableOverlay", id: "general.enableOverlay", ctl: "switch", level: "advanced", val: true },
  { k: "general.language", id: "general.language", ctl: "select", o: INTERFACE_LANGS, enabledO: [...UI_LANG_IDS], level: "all", val: VCMI_DEFAULT_LANG },
  { k: "general.enableSubtitle", id: "general.enableSubtitle", ctl: "switch", level: "all", val: true },
  { k: "general.startTurnAutosave", id: "general.startTurnAutosave", ctl: "switch", level: "all", val: false },
  { k: "general.saveRandomMaps", id: "general.saveRandomMaps", ctl: "switch", level: "all", val: false },
  { k: "general.saveFrequency", id: "general.saveFrequency", ctl: "number", min: 0, max: 100, val: 1, level: "all" },
  { k: "general.useSavePrefix", id: "general.useSavePrefix", ctl: "switch", level: "all", val: false },
  { k: "general.savePrefix", id: "general.savePrefix", ctl: "text", level: "all", val: "", disabledWhen: { key: "general.useSavePrefix", neq: true } },
  { k: "general.notifications", id: "general.notifications", ctl: "switch", level: "all", val: false },
  { k: "general.extraDump", id: "general.extraDump", ctl: "switch", level: "all", val: false },
  { k: "general.userRelativePointer", id: "general.userRelativePointer", ctl: "switch", level: "all", val: false },
  { k: "general.relativePointerSpeedMultiplier", id: "general.relativePointerSpeedMultiplier", ctl: "number", min: 0.5, max: 1.5, step: 0.05, val: 1, level: "all" },
  { k: "general.hapticFeedback", id: "general.hapticFeedback", ctl: "switch", level: "all", val: true },
  { k: "general.longTouchTimeMilliseconds", id: "general.longTouchTimeMilliseconds", ctl: "btnList", level: "all", candidates: ["500","750","1000","1250","1500","1750","2000"], val: 1000 },
  { k: "general.ignoreMuteSwitch", id: "general.ignoreMuteSwitch", ctl: "switch", level: "all", val: true },
  { k: "general.highscoreEntriesCap", id: "general.highscoreEntriesCap", ctl: "number", min: 0, max: 1000, val: 100, level: "all" },
  { k: "general.highscoreStatisticEntriesCap", id: "general.highscoreStatisticEntriesCap", ctl: "number", min: 0, max: 1000, val: 15, level: "all" },
  { k: "general.enableDiscordRichPresence", id: "general.enableDiscordRichPresence", ctl: "switch", level: "all", val: true },
  { k: "general.multiPlayerNames", id: "general.multiPlayerNames", ctl: "json", level: "all", val: [] },
];

/* ===================== 视频 Video ===================== */
// 分辨率：组合下拉（dynamic 由后端 list_resolutions 提供候选），选中后拆写 width/height。
// 默认 1280x720 与 VCMI 源码一致。
const VIDEO: SettingItem[] = [
  { k: "video.fullscreen", id: "video.fullscreen", ctl: "fullscreenGroup", level: "common", val: false, altKey: "video.realFullscreen" },
  { k: "video.resolution.width", id: "video.resolution.width", ctl: "resolutionCombo", level: "common", altKey: "video.resolution.height", dynamic: "resolution", disabledWhen: { key: "video.fullscreen", neq: false }, val: 1280 },
  { k: "video.resolution.height", id: "video.resolution.height", ctl: "number", level: "common", val: 720 },
  { k: "video.resolution.scaling", id: "video.resolution.scaling", ctl: "scalingGroup", level: "common", val: 0, dynamic: "scaling" },
  { k: "video.driver", id: "video.driver", ctl: "select", o: ["auto", "detect", "opengl33", "direct3d11", "software"], level: "advanced", val: "auto" },
  { k: "video.cursor", id: "video.cursor", ctl: "select", o: ["hardware", "software"], level: "advanced", val: "hardware" },
  { k: "video.useHdTextures", id: "video.useHdTextures", ctl: "switch", level: "common", val: true },
  { k: "video.vsync", id: "video.vsync", ctl: "switch", level: "common", val: true },
  { k: "video.reservedWidth", id: "video.reservedWidth", ctl: "number", min: 0, max: 1024, val: 0, level: "all" },
  { k: "video.displayIndex", id: "video.displayIndex", ctl: "number", min: 0, max: 8, val: 0, level: "all" },
  { k: "video.targetfps", id: "video.targetfps", ctl: "number", min: 30, max: 240, val: 60, level: "advanced", disabledWhen: { key: "video.vsync", eq: true } },
  { k: "video.fontsType", id: "video.fontsType", ctl: "select", o: ["auto", "original", "scalable"], level: "advanced", val: "auto" },
  { k: "video.cursorScalingFactor", id: "video.cursorScalingFactor", ctl: "number", min: 1, max: 4, val: 1, level: "advanced" },
  { k: "video.fontScalingFactor", id: "video.fontScalingFactor", ctl: "number", min: 0.5, max: 1.5, step: 0.05, val: 1, level: "advanced" },
  { k: "video.upscalingFilter", id: "video.upscalingFilter", ctl: "select", o: ["auto", "none", "xbrz2", "xbrz3", "xbrz4"], level: "advanced", val: "auto" },
  { k: "video.fontUpscalingFilter", id: "video.fontUpscalingFilter", ctl: "select", o: ["nearest", "bilinear", "xbrz"], level: "advanced", val: "nearest" },
  { k: "video.downscalingFilter", id: "video.downscalingFilter", ctl: "select", o: ["nearest", "linear", "best"], level: "advanced", val: "best" },
  { k: "video.asyncUpscaling", id: "video.asyncUpscaling", ctl: "switch", level: "advanced", val: true },
  { k: "video.allowPortrait", id: "video.allowPortrait", ctl: "switch", level: "all", val: false },
  { k: "video.showIntro", id: "video.showIntro", ctl: "switch", level: "all", val: true },
  { k: "video.spellbookAnimation", id: "video.spellbookAnimation", ctl: "switch", level: "all", val: true },
  { k: "video.performanceOverlay.show", id: "video.performanceOverlay.show", ctl: "switch", level: "all", val: false },
  { k: "video.performanceOverlay.edge", id: "video.performanceOverlay.edge", ctl: "select", o: ["topleft", "topright", "bottomleft", "bottomright"], level: "all", val: "bottomleft" },
  { k: "video.performanceOverlay.marginTopBottom", id: "video.performanceOverlay.marginTopBottom", ctl: "number", min: 0, max: 100, val: 5, level: "all" },
  { k: "video.performanceOverlay.marginLeftRight", id: "video.performanceOverlay.marginLeftRight", ctl: "number", min: 0, max: 100, val: 7, level: "all" },
];

/* ===================== 冒险 Adventure ===================== */
const ADVENTURE: SettingItem[] = [
  { k: "adventure.quickCombat", id: "adventure.quickCombat", ctl: "switch", level: "common", val: false },
  { k: "adventure.heroMoveTime", id: "adventure.heroMoveTime", ctl: "toggleGroup", level: "common", val: 150, candidates: ["200","150","100","50","25","0"] },
  { k: "adventure.enemyMoveTime", id: "adventure.enemyMoveTime", ctl: "toggleGroup", level: "common", val: 150, candidates: ["-1","0","25","50","100","150"] },
  { k: "adventure.scrollSpeedPixels", id: "adventure.scrollSpeedPixels", ctl: "toggleGroup", level: "common", val: 800, candidates: ["200","400","800","1200","2400","4800"] },
  { k: "adventure.showMovePath", id: "adventure.showMovePath", ctl: "switch", level: "advanced", val: true },
  { k: "adventure.borderScroll", id: "adventure.borderScroll", ctl: "switch", level: "advanced", val: true },
  { k: "adventure.heroReminder", id: "adventure.heroReminder", ctl: "switch", level: "advanced", val: true },
  { k: "adventure.replayShowBattles", id: "adventure.replayShowBattles", ctl: "switch", level: "advanced", val: true },
  { k: "adventure.leftButtonDrag", id: "adventure.leftButtonDrag", ctl: "switch", level: "advanced", val: false },
  { k: "adventure.rightButtonDrag", id: "adventure.rightButtonDrag", ctl: "switch", level: "advanced", val: false },
  { k: "adventure.loadLastSave", id: "adventure.loadLastSave", ctl: "switch", level: "all", val: true },
  { k: "adventure.minimapShowHeroes", id: "adventure.minimapShowHeroes", ctl: "switch", level: "all", val: true },
  { k: "adventure.smoothDragging", id: "adventure.smoothDragging", ctl: "switch", level: "all", val: true },
  { k: "adventure.hideBackground", id: "adventure.hideBackground", ctl: "switch", level: "all", val: false },
  { k: "adventure.objectAnimation", id: "adventure.objectAnimation", ctl: "switch", level: "all", val: true },
  { k: "adventure.terrainAnimation", id: "adventure.terrainAnimation", ctl: "switch", level: "all", val: true },
  { k: "adventure.forceQuickCombat", id: "adventure.forceQuickCombat", ctl: "switch", level: "all", val: false },
  { k: "adventure.backgroundDimLevel", id: "adventure.backgroundDimLevel", ctl: "slider", min: 0, max: 255, val: 128, level: "all" },
  { k: "adventure.backgroundDimSmallWindows", id: "adventure.backgroundDimSmallWindows", ctl: "switch", level: "all", val: false },
  { k: "adventure.tileZoom", id: "adventure.tileZoom", ctl: "number", min: 32, max: 64, val: 32, level: "all" },
];

/* ===================== 战斗 Battle ===================== */
const BATTLE: SettingItem[] = [
  { k: "battle.speedFactor", id: "battle.speedFactor", ctl: "toggleGroup", level: "common", val: 2, candidates: ["1","2","3","6","9","18"] },
  { k: "battle.enableAutocombatSpells", id: "battle.enableAutocombatSpells", ctl: "switch", level: "common", val: true },
  { k: "battle.stickyHeroInfoWindows", id: "battle.stickyHeroInfoWindows", ctl: "switch", level: "common", val: true },
  { k: "battle.showHealthBar", id: "battle.showHealthBar", ctl: "switch", level: "common", val: false },
  { k: "battle.stackRange", id: "battle.stackRange", ctl: "switch", level: "common", val: true },
  { k: "battle.rangeLimitHighlightOnHover", id: "battle.rangeLimitHighlightOnHover", ctl: "switch", level: "common", val: false },
  { k: "battle.cellBorders", id: "battle.cellBorders", ctl: "switch", level: "common", val: false },
  { k: "battle.movementHighlightOnHover", id: "battle.movementHighlightOnHover", ctl: "switch", level: "common", val: true },
  { k: "battle.endWithAutocombat", id: "battle.endWithAutocombat", ctl: "switch", level: "common", val: false },
  { k: "battle.mouseShadow", id: "battle.mouseShadow", ctl: "switch", level: "advanced", val: true },
  { k: "battle.stackInfoBasicPanel", id: "battle.stackInfoBasicPanel", ctl: "switch", level: "advanced", val: true },
  { k: "battle.enableQuickSpellPanel", id: "battle.enableQuickSpellPanel", ctl: "switch", level: "advanced", val: true },
  { k: "battle.showQueue", id: "battle.showQueue", ctl: "switch", level: "all", val: true },
  { k: "battle.queueSize", id: "battle.queueSize", ctl: "select", o: ["auto", "small", "big"], level: "all", val: "auto" },
  { k: "battle.swipeAttackDistance", id: "battle.swipeAttackDistance", ctl: "number", level: "all", min: 0, max: 1000, val: 250 },
  { k: "battle.queueSmallSlots", id: "battle.queueSmallSlots", ctl: "number", min: 1, max: 20, val: 10, level: "all" },
  { k: "battle.queueSmallOutside", id: "battle.queueSmallOutside", ctl: "switch", level: "all", val: false },
];

/* ===================== 游戏调整 Game Tweaks ===================== */
const GAME_TWEAKS: SettingItem[] = [
  { k: "gameTweaks.skipBattleIntroMusic", id: "gameTweaks.skipBattleIntroMusic", ctl: "switch", level: "advanced", val: false },
  { k: "gameTweaks.enableLargeSpellbook", id: "gameTweaks.enableLargeSpellbook", ctl: "switch", level: "advanced", val: true },
  { k: "gameTweaks.availableCreaturesAsDwellingLabel", id: "gameTweaks.availableCreaturesAsDwellingLabel", ctl: "creatureDisplay", level: "advanced", val: false, altKey: "gameTweaks.creatureGrowthAsDwellingLabel" },
  { k: "gameTweaks.numericCreaturesQuantities", id: "gameTweaks.numericCreaturesQuantities", ctl: "switch", level: "advanced", val: false },
  { k: "gameTweaks.infoBarPick", id: "gameTweaks.infoBarPick", ctl: "switch", level: "advanced", val: false },
  { k: "gameTweaks.infoBarCreatureManagement", id: "gameTweaks.infoBarCreatureManagement", ctl: "switch", level: "advanced", val: true },
  { k: "gameTweaks.compactTownCreatureInfo", id: "gameTweaks.compactTownCreatureInfo", ctl: "switch", level: "all", val: false },
  { k: "gameTweaks.showGrid", id: "gameTweaks.showGrid", ctl: "switch", level: "all", val: false },
  { k: "gameTweaks.forceMovementInfo", id: "gameTweaks.forceMovementInfo", ctl: "switch", level: "all", val: false },
  { k: "gameTweaks.simpleObjectSelection", id: "gameTweaks.simpleObjectSelection", ctl: "switch", level: "all", val: true },
  { k: "gameTweaks.skipAdventureMapAnimations", id: "gameTweaks.skipAdventureMapAnimations", ctl: "switch", level: "all", val: false },
];

/* ===================== 电脑智能 AI ===================== */
const AI: SettingItem[] = [
  { k: "ai.adventureEnemyAI", id: "ai.adventureEnemyAI", ctl: "select", o: ["EmptyAI", "Nullkiller", "Nullkiller2"], level: "advanced", val: "Nullkiller2" },
  { k: "ai.adventureAlliedAI", id: "ai.adventureAlliedAI", ctl: "select", o: ["EmptyAI", "Nullkiller", "Nullkiller2"], level: "advanced", val: "Nullkiller2" },
  { k: "ai.combatEnemyAI", id: "ai.combatEnemyAI", ctl: "select", o: ["EmptyAI", "StupidAI", "BattleAI", "MMAI"], level: "advanced", val: "BattleAI" },
  { k: "ai.combatAlliedAI", id: "ai.combatAlliedAI", ctl: "select", o: ["EmptyAI", "StupidAI", "BattleAI", "MMAI"], level: "advanced", val: "BattleAI" },
  { k: "ai.combatNeutralAI", id: "ai.combatNeutralAI", ctl: "select", o: ["EmptyAI", "StupidAI", "BattleAI", "MMAI"], level: "advanced", val: "BattleAI" },
];

/* ===================== 输入 Input ===================== */
const INPUT: SettingItem[] = [
  { k: "input.radialWheelGarrisonSwipe", id: "input.radialWheelGarrisonSwipe", ctl: "switch", level: "all", val: true },
  { k: "input.touchToleranceDistance", id: "input.touchToleranceDistance", ctl: "slider", min: 0, max: 100, val: 20, level: "all" },
  { k: "input.mouseToleranceDistance", id: "input.mouseToleranceDistance", ctl: "slider", min: 0, max: 100, val: 0, level: "all" },
  { k: "input.shortcutToleranceDistance", id: "input.shortcutToleranceDistance", ctl: "slider", min: 0, max: 100, val: 0, level: "all" },
  { k: "input.enableMouse", id: "input.enableMouse", ctl: "switch", level: "all", val: true },
  { k: "input.enableTouch", id: "input.enableTouch", ctl: "switch", level: "all", val: true },
  { k: "input.enableController", id: "input.enableController", ctl: "switch", level: "all", val: true },
  { k: "input.controllerTriggerTreshold", id: "input.controllerTriggerTreshold", ctl: "slider", min: 0, max: 1, step: 0.05, val: 0.3, level: "all" },
  { k: "input.controllerAxisDeadZone", id: "input.controllerAxisDeadZone", ctl: "slider", min: 0, max: 1, step: 0.05, val: 0.2, level: "all" },
  { k: "input.controllerAxisFullZone", id: "input.controllerAxisFullZone", ctl: "slider", min: 0, max: 1, step: 0.05, val: 1.0, level: "all" },
  { k: "input.controllerAxisSpeed", id: "input.controllerAxisSpeed", ctl: "number", min: 0, max: 5000, val: 1000, level: "all" },
  { k: "input.controllerAxisScale", id: "input.controllerAxisScale", ctl: "number", min: 1, max: 10, val: 2, level: "all" },
  { k: "input.handleBackRightMouseButton", id: "input.handleBackRightMouseButton", ctl: "switch", level: "all", val: false },
];

/* ===================== FV 启动器（launcher.json） ===================== */
/**
 * **FV 启动器自身的配置**（`launcher.json`），与其它标签的 VCMI `settings.json` **完全不同源**。
 *
 * ⚠️ 区分约定：`k` 一律以 `fv.` 开头 —— 渲染层据此走 launcher.json 的读写通道
 * （`readLauncherConfig` / `saveAiHdEnhance` / `saveTownPreview` / `set_ui_lang`），
 * 而不是 VCMI 设置通道。**不要**把 `fv.` 项混进 `ALL_ITEMS` 的值比对逻辑之外的 VCMI 路径。
 *
 * 可见等级与 VCMI 项同一套语义（common/advanced/all），由同页顶部的「可见级别」统一过滤。
 * `fv.aiHdEnhance` 默认 **关**：普通用户不关心；发行版可在随包的 `launcher.json` 里
 * 预置 `"hd": { "ai": { "enhance": true, "model": "realesrgan_anime6b.onnx" } }` 默认开启
 * （磁盘结构按用途分组，见 `vcmi/config.rs`；⚠️ 开关与模型缺一不可，**无默认模型**）。
 */
const FV_LAUNCHER: SettingItem[] = [
  { k: "fv.uiLang", id: "fv.uiLang", ctl: "select", o: ["auto", ...UI_LANG_IDS], level: "common", val: "auto" },
  { k: "fv.townPreview", id: "fv.townPreview", ctl: "switch", level: "common", val: false },
  { k: "fv.hdEnhance", id: "fv.hdEnhance", ctl: "switch", level: "all", val: false },
  { k: "fv.aiHdEnhance", id: "fv.aiHdEnhance", ctl: "switch", level: "all", val: false },
  { k: "fv.aiModel", id: "fv.aiModel", ctl: "select", o: [], level: "all", val: "", dynamic: "aiModels" },
  { k: "fv.hdEditionUse", id: "fv.hdEditionUse", ctl: "select", o: ["auto", "off"], level: "all", val: "auto" },
  { k: "fv.hdEditionScale", id: "fv.hdEditionScale", ctl: "select", o: ["auto", "x2"], level: "all", val: "auto" },
  { k: "fv.logLevel", id: "fv.logLevel", ctl: "select", o: ["error", "warn", "info", "debug", "trace"], level: "all", val: "info" },
  { k: "fv.networkProxy", id: "fv.networkProxy", ctl: "text", level: "all", val: "" },
  // L2 镜像规则表（D1 取代旧 fv.networkMirror 单行前缀）：结构复杂、level=all，
  // 不进本 schema，由设置页专用卡 `MirrorRulesCard`（FV启动器标签·全部级别）自带文案与保存通路。
];

/* ===================== 模组 Mods ===================== */
const MODS: SettingItem[] = [
  { k: "mods.validation", id: "mods.validation", ctl: "select", o: ["off", "basic", "full"], level: "all", val: "basic" },
];

/* ===================== 启动器 Launcher ===================== */
const LAUNCHER: SettingItem[] = [
  { k: "launcher.defaultRepositoryEnabled", id: "launcher.defaultRepositoryEnabled", ctl: "switch", level: "all", val: true },
  { k: "launcher.defaultRepositoryURL", id: "launcher.defaultRepositoryURL", ctl: "text", level: "all", val: "https://raw.githubusercontent.com/vcmi/vcmi-mods-repository/develop/vcmi-1.7.json", disabledWhen: { key: "launcher.defaultRepositoryEnabled", neq: true } },
  { k: "launcher.extraRepositoryEnabled", id: "launcher.extraRepositoryEnabled", ctl: "switch", level: "all", val: false },
  { k: "launcher.extraRepositoryURL", id: "launcher.extraRepositoryURL", ctl: "text", level: "all", val: "", disabledWhen: { key: "launcher.extraRepositoryEnabled", neq: true } },
  // T1 定案：机制与官方一致（true=启动后自动检查一轮），但 **FV 默认关**（缺失/false=纯手工，后端同语义）
  { k: "launcher.autoCheckRepositories", id: "launcher.autoCheckRepositories", ctl: "switch", level: "all", val: false },
  { k: "launcher.fullModExtraction", id: "launcher.fullModExtraction", ctl: "switch", level: "all", val: false },
  { k: "launcher.ignoreSslErrors", id: "launcher.ignoreSslErrors", ctl: "switch", level: "all", val: false },
  { k: "launcher.updateOnStartup", id: "launcher.updateOnStartup", ctl: "switch", level: "all", val: true },
  { k: "launcher.updateConfigUrl", id: "launcher.updateConfigUrl", ctl: "text", level: "all", val: "https://raw.githubusercontent.com/vcmi/vcmi-updates/master/vcmi-updates.json" },
  { k: "launcher.trackClipboardState", id: "launcher.trackClipboardState", ctl: "switch", level: "all", val: true },
];

/* ===================== 远程联机 Server ===================== */
const SERVER: SettingItem[] = [
  { k: "server.localHostname", id: "server.localHostname", ctl: "text", level: "all", val: "127.0.0.1" },
  { k: "server.localPort", id: "server.localPort", ctl: "number", min: 1, max: 65535, val: 3030, level: "advanced" },
  { k: "server.remoteHostname", id: "server.remoteHostname", ctl: "text", level: "all", val: "" },
  { k: "server.remotePort", id: "server.remotePort", ctl: "number", min: 1, max: 65535, val: 3030, level: "all" },
  { k: "server.seed", id: "server.seed", ctl: "number", min: 0, max: 2147483647, val: 0, level: "all" },
  { k: "server.replayRoundsKept", id: "server.replayRoundsKept", ctl: "number", min: 0, max: 100, val: 1, level: "all" },
];

/* ===================== 联机大厅 Lobby ===================== */
const LOBBY: SettingItem[] = [
  { k: "lobby.mapPreview", id: "lobby.mapPreview", ctl: "switch", level: "all", val: true },
  { k: "lobby.hostname", id: "lobby.hostname", ctl: "text", level: "all", val: "lobby.vcmi.eu" },
  { k: "lobby.port", id: "lobby.port", ctl: "number", min: 1, max: 65535, val: 3031, level: "all" },
  { k: "lobby.roomPlayerLimit", id: "lobby.roomPlayerLimit", ctl: "number", min: 1, max: 8, val: 2, level: "all" },
  { k: "lobby.roomType", id: "lobby.roomType", ctl: "number", min: 0, max: 10, val: 0, level: "all" },
  { k: "lobby.roomMode", id: "lobby.roomMode", ctl: "number", min: 0, max: 10, val: 0, level: "all" },
  { k: "lobby.languageRooms", id: "lobby.languageRooms", ctl: "json", level: "all", val: [] },
];

/* ===================== 日志 Logging ===================== */
const LOGGING: SettingItem[] = [
  { k: "logging.console.format", id: "logging.console.format", ctl: "text", level: "all", val: "%m" },
  { k: "logging.console.threshold", id: "logging.console.threshold", ctl: "select", o: ["trace", "debug", "info", "warn", "error"], level: "all", val: "info" },
  { k: "logging.console.coloredOutputEnabled", id: "logging.console.coloredOutputEnabled", ctl: "switch", level: "all", val: true },
  { k: "logging.console.colorMapping", id: "logging.console.colorMapping", ctl: "json", level: "all", val: [["trace", "green"], ["debug", "blue"], ["info", "default"], ["warn", "yellow"], ["error", "red"]] },
  { k: "logging.file.format", id: "logging.file.format", ctl: "text", level: "all", val: "[%c] %l [%t] %n - %m" },
  { k: "logging.loggers", id: "logging.loggers", ctl: "json", level: "all", val: [["global", "trace"], ["rng", "info"]] },
];

export const SETTING_TAGS: SettingTag[] = [
  { id: "general", icon: "Cog", items: GENERAL },
  { id: "video", icon: "MonitorPlay", items: VIDEO },
  { id: "adventure", icon: "Map", items: ADVENTURE },
  { id: "battle", icon: "Swords", items: BATTLE },
  { id: "gameTweaks", icon: "Wrench", items: GAME_TWEAKS },
  { id: "ai", icon: "Bot", items: AI },
  { id: "input", icon: "Mouse", items: INPUT },
  { id: "launcher", icon: "Rocket", items: LAUNCHER },
  { id: "mods", icon: "Package", items: MODS },
  { id: "server", icon: "Network", items: SERVER },
  { id: "lobby", icon: "Users", items: LOBBY },
  { id: "logging", icon: "ScrollText", items: LOGGING },
  // 第 13 个标签：**FV 启动器自身**的配置（launcher.json）。
  // 刻意排在最后、名字带 FV 前缀：与第 8 个「启动器」标签（VCMI settings.json 的 launcher 段）
  // 区分开 —— 后者是 VCMI 的启动器选项，前者才是本启动器的配置。
  { id: "fvLauncher", icon: "SlidersHorizontal", items: FV_LAUNCHER },
];

export const ALL_ITEMS: SettingItem[] = SETTING_TAGS.flatMap((s) => s.items);

/**
 * 官方默认值表：key -> 官方默认值。
 *
 * 数据来源改为「VCMI 官方 settings schema 自动生成」(src/generated/settingDefaults.ts)，
 * 由 `npm run gen:settings` 从 docs/plan/settings.json 提取，跟随 VCMI 源码迭代，
 * 不再手工逐行维护硬编码默认值。
 *
 * SETTING_DEFAULT_OVERRIDES 为「产品决策覆盖层」：仅承接那些刻意与 VCMI 源码默认值
 * 不同的键（例如启动器要求 useSavePrefix 默认关闭）。其余键一律以 schema 默认值为准。
 */
import { SETTING_SCHEMA_DEFAULTS, SETTING_SCHEMA_ENUM_OPTIONS, SETTING_SCHEMA_STRING_KEYS } from "@/generated/settingDefaults";

export { SETTING_SCHEMA_ENUM_OPTIONS, SETTING_SCHEMA_STRING_KEYS };

/** 产品决策覆盖层：刻意偏离 VCMI 官方默认值的键（需在此显式登记并注明原因） */
export const SETTING_DEFAULT_OVERRIDES: Record<string, unknown> = {
  // 启动器默认不启用存档前缀（源码默认 true），关闭时禁用输入框 —— 用户此前拍板决策
  "general.useSavePrefix": false,
};

export const OFFICIAL_DEFAULTS: Record<string, unknown> = {
  ...SETTING_SCHEMA_DEFAULTS,
  ...SETTING_DEFAULT_OVERRIDES,
};
