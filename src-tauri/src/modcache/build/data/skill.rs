//! 技能（skill）缓存构建：名称 / 描述 / 图标 / 等级 / 属性。
//!
//! 由 `build_mod_wiki_once` 的「技能」节原样搬出（S3）；参数与原局部变量同名，节内容零改动。

use std::collections::HashMap;
use std::path::Path;

use serde_json::{json, Value};

use crate::modcache::assets::{extract_game_def_frame_and_save, extract_icon_and_save};
use crate::modcache::collect::collect_config_jsons_merged;
use crate::modcache::i18n::{
    i18n_get_multi, i18n_mod_scope, name_obj_multi_key, name_obj_with_inline, obj_from_lang_map,
};
use crate::vcmi::h3core::I18nMap;

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
        // 图标来源：VCMI `skill.json` schema 的 `images` 全集 =
        // `small`(32×32) / `medium`(44×44) / `large`(82×93) / `scenarioBonus`(58×64)，
        // 挂在 `basic` / `advanced` / `expert` **任一级**下（schema 里 required 四个都有，
        // 但实测大量 mod 只写其中一两个，且只挂在 basic 之外的档位）。
        //
        // ⚠️ 此前硬取 `basic.images.medium` 单一路径：medium 缺失、或图标只写在
        // advanced/expert 档时 → `image` 恒为 null（用户报"有些技能也是没有图片"）。
        // 口径：先按档位（basic → advanced → expert），档内按尺寸（medium → large → small
        // → scenarioBonus）——medium 是技能栏标准尺寸，取不到再退而求其次。
        let img = (["basic", "advanced", "expert"])
            .iter()
            .find_map(|lvl| {
                obj.get(*lvl)
                    .and_then(|b| b.get("images"))
                    .and_then(|i| {
                        i.get("medium")
                            .or_else(|| i.get("large"))
                            .or_else(|| i.get("small"))
                            .or_else(|| i.get("scenarioBonus"))
                    })
                    .and_then(|v| v.as_str())
            })
            .and_then(|r| {
                extract_icon_and_save(&p, game_dir, cache_root, mod_id, "skill", &id, Some(r))
            });
        // **内部记账技能不展示**（2026-10-16）：HotA / 潮战的「符文等级计数器」
        // （`RUNE_LEVEL_COUNTER` / `YETI_RUNE_LEVEL_COUNTER` / `APE_RUNE_LEVEL_COUNTER`）
        // 三档都没有 `images`（VCMI 侧 `RenderHandler.cpp:496-501` 空名直接跳过注册 →
        // 渲染时按 `index*3+3` 取不到帧 → **空图标**），也没有任何语言的名称
        // （列表里只剩裸 id），描述原文更是写着"If this is shown, it's a bug with the …"。
        //
        // VCMI schema（`config/schemas/skill.json`）**没有** `hidden` 字段，故不存在
        // "VCMI 官方过滤"可对齐；这里用「无图标 **且** 无名称」双条件兜底 ——
        // 只缺图标的技能仍保留（VCMI 也照样列出来，只是图标空白），避免误伤。
        let has_i18n_name = i18n_get_multi(&i18n, &format!("skill.{}.{}.name", scope, &id))
            .map(|m| m.values().any(|s| !s.trim().is_empty()))
            .unwrap_or(false);
        let has_inline_name = match obj.get("name") {
            Some(Value::String(s)) => !s.trim().is_empty(),
            Some(Value::Object(o)) => o
                .values()
                .any(|x| x.as_str().map(|s| !s.trim().is_empty()).unwrap_or(false)),
            _ => false,
        };
        if img.is_none() && !has_i18n_name && !has_inline_name {
            continue;
        }
        // 名称：i18n 键 → 配置内嵌文本（部分 mod 把技能名直接写在配置里）→ id
        let skill_name_obj = name_obj_with_inline(
            &i18n,
            &format!("skill.{}.{}.name", scope, id),
            obj.get("name"),
            &id,
        );
        // VCMI 技能键：`skill.<scope>.<id>.description.<basic|advanced|expert>`。
        // 此前拼成 `...<lvl>.description`，**模组技能的三级描述因此全部落空**
        // （实测 `skill.andruids-expansion.initiative.initiative.description.advanced`）；
        // 旧写法保留为回退，兼容个别写反的 mod。
        // 第三路：**配置内嵌** `basic/advanced/expert.description`（schema 允许；实测各 78 个配置，
        // 作者把描述直接写在技能配置里）——审计器（scripts/wiki-audit.mjs）发现。
        // 走多语言对象组装（两条 i18n 键 + 内嵌文本），缺中文就不写 `chinese`，
        // 由前端 `pickName` 回退 english；不再按语言取单值再写两个键（那会伪造中文副本）。
        let lvl_obj = |lvl: &str| -> Value {
            name_obj_multi_key(
                &i18n,
                &[
                    format!("skill.{}.{}.description.{}", scope, id, lvl),
                    // 旧写法（键序写反）兼容，个别 mod 仍是这种形态
                    format!("skill.{}.{}.{}.description", scope, id, lvl),
                ],
                obj.get(lvl).and_then(|x| x.get("description")),
                "",
            )
        };
        skills.push(json!({
            "id": id,
            "name": skill_name_obj,
            "levels": {
                "basic": lvl_obj("basic"),
                "advanced": lvl_obj("advanced"),
                "expert": lvl_obj("expert"),
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
            // **真实语言直落**：h3-core 文案表按语言分文件（english.skills.json /
            // chinese.skills.json），表里有什么语言就写什么语言；缺中文不拿英文冒充，
            // 前端 `pickName` 回退 english。
            let name_obj = obj_from_lang_map(
                i18n_get_multi(&i18n, &format!("skill.core.{}.name", sid)),
                "",
            );
            if name_obj.as_object().map(|o| o.is_empty()).unwrap_or(true) {
                continue;
            }
            let lvl_obj = |sub: &str| {
                obj_from_lang_map(
                    i18n_get_multi(&i18n, &format!("skill.core.{}.{}", sid, sub)),
                    "",
                )
            };
            let lv = [lvl_obj("basic"), lvl_obj("advanced"), lvl_obj("expert")];
            // 纯字符串版（英雄页对非本 mod 技能的直接显示回退）：中英优先，不带语言键
            let lv_text = |o: &Value| -> String {
                o.get("chinese")
                    .or_else(|| o.get("english"))
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string()
            };
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
            skills.push(json!({
                "id": sid,
                "name": name_obj,
                "levels": {
                    "basic": lv[0],
                    "advanced": lv[1],
                    "expert": lv[2],
                },
                "levelDescriptions": [lv_text(&lv[0]), lv_text(&lv[1]), lv_text(&lv[2])],
                "image": icon,
                "builtin": true,
            }));
            seen.insert(sid.clone());
        }
    }

    skills
}

/// 技能**内部记账条目**回归（2026-10-16）：无图标 **且** 无名称的技能不进列表。
///
/// 实测形态：HotA / 潮战的符文等级计数器（`RUNE_LEVEL_COUNTER` 等）——三档都没有
/// `images`，也没有任何语言的名称，列表里只剩一个裸 id 与一张空图标。
/// 只缺图标的技能（VCMI 也会照样列出，只是图标空白）**必须保留**，不得误伤。
#[cfg(test)]
mod visibility_tests {
    use super::*;
    use std::fs;

    fn build(cfg: &str) -> Vec<String> {
        let root = std::env::temp_dir().join("vcmi_fv_skill_visibility");
        let _ = fs::remove_dir_all(&root);
        let dir = root.join("Content").join("config").join("skills");
        fs::create_dir_all(&dir).expect("建目录");
        fs::write(dir.join("skills.json"), cfg).expect("写配置");
        let cache = root.join("cache");
        fs::create_dir_all(&cache).unwrap();
        let i18n: I18nMap = HashMap::new();
        let idx: HashMap<String, i64> = HashMap::new();
        let out = build_skills(&root, "t", &cache, Path::new(""), &i18n, &idx);
        let _ = fs::remove_dir_all(&root);
        out.iter()
            .filter_map(|s| s.get("id").and_then(|v| v.as_str()).map(|x| x.to_string()))
            .collect()
    }

    #[test]
    fn iconless_and_nameless_skill_is_hidden() {
        let ids = build(
            r#"{
                "RUNE_LEVEL_COUNTER": { "basic": { "effects": {} } },
                "namedNoIcon":       { "name": "吸血", "basic": { "effects": {} } },
                "inlineZhNoIcon":    { "name": { "chinese": "活力再生" } }
            }"#,
        );
        assert!(
            !ids.iter().any(|i| i == "RUNE_LEVEL_COUNTER"),
            "无图标且无名称的记账技能不应收录：{ids:?}"
        );
        assert!(
            ids.iter().any(|i| i == "namedNoIcon"),
            "有名称、只缺图标的技能必须保留（VCMI 也照样列出）：{ids:?}"
        );
        assert!(
            ids.iter().any(|i| i == "inlineZhNoIcon"),
            "内嵌中文名同样算有名称：{ids:?}"
        );
    }
}
