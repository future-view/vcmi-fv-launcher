//! 激活 / 停用：`dirs.json` 备份改写、modSettings 链接、`Maps` 阀门切换、停用回滚。
//!
//! 激活流程 = 备份 dirs.json → 写入领域目录映射 → 建 modSettings 链接 → （可选）封锁官方 Maps。
//! 停用流程 = 恢复 dirs.json 备份 → 解链接 → 解锁 Maps。

use crate::infra::berror::be_err;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
use super::base::{
    DIRS_BACKUP_SUFFIX, MAPS_NATIVE_SUFFIX, VCMI_GAME_PROCESS_NAMES, ensure_realm_dirs, game_dir_of,
    global_user_config_dir, guard_vcmi_not_running, persist, read_config,
};
use super::meta::{get_realm, list_realm_ids};
use super::maps::apply_maps_valve;
use super::presets::{restore_prev_active_preset, write_realm_preset};

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::State;

use crate::infra::links::{self, create_file_link};
use crate::state::AppPaths;
use crate::vcmi::config::LauncherConfig;
use crate::vcmi::paths::{normalize_abs, VcmiPaths};

/// 激活领域：改写 dirs.json 指向领域隔离目录 + 提取用户设置（§9.7.2）。
#[tauri::command]
pub fn activate_realm(realm_id: String, state: State<AppPaths>) -> Result<(), String> {
    guard_vcmi_not_running()?;

    let id = VcmiPaths::sanitize_realm_id(&realm_id);
    let realm = get_realm(&id)?; // 领域配置来自 `{realm-dir}/realm.json`
    let mut cfg = read_config().ok_or_else(|| be_err("common_not_initialized", ""))?;
    tracing::info!(target: "realm", "开始激活领域：{id}");

    let game_dir = game_dir_of(&state);
    if !game_dir.join("VCMI_client.exe").is_file() {
        return Err(be_err("realm_activate_invalid_gamedir", ""));
    }
    let base = VcmiPaths::realm_vcmi_dir(&id)
        .ok_or_else(|| be_err("common_data_dir_unknown", ""))?;
    ensure_realm_dirs(&id)?;

    // 1) 按领域 `dirs` 配置改写 dirs.json（保留原文备份，仅首次创建）
    //    **落盘必须是绝对路径**：launcher.json 允许相对路径（便携可移植），
    //    但 dirs.json 中的相对路径由 VCMI 按进程工作目录解析（`dataPaths()` 硬编码 "."），
    //    不是相对游戏目录，会随启动方式变化而失效。
    //    故只在内存中解析一份用于落盘；`cfg` 原文仍以相对形式持久化，保持可移植性。
    let dirs_config = realm.get("dirs").cloned().unwrap_or(Value::Null);
    // 克隆一份解析：resolve_relative_to 是 consuming，cfg 原文还要按相对形式持久化
    let cfg_abs = cfg.clone().resolve_relative_to(&VcmiPaths::exe_dir());
    let dirs_value = build_realm_dirs(&cfg_abs, &base, &dirs_config);
    backup_and_write_dirs(&game_dir, &dirs_value)?;

    // 2) 配置共享阀门（§配置共享）：按 dirs.configShare 决定可共享配置文件
    //    复用全局（硬链接，实时双向）/ 隔离领域（私有副本）。
    //    仅 configPath=realm 时领域配置目录才是活动配置目录，才有链接意义；全局态跳过。
    let config_path_is_realm = dirs_config
        .get("configPath")
        .and_then(|v| v.as_str())
        .unwrap_or("realm")
        != "global";
    if config_path_is_realm {
        let realm_config = base.join("config");
        let config_share = dirs_config.get("configShare").cloned().unwrap_or(Value::Null);
        ensure_config_links(&global_user_config_dir(), &realm_config, &config_share)?;
        // 领域配置目录是新建的：补播种 `launcher.setupCompleted`，否则官方启动器
        // 把每个新领域都当全新安装而弹首次运行向导（vcmiqt mainwindow_moc.cpp:83,129-134）。
        // 共享态下写入的是硬链接目标（全局文件也同步），隔离态下只写领域私有副本。
        crate::vcmi::setup::seed_setup_completed(&realm_config)?;
    }

    // 3) 地图阀门：按 `dirs.gameMaps` 决定是否屏蔽游戏自带地图（§8.3）
    let block = dirs_config
        .get("gameMaps")
        .and_then(|v| v.as_str())
        .map(|s| s == "hide")
        .unwrap_or(false);
    apply_maps_valve(&game_dir, block)?;

    // 3.5) 领域地图收纳仓库阀门（§0.4）：迁移存量 → 重建 Maps/ 下语言 junction 集合
    super::maps::rebuild_realm_maps_valve(&id)?;

    // 4) 写入领域 `__realm__` 预设（§3.3）：携带领域模组组合，并设为 activePreset
    write_realm_preset(&id)?;

    // 5) 落地领域模组组合（§3.4）：借用联接 + 整合复制。放最后，确保 realm/Mods 目录已就绪。
    {
        let mods_val = realm.get("mods").cloned().unwrap_or(Value::Null);
        let to_strs = |v: Option<&Value>| -> Vec<String> {
            v.and_then(|a| a.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect()
                })
                .unwrap_or_default()
        };
        let borrowed = to_strs(mods_val.get("borrowed"));
        let bundled = to_strs(mods_val.get("bundled"));
        // 领域私有副本台账（mod-acquisition-update 方案 §3.2/§4.4）：不参与联接/复制/清理。
        // 条目形态 `[{id, version, …}]`，也宽容纯字符串数组的手工写法。
        let pinned_arr = mods_val.get("pinned").cloned().unwrap_or(Value::Null);
        let pinned: Vec<String> = match &pinned_arr {
            Value::Array(items) => items
                .iter()
                .map(|x| match x {
                    Value::String(s) => s.clone(),
                    obj => obj.get("id").and_then(|i| i.as_str()).unwrap_or_default().to_string(),
                })
                .filter(|s| !s.is_empty())
                .collect(),
            _ => Vec::new(),
        };
        // 借用源目录同样必须是绝对路径（相对值会按进程 CWD 解析，无法稳定命中玩家模组）
        let cfg_paths = read_config()
            .ok_or_else(|| be_err("common_not_initialized", ""))?
            .resolve_relative_to(&VcmiPaths::exe_dir());
        let game_mods = cfg_paths
            .paths
            .game_mods_dir
            .clone()
            .unwrap_or_else(|| game_dir.join("Mods"));
        links::apply_realm_mods(&base, &borrowed, &bundled, &pinned, &cfg_paths.paths.user_mods_dir, &game_mods)?;
    }

    // 6) 记录激活态
    cfg.realms.active = Some(id.clone());
    persist(&cfg)?;
    tracing::info!(target: "realm", "领域激活完成：{id}");
    Ok(())
}

/// 强制结束**游戏本体**进程（§4.4「结束游戏」按钮）。best-effort，不报错。
///
/// 只杀 `VCMI_GAME_PROCESS_NAMES`（client / server / mapeditor）：
/// 官方启动器 `VCMI_launcher.exe` 与切换领域可共存（见 `vcmi_is_running`），
/// 玩家可能正拿它当模组管理器用，不该被「结束游戏」顺手关掉。
#[tauri::command]
pub fn kill_vcmi() -> Result<(), String> {
    for name in VCMI_GAME_PROCESS_NAMES {
        let mut cmd = std::process::Command::new("cmd");
        cmd.args(["/C", "taskkill", "/IM", name, "/F"]);
        #[cfg(windows)]
        cmd.creation_flags(CREATE_NO_WINDOW);
        let _ = cmd.status();
    }
    Ok(())
}

/// 停用领域：用原文备份还原 dirs.json，清除激活态（设置页回到全局 VCMI 配置）。
#[tauri::command]
pub fn deactivate_realm(state: State<AppPaths>) -> Result<(), String> {
    deactivate_realm_inner(&state)
}

pub(crate) fn deactivate_realm_inner(state: &State<AppPaths>) -> Result<(), String> {
    guard_vcmi_not_running()?;
    let mut cfg = read_config().ok_or_else(|| be_err("common_not_initialized", ""))?;
    let game_dir = game_dir_of(state);
    // 恢复进入领域前的 `activePreset`（共享配置下尤其重要：否则全局模式会停在领域预设）。
    // 失败不阻断停用——还原 dirs.json 与清除激活态才是关键。
    if let Some(id) = cfg.realms.active.clone() {
        if let Err(e) = restore_prev_active_preset(&id) {
            tracing::warn!(target: "realm", "恢复进入领域前的 activePreset 失败（可忽略）：{e}");
        }
    }
    restore_dirs(&game_dir)?;
    // ③ 还原 Maps 联接（设计 §4.2）：停用即回到「官方预设」的透明态，游戏自带地图必须重新可见。
    // **必须先判断已整理过**：`apply_maps_valve` 的首步会把真实 `Maps` 改名收纳，对从未启用过
    // 「屏蔽游戏自带地图」的用户调用它会永久改动游戏目录，直接违反 §8.3.4「懒执行」原则。
    // 失败不阻断停用——还原 dirs.json 才是关键，残余留给下次启动的自检自愈。
    let native = game_dir.join(format!("Maps.{MAPS_NATIVE_SUFFIX}"));
    if native.is_dir() {
        if let Err(e) = apply_maps_valve(&game_dir, false) {
            tracing::warn!(target: "realm", "停用领域时还原 Maps 联接失败（交由下次自检自愈）：{e}");
        }
    }
    cfg.realms.active = None;
    persist(&cfg)?;
    tracing::info!(target: "realm", "已停用领域：dirs.json 已还原、激活态已清除");
    Ok(())
}

/// 备份原文 dirs.json（仅当备份尚不存在），并写入指向领域目录的新 dirs.json。
pub(crate) fn backup_and_write_dirs(game_dir: &Path, dirs: &Value) -> Result<(), String> {
    let dirs_path = game_dir.join("config").join("dirs.json");
    let backup = game_dir
        .join("config")
        .join(format!("dirs.json.{DIRS_BACKUP_SUFFIX}"));
    // 仅首次从全局激活时保留原文备份；切换领域不覆盖备份
    if dirs_path.is_file() && !backup.is_file() {
        fs::copy(&dirs_path, &backup).map_err(|e| be_err("realm_dirs_backup_failed", &format!("{}", e)))?;
    }
    let text = serde_json::to_string_pretty(dirs)
        .map_err(|e| be_err("realm_dirs_serialize_failed", &format!("{}", e)))?;
    if let Some(parent) = dirs_path.parent() {
        fs::create_dir_all(parent).map_err(|e| be_err("realm_config_dir_create_failed", &format!("{}", e)))?;
    }
    fs::write(&dirs_path, text).map_err(|e| be_err("realm_dirs_write_failed", &format!("{}", e)))
}

/// 按领域 `dirs` 配置（§3.2.1 / B7）构建 dirs.json 的 5 个真实 key：
/// - `"realm"`：指向领域隔离目录（默认，隔离的核心）；
/// - `"global"`：显式写回全局 VCMI 用户目录（省略字段会被 userDataPath 派生，反而落在领域内，故必须显式写）。
/// `gameMaps` 字段不在此处处理，由调用方交给地图阀门。
pub(crate) fn build_realm_dirs(cfg: &LauncherConfig, realm_vcmi: &Path, dirs_config: &Value) -> Value {
    let realm = |p: PathBuf| json!(p.to_string_lossy().to_string());
    let global = |p: PathBuf| json!(p.to_string_lossy().to_string());
    let data = if dirs_config.get("dataPath").and_then(|v| v.as_str()) == Some("global") {
        global(cfg.paths.user_data_dir.clone())
    } else {
        realm(realm_vcmi.to_path_buf())
    };
    let cache = if dirs_config.get("cachePath").and_then(|v| v.as_str()) == Some("global") {
        global(cfg.paths.user_data_dir.join("cache"))
    } else {
        realm(realm_vcmi.join("cache"))
    };
    let config = if dirs_config.get("configPath").and_then(|v| v.as_str()) == Some("global") {
        global(cfg.paths.user_config_dir.clone())
    } else {
        realm(realm_vcmi.join("config"))
    };
    let logs = if dirs_config.get("logsPath").and_then(|v| v.as_str()) == Some("global") {
        global(cfg.paths.user_data_dir.join("logs"))
    } else {
        realm(realm_vcmi.join("logs"))
    };
    let save = if dirs_config.get("savePath").and_then(|v| v.as_str()) == Some("global") {
        global(cfg.paths.user_data_dir.join("Saves"))
    } else {
        realm(realm_vcmi.join("Saves"))
    };
    json!({
        "userDataPath": data,
        "userConfigPath": config,
        "userCachePath": cache,
        "userLogsPath": logs,
        "userSavePath": save,
    })
}

/// 用原文备份还原 dirs.json（设计 §3.2：必须写回备份原文，不可写默认路径）。
pub(crate) fn restore_dirs(game_dir: &Path) -> Result<(), String> {
    let dirs = game_dir.join("config").join("dirs.json");
    let backup = game_dir
        .join("config")
        .join(format!("dirs.json.{DIRS_BACKUP_SUFFIX}"));
    if backup.is_file() {
        fs::copy(&backup, &dirs).map_err(|e| be_err("realm_dirs_restore_failed", &format!("{}", e)))?;
    } else if dirs.is_file() {
        // 无备份（理论上不会发生）：保守删除当前领域化的 dirs.json，回退 VCMI 默认探测
        let _ = fs::remove_file(&dirs);
    }
    Ok(())
}

/// 配置共享阀门（§配置共享）：对 realm 配置目录里的可共享配置文件，按 `configShare`
/// （默认全部复用全局）决定「复用全局」（硬链接，实时双向）还是「隔离领域」（私有副本，
/// 起点=全局当前内容）。文件清单：settings.json / keyBindingsConfig.json /
/// persistentStorage.json / mapeditor.ini。
///
/// - 复用全局：realm 侧已存在（链接或副本）先 `remove_file` 再建链；全局源缺失则先建空壳
///   （`{}` 或空 ini），VCMI 会填充。硬链接共享 inode，VCMI 的 `trunc` 覆盖写使双向实时同步。
/// - 隔离领域：仅当该文件当前为共享链接时才转为私有副本（以标记文件判定，避免误删领域已改动副本）；
///   已是真实副本则原样保留（不丢失领域内已有改动）。
/// 安全红线：删除一律用 `fs::remove_file`，绝不 `remove_dir`/`remove_dir_all`，避免破坏全局源。
///
/// 标记文件 `.vcmi-fv-shared.json` 记录当前被共享（硬链接）的文件集合：稳定 Rust 无法用
/// `number_of_links` 判定硬链接，故用标记在多次激活间区分「共享链接」与「领域私有副本」。
pub(crate) fn ensure_config_links(
    global_config: &Path,
    realm_config: &Path,
    config_share: &Value,
) -> Result<(), String> {
    fs::create_dir_all(realm_config).map_err(|e| be_err("realm_realm_config_dir_create_failed", &format!("{}", e)))?;
    let marker_path = realm_config.join(".vcmi-fv-shared.json");
    // 当前磁盘上已被共享（硬链接）的文件集合，由标记文件记录，避免重激活时误删领域私有副本。
    let shared: std::collections::HashSet<String> = fs::read_to_string(&marker_path)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.as_object().map(|o| o.keys().cloned().collect()))
        .unwrap_or_default();

    let entries: [(&str, Option<&Value>); 4] = [
        ("settings.json", config_share.get("settings")),
        ("keyBindingsConfig.json", config_share.get("keybinds")),
        ("persistentStorage.json", config_share.get("persistent")),
        ("mapeditor.ini", config_share.get("mapeditor")),
    ];
    let mut next_shared: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (name, share_opt) in entries {
        let share = share_opt.and_then(|v| v.as_bool()).unwrap_or(true); // 默认复用全局
        let realm_file = realm_config.join(name);
        let global_file = global_config.join(name);
        if share {
            // 复用全局：确保 realm 侧是指向全局的硬链接（实时双向）
            if realm_file.exists() {
                // remove_file：硬链接只删链接项（全局安全）；副本删副本；符号链接删链接
                let _ = fs::remove_file(&realm_file);
            }
            if !global_file.exists() {
                let seed = if name.ends_with(".ini") { "" } else { "{}" };
                fs::write(&global_file, seed).map_err(|e| be_err("realm_global_init_failed", &format!("{} {}", name, e)))?;
            }
            create_file_link(&realm_file, &global_file)
                .map_err(|e| be_err("realm_share_link_failed", &format!("{} {}", name, e)))?;
            next_shared.insert(name.to_string());
        } else if shared.contains(name) {
            // 当前为共享链接 → 转为领域私有副本（起点=全局当前内容），保留玩家共享期改动
            fs::remove_file(&realm_file).map_err(|e| be_err("realm_share_link_remove_failed", &format!("{} {}", name, e)))?;
            if global_file.exists() {
                fs::copy(&global_file, &realm_file)
                    .map_err(|e| be_err("realm_isolate_failed", &format!("{} {}", name, e)))?;
            } else {
                let seed = if name.ends_with(".ini") { "" } else { "{}" };
                fs::write(&realm_file, seed).map_err(|e| be_err("realm_init_failed", &format!("{} {}", name, e)))?;
            }
        } else {
            // 已是领域私有副本：保留，不覆盖玩家改动
        }
    }
    // 回写标记，记录当前被共享（硬链接）的文件
    let mut marker_val = serde_json::Value::Object(serde_json::Map::new());
    for k in &next_shared {
        marker_val[k] = serde_json::Value::Bool(true);
    }
    fs::write(&marker_path, serde_json::to_string(&marker_val).unwrap_or_default())
        .map_err(|e| be_err("realm_shared_flag_write_failed", &format!("{}", e)))?;
    Ok(())
}

/// 读取游戏目录的 dirs.json 全文（不存在返回 None）。
pub(crate) fn read_dirs(game_dir: &Path) -> Option<Value> {
    let p = game_dir.join("config").join("dirs.json");
    if p.is_file() {
        fs::read_to_string(&p)
            .ok()
            .and_then(|t| crate::infra::json::parse_json_lenient(&t).ok())
    } else {
        None
    }
}

/// 判断 dirs.json 当前指向哪个领域（按 userDataPath 与已知领域隔离目录比对）。
pub(crate) fn dirs_realm_id(game_dir: &Path) -> Option<String> {
    let dirs = read_dirs(game_dir)?;
    let udp = dirs.get("userDataPath").and_then(|v| v.as_str())?;
    let udp = Path::new(udp).to_string_lossy();
    // 领域清单来自目录扫描（§3.1），不再依赖 launcher.json
    for id in list_realm_ids() {
        if let Some(rd) = VcmiPaths::realm_vcmi_dir(&id) {
            if udp.eq_ignore_ascii_case(&rd.to_string_lossy()) {
                return Some(id);
            }
        }
    }
    None
}

/// 判断当前 `dirs.json` 是否等于「该领域激活后应写入的内容」。
///
/// 用于**全 global 领域**（`dirs.*` 全为 `"global"`）：激活后 dirs.json 指向全局用户目录，
/// `dirs_realm_id()` 恒为 `None`，与「未激活」在文件层面无法区分，自检必须靠本函数区分，
/// 否则会把正常激活的领域误判成残留记录。
///
/// 口径与 `activate_realm` 严格一致：同一份 `read_config()` + `build_realm_dirs`，
/// 且只比对 VCMI 认的 5 个真实 key。
pub(crate) fn realm_dirs_match(game_dir: &Path, realm_id: &str) -> bool {
    let (Some(cfg), Some(dirs)) = (read_config(), read_dirs(game_dir)) else {
        return false;
    };
    // 与 `activate_realm` 完全同口径：同样解析为绝对路径后再比对，
    // 否则期望值（相对）与 dirs.json 实际值（绝对）永远对不上。
    let cfg = cfg.resolve_relative_to(&VcmiPaths::exe_dir());
    let (Ok(realm), Some(base)) = (
        get_realm(realm_id),
        VcmiPaths::realm_vcmi_dir(realm_id),
    ) else {
        return false;
    };
    let dirs_config = realm.get("dirs").cloned().unwrap_or(Value::Null);
    let expected = build_realm_dirs(&cfg, &base, &dirs_config);
    // 两侧都先规范化再比较：兼容历史遗留的 `D:\game\./userdata` 写法与 `/` `\` 分隔符差异，
    // 否则这些只是「形式不同」的旧配置会被误判成残留激活记录。
    let exe = VcmiPaths::exe_dir();
    let norm = |v: Option<&Value>| -> Option<PathBuf> {
        v.and_then(|x| x.as_str())
            .map(|s| normalize_abs(&exe, Path::new(s)))
    };
    [
        "userDataPath",
        "userCachePath",
        "userConfigPath",
        "userLogsPath",
        "userSavePath",
    ]
    .iter()
    .all(|k| match (norm(dirs.get(*k)), norm(expected.get(*k))) {
        (Some(a), Some(b)) => a == b,
        _ => false,
    })
}
