/**
 * 启动器自动更新（命令契约见 commands/updater.rs）。
 *
 * 拆分自原 `@/lib/tauri.ts` 单体；对外仍由该 barrel 统一导出，调用方无需改动。
 */
import { invoke } from "@tauri-apps/api/core";
import { backendErrorText, feErr } from "@/lib/backendError";
import { isTauri } from "./core";

/* ----------------------------- 启动器自动更新（命令契约见 commands/updater.rs） ----------------------------- */

/** 后端 `UpdateInfo`（camelCase 序列化，见 updater/manifest.rs） */
export interface UpdateInfo {
  currentVersion: string;
  latestVersion: string;
  hasUpdate: boolean;
  /** `current < 清单 minSupported`：全屏阻断，只许更新或退出 */
  forced: boolean;
  critical: boolean;
  size?: number;
  publishedAt?: string;
  /** 多语言更新说明：按 ui_lang 取词（`pickNotes`），english 兜底 */
  notesI18n?: Record<string, string[]>;
  /** 多语言更新说明的**完整 Markdown 原文**（`notesMd` 投影；缺省 = 旧清单，
   *  展示层回退到 `notesI18n` 的行数组，见 `pickNotesMarkdown`） */
  notesMdI18n?: Record<string, string>;
  state: "idle" | "available" | "downloading" | "downloaded" | "error";
  error?: string;
  /** 本次结果来自 24h 内清单缓存 */
  cached?: boolean;
  /** 清单是否已配置（未配置时手动检查提示） */
  configured: boolean;
  /**
   * 包类型提示：`full`（缺省，含 exe，需关闭启动器替换）/ `config`（纯配置，可直接应用）。
   * 仅用于「应用前」的 UI 预判；实际行为以包内容为准（见 `download::staged_has_exe`）。
   */
  kind?: string;
  /**
   * 更新方式（ADR 0008 D7）：`inplace`（缺省）/ `appimage` / `bundle` / `manual`。
   * `manual` = 该平台不做应用内更新（如 macOS 无签名证书的过渡期）——UI 只提示手动下载。
   */
  updateMode?: string;
  /** 更新包下载地址（`manual` 模式给「打开下载页」用） */
  downloadUrl?: string;
  /**
   * 是否应**自动弹出**更新弹窗（可选更新感知 L4，7 天节流）：
   * 有可选更新且未跳过时，每版本首次发现立即为 true；同版本距上次自动弹窗满 7 天才再次为 true。
   * 手动检查（`force`）与强制更新恒为 false。
   */
  shouldPrompt: boolean;
  /**
   * 是否已**下载完成、待用户应用**（"更新包已就绪"）。
   * 与 `hasUpdate` 不同：它来自持久化的"待应用版本"，跨进程/跨会话恢复，
   * **不受 7 天自动弹窗节流**——每次启动都应在启动期提示用户「点击重启并应用」，
   * 直到被应用或跳过。
   */
  readyToApply: boolean;
  /**
   * 首启向导专用：「清单可达性探测」结果。
   * 常规检查（含启动静默检查）恒为 `undefined`。
   */
  probe?: ManifestProbe;
}

/** 更新清单可达性探测结果（后端 `updater::manifest::ManifestProbe`） */
export interface ManifestProbe {
  /** 是否收到任何 HTTP 响应（404 也算通：只是地址不对） */
  ok: boolean;
  /** 清单地址是否为 GitHub 主机（只有 GitHub 才建议开镜像） */
  github: boolean;
  status?: number;
  error?: string;
}

/** 下载进度（事件 `launcher-update-progress`） */
export interface UpdateProgress {
  received: number;
  total: number;
  done?: boolean;
}

/** 后端取消哨兵错误串：识别后不弹错误提示 */
export const UPDATE_DOWNLOAD_CANCELLED = "UPDATE_DOWNLOAD_CANCELLED";

/** 模块级缓存：启动静默检查一次，顶栏红点 / 设置页 / 更新弹窗共享，领域切换不重查 */
let lastUpdateInfoCache: UpdateInfo | null = null;

/** 最近一次检查结果（未检查过为 null） */
export function lastUpdateInfo(): UpdateInfo | null {
  return lastUpdateInfoCache;
}

/**
 * 检查更新。`force=false` 为启动静默检查（失败静默返回 null）；`force=true` 手动检查（错误在 info.error）。
 *
 * ⚠️ `probe=true` 是**另一条路径**（首启向导用）：只探测清单地址连不连得上，
 * 不检查更新、不触碰更新状态机，结果在 `info.probe`；**不写入模块级缓存**
 * （它不是一次"检查结果"，不能污染顶栏红点与更新弹窗）。
 */
export async function checkLauncherUpdate(force = false, probe = false): Promise<UpdateInfo | null> {
  if (!isTauri()) return null;
  try {
    const info = await invoke<UpdateInfo>("check_launcher_update", { force, probe });
    if (!probe) lastUpdateInfoCache = info;
    return info;
  } catch (e) {
    console.warn("检查更新失败", e);
    return null;
  }
}

/**
 * 首启向导用：探测「更新清单地址是否可达」（连接 5s / 总 8s，**直连不回退**）。
 *
 * 返回 `null` = 探测没跑起来（命令失败 / 非 Tauri），调用方据此不显示任何网络建议。
 * `configured=false`（未配置清单）也返回 `null`：没有清单就没有"要不要开镜像"的问题。
 */
export async function probeUpdateManifest(): Promise<ManifestProbe | null> {
  const info = await checkLauncherUpdate(false, true);
  if (!info || !info.configured) return null;
  return info.probe ?? null;
}

/** 下载更新包（阻塞至完成 + sha256 校验）。失败返回 `{ok:false, error}`（取消时 error 为哨兵串） */
export async function downloadLauncherUpdate(): Promise<{ ok: boolean; path?: string; error?: string }> {
  if (!isTauri()) return { ok: false, error: feErr("frontend_not_tauri") };
  try {
    return await invoke<{ ok: boolean; path?: string }>("download_launcher_update");
  } catch (e) {
    return { ok: false, error: backendErrorText(e) };
  }
}

/** 取消正在进行的下载（后端下一块读盘时退出；状态回 available 可重试） */
export async function cancelLauncherUpdate(): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("cancel_launcher_update");
  } catch {
    // 命令不可用或已结束，忽略
  }
}

/**
 * 应用更新。两种结果：
 *
 * - `restarted: true`（含 exe 的包）→ 后端已生成 detached 替换脚本，
 *   **由调用方延迟 ~800ms 调 `closeWindow()`**（后端绝不自行 exit，保证 IPC 响应送达）；
 *   脚本等进程退出后 `move 旧→.old`、`move 新→原位` 并拉起新版本。
 * - `restarted: false`（**纯配置更新包**，zip 内不含 exe）→ 已**就地应用**，
 *   调用方**不要关闭窗口**；`failed` 非空表示个别文件被占用（重启后生效）。
 */
export async function applyLauncherUpdate(): Promise<{
  ok: boolean;
  /** `false` = 纯配置包已就地应用，**不要关窗** */
  restarted?: boolean;
  /** 就地应用时未能落地的相对路径（被占用） */
  failed?: string[];
  error?: string;
}> {
  if (!isTauri()) return { ok: false, error: feErr("frontend_not_tauri") };
  try {
    return await invoke<{ ok: boolean; restarted?: boolean; failed?: string[] }>(
      "apply_launcher_update",
    );
  } catch (e) {
    return { ok: false, error: backendErrorText(e) };
  }
}

/**
 * 写入更新偏好（**三态语义**，见后端 `set_update_pref` 注释）：
 * - `autoCheck`：`true` / `false` = 修改启动自动检查开关；`null` = 保持不变；
 * - `skipVersion`：`null` = 清空「跳过此版本」，非空 = 设置（仅普通更新生效）。
 */
export async function setUpdatePref(
  autoCheck: boolean | null,
  skipVersion: string | null,
): Promise<void> {
  if (!isTauri()) return;
  return invoke("set_update_pref", {
    autoCheck,
    // 空串（而非 null）= 清空跳过记录；`null` 在后端语义为「保持不变」
    skipVersion: skipVersion ?? "",
    promptVersion: null,
  });
}

/**
 * 记录「已自动弹出更新提示」（可选更新的 7 天节流；写 `launcher.json` 的
 * `update.prompt_version` / `update.prompt_at`）。
 *
 * 复用 `set_update_pref` 承载（后端注释：本机工具链对**新增** Tauri 命令会触发
 * never-type 硬错误，见 03-architecture「约束」）；`autoCheck` / `skipVersion`
 * 传 `null` = 保持不变（不会清掉用户的「跳过此版本」记录）。
 *
 * 由前端在**弹窗展示**时调用（计时从展示起算，见 settings-panel-design §2.3）；
 * 失败静默——节流只是体验优化，不影响更新功能本身。
 */
export async function markUpdatePrompted(version: string): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("set_update_pref", {
      autoCheck: null,
      skipVersion: null,
      promptVersion: version,
    });
  } catch (e) {
    console.warn("记录更新提醒时间失败", e);
  }
}

/** 写入更新清单地址（https 直链）；传 null = 清除（完全不检查） */
export async function setUpdateManifestUrl(url: string | null): Promise<void> {
  if (!isTauri()) return;
  return invoke("set_update_manifest_url", { url });
}

/** 订阅下载进度，返回取消订阅函数 */
export async function onUpdateProgress(cb: (p: UpdateProgress) => void): Promise<() => void> {
  if (!isTauri()) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  return await listen<UpdateProgress>("launcher-update-progress", (e) => cb(e.payload));
}
