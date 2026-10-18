//! 生成模组（`chronicles` / `hd-edition`）的 **mod.json 文案表**。
//!
//! # 为什么文案放在 JSON 而不是 `.rs`
//!
//! 译文属于**数据**，不属于逻辑。全部放在 `resources/mod-meta.json`，用
//! [`include_str!`] 编译期嵌入（与 `modcache/schema.rs` 的 `mod-cache-protocol.json`
//! 同款）：改文案是纯 JSON diff，`.rs` 只保留"字段怎么拼"的契约。
//!
//! 这里**不**复用 `h3-core` 那套"运行期找 data 目录 + 内容指纹"：
//! 那套是为"多语言 × 十几类 × 上千条、由脚本独立生成"的数据准备的；本表只有几十条、
//! 与二进制同版本发布，编译期嵌入可以彻底避免"打包漏文件 → 静默退英文"的失败路径。
//!
//! # 多语言遵循 VCMI 的 mod 翻译方案
//!
//! `references/vcmi/docs/modders/Mod_File_Format.md:209-223`：`mod.json` 里以**语言全名**
//! 为键写语言块（`chinese` / `polish` / `russian`…，取值见
//! `config/schemas/mod.json:182` 的 29 项枚举），块内可覆盖 `name` / `description` /
//! `author` / `translations`。`ModDescription::getLocalizedValue`
//! （`lib/modding/ModDescription.cpp:136-163`）是**逐字段**回退到根字段，所以块里只写
//! 有译文的字段即可；根字段保留英文作为兜底。
//!
//! 本模块只产出**元数据**语言块。两个模组都不引入游戏内字符串（纯素材 + 地图），
//! 因此不需要 `content/translation/<lang>.json`（空表没有意义，官方也不生成）。
//!
//! # 译文出处（不自行翻译）
//!
//! - 启动器目录 `launcher/translation/<lang>.ts` 的 `Heroes Chronicles`
//!   —— 与官方导入器 `tr("Heroes Chronicles")` 完全同源（12 种语言已完成）；
//! - 俄语在该目录仍是 unfinished，取核心模组
//!   `Mods/vcmi/Content/config/translations/russian.json` 的
//!   `vcmi.campaignSet.chronicles`（同一短语的游戏内官方译名）；
//! - 篇名（`Warlords of the Wasteland`…）在任何官方目录里都没有译文，保持英文
//!   （官方产物同样是英文）。

use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::Deserialize;
use serde_json::{json, Map, Value};

/// 编译期嵌入的文案表（`src-tauri/resources/mod-meta.json`）。
const META_JSON: &str = include_str!("../../resources/mod-meta.json");

#[derive(Debug, Deserialize)]
struct MetaFile {
    chronicles: ChroniclesMeta,
    #[serde(rename = "hd-edition")]
    hd_edition: HdMeta,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ChroniclesMeta {
    name: String,
    description: String,
    author: String,
    version: String,
    contact: String,
    /// 篇章 `name` 模板（`%1` = 篇号，`%2` = 篇名）
    chapter_name: String,
    /// 篇章 `description` 模板（同上）
    chapter_description: String,
    #[serde(default)]
    languages: BTreeMap<String, LangMeta>,
}

/// 单个语言的覆盖项；`None` = 沿用根字段（逐字段回退，见模块文档）。
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LangMeta {
    name: Option<String>,
    description: Option<String>,
    chapter_description: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HdMeta {
    name: String,
    description: String,
    author: String,
    version: String,
    contact: String,
    /// `HD (x%1)`
    scale_name: String,
    /// `Resources (x%1)`
    scale_description: String,
    /// `HD Localisation (%1) (x%2)`（`%1` = 语言全名，`%2` = 倍率）
    loc_name: String,
    /// `Translated Resources (x%1)`
    loc_description: String,
}

fn meta() -> &'static MetaFile {
    static META: OnceLock<MetaFile> = OnceLock::new();
    META.get_or_init(|| {
        serde_json::from_str(META_JSON)
            .expect("resources/mod-meta.json 解析失败（编译期嵌入 ⇒ 单测与首次调用即暴露）")
    })
}

/// 极简占位符替换：`%1`、`%2`…（与官方 `QString::arg` 的编号语义一致）。
fn fill(template: &str, args: &[&str]) -> String {
    let mut out = template.to_string();
    for (i, a) in args.iter().enumerate() {
        out = out.replace(&format!("%{}", i + 1), a);
    }
    out
}

/// 各模组共用的身份字段（`name` / `description` / `author` / `version` / `contact`）。
fn identity_fields(
    name: &str,
    description: &str,
    author: &str,
    version: &str,
    contact: &str,
) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("name".to_string(), json!(name));
    out.insert("description".to_string(), json!(description));
    out.insert("author".to_string(), json!(author));
    out.insert("version".to_string(), json!(version));
    out.insert("contact".to_string(), json!(contact));
    out
}

/// 历代记**基础模组**的元数据字段（`modType` / `heroes` / `settings` 由调用方补齐）。
///
/// 语言块里同时覆盖 `name` 与 `description`：官方基础模组这两者是同一句
/// `tr("Heroes Chronicles")`。
pub fn chronicles_base_fields() -> Map<String, Value> {
    let m = &meta().chronicles;
    let mut out = identity_fields(&m.name, &m.description, &m.author, &m.version, &m.contact);
    for (lang, loc) in &m.languages {
        let Some(name) = &loc.name else { continue };
        let mut block = Map::new();
        block.insert("name".to_string(), json!(name));
        block.insert(
            "description".to_string(),
            json!(loc.description.as_ref().unwrap_or(name)),
        );
        out.insert(lang.clone(), Value::Object(block));
    }
    out
}

/// 历代记**篇章子模组**的元数据字段（`modType` 由调用方补齐）。
///
/// 官方 `createChronicleMod`（`chroniclesextractor.cpp:145-158`）：
/// `name = "%1 - %2"`、`description = tr("Heroes Chronicles %1 - %2")`。
/// 篇名本身无官方译文，故语言块只覆盖 `description`（前缀翻译），`name` 保持根值。
pub fn chronicles_chapter_fields(no: usize, title: &str) -> Map<String, Value> {
    let m = &meta().chronicles;
    let num = no.to_string();
    let mut out = identity_fields(
        &fill(&m.chapter_name, &[&num, title]),
        &fill(&m.chapter_description, &[&num, title]),
        &m.author,
        &m.version,
        &m.contact,
    );
    for (lang, loc) in &m.languages {
        let Some(tpl) = chapter_description_template(loc) else {
            continue;
        };
        out.insert(
            lang.clone(),
            json!({ "description": fill(&tpl, &[&num, title]) }),
        );
    }
    out
}

/// 篇章 `description` 的语言模板。
///
/// 官方启动器里所有语言的模板都等于「译名 + ` %1 - %2`」（`launcher/translation/*.ts`），
/// 所以 JSON 只写译名即可，避免 13 份重复模板漂移；个别语言若要特殊句式，
/// 显式写 `chapterDescription` 覆盖。
fn chapter_description_template(loc: &LangMeta) -> Option<String> {
    if let Some(tpl) = &loc.chapter_description {
        return Some(tpl.clone());
    }
    loc.name.as_ref().map(|n| format!("{n} %1 - %2"))
}

/// HD 父模组显示名（导入报告 / 前端通知用，避免与 `mod.json` 文案各写一份）。
pub fn hd_name() -> &'static str {
    &meta().hd_edition.name
}

/// HD 素材**父模组**（官方 `hdextractor.cpp:150-165` 的 nullopt 分支）。
pub fn hd_parent() -> Value {
    let h = &meta().hd_edition;
    let mut out = Map::new();
    out.insert("modType".to_string(), json!("Graphical"));
    out.extend(identity_fields(
        &h.name,
        &h.description,
        &h.author,
        &h.version,
        &h.contact,
    ));
    Value::Object(out)
}

/// HD 倍率子模组 `x2` / `x3`（官方 `hdextractor.cpp:134-144`）。
pub fn hd_scale(scale: u32) -> Value {
    let h = &meta().hd_edition;
    let s = scale.to_string();
    let mut out = Map::new();
    out.insert("modType".to_string(), json!("Graphical"));
    out.extend(identity_fields(
        &fill(&h.scale_name, &[&s]),
        &fill(&h.scale_description, &[&s]),
        &h.author,
        &h.version,
        &h.contact,
    ));
    Value::Object(out)
}

/// HD 本地化子模组（官方 `hdextractor.cpp:122-133`）：`modType=Translation` +
/// `language` 声明素材适用的语言（语言全名，非短码）。
pub fn hd_localization(language: &str, scale: u32) -> Value {
    let h = &meta().hd_edition;
    let s = scale.to_string();
    let mut out = Map::new();
    out.insert("modType".to_string(), json!("Translation"));
    out.extend(identity_fields(
        &fill(&h.loc_name, &[language, &s]),
        &fill(&h.loc_description, &[&s]),
        &h.author,
        &h.version,
        &h.contact,
    ));
    out.insert("language".to_string(), json!(language));
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// VCMI 合法语言键（`config/schemas/mod.json:182` 的 `language` 枚举，29 项）。
    const VCMI_LANGUAGES: [&str; 29] = [
        "belarusian",
        "bulgarian",
        "czech",
        "chinese",
        "tchinese",
        "dutch",
        "english",
        "filipino",
        "finnish",
        "french",
        "german",
        "greek",
        "hungarian",
        "italian",
        "japanese",
        "korean",
        "latvian",
        "lithuanian",
        "norwegian",
        "polish",
        "portuguese",
        "romanian",
        "russian",
        "serbian",
        "spanish",
        "swedish",
        "turkish",
        "ukrainian",
        "vietnamese",
    ];

    /// 语言键必须是 VCMI 认得的全名（写 `zh-CN` / `en` 之类会让语言块整块失效）。
    #[test]
    fn language_keys_are_vcmi_languages() {
        let m = &meta().chronicles;
        assert!(!m.languages.is_empty(), "至少要有中文/英文之外的覆盖项");
        for key in m.languages.keys() {
            assert!(
                VCMI_LANGUAGES.contains(&key.as_str()),
                "非法语言键 `{key}`（应取 VCMI 语言全名）"
            );
        }
        // 英文走根字段，不应出现在语言块里（重复且易漂移）
        assert!(!m.languages.contains_key("english"), "english 应留在根字段");
    }

    /// 模板占位符必须存在且只剩 `%1`/`%2`，避免渲染出字面量 `%1`。
    #[test]
    fn templates_are_well_formed() {
        let m = &meta().chronicles;
        assert_eq!(m.chapter_name.matches("%1").count(), 1);
        assert_eq!(m.chapter_name.matches("%2").count(), 1);
        assert_eq!(m.chapter_description.matches("%1").count(), 1);
        assert_eq!(m.chapter_description.matches("%2").count(), 1);
        for (lang, loc) in &m.languages {
            let Some(n) = &loc.name else {
                continue;
            };
            assert!(!n.trim().is_empty(), "{lang} 名称为空");
            let tpl = chapter_description_template(loc).expect("有译名就应有模板");
            assert_eq!(tpl.matches("%1").count(), 1, "{lang} 缺少 %1");
            assert_eq!(tpl.matches("%2").count(), 1, "{lang} 缺少 %2");
            // 模板必须由译名派生（或显式覆盖），不能残留英文前缀
            assert!(
                tpl.starts_with(n.as_str()),
                "{lang} 模板 `{tpl}` 应以译名 `{n}` 开头"
            );
        }
        let h = &meta().hd_edition;
        assert_eq!(h.scale_name.matches("%1").count(), 1);
        assert_eq!(h.scale_description.matches("%1").count(), 1);
        assert_eq!(h.loc_name.matches("%1").count(), 1);
        assert_eq!(h.loc_name.matches("%2").count(), 1);
        assert_eq!(h.loc_description.matches("%1").count(), 1);
    }

    /// 渲染结果：根字段英文 + 逐语言块；篇号/篇名正确展开。
    #[test]
    fn chronicles_render_matches_official_and_translates() {
        let base = Value::Object(chronicles_base_fields());
        assert_eq!(base["name"], "Heroes Chronicles");
        assert_eq!(base["description"], "Heroes Chronicles");
        assert_eq!(base["chinese"]["name"], "英雄无敌历代记");
        assert_eq!(base["chinese"]["description"], "英雄无敌历代记");
        assert_eq!(base["russian"]["name"], "Хроники Героев");
        assert!(base.get("english").is_none(), "english 不应有语言块");

        let ch = Value::Object(chronicles_chapter_fields(1, "Warlords of the Wasteland"));
        assert_eq!(ch["name"], "1 - Warlords of the Wasteland");
        assert_eq!(
            ch["description"],
            "Heroes Chronicles 1 - Warlords of the Wasteland"
        );
        assert_eq!(
            ch["chinese"]["description"],
            "英雄无敌历代记 1 - Warlords of the Wasteland"
        );
        assert_eq!(
            ch["polish"]["description"],
            "Heroes Kroniki 1 - Warlords of the Wasteland"
        );
        // 篇名不翻译 ⇒ 语言块不覆盖 name（逐字段回退到根值）
        assert!(ch["chinese"].get("name").is_none());
        // 渲染后不得残留占位符
        let text = serde_json::to_string(&ch).unwrap();
        assert!(!text.contains("%1") && !text.contains("%2"), "{text}");
    }

    /// HD 三个层级的文案（父 / 倍率 / 本地化）与官方逐字一致。
    #[test]
    fn hd_render_matches_official() {
        let parent = hd_parent();
        assert_eq!(parent["modType"], "Graphical");
        assert_eq!(parent["name"], "Heroes III HD Edition");
        assert_eq!(
            parent["description"],
            "Extracted resources from official Heroes HD to make it usable on VCMI"
        );

        let x2 = hd_scale(2);
        assert_eq!(x2["name"], "HD (x2)");
        assert_eq!(x2["description"], "Resources (x2)");

        let loc = hd_localization("english", 2);
        assert_eq!(loc["modType"], "Translation");
        assert_eq!(loc["name"], "HD Localisation (english) (x2)");
        assert_eq!(loc["description"], "Translated Resources (x2)");
        assert_eq!(loc["language"], "english");
    }
}
