//! 缓存构建的数据收集：配置 JSON 合并、原始配置快照、派生数值计算。
//!
//! 依赖方向：可依赖 `infra` 与 `vcmi`（见 `docs/architecture/backend.zh-CN.md` §4.1）。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::infra::json::{collect_json_files, parse_vcmi_json};
use crate::vcmi::gamedata::push_config_object;

/// 深度合并两个 JSON：`b` 的字段并入 `a`（对象递归合并，其余类型以 `b` 覆盖）。
///
/// 对应 VCMI 的模组配置合并语义：**同一实体可以分散在多个文件中**
/// （典型：`creatures/xxx.json` 定义名称/属性/图形，`creatures/stack_exp.json` 只补
/// `stackExperience`），必须合并而非整条覆盖，否则后者会把完整定义冲成残缺定义
/// （表现为生物名回落 id、属性全 0、无图）。
pub(crate) fn merge_json_in_place(a: &mut Value, b: &Value) {
    match (a, b) {
        (Value::Object(am), Value::Object(bm)) => {
            for (k, bv) in bm {
                match am.get_mut(k) {
                    Some(av) => merge_json_in_place(av, bv),
                    None => {
                        am.insert(k.clone(), bv.clone());
                    }
                }
            }
        }
        (a_slot, bv) => *a_slot = bv.clone(),
    }
}

/// 收集指定类型配置并按实体 id **深度合并**（同 id 多文件合并为一条）。
/// 文件按路径排序，保证合并顺序确定（跨运行稳定，避免缓存抖动）。
pub(crate) fn collect_config_jsons_merged(mod_path: &Path, type_subdir: &str) -> Vec<(String, Value)> {
    let mut raw = collect_config_jsons(mod_path, type_subdir);
    raw.sort_by(|x, y| x.0.cmp(&y.0));
    let mut map: HashMap<String, Value> = HashMap::new();
    for (id, v) in raw {
        match map.get_mut(&id) {
            Some(existing) => merge_json_in_place(existing, &v),
            None => {
                map.insert(id, v);
            }
        }
    }
    map.into_iter().collect()
}

/// 收集 mod 配置目录下指定类型子目录的所有 JSON（含 Content 目录 + content.zip）。
/// 返回 `(对象 id, 对象根 Value)`。类型如 "creatures"/"heroes"/"spells"/"artifacts"/"skills"。
/// 每个 JSON 顶层 `{ "<id>": { ... } }`，也可能直接是对象体（无包裹），此时 id 用文件名。
pub(crate) fn collect_config_jsons(mod_path: &Path, type_subdir: &str) -> Vec<(String, Value)> {
    let mut out: Vec<(String, Value)> = Vec::new();
    let type_subdir = type_subdir.trim_matches('/');

    // 1) Content/config/**/<type>/*.json
    let content_dir = mod_path.join("Content").join("config");
    if content_dir.is_dir() {
        let mut files = Vec::new();
        collect_json_files(&content_dir, type_subdir, &mut files);
        for f in files {
            if let Ok(text) = fs::read_to_string(&f) {
                let v = parse_vcmi_json(&text);
                push_config_object(&mut out, &f.to_string_lossy(), v, type_subdir);
            }
        }
    }

    // 2) content.zip 内 config/**/<type>/*.json
    let zip_path = mod_path.join("content.zip");
    if let Ok(file) = fs::File::open(&zip_path) {
        if let Ok(mut archive) = zip::ZipArchive::new(file) {
            for i in 0..archive.len() {
                if let Ok(mut entry) = archive.by_index(i) {
                    let name = entry.name().to_string();
                    let nl = name.replace('\\', "/");
                    let nl_lower = nl.to_lowercase();
                    // config/<faction>/<type>/xxx.json 或 config/<type>/xxx.json；
                    // 含大小写/单复数容错与聚合文件（`<mod>/heroes.json`），
                    // 排除 `town/`（魔法行会概率表）与 translation（见 is_type_config_path）
                    let is_target = nl_lower.ends_with(".json")
                        && nl_lower.split('/').any(|s| s == "config")
                        && crate::infra::json::is_type_config_path(&nl_lower, type_subdir);
                    if is_target {
                        let mut buf = Vec::new();
                        if std::io::Read::read_to_end(&mut entry, &mut buf).is_ok() {
                            let text = String::from_utf8_lossy(&buf);
                            let v = parse_vcmi_json(&text);
                            push_config_object(&mut out, &name, v, type_subdir);
                        }
                    }
                }
            }
        }
    }
    out
}

/// 全量收集 mod 的所有 JSON 配置并做**多文件归一**（保底数据层，写入独立 raw.json）。
/// 归一规则（完全兼容原始数据接口，仅聚合、不改写原始字段）：
/// - 已知类型（路径含 config/<type>/）：`{ "<type>": { "<id>": 原始对象 } }`，同类型多文件按键合并
///   类型：creatures/heroes/heroClasses/towns(town|towns|faction|factions)/spells/artifacts/skills/
///        terrains/rivers/roads/objects/obstacles/battlefields/resources/abilities/bonuses
/// - translation/*.json：`{ "translation": { "<文件名>": 原始内容 } }`
/// - 根 mod.json / Content/mod.json：`{ "mod": { ...原始字段 } }`
/// - 其余无法归类：`{ "files": { "<相对路径>": 原始内容 } }` 兜底
pub(crate) fn collect_all_raw_configs(mod_path: &Path) -> serde_json::Map<String, Value> {
    // 先收集 (归一类型 or None, 原始 Value, 相对路径)
    enum Slot {
        Type(&'static str),
        Translation(String),
        Mod,
        File(String),
    }
    let mut items: Vec<(Slot, Value)> = Vec::new();

    fn known_type(rel_lower: &str) -> Option<&'static str> {
        const TYPES: &[&str] = &[
            "creatures", "heroes", "heroclasses", "spells", "artifacts", "skills",
            "terrains", "rivers", "roads", "objects", "obstacles", "battlefields",
            "resources", "abilities", "bonuses",
        ];
        let norm = |t: &'static str| match t {
            "heroclasses" => "heroClasses",
            other => other,
        };
        // 1) **文件名优先**：`config/terrain/obstacles.json` 归 obstacles 而不是 terrains
        //    （否则障碍物/河流/道路定义会混进地形池）；`terrains.json` / `rivers.json` 同理。
        let fname = rel_lower
            .rsplit('/')
            .next()
            .unwrap_or("")
            .trim_end_matches(".json");
        for &t in TYPES {
            if crate::infra::json::type_seg_match(fname, t) {
                return Some(norm(t));
            }
        }
        // 2) 目录段回退：**段优先、从路径末段往前**（越靠后越具体）——
        //    容错大小写、单复数与 `<前缀>-<类型>` 变体（`artifact/`、`Creatures/`、`Abode-creatures/`）。
        //    必须"段优先"而非"类型优先"：mod 目录名可能以类型名收尾（`creature-heroes/`），
        //    类型优先会让 `config/creature-heroes/skills/x.json` 被判成 heroes（实测回归）。
        let segs: Vec<&str> = rel_lower.split('/').filter(|s| !s.is_empty()).collect();
        for seg in segs.iter().rev() {
            for &t in TYPES {
                if crate::infra::json::type_seg_match(seg, t) {
                    return Some(norm(t));
                }
            }
        }
        // HuaXia 等把职业配置放 config/<mod>/classes/（分段 "classes"）
        if rel_lower.split('/').any(|s| s == "classes") {
            return Some("heroClasses");
        }
        // 城镇/阵营配置：town|towns|faction|factions 归入 towns（兼容 town.json 这类带扩展名的文件名）
        if rel_lower
            .split('/')
            .any(|s| s.starts_with("town") || s.starts_with("faction") || matches!(s, "towns" | "factions"))
        {
            return Some("towns");
        }
        None
    }

    let mut collect = |rel: String, v: Value| {
        let rel_lower = rel.to_lowercase();
        let slot = if rel == "mod.json" || rel == "Content/mod.json" {
            Slot::Mod
        } else if let Some(t) = known_type(&rel_lower) {
            Slot::Type(t)
        } else if rel_lower.contains("translation") {
            let stem = rel.rsplit('/').next().unwrap_or(&rel).trim_end_matches(".json").to_string();
            Slot::Translation(stem)
        } else {
            // 兜底：内容嗅探 town 配置（HuaXia 等路径不含 town/faction 分段，如 config/<mod>/xxx.json）
            // 同时检查对象自身与一级子对象（兼容平级结构 {town, buildings, clientInfo} 与包裹结构 {<fid>: {...}}）
            let has_town_marks = |x: &Value| {
                x.get("clientInfo").is_some() || x.get("townScreen").is_some() || x.get("buildings").is_some()
            };
            let looks_town = has_town_marks(&v)
                || v.as_object().map(|obj| obj.values().any(|val| has_town_marks(val))).unwrap_or(false);
            if looks_town {
                items.push((Slot::Type("towns"), v));
                return;
            }
            items.push((Slot::File(rel.clone()), v));
            return;
        };
        items.push((slot, v));
    };

    // 1) 根 mod.json / Content/mod.json
    for rel in ["mod.json", "Content/mod.json"] {
        let fp = mod_path.join(rel);
        if fp.is_file() {
            if let Ok(text) = fs::read_to_string(&fp) {
                collect(rel.to_string(), parse_vcmi_json(&text));
            }
        }
    }
    // 2) Content/config/**/*.json（磁盘）
    let cfg_dir = mod_path.join("Content").join("config");
    let mut files = Vec::new();
    fn walk(dir: &Path, out_files: &mut Vec<PathBuf>) {
        if let Ok(entries) = fs::read_dir(dir) {
            for e in entries.flatten() {
                let fp = e.path();
                if fp.is_dir() {
                    walk(&fp, out_files);
                } else if fp.extension().and_then(|x| x.to_str()).map(|x| x.eq_ignore_ascii_case("json")).unwrap_or(false) {
                    out_files.push(fp);
                }
            }
        }
    }
    if cfg_dir.is_dir() {
        walk(&cfg_dir, &mut files);
        for f in files {
            let rel = f
                .strip_prefix(mod_path.join("Content"))
                .unwrap_or(&f)
                .to_string_lossy()
                .replace('\\', "/");
            if let Ok(text) = fs::read_to_string(&f) {
                collect(rel, parse_vcmi_json(&text));
            }
        }
    }
    // 3) content.zip 内全部 .json
    let zip_path = mod_path.join("content.zip");
    if let Ok(file) = fs::File::open(&zip_path) {
        if let Ok(mut archive) = zip::ZipArchive::new(file) {
            for i in 0..archive.len() {
                if let Ok(mut entry) = archive.by_index(i) {
                    let name = entry.name().to_string();
                    let nl = name.replace('\\', "/");
                    if !nl.to_lowercase().ends_with(".json") || nl.contains("__MACOSX") {
                        continue;
                    }
                    let mut buf = Vec::new();
                    if std::io::Read::read_to_end(&mut entry, &mut buf).is_ok() {
                        let text = String::from_utf8_lossy(&buf);
                        collect(nl, parse_vcmi_json(&text));
                    }
                }
            }
        }
    }

    // 归一合并（深合并：VCMI 常把 town 节点拆到多个文件，如 structures.json/faction.json，
    // 浅覆盖会丢失兄弟字段——如先并入 town.buildings 再并入 town.clientInfo 会互删）
    fn deep_merge(dst: &mut serde_json::Map<String, Value>, src: &serde_json::Map<String, Value>) {
        for (k, val) in src {
            match dst.get_mut(k) {
                Some(Value::Object(d)) if val.is_object() => {
                    if let Some(s) = val.as_object() {
                        deep_merge(d, s);
                    }
                }
                _ => {
                    dst.insert(k.clone(), val.clone());
                }
            }
        }
    }
    let mut out = serde_json::Map::new();
    for (slot, v) in items {
        match slot {
            Slot::Mod => {
                let dst = out.entry("mod".to_string()).or_insert_with(|| json!({}));
                if let (Some(d), Some(s)) = (dst.as_object_mut(), v.as_object()) {
                    deep_merge(d, s);
                }
            }
            Slot::Type(t) => {
                let type_map = out.entry(t.to_string()).or_insert_with(|| json!({}));
                if let Some(d) = type_map.as_object_mut() {
                    if v.is_object() {
                        // 类型下按实体 id 深合并（同 id 跨文件补全字段）
                        if let Some(s) = v.as_object() {
                            for (eid, ev) in s {
                                match d.get_mut(eid) {
                                    Some(Value::Object(dd)) if ev.is_object() => {
                                        if let Some(s2) = ev.as_object() {
                                            deep_merge(dd, s2);
                                        }
                                    }
                                    _ => {
                                        d.insert(eid.clone(), ev.clone());
                                    }
                                }
                            }
                        }
                    } else {
                        d.insert("__raw__".to_string(), v);
                    }
                }
            }
            Slot::Translation(stem) => {
                let tr = out.entry("translation".to_string()).or_insert_with(|| json!({}));
                if let Some(d) = tr.as_object_mut() {
                    d.insert(stem, v);
                }
            }
            Slot::File(rel) => {
                let fl = out.entry("files".to_string()).or_insert_with(|| json!({}));
                if let Some(d) = fl.as_object_mut() {
                    d.insert(rel, v);
                }
            }
        }
    }
    out
}

/// 递归累加升级链完整招募成本（对齐 VCMI getFullRecruitCost）：
/// full = 自身 cost + 所有升级来源（沿 upgradedFrom 链）cost 之和。
pub(crate) fn accumulate_full_cost(
    creature_jsons: &std::collections::HashMap<String, Value>,
    upgraded_from: &std::collections::HashMap<String, Vec<String>>,
    id: &str,
    seen: &mut std::collections::HashSet<String>,
) -> Value {
    let mut result = serde_json::Map::new();
    if !seen.insert(id.to_string()) {
        return Value::Object(result); // 防环
    }
    let add_cost = |dst: &mut serde_json::Map<String, Value>, c: &Value| {
        if let Some(cost) = c.get("cost").and_then(|x| x.as_object()) {
            for (k, v) in cost {
                let n = v.as_i64().unwrap_or(0);
                if n != 0 {
                    let cur = dst.get(k).and_then(|x| x.as_i64()).unwrap_or(0);
                    dst.insert(k.clone(), json!(cur + n));
                }
            }
        }
    };
    if let Some(obj) = creature_jsons.get(id) {
        add_cost(&mut result, obj);
    }
    if let Some(parents) = upgraded_from.get(id) {
        for pid in parents {
            let sub = accumulate_full_cost(creature_jsons, upgraded_from, pid, seen);
            if let Some(m) = sub.as_object() {
                for (k, v) in m {
                    let cur = result.get(k).and_then(|x| x.as_i64()).unwrap_or(0);
                    result.insert(k.clone(), json!(cur + v.as_i64().unwrap_or(0)));
                }
            }
        }
    }
    seen.remove(id);
    Value::Object(result)
}
