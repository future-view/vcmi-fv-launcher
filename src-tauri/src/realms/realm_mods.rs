//! 领域沙箱内的模组操作（mod-acquisition-update 方案 §4.4 / §9.2）。
//!
//! 仅 RealmEditor（维护者入口，C4 决策）调用：
//! - `unlink`：删除 `{realm}/vcmi/Mods/<id>` 的**私有副本**（真实目录；联接也一并删除），
//!   并返回全局是否存在来源目录——前端据此决定能否接着「恢复为全局版本」。
//! - `link`：把全局来源重新联接进领域目录（幂等，来源缺失时跳过不报错）。
//!
//! `pinned` 台账（realm.json 的 `mods.pinned`）由领域保存流（`update_realm`）维护，
//! 本模块不碰 realm.json：操作成功后前端更新状态并随保存落盘。

use crate::infra::berror::be_err;
use serde_json::{json, Value};
use std::fs;
use tauri::State;

use super::base::guard_vcmi_not_running;

use crate::infra::links::{self, is_link, remove_link};
use crate::state::AppPaths;
use crate::vcmi::paths::VcmiPaths;

/// 领域私有副本操作：`unlink`（删私有副本/联接）| `link`（重建借用联接）。
#[tauri::command]
pub fn realm_mod_ops(
    state: State<AppPaths>,
    realm_id: String,
    module: String,
    action: String,
) -> Result<Value, String> {
    guard_vcmi_not_running()?;
    let id = VcmiPaths::sanitize_realm_id(&realm_id);
    let base =
        VcmiPaths::realm_vcmi_dir(&id).ok_or_else(|| be_err("common_data_dir_unknown", ""))?;
    let realm_mod = base.join("Mods").join(&module);

    let (user_mods, game_mods) = {
        let paths = state.0.lock().expect("paths lock poisoned");
        (paths.user_mods_dir(), paths.game_mods_dir())
    };
    let global_src = user_mods.join(&module);
    let game_src = game_mods.join(&module);
    let has_global = global_src.is_dir() || game_src.is_dir();

    match action.as_str() {
        "unlink" => {
            if realm_mod.exists() {
                if is_link(&realm_mod) {
                    remove_link(&realm_mod)?;
                } else if realm_mod.is_dir() {
                    fs::remove_dir_all(&realm_mod).map_err(|e| {
                        be_err("realm_private_copy_delete_failed", &format!("{}", e))
                    })?;
                }
            }
            Ok(json!({ "ok": true, "removed": true, "hasGlobal": has_global }))
        }
        "link" => {
            let src = if global_src.is_dir() {
                Some(global_src)
            } else if game_src.is_dir() {
                Some(game_src)
            } else {
                None
            };
            match src {
                Some(s) => {
                    fs::create_dir_all(base.join("Mods")).map_err(|e| {
                        be_err("infra_realm_mods_dir_create_failed", &format!("{}", e))
                    })?;
                    if is_link(&realm_mod) {
                        let _ = remove_link(&realm_mod);
                    }
                    if !realm_mod.exists() {
                        links::ensure_link(&realm_mod, &s)?;
                    }
                    Ok(json!({ "ok": true, "linked": true }))
                }
                None => Ok(json!({ "ok": true, "linked": false, "error": "全局不存在该模组来源" })),
            }
        }
        other => Err(be_err("realm_unknown_mod_op", &format!("{}", other))),
    }
}
