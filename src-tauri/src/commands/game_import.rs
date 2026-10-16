//! 「导入游戏文件」命令：`import_game_files`。
//!
//! 分类 / 预检 / 执行 / HD 素材生成都在 `vcmi::game_import`（可单测、无 Tauri 依赖）；
//! 本层只做 `State<AppPaths>` 解包与参数解析。
//!
//! `dryRun=true` → 预检报告（不写盘）；`false` → 执行报告（含 `modsInstalled`，
//! 前端据此重扫 + 自动启用 + 加入当前预设）。

use crate::infra::berror::be_err;
use std::collections::BTreeMap;

use serde_json::Value;
use tauri::State;

use crate::state::AppPaths;
use crate::vcmi::game_import::{self, Conflict, ImportEnv};

#[tauri::command]
pub fn import_game_files(
    state: State<AppPaths>,
    paths: Vec<String>,
    conflict: String,
    dry_run: bool,
    overrides: Option<BTreeMap<String, String>>,
) -> Result<Value, String> {
    let env = {
        let p = state.0.lock().expect("paths lock poisoned");
        ImportEnv::from_paths(&p)
    };
    let conflict = Conflict::parse(&conflict)?;
    let overrides = overrides.unwrap_or_default();
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
}
