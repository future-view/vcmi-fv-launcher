//! 领域 mod 冲突检测与就绪检查：目录指纹对比、内置 mod 包冲突、必需 mod 缺失提示。

use super::meta::get_realm;
use super::transfer::collect_dir;
use crate::infra::berror::be_err;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::State;

use crate::infra::links::is_link;
use crate::infra::moddir::resolve_mod_dir;
use crate::state::AppPaths;
use crate::vcmi::paths::VcmiPaths;

/// 目录指纹：按相对路径排序，依次混入 (相对路径, 字节数, 文件内容) 后取 SHA-256。
/// 用于判定「同名模组的内容是否已经分叉」。
pub(crate) fn dir_fingerprint(dir: &Path) -> Result<String, String> {
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    collect_dir(dir, "", &mut files)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = Sha256::new();
    for (rel, abs) in &files {
        hasher.update(rel.as_bytes());
        let meta = fs::metadata(abs)
            .map_err(|e| be_err("realm_file_meta_read_failed", &format!("{}", e)))?;
        hasher.update(meta.len().to_string().as_bytes());
        let content =
            fs::read(abs).map_err(|e| be_err("infra_file_read_failed", &format!("{}", e)))?;
        hasher.update(&content);
    }
    Ok(crate::infra::hash::to_hex(&hasher.finalize()))
}

/// 整合模组的重名冲突检测（§9.3.3）：返回 `[{id, source, reason}]`。
///
/// 只在确有风险时才返回条目，避免「整合即弹窗」的噪音：
/// - `game`：来源在游戏目录（永远可见）→ 进入领域后与领域副本重复挂载同 ID；
/// - `diverged`：领域副本与来源目录指纹不一致（同名但内容不同）→ 共用百科缓存会串味。
///
/// 领域内还没有副本（尚未整合 / 只是借用联接）时不算冲突——复制过去内容必然一致。
pub(crate) fn detect_bundle_conflicts(
    user_mods: &Path,
    game_mods: &Path,
    realm_mods: &Path,
    bundled: &[String],
) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for id in bundled {
        // ⚠️ 走大小写不敏感 + 嵌套感知的解析（`id` 是 VCMI modId 小写形态，
        // 磁盘目录保留原名且子模组位于 `<根>/<父>/Mods/<子>`），不能用 `join(id)`。
        let (src_dir, source) = if let Some(u) = resolve_mod_dir(user_mods, id) {
            (u, "user")
        } else if let Some(g) = resolve_mod_dir(game_mods, id) {
            (g, "game")
        } else {
            continue; // 来源缺失：交给「引用模组缺失」校验处理
        };
        let realm_copy = realm_mods.join(id);
        if !realm_copy.is_dir() || is_link(&realm_copy) {
            continue;
        }
        let reason = if source == "game" {
            Some("game")
        } else {
            match (dir_fingerprint(&src_dir), dir_fingerprint(&realm_copy)) {
                (Ok(a), Ok(b)) if a != b => Some("diverged"),
                _ => None,
            }
        };
        if let Some(reason) = reason {
            out.push(json!({ "id": id, "source": source, "reason": reason }));
        }
    }
    out
}

/// 校验整合模组（`mods.bundled`）与已安装模组的 ID 重名是否**真的有风险**（§9.3.3）。
///
/// 只在确有必要打扰玩家时才报冲突，避免「整合即弹窗」的噪音：
/// - `game`：来源在游戏目录。游戏目录 `Mods/` 永远可见，进入领域后会与领域内副本**重复挂载**同 ID。
/// - `diverged`：领域内已有整合副本，且其目录指纹与来源**不一致**（同名但内容不同）→
///   两者共用同一份按 `modId` 键控的百科缓存，会串味。
///
/// 领域内还没有副本（尚未整合 / 只是借用联接）时不算冲突——复制过去内容必然一致。
#[tauri::command]
pub fn check_realm_mod_conflicts(
    state: State<AppPaths>,
    realm_id: String,
    bundled: Vec<String>,
) -> Value {
    let paths = state.0.lock().expect("paths lock poisoned");
    let user_mods = paths.user_mods_dir();
    let game_mods = paths.game_mods_dir();
    let base = match VcmiPaths::realm_vcmi_dir(&realm_id) {
        Some(p) => p,
        None => return json!({ "conflicts": [] }),
    };
    let realm_mods = base.join("Mods");

    let mut out: Vec<Value> = Vec::new();
    for id in bundled {
        // 同上一处：必须走大小写不敏感 + 嵌套感知的解析（不能用 `join(&id)`）
        let (src_dir, source) = if let Some(u) = resolve_mod_dir(&user_mods, &id) {
            (u, "user")
        } else if let Some(g) = resolve_mod_dir(&game_mods, &id) {
            (g, "game")
        } else {
            continue; // 来源不存在：交给 §9.2.2 的「引用模组缺失」校验处理
        };

        let realm_copy = realm_mods.join(&id);
        // 领域内没有副本（尚未整合），或该副本是借用的目录联接 → 不构成内容冲突
        if !realm_copy.is_dir() || is_link(&realm_copy) {
            continue;
        }

        let reason = if source == "game" {
            Some("game")
        } else {
            match (dir_fingerprint(&src_dir), dir_fingerprint(&realm_copy)) {
                (Ok(a), Ok(b)) if a != b => Some("diverged"),
                _ => None,
            }
        };
        if let Some(reason) = reason {
            out.push(json!({ "id": id, "source": source, "reason": reason }));
        }
    }
    json!({ "conflicts": out })
}

/// 启动游戏前的模组校验（§9.2.3）：按 `bundled` / `borrowed` **分别**校验，
/// 并附带 §9.3.3 的重名冲突，供前端按不同文案与出口提示玩家。
///
/// 返回 `{ missingBundled, missingBorrowed, conflicts }`：
/// - `missingBundled`：整合模组在来源与领域内都找不到；
/// - `missingBorrowed`：借用模组在来源目录中找不到；
/// - `conflicts`：见 `detect_bundle_conflicts`。
#[tauri::command]
pub fn check_realm_mods_ready(state: State<AppPaths>, realm_id: String) -> Value {
    let empty = || {
        json!({
            "missingBundled": Vec::<String>::new(),
            "missingBorrowed": Vec::<String>::new(),
            "conflicts": Vec::<Value>::new(),
        })
    };
    let paths = state.0.lock().expect("paths lock poisoned");
    let user_mods = paths.user_mods_dir();
    let game_mods = paths.game_mods_dir();
    // 领域配置来自目录内 realm.json（§3.1）
    let realm = match get_realm(&realm_id) {
        Ok(r) => r,
        Err(_) => return empty(),
    };
    let mods = realm.get("mods").cloned().unwrap_or(Value::Null);
    let borrowed = mods
        .get("borrowed")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let bundled = mods
        .get("bundled")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let realm_mods = VcmiPaths::realm_vcmi_dir(&realm_id).map(|p| p.join("Mods"));

    let mut missing_bundled: Vec<String> = Vec::new();
    let mut missing_borrowed: Vec<String> = Vec::new();
    let mut bundled_ids: Vec<String> = Vec::new();
    for m in borrowed.iter() {
        if let Some(id) = m.as_str() {
            // 大小写不敏感 + 嵌套感知：`id` 为小写 modId，磁盘目录保留原名
            // （直接用 `join` 会在 Linux 上把所有已装模组误报为"缺失"）
            if resolve_mod_dir(&user_mods, id).is_none()
                && resolve_mod_dir(&game_mods, id).is_none()
            {
                missing_borrowed.push(id.to_string());
            }
        }
    }
    for m in bundled.iter() {
        if let Some(id) = m.as_str() {
            bundled_ids.push(id.to_string());
            // 已整合到领域内的副本优先，不再要求来源存在。
            // 领域内副本由本启动器创建，目录名即 id，`join` 即可（无需大小写容错）。
            if let Some(rm) = &realm_mods {
                if rm.join(id).is_dir() {
                    continue;
                }
            }
            if resolve_mod_dir(&user_mods, id).is_none()
                && resolve_mod_dir(&game_mods, id).is_none()
            {
                missing_bundled.push(id.to_string());
            }
        }
    }
    let conflicts = match &realm_mods {
        Some(rm) => detect_bundle_conflicts(&user_mods, &game_mods, rm, &bundled_ids),
        None => Vec::new(),
    };
    json!({
        "missingBundled": missing_bundled,
        "missingBorrowed": missing_borrowed,
        "conflicts": conflicts,
    })
}
