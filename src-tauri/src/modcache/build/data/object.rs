//! 地图对象（VCMI adventure map objects）：对象类名 / 子类型名 / 描述 / 访问事件文本。
//!
//! 数据来源 = **翻译键**（VCMI 源码实证）+ 配置内嵌回退：
//! - 对象**类**名：`object.<scope>.<id>.name`（`ObjectClass::getNameTextID`，
//!   CObjectClassesHandler.cpp:270）—— 即"船坞 / 魔井"这类族名；
//! - 子类型：`mapObject.<scope>.<type>.<sub>.name` / `.description`（`AObjectTypeHandler::getBaseTextID`，
//!   TextIdentifier("mapObject", scope, typeName, subTypeName)）；
//! - 访问事件：`mapObject.<scope>.<type>.<sub>.<onSelect|onEmpty>[.<i>][.message]`
//!   （`rewardable/Info.cpp`：旧写法无序号、新写法 `TextIdentifier(object, kind, i, "message")`；
//!   值形如 `{标题}\n正文`）。
//!
//! ⚠️ **i18n 键统一小写**（见 `vcmi::i18n`，VCMI 查找大小写不敏感）——前缀必须用小写，
//! 否则整批漏采（`mapObject.` 曾因大写 O 导致 8622 条键全部未提取）。
//!
//! 说明：对象**定义**（`config/**/objects.json`）在样本包中缺失（多为 `core:<id>` 类型补丁），
//! 故本模块不读出模板/图标，只做文本保真入库；配置里能读到的（`<scope>:<typeId>` 定义）
//! 会补充内嵌 name/description/rewards 文本。

use std::collections::{BTreeMap, HashMap};

use serde_json::{json, Value};

use crate::modcache::i18n::{
    i18n_mod_scope, lang_map_to_obj, name_desc_maps_from_keys, name_obj_multi_key,
};
use crate::vcmi::h3core::I18nMap;

/// 子类型聚合：名称 / 描述 / 事件
type SubEntry = (Option<Value>, Option<Value>, Vec<Value>);

/// 构建该 mod 的地图对象列表（按 对象类 id 排序）。
///
/// `raw_objects` = 归一架的 `objects`（`<scope>:<typeId>` 形式，如
/// `hota.mapobjects:warehouses`）；仅当 scope 首段等于本 mod 时视为**定义**，
/// 其内嵌 `name` / `types.*.description` / `types.*.rewards[].message` 作为回退。
pub(crate) fn build_objects(
    i18n: &I18nMap,
    raw_objects: Option<&serde_json::Map<String, Value>>,
    mod_id: &str,
) -> Vec<Value> {
    let scope = i18n_mod_scope(mod_id).to_lowercase();
    let pfx_obj = format!("object.{scope}.");
    // 小写！i18n 键统一小写（`mapObject.*` 在表里是 `mapobject.*`）
    let pfx_map = format!("mapobject.{scope}.");
    // 对象类 id → 名称对象
    let mut classes: BTreeMap<String, Value> = BTreeMap::new();
    // 对象类 id → 子类型 id → (name, description, events)
    let mut subs: BTreeMap<String, BTreeMap<String, SubEntry>> = BTreeMap::new();

    // ---- 配置侧（先填，i18n 命中会覆盖）：`<scope>:<typeId>` 定义 + 内嵌文本 ----
    if let Some(cfg) = raw_objects {
        for (k, v) in cfg {
            let Some((sc, ty)) = k.split_once(':') else {
                continue;
            };
            // scope 首段须等于本 mod（`hota.mapobjects` → `hota`）；跨 mod 补丁（`core:xxx`）跳过
            if sc.split('.').next().unwrap_or(sc) != scope || !v.is_object() {
                continue;
            }
            classes
                .entry(ty.to_string())
                .or_insert_with(|| name_obj_multi_key(i18n, &[], v.get("name"), ty));
            if let Some(types) = v.get("types").and_then(|x| x.as_object()) {
                for (sub, subv) in types {
                    let e = subs
                        .entry(ty.to_string())
                        .or_default()
                        .entry(sub.clone())
                        .or_insert_with(|| (None, None, Vec::new()));
                    if let Some(n) = subv.get("name") {
                        e.0 = Some(name_obj_multi_key(i18n, &[], Some(n), sub));
                    }
                    if let Some(d) = subv.get("description") {
                        e.1 = Some(name_obj_multi_key(i18n, &[], Some(d), ""));
                    }
                    // rewards[].message：访问奖励消息（`{标题}\n正文` 或纯文本）
                    if let Some(rs) = subv.get("rewards").and_then(|x| x.as_array()) {
                        for r in rs {
                            if let Some(m) = r.get("message") {
                                let (title, text) = split_inline_msg(m);
                                if !title.is_empty() || !text.is_empty() {
                                    e.2.push(json!({
                                        "kind": "reward",
                                        "title": lang_map_to_obj(title),
                                        "text": lang_map_to_obj(text),
                                    }));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    for k in i18n.keys() {
        // ---- 类名 `object.<scope>.<…>.name` ----
        // scope 可能是多段（`object.hota.mapobjects.warehouses.name`，对象定义在子模组、
        // 翻译写在主包）→ 取**最后一段**作类 id，与 mapObject 侧的 type 对齐合并。
        if let Some(rest) = k.strip_prefix(&pfx_obj) {
            if let Some(full) = rest.strip_suffix(".name") {
                let id = full.rsplit('.').next().unwrap_or(full);
                if !id.is_empty() {
                    classes.insert(
                        id.to_string(),
                        name_obj_multi_key(i18n, &[k.clone()], None, id),
                    );
                }
            }
            continue;
        }
        // ---- 子类型 `mapobject.<scope>.<type>.<sub>.<field>[.<i>][.message]` ----
        let Some(rest) = k.strip_prefix(&pfx_map) else {
            continue;
        };
        let segs: Vec<&str> = rest.split('.').collect();
        // 从右往左剥离：可选 `.message`（新版 rewardable 写法）→ 可选序号 → 字段名
        let mut idx = segs.len();
        if idx > 0 && segs[idx - 1] == "message" {
            idx -= 1;
        }
        if idx > 0 && !segs[idx - 1].is_empty() && segs[idx - 1].chars().all(|c| c.is_ascii_digit())
        {
            idx -= 1;
        }
        if idx < 3 {
            continue;
        }
        let field = segs[idx - 1];
        if !matches!(field, "name" | "description" | "onselect" | "onempty") {
            continue;
        }
        // `[<scope 多段…>, <type>, <sub>]`：type/sub 取剥除后的最后两段
        let ty = segs[idx - 3];
        let sub = segs[idx - 2];
        let entry = subs
            .entry(ty.to_string())
            .or_default()
            .entry(sub.to_string())
            .or_insert_with(|| (None, None, Vec::new()));
        match field {
            "name" => {
                entry.0 = Some(name_obj_multi_key(i18n, &[k.clone()], None, sub));
            }
            "description" => {
                entry.1 = Some(name_obj_multi_key(i18n, &[k.clone()], None, ""));
            }
            _ => {
                // 值形如 `{标题}\n正文`：拆为 标题/正文（语言各自拆分）
                if let Some((title, text)) = name_desc_maps_from_keys(i18n, &[k.clone()], 0, None) {
                    entry.2.push(json!({
                        "kind": field,
                        "title": lang_map_to_obj(title),
                        "text": lang_map_to_obj(text),
                    }));
                }
            }
        }
    }

    // 合并：以类 id 为纲（子类型挂在类下；只有子类型没有类名的也保留）
    let mut ids: Vec<String> = classes.keys().cloned().collect();
    for ty in subs.keys() {
        if !classes.contains_key(ty) {
            ids.push(ty.clone());
        }
    }
    ids.sort();
    ids.dedup();
    let mut out = Vec::new();
    for id in ids {
        let name = classes
            .get(&id)
            .cloned()
            .unwrap_or_else(|| name_obj_multi_key(i18n, &[], None, &id));
        let mut sub_list: Vec<Value> = Vec::new();
        if let Some(m) = subs.get(&id) {
            for (sub_id, (sub_name, sub_desc, events)) in m {
                let nm = sub_name
                    .clone()
                    .unwrap_or_else(|| name_obj_multi_key(i18n, &[], None, sub_id));
                let mut o = serde_json::Map::new();
                o.insert("id".into(), json!(sub_id));
                o.insert("name".into(), nm);
                if let Some(d) = sub_desc {
                    o.insert("description".into(), d.clone());
                }
                if !events.is_empty() {
                    // 稳定输出：i18n 为 HashMap（遍历顺序不定），按 (kind, 标题) 排序后落盘
                    let mut evs = events.clone();
                    evs.sort_by(|a, b| {
                        let key_of = |v: &Value| {
                            let kind = v.get("kind").and_then(|x| x.as_str()).unwrap_or("");
                            let t = v
                                .get("title")
                                .and_then(|x| x.get("chinese").or_else(|| x.get("english")))
                                .and_then(|x| x.as_str())
                                .unwrap_or("");
                            format!("{kind}\u{0}{t}")
                        };
                        key_of(a).cmp(&key_of(b))
                    });
                    evs.dedup();
                    o.insert("events".into(), json!(evs));
                }
                sub_list.push(Value::Object(o));
            }
        }
        let mut o = serde_json::Map::new();
        o.insert("id".into(), json!(id));
        o.insert("name".into(), name);
        if !sub_list.is_empty() {
            o.insert("types".into(), json!(sub_list));
        }
        out.push(Value::Object(o));
    }
    out
}

/// 拆分配置内嵌消息：字符串按 `{标题}\n正文` 拆；对象（多语言）整体作标题。
fn split_inline_msg(v: &Value) -> (HashMap<String, String>, HashMap<String, String>) {
    let mut title = HashMap::new();
    let mut text = HashMap::new();
    match v {
        Value::String(s) => match s.strip_prefix('{').and_then(|r| r.split_once('}')) {
            Some((t, rest)) => {
                if !t.trim().is_empty() {
                    title.insert("auto".to_string(), t.to_string());
                }
                let d = rest.trim_start_matches('\n').trim();
                if !d.is_empty() {
                    text.insert("auto".to_string(), d.to_string());
                }
            }
            None => {
                if !s.trim().is_empty() {
                    title.insert("auto".to_string(), s.trim().to_string());
                }
            }
        },
        Value::Object(m) => {
            for (lang, s) in m {
                if let Some(s) = s.as_str() {
                    if !s.trim().is_empty() {
                        title.insert(lang.clone(), s.to_string());
                    }
                }
            }
        }
        _ => {}
    }
    (title, text)
}
