//! 英雄（hero）缓存构建：属性 / 头像 / 特长 / 初始部队 / 所属城镇。
//!
//! 由 `build_mod_wiki_once` 的「英雄」节原样搬出（S3）；参数与原局部变量同名，节内容零改动。

use std::collections::HashMap;
use std::path::Path;

use serde_json::{json, Value};

use crate::vcmi::gamedata::SPELL_ID_TABLE;
use crate::vcmi::h3core::{h3_hero_stats, I18nMap};
use crate::vcmi::h3core::h3_hero_class_stats;
use crate::vcmi::town::{read_game_root_configs, TownConfig};
use crate::modcache::assets::{
    extract_game_def_frame_and_save, extract_icon_and_save,
    extract_portrait_frame_mod, resolve_specialty_icon_mod,
};
use crate::modcache::collect::collect_config_jsons_merged;
use crate::modcache::i18n::{i18n_get_multi, i18n_mod_scope, name_obj_en_zhcn, name_obj_with_inline};

pub(crate) fn build_heroes(
    p: &Path,
    mod_id: &str,
    cache_root: &Path,
    game_dir: &Path,
    configs: &[TownConfig],
    i18n: &I18nMap,
    is_core: bool,
    core_faction: Option<&str>,
    raw_cfgs: &serde_json::Map<String, Value>,
    skill_index_map: &HashMap<String, i64>,
    hero_class_index_map: &HashMap<String, i64>,
) -> Vec<Value> {
    // heroClass affinity（might/magic）：wiki 英雄表按此划分两个表（对齐 VCMI addHeroesTable）；
    // 同时记录 class → faction，用于推导英雄阵营（alignment 带色副标题）。
    // 数据源用归一 rawConfigs（HuaXia 等职业配置在 config/<mod>/classes/，无标准 heroClasses 目录）
    let mut hero_class_affinity: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut hero_class_faction: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    // classId → (attack, defense, spellPower, knowledge)
    let mut hero_class_prim: std::collections::HashMap<String, (i64, i64, i64, i64)> =
        std::collections::HashMap::new();
    // classId → 职业内嵌名：部分 mod 不写翻译文件，职业名只写在 `heroClasses/*.json`
    // （此前未读 → 英雄表"职业"列回落成 class_id）
    let mut hero_class_names: std::collections::HashMap<String, Value> =
        std::collections::HashMap::new();
    // 原版英雄职业四维：HCTRAITS.TXT 已随 h3-core 烘入 heroClasses.json；
    // 用 heroClass index 做桥接，使 core 英雄在 VCMI 未声明 primarySkills 时仍有基础值。
    let h3_class_stats = h3_hero_class_stats();
    for (cid, idx) in hero_class_index_map {
        if let Some(stats) = h3_class_stats
            .as_ref()
            .and_then(|m| m.get(&format!("class{}", idx)))
        {
            hero_class_prim.insert(cid.clone(), *stats);
        }
    }
    for (cid, co) in raw_cfgs
        .get("heroClasses")
        .and_then(|h| h.as_object())
        .map(|o| o.iter().map(|(k, v)| (k.clone(), v.clone())).collect::<Vec<_>>())
        .unwrap_or_default()
    {
        if let Some(a) = co.get("affinity").and_then(|x| x.as_str()) {
            hero_class_affinity.insert(cid.clone(), a.to_string());
        }
        if let Some(f) = co.get("faction").and_then(|x| x.as_str()) {
            // faction 可能是 "<scope>:<id>" 形式，取尾段与本地 faction_id 匹配
            let short = f.rsplit(':').next().unwrap_or(f).to_string();
            hero_class_faction.insert(cid.clone(), short);
        }
        if let Some(n) = co.get("name") {
            hero_class_names.insert(cid.clone(), n.clone());
        }
        // 初始四主属性：多数英雄不在自身配置里声明，而是继承 heroClass.primarySkills
        // （VCMI 拼写为 defence / spellpower，此处统一成本缓存的 defense / spellPower）。
        //
        // ⚠️ 仅当职业**显式声明** primarySkills 时才覆盖：原版增强版 `config/heroClasses.json`
        // 只含 index/faction/affinity 等，并**不含** primarySkills。若像旧实现那样无条件
        // `unwrap_or(0)`，会把上方用 h3-core（HCTRAITS.TXT）桥接得到的正确四维清成 0/0/0/0
        // —— 这正是原版英雄四维全 0 的根因（约克/野蛮人等）。缺失字段保留桥接基线值。
        if let Some(ps) = co.get("primarySkills").filter(|p| p.is_object()) {
            let base = hero_class_prim.get(&cid).copied().unwrap_or((0, 0, 0, 0));
            let gp = |k: &str| -> Option<i64> { ps.get(k).and_then(|v| v.as_i64()) };
            hero_class_prim.insert(
                cid,
                (
                    gp("attack").unwrap_or(base.0),
                    gp("defence").or_else(|| gp("defense")).unwrap_or(base.1),
                    gp("spellpower").or_else(|| gp("spellPower")).unwrap_or(base.2),
                    gp("knowledge").unwrap_or(base.3),
                ),
            );
        }
    }
    // faction_id → alignment（供英雄副标题阵营着色）
    let faction_alignment: std::collections::HashMap<String, String> = configs
        .iter()
        .map(|c| (c.faction_id.clone(), c.alignment.clone()))
        .collect();
    // 次级技能名/等级描述、原版法术名/等级/各级描述统一取自打包的 h3-core 文案表
    // （`skill.core.<id>.*`、`spell.core.<id>.*`），不再运行时解码 SSTRAITS/SPTRAITS.TXT。
    // skill_index_map 仍须保留：SECSK32 技能图标帧号 = index*3+3，该 index 来自 JSON 配置。
    let mut heroes = Vec::new();
    // 原版英雄四维 / 初始部队：HOTRAITS.TXT 数据已随 h3-core 烘入 heroes.json，
    // core 分支从此取，避免 hero JSON / heroClass.primarySkills 缺失导致全 0。
    let hero_core_stats = h3_hero_stats();
    // 原版英雄不在任何 mod 内，位于游戏根 config/heroes；仅对 core 系并入，
    // 使原版城镇的英雄区能命中 modCache.heroes（头像经游戏根 TPLICHxx.DEF 回退出图）。
    // 原版英雄无 faction 字段（只有 class），但按阵营分文件存放，故同样以文件作用域切分。
    let mut hero_jsons: Vec<(String, Value)> = collect_config_jsons_merged(&p, "heroes");
    if is_core {
        for (id, v) in read_game_root_configs(game_dir, "heroes", core_faction) {
            if !hero_jsons.iter().any(|(i, _)| i == &id) {
                hero_jsons.push((id, v));
            }
        }
    }
    for (id, obj) in hero_jsons {
        // 图标索引（VCMI: iconIndex，注册 UN32/UN44、PortraitsSmall 帧号）
        let image_index = obj
            .get("iconIndex")
            .and_then(|v| v.as_i64())
            .or_else(|| obj.get("index").and_then(|v| v.as_i64()))
            .unwrap_or(-1);
        let img = extract_icon_and_save(&p, game_dir, cache_root, mod_id, "hero", &id, obj.get("images").and_then(|g| g.get("large")).and_then(|v| v.as_str()))
            // 原版 H3 英雄不在 JSON 中声明 images.large（VCMI 运行期按 imageIndex 从 PortraitsLarge 推算头像）。
            // 归位到 core.<faction>/images/hero/<id>.webp（中性归入 core/images/hero/），
            // 与第三方 town mod 的 images/hero/<id>.webp 一致；不再引用全局 _shared/portraitsLarge。
            .or_else(|| {
                if image_index < 0 {
                    return None;
                }
                extract_portrait_frame_mod(game_dir, cache_root, mod_id, &id, image_index)
            });
        let scope = i18n_mod_scope(mod_id);
        // 内嵌文本回退来源：标准写法在 `texts.name` / `texts.biography`；
        // 部分中文 mod 直接写顶层 `name` / `biography`（实测 old-chinese-mods 系列，
        // 此前只读 `texts.*` → 取不到 → 英雄名回落成 id）。
        let inline_name = obj
            .get("texts")
            .and_then(|t| t.get("name"))
            .or_else(|| obj.get("name"));
        let inline_bio = obj
            .get("texts")
            .and_then(|t| t.get("biography"))
            .or_else(|| obj.get("biography"));
        let name_obj = name_obj_with_inline(&i18n, &format!("hero.{}.{}.name", scope, id), inline_name, &id);
        let bio_obj = name_obj_with_inline(&i18n, &format!("hero.{}.{}.biography", scope, id), inline_bio, "");
        // VCMI 文本键：hero.<mod>.<id>.specialty.name / description / tooltip
        let spec_obj = name_obj_en_zhcn(&i18n, &format!("hero.{}.{}.specialty.name", scope, id), "");
        let spec_desc_obj = name_obj_en_zhcn(&i18n, &format!("hero.{}.{}.specialty.description", scope, id), "");
        let spec_tooltip_obj = name_obj_en_zhcn(&i18n, &format!("hero.{}.{}.specialty.tooltip", scope, id), "");
        // 特长附加能力文本（schema `hero.json`：`specialty.bonuses.<key>.name/description`，
        // 实测 22+ 配置，如 `specialty.bonuses.heroAssist6.description`）——
        // 信息保真：进缓存供未来展示/检索，此前完全未读。
        let spec_bonuses: Vec<Value> = obj
            .get("specialty")
            .and_then(|s| s.get("bonuses"))
            .and_then(|b| b.as_object())
            .map(|m| {
                m.iter()
                    .map(|(k, v)| {
                        json!({
                            "key": k,
                            "name": name_obj_with_inline(
                                &i18n,
                                &format!("hero.{}.{}.specialty.bonuses.{}.name", scope, id, k),
                                v.get("name"),
                                "",
                            ),
                            "description": name_obj_with_inline(
                                &i18n,
                                &format!("hero.{}.{}.specialty.bonuses.{}.description", scope, id, k),
                                v.get("description"),
                                "",
                            ),
                        })
                    })
                    .filter(|e| {
                        // 只为"带文本"的条目输出：多数 specialty.bonuses 是纯结构定义
                        // （type/subtype/val），全量输出会让缓存膨胀且无展示价值
                        let has = |v: &Value| {
                            v.as_object()
                                .map(|o| {
                                    o.values().any(|x| {
                                        x.as_str().map(|s| !s.trim().is_empty()).unwrap_or(false)
                                    })
                                })
                                .unwrap_or(false)
                        };
                        has(&e["name"]) || has(&e["description"])
                    })
                    .collect()
            })
            .unwrap_or_default();
        // 特长图标（VCMI: images.specialtySmall/specialtyLarge；"default" → UN32/UN44 帧）
        // 特长图标：第三方 mod 显式声明 specialtyLarge/Small；原版 H3 英雄 JSON 无 images 字段，
        // 缺失时回退 VCMI 默认行为（"default" → 游戏 UN32/UN44.DEF 帧 = imageIndex），
        // 仅对原版（is_core）启用，避免改变第三方 mod 的既有表现。
        let spec_large_raw = obj
            .get("images")
            .and_then(|g| g.get("specialtyLarge"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| if is_core { "default".to_string() } else { String::new() });
        let spec_small_raw = obj
            .get("images")
            .and_then(|g| g.get("specialtySmall"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| if is_core { "default".to_string() } else { String::new() });
        let specialty_icon = resolve_specialty_icon_mod(&p, game_dir, cache_root, mod_id, &id, &spec_large_raw, image_index, false);
        let specialty_icon_small = resolve_specialty_icon_mod(&p, game_dir, cache_root, mod_id, &id, &spec_small_raw, image_index, true);
        let gen = obj.get("general").and_then(|g| g.as_object());
        let class_id = obj.get("class").and_then(|v| v.as_str()).unwrap_or("").to_string();
        // 职业名：原版按 config/heroClasses.json 的 index 命中 h3-core 中文名；
        // 第三方 mod 直接用 class_id 在自己的翻译 / 配置里查。
        let hero_class_key = if scope == "core" {
            hero_class_index_map
                .get(&class_id)
                .map(|ix| format!("heroClass.core.class{}.name", ix))
                .unwrap_or_else(|| format!("heroClass.core.{}.name", class_id))
        } else {
            format!("heroClass.{}.{}.name", scope, class_id)
        };
        let class_name = name_obj_with_inline(
            &i18n,
            &hero_class_key,
            hero_class_names.get(&class_id),
            &class_id,
        );
        // 优先英雄自身声明 → general 段 → 继承 heroClass.primarySkills（VCMI 配置）。
        // 若 VCMI 配置未声明，再回退到 h3-core 从 HCTRAITS.TXT 提取的职业初始四维。
        let h3_stats = hero_core_stats.as_ref().and_then(|m| m.get(&id));
        let get_prim = |k: &str| -> i64 {
            obj.get(k)
                .and_then(|v| v.as_i64())
                .or_else(|| gen.and_then(|g| g.get(k)).and_then(|v| v.as_i64()))
                .or_else(|| match k {
                    "attack" => hero_class_prim.get(&class_id).map(|x| x.0),
                    "defense" => hero_class_prim.get(&class_id).map(|x| x.1),
                    "spellPower" => hero_class_prim.get(&class_id).map(|x| x.2),
                    "knowledge" => hero_class_prim.get(&class_id).map(|x| x.3),
                    _ => None,
                })
                .unwrap_or(0)
        };
        let army = obj
            .get("army")
            .and_then(|a| a.as_array())
            .cloned()
            .or_else(|| obj.get("startingArmy").and_then(|a| a.as_array()).cloned())
            .or_else(|| {
                // core 英雄从 h3-core 取 HOTRAITS 初始部队（creature 为 H3 原名）。
                h3_stats
                    .and_then(|v| v.get("army"))
                    .and_then(|a| a.as_array())
                    .cloned()
            })
            .unwrap_or_default();
        // 初始法术 + 魔法书（VCMI CHeroHandler.cpp:130-132）：
        // `spellbook` 存在即视为有魔法书（哪怕是空数组，0 个法术），其项为初始法术
        let spells_list: Vec<String> = obj
            .get("spellbook")
            .and_then(|s| s.as_array())
            .map(|a| a.iter().filter_map(|x| x.as_str().map(|s| s.to_string())).collect())
            .unwrap_or_default();
        let have_spellbook = obj.get("spellbook").map(|v| !v.is_null()).unwrap_or(false);
        // 初始法术详情：identifier → index（SPELL_ID_TABLE，仅用于图标帧号），
        // 名称/等级/各级描述统一取自 h3-core 文案表（`spell.core.<id>.*`），
        // 不再运行时解码 SPTRAITS.TXT。原版法术不在 mod 缓存里，须在英雄条目内预解析。
        let spell_details: Vec<Value> = spells_list
            .iter()
            .map(|sid| {
                let idx_opt = SPELL_ID_TABLE
                    .iter()
                    .find(|(id, _)| *id == sid.as_str())
                    .map(|(_, i)| *i);
                let text = |sub: &str| -> String {
                    i18n_get_multi(&i18n, &format!("spell.core.{}.{}", sid, sub))
                        .and_then(|m| m.get("chinese").or_else(|| m.get("english")).cloned())
                        .unwrap_or_default()
                };
                let name = {
                    let n = text("name");
                    if n.is_empty() { sid.clone() } else { n }
                };
                let level = text("level").parse::<i64>().unwrap_or(1);
                json!({
                    "id": sid,
                    "index": idx_opt.unwrap_or(-1),
                    "name": { "chinese": name, "english": name, "auto": name },
                    // 法术书图标 SPELLBON 帧 = 法术 index
                    "iconIndex": idx_opt.unwrap_or(-1),
                    "level": level,
                    // 初/中/高级描述
                    "levelDescriptions": json!(vec![text("basic"), text("advanced"), text("expert")]),
                })
            })
            .collect();
        // 技能名：优先 h3-core（`skill.core.<id>.name`），未命中再回落 mod 自身 i18n
        let skill_name_obj = |sid: &str| -> Value {
            let core = name_obj_en_zhcn(&i18n, &format!("skill.core.{}.name", sid), "");
            let hit = ["chinese", "english"].iter().any(|k| {
                core.get(k)
                    .and_then(|v| v.as_str())
                    .map(|s| !s.is_empty())
                    .unwrap_or(false)
            });
            if hit {
                core
            } else {
                name_obj_en_zhcn(&i18n, &format!("skill.{}.{}.name", i18n_mod_scope(mod_id), sid), sid)
            }
        };
        // 技能各等级描述（h3-core 的 basic / advanced / expert）
        let skill_level_desc = |sid: &str| -> Option<Vec<String>> {
            let pick = |sub: &str| -> String {
                i18n_get_multi(&i18n, &format!("skill.core.{}.{}", sid, sub))
                    .and_then(|m| m.get("chinese").or_else(|| m.get("english")).cloned())
                    .unwrap_or_default()
            };
            let v = vec![pick("basic"), pick("advanced"), pick("expert")];
            if v.iter().all(|s| s.is_empty()) {
                None
            } else {
                Some(v)
            }
        };
        // 初始技能（VCMI CHeroHandler.cpp:110 用 `skills` 数组：[{skill, level}]）
        let mut secondary: Vec<Value> = Vec::new();
        if let Some(arr) = obj
            .get("skills")
            .or_else(|| obj.get("secondarySkills"))
            .and_then(|s| s.as_array())
        {
            for s in arr {
                if let Some(o) = s.as_object() {
                    let sid = o.get("skill").and_then(|x| x.as_str()).unwrap_or("").to_string();
                    // VCMI CHeroHandler.cpp:112：level 为字符串（none/basic/advanced/expert，
                    // 见 StringConstants.h:49），缺失或无效则**跳过该技能**（find 失败 ⇒ 不入列表）
                    let lvl = match o.get("level") {
                        Some(Value::String(st)) => match st.as_str() {
                            "none" => Some(0i64),
                            "basic" => Some(1),
                            "advanced" => Some(2),
                            "expert" => Some(3),
                            _ => None,
                        },
                        Some(Value::Number(n)) => n.as_i64().filter(|v| (0..=3).contains(v)),
                        _ => None,
                    };
                    let Some(lvl) = lvl else { continue; };
                    if !sid.is_empty() {
                        // 技能图标：VCMI WikiWindow.cpp:652 —— SECSK32 帧 = index*3+3（原版技能）
                        let sk_icon = skill_index_map.get(&sid).and_then(|ix| {
                            extract_game_def_frame_and_save(
                                game_dir,
                                cache_root,
                                mod_id,
                                "SECSK32",
                                0,
                                (*ix as usize) * 3 + 3,
                                "secsk32",
                                &sid,
                            )
                        });
                        secondary.push(json!({
                            "skill": sid,
                            "level": lvl,
                            "name": skill_name_obj(&sid),
                            "levelDescriptions": skill_level_desc(&sid),
                            "icon": sk_icon,
                        }));
                    }
                } else if let Some(sid) = s.as_str() {
                    let sk_icon = skill_index_map.get(sid).and_then(|ix| {
                        extract_game_def_frame_and_save(
                            game_dir,
                            cache_root,
                            mod_id,
                            "SECSK32",
                            0,
                            (*ix as usize) * 3 + 3,
                            "secsk32",
                            sid,
                        )
                    });
                    secondary.push(json!({
                        "skill": sid,
                        "level": 1,
                        "name": skill_name_obj(sid),
                        "levelDescriptions": skill_level_desc(sid),
                        "icon": sk_icon,
                    }));
                }
            }
        }
        heroes.push(json!({
            "id": id,
            "name": name_obj,
            "biography": bio_obj,
            "specialty": spec_obj,
            "specialtyDescription": spec_desc_obj,
            "specialtyTooltip": spec_tooltip_obj,
            // 特长附加能力（配置内嵌文本/翻译键，见上方提取）
            "specialtyBonuses": spec_bonuses,
            "specialtyIcon": specialty_icon,
            "specialtyIconSmall": specialty_icon_small,
            "imageIndex": image_index,
            // heroClass affinity：might/magic（wiki 英雄表分组依据）
            "affinity": hero_class_affinity.get(&class_id).cloned().unwrap_or_default(),
            // 阵营 id（由 heroClass.faction 推导）：百科「英雄按阵营筛选」用。
            // `alignment` 只表达善恶、无法区分具体阵营，故单独输出 id；
            // 显示名由 cache.json 顶层 `factionNames` 映射解析（见 build/mod.rs）
            "factionId": hero_class_faction.get(&class_id).cloned().unwrap_or_default(),
            // 阵营（由 heroClass.faction 推导，用于副标题带色，对齐 VCMI wiki 英雄页副标题）
            "alignment": hero_class_faction
                .get(&class_id)
                .and_then(|f| faction_alignment.get(f))
                .cloned()
                .unwrap_or_default(),
            // special 英雄不入 wiki 列表（对齐 VCMI addHeroesTable 过滤）
            "special": obj.get("special").and_then(|v| v.as_bool()).unwrap_or(false),
            "class": class_id,
            "className": class_name,
            "female": obj.get("female").and_then(|v| v.as_bool()).unwrap_or(false),
            "attack": get_prim("attack"),
            "defense": get_prim("defense"),
            "spellPower": get_prim("spellPower"),
            "knowledge": get_prim("knowledge"),
            "secondarySkills": secondary,
            "army": army,
            "spells": spells_list,
            "spellDetails": spell_details,
            "haveSpellBook": have_spellbook,
            "image": img,
        }));
    }

    heroes
}
