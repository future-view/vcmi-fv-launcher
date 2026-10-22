import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { backendErrorText, feErr } from "@/lib/backendError";

/** 关闭当前窗口（Tauri 桌面端有效） */
export async function closeWindow(): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("close_window");
  } catch (e) {
    console.error("关闭窗口失败", e);
  }
}

/** 最小化当前窗口（Tauri 桌面端有效） */
export async function minimizeWindow(): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("minimize_window");
  } catch (e) {
    console.error("最小化窗口失败", e);
  }
}

/**
 * 切换当前窗口最大化 / 还原（Tauri 桌面端有效）。
 *
 * 走自定义命令而非 JS 的 `window.maximize()`：`core:window:default` 只含只读状态查询
 * 与 `allow-internal-toggle-maximize`，显式 `maximize`/`unmaximize` 需额外 capability；
 * 自定义命令与既有的 `close_window`/`minimize_window` 保持一致。
 */
export async function toggleMaximizeWindow(): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("toggle_maximize_window");
  } catch (e) {
    console.error("切换窗口最大化失败", e);
  }
}

/** 当前窗口是否处于最大化（浏览器 mock 模式恒为 false）。`allow-is-maximized` 在默认权限内。 */
export async function isWindowMaximized(): Promise<boolean> {
  if (!isTauri()) return false;
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    return await getCurrentWindow().isMaximized();
  } catch {
    return false;
  }
}

/**
 * 订阅窗口尺寸变化，用于同步「最大化 / 还原」图标。
 *
 * 必须监听系统事件而不是只信自己维护的布尔值：最大化后用户拖动还原、
 * 或双击标题栏最大化，都不会经过本应用的按钮。返回取消订阅函数。
 */
export async function onWindowResized(cb: () => void): Promise<() => void> {
  if (!isTauri()) return () => {};
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    return await getCurrentWindow().onResized(() => cb());
  } catch {
    return () => {};
  }
}

/**
 * 订阅窗口**焦点**变化（Tauri v2 window API，`onFocusChanged`）。
 *
 * 用途：后台轮询类任务（如 wiki 构建进度的 400ms 兜底轮询）在窗口失焦时
 * 暂停 —— 用户把鼠标/注意力移出窗口后，主线程不该再被周期性 IPC 持续唤醒
 * （真机反馈：构建期鼠标移出窗口会不定期遇到光标转圈，即 WebView 对
 * `WM_SETCURSOR` 的响应被后台工作延迟）。恢复焦点时由调用方立即补一次。
 *
 * 非 Tauri / 失败返回 `null`（调用方退化为"始终视为聚焦"）。
 */
export async function onWindowFocusChanged(
  cb: (focused: boolean) => void,
): Promise<(() => void) | null> {
  if (!isTauri()) return null;
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    return await getCurrentWindow().onFocusChanged(({ payload: focused }) => cb(focused));
  } catch {
    return null;
  }
}

/**
 * 设计稿画布（**全站唯一坐标系**，单位 = CSS 像素，16:9）。
 *
 * 与「阵营巡礼」播放页的定尺画布（`ShowcaseOverlay`：`scale = min(vw/DW, vh/DH)`）
 * 是**同一个** 1280×720。窗口严格 16:9 时 `Z = H / DH`，于是
 * ```
 * 逻辑视口 = (W/Z) × (H/Z) = (16/9·DH) × DH = DW × DH
 * ```
 * **恒等于设计稿** —— 1080p / 2K / 4K 下排布完全一致，只有物理放大倍数不同。
 */
export const DESIGN_CANVAS_W = 1280;
export const DESIGN_CANVAS_H = 720;
// 默认窗口尺寸的常量（最小尺寸 / 0.85 填充 / 1920×1080 上限）已随计算逻辑一起搬到
// Rust 侧：`infra/window.rs`（窗口必须在显示前定尺寸，前端拿不到那个时机）。
/** 窗口尺寸变化后重算缩放的防抖（拖动 / 最大化动画结束后只算一次） */
const UI_SCALE_RESIZE_DEBOUNCE_MS = 120;

/**
 * 由**客户区逻辑尺寸**推导界面缩放（窗口驱动）。
 *
 * 设计稿 1280×720 = 1.0（与 `DESIGN_CANVAS_*` 同值）：窗口比设计稿大多少，UI 就等比放大多少，
 * 于是「最大化窗口」不会让元素显得更小——任何屏幕上最大化后的视觉密度一致。
 * 取横纵比例的**较小者**（不会从短边溢出），并**下限 1.0**：窗口小于设计稿时不缩小，
 * 保持设计稿的可读性，超出部分交给页面内部滚动。
 */
export function computeWindowScale(logicalWidth: number, logicalHeight: number): number {
  if (!(logicalWidth > 0) || !(logicalHeight > 0)) return 1;
  return Math.max(
    1,
    Math.min(logicalWidth / DESIGN_CANVAS_W, logicalHeight / DESIGN_CANVAS_H),
  );
}

/** 已应用的 WebView 缩放（值未变时跳过 setZoom，避免无谓重排） */
let appliedZoom = 1;

/**
 * 安装「窗口尺寸驱动」的界面缩放：启动算一次，窗口尺寸变化（最大化 / 还原 / 拖动）后防抖重算。
 *
 * **输入必须用 Tauri 的窗口逻辑尺寸（物理尺寸 ÷ scaleFactor），不能用 `window.innerWidth`**：
 * WebView 缩放会改变 `innerWidth`，用它推导会形成「缩放 → 视口变化 → 再缩放」的反馈震荡；
 * 窗口逻辑尺寸只与窗口本身有关、与缩放无关，天然稳定。
 *
 * 整页缩放会把 px / rem / vh 一并等比放大，故主界面、领域页、抽屉、弹层全部自动生效。
 */
export async function installUiScale(): Promise<void> {
  if (!isTauri()) return;
  let timer: number | undefined;
  const apply = async () => {
    try {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const win = getCurrentWindow();
      const [physical, dpr, minimized] = await Promise.all([
        win.innerSize(),
        win.scaleFactor(),
        win.isMinimized(),
      ]);
      // 最小化（或其瞬间的退化尺寸）不重算缩放：Windows 下最小化会触发
      // `Resized(0×0)`，若照常计算 `computeWindowScale` 兜底返回 1 → 整页缩放回
      // 100%，还原时再跳回原比例，造成可见的「缩放比例重新调整」。最小化期间
      // 窗口不可见，本就该保持原有缩放不变；还原后的真实尺寸与最小化前相同，
      // 下面 `scale` 与 `appliedZoom` 一致会被跳过，故全程无跳变。
      if (minimized) return;
      const logical = physical.toLogical(dpr);
      // 退化尺寸（宽或高为 0，极少数 Windows 构建在还原首帧仍会短暂报 0）同样忽略，
      // 避免错误地 `setZoom(1)`。
      if (!(logical.width > 0) || !(logical.height > 0)) return;
      const scale = computeWindowScale(logical.width, logical.height);
      if (Math.abs(scale - appliedZoom) < 0.005) return;
      const { getCurrentWebview } = await import("@tauri-apps/api/webview");
      await getCurrentWebview().setZoom(scale);
      appliedZoom = scale;
    } catch (e) {
      console.warn("应用界面缩放失败", e);
    }
  };
  await apply();
  await onWindowResized(() => {
    window.clearTimeout(timer);
    timer = window.setTimeout(() => void apply(), UI_SCALE_RESIZE_DEBOUNCE_MS);
  });
}

/**
 * ⚠️ 窗口初始尺寸**不在前端计算 / 应用**：由 Rust 在窗口显示前定好 ——
 * `infra/window.rs::compute_window_size`（唯一公式来源）+ `commands/app.rs::apply_main_window_geometry`，
 * 配合 `tauri.conf.json` 的 `"visible": false` 与 setup 里的 `show()`。
 *
 * 前端参与会带来两个问题：① 窗口先以配置默认尺寸露出、再跳大（可见跳变）；
 * ② Windows 的 `SetWindowPos` 改尺寸**以左上角为锚**向右下扩张，放大后右下角顶出屏幕。
 * 前端只**消费**结果：`installUiScale` 读到的已是最终逻辑尺寸，首帧缩放即正确。
 * 持久化字段仍是 `launcher.json` 的 `ui.window_size`（snake_case），改默认策略时清掉它即可回到自适应。
 */


export interface VcmiDirs {
  /** 是否探测到有效目录组合（仅用于回填初始化表单，不代表已就绪） */
  detected?: boolean;
  ready: boolean;
  exeDir: string;
  gameDir: string;
  userDir: string;
  userConfigDir: string;
  logsDir: string;
  userModsDir: string;
  savesDir?: string;
  gameModsDir?: string;
  fromConfig?: boolean;
  /**
   * 启动器专属用户数据目录（exe 同目录 `userdata`）：首启向导「自定义用户数据目录」的默认值。
   * 与 `systemUserDataDir` 一起由 `detect_vcmi_paths` 返回（**只读**，无任何磁盘副作用）。
   */
  portableUserDataDir?: string;
  portableUserDataDirExists?: boolean;
  /**
   * 解析出的用户目录（`userDir`）是否已存在于磁盘。
   *
   * 它是「复制既有数据」的**首选源**：`dirs.json` 可能把用户目录指向别处
   * （如增强版），此时官方默认目录里并没有真正的数据。
   */
  userDirExists?: boolean;
  /** VCMI 官方默认用户数据目录（`文档\My Games\vcmi`）：无 `dirs.json` 时的复制源 */
  systemUserDataDir?: string;
  systemUserDataDirExists?: boolean;
}

export interface LauncherConfigData {
  gameDir: string;
  userDataDir: string;
  /** 可选：留空时后端按 VCMI 规则从 userDataDir 推导 */
  userConfigDir?: string;
  /** 可选：留空时后端按 VCMI 规则从 userDataDir 推导 */
  userModsDir?: string;
  /** 界面语言（VCMI 语言标识，如 chinese/english/russian/polish/german）。向导保存时一并写入，供便携包统一语言。 */
  uiLang?: string;
  /**
   * 是否接管 `game_dir/config/dirs.json` 的 5 个 key 指向 `userDataDir`
   * （`paths.redirect_user_data`）。首启向导选「启动器专属用户数据目录」时传 `true`。
   * 缺省（不传）= 沿用既有配置，旧调用行为不变。
   */
  redirectUserData?: boolean;
  /**
   * 保存前把该目录**迁移**到 `userDataDir`（目标已存在 / 源不存在则静默跳过）。
   *
   * ⚠️ 顺序由后端保证：迁移 → 写 launcher.json → 接管 dirs.json。
   * 迁移中途关闭启动器不会留下任何已生效的配置（下次启动重入向导）。
   *
   * 现仅由「同步数据」按钮配合 {@link copyUserData}（`copyOnly`）触发：
   * **保存按钮不再隐式复制**（单一职责，进度可见可跳过）。
   */
  copyFrom?: string | null;
  /**
   * `copyOnly: true` = **只迁移、不写任何配置**（向导第 2 步的「同步数据」按钮）。
   * 复用 `write_launcher_config` 而非新增命令（后端：新增 Tauri 命令会触发 never-type 硬错误）。
   */
  copyOnly?: boolean;
  /**
   * 仅置「跳过复制」哨兵并立即返回（不校验 gameDir），供向导的「跳过复制」按钮使用。
   * 复用 `write_launcher_config` 而非新增命令（后端：新增 Tauri 命令会触发 never-type 硬错误）。
   */
  cancelCopy?: boolean;
}

export function isTauri(): boolean {
  return typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__ != null;
}

/** 浏览器自测模式（非 Tauri）下，mock 缓存数据/图片的 URL 前缀（由 vite mock-cache 中间件服务） */
export function mockCacheBase(): string {
  return "/mock-cache/mods";
}

/** 非 Tauri 环境下 fetch mock 缓存 JSON，统一转 WikiReadResult 形状 */
async function fetchMockCacheJson(rel: string): Promise<WikiReadResult | null> {
  try {
    const r = await fetch(`${mockCacheBase()}/${rel}`, { cache: "no-cache" });
    if (!r.ok) return null;
    const data = await r.json();
    return { ok: true, data } as WikiReadResult;
  } catch {
    return null;
  }
}

/** 当前是否为调试构建（tauri build --debug / tauri dev）。用于显示“按 F12 打开控制台”提示 */
export async function isDebug(): Promise<boolean> {
  if (!isTauri()) return import.meta.env.DEV;
  try {
    return await invoke<boolean>("is_debug");
  } catch {
    return false;
  }
}

/** 探测当前 VCMI 目录（优先 launcher.json，其次自动探测） */
export async function detectVcmiPaths(): Promise<VcmiDirs | null> {
  // 浏览器自测模式：由 mock launcher.json 组装就绪目录，跳过首启向导
  if (!isTauri()) {
    const cfg = await readLauncherConfig();
    if (!cfg?.gameDir) return null;
    return {
      ready: true,
      exeDir: cfg.gameDir,
      gameDir: cfg.gameDir,
      userDir: cfg.userDataDir ?? "",
      userConfigDir: cfg.userConfigDir ?? "",
      logsDir: "",
      userModsDir: cfg.userModsDir ?? "",
      fromConfig: true,
    } as VcmiDirs;
  }
  try {
    return await invoke<VcmiDirs>("detect_vcmi_paths");
  } catch {
    // 探测失败视作「目录未就绪」：交给初始化向导，而不是把异常抛进启动编排
    return null;
  }
}

/** 持久化 launcher.json 的读回结构（含设置面板视图偏好） */
export interface LauncherConfig {
  gameDir: string;
  userDataDir: string;
  userConfigDir?: string;
  userModsDir?: string;
  showAllSettings?: boolean;
  /** 设置面板可见级别：common / advanced / all */
  settingsLevel?: "common" | "advanced" | "all";
  /** 是否显示详细描述 */
  settingsShowDesc?: boolean;
  /**
   * **AI 高清增强**（默认 false）：用 Real-ESRGAN 超分插画类素材。
   *
   * ⚠️ 这只是"开关"：真正生效还要求 {@link LauncherConfig.aiModel} 有值且文件存在
   * （**无默认模型**）。发行版可预置
   * `"hd": { "ai": { "enhance": true, "model": "realesrgan_anime6b.onnx" } }`。
   */
  aiHdEnhance?: boolean;
  /**
   * **AI 模型文件名**（组件目录 `runtime/ai/` 内的 `*.onnx`，缺省 / 空 = **AI 不生效**）。
   *
   * 候选来自 {@link listAiModels} 的目录扫描（含用户自备模型）；
   * 想知道某个已知模型是否损坏，用 {@link verifyAiModel}。
   */
  aiModel?: string;
  /**
   * **高清素材生成总开关**（默认 false）：开启后才生成 `@Nx` 变体。
   * 关闭时前端**完全不传 hd 参数** → 后端直接跳过高清分支（"标清可用"的最快路径）。
   */
  hdEnhance?: boolean;
  /**
   * **HD 素材来源**（官方 HD 版素材 mod `hd-edition` 的 `.pak`，默认 `"auto"`）：
   * - `"auto"`：检测到即用 —— `@Nx` 变体优先取真高清生成；**城镇幕布与兵种背景**的基准图
   *   也会换成 HD 尺寸的合成图（其余类的基准图仍是原版 1x）；
   * - `"off"`：完全不用（全链路退化为"原版素材放大"）。
   *
   * ⚠️ 只在**构建期**读 pak，产物仍落 `.vcmi-fv-launcher/mods/`，运行期不读 pak。
   */
  hdEditionUse?: string;
  /** HD 素材档位：`"auto"`（取可用最高档，x3 优先）/ `"x2"`（只用 x2，省磁盘） */
  hdEditionScale?: string;
  /** 内置预设（发行版可在 launcher.json 追加，与源码内置 `__official__` 合并） */
  builtinPresets?: BuiltinPresetConfig[];
  /** 全局强制模组（潜规则）：每次写 modSettings 始终启用，用户不可关闭 */
  requiredMods?: string[];
  /** 界面语言（VCMI 语言标识）：便携包可预置，前端启动时据此覆盖系统语言探测值 */
  uiLang?: string;
  /** 日志等级：error|warn|info|debug|trace */
  logLevel?: string;
  /** 启动器窗口尺寸（首次启动按显示器自适应后保存，逻辑像素） */
  windowSize?: { width: number; height: number };
  /** 自动更新：清单地址 / 自动检查 / 跳过版本（见「启动器自动更新」章节） */
  updateManifestUrl?: string;
  updateAutoCheck?: boolean;
  updateSkipVersion?: string;
  /** 模组列表 hover 时是否弹出城镇背景大图（默认 false，真值见 launcher.json `ui.town_preview`） */
  townPreview?: boolean;
  /** 网络代理（launcher.json `network.proxy`）：http(s) 地址；null/空/off = 直连（默认） */
  networkProxy?: string | null;
  /** GitHub 镜像规则表（launcher.json `network.mirror_rules`，L2）：声明序=尝试序；空=不用（默认不内置） */
  mirrorRules?: MirrorRule[] | null;
  /** 「自动检测更新」开关（launcher.json `realms.mod_auto_check`，默认开）：仅领域模组管理页生效，
   * 打开管理页时静默检查一次模组仓库索引（fetch_mod_index，与 mods 页同一索引与缓存）；
   * null / 缺省 = 开 */
  realmsModAutoCheck?: boolean | null;
  /** 「导出领域」上次用的仓库目录（launcher.json `realms.publish_repo_dir`）；null/缺省 = 未配置 */
  realmsPublishRepoDir?: string | null;
  /** 下载前缀的**人工覆盖值**（launcher.json `realms.publish_url_base`）；留空 = 后端自动推导 */
  realmsPublishUrlBase?: string | null;
  /** 导出弹窗上次选的分支（launcher.json `realms.publish_mode`）；null/缺省 = share */
  realmsPublishMode?: string | null;
}

/** 单条 GitHub 镜像规则（与后端 `infra::http::MirrorRule` 对齐，见方案 §3.2.2） */
export interface MirrorRule {
  enabled: boolean;
  note?: string | null;
  hosts: string[];
  mode: "prefix" | "template";
  prefix?: string | null;
  template?: string | null;
}

/** launcher.json 中内置预设的配置项（与 src/builtinPresets.ts 的 BuiltinPreset 对齐） */
export interface BuiltinPresetConfig {
  id: string;
  label?: Record<string, string>;
  mods?: string[];
  modsByLang?: Record<string, string[]>;
  locked?: boolean;
}

/** 读取持久化的 launcher.json */
export async function readLauncherConfig(): Promise<LauncherConfig | null> {
  // 浏览器自测模式：读 mock 缓存根旁的真实 launcher.json（分组 snake_case → camelCase），跳过首启向导
  if (!isTauri()) {
    try {
      const r = await fetch("/mock-cache/launcher.json", { cache: "no-cache" });
      if (!r.ok) return null;
      const raw = (await r.json()) as Record<string, unknown>;
      const paths = (raw.paths ?? {}) as Record<string, unknown>;
      const ui = (raw.ui ?? {}) as Record<string, unknown>;
      const hd = (raw.hd ?? {}) as Record<string, unknown>;
      const hdAi = (hd.ai ?? {}) as Record<string, unknown>;
      const runtime = (raw.runtime ?? {}) as Record<string, unknown>;
      const realms = (raw.realms ?? {}) as Record<string, unknown>;
      return {
        gameDir: String(paths.game_dir ?? ""),
        userDataDir: String(paths.user_data_dir ?? ""),
        userConfigDir: paths.user_config_dir ? String(paths.user_config_dir) : undefined,
        userModsDir: paths.user_mods_dir ? String(paths.user_mods_dir) : undefined,
        showAllSettings: ui.show_all_settings as boolean | undefined,
        settingsLevel: ui.settings_level as LauncherConfig["settingsLevel"],
        settingsShowDesc: ui.settings_show_desc as boolean | undefined,
        townPreview: ui.town_preview === true,
        aiHdEnhance: hdAi.enhance === true,
        aiModel: hdAi.model ? String(hdAi.model) : undefined,
        hdEnhance: hd.enhance === true,
        hdEditionUse: hd.edition_use ? String(hd.edition_use) : "auto",
        hdEditionScale: hd.edition_scale ? String(hd.edition_scale) : "auto",
        uiLang: ui.lang as string | undefined,
        logLevel: runtime.log_level as string | undefined,
        realmsModAutoCheck: realms.mod_auto_check as boolean | undefined,
        realmsPublishRepoDir: realms.publish_repo_dir ? String(realms.publish_repo_dir) : null,
        realmsPublishUrlBase: realms.publish_url_base ? String(realms.publish_url_base) : null,
        realmsPublishMode: realms.publish_mode ? String(realms.publish_mode) : null,
      };
    } catch {
      return null;
    }
  }
  try {
    return await invoke<LauncherConfig>("read_launcher_config");
  } catch {
    // 命令失败（如阻塞任务异常）与"无配置"同义：交给调用方走未就绪路径，不把异常抛进启动编排
    return null;
  }
}

/** 保存 launcher.json 并刷新后端路径状态 */
export async function writeLauncherConfig(data: LauncherConfigData): Promise<void> {
  if (!isTauri()) throw new Error(feErr("frontend_not_tauri"));
  return invoke("write_launcher_config", { data });
}

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

/** 打开文件夹选择对话框（`title` 必传：由调用方从 i18n 词典取词，避免硬编码回退） */
export async function pickDirectory(title: string): Promise<string | null> {
  if (!isTauri()) return null;
  const path = await openDialog({ directory: true, title });
  return path ? String(path) : null;
}

/** 选择单个文件（可传扩展名过滤，如 `["exe"]`）；非 Tauri 环境返回 null */
export async function pickFile(title: string, extensions?: string[]): Promise<string | null> {
  if (!isTauri()) return null;
  const path = await openDialog({
    directory: false,
    title,
    filters: extensions && extensions.length > 0 ? [{ name: title, extensions }] : undefined,
  });
  return path ? String(path) : null;
}

/**
 * 轮询批量构建进度（事件兜底：busy 时每 400ms 拉取一次）。
 *
 * `prepDone`/`prepTotal` = 准备阶段（成本估算）进度，与 `done`/`total` 分开记账
 * （两者计数域同为 `0..mods.len()`，混用会让进度条"冲满后归零"）。
 */
export async function fetchWikiBuildProgress(): Promise<{
  done: number;
  total: number;
  prepDone: number;
  prepTotal: number;
} | null> {
  if (!isTauri()) return null;
  try {
    return await invoke("cache_build_progress");
  } catch {
    return null;
  }
}

/** 请求取消正在进行的批量构建（已分发到线程的 mod 仍会跑完，之后不再构建新 mod） */
export async function cancelWikiBuild(): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("cancel_cache_build");
  } catch {
    // 命令不可用或构建已结束，忽略
  }
}

/* --------------------------- 领域加急队列（S1） --------------------------- */

/** 领域加急构建的返回（与 `buildModsWikiBatch` 同形） */
export interface RealmUrgentResult {
  ok: boolean;
  total: number;
  failed: string[];
  hd?: { generated: number; skipped: number; failed: number };
}

/**
 * **领域加急构建**：只为「当前领域所需素材」构建，进度走**独立通道**。
 *
 * 与 `buildModsWikiBatch` 的三点刻意差异（服务端实现）：
 * 1. 独立进度/取消计数器（`BuildSlot::realm`）—— 不会被启动时的全局构建覆盖；
 * 2. turbo 档 —— 用户在进度页等着，不再"空闲才提档"；
 * 3. 两阶段一次跑完 —— 省一次 IPC 往返。
 */
export async function buildRealmCacheUrgent(
  gameDir: string,
  mods: { modId: string; modPath: string }[],
  onProgress?: (done: number, total: number) => void,
  hd?: HdBuildOptions,
): Promise<RealmUrgentResult | null> {
  if (!isTauri() || mods.length === 0) return null;
  const hdArgs =
    hd && hd.rate >= 2
      ? hd.upscaler
        ? { hdRate: hd.rate, hdPhys: hd.phys, hdAi: hd.ai, hdUpscaler: hd.upscaler }
        : { hdRate: hd.rate, hdPhys: hd.phys, hdAi: hd.ai }
      : {};
  const run = () =>
    invoke<RealmUrgentResult>("build_realm_cache_urgent", { gameDir, mods, ...hdArgs });
  if (!onProgress) {
    try {
      return await run();
    } catch {
      return null;
    }
  }
  try {
    const { listen } = await import("@tauri-apps/api/event");
    const un = await listen<{ done: number; total: number }>("realm-cache-build-progress", (e) => {
      onProgress(e.payload.done, e.payload.total);
    });
    try {
      return await run();
    } finally {
      un();
    }
  } catch {
    return null;
  }
}

/** 领域加急构建进度（轮询兜底：事件在部分环境可能丢失） */
export async function fetchRealmBuildProgress(): Promise<{ done: number; total: number } | null> {
  if (!isTauri()) return null;
  try {
    return await invoke<{ done: number; total: number }>("realm_cache_build_progress");
  } catch {
    return null;
  }
}

/** 取消领域加急构建（进度页「返回」按钮） */
export async function cancelRealmCacheBuild(): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("cancel_realm_cache_build");
  } catch {
    // 命令不可用或构建已结束，忽略
  }
}

/**
 * 领域所需素材是否**已就绪**（领域信息 / 阵营巡礼入口的守门人）。
 *
 * 检查失败时返回 `ready: true`：宁可让用户进去看占位图，也不要卡在入口。
 */
export async function checkRealmAssets(
  ids: string[],
): Promise<{ ready: boolean; missing: string[] }> {
  if (!isTauri() || ids.length === 0) return { ready: true, missing: [] };
  try {
    const r = await invoke<{ ready: boolean; missing: string[] }>("check_realm_assets", { ids });
    return { ready: r?.ready !== false, missing: r?.missing ?? [] };
  } catch {
    return { ready: true, missing: [] };
  }
}

/**
 * 模组扫描的**进程内记忆**（模块级 promise）。
 *
 * 理由（见 `docs/features/startup-responsiveness.zh-CN.md`）：
 * - 视图互斥卸载：切走再切回「模组」页会重挂载 ModView 并重跑全量扫描（R5）；
 * - 启动期 `ensureWikiCacheBuilt` 与领域侧 `realmAssets` / `buildTargets` 也各扫一次 ——
 *   全是同一份磁盘状态。
 *
 * ⚠️ 安装 / 删除 / 更新模组后**必须**调用 [`invalidateModsScan`]，否则会用到陈旧列表。
 */
let modsScanPromise: Promise<unknown[]> | null = null;

/** 扫描真实 Mods 目录（进程内只真正扫一次，见 `modsScanPromise`） */
export async function scanRealMods(): Promise<unknown[]> {
  if (!isTauri()) return [];
  if (!modsScanPromise) {
    modsScanPromise = invoke<unknown[]>("scan_mods").catch((e) => {
      // 失败不缓存：一次瞬时失败不该让整个会话卡在空列表（下次调用可重试）
      modsScanPromise = null;
      throw e;
    });
  }
  return modsScanPromise;
}

/** 使模组扫描的进程内记忆失效（安装 / 删除 / 更新模组后必须调用） */
export function invalidateModsScan(): void {
  modsScanPromise = null;
}

/** 重扫：先失效进程内记忆，再取最新列表（安装 / 删除 / 更新后的唯一正确入口） */
export async function rescanRealMods(): Promise<unknown[]> {
  invalidateModsScan();
  return scanRealMods();
}

/**
 * 多线程批量构建 Wiki 缓存（进度经 wiki-build-progress 事件推送）。
 *
 * `throttle` 为构建档位，后端据此限制 rayon 线程数（并发度即限速）：
 * 0=eco（默认，后台静默构建）/ 1=normal / 2=turbo（用户主动触发或长时间空闲）。
 *
 * `phase` 为构建阶段：0=仅数据（阶段一，廉价）/ 1=仅素材（阶段二，重活）/ 2=两阶段。
 * 首次启动按「全部 mod 阶段一 → 全部 mod 阶段二」分批调用，使 wiki/realm 页面
 * 先拿到全部文本数据（无图可接受），图片再由阶段二在后台补齐。
 */
export interface HdBuildOptions {
  /** 倍率**上限**（**≥2 才生成**；1 = 只用原素材）。由显示器物理宽推导，见 `lib/hd.ts` */
  rate: number;
  /**
   * 基准物理倍率 = 最大显示器物理宽 / 1280（见 `lib/hd.ts::computeHdPhys`）。
   *
   * 后端用它**逐图按用途分级**算倍率（`需求物理 = 目标CSS × phys`）：
   * 兵种/英雄拿高倍率、城镇幕布只拿够用的倍率、源图已够大的跳过。
   * 缺省时后端按 `rate` 反推上界（钳位情形会偏保守）。
   */
  phys?: number;
  /** 放大算法 id；缺省 = 后端默认算法（xBRZ） */
  upscaler?: string;
  /**
   * **AI 高清增强**（`launcher.json` 的 `aiHdEnhance`，默认关）。
   * 开启后插画档走 Real-ESRGAN 超分（**仅 4x**），失败静默回退 Lanczos3；像素画仍走 xBRZ。
   */
  ai?: boolean;
}

export async function buildModsWikiBatch(
  gameDir: string,
  mods: { modId: string; modPath: string }[],
  onProgress?: (done: number, total: number, prep?: boolean) => void,
  throttle: 0 | 1 | 2 = 0,
  phase: 0 | 1 | 2 = 2,
  /** 高清素材生成选项（缺省 = 不生成）。只在素材阶段（phase 1/2）生效 */
  hd?: HdBuildOptions
): Promise<{ ok: boolean; total: number; failed: string[]; hd?: { generated: number; skipped: number; failed: number } } | null> {
  if (!isTauri()) return null;
  // `rate < 2` 时**不带任何 hd 参数**（而不是传 1）：后端据此完全跳过高清分支
  const hdArgs =
    hd && hd.rate >= 2
      ? hd.upscaler
        ? { hdRate: hd.rate, hdPhys: hd.phys, hdAi: hd.ai, hdUpscaler: hd.upscaler }
        : { hdRate: hd.rate, hdPhys: hd.phys, hdAi: hd.ai }
      : {};
  if (onProgress) {
    try {
      const { listen } = await import("@tauri-apps/api/event");
      // `prep=true` = 准备阶段（排序前的 mod 成本估算，UI 的"扫描模组目录"行）。
      // 冷缓存下它是分钟级且发生在第一个 mod 之前，不透传就会表现为"0% 静止一分钟"。
      let un: (() => void) | null = null;
      const startListen = async () => {
        if (un) return;
        un = await listen<{ done: number; total: number; prep?: boolean }>(
          "wiki-build-progress",
          (e) => {
            onProgress(e.payload.done, e.payload.total, e.payload.prep === true);
          },
        );
      };
      const stopListen = () => {
        un?.();
        un = null;
      };
      await startListen();
      // 窗口失焦时**退订事件**（不只停轮询）：Tauri 事件是 push 进 JS 的，
      // 失焦后每秒 ~10 次的回调 + 后续 status 处理仍是主线程的持续唤醒源。
      // 后端构建与窗口焦点无关，照常推进；聚焦后立即重订，后端的下一个事件
      // 即把进度追上（完成事件同样会到，蒙层不会卡在旧值）。
      const unFocus = await onWindowFocusChanged((focused) => {
        if (focused) void startListen();
        else stopListen();
      });
      try {
        return await invoke("build_mods_cache_batch", { gameDir, mods, throttle, phase, ...hdArgs });
      } finally {
        stopListen();
        void unFocus?.();
      }
    } catch {
      return null;
    }
  }
  try {
    return await invoke("build_mods_cache_batch", { gameDir, mods, throttle, phase, ...hdArgs });
  } catch {
    return null;
  }
}

/** 预检返回：哪些 mod 需要重建（数据 / 素材），以及索引是否已可用。 */
export interface WikiCacheState {
  ok: boolean;
  total: number;
  indexReady: boolean;
  needData: string[];
  needAssets: string[];
}

/**
 * **启动预检**：本次是否需要重建缓存（与真实构建**同源判定**，见后端 `wiki_cache_state`）。
 *
 * 用途：`ensureWikiCacheBuilt` 据此在"全部命中"时**完全跳过构建**（不置 `busy` → 不弹进度蒙层），
 * 让 wiki / mods / 领域页直接用现有 `index.json` 渲染 —— 修复"每次启动都弹重建进度条"。
 * 返回 `null`（命令不可用 / 异常）时调用方须退化为"照常构建"，**绝不因预检失败而漏建**。
 */
export async function probeWikiCacheState(
  gameDir: string,
  mods: { modId: string; modPath: string }[]
): Promise<WikiCacheState | null> {
  if (!isTauri()) return null;
  try {
    return await invoke<WikiCacheState>("wiki_cache_state", { gameDir, mods });
  } catch {
    return null;
  }
}

/** 把领域**整合模组（bundled）**的资料分析进百科缓存 `.vcmi-fv-launcher/mods`。
 *
 * 这些模组随领域导入解包到 `{realm-dir}/vcmi/Mods/<modId>`，**不在全局已安装模组列表**里，
 * 因此 `ensureWikiCacheBuilt()`（只扫全局用户目录与游戏目录）永远不会构建它们 ——
 * 领域信息页会拿不到模组描述、整合的城镇也没有预览图。导入后调用本函数补齐。 */
export async function buildRealmBundledWiki(
  bundledMods: { modId: string; modPath: string }[],
): Promise<void> {
  if (!isTauri() || bundledMods.length === 0) return;
  try {
    const cfg = await readLauncherConfig();
    await buildModsWikiBatch(cfg?.gameDir ?? "", bundledMods);
  } catch {
    /* 缓存构建失败不影响导入结果，静默处理 */
  }
}

/**
 * 释放**整轮缓存构建**留在后端进程里的加速缓存，让内存回落。
 *
 * 时机很关键：一轮构建是「阶段一 → 阶段二（城镇批 / 其余批）→ 阶段三（高清）」共 3~5 次
 * 独立的 `build_mods_cache_batch`，后端单看一批无法判断"后面还有没有"，所以**批间不能清**
 * （清了下一批要重新扫一遍目录/解析一遍配置）。只有前端知道整轮是否结束，故由本函数收尾。
 *
 * 只清可重建的派生缓存（目录索引 / 配置解析结果 / 编码产物 / 翻译表 / 去重集合），
 * 不影响已落盘的任何产物；构建前调用也无害（幂等）。
 */
export async function releaseWikiCaches(): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("release_wiki_caches");
  } catch {
    // 命令不可用（旧后端）时忽略：不释放只是内存不回落，不影响功能
  }
}

/**
 * bundled 补建的**串行链**（模块级单链）。
 *
 * 为什么必须串行：bundled 补建与全局后台构建**共用** `build_mods_cache_batch`
 * 命令和 `wiki-build-progress` 事件通道（payload 无批次标识）—— 并发的两个
 * 批次会把对方的进度账本互相打乱（2026-10-21 真机实测：全局蒙层 data 行停在
 * 中途值 18/124 而 assets 已在 21/124），且两份 rayon 并发争抢 CPU
 * （光标忙碌的诱因之一）。连续打开多个领域时，两次 bundled 之间同样会互踩。
 */
let bundledWikiChain: Promise<void> = Promise.resolve();

/** 按「领域 id + 整合模组 id 列表」补建百科缓存，用于**激活领域后**。
 *
 * 编辑器里标记「整合」只是往 realm.json 记 id，模组文件要等激活时
 * （`activate_realm` 步骤 5）才复制到 `{realm-dir}/vcmi/Mods/<modId>`；
 * 这条路径不走导入，也不在全局构建的扫描范围内，故在此补齐。
 * 构建是增量的（源 hash 命中即跳过），重复调用开销很小。
 *
 * ⚠️ **排队执行**（模块级串行链）：与全局构建并发会互相污染进度 + 争抢 CPU，
 * 见 `bundledWikiChain` 注释。与全局构建的排队等待由调用方（`App.tsx`）
 * 用 `isWikiBuildActive()` 处理 —— tauri 层不反向依赖 modcache/status。
 */
export async function buildRealmBundledWikiByIds(
  realmId: string,
  modIds: string[],
): Promise<void> {
  if (!isTauri() || modIds.length === 0) return;
  const run = async () => {
    const dir = await realmDir(realmId);
    if (!dir) return;
    await buildRealmBundledWiki(
      modIds.map((id) => ({ modId: id, modPath: `${dir}/vcmi/Mods/${id}` })),
    );
  };
  // 串到链尾（前面的失败不影响后面的入链）
  bundledWikiChain = bundledWikiChain.then(run).catch(() => {});
  return bundledWikiChain;
}

/** 读取 modSettings.json。
 * `configDir` 可选：传入领域配置目录时读取该领域隔离目录下的 modSettings.json（§3.5.1）。 */
export async function readModSettingsJson(configDir?: string): Promise<Record<string, unknown> | null> {
  if (!isTauri()) return null;
  try {
    return await invoke("read_mod_settings", configDir ? { configDir } : {});
  } catch {
    return null;
  }
}

/** 写回 modSettings.json */
export async function writeModSettingsJson(data: unknown): Promise<void> {
  if (!isTauri()) throw new Error(feErr("frontend_not_tauri"));
  return invoke("write_mod_settings", { data });
}

/** 读取 VCMI settings.json。configDir 可选：传入领域配置目录时读取该领域隔离目录（§3.5.1） */
export async function readVcmiSettings(configDir?: string): Promise<Record<string, unknown> | null> {
  if (!isTauri()) return null;
  try {
    return await invoke("read_vcmi_settings", { configDir: configDir ?? null });
  } catch {
    return null;
  }
}

/** 写回 VCMI settings.json（入参为平面键字典，写出前还原为嵌套结构）。configDir 可选：写入领域目录（§3.5.1） */
export async function writeVcmiSettings(data: unknown, configDir?: string): Promise<void> {
  if (!isTauri()) throw new Error(feErr("frontend_not_tauri"));
  const payload = isPlainObject(data) ? unflatten(data as Record<string, unknown>) : data;
  return invoke("write_vcmi_settings", { data: payload, configDir: configDir ?? null });
}

/* ----------------------- 嵌套 <-> 平面键 转换 ----------------------- */
// VCMI settings.json 是嵌套结构（"general":{...}），而前端 schema 使用平面键
// （"general.language"）。读写时在此处做转换，对外文件保持 VCMI 原生嵌套。

function isPlainObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

/** 把嵌套对象展开为平面键字典（"a":{"b":1} -> "a.b":1） */
export function flatten(obj: Record<string, unknown>, prefix = ""): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(obj)) {
    const key = prefix ? `${prefix}.${k}` : k;
    if (isPlainObject(v)) Object.assign(out, flatten(v, key));
    else out[key] = v;
  }
  return out;
}

/** 把平面键字典还原为嵌套对象（"a.b":1 -> {a:{b:1}}） */
export function unflatten(flat: Record<string, unknown>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(flat)) {
    const parts = k.split(".");
    let cur = out;
    for (let i = 0; i < parts.length - 1; i++) {
      const p = parts[i];
      if (!isPlainObject(cur[p])) cur[p] = {};
      cur = cur[p] as Record<string, unknown>;
    }
    cur[parts[parts.length - 1]] = v;
  }
  return out;
}

/**
 * 判断平面键路径是否包含数组索引段（纯数字段，如 "logging.loggers.0"）。
 * 后端 remove_path 对数组索引执行缩位删除，易产生空洞/错位，故需特殊处理。
 */
function hasArrayIndexSegment(key: string): boolean {
  return key.split(".").some((seg) => seg !== "" && /^\d+$/.test(seg));
}

/**
 * 把数组元素路径归并到其所属的「数组键」整键。
 * 例："logging.loggers.0.name" / "logging.loggers.0" → "logging.loggers"。
 * 用途：重置时按整键删除数组（而非逐元素缩位删除），避免破坏数组结构。
 * 非数组路径原样返回。
 */
function collapseToArrayKey(key: string): string {
  const parts = key.split(".");
  const idx = parts.findIndex((seg) => seg !== "" && /^\d+$/.test(seg));
  if (idx === -1) return key;
  return parts.slice(0, idx).join(".");
}

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

export interface LaunchResult {
  ok: boolean;
  error?: string;
}

async function wrapLaunch(fn: () => Promise<void>): Promise<LaunchResult> {
  try {
    await fn();
    return { ok: true };
  } catch (e: any) {
    return { ok: false, error: backendErrorText(e) };
  }
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

/* ----------------------------- Wiki 数据源（mods/{modId}/cache.json + index.json） ----------------------------- */

/** 读取单个 mod 的 Wiki 缓存（命中缓存时不再解析原始 mod） */
export async function readModWikiCache(modId: string): Promise<WikiReadResult | null> {
  // 浏览器自测模式：直接读 mock 缓存目录的 cache.json
  if (!isTauri()) return fetchMockCacheJson(`${modId}/cache.json`);
  try {
    // 后端返回 **JSON 原文**（不在 Rust 侧解析）：省掉一次整份解析（1.7 MB ≈ 33 ms），
    // 且后端可缓存原文、命令层只传字符串。`JSON.parse` 本来就不可避免。
    const text = await invoke<string>("read_mod_cache", { modId });
    if (!text) return null;
    return { ok: true, data: JSON.parse(text) as Record<string, unknown> } as WikiReadResult;
  } catch {
    return null;
  }
}

/** 城镇预览合成（后台按需申请）：叠加 structures 到 townScreen 背景，结果缓存并写回 preview */
export async function generateTownPreview(
  modId: string,
  factionId: string
): Promise<{ ok: boolean; cached?: boolean; preview?: string; error?: string } | null> {
  if (!isTauri()) return null;
  try {
    return await invoke("generate_town_preview", { modId, factionId });
  } catch {
    return null;
  }
}

/** 批量构建所有 Town mod 的 Wiki 缓存并生成 mods/index.json 索引 */
export async function buildAllWikiCache(mods: unknown[]): Promise<WikiBuildAllResult | null> {
  if (!isTauri()) return null;
  try {
    return await invoke<WikiBuildAllResult>("build_all_mod_cache", { mods });
  } catch {
    return null;
  }
}

/** 读取 mods/index.json 索引 */
export async function readWikiIndex(): Promise<WikiReadResult | null> {
  // 浏览器自测模式：读 mock 缓存目录的 index.json
  if (!isTauri()) return fetchMockCacheJson("index.json");
  try {
    return await invoke<WikiReadResult>("read_cache_index");
  } catch {
    return null;
  }
}

/** 城镇卡片（领域信息页专用，后端只回传必要字段，避免读整份 cache.json） */
export interface TownCard {
  modId: string;
  factionId: string;
  /** 多语言名称 {chinese, english, ...} */
  name?: Record<string, string> | null;
  /** 多语言描述，或纯字符串 */
  description?: Record<string, string> | string | null;
  /** 合成预览图的相对路径（images/...，铺满全部建筑），可能为空 */
  preview?: string | null;
  /** 城镇底图相对路径（images/...）。原版 `core.<faction>` 只有它（合成图需 raw towns，原版没有） */
  background?: string | null;
}

/**
 * 批量读取**城镇卡片**最小字段。
 *
 * 替代"逐个 `read_mod_cache` 读整份 cache.json"：本例 86 个城镇合计 31 MB，
 * 走本命令只有数十 KB，领域信息页因此不必再做视口懒加载。
 */
export async function readTownCards(ids: string[]): Promise<TownCard[]> {
  if (!isTauri() || ids.length === 0) return [];
  try {
    const r = await invoke<{ cards?: TownCard[] }>("read_town_cards", { ids });
    return r?.cards ?? [];
  } catch {
    return [];
  }
}

/** 阵营巡礼单 slide 载荷（裁剪后的 cache 数据，设计 §8.1） */
export interface ShowcaseSlide {
  modId: string;
  factionId: string;
  name?: Record<string, string>;
  description?: Record<string, string>;
  /** 幕布：preview ?? background（相对缓存根，走 fullImgUrl） */
  backdrop?: string;
  /** 阵营音乐：磁盘已有 → 绝对路径；否则 null */
  music?: string | null;
  /** zip 内音乐描述（前端调 ensureShowcaseAudio 异步抽取）；无则 null */
  musicZip?: { modId: string; rel: string } | null;
  /** 城镇结构幕布坐标 + 金色描边图（§5.9），用于高亮罩住整栋建筑 */
  structures: { id: string; x: number; y: number; z: number; w: number; h: number; image: string }[];
  /** `upgradedFrom` = 升级来源（空数组/undefined = 基础形态），前端据此区分未升级 / 仅进阶 */
  creatures: {
    id: string;
    name?: Record<string, string>;
    /** 描述（多语言，与 `name` 同构）。取不到时省略 —— 前端显示「暂无描述」，**不编造** */
    description?: Record<string, string>;
    /**
     * 该生物的**巢穴建筑名**（多语言 → 名称数组）。
     * 来源：城镇配置里 `building.creatures` 的反查（`town.rs` 的 `creature_dwellings`）。
     * 前端据此做「招募建筑」的**权威匹配**（按名称比对建筑清单），
     * 比"按 level 猜 id"稳 —— 猜 id 在第三方模组上常落空，表现就是"没有招募建筑"。
     */
    dwellingNames?: Record<string, string[]>;
    level?: number;
    image?: string;
    imageLarge?: string;
    upgradedFrom?: string[];
  }[];
  creatureTotal: number;
  /** `className` 在缓存里是 VCMI 多语言对象（与 name 同构），个别模组也可能是纯字符串 */
  heroes: {
    id: string;
    name?: Record<string, string>;
    /** 描述（多语言）；原版英雄为特长说明（`specialty`） */
    description?: Record<string, string>;
    className?: Record<string, string> | string;
    image?: string;
  }[];
  buildings: {
    id: string;
    name?: Record<string, string>;
    /** 描述（多语言） */
    description?: Record<string, string>;
    image?: string;
  }[];
  /** 后端解析到的 VCMI AudioPath（诊断用；null = 该阵营配置里没有 musicTheme） */
  musicTheme?: string | null;
  /** 数据缺失原因（缓存未构建 / faction 不存在） */
  missing?: string;
}

/** 阵营巡礼请求项（前端带入 modPath 用于解析音乐，设计 §8.1） */
export interface ShowcaseEntry {
  modId: string;
  factionId: string;
  modPath?: string;
}

/** 读取阵营巡礼 slide 载荷（批量裁剪，避免读整份 cache.json） */
export async function readShowcaseSlides(entries: ShowcaseEntry[]): Promise<ShowcaseSlide[]> {
  if (!isTauri() || entries.length === 0) return [];
  try {
    const r = await invoke<{ slides?: ShowcaseSlide[] }>("read_showcase_slides", { entries });
    return r?.slides ?? [];
  } catch {
    return [];
  }
}

/** zip 内音乐单条目抽取落盘，返回绝对路径（null = 跳过/失败） */
export async function ensureShowcaseAudio(
  modPath: string,
  modId: string,
  rel: string,
): Promise<string | null> {
  if (!isTauri()) return null;
  try {
    const r = await invoke<{ ok: boolean; path?: string | null }>("ensure_showcase_audio", {
      modPath,
      modId,
      rel,
    });
    return r?.path ?? null;
  } catch {
    return null;
  }
}

/**
 * 批量读取**模组元数据**（`cache.json.modMeta`：多语言名/描述 + 作者/版本/类型/依赖）。
 *
 * 替代 `listInstalledMods()` 里"每个模组各读一次整份 cache.json"的开销
 * （实测 652 个 mod ≈ 126 MB）。只传真正需要的 id 列表。
 */
export async function readModMetas(
  ids: string[],
): Promise<Record<string, InstalledMod["meta"]>> {
  if (!isTauri() || ids.length === 0) return {};
  try {
    const r = await invoke<{ metas?: Record<string, InstalledMod["meta"]> }>("read_mod_metas", {
      ids,
    });
    return r?.metas ?? {};
  } catch {
    return {};
  }
}

/** 获取 mods 缓存根目录绝对路径（用于拼接 convertFileSrc 完整图片路径） */
let modsCacheDirPromise: Promise<string | null> | null = null;

export async function getModsCacheDir(): Promise<string | null> {
  if (!isTauri()) return null;
  // **模块级单次记忆**：后端每次都会 `create_dir_all` + 磁盘 IO，而列表页每张卡片都会调用
  // （数百张卡片 = 数百次 IPC + 建目录）。目录由游戏 / 用户目录推导，会话内不会变，可安全记忆。
  if (!modsCacheDirPromise) {
    modsCacheDirPromise = invoke<{ ok: boolean; cacheDir?: string }>("get_mods_cache_dir")
      .then((res) => res?.cacheDir ?? null)
      .catch(() => {
        modsCacheDirPromise = null; // 失败不缓存，下次调用可重试
        return null;
      });
  }
  return modsCacheDirPromise;
}

export interface WikiReadResult {
  ok: boolean;
  data?: unknown;
  error?: string;
}

export interface WikiBuildAllResult {
  ok: boolean;
  built?: number;
  cached?: number;
  skipped?: number;
  encrypted?: number;
  total?: number;
  indexPath?: string;
  totalIndexed?: number;
}

/* ----------------------------- 设置面板（VCMI settings.json） ----------------------------- */

/**
 * 深度相等比较（用于判断设置项是否等于官方默认值，决定是否写入）。
 */
function deepEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (typeof a !== typeof b) return false;
  if (a === null || b === null || typeof a !== "object") return a === b;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  const ka = Object.keys(a as object);
  const kb = Object.keys(b as object);
  if (ka.length !== kb.length) return false;
  return ka.every((k) => deepEqual((a as any)[k], (b as any)[k]));
}

/**
 * 读取 VCMI settings.json 实时内容，并与官方默认值合并补全缺失键。
 * 纯前端（非 Tauri）环境下返回官方默认值表，便于离线预览。
 */
export async function loadVcmiSettings(
  officialDefaults: Record<string, unknown>,
  configDir?: string
): Promise<Record<string, unknown>> {
  const base: Record<string, unknown> = { ...officialDefaults };
  const real = await readVcmiSettings(configDir);
  if (real) {
    // VCMI 文件是嵌套结构，展开为平面键后再合并，避免与 schema 平面键混用
    const flat = flatten(real);
    for (const [k, v] of Object.entries(flat)) base[k] = v;
  }
  return base;
}

/**
 * 计算"仅差异"部分：从 current 中剔除与默认值深度相等的键。
 * 设计文档第 13 行要求：只对与默认值不同的配置进行写入。
 */
function diffFromDefaults(
  current: Record<string, unknown>,
  defaults: Record<string, unknown>
): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(current)) {
    const def = defaults[k];
    // 官方未定义的键（如旧版遗留）一律保留
    if (def === undefined) {
      out[k] = v;
      continue;
    }
    if (!deepEqual(v, def)) out[k] = v;
  }
  return out;
}

/**
 * 单键写回 VCMI settings.json。
 * - 设为「非默认」值：直接更新该键（合并写）。
 * - 显式改回「默认值」：删除该键，让 VCMI 回退到内置默认（而非"不写"）。
 * 注意：不能走 diffFromDefaults 的"等于默认就剔除"逻辑——否则改回默认时
 * 差异为空、后端合并写不会动文件里的旧非默认值，造成"改不回去"的缺陷。
 */
export async function writeVcmiSetting(
  key: string,
  value: unknown,
  defaults: Record<string, unknown>,
  configDir?: string
): Promise<void> {
  const isKnown = key in defaults;
  const equalToDefault = isKnown && deepEqual(value, defaults[key]);
  if (equalToDefault) {
    // 改回默认：从文件中删除该键，让 VCMI 回退到内置默认。
    // 防御：仅对「非数组索引路径」执行删除——含索引的数组键若走 remove_path
    // 会逐元素缩位、破坏数组结构；此类键改为整键更新（保留其当前值）。
    if (!hasArrayIndexSegment(key)) {
      await writeVcmiSettings({ __vcmi_fv_delete__: [collapseToArrayKey(key)] }, configDir);
      return;
    }
  }
  // 非默认 / 或含数组索引的键：更新该键（平面键会被 writeVcmiSettings 还原为嵌套）
  await writeVcmiSettings({ [key]: value }, configDir);
}

/**
 * 重置为默认：读出当前 settings.json，收集其中所有「本启动器已知且可管理」的键
 * （即用户可能自定义过的官方参数），交由后端将该键从文件中删除 ⇒
 * 这些参数回到 VCMI 内置默认值；不写入任何硬编码默认值，也不动其它遗留键。
 * 这与设计目标一致：减少硬编码逻辑，跟随 VCMI 内置默认。
 */
export async function resetVcmiSettings(
  defaults: Record<string, unknown>,
  configDir?: string
): Promise<void> {
  const real = await readVcmiSettings(configDir);
  const flat = real ? flatten(real) : {};
  const keysToClear = new Set<string>();
  for (const k of Object.keys(flat)) {
    // 仅收集官方 schema 定义的已知键，遗留键不动
    if (k in defaults) {
      // 数组元素路径（如 logging.loggers.0）归并到整键（logging.loggers）删除，
      // 避免后端对数组做逐元素缩位删除而破坏结构
      keysToClear.add(collapseToArrayKey(k));
    }
  }
  // 通过 write_vcmi_settings 的删除指令（__vcmi_fv_delete__）真正移除这些键，
  // 使它们回退到 VCMI 内置默认值；不写入任何硬编码默认值。
  await writeVcmiSettings({ __vcmi_fv_delete__: [...keysToClear] }, configDir);
}

/* ----------------------------- 领域（Realm）系统 ----------------------------- */

/**
 * 领域元信息（设计 §6；唯一权威是 `{realm-dir}/realm.json`）。
 * 类型定义统一放在 `src/realm/types.ts`，此处仅转发，避免出现第二份结构定义。
 */
export type { Realm, RealmList } from "@/lib/realm/types";
import type { Realm, RealmList } from "@/lib/realm/types";

/** 列出全部领域 + 当前激活 id */
export async function listRealms(): Promise<RealmList> {
  if (!isTauri()) return { realms: [], activeRealm: null };
  try {
    return await invoke<RealmList>("list_realms");
  } catch {
    return { realms: [], activeRealm: null };
  }
}

/** 当前激活领域 id */
export async function getActiveRealm(): Promise<string | null> {
  if (!isTauri()) return null;
  try {
    const r = await invoke<{ activeRealm: string | null }>("get_active_realm");
    return r.activeRealm ?? null;
  } catch {
    return null;
  }
}

/** 取某领域隔离目录的 config 绝对路径（作为后续读写领域设置的 configDir） */
export async function realmConfigDir(realmId: string): Promise<string | null> {
  if (!isTauri()) return null;
  try {
    return await invoke<string>("realm_config_dir", { realmId });
  } catch {
    return null;
  }
}

/** 写入领域的**全部模组预设**（多预设支持，§3.3）：整体接管 `presets` 与 `activePreset`。
 * 这是「启用组合」的权威来源；与 realm.json 的 borrowed / bundled（携带清单）解耦。 */
export async function writeRealmPresets(
  realmId: string,
  presets: Record<string, { mods: string[]; settings?: Record<string, unknown> }>,
  activePreset: string,
): Promise<void> {
  return invoke("write_realm_presets", { realmId, presets, activePreset });
}

/**
 * 把启动器语言写入 VCMI 生效的 `settings.json` 的 `general.language`（语言设置层）。
 *
 * 必须与翻译模组注入**成对**执行：VCMI 会把 `language` 与 `general.language` 不一致的
 * `modType: "Translation"` 模组判为 BROKEN 并从激活预设中移除
 * （`references/vcmi/lib/modding/ModManager.cpp:829-832`），只写预设不写该设置会被立刻删掉。
 *
 * `configDir` 传入时作用于该领域配置目录，否则作用于全局用户配置目录。幂等（值一致不写盘）。
 */
export async function setVcmiLanguage(lang: string, configDir?: string): Promise<void> {
  if (!isTauri()) return;
  return invoke("set_vcmi_language", { lang, configDir });
}

/**
 * 按启动器语言同步某预设的「语言相关模组」（语言层）：从预设移除 `remove`、加入 `add`。
 * `configDir` 传入时作用于该领域隔离目录的 modSettings.json，否则作用于全局。
 * 返回 `add` 中磁盘上找不到的模组 id（仅用于提示，不阻断）。
 */
export async function patchPresetLangMods(
  configDir: string | undefined,
  presetId: string,
  add: string[],
  remove: string[],
): Promise<string[]> {
  if (!isTauri()) return [];
  const r = await invoke<{ missing?: string[] }>("patch_preset_lang_mods", {
    configDir,
    presetId,
    add,
    remove,
  });
  return r?.missing ?? [];
}

/** 新建领域（**id 必填**）：立即建目录并落盘 realm.json，返回创建后的领域 */
export async function createRealm(data: unknown): Promise<Realm | null> {
  if (!isTauri()) throw new Error(feErr("frontend_not_tauri"));
  return invoke<Realm>("create_realm", { data });
}

/** 领域 ID 可用性校验（新建弹窗实时校验）：`available=false` 时 `reason` ∈ empty/invalid/occupied */
export async function checkRealmId(
  id: string,
): Promise<{ available: boolean; reason: string }> {
  if (!isTauri()) return { available: true, reason: "" };
  return invoke<{ available: boolean; reason: string }>("check_realm_id", { id });
}

/** 更新已存在领域（按 id 合并字段） */
export async function updateRealm(realmId: string, data: unknown): Promise<void> {
  if (!isTauri()) throw new Error(feErr("frontend_not_tauri"));
  return invoke("update_realm", { realmId, data });
}

/** 删除领域（若激活中则先停用还原 dirs.json） */
export async function deleteRealm(realmId: string): Promise<void> {
  if (!isTauri()) throw new Error(feErr("frontend_not_tauri"));
  return invoke("delete_realm", { realmId });
}

/** 激活领域：改写 dirs.json 指向领域隔离目录 + 从全局提取用户设置（§9.7.2） */
export async function activateRealm(realmId: string): Promise<LaunchResult> {
  if (!isTauri()) return { ok: false, error: feErr("frontend_not_tauri") };
  const res = await wrapLaunch(() => invoke("activate_realm", { realmId }));
  // 领域激活会改变模组的可见集合（bundled 复制 / borrowed 借用）→ 扫描记忆必须失效，
  // 否则回到模组页看到的是陈旧列表（见 scanRealMods 的进程内记忆注释）
  if (res.ok) invalidateModsScan();
  return res;
}

/** 停用领域：用原文备份还原 dirs.json，设置回到全局 */
export async function deactivateRealm(): Promise<LaunchResult> {
  if (!isTauri()) return { ok: false, error: feErr("frontend_not_tauri") };
  const res = await wrapLaunch(() => invoke("deactivate_realm"));
  if (res.ok) invalidateModsScan();
  return res;
}

/** 切换领域「屏蔽游戏自带地图」阀门（§8.3），并持久化偏好 */
export async function setRealmMapsBlocked(realmId: string, blocked: boolean): Promise<LaunchResult> {
  if (!isTauri()) return { ok: false, error: feErr("frontend_not_tauri") };
  return wrapLaunch(() => invoke("set_realm_maps_blocked", { realmId, blocked }));
}

/** 进程守卫（§4.4）：强制结束**游戏本体**（「结束游戏」按钮；不动官方启动器） */
export async function killVcmi(): Promise<void> {
  if (!isTauri()) return;
  return invoke("kill_vcmi");
}

/** 崩溃自检（§4.5）报告 */
export interface RealmSelfCheck {
  ok: boolean;
  status: string;
  activeRealm: string | null;
  dirsRealm: string | null;
  /** 后端诊断原文（中文，仅供日志/兜底；UI 文案按 status 走 i18n） */
  message: string;
  /** 是否可由 `realm_self_heal` 安全自动修复 */
  fixable: boolean;
  /** 可选修复动作（§4.5）：heal=自动自愈；unlink/merge/revert=地图阀门的人工修复 */
  actions?: string[];
}

/** 崩溃自检（§4.5）：启动后调用，返回异常报告 */
export async function realmSelfCheck(): Promise<RealmSelfCheck | null> {
  if (!isTauri()) return null;
  try {
    return await invoke<RealmSelfCheck>("realm_self_check");
  } catch {
    return null;
  }
}

/** 崩溃自愈（§4.5）：执行安全自动修复，返回修复后的报告 */
export async function realmSelfHeal(): Promise<RealmSelfCheck | null> {
  if (!isTauri()) return null;
  try {
    return await invoke<RealmSelfCheck>("realm_self_heal");
  } catch {
    return null;
  }
}

/**
 * 修复崩坏的地图阀门形态（§8.3.3 / §4.5）。
 * action：`unlink` 删悬空联接 / `merge` 合并新出现的自带地图 / `revert` 彻底还原游戏目录。
 */
export async function repairGameMaps(action: string): Promise<LaunchResult> {
  if (!isTauri()) return { ok: false, error: feErr("frontend_not_tauri") };
  return wrapLaunch(() => invoke("repair_game_maps", { action }));
}

/** 已安装模组列表（供领域借用 / 整合选择，设计 §3.4）。返回 { mods: {id,name}[] } */
export interface InstalledMod {
  /** VCMI modId：磁盘目录名**小写**归一（与 VCMI 主键同形态） */
  id: string;
  /**
   * 磁盘目录原名（保留大小写、可能含空格）。
   *
   * ⚠️ 磁盘定位必须用它，**不要用 `id`** —— `id` 是小写归一后的 VCMI modId，
   * 在大小写敏感的文件系统（Linux/macOS）上 `join(id)` 找不到目录。
   * 缺省（旧后端）时只能以 `id` 兜底。
   */
  diskName?: string;
  /** 模组目录绝对路径（后端扫描回传；子模组的 `<父>/Mods/<子>` 布局靠它才准） */
  modPath?: string;
  name: string;
  /** 模组描述（mod.json 的 `description`，英文原文）。领域信息页等非 wiki 场景曾用此字段 */
  desc?: string;
  /** `mod.json` 多语言块（scan 透传，与领域已实装同口径）：名称/描述按界面语言选词，
   *  领域「可引用」页的行名称与抽屉简介依赖它做本地化 */
  i18n?: Record<string, { name?: string; description?: string } | null>;
  /** `mod.json` 的 `version` / `author` 原文（scan 透传；缓存未构建时的回退值） */
  version?: string | null;
  author?: string | null;
  /** 模组元数据（**缓存投影 cache.json 的 modMeta**）：多语言名/描述 + 作者/版本/类型/依赖。
   *  缓存未构建时缺失——此时只能回退 `name` / `desc`（英文原文）。 */
  meta?: {
    modId?: string;
    name?: Record<string, string>;
    description?: Record<string, string>;
    author?: string;
    version?: string;
    modType?: string;
    depends?: unknown[];
    conflicts?: unknown[];
    compatibility?: Record<string, unknown>;
  };
  /** 来源目录（§9.6.1）："user"=玩家安装（全局用户目录）；"game"=游戏自带（游戏目录 Mods，永远可见） */
  source?: "user" | "game";
  /** 模组类型（modType，由后端 classify_mod 识别），供领域模组 tab 复用 Mods 页面的类型筛选 */
  type?: string;
  /** `mod.json` 的 `modType` **原文**（未推断；用于翻译模组语言判定等需与引擎同口径的场景） */
  rawType?: string | null;
  /** 引擎兼容区间（`mod.json` 的 `compatibility`），透传给领域模组列表做兼容提示 */
  compatibility?: { min?: string | null; max?: string | null } | null;
  /** 翻译类模组的基准语言（`mod.json` 的 `language`；官方判定缺失时视为 english） */
  language?: string | null;
  /** 顶层父 mod id（子模组指向父；顶层为自身），供复用 Mods 页面的父子树布局 */
  parentId?: string;
  /** 是否为子模组 */
  isSubmod?: boolean;
  /** 子模组默认是否保持禁用（VCMI mod.json 的 keepDisabled：true=安装后不自动启用） */
  keepDisabled?: boolean;
  /** 城镇阵营列表（真城镇才有），供类型筛选/缩略图 */
  factions?: string[];
  /** 必需依赖（VCMI mod.json 的 `depends`）：禁用本模组前需校验仍有启用模组依赖它（§3.3 依赖守卫） */
  depends?: string[];
  /** 可选依赖（VCMI mod.json 的 `softDepends`）：缺失不影响加载，仅增强 */
  softDepends?: string[];
  /** 互斥模组（VCMI mod.json 的 `conflicts`）：同时启用即冲突 */
  conflicts?: string[];
}
export async function listInstalledMods(): Promise<InstalledMod[]> {
  if (!isTauri()) return [];
  try {
    const r = await invoke<{ mods: InstalledMod[] }>("list_installed_mods");
    return r.mods || [];
  } catch {
    return [];
  }
}

/** 导出领域快照（§7.1）：打包为 .vcmirealm（ZIP）。dest 为目标文件绝对路径。 */
export interface ExportRealmResult {
  ok: boolean;
  /** 用户中途取消（半成品已删除，`path` 仅供展示） */
  cancelled?: boolean;
  /** 导出的 `.vcmirealm` 路径 */
  path: string;
  /** 同步导出的领域索引片段路径（`<stem>.index.json`，可并入仓库 index.json 用于分发） */
  indexPath?: string;
  /** 直写本地仓库的结果（§7.3）；未开该模式为 null */
  repo?: RealmPublishResult | null;
  /** 发布自检告警（code 供 i18n 映射，arg 是已格式化的补充信息） */
  warnings?: RealmPublishWarning[];
  fileCount: number;
  /** 包内条目校验和（`realm.json.checksum`，导入侧校验用） */
  checksum: string;
  /** **整个包文件的 SHA-256**（仓库索引 `sha256` 字段用，与下载校验同源） */
  sha256?: string;
  /** 产出的 `.vcmirealm` 字节数 */
  size?: number;
  /** 打包前的源内容总字节数（进度分母） */
  bytesTotal?: number;
  /** 是否附带了模组巡礼素材缓存 */
  showcaseCache?: boolean;
  /** 分片片数（0 = 未分片）。仅发布分支可能产生分片（分享分支恒为 0） */
  partCount?: number;
}

/** 发布自检告警（与后端 `realms::transfer::warn` 同构）。 */
export interface RealmPublishWarning {
  /** `over-download-max` | `over-git-max` | `bundled` | `bad-id` | `no-url-base` | `version-unchanged` | `pkg-unchanged` | `copied-into-repo` */
  code: string;
  /** 已格式化的补充信息（体积 MB / 模组清单 / 落地路径等） */
  arg?: string;
}

/** 下载地址前缀的来源（与后端 `realms::transfer::URL_SOURCE_*` 同值）。 */
export type RealmUrlSource = "manual" | "declared" | "git" | "entries" | "none";

/** 直写本地领域仓库工作树的产物摘要（§7.3）。 */
export interface RealmPublishResult {
  /** 仓库根（含 index.json 的那级） */
  repoDir: string;
  /** 包在仓库内的落地路径 `realms/<id>/<file>` */
  pkgPath: string;
  /** 封面落地路径；领域未配封面时为 null */
  coverPath?: string | null;
  /** 被合并写回的 index.json */
  indexPath: string;
  /** 本次对仓库条目的动作：新增 / 字段有变化 / 完全重复（条目已存在且无字段变化） */
  entryState: "created" | "updated" | "unchanged";
  /** 值发生变化的条目字段名（供结果态摘要展示） */
  changedFields: string[];
  /** 本次 download 的前缀来源 */
  urlSource: RealmUrlSource;
  /** 本次是否把推导结果回写成了仓库的 `urlBase`（自举） */
  urlBootstrapped?: boolean;
  /** 分片片数（0 = 未分片，包未超过单片上限） */
  partCount?: number;
  /** 分片产物清单（仅分片时非空；路径由后端给出，前端不自行拼名） */
  parts?: RealmPartInfo[];
}

/** 分片产物（`docs/features/realms-sharded-package.zh-CN.md` §5）。 */
export interface RealmPartInfo {
  /** 分片在仓库工作树内的绝对路径 */
  path: string;
  size: number;
  sha256: string;
}

/**
 * 「发布到领域仓库」的落地预览（§7.3）。
 *
 * 弹窗在用户点「导出」**之前**就要把会动哪三个文件、下载地址是什么显示出来，
 * 让用户确认而不是猜——这是取消「下载 URL 前缀」手填框的前提。
 * 推导与实际发布共用后端的 `plan_publish`，两边说法必然一致。
 */
export interface RealmPublishPreview {
  repoDir: string;
  /** 仓库是否已有 index.json */
  indexExists: boolean;
  /** 已有但解析失败：发布会中止（绝不覆盖坏数据），弹窗需提前标红 */
  indexParseOk: boolean;
  /** 包的落地路径 `realms/<id>/<id>.vcmirealm` */
  pkgPath: string;
  /** 封面落地文件名；领域未配封面（或封面实体缺失）为 null */
  coverName?: string | null;
  /** 推导出的 URL 前缀（带末尾 `/`）；推不出为 null */
  urlBase?: string | null;
  urlSource: RealmUrlSource;
  /** 最终写进索引的 download 地址；无地址为 null */
  downloadUrl?: string | null;
  /** 仓库里是否已有该领域条目（预告 created 还是 updated） */
  entryExists: boolean;
  /** 仓库既有条目版本；无条目 / 手工条目缺 version 时为 null */
  prevVersion?: string | null;
  /** 本次将发布的版本（取 realm.json，与索引条目同口径） */
  nextVersion: string;
  /** 版本关系：none = 无基准；down = 降级（发布将被阻断）；up = 升版；same = 同版 */
  versionRel: "none" | "down" | "up" | "same";
  /**
   * 分片预估片数：0 = 预计不分包（包未超过单片上限）。
   * 由后端按待打包内容字节数估算（不读文件内容），**实际片数以打包后为准**。
   */
  estimatedPartCount?: number;
  /** 单片上限字节数（提示语用，避免前端写死 45 MiB） */
  partMaxBytes?: number;
}

/** 导出进度（事件 `realm-export-progress`）。 */
export interface RealmExportProgress {
  realmId: string;
  /** prepare = 算素材清单；collect = 扫盘；pack = 写 zip；index = 算索引；publish = 写仓库 */
  phase: "prepare" | "collect" | "pack" | "index" | "publish";
  done: number;
  total: number;
  bytesDone: number;
  bytesTotal: number;
  /** 当前正在打包的条目相对路径 */
  current?: string;
}

/**
 * 导出领域快照（§7.1）。
 *
 * `opts`（S2）：
 * - `includeShowcaseCache`（默认 false）：把「阵营巡礼」所需素材缓存打进包
 *   （`showcase-cache/<modId>/{cache.json,images/}`），接收方导入后**无需重新采集**；
 *   体积随之增大。**原版伪模组（core / core.<faction>）不随包分发**（由接收方本机生成）。
 * - `repoDir`：给出即走「发布到领域仓库」分支（§7.3）——包与封面落 `realms/<id>/`、
 *   直接合并仓库 `index.json`，此时 `includeIndex` 恒为真（发布必然写索引）。
 * - `includeIndex`：仅分享分支有意义——true 时另写 `<stem>.index.json` 片段，
 *   供把包传到自己服务器后手工并入远程仓库。
 * - `urlBase`：仓库 URL 前缀的**人工覆盖值**（只存本机，不写回仓库）；留空由后端按
 *   「仓库声明 > git remote+分支 > 既有条目」推导。
 *
 * 后端已移出主线程执行（重 IO），进度经 `onRealmExportProgress` 推送，可用
 * `cancelRealmExport` 中途取消。
 */
export async function exportRealm(
  id: string,
  dest: string,
  opts?: {
    includeShowcaseCache?: boolean;
    includeIndex?: boolean;
    /** 要打包的缓存 mod（`realmAssetTargets` 的产物，见 `lib/modcache/realmAssets.ts`） */
    cacheMods?: string[];
    /** 本地领域仓库工作树根：给出即走「直写仓库」发布流程 */
    repoDir?: string;
    /** download / cover 的 URL 前缀；留空由后端从既有条目推导 */
    urlBase?: string;
  },
): Promise<ExportRealmResult> {
  if (!isTauri()) throw new Error(feErr("frontend_desktop_only", "导出领域"));
  return await invoke<ExportRealmResult>("export_realm", {
    realmId: id,
    dest,
    includeShowcaseCache: opts?.includeShowcaseCache ?? false,
    includeIndex: opts?.includeIndex ?? true,
    cacheMods: opts?.cacheMods ?? null,
    repoDir: opts?.repoDir?.trim() ? opts.repoDir.trim() : null,
    urlBase: opts?.urlBase?.trim() ? opts.urlBase.trim() : null,
  });
}

/** 订阅领域导出进度（事件 `realm-export-progress`），返回取消订阅函数。 */
export async function onRealmExportProgress(
  cb: (p: RealmExportProgress) => void,
): Promise<() => void> {
  if (!isTauri()) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  return await listen<RealmExportProgress>("realm-export-progress", (e) => cb(e.payload));
}

/** 请求取消正在进行的领域导出（中止点在打包循环，随后 `exportRealm` 返回 `cancelled: true`）。 */
export async function cancelRealmExport(id: string): Promise<void> {
  if (!isTauri()) return;
  await invoke("cancel_realm_export", { realmId: id });
}

/**
 * 记住「导出领域」的上次仓库目录、前缀覆盖与分支选择（传空 = 清空）。
 * 走 `launcher.json` 而非 localStorage：便携包要求偏好随目录走。
 */
export async function setRealmPublishTarget(
  repoDir: string | null,
  urlBase: string | null,
  mode: "share" | "publish" | null,
): Promise<void> {
  if (!isTauri()) return;
  await invoke("set_realm_publish_target", { repoDir, urlBase, mode });
}

/**
 * 预览「发布到领域仓库」会写什么（只读本地小文件，无网络无写入）。
 * 目录为空返回 null；目录不存在 / 不是领域仓库时后端报错，由调用方就地显示。
 */
export async function previewRealmPublish(
  repoDir: string,
  realmId: string,
  urlBase?: string | null,
): Promise<RealmPublishPreview | null> {
  if (!isTauri()) return null;
  const dir = repoDir.trim();
  if (!dir) return null;
  return await invoke<RealmPublishPreview>("preview_realm_publish", {
    repoDir: dir,
    realmId,
    urlBase: urlBase?.trim() ? urlBase.trim() : null,
  });
}

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

/** 领域专属地图清单项（B7）。 */
export interface RealmMapEntry {
  file: string;
  size: number;
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

/* ----------------------------- 远程领域仓库 ----------------------------- */

/** 远程领域索引条目（后端已补齐 `id` / `tags` / `state` / `installedVersion`）。 */
export interface RemoteRealm {
  id: string;
  /** 领域版本 */
  version?: string;
  author?: string;
  /** 领域名称（默认语言字符串，即 `defaultLang` 对应语言）。其他语言见 `i18n.<lang>.name` */
  name?: string;
  /** 领域描述（默认语言字符串）。其他语言见 `i18n.<lang>.description` */
  description?: string;
  /** 领域标签（默认语言）。其他语言见 `i18n.<lang>.tags` */
  tags?: string[];
  /** 顶层 `name` / `description` / `tags` 所属语言（VCMI 语言标识），缺省 chinese */
  defaultLang?: string;
  /** 逐语言覆盖块：语言 id -> 覆盖 `name` / `description` / `tags`。
   *  匹配界面语言时整体覆盖对应字段；无匹配按回退链兜底到顶层值。 */
  i18n?: Record<string, { name?: string; description?: string; tags?: string[] }>;
  /** 封面图地址（由**后端**下载后本地缓存，前端不直连，规避图床防盗链） */
  cover?: string;
  download?: string;
  sha256?: string;
  /** 包体积（字节） */
  size?: number;
  /** 依赖的外部模组（不随领域分发，需玩家本机已安装） */
  requiresMods?: string[];
  /** 该 realm 所属仓库源地址（聚合多源时由后端打上，用于校验/下载定位） */
  repositoryUrl?: string;
  gameEdition?: string;
  /** `available` 未安装 / `installed` 已安装且最新 / `updatable` 有更新 */
  state: "available" | "installed" | "updatable";
  installedVersion?: string | null;
}

export interface RealmRepositoryIndex {
  realms: RemoteRealm[];
  /** 各仓库源的拉取状态（成功/失败、领域数量、缓存时间、错误原因） */
  sources?: RealmRepositorySource[];
  /** 聚合层面的错误：仅当所有源都失败且无任何缓存时非空 */
  error?: string | null;
}

export interface RealmRepositorySource {
  url: string;
  /** 本次拉取是否成功（成功或有可用缓存均为 true） */
  ok: boolean;
  error?: string | null;
  fetchedAt?: number | null;
  realmCount: number;
}

/** 当前仓库源列表（不触发网络请求）。 */
export async function getRealmRepositories(): Promise<{ urls: string[] }> {
  if (!isTauri()) return { urls: [] };
  return await invoke<{ urls: string[] }>("get_realm_repositories");
}

/** 新增一个仓库源地址（去重、校验 https + .json）。 */
export async function addRealmRepository(url: string): Promise<{ urls: string[] }> {
  if (!isTauri()) throw new Error(feErr("frontend_desktop_only", "配置领域仓库源"));
  return await invoke<{ urls: string[] }>("add_realm_repository", { url });
}

/** 移除一个仓库源地址。 */
export async function removeRealmRepository(url: string): Promise<{ urls: string[] }> {
  if (!isTauri()) throw new Error(feErr("frontend_desktop_only", "配置领域仓库源"));
  return await invoke<{ urls: string[] }>("remove_realm_repository", { url });
}

/** 测试索引地址是否可用（不写入配置）。 */
export async function testRealmRepository(
  url: string,
): Promise<{ ok: boolean; url: string; realmCount: number; elapsedMs: number }> {
  if (!isTauri()) throw new Error(feErr("frontend_desktop_only", "测试领域仓库源"));
  return await invoke("test_realm_repository", { url });
}

/** 拉取远程领域列表。`force` 为 true 时忽略缓存 TTL 强制刷新。 */
export async function fetchRealmIndex(force = false): Promise<RealmRepositoryIndex> {
  if (!isTauri()) {
    return { realms: [], sources: [], error: feErr("frontend_mock_unsupported", "远程领域") };
  }
  return await invoke<RealmRepositoryIndex>("fetch_realm_index", { force });
}

/** 下载并安装远程领域（后端**强制**校验 sha256 后才导入）。conflict 语义同本地导入。 */
export async function installRemoteRealm(
  realmId: string,
  conflict: "error" | "overwrite" | "rename" = "error",
): Promise<ImportRealmResult> {
  if (!isTauri()) throw new Error(feErr("frontend_desktop_only", "下载远程领域"));
  return await invoke<ImportRealmResult>("install_remote_realm", { realmId, conflict });
}

/** 取消正在进行的领域下载。 */
export async function cancelRealmDownload(realmId: string): Promise<void> {
  if (!isTauri()) return;
  await invoke("cancel_realm_download", { realmId });
}

/** 下载并缓存远程领域封面，返回 `领域 id → 本地图片绝对路径`。 */
export async function ensureRealmCovers(): Promise<Record<string, string>> {
  if (!isTauri()) return {};
  try {
    return await invoke<Record<string, string>>("ensure_realm_covers");
  } catch {
    return {};
  }
}

/** 封面本地绝对路径 → `<img>` 可用 URL（§5.4：本地资源必须经 convertFileSrc）。 */
export function coverUrl(absPath: string | undefined): string {
  return absPath ? convertFileSrc(absPath) : "";
}

export interface RealmDownloadProgress {
  realmId: string;
  received: number;
  total: number;
  done?: boolean;
}

/** 订阅领域下载进度，返回取消订阅函数。 */
export async function onRealmDownloadProgress(
  cb: (p: RealmDownloadProgress) => void,
): Promise<() => void> {
  if (!isTauri()) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  return await listen<RealmDownloadProgress>("realm-download-progress", (e) => cb(e.payload));
}

/** 取消下载的哨兵错误串（后端约定）：前端识别后不弹错误提示。 */
export const REALM_DOWNLOAD_CANCELLED = "REALM_DOWNLOAD_CANCELLED";

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

/* ----------------------------- 运行信息弹窗 ----------------------------- */

export interface Resolution {
  width: number;
  height: number;
  primary?: boolean;
}

export interface RuntimeInfo {
  vcmiVersion: string;
  modsInstalled: number;
  modsEnabled: number;
  recommendedResolution: { width: number; height: number };
  gameDataLanguage: string;
  lastMap: string;
  lastSave: string;
  lastSettingsTab: string;
  setupCompleted: boolean;
  mainWindow: { geometry?: { x?: number; y?: number; width?: number; height?: number } } | null;
}

/** 常用分辨率候选列表（用于「分辨率」ButtonList） */
export async function listResolutions(): Promise<Resolution[]> {
  if (!isTauri()) {
    return [
      { width: 1280, height: 720 },
      { width: 1920, height: 1080 },
      { width: 2560, height: 1440 },
      { width: 3840, height: 2160 },
    ];
  }
  try {
    return await invoke<Resolution[]>("list_resolutions");
  } catch {
    return [];
  }
}

/**
 * 聚合运行态只读信息（版本 / 已安装 / 已启用 / 推荐分辨率）。
 * `configDir` 可选：传入领域配置目录时，**跟随作用域**的字段（最近地图 / 最近存档 /
 * 上次设置标签 / 游戏数据语言 / setupCompleted / 主窗口几何 / 已启用模组数）读该目录；
 * 不传 = 全局用户配置目录（见 settings-panel-design §六）。
 */
export async function getRuntimeInfo(configDir?: string): Promise<RuntimeInfo | null> {
  // 浏览器自测模式：模拟中文内容语言（与实际使用环境一致），其余运行态信息不可用
  if (!isTauri()) {
    return { gameDataLanguage: "chinese" } as RuntimeInfo;
  }
  try {
    return await invoke<RuntimeInfo>("get_runtime_info", { configDir: configDir ?? null });
  } catch {
    return null;
  }
}

/* ----------------------------- 本地 zip 安装模组 ----------------------------- */

/** zip 包探查结果（安装前确认用；后端 `preview_mod_zip`） */
export interface ModZipPreview {
  /** 归一后的模组 id（= 目标目录名；已去掉尾随版本号） */
  modId: string;
  name: string;
  version: string;
  modType: string;
  /** 将安装的文件数（不含目录条目） */
  entryCount: number;
  /** 安装目标目录（绝对路径，位于用户 Mods 目录下） */
  target: string;
  /** 目标目录是否已存在（true → 需确认覆盖） */
  exists: boolean;
}

/** 探查 zip 包（**不写盘**）：识别 mod 根、读取元数据、推导 id 与目标目录。 */
export async function previewModZip(zipPath: string): Promise<ModZipPreview> {
  if (!isTauri()) throw new Error(feErr("frontend_not_tauri"));
  return await invoke<ModZipPreview>("preview_mod_zip", { zipPath });
}

export interface ModInstallResult {
  modId: string;
  dest: string;
  installedFiles: number;
  /** 是否替换了已存在的同名模组 */
  replaced: boolean;
}

/**
 * 安装本地 zip 到用户 Mods 目录（`<userModsDir>/<modId>`）。
 *
 * `overwrite=false` 且同名已存在 → 抛错（中文提示），由调用方确认后改传 `true`。
 * 后端保证：失败不留半截目录、不破坏既有安装（先解压到临时目录再就位）。
 */
export async function installLocalMod(
  zipPath: string,
  overwrite: boolean,
): Promise<ModInstallResult> {
  if (!isTauri()) throw new Error(feErr("frontend_not_tauri"));
  return await invoke<ModInstallResult>("install_local_mod", { zipPath, overwrite });
}

/* ----------------------------- 导入游戏文件（底栏入口） ----------------------------- */

/** 地图元数据（`infra::h3m::MapMeta`，camelCase；仅声明前端展示用到的字段） */
export interface GameImportMapMeta {
  kind: string;
  lang: string;
  name: string;
  description: string;
  version: string;
  players: number;
  /** S / M / L / XL / H / XH / G */
  sizeClass: string;
  /** 战役场景数（仅 .h3c） */
  scenarioCount: number;
}

/** zip 包判定为模组时的探查结果 */
export interface GameImportModPreview {
  modId: string;
  name: string;
  version: string;
  entryCount: number;
  target: string;
  exists: boolean;
}

/** zip 识别结论（`overridden` = 用户改判生效） */
export interface GameImportZipPreview {
  detected: "mod" | "maps" | "saves";
  overridden: boolean;
  /** false + detected=mod → 内容未识别，按模组尝试 */
  hasModJson: boolean;
  name?: string;
  version?: string;
  mapCount: number;
  saveCount: number;
}

/** HD 目录预检：校验 / 语言 / pak 命中数 */
export interface GameImportHdPreview {
  valid: boolean;
  /** VCMI 语言 identifier（english…） */
  language?: string;
  /** 短码（EN…） */
  languageCode?: string;
  paks: { found: number; total: number };
  missing: string[];
}

/** 普通目录递归预检：各类型计数 + 忽略数 + 冲突数（一项多文件，与 zip 同口径） */
export interface GameImportDirPreview {
  mapCount: number;
  campaignCount: number;
  saveCount: number;
  configCount: number;
  zipCount: number;
  /** 目录内不支持（无关文件 / `.exe` / 嵌套 HD 标记）的项数 */
  ignored: number;
  /** 平铺文件目标已存在的项数（zip 子项不计） */
  conflictCount: number;
}

/** 预检条目（后端 `vcmi::game_import::PreviewItem`） */
export interface GameImportPreviewItem {
  path: string;
  kind: "map" | "campaign" | "zip" | "save" | "config" | "chronicles" | "hd" | "dir" | "unsupported";
  ext: string;
  size: number;
  meta?: GameImportMapMeta;
  mod?: GameImportModPreview;
  zip?: GameImportZipPreview;
  hd?: GameImportHdPreview;
  /** 仅普通目录输入：递归收集结果统计 */
  dir?: GameImportDirPreview;
  /** 仅 .exe：Inno Setup 特征串识别结论 */
  inno?: boolean;
  target: string;
  conflict: boolean;
  ok: boolean;
  reason?: string;
}

export interface GameImportPreviewReport {
  items: GameImportPreviewItem[];
  summary: {
    total: number;
    byKind: Record<string, number>;
    conflicts: number;
    unsupported: number;
  };
}

/** 执行结果条目 */
export interface GameImportExecItem {
  path: string;
  kind: string;
  dest?: string;
  status: "added" | "replaced" | "skipped" | "failed" | "unsupported";
  reason?: string;
}

/** 本次装成的模组（前端据此发 `vcmi-fv:mods-installed` 事件） */
export interface GameImportModInstalled {
  modId: string;
  name: string;
  version: string;
}

export interface GameImportExecReport {
  total: number;
  added: number;
  skipped: number;
  failed: number;
  replaced: number;
  unsupported: number;
  items: GameImportExecItem[];
  modsInstalled: GameImportModInstalled[];
}

export type GameImportConflict = "skip" | "overwrite" | "rename";
export type GameImportOverride = "mod" | "maps" | "saves";

interface GameImportOpts {
  paths: string[];
  conflict: GameImportConflict;
  dryRun: boolean;
  /** zip 逐项改判（`path → mod|maps|saves`） */
  overrides?: Record<string, GameImportOverride>;
  /**
   * 可选覆盖目录（首启向导用：配置尚未保存，导入必须按用户选的目录落盘；
   * 历代记还要从游戏目录取内置核心模组的 `portraitsChronicles.json`）。
   * 不传 = 沿用已保存配置。
   */
  gameDir?: string;
  userDataDir?: string;
}

/** 预检（不写盘）：分类 / 识别 / 冲突检查。 */
export function importGameFiles(opts: GameImportOpts & { dryRun: true }): Promise<GameImportPreviewReport>;
/** 执行导入（按 `conflict` 落盘；`overrides` 对 zip 生效）。 */
export function importGameFiles(opts: GameImportOpts & { dryRun: false }): Promise<GameImportExecReport>;
export async function importGameFiles(
  opts: GameImportOpts,
): Promise<GameImportPreviewReport | GameImportExecReport> {
  if (!isTauri()) throw new Error(feErr("frontend_not_tauri"));
  return await invoke("import_game_files", {
    paths: opts.paths,
    conflict: opts.conflict,
    dryRun: opts.dryRun,
    overrides: opts.overrides ?? null,
    gameDir: opts.gameDir,
    userDataDir: opts.userDataDir,
  });
}

/** 跨 mod 定位并返回 `.def` 动画文件的原始字节（Uint8Array），供前端直接解码绘制。 */
export async function readDefBytes(
  modId: string,
  resource: string,
): Promise<Uint8Array | null> {
  if (!isTauri()) return null;
  try {
    return await invoke<Uint8Array>("read_def_bytes", { modId, resource });
  } catch {
    return null;
  }
}

/** 高清动画的**单帧**：位置是画布像素坐标，字节是该帧的 webp。 */
export interface HdDefFrame {
  x: number;
  y: number;
  w: number;
  h: number;
  /** webp 字节（后端侧已把 HD 的独立阴影切片合进这一张） */
  data: number[] | Uint8Array;
}

/** 高清动画分组（后端 `read_def_group_hd`；`null` = 该组不可高清化，需回退 1x）。 */
export interface HdDefGroup {
  /** 画布尺寸 = 显示窗口（可能已自适应外扩）× 档位 */
  w: number;
  h: number;
  /**
   * 显示窗口左上角在 **DEF 逻辑画布**（原版 450×400）中的坐标。
   * 背景图（100×130）固定贴在基准窗口 `(150|170, 155)`，前端靠它把两层对齐。
   */
  wx?: number;
  wy?: number;
  /** 幕布窗口（1x 画布坐标）：两种布局之一，100×130 或 200×130 */
  winX?: number;
  winY?: number;
  winW?: number;
  winH?: number;
  /** 适配参考尺寸（`win ∪ 待机/移动/悬停/防御` 的包围盒） */
  fitW?: number;
  fitH?: number;
  /** 档位（2 / 3） */
  scale: number;
  frames: HdDefFrame[];
}

/**
 * 取 `.def` 某分组的动画帧（**高清优先、1x 兜底**，VCMI 显示窗口已由后端合成）。
 *
 * - `doubleWide`：双格生物（窗口左上角 x=170，否则 150）；
 * - `need`：该区域实际要占的**物理像素**（显示框 CSS × 屏幕物理倍率），决定取哪一档。
 *
 * 返回 `null` = 该组没有可播放的帧（前端回退 TS 解码 / 静态图）。
 */
export async function readDefGroupHd(
  modId: string,
  resource: string,
  group: number,
  doubleWide: boolean,
  need: number,
  kind?: string | null,
): Promise<HdDefGroup | null> {
  if (!isTauri()) return null;
  try {
    // 后端以**二进制**回传（`[4B LE 元数据长度][元数据 JSON][帧体...]`）：
    // 帧字节不再走 JSON 数字数组，省掉 3~4 倍载荷膨胀与逐元素转 `Uint8Array` 的开销。
    const buf = await invoke<ArrayBuffer>("read_def_group_hd", {
      modId,
      resource,
      group,
      doubleWide,
      need,
      kind: kind ?? null,
    });
    return unpackHdGroup(buf);
  } catch {
    return null;
  }
}

/**
 * 解开后端二进制分组载荷：`[4B LE 元数据长度][元数据 JSON][帧体...]`。
 *
 * 元数据 `frames[i]` 只含 `{x,y,w,h,len}`，帧字节紧跟在元数据之后 —— 这里只做
 * `Uint8Array` 视图切片（零拷贝），不再逐字节拼数组。`frames[i]` 的字节交给
 * `Blob`/`createImageBitmap` 时会被固化，视图生命周期没有问题。
 */
function unpackHdGroup(buf: ArrayBuffer | Uint8Array): HdDefGroup | null {
  // `invoke` 对二进制响应通常给 `ArrayBuffer`；某些运行时给 `Uint8Array`，两者都接受。
  const ab: ArrayBuffer =
    buf instanceof Uint8Array
      ? (buf.buffer.slice(buf.byteOffset, buf.byteOffset + buf.byteLength) as ArrayBuffer)
      : buf;
  if (!ab || ab.byteLength < 4) return null;
  const dv = new DataView(ab);
  const metaLen = dv.getUint32(0, true);
  if (metaLen === 0 || 4 + metaLen > ab.byteLength) return null;
  let meta: HdDefGroup & { frames?: { x: number; y: number; w: number; h: number; len: number }[] };
  try {
    meta = JSON.parse(new TextDecoder().decode(new Uint8Array(ab, 4, metaLen)));
  } catch {
    return null;
  }
  const list = meta.frames ?? [];
  if (list.length === 0) return null;
  let off = 4 + metaLen;
  const frames: HdDefFrame[] = [];
  for (const f of list) {
    const len = f.len | 0;
    if (len <= 0 || off + len > ab.byteLength) return null;
    frames.push({ x: f.x, y: f.y, w: f.w, h: f.h, data: new Uint8Array(ab, off, len) });
    off += len;
  }
  return { ...meta, frames };
}

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

// ── 外挂运行时组件（runtime）──────────────────────────────────────────────
//
// 「外挂组件」= 不随每次版本更新变化的外部依赖（首个 = AI 超分的 ONNX Runtime + 模型）。
// 组件 id 与文件清单由后端 `runtime::COMPONENTS` 定义，前端只按 id 操作。
// 分发形态：便携/安装包内置（文件已就位）或远程下载（launcher.json 配了 package）。

/** 组件内单个文件的状态 */
export interface RuntimeFileStatus {
  name: string;
  present: boolean;
  sha_ok: boolean;
}

/** 组件状态（`runtime_status` 返回） */
export interface RuntimeStatus {
  id: string;
  /** 组件目录（绝对路径，`/` 分隔）；`null` = 无法确定 */
  dir: string | null;
  /** 文件齐备且 sha256 全部匹配 */
  installed: boolean;
  /** 已安装组件的版本；未安装时为 null */
  version: string | null;
  files: RuntimeFileStatus[];
  /** 是否配置了远程包 —— 决定能否显示「安装」 */
  has_package: boolean;
}

/** 安装进度（事件 `runtime-install-progress`） */
export interface RuntimeProgress {
  id: string;
  received: number;
  total: number;
  done?: boolean;
}

/** AI 组件 id（与后端 `runtime::AI` 对齐） */
export const AI_RUNTIME_ID = "ai";

/** 查询组件状态；失败返回 null（组件未配置 / 未知 id 都属正常） */
export async function fetchRuntimeStatus(id: string): Promise<RuntimeStatus | null> {
  if (!isTauri()) return null;
  try {
    return (await invoke("runtime_status", { id })) as RuntimeStatus;
  } catch (e) {
    console.error("查询运行时组件失败", e);
    return null;
  }
}

/** 安装组件（**阻塞**至完成），返回安装后的最新状态 */
export async function installRuntime(id: string): Promise<RuntimeStatus> {
  const r = (await invoke("runtime_install", { id })) as { ok: boolean; status: RuntimeStatus };
  return r.status;
}

/** 取消安装（置哨兵，下载循环下一块退出） */
export async function cancelRuntime(id: string): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("runtime_cancel", { id });
  } catch (e) {
    console.error("取消安装失败", e);
  }
}

/** 卸载组件（只删程序文件，不动用户数据），返回最新状态 */
export async function uninstallRuntime(id: string): Promise<RuntimeStatus> {
  const r = (await invoke("runtime_uninstall", { id })) as { ok: boolean; status: RuntimeStatus };
  return r.status;
}

/** 订阅组件安装进度，返回取消订阅函数 */
export async function onRuntimeProgress(cb: (p: RuntimeProgress) => void): Promise<() => void> {
  if (!isTauri()) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  return await listen<RuntimeProgress>("runtime-install-progress", (e) => cb(e.payload));
}
