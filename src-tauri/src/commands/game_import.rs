//! 游戏文件导入：`import_game_files`。
//!
//! 只做 `State<AppPaths>` 解包，判定与落盘逻辑在 `vcmi::game_import`（可单测、无 Tauri）。

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde_json::Value;
use tauri::State;

use crate::infra::berror::be_err;
use crate::state::AppPaths;
use crate::vcmi::game_import::{self, Conflict, ImportEnv};

/// 导入游戏文件（地图 / 战役 / 存档 / 配置 / zip / 历代记安装器 / HD 目录 / 普通目录）。
///
/// `dryRun=true`：只预检（分类 / 识别 / 冲突），不写盘。
///
/// `gameDir` / `userDataDir` 为**可选覆盖**（首启向导用：配置还没保存，`State<AppPaths>`
/// 里没有有效路径，而 HD 素材生成 / 历代记模组生成必须按用户选的目录落盘；
/// 历代记还要从游戏目录取内置核心模组的 `portraitsChronicles.json`）。
/// 不传 = 完全沿用已保存配置（主界面「导入游戏文件」行为不变）。
///
/// ⚠️ **异步 + `spawn_blocking`**（2026-10-17 改）：HD 素材要拷几百 MB、历代记要内建解包
/// 675 MiB（实测约 57s），同步命令会把 WebView 整块冻住（向导点「安装」后看起来像卡死）。
/// 环境构造在锁内完成并克隆，重活全部扔进阻塞线程池，主线程只等结果。
#[tauri::command]
pub async fn import_game_files(
    state: State<'_, AppPaths>,
    paths: Vec<String>,
    conflict: String,
    dry_run: bool,
    overrides: Option<BTreeMap<String, String>>,
    game_dir: Option<String>,
    user_data_dir: Option<String>,
) -> Result<Value, String> {
    let conflict = Conflict::parse(&conflict)?;
    let overrides = overrides.unwrap_or_default();
    let env = {
        let guard = state.0.lock().expect("paths lock poisoned");
        match (game_dir.as_deref(), user_data_dir.as_deref()) {
            (Some(g), Some(u)) if !g.trim().is_empty() && !u.trim().is_empty() => {
                ImportEnv::for_dirs(PathBuf::from(g.trim()), PathBuf::from(u.trim()))
            }
            _ => ImportEnv::from_paths(&guard),
        }
    };

    tauri::async_runtime::spawn_blocking(move || {
        if dry_run {
            let report = game_import::preflight(&env, &paths, &overrides);
            serde_json::to_value(report)
                .map_err(|e| be_err("cmd_preflight_serialize_failed", &format!("{}", e)))
        } else {
            let report = game_import::execute(&env, &paths, conflict, &overrides);
            tracing::info!(
                target: "mods",
                "导入游戏文件：{} 项（新增 {} / 覆盖 {} / 跳过 {} / 失败 {} / 不支持 {}）",
                report.total,
                report.added,
                report.replaced,
                report.skipped,
                report.failed,
                report.unsupported
            );
            serde_json::to_value(report)
                .map_err(|e| be_err("cmd_import_serialize_failed", &format!("{}", e)))
        }
    })
    .await
    .map_err(|e| be_err("cmd_blocking_task_failed", &format!("{}", e)))?
}
