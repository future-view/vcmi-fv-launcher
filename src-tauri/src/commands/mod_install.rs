//! 本地 zip 安装模组：`preview_mod_zip` / `install_local_mod`。
//!
//! 判定与解压在 `vcmi::mod_install`（可单测、无 Tauri 依赖）；本层只做 `State<AppPaths>` 解包。
//!
//! **安装位置固定为 `userModsDir`**（玩家安装目录，与官方 `CLauncherDirs::modsPath` 一致）：
//! 游戏目录的 `Mods/` 是发行包自带内容，安装流程不应写入那里。

use std::path::Path;

use serde_json::{json, Value};
use tauri::State;

use crate::state::AppPaths;
use crate::vcmi::mod_install;

/// 预览 zip 安装：识别 mod 根、读取元数据、推导 mod id 与目标目录（**不写盘**）。
///
/// 前端据此弹确认框（展示 id / 名称 / 版本 / 目标路径 / 是否已存在）后再调 `install_local_mod`。
#[tauri::command]
pub fn preview_mod_zip(state: State<AppPaths>, zip_path: String) -> Result<Value, String> {
    let mods_dir = {
        let paths = state.0.lock().expect("paths lock poisoned");
        paths.user_mods_dir()
    };
    let info = mod_install::inspect(Path::new(&zip_path), &mods_dir)?;
    Ok(json!({
        "modId": info.mod_id,
        "name": info.name,
        "version": info.version,
        "modType": info.mod_type,
        "entryCount": info.entry_count,
        "target": info.target.to_string_lossy(),
        "exists": info.exists,
    }))
}

/// 安装本地 zip 到 `userModsDir/<modId>`。
///
/// `overwrite=false` 且目标已存在 → 返回中文错误（前端提示用户后改传 `true`）。
/// 安装失败不会留下半截目录，也不会破坏既有安装（见 `vcmi::mod_install` 模块头）。
#[tauri::command]
pub fn install_local_mod(
    state: State<AppPaths>,
    zip_path: String,
    overwrite: bool,
) -> Result<Value, String> {
    let mods_dir = {
        let paths = state.0.lock().expect("paths lock poisoned");
        paths.user_mods_dir()
    };
    let out = mod_install::install(Path::new(&zip_path), &mods_dir, overwrite)?;
    // 新模组落地 → 立刻作废模组表 / 扫描缓存 / 跨模组正负结果，
    // 否则本次运行期间资源定位仍按旧表走（新 mod 的素材找不到）。
    crate::vcmi::scan::invalidate_mod_paths();
    tracing::info!(
        target: "mods",
        "安装本地模组 {} → {}（{} 个文件，覆盖={}）",
        out.mod_id,
        out.dest.display(),
        out.installed_files,
        out.replaced
    );
    Ok(json!({
        "modId": out.mod_id,
        "dest": out.dest.to_string_lossy(),
        "installedFiles": out.installed_files,
        "replaced": out.replaced,
    }))
}
