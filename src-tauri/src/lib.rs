mod commands;
mod infra;
mod modcache;
mod realms;
mod repository;
mod runtime;
mod state;
mod updater;
mod vcmi;

use tauri::Manager;

use state::AppPaths;

use commands::{
    apply_launcher_update, cancel_launcher_update, check_launcher_update, download_launcher_update,
    set_update_manifest_url, set_update_pref,
};
use commands::{
    build_all_mod_cache, build_mods_cache_batch, build_realm_cache_urgent, cache_build_progress,
    cancel_cache_build, cancel_realm_cache_build, check_realm_assets, clear_mod_validation,
    close_window, detect_edition, detect_vcmi_paths, ensure_showcase_audio, gamedata_health_check,
    generate_town_preview, get_compile_info, get_dirs, get_mods_cache_dir, get_primary_monitor,
    get_runtime_info, hd_edition_status, import_game_files, install_local_mod, is_debug,
    launch_editor, launch_game, launch_native_launcher, list_ai_models, list_installed_mods,
    list_monitors, list_resolutions, log_frontend, minimize_window, open_in_explorer,
    patch_preset_lang_mods, preview_mod_zip, read_cache_index, read_def_bytes,
    read_launcher_config, read_mod_cache, read_mod_metas, read_mod_settings, read_showcase_slides,
    release_wiki_caches,
    read_town_cards, read_vcmi_settings, realm_cache_build_progress, runtime_cancel,
    runtime_install, runtime_status, runtime_uninstall, scan_mods, set_ai_hd_enhance, set_ai_model,
    set_hd_edition_scale, set_hd_edition_use, set_hd_enhance, set_launcher_pref,
    set_network_mirror_rules, set_network_proxy, set_realm_mod_auto_check,
    set_realm_publish_target, set_town_preview, set_ui_lang, set_vcmi_language,
    suggest_user_data_dir, toggle_maximize_window, verify_ai_model, wiki_cache_state,
    write_launcher_config, write_mod_settings, write_vcmi_settings,
};
use realms::{
    activate_realm, apply_realm_maps_valve, cancel_realm_export, check_realm_id,
    check_realm_mod_conflicts, check_realm_mods_ready, create_realm, deactivate_realm,
    delete_realm, delete_realm_map, ensure_realm_lang_visible, export_realm, get_active_realm,
    get_bgm_muted, get_intro_state, import_realm, import_realm_asset, import_realm_maps_dir,
    is_vcmi_running, kill_vcmi, list_realm_maps, list_realm_mods, list_realms,
    list_vcmi_game_processes, list_vcmi_processes, mark_intro_played, move_realm_map,
    native_maps_status, preview_realm_publish, realm_config_dir, realm_dir, realm_mod_bundle,
    realm_mod_delete, realm_mod_import, realm_mod_ops, realm_mod_ref, realm_mod_unref,
    realm_self_check, realm_self_heal, rename_realm, repair_game_maps, set_bgm_muted,
    set_realm_map_filter, set_realm_maps_blocked, update_realm, upload_realm_map,
    write_realm_presets,
};
use repository::{
    add_realm_repository, auto_check_repos_enabled, cancel_mod_download, cancel_realm_download,
    delete_mod, ensure_realm_covers, fetch_mod_index, fetch_realm_index, get_realm_repositories,
    install_mod_from_repo, install_remote_realm, list_mod_ledgers, remove_realm_repository,
    test_realm_repository,
};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            // 配置补丁（更新包携带的 `.vcmi-fv-launcher/launcher.patch.json`）：
            // **必须最早执行** —— 否则补丁里的 `log_level` 等要等下次启动才生效。
            // 一次性语义：合并成功即删除补丁；失败只记日志，绝不阻断启动。
            updater::config_patch::apply();
            // 删除清单（更新包携带的 `.vcmi-fv-launcher/launcher.delete.json`）：
            // 同样在最早阶段执行 —— 被删的可能是后续步骤要读取的缓存 / 领域。
            // 白名单 + 联接安全删除，见 `updater/delete_patch.rs`。
            updater::delete_patch::apply();

            // 日志必须在 detect() **之前**初始化：detect 会做目录初始化（创建 userdata
            // 子目录、便携接管 dirs.json），后续的领域激活 / 自检 / 缓存构建也都需要被记录。
            // 日志目录由 launcher_data_dir() 派生（不依赖 launcher.json）；
            // 等级从 launcher.json 的 `runtime.log_level` 读取（用宽容读取，不要求配置完整有效）。
            if let Some(logs_dir) = vcmi::paths::VcmiPaths::launcher_data_dir().map(|d| d.join("logs")) {
                let level = vcmi::paths::VcmiPaths::read_launcher_config_lenient()
                    .and_then(|c| c.runtime.log_level)
                    .map(|s| infra::logging::LogLevel::parse(&s))
                    .unwrap_or(infra::logging::LogLevel::Info);
                match infra::logging::init(&logs_dir, level) {
                    Ok(()) => tracing::info!(target: "launcher", "启动器启动，日志等级={:?}", level),
                    Err(e) => eprintln!("[vcmi-fv] 日志初始化失败（可忽略）：{e}"),
                }
            }

            // 主窗口几何：**在窗口显示前**完成尺寸与居中（配置配 `"visible": false` + 此处 `show()`），
            // 否则用户会看到"先 1280×720 再跳大"的可见跳变 —— 见 `commands/app.rs::apply_main_window_geometry`。
            // 放在日志初始化之后：几何结果（含取不到显示器的兜底）能进日志；
            // 放在耗时逻辑（detect 等）之前：窗口尽早显示，前端 JS 也要等 setup 结束后的消息循环才会执行。
            commands::apply_main_window_geometry(app.handle());

            // 外挂运行时组件配置注入（`launcher.json` 的 `runtime.components.*`）：
            // **只记录目录与远程包，不加载** —— 真正加载在首次用到 AI 超分时
            // （`infra::ai` 懒加载），未开启「AI 高清增强」的用户零开销。
            // ⚠️ 用宽容读取：不要求 launcher.json 完整有效（与 log_level 同一口径）。
            let runtimes = vcmi::paths::VcmiPaths::read_launcher_config_lenient()
                .map(|c| c.runtime.components)
                .unwrap_or_default();
            runtime::configure(runtimes);
            // 网络出口：把 `launcher.json` 的 `network.proxy` 注入进程级代理（默认 None=直连）。
            // 与 log_level / runtimes 同一口径（宽容读取）；运行时改动即时生效走 `set_network_proxy`。
            infra::http::configure_proxy(
                vcmi::paths::VcmiPaths::read_launcher_config_lenient().and_then(|c| c.network.proxy),
            );
            // 网络出口：同样注入 GitHub 镜像规则表（L2，默认空=不用）；改动走 `set_network_mirror_rules`。
            infra::http::configure_mirror_rules(
                vcmi::paths::VcmiPaths::read_launcher_config_lenient()
                    .map(|c| c.network.mirror_rules)
                    .unwrap_or_default(),
            );
            // 版本迁移（幂等、可重入；任何失败只记日志，**绝不阻断启动**）。
            // 见 `updater/migrate.rs` 模块头：能靠 bump 版本常量解决的清理不要写进迁移。
            updater::migrate::run();
            // AI：把「已通过校验的组件目录」注入 `infra::ai`。
            // ⚠️ `infra` 禁止反向依赖 `runtime` / `vcmi`（会成环），只能由这里传下去；
            // 传 `None` 时 AI 保持禁用（组件未安装是正常的）。
            infra::ai::configure_runtime(if runtime::is_ready(runtime::AI) {
                runtime::dir_of(runtime::AI)
            } else {
                None
            });

            let paths = vcmi::paths::VcmiPaths::detect();
            app.manage(AppPaths(std::sync::Mutex::new(paths)));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            read_mod_settings,
            write_mod_settings,
            patch_preset_lang_mods,
            clear_mod_validation,
            gamedata_health_check,
            preview_mod_zip,
            install_local_mod,
            import_game_files,
            log_frontend,
            get_dirs,
            scan_mods,
            detect_edition,
            launch_game,
            launch_editor,
            open_in_explorer,
            read_launcher_config,
            write_launcher_config,
            set_ui_lang,
            set_launcher_pref,
            set_ai_hd_enhance,
            set_ai_model,
            list_ai_models,
            verify_ai_model,
            set_hd_enhance,
            set_town_preview,
            set_realm_mod_auto_check,
            set_realm_publish_target,
            set_network_proxy,
            set_network_mirror_rules,
            // HD 素材来源（官方 HD 版 `hd-edition` 的 .pak）：开关 / 档位 / 状态
            set_hd_edition_use,
            set_hd_edition_scale,
            hd_edition_status,
            detect_vcmi_paths,
            suggest_user_data_dir,
            read_vcmi_settings,
            write_vcmi_settings,
            set_vcmi_language,
            launch_native_launcher,
            get_primary_monitor,
            list_monitors,
            // 外挂运行时组件（runtime::AI 等）
            // ⚠️ 必须**每行一个**：`check:invoke-parity` 的注册表解析只认整行单命令名
            runtime_status,
            runtime_install,
            runtime_cancel,
            runtime_uninstall,
            get_compile_info,
            read_mod_cache,
            cancel_cache_build,
            build_realm_cache_urgent,
            cancel_realm_cache_build,
            realm_cache_build_progress,
            check_realm_assets,
            generate_town_preview,
            build_mods_cache_batch,
            release_wiki_caches,
            cache_build_progress,
            wiki_cache_state,
            build_all_mod_cache,
            read_cache_index,
            read_mod_metas,
            read_town_cards,
            read_showcase_slides,
            ensure_showcase_audio,
            get_mods_cache_dir,
            close_window,
            minimize_window,
            toggle_maximize_window,
            is_debug,
            list_resolutions,
            list_installed_mods,
            get_runtime_info,
            read_def_bytes,
            list_realms,
            get_active_realm,
            realm_config_dir,
            write_realm_presets,
            create_realm,
            check_realm_id,
            update_realm,
            delete_realm,
            activate_realm,
            deactivate_realm,
            set_realm_maps_blocked,
            kill_vcmi,
            realm_self_check,
            realm_self_heal,
            export_realm,
            cancel_realm_export,
            preview_realm_publish,
            import_realm,
            realm_dir,
            get_intro_state,
            mark_intro_played,
            get_bgm_muted,
            set_bgm_muted,
            import_realm_asset,
            check_realm_mod_conflicts,
            check_realm_mods_ready,
            rename_realm,
            list_realm_maps,
            upload_realm_map,
            delete_realm_map,
            native_maps_status,
            repair_game_maps,
            import_realm_maps_dir,
            move_realm_map,
            apply_realm_maps_valve,
            set_realm_map_filter,
            is_vcmi_running,
            list_vcmi_processes,
            list_vcmi_game_processes,
            get_realm_repositories,
            add_realm_repository,
            remove_realm_repository,
            test_realm_repository,
            fetch_realm_index,
            install_remote_realm,
            cancel_realm_download,
            ensure_realm_covers,
            // 模组仓库（获取与更新，docs/features/mods-repository.zh-CN.md）
            // ⚠️ 每行一个：check:invoke-parity 只认整行单命令名
            fetch_mod_index,
            install_mod_from_repo,
            cancel_mod_download,
            auto_check_repos_enabled,
            list_mod_ledgers,
            delete_mod,
            realm_mod_ops,
            // 领域模组管理（realm-mods-management §10.1）：已实装 / 引用 / 整合 / 删除 / 导入
            list_realm_mods,
            realm_mod_ref,
            realm_mod_unref,
            realm_mod_bundle,
            realm_mod_delete,
            realm_mod_import,
            ensure_realm_lang_visible,
            check_launcher_update,
            download_launcher_update,
            cancel_launcher_update,
            apply_launcher_update,
            set_update_pref,
            set_update_manifest_url
        ])
        .on_window_event(|window, event| {
            // 关闭即应用：主窗口收到**关闭请求**时（自定义 X / Alt+F4 / 系统级关闭一视同仁），
            // 若有「已下载待应用」的含 exe 更新，先 spawn 静默替换脚本再放行关闭——
            // 脚本等本进程退出后原地替换 + 拉起新版本，用户下次打开即新版。
            // `close_window` 命令里也有一道同样的判定：谁先命中谁应用，另一边
            // 因 phase 已回 Idle / pending 已清空而天然跳过（不会双重 spawn）。
            if let tauri::WindowEvent::CloseRequested { .. } = event {
                if window.label() == "main" && crate::updater::apply::should_auto_apply_on_close() {
                    if let Err(e) = crate::updater::apply::apply_update(false) {
                        tracing::warn!(target: "updater", "关闭时自动应用更新失败，回退普通关闭：{e}");
                    }
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
