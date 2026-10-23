/**
 * 启动器配置（`launcher.json`）的类型、读取、写入与 VCMI 目录探测。
 *
 * 拆分自原 `@/lib/tauri.ts` 单体；对外仍由该 barrel 统一导出，调用方无需改动。
 */
import { invoke } from "@tauri-apps/api/core";
import { feErr } from "@/lib/backendError";
import { isTauri } from "./core";

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


/** 非 Tauri 环境下 fetch mock 缓存 JSON，统一转 WikiReadResult 形状 */

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
