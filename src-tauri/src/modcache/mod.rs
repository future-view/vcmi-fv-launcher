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

/// 已解析的 `cache.json` 小缓存：`key = 路径|mtime|len` → `Arc<Value>`。
///
/// 详情页、右键弹窗、城镇卡片、城镇预览都会读**同一份** `cache.json`
/// （最大 1.7 MB，整份解析实测 33 ms）。缓存后重复访问只付一次 `Value::clone`。
/// 容量由 `infra::cache::CFG_CACHE_CAP`（4）决定：单份解析后约 5~8 MB，4 份 ≈ 30 MB。
static CACHE_JSON_LRU: std::sync::OnceLock<std::sync::Mutex<Vec<(String, std::sync::Arc<Value>)>>> =
    std::sync::OnceLock::new();

/// 读并解析 `cache.json`（按 `(路径, mtime, len)` 缓存解析结果）。
///
/// 返回 `None` = 文件不存在 / 读取失败；解析失败返回 `Value::Null` 的 Arc（调用方按空对象处理）。
pub(crate) fn read_cached_json_path(path: &std::path::Path) -> Option<std::sync::Arc<Value>> {
    let md = std::fs::metadata(path).ok()?;
    let mt = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let key = format!("{}|{}|{}", path.display(), mt, md.len());
    Some(crate::infra::cache::cfg_cache_get(
        &CACHE_JSON_LRU,
        key,
        || match std::fs::read_to_string(path) {
            Ok(t) => crate::infra::json::parse_vcmi_json(&t),
            Err(_) => Value::Null,
        },
    ))
}

/// `cache.json` **原文**的缓存：`key = 路径|mtime|len` → `Arc<String>`。
///
/// 为什么单独缓存文本而不复用 `Arc<Value>`：`Value` 的深拷贝（1.7 MB 的 cache.json
/// 实测约 33 ms）与重新解析**等价**，缓存 Value 毫无收益。原文缓存则让后端零解析，
/// 命令层只把字符串传给前端（前端反正要 `JSON.parse`）。
static CACHE_TEXT_LRU: std::sync::OnceLock<
    std::sync::Mutex<Vec<(String, std::sync::Arc<String>)>>,
> = std::sync::OnceLock::new();

/// 读文件原文（带 `(路径, mtime, len)` 缓存）。
pub(crate) fn read_cached_text(path: &std::path::Path) -> Option<std::sync::Arc<String>> {
    let md = std::fs::metadata(path).ok()?;
    let mt = md
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let key = format!("{}|{}|{}", path.display(), mt, md.len());
    Some(crate::infra::cache::cfg_cache_get(
        &CACHE_TEXT_LRU,
        key,
        || std::fs::read_to_string(path).unwrap_or_default(),
    ))
}

/// 读取已构建缓存里的**模组元数据**（`modMeta`：多语言名/描述 + 作者/版本/类型/依赖）。
///
/// 供 `list_installed_mods`（**mods / realms 页面**）复用——此前这两处只能拿到
/// VCMI `mod.json` 的**英文原文**（见 `src/lib/tauri.ts` 的 InstalledMod.desc 注释），
/// 现在可按界面语言显示中文名/描述。缓存缺失时返回 None（不影响既有字段）。
///
/// 优先读构建期落盘的**独立小文件** `<modId>/meta.json`（几 KB）；旧缓存没有该文件时
/// 回退整份 `cache.json`（最大 1.7 MB / 33 ms）—— 兼容升级前已建好的缓存。
pub(crate) fn cached_mod_meta(
    cache_root: &std::path::Path,
    mod_id: &str,
) -> Option<serde_json::Value> {
    if cache_root.as_os_str().is_empty() || mod_id.is_empty() {
        return None;
    }
    // ⚠️ 目录名必须与落盘口径一致（`sanitize_id`）：旧实现另写一套 sanitize 规则，
    // modId 含空格时落盘目录保留空格、这里却替换成 `_`，导致 meta 永远读不到。
    let dir = cache_root.join(sanitize_id(mod_id));
    if let Some(m) = read_meta_file(&dir) {
        return Some(m);
    }
    let text = std::fs::read_to_string(dir.join("cache.json")).ok()?;
    let v = crate::infra::json::parse_vcmi_json(&text);
    let meta = v.get("modMeta")?.clone();
    if meta.is_object() {
        // 惰性迁移：旧缓存没有 `meta.json`，首次读到就顺手落盘（下次直接读小文件）
        write_meta_file(&dir, &meta);
        Some(meta)
    } else {
        None
    }
}

/// 把 `modMeta` 落成 `<modId>/meta.json`（构建期与惰性迁移共用）。
fn write_meta_file(dir: &std::path::Path, meta: &Value) {
    if let Ok(text) = serde_json::to_string(meta) {
        let _ = std::fs::write(dir.join("meta.json"), text);
    }
}

/// 读 `<modId>/meta.json`（构建期从 `cache.json` 的 `modMeta` 落盘的小文件）。
fn read_meta_file(dir: &std::path::Path) -> Option<Value> {
    let text = std::fs::read_to_string(dir.join("meta.json")).ok()?;
    let v = crate::infra::json::parse_vcmi_json(&text);
    if v.is_object() {
        Some(v)
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
fn resolve_cache_dir(
    idx: &std::collections::HashMap<String, String>,
    mod_id: &str,
) -> Option<String> {
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
        let Some(dir) = resolve_cache_dir(&idx, mid) else {
            continue;
        };
        // 优先独立小文件（几 KB / 亚毫秒）；旧缓存回退整份解析。
        // 577 份 cache.json 合计 58.5 MB，逐份整读实测 785ms —— 这是领域页首屏的主开销。
        let dirp = cache_root.join(&dir);
        if let Some(meta) = read_meta_file(&dirp) {
            metas.insert(mid.clone(), meta);
            continue;
        }
        let Ok(text) = std::fs::read_to_string(dirp.join("cache.json")) else {
            continue;
        };
        let v = crate::infra::json::parse_vcmi_json(&text);
        if let Some(meta) = v.get("modMeta").filter(|m| m.is_object()) {
            // 惰性迁移（同 `cached_mod_meta`）：读到就落盘，下次直接读小文件
            write_meta_file(&dirp, meta);
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
        let Some(dir) = resolve_cache_dir(&idx, mid) else {
            continue;
        };
        // 走解析缓存：领域页一次要读几十份 cache.json，重复进入时不再重复解析
        let Some(v) = read_cached_json_path(&cache_root.join(dir).join("cache.json")) else {
            continue;
        };
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

/// 将 mod_id 清洗为安全的缓存目录名。
///
/// ⚠️ 对**合法 modId 是恒等变换** —— 这是刻意的，理由见下。
///
/// VCMI 的 modId **就是磁盘目录路径**（`lib/modding/ModManager.cpp:44-59`：遍历 `MODS/`
/// 取子目录名、`to_lower_copy` 归一、子模组用 `.` 连接），因此 modId 必然是一个合法目录名，
/// **不可能**含 `\ / : * ? " < > |`。所以保留「字母数字 + `_` `-` `.` + 空格」之后，
/// 替换分支实际上永不触发，缓存目录名与 modId 保持**双射**（可由目录名反推 modId，
/// 跨模组引用 `<modId>:<id>` 也能直接落盘定位）。
///
/// 历史教训：此前只保留 `_-.`、把**空格替换成 `_`**，实测 572 个模组里 50 个中招
/// （`29- Jadelake-town` → `29-_Jadelake-town`、`66expfixmod.New Pavilion` →
/// `66expfixmod.New_Pavilion`、`Forgotten Field---sw` → `Forgotten_Field---sw`）。
/// 后果有二：① 双射被破坏，无法从缓存目录反推 modId；② 与 `a b` / `a_b`、`a,b` / `a.b`
/// 这类真实存在的不同 modId 存在**撞名**隐患（当前侥幸为 0 组，但规则本身不安全）。
pub(crate) fn sanitize_id(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// **产物文件名**的安全化：只替换 Windows 非法字符，**保留 `~`**。
///
/// ⚠️ 与 [`sanitize_id`]（modId→目录名归一）是**两套口径，禁止互用**：
/// - `sanitize_id` 保持 modId 双射，会把 `~` 也替换成 `_`；
/// - 本函数保留 `~` —— 产物名约定为「资源标识 `~` 帧标识」（见 `assets::asset_name`），
///   替换 `~` 会抹掉这层分隔，使 `a~g0f0` 与真实的 `a_g0f0` 撞名。
///
/// 单点定义的原因：`wiki_image_rel`（落盘/存在性判定/cache.json 记录）与
/// `hd_map::record`（HD 登记键，阶段三按**磁盘文件 stem** 反查）必须同口径 ——
/// 曾因登记用了 `sanitize_id`、磁盘用本函数，导致 `cprsmall_def_g0f100`（登记）与
/// `cprsmall_def~g0f100`（磁盘）全线错位，140 条登记无一命中（2026-10-20 实修复）。
pub(crate) fn sanitize_asset_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect()
}

#[cfg(test)]
mod smoke_tests;
