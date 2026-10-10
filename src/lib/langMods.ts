import {
  fetchModIndex,
  isTauri,
  listRealmMods,
  patchPresetLangMods,
  readModSettingsJson,
  readVcmiSettings,
  realmConfigDir,
  scanRealMods,
  setVcmiLanguage,
  ensureRealmLangVisible,
} from "./tauri";
import type { ModRepoEntry } from "./tauri";
import { localizedName, type ModInfo } from "./modScanner";
import { backendErrorText } from "./backendError";

/**
 * 语言模组层：把「当前 VCMI 语言匹配的翻译类模组」同步进生效预设。
 *
 * 设计要点（相对旧版的重大调整，详见 docs/features/lang-mods.zh-CN.md）：
 * - 旧版按内置预设 `modsByLang` 写死的模组名字映射（如 `simplified-chinese-translation`），
 *   但发行版碎片化使中文包名字无规范（增强版 / Expansion / 官方各不相同）。
 * - 新版**按类型+语言发现**，不依赖任何模组 id 名字：从已装模组中筛选
 *   `rawType=="Translation" && language==当前语言` 的模组并启用。VCMI 标准化的是
 *   翻译模组的分类口径（modType + language），不是 id，故对一切发行版通用。
 * - 不再主动移除「其它语言的翻译模组」：VCMI 自身会按 language trait 过滤不匹配的
 *   翻译模组（ModManager.cpp:829-832），启动器移除反而可能与 VCMI 写盘打架。
 * - 两层缺一不可：① 语言设置层 `setVcmiLanguage`；② 预设层把匹配翻译模组写进预设。
 *   缺①会导致 VCMI 把语言不符的翻译模组判 BROKEN 并移除。
 * - ② 的静默启用**只在「语言真正切换」时发生**（对比切换前 settings.json 的
 *   `general.language`）。语言未变（如每次启动恢复）时不强行加回被用户显式关闭的
 *   语言包——否则用户的关闭会被下次启动悄悄撤销。该情况交由启动弹窗建议（Tier 3）。
 * - 目标预设：领域取 `realm.json` 的 `presetId`，非领域取 `modSettings.activePreset`。
 * - 幂等，可安全在切语言 / 激活领域 / 启动恢复等时机重复调用。
 * - 「完全没装匹配语言包」不在此处处理（此处只启用已装模组），改由启动前的 Tier 3
 *   检查（见 `checkLaunchLangMod` / 启动弹窗）提供仓库安装建议。
 */

/** 领域预设 id 兜底（与 `realms.rs` 的 REALM_PRESET_ID 一致）。 */
const REALM_PRESET_FALLBACK = "__realm__";

/* ===================== 会话级（**不落盘**）用户选择 =====================
 * 启动前预检每次启动都会跑，若用户每次都选「直接启动」却仍被反复提示，体验比"奇怪"更糟。
 * 因此把用户在该弹窗里的选择记在**进程内存**里，本次会话内生效，重启即复位：
 * - `sessionSkipCheck`：选了「直接启动」→ 本会话不再预检、直接放行。
 * - `sessionAutoEnable`：勾了「以后自动启用」→ 本会话内静默启用匹配语言包，且不再预检。
 * 不落盘是刻意的：语言包状态随时会变（用户装/卸模组），持久化的"不再提示"会变成
 * 长期静默失效，反而更难排查。
 */
let sessionSkipCheck = false;
let sessionAutoEnable = false;

/** 本次会话是否应跳过启动前预检（用户已选择直接启动 / 已选以后自动启用）。 */
export function shouldSkipLangCheck(): boolean {
  return sessionSkipCheck || sessionAutoEnable;
}

/** 记录「直接启动」：本会话内不再提示。 */
export function setSessionSkipLangCheck(v: boolean): void {
  sessionSkipCheck = v;
}

/** 记录「以后自动启用」：本会话内静默启用，不再提示。 */
export function setSessionAutoEnable(v: boolean): void {
  sessionAutoEnable = v;
}

/** 本会话内是否应对匹配语言包静默启用（用户勾了「以后自动启用」）。 */
export function sessionAutoEnabled(): boolean {
  return sessionAutoEnable;
}

/**
 * 从已装模组中筛选「语言 == vcmiLang 的**基础游戏语言包**」id（按类型+语言，**并区分基础包与汉化包**）。
 *
 * ⚠️ **只认父模组本身（基础语言包），绝不递归子模组。**
 * VCMI 模组设计过于自由：一个父模组类型标 `Translation`，其真正的其它类型能力
 * （字体/UI/地图等）往往由子模组提供。因此我们只取「父模组自身的 `rawType`/`language`」，
 * 把那一个父 id 写进预设，VCMI 的依赖解析会把子模组一并带起。
 * - 不把子模组枚举为独立语言包（子模组的 `rawType`/`language` 无论是什么都不看）。
 * - 父模组自身没声明 `language` 时直接排除（不回退到子模组去"挖"语言），这是刻意口径。
 * 防御：`scanRealMods()` 本就返回顶层列表（`subs` 嵌套在父内），这里再叠 `!isSubmod`
 * 防止将来若改成展平列表时误把子模组当语言包。
 *
 * ⚠️ **必须严格用 `rawType`（mod.json `modType` 原文）**，绝不能回落到推断的 `type`：
 * `type` 是后端 `classify_mod` 按名字/描述关键词**推断**的，会把名字里带 "chinese" 的
 * **普通内容模组**（如 `new-chinese-mods` / `old-chinese-mods`）误判成 Translation，
 * 从而被错误地当作语言包列出并启用（实测反馈，与 `ModCard.tsx` 的语言不符角标同一条铁律）。
 * `rawType` 为空 = 作者没声明翻译类型 → 不属于语言包，直接排除。
 *
 * ⚠️ **`modType == "Translation"` 只是"这是个翻译模组"的必要条件，不是"这是基础语言包"的充分条件。**
 * 中文社区常把内容模组的汉化打包成独立模组（如 `new-chinese-mods` / `old-chinese-mods`），
 * 它们同样声明 `modType=="Translation"` + `language=="chinese"`，但**只翻译内容模组、不翻译游戏本体**。
 * 真正的"基础游戏语言包"（如 `simplified-chinese-translation`）需进一步确认——用两个正交信号（任一命中即认）：
 *   1. **目录结构标记** `baseTranslation`：后端探测到该模组提供了 VCMI 基础翻译作用域文件
 *      `content/translation/{game,campaigns,maps,chronicles}.json`（这是 VCMI 把游戏/战役/地图/历代记
 *      文本翻译出来的硬签名；汉化包的文件是按被翻译模组命名的，不会有这些固定文件）。
 *   2. **VCMI 官方命名契约** `id` 形如 `<language>-translation`（官方强制约定，如
 *      `simplified-chinese-translation` / `chinese-translation-vcmi`）。
 * 两者都不中（非规范发行版，如 Expansion 的 `ChineseTrueTypeFonts` 不暴露上述结构/命名）时，
 * 退回"任意满足 `rawType+language` 的顶层模组"宽松集合，保证不失效、不回归。
 */
/** VCMI 官方基础语言包命名契约：mod id 形如 `<language>-translation`。 */
const BASE_TRANSLATION_ID = /(?:^|[-_\s])translation(?:[-_\s]|$)/i;

/** 语言包候选最小字段集（`scanRealMods` 的 `ModInfo` 与 `listRealmMods` 的 `RealmInstalledMod` 公共部分）。 */
export interface LangPackCandidate {
  id: string;
  name?: string;
  i18n?: Record<string, { name?: string } | null>;
  rawType?: string | null;
  language?: string | null;
  isSubmod?: boolean;
  baseTranslation?: boolean;
}

/** 是否为基础游戏语言包（结构标记优先，命名契约兜底）。 */
function isBaseGameLangPack(m: LangPackCandidate): boolean {
  if (m.baseTranslation) return true;
  return BASE_TRANSLATION_ID.test(m.id ?? "");
}

/**
 * 按「类型+语言」筛出语言包 id（基础语言包优先，否则退回宽松集合）。
 * 全局 `scanRealMods` 与领域 `listRealmMods` 两个数据源**同口径**，避免两处判定分叉。
 */
export function pickLangPacks(vcmiLang: string, list: LangPackCandidate[]): string[] {
  const cands = list.filter(
    (m) => !m.isSubmod && m.rawType === "Translation" && (m.language ?? "") === vcmiLang,
  );
  const base = cands.filter(isBaseGameLangPack);
  // 命中基础语言包就只认它；否则退回宽松集合（非规范发行版兜底）
  return (base.length > 0 ? base : cands).map((m) => m.id);
}

export function translationModsForLang(vcmiLang: string, installed: ModInfo[]): string[] {
  return pickLangPacks(vcmiLang, installed);
}

/**
 * 语言包发现源（**领域感知**）：
 * - 领域模式用 `list_realm_mods`：它含领域自带（`bundled` 复制 / `pinned` 私有副本）、借用联接
 *   与 game 源，是「VCMI 在领域内实际能看到什么」的权威口径；
 *   ⚠️ **不能用 `scanRealMods`**——它按 `AppPaths` 只扫「全局 user + 游戏目录」，看不到领域自身
 *   Mods，会把领域自带的语言包误判为缺失、反复催促安装并重复装一份全局包。
 * - 全局模式用 `scanRealMods`（全局 user + 游戏目录）。
 */
async function discoverLangPacks(
  realmId: string | null | undefined,
): Promise<LangPackCandidate[]> {
  if (realmId) {
    try {
      return await listRealmMods(realmId);
    } catch {
      return [];
    }
  }
  try {
    return (await scanRealMods()) as ModInfo[];
  } catch {
    return [];
  }
}

/**
 * 从仓库索引条目中筛选「语言 == vcmiLang 的翻译类模组」（供启动前 Tier 3 建议安装用）。
 * 索引条目需经 enrich 回填 `modType` / `language`（见 `repository/mods.rs`）；若某条目缺
 * `language`（纯缓存未回填），则退化为仅按 `modType=="Translation"` 过滤，由调用方让用户选择。
 *
 * 同样遵循「只认基础语言包」口径（见 `translationModsForLang`）：仓库索引只列顶层模组，
 * 返回的候选即父模组本身；下载并启用时只装这一个父 id，子模组由 VCMI 依赖解析带入。
 */
export function repoTranslationMods(
  index: ModRepoEntry[],
  vcmiLang: string,
): ModRepoEntry[] {
  const out: ModRepoEntry[] = [];
  for (const e of index) {
    const modType = e.modType ?? "";
    const lang = e.language ?? "";
    if (modType !== "Translation") continue;
    if (lang && lang !== vcmiLang) continue;
    out.push(e);
  }
  return out;
}

export interface SyncLangModsOptions {
  /** VCMI 语言标识（chinese / english ...），取自 `useI18n()` 的 `vcmiLang` */
  vcmiLang: string;
  /** 激活领域 id；为空 / null 表示非领域（全局）模式 */
  realmId?: string | null;
  /** 领域预设 id（`realm.json.presetId`），优先使用；缺省回退 `activePreset` / `__realm__` */
  presetId?: string;
  /**
   * 强制启用：跳过「尊重用户显式关闭」的闸门。
   * 用于**用户明确点了「启用并启动」/「下载并启用」**的场景——此时启用就是用户当下的
   * 显式意图，不该被"语言未切换"判定挡掉（否则点了按钮却没启用）。
   */
  force?: boolean;
}

export interface SyncLangModsResult {
  ok: boolean;
  /** 无事可做（非 Tauri 环境 / 当前语言无匹配语言包 / 尊重用户显式关闭）时为 true */
  skipped: boolean;
  target: "realm" | "global";
  /** 已加入但磁盘上找不到的模组 id（理论为空：add 仅含已装模组） */
  missing: string[];
  /** 因「语言未切换 + 语言包被用户关闭」而故意不加回（避免与用户显式选择打架） */
  respected?: boolean;
  error?: string;
}

/**
 * 读取目标配置目录当前的 `general.language`（settings.json）。
 * 不用 `loadVcmiLanguage`：它有模块级缓存，切换配置目录（全局 ↔ 领域）时会读到旧值。
 */
async function currentOnDiskLanguage(configDir: string | undefined): Promise<string | undefined> {
  try {
    const s = await readVcmiSettings(configDir);
    const raw = (s?.general as Record<string, unknown> | undefined)?.language;
    return typeof raw === "string" && raw.trim() ? raw.trim() : undefined;
  } catch {
    return undefined;
  }
}

/**
 * 把当前语言的翻译模组同步进生效预设（启用已装的匹配包）。
 * 失败只记录不抛出——语言模组属「锦上添花」，不应阻断进入领域或切换语言。
 */
export async function syncLanguageMods(opts: SyncLangModsOptions): Promise<SyncLangModsResult> {
  const target: "realm" | "global" = opts.realmId ? "realm" : "global";

  if (!isTauri()) {
    return { ok: true, skipped: true, target, missing: [] };
  }

  // 发现匹配语言包（领域感知：领域模式含领域自带/借用/game 源；全局模式为全局 user + game。
  // 仅已装模组，故 add 一定存在于对应 mod 目录，不会触发 missing）。
  let add = pickLangPacks(opts.vcmiLang, await discoverLangPacks(opts.realmId));

  try {
    let configDir: string | undefined;
    // 本次**新**建立可见性（此前对该领域不可见）的语言包 id：不存在"用户显式关闭"一说，
    // 故可绕开下方「尊重用户显式关闭」闸门自动并入生效预设。
    let newlyVisibleId: string | undefined;
    if (opts.realmId) {
      const dir = await realmConfigDir(opts.realmId);
      // 非 Tauri 环境返回 null：此时不应报错，直接跳过
      if (!dir) return { ok: true, skipped: true, target, missing: [] };
      configDir = dir;
      // 语言层（启动器自有、不写领域数据）：确保领域能看到当前语言的语言包。
      // 领域自带 / 游戏目录 / 领域未隔离 → 已可见，零动作；领域隔离且只在全局 user → 临时建联接。
      // 失败不阻断：语言包属锦上添花，不应卡住启动。
      try {
        const r = await ensureRealmLangVisible(opts.realmId, opts.vcmiLang);
        if (r.found && r.id) {
          if (r.linked) newlyVisibleId = r.id;
          // 刚建联接时领域扫描尚看不到它——手动并入待启用集合，让下方按同一套预设回退链注入；
          // 启动时 VCMI 经联接读到该包。
          if (!add.includes(r.id)) add = [...add, r.id];
        }
      } catch {
        /* 语言层失败忽略，交给下方流程 */
      }
    }

    // 先记录切换**前**的语言：仅在「语言真正发生切换」时才静默启用匹配语言包。
    // 语言未变（如每次启动恢复）时不强行加回——否则用户在 ModView 里显式关闭的
    // 语言包会在下次启动被偷偷加回来，与用户选择打架。该情况改由启动弹窗建议（Tier 3）。
    const onDiskLang = await currentOnDiskLanguage(configDir);
    const languageChanged = !onDiskLang || onDiskLang !== opts.vcmiLang;

    // ① 语言设置层（**必须先于**预设注入）：把 `general.language` 同步成启动器语言。
    //    否则 VCMI 会把语言不匹配的 Translation 模组判为 BROKEN，并在启动时把它
    //    从激活预设中移除（ModManager.cpp:829-832）——步骤②的注入等于白写。
    await setVcmiLanguage(opts.vcmiLang, configDir);

    // ② 预设层：无匹配语言包可加时，无需读写 modSettings
    if (add.length === 0) {
      return { ok: true, skipped: true, target, missing: [] };
    }

    // 语言未切换：匹配语言包已装但未启用，视为用户**显式关闭**，不强行加回。
    // 启动时的 Tier 3 检查会给出「已安装但未启用，是否启用并启动？」的建议。
    // `force` 表示用户在弹窗里明确点了启用；「以后自动启用」则让本会话内都按此处理。
    // `newlyVisibleId` 例外：该包本轮才刚对本领域可见，不存在"用户显式关闭"。
    if (!languageChanged && !opts.force && !sessionAutoEnabled() && !newlyVisibleId) {
      return { ok: true, skipped: true, target, missing: [], respected: true };
    }

    let presetId = opts.presetId;
    if (opts.realmId) {
      const s = await readModSettingsJson(configDir);
      const presets = s?.presets as Record<string, unknown> | undefined;
      // 领域模式下同样要求预设已存在：尚未初始化时不抢写，交给激活流程。
      if (!s || !presets || Object.keys(presets).length === 0) {
        return { ok: true, skipped: true, target, missing: [] };
      }
      const ap = s.activePreset as string | undefined;
      // **以 `activePreset` 为准**：领域支持多个预设，玩家/编辑器可在领域内切换，而 VCMI 启动只认
      // `activePreset`。`realm.json.presetId`（默认 `__realm__`）只是领域自带预设的种子 id，
      // 不能优先于实际生效者——否则切到其它预设后注入会写到 `__realm__` 上，"补了却没生效"。
      // 回退链：activePreset → presetId → activePreset → presetId → __realm__
      presetId =
        (ap && presets[ap] ? ap : undefined) ??
        (opts.presetId && presets[opts.presetId] ? opts.presetId : undefined) ??
        ap ??
        opts.presetId ??
        REALM_PRESET_FALLBACK;
    } else {
      const s = await readModSettingsJson();
      const presets = s?.presets as Record<string, unknown> | undefined;
      // 全局模式：尚无预设时不主动创建 modSettings.json。
      if (!s || !presets || Object.keys(presets).length === 0) {
        return { ok: true, skipped: true, target, missing: [] };
      }
      presetId = String((s?.activePreset as string | undefined) ?? "") || "default";
    }

    const missing = await patchPresetLangMods(configDir, presetId, add, []);
    return { ok: true, skipped: false, target, missing };
  } catch (e) {
    return {
      ok: false,
      skipped: false,
      target,
      missing: [],
      error: backendErrorText(e),
    };
  }
}

/** 启动前语言包检查结果（Tier 3 建议性提示的依据）。 */
export type LaunchLangModStatus =
  | "ok" // 已有匹配语言包启用，正常启动
  | "installed-disabled" // 已装但未启用，建议启用后启动
  | "missing"; // 完全没装（仓库候选由 resolveLangPackRepo 异步补全，不在此联网）

export interface CheckLaunchLangModOptions {
  vcmiLang: string;
  realmId?: string | null;
  presetId?: string;
}

export interface LaunchLangModCheck {
  status: LaunchLangModStatus;
  /** 当前语言匹配的已装翻译模组 id（含已启用与未启用） */
  matchingIds: string[];
  /** 其中已在生效预设中启用的 id */
  enabledMatching: string[];
  /** 已装匹配语言包的展示名（名称（id）），供弹窗告知用户「将启用哪个」；多个用「、」连接 */
  matchingLabels: string[];
  /** 仓库候选（status==="missing-repo" 时非空） */
  repoCandidates: ModRepoEntry[];
}

async function resolvePresetId(
  realmId: string | null | undefined,
  configDir: string | undefined,
  presetId: string | undefined,
): Promise<string | undefined> {
  if (realmId) {
    const s = await readModSettingsJson(configDir);
    const presets = s?.presets as Record<string, unknown> | undefined;
    if (!s || !presets || Object.keys(presets).length === 0) return undefined;
    const ap = s.activePreset as string | undefined;
    // 同 `syncLanguageMods`：领域以 `activePreset` 为准（多预设下 VCMI 只认它）
    return (
      (ap && presets[ap] ? ap : undefined) ??
      (presetId && presets[presetId] ? presetId : undefined) ??
      ap ??
      presetId ??
      REALM_PRESET_FALLBACK
    );
  }
  const s = await readModSettingsJson();
  const presets = s?.presets as Record<string, unknown> | undefined;
  if (!s || !presets || Object.keys(presets).length === 0) return undefined;
  return String((s?.activePreset as string | undefined) ?? "") || "default";
}

/**
 * 启动前「本地」扫描：当前语言是否有「已启用」的匹配翻译包。
 * 纯本地（扫已装 + 读预设），毫秒级、**不联网**。
 * 不阻断——返回状态由调用方决定弹窗或直接启动（全 ok 时不弹窗）。
 *
 * 仓库候选（status==="missing" 时是否需要）由 `resolveLangPackRepo` **异步**补全，
 * 不在此处联网，避免阻塞「启动前检测」弹窗的出现（详见 `docs/features/lang-mods` §10.3）。
 */
export async function scanLangPack(
  opts: CheckLaunchLangModOptions,
): Promise<LaunchLangModCheck> {
  const empty: LaunchLangModCheck = {
    status: "ok",
    matchingIds: [],
    enabledMatching: [],
    matchingLabels: [],
    repoCandidates: [],
  };
  // 本会话内用户已选择「直接启动」或「以后自动启用」→ 不再预检，直接放行
  if (!isTauri() || shouldSkipLangCheck()) return empty;

  // 领域感知发现：领域自带（bundled/pinned）的语言包必须算"已有"，否则会被误判缺失、
  // 每次启动都催促安装（而领域其实已经能翻译）。
  const discovered = await discoverLangPacks(opts.realmId);
  const matching = pickLangPacks(opts.vcmiLang, discovered);
  // 生成「名称（id）」展示串：让用户清楚到底会启用哪个语言包（常见仅 1 项）
  const matchingLabels = matching.map((id) => {
    const mod = discovered.find((m) => m.id === id);
    const nm = mod ? localizedName(mod as unknown as ModInfo, opts.vcmiLang) : "";
    return nm && nm !== id ? `${nm}（${id}）` : id;
  });

  // 解析生效预设，判断匹配包是否已在其中启用
  let configDir: string | undefined;
  if (opts.realmId) {
    const dir = await realmConfigDir(opts.realmId);
    if (!dir) return empty;
    configDir = dir;
  }
  const presetId = await resolvePresetId(opts.realmId, configDir, opts.presetId);
  let enabledMatching: string[] = [];
  if (presetId) {
    const s = await readModSettingsJson(configDir);
    const preset = (s?.presets as Record<string, { mods?: string[] }> | undefined)?.[presetId];
    const mods = new Set(preset?.mods ?? []);
    enabledMatching = matching.filter((id) => mods.has(id));
  }

  if (enabledMatching.length > 0) {
    return {
      status: "ok",
      matchingIds: matching,
      enabledMatching,
      matchingLabels,
      repoCandidates: [],
    };
  }
  if (matching.length > 0) {
    return {
      status: "installed-disabled",
      matchingIds: matching,
      enabledMatching,
      matchingLabels,
      repoCandidates: [],
    };
  }
  // 完全没装：本地到此为止，仓库候选交给resolveLangPackRepo异步补全
  return {
    status: "missing",
    matchingIds: [],
    enabledMatching: [],
    matchingLabels: [],
    repoCandidates: [],
  };
}

/** 仓库候选查询超时（ms）：启动前检查是建议性的，拿不到就当"仓库不可用"，不应长时间阻塞。 */
const REPO_QUERY_TIMEOUT_MS = 10_000;

/**
 * 启动前「仓库候选」异步补全：仅当本地 `missing` 时由弹窗内调用。
 * 带 10s 短超时兜底（失败/超时返回空，交由 UI 显示"仓库不可用"），
 * 不冻结按钮、不阻塞「跳过并启动」（详见 `docs/features/lang-mods` §10.3）。
 */
export async function resolveLangPackRepo(vcmiLang: string): Promise<ModRepoEntry[]> {
  try {
    const idx = await Promise.race([
      fetchModIndex(false, false),
      new Promise<never>((_, reject) =>
        setTimeout(() => reject(new Error("repo_query_timeout")), REPO_QUERY_TIMEOUT_MS),
      ),
    ]);
    return repoTranslationMods(idx.mods, vcmiLang);
  } catch {
    return [];
  }
}
