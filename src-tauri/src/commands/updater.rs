//! 启动器自动更新命令（IPC 壳）：检查 / 下载 / 取消 / 应用 / 偏好与清单地址。
//!
//! 事件 emit 只在本层（`launcher-update-progress`）；业务一律在 `updater/`。
//! ⚠️ 命令名 + 参数名 = 前端契约；改后必跑 `npm run check:invoke-parity`。
//!
//! 退出时序约定：`apply_launcher_update` 成功后**由前端延迟调用 `close_window`**，
//! 本层不主动 `app.exit`（否则 IPC 响应可能送不达）。

use crate::infra::berror::be_err;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use crate::updater;

/// 检查启动器更新。`force=false` 为启动静默检查（失败静默不回显）；
/// `force=true` 为手动检查（失败填 `error` 供界面展示，忽略「跳过此版本」）。
///
/// `probe=Some(true)`：**只探测清单可达性**（首启向导用它判断是否建议开启 GitHub 镜像），
/// 不解析清单、不比较版本、不触碰更新状态机；结果在 `UpdateInfo::probe`。
/// 复用本命令扩参数而非新增命令 —— 本机 Rust 工具链对**新增** Tauri 命令会触发
/// never-type 硬错误（见 `docs/architecture/ipc.zh-CN.md` §1）；旧调用不传 `probe`，行为不变。
///
/// ⚠️ `async` + `spawn_blocking`：清单拉取 / 探测都是网络 IO，同步命令会冻结 WebView
/// （探测本身最长 8s，绝不能占主线程）。
#[tauri::command]
pub async fn check_launcher_update(
    force: bool,
    probe: Option<bool>,
) -> updater::manifest::UpdateInfo {
    if probe == Some(true) {
        return tauri::async_runtime::spawn_blocking(updater::manifest::probe_manifest)
            .await
            .unwrap_or_else(|e| updater::manifest::error_info(&format!("{}", e)));
    }
    tauri::async_runtime::spawn_blocking(move || updater::manifest::check_update(force))
        .await
        .unwrap_or_else(|e| updater::manifest::error_info(&format!("{}", e)))
}

/// 下载更新包（**阻塞**至完成）并强制 sha256 校验。
/// 进度经 `launcher-update-progress` 事件推送 `{received, total, done?}`，每 200ms 一次。
///
/// ⚠️ **必须 `async` + `spawn_blocking`**：Tauri 的同步命令在主线程执行，下载（慢网可达数分钟）
/// 会把 WebView 消息循环一起冻住 —— 表现为「最小化再还原后窗口变灰 / 标题栏被判『未响应』」，
/// 而下载本身还在推进（进度条照动），用户只能等它跑完；同期点「取消下载」也进不来
/// （命令排在阻塞的下载之后）。与 P8（`fetch_mod_index`）、R1（启动期命令）同一口径。
#[tauri::command]
pub async fn download_launcher_update(app: AppHandle) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let emitter = app.clone();
        let path = updater::download::download_and_verify(move |received, total| {
            let _ = emitter.emit(
                "launcher-update-progress",
                json!({ "received": received, "total": total }),
            );
        })?;
        let _ = app.emit(
            "launcher-update-progress",
            json!({ "received": 0, "total": 0, "done": true }),
        );
        Ok(json!({ "ok": true, "path": path.to_string_lossy() }))
    })
    .await
    .map_err(|e| be_err("cmd_blocking_task_failed", &format!("{}", e)))?
}

/// 取消正在进行的更新下载（置哨兵，下载循环下一块退出；状态回 available 可重试）。
#[tauri::command]
pub fn cancel_launcher_update() -> Result<(), String> {
    updater::set_cancelled(true);
    Ok(())
}

/// 应用更新。分两条路径：
///
/// - **纯配置更新包**（zip 内不含 exe）→ **进程内直接落地**，**无需关闭启动器**；
///   返回 `restarted: false`，`failed` 非空表示个别文件被占用（提示用户关闭后重试）。
/// - 其余（含 exe / 单 exe 包）→ 生成 detached 替换脚本，前端随后延迟 `close_window`；
///   脚本等进程退出后 `move 旧→.old`、`move 新→原位`、拉起新版本并自删；返回 `restarted: true`。
///
/// ⚠️ `async` + `spawn_blocking`：就地落地会逐文件拷贝（更新包可达 200MB），同步命令在主线程跑
/// 同样会冻住消息循环（同 `download_launcher_update`）；`relaunch` 的脚本 spawn 本身是毫秒级，
/// 放阻塞线程池无副作用。
#[tauri::command]
pub async fn apply_launcher_update() -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(apply_launcher_update_blocking)
        .await
        .map_err(|e| be_err("cmd_blocking_task_failed", &format!("{}", e)))?
}

fn apply_launcher_update_blocking() -> Result<Value, String> {
    // 调试构建不做任何落地（tauri dev 产物受保护）；`apply_update` 内亦有一道同样的保护
    if cfg!(debug_assertions) {
        return Err(be_err("cmd_debug_no_update_apply", ""));
    }
    // 与「组件安装」互斥：两边都会写 `runtime/**`（更新包带组件时）
    if crate::runtime::install::any_busy() {
        return Err(be_err("updater_launcher_runtime_busy", ""));
    }
    if let Some(entry) = updater::target().clone() {
        if entry.is_zip() && !updater::download::staged_has_exe(&entry.version) {
            let failed = updater::apply::apply_inplace(&entry.version)?;
            // 记「已应用」（按 sha256 + exe 版本）—— 纯配置包不改版本号，
            // 不记录会被每次启动反复提示（见 `updater::is_applied`）。
            updater::mark_applied(&entry.sha256);
            // 清除"待应用"标记：已成功落地，下次启动不再提示
            updater::set_pending_apply(None);
            // 就地应用完成后清理解压目录（`.zip` 本体保留，便于复用 / 重装）
            if let Ok(d) = updater::download::staged_dir(&entry.version) {
                let _ = std::fs::remove_dir_all(&d);
            }
            updater::set_phase(updater::UpdatePhase::Idle);
            *updater::target() = None;
            tracing::info!(
                target: "updater",
                "纯配置更新包已就地应用（{}），未落地 {} 项",
                entry.version,
                failed.len()
            );
            return Ok(json!({ "ok": true, "restarted": false, "failed": failed }));
        }
    }
    // 手动「重启并应用」：替换后重新拉起新版本
    updater::apply::apply_update(true)?;
    // 清除"待应用"标记：含 exe 的路径由 detached 脚本替换并拉起新版本，
    // 前端随后 close_window；标记在此先清，避免下次启动（若脚本异常未替换）重复提示错误版本
    updater::set_pending_apply(None);
    Ok(json!({ "ok": true, "restarted": true }))
}

/// 写入更新偏好（**三态语义**，均不校验游戏目录）：
///
/// - `auto_check`：`Some` = 修改启动自动检查开关；`None` = 保持不变。
/// - `skip_version`：`Some("")` = 清空「跳过此版本」；`Some(非空)` = 设置；
///   `None` = 保持不变（**注意**：空串与 null 语义不同，前端 `setUpdatePref` 已封装）。
/// - `prompt_version`：`Some(非空)` = 记录「已自动弹出该版本提醒」，与
///   `update.prompt_at`（Unix 秒）一起写入，作为 7 天节流的起算点；`None` / 空 = 保持不变。
///
/// ⚠️ 为什么不另立 `mark_update_prompted`：本机 Rust 工具链对**新增** Tauri 命令会触发
/// never-type 硬错误（见 `03-architecture`「约束」），故复用本命令承载"记录提醒"。
/// 参数为**向后兼容扩展**：旧调用 `{autoCheck, skipVersion}` 行为不变。
#[tauri::command]
pub fn set_update_pref(
    auto_check: Option<bool>,
    skip_version: Option<String>,
    prompt_version: Option<String>,
) -> Result<(), String> {
    let mut cfg = crate::vcmi::paths::VcmiPaths::read_launcher_config()
        .ok_or_else(|| be_err("settings_not_init", ""))?;
    if let Some(v) = auto_check {
        cfg.update.auto_check = v;
    }
    if let Some(sv) = skip_version {
        let t = sv.trim().to_string();
        cfg.update.skip_version = if t.is_empty() { None } else { Some(t.clone()) };
        // 跳过当前「待应用」版本 → 清掉标记，否则跨会话仍会弹"更新包已就绪"
        if !t.is_empty() && updater::pending_apply().as_deref() == Some(t.as_str()) {
            updater::set_pending_apply(None);
        }
    }
    if let Some(pv) = prompt_version
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        // 空版本号（清单异常）不记录：避免把节流算到空版本上导致后续版本也不再提醒
        cfg.update.prompt_version = Some(pv);
        cfg.update.prompt_at = Some(crate::infra::time::now_secs());
        // 同时记下"当时的 exe 版本"：节流只在 exe 版本未变时生效
        // （升级 / 降级 / 手工换包后旧记录自动失效，见 `UpdateCfg::prompt_exe_version`）。
        cfg.update.prompt_exe_version = Some(crate::updater::manifest::CURRENT_VERSION.to_string());
    }
    crate::vcmi::paths::VcmiPaths::write_launcher_config(&cfg)
}

/// 写入更新清单地址（发行版可随包预置，设置页可改）。空 / null = 清除（完全不检查）。
#[tauri::command]
pub fn set_update_manifest_url(url: Option<String>) -> Result<(), String> {
    let trimmed = url.map(|u| u.trim().to_string()).filter(|s| !s.is_empty());
    if let Some(u) = &trimmed {
        // 默认只认 https；明文 http 需环境变量 VCMI_FV_UPDATER_ALLOW_HTTP=1 显式放行
        // （本地预演 / 只有 http 的自建 CDN）。与 manifest.rs::download_scheme_ok 同口径。
        let allow_http = std::env::var("VCMI_FV_UPDATER_ALLOW_HTTP")
            .map(|v| v == "1")
            .unwrap_or(false);
        if !u.starts_with("https://") && !(u.starts_with("http://") && allow_http) {
            return Err(be_err("cmd_update_manifest_not_https", ""));
        }
    }
    let mut cfg = crate::vcmi::paths::VcmiPaths::read_launcher_config()
        .ok_or_else(|| be_err("settings_not_init", ""))?;
    cfg.update.manifest_url = trimmed;
    crate::vcmi::paths::VcmiPaths::write_launcher_config(&cfg)
}
