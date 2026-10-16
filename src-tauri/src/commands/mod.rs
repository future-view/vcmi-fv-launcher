//! IPC 命令层（`commands/`）—— **唯一允许出现 `tauri` 类型的层**。
//!
//! 依赖方向：`commands → wiki / realms / repository / vcmi / infra`（见设计文档 §4.1）。
//! 每个文件只做「`State<AppPaths>` 解包 → 调下层 → `Result<_, String>` 转换」，
//! 业务逻辑一律下沉到 `vcmi::*` / `wiki::*` / `infra::*`，以便单测与复用。
//!
//! ⚠️ **函数名 + 参数名 = 前端契约**（`invoke("xxx", {...})`）：重命名或改参数会静默 404。
//! 任何改动后跑 `npm run check:invoke-parity`（前端 invoke / 本层 `#[tauri::command]` /
//! `lib.rs` 注册表三方比对）。
//!
//! 文件划分：
//! - [`mods`]　        模组扫描 / 已安装清单 / 版本探测 / DEF 资源字节
//! - [`mod_install`]　 本地 zip 安装模组（预览 / 安装）
//! - [`game_import`]　「导入游戏文件」（地图/战役/存档/模组/配置/HD 素材）
//! - [`mod_settings`]　`modSettings.json` 读写与预设语言补丁
//! - [`settings`]　    启动器自身配置（`launcher.json`，三层格式契约见 §7.1）
//! - [`vcmi_settings`] VCMI 游戏设置（`config/settings.json`）
//! - [`paths`]　       目录识别与建议
//! - [`launch`]　      启动游戏 / 地图编辑器 / 官方启动器
//! - [`app`]　         窗口控制、运行信息、编译信息（含 Windows PE 版本 FFI）
//! - [`wiki`]　        百科缓存命令（读取 / 预览 / 批量构建 / 进度 / 取消）

mod app;
mod cache;
mod game_import;
mod gamedata;
mod launch;
mod mod_install;
mod mod_settings;
mod mods;
mod paths;
mod runtime;
mod settings;
mod updater;
mod vcmi_settings;

pub use app::{
    apply_main_window_geometry, close_window, get_compile_info, get_primary_monitor,
    get_runtime_info, is_debug, list_monitors, list_resolutions, log_frontend, minimize_window,
    open_in_explorer, toggle_maximize_window,
};
pub use cache::{
    build_all_mod_cache,
    build_mods_cache_batch,
    // 领域加急队列（S1）：独立进度通道 + 就绪检查
    build_realm_cache_urgent,
    cache_build_progress,
    cancel_cache_build,
    cancel_realm_cache_build,
    check_realm_assets,
    ensure_showcase_audio,
    generate_town_preview,
    get_mods_cache_dir,
    read_cache_index,
    read_mod_cache,
    read_mod_metas,
    read_showcase_slides,
    read_town_cards,
    realm_cache_build_progress,
    // 整轮构建结束后的内存回收（构建期进程级缓存的统一释放入口）
    release_wiki_caches,
    // 启动预检：是否需要重建缓存（供前端决定是否弹蒙层）
    wiki_cache_state,
};
pub use game_import::import_game_files;
pub use gamedata::gamedata_health_check;
pub use launch::{launch_editor, launch_game, launch_native_launcher};
pub use mod_install::{install_local_mod, preview_mod_zip};
pub use mod_settings::{
    clear_mod_validation, patch_preset_lang_mods, read_mod_settings, write_mod_settings,
};
pub use mods::{detect_edition, list_installed_mods, read_def_bytes, scan_mods};
pub use paths::{detect_vcmi_paths, get_dirs, suggest_user_data_dir};
pub use runtime::{runtime_cancel, runtime_install, runtime_status, runtime_uninstall};
pub use settings::{
    hd_edition_status, list_ai_models, read_launcher_config, set_ai_hd_enhance, set_ai_model,
    set_hd_edition_scale, set_hd_edition_use, set_hd_enhance, set_launcher_pref,
    set_network_mirror_rules, set_network_proxy, set_realm_mod_auto_check,
    set_realm_publish_target, set_town_preview, set_ui_lang, verify_ai_model,
    write_launcher_config,
};
pub use updater::{
    apply_launcher_update, cancel_launcher_update, check_launcher_update, download_launcher_update,
    set_update_manifest_url, set_update_pref,
};
pub use vcmi_settings::{read_vcmi_settings, set_vcmi_language, write_vcmi_settings};
