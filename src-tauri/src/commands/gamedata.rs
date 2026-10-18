//! 游戏数据完整性体检：`gamedata_health_check`。
//!
//! 只做 `State<AppPaths>` 解包，判定逻辑在 `vcmi::health` / `vcmi::h3data`
//! （可单测、无 Tauri 依赖）。

use serde_json::{json, Value};
use tauri::State;

use crate::state::AppPaths;

/// 游戏数据完整性体检（只读）：
/// 返回 `{ ok, roots, items: [{ id, level, ok, detail }], h3Data, dataInstallCandidates }`。
///
/// `id` 供前端 i18n 取文案（`data.health.items.<id>`），`level` ∈ `fatal|warn`。
///
/// `game_dir` / `user_data_dir` 为**可选覆盖**（首启向导用：目录还没保存进 launcher.json，
/// `State<AppPaths>` 里没有有效路径）。不传 = 完全沿用已保存配置（主界面「游戏数据」抽屉
/// 的行为与扩展前逐字节一致）。
///
/// - `h3Data`：原版数据闸门（官方口径，见 `vcmi::h3data`），向导据此禁用保存按钮；
/// - `dataInstallCandidates`：注册表里发现的本机原版安装路径（Windows；用于预填数据目录）。
///   仅在 `with_candidates = Some(true)` 时查询 —— `reg query` ×4 约百毫秒，
///   常态状态探测不必付这个开销（确认缺失时前端会补一次带候选的调用）。
///
/// ⚠️ 本命令**保持同步**：LOD 只读文件头（几百 KB），非整个容器；注册表查询为毫秒级。
/// 若未来要搬更重的活，改异步 + `spawn_blocking`，别在主线程上堆 IO。
#[tauri::command]
pub fn gamedata_health_check(
    state: State<'_, AppPaths>,
    game_dir: Option<String>,
    user_data_dir: Option<String>,
    with_candidates: Option<bool>,
) -> Value {
    let has_override = game_dir.is_some() || user_data_dir.is_some();
    let want_candidates = with_candidates.unwrap_or(true);
    let guard = state.0.lock().expect("paths lock poisoned");

    // 无覆盖：走既有路径（VcmiPaths 自带的资源根推导）
    if !has_override {
        let mut out = crate::vcmi::health::health_check(&guard);
        append_h3(&mut out, &guard.game_dir, &guard.user_dir, want_candidates);
        return out;
    }

    let game = game_dir
        .map(std::path::PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| guard.game_dir.clone());
    let user = user_data_dir
        .map(std::path::PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| guard.user_dir.clone());

    // 资源根：用户数据目录优先（与 VCMI 双根挂载一致；存在性判定与顺序无关）
    let mut roots: Vec<std::path::PathBuf> = Vec::new();
    for p in [&user, &game] {
        if !p.as_os_str().is_empty() && !roots.iter().any(|x| x == p) {
            roots.push(p.clone());
        }
    }
    let mut out = crate::vcmi::health::health_check_with_roots(&roots);
    append_h3(&mut out, &game, &user, want_candidates);
    out
}

/// 附上原版数据闸门与注册表安装候选（两个分支共用）。
fn append_h3(
    out: &mut Value,
    game: &std::path::Path,
    user: &std::path::Path,
    want_candidates: bool,
) {
    let h3 = crate::vcmi::h3data::detect(game, user);
    // 诊断日志：闸门不放过任何"数据其实在但被判缺"的情形（用户报障时第一手证据）
    tracing::info!(
        target: "paths",
        "原版数据探测：found={} roe={} sod={} demo={} missing={:?} game={} user={}",
        h3.found,
        h3.roe,
        h3.sod,
        h3.demo,
        h3.missing,
        game.display(),
        user.display()
    );
    if let Some(obj) = out.as_object_mut() {
        obj.insert("h3Data".into(), json!(h3));
        let candidates: Vec<String> = if want_candidates {
            crate::vcmi::h3data::locate_installs()
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect()
        } else {
            Vec::new()
        };
        obj.insert("dataInstallCandidates".into(), json!(candidates));
    }
}
