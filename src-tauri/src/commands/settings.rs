//! 启动器自身配置：`read_launcher_config` / `write_launcher_config` / `set_ui_lang` /
//! `set_launcher_pref`。
//!
//! ⚠️ **三层格式各自冻结**（见 `docs/architecture/backend.zh-CN.md` §7.1）：
//! 磁盘 `launcher.json` 为**分组的 snake_case**（`LauncherConfig`：paths / ui / hd / mods /
//! realms / update / runtime 七个组），本层出参为手写 camelCase（**扁平**，与磁盘分组解耦），
//! 入参亦为 camelCase。任一层改动都会导致用户配置丢失。
//! 前端只看得到本层的 camelCase，故磁盘结构可以（也已经）独立演进。

use crate::infra::berror::be_err;
use serde_json::{json, Value};
use std::path::PathBuf;
use tauri::State;

use crate::state::AppPaths;
use crate::vcmi::config::{
    AiCfg, ArchivesCfg, HdCfg, LauncherConfig, ModsCfg, NetworkCfg, PathsCfg, RealmsCfg,
    RuntimeCfg, UiCfg, UpdateCfg,
};
use crate::vcmi::paths::VcmiPaths;

/// 读取 launcher.json 持久化配置
#[tauri::command]
pub async fn read_launcher_config() -> Result<Value, String> {
    // launcher.json 读取 + 结构解析属磁盘 IO；同步命令会占用主线程、冻结 WebView 消息循环。
    tauri::async_runtime::spawn_blocking(read_launcher_config_blocking)
        .await
        .map_err(|e| be_err("cmd_blocking_task_failed", &format!("{}", e)))?
}

/// `read_launcher_config` 的同步实现（在阻塞线程池执行）。
fn read_launcher_config_blocking() -> Result<Value, String> {
    match VcmiPaths::read_launcher_config() {
        Some(cfg) => {
            let launcher_path = VcmiPaths::launcher_config_path()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_default();
            Ok(json!({
                "configFile": launcher_path,
                "gameDir": cfg.paths.game_dir,
                "userDataDir": cfg.paths.user_data_dir,
                "userConfigDir": cfg.paths.user_config_dir,
                "userModsDir": cfg.paths.user_mods_dir,
                "gameModsDir": cfg.paths.game_mods_dir,
                "windowSize": cfg.ui.window_size,
                "showAllSettings": cfg.ui.show_all_settings,
                "settingsLevel": cfg.ui.settings_level,
                "settingsShowDesc": cfg.ui.settings_show_desc,
                "realmsRepositoryUrls": cfg.realms.repository_urls,
                // 「自动检测更新」本地开关（默认开，仅领域「模组管理」页生效）
                "realmsModAutoCheck": cfg.realms.mod_auto_check,
                // 导出发布分支的上次目录 / URL 前缀覆盖 / 分支选择（供导出弹窗回填）
                "realmsPublishRepoDir": cfg.realms.publish_repo_dir,
                "realmsPublishUrlBase": cfg.realms.publish_url_base,
                "realmsPublishMode": cfg.realms.publish_mode,
                "builtinPresets": cfg.mods.builtin_presets,
                "requiredMods": cfg.mods.required,
                // 界面语言（便携包可预置，前端启动时据此覆盖 localStorage 探测值）
                "uiLang": cfg.ui.lang,
                // 日志等级（仅展示/调试用，前端不参与日志实现）
                "logLevel": cfg.runtime.log_level,
                // 自动更新（清单地址发行版可预置；开关/跳过版本由更新弹窗写入）
                "updateManifestUrl": cfg.update.manifest_url,
                "updateAutoCheck": cfg.update.auto_check,
                "updateSkipVersion": cfg.update.skip_version,
                // 网络出口：GitHub 代理地址（http(s)，空/null = 直连）
                "networkProxy": cfg.network.proxy,
                // 网络出口：GitHub 镜像规则表（L2，空数组 = 不用，默认不内置）
                "mirrorRules": cfg.network.mirror_rules,
                // 列表 hover 预览城镇背景（默认 false，见 UiCfg::town_preview）
                "townPreview": cfg.ui.town_preview,
                // AI 高清增强（默认 false）：用 Real-ESRGAN 超分插画类素材
                "aiHdEnhance": cfg.hd.ai.enhance,
                // AI 模型文件名（缺省 null = **AI 不生效**，无默认模型）；
                // 候选列表由组件目录扫描得出（命令 list_ai_models）
                "aiModel": cfg.hd.ai.model,
                // 高清素材生成总开关（默认 false）：开启后才生成 @Nx 变体
                "hdEnhance": cfg.hd.enhance,
                // HD 素材来源（hd-edition 的 .pak）：缺省/auto = 检测到即用，off = 不用
                "hdEditionUse": cfg.hd.edition_use,
                // HD 素材档位：缺省/auto = 取可用最高档（x3 优先），x2 = 只用 x2
                "hdEditionScale": cfg.hd.edition_scale,
                "valid": cfg.is_valid(),
            }))
        }
        None => Ok(Value::Null),
    }
}

/// 保存 launcher.json 并刷新运行时的路径状态。
/// 仅需要游戏目录与用户数据目录：config/mods 均按 VCMI 规则由用户目录推导。
#[tauri::command]
pub fn write_launcher_config(state: State<AppPaths>, data: Value) -> Result<(), String> {
    let game_dir = data
        .get("gameDir")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .ok_or_else(|| be_err("settings_missing_gamedir", ""))?;
    // 游戏目录必须包含客户端
    if !game_dir.join("VCMI_client.exe").is_file() {
        return Err(be_err("settings_gamedir_no_exe", ""));
    }
    let user_data_dir = data
        .get("userDataDir")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .ok_or_else(|| be_err("settings_missing_userdata", ""))?;
    // 允许用户数据目录尚不存在：首次进入后由启动器/VCMI 创建
    if user_data_dir.as_os_str().is_empty() {
        return Err(be_err("settings_missing_userdata", ""));
    }
    let user_config_dir = data
        .get("userConfigDir")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| user_data_dir.join("config"));
    let user_mods_dir = data
        .get("userModsDir")
        .and_then(|v| v.as_str())
        .map(PathBuf::from)
        .unwrap_or_else(|| user_data_dir.join("Mods"));
    let game_mods_dir = game_dir.join("Mods");

    // 保留既有偏好 / 预置值：仅当入参 `data` 未显式提供时才沿用磁盘上的旧值，
    // 避免"保存目录"这类操作把用户偏好或便携包预置的内容清掉。
    //
    // 两个来源都要看：
    // - `prev`（严格）：要求配置**完整有效**，能取到派生后的默认路径；
    // - `prev_any`（宽容）：即便既有 launcher.json 不完整（缺 game_dir/user_data_dir 被判为无效），
    //   也能把里面已 seeded 的语言 / 高清开关 / 组件地址取回来 —— 便携包正是靠这种
    //   "只有零散键"的预置文件下发的，**漏了它等于白预置**。
    let prev = VcmiPaths::read_launcher_config();
    let prev_any = VcmiPaths::read_launcher_config_lenient();
    let inherit = prev.as_ref().or(prev_any.as_ref());

    let cfg = LauncherConfig {
        paths: PathsCfg {
            game_dir: game_dir.clone(),
            user_data_dir,
            user_config_dir,
            user_mods_dir,
            game_mods_dir: Some(game_mods_dir),
            // 保留既有便携接管开关
            redirect_user_data: inherit.map(|p| p.paths.redirect_user_data).unwrap_or(false),
        },
        ui: UiCfg {
            // 界面语言：优先取入参（初始化向导保存时一并写入），否则保留既有配置
            lang: data
                .get("uiLang")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .or_else(|| inherit.and_then(|p| p.ui.lang.clone())),
            window_size: inherit.and_then(|p| p.ui.window_size.clone()),
            show_all_settings: inherit.map(|p| p.ui.show_all_settings).unwrap_or(false),
            settings_level: data
                .get("settingsLevel")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| inherit.map(|p| p.ui.settings_level.clone()))
                .unwrap_or_else(|| "common".to_string()),
            settings_show_desc: data
                .get("settingsShowDesc")
                .and_then(|v| v.as_bool())
                .or_else(|| inherit.map(|p| p.ui.settings_show_desc))
                .unwrap_or(false),
            town_preview: data
                .get("townPreview")
                .and_then(|v| v.as_bool())
                .or_else(|| inherit.map(|p| p.ui.town_preview))
                .unwrap_or(false),
        },
        hd: HdCfg {
            // 高清总开关 / AI 子链路：**只保留既有值**（常规写入走 `set_hd_enhance` /
            // `set_ai_hd_enhance` / `set_ai_model`，不经过这里），否则向导/设置页保存目录时
            // 会把开关清回默认 false
            enhance: inherit.map(|p| p.hd.enhance).unwrap_or(false),
            ai: AiCfg {
                enhance: inherit.map(|p| p.hd.ai.enhance).unwrap_or(false),
                // 模型与混合权重跟随原配置（缺省 = None：无默认模型 / 混合回落 0.75）
                model: inherit.and_then(|p| p.hd.ai.model.clone()),
                mix: inherit.and_then(|p| p.hd.ai.mix),
            },
            // HD 调优项跟随原配置（缺省时由读取侧回落到默认档：USM 0.8）
            usm: inherit.and_then(|p| p.hd.usm),
            edition_use: data
                .get("hdEditionUse")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| inherit.map(|p| p.hd.edition_use.clone()))
                .unwrap_or_else(|| HdCfg::default().edition_use),
            edition_scale: data
                .get("hdEditionScale")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| inherit.map(|p| p.hd.edition_scale.clone()))
                .unwrap_or_else(|| HdCfg::default().edition_scale),
        },
        mods: ModsCfg {
            // 保留既有内置预设与强制模组（避免写盘时清空发行版追加的配置）
            required: inherit.map(|p| p.mods.required.clone()).unwrap_or_default(),
            builtin_presets: inherit
                .map(|p| p.mods.builtin_presets.clone())
                .unwrap_or_default(),
        },
        realms: RealmsCfg {
            // 保存目录配置时保留当前激活领域（领域元数据本身在各领域目录的 realm.json 中）
            active: data
                .get("activeRealm")
                .and_then(|v| v.as_str().map(|s| s.to_string()))
                .or_else(|| inherit.and_then(|p| p.realms.active.clone())),
            // 领域仓库源：保留既有列表；完全去硬编码，缺失即空（由发行版 launcher.json 预置或用户手动添加）
            repository_urls: inherit
                .map(|p| p.realms.repository_urls.clone())
                .unwrap_or_default(),
            // 「自动检测更新」本地开关：保留既有值（修改走 set_realm_mod_auto_check）
            mod_auto_check: inherit.and_then(|p| p.realms.mod_auto_check),
            // 导出发布分支的上次目录 / 前缀覆盖 / 分支：保留既有值（修改走 set_realm_publish_target）
            publish_repo_dir: inherit.and_then(|p| p.realms.publish_repo_dir.clone()),
            publish_url_base: inherit.and_then(|p| p.realms.publish_url_base.clone()),
            publish_mode: inherit.and_then(|p| p.realms.publish_mode.clone()),
        },
        update: UpdateCfg {
            // 自动更新五项：仅保留既有值（发行版可预置 manifest_url）；
            // 偏好修改走 set_update_pref / set_update_manifest_url
            manifest_url: inherit.and_then(|p| p.update.manifest_url.clone()),
            auto_check: data
                .get("updateAutoCheck")
                .and_then(|v| v.as_bool())
                .or_else(|| inherit.map(|p| p.update.auto_check))
                .unwrap_or(true),
            skip_version: inherit.and_then(|p| p.update.skip_version.clone()),
            // 7 天节流记录（上次自动弹窗的版本与时间 + 当时的 exe 版本）：
            // 只保留既有值（写走 set_update_pref），否则保存设置会把节流记录抹掉
            prompt_version: inherit.and_then(|p| p.update.prompt_version.clone()),
            prompt_at: inherit.and_then(|p| p.update.prompt_at),
            prompt_exe_version: inherit.and_then(|p| p.update.prompt_exe_version.clone()),
        },
        runtime: RuntimeCfg {
            // 日志等级不在向导/设置页暴露，仅保留既有（发行版/开发者手工预置）
            log_level: inherit.and_then(|p| p.runtime.log_level.clone()),
            // 外挂运行时组件：**只保留既有值**（由发行版预置或手工编辑 launcher.json，
            // 不经过任何 UI 写入），否则向导 / 设置页保存目录时会把配置清掉
            components: inherit
                .map(|p| p.runtime.components.clone())
                .filter(|m| !m.is_empty())
                .unwrap_or_default(),
        },
        network: NetworkCfg {
            // 网络代理/镜像规则：只保留既有值（常规修改走 set_network_proxy / set_network_mirror_rules），避免保存目录时清掉
            proxy: inherit.and_then(|p| p.network.proxy.clone()),
            mirror_rules: inherit
                .map(|p| p.network.mirror_rules.clone())
                .unwrap_or_default(),
        },
        archives: ArchivesCfg {
            // 压缩包解密密码表：仅保留既有值（由发行版预置，不暴露任何 UI 写入），
            // 否则保存目录时会把发行版预置的密码清掉
            zip_passwords: inherit
                .map(|p| p.archives.zip_passwords.clone())
                .unwrap_or_default(),
        },
    };
    if !cfg.is_valid() {
        return Err(be_err("settings_gamedir_invalid", ""));
    }
    VcmiPaths::write_launcher_config(&cfg)?;

    {
        let mut paths = state.0.lock().expect("paths lock poisoned");
        paths.update_from_config(&cfg);
        // 保存配置（初始化向导 / 目录设置）后立即按新的 game_dir / user_data_dir
        // 完成目录初始化，不等后续 detect 才补；失败非致命（目录可能只读）。
        if let Err(e) = VcmiPaths::ensure_user_data_dir(&paths, cfg.realms.active.as_deref()) {
            tracing::warn!(target: "paths", "保存配置后目录初始化失败（可忽略）：{e}");
        }
    }
    tracing::info!(target: "config", "已保存 launcher.json");
    Ok(())
}

/// 持久化启动器界面语言到 `launcher.json`。
///
/// 供底栏语言切换调用。初始化向导阶段配置尚不存在，语言随 `write_launcher_config`
/// 的 `uiLang` 参数一并写入，不走本命令。
#[tauri::command]
pub fn set_ui_lang(lang: String) -> Result<(), String> {
    let lang = lang.trim().to_string();
    if lang.is_empty() {
        return Err(be_err("settings_lang_empty", ""));
    }
    let mut cfg =
        VcmiPaths::read_launcher_config().ok_or_else(|| be_err("settings_not_init", ""))?;
    cfg.ui.lang = Some(lang.clone());
    VcmiPaths::write_launcher_config(&cfg)?;
    tracing::info!(target: "config", "界面语言已切换为 {lang}");
    Ok(())
}

/// 仅更新启动器偏好字段（如设置面板的"显示所有参数"开关），不校验游戏目录。
/// 持久化设置面板的视图偏好（可见级别 + 是否显示详细描述）到 launcher.json。
/// 读回现有 launcher.json 后合并再写回，避免破坏其它字段（gameDir 等）。
#[tauri::command]
pub fn set_launcher_pref(
    show_all_settings: bool,
    settings_level: String,
    settings_show_desc: bool,
) -> Result<(), String> {
    let mut cfg = match VcmiPaths::read_launcher_config() {
        Some(c) => c,
        None => {
            return Err(be_err("settings_not_init", ""));
        }
    };
    cfg.ui.show_all_settings = show_all_settings;
    cfg.ui.settings_level = settings_level;
    cfg.ui.settings_show_desc = settings_show_desc;
    VcmiPaths::write_launcher_config(&cfg)
}

/// 仅更新「列表 hover 预览城镇背景」开关，**不校验游戏目录**。
///
/// 与 `set_update_pref` 同款：读回现有 launcher.json 后只改这一个字段再写回，
/// 避免影响 gameDir 等路径字段，也避免把该开关卷进设置面板的偏好写入
/// （`set_launcher_pref` 由设置页挂载即调用，带上它反而会被默认值覆盖）。
/// 仅更新「AI 高清增强」开关，**不校验游戏目录**（同 `set_town_preview` 模式）。
///
/// 走独立命令而不并入 `set_launcher_pref`：后者由设置页挂载即调用，带上本字段会被默认值覆盖。
#[tauri::command]
pub fn set_ai_hd_enhance(enabled: bool) -> Result<(), String> {
    let mut cfg =
        VcmiPaths::read_launcher_config().ok_or_else(|| be_err("settings_not_init", ""))?;
    cfg.hd.ai.enhance = enabled;
    VcmiPaths::write_launcher_config(&cfg)
}

/// 仅更新「AI 模型」（`None` / 空串 = 清除 → AI 随之不生效），**不校验游戏目录**。
///
/// 取值 = 组件目录（`runtime/ai/`）内的 `*.onnx` 文件名，**含用户自备模型**；
/// 含路径分隔符 / `..` 的值直接拒绝（避免把配置写脏，读取侧还会再挡一次）。
#[tauri::command]
pub fn set_ai_model(model: Option<String>) -> Result<(), String> {
    let mut cfg =
        VcmiPaths::read_launcher_config().ok_or_else(|| be_err("settings_not_init", ""))?;
    let next = model
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    if let Some(m) = &next {
        if m.contains(['/', '\\', ':']) || m.contains("..") {
            return Err(be_err("cmd_illegal_model_name", &format!("{}", m)));
        }
    }
    cfg.hd.ai.model = next;
    VcmiPaths::write_launcher_config(&cfg)?;
    tracing::info!(
        target: "config",
        "AI 模型已设为 {}",
        cfg.hd.ai.model.as_deref().unwrap_or("(未指定 —— AI 不生效)")
    );
    Ok(())
}

/// 列出组件目录内可用的 AI 模型（扫描 `*.onnx`，供设置页下拉）。
///
/// 返回项的 `file` 即 [`set_ai_model`] 可接受的取值；`known=true` = 在组件清单内（"已验证"）。
/// 组件未安装 / 目录为空 → 空列表（前端据此提示"先安装 AI 组件"）。
#[tauri::command]
pub fn list_ai_models() -> Vec<crate::runtime::AiModelInfo> {
    crate::runtime::scan_ai_models()
}

/// 校验某个**已知**模型的 sha256（按需调用：会读整个文件，17.5MB 级约几十毫秒）。
///
/// 返回 `null` = 该文件不在组件清单内（**未验证**，不是失败）；`false` = 已知但已损坏。
#[tauri::command]
pub fn verify_ai_model(file: String) -> Option<bool> {
    crate::runtime::verify_ai_model(&file)
}

/// 仅更新「高清素材生成」总开关，**不校验游戏目录**（模式同 `set_ai_hd_enhance`）。
#[tauri::command]
pub fn set_hd_enhance(enabled: bool) -> Result<(), String> {
    let mut cfg =
        VcmiPaths::read_launcher_config().ok_or_else(|| be_err("settings_not_init", ""))?;
    cfg.hd.enhance = enabled;
    VcmiPaths::write_launcher_config(&cfg)
}

/// 仅更新「列表 hover 预览城镇背景」开关，**不校验游戏目录**（模式同上方 `set_ai_hd_enhance`）。
#[tauri::command]
pub fn set_town_preview(enabled: bool) -> Result<(), String> {
    let mut cfg =
        VcmiPaths::read_launcher_config().ok_or_else(|| be_err("settings_not_init", ""))?;
    cfg.ui.town_preview = enabled;
    VcmiPaths::write_launcher_config(&cfg)?;
    tracing::info!(target: "config", "城镇背景 hover 预览已{}", if enabled { "开启" } else { "关闭" });
    Ok(())
}

/// 仅更新「自动检测更新」开关（launcher.json `realms.mod_auto_check`，默认开，
/// **不校验游戏目录**，模式同 `set_town_preview`）：仅领域「模组管理」页生效——
/// 打开管理页时静默检查一次模组仓库索引（fetch_mod_index，与 mods 页同一索引与缓存）；
/// 发行版不随包分发该字段，由各人本地开关决定。
#[tauri::command]
pub fn set_realm_mod_auto_check(enabled: bool) -> Result<(), String> {
    let mut cfg =
        VcmiPaths::read_launcher_config().ok_or_else(|| be_err("settings_not_init", ""))?;
    cfg.realms.mod_auto_check = Some(enabled);
    VcmiPaths::write_launcher_config(&cfg)?;
    tracing::info!(target: "config", "领域自动检测更新已{}", if enabled { "开启" } else { "关闭" });
    Ok(())
}

/// 记住「导出领域」发布分支的上次目录、URL 前缀覆盖与分支选择（launcher.json `realms.publish_*`）。
/// 三参均可为 `null`（清空）；导出弹窗打开时回填，避免每次重选。模式同 `set_realm_mod_auto_check`。
#[tauri::command]
pub fn set_realm_publish_target(
    repo_dir: Option<String>,
    url_base: Option<String>,
    mode: Option<String>,
) -> Result<(), String> {
    let mut cfg =
        VcmiPaths::read_launcher_config().ok_or_else(|| be_err("settings_not_init", ""))?;
    cfg.realms.publish_repo_dir = repo_dir.filter(|s| !s.trim().is_empty());
    cfg.realms.publish_url_base = url_base.filter(|s| !s.trim().is_empty());
    // 分支只接受两个已知值，写进配置的不能是任意字符串
    cfg.realms.publish_mode = mode
        .filter(|s| s == "share" || s == "publish")
        .map(|s| s.to_string());
    VcmiPaths::write_launcher_config(&cfg)?;
    Ok(())
}

/// 仅更新「HD 素材来源」（**不校验游戏目录**，模式同 `set_town_preview`）。
///
/// `kind`：`"auto"`（检测到 hd-edition 即用真高清）/ `"off"`（完全不用）。
/// 非法值一律按 `auto` 归一（宽容，避免前端传旧值把配置写脏）。
#[tauri::command]
pub fn set_hd_edition_use(kind: String) -> Result<(), String> {
    let mut cfg =
        VcmiPaths::read_launcher_config().ok_or_else(|| be_err("settings_not_init", ""))?;
    cfg.hd.edition_use = if kind.eq_ignore_ascii_case("off") {
        "off".to_string()
    } else {
        "auto".to_string()
    };
    VcmiPaths::write_launcher_config(&cfg)?;
    tracing::info!(target: "config", "HD 素材来源已设为 {}", cfg.hd.edition_use);
    Ok(())
}

/// 仅更新「HD 素材档位」：`"auto"`（x3 优先）/ `"x2"`（只用 x2，省磁盘）。
#[tauri::command]
pub fn set_hd_edition_scale(scale: String) -> Result<(), String> {
    let mut cfg =
        VcmiPaths::read_launcher_config().ok_or_else(|| be_err("settings_not_init", ""))?;
    cfg.hd.edition_scale = if scale.eq_ignore_ascii_case("x2") {
        "x2".to_string()
    } else {
        "auto".to_string()
    };
    VcmiPaths::write_launcher_config(&cfg)
}

/// 仅更新「网络代理」（`launcher.json` 的 `network.proxy`），**不校验游戏目录**。
///
/// 写入后立即刷新进程级代理（`infra::http::configure_proxy`），无需重启即可生效。
/// 归一：`None` / 空 / `"off"` = 直连；非 http(s) 方案（含 socks5）返回明确指引（reqwest 已移除内置 SOCKS）。
#[tauri::command]
pub fn set_network_proxy(proxy: Option<String>) -> Result<(), String> {
    let mut cfg =
        VcmiPaths::read_launcher_config().ok_or_else(|| be_err("settings_not_init", ""))?;
    let norm: Option<String> = match proxy
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        None => None,
        Some(s) if s.eq_ignore_ascii_case("off") => None,
        Some(s) => {
            let l = s.to_ascii_lowercase();
            if l.starts_with("http://") || l.starts_with("https://") {
                Some(s)
            } else if l.starts_with("socks5://")
                || l.starts_with("socks5h://")
                || l.starts_with("socks4://")
                || l.starts_with("socks://")
            {
                return Err(be_err("settings_proxy_socks", ""));
            } else {
                return Err(be_err("settings_proxy_prefix", ""));
            }
        }
    };
    cfg.network.proxy = norm.clone();
    VcmiPaths::write_launcher_config(&cfg)?;
    crate::infra::http::configure_proxy(norm.clone());
    tracing::info!(
        target: "config",
        "网络代理已设为 {}",
        norm.as_deref().unwrap_or("(直连)")
    );
    Ok(())
}

/// 更新「GitHub 镜像规则表」（`launcher.json` 的 `network.mirror_rules`，L2），**不校验游戏目录**。
///
/// 面向无本地代理用户：写入后归一化持久化并立即刷新进程级规则（`infra::http::configure_mirror_rules`），无需重启即生效。
/// 校验（D1–D6 定案后的权威层）：上限 10 条；逐条 `normalize_rule`（hosts 非空、http(s)、模板含占位符），
/// 任一条无效即整体拒绝并把序号写进错误文案（前端就地高亮）。
#[tauri::command]
pub fn set_network_mirror_rules(rules: Vec<crate::infra::http::MirrorRule>) -> Result<(), String> {
    if rules.len() > 10 {
        return Err(be_err("settings_mirror_max", ""));
    }
    let mut norm: Vec<crate::infra::http::MirrorRule> = Vec::with_capacity(rules.len());
    for (i, r) in rules.iter().enumerate() {
        let n = crate::infra::http::normalize_rule(r).ok_or_else(|| {
            format!(
                "第 {} 条规则无效：生效主机需非空；前缀/模板需为 http(s) 地址{}",
                i + 1,
                if r.mode == "template" {
                    "且至少含一个占位符（如 {url}）"
                } else {
                    ""
                }
            )
        })?;
        norm.push(n);
    }
    let mut cfg =
        VcmiPaths::read_launcher_config().ok_or_else(|| be_err("settings_not_init", ""))?;
    cfg.network.mirror_rules = norm.clone();
    VcmiPaths::write_launcher_config(&cfg)?;
    crate::infra::http::configure_mirror_rules(norm.clone());
    tracing::info!(
        target: "config",
        "镜像规则已更新（{} 条，其中 {} 条启用）",
        norm.len(),
        norm.iter().filter(|r| r.enabled).count()
    );
    Ok(())
}

/// 探测 HD 素材（`hd-edition`）当前状态（设置页「HD 素材」卡片展示，**只读、不注入**）。
#[tauri::command]
pub fn hd_edition_status() -> serde_json::Value {
    crate::vcmi::hd_edition::status_json()
}
