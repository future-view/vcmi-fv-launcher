//! 领域自检与自愈：`realm_self_check` 汇总各项一致性检查，`realm_self_heal` 按检查结果修复。

use super::activate::{dirs_realm_id, realm_dirs_match, restore_dirs};
use super::base::{game_dir_of, persist, read_config, MAPS_NATIVE_SUFFIX};
use super::maps::{apply_maps_valve, is_reparse_point, realm_blocked};
use super::meta::realm_id_exists;

use serde_json::{json, Value};
use tauri::State;

use crate::state::AppPaths;
use crate::vcmi::config::LauncherConfig;

use crate::infra::berror::be_err;

/// 各 state 对应的修复动作（前端据此渲染按钮，文案走 i18n，避免后端把中文硬编码到 UI）。
///
/// - `"heal"`：走 `realm_self_heal`（还原 dirs.json / 重建联接），安全自动；
/// - `"unlink"` / `"merge"` / `"revert"`：`repair_game_maps` 的三个动作用于地图阀门的坏形态，
///   都涉及删除联接或搬动真实地图文件，**必须**由玩家显式选择，不参与自动自愈。
fn actions_for(status: &str) -> Vec<&'static str> {
    match status {
        "realm_missing" | "dirs_mismatch" | "active_mismatch" | "maps_orphan" => vec!["heal"],
        "maps_dangling" => vec!["unlink"],
        "maps_realboth" => vec!["merge", "revert"],
        _ => Vec::new(),
    }
}

/// 构建自检报告（§4.5 表格）：比较 `activeRealm` 与 dirs.json 实际指向，以及 Maps 阀门状态。
pub(crate) fn build_self_check(cfg: &LauncherConfig) -> Value {
    let game_dir = cfg.paths.game_dir.clone();
    if !game_dir.join("VCMI_client.exe").is_file() {
        return json!({
            "ok": true, "status": "ok",
            "activeRealm": cfg.realms.active, "dirsRealm": Value::Null,
            "message": "", "fixable": false, "actions": [],
        });
    }
    let active = cfg.realms.active.clone();
    let dirs_realm = dirs_realm_id(&game_dir);
    let realm_exists = |id: &str| realm_id_exists(id);

    let mut status = "ok";
    let mut message = "";
    let mut fixable = true;

    if let Some(x) = &dirs_realm {
        if !realm_exists(x) {
            status = "realm_missing";
            message = "检测到上次进入的领域目录已丢失，修复将自动还原为游戏默认目录";
        } else if active.as_deref() != Some(x.as_str()) {
            status = "dirs_mismatch";
            message = "检测到领域切换未完成（dirs.json 指向与激活记录不一致），修复将还原为游戏默认目录，你可重新进入该领域";
        }
    } else if active.is_some() {
        // dirs.json 未指向任何领域目录：既可能是「残留激活记录」，
        // 也可能是**全 global 领域**——它的 dirs.json 指向全局用户目录，
        // `dirs_realm_id()` 无法识别，与「未激活」在文件层面完全同形。
        // 因此用「该领域激活后应写入的内容」比对来区分，避免误判为残留记录
        // （否则用户一点修复就会被清掉 active_realm、领域被踢出）。
        let matched = active
            .as_deref()
            .map(|id| realm_dirs_match(&game_dir, id))
            .unwrap_or(false);
        if !matched {
            status = "active_mismatch";
            message = "检测到残留的领域激活记录，但当前为游戏默认目录，修复将清除该记录";
        }
    }

    // Maps 阀门自检（仅在 dirs 状态正常时覆盖）
    if status == "ok" {
        let maps = game_dir.join("Maps");
        let native = game_dir.join(format!("Maps.{MAPS_NATIVE_SUFFIX}"));
        let native_exists = native.is_dir();
        let maps_exists = maps.exists();
        let maps_link = is_reparse_point(&maps);
        if native_exists && !maps_exists {
            // 阀门意图的权威：优先 dirs.json 能识别出的领域；识别不出时**回退到激活记录**。
            // 全 global / `dataPath=global` 的领域激活后 dirs.json 指向全局用户目录，
            // `dirs_realm_id()` 恒为 `None`（与「未激活」在文件层面同形，见 `realm_dirs_match`
            // 注释）——此时若仍按「非屏蔽」处理，就会把玩家主动开启的「屏蔽游戏自带地图」
            // 误判成「联接创建中断」：每次启动弹一次假告警，且一「修复」就撤销玩家的屏蔽。
            let intent = dirs_realm.clone().or_else(|| active.clone());
            let blocked = intent.as_deref().map(realm_blocked).unwrap_or(false);
            if !blocked {
                status = "maps_orphan";
                message = "检测到地图目录联接缺失，修复将重建联接（透明态）";
            }
        } else if maps_link && !native_exists {
            status = "maps_dangling";
            fixable = false;
            message = "检测到地图联接悬空（游戏自带 Maps 已不在），需通过「还原游戏目录」手动处理";
        } else if native_exists && maps_exists && !maps_link {
            status = "maps_realboth";
            fixable = false;
            message = "检测到游戏目录出现了新的自带地图，建议合并后重建联接";
        }
    }

    json!({
        "ok": status == "ok",
        "status": status,
        "activeRealm": active,
        "dirsRealm": dirs_realm,
        "message": message,
        "fixable": fixable,
        "actions": actions_for(status),
    })
}

/// 崩溃自检（§4.5）：启动器每次启动调用，返回异常报告供前端弹出修复对话框。
#[tauri::command]
pub fn realm_self_check() -> Value {
    match read_config() {
        Some(cfg) => {
            let report = build_self_check(&cfg);
            let status = report
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("ok");
            if status == "ok" {
                tracing::debug!(target: "realm", "领域自检通过");
            } else {
                tracing::warn!(
                    target: "realm",
                    "领域自检异常：status={} message={}",
                    status,
                    report.get("message").and_then(|v| v.as_str()).unwrap_or("")
                );
            }
            report
        }
        None => json!({
            "ok": true, "status": "ok",
            "activeRealm": Value::Null, "dirsRealm": Value::Null,
            "message": "", "fixable": false, "actions": [],
        }),
    }
}

/// 崩溃自愈（§4.5）：对可安全自动修复的项执行修复，返回修复后的最新报告。
#[tauri::command]
pub fn realm_self_heal(state: State<AppPaths>) -> Result<Value, String> {
    let mut cfg = read_config().ok_or_else(|| be_err("common_not_initialized", ""))?;
    let game_dir = game_dir_of(&state);
    let status = build_self_check(&cfg)
        .get("status")
        .and_then(|v| v.as_str())
        .unwrap_or("ok")
        .to_string();

    match status.as_str() {
        "realm_missing" | "dirs_mismatch" | "active_mismatch" => {
            restore_dirs(&game_dir)?;
            cfg.realms.active = None;
            persist(&cfg)?;
        }
        "maps_orphan" => {
            apply_maps_valve(&game_dir, false)?;
        }
        _ => { /* 非自动可修复项（maps_dangling / maps_realboth）：留给用户手动处理 */
        }
    }
    Ok(build_self_check(&cfg))
}
