//! 法术（spell）缓存构建：名称 / 描述 / 图标 / 等级 / 属性。
//!
//! 由 `build_mod_wiki_once` 的「法术」节原样搬出（S3）；参数与原局部变量同名，节内容零改动。

use std::path::Path;

use serde_json::{json, Value};

use crate::modcache::assets::extract_icon_and_save;
use crate::modcache::collect::collect_config_jsons_merged;
use crate::modcache::i18n::{
    i18n_get_multi, i18n_mod_scope, name_obj_multi_key, name_obj_with_inline, obj_from_lang_map,
};
use crate::vcmi::gamedata::SPELL_ID_TABLE;
use crate::vcmi::h3core::I18nMap;

/// 原版法术的 SPELLBON 图标帧路径（`core/images/spellbon/<idx>.webp`）。
///
/// 产物由 [`crate::modcache::global::extract_shared_ui_icons`] 的 SPELLBON 段统一生成
/// （`group 0` 全部帧按序号落盘），与 `build_spells` 的 core 分支同源。
///
/// 只认 `SPELL_ID_TABLE`（1..78，必然在帧范围内）：**不用配置里的 `iconIndex`** ——
/// 实测大量 mod 法术写 `iconIndex: -1`（无图标），而越界帧号只会让前端拿到 404。
fn core_spellbon_image(id: &str) -> Option<String> {
    // 形如 `core:hypnotize` / `<scope>.hypnotize` 的跨 mod 覆盖键，取末段
    let bare = id.rsplit([':', '.']).next().unwrap_or(id);
    let idx = SPELL_ID_TABLE
        .iter()
        .find(|(name, _)| *name == bare)
        .map(|(_, i)| *i)?;
    Some(format!("core/images/spellbon/{}.webp", idx))
}

pub(crate) fn build_spells(
    p: &Path,
    mod_id: &str,
    cache_root: &Path,
    game_dir: &Path,
    i18n: &I18nMap,
) -> Vec<Value> {
    let mut spells = Vec::new();
    for (id, obj) in collect_config_jsons_merged(&p, "spells") {
        // **VCMI 可见性对齐**（2026-10-16 修正）：VCMI 的法术书与内建百科都不展示两类法术 ——
        // ① `"type": "ability"` → `CSpellHandler.cpp:213-217` 置 `creatureAbility=true`，
        //    过滤点 `client/windows/CSpellWindow.cpp:338`（连"显示全部"也不放行）、
        //    `client/windows/wiki/WikiModContent.cpp:88,337`（`!isSpecial() && !isCreatureAbility()`）；
        // ② `flags.special: true`（schema `config/schemas/spell.json:437-440`，"只能由 Bonus 授予"），
        //    过滤点同上（护城河 `moat` / `moatTrigger`、内部触发法术几乎都带它）。
        // 二者恰恰是"列表里有名无图"的主力：实测 592 条法术里 92 条无图，其中
        // 70 条是 ability、17 条是 special —— 它们**本来就没有图标资源**（配置里
        // `graphics` 为空或干脆不写），补齐图标无从谈起，正确做法是跟随 VCMI 不展示。
        // 综合判定见 `CSpell::isCommonHeroSpell()`（`lib/spells/CSpell.cpp:238-241`）。
        let is_creature_ability = obj.get("type").and_then(|v| v.as_str()) == Some("ability");
        let is_special = obj
            .get("flags")
            .and_then(|f| f.get("special"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if is_creature_ability || is_special {
            continue;
        }
        // 另两类**根本不是一个法术条目**的配置（同样不该进百科）：
        // ③ `hidden: true` —— 明确标注不展示的内部法术（实测 hota.Factory 的
        //    `heatStrokeBonusType.json`：`{"hidden": true, "description": "not shown"}`）；
        // ④ 缺 `school` / `level` / `name` 三要素的条目 —— 它们是**生物能力 / bonus 定义**，
        //    只是被放进了 `spells/` 目录（实测同目录的 `castNoTurn.json`：只有 id/type/
        //    parameters，是 CAST_SPELL 能力；`USED_HEATSTROKE_ABILITY` 同样如此）。
        //    VCMI 按 `vcmi:spell` schema 装载这类条目会失败或不可见，百科自然也不会有。
        let hidden = obj.get("hidden").and_then(|v| v.as_bool()).unwrap_or(false);
        let has_school = obj
            .get("school")
            .and_then(|s| s.as_object())
            .is_some_and(|m| !m.is_empty());
        let has_level = obj.get("level").is_some();
        let has_name =
            obj.get("name").is_some() || obj.get("text").and_then(|t| t.get("name")).is_some();
        if hidden || (!has_school && !has_level && !has_name) {
            continue;
        }
        let scope = i18n_mod_scope(mod_id);
        // 图标字段：**VCMI `spell.json` schema 的 graphics 全集**，按优先级取
        // （iconBook → iconScroll → iconEffect → iconImmune → iconScenarioBonus）。
        //
        // ⚠️ 此前只认 `iconBook → image → icon`：而 schema 里根本没有 `image`/`icon`，
        // 却有 `iconEffect` —— 实测 tidesOfWar（巨浪冲击 `con03eff` 等 63 处）、
        // the-great-expansion（absoluteDefense / onslaught / swiftStriker）都把图标写在
        // `iconEffect` 里 → 这批法术的 `image` 恒为 null（用户报"有些法术图片没有提取出来"）。
        // 末尾保留 `image`/`icon` 宽容兜底：个别 mod 不按 schema 写，别把它们改回归零。
        let g = obj.get("graphics");
        let icon_res = g
            .and_then(|x| {
                x.get("iconBook")
                    .or_else(|| x.get("iconScroll"))
                    .or_else(|| x.get("iconEffect"))
                    .or_else(|| x.get("iconImmune"))
                    .or_else(|| x.get("iconScenarioBonus"))
                    .or_else(|| x.get("image"))
                    .or_else(|| x.get("icon"))
            })
            .and_then(|v| v.as_str());
        let img = extract_icon_and_save(&p, game_dir, cache_root, mod_id, "spell", &id, icon_res)
            .or_else(|| {
                // mod **只覆盖原版法术的属性**（`core:<id>` 形式，不带 graphics）时，
                // 启动器的配置收集只看本 mod，看不到原版那份定义 → 此前恒为无图。
                // VCMI 客户端对这种法术仍用 SPELLBON 的 iconIndex 帧（原版法术本就
                // 没有 graphics，靠 index 取默认图标，见 `CSpellHandler` + 客户端
                // `AnimationPath::builtin("SPELLBON")`），故此处按同一口径回退。
                core_spellbon_image(&id)
            });
        // 名称/描述：i18n 键 → 配置内嵌文本（部分中文 mod 的 `spells/*.json` 直接写
        // `name: "护城河"`，此前不读 → 回落成法术 id）→ 兜底
        let spell_name_obj = name_obj_with_inline(
            &i18n,
            &format!("spell.{}.{}.name", scope, id),
            obj.get("name"),
            &id,
        );
        let spell_desc_obj = name_obj_with_inline(
            &i18n,
            &format!("spell.{}.{}.description", scope, id),
            obj.get("description"),
            "",
        );
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
                || seen.iter().any(|id| {
                    id.ends_with(&format!(":{identifier}"))
                        || id.ends_with(&format!(".{identifier}"))
                })
            {
                continue;
            }
            // 名称/等级/派系/各级描述统一取自 h3-core（`spell.core.<id>.*`）。
            // **真实语言直落**：h3-core 文案表按语言分文件（english.spells.json /
            // chinese.spells.json），键的 map 里有什么语言就写什么语言；缺中文不拿英文冒充，
            // 前端 `pickName` 回退 english。此前"按语言取单值再写两个键"会让
            // `chinese` 变成英文副本（数据失真）。
            let text = |sub: &str| -> String {
                i18n_get_multi(&i18n, &format!("spell.core.{}.{}", identifier, sub))
                    .and_then(|m| m.get("chinese").or_else(|| m.get("english")).cloned())
                    .unwrap_or_default()
            };
            let name_obj = obj_from_lang_map(
                i18n_get_multi(&i18n, &format!("spell.core.{}.name", identifier)),
                "",
            );
            if name_obj.as_object().map(|o| o.is_empty()).unwrap_or(true) {
                continue;
            }
            let level = text("level").parse::<i64>().unwrap_or(1);
            let schools: Vec<String> = text("schools")
                .split(',')
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect();
            // 分级描述改为多语言对象（与模组法术同形态；前端兼容字符串与对象两种形态）
            let level_obj = |sub: &str| {
                obj_from_lang_map(
                    i18n_get_multi(&i18n, &format!("spell.core.{}.{}", identifier, sub)),
                    "",
                )
            };
            let lv = [
                level_obj("basic"),
                level_obj("advanced"),
                level_obj("expert"),
            ];
            // 原版派系多语言名（h3-core 的 `spellSchool.core.<id>.name`：气系/火系/土系/水系）
            let core_school_names: Vec<Value> = schools
                .iter()
                .map(|s| {
                    name_obj_multi_key(&i18n, &[format!("spellSchool.core.{}.name", s)], None, s)
                })
                .collect();
            spells.push(json!({
                "id": identifier,
                "name": name_obj,
                "schools": schools,
                "schoolNames": core_school_names,
                "level": level,
                "iconIndex": idx,
                "levelDescriptions": {
                    "basic": lv[0],
                    "advanced": lv[1],
                    "expert": lv[2],
                },
                "image": format!("core/images/spellbon/{idx}.webp"),
                "builtin": true,
            }));
            seen.insert(identifier.to_string());
        }
    }

    spells
}

/// 法术**可见性**回归（2026-10-16）：VCMI 不展示的两类法术，缓存也不得收录。
///
/// 实测（572 mod 发行版）：592 条法术里 92 条"列表里有名无图"，其中 70 条
/// `type=ability`、17 条 `flags.special` —— 它们的配置里根本没有图标字段
/// （`graphics` 为空或缺失），不是"图标没提取出来"。
#[cfg(test)]
mod visibility_tests {
    use super::*;
    use std::collections::HashMap;
    use std::fs;

    fn build(cfg: &str) -> Vec<String> {
        let root = std::env::temp_dir().join("vcmi_fv_spell_visibility");
        let _ = fs::remove_dir_all(&root);
        let dir = root.join("Content").join("config").join("spells");
        fs::create_dir_all(&dir).expect("建目录");
        fs::write(dir.join("spells.json"), cfg).expect("写配置");
        let cache = root.join("cache");
        fs::create_dir_all(&cache).unwrap();
        let i18n: I18nMap = HashMap::new();
        let out = build_spells(&root, "t", &cache, Path::new(""), &i18n);
        let _ = fs::remove_dir_all(&root);
        out.iter()
            .filter_map(|s| s.get("id").and_then(|v| v.as_str()).map(|x| x.to_string()))
            .collect()
    }

    #[test]
    fn creature_ability_and_special_spells_are_hidden() {
        let ids = build(
            r#"{
                "moat":      { "type": "ability", "flags": { "special": true }, "name": "Ditch" },
                "moatTrig":  { "type": "ability", "name": "Ditch" },
                "onlySpec":  { "type": "combat", "flags": { "special": true }, "name": "X" },
                "iceShot":   { "type": "combat", "name": "Ice" },
                "adventure": { "type": "adventure", "name": "Adv" }
            }"#,
        );
        assert!(
            !ids.iter().any(|i| i == "moat"),
            "生物能力法术不得收录：{ids:?}"
        );
        assert!(
            !ids.iter().any(|i| i == "moatTrig"),
            "生物能力法术不得收录：{ids:?}"
        );
        assert!(
            !ids.iter().any(|i| i == "onlySpec"),
            "special 法术不得收录：{ids:?}"
        );
        assert!(
            ids.iter().any(|i| i == "iceShot"),
            "普通战斗法术应保留：{ids:?}"
        );
        assert!(
            ids.iter().any(|i| i == "adventure"),
            "冒险法术应保留：{ids:?}"
        );
    }
}
