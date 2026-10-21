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
//! - **标量数组**（元素全为 string/number/bool）：**去重合并** —— 用户既有项保持原序在前，
//!   补丁中缺失的项追加在后。服务「用户可增删的源列表」（如 `realms.repository_urls`）：
//!   升级补丁带入官方默认源时**不清空**用户自定义源（2026-10-21 决策，取代早先的
//!   "数组整体替换"——那个口径会让每版补丁都覆盖掉用户手加的仓库地址）
//! - **含对象/嵌套数组的数组、标量**：整体以**补丁为准**（对象数组无唯一标识，且
//!   `network.mirror_rules` 等有声明序语义，追加合并会静默改变优先级）
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
//!   "mods": { "required": ["h3e-main-menu"] }       // 标量数组：去重合并（用户既有项保留，缺失项追加）
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

/// 启动时调用：应用配置补丁（不存在则什么都不做）。
///
/// 查找顺序（按序取第一个存在的；ADR 0008 D10 + macOS `.app` 布局）：
/// 1. **数据目录**（与 `launcher.json` 同级）——Windows 便携版 / 更新包解压落地处；
/// 2. **exe 同级** `.vcmi-fv-launcher/`——macOS/Linux 手动解压运行的便携布局；
/// 3. **`<exe>/../Resources/.vcmi-fv-launcher/`**——macOS `.app`：`bundle.resources`
///    把补丁放进 `Contents/Resources/`，而 exe 在 `Contents/MacOS/`（两者不同级）。
pub fn apply() {
    let Some(config) = VcmiPaths::launcher_config_path() else {
        return;
    };
    let Some(patch) = find_patch(&config) else {
        return;
    };
    match apply_at(&config, &patch) {
        Ok(true) => tracing::info!(
            target: "updater",
            "已合并配置补丁并写回 launcher.json（补丁文件已删除）"
        ),
        Ok(false) => {}
        Err(e) => tracing::warn!(target: "updater", "配置补丁未生效（可忽略）：{e}"),
    }
}

/// 补丁候选位置（按序取第一个存在的）
fn find_patch(config_path: &Path) -> Option<PathBuf> {
    let rel = ".vcmi-fv-launcher/launcher.patch.json";
    let mut candidates = vec![config_path.with_file_name("launcher.patch.json")];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // 便携布局：exe 同级
            candidates.push(dir.join(rel));
            // macOS .app：exe 在 Contents/MacOS/，资源在 Contents/Resources/
            if let Some(contents) = dir.parent() {
                candidates.push(contents.join("Resources").join(rel));
            }
        }
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// 合并实现（可测）：返回是否真的写回了配置。
///
/// 找不到补丁 → `Ok(false)`（未改动）。
pub(crate) fn apply_at(config_path: &Path, patch_path: &Path) -> Result<bool, String> {
    if !patch_path.is_file() {
        return Ok(false);
    }
    let text = fs::read_to_string(patch_path)
        .map_err(|e| be_err("upd_patch_read_failed", &format!("{}", e)))?;
    let patch = parse_json_lenient(&text)
        .map_err(|e| be_err("upd_patch_invalid_json", &format!("{}", e)))?;
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

/// 深度合并（语义见模块 doc「合并语义」）：
///
/// - **对象**：递归合并 —— 补丁未提及的键原样保留；
/// - **标量数组**：去重合并 —— 用户既有项保持原序在前，补丁中缺失的项追加在后；
/// - **含对象/嵌套的数组、标量、键缺失、类型不符**：整体以 `patch` 为准。
fn merge(base: &mut Value, patch: &Value) {
    if let (Value::Object(b), Value::Object(p)) = (&mut *base, patch) {
        for (k, v) in p {
            match b.get_mut(k) {
                Some(slot) if v.is_object() => merge(slot, v),
                // 标量数组合并：补丁空数组 = 无项可加（不清空用户列表）
                Some(Value::Array(slot)) if is_scalar_array(v) => union_scalars(slot, v),
                _ => {
                    b.insert(k.clone(), v.clone());
                }
            }
        }
    } else {
        *base = patch.clone();
    }
}

/// 补丁值是「标量数组」？—— 元素全为 string / number / bool（空数组 vacuously 成立）。
///
/// 含 `null` / 对象 / 嵌套数组 → `false`（走整体替换：对象数组无唯一标识，
/// 且 `network.mirror_rules` 等有声明序语义，追加合并会静默改变优先级）。
fn is_scalar_array(v: &Value) -> bool {
    match v {
        Value::Array(items) => items
            .iter()
            .all(|i| i.is_string() || i.is_number() || i.is_boolean()),
        _ => false,
    }
}

/// 标量数组合并：用户既有项保持原序在前，补丁中缺失的项（按值去重）追加在后。
fn union_scalars(base: &mut Vec<Value>, patch: &Value) {
    let Value::Array(items) = patch else { return };
    for it in items {
        if !base.contains(it) {
            base.push(it.clone());
        }
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
        write(
            &pat,
            r#"{"hd":{"enhance":true},"runtime":{"log_level":"debug"}}"#,
        );

        assert!(apply_at(&cfg, &pat).unwrap());
        let v = read_json(&cfg);
        assert_eq!(v["hd"]["enhance"], true, "补丁键应覆盖");
        assert_eq!(v["runtime"]["log_level"], "debug", "补丁新键应写入");
        assert_eq!(v["paths"]["game_dir"], "D:/g", "未提及的键必须原样保留");
        assert_eq!(v["ui"]["town_preview"], true, "未提及的键必须原样保留");
        assert!(
            cfg.with_file_name("launcher.json.bak").is_file(),
            "应留备份"
        );
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
            v["runtime"]["components"]["ai"]["dir"], "custom/ai",
            "对象递归：用户子键保留"
        );
        assert_eq!(
            v["runtime"]["components"]["ai"]["package"]["url"],
            "https://x"
        );
        assert_eq!(v["realms"]["active"], "old");
        let _ = fs::remove_dir_all(&d);
    }

    /// 对象数组：仍**整体替换**（无唯一标识；`mirror_rules` 有声明序语义）
    #[test]
    fn patch_replaces_object_arrays_wholesale() {
        let d = tmp("object-array");
        let cfg = d.join("launcher.json");
        let pat = d.join("launcher.patch.json");
        write(
            &cfg,
            r#"{"network":{"mirror_rules":[{"enabled":true,"hosts":["a.com"]}]}}"#,
        );
        write(
            &pat,
            r#"{"network":{"mirror_rules":[{"enabled":false,"hosts":["b.com"]}]}}"#,
        );

        apply_at(&cfg, &pat).unwrap();
        assert_eq!(
            read_json(&cfg)["network"]["mirror_rules"],
            serde_json::json!([{ "enabled": false, "hosts": ["b.com"] }])
        );
        let _ = fs::remove_dir_all(&d);
    }

    /// 标量数组：**去重合并** —— 用户既有项保持原序在前，补丁缺失项追加
    /// （`realms.repository_urls` 场景：升级带入官方源，不清空用户自定义源）
    #[test]
    fn patch_unions_scalar_arrays() {
        let d = tmp("scalar-array");
        let cfg = d.join("launcher.json");
        let pat = d.join("launcher.patch.json");
        write(
            &cfg,
            r#"{"realms":{"repository_urls":["https://custom/index.json"]}}"#,
        );
        write(
            &pat,
            r#"{"realms":{"repository_urls":["https://official/index.json"]}}"#,
        );

        apply_at(&cfg, &pat).unwrap();
        assert_eq!(
            read_json(&cfg)["realms"]["repository_urls"],
            serde_json::json!(["https://custom/index.json", "https://official/index.json"])
        );
        let _ = fs::remove_dir_all(&d);
    }

    /// 补丁空数组 = 无项可加：**不清空**用户列表，且不写回（保住手写注释）
    #[test]
    fn patch_empty_scalar_array_keeps_user_items() {
        let d = tmp("scalar-array-empty");
        let cfg = d.join("launcher.json");
        let pat = d.join("launcher.patch.json");
        write(
            &cfg,
            "{\n  // 我的手写注释\n  \"realms\": { \"repository_urls\": [\"https://custom/index.json\"] }\n}\n",
        );
        write(&pat, r#"{"realms":{"repository_urls":[]}}"#);

        assert!(!apply_at(&cfg, &pat).unwrap(), "无新增项 → 视为未改动");
        let raw = fs::read_to_string(&cfg).unwrap();
        assert!(raw.contains("我的手写注释"), "未改动时不应丢注释");
        let _ = fs::remove_dir_all(&d);
    }

    /// 合并幂等：补丁项已在用户列表中 → 无改动、不写回（重放同一补丁也安全）
    #[test]
    fn patch_union_is_idempotent() {
        let d = tmp("scalar-array-idem");
        let cfg = d.join("launcher.json");
        let pat = d.join("launcher.patch.json");
        write(
            &cfg,
            r#"{"realms":{"repository_urls":["https://custom/index.json","https://official/index.json"]}}"#,
        );
        write(
            &pat,
            r#"{"realms":{"repository_urls":["https://official/index.json"]}}"#,
        );

        assert!(!apply_at(&cfg, &pat).unwrap(), "值相同 → 视为未改动");
        assert_eq!(
            read_json(&cfg)["realms"]["repository_urls"],
            serde_json::json!(["https://custom/index.json", "https://official/index.json"]),
            "原序原值保留"
        );
        let _ = fs::remove_dir_all(&d);
    }

    /// 全新配置（无 `realms` 组）：补丁整键写入 —— fresh install 拿到官方默认源
    #[test]
    fn patch_writes_missing_group_wholesale() {
        let d = tmp("fresh-group");
        let cfg = d.join("launcher.json");
        let pat = d.join("launcher.patch.json");
        write(&cfg, r#"{"paths":{"game_dir":"D:/g"}}"#);
        write(
            &pat,
            r#"{"realms":{"repository_urls":["https://official/index.json"]}}"#,
        );

        apply_at(&cfg, &pat).unwrap();
        let v = read_json(&cfg);
        assert_eq!(
            v["realms"]["repository_urls"],
            serde_json::json!(["https://official/index.json"])
        );
        assert_eq!(v["paths"]["game_dir"], "D:/g", "未提及的键必须原样保留");
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
        assert_eq!(
            read_json(&cfg)["hd"]["enhance"],
            true,
            "一次性语义：不永久压用户"
        );
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
