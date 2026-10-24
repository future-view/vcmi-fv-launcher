//! 阵营巡礼 slide 组装 + 阵营音乐解析/抽取（设计见 `docs/features/faction-showcase.zh-CN.md` §8）。
//!
//! 依赖方向：可依赖 `infra` 与 `vcmi`（同 `modcache` 其它模块）；不出现 `tauri` 类型。

use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use crate::modcache::index::read_mod_wiki_inner;
use crate::modcache::sanitize_id;
use crate::vcmi::resource::{read_zip_entry_bytes, zip_entry_ext, zip_resource_index};

/// 候选音频扩展名（VCMI 阵营音乐常见格式）
const AUDIO_EXTS: &[&str] = &["mp3", "ogg", "wav", "flac"];
/// zip 内音乐单次抽取上限（25MB），超界跳过（降级领域 BGM，§4.5）
const MAX_AUDIO_BYTES: usize = 25 * 1024 * 1024;

/// 读取阵营巡礼 slide 载荷（裁剪后的 `ShowcaseSlide[]`，见设计 §8.1）。
///
/// `entries: [{ modId, factionId, modPath? }]`。`modPath` 由前端从已扫描 mod 列表带入，
/// 用于解析阵营音乐磁盘路径 / zip 条目；缺失时仅返回磁盘已落地音乐，zip 内音乐返回
/// `musicZip` 供前端调 `ensure_showcase_audio` 异步抽取。
pub(crate) fn read_showcase_slides_inner(
    cache_root: &Path,
    game_dir: &Path,
    entries: &[Value],
) -> Value {
    let mut slides = Vec::with_capacity(entries.len());
    for e in entries {
        let mod_id = e
            .get("modId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let faction_id = e
            .get("factionId")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let mod_path = e
            .get("modPath")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let cache = read_mod_wiki_inner(cache_root, &mod_id);
        let data_opt: Option<Value> = cache.get("data").cloned();
        let Some(data) = data_opt.as_ref() else {
            slides.push(missing_slide(&mod_id, &faction_id, "nocache"));
            continue;
        };

        let faction = data
            .get("factions")
            .and_then(|v| v.as_array())
            .and_then(|arr| {
                arr.iter().find(|f| {
                    f.get("factionId").and_then(|x| x.as_str()) == Some(faction_id.as_str())
                })
            });
        let Some(faction) = faction else {
            slides.push(missing_slide(&mod_id, &faction_id, "nofaction"));
            continue;
        };

        // 兵种格：以 faction.creatureList 为权威清单（已含升级链），按 level 升序、去重。
        let creatures_all = data.get("creatures").and_then(|v| v.as_array());
        let creature_list = faction
            .get("creatureList")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut seen = std::collections::HashSet::new();
        let mut creatures: Vec<Value> = Vec::new();
        for cid in &creature_list {
            let Some(cid) = cid.as_str() else { continue };
            if !seen.insert(cid.to_string()) {
                continue;
            }
            if let Some(c) = creatures_all.and_then(|arr| {
                arr.iter()
                    .find(|c| c.get("id").and_then(|x| x.as_str()) == Some(cid))
            }) {
                creatures.push(json!({
                    "id": c.get("id").cloned().unwrap_or(Value::Null),
                    "name": c.get("name").cloned().unwrap_or(Value::Null),
                    // 描述（多语言对象，与 name 同构）：**手工查看的详情卡**用。
                    // cache 里是 `description`（= CRTRAITS 的 abilityText / 模组翻译键），
                    // 取不到就是 null → 前端显示「暂无描述」，不编造。
                    "description": c.get("description").cloned().unwrap_or(Value::Null),
                    // 该生物的**巢穴建筑名**（多语言 → 名称数组），来自缓存里
                    // 「城镇配置 `building.creatures` 反查」的结果 —— 见 `town.rs` 的 `creature_dwellings`。
                    // 前端用它做「招募建筑」的**权威匹配**：按名称比对建筑清单，
                    // 比"按 level 猜 id"稳（VCMI 的 `dwellingUpLvl<N>` 命名并不总成立，
                    // 有的模组用 `dwelling2up` / 带前缀等写法，猜错就完全没有联动）。
                    "dwellingNames": c.get("dwellings").cloned().unwrap_or(Value::Null),
                    "level": c.get("level").cloned().unwrap_or(Value::Null),
                    "image": c.get("image").cloned().unwrap_or(Value::Null),
                    "imageLarge": c.get("imageLarge").cloned().unwrap_or(Value::Null),
                    // 升级来源（cache.json 的 creature.rs 已落盘）：前端据此区分
                    // 「未升级 / 仅进阶 / 全部」三种 creatureMode（空数组 = 基础形态）。
                    "upgradedFrom": c.get("upgradedFrom").cloned().unwrap_or(Value::Null),
                }));
            }
        }
        creatures.sort_by_key(|c| c.get("level").and_then(|v| v.as_i64()).unwrap_or(0));

        // 英雄格：直接取该 mod 的 heroes[]（§4.3 按 mod 归属）。
        let heroes: Vec<Value> = data
            .get("heroes")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|h| {
                        // 英雄描述取**特长说明**（`specialtyDescription`，短且有信息量），
                        // 为空再退到 `biography`（常是一整段传记，详情卡里太长）。
                        let desc = h
                            .get("specialtyDescription")
                            .filter(|v| !is_empty_text(v))
                            .or_else(|| h.get("biography"))
                            .cloned()
                            .unwrap_or(Value::Null);
                        json!({
                            "id": h.get("id").cloned().unwrap_or(Value::Null),
                            "name": h.get("name").cloned().unwrap_or(Value::Null),
                            "description": desc,
                            "className": h.get("className").cloned().unwrap_or(Value::Null),
                            "image": h.get("image").cloned().unwrap_or(Value::Null),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        // 建筑条
        let buildings: Vec<Value> = faction
            .get("buildings")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|b| {
                        json!({
                            "id": b.get("id").cloned().unwrap_or(Value::Null),
                            "name": b.get("name").cloned().unwrap_or(Value::Null),
                            // 描述（多语言）：cache 的建筑已带 `description`
                            "description": b.get("description").cloned().unwrap_or(Value::Null),
                            "image": b.get("image").cloned().unwrap_or(Value::Null),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        // 幕布：preview ?? background（预览未合成时为 null，§5.8）
        let backdrop = faction
            .get("preview")
            .cloned()
            .or_else(|| faction.get("background").cloned())
            .unwrap_or(Value::Null);

        // 音乐：解析 musicTheme → 磁盘路径 / zip 条目（§4.5）。
        // `musicTheme` 一并回传（诊断用）：前端据此可区分「配置里就没有」与「有 theme 但文件没找到」。
        let music_theme = music_theme_for(cache_root, game_dir, &mod_id, &faction_id, Some(faction));
        let (music, music_zip) = match music_theme.as_deref() {
            Some(th) => resolve_music_from_theme(game_dir, &mod_id, th, mod_path.as_deref()),
            None => (Value::Null, Value::Null),
        };

        slides.push(json!({
            "modId": mod_id,
            "factionId": faction_id,
            "name": faction.get("name").cloned().unwrap_or(Value::Null),
            "description": faction.get("description").cloned().unwrap_or(Value::Null),
            "backdrop": backdrop,
            "music": music,
            "musicZip": music_zip,
            // 解析到的 AudioPath（null = 配置里没有该阵营的 musicTheme）。仅诊断用，前端不参与播放逻辑。
            "musicTheme": music_theme,
            "creatures": creatures,
            "creatureTotal": creatures.len(),
            "heroes": heroes,
            "buildings": buildings,
            "structures": faction.get("structures").cloned().unwrap_or(Value::Null),
        }));
    }
    json!({ "slides": slides })
}

fn missing_slide(mod_id: &str, faction_id: &str, reason: &str) -> Value {
    json!({
        "modId": mod_id,
        "factionId": faction_id,
        "missing": reason,
        "creatures": [],
        "heroes": [],
        "buildings": [],
        "structures": Value::Null,
    })
}

/// 多语言文本对象是否"空"（`null` / 空串 / `{}` / 所有值都是空串）。
/// 用于"有值才回退到下一个候选"的判断 —— 直接 `is_null()` 会把 `{}` 当成有值。
fn is_empty_text(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        Value::Object(m) => m
            .values()
            .all(|x| x.as_str().map(|s| s.trim().is_empty()).unwrap_or(true)),
        _ => false,
    }
}

/// VCMI `AudioPath` 形如 `music/<rest>`；磁盘上的实际文件名是 `<rest>`（**必须去掉 `music/` 前缀**）。
///
/// 实测：原版 `config/factions/castle.json` → `"musicTheme": ["music/CstleTown"]`，
/// 而文件是 `<gameDir>/MP3/CstleTown.mp3`（不是 `MP3/music/CstleTown.mp3`）。
fn strip_music_prefix(theme: &str) -> &str {
    theme.strip_prefix("music/").unwrap_or(theme)
}

/// `musicTheme` 在 VCMI schema 里是 **`string[]`**（实测 `["music/CstleTown"]`），
/// 个别模组也可能写成裸字符串 —— 两种都要能吃下（此前只按 `as_str()` 取值，永远拿不到）。
fn first_music_theme(v: &Value) -> Option<String> {
    if let Some(s) = v.as_str() {
        if !s.is_empty() {
            return Some(s.to_string());
        }
    }
    if let Some(arr) = v.as_array() {
        for x in arr {
            if let Some(s) = x.as_str() {
                if !s.is_empty() {
                    return Some(s.to_string());
                }
            }
        }
    }
    None
}

/// 由 `musicTheme`（AudioPath）解析到可播放的音频：返回 `(music 绝对路径 | null, musicZip 描述 | null)`。
pub(crate) fn resolve_music_from_theme(
    game_dir: &Path,
    mod_id: &str,
    theme: &str,
    mod_path: Option<&str>,
) -> (Value, Value) {
    let rest = strip_music_prefix(theme);

    // 待试的文件名（相对 `music/`），按可靠性降序：
    // ① theme 自带扩展名 → 先试原样（实测 hota.bulwark = `music/factions/BulwarkTown.mp3`，
    //    只按「拼接扩展名」找会拼出 `BulwarkTown.mp3.mp3` 全部落空）；
    // ② 去掉扩展名后按各音频扩展名重试 —— 覆盖「配置写的扩展名与实际文件不符」
    //    （实测 peachville-town 写 `.mp3` 而盘上是 `.ogg`、slothlux-town 同款）。
    let mut names: Vec<String> = Vec::new();
    let lower = rest.to_lowercase();
    let hit_ext = AUDIO_EXTS.iter().find(|e| lower.ends_with(&format!(".{e}")));
    if hit_ext.is_some() {
        names.push(rest.to_string());
    }
    let stem = hit_ext
        .map(|e| &rest[..rest.len() - e.len() - 1])
        .unwrap_or(rest);
    for ext in AUDIO_EXTS {
        let n = format!("{stem}.{ext}");
        if !names.iter().any(|x| x == &n) {
            names.push(n);
        }
    }

    // 1) 磁盘：mod 的 Content/music/<name>
    if let Some(mp) = mod_path {
        for n in &names {
            let p = Path::new(mp).join("Content").join("music").join(n);
            if p.is_file() {
                return (json!(p.to_string_lossy().to_string()), Value::Null);
            }
        }
    }
    // 2) 磁盘：游戏根 MP3/<name>
    for n in &names {
        let p = game_dir.join("MP3").join(n);
        if p.is_file() {
            return (json!(p.to_string_lossy().to_string()), Value::Null);
        }
    }
    // 3) zip 内：content.zip（需 mod_path；命中即返回描述供异步抽取，§4.5.3）
    if let Some(mp) = mod_path {
        for n in &names {
            let rel = format!("music/{n}");
            if zip_resource_index(Path::new(mp), &rel).is_some() {
                return (Value::Null, json!({ "modId": mod_id, "rel": rel }));
            }
        }
    }
    (Value::Null, Value::Null)
}

/// 取阵营 `musicTheme`（AudioPath，可能带扩展名）。三级来源，按可靠性降序：
/// - 缓存阵营条目 `cache.json` → `factions[].musicTheme`（构建期解析，覆盖全部配置落点）；
/// - 第三方 Town 模组：`raw.json` → `rawConfigs.towns[<factionId>].town.musicTheme`（只含 `config/**` 树）；
/// - 原版 / `core.<faction>`：游戏根 `config/factions/<factionId>.json`（见下）。
fn music_theme_for(
    cache_root: &Path,
    game_dir: &Path,
    mod_id: &str,
    faction_id: &str,
    faction: Option<&Value>,
) -> Option<String> {
    // 0) 构建期已解析（`cache.json` 的 `factions[].musicTheme`）：**权威入口**。
    // 它由 `read_town_configs` 的变体目录/zip 扫描产出，覆盖全部配置落点；
    // 而 `raw.json` 只聚合 `config/**` 树 —— 实测沙堡（New Pavilion）的 `faction.json`
    // 在 `Content/townPavilion/` 下，只认 raw.json 就永远读不到它的 musicTheme。
    if let Some(th) = faction
        .and_then(|f| f.get("musicTheme"))
        .and_then(first_music_theme)
    {
        return Some(th);
    }
    let raw_path = cache_root.join(format!("{}/raw.json", sanitize_id(mod_id)));
    if let Some(t) = read_music_theme_from_raw(&raw_path, faction_id) {
        return Some(t);
    }
    let gp = game_dir
        .join("config")
        .join("factions")
        .join(format!("{faction_id}.json"));
    if let Ok(text) = fs::read_to_string(&gp) {
        // ⚠️ 游戏根配置**含 `//` 注释**，`serde_json` 会直接解析失败 → 必须用项目宽容解析器。
        let v = crate::infra::json::parse_vcmi_json(&text);
        if let Some(th) = game_root_music_theme(&v, faction_id) {
            return Some(th);
        }
    }
    None
}

/// 从游戏根 `config/factions/<fid>.json` 中取 `musicTheme`。
///
/// ⚠️ 实测该文件的真实结构是**以阵营 id 再包一层**：
/// ```jsonc
/// { "rampart": { "town": { "musicTheme": ["music/Rampart"], ... } } }
/// ```
/// **不是** `{ "town": { ... } }` —— 此前只查 `v.town` / `v.musicTheme`，两条都落空，
/// 这正是「原版 9 族没有阵营音乐」的根因（第三方 mod 的 `rawConfigs.towns` 才是平铺的）。
fn game_root_music_theme(v: &Value, faction_id: &str) -> Option<String> {
    /// 从一个「阵营节点」里取 `town.musicTheme`（兼容无 `town` 包裹的扁平写法）。
    fn pick(node: &Value) -> Option<String> {
        node.get("town")
            .and_then(|t| t.get("musicTheme"))
            .and_then(first_music_theme)
            .or_else(|| node.get("musicTheme").and_then(first_music_theme))
    }
    // ① 阵营 id 包一层（实测结构）
    if let Some(node) = v.get(faction_id) {
        if let Some(t) = pick(node) {
            return Some(t);
        }
    }
    // ② 大小写不敏感兜底（配置里的 key 与 factionId 大小写可能不一致）
    if let Some(obj) = v.as_object() {
        for (k, node) in obj {
            if k.eq_ignore_ascii_case(faction_id) {
                if let Some(t) = pick(node) {
                    return Some(t);
                }
            }
        }
    }
    // ③ 兼容扁平写法
    pick(v)
}

fn read_music_theme_from_raw(raw_path: &Path, faction_id: &str) -> Option<String> {
    let text = fs::read_to_string(raw_path).ok()?;
    let v: Value = crate::infra::json::parse_vcmi_json(&text);
    let towns = v
        .get("rawConfigs")
        .and_then(|r| r.get("towns"))
        .and_then(|t| t.as_object())?;
    let town = towns
        .get(faction_id)
        .or_else(|| towns.get(&faction_id.to_lowercase()))?;
    town.get("town")
        .and_then(|x| x.get("musicTheme"))
        .and_then(first_music_theme)
        .or_else(|| town.get("musicTheme").and_then(first_music_theme))
}

/// zip 内音乐单条目抽取落盘，返回绝对路径（已存在直接返回；>25MB 跳过返回 null）。
pub(crate) fn ensure_showcase_audio_inner(
    cache_root: &Path,
    mod_id: &str,
    mod_path: &Path,
    rel: &str,
) -> Option<String> {
    let index = zip_resource_index(mod_path, rel)?;
    let buf = read_zip_entry_bytes(mod_path, index)?;
    if buf.len() > MAX_AUDIO_BYTES {
        return None;
    }
    let ext = zip_entry_ext(mod_path, index).unwrap_or_else(|| default_ext(rel));
    let out_dir = cache_root.join(format!("{}/audio", sanitize_id(mod_id)));
    let _ = fs::create_dir_all(&out_dir);
    let fname = rel.replace(['/', '\\', ':'], "_");
    let out_path = out_dir.join(format!("{fname}.{ext}"));
    if out_path.is_file() {
        return Some(out_path.to_string_lossy().to_string());
    }
    if fs::write(&out_path, &buf).is_ok() {
        Some(out_path.to_string_lossy().to_string())
    } else {
        None
    }
}

fn default_ext(rel: &str) -> String {
    Path::new(rel)
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_lowercase())
        .unwrap_or_else(|| "mp3".to_string())
}
