//! 目录识别与建议：`detect_vcmi_paths` / `get_dirs` / `suggest_user_data_dir`。
//!
//! ⚠️ `get_dirs` 出参 6 个 key（`exeDir` / `gameDir` / `userDataDir` / `userConfigDir` /
//! `userModsDir` / `logsDir`）为前端契约（见设计文档 §7.1）。

use crate::infra::berror::be_err;
use std::path::PathBuf;

use serde_json::{json, Value};
use tauri::State;

use crate::state::AppPaths;
use crate::vcmi::paths::VcmiPaths;

/// 探测当前可用的 VCMI 目录组合，**并完成目录初始化**。
///
/// `VcmiPaths::detect()` 会读取 `launcher.json`（无论来自初始化向导保存还是便携包预置），
/// 按其 `game_dir` / `user_data_dir` 解析出全部路径，并执行目录初始化：
/// 创建 `<user_data_dir>/{cache,config,logs,Saves,Mods}`，
/// 以及（`redirect_user_data` 且无激活领域时）接管 `game_dir/config/dirs.json`。
///
/// 结果**必须回写受管 `AppPaths`**：否则后续 `read_mod_settings` / `activate_realm` /
/// `patch_preset_lang_mods` 等使用 `State<AppPaths>` 的命令仍看不到刚初始化的路径。
///
/// ⚠️ `async` + `spawn_blocking`：`detect()` 含配置读取 / 目录探测 / 目录初始化（建目录、
/// 可能改写 `dirs.json`），是启动路径上的重量级 IO，不得占用主线程。
///
/// 出参另含首启向导「自定义用户数据目录」所需的四个字段（**只读，无副作用**）：
/// - `portableUserDataDir` / `portableUserDataDirExists`：启动器专属目录（exe 同目录 `userdata`）；
/// - `systemUserDataDir` / `systemUserDataDirExists`：VCMI 官方默认目录（`文档\My Games\vcmi`），
///   同时是「首次初始化时复制既有数据」的**源目录**。
#[tauri::command]
pub async fn detect_vcmi_paths(state: State<'_, AppPaths>) -> Result<Value, String> {
    let paths = tauri::async_runtime::spawn_blocking(VcmiPaths::detect)
        .await
        .map_err(|e| be_err("cmd_blocking_task_failed", &format!("{}", e)))?;
    let portable = VcmiPaths::portable_user_data_dir();
    let portable_exists = portable.is_dir();
    let system = crate::vcmi::paths::default_user_dir();
    let system_exists = system.is_dir();
    let out = json!({
        "detected": paths.detected,
        "ready": paths.is_ready(),
        "exeDir": paths.exe_dir,
        "gameDir": paths.game_dir,
        "userDir": paths.user_dir,
        "userConfigDir": paths.user_config_dir,
        "logsDir": paths.logs_dir,
        "savesDir": paths.saves_dir,
        "userModsDir": paths.user_mods_dir(),
        "gameModsDir": paths.game_mods_dir(),
        "fromConfig": paths.from_config,
        // 首启向导：解析出的用户目录是否存在（dirs.json 指向了别的目录时，它才是"既有数据"的复制源）
        "userDirExists": paths.user_dir.is_dir(),
        // 首启向导：自定义用户数据目录的两个候选及其存在性（复制源 / 默认目标）
        "portableUserDataDir": portable,
        "portableUserDataDirExists": portable_exists,
        "systemUserDataDir": system,
        "systemUserDataDirExists": system_exists,
    });
    {
        let mut guard = state.0.lock().expect("paths lock poisoned");
        *guard = paths;
    }
    Ok(out)
}

/// 返回识别出的目录信息（游戏数据抽屉用）
#[tauri::command]
pub fn get_dirs(state: State<AppPaths>) -> Value {
    let paths = state.0.lock().expect("paths lock poisoned");
    serde_json::json!({
        "exeDir": paths.exe_dir,
        "gameDir": paths.game_dir,
        "userDir": paths.user_dir,
        "userConfigDir": paths.user_config_dir,
        "logsDir": paths.logs_dir,
        "savesDir": paths.saves_dir,
        "userModsDir": paths.user_mods_dir(),
        "gameModsDir": paths.game_mods_dir(),
        "ready": paths.is_ready(),
    })
}

/// 根据游戏目录推导用户数据目录（dirs.json 优先，否则默认 My Games/vcmi）。
/// 供初始化页在用户选择游戏目录后自动填充。
#[tauri::command]
pub fn suggest_user_data_dir(game_dir: String) -> Result<Value, String> {
    let gd = PathBuf::from(game_dir);
    if !gd.join("VCMI_client.exe").is_file() {
        return Err(be_err("settings_gamedir_no_exe", ""));
    }
    let user_dir = VcmiPaths::resolve_user_dir(&gd);
    Ok(json!({
        "gameDir": gd,
        "userDataDir": user_dir,
        "userConfigDir": user_dir.join("config"),
        "userModsDir": user_dir.join("Mods"),
        "exists": user_dir.is_dir(),
    }))
}
