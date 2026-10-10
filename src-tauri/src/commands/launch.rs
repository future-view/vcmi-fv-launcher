//! 启动游戏 / 地图编辑器 / 官方启动器：`launch_game` / `launch_editor` / `launch_native_launcher`。
//!
//! ⚠️ **必须显式设置工作目录为游戏目录**（`current_dir`）：VCMI 相对路径按**进程工作目录**解析
//! （`lib/filesystem/CFilesystemLoader.cpp` 的 `dataPaths()` 硬编码返回 `"."`），
//! 从启动器 spawn 时继承的是启动器自身的工作目录（可能是任意位置），
//! 一旦 `dirs.json` 里写了相对路径就会解析到错误位置。

use crate::infra::berror::be_err;
use std::path::Path;
use std::process::Command;
use tauri::State;

use crate::state::AppPaths;

/// 统一的 spawn 入口：显式 `current_dir(game_dir)`，失败返回中文错误。
fn spawn_in_game_dir(exe: &Path, game_dir: &Path, label: &str) -> Result<(), String> {
    if !exe.exists() {
        return Err(be_err("vcmi_exe_missing", &format!("{}", exe.display())));
    }
    Command::new(exe)
        .current_dir(game_dir)
        .spawn()
        .map_err(|e| be_err("cmd_generic_two_arg", &format!("{} {}", label, e)))?;
    Ok(())
}

/// 启动游戏：spawn VCMI_client.exe
#[tauri::command]
pub fn launch_game(state: State<AppPaths>) -> Result<(), String> {
    let paths = state.0.lock().expect("paths lock poisoned");
    spawn_in_game_dir(
        &paths.game_dir.join("VCMI_client.exe"),
        &paths.game_dir,
        "启动",
    )
}

/// 启动地图编辑器：spawn VCMI_mapeditor.exe
#[tauri::command]
pub fn launch_editor(state: State<AppPaths>) -> Result<(), String> {
    let paths = state.0.lock().expect("paths lock poisoned");
    spawn_in_game_dir(
        &paths.game_dir.join("VCMI_mapeditor.exe"),
        &paths.game_dir,
        "启动",
    )
}

/// 启动 VCMI 自带 launcher：spawn VCMI_launcher.exe
#[tauri::command]
pub fn launch_native_launcher(state: State<AppPaths>) -> Result<(), String> {
    let paths = state.0.lock().expect("paths lock poisoned");
    spawn_in_game_dir(
        &paths.game_dir.join("VCMI_launcher.exe"),
        &paths.game_dir,
        "启动原始启动器",
    )
}
