//! 法术（spell）缓存构建：名称 / 描述 / 图标 / 等级 / 属性。
//!
//! 由 `build_mod_wiki_once` 的「法术」节原样搬出（S3）；参数与原局部变量同名，节内容零改动。

use std::path::Path;

use serde_json::{json, Value};

use crate::vcmi::gamedata::SPELL_ID_TABLE;
use crate::vcmi::h3core::I18nMap;
use crate::modcache::assets::extract_icon_and_save;
use crate::modcache::collect::collect_config_jsons_merged;
use crate::modcache::i18n::{
    i18n_get_multi, i18n_mod_scope, name_obj_multi_key, name_obj_with_inline,
};

pub(crate) fn build_spells(
    p: &Path,
    mod_id: &str,
    cache_root: &Path,
    game_dir: &Path,
    i18n: &I18nMap,
) -> Vec<Value> {
    let mut spells = Vec::new();
    for (id, obj) in collect_config_jsons_merged(&p, "spells") {
        let scope = i18n_mod_scope(mod_id);
        // 图标字段多写法兼容：iconBook（VCMI 规范）→ image → icon（部分 mod 写法）
        let g = obj.get("graphics");
        let icon_res = g
            .and_then(|x| x.get("iconBook").or_else(|| x.get("image")).or_else(|| x.get("icon")))
            .and_then(|v| v.as_str());
        let img = extract_icon_and_save(&p, game_dir, cache_root, mod_id, "spell", &id, icon_res);
        // 名称/描述：i18n 键 → 配置内嵌文本（部分中文 mod 的 `spells/*.json` 直接写
        // `name: "护城河"`，此前不读 → 回落成法术 id）→ 兜底
        let spell_name_obj = name_obj_with_inline(&i18n, &format!("spell.{}.{}.name", scope, id), obj.get("name"), &id);
        let spell_desc_obj = name_obj_with_inline(&i18n, &format!("spell.{}.{}.description", scope, id), obj.get("description"), "");
        // 派系（schools 数组，兼容单 school 字段）
        let mut schools = Vec::new();
        if let Some(arr) = obj.get("schools").and_then(|s| s.as_array()) {
            for s in arr {
                if let Some(str) = s.as_str() {
                    schools.push(str.to_string());
                }
            }
        } else if let Some(str) = obj.get("school").and_then(|s| s.as_str()) {
            if !str.is_empty() {
                schools.push(str.to_string());
            }
        }
        let mana = obj.get("mana").and_then(|v| v.as_i64()).unwrap_or(0);
        // 分级描述（VCMI CSpell.cpp:141-145）：spell.<scope>.<id>.description.<none|basic|advanced|expert>。
        // 无等级法术（level=0/无 basic 文本）VCMI 用 `description.none`（翻译池里 1249 条）——
        // 此前未查该键，无等级法术的分级描述整批落空；现在 basic/advanced/expert 为空时回退到它。
        // 配置内嵌：schema/实测允许 `levels.<none|base|basic|advanced|expert>.description`
        // （审计器发现：base 166 / advanced 125 / expert 125 / basic 122 / none 105 个配置），
        // 未命中翻译键时按 `levels.<档>.description` → `levels.base.description` → `levels.none.description` 回退。
        let inline_level = |lvl: &str| -> Option<&Value> {
            obj.get("levels")
                .and_then(|l| l.get(lvl))
                .and_then(|x| x.get("description"))
        };
        let none_obj = name_obj_with_inline(
            &i18n,
            &format!("spell.{}.{}.description.none", scope, id),
            inline_level("none"),
            "",
        );
        let is_empty_obj = |v: &Value| v.as_object().map(|o| o.is_empty()).unwrap_or(true);
        let level_desc = |ln: &str| -> Value {
            let v = name_obj_with_inline(
                &i18n,
                &format!("spell.{}.{}.description.{}", scope, id, ln),
                inline_level(ln),
                "",
            );
            if !is_empty_obj(&v) {
                return v;
            }
            // VCMI 的 `base` 档 = 通用基础描述（无等级法术常见）
            for fb in ["base", "none"] {
                let v = name_obj_with_inline(
                    &i18n,
                    &format!("spell.{}.{}.description.{}", scope, id, fb),
                    inline_level(fb),
                    "",
                );
                if !is_empty_obj(&v) {
                    return v;
                }
            }
            none_obj.clone()
        };
        // 派系**多语言名**（与 `schools` 同序）：`spellSchool.<scope>.<id>.name` → `spellSchool.core.<id>.name`。
        // 此前页面直接显示 `schools` 的英文 id（air/fire/…）；原版派系名由 h3-core 提供（气系/火系/土系/水系）。
        let school_names: Vec<Value> = schools
            .iter()
            .map(|s| {
                name_obj_multi_key(
                    &i18n,
                    &[
                        format!("spellSchool.{}.{}.name", scope, s),
                        format!("spellSchool.core.{}.name", s),
                    ],
                    None,
                    s,
                )
            })
            .collect();
        spells.push(json!({
            "id": id,
            "name": spell_name_obj,
            "type": obj.get("type").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            "schools": schools,
            // 派系多语言名（新增；`schools` 保留原 id 数组，前端零破坏）
            "schoolNames": school_names,
            "level": obj.get("level").and_then(|v| v.as_i64()).unwrap_or(0),
            "mana": mana,
            "iconIndex": obj.get("iconIndex").and_then(|v| v.as_i64()).unwrap_or(-1),
            "power": obj.get("power").cloned().unwrap_or(json!({})),
            "cost": obj.get("cost").cloned().unwrap_or(json!({})),
            "description": spell_desc_obj,
            // 初/中/高级描述（VCMI 法术页按等级展开显示）
            "levelDescriptions": {
                "basic": level_desc("basic"),
                "advanced": level_desc("advanced"),
                "expert": level_desc("expert"),
            },
            "image": img,
        }));
    }
    // 原版法术注入（VCMI 法术列表 = 原版 + mod 自定义）：
    // 原版定义只存在于 SPTRAITS.TXT legacy（无 JSON），identifier 表来自 SpellID 枚举
    // （scripts/gen-spell-ids.mjs 生成）。79-81 为生物能力（ability 段），非可学法术。
    // 仅对原版 core 伪模组注入：builtin 法术收敛到 core 一份（不再逐 mod 复制 81 条，
    // 前端 spellMap 按 id 去重、mod 自定义优先于原版）。
    if mod_id == "core" {
        let mut seen: std::collections::HashSet<String> = spells
            .iter()
            .filter_map(|s| s.get("id").and_then(|x| x.as_str()).map(|x| x.to_string()))
            .collect();
        for (identifier, idx) in SPELL_ID_TABLE {
            if *idx > 78 {
                continue;
            }
            // mod 对原版法术的覆盖（core:<id> / <scope>.<id> 等形式）已存在则跳过
            if seen.contains(*identifier)
                || seen
                    .iter()
                    .any(|id| id.ends_with(&format!(":{identifier}")) || id.ends_with(&format!(".{identifier}")))
            {
                continue;
            }
            // 名称/等级/派系/各级描述统一取自 h3-core（`spell.core.<id>.*`）。
            // **中英分列**：h3-core 文案表按语言分文件（english.spells.json / chinese.spells.json），
            // 键的 map 同时含 chinese/english；此前只取单值（中文优先）写进两个语言键，
            // 导致英文界面下原版法术名与分级描述显示中文。
            let pair = |sub: &str| -> (String, String) {
                let m = i18n_get_multi(&i18n, &format!("spell.core.{}.{}", identifier, sub));
                let pick = |order: &[&str]| -> String {
                    m.as_ref()
                        .and_then(|x| order.iter().find_map(|k| x.get(*k)).cloned())
                        .unwrap_or_default()
                };
                (
                    pick(&["chinese", "auto", "english"]),
                    pick(&["english", "auto", "chinese"]),
                )
            };
            let (name_zh, name_en) = pair("name");
            if name_zh.is_empty() && name_en.is_empty() {
                continue;
            }
            let level = pair("level").0.parse::<i64>().unwrap_or(1);
            let schools: Vec<String> = pair("schools")
                .0
                .split(',')
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect();
            // 分级描述改为多语言对象（与模组法术同形态；前端兼容字符串与对象两种形态）
            let lv: Vec<(String, String)> = ["basic", "advanced", "expert"]
                .iter()
                .map(|sub| pair(sub))
                .collect();
            let auto = if name_zh.is_empty() { name_en.clone() } else { name_zh.clone() };
            // 原版派系多语言名（h3-core 的 `spellSchool.core.<id>.name`：气系/火系/土系/水系）
            let core_school_names: Vec<Value> = schools
                .iter()
                .map(|s| {
                    name_obj_multi_key(
                        &i18n,
                        &[format!("spellSchool.core.{}.name", s)],
                        None,
                        s,
                    )
                })
                .collect();
            spells.push(json!({
                "id": identifier,
                "name": { "chinese": name_zh, "english": name_en, "auto": auto },
                "schools": schools,
                "schoolNames": core_school_names,
                "level": level,
                "iconIndex": idx,
                "levelDescriptions": {
                    "basic": { "chinese": lv[0].0, "english": lv[0].1 },
                    "advanced": { "chinese": lv[1].0, "english": lv[1].1 },
                    "expert": { "chinese": lv[2].0, "english": lv[2].1 },
                },
                "image": format!("core/images/spellbon/{idx}.webp"),
                "builtin": true,
            }));
            seen.insert(identifier.to_string());
        }
    }

    spells
}
