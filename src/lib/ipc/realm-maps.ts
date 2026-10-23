/**
 * 领域地图收纳（清单 / 导入 / 阀门 / 上传删除）、素材与登场演出，以及领域导入。
 *
 * 拆分自原 `@/lib/tauri.ts` 单体；对外仍由该 barrel 统一导出，调用方无需改动。
 */
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { save as saveDialog } from "@tauri-apps/plugin-dialog";
import { feErr } from "@/lib/backendError";
import { isTauri } from "./core";

/** 弹出「保存文件」对话框（Tauri 桌面端），返回路径或 null（取消）。 */
export async function pickSavePath(defaultPath: string, filters?: { name: string; extensions: string[] }[]): Promise<string | null> {
  if (!isTauri()) return null;
  try {
    return await saveDialog({ defaultPath, filters });
  } catch {
    return null;
  }
}

/** 弹出「打开文件」对话框（Tauri 桌面端），返回路径或 null（取消）。 */
export async function pickOpenPath(filters?: { name: string; extensions: string[] }[]): Promise<string | null> {
  if (!isTauri()) return null;
  try {
    return await openDialog({ multiple: false, filters });
  } catch {
    return null;
  }
}

/** 弹出「选择目录」对话框（Tauri 桌面端），返回目录路径或 null（取消）。 */
export async function pickOpenDirectory(): Promise<string | null> {
  if (!isTauri()) return null;
  try {
    return await openDialog({ multiple: false, directory: true });
  } catch {
    return null;
  }
}

/** 弹出「打开文件（可多选）」对话框（Tauri 桌面端），返回路径数组（取消为空数组）。 */
export async function pickOpenPaths(
  filters?: { name: string; extensions: string[] }[],
  title?: string,
): Promise<string[]> {
  if (!isTauri()) return [];
  try {
    const r = await openDialog({ multiple: true, filters, title });
    if (!r) return [];
    return Array.isArray(r) ? r.map(String) : [String(r)];
  } catch {
    return [];
  }
}

/** 领域目录绝对路径（`{realm-data}/realms/<id>`），用于解析 `assets/` 素材。 */
export async function realmDir(realmId: string): Promise<string> {
  return invoke<string>("realm_dir", { realmId });
}

/** 登场演出上次播放时间（unix 秒），无记录返回 null（§9.1 的 24 小时策略）。 */
export async function getIntroState(realmId: string): Promise<{ playedAt: number | null }> {
  return invoke<{ playedAt: number | null }>("get_intro_state", { realmId });
}

/** 记录登场演出已播放（写入 `.introPlayedAt`）。 */
export async function markIntroPlayed(realmId: string): Promise<void> {
  await invoke<void>("mark_intro_played", { realmId });
}

/** 读取当前启动器下该领域是否静音背景音乐（用户态开关，不随导出走）。 */
export async function getBgmMuted(realmId: string): Promise<boolean> {
  if (!isTauri()) return false;
  return invoke<boolean>("get_bgm_muted", { realmId });
}

/** 写入当前启动器下该领域的背景音乐静音开关（`.bgmMuted` 点文件）。 */
export async function setBgmMuted(realmId: string, muted: boolean): Promise<void> {
  if (!isTauri()) return;
  return invoke<void>("set_bgm_muted", { realmId, muted });
}

/**
 * 导入领域素材（壁纸 / 封面 / 视频 / 音乐）到 `{realm-dir}/assets/`，返回相对领域目录的路径。
 *
 * `sub` / `stem` 为可选的命名覆盖：**阵营巡礼的每阵营自定义底图**必须各自落到独立文件
 * （都走扁平 `assets/<kind stem>.<ext>` 会互相覆盖），所以传 `sub="showcase"` +
 * `stem="<modId>-<factionId>"`。两个片段在后端都会做路径安全净化。
 */
export async function importRealmAsset(
  realmId: string,
  src: string,
  kind: "wallpaper" | "cover" | "video" | "music" | "introMusic" | "showcaseBg",
  sub?: string,
  stem?: string,
): Promise<string> {
  const res = await invoke<{ path: string }>("import_realm_asset", {
    realmId,
    src,
    kind,
    sub,
    stem,
  });
  return res.path;
}

/** 整合模组重名冲突（§9.3.3）。仅在确有风险时返回，避免「整合即弹窗」的噪音。 */
export interface ModConflict {
  id: string;
  source: "user" | "game";
  /** game = 来源在游戏目录（永远可见，会重复挂载）；diverged = 领域副本与来源内容已分叉 */
  reason: "game" | "diverged";
}
export async function checkRealmModConflicts(
  realmId: string,
  bundled: string[],
): Promise<ModConflict[]> {
  if (!isTauri() || bundled.length === 0) return [];
  try {
    const res = await invoke<{ conflicts: ModConflict[] }>("check_realm_mod_conflicts", {
      realmId,
      bundled,
    });
    return res.conflicts ?? [];
  } catch {
    return []; // 校验失败不阻塞保存
  }
}

/** 重命名领域（B8）：把整个领域目录重命名为新 ID，并同步元数据；ID 占用或领域使用中时报错。 */
export async function renameRealm(oldId: string, newId: string): Promise<void> {
  if (!isTauri()) return; // 仅桌面端
  await invoke("rename_realm", { oldId, newId });
}

/** 战役场景条目（H3C 头部内嵌场景表；与 `@/lib/realm/types` 的 RealmMapScenario 对应）。 */
export interface RealmMapScenario {
  /** 场景名（regionName）：官方战役为地区 / 关卡名，自制战役常填剧情梗概；可为空 */
  name: string;
  /** 子地图文件名（如 wan003.h3m）；磁盘上可为空 */
  mapFile: string;
  /** 场景默认难度 0..4（简单/普通/困难/专家/不可能），未知为 -1 */
  difficulty: number;
}

/** 领域专属地图清单项（B7 / 集成地图增强）。`file` 为收纳仓库内相对路径（含 maps/ 或 campaigns/ 前缀）。 */
export interface RealmMapEntry {
  file: string;
  /** `map` | `campaign` | `vmap` */
  kind?: string;
  /** VCMI 语言 identifier（chinese/english/…），未识别为 unknown */
  lang?: string;
  encoding?: string;
  name?: string;
  description?: string;
  /** ROE / AB / SOD / HOTA / WOG / VCMI */
  version?: string;
  difficulty?: number;
  players?: number;
  humans?: number;
  sizeW?: number;
  sizeH?: number;
  sizeClass?: string;
  victory?: string;
  loss?: string;
  scenarioCount?: number;
  /** 战役场景表（仅 `kind === "campaign"`；index.json v2 起结构化） */
  scenarios?: RealmMapScenario[];
  size: number;
  /** 文件修改时间（unix 毫秒） */
  modifiedAt?: number;
  parseError?: string;
}

/** 目录引入预检/确认报告。 */
export interface RealmMapImportReport {
  total: number;
  added: number;
  skipped: number;
  failed: number;
  byKind: Record<string, number>;
  byLang: Record<string, number>;
  conflicts: string[];
  files?: { rel: string; dest: string; kind: string; lang: string; conflict: boolean }[];
}

/** 列出领域专属地图（B7 / 集成地图增强）：重建收纳仓库 index.json 并返回完整条目与语言过滤配置。 */
export async function listRealmMaps(
  realmId: string,
): Promise<{ entries: RealmMapEntry[]; filterLang: boolean; filterLangTarget: string }> {
  if (!isTauri()) return { entries: [], filterLang: false, filterLangTarget: "english" };
  try {
    return await invoke<{
      entries: RealmMapEntry[];
      filterLang: boolean;
      filterLangTarget: string;
    }>("list_realm_maps", { realmId });
  } catch {
    return { entries: [], filterLang: false, filterLangTarget: "english" };
  }
}

/** 从目录引入地图（两段式）：dry_run=true 只读预检，false 按 conflict 策略实际入库。 */
export async function importRealmMapsDir(
  realmId: string,
  srcDir: string,
  keepStructure: boolean,
  conflict: "skip" | "overwrite" | "rename",
  dryRun: boolean,
): Promise<RealmMapImportReport> {
  if (!isTauri()) throw new Error(feErr("frontend_desktop_only", "引入地图"));
  return await invoke<RealmMapImportReport>("import_realm_maps_dir", {
    realmId,
    srcDir,
    keepStructure,
    conflict,
    dryRun,
  });
}

/** 移动领域地图到其他目录（禁止地图 ↔ 战役跨类型）。返回移动后的新仓库相对路径。 */
export async function moveRealmMap(
  realmId: string,
  file: string,
  destDir: string,
): Promise<{ file: string }> {
  if (!isTauri()) throw new Error(feErr("frontend_desktop_only", "移动地图"));
  return await invoke<{ file: string }>("move_realm_map", { realmId, file, destDir });
}

/** 重建领域 Maps 可视目录（语言过滤 junction 集合）；仅激活领域即时生效。 */
export async function applyRealmMapsValve(realmId: string): Promise<void> {
  if (!isTauri()) return;
  await invoke("apply_realm_maps_valve", { realmId });
}

/** 设置语言过滤（开关 + 目标语言）并持久化；激活领域即时重建阀门。 */
export async function setRealmMapFilter(
  realmId: string,
  enabled: boolean,
  target?: string,
): Promise<void> {
  if (!isTauri()) return;
  await invoke("set_realm_map_filter", { realmId, enabled, target: target ?? null });
}

/** 上传领域专属地图（B7）：复制玩家选择的文件到 `{realm-dir}/vcmi/Maps/`。 */
export async function uploadRealmMap(realmId: string, src: string): Promise<RealmMapEntry> {
  if (!isTauri()) throw new Error(feErr("frontend_desktop_only", "上传地图"));
  return await invoke<RealmMapEntry>("upload_realm_map", { realmId, src });
}

/** 删除领域专属地图（B7）。 */
export async function deleteRealmMap(realmId: string, file: string): Promise<void> {
  if (!isTauri()) return;
  await invoke("delete_realm_map", { realmId, file });
}

/** 地图阀门状态（§8.3.4）：`organized` = 游戏自带 Maps 已收纳进 `Maps.vcmi-fv-native`。 */
export async function nativeMapsStatus(): Promise<{ organized: boolean; blocked: boolean }> {
  if (!isTauri()) return { organized: false, blocked: false };
  try {
    return await invoke<{ organized: boolean; blocked: boolean }>("native_maps_status");
  } catch {
    return { organized: false, blocked: false };
  }
}

/** 进程守卫（§4.4）：游戏本体是否在运行（不含官方启动器）。
 * 用于游戏退出后自动淡回领域覆盖层（§9.2.3）。 */
export async function isVcmiRunning(): Promise<boolean> {
  if (!isTauri()) return false;
  try {
    return await invoke<boolean>("is_vcmi_running");
  } catch {
    return false;
  }
}

/**
 * 进程守卫（§4.4）：**正在运行的 VCMI 进程名**（空数组 = 未运行）。
 *
 * 比 `isVcmiRunning` 多给出"是哪个进程"，前端据此把原因直接显示出来 ——
 * 只说"VCMI 正在运行"时用户无从判断该关掉什么（常见是残留的 `VCMI_client.exe`）。
 *
 * 含官方启动器 `VCMI_launcher.exe`，仅作**提示**：它不持有游戏状态，
 * 与切换领域可共存（阻塞判定请用 `listVcmiGameProcesses`）。
 */
export async function listVcmiProcesses(): Promise<string[]> {
  if (!isTauri()) return [];
  try {
    return await invoke<string[]>("list_vcmi_processes");
  } catch {
    return [];
  }
}

/** 官方启动器进程名（提示口径，不参与阻塞判定）。 */
const NATIVE_LAUNCHER_PROCESS = "VCMI_launcher.exe";

/** 从 `listVcmiProcesses` 结果里挑出官方启动器进程（空数组 = 只有 / 没有它）。 */
export function pickNativeLauncherProcs(procs: string[]): string[] {
  return procs.filter((p) => p.toLowerCase() === NATIVE_LAUNCHER_PROCESS);
}

/**
 * 进程守卫（§4.4）：**会阻塞操作的游戏本体进程名**（空数组 = 不挡路）。
 *
 * 只含 `VCMI_client` / `VCMI_server` / `VCMI_mapeditor`：这类进程持有存档与游戏状态，
 * 运行中改写 `dirs.json` 会让它们把存档 / 设置写到切换前的目录。
 * 官方启动器不在此列 —— 它的路径在**自身启动时**就已快照，切换领域不会破坏它已打开的数据，
 * 只是它之后的改动可能落在切换前的目录（不生效，但不是数据损坏）。
 */
export async function listVcmiGameProcesses(): Promise<string[]> {
  if (!isTauri()) return [];
  try {
    return await invoke<string[]>("list_vcmi_game_processes");
  } catch {
    return [];
  }
}

/** 启动游戏前的模组校验结果（§9.2.3）：按 bundled / borrowed 分别列出，并附重名冲突。 */
export interface RealmModsReady {
  missingBundled: string[];
  missingBorrowed: string[];
  conflicts: ModConflict[];
}
export async function checkRealmModsReady(realmId: string): Promise<RealmModsReady> {
  if (!isTauri()) return { missingBundled: [], missingBorrowed: [], conflicts: [] };
  try {
    return await invoke<RealmModsReady>("check_realm_mods_ready", { realmId });
  } catch {
    return { missingBundled: [], missingBorrowed: [], conflicts: [] }; // 校验失败不阻塞启动
  }
}

/** 领域素材相对路径 → `<img>/<video>` 可用 URL（§5.4：本地资源必须经 convertFileSrc）。
 * 支持绝对路径（新建领域预览时传入本地暂存文件的绝对路径）。 */
export function realmAssetUrl(root: string, rel: string | undefined): string {
  if (!rel) return "";
  if (/^(https?:|asset:|data:|file:)/i.test(rel)) return rel;
  if (/^[A-Za-z]:[\\/]/.test(rel) || rel.startsWith("\\\\")) return convertFileSrc(rel);
  return convertFileSrc(`${root}/${rel.replace(/^\/+/, "")}`);
}

/** 导入领域快照（§7.2）。conflict ∈ "error" | "overwrite" | "rename"。 */
export interface ImportRealmResult {
  ok: boolean;
  realmId: string;
  realmName: string;
  needsMods: boolean;
  missingMods: string[];
  /** 随包解包出来的整合模组（`{realm-dir}/vcmi/Mods/<modId>`），供导入后补建百科缓存 */
  bundledMods?: { modId: string; modPath: string }[];
  /** 与本机已安装模组**重名**的整合模组（§9.3.3）：会共用百科缓存导致资料串味，仅提示不阻断 */
  duplicateMods?: string[];
}
export class RealmImportError extends Error {
  code: string;
  realmId?: string;
  realmName?: string;
  constructor(payload: { code: string; msg?: string; realmId?: string; realmName?: string }) {
    super(payload.msg || payload.code);
    this.code = payload.code;
    this.realmId = payload.realmId;
    this.realmName = payload.realmName;
  }
}
export async function importRealm(path: string, conflict: "error" | "overwrite" | "rename" = "error"): Promise<ImportRealmResult> {
  if (!isTauri()) throw new Error(feErr("frontend_desktop_only", "导入领域"));
  try {
    return await invoke<ImportRealmResult>("import_realm", { path, conflict });
  } catch (e) {
    // 后端错误为结构化 JSON 字符串：{ code, msg?, realmId?, realmName? }
    try {
      const p = typeof e === "string" ? JSON.parse(e) : e;
      if (p && p.code) {
        throw new RealmImportError({ code: p.code, msg: p.msg, realmId: p.realmId, realmName: p.realmName });
      }
    } catch {
      /* 非结构化错误原样抛出 */
    }
    throw e instanceof Error ? e : new Error(String(e));
  }
}
