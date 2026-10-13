//! 缓存索引（`mods/index.json`）重建与单 mod 缓存读取。
//!
//! - 索引条目由各 mod 的 `mod.json` 摘要 + 扫描期携带的 `diskName`/`parentId`/`type`/`name` 组装；
//! - 索引**以 modId 为 key 合并**：分多批构建时互不覆盖（见 `batch.rs`）。
//!
//! 依赖方向：可依赖 `infra` 与 `vcmi`（见 `docs/architecture/backend.zh-CN.md` §4.1）。

use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use crate::vcmi::paths::VcmiPaths;
use crate::vcmi::resource::{set_wiki_mod_paths, set_wiki_user_data_dir};
use crate::vcmi::scan::cached_mod_paths;
use crate::modcache::build::build_mod_wiki_inner;
use crate::modcache::global::extract_resource_icons_for_mod;
use crate::modcache::sanitize_id;

/// 读取单个城镇 mod 的 Wiki 缓存 JSON（命中缓存时不再解析原始 mod，核心逻辑）。
pub(crate) fn read_mod_wiki_inner(cache_root: &Path, mod_id: &str) -> Value {
    let p = cache_root.join(format!("{}/cache.json", sanitize_id(mod_id)));
    if !p.is_file() {
        return json!({ "ok": false, "error": "notcached" });
    }
    let text = match fs::read_to_string(&p) {
        Ok(t) => t,
        Err(_) => return json!({ "ok": false, "error": "io" }),
    };
    let v: Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return json!({ "ok": false, "error": "parse" }),
    };
    json!({ "ok": true, "data": v })
}

/// 原版伪模组（`core` / `core.<faction>`）的**多语言显示名**；非 core 模组返回 `None`。
///
/// 这类伪模组没有 `mod.json`（名称本应由模组作者以多语言块提供），因此名称一律从**既有数据**派生，
/// 不在任何一侧写死语言标签（早期前端 `CORE_FACTION_NAMES` 只写中文 → 英文界面也显示中文，
/// 且中英文同值）：
/// - `core.<faction>`：城镇名优先取**已构建缓存里的阵营名**（`town_names`：游戏
///   `config/factions/*.json` + 各翻译 mod，语言覆盖最全，可含 tchinese/russian/…），
///   缓存没有时回退随包 h3-core 文案表的 `faction.core.<id>.name`（english + chinese，
///   见 [`crate::vcmi::h3core::load_h3_core_i18n`]），前缀语言中立的引擎名；
/// - `core`：引擎名本身（承载原版全局数据：法术/技能/宝物/地形/中立生物）。
///
/// 两个来源都不可用时回退 `factionId` —— **必须返回非空名**：空名会让 wiki 模组列表出现空白行。
pub(crate) fn core_mod_display_name(
    mod_id: &str,
    town_names: Option<&serde_json::Map<String, Value>>,
) -> Option<Value> {
    /// 归属前缀 / 父模组名：语言中立的引擎名（游戏数据里没有"原版"这类可翻译文案）
    const ENGINE: &str = "VCMI";
    let (is_core, faction) = crate::vcmi::h3core::parse_core_mod(mod_id);
    if !is_core {
        return None;
    }
    let Some(faction) = faction else {
        return Some(json!({ "chinese": ENGINE, "english": ENGINE }));
    };
    // 回退来源：随包 h3-core 文案表（只有 en/zh 两种语言）
    let h3_names: Option<serde_json::Map<String, Value>> = crate::vcmi::h3core::h3_core_data_dir()
        .and_then(|d| crate::vcmi::h3core::load_h3_core_i18n(&d))
        .and_then(|table| table.get(&format!("faction.core.{}.name", faction.to_lowercase())).cloned())
        .map(|m| m.into_iter().map(|(k, v)| (k, Value::String(v))).collect());
    let mut names = serde_json::Map::new();
    if let Some(src) = town_names.filter(|m| !m.is_empty()).or(h3_names.as_ref()) {
        for (lang, v) in src {
            let s = v.as_str().unwrap_or("");
            if !s.trim().is_empty() {
                names.insert(lang.clone(), json!(format!("{ENGINE} · {s}")));
            }
        }
    }
    if names.is_empty() {
        let fallback = format!("{ENGINE} · {faction}");
        names.insert("chinese".to_string(), json!(fallback.clone()));
        names.insert("english".to_string(), json!(fallback));
    }
    Some(Value::Object(names))
}

/// 批量构建多个城镇 mod 的 Wiki 缓存，并汇总生成 `mods/index.json` 索引（核心逻辑，可测试）。
pub(crate) fn build_all_wiki_inner(mods: &Value, cache_root: &Path) -> Value {
    let paths = VcmiPaths::detect();
    let game_dir = paths.game_dir.clone();
    // 让资源回退解析也能搜到 `user_data_dir/Data`（便携运行原版数据常在此处）
    set_wiki_user_data_dir(&paths.user_dir);
    // HD 素材来源：与 batch 入口同口径注入（否则这个遗留入口的产物缺 HD 幕布/兵种背景）
    crate::vcmi::hd_edition::setup_from_config(
        VcmiPaths::read_launcher_config().as_ref(),
        &paths.game_dir,
    );
    // 让资源回退解析也能搜到其它已装模组内的素材（跨模组引用，如子模组 Fen 的精灵被父模组引用）
    set_wiki_mod_paths(&cached_mod_paths(&paths));
    let mut index_mods: Vec<Value> = Vec::new();
    let mut built = 0;
    let mut cached = 0;
    let skipped = 0;
    let mut encrypted = 0;

    // 递归展开 mod 树（含嵌套子 mod），收集所有节点
    fn flatten(node: &Value, out: &mut Vec<Value>) {
        out.push(node.clone());
        if let Some(subs) = node.get("subs").and_then(|x| x.as_array()) {
            for s in subs {
                flatten(s, out);
            }
        }
    }
    let mut nodes: Vec<Value> = Vec::new();
    if let Some(arr) = mods.as_array() {
        for m in arr {
            flatten(m, &mut nodes);
        }
    }
    // 注入原版（Core）伪 mod：从内置 vcmi 目录提取原版 9 城镇/生物/英雄等。
    // `name` 留空 —— 显示名由 `core_mod_display_name` 从数据派生（多语言，见该函数注释）。
    let core_mod_path = game_dir.join("Mods").join("vcmi");
    if core_mod_path.is_dir() {
        nodes.push(json!({
            "id": "core",
            "modPath": core_mod_path.to_string_lossy().to_string(),
            "diskName": "core",
            "parentId": "core",
            "type": "Core",
            "name": "",
            "subs": []
        }));
    }

    for m in &nodes {
        let id = m.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let mod_path = m.get("modPath").and_then(|x| x.as_str()).unwrap_or("").to_string();
        // 跳过内置 vcmi mod（与原版 core 同源，避免重复构建与空名称条目）
        let norm = mod_path.replace('\\', "/");
        let is_vcmi = id == "vcmi" || norm.ends_with("Mods/vcmi") || norm.ends_with("Mods/vcmi/");
        if id.is_empty() || mod_path.is_empty() || is_vcmi {
            continue;
        }
        // 提取 VCMI 资源图标到该 mod 缓存目录，供前端花费显示使用
        extract_resource_icons_for_mod(&game_dir, cache_root, &id);
        let type_str = m.get("type").and_then(|x| x.as_str()).unwrap_or("Other").to_string();
        // 尝试为所有 mod 构建（含生物/英雄/法术等类型；无内容时返回 notown/notfound 属预期）
        let res = build_mod_wiki_inner(&mod_path, &id, cache_root, &game_dir);
        if id == "core" {
            tracing::debug!(target: "wiki-core", "core 构建结果：{res}");
        }
        let cc = res.get("contentCount").cloned().unwrap_or(json!({}));
        if res.get("cached").and_then(|x| x.as_bool()).unwrap_or(false) {
            cached += 1;
        } else if res.get("ok").and_then(|x| x.as_bool()).unwrap_or(false) {
            built += 1;
        } else {
            let err = res.get("error").and_then(|x| x.as_str()).unwrap_or("?").to_string();
            if err == "encrypted" {
                encrypted += 1;
            }
        }
        // 多语言模组名：① 原版伪模组无 mod.json，名称由数据派生（见 `core_mod_display_name`）；
        // ② 其余优先取本 mod 缓存的 modMeta.name（与 mods/realms 页同源）；
        // ③ 缺失时回退 m.name 单串（中英文同值），使 wiki 模组列表随界面语言切换。
        let town_names = res
            .pointer("/factions/0/name")
            .and_then(|x| x.as_object());
        let mod_name_val: Value = if let Some(core_name) = core_mod_display_name(&id, town_names) {
            core_name
        } else {
            let meta_name = res.get("modMeta").and_then(|x| x.get("name")).and_then(|x| x.as_object());
            let fallback = || {
                let s = m.get("name").and_then(|x| x.as_str()).unwrap_or("");
                json!({ "chinese": s, "english": s })
            };
            if let Some(n) = meta_name {
                if !n.is_empty() {
                    Value::Object(n.clone())
                } else {
                    fallback()
                }
            } else {
                fallback()
            }
        };
        index_mods.push(json!({
            "modId": id,
            "diskName": m.get("diskName").and_then(|x| x.as_str()).unwrap_or(""),
            "parentId": m.get("parentId").and_then(|x| x.as_str()).unwrap_or(""),
            "type": type_str,
            "name": mod_name_val,
            "encrypted": m.get("contentZipEncrypted").and_then(|x| x.as_bool()).unwrap_or(false),
            "h3DataMissing": res.get("h3DataMissing").and_then(|x| x.as_bool()).unwrap_or(false),
            "cacheDir": format!("mods/{}", sanitize_id(&id)),
            "contentCount": cc,
        }));
    }

    // 与批量入口（batch.rs）统一顺序口径：按 modId 排序，使 index.json 的 mods 数组确定
    // （扫描顺序受目录枚举影响，不该外泄到产物里）
    index_mods.sort_by(|a, b| {
        a.get("modId")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .cmp(b.get("modId").and_then(|x| x.as_str()).unwrap_or(""))
    });
    let index = json!({
        "generatedAt": crate::infra::time::now_millis(),
        "version": 2,
        "mods": index_mods,
    });
    let idx_path = cache_root.join("index.json");
    if fs::write(&idx_path, serde_json::to_string_pretty(&index).unwrap()).is_err() {
        return json!({ "ok": false, "error": "io", "built": built, "skipped": skipped });
    }
    json!({
        "ok": true,
        "built": built,
        "cached": cached,
        "skipped": skipped,
        "encrypted": encrypted,
        "total": nodes.len(),
        "indexPath": idx_path.to_string_lossy(),
        "totalIndexed": index_mods.len(),
    })
}
