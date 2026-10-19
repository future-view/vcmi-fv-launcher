//! 词汇表（VCMI 游戏内 Wiki 的 Glossary 分类）。
//!
//! VCMI 侧实现在 `client/windows/wiki/WikiWindow.cpp:400-470`：
//! `JsonUtils::assembleFromFiles("config/wikiGlossary.json")` 把所有**激活 mod** 的
//! `entries` 合并成一张表，正文按 Markdown 渲染。启动器按 mod 各自入库、前端按 id 合并，
//! 与其它类目的处理方式一致（缓存不依赖"当前预设激活了哪些 mod"）。
//!
//! 数据形态（实测 `Mods/extra creatures specialty/content/config/wikiGlossary.json`）：
//! ```json
//! { "entries": { "vcmi.wiki.glossary.proverb": {
//!     "name": "vcmi.wiki.glossary.proverb.name",
//!     "description": "vcmi.wiki.glossary.proverb.description" } } }
//! ```
//! `name` / `description` 既可能是翻译 key，也可能直接是文本（VCMI 找不到 key 时按字面量），
//! 统一交给 `name_obj_with_inline` 处理；缺翻译的语言不伪造（见该函数文档）。

use std::collections::HashMap;
use std::path::Path;

use serde_json::{json, Value};

use crate::modcache::collect::collect_config_jsons;
use crate::modcache::i18n::name_obj_with_inline;
use crate::vcmi::h3core::I18nMap;

/// 构建该 mod 的词汇表条目（按显示名升序，与游戏内列表一致）。
pub(crate) fn build_glossary(mod_path: &Path, i18n: &I18nMap) -> Vec<Value> {
    // 同一 mod 内出现重复 id 时后者覆盖前者（VCMI 的 struct 合并语义）
    let mut entries: HashMap<String, Value> = HashMap::new();

    // `collect_config_jsons` 对 `{"entries": {...}}` 返回 `("entries", <entries 映射>)`；
    // 若文件同时带 `categories`，则每个顶层键各一条，故这里按 key 取 `entries`。
    for (key, val) in collect_config_jsons(mod_path, "wikiGlossary") {
        let map = if key.eq_ignore_ascii_case("entries") {
            val.as_object()
        } else {
            val.get("entries").and_then(|x| x.as_object())
        };
        let Some(map) = map else { continue };

        for (id, raw) in map {
            if !raw.is_object() {
                continue;
            }
            let name_raw = raw.get("name");
            let desc_raw = raw.get("description");

            // fallback 传空：缺语言时宁可让前端用 id/中文兜底，也不要把 id 写成 english
            // （glossary 的 `name` 几乎总是翻译 key，见 i18n::build_text_obj 的 inline_is_key）
            let name = match name_raw {
                // 字符串：可能既是翻译 key 也可能是原文 —— 由统一入口判定
                Some(Value::String(s)) => name_obj_with_inline(i18n, s, name_raw, ""),
                Some(v) => name_obj_with_inline(i18n, "", Some(v), ""),
                None => name_obj_with_inline(i18n, "", None, ""),
            };
            // 描述没有兜底名（缺失时保持空对象，前端显示"暂无内容"）
            let description = match desc_raw {
                Some(Value::String(s)) => name_obj_with_inline(i18n, s, desc_raw, ""),
                Some(v) => name_obj_with_inline(i18n, "", Some(v), ""),
                None => json!({}),
            };

            let mut o = serde_json::Map::new();
            o.insert("id".to_string(), json!(id));
            o.insert("name".to_string(), name);
            o.insert("description".to_string(), description);
            // 自定义分类 / 排序：VCMI 支持（`WikiWindow.cpp:407-465`），样本未使用；
            // 保留原字段避免未来接入时丢信息。
            if let Some(c) = raw.get("category").and_then(|x| x.as_str()) {
                o.insert("category".to_string(), json!(c));
            }
            if let Some(n) = raw.get("order").and_then(|x| x.as_i64()) {
                o.insert("order".to_string(), json!(n));
            }
            entries.insert(id.clone(), Value::Object(o));
        }
    }

    let mut out: Vec<Value> = entries.into_values().collect();
    // 与 VCMI `WikiWindow.cpp:467-472` 一致：glossary 按**显示名**升序。
    // Rust 的 str 比较即 UTF-8 字节序，与 C++ std::string 的比较口径一致。
    out.sort_by(|a, b| glossary_sort_key(a).cmp(&glossary_sort_key(b)));
    out
}

fn glossary_sort_key(v: &Value) -> String {
    let name = v.get("name");
    for lang in ["chinese", "english", "auto"] {
        if let Some(s) = name.and_then(|n| n.get(lang)).and_then(|x| x.as_str()) {
            if !s.is_empty() {
                return s.to_string();
            }
        }
    }
    v.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 形态对齐实测的 Expansion mod：小写 `content/`、JSON 带注释与尾逗号、
    /// 翻译在 `config/translations/chinese.json`（mod.json 的 chinese 块声明）。
    #[test]
    fn reads_entries_and_resolves_chinese_translations() {
        let root = std::env::temp_dir().join(format!("vcmi_fv_glossary_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("content/config/translations")).unwrap();
        std::fs::write(
            root.join("mod.json"),
            r#"{"name":"t","chinese":{"translations":["config/translations/chinese.json"]}}"#,
        )
        .unwrap();
        std::fs::write(
            root.join("content/config/wikiGlossary.json"),
            r#"{
                // 注释与尾逗号都必须被宽松解析
                "entries": {
                    "test.b": { "name": "test.b.name", "description": "test.b.description", },
                    "test.a": { "name": "test.a.name", "description": "test.a.description" }
                }
            }"#,
        )
        .unwrap();
        std::fs::write(
            root.join("content/config/translations/chinese.json"),
            r#"{
                "test.a.name": "alpha", "test.a.description": "甲正文",
                "test.b.name": "beta",  "test.b.description": "乙正文"
            }"#,
        )
        .unwrap();

        let i18n = crate::vcmi::i18n::read_i18n_map(&root);
        let out = build_glossary(&root, &i18n);

        assert_eq!(out.len(), 2, "两条 entries 都应产出");
        // 按显示名升序：alpha < beta
        assert_eq!(out[0].get("id").and_then(|x| x.as_str()), Some("test.a"));
        assert_eq!(
            out[0].get("name").and_then(|n| n.get("chinese")).and_then(|x| x.as_str()),
            Some("alpha")
        );
        assert_eq!(
            out[0]
                .get("description")
                .and_then(|n| n.get("chinese"))
                .and_then(|x| x.as_str()),
            Some("甲正文")
        );
        // 只提供中文翻译时，绝不能把翻译 key 写进 english
        // （否则英文界面显示 `test.a.name` / `test.a.description`）。
        assert!(
            out[0].get("name").and_then(|n| n.get("english")).is_none(),
            "name.english 不应是翻译 key"
        );
        assert!(
            out[0].get("description").and_then(|n| n.get("english")).is_none(),
            "description.english 不应是翻译 key"
        );

        let _ = std::fs::remove_dir_all(&root);
    }
}
