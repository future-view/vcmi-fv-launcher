//! 宝物（artifact）缓存构建：名称 / 描述 / 图标 / 等级 / 属性。
//!
//! 由 `build_mod_wiki_once` 的「宝物」节原样搬出（S3）；参数与原局部变量同名，节内容零改动。

use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use crate::infra::json::parse_vcmi_json;
use crate::modcache::assets::{extract_game_def_frame_and_save, extract_icon_and_save};
use crate::modcache::collect::collect_config_jsons_merged;
use crate::modcache::i18n::{
    i18n_mod_scope, lower_camel, name_desc_from_keys, name_obj_en_zhcn, name_obj_with_inline,
};
use crate::vcmi::h3core::I18nMap;

pub(crate) fn build_artifacts(
    p: &Path,
    mod_id: &str,
    cache_root: &Path,
    game_dir: &Path,
    i18n: &I18nMap,
) -> Vec<Value> {
    let mut artifacts = Vec::new();
    for (id, obj) in collect_config_jsons_merged(&p, "artifacts") {
        let scope = i18n_mod_scope(mod_id);
        // 图标字段：**VCMI `artifact.json` schema 的 graphics 全集** =
        // `image`（英雄界面大图）/ `scenarioBonus`（58×64）/ `map`（冒险地图 .def）。
        //
        // ⚠️ 此前只取 `image → iconBook → icon`：schema 里没有 `iconBook`/`icon`，
        // 却有 `scenarioBonus` —— 实测 66expfixmod（`bastille:dwarftank` 等）只写
        // `scenarioBonus`/`map` → `image` 恒为 null（用户报"有些宝物也是没有图片"）。
        // `map` 是冒险地图用的 .def，不作为宝物栏图标（尺寸/语义都不对）。
        let g = obj.get("graphics");
        let icon_res = g
            .and_then(|x| {
                x.get("image")
                    .or_else(|| x.get("scenarioBonus"))
                    // 宽容兜底：个别 mod 不按 schema 写
                    .or_else(|| x.get("iconBook"))
                    .or_else(|| x.get("icon"))
            })
            .and_then(|v| v.as_str());
        let img =
            extract_icon_and_save(&p, game_dir, cache_root, mod_id, "artifact", &id, icon_res);
        // 名称/描述：i18n 键 → 配置内嵌文本 → 兜底。
        // ⚠ 字段来源以 VCMI schema（artifact.json）为准：文本写在 **`text.name` /
        // `text.description` / `text.event`** 子对象里（实测 247/261/247 个配置），
        // 顶层 `name`/`description` 仅为旧写法兼容——此前只读顶层 → 内嵌文本整批未用。
        let text = obj.get("text");
        let inline_name = text.and_then(|t| t.get("name")).or_else(|| obj.get("name"));
        let inline_desc = text
            .and_then(|t| t.get("description"))
            .or_else(|| obj.get("description"));
        let artifact_name_obj = name_obj_with_inline(
            &i18n,
            &format!("artifact.{}.{}.name", scope, id),
            inline_name,
            &id,
        );
        // 描述：i18n 键 `artifact.<scope>.<id>.description` → 配置内嵌 description → 空。
        // 此前模组分支**完全不输出该字段** → 267 条模组宝物描述永远为空
        // （其实翻译文件里多有该键，如 crossroads 系列）。
        let artifact_desc_obj = name_obj_with_inline(
            &i18n,
            &format!("artifact.{}.{}.description", scope, id),
            inline_desc,
            "",
        );
        // 事件描述（`text.event`，如战役宝物触发文本；信息保真：进缓存供未来展示）
        let artifact_event_obj = name_obj_with_inline(
            &i18n,
            &format!("artifact.{}.{}.event", scope, id),
            text.and_then(|t| t.get("event")),
            "",
        );
        let components = obj
            .get("components")
            .and_then(|c| c.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        // 装备槽（slot）：字符串 / 字符串数组 / wearable 对象 / 旧写 `slots` 数组。
        //
        // VCMI `CArtHandler::loadSlots`（lib/entities/artifact/CArtHandler.cpp:350-362）读的是
        // **单数 `slot`**，且按"字符串就单个、数组就逐项"处理 —— 我们此前**只认数组**，
        // 于是最常见写法 `"slot": "RIGHT_HAND"`（实测 hota `fakeHorn.json`）解析成空槽，
        // 百科里表现为"装备槽 —"的假宝物。
        let mut slots = Vec::new();
        match obj.get("slot") {
            Some(Value::String(s)) if !s.trim().is_empty() => slots.push(s.trim().to_string()),
            Some(Value::Array(arr)) => {
                for s in arr {
                    if let Some(str) = s.as_str() {
                        slots.push(str.to_string());
                    }
                }
            }
            _ => {}
        }
        if slots.is_empty() {
            // 旧写法 / 少数 mod 的 `slots` 数组（VCMI 不读，但存在真实数据，保留兜底）
            if let Some(arr) = obj.get("slots").and_then(|s| s.as_array()) {
                for s in arr {
                    if let Some(str) = s.as_str() {
                        slots.push(str.to_string());
                    }
                }
            }
        }
        if slots.is_empty() {
            if let Some(w) = obj.get("wearable").and_then(|w| w.as_object()) {
                for k in w.keys() {
                    if k != "hidden" {
                        slots.push(k.clone());
                    }
                }
            }
        }
        let is_combined = components.len() > 0;
        // 效果（bonuses）：type/val/name + 多语言描述
        let mut bonuses = Vec::new();
        if let Some(arr) = obj.get("bonuses").and_then(|b| b.as_array()) {
            for (i, b) in arr.iter().enumerate() {
                let btype = b
                    .get("type")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                let bval = b.get("val").and_then(|x| x.as_i64()).unwrap_or(0);
                // 效果文本：VCMI 主格式为 `artifact.<scope>.<id>.bonus.<bonusType 小驼峰>`，
                // 值为 `{名称}\n描述` 组合文本（如 `artifact.core.orbOfVulnerability.bonus.noResistance`）；
                // 兼容旧版按序号写的 `.bonus.<i>.name/.description`，最后回退配置内嵌文本。
                let (bname_obj, bdesc_obj) = match name_desc_from_keys(
                    &i18n,
                    &[format!(
                        "artifact.{}.{}.bonus.{}",
                        scope,
                        id,
                        lower_camel(&btype)
                    )],
                    bval,
                    None,
                ) {
                    Some((n, d)) => (n, d),
                    None => (
                        name_obj_with_inline(
                            &i18n,
                            &format!("artifact.{}.{}.bonus.{}.name", scope, id, i),
                            b.get("name"),
                            "",
                        ),
                        name_obj_with_inline(
                            &i18n,
                            &format!("artifact.{}.{}.bonus.{}.description", scope, id, i),
                            b.get("description"),
                            "",
                        ),
                    ),
                };
                bonuses.push(json!({
                    "type": btype,
                    "val": bval,
                    "name": bname_obj,
                    "description": bdesc_obj,
                }));
            }
        }
        artifacts.push(json!({
            "id": id,
            "name": artifact_name_obj,
            "description": artifact_desc_obj,
            // 事件描述（`text.event`；信息保真，供未来展示/检索）
            "eventDescription": artifact_event_obj,
            "class": obj.get("class").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            "value": obj.get("value").and_then(|v| v.as_i64()).unwrap_or(0),
            "price": obj.get("value").and_then(|v| v.as_i64()).unwrap_or(0),
            "slots": slots,
            "components": components,
            "isCombined": is_combined,
            "partOf": Vec::<String>::new(),
            "bonuses": bonuses,
            "image": img,
        }));
    }

    // 原版宝物：游戏根 config/artifacts.json + h3-core（ARTRAITS.TXT）元数据（仅 core 伪模组）。
    // 游戏根配置只声明 index/type/components/bonuses；售价 / 装备位 / 等级 / 描述
    // 由 h3-core 的 `artifact.core.<id>.*`（名称/描述）与 `h3_artifact_meta`（value/slots/class）补齐。
    if mod_id == "core" {
        let core_path = game_dir.join("config").join("artifacts.json");
        if let Ok(text) = fs::read_to_string(&core_path) {
            if let Value::Object(map) = parse_vcmi_json(&text) {
                let meta = crate::vcmi::h3core::h3_artifact_meta();
                let existing: std::collections::HashSet<String> = artifacts
                    .iter()
                    .filter_map(|a| a.get("id").and_then(|x| x.as_str()).map(|s| s.to_string()))
                    .collect();
                for (id, obj) in map {
                    if id == "version" {
                        continue;
                    }
                    if existing.contains(&id) {
                        continue;
                    }
                    let index = obj.get("index").and_then(|x| x.as_i64()).unwrap_or(-1);
                    let img = if index >= 0 {
                        extract_game_def_frame_and_save(
                            game_dir,
                            cache_root,
                            "core",
                            "ARTIFACT.DEF",
                            0,
                            index as usize,
                            "artifact",
                            &id,
                        )
                    } else {
                        None
                    };
                    let name = name_obj_en_zhcn(&i18n, &format!("artifact.core.{}.name", id), &id);
                    // 描述（ARTRAITS.TXT 原文形如 `{名称}\n\n正文`，去掉花括号标题行）
                    let desc =
                        name_obj_en_zhcn(&i18n, &format!("artifact.core.{}.description", id), "");
                    let desc = strip_braced_header(desc);
                    // 元数据：value / slots / class（语言无关，来自 ARTRAITS.TXT）
                    let m = meta.as_ref().and_then(|mm| mm.get(&id));
                    // 组合宝物在 ARTRAITS.TXT 的 Cost 列为 0，售价写在游戏根配置的 `value`
                    // （如鬼王斗蓬 12000）→ meta 值为 0 时回退配置值。
                    let meta_value = m
                        .and_then(|x| x.get("value"))
                        .and_then(|x| x.as_i64())
                        .unwrap_or(0);
                    let cfg_value = obj.get("value").and_then(|x| x.as_i64()).unwrap_or(0);
                    let value = if meta_value > 0 {
                        meta_value
                    } else {
                        cfg_value
                    };
                    let class = m
                        .and_then(|x| x.get("class"))
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();
                    let mut slots: Vec<String> = m
                        .and_then(|x| x.get("slots"))
                        .and_then(|x| x.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                                .collect()
                        })
                        .unwrap_or_default();
                    // 回退：type 数组中非 HERO 的项视为槽位（少数 mod 配置写法）
                    if slots.is_empty() {
                        if let Some(arr) = obj.get("type").and_then(|x| x.as_array()) {
                            for v in arr {
                                if let Some(s) = v.as_str() {
                                    if s.to_uppercase() != "HERO" {
                                        slots.push(s.to_string());
                                    }
                                }
                            }
                        }
                    }
                    // 组合宝物组件（游戏根配置声明；此前 core 分支漏读导致"组合宝物组件"为空）
                    let components = obj
                        .get("components")
                        .and_then(|c| c.as_array())
                        .map(|a| {
                            a.iter()
                                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    let is_combined = !components.is_empty();
                    // bonuses 对象 → 数组（描述走 `core.bonus.<TYPE>.description`，与 VCMI 一致）。
                    // VCMI 百科对原版宝物只显示有本地化文案的效果行；未翻译的类型（如
                    // UNDEAD_RAISE_PERCENTAGE / PERCENTAGE_DAMAGE_BOOST）不显示裸类型名——
                    // 其效果说明已包含在 ARTRAITS 的描述正文里。带自定义文案的 mod 走上面的循环。
                    let mut bonuses = Vec::new();
                    if let Some(bmap) = obj.get("bonuses").and_then(|x| x.as_object()) {
                        for (_bname, b) in bmap {
                            let btype = b.get("type").and_then(|x| x.as_str()).unwrap_or("");
                            let bval = b.get("val").and_then(|x| x.as_i64()).unwrap_or(0);
                            let bsub = b
                                .get("subtype")
                                .and_then(|x| x.as_str())
                                .unwrap_or("")
                                .to_string();
                            let sub_base = bsub.rsplit(':').next().unwrap_or(&bsub).to_string();
                            let sub_names = if sub_base.is_empty() {
                                None
                            } else {
                                [
                                    format!("creature.core.{}.name.plural", sub_base),
                                    format!("spell.core.{}.name", sub_base),
                                ]
                                .iter()
                                .find_map(|k| crate::modcache::i18n::i18n_get_multi(&i18n, k))
                            };
                            let Some((names, descs)) =
                                crate::modcache::i18n::ability_text_from_bonus_type(
                                    &i18n,
                                    btype,
                                    bval,
                                    sub_names.as_ref(),
                                )
                            else {
                                continue;
                            };
                            bonuses.push(json!({
                                "type": btype,
                                "val": bval,
                                "name": serde_json::to_value(&names).unwrap_or(json!({})),
                                "description": serde_json::to_value(&descs).unwrap_or(json!({})),
                            }));
                        }
                    }
                    artifacts.push(json!({
                        "id": id,
                        "name": name,
                        "description": desc,
                        "class": class,
                        "value": value,
                        "price": value,
                        "slots": slots,
                        "components": components,
                        "isCombined": is_combined,
                        "partOf": Vec::<String>::new(),
                        "bonuses": bonuses,
                        "image": img,
                    }));
                }
            }
        }
    }

    // 组合宝物 → 组件反向索引 partOf（对齐 VCMI wiki partOf 区块）
    {
        let owners: Vec<(String, Vec<String>)> = artifacts
            .iter()
            .filter(|a| a.get("isCombined").and_then(|x| x.as_bool()) == Some(true))
            .filter_map(|a| {
                let id = a.get("id").and_then(|x| x.as_str())?.to_string();
                let comps: Vec<String> = a
                    .get("components")
                    .and_then(|c| c.as_array())
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                Some((id, comps))
            })
            .collect();
        for a in artifacts.iter_mut() {
            let aid = a
                .get("id")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let mut parts: Vec<String> = owners
                .iter()
                .filter(|(_oid, comps)| comps.iter().any(|c| *c == aid))
                .map(|(oid, _)| oid.clone())
                .collect();
            parts.sort();
            parts.dedup();
            if let Some(obj) = a.as_object_mut() {
                obj.insert("partOf".to_string(), json!(parts));
            }
        }
    }

    artifacts
}

/// 去掉 H3 legacy 文案的 `{标题}` 前缀（`{吸血鬼披风}\n\n正文` → `正文`），保留多语言结构。
///
/// ARTRAITS.TXT / SSTRAITS.TXT 等文本统一以 `{名称}` 起头（VCMI 用它渲染标题行），
/// 百科详情页已单独展示名称，正文再带一次标题会重复。
fn strip_braced_header(v: Value) -> Value {
    fn strip_one(s: &str) -> String {
        let t = s.trim();
        if let Some(rest) = t.strip_prefix('{') {
            if let Some((_, body)) = rest.split_once('}') {
                return body.trim().to_string();
            }
        }
        t.to_string()
    }
    match v {
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(k, val)| {
                    let text = val.as_str().map(strip_one).unwrap_or_default();
                    (k, Value::String(text))
                })
                .collect(),
        ),
        other => other,
    }
}
