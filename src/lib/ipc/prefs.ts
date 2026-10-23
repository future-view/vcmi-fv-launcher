/**
 * 碎片化的偏好项写入（界面语言、网络代理、镜像规则、HD/超分、AI 模型、领域自检）与用户数据迁移。
 *
 * 拆分自原 `@/lib/tauri.ts` 单体；对外仍由该 barrel 统一导出，调用方无需改动。
 */
import { invoke } from "@tauri-apps/api/core";
import { MirrorRule } from "./config";
import { isTauri } from "./core";

/** 「同步数据」迁移结果（`write_launcher_config` 出参，见 commands/settings.rs） */
export interface UserDataCopyResult {
  /** 是否真的保存了配置（`copyOnly` 时为 false） */
  saved: boolean;
  copied: boolean;
  cancelled: boolean;
  files: number;
  bytes: number;
}

/**
 * 首启向导第 2 步的「同步数据」：**只迁移用户数据，不写任何配置**。
 *
 * 中断/关闭窗口 ⇒ 磁盘上没有任何已生效配置（staging 被丢弃），下次启动原样重入向导。
 */
export async function copyUserData(
  gameDir: string,
  from: string,
  to: string,
): Promise<UserDataCopyResult> {
  const r = await invoke<UserDataCopyResult>("write_launcher_config", {
    data: { gameDir, userDataDir: to, copyFrom: from, copyOnly: true },
  });
  return r;
}

/** 请求中断正在进行的用户数据迁移（「跳过复制」） */
export async function cancelUserDataCopy(): Promise<void> {
  if (!isTauri()) return;
  await invoke("write_launcher_config", { data: { cancelCopy: true } });
}

/** 用户数据迁移进度（事件 `user-data-copy-progress`） */
export interface UserDataCopyProgress {
  /**
   * 当前复制阶段（旧后端不推 → 省略，按通用文案显示）：
   * - `data`：原版 `Data` 下的 `.lod` / `.snd` / `.vid`
   * - `mp3`：音轨；`maps`：地图（两段各有独立分母，标题必须区分）
   */
  stage?: "data" | "mp3" | "maps";
  doneFiles: number;
  doneBytes: number;
  totalFiles: number;
  totalBytes: number;
}

/** 订阅用户数据迁移进度，返回取消订阅函数（节流在后端，200ms 一帧 + 末帧） */
export async function onUserDataCopyProgress(
  cb: (p: UserDataCopyProgress) => void,
): Promise<() => void> {
  if (!isTauri()) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  return await listen<UserDataCopyProgress>("user-data-copy-progress", (e) => cb(e.payload));
}

/**
 * 持久化启动器界面语言到 launcher.json。
 *
 * 底栏语言切换时调用。初始化向导阶段配置尚未就绪（`set_ui_lang` 会报错），
 * 此时语言随 `writeLauncherConfig({ uiLang })` 一并写入，故调用方应忽略此处异常。
 */
export async function setUiLang(lang: string): Promise<void> {
  if (!isTauri()) return;
  return invoke("set_ui_lang", { lang });
}

/**
 * 前端日志上报：写入与后端同一日志文件（`<exe>/.vcmi-fv-launcher/logs`）。
 *
 * **绝不抛错 / 阻塞**：日志失败不能影响业务，故 fire-and-forget 并吞掉异常。
 * 限流由调用方（`src/lib/log.ts`）负责。
 */
export function logFrontend(
  level: "error" | "warn" | "info" | "debug" | "trace",
  target: string,
  message: string,
): void {
  if (!isTauri()) return;
  void invoke("log_frontend", { level, target, message }).catch(() => {});
}

/** 仅写入启动器偏好（设置面板视图：可见级别 + 详细描述），不校验游戏目录 */
export async function saveSettingsPrefs(
  settingsLevel: "common" | "advanced" | "all",
  settingsShowDesc: boolean
): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("set_launcher_pref", {
      showAllSettings: settingsLevel === "all",
      settingsLevel,
      settingsShowDesc,
    });
  } catch (e) {
    console.error("写入启动器偏好失败", e);
  }
}

/**
 * 写入启动器界面语言（`launcher.json` 的 `uiLang`）——**下次启动生效**。
 *
 * ⚠️ 只改启动器自身界面语言，与「通用」标签下的 `general.language`（游戏内文本语言）无关。
 */
export async function saveUiLang(lang: string): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("set_ui_lang", { lang });
  } catch (e) {
    console.error("写入启动器界面语言失败", e);
  }
}

/**
 * 写入 `FV启动器` 标签里的 `fv.*` 项（**launcher.json**，与 VCMI `settings.json` 不同源）。
 *
 * 按 key 分派到各自的专用命令；未支持的 key 只告警不抛错（避免拖垮整页保存）。
 * 用户改这些项时不应影响 VCMI 设置通道，故 `SettingsView` 会按 `fv.` 前缀分流到这里。
 */
export async function saveFvLauncherItem(key: string, value: unknown): Promise<void> {
  switch (key) {
    case "fv.aiHdEnhance":
      return saveAiHdEnhance(value === true);
    case "fv.aiModel":
      return saveAiModel(value == null || value === "" ? null : String(value));
    case "fv.townPreview":
      return saveTownPreview(value === true);
    case "fv.hdEnhance":
      return saveHdEnhance(value === true);
    case "fv.hdEditionUse":
      return saveHdEditionUse(String(value));
    case "fv.hdEditionScale":
      return saveHdEditionScale(String(value));
    case "fv.uiLang":
      return saveUiLang(String(value));
    case "fv.networkProxy":
      return saveNetworkProxy(value == null || value === "" ? null : String(value));
    default:
      console.warn("[fv] 尚未支持写入的项：", key);
  }
}

/**
 * 写入「网络代理」（`launcher.json` 的 `network.proxy`）。
 *
 * 与 `set_hd_enhance` 同款走**独立命令**（不入 `set_launcher_pref`）：写入即后端刷新进程级代理、**即时生效**。
 * 仅对 GitHub 地址生效（先直连、连接失败自动改走代理）；`null`/`""`/`"off"` = 直连。见网络方案文档 L1。
 */
export async function saveNetworkProxy(proxy: string | null): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("set_network_proxy", { proxy });
  } catch (e) {
    console.error("写入网络代理失败", e);
  }
}

/**
 * 写入「GitHub 镜像规则表」（`launcher.json` 的 `network.mirror_rules`，L2，取代旧单行前缀）。
 *
 * 面向无本地代理用户：写入即后端归一化持久化并刷新进程级规则（`set_network_mirror_rules`），**即时生效**。
 * 回退序：直连→逐条镜像→代理（D2：镜像 403/429/5xx 推进下一条，404 停止）；默认空、**不内置镜像**（N10）。
 * ⚠️ 故意**不吞异常**：校验失败（第 N 条无效/超上限）throw 给调用方，由 `MirrorRulesCard` 就地展示错误。
 */
export async function saveNetworkMirrorRules(rules: MirrorRule[]): Promise<void> {
  if (!isTauri()) return;
  await invoke("set_network_mirror_rules", { rules });
}

/**
 * 写入「高清素材生成」总开关（默认关闭）。
 *
 * 见 `LauncherConfig.hdEnhance`：关闭时前端不传 hd 参数，构建阶段完全跳过高清，
 * 用户只等"标清可用"；之后可在设置里开启并重跑（增量，只补高清）。
 */
export async function saveHdEnhance(enabled: boolean): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("set_hd_enhance", { enabled });
  } catch (e) {
    console.error("写入高清素材开关失败", e);
  }
}

/**
 * 写入「AI 高清增强」开关（默认关闭）。
 *
 * 与 `saveTownPreview` 同款：走**独立命令**而非 `set_launcher_pref`，
 * 因为后者由设置页挂载即调用，会把本字段覆盖成默认值。
 */
export async function saveAiHdEnhance(enabled: boolean): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("set_ai_hd_enhance", { enabled });
  } catch (e) {
    console.error("写入 AI 高清增强开关失败", e);
  }
}

/**
 * 写入「AI 模型」（组件目录内的 `*.onnx` 文件名）。
 *
 * `null` / 空串 = 清除 → **AI 随之不生效**（无默认模型）。
 * 取值建议来自 {@link listAiModels}；含路径分隔符或 `..` 的值后端会拒绝（计入错误日志）。
 */
export async function saveAiModel(model: string | null): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("set_ai_model", { model });
  } catch (e) {
    console.error("写入 AI 模型失败", e);
  }
}

/** 组件目录里可用的 AI 模型（`list_ai_models` 的返回项） */
export interface AiModelInfo {
  /** 文件名（含 `.onnx`）—— 即 {@link saveAiModel} 可接受的取值 */
  file: string;
  /** 文件大小（字节） */
  size: number;
  /** 是否在组件清单内（可标"已验证"；`false` = 用户自备，未验证但可用） */
  known: boolean;
  /** 是否为推荐模型（`realesrgan_anime6b.onnx`） */
  recommended: boolean;
}

/** 扫描组件目录，列出可用的 AI 模型（组件未安装 / 目录为空 → 空数组） */
export async function listAiModels(): Promise<AiModelInfo[]> {
  if (!isTauri()) return [];
  try {
    return await invoke<AiModelInfo[]>("list_ai_models");
  } catch (e) {
    console.error("扫描 AI 模型失败", e);
    return [];
  }
}

/**
 * 校验某个**已知**模型的 sha256（按需调用：会读整个文件，17.5MB 级约几十毫秒）。
 *
 * 返回 `null` = 不在清单内（**未验证**，不是失败）；`false` = 已知但已损坏。
 */
export async function verifyAiModel(file: string): Promise<boolean | null> {
  if (!isTauri()) return null;
  try {
    return await invoke<boolean | null>("verify_ai_model", { file });
  } catch (e) {
    console.error("校验 AI 模型失败", e);
    return null;
  }
}

/**
 * 写入「列表 hover 预览城镇背景」开关（默认关闭）。
 * 走独立命令而非 `set_launcher_pref`：设置页挂载即调用后者，带进去会被默认值覆盖。
 */
export async function saveTownPreview(enabled: boolean): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("set_town_preview", { enabled });
  } catch (e) {
    console.error("写入城镇背景预览开关失败", e);
  }
}

/**
 * 写入「自动检测更新」开关（launcher.json `realms.mod_auto_check`，默认开）——
 * 仅领域「模组管理」页生效：打开管理页时静默检查一次模组仓库索引（fetch_mod_index）。
 * 走独立命令而非 `set_launcher_pref`（同 `saveTownPreview`）：后者由设置页挂载即调用，
 * 带进去会被默认值覆盖。
 */
export async function saveRealmModAutoCheck(enabled: boolean): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("set_realm_mod_auto_check", { enabled });
  } catch (e) {
    console.error("写入领域自动检测更新开关失败", e);
  }
}

/**
 * 写入「HD 素材来源」（官方 HD 版 `hd-edition` 的 `.pak`，默认 `auto`）。
 *
 * 走独立命令而非 `set_launcher_pref`（同 `saveTownPreview`）：后者由设置页挂载即调用，
 * 会把本字段覆盖成默认值。改动后需**重建素材缓存**才会生效（首次会重跑素材阶段）。
 */
export async function saveHdEditionUse(kind: string): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("set_hd_edition_use", { kind });
  } catch (e) {
    console.error("写入 HD 素材来源失败", e);
  }
}

/** 写入「HD 素材档位」：`"auto"`（x3 优先）/ `"x2"`（只用 x2，省磁盘）。 */
export async function saveHdEditionScale(scale: string): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("set_hd_edition_scale", { scale });
  } catch (e) {
    console.error("写入 HD 素材档位失败", e);
  }
}

/** HD 素材探测状态（设置页「HD 素材」卡片用；只读，不触发注入） */
export interface HdEditionStatus {
  found: boolean;
  /** 命中的来源目录（`hd-edition` 本体）；未检测到为 null */
  dir: string | null;
  /** 可用档位（降序，如 [3, 2]）；未检测到为空数组 */
  scales: number[];
  /** 配置开关是否开启（`hd_edition_use != "off"`） */
  enabled: boolean;
}

/**
 * 探测 HD 素材是否可用（只读；非 Tauri 环境返回 null）。
 *
 * `opts` 为**可选目录覆盖**（首启向导：配置尚未落盘，需要按用户刚选的目录探测）；
 * 不传 = 沿用已保存配置（设置页行为不变）。
 */
export async function hdEditionStatus(opts?: {
  gameDir?: string;
  userDataDir?: string;
}): Promise<HdEditionStatus | null> {
  if (!isTauri()) return null;
  try {
    const args = opts
      ? { gameDir: opts.gameDir?.trim() || null, userDataDir: opts.userDataDir?.trim() || null }
      : undefined;
    return (await invoke("hd_edition_status", args)) as HdEditionStatus;
  } catch {
    return null;
  }
}

/** 根据游戏目录推导用户数据目录（dirs.json 优先，否则默认 My Games/vcmi） */
export async function suggestUserDataDir(
  gameDir: string
): Promise<{ gameDir: string; userDataDir: string; userConfigDir: string; userModsDir: string; exists: boolean } | null> {
  if (!isTauri()) return null;
  try {
    return await invoke("suggest_user_data_dir", { gameDir });
  } catch {
    return null;
  }
}
