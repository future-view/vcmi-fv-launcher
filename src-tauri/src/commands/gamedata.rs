//! 游戏数据完整性体检：`gamedata_health_check`。
//!
//! 只做 `State<AppPaths>` 解包，判定逻辑在 `vcmi::health`（可单测、无 Tauri 依赖）。

use serde_json::Value;
use tauri::State;

use crate::state::AppPaths;

/// 游戏数据完整性体检（只读）：
/// 返回 `{ ok, roots, items: [{ id, level, ok, detail }] }`。
///
/// `id` 供前端 i18n 取文案（`data.health.items.<id>`），`level` ∈ `fatal|warn`。
#[tauri::command]
pub fn gamedata_health_check(state: State<AppPaths>) -> Value {
    let paths = state.0.lock().expect("paths lock poisoned");
    crate::vcmi::health::health_check(&paths)
}
