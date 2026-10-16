//! 生物（creature）缓存构建：属性 / 图标 / 背景 / 招募建筑 / 战争机器关联。
//!
//! 由 `build_mod_wiki_once` 的「生物」节原样搬出（S3）；参数与原局部变量同名，节内容零改动。

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use crate::infra::json::parse_vcmi_json;
use crate::modcache::assets::{extract_game_def_frame_and_save_mod, extract_icon_and_save};
use crate::modcache::collect::{accumulate_full_cost, collect_config_jsons};
use crate::modcache::i18n::{
    ability_text_from_bonus_type, i18n_get_multi, i18n_mod_scope, name_desc_maps_from_keys,
    name_obj_multi_key, name_obj_with_inline,
};
use crate::vcmi::h3core::I18nMap;

/// 合并两组 `(名称表, 描述表)`：`over` 覆盖同语言，`over` 缺失处保留 `base`。
fn merge_pairs(
    base: Option<(HashMap<String, String>, HashMap<String, String>)>,
    over: Option<(HashMap<String, String>, HashMap<String, String>)>,
) -> Option<(HashMap<String, String>, HashMap<String, String>)> {
    match (base, over) {
        (None, x) => x,
        (x, None) => x,
        (Some((mut bn, mut bd)), Some((on, od))) => {
            for (k, v) in on {
                bn.insert(k, v);
            }
            for (k, v) in od {
                bd.insert(k, v);
            }
            Some((bn, bd))
        }
    }
}

/// 配置内嵌能力文本：`{"name": ..., "description": ...}`（字符串 / 多语言对象）
/// → `(名称表, 描述表)`（语言 → 文本），与 `ability_text_from_bonus_type` 同形态可拼接。
///
/// VCMI schema（`creature.json`）允许 ability 项自带 name/description（实测 122+ 配置，
/// 如 `abilities.fearless.description`）——此前未读，能力描述缺一路来源。
fn inline_ability_text(b: &Value) -> Option<(HashMap<String, String>, HashMap<String, String>)> {
    let pick = |v: Option<&Value>| -> HashMap<String, String> {
        let mut m = HashMap::new();
        match v {
            Some(Value::String(s)) if !s.trim().is_empty() => {
                m.insert("english".to_string(), s.clone());
            }
            Some(Value::Object(o)) => {
                for (k, v) in o {
                    if let Some(s) = v.as_str() {
                        if !s.trim().is_empty() {
                            m.insert(k.clone(), s.to_string());
                        }
                    }
                }
            }
            _ => {}
        }
        m
    };
    let names = pick(b.get("name"));
    let descs = pick(b.get("description"));
    if names.is_empty() && descs.is_empty() {
        None
    } else {
        Some((names, descs))
    }
}

pub(crate) fn build_creatures(
    p: &Path,
    mod_id: &str,
    cache_root: &Path,
    game_dir: &Path,
    i18n: &I18nMap,
    factions: &[Value],
    creature_dwellings: &mut HashMap<String, Value>,
    creature_jsons: &HashMap<String, Value>,
    upgraded_from: &HashMap<String, Vec<String>>,
) -> Vec<Value> {
    // 战争机器关联：神器配置的 `warMachine` 指向生物 id（VCMI CArtHandler.cpp:230-235），
    // 反查得到"生物 → 神器 id"，供英雄弹窗战争机器区识别（CHeroOverview.cpp:181 判空）。
    let mut war_machine_of: std::collections::HashMap<String, String> =
        std::collections::HashMap::new();
    for (aid, aobj) in collect_config_jsons(&p, "artifacts") {
        if let Some(cid) = aobj.get("warMachine").and_then(|v| v.as_str()) {
            war_machine_of.insert(cid.to_string(), aid);
        }
    }
    // 游戏核心神器（原版 ballista / firstAidTent / ammoCart 等战争机器）
    let core_artifacts = game_dir.join("config").join("artifacts.json");
    if let Ok(text) = fs::read_to_string(&core_artifacts) {
        if let Some(obj) = parse_vcmi_json(&text).as_object() {
            for (aid, aobj) in obj {
                if let Some(cid) = aobj.get("warMachine").and_then(|v| v.as_str()) {
                    war_machine_of
                        .entry(cid.to_string())
                        .or_insert_with(|| aid.clone());
                }
            }
        }
    }
    // 原版伪模组（core / core.<faction>）的资源需归位到各自模组目录而非全局 _shared；
    // 用 mod_id 约定判断（与 parse_core_mod 同源），避免改动函数签名。
    let is_core = mod_id == "core" || mod_id.starts_with("core.");

    let mut creatures = Vec::new();
    for (id, obj) in creature_jsons {
        // 生物小图标（对齐 VCMI wiki：CPRSMALL 帧 iconIndex，带浅色底图非透明抠图）。
        // 优先级：mod 自定义 graphics.iconSmall → 游戏 CPRSMALL[iconIndex]（公共素材 _shared/cprsmall）→ 无
        // VCMI CCreatureHandler.cpp:585 —— 配置未显式写 iconIndex 时
        // `iconIndex = 生物索引 + 2`（原版生物均走此分支）
        let icon_index = obj
            .get("iconIndex")
            .and_then(|v| v.as_i64())
            .or_else(|| obj.get("index").and_then(|v| v.as_i64()).map(|i| i + 2))
            .unwrap_or(-1);
        let img = extract_icon_and_save(
            &p,
            game_dir,
            cache_root,
            mod_id,
            "creature",
            &id,
            obj.get("graphics")
                .and_then(|g| g.get("iconSmall"))
                .and_then(|v| v.as_str()),
        )
        .or_else(|| {
            if icon_index < 0 {
                return None;
            }
            // 原版 H3 生物不在 JSON 声明 iconSmall，VCMI 运行期按 iconIndex 从 CPRSMALL.DEF 推算小图。
            // 归位到各自模组目录（core.<faction>/images/creature/<id>.webp，中性归入 core/images/creature/），
            // 不再混进全局 _shared/cprsmall，使原版城镇模组自包含、便于打包迁移。
            extract_game_def_frame_and_save_mod(
                game_dir,
                cache_root,
                mod_id,
                "CPRSMALL",
                0,
                icon_index as usize,
                "creature",
                &id,
            )
        })
        .or_else(|| {
            // 回退：advMap 小图
            extract_icon_and_save(
                &p,
                game_dir,
                cache_root,
                mod_id,
                "creature",
                &format!("{}-adv", id),
                obj.get("graphics")
                    .and_then(|g| g.get("map"))
                    .and_then(|v| v.as_str()),
            )
        });
        let img_large = extract_icon_and_save(
            &p,
            game_dir,
            cache_root,
            mod_id,
            "creature",
            &format!("{}-large", id),
            obj.get("graphics")
                .and_then(|g| g.get("iconLarge"))
                .and_then(|v| v.as_str()),
        )
        // 原版 H3 生物不在 JSON 中声明 iconLarge（VCMI 运行期按 iconIndex 从 TWCRPORT.DEF 推算大图），
        // 归位到 core.<faction>/images/creature/<id>-large.webp（中性归入 core/images/creature/），
        // 与第三方 town mod 的 images/creature/<id>-large.webp 一致。
        .or_else(|| {
            if icon_index < 0 {
                return None;
            }
            extract_game_def_frame_and_save_mod(
                game_dir,
                cache_root,
                mod_id,
                "TWCRPORT",
                0,
                icon_index as usize,
                "creature",
                &format!("{}-large", id),
            )
        });
        let scope = i18n_mod_scope(mod_id);
        // 内嵌名：VCMI 标准写法 `name: {singular, plural}`；同时兼容 `name: "字符串"` 与
        // 内嵌多语言对象 `{"chinese": ...}`（部分 mod 的写法）——此前只取 `.singular 字符串`，
        // 对象/纯字符串写法会整条回落成 id。
        let inline_name = obj
            .get("name")
            .and_then(|n| n.get("singular"))
            .or_else(|| obj.get("name"))
            // 兼容 `text.name` 写法（与 artifact 的 `text.name` 一致；实测 new-chinese-mods 系列）
            .or_else(|| {
                obj.get("text")
                    .and_then(|t| t.get("name"))
                    .and_then(|n| n.get("singular"))
            })
            .or_else(|| obj.get("text").and_then(|t| t.get("name")));
        // 生物 i18n key 是复数 `creatures.<modId>.<id>.name.singular`
        let name_obj = name_obj_with_inline(
            &i18n,
            &format!("creatures.{}.{}.name.singular", scope, id),
            inline_name,
            &id,
        );
        let upgrades = obj
            .get("upgrades")
            .and_then(|u| u.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let gr = obj.get("graphics");
        let anim = gr
            .and_then(|g| g.get("animation"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let anim_map = gr
            .and_then(|g| g.get("map"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let anim_attack_l = gr
            .and_then(|g| g.get("mapAttackFromLeft"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let anim_attack_r = gr
            .and_then(|g| g.get("mapAttackFromRight"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        // 杀伤力（min/max），支持 {"min":x,"max":y} 或 [min,max]
        let (dmg_min, dmg_max) = match obj.get("damage") {
            Some(v) if v.is_object() => (
                v.get("min").and_then(|x| x.as_i64()).unwrap_or(0),
                v.get("max").and_then(|x| x.as_i64()).unwrap_or(0),
            ),
            Some(v) if v.is_array() => {
                let arr = v.as_array().unwrap();
                (
                    arr.first().and_then(|x| x.as_i64()).unwrap_or(0),
                    arr.get(1).and_then(|x| x.as_i64()).unwrap_or(0),
                )
            }
            _ => (0, 0),
        };
        let cost = obj.get("cost").cloned().unwrap_or(json!({}));
        // 地图预览图标（graphics.map）
        let adv_map = gr
            .and_then(|g| g.get("map"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());
        let adv_map_img = extract_icon_and_save(
            &p,
            game_dir,
            cache_root,
            mod_id,
            "creature",
            &format!("{}-advmap", id),
            adv_map.as_deref(),
        )
        .or_else(|| {
            if is_core && adv_map.as_deref().is_none() {
                // 原版 H3 生物未在 JSON 声明 graphics.map；冒险地图精灵回退到小图（与 VCMI 行为一致），
                // 归位本模组目录避免 null，与第三方 town mod 的 images/creature/<id>-advmap.webp 对齐。
                img.clone()
            } else {
                None
            }
        });
        // 描述（VCMI getDescriptionTranslated）。
        // 翻译池实际写法是**复数** `creatures.<scope>.<id>.description`（实测 936 键），
        // 而历史代码只查单数 `creature.`（仅 h3-core 内部自产的键用单数）→ 模组生物描述整批落空。
        let desc_obj = name_obj_multi_key(
            &i18n,
            &[
                format!("creatures.{}.{}.description", scope, id),
                format!("creature.{}.{}.description", scope, id),
            ],
            // 配置内嵌：顶层 `description` → `text.description`（后者的写法见协议/审计登记）
            obj.get("description")
                .or_else(|| obj.get("text").and_then(|t| t.get("description"))),
            "",
        );
        // 事件文本：`text.event`（与 artifact 的 eventDescription 对应；实测 new-chinese-mods.yanHuang）
        let event_obj = name_obj_multi_key(
            &i18n,
            &[format!("creatures.{}.{}.event", scope, id)],
            obj.get("text").and_then(|t| t.get("event")),
            "",
        );
        // 囤兵数（horde growth）
        let horde = obj.get("horde").and_then(|v| v.as_i64()).unwrap_or(0);
        // 双格单位
        let double_wide = obj
            .get("doubleWide")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        // 地图数量范围 advMapAmount: {min,max} 或 [min,max]
        let (adv_min, adv_max) = match obj.get("advMapAmount") {
            Some(v) if v.is_object() => (
                v.get("min").and_then(|x| x.as_i64()).unwrap_or(0),
                v.get("max").and_then(|x| x.as_i64()).unwrap_or(0),
            ),
            Some(v) if v.is_array() => {
                let arr = v.as_array().unwrap();
                (
                    arr.first().and_then(|x| x.as_i64()).unwrap_or(0),
                    arr.get(1).and_then(|x| x.as_i64()).unwrap_or(0),
                )
            }
            _ => (0, 0),
        };
        // 施法次数（casts，来自 bonus 或字段）
        let casts = obj.get("casts").and_then(|v| v.as_i64()).unwrap_or(0);
        // 能力多语言描述。
        // VCMI 生物配置用 `abilities` 对象（key = 能力标识，同时是 i18n key 末段，
        // 见 CHeroHandler/CBonusType 的 ability.<name> 解析）；少数旧 mod 用 `bonuses` 数组。
        let mut abilities = Vec::new();
        // VCMI `WikiCreatureContent.cpp:306-307` 的去重集合：`(type, subtype, text)` 相同只留第一行
        let mut seen_abilities: std::collections::HashSet<(String, String, String)> =
            std::collections::HashSet::new();
        let ability_entries: Vec<(String, &Value)> =
            match obj.get("abilities").and_then(|a| a.as_object()) {
                Some(map) => map.iter().map(|(k, v)| (k.clone(), v)).collect(),
                None => obj
                    .get("bonuses")
                    .and_then(|b| b.as_array())
                    .map(|arr| {
                        arr.iter()
                            .map(|v| {
                                (
                                    v.get("name")
                                        .and_then(|x| x.as_str())
                                        .unwrap_or("")
                                        .to_string(),
                                    v,
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default(),
            };
        {
            for (i, (bname, b)) in ability_entries.iter().enumerate() {
                let bname = bname.as_str();
                // VCMI `WikiCreatureContent.cpp:302`：`if(b->hidden) continue;` ——
                // `hidden: true` 的能力**不出现在百科能力表**（作者用它隐藏实现细节，
                // 实测 tears-of-ashan 的 `waterReduction` 等；VCMI 界面也不显示）。
                if b.get("hidden").and_then(|x| x.as_bool()).unwrap_or(false) {
                    continue;
                }
                let btype = b.get("type").and_then(|x| x.as_str()).unwrap_or("");
                let bval = b.get("val").and_then(|x| x.as_i64()).unwrap_or(0);
                // 子类型（VCMI 用 ${subtype.creature} / ${subtype.spell} 引用子类型实体名）
                let bsub = b
                    .get("subtype")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                // 图标查表键：`subtypeIcons` 的键是 **dotted 原文**（`spellSchool.earth`、
                // `creature.xxx`），故只去掉模组作用域前缀（`core:`）
                let sub_base = bsub.rsplit(':').next().unwrap_or(&bsub).to_string();
                // 子类型**实体名**的查询键 = 末段：`spell.blind` / `creature.halfling` /
                // `core:blind` 里的 `blind` / `halfling`（`spell.` `creature.` 是 VCMI 的
                // 实体**类型前缀**，不属于 id）。此前直接拿 `spell.blind` 拼
                // `spell.core.spell.blind.name` 这类键 → 永远不命中 → 文案里的
                // `${subtype.spell}` 被替换成空串（实测"免疫${subtype.spell}魔法"显示成"免疫魔法"）。
                let sub_id = sub_base.rsplit('.').next().unwrap_or(&sub_base).to_string();
                // 子类型实体名的多语言表：生物复数名 → 法术名（VCMI 文本键）
                let sub_names: Option<std::collections::HashMap<String, String>> =
                    if sub_id.is_empty() {
                        None
                    } else {
                        // 候选键按**翻译池实际写法**排列：生物用复数前缀 `creatures.*`
                        // （`creature.` 单数是 h3-core 内部历史写法）；法术为 `spell.<scope>.<id>.name`
                        // 与 h3-core 的 `spell.core.<id>.name`（此前写成 `core.spell.` 永远不命中）。
                        [
                            format!("creatures.{}.{}.name.plural", scope, sub_id),
                            format!("creature.{}.{}.name.plural", scope, sub_id),
                            format!("creatures.core.{}.name.plural", sub_id),
                            format!("creatures.core.{}.name.singular", sub_id),
                            format!("spell.{}.{}.name", scope, sub_id),
                            format!("spell.core.{}.name", sub_id),
                            format!("core.spell.{}.name", sub_id),
                        ]
                        .iter()
                        .find_map(|k| i18n_get_multi(&i18n, k))
                    };
                // VCMI WikiCreatureContent.cpp:302-304 用 bonus 描述文本（bonus.<TYPE>.description），
                // 缺失时回退 mod 为该生物该能力定制的 i18n key。
                // 效果文本三层**合并**（后者补前者空缺，不整体覆盖，避免"内嵌只有描述 →
                // 名称回归成配置里的能力标识"）：
                //   1) 核心通用文本 `core.bonus.<TYPE>.description`（`{名称}\n描述`）
                //   2) 配置内嵌 `abilities.<id>.name/description`（作者为该生物定制）
                //   3) mod 翻译键 `creatures.<scope>.<id>.bonus.<bonusType 小驼峰>`（最高优先）
                let bonus_text = merge_pairs(
                    ability_text_from_bonus_type(&i18n, btype, bval, sub_names.as_ref()),
                    inline_ability_text(b),
                );
                let bonus_text = merge_pairs(
                    bonus_text,
                    name_desc_maps_from_keys(
                        &i18n,
                        &[format!(
                            "creatures.{}.{}.bonus.{}",
                            scope,
                            id,
                            crate::modcache::i18n::lower_camel(btype)
                        )],
                        bval,
                        sub_names.as_ref(),
                    ),
                );
                let bt_zh = bonus_text
                    .as_ref()
                    .and_then(|(n, _)| {
                        n.get("chinese")
                            .or_else(|| n.get("auto"))
                            .or_else(|| n.get("english"))
                            .or_else(|| n.values().next())
                            .cloned()
                    })
                    .or_else(|| {
                        i18n_get_multi(
                            &i18n,
                            &format!("creature.{}.{}.ability.{}", scope, id, bname),
                        )
                        .or_else(|| {
                            i18n_get_multi(
                                &i18n,
                                &format!("creature.{}.{}.ability.{}", scope, id, i),
                            )
                        })
                        .and_then(|m| {
                            m.get("chinese")
                                .cloned()
                                .or_else(|| m.get("auto").cloned())
                                .or_else(|| m.get("english").cloned())
                                .or_else(|| m.values().next().cloned())
                        })
                    });
                let bt_en = bonus_text
                    .as_ref()
                    .and_then(|(n, _)| {
                        n.get("english")
                            .or_else(|| n.get("auto"))
                            .or_else(|| n.get("chinese"))
                            .or_else(|| n.values().next())
                            .cloned()
                    })
                    .or_else(|| {
                        i18n_get_multi(
                            &i18n,
                            &format!("creature.{}.{}.ability.{}", scope, id, bname),
                        )
                        .or_else(|| {
                            i18n_get_multi(
                                &i18n,
                                &format!("creature.{}.{}.ability.{}", scope, id, i),
                            )
                        })
                        .and_then(|m| {
                            m.get("english")
                                .cloned()
                                .or_else(|| m.get("auto").cloned())
                                .or_else(|| m.get("chinese").cloned())
                                .or_else(|| m.values().next().cloned())
                        })
                    })
                    .or_else(|| bt_zh.clone());
                // ── 行文本：无文案的能力**整行不显示**（对齐 VCMI `WikiCreatureContent.cpp:298-307`）──
                //
                // VCMI 的行文本 = 内嵌 `description`（非空时）否则 `bonusToString(b, creature)`；
                // 两者都空 → `if(text.empty()) continue;`。启动器把这段文本拆成"名称 / 描述"两行，
                // 名称优先 bonus 类型文本的 `{名称}` 段，其次 mod 为**该生物该能力**定制的
                // `creature.<scope>.<id>.ability.<key>`，再次内嵌 `description`（VCMI 此时就把
                // 这段当行文本）。
                //
                // ⚠️ 刻意**不**回退到配置里的能力标识（`debuff1` / `noCastByDefault` /
                // `castearthquake2` …）：它从来不是 VCMI 的文案来源，只是作者为"多触发一条 bonus"
                // 随手起的 key。回退到它 = 凭空多出"没图、没描述、名字是 camelCase"的垃圾行
                // （用户实测 ultraman 的生物多出 casts / debuff1–6 / buff1–6 十余行）。
                let inline_text = b
                    .get("description")
                    .and_then(|x| x.as_str())
                    .filter(|s| !s.is_empty())
                    // 内嵌 `description` 是**文本 id**（VCMI `JsonBonus.cpp` 语义），解析不到才用原文
                    .map(|s| {
                        i18n_get_multi(&i18n, s).unwrap_or_else(|| {
                            let mut m = std::collections::HashMap::new();
                            m.insert("chinese".to_string(), s.to_string());
                            m.insert("english".to_string(), s.to_string());
                            m
                        })
                    });
                let inline_zh = inline_text.as_ref().and_then(|m| {
                    m.get("chinese")
                        .or_else(|| m.get("auto"))
                        .or_else(|| m.get("english"))
                        .or_else(|| m.values().next())
                        .cloned()
                });
                let inline_en = inline_text.as_ref().and_then(|m| {
                    m.get("english")
                        .or_else(|| m.get("auto"))
                        .or_else(|| m.get("chinese"))
                        .or_else(|| m.values().next())
                        .cloned()
                });
                let bt_zh = bt_zh.or_else(|| inline_zh.clone());
                let bt_en = bt_en.or_else(|| inline_en.clone());
                if bt_zh.is_none() && bt_en.is_none() {
                    continue;
                }
                // VCMI `:306-307` 同款去重：`(type, subtype, text)` 相同只留第一行。
                // 实测 ultraman 的生物把同一条 `SPELL_DAMAGE_REDUCTION / spellSchool.earth`
                // 写了两遍 → 此前列表里出现两行完全相同的"魔法抗性"。
                if !seen_abilities.insert((
                    btype.to_string(),
                    bsub.clone(),
                    bt_zh.clone().unwrap_or_default(),
                )) {
                    continue;
                }
                // 空值兜底：`name` 一律是字符串（前端 `pickName` 按字符串处理，别发 null 出去）
                let name_zh = bt_zh.clone().or_else(|| bt_en.clone()).unwrap_or_default();
                let name_en = bt_en.clone().or_else(|| bt_zh.clone()).unwrap_or_default();
                // 能力描述（VCMI CStackWindow 能力卡第二行）：
                // 内嵌 `description`（已按文本 id 解析）→ `bonus.<TYPE>` 的翻译描述 → 回退名称
                let bonus_desc = bonus_text.as_ref().and_then(|(_, d)| {
                    d.get("chinese")
                        .or_else(|| d.get("auto"))
                        .or_else(|| d.get("english"))
                        .or_else(|| d.values().next())
                        .cloned()
                });
                let bonus_desc_en = bonus_text.as_ref().and_then(|(_, d)| {
                    d.get("english")
                        .or_else(|| d.get("auto"))
                        .or_else(|| d.get("chinese"))
                        .or_else(|| d.values().next())
                        .cloned()
                });
                let bdesc_zh = inline_zh
                    .clone()
                    .or(bonus_desc)
                    .or_else(|| bt_zh.clone())
                    .or_else(|| bt_en.clone())
                    .unwrap_or_default();
                let bdesc_en = inline_en
                    .clone()
                    .or(bonus_desc_en)
                    .or_else(|| bt_en.clone())
                    .or_else(|| bt_zh.clone())
                    .unwrap_or_default();
                // 能力来源（CStackWindow::BonusesSection 图标框底部标签，如"技能"）
                let bsource = b
                    .get("source")
                    .and_then(|v| v.as_str())
                    .unwrap_or("CREATURE_ABILITY")
                    .to_string();
                // 能力图标：**两级**来源，完全对齐 VCMI
                // `WikiCreatureContent.cpp:307-309`：
                //   ① `!b->customIconPath.empty() ? b->customIconPath`
                //      —— `JsonBonus.cpp:893` 读的 `ability["icon"]`；
                //   ② `: LIBRARY->bth->bonusToGraphics(b)`
                //      —— 按 bonus type 查 `bonuses.json` 的
                //         `subtypeIcons[subtype] → valueIcons[val] → icon`（见 `vcmi::bonus`）。
                //
                // 此前只实现 ①：而 `icon` 是作者给**单个生物**定制时才写的字段，
                // 绝大多数能力不带它 → 能力表所有图标恒为空（用户报"所有技能图片均为空图"）。
                // 真实图标来源是 `vcmi-extras` 的 Bonus Icons 子模组（`core:<TYPE>` → icon）。
                let icon_name = format!(
                    "{}-{}",
                    id,
                    if bname.is_empty() {
                        i.to_string()
                    } else {
                        bname.to_string()
                    }
                );
                let bicon = extract_icon_and_save(
                    &p,
                    game_dir,
                    cache_root,
                    mod_id,
                    "ability",
                    &icon_name,
                    b.get("icon").and_then(|x| x.as_str()),
                )
                .or_else(|| {
                    // ②a `SPELL_IMMUNITY` **特例**（`CBonusTypeHandler.cpp:109-113`，命中即 return）：
                    //     图标取「被免疫的那个法术」自己的 `graphics.iconImmune`，与 bonuses.json 无关。
                    //     实测 Bonus Icons 的 `core:SPELL_IMMUNITY` 图形段就是空的
                    //     （注释 "Uses icon from spell config"）→ 不走特例时这类能力必然无图。
                    if btype.eq_ignore_ascii_case("SPELL_IMMUNITY") {
                        let immune = crate::vcmi::spell_icons::spell_immune_icon(game_dir, &bsub);
                        if let Some(res) = immune {
                            if let Some(icon) = extract_icon_and_save(
                                &p,
                                game_dir,
                                cache_root,
                                mod_id,
                                "ability",
                                &icon_name,
                                Some(res.as_str()),
                            ) {
                                return Some(icon);
                            }
                        }
                    }
                    // ②b 常规链：`subtypeIcons[subtype] → valueIcons[val] → icon`。
                    //
                    // 与 VCMI 的唯一偏差：法术查不到 `iconImmune`（原版法术无 JSON、mod 法术没写）
                    // 时 VCMI 直接返回空图，这里继续往下落到 abilities.json/bonuses.json 的通用图标。
                    // 空图不会比"有张通用图标"更好，故保留回退（既有行为，不构成回归）。
                    let table = crate::vcmi::bonus::bonus_graphics_map(game_dir);
                    let g = table.get(&btype.to_ascii_uppercase())?;
                    let res = g.resolve(&sub_base, bval)?;
                    extract_icon_and_save(
                        &p,
                        game_dir,
                        cache_root,
                        mod_id,
                        "ability",
                        &icon_name,
                        Some(res.as_str()),
                    )
                });
                abilities.push(json!({
                    "type": btype,
                    "subtype": bsub,
                    "value": bval,
                    "name": { "chinese": name_zh, "english": name_en },
                    "description": { "chinese": bdesc_zh, "english": bdesc_en },
                    "icon": bicon,
                    // 能力来源（CStackWindow::BonusesSection 图标框底部标签）
                    "source": bsource,
                }));
            }
        }
        // 生物弹窗背景（VCMI CCreaturePic：阵营 creatureBackground 130px）
        let fac_id_for_bg = obj.get("faction").and_then(|v| v.as_str()).unwrap_or("");
        let creature_bg = factions
            .iter()
            .find(|f| {
                let fid = f.get("factionId").and_then(|x| x.as_str()).unwrap_or("");
                !fid.is_empty()
                    && (fid == fac_id_for_bg
                        || fac_id_for_bg.ends_with(&format!(".{fid}"))
                        || fid.ends_with(&format!(".{fac_id_for_bg}")))
            })
            .and_then(|f| f.get("creatureBackground").cloned())
            .unwrap_or(Value::Null);
        // 士气/运气：CStackWindow 用 valOfBonuses(MORALE/LUCK)，生物自身配置通常为 0。
        // 若配置显式写了 MORALE/LUCK bonus，取其数值。
        let stat_val = |t: &str| -> i64 {
            abilities
                .iter()
                .filter_map(|a| a.as_object())
                .find(|a| a.get("type").and_then(|v| v.as_str()) == Some(t))
                .and_then(|a| a.get("value").and_then(|v| v.as_i64()))
                .unwrap_or(0)
        };
        let morale = stat_val("MORALE");
        let luck = stat_val("LUCK");
        creatures.push(json!({
            "id": id,
            "name": name_obj,
            "image": img,
            "imageLarge": img_large,
            "advMapImage": adv_map_img,
            "attack": obj.get("attack").and_then(|v| v.as_i64()).unwrap_or(0),
            "defense": obj.get("defense").and_then(|v| v.as_i64()).unwrap_or(0),
            "hitPoints": obj.get("hitPoints").and_then(|v| v.as_i64()).unwrap_or(0),
            "speed": obj.get("speed").and_then(|v| v.as_i64()).unwrap_or(0),
            "shots": obj.get("shots").and_then(|v| v.as_i64()).unwrap_or(0),
            "damageMin": dmg_min,
            "damageMax": dmg_max,
            "growth": obj.get("growth").and_then(|v| v.as_i64()).unwrap_or(0),
            "aiValue": obj.get("aiValue").and_then(|v| v.as_i64()).unwrap_or(0),
            "combatValue": obj.get("combatValue").and_then(|v| v.as_i64()).unwrap_or(0),
            "cost": cost,
            // 完整招募成本（自身 + 升级链差价，对齐 VCMI getFullRecruitCost）
            "fullRecruitCost": accumulate_full_cost(creature_jsons, upgraded_from, &id, &mut std::collections::HashSet::new()),
            // CPRSMALL/TWCRPORT 图标帧号（VCMI: creature json iconIndex，缺省 = index + 2）
            "iconIndex": icon_index,
            "level": obj.get("level").and_then(|v| v.as_i64()).unwrap_or(0),
            // 士气/运气（CStackWindow 用 MoraleLuckBox 展示，生物自身通常为 0）
            "morale": morale,
            "luck": luck,
            "faction": obj.get("faction").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            "upgrades": upgrades,
            "abilities": abilities,
            "animation": anim,
            "mapAnimation": anim_map,
            "mapAttackFromLeft": anim_attack_l,
            "mapAttackFromRight": anim_attack_r,
            "description": desc_obj,
            "eventDescription": event_obj,
            "horde": horde,
            "doubleWide": double_wide,
            "advMapAmountMin": adv_min,
            "advMapAmountMax": adv_max,
            "casts": casts,
            "upgradedFrom": upgraded_from.get(id.as_str()).cloned().unwrap_or_default(),
            "dwellings": creature_dwellings.get(id.as_str()).cloned().unwrap_or(json!({})),
            "creatureBackground": creature_bg,
            // 战争机器：VCMI 由神器的 `warMachine` 反查（CArtHandler.cpp:230），
            // 值为绑定的神器 id（空串 = 非战争机器）
            "warMachine": war_machine_of.get(id.as_str()).cloned().unwrap_or_default(),
            // 战争机器预览动画分组（VCMI startPreview(siegeWeapon)）：投石车/弩车等走 machPreviewList
            "siegeWeapon": obj
                .get("bonuses")
                .and_then(|b| b.as_array())
                .map(|arr| arr.iter().any(|x| x.get("type").and_then(|t| t.as_str()) == Some("SIEGE_WEAPON")))
                .unwrap_or(false),
        }));
    }

    // 反查升级来源：先收集所有 (id, 升级目标) 配对，再可变借用填充
    let upgrade_pairs: Vec<(String, Vec<String>)> = creatures
        .iter()
        .map(|c| {
            let cid = c
                .get("id")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let ups = c
                .get("upgrades")
                .and_then(|u| u.as_array())
                .map(|a| {
                    a.iter()
                        .filter_map(|x| x.as_str().map(|s| s.to_string()))
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            (cid, ups)
        })
        .collect();
    for (cid, ups) in upgrade_pairs {
        for tid in ups {
            if let Some(target) = creatures
                .iter_mut()
                .find(|x| x.get("id").and_then(|y| y.as_str()) == Some(tid.as_str()))
            {
                if let Some(arr) = target
                    .get_mut("upgradedFrom")
                    .and_then(|x| x.as_array_mut())
                {
                    arr.push(json!(cid.clone()));
                }
            }
        }
    }

    creatures
}
