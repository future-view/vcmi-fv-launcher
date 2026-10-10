//! 领域系统共享底座：VCMI 进程守卫、启动器配置读写、realm 根路径与锁、日期工具。
//!
//! 本模块只放「被多个子模块复用」的低层能力，不含业务命令（唯一例外是
//! `is_vcmi_running`——它是对 `vcmi_is_running` 的薄封装，前端进程守卫要用）。

use crate::infra::berror::be_err;
use std::fs;
use std::path::PathBuf;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

use crate::vcmi::config::LauncherConfig;
use crate::vcmi::paths::VcmiPaths;

use tauri::State;

use crate::state::AppPaths;

/// dirs.json 原文备份后缀（仅首次从全局激活时创建，用于停用时还原）。
pub(crate) const DIRS_BACKUP_SUFFIX: &str = "vcmi-fv-backup";

/// 游戏自带 Maps 改名后的真实目录后缀（§8.3）。
pub(crate) const MAPS_NATIVE_SUFFIX: &str = "vcmi-fv-native";

/// VCMI 相关进程名（进程守卫 §4.4）。仅用于**枚举展示**（`vcmi_running_processes`）。
pub(crate) const VCMI_PROCESS_NAMES: &[&str] = &[
    "VCMI_client.exe",
    "VCMI_server.exe",
    "VCMI_mapeditor.exe",
    "VCMI_launcher.exe",
];

/// 游戏本体进程名（进程守卫 §4.4 的**阻塞口径**，见 `vcmi_is_running`）。
///
/// 只列会持有存档 / 游戏状态、改动 `dirs.json` 会让它写错位置的进程；
/// 官方启动器 `VCMI_launcher.exe` 不在此列（可共存，理由见 `vcmi_is_running`）。
pub(crate) const VCMI_GAME_PROCESS_NAMES: &[&str] = &[
    "VCMI_client.exe",
    "VCMI_server.exe",
    "VCMI_mapeditor.exe",
];

/// 当前进程自身的 exe 文件名（小写）。用于**把自己排除在进程守卫之外**。
///
/// 为什么必须排除：启动器常被放进游戏目录、甚至改名为 `VCMI_launcher.exe` 以替换官方启动器，
/// 此时旧的"tasklist 全文 contains"会**把启动器自己判成游戏在运行** →
/// 每次「进入领域」都弹「VCMI 正在运行」，无法进入。
#[cfg(windows)]
fn self_exe_name() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_lowercase()))
        .unwrap_or_default()
}

/// 进程守卫（§4.4）：枚举正在运行的 **VCMI 进程名**（空 vec = 没有在跑）。
///
/// 判定口径（两处都对旧实现做了修正）：
/// ① **精确**比配 `tasklist` CSV 的第一列（image name），忽略大小写
///    —— 旧实现是全文 `contains`，任何名字里含 `VCMI_client.exe` 的进程都会误命中；
/// ② 排除**启动器自身**（见 `self_exe_name`）。
///
/// 返回值直接给前端用：只说"VCMI 正在运行"用户无从判断，报出进程名才能自救。
#[cfg(windows)]
pub(crate) fn vcmi_running_processes() -> Vec<String> {
    let Ok(o) = std::process::Command::new("cmd")
        .args(["/C", "tasklist", "/FO", "CSV", "/NH"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&o.stdout);
    let me = self_exe_name();
    let mut hit: Vec<String> = Vec::new();
    // 每行形如 `"VCMI_client.exe","1234","Console","1","123,456 K"` → 只取第一列
    for line in text.lines() {
        let name = match line.split(',').next() {
            Some(n) => n.trim().trim_matches('"').trim(),
            None => continue,
        };
        if name.is_empty() || name.eq_ignore_ascii_case(&me) {
            continue;
        }
        if VCMI_PROCESS_NAMES
            .iter()
            .any(|n| name.eq_ignore_ascii_case(n))
        {
            hit.push(name.to_string());
        }
    }
    hit
}

/// 进程守卫（§4.4）：运行中的**游戏本体**进程（`VCMI_GAME_PROCESS_NAMES` 命中项）。
#[cfg(windows)]
pub(crate) fn vcmi_game_processes() -> Vec<String> {
    vcmi_running_processes()
        .into_iter()
        .filter(|n| {
            VCMI_GAME_PROCESS_NAMES
                .iter()
                .any(|g| n.eq_ignore_ascii_case(g))
        })
        .collect()
}

/// 进程守卫（§4.4）：非 Windows 平台恒为空。
#[cfg(not(windows))]
pub(crate) fn vcmi_game_processes() -> Vec<String> {
    Vec::new()
}

/// 进程守卫（§4.4）：**游戏本体**是否在运行。改写 `dirs.json` 前必须未运行，
/// 否则已启动的进程会继续读写旧目录（存档 / 设置写错位置）。
///
/// 为什么**不含** `VCMI_launcher.exe`（官方启动器，可共存）：
/// 1. 它不持有存档 / 游戏状态，改写 `dirs.json` 不会让它把游戏数据写错位置；
/// 2. VCMI 的路径是**进程启动时快照**（`VCMIDirsWIN32` 构造时读一次 `dirs.json`，
///    之后不再重读），切换领域对它**已打开**的数据没有影响；只是它之后的改动会落到
///    它启动时解析到的目录（可能是切换前的目录）——属"改动不生效"，不是数据损坏；
/// 3. 领域下的 `settings.json` 默认与全局**硬链接共享**，官方启动器改它两边同步；
///    唯一会被它整体回写的 `modSettings.json`，只在玩家主动保存模组组合时才写。
///
/// 因此「只开着官方启动器」不阻塞切换领域（§4.4 分级守卫）。
#[cfg(windows)]
pub(crate) fn vcmi_is_running() -> bool {
    !vcmi_game_processes().is_empty()
}

/// 进程守卫（§4.4）：枚举正在运行的 VCMI 进程（非 Windows 平台恒为空）。
#[cfg(not(windows))]
pub(crate) fn vcmi_running_processes() -> Vec<String> {
    Vec::new()
}

/// 进程守卫（§4.4）：非 Windows 平台恒为 false。
#[cfg(not(windows))]
pub(crate) fn vcmi_is_running() -> bool {
    false
}

/// 进程守卫的阻塞判定：游戏本体在运行则返回 Err，调用方应中止操作并提示用户先退出游戏。
/// 官方启动器不在此判定内（见 `vcmi_is_running`）。
/// 文案保持中性（切换领域 / 安装更新 / 导入删除等多入口共用；安装路径的专用提示在前端预检）。
pub(crate) fn guard_vcmi_not_running() -> Result<(), String> {
    if vcmi_is_running() {
        return Err(be_err("realm_guard_vcmi_running", ""));
    }
    Ok(())
}

/// 进程守卫（§4.4）：游戏本体是否正在运行（不含官方启动器）。
/// 供前端在启动游戏后淡出领域覆盖层，并在游戏退出后自动淡回（§9.2.3）。
#[tauri::command]
pub fn is_vcmi_running() -> bool {
    vcmi_is_running()
}

/// 进程守卫（§4.4）：列出正在运行的 VCMI 进程名（空数组 = 未运行）。
/// 前端用它把"到底哪个进程在挡路"直接展示给用户（只回 bool 时用户无从判断）。
/// 含官方启动器——用于**提示**，不用于阻塞（阻塞口径见 `list_vcmi_game_processes`）。
#[tauri::command]
pub fn list_vcmi_processes() -> Vec<String> {
    vcmi_running_processes()
}

/// 进程守卫（§4.4）：列出正在运行的**游戏本体**进程（空数组 = 不挡路）。
///
/// 与 `list_vcmi_processes` 的区别：后者**含** `VCMI_launcher.exe`（仅作提示），
/// 本命令只回会真正阻塞切换的那部分（client / server / mapeditor），
/// 前端据此决定"要不要弹阻断框"。
#[tauri::command]
pub fn list_vcmi_game_processes() -> Vec<String> {
    vcmi_game_processes()
}

pub(crate) fn read_config() -> Option<LauncherConfig> {
    VcmiPaths::read_launcher_config()
}

pub(crate) fn persist(cfg: &LauncherConfig) -> Result<(), String> {
    VcmiPaths::write_launcher_config(cfg)
}

pub(crate) fn game_dir_of(state: &State<AppPaths>) -> PathBuf {
    {
        let p = state.inner().0.lock().expect("paths lock poisoned");
        if !p.game_dir.as_os_str().is_empty() {
            return p.game_dir.clone();
        }
    }
    read_config().map(|c| c.paths.game_dir).unwrap_or_default()
}

/// 返回全局用户配置目录（settings.json 所在），用于「提取用户设置」。
/// 始终来自已持久化的 launcher.json，不受运行时 state 影响（state 保持全局，不被领域改写）。
/// **必须解析为绝对路径**：launcher.json 允许写 `/userdata` 这类无盘符盘根路径，
/// 不解析直接传给 mklink 会被 `cmd` 当成命令开关（报 `无效开关 - "userdata"`）。
/// 此处按启动器便携模型以 exe 所在盘补全盘符（与 `normalize_abs` 同口径）。
pub(crate) fn global_user_config_dir() -> PathBuf {
    read_config()
        .map(|c| c.resolve_relative_to(&VcmiPaths::exe_dir()).paths.user_config_dir)
        .unwrap_or_default()
}

/// 领域激活时自动应用的模组预设 id（设计 §3.3：固定 `__realm__`）。
pub(crate) const REALM_PRESET_ID: &str = "__realm__";

/// 领域列表根目录：`{realm-data}/realms`
pub(crate) fn realms_root() -> Option<PathBuf> {
    VcmiPaths::launcher_data_dir().map(|d| d.join("realms"))
}

/// 领域根目录（Option 版，供内部判定；命令侧仍用 `realm_root` 的 Result 版）。
pub(crate) fn realm_root_of(id: &str) -> Option<PathBuf> {
    let id = VcmiPaths::sanitize_realm_id(id);
    VcmiPaths::realm_vcmi_dir(&id).and_then(|p| p.parent().map(|pp| pp.to_path_buf()))
}

/// 领域元数据文件路径：`{realm-dir}/realm.json`
pub(crate) fn realm_meta_path(id: &str) -> Option<PathBuf> {
    realm_root_of(id).map(|r| r.join("realm.json"))
}

/// 当前 UTC 日期串（`YYYY-MM-DD`），用于 `createdAt` / `updateAt`（设计 §6）。
pub fn now_date() -> String {
    crate::infra::time::today_utc()
}

/// 确保领域隔离目录的 5 个真实 key 子目录存在（VCMI 只自动建 cache/config/logs/Saves，不建 Mods）。
pub(crate) fn ensure_realm_dirs(id: &str) -> Result<(), String> {
    let base = VcmiPaths::realm_vcmi_dir(id)
        .ok_or_else(|| be_err("common_data_dir_unknown", ""))?;
    for d in VcmiPaths::realm_vcmi_subdirs(&base) {
        fs::create_dir_all(&d).map_err(|e| be_err("realm_dir_create_failed", &format!("{}", e)))?;
    }
    Ok(())
}

/// 领域编辑权标记路径（§6.2）：`{realm-dir}/lock`
pub(crate) fn realm_lock_path(id: &str) -> Option<PathBuf> {
    VcmiPaths::realm_vcmi_dir(id).and_then(|base| base.parent().map(|p| p.join("lock")))
}

/// 写入编辑权标记（§6.2）。导出时 lock 已被 `collect_realm_files` 排除（§7.1 不打包 lock）。
pub(crate) fn write_realm_lock(id: &str) -> Result<(), String> {
    let p = realm_lock_path(id).ok_or_else(|| be_err("common_data_dir_unknown", ""))?;
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).map_err(|e| be_err("realm_dir_create_failed", &format!("{}", e)))?;
    }
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    fs::write(&p, ts.to_string()).map_err(|e| be_err("realm_edit_lock_write_failed", &format!("{}", e)))
}

/// 点文件名：进入领域前的 `activePreset` 快照。与 `.bgmMuted` / `.introPlayedAt` 同构，
/// 属用户态、`collect_realm_files` 已排除，不随领域导出走。
pub(crate) const PREV_ACTIVE_PRESET_FILE: &str = ".prevActivePreset";

/// 领域目录（内部用）：`{realm-data}/realms/<id>`
pub(crate) fn realm_root(realm_id: &str) -> Result<PathBuf, String> {
    let id = VcmiPaths::sanitize_realm_id(realm_id);
    let base =
        VcmiPaths::realm_vcmi_dir(&id).ok_or_else(|| be_err("common_data_dir_unknown", ""))?;
    base.parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| be_err("common_data_dir_unknown", ""))
}
