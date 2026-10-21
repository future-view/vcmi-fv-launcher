//! 英雄**特长文本**回归（2026-10-21，构建逻辑修复，`WIKI_CACHE_VERSION` 未变）。
//!
//! 背景：VCMI `CHeroHandler.cpp:59-61` 把 `texts.specialty.name / tooltip / description`
//! 直接注册进文本系统，因此一大批**不带 `translations/`** 的中文 mod 把特长文案全写在配置里。
//! 旧实现这三个字段只用 `name_obj_en_zhcn`（仅查 i18n 翻译表），拿不到 → 三字段空对象 →
//! 英雄详情 / 右键弹窗的整块特长区被 `pickName(spec, lang)` 判空跳过。
//!
//! 本组验证修复后**产物**（cache.json）的不变量：`texts.specialty.*` 内嵌文本必须落到
//! `specialty / specialtyTooltip / specialtyDescription`。
//!
//! 运行：`VCMI_GAME_DIR=<游戏目录> cargo test --lib hero_specialty_text -- --ignored --nocapture`
//! （游戏目录缺省回落仓库 `references/Heroes3-VCMI` fixture，未装对应 mod 时整组跳过）

use super::*;
use crate::modcache::collect::collect_config_jsons_merged;

/// 取 `<游戏目录>/Mods/<相对路径>`，不存在返回 None（用例跳过）。
fn game_mod(mod_rel: &str) -> Option<PathBuf> {
    let p = game_dir().join("Mods").join(mod_rel);
    p.is_dir().then_some(p)
}

/// 全新构建单个 mod 的**阶段一（仅数据）** 并读回 `cache.json`。
///
/// 只跑阶段一：本组断言全在数据层（特长文案），跑素材阶段会白等 HDD 上的 DEF/肖像提取。
/// 每次先删掉本 mod 的缓存目录 —— 构建逻辑的改动不在缓存键里，残留缓存会让断言读到旧产物。
fn build_and_read(mod_dir: &Path, mod_id: &str, game_dir: &Path) -> Value {
    let cache_root = std::env::temp_dir().join("vcmi_fv_hero_specialty_test");
    let mod_cache = cache_root.join(crate::modcache::sanitize_id(mod_id));
    let _ = fs::remove_dir_all(&mod_cache);
    let _ = fs::create_dir_all(&cache_root);
    let r = build_mod_wiki_phase(&mod_dir.to_string_lossy(), mod_id, &cache_root, game_dir, 0);
    assert_eq!(
        r.get("ok").and_then(|x| x.as_bool()),
        Some(true),
        "构建失败 {mod_id}: {r}"
    );
    let text = fs::read_to_string(mod_cache.join("cache.json"))
        .unwrap_or_else(|e| panic!("{mod_id} cache.json 未落盘: {e}"));
    serde_json::from_str(&text).expect("cache.json 解析失败")
}

/// 取英雄的多语言文本字段（优先 chinese，回落 english / auto）。
fn text_of(h: &Value, field: &str) -> String {
    h.get(field)
        .and_then(|v| v.as_object())
        .and_then(|o| {
            ["chinese", "english", "auto"]
                .iter()
                .find_map(|k| o.get(*k).and_then(|x| x.as_str()).filter(|s| !s.is_empty()))
        })
        .unwrap_or("")
        .to_string()
}

/// 从 mod 的 heroes 配置收集每个英雄的 `texts.specialty.*` 原文（空串视为"作者没写"）。
///
/// 用途：断言只对**作者确实写了**的字段要求进缓存 —— 实测 `karmicCourtHero16` 的
/// `tooltip` 就是空串，无条件断言三段非空会误报。
fn inline_spec_sources(mod_dir: &Path) -> std::collections::HashMap<String, [String; 3]> {
    let mut m = std::collections::HashMap::new();
    for (id, obj) in collect_config_jsons_merged(mod_dir, "heroes") {
        let spec = obj.get("texts").and_then(|t| t.get("specialty"));
        let get = |k: &str| -> String {
            spec.and_then(|s| s.get(k))
                .and_then(|v| v.as_str())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or("")
                .to_string()
        };
        m.insert(id, [get("name"), get("description"), get("tooltip")]);
    }
    m
}

/// 逐个英雄断言：作者在 `texts.specialty.*` 里写了的，必须落到缓存对应字段。
fn assert_all_heroes_have_specialty_text(data: &Value, mod_dir: &Path, mod_label: &str) {
    let src = inline_spec_sources(mod_dir);
    let heroes = data
        .get("heroes")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(!heroes.is_empty(), "{mod_label} 未产出英雄");
    let fields = [
        ("specialty", 0usize),
        ("specialtyDescription", 1),
        ("specialtyTooltip", 2),
    ];
    let mut hit = [0usize; 3];
    for h in &heroes {
        let id = h.get("id").and_then(|x| x.as_str()).unwrap_or("?");
        let wanted = src.get(id).cloned().unwrap_or_default();
        for (field, ix) in fields {
            let cached = text_of(h, field);
            if wanted[ix].is_empty() {
                continue;
            }
            assert!(
                !cached.is_empty(),
                "{mod_label} 英雄 {id} 的 {field} 应取自配置内嵌 texts.specialty（不得为空）：{h}"
            );
            hit[ix] += 1;
        }
    }
    for (field, ix) in fields {
        assert!(
            hit[ix] > 0,
            "{mod_label} 没有任何英雄解析出 {field}（三段都要抽样命中，否则说明回退未生效）"
        );
    }
}

/// `karmicCourt`（地府）：文案全内嵌、无 `translations/`，16 个英雄此前特长三字段全空。
#[test]
#[ignore]
fn karmic_court_inline_specialty_text() {
    let Some(mod_dir) = game_mod("karmicCourt") else {
        eprintln!("跳过：未安装 karmicCourt");
        return;
    };
    let game_dir = game_dir();
    crate::vcmi::resource::set_wiki_user_data_dir(&user_data_dir(&game_dir));
    let data = build_and_read(&mod_dir, "karmicCourt", &game_dir);
    assert_all_heroes_have_specialty_text(&data, &mod_dir, "karmicCourt");
}

/// `shuHan`（神州）：同型中文 mod（无翻译目录，特长写在 `texts.specialty` 下）。
#[test]
#[ignore]
fn shu_han_inline_specialty_text() {
    let Some(mod_dir) = game_mod("shuHan") else {
        eprintln!("跳过：未安装 shuHan");
        return;
    };
    let game_dir = game_dir();
    crate::vcmi::resource::set_wiki_user_data_dir(&user_data_dir(&game_dir));
    let data = build_and_read(&mod_dir, "shuHan", &game_dir);
    assert_all_heroes_have_specialty_text(&data, &mod_dir, "shuHan");
}
