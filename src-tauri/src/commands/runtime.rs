//! 外挂运行时组件命令（IPC 壳）：状态 / 安装 / 取消 / 卸载。
//!
//! 事件 emit 只在本层（`runtime-install-progress`）；业务一律在 `runtime/`。
//! ⚠️ 命令名 + 参数名 = 前端契约；改后必跑 `npm run check:invoke-parity`。
//!
//! ## 分发形态
//! - 便携 / 安装包内置：文件已就位，`runtime_status` 直接报 `installed: true`；
//! - 远程下载：launcher.json 配了 `runtime.components.<id>.package` 才有 `has_package: true`，
//!   由用户点「安装」触发 [`runtime_install`]。

use crate::infra::berror::be_err;
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

use crate::runtime;

/// 组件变化后把「已校验的目录」重新注入 `infra::ai` —— 否则安装完仍沿用旧的禁用状态。
///
/// ⚠️ `infra` 不能反过来读 `runtime`（分层），只能由命令层同步过去。
fn sync_ai(id: &str) {
    if id == runtime::AI {
        crate::infra::ai::configure_runtime(if runtime::is_ready(id) {
            runtime::dir_of(id)
        } else {
            None
        });
    }
}

/// 查询组件状态：目录、文件齐备情况、sha256 是否匹配、是否可在线安装。
#[tauri::command]
pub fn runtime_status(id: String) -> Result<Value, String> {
    let st = runtime::status(&id)?;
    serde_json::to_value(st).map_err(|e| be_err("cmd_runtime_state_serialize_failed", &format!("{}", e)))
}

/// 安装组件（**阻塞**至完成）：下载 → 整包 sha256 → 白名单解压 → 逐文件校验 → 原子替换。
///
/// 进度经 `runtime-install-progress` 事件推送 `{id, received, total, done?}`，每 200ms 一次。
/// 返回安装后的最新状态，便于前端直接刷新 UI。
#[tauri::command]
pub fn runtime_install(app: AppHandle, id: String) -> Result<Value, String> {
    // 与「启动器更新」互斥：更新包可能也要写 `runtime/**`（打包预置组件），
    // 与设置页的组件安装并发会互相踩踏同一目录。
    if !matches!(
        crate::updater::phase(),
        crate::updater::UpdatePhase::Idle
            | crate::updater::UpdatePhase::Available
            | crate::updater::UpdatePhase::Error
    ) {
        return Err(be_err("runtime_install_busy", ""));
    }
    let emitter = app.clone();
    let emit_id = id.clone();
    runtime::install::install(&id, move |received, total| {
        let _ = emitter.emit(
            "runtime-install-progress",
            json!({ "id": emit_id.as_str(), "received": received, "total": total }),
        );
    })?;
    let _ = app.emit(
        "runtime-install-progress",
        json!({ "id": id.as_str(), "received": 0, "total": 0, "done": true }),
    );
    sync_ai(&id);
    let st = runtime::status(&id)?;
    Ok(json!({ "ok": true, "status": st }))
}

/// 取消安装（置哨兵，下载循环读到即退出，临时文件会被清理）。
#[tauri::command]
pub fn runtime_cancel(id: String) -> Result<(), String> {
    runtime::install::cancel(&id);
    Ok(())
}

/// 卸载组件：删除整个组件目录（**只删程序文件，不动用户数据**）。
#[tauri::command]
pub fn runtime_uninstall(id: String) -> Result<Value, String> {
    runtime::install::uninstall(&id)?;
    sync_ai(&id);
    let st = runtime::status(&id)?;
    Ok(json!({ "ok": true, "status": st }))
}
