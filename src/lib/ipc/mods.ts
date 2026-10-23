/**
 * 模组仓库索引 + 领域模组引用管理（借用 / 引用 / 卸载 / 导入）。
 *
 * 拆分自原 `@/lib/tauri.ts` 单体；对外仍由该 barrel 统一导出，调用方无需改动。
 */
import { invoke } from "@tauri-apps/api/core";
import { feErr } from "@/lib/backendError";
import { isTauri } from "./core";

/* ----------------------------- 模组仓库（获取与更新） ----------------------------- */

/** 仓库索引条目（官方 `availableMods` 结构规范化 + FV 扩展字段，见 `repository/mods.rs`）。 */
export interface ModRepoEntry {
  id: string;
  modJsonUrl: string;
  downloadUrl: string;
  /** 官方条目为下载体积 MB 数字串；FV 聚合索引同位 */
  downloadSize?: unknown;
  githubStars?: unknown;
  screenshots?: unknown;
  /** FV 预聚合才有的元数据；空串 = 未预聚合（懒取兼容官方索引） */
  version: string;
  name: string;
  /** 官方索引条目不带名称/描述/类型：后端 enrich 逐条拉 mod.json 回填（与官方启动器同源） */
  description?: string;
  modType?: string;
  /** P11：翻译类模组基准语言（mod.json 的 `language`），enrich 从 mod.json 回填。
   *  前端据此 + modType 按类型（而非名字）发现语言包。 */
  language?: string;
  /** P10：抽屉详情展示作者（同样由 enrich 从 mod.json 回填） */
  author?: string;
  /** P10.1：mod.json 多语言块（enrich 回填），抽屉/名称按启动器语言选词 */
  i18n?: Record<string, { name?: string; description?: string } | null>;
  changelog?: Record<string, string[]> | null;
  sha256?: string | null;
  repositoryUrl: string;
  repositoryHost: string;
  /** 合并索引源标记（realm-mods-management §9.4）：default = 默认源；extra = 额外源 */
  sourceKind?: "default" | "extra";
  /** 条目实际来源索引地址（多源合并后逐条标记） */
  sourceUrl?: string;
}

export interface ModRepoIndex {
  mods: ModRepoEntry[];
  sources: { url: string; ok: boolean; error?: string | null; fetchedAt?: number | null; modCount: number }[];
  error?: string | null;
}

export interface ModInstallResult {
  ok: boolean;
  modId: string;
  version: string;
  modName: string;
  installedFiles: number;
  /** true = 替换已装版本（更新，保留启用态）；false = 新装（D1：默认不启用） */
  replaced: boolean;
  sha256: string;
  /** `verified` = sha256 强校验；`weak` = 弱校验（C2） */
  check: "verified" | "weak";
  warn: string[];
  dest: string;
}

export interface ModDownloadProgress {
  modId: string;
  received: number;
  total: number;
  done?: boolean;
}

/** 拉取模组仓库索引。TTL 内用缓存（零请求）；过期走 SWR（先返回缓存、后台校验）；`force` 阻塞全量触网。
 * `manual`（A1）：手动「检查更新」忽略 TTL 新鲜度，总是触发后台条件校验。 */
export async function fetchModIndex(force = false, manual = false): Promise<ModRepoIndex> {
  if (!isTauri()) return { mods: [], sources: [], error: feErr("frontend_desktop_only", "模组仓库") };
  return await invoke<ModRepoIndex>("fetch_mod_index", { force, manual });
}

/** 是否启用启动自动检查仓库（T1：机制同官方 `launcher.autoCheckRepositories`，但 FV 默认关=纯手工）。 */
export async function autoCheckReposEnabled(): Promise<boolean> {
  if (!isTauri()) return false;
  try {
    return await invoke<boolean>("auto_check_repos_enabled");
  } catch {
    // 读不到 = 视为未开启（默认关，与后端缺省语义一致）
    return false;
  }
}

/** 模组安装台账（管理页「来源/校验」列）；key = 模组目录名小写，无台账的模组不在其中（=随包/本地）。 */
export interface ModLedgerInfo {
  source: string;
  repositoryUrl: string;
  /** 实际出口：direct / mirror:<域名> / proxy */
  via: string;
  /** 校验等级：strong / weak */
  check: string;
  installedAt: string;
}

/** 批量读全局模组的 `.fv-install.json` 台账（进管理页时拉一次）。 */
export async function listModLedgers(): Promise<Record<string, ModLedgerInfo>> {
  if (!isTauri()) return {};
  try {
    return await invoke<Record<string, ModLedgerInfo>>("list_mod_ledgers");
  } catch {
    // 台账读取失败按"无台账"展示（不阻断管理页），与后端空结果语义一致
    return {};
  }
}

/** 物理删除模组目录（模组管理页；后端路径护栏：仅用户/游戏模组目录的直接子目录）。 */
export async function deleteMod(path: string): Promise<void> {
  if (!isTauri()) throw new Error(feErr("frontend_desktop_only", "删除模组"));
  await invoke("delete_mod", { path });
}

/** 从仓库安装/更新模组；`scope` 缺省为全局，`realm:<id>` 写领域私有副本（§4.4）。 */
export async function installModFromRepo(
  modId: string,
  scope?: string,
): Promise<ModInstallResult> {
  if (!isTauri()) throw new Error(feErr("frontend_desktop_only", "从仓库安装"));
  return await invoke<ModInstallResult>("install_mod_from_repo", { module: modId, scope });
}

/** 取消正在进行的模组下载。 */
export async function cancelModDownload(modId: string): Promise<void> {
  if (!isTauri()) return;
  await invoke("cancel_mod_download", { module: modId });
}

/** 订阅模组下载进度，返回取消订阅函数。 */
export async function onModDownloadProgress(
  cb: (p: ModDownloadProgress) => void,
): Promise<() => void> {
  if (!isTauri()) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  return await listen<ModDownloadProgress>("mod-download-progress", (e) => cb(e.payload));
}

/** 模组下载取消哨兵错误串（后端约定）。 */
export const MOD_DOWNLOAD_CANCELLED = "MOD_DOWNLOAD_CANCELLED";

/** 检查更新两阶段进度（P9，D2 定案）：index = 逐源拉索引；meta = 批量补全 mod.json 元数据 */
export interface ModIndexProgress {
  phase: "index" | "meta";
  done: number;
  total: number;
}

/** 订阅检查更新进度（mod-index-progress），返回取消订阅函数。 */
export async function onModIndexProgress(
  cb: (p: ModIndexProgress) => void,
): Promise<() => void> {
  if (!isTauri()) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  return await listen<ModIndexProgress>("mod-index-progress", (e) => cb(e.payload));
}

/** 订阅「索引已更新」事件（A1 SWR：后台校验发现远端索引真变后触发，缓存已刷新）——
 * 收到后重拉 `fetchModIndex(false)` 即为 TTL 内秒回。返回取消订阅函数。 */
export async function onModIndexUpdated(cb: () => void): Promise<() => void> {
  if (!isTauri()) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  return await listen("mod-index-updated", () => cb());
}

/** 领域沙箱模组操作：`unlink` 删私有副本 / `link` 重建借用联接。
 * realm-mods-management 重构后 UI 入口改由 `realmModUnref` / `realmModDelete` 承担；
 * 本命令作为底层能力保留（供后续「删除私有副本恢复引用」等入口复用）。 */
export async function realmModOps(
  realmId: string,
  modId: string,
  action: "unlink" | "link",
): Promise<{ ok: boolean; removed?: boolean; hasGlobal?: boolean; linked?: boolean; error?: string }> {
  if (!isTauri()) throw new Error(feErr("frontend_desktop_only", "领域模组操作"));
  return await invoke("realm_mod_ops", { realmId, module: modId, action });
}

/* ----------------------------- 领域模组管理（realm-mods-management） ----------------------------- */

/** 领域已实装模组条目（`list_realm_mods`：领域目录扫描 ∪ 台账补漏；含子模组）。 */
export interface RealmInstalledMod {
  id: string;
  /** 磁盘目录名（与 id 可能大小写不同） */
  diskName?: string;
  name?: string;
  desc?: string;
  type?: string;
  rawType?: string | null;
  version?: string | null;
  author?: string | null;
  depends?: string[];
  softDepends?: string[];
  conflicts?: string[];
  factions?: string[];
  /** mod.json 多语言块（scan 产物；名称/描述按语言选词） */
  i18n?: Record<string, { name?: string; description?: string } | null>;
  isSubmod?: boolean;
  keepDisabled?: boolean;
  /** mod.json 的 language（翻译类模组基准语言）：领域感知的语言包发现依赖它 */
  language?: string | null;
  /** 是否为基础游戏语言包（区分「基础语言包」与「内容汉化」） */
  baseTranslation?: boolean;
  /** 目录联接（user 源引用） */
  isLink: boolean;
  /** `.fv-install.json` 安装台账（下载 / 导入所得） */
  hasLedger: boolean;
  /** 来源：realm = 领域位置；user / game = 引用来源 */
  source: "realm" | "user" | "game";
  inBorrowed: boolean;
  inBundled: boolean;
  inPinned: boolean;
  /** 能否解析到 mod.json（false = 引用缺失，仅有 id） */
  metaOk: boolean;
}

/**
 * 领域语言包候选（`realm_lang_pack_candidates` 产物，`scan_mod_dir` 元数据子集 + source）。
 * 只含「`modType=Translation` + `language` 匹配当前 VCMI 语言」的**顶层**模组。
 */
export interface RealmLangPackCandidate {
  id: string;
  name?: string;
  /** mod.json 多语言块（名称按语言选词） */
  i18n?: Record<string, { name?: string; description?: string } | null>;
  /** mod.json 的 modType 原文（判定必须用真值，不用推断的 type） */
  rawType?: string | null;
  /** 翻译类模组的基准语言 */
  language?: string | null;
  isSubmod?: boolean;
  /** 是否为基础游戏语言包（提供 VCMI 基础翻译作用域文件的硬签名） */
  baseTranslation?: boolean;
  /** 来源：realm = 领域位置；game = 游戏目录（天然可见）；user = 全局用户目录（仅未隔离时返回） */
  source: "realm" | "game" | "user";
}

/** 领域引用 / 取消引用逐条结果。 */
export interface RealmRefResult {
  id: string;
  ok: boolean;
  /** 来源解析结果（user 优先，对齐全局扫描口径） */
  source?: "user" | "game";
  /** ok=false 时：missing（user/game 目录均无） */
  reason?: string;
  /** unref：是否实际删除了目录联接（仅领域激活态下可能为 true） */
  removed?: boolean;
}

/** 领域模组整合（复制）结果（`realm_mod_bundle`）。 */
export interface RealmBundleResult {
  ok: boolean;
  modId: string;
  source?: "user" | "game";
  /** ok=false 时的冲突原因：game（游戏目录恒可见重复挂载）/ diverged（指纹不一致） */
  reason?: string;
  /** ok=true 且指纹一致（已整合过） */
  skipped?: "same";
  /** ok=true 且替换了旧副本 */
  replaced?: boolean;
}

/** 领域模组导入逐条结果（`realm_mod_import`）。 */
export interface RealmImportResult {
  path: string;
  ok: boolean;
  modId?: string;
  diskName?: string;
  name?: string;
  version?: string;
  /** ok=false 且目标已存在（overwrite=false 探询）：二次确认后以 overwrite=true 重调 */
  exists?: boolean;
  replaced?: boolean;
  files?: number;
  /** ok=false 的原因（not-mod / bad-name / missing / 具体错误串） */
  reason?: string;
}

/** 领域已实装模组列表（领域目录扫描 ∪ 台账补漏）。 */
export async function listRealmMods(realmId: string): Promise<RealmInstalledMod[]> {
  if (!isTauri()) return [];
  const r = await invoke<{ mods: RealmInstalledMod[] }>("list_realm_mods", { realmId });
  return r.mods ?? [];
}

/** 批量引用：纯台账申报（只校验来源存在性；user 源目录联接由激活领域时统一建立）。 */
export async function realmModRef(realmId: string, modIds: string[]): Promise<RealmRefResult[]> {
  if (!isTauri()) throw new Error(feErr("frontend_desktop_only", "领域模组管理"));
  const r = await invoke<{ results: RealmRefResult[] }>("realm_mod_ref", { realmId, modIds });
  return r.results ?? [];
}

/** 批量取消引用（台账移除；仅领域激活态下同步删目录联接，真实目录绝不删除）。 */
export async function realmModUnref(realmId: string, modIds: string[]): Promise<RealmRefResult[]> {
  if (!isTauri()) throw new Error(feErr("frontend_desktop_only", "领域模组管理"));
  const r = await invoke<{ results: RealmRefResult[] }>("realm_mod_unref", { realmId, modIds });
  return r.results ?? [];
}

/**
 * 确保领域在当前语言下**能看到**匹配的语言包（语言层：启动器自有，**不写领域数据**）。
 *
 * - 扫描含**领域自身 Mods**（`scan_mods` 只扫全局 user + 游戏目录，看不到领域自带的 bundled/pinned，
 *   故必须由后端补上），按 领域自带 → 游戏目录 → 全局 user 的优先级取用（基础语言包优先）；
 * - 领域隔离且只在全局 user 时，于 `{realm}/vcmi/Mods/<id>` 建一条**临时联接**（不落 `realm.json`
 *   台账，下次激活会被清理，每次启动前重建即可）；
 * - 领域未隔离（`dataPath=global`）、游戏目录已有、领域自带 → 零动作（本就可见）。
 *
 * 返回 `linked=true` 表示本次**新建立**了联接（此前对该领域不可见），上层据此把它自动并入生效预设。
 * 找不到返回 `{found:false}`，交由上层走仓库安装兜底（装到全局即可）。仅确认已安装的包，不下载。
 */
export async function ensureRealmLangVisible(
  realmId: string,
  vcmiLang: string,
): Promise<{
  found: boolean;
  id?: string;
  source?: "realm" | "user" | "game";
  linked?: boolean;
}> {
  if (!isTauri()) return { found: false };
  return await invoke<{
    found: boolean;
    id?: string;
    source?: "realm" | "user" | "game";
    linked?: boolean;
  }  >("ensure_realm_lang_visible", { realmId, vcmiLang });
}

/**
 * 领域语言包候选（**只读**，三源扫描：领域自带 / 游戏目录 / 全局 user[未隔离时]）——
 * 启动前预检与语言同步的发现源。
 *
 * ⚠️ 为什么不能用 `listRealmMods`：那只汇总「领域 Mods 目录物理条目 + `realm.json` 台账
 * 登记的引用」，**游戏目录 `Mods/` 下未登记的 game 源语言包完全不可见**——游戏目录明明
 * 装着中文包却报"未找到"（2026-10-22 实测坑）。本命令按 VCMI 实际可见性扫描，与后端
 * `ensure_realm_lang_visible` 共用同一收集口径（`collect_lang_cands`），两处永不分裂。
 *
 * 领域隔离（`dataPath=realm`，默认）时**不含**全局 user 源候选：它要经
 * `ensureRealmLangVisible` 建临时联接后才对 VCMI 可见，提前报"已安装"会把读不到的
 * id 写进预设。该场景由 ensure 的 `linked` 结果另行并入（见 `syncLanguageMods`）。
 */
export async function realmLangPackCandidates(
  realmId: string,
  vcmiLang: string,
): Promise<RealmLangPackCandidate[]> {
  if (!isTauri()) return [];
  const r = await invoke<{ candidates: RealmLangPackCandidate[] }>(
    "realm_lang_pack_candidates",
    { realmId, vcmiLang },
  );
  return r.candidates ?? [];
}

/** 整合（复制本体到领域 Mods）。`force=true` 跳过 game / diverged 冲突检测（用户在确认框选择继续）。 */export async function realmModBundle(
  realmId: string,
  modId: string,
  force = false,
): Promise<RealmBundleResult> {
  if (!isTauri()) throw new Error(feErr("frontend_desktop_only", "领域模组管理"));
  return await invoke<RealmBundleResult>("realm_mod_bundle", { realmId, modId, force });
}

/** 删除领域位置副本（pinned / bundled）。联接安全删除（remove_dir），真实目录递归删除。 */
export async function realmModDelete(
  realmId: string,
  modId: string,
  kind: "bundled" | "pinned" | "local",
): Promise<{ ok: boolean; removed: boolean }> {
  if (!isTauri()) throw new Error(feErr("frontend_desktop_only", "领域模组管理"));
  return await invoke<{ ok: boolean; removed: boolean }>("realm_mod_delete", { realmId, modId, kind });
}

/** 导入模组文件（zip / 含 mod.json 的目录）到领域 Mods，落 pinned 台账（source: "import"）。 */
export async function realmModImport(
  realmId: string,
  paths: string[],
  overwrite = false,
): Promise<RealmImportResult[]> {
  if (!isTauri()) throw new Error(feErr("frontend_desktop_only", "领域模组管理"));
  const r = await invoke<{ results: RealmImportResult[] }>("realm_mod_import", {
    realmId,
    paths,
    overwrite,
  });
  return r.results ?? [];
}
