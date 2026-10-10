//! 内置 VCMI 配置定义（JSON Schema）与提取协议表：定位、加载、覆盖率校验。
//!
//! - `resources/vcmi-schemas/*.json`：由 `scripts/sync-vcmi-schemas.mjs` 同步（含 `manifest.json`）；
//! - `resources/mod-cache-protocol.json`：提取协议表（字段路径候选链 → 缓存字段 → 翻译键模板）。
//!
//! 目录查找顺序（与 `vcmi::h3core::h3_core_data_dir` 一致）：
//! 1. `env!("CARGO_MANIFEST_DIR")/resources/<name>` —— **编译期常量**，开发运行（tauri dev）必定可用；
//! 2. exe 同级 `resources/<name>`（打包后 bundle.resources 的落点）；
//! 3. exe 同级 `<name>`（另一种打包布局）。
//!
//! 依赖方向：只依赖 `infra`（见 `docs/architecture/backend.zh-CN.md` §4.1）。

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::infra::json::parse_vcmi_json;

/// 协议表（编译期嵌入，零运行时开销；不依赖打包资源，CI/单测也可用）
const PROTOCOL_JSON: &str = include_str!("../../resources/mod-cache-protocol.json");

fn resource_dir(name: &str) -> Option<PathBuf> {
    let dev = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("resources")
        .join(name);
    if dev.is_dir() {
        return Some(dev);
    }
    let exe = std::env::current_exe().ok()?;
    let base = exe.parent()?;
    for rel in [
        format!("resources/{}", name),
        name.to_string(),
    ] {
        let p = base.join(&rel);
        if p.is_dir() {
            return Some(p);
        }
    }
    None
}

/// 内置 VCMI 配置定义（schema）目录；未随包携带时返回 None（协议覆盖率校验会跳过并报告）。
pub(crate) fn schemas_dir() -> Option<PathBuf> {
    resource_dir("vcmi-schemas")
}

/// 内置 schema 的 `contentHash`（`manifest.json`）。
///
/// 用于**缓存失效判定**：VCMI 升级 / 同步脚本重跑后 schema 变化 → 缓存静默重建，
/// 避免"换了内置 schema 但缓存没重建 → 新字段不生效"（开发者忘记递增 `WIKI_CACHE_VERSION` 时尤其重要）。
pub(crate) fn schema_content_hash() -> String {
    schemas_dir()
        .and_then(|d| fs::read_to_string(d.join("manifest.json")).ok())
        .map(|t| parse_vcmi_json(&t))
        .and_then(|v| {
            v.get("contentHash")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_default()
}

/// 提取协议表（`resources/mod-cache-protocol.json`）。
pub(crate) fn protocol() -> Option<Value> {
    parse_vcmi_json(PROTOCOL_JSON).as_object().cloned().map(Value::Object)
}

/// 文本字段判定：schema 字符串字段且末段是文本语义（与脚本 `mod-cache-audit.mjs` 一致）。
///
/// 仅被 [`check_protocol_coverage`] 使用，与之一同在非 debug 构建下裁掉（见该函数的说明）。
#[cfg(any(debug_assertions, test))]
fn is_text_field(path: &str) -> bool {
    let last = path.rsplit('.').next().unwrap_or(path);
    let last = last.trim_end_matches("[]");
    matches!(
        last.to_lowercase().as_str(),
        "name" | "desc" | "description" | "biography" | "tooltip" | "event" | "message" | "title" | "text" | "infix"
    )
}

/// 递归收集 schema 中的文本类字符串字段路径（`[]` = 数组项，`*` = additionalProperties）。
///
/// 仅被 [`check_protocol_coverage`] 使用，与之一同在非 debug 构建下裁掉（见该函数的说明）。
#[cfg(any(debug_assertions, test))]
fn collect_text_paths(node: &Value, prefix: &str, acc: &mut std::collections::BTreeSet<String>, depth: usize) {
    if depth > 8 {
        return;
    }
    let Some(obj) = node.as_object() else { return };
    if obj.get("type").and_then(|x| x.as_str()) == Some("string") {
        if is_text_field(prefix) {
            acc.insert(prefix.to_string());
        }
        return;
    }
    if let Some(items) = obj.get("items") {
        collect_text_paths(items, &format!("{}[]", prefix), acc, depth + 1);
    }
    for comb in ["oneOf", "anyOf", "allOf"] {
        if let Some(Value::Array(arr)) = obj.get(comb) {
            for v in arr {
                collect_text_paths(v, prefix, acc, depth + 1);
            }
        }
    }
    if let Some(Value::Object(props)) = obj.get("properties") {
        for (k, v) in props {
            let p = if prefix.is_empty() {
                k.clone()
            } else {
                format!("{}.{}", prefix, k)
            };
            collect_text_paths(v, &p, acc, depth + 1);
        }
    }
    if let Some(Value::Object(ap)) = obj.get("additionalProperties") {
        let p = if prefix.is_empty() {
            "*".to_string()
        } else {
            format!("{}.*", prefix)
        };
        collect_text_paths(&Value::Object(ap.clone()), &p, acc, depth + 1);
    }
}

/// 协议实体的登记路径集合（`ignoredPaths` + `textFields[].configPaths`）。
pub(crate) fn registered_paths(ent: &Value) -> std::collections::BTreeSet<String> {
    let mut set = std::collections::BTreeSet::new();
    for p in ent.get("ignoredPaths").and_then(|x| x.as_array()).unwrap_or(&vec![]) {
        if let Some(s) = p.as_str() {
            set.insert(s.to_string());
        }
    }
    for tf in ent.get("textFields").and_then(|x| x.as_array()).unwrap_or(&vec![]) {
        for p in tf.get("configPaths").and_then(|x| x.as_array()).unwrap_or(&vec![]) {
            if let Some(s) = p.as_str() {
                set.insert(s.to_string());
            }
        }
    }
    set
}

/// 登记路径匹配：精确、或登记含 `*`/`[]` 通配、或为子路径前缀。
pub(crate) fn is_registered(path: &str, registered: &std::collections::BTreeSet<String>) -> bool {
    if registered.contains(path) {
        return true;
    }
    for r in registered {
        if path.starts_with(&format!("{}.", r)) {
            return true;
        }
        if r.contains('*') || r.contains("[]") {
            let mut re = String::from("^");
            for ch in r.chars() {
                match ch {
                    '.' => re.push_str("\\."),
                    '*' => re.push_str("[^.]+"),
                    '[' => re.push_str("\\["),
                    ']' => re.push_str("\\]"),
                    other => re.push(other),
                }
            }
            re.push('$');
            if let Ok(rx) = regex_lite(&re, path) {
                if rx {
                    return true;
                }
            }
        }
    }
    false
}

/// 极简通配匹配（只支持 `[^.]+` 与转义 `\.`/`\[`/`\]`），避免引入正则依赖。
///
/// 语义：pattern 已由 [`is_registered`] 构造为 `^<字面>[^.]+<字面>…$`；
/// 本函数按 `[^.]+` 切分字面段，逐段在 value 中推进匹配（通配处至少 1 个字符）。
///
/// 历史 bug（80 起修复）：旧实现按 `part[1..]` / `part.len()` 计算字面长度，
/// **未剥离 `\.` 转义的反斜杠**，导致所有含通配的登记路径（如
/// `levels.*.description`、`abilities.*.description`）永远匹配失败——
/// 审计器上线首日即以 802 条"假未登记"暴露该问题（CI 的覆盖率检查同受影响）。
fn regex_lite(pattern: &str, value: &str) -> Result<bool, ()> {
    let pat = pattern.trim_start_matches('^').trim_end_matches('$');
    let parts: Vec<String> = pat.split("[^.]+").map(|s| s.replace('\\', "")).collect();
    let Some(first) = parts.first() else {
        return Ok(true);
    };
    if !value.starts_with(first.as_str()) {
        return Ok(false);
    }
    let mut rest = &value[first.len()..];
    for (i, lit) in parts.iter().enumerate().skip(1) {
        let is_last = i == parts.len() - 1;
        if is_last {
            // 末段：必须以字面段收尾（且通配处非空）
            if rest.len() < lit.len() || !rest.ends_with(lit.as_str()) {
                return Ok(false);
            }
            return Ok(true);
        }
        // 中间段：找字面段位置；其前至少 1 字符（通配非空）
        match rest.find(lit.as_str()) {
            Some(idx) if idx >= 1 => rest = &rest[idx + lit.len()..],
            _ => return Ok(false),
        }
    }
    Ok(true)
}

/// 校验提取协议表对**内置 schema 文本字段**的覆盖率（CI / 单测用）。
///
/// 返回 `Ok(已覆盖字段数)`；有未登记的字段时返回 `Err(未登记路径列表)`
///（`outOfScope` 声明的 schema 会跳过）。
///
/// 调用方只有两处，都在 debug / test 下生效：
/// - `cargo test` 的 `smoke_tests::protocol`（CI 阻断）；
/// - `batch::build_mods_cache_batch_inner_hd` 里 `#[cfg(debug_assertions)]` 的每批自检。
///
/// 因此整个函数（连同其私有的 `is_text_field` / `collect_text_paths`）在非 debug
/// 构建下没有任何调用者，直接按 cfg 裁掉——否则 release 构建必报 3 条 dead_code 警告。
#[cfg(any(debug_assertions, test))]
pub(crate) fn check_protocol_coverage() -> Result<usize, Vec<String>> {
    let Some(protocol) = protocol() else {
        return Err(vec!["协议表解析失败".to_string()]);
    };
    let Some(dir) = schemas_dir() else {
        return Err(vec!["未找到内置 schema 目录（resources/vcmi-schemas）".to_string()]);
    };
    let entities = protocol.get("entities").and_then(|x| x.as_object()).cloned().unwrap_or_default();
    let out_of_scope: std::collections::BTreeSet<String> = protocol
        .get("outOfScope")
        .and_then(|x| x.get("schemas"))
        .and_then(|x| x.as_object())
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();

    // schema 文件 → (类目, 已登记路径集合)
    let mut by_schema: std::collections::BTreeMap<String, std::collections::BTreeSet<String>> =
        std::collections::BTreeMap::new();
    for (_cat, ent) in entities.iter() {
        let Some(schema) = ent.get("schema").and_then(|x| x.as_str()) else {
            continue;
        };
        let set = by_schema.entry(schema.to_string()).or_default();
        for p in registered_paths(ent) {
            set.insert(p);
        }
    }

    let mut violations = Vec::new();
    let mut covered = 0usize;
    let mut files: Vec<String> = fs::read_dir(&dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().extension().map(|x| x == "json").unwrap_or(false))
                .filter(|e| e.file_name().to_string_lossy() != "manifest.json")
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    for f in files {
        if out_of_scope.contains(&f) {
            continue;
        }
        let Ok(txt) = fs::read_to_string(dir.join(&f)) else { continue };
        let schema = parse_vcmi_json(&txt);
        if !schema.is_object() {
            continue;
        }
        let mut texts = std::collections::BTreeSet::new();
        collect_text_paths(&schema, "", &mut texts, 0);
        let registered = by_schema.get(&f);
        for p in texts {
            match registered {
                Some(set) if is_registered(&p, set) => covered += 1,
                _ => violations.push(format!("{}: {}", f, p)),
            }
        }
    }
    if violations.is_empty() {
        Ok(covered)
    } else {
        Err(violations)
    }
}
