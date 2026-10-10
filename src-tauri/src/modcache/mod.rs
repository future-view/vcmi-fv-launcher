//! 百科（wiki）缓存域：把 mod / 原版数据构建为前端可直读的缓存（`mods/<id>/cache.json` + `images/`）。
//!
//! 依赖方向（见 `docs/architecture/backend.zh-CN.md` §4.1）：
//! 可依赖 `infra` 与 `vcmi`；**不得**依赖 `realms` / `repository` / `commands`，
//! 也不得出现 `tauri` 类型（`State` / `AppHandle` / `Window`）——事件推送由命令层负责。

pub mod assets;
pub mod audit;
pub mod batch;
pub mod build;
pub mod collect;
pub mod global;
pub mod hd;
pub mod hd_map;
pub mod i18n;
pub mod index;
pub mod preview;
pub mod schema;
pub mod showcase;
pub mod version;

use serde_json::{json, Value};

// 原版 core 城镇的 description 不在 VCMI 自身 faction.json 里，而是烘入 h3-core
// （`data/h3-core/<lang>.factions.json` 的 `faction.core.<id>.description`）。
// `cached_town_cards` 用它做回退，使城镇卡片也能显示原版描述。
use crate::vcmi::h3core::{h3_core_data_dir, load_h3_core_i18n};

// 素材提取开关（线程局部，默认开启）。
// 两阶段构建：阶段一关掉（只解析配置出数据，秒出 mod.json，图片字段为 null），
// 阶段二打开（重跑一次并补齐图片后改写 mod.json）。
// 之所以用线程局部而非层层传参：素材落盘函数分布广且被大量调用点使用，
// 加参数会牵动几十处签名；构建在 rayon 工作线程内执行，线程局部语义正确。
thread_local! {
    static WIKI_ASSETS_ENABLED: std::cell::Cell<bool> = const { std::cell::Cell::new(true) };
}

pub(crate) fn assets_enabled() -> bool {
    WIKI_ASSETS_ENABLED.with(|c| c.get())
}

/// 读取已构建缓存里的**模组元数据**（`cache.json` 的 `modMeta`：多语言名/描述 + 作者/版本/类型/依赖）。
///
/// 供 `list_installed_mods`（**mods / realms 页面**）复用——此前这两处只能拿到
/// VCMI `mod.json` 的**英文原文**（见 `src/lib/tauri.ts` 的 InstalledMod.desc 注释），
/// 现在可按界面语言显示中文名/描述。缓存缺失时返回 None（不影响既有字段）。
pub(crate) fn cached_mod_meta(cache_root: &std::path::Path, mod_id: &str) -> Option<serde_json::Value> {
    if cache_root.as_os_str().is_empty() || mod_id.is_empty() {
        return None;
    }
    let dir: String = mod_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let text = std::fs::read_to_string(cache_root.join(dir).join("cache.json")).ok()?;
    let v = crate::infra::json::parse_vcmi_json(&text);
    let meta = v.get("modMeta")?.clone();
    if meta.is_object() {
        Some(meta)
    } else {
        None
    }
}

/// 缓存根下的**目录名索引**（小写 → 磁盘原名），供按 modId 定位缓存目录。
///
/// 缓存目录名沿用**磁盘 mod 目录原名**（混合大小写，如 `Fairytale`），
/// 而 VCMI 的规范 modId 为小写（`fairytale`，见 `references/vcmi/lib/modding/ModManager.cpp`
/// 的 `to_lower_copy`）——预设里读到的是小写，直接拼路径在非 Windows 上会落空。
/// 建一次索引即可在两次 HashMap 查找内完成大小写无关的解析。
fn cache_dir_index(cache_root: &std::path::Path) -> std::collections::HashMap<String, String> {
    let mut m = std::collections::HashMap::new();
    if let Ok(rd) = std::fs::read_dir(cache_root) {
        for e in rd.flatten() {
            if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                if let Some(n) = e.file_name().to_str() {
                    m.insert(n.to_lowercase(), n.to_string());
                }
            }
        }
    }
    m
}

/// 按 modId（大小写无关）解析出缓存目录的磁盘名。
fn resolve_cache_dir(idx: &std::collections::HashMap<String, String>, mod_id: &str) -> Option<String> {
    let key = sanitize_id(mod_id).to_lowercase();
    idx.get(&key).cloned()
}

/// 批量读取**多个模组的元数据**（`cache.json` 的 `modMeta`：多语言名/描述 + 作者/版本/类型/依赖）。
///
/// 与 `cached_mod_meta`（单个）的区别：**一次调用取回 N 个**，避免前端为拿 meta
/// 逐个读整份 `cache.json`。实测本例 652 个 mod 合计 **126 MB** 全量 JSON 解析，
/// 是 mods / realms 页面首屏卡顿的头号元凶。
///
/// 返回 `{ metas: { <modId>: modMeta } }`；缓存缺失的 id 不出现在结果里（前端回落单语言名）。
pub(crate) fn cached_mod_metas(cache_root: &std::path::Path, mod_ids: &[String]) -> Value {
    let idx = cache_dir_index(cache_root);
    let mut metas = serde_json::Map::new();
    for mid in mod_ids {
        let Some(dir) = resolve_cache_dir(&idx, mid) else { continue };
        let Ok(text) = std::fs::read_to_string(cache_root.join(dir).join("cache.json")) else {
            continue;
        };
        let v = crate::infra::json::parse_vcmi_json(&text);
        if let Some(meta) = v.get("modMeta").filter(|m| m.is_object()) {
            metas.insert(mid.clone(), meta.clone());
        }
    }
    json!({ "metas": Value::Object(metas) })
}

/// 批量读取**城镇卡片所需的最小字段**（领域信息页 §9.3.5 的城镇网格专用）。
///
/// 只回传 `{modId, factionId, name, description, preview}`：
/// 相比让前端读整份 `cache.json`（本例 86 个城镇合计 **31 MB**）降到数十 KB，
/// 领域信息页因此不必再为城镇做视口懒加载。
///
/// 返回 `{ cards: [...] }`；无 factions 或缓存缺失的 mod 不产出卡片。
pub(crate) fn cached_town_cards(cache_root: &std::path::Path, mod_ids: &[String]) -> Value {
    let idx = cache_dir_index(cache_root);
    // 一次性加载 h3-core i18n，供下方对原版城镇做 description 回退（避免每个卡片重复加载）
    let h3_i18n = h3_core_data_dir().and_then(|d| load_h3_core_i18n(&d));
    let mut cards: Vec<Value> = Vec::new();
    for mid in mod_ids {
        let Some(dir) = resolve_cache_dir(&idx, mid) else { continue };
        let Ok(text) = std::fs::read_to_string(cache_root.join(dir).join("cache.json")) else {
            continue;
        };
        let v = crate::infra::json::parse_vcmi_json(&text);
        let Some(factions) = v.get("factions").and_then(|x| x.as_array()) else {
            continue;
        };
        for f in factions {
            // 跨 mod 引用条目（`<modId>:<entityId>` 补丁语法，构建期被 `mark_ref_entries`
            // 标上 `ref:true`）：本身没有名称/图片，渲染出来就是「占位块 + 原始 id」的脏卡片。
            // 百科页早就跳过它们，领域信息页同样必须跳过（实测 iceland-terrain-support 里
            // 参考 `new-chinese-mods.stalkerbarge-town:stalkerbarge` 就是这样一张卡）。
            if f.get("ref").and_then(|x| x.as_bool()).unwrap_or(false) {
                continue;
            }
            let fid = f
                .get("id")
                .or_else(|| f.get("factionId"))
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let raw_desc = f.get("description").cloned().unwrap_or(Value::Null);
            // 原版 core 城镇：VCMI faction.json 无 description 字段 → 回退到 h3-core 注入的文本。
            // h3-core 键固定为 `faction.core.<factionId>.description`（见 `h3core.rs`），
            // 所以只对本 scope 为 `core*` 的模组查表，非 core 模组一律不查（省开销）。
            let h3_desc = h3_i18n
                .as_ref()
                .filter(|_| {
                    let l = mid.to_lowercase();
                    l == "core" || l.starts_with("core.")
                })
                .and_then(|m| {
                    let key = format!("faction.core.{}.description", fid).to_lowercase();
                    m.get(&key).map(|inner| json!(inner.clone()))
                });
            let description = if raw_desc.is_null()
                || (raw_desc.is_string() && raw_desc.as_str().unwrap_or("").is_empty())
            {
                h3_desc.unwrap_or(Value::Null)
            } else {
                raw_desc
            };
            cards.push(json!({
                "modId": mid,
                "factionId": fid,
                "name": f.get("name").cloned().unwrap_or(Value::Null),
                "description": description,
                // `background` = 城镇幕布图（`build_towns` 产出：底图 + 建筑叠加，跨文件并集口径）；
                // `preview` = 幕布图引用（构建期直接复用 `background`，两字段通常是**同一文件**；
                // 仅当 background 缺失时才由 `compose_town_preview_for` 兜底合成 `images/townscreen/`）。
                // 两个字段都回传，由前端 `preview ?? background` 决定显示哪张。
                "preview": f.get("preview").cloned().unwrap_or(Value::Null),
                "background": f.get("background").cloned().unwrap_or(Value::Null),
            }));
        }
    }
    json!({ "cards": cards })
}

pub(crate) fn set_assets_enabled(v: bool) {
    WIKI_ASSETS_ENABLED.with(|c| c.set(v))
}

/// 将 mod_id 清洗为安全的缓存文件名
pub(crate) fn sanitize_id(id: &str) -> String {
    id.chars()
        .map(|c| if c.is_alphanumeric() || c == '_' || c == '-' || c == '.' { c } else { '_' })
        .collect()
}

#[cfg(test)]
mod smoke_tests;
