//! 构建期文案构造：把 VCMI i18n key 组织成前端直读的名称 / 描述对象。
//!
//! 与 `vcmi::i18n`（读取 mod 自身的 i18n 表）分工不同 —— 本模块负责**生成**缓存里的
//! `name` / `description` 字段（多语言回退、原版 h3-core 文案注入、特长描述等）。
//!
//! 依赖方向：可依赖 `infra` 与 `vcmi`（见 `docs/architecture/backend.zh-CN.md` §4.1）。

use serde_json::{json, Value};

use crate::vcmi::h3core::I18nMap;

/// 构建期共享的 VCMI 核心翻译表（`Mods/vcmi`，含 `core.bonus.*` 等能力文本与原版实体名）。
///
/// 原实现在**每个 mod** 里把整张核心表合并进该 mod 的 i18n map，380 个 mod 即 380 次全表合并
/// （外加 380 次全量解析），是构建 CPU 的主要来源之一。
/// 改为全局持有一次、按 key 两级查询：核心表零克隆，只在命中时克隆该 key 的少量语言条目。
pub(crate) static CORE_I18N_FOR_BUILD: std::sync::OnceLock<
    std::sync::Mutex<Option<std::sync::Arc<I18nMap>>>,
> = std::sync::OnceLock::new();

/// 设置核心翻译表（幂等：仅首次生效，构建期内 game_dir 不变）。
pub(crate) fn set_core_i18n(m: std::sync::Arc<I18nMap>) {
    let slot = CORE_I18N_FOR_BUILD.get_or_init(|| std::sync::Mutex::new(None));
    if let Ok(mut g) = slot.lock() {
        if g.is_none() {
            *g = Some(m);
        }
    }
}

pub(crate) fn core_i18n() -> Option<std::sync::Arc<I18nMap>> {
    let slot = CORE_I18N_FOR_BUILD.get()?;
    slot.lock().ok()?.clone()
}

/// 释放核心翻译表（构建全部结束后调用；见 [`crate::infra::cache_gc`]）。
///
/// 整张 `Mods/vcmi` 合并表（18 个 h3-core JSON）在 `OnceLock` 里常驻到进程退出；
/// 改为可置空后，构建结束即可归还，下次构建按需重建。
pub(crate) fn release_caches() {
    if let Some(slot) = CORE_I18N_FOR_BUILD.get() {
        if let Ok(mut g) = slot.lock() {
            *g = None;
        }
    }
}

/// VCMI 翻译键的 mod 作用域归一化。
///
/// 原版（core）子模组在磁盘上被拆成 `core.<faction>`，但 VCMI 翻译表里的作用域仍是 `core`。
/// 若直接用 `core.<faction>` 作 i18n 键前缀，会命中空（h3-core 与核心翻译均无该键），
/// 导致 town/creature/hero/building 等名称回退成英文 id。
/// 第三方 mod 则保持其自身 modId 不变。
pub(crate) fn i18n_mod_scope(mod_id: &str) -> &str {
    if mod_id == "core" || mod_id.starts_with("core.") {
        "core"
    } else {
        mod_id
    }
}

/// 大小写不敏感的键查找：原样 key → 小写 key。
///
/// VCMI 的翻译键在磁盘上大小写不统一：同一实体在 modId 里是混合大小写
/// （`crossroads-of-worlds.Sand Castle`、`Fairytale`），而翻译文件里的作用域写成全小写
/// （`faction.crossroads-of-worlds.sand castle.sandcastle.name` = 红城、
/// `faction.fairytale.fairytale.name` = 花仙）。只做精确匹配会命中失败、名称回退英文。
fn lookup<'a>(m: &'a I18nMap, key: &str) -> Option<&'a std::collections::HashMap<String, String>> {
    if let Some(v) = m.get(key) {
        return Some(v);
    }
    let lower = key.to_lowercase();
    if lower != key {
        m.get(&lower)
    } else {
        None
    }
}

/// 从 i18n 字典取某 key 的多语言文本（按 VCMI 语言 key 组织）。
///
/// 查询顺序：核心翻译表（基础）→ mod 自身翻译（覆盖同名语言），
/// 与原先"逐 key 合并整张核心表"的语义等价，但代价从 O(全表) 降为 O(该 key 的语言数)。
pub(crate) fn i18n_get_multi(
    i18n: &I18nMap,
    key: &str,
) -> Option<std::collections::HashMap<String, String>> {
    let core = core_i18n().and_then(|c| lookup(c.as_ref(), key).cloned());
    let own = lookup(i18n, key);
    match (core, own) {
        (None, None) => None,
        (Some(m), None) => Some(m.clone()),
        (None, Some(m)) => Some(m.clone()),
        (Some(c), Some(o)) => {
            let mut m = c.clone();
            for (lang, text) in o {
                m.insert(lang.clone(), text.clone());
            }
            Some(m)
        }
    }
}

/// 解析 VCMI 核心翻译的 `bonus.<TYPE>.description`：
/// 文本格式为 `{名称}\n{描述}`（见 core 翻译文件）。
/// 占位符（CBonusTypeHandler.cpp:93-100）：`${val}` = 能力数值，
/// `${subtype.creature}` = 子类型生物名，`${subtype.spell}` = 子类型法术名。
/// 返回 (名称, 描述) 的多语言映射（语言 → 文本）。
pub(crate) fn ability_text_from_bonus_type(
    i18n: &std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    btype: &str,
    val: i64,
    // 子类型实体的多语言名（语言 → 名称），用于 ${subtype.creature} / ${subtype.spell}
    sub_names: Option<&std::collections::HashMap<String, String>>,
) -> Option<(
    std::collections::HashMap<String, String>,
    std::collections::HashMap<String, String>,
)> {
    // 核心翻译文件里的 key 带 mod 作用域前缀（`core.bonus.<TYPE>.description`）
    let raw = i18n_get_multi(i18n, &format!("bonus.{}.description", btype))
        .or_else(|| i18n_get_multi(i18n, &format!("core.bonus.{}.description", btype)))?;
    let mut names = std::collections::HashMap::new();
    let mut descs = std::collections::HashMap::new();
    for (lang, text) in raw {
        // VCMI bonusToString：${val} → 数值；${subtype.creature}/${subtype.spell} → 子类型实体名
        // 按当前语言取对应的实体名，避免多语言串味
        let sub = sub_names
            .and_then(|m| m.get(&lang))
            .map(|s| s.as_str())
            .unwrap_or("");
        let filled = text
            .replace("${val}", &val.to_string())
            .replace("${subtype.creature}", sub)
            .replace("${subtype.spell}", sub);
        match filled.strip_prefix('{').and_then(|r| r.split_once('}')) {
            Some((name, rest)) => {
                names.insert(lang.clone(), name.to_string());
                let desc = rest.trim_start_matches('\n').trim().to_string();
                if !desc.is_empty() {
                    descs.insert(lang.clone(), desc);
                }
            }
            None => {
                names.insert(lang.clone(), filled.trim().to_string());
            }
        }
    }
    if names.is_empty() {
        None
    } else {
        Some((names, descs))
    }
}

/// 从多语言字典构建名称对象（VCMI key：chinese/english/tchinese/...），
/// 并额外生成 `auto` 字段（按 chinese→english→其它 的回退），供前端 "auto" 语言使用。
/// 同时并入 `inner`（如 town.json 内嵌 name 的多语言对象）作为补充来源。
pub(crate) fn build_name_object(
    i18n: &std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    key: &str,
    inner: Option<&Value>,
) -> Value {
    let mut m: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    // 内嵌 name（多语言对象）优先作为基础
    if let Some(Value::Object(obj)) = inner {
        for (k, v) in obj {
            if let Some(s) = v.as_str() {
                m.insert(k.to_string(), s.to_string());
            }
        }
    } else if let Some(Value::String(s)) = inner {
        // 单语言字符串 = 原文（VCMI 翻译基准语言为英文），注入 english 键
        if !s.is_empty() {
            m.insert("english".to_string(), s.clone());
        }
    }
    // i18n 覆盖/补充
    if let Some(map) = i18n_get_multi(i18n, key) {
        for (lang, text) in map {
            m.insert(lang.clone(), text.clone());
        }
    }
    // auto 兜底：chinese -> english（不取任意第三方语言原文）
    if !m.contains_key("auto") {
        let auto = m.get("chinese").or_else(|| m.get("english")).cloned();
        if let Some(a) = auto {
            m.insert("auto".to_string(), a);
        }
    }
    let mut obj = serde_json::Map::new();
    for (k, v) in m {
        obj.insert(k, Value::String(v));
    }
    Value::Object(obj)
}

/// 从 VCMI 多语言字典取某 key 的多语言名称对象（兼容前端既有结构）。
/// `chinese` 只在**确有中文译文**时写入；`english` 另含展示兜底（见 [`build_text_obj`]）。
pub(crate) fn name_obj_en_zhcn(
    i18n: &std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    key: &str,
    fallback: &str,
) -> Value {
    build_text_obj(i18n_get_multi(i18n, key), None, fallback)
}

/// 同 [`name_obj_en_zhcn`]，但支持**配置内嵌文本**作为回退来源。
///
/// 大量模组不写 `translation/` 翻译文件，而是把文本直接写在配置里（实测：中文 mod 的
/// `heroes/*.json` 顶层 `name: "冰狼"`、`spells/*.json` 的 `name: "护城河"`）；
/// 此前只查 i18n 键、拿不到就回落成实体 id，导致这些条目的中英文全部丢失。
///
/// 内嵌文本支持两种写法：
/// - 字符串（VCMI 基准语言为英文，但中文 mod 常直接写中文）；
/// - 多语言对象 `{"chinese": "...", "english": "..."}`。
pub(crate) fn name_obj_with_inline(
    i18n: &std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    key: &str,
    inline: Option<&Value>,
    fallback: &str,
) -> Value {
    build_text_obj(i18n_get_multi(i18n, key), inline, fallback)
}

/// 多键候选版：按顺序尝试多个 i18n 键（VCMI 存在前缀/别名写法差异，
/// 如生物描述 `creatures.<scope>.<id>.description` 与历史单数 `creature.<scope>.<id>.description`），
/// 取首个命中的语言映射，再按 [`name_obj_with_inline`] 的回退链组装。
pub(crate) fn name_obj_multi_key(
    i18n: &std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    keys: &[String],
    inline: Option<&Value>,
    fallback: &str,
) -> Value {
    let m = keys.iter().find_map(|k| i18n_get_multi(i18n, k));
    build_text_obj(m, inline, fallback)
}

/// `RAISES_MORALE` → `raisesMorale`：VCMI 的 bonus 文本键用小驼峰
/// （实测 `creatures.core.angel.bonus.raisesMorale`、`artifact.core.orbOfVulnerability.bonus.noResistance`）。
pub(crate) fn lower_camel(s: &str) -> String {
    let mut out = String::new();
    for (i, part) in s.split('_').filter(|p| !p.is_empty()).enumerate() {
        if i == 0 {
            out.push_str(&part.to_ascii_lowercase());
        } else {
            let mut cs = part.chars();
            if let Some(c) = cs.next() {
                out.push(c.to_ascii_uppercase());
            }
            out.push_str(&cs.as_str().to_ascii_lowercase());
        }
    }
    out
}

/// 解析 VCMI 的 `{名称}\n描述` 组合文本（core.bonus / creatures.*.bonus / artifact.*.bonus 同格式），
/// 按语言拆分并填充占位符（`${val}` / `${subtype.creature}` / `${subtype.spell}`），
/// 返回 (名称对象, 描述对象)。无 `{}` 头时整串作为名称。
pub(crate) fn name_desc_from_keys(
    i18n: &std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    keys: &[String],
    val: i64,
    sub_names: Option<&std::collections::HashMap<String, String>>,
) -> Option<(Value, Value)> {
    let (n, d) = name_desc_maps_from_keys(i18n, keys, val, sub_names)?;
    Some((lang_map_to_obj(n), lang_map_to_obj(d)))
}

/// 同 [`name_desc_from_keys`]，但返回 (名称表, 描述表)（语言 → 文本），
/// 便于与 `ability_text_from_bonus_type` 的返回形态统一拼接使用。
pub(crate) fn name_desc_maps_from_keys(
    i18n: &std::collections::HashMap<String, std::collections::HashMap<String, String>>,
    keys: &[String],
    val: i64,
    sub_names: Option<&std::collections::HashMap<String, String>>,
) -> Option<(
    std::collections::HashMap<String, String>,
    std::collections::HashMap<String, String>,
)> {
    let raw = keys.iter().find_map(|k| i18n_get_multi(i18n, k))?;
    let mut names: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    let mut descs: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for (lang, text) in raw {
        // 按当前语言取子类型实体名，避免多语言串味（与 ability_text_from_bonus_type 一致）
        let sub = sub_names
            .and_then(|m| m.get(&lang))
            .map(|s| s.as_str())
            .unwrap_or("");
        let filled = text
            .replace("${val}", &val.to_string())
            .replace("${subtype.creature}", sub)
            .replace("${subtype.spell}", sub);
        match filled.strip_prefix('{').and_then(|r| r.split_once('}')) {
            Some((n, rest)) => {
                if !n.trim().is_empty() {
                    names.insert(lang.clone(), n.to_string());
                }
                let d = rest.trim_start_matches('\n').trim().to_string();
                if !d.is_empty() {
                    descs.insert(lang.clone(), d);
                }
            }
            None => {
                let t = filled.trim();
                if !t.is_empty() {
                    names.insert(lang.clone(), t.to_string());
                }
            }
        }
    }
    if names.is_empty() {
        return None;
    }
    Some((names, descs))
}

/// `{lang: text}` → 前端名称对象（真实语言键 + `english` 展示兜底 + auto）。
///
/// `chinese` 只来自真实的 `chinese` 条目：缺中文就**不写**该键（前端 `pickName`
/// 回退 `english`），不拿 english/auto 冒充中文译文。
pub(crate) fn lang_map_to_obj(mut m: std::collections::HashMap<String, String>) -> Value {
    let zh = m.get("chinese").cloned().unwrap_or_default();
    let en = m
        .get("english")
        .cloned()
        .or_else(|| m.get("auto").cloned())
        .unwrap_or_default();
    let mut obj = serde_json::Map::new();
    for (k, v) in m.drain() {
        if k == "chinese" || k == "english" || k == "auto" || v.is_empty() {
            continue;
        }
        obj.insert(k, json!(v));
    }
    if !zh.is_empty() {
        obj.insert("chinese".to_string(), json!(zh));
    }
    if !en.is_empty() {
        obj.insert("english".to_string(), json!(en));
    }
    let auto = if !zh.is_empty() { zh } else { en };
    if !auto.is_empty() {
        obj.insert("auto".to_string(), json!(auto));
    }
    Value::Object(obj)
}

/// 语言表 → 多语言对象；**全空**时把 `fallback` 写进 `english`（不写 `chinese`）。
///
/// 用于原版 h3-core 文案（表里只有真实语言条目）：缺某语言就不写该键，
/// 前端 `pickName` 回退 `english`。`fallback` 通常传实体 id，空串表示"没有就不写"。
pub(crate) fn obj_from_lang_map(
    m: Option<std::collections::HashMap<String, String>>,
    fallback: &str,
) -> Value {
    let mut o = lang_map_to_obj(m.unwrap_or_default());
    if o.as_object().map(|x| x.is_empty()).unwrap_or(true) && !fallback.is_empty() {
        o = json!({ "english": fallback });
    }
    o
}

/// 文本对象组装核心：i18n（翻译文件）→ 内嵌文本（配置原文）→ 最终兜底字符串。
///
/// **语言键只写真实来源，不伪造译文**：
/// - `chinese` 只取 i18n.chinese → 内嵌对象的 `.chinese`。缺中文就不写该键，
///   由前端 `pickName` 按 VCMI 口径回退（当前语言 → 英文）；
/// - `english` 额外承担**展示兜底**（i18n.en → 内嵌 en/auto/裸串/中文 → fallback），
///   因为英文是 `pickName` 的最终回退语言，缺了就只能显示空白/id。
///
/// 不把英文（或 `auto`、兜底 id）填进 `chinese`：那会让"哪些语言真的有译文"失真，
/// 也让后续翻译审计无法区分真译文与兜底。
fn build_text_obj(
    m: Option<std::collections::HashMap<String, String>>,
    inline: Option<&Value>,
    fallback: &str,
) -> Value {
    let pick = |order: &[&str]| -> Option<String> {
        m.as_ref()
            .and_then(|x| order.iter().find_map(|k| x.get(*k)).cloned())
            .filter(|s| !s.is_empty())
    };
    // `allow_plain=false`：裸字符串语言未知（中文 mod 也常直接写中文），
    // 不能当成中文译文；它只配进 `english` 作展示兜底。
    let inline_pick = |order: &[&str], allow_plain: bool| -> Option<String> {
        match inline? {
            Value::String(s) => {
                let t = s.trim();
                if t.is_empty() || !allow_plain {
                    None
                } else {
                    Some(s.clone())
                }
            }
            Value::Object(o) => order
                .iter()
                .find_map(|k| o.get(*k).and_then(|x| x.as_str()))
                .filter(|s| !s.trim().is_empty())
                .map(|s| s.to_string()),
            _ => None,
        }
    };
    // 中文：只有真实中文来源（i18n.chinese / 内嵌对象.chinese），缺了就留空。
    let zh = pick(&["chinese"]).or_else(|| inline_pick(&["chinese"], false));
    // en：i18n.english → 内嵌.english → i18n.auto → 内嵌.auto → 内嵌.chinese → 兜底
    // （i18n.chinese 不兜：避免英文环境整页中文；内嵌文本是配置原文、无语言属性，可兜）
    let en = pick(&["english"])
        .or_else(|| inline_pick(&["english"], true))
        .or_else(|| pick(&["auto"]))
        .or_else(|| inline_pick(&["auto"], true))
        .or_else(|| inline_pick(&["chinese"], true))
        .unwrap_or_else(|| fallback.to_string());
    // 完整保留全部可用语言（russian/french/...），供前端任意内容语言回退
    let mut obj = serde_json::Map::new();
    if let Some(am) = &m {
        for (k, v) in am {
            if k == "chinese" || k == "english" || k == "auto" || v.is_empty() {
                continue;
            }
            obj.insert(k.clone(), json!(v));
        }
    }
    if let Some(z) = zh.as_deref().filter(|s| !s.is_empty()) {
        obj.insert("chinese".to_string(), json!(z));
    }
    if !en.is_empty() {
        obj.insert("english".to_string(), json!(en));
    }
    // auto：中文优先，其次英文（供内容语言为 auto 的场景）
    let auto = zh.as_deref().filter(|s| !s.is_empty()).unwrap_or(&en);
    if !auto.is_empty() {
        obj.insert("auto".to_string(), json!(auto));
    }
    Value::Object(obj)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn i18n_map(pairs: &[(&str, &str)]) -> HashMap<String, HashMap<String, String>> {
        let mut m = HashMap::new();
        m.insert(
            "k".to_string(),
            pairs
                .iter()
                .map(|(lang, text)| (lang.to_string(), text.to_string()))
                .collect(),
        );
        m
    }

    /// 缺中文时**不写** `chinese`（前端 `pickName` 回退 english），不拿英文冒充中文。
    #[test]
    fn text_obj_does_not_fake_chinese() {
        let v = name_obj_en_zhcn(&i18n_map(&[("english", "Sword")]), "k", "k");
        assert!(v.get("chinese").is_none(), "不应伪造中文：{v}");
        assert_eq!(v["english"], "Sword");
        // 兜底值（常为实体 id）同样不能写进 chinese
        let v = name_obj_en_zhcn(&i18n_map(&[]), "k", "k");
        assert!(v.get("chinese").is_none(), "兜底值不应冒充中文：{v}");
        assert_eq!(v["english"], "k");
    }

    /// 真有中文时两种语言各自保留，中文不是英文副本。
    #[test]
    fn text_obj_keeps_real_chinese() {
        let v = name_obj_en_zhcn(
            &i18n_map(&[("chinese", "长剑"), ("english", "Sword")]),
            "k",
            "k",
        );
        assert_eq!(v["chinese"], "长剑");
        assert_eq!(v["english"], "Sword");
        assert_eq!(v["auto"], "长剑");
    }

    /// 内嵌**裸字符串**语言未知（中文 mod 也常直接写中文）→ 只进 english 展示兜底；
    /// 只有显式多语言对象的 `chinese` 才算真中文。
    #[test]
    fn inline_plain_string_is_not_chinese() {
        let i18n = HashMap::new();
        let v = name_obj_with_inline(&i18n, "k", Some(&json!("冰狼")), "k");
        assert!(v.get("chinese").is_none(), "裸串不应冒充中文：{v}");
        assert_eq!(v["english"], "冰狼");
        let v = name_obj_with_inline(&i18n, "k", Some(&json!({ "chinese": "冰狼" })), "k");
        assert_eq!(v["chinese"], "冰狼");
    }

    /// `lang_map_to_obj` 同口径：没有 chinese 条目就不写 chinese 键。
    #[test]
    fn lang_map_does_not_fake_chinese() {
        let v = lang_map_to_obj(HashMap::from([("english".to_string(), "Sword".to_string())]));
        assert!(v.get("chinese").is_none(), "不应伪造中文：{v}");
        assert_eq!(v["english"], "Sword");
        assert_eq!(v["auto"], "Sword");
    }
}
