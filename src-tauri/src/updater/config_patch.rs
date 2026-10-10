//! 配置补丁：把更新包携带的 `.vcmi-fv-launcher/launcher.patch.json`
//! **一次性**合并进用户的 `launcher.json`，随后删除补丁。
//!
//! ## 为什么需要
//!
//! 版本升级常伴随新配置项（如 `runtimes.ai.package` 的远程组件地址），或修正旧版写坏的默认值。
//! 直接覆盖用户的 `launcher.json` 会丢用户配置；而"该不该覆盖某个键"在用户侧**无法自动判断**
//! —— 分不清"用户有意设置"与"用户设置错了"。
//! ⇒ 由**发行方在补丁里显式列出要改的键**，合并一次即删除：
//! 一次性覆盖**不会永久压住**用户之后在设置面板里的修改。
//!
//! ## 合并语义（拍板口径）
//!
//! - **对象**：递归合并 —— 补丁未提及的键**原样保留**
//! - **数组 / 标量**：整体以**补丁为准**（不逐元素合并：数组的"追加"语义无法预测）
//! - 补丁处理完**立即删除** → 天然幂等：同一补丁只应用一次
//! - 只有**确实产生改动**时才写回配置；写回前备份 `launcher.json.bak`
//! - 任何失败只记日志，**绝不阻断启动**
//!
//! ## 时机
//!
//! **必须早于日志初始化**（见 `lib.rs` setup 首行）：否则补丁里的 `log_level`
//! 要等下次启动才生效。
//!
//! ## 示例
//!
//! ```json5
//! // .vcmi-fv-launcher/launcher.patch.json（发行方提供）
//! {
//!   "runtime": {                                    // 顶层组：按子键递归合并
//!     "log_level": "info",                          // 标量：直接覆盖用户值
//!     "components": {                               // 对象：递归合并，用户其它子键保留
//!       "ai": { "package": { "url": "https://…", "sha256": "…" } }
//!     }
//!   },
//!   "mods": { "required": ["h3e-main-menu"] }       // 数组：整体替换
//! }
//! ```
//!
//! ⚠️ 补丁**必须按当前的分组结构书写**（与磁盘 `launcher.json` 同形，见 `vcmi/config.rs`）：
//! 项目不做旧版扁平键的兼容层，写错了不会被纠正，只会被当作无关字段忽略。

use crate::infra::berror::be_err;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::infra::json::parse_json_lenient;
use crate::vcmi::paths::VcmiPaths;

/// 补丁在更新包中的相对路径（白名单里唯一放行的配置文件名）。
///
/// ⚠️ 与用户的 `launcher.json` **刻意为不同文件**：同名就等于直接覆盖，合并逻辑无从介入。
pub const PATCH_REL: &str = ".vcmi-fv-launcher/launcher.patch.json";

/// 启动时调用：应用 `<exe>/.vcmi-fv-launcher/launcher.patch.json`（不存在则什么都不做）
pub fn apply() {
    let Some(config) = VcmiPaths::launcher_config_path() else {
        return;
    };
    let patch = config.with_file_name("launcher.patch.json");
    match apply_at(&config, &patch) {
        Ok(true) => tracing::info!(
            target: "updater",
            "已合并配置补丁并写回 launcher.json（补丁文件已删除）"
        ),
        Ok(false) => {}
        Err(e) => tracing::warn!(target: "updater", "配置补丁未生效（可忽略）：{e}"),
    }
}

/// 合并实现（可测）：返回是否真的写回了配置。
///
/// 找不到补丁 → `Ok(false)`（未改动）。
pub(crate) fn apply_at(config_path: &Path, patch_path: &Path) -> Result<bool, String> {
    if !patch_path.is_file() {
        return Ok(false);
    }
    let text = fs::read_to_string(patch_path).map_err(|e| be_err("upd_patch_read_failed", &format!("{}", e)))?;
    let patch =
        parse_json_lenient(&text).map_err(|e| be_err("upd_patch_invalid_json", &format!("{}", e)))?;
    if !patch.is_object() {
        return Err(be_err("updater_patch_top_not_object", ""));
    }

    // 用户配置用宽容解析：手工编辑常带 `//` 注释与尾逗号
    let mut user: Value = fs::read_to_string(config_path)
        .ok()
        .and_then(|t| parse_json_lenient(&t).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| Value::Object(Map::new()));

    let before = user.clone();
    merge(&mut user, &patch);
    let changed = user != before;

    if changed {
        if config_path.is_file() {
            // 与 modSettings 的 `.backup` 惯例区分开：配置补丁留一份 `.bak` 便于人工回退
            let bak = PathBuf::from(format!("{}.bak", config_path.display()));
            let _ = fs::copy(config_path, &bak);
        }
        if let Some(parent) = config_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let text = serde_json::to_string_pretty(&user)
            .map_err(|e| be_err("updater_launcher_json_serialize_failed", &format!("{}", e)))?;
        fs::write(config_path, format!("{text}\n"))
            .map_err(|e| be_err("upd_launcher_json_write_failed", &format!("{}", e)))?;
    }

    // 无论是否改动，补丁都视为"已处理" → 删除（一次性语义；失败也只是下次重放一次，无害）
    let _ = fs::remove_file(patch_path);
    Ok(changed)
}

/// 深度合并：**对象递归、数组/标量以 `patch` 为准**
fn merge(base: &mut Value, patch: &Value) {
    if let (Value::Object(b), Value::Object(p)) = (&mut *base, patch) {
        for (k, v) in p {
            match b.get_mut(k) {
                Some(slot) if v.is_object() => merge(slot, v),
                _ => {
                    b.insert(k.clone(), v.clone());
                }
            }
        }
    } else {
        *base = patch.clone();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vcmi-patch-test-{name}"));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(p: &Path, s: &str) {
        fs::write(p, s).unwrap();
    }

    fn read_json(p: &Path) -> Value {
        serde_json::from_str(&fs::read_to_string(p).unwrap()).unwrap()
    }

    #[test]
    fn patch_overrides_scalar_and_keeps_untouched_keys() {
        let d = tmp("merge");
        let cfg = d.join("launcher.json");
        let pat = d.join("launcher.patch.json");
        write(
            &cfg,
            r#"{"paths":{"game_dir":"D:/g"},"hd":{"enhance":false},"ui":{"town_preview":true}}"#,
        );
        write(&pat, r#"{"hd":{"enhance":true},"runtime":{"log_level":"debug"}}"#);

        assert!(apply_at(&cfg, &pat).unwrap());
        let v = read_json(&cfg);
        assert_eq!(v["hd"]["enhance"], true, "补丁键应覆盖");
        assert_eq!(v["runtime"]["log_level"], "debug", "补丁新键应写入");
        assert_eq!(v["paths"]["game_dir"], "D:/g", "未提及的键必须原样保留");
        assert_eq!(v["ui"]["town_preview"], true, "未提及的键必须原样保留");
        assert!(cfg.with_file_name("launcher.json.bak").is_file(), "应留备份");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn patch_merges_objects_recursively() {
        let d = tmp("nested");
        let cfg = d.join("launcher.json");
        let pat = d.join("launcher.patch.json");
        write(
            &cfg,
            r#"{"runtime":{"components":{"ai":{"dir":"custom/ai","package":null}}},"realms":{"active":"old"}}"#,
        );
        write(
            &pat,
            r#"{"runtime":{"components":{"ai":{"package":{"url":"https://x","sha256":"ab"}}}}}"#,
        );

        apply_at(&cfg, &pat).unwrap();
        let v = read_json(&cfg);
        assert_eq!(
            v["runtime"]["components"]["ai"]["dir"],
            "custom/ai",
            "对象递归：用户子键保留"
        );
        assert_eq!(v["runtime"]["components"]["ai"]["package"]["url"], "https://x");
        assert_eq!(v["realms"]["active"], "old");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn patch_replaces_arrays_wholesale() {
        let d = tmp("array");
        let cfg = d.join("launcher.json");
        let pat = d.join("launcher.patch.json");
        write(&cfg, r#"{"mods":{"required":["a","b"]}}"#);
        write(&pat, r#"{"mods":{"required":["c"]}}"#);

        apply_at(&cfg, &pat).unwrap();
        assert_eq!(read_json(&cfg)["mods"]["required"], serde_json::json!(["c"]));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn patch_is_one_shot_and_idempotent() {
        let d = tmp("once");
        let cfg = d.join("launcher.json");
        let pat = d.join("launcher.patch.json");
        write(&cfg, r#"{"hd":{"enhance":true}}"#);
        write(&pat, r#"{"hd":{"enhance":false}}"#);

        apply_at(&cfg, &pat).unwrap();
        assert!(!pat.is_file(), "补丁应被删除");
        // 用户之后改成 true —— 补丁已不在，不会被再次压回去
        write(&cfg, r#"{"hd":{"enhance":true}}"#);
        assert!(!apply_at(&cfg, &pat).unwrap(), "无补丁 → 不写回");
        assert_eq!(read_json(&cfg)["hd"]["enhance"], true, "一次性语义：不永久压用户");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn no_change_does_not_rewrite_config() {
        let d = tmp("nochange");
        let cfg = d.join("launcher.json");
        let pat = d.join("launcher.patch.json");
        // 用户配置带注释（宽容解析能读）—— 若被写回会丢失注释，故"无改动不写回"
        write(
            &cfg,
            "{\n  // 我的手写注释\n  \"hd\": { \"enhance\": true }\n}\n",
        );
        write(&pat, r#"{"hd":{"enhance":true}}"#);

        assert!(!apply_at(&cfg, &pat).unwrap(), "值相同 → 视为未改动");
        let raw = fs::read_to_string(&cfg).unwrap();
        assert!(raw.contains("我的手写注释"), "未改动时不应丢注释");
        assert!(!pat.is_file());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn missing_config_is_created() {
        let d = tmp("fresh");
        let cfg = d.join("launcher.json");
        let pat = d.join("launcher.patch.json");
        write(&pat, r#"{"runtime":{"log_level":"warn"}}"#);

        assert!(apply_at(&cfg, &pat).unwrap());
        assert_eq!(read_json(&cfg)["runtime"]["log_level"], "warn");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn invalid_patch_is_ignored_and_removed() {
        let d = tmp("bad");
        let cfg = d.join("launcher.json");
        let pat = d.join("launcher.patch.json");
        write(&cfg, r#"{"hd":{"enhance":false}}"#);
        write(&pat, "[1,2,3]"); // 顶层不是对象

        assert!(apply_at(&cfg, &pat).is_err());
        assert_eq!(read_json(&cfg)["hd"]["enhance"], false, "配置不应被破坏");
        let _ = fs::remove_dir_all(&d);
    }
}
