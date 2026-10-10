//! 宽容 JSON 解析（JSON5 子集）。
//!
//! VCMI 及其配置文件（`mod.json` / `town.json` / `dirs.json` / `modSettings.json` 等）
//! 允许 JSON5 风格写法：行内 `//` 注释、`\'` 转义、无引号键、尾逗号。
//! 手工编辑的 `launcher.json` / `realm.json` 也常带注释与尾逗号。
//!
//! 策略：**标准 JSON 优先**（避免预处理破坏合法 JSON），失败再走宽容预处理。
//!
//! 两个入口：
//! - [`parse_json_lenient`]：返回 `Result`。**写回前必须用它并据错中止**——
//!   把解析失败静默降级为空对象会在回写时清空用户已有全部数据。
//! - [`read_json_file_lenient`]：读文件便利函数，文件不存在返回 `Ok(None)`。

use crate::infra::berror::be_err;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// 宽容解析 JSON5 子集；返回 `Err` 表示确实无法解析（**不要**降级为空对象）。
pub fn parse_json_lenient(text: &str) -> Result<Value, String> {
    // 标准 JSON 优先：能直接解析就直接返回（避免预处理破坏合法 JSON）
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        return Ok(v);
    }
    let cleaned = sanitize_vcmi_json(text);
    serde_json::from_str::<Value>(&cleaned).map_err(|e| be_err("cmd_json_parse_failed", &format!("{}", e)))
}

/// 读取文件并宽容解析。
///
/// - 文件不存在 → `Ok(None)`
/// - 读文件失败 / 解析失败 → `Err`（调用方应报错，不得静默降级）
pub fn read_json_file_lenient(path: &Path) -> Result<Option<Value>, String> {
    if !path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(path).map_err(|e| be_err("infra_file_read_failed", &format!("{}", e)))?;
    parse_json_lenient(&text).map(Some)
}

/// 将 VCMI 风格的 JSON 预处理为合法 JSON：
/// 1. 去掉行内 `//` 注释（不在字符串内时才删）。
/// 2. 将字符串内的 `\'` 转义替换为字面 `'`（VCMI 允许 `'` 作为转义引号）。
/// 3. 去除对象/数组末尾多余的逗号。
fn sanitize_vcmi_json(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let mut in_str = false;
    while i < chars.len() {
        let c = chars[i];
        if in_str {
            // 处理转义
            if c == '\\' && i + 1 < chars.len() {
                let nxt = chars[i + 1];
                if nxt == '\'' {
                    out.push('\'');
                    i += 2;
                    continue;
                }
                out.push(c);
                out.push(nxt);
                i += 2;
                continue;
            }
            out.push(c);
            if c == '"' {
                in_str = false;
            }
            i += 1;
        } else if c == '"' {
            in_str = true;
            out.push(c);
            i += 1;
        } else if c == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
            // 跳过直到行尾
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    strip_trailing_commas(&out)
}

/// 去除尾逗号（`,` 后跟可选空白再跟 `}` 或 `]`），逐字符扫描。
fn strip_trailing_commas(s: &str) -> String {
    let v: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < v.len() {
        let c = v[i];
        if c == ',' && i + 1 < v.len() {
            let mut j = i + 1;
            while j < v.len() && matches!(v[j], ' ' | '\t' | '\n' | '\r') {
                j += 1;
            }
            if j < v.len() && (v[j] == '}' || v[j] == ']') {
                i = j;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// 统一解析 VCMI 模组配置文件（mod.json / town.json / faction.json / creatures 等）。
/// VCMI `JsonParser` 在 JSON5 模式下容忍：行内 `//` 注释、`\'` 转义、无引号键、尾逗号。
/// `parse_vcmi_json` 先尝试标准 JSON，失败再走宽容预处理，故合法 JSON 也不受影响。
///
/// 这是读取 VCMI 模组配置的唯一入口：scan 阶段（识别 modType / factions）与 build 阶段
/// （读取 town / creature 配置）共用同一解析器。此前 scan 用另一套"只去尾逗号"的
/// `read_mod_json`，遇到含 `//` 注释的 mod.json 会解析失败、误判 modType，与 build 阶段
/// 表现不一致——这是城镇不全的根因之一。
pub(crate) fn read_vcmi_json(path: &Path) -> Value {
    // 「尽力而为」：文件缺失 / 解析失败一律返回 Null（模组配置允许缺失）
    crate::infra::json::read_json_file_lenient(path)
        .ok()
        .flatten()
        .unwrap_or(Value::Null)
}

pub(crate) fn read_vcmi_json_bytes(bytes: &[u8]) -> Value {
    let text = String::from_utf8_lossy(bytes);
    parse_vcmi_json(&text)
}

pub(crate) fn parse_vcmi_json(text: &str) -> Value {
    // 宽容解析（JSON5 子集：去 // 注释 + 处理 \' 转义 + 去尾逗号）见 `crate::infra::json`。
    // 这里是「尽力而为」包装：失败返回 Null（模组配置允许缺失/损坏）。
    crate::infra::json::parse_json_lenient(text).unwrap_or(Value::Null)
}

pub(crate) fn collect_all_json_files(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                collect_all_json_files(&p, out);
            } else if p
                .file_name()
                .map(|n| n.to_string_lossy().ends_with(".json"))
                .unwrap_or(false)
            {
                out.push(p);
            }
        }
    }
}

/// 段名与类型名的等价判定（含单复数容错与别名）。
fn type_name_eq(s: &str, t: &str) -> bool {
    if s == t {
        return true;
    }
    // 单复数容错：artifact ↔ artifacts、creature ↔ creatures、hero ↔ heroes…
    if t.ends_with('s') && s == t.trim_end_matches('s') {
        return true;
    }
    if s.ends_with('s') && s.trim_end_matches('s') == t {
        return true;
    }
    // heroClasses 的目录别名 classes（HuaXia 等把职业配置放 config/<mod>/classes/）
    if t == "heroclasses" && s == "classes" {
        return true;
    }
    false
}

/// 配置目录名/文件名与类型名的容错匹配。
///
/// VCMI 生态里存在**大小写与单复数写法差异**，严格段相等（`s == type_subdir`）会整批漏采：
/// - 实测 `config/artifact/ffArt1.json`（单数目录）→ 8 个宝物漏采；
/// - `config/<mod>/Creatures/<x>.json`（大写目录）→ 5 个生物漏采；
/// - `config/<mod>/classes/*.json`（heroClasses 别名，HuaXia）；
/// - **分隔符后缀**目录：`config/Abode-creatures/`、`config/Abode-heroes/`
///   （家园 mod 的写法）→ 生物/英雄整批漏采（index `creatures:0, heroes:0`）。
///   末段与类型名等价才命中（不做"包含即命中"，避免误判）。
pub(crate) fn type_seg_match(seg: &str, type_subdir: &str) -> bool {
    let s = seg.to_lowercase();
    let t = type_subdir.to_lowercase();
    if type_name_eq(&s, &t) {
        return true;
    }
    // `<前缀>-<类型>` / `<前缀>_<类型>`：取最后一个 `-`/`_` 之后的部分比对
    let tail = s.rsplit(|c| c == '-' || c == '_').next().unwrap_or(&s);
    tail != s && type_name_eq(tail, &t)
}

/// 该路径是否为指定类型的实体配置（目录段容错 + 聚合文件名匹配）。
///
/// 聚合文件名（如 `config/<mod>/heroes.json`）也算命中，但**排除 town 配置目录**：
/// `<faction>/town/spells.json` 是魔法行会概率表（`{ "<fid>": { "town": { "guildSpells": ... } } }`），
/// 若按聚合文件采入会生成以 faction id 为 id 的伪实体。
pub(crate) fn is_type_config_path(rel_lower: &str, type_subdir: &str) -> bool {
    let segs: Vec<&str> = rel_lower.split('/').collect();
    let fname = segs.last().copied().unwrap_or("");
    let in_dir = segs[..segs.len().saturating_sub(1)]
        .iter()
        .any(|s| type_seg_match(s, type_subdir));
    let is_agg = fname.trim_end_matches(".json") == type_subdir.to_lowercase()
        && !segs
            .iter()
            .any(|s| matches!(*s, "town" | "towns" | "translation"));
    in_dir || is_agg
}

/// 递归收集 `config/**/<type_subdir>/*.json` 的文件路径（含大小写/单复数容错与聚合文件）。
pub(crate) fn collect_json_files(dir: &Path, type_subdir: &str, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                collect_json_files(&p, type_subdir, out);
            } else if p
                .file_name()
                .map(|n| n.to_string_lossy().to_lowercase().ends_with(".json"))
                .unwrap_or(false)
            {
                let rel = p.to_string_lossy().replace('\\', "/").to_lowercase();
                if is_type_config_path(&rel, type_subdir) {
                    out.push(p);
                }
            }
        }
    }
}
