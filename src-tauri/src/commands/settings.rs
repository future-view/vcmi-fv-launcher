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
use tauri::{AppHandle, Emitter, State};

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
///
/// 首启向导的两项扩展（**均为可选**，旧调用不传即无此行为）：
/// - `redirectUserData: bool` —— 接管 `game_dir/config/dirs.json` 的 5 个 key 指向 `userDataDir`
///   （持久化到 `paths.redirect_user_data`，后续启动由 `ensure_user_data_dir` 保持接管）；
/// - `copyFrom: string` —— **在写任何配置之前**把该目录迁移到 `userDataDir`
///   （目标已存在 / 源不存在则静默跳过；进度经事件 `user-data-copy-progress` 推送）；
/// - `copyOnly: true` —— **只迁移、不写任何配置**（首启向导第 2 步「同步数据」/
///   第 3 步「复制数据文件」）：结果随出参返回
///   `{saved:false, copied, cancelled, files, bytes, h3Files, h3Bytes}`；
///   保存按钮本身**不再隐式复制**（单一职责：同步是独立动作）；
/// - `copyH3Data: true`（须与 `copyOnly` 同用）—— 从 `h3DataFrom`（`Data` 目录或含 `Data`
///   的游戏根目录）复制 `.lod` / `.snd` / `.vid` 到 `h3DataTo`（必须是 `…/Data` 目标目录），
///   不覆盖已存在文件（`.vid` 缺了 VCMI 会报「未检测到视频文件」）；
/// - `cancelCopy: true` —— 只置「跳过复制」哨兵并立即返回（前端「跳过复制」按钮用，
///   复用本命令而非新增命令，见 `docs/architecture/ipc.zh-CN.md` §1）。
///
/// ⚠️ **顺序是契约**：先迁移 → 再写 launcher.json → 最后接管 dirs.json。
/// 迁移中途关掉启动器（进程退出）时配置尚未落盘 ⇒ 下次启动原样重入向导，零副作用；
/// 反过来若先写配置，中断就会留下"dirs.json 指向半截目录"的残缺状态。
///
/// ⚠️ `async` + `spawn_blocking`：迁移可能是 GB 级复制，同步命令会冻结 WebView。
#[tauri::command]
pub async fn write_launcher_config(
    app: AppHandle,
    state: State<'_, AppPaths>,
    data: Value,
) -> Result<Value, String> {
    // 0) 仅用于中断迁移：与真正的保存互斥（先于任何校验返回，无需 gameDir）
    if data.get("cancelCopy").and_then(|v| v.as_bool()) == Some(true) {
        crate::infra::fs::cancel_dir_copy();
        tracing::info!(target: "paths", "用户数据迁移已请求取消");
        return Ok(json!({ "saved": false }));
    }
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
    // 用户数据目录 = 游戏目录 → dirs.json 会被自己改写，且 VCMI 双重挂载彻底乱套
    if user_data_dir == game_dir {
        return Err(be_err(
            "setup_userdata_invalid",
            &user_data_dir.to_string_lossy(),
        ));
    }

    // 1) 迁移既有用户数据 / 复制原版 H3 数据（可选）：必须在写盘之前完成，保证中断无副作用
    let copy_only = data.get("copyOnly").and_then(|v| v.as_bool()) == Some(true);
    let copy_h3 = data.get("copyH3Data").and_then(|v| v.as_bool()) == Some(true);
    let copy_from = data
        .get("copyFrom")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    let mut migrated = crate::vcmi::paths::UserDataMigration {
        copied: false,
        cancelled: false,
        files: 0,
        bytes: 0,
    };
    if let Some(src) = copy_from {
        // 「目标已存在则不复制」由 migrate 内部判定；这里只排除"源不存在"这种无需开工的情况
        if src.is_dir() && !user_data_dir.exists() {
            // 结构闸门：源不是 VCMI 用户数据目录（游戏目录 / mod 目录 / 压缩包
            // 解压的中间层）⇒ 一个字节都不复制。与第 ③ 步 `ensure_supported_source`
            // 同款"后端防绕过"定位；前端只做路径关系校验（无目录读取能力）。
            if !crate::vcmi::paths::looks_like_vcmi_user_data(&src) {
                return Err(be_err(
                    "setup_userdata_source_invalid",
                    &src.to_string_lossy(),
                ));
            }
            let emitter = app.clone();
            let dst = user_data_dir.clone();
            let res = tauri::async_runtime::spawn_blocking(move || {
                VcmiPaths::migrate_user_data_dir(&src, &dst, &mut |p| {
                    let _ = emitter.emit(
                        "user-data-copy-progress",
                        json!({
                            "doneFiles": p.done_files,
                            "doneBytes": p.done_bytes,
                            "totalFiles": p.total_files,
                            "totalBytes": p.total_bytes,
                        }),
                    );
                })
            })
            .await
            .map_err(|e| be_err("cmd_blocking_task_failed", &format!("{}", e)))??;
            if res.cancelled {
                return Err(be_err("setup_copy_cancelled", ""));
            }
            migrated = res;
            tracing::info!(
                target: "paths",
                "用户数据迁移结束：copied={} files={} bytes={}",
                migrated.copied,
                migrated.files,
                migrated.bytes
            );
        }
    }
    // 1.5) 只迁移模式：立即返回，**不写 launcher.json / dirs.json**
    //      （首启向导第 2 步「同步数据」/ 第 3 步「复制数据文件」；保存按钮不再隐式复制）
    if copy_only {
        // 第 3 步的 H3 数据复制：源目录（Data 或含 Data 的游戏根）→ 目标 Data 目录，
        // 另可按需搬 Mp3 / Maps 到**用户数据目录**（官方首启同口径）
        let mut h3_files = 0u64;
        let mut h3_bytes = 0u64;
        if copy_h3 {
            let from = data
                .get("h3DataFrom")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .ok_or_else(|| be_err("setup_h3_data_dir_missing", ""))?;
            let to = data
                .get("h3DataTo")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .ok_or_else(|| be_err("setup_h3_data_dir_missing", ""))?;
            let user_dir = data
                .get("h3UserDataDir")
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .ok_or_else(|| be_err("settings_missing_userdata", ""))?;
            // ⚠️ Mp3 / Maps 与 .lod 同一个 radio 目标根（用户实测：恒落用户数据目录 ⇒ 已修）
            let extra_root = user_dir.clone();
            let (from_root, from_data) = crate::vcmi::h3data::resolve_source(&from)
                .ok_or_else(|| be_err("setup_h3_data_dir_invalid", &from.to_string_lossy()))?;
            // 版本闸门：RoE-only / AB-only / HD Edition 搬过去 VCMI 也起不来
            // （`EntryPoint.cpp:329-332` 对 ROE 数据直接 fatal）。必须在**复制之前**拒绝，
            // 否则会白搬几百 MB 并污染游戏目录。前端也做同样预检，这里是防绕过的双保险。
            crate::vcmi::h3data::ensure_supported_source(&from)?;
            if !to
                .file_name()
                .map(|n| n.eq_ignore_ascii_case("Data"))
                .unwrap_or(false)
            {
                return Err(be_err(
                    "setup_h3_data_dir_invalid",
                    &to.to_string_lossy(),
                ));
            }
            let copy_mp3 = data.get("h3CopyMp3").and_then(|v| v.as_bool()).unwrap_or(true);
            let copy_maps = data.get("h3CopyMaps").and_then(|v| v.as_bool()).unwrap_or(false);
            // 诊断日志：数据到底落在哪，一目了然（radio 归属争议时第一手证据）
            tracing::info!(
                target: "paths",
                "原版数据复制：lod → {}；mp3/maps → {}（源 Data = {}）",
                to.display(),
                user_dir.display(),
                from_data.display()
            );
            let emitter = app.clone();
            let (files, bytes) = tauri::async_runtime::spawn_blocking(move || {
                // ① .lod / .snd / .vid → <游戏目录>/Data（判定根之一，与用户数据目录无关）。
                //    .snd/.vid 不可省：缺 VIDEO.VID 时 VCMI 会弹「未检测到视频文件」。
                let (f, b) = crate::vcmi::h3data::copy_data_files(&from_data, &to, |f, b, tf, tb| {
                    let _ = emitter.emit(
                        "user-data-copy-progress",
                        json!({
                            "stage": "data",
                            "doneFiles": f,
                            "doneBytes": b,
                            "totalFiles": tf,
                            "totalBytes": tb,
                        }),
                    );
                })?;
                // ② Mp3 / Maps → **与 .lod 同一个 radio 目标根**（VCMI 的 MUSIC / MAPS
                //    挂载点同时挂游戏目录与用户数据目录，两处都可见）；
                //    Maps 目录即使不复制也会创建（VCMI 不自建）
                let emitter2 = emitter.clone();
                let (f2, b2) = crate::vcmi::h3data::copy_extra(
                    &from_root,
                    &extra_root,
                    copy_mp3,
                    copy_maps,
                    // 阶段名小写（`mp3` / `maps`）：前端据此换标题，避免两段进度看起来一模一样
                    |stage, f, b, tf, tb| {
                        let _ = emitter2.emit(
                            "user-data-copy-progress",
                            json!({
                                "stage": stage.to_ascii_lowercase(),
                                "doneFiles": f,
                                "doneBytes": b,
                                "totalFiles": tf,
                                "totalBytes": tb,
                            }),
                        );
                    },
                )?;
                Ok::<(u64, u64), String>((f + f2, b + b2))
            })
            .await
            .map_err(|e| be_err("cmd_blocking_task_failed", &format!("{}", e)))??;
            h3_files = files;
            h3_bytes = bytes;
        }
        return Ok(json!({
            "saved": false,
            "copied": migrated.copied,
            "cancelled": migrated.cancelled,
            "files": migrated.files,
            "bytes": migrated.bytes,
            "h3Files": h3_files,
            "h3Bytes": h3_bytes,
        }));
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
            // 接管开关：显式传入时以其为准（首启向导「使用启动器专属用户数据目录」），
            // 否则沿用既有配置（便携包可预置）
            redirect_user_data: data
                .get("redirectUserData")
                .and_then(|v| v.as_bool())
                .or_else(|| inherit.map(|p| p.paths.redirect_user_data))
                .unwrap_or(false),
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
        // 完成目录初始化，不等后续 detect 才补。
        //
        // ⚠️ 请求了接管（自定义用户数据目录）时失败**必须上报**：dirs.json 没改成，
        // VCMI 仍会用官方目录，界面显示的目录与实际生效的不一致 —— 属静默错配，不可吞。
        // 未请求接管时保持"非致命"（目录可能只读，旧行为不变）。
        if let Err(e) = VcmiPaths::ensure_user_data_dir(&paths, cfg.realms.active.as_deref()) {
            if cfg.paths.redirect_user_data {
                return Err(e);
            }
            tracing::warn!(target: "paths", "保存配置后目录初始化失败（可忽略）：{e}");
        }
    }
    tracing::info!(target: "config", "已保存 launcher.json");
    Ok(json!({
        "saved": true,
        "copied": migrated.copied,
        "cancelled": migrated.cancelled,
        "files": migrated.files,
        "bytes": migrated.bytes,
    }))
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

/// 探测 HD 素材（`hd-edition`）当前状态（设置页卡片 + 首启向导第 ④ 步，**只读、不注入**）。
///
/// `gameDir` / `userDataDir` 为**可选覆盖**（同 `import_game_files`）：首启向导阶段
/// `launcher.json` 还没写，必须能按用户刚选的目录探测；不传 = 沿用已保存配置。
#[tauri::command]
pub fn hd_edition_status(
    game_dir: Option<String>,
    user_data_dir: Option<String>,
) -> serde_json::Value {
    let g = game_dir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(std::path::Path::new);
    let u = user_data_dir
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(std::path::Path::new);
    crate::vcmi::hd_edition::status_json_at(g, u)
}
