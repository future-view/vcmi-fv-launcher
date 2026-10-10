//! 技能（skill）缓存构建：名称 / 描述 / 图标 / 等级 / 属性。
//!
//! 由 `build_mod_wiki_once` 的「技能」节原样搬出（S3）；参数与原局部变量同名，节内容零改动。

use std::collections::HashMap;
use std::path::Path;

use serde_json::{json, Value};

use crate::vcmi::h3core::I18nMap;
use crate::modcache::assets::{
    extract_game_def_frame_and_save, extract_icon_and_save,
};
use crate::modcache::collect::collect_config_jsons_merged;
use crate::modcache::i18n::{i18n_get_multi, i18n_mod_scope, name_obj_with_inline};

pub(crate) fn build_skills(
    p: &Path,
    mod_id: &str,
    cache_root: &Path,
    game_dir: &Path,
    i18n: &I18nMap,
    skill_index_map: &HashMap<String, i64>,
) -> Vec<Value> {
    let mut skills = Vec::new();
    for (id, obj) in collect_config_jsons_merged(&p, "skills") {
        let scope = i18n_mod_scope(mod_id);
        let img = obj.get("basic").and_then(|b| b.get("images")).and_then(|i| i.get("medium"))
            .and_then(|v| v.as_str())
            .and_then(|r| extract_icon_and_save(&p, game_dir, cache_root, mod_id, "skill", &id, Some(r)));
        // 名称：i18n 键 → 配置内嵌文本（部分 mod 把技能名直接写在配置里）→ id
        let skill_name_obj = name_obj_with_inline(&i18n, &format!("skill.{}.{}.name", scope, id), obj.get("name"), &id);
        // VCMI 技能键：`skill.<scope>.<id>.description.<basic|advanced|expert>`。
        // 此前拼成 `...<lvl>.description`，**模组技能的三级描述因此全部落空**
        // （实测 `skill.andruids-expansion.initiative.initiative.description.advanced`）；
        // 旧写法保留为回退，兼容个别写反的 mod。
        // 第三路：**配置内嵌** `basic/advanced/expert.description`（schema 允许；实测各 78 个配置，
        // 作者把描述直接写在技能配置里）——审计器（scripts/wiki-audit.mjs）发现。
        let lvl_desc = |lvl: &str, lang: &str| -> String {
            let hit = i18n_get_multi(&i18n, &format!("skill.{}.{}.description.{}", scope, id, lvl))
                .or_else(|| {
                    i18n_get_multi(&i18n, &format!("skill.{}.{}.{}.description", scope, id, lvl))
                });
            let from_i18n = hit
                .and_then(|m| {
                    if lang == "chinese" {
                        m.get("chinese").cloned()
                            .or_else(|| m.get("auto").cloned())
                            .or_else(|| m.get("english").cloned())
                            .or_else(|| m.values().next().cloned())
                    } else {
                        m.get("english").cloned()
                            .or_else(|| m.get("auto").cloned())
                            .or_else(|| m.get("chinese").cloned())
                            .or_else(|| m.values().next().cloned())
                    }
                })
                .unwrap_or_default();
            if !from_i18n.is_empty() {
                return from_i18n;
            }
            crate::modcache::i18n::inline_text_for_lang(
                obj.get(lvl).and_then(|x| x.get("description")),
                lang,
            )
        };
        skills.push(json!({
            "id": id,
            "name": skill_name_obj,
            "levels": {
                "basic": { "chinese": lvl_desc("basic", "chinese"), "english": lvl_desc("basic", "english") },
                "advanced": { "chinese": lvl_desc("advanced", "chinese"), "english": lvl_desc("advanced", "english") },
                "expert": { "chinese": lvl_desc("expert", "chinese"), "english": lvl_desc("expert", "english") },
            },
            "image": img,
        }));
    }
    // 原版技能注入（VCMI 技能列表 = 原版 + mod 自定义）：
    // 名称/等级描述来自 SSTRAITS.TXT（CSkillHandler.cpp:174），index 来自 config/skills.json，
    // 图标 SECSK32 帧 = index*3+3（WikiWindow.cpp:652）
    // 仅对原版 core 伪模组注入：builtin 技能收敛到 core 一份（不再逐 mod 复制，
    // 前端 spellMap 按 id 去重、mod 自定义优先于原版）。
    if mod_id == "core" {
        let mut seen: std::collections::HashSet<String> = skills
            .iter()
            .filter_map(|s| s.get("id").and_then(|x| x.as_str()).map(|x| x.to_string()))
            .collect();
        for (sid, ix) in skill_index_map {
            if seen.contains(sid)
                || seen
                    .iter()
                    .any(|id| id.ends_with(&format!(":{sid}")) || id.ends_with(&format!(".{sid}")))
            {
                continue;
            }
            // 名称/三级描述统一取自 h3-core（`skill.core.<id>.*`）；
            // index 仍用于 SECSK32 图标帧号（来自 JSON 配置，非文本）。
            // **中英分列**：h3-core 文案表按语言分文件（english.skills.json / chinese.skills.json），
            // 键的 map 同时含 chinese/english；此前只取单值（中文优先）写进两个语言键，
            // 导致英文界面下原版技能名/描述显示中文。
            let pair = |sub: &str| -> (String, String) {
                let m = i18n_get_multi(&i18n, &format!("skill.core.{}.{}", sid, sub));
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
            let lv: Vec<(String, String)> = ["basic", "advanced", "expert"]
                .iter()
                .map(|sub| pair(sub))
                .collect();
            let icon = extract_game_def_frame_and_save(
                game_dir,
                cache_root,
                mod_id,
                "SECSK32",
                0,
                (*ix as usize) * 3 + 3,
                "secsk32",
                sid,
            );
            let auto = if name_zh.is_empty() { name_en.clone() } else { name_zh.clone() };
            skills.push(json!({
                "id": sid,
                "name": { "chinese": name_zh, "english": name_en, "auto": auto },
                "levels": {
                    "basic": { "chinese": lv[0].0, "english": lv[0].1 },
                    "advanced": { "chinese": lv[1].0, "english": lv[1].1 },
                    "expert": { "chinese": lv[2].0, "english": lv[2].1 },
                },
                "levelDescriptions": [lv[0].0.clone(), lv[1].0.clone(), lv[2].0.clone()],
                "image": icon,
                "builtin": true,
            }));
            seen.insert(sid.clone());
        }
    }

    skills
}
