//! 提取协议审计（L2 常驻版）：扫描**真实配置**，发现"配置里有、协议未登记"的文本字段。
//!
//! 与 `scripts/mod-cache-audit.mjs` 的分工：
//! - 脚本 `--ci`（Rust 等价物 = `schema::check_protocol_coverage`）：校验**内置 schema** 的
//!   文本字段是否被协议表 100% 登记（开发期 / CI）；
//! - 本模块：在**每次缓存构建**时对真实 mod 配置做扫描，产出 `<mod>/audit.json`，
//!   用于发现"某个 mod 用了新写法（字段路径）而提取没覆盖"——无需手工跑脚本。
//!
//! 扫描口径（与脚本 `--game-dir` 一致）：字段名**末段**匹配文本键
//! （name/desc/description/biography/tooltip/event/message/title/text/infix，允许尾随 s）；
//! 路径从**实体内部**计（归一配置已剥离实体层）；跳过跨 mod 引用（`<mod>:<id>`）。

use std::collections::BTreeMap;

use serde_json::{json, Map, Value};

use super::schema;

/// 文本字段名（末段）判定（与脚本 `TEXT_KEY` 一致）。
fn is_text_key(k: &str) -> bool {
    let lower = k.to_ascii_lowercase();
    let base = lower.strip_suffix('s').unwrap_or(&lower);
    matches!(
        base,
        "name"
            | "desc"
            | "description"
            | "biography"
            | "tooltip"
            | "event"
            | "message"
            | "title"
            | "text"
            | "infix"
    )
}

/// 递归扫描对象内的文本字段路径（`[]` 表示数组项；深度上限 6，与脚本一致）。
fn scan(node: &Value, prefix: &str, out: &mut BTreeMap<String, usize>, depth: usize) {
    if depth > 6 {
        return;
    }
    match node {
        Value::Object(m) => {
            for (k, v) in m {
                let p = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                match v {
                    Value::String(_) => {
                        if is_text_key(k) {
                            *out.entry(p).or_insert(0) += 1;
                        }
                    }
                    Value::Object(_) | Value::Array(_) => scan(v, &p, out, depth + 1),
                    _ => {}
                }
            }
        }
        Value::Array(a) => {
            let p = format!("{prefix}[]");
            for v in a.iter().take(3) {
                scan(v, &p, out, depth + 1);
            }
        }
        _ => {}
    }
}

/// 审计全部类目的真实配置：返回未登记文本字段 + 城镇提取缺口报告；均无问题时返回 `None`。
///
/// 报告结构：`{ total, cats: { <cat>: { <相对路径>: <命中次数> } }, townGaps: [...] }`。
///
/// `factions` = 已构建的城镇缓存条目（`build_towns` 产物），用于"声明 vs 提取"比对。
pub(crate) fn audit_raw_configs(
    raw_cfgs: &Map<String, Value>,
    factions: &[Value],
) -> Option<Value> {
    let protocol = schema::protocol()?;
    let entities = protocol.get("entities")?.as_object()?;
    let mut unregistered: BTreeMap<String, BTreeMap<String, usize>> = BTreeMap::new();
    for (cat, ent) in entities {
        let Some(cfg) = raw_cfgs.get(cat).and_then(|x| x.as_object()) else {
            continue;
        };
        let registered = schema::registered_paths(ent);
        let mut hits: BTreeMap<String, usize> = BTreeMap::new();
        for (id, val) in cfg {
            if id.contains(':') || !val.is_object() {
                continue;
            }
            scan(val, "", &mut hits, 0);
        }
        let unreg: BTreeMap<String, usize> = hits
            .into_iter()
            .filter(|(p, _)| !schema::is_registered(p, &registered))
            .collect();
        if !unreg.is_empty() {
            unregistered.insert(cat.clone(), unreg);
        }
    }
    let town_gaps = audit_town_gaps(raw_cfgs, factions);
    if unregistered.is_empty() && town_gaps.is_empty() {
        return None;
    }
    let total: usize = unregistered
        .values()
        .map(|m| m.values().sum::<usize>())
        .sum::<usize>();
    Some(json!({
        "note": "配置里出现但协议未登记的文本字段（路径从实体内部计，值为该 mod 内命中次数）；townGaps 为声明与提取不符的城镇",
        "total": total,
        "cats": unregistered,
        "townGaps": town_gaps,
    }))
}

/// 在对象内任意深度查找 `buildingsIcons`（VCMI 写法层级不一：`town.clientInfo.*` / `clientInfo.*`）。
fn find_icons_ref(v: &Value, depth: usize) -> Option<String> {
    if depth > 4 {
        return None;
    }
    let m = v.as_object()?;
    if let Some(s) = m.get("buildingsIcons").and_then(|x| x.as_str()) {
        return Some(s.to_string());
    }
    m.values().find_map(|x| find_icons_ref(x, depth + 1))
}

/// 城镇"声明 vs 提取"缺口：配置声明了 `buildings` / `clientInfo.buildingsIcons`，
/// 但缓存 `factions[].buildings` 为 0、或建筑全部无图标 → 记入报告。
///
/// 这类**静默失败**曾长期存在（家园建筑列表为空、5 个城镇建筑图标全空，运行日志
/// ERROR/WARN=0，只能靠肉眼发现）。本检查随每次构建产出，任何回归都会在 audit.json 暴露。
fn audit_town_gaps(raw_cfgs: &Map<String, Value>, factions: &[Value]) -> Vec<Value> {
    let Some(towns) = raw_cfgs.get("towns").and_then(|x| x.as_object()) else {
        return Vec::new();
    };
    let count_declared = |v: Option<&Value>| -> usize {
        match v {
            Some(Value::Object(m)) => m.len(),
            Some(Value::Array(a)) => a.len(),
            _ => 0,
        }
    };
    let buildings_of = |fid: &str| -> Option<&Vec<Value>> {
        factions
            .iter()
            .find(|f| f.get("factionId").and_then(|x| x.as_str()) == Some(fid))
            .and_then(|f| f.get("buildings").and_then(|x| x.as_array()))
    };
    let mut gaps = Vec::new();
    for (fid, t) in towns {
        if fid.contains(':') || !t.is_object() {
            continue;
        }
        // 1) 声明了建筑但提取为 0（家园：`config/Abode/buildings.json` 曾整批漏读）
        let declared = count_declared(t.get("buildings"))
            .max(count_declared(t.get("town").and_then(|x| x.get("buildings"))));
        let extracted = buildings_of(fid).map(|a| a.len()).unwrap_or(0);
        if declared > 0 && extracted == 0 {
            gaps.push(json!({
                "factionId": fid,
                "kind": "buildings-missing",
                "declared": declared,
                "extracted": extracted,
            }));
        }
        // 2) 声明了建筑图标但全部无图（xanadu/cove/bulwark/christmas/shuHan 曾 100% 缺失）。
        //    阶段一不产图（assets 关闭）→ image 恒为 null，此处必须跳过，否则必然误报。
        if !crate::modcache::assets_enabled() {
            continue;
        }
        if let Some(r) = find_icons_ref(t, 0) {
            if let Some(arr) = buildings_of(fid) {
                let total = arr.len();
                let no_img = arr
                    .iter()
                    .filter(|b| b.get("image").map(|v| v.is_null()).unwrap_or(true))
                    .count();
                if total > 0 && no_img == total {
                    gaps.push(json!({
                        "factionId": fid,
                        "kind": "icons-missing",
                        "iconsRef": r,
                        "buildings": total,
                        "noImage": no_img,
                    }));
                }
            }
        }
    }
    gaps
}
