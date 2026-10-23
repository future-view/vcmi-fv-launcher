/**
 * 目录解析、启动游戏 / 编辑器、显示器与编译信息、运行环境体检、H3 资源导入。
 *
 * 拆分自原 `@/lib/tauri.ts` 单体；对外仍由该 barrel 统一导出，调用方无需改动。
 */
import { invoke } from "@tauri-apps/api/core";
import { feErr } from "@/lib/backendError";
import { VcmiDirs } from "./config";
import { isTauri, wrapLaunch, type LaunchResult } from "./core";
import { UserDataCopyResult } from "./prefs";

/** 获取当前识别的目录 */
export async function getRealDirs(): Promise<VcmiDirs | null> {
  if (!isTauri()) return null;
  return invoke("get_dirs");
}

/** 检测当前是官方版还是增强版 */
export async function detectEdition(): Promise<"official" | "enhanced"> {
  if (!isTauri()) return "official";
  const edition = await invoke<string>("detect_edition");
  return edition === "enhanced" ? "enhanced" : "official";
}

/** 启动游戏 */
export async function launchGame(): Promise<LaunchResult> {
  if (!isTauri()) return { ok: false, error: feErr("frontend_not_tauri") };
  return wrapLaunch(() => invoke("launch_game"));
}

/** 启动地图编辑器 */
export async function launchEditor(): Promise<LaunchResult> {
  if (!isTauri()) return { ok: false, error: feErr("frontend_not_tauri") };
  return wrapLaunch(() => invoke("launch_editor"));
}

/** 用资源管理器打开目录 */
export async function openInExplorer(path: string): Promise<LaunchResult> {
  if (!isTauri()) return { ok: false, error: feErr("frontend_not_tauri") };
  return wrapLaunch(() => invoke("open_in_explorer", { path }));
}

/** 启动 VCMI 自带 launcher（VCMI_launcher.exe） */
export async function launchNativeLauncher(): Promise<LaunchResult> {
  if (!isTauri()) return { ok: false, error: feErr("frontend_not_tauri") };
  return wrapLaunch(() => invoke("launch_native_launcher"));
}

/** 主显示器尺寸（DPI scale 后逻辑像素） */
export interface MonitorInfo {
  width: number;
  height: number;
  scale: number;
}
export async function getPrimaryMonitorInfo(): Promise<MonitorInfo | null> {
  if (!isTauri()) return null;
  try {
    return await invoke<MonitorInfo>("get_primary_monitor");
  } catch {
    return null;
  }
}

/**
 * 列出**全部**显示器（多显示器 → 高清素材倍率取最大物理宽，见 `lib/hd.ts`）。
 *
 * 失败返回空数组而非 null：调用方据此回退到 1x（原素材），不影响可用性。
 */
export async function listMonitors(): Promise<MonitorInfo[]> {
  if (!isTauri()) return [];
  try {
    return await invoke<MonitorInfo[]>("list_monitors");
  } catch {
    return [];
  }
}

/** 编译信息（VCMI 引擎版本 + 内置核心 mod 版本 + OS 版本） */
export interface CompileInfo {
  /** 引擎版本（`1.7.4` 形式；探测三级：`VCMI_client.exe -v` → PE 版本 → exe 字节扫描） */
  vcmiVersion: string;
  /**
   * 内置核心 mod 的**资产版本**（`Mods/vcmi/mod.json` 的 `version`，如 `1.5`）。
   * ⚠️ **不是引擎版本** —— 仅供诊断；兼容性判定必须用 `vcmiVersion`。
   */
  coreModVersion?: string;
  /** 版本来源：cli / pe / scan / none（排查"版本是怎么读出来的"） */
  versionSource?: string;
  osVersion: string;
}
export async function getCompileInfo(): Promise<CompileInfo | null> {
  if (!isTauri()) return null;
  try {
    return await invoke<CompileInfo>("get_compile_info");
  } catch {
    return null;
  }
}

/** 游戏数据完整性体检项（后端 `vcmi::health` 的 items 项） */
export interface HealthItem {
  /** 体检项 id（i18n 文案：`data.healthItems.<id>`） */
  id: string;
  /** `fatal` = 缺了游戏无法启动；`warn` = 缺了影响体验 */
  level: "fatal" | "warn";
  ok: boolean;
  /** 通过时为命中路径，缺失时为缺失说明 */
  detail: string;
}

export interface GamedataHealth {
  /** 全部 `fatal` 项通过 */
  ok: boolean;
  /** 实际检查的资源根（游戏目录 / 用户数据目录，两者 VCMI 都会挂载） */
  roots: string[];
  items: HealthItem[];
  /** 原版 H3 数据闸门（后端 `vcmi::h3data::H3DataStatus`，官方口径） */
  h3Data?: H3DataStatus;
  /** 可选内容（hd-edition / chronicles）是否**已存在**：向导第 ③ 步据此显示「已就绪」 */
  optionalMods?: OptionalModsStatus;
  /** 注册表里发现的本机原版安装路径（Windows；用于预填数据目录） */
  dataInstallCandidates?: string[];
}

/** 可选内容检测结果（游戏目录 / 用户数据目录两处 Mods 任一命中即为 true） */
export interface OptionalModsStatus {
  /** `Mods/hd-edition` 里有可用 pak */
  hdEdition: boolean;
  /** `Mods/chronicles/mod.json` 存在 */
  chronicles: boolean;
  /** `Mods/chronicles_<n>` 子模组数量（0~8） */
  chroniclesChapters: number;
}

/**
 * 数据模态（VCMI `GameLibrary::GameDataMode` 口径）。
 *
 * 只有 `complete`（SoD/Complete 哨兵 `TENTCOLR.TXT`）与 `demo` 能启动；
 * `abOnly` / `hdEdition` / `roeOnly` 在运行期都会被判成 ROE 而 fatal，
 * 保留它们只为给出"该装什么"的准确提示（AB 无独立安装态）。
 */
export type H3DataKind =
  | "complete"
  | "demo"
  | "abOnly"
  | "hdEdition"
  | "roeOnly"
  | "empty";

/** 原版 H3 数据状态（官方口径，见后端 `vcmi::h3data`） */
export interface H3DataStatus {
  /** 向导闸门：ROE && (SOD || demo) && 运行期 fatal 项齐全 */
  found: boolean;
  /** 数据模态：决定缺失/不支持的具体文案 */
  kind: H3DataKind;
  /** `Data/*.pak` 存在（HD Edition 标记） */
  hdEdition: boolean;
  /** `Data/H3ab*.lod` 存在（末日之刃数据层） */
  abLods: boolean;
  roe: boolean;
  sod: boolean;
  demo: boolean;
  helpTxt: boolean;
  playersPal: boolean;
  defaultDef: boolean;
  inGame: boolean;
  inUser: boolean;
  /** 用户可操作的缺失文件（H3Bitmap.lod / H3Sprite.lod） */
  missing: string[];
}

/**
 * 游戏数据完整性体检（只读，不改动任何文件）。非 Tauri 环境返回 null。
 *
 * `gameDir` / `userDataDir` 为**可选覆盖**（首启向导用：目录还没保存进 launcher.json）。
 * 不传 = 沿用已保存配置（主界面「游戏数据」抽屉行为不变）。
 * `withCandidates`：是否查注册表安装候选（`reg query` ×4，约百毫秒）；
 * 向导常态探测传 `false`，确认缺失后再补一次 `true`。
 */
export async function gamedataHealthCheck(
  gameDir?: string,
  userDataDir?: string,
  withCandidates = true,
): Promise<GamedataHealth | null> {
  if (!isTauri()) return null;
  try {
    return await invoke<GamedataHealth>("gamedata_health_check", {
      gameDir,
      userDataDir,
      withCandidates,
    });
  } catch (e) {
    console.error("数据体检失败", e);
    return null;
  }
}

/**
 * 首启向导第 3 步：从外部数据目录复制原版数据。
 * - `.lod` / `.snd` / `.vid` → `<toDataDir>`（radio 目标的 Data，判定根之一）；
 *   官方 `classifyTargetByExt` 同口径，`.snd`/`.vid` 缺了 VCMI 会弹「未检测到视频文件」；
 * - `Mp3`（默认）/ `Maps`（可选）→ `<extraRoot>`（**与数据文件同一个 radio 目标根**；
 *   VCMI 的 MUSIC / MAPS 挂载点；`Maps` 目录即使不复制也会被创建——VCMI 不自建它）。
 * 复用 `write_launcher_config` 的 `copyOnly` 通道（不新增命令，见后端约束）。
 */
export async function copyH3Data(
  gameDir: string,
  extraRoot: string,
  fromDir: string,
  toDataDir: string,
  copyMp3 = true,
  copyMaps = false,
): Promise<UserDataCopyResult> {
  const r = await invoke<UserDataCopyResult>("write_launcher_config", {
    data: {
      gameDir,
      userDataDir: toDataDir,
      copyOnly: true,
      copyH3Data: true,
      h3DataFrom: fromDir,
      h3DataTo: toDataDir,
      h3UserDataDir: extraRoot,
      h3CopyMp3: copyMp3,
      h3CopyMaps: copyMaps,
    },
  });
  return r;
}

/**
 * 清除指定模组在 `validatedMods` 里的校验缓存（强制 VCMI 下次启动重新校验）。
 *
 * ⚠️ `validatedMods` 的 checksum 只覆盖 `mod.json` + config json，玩家在游戏外改过
 * 模组素材/文件时 checksum 往往不变、引擎不再重查文件完整性 —— 排查模组加载故障时用它。
 *
 * `configDir`：传入时作用于领域隔离目录的 `modSettings.json`，否则全局。
 * 返回实际清除的条目数（0 = 本就没有缓存条目）。
 */
export async function clearModValidation(
  configDir: string | undefined,
  modIds: string[],
): Promise<number> {
  if (!isTauri() || modIds.length === 0) return 0;
  try {
    return await invoke<number>("clear_mod_validation", {
      configDir: configDir ?? null,
      modIds,
    });
  } catch (e) {
    console.error("清除模组校验缓存失败", e);
    return 0;
  }
}
