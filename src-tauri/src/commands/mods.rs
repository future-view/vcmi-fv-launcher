//! 模组扫描与资源：`scan_mods` / `list_installed_mods` / `detect_edition` / `read_def_bytes`。
//!
//! 实现分别在 `vcmi::scan`（目录遍历与 submod 归并）与 `vcmi::resource`（资源定位）；
//! 本文件只做 `State<AppPaths>` 解包、缓存目录推导与错误包装。
//!
//! ⚠️ **含全量目录遍历 / 文件读取的命令必须是 `async fn` + `spawn_blocking`**：
//! Tauri 的**同步命令在主线程执行**，会冻结 WebView 消息循环 —— 表现为整个窗口无响应
//! （连点页签都不生效）。命令名 / 参数 / 出参保持不变（前端契约冻结，
//! 见 `docs/features/startup-responsiveness.zh-CN.md`）。

use serde_json::{json, Value};
use std::path::PathBuf;
use tauri::State;

use crate::infra::berror::be_err;
use crate::state::AppPaths;
use crate::vcmi::paths::VcmiPaths;
use crate::vcmi::resource::read_def_bytes_inner;
use crate::vcmi::scan::{push_installed_mod, scan_user_mods_dir};

/// 扫描真实 Mods 目录，返回顶层 mod 清单（含子 mod 树），与前端 ModInfo 对齐。
/// 同时扫描游戏目录 Mods 与用户数据目录 Mods；同名模组以用户目录为准。
#[tauri::command]
pub async fn scan_mods(state: State<'_, AppPaths>) -> Result<Value, String> {
    // ⚠️ `MutexGuard` 非 Send：先取路径快照，**不得跨 `.await` 持锁**。
    let (game_mods_dir, user_mods_dir) = {
        let paths = state.0.lock().expect("paths lock poisoned");
        (paths.game_mods_dir(), paths.user_mods_dir())
    };
    tauri::async_runtime::spawn_blocking(move || scan_mods_blocking(game_mods_dir, user_mods_dir))
        .await
        .map_err(|e| be_err("cmd_blocking_task_failed", &format!("{}", e)))?
}

/// `scan_mods` 的同步实现（在阻塞线程池执行）。
fn scan_mods_blocking(game_mods_dir: PathBuf, user_mods_dir: PathBuf) -> Result<Value, String> {
    fn collect_ids(mods: &[Value]) -> Vec<String> {
        mods.iter()
            .map(|m| {
                m.get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string()
            })
            .collect()
    }

    let game_mods = scan_user_mods_dir(&game_mods_dir);
    let user_mods = scan_user_mods_dir(&user_mods_dir);

    if game_mods.is_empty() {
        return Ok(json!(user_mods));
    }
    if user_mods.is_empty() {
        return Ok(json!(game_mods));
    }

    let user_ids: std::collections::HashSet<String> = collect_ids(&user_mods).into_iter().collect();
    let mut merged = user_mods;
    for m in game_mods {
        let id = m
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if !id.is_empty() && !user_ids.contains(&id) {
            merged.push(m);
        }
    }
    Ok(json!(merged))
}

/// 列出当前「已安装」的模组（供领域借用 / 整合选择，设计 §3.4 / §9.6.1）。
/// 扫描全局用户 Mods 与游戏目录 Mods（与 `scan_mods` 同来源），返回
/// `{id, name, source, type, parentId, isSubmod, factions}`，且不受领域激活影响
/// ——始终基于启动器持久化的全局路径（离线扫描，见 §3.5）。
///
/// ⚠️ `async` + `spawn_blocking`：两次全量目录扫描 + 逐 mod 读缓存，重量同 `scan_mods`。
#[tauri::command]
pub async fn list_installed_mods(state: State<'_, AppPaths>) -> Result<Value, String> {
    let (game_mods_dir, user_mods_dir) = {
        let paths = state.0.lock().expect("paths lock poisoned");
        (paths.game_mods_dir(), paths.user_mods_dir())
    };
    tauri::async_runtime::spawn_blocking(move || {
        list_installed_mods_blocking(game_mods_dir, user_mods_dir)
    })
    .await
    .map_err(|e| be_err("cmd_blocking_task_failed", &format!("{}", e)))?
}

/// `list_installed_mods` 的同步实现（在阻塞线程池执行）。
fn list_installed_mods_blocking(
    game_mods_dir: PathBuf,
    user_mods_dir: PathBuf,
) -> Result<Value, String> {
    let game_mods = scan_user_mods_dir(&game_mods_dir);
    let user_mods = scan_user_mods_dir(&user_mods_dir);

    let mut out: Vec<Value> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    // 用户目录优先（同 id 以用户目录为准）。
    // `source` 区分来源目录（§9.6.1）："user"=玩家安装（全局用户目录），"game"=游戏自带（游戏目录 Mods，永远可见）。
    let tagged = user_mods
        .into_iter()
        .map(|m| (m, "user"))
        .chain(game_mods.into_iter().map(|m| (m, "game")));
    for (m, source) in tagged {
        push_installed_mod(&m, source, &mut seen, &mut out);
    }
    // 用已构建缓存（`cache.json` 的 `modMeta`）补充**多语言名/描述**：
    // 此前 mods / realms 页面只能拿到 VCMI `mod.json` 的英文原文（见 InstalledMod.desc 注释），
    // 现在附 `meta`（多语言 name/description + author/version/modType/depends），
    // 前端可按界面语言显示；缓存未构建时该项缺失（不影响既有字段与行为）。
    let cache_root = VcmiPaths::mods_cache_dir()
        .unwrap_or_else(|| PathBuf::from(".vcmi-fv-launcher").join("mods"));
    for item in out.iter_mut() {
        let id = item
            .get("id")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if let Some(meta) = crate::modcache::cached_mod_meta(&cache_root, &id) {
            if let Some(o) = item.as_object_mut() {
                o.insert("meta".to_string(), meta);
            }
        }
    }
    Ok(json!({ "mods": out }))
}

/// 检测版本特征：Mods 下是否含 h3e-main-menu（增强版）
#[tauri::command]
pub fn detect_edition(state: State<AppPaths>) -> String {
    let paths = state.0.lock().expect("paths lock poisoned");
    for dir in [paths.user_mods_dir(), paths.game_mods_dir()] {
        if dir.is_dir() && dir.join("h3e-main-menu").is_dir() {
            return "enhanced".into();
        }
    }
    "official".into()
}

/// 读取 `.def` 资源的原始字节（前端详情页按需解码播放动画；见 `vcmi::resource::read_def_bytes_inner`）。
#[tauri::command]
pub async fn read_def_bytes(mod_id: String, resource: String) -> Result<Vec<u8>, String> {
    // `VcmiPaths::detect()` 每次都重新读配置 / 探测目录，DEF 解码又是 CPU 活 —— 一律下沉阻塞线程池。
    tauri::async_runtime::spawn_blocking(move || {
        let paths = VcmiPaths::detect();
        read_def_bytes_inner(&paths, &mod_id, &resource)
    })
    .await
    .map_err(|e| be_err("cmd_blocking_task_failed", &format!("{}", e)))?
}
