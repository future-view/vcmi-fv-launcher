//! 城镇（faction）配置读取：`Content/config/**/town/**` 与游戏根 `config/factions/*.json`。
//!
//! 覆盖城镇背景 / 生物背景 / 建筑列表与图标 / 阵营元信息，对齐 VCMI `CTownHandler`。
//! 原版（core 伪模组）的城镇来自游戏根 `config/factions/<id>.json`（见 [`read_game_root_factions`]），
//! 与第三方 town mod 的 `Content/config/<faction>/town/**` 形态不同，但归一化为同一 [`TownConfig`]。
//!
//! 依赖方向：只依赖 `infra`（见 `docs/architecture/backend.zh-CN.md` §4.1）。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::infra::cache::{cfg_cache_get, fingerprint_of};
use crate::infra::json::{
    collect_json_files, parse_vcmi_json, read_vcmi_json, read_vcmi_json_bytes,
};
use crate::vcmi::gamedata::push_config_object;
use crate::vcmi::h3core::{h3_building_cost, CORE_FACTIONS};
use crate::vcmi::paths::VcmiPaths;

/// 单个城镇（faction）的配置信息。
#[derive(Debug, Clone)]
pub(crate) struct TownConfig {
    pub(crate) faction_id: String,
    pub(crate) town_background: Option<String>,
    pub(crate) creature_background: Option<String>, // 已解析为具体资源路径（取 130px 或首个）
    /// 该城镇可招募的生物 id（按等级从 town.json 的 `creatures` 数组展平）。
    pub(crate) creature_ids: Vec<String>,
    /// 该城镇的建筑 id 列表（town.json / faction.json 的 `buildings` 对象 keys）。
    pub(crate) building_ids: Vec<String>,
    /// 阵营善恶（alignment）：good/evil/neutral。
    pub(crate) alignment: String,
    /// 本族地形（nativeTerrain）。
    pub(crate) native_terrain: String,
    /// 城镇名（`town.json` / `faction.json` 的 `name`，可能是字符串或多语言对象）。
    /// 部分模组**没有 translation 目录**，直接把中文名写在配置里；
    /// 若只走 i18n 键取名会未命中而回落成英文 id（表现为中英混杂），故保留兜底。
    ///
    /// 保留**原始 JSON 形态**（不在此处坍缩成字符串）：多语言对象要按语言键原样透传，
    /// 裸字符串则按「语言未知的原文」交给 `name_obj_with_inline` 处理。
    pub(crate) town_name: Option<Value>,
    /// 城镇自带描述（`town.json` 顶层 `description`，可能是字符串或多语言对象）。
    ///
    /// 多数中文城镇模组把整段设定（「种族阵营 / 种族地形 / 所在地」+ 正文）全写在这一个字段里
    /// （实测 `bastille/town.json`），而 mod.json 的 description 往往只有一句简介 ——
    /// 用后者会把前者整段顶掉，所以城镇自己的描述必须优先。
    pub(crate) town_desc: Option<Value>,
    /// 建筑详情（id + cost），用于百科建筑表显示花费。
    pub(crate) buildings_detail: Vec<Value>,
    /// 城镇界面建筑图标 DEF（clientInfo.buildingsIcons），帧号 = 建筑数字 id（bid）。
    pub(crate) buildings_icons: Option<String>,
    /// 城镇界面**建筑动画** DEF 资源名：`town.structures` 的 `<bid> -> { animation: "xxx.def" }`。
    /// 只保存资源名（纯文本引用，零解码成本）；实际 DEF 解码由详情页按需懒加载（设计 §11），
    /// 不在构建期提取——原版 town-screen 有 2452 个 .def，全量预解会显著拖慢后台构建。
    pub(crate) building_animations: HashMap<String, String>,
    /// 建筑屏幕位置（`town.structures` / `clientInfo.townScreen.structures`）：
    /// key → `{ animation, x, y, z, builds }`。用于把建筑精灵叠加到**底图**，
    /// 使原版城镇背景（只有 base 图）与其它已预合成背景的城镇视觉一致。
    pub(crate) structures: HashMap<String, Value>,
    /// 建筑原始对象（含 `upgrades`），供叠加时的升级链去重（与 `compose_town_preview_for` 一致）。
    pub(crate) buildings_raw: HashMap<String, Value>,
    /// 阵营主题曲（`town.musicTheme`，VCMI `AudioPath` 形如 `music/BastionTown`）。
    ///
    /// 实测 `New Pavilion`（沙堡）把它写在 `Content/townPavilion/faction.json`——该文件由
    /// [`read_town_configs`] 的变体目录扫描命中，却**不在 `config/**` 树下**，因此进不了
    /// `raw.json` 的 `rawConfigs.towns`（那是另一套只聚合 `config/**` 的收集口径）。
    /// 只依赖 raw.json 取主题曲会让这类模组永远「没有阵营音乐」，故在配置解析层统一带出。
    pub(crate) music_theme: Option<String>,
}

pub(crate) static BUILDING_ID_CACHE: std::sync::OnceLock<
    Mutex<Vec<(String, std::sync::Arc<HashMap<String, i64>>)>>,
> = std::sync::OnceLock::new();

/// 建筑 bid 标准表（候选路径见 building_id_candidates）。
pub(crate) fn cached_building_id_library(game_dir: &Path) -> std::sync::Arc<HashMap<String, i64>> {
    let cands = building_id_candidates(game_dir);
    let key = format!("bid:{}", fingerprint_of(&cands));
    cfg_cache_get(&BUILDING_ID_CACHE, key, || {
        read_building_id_library(game_dir)
    })
}

/// VCMI 标准建筑表 `config/buildingsLibrary.json`：建筑标识 → 数字 bid。
///
/// 多数城镇 mod 的建筑只写名称（如 `mageGuild1`）而不写 `id`，而 VCMI 的
/// `CTownHandler::loadBuilding` 取 `source["id"]` 得到 bid，百科建筑表再用
/// `CAnimImage(clientInfo.buildingsIcons, bid)` 取帧，故必须还原 bid 才能对齐。
/// 该文件含注释与尾逗号，需用宽容解析器。
pub(crate) fn building_id_candidates(game_dir: &Path) -> Vec<PathBuf> {
    vec![
        game_dir.join("config").join("buildingsLibrary.json"),
        game_dir
            .join("Mods")
            .join("vcmi")
            .join("config")
            .join("buildingsLibrary.json"),
        VcmiPaths::detect()
            .game_dir
            .join("config")
            .join("buildingsLibrary.json"),
    ]
}

pub(crate) fn read_building_id_library(game_dir: &Path) -> HashMap<String, i64> {
    let mut map = HashMap::new();
    for cand in building_id_candidates(game_dir) {
        if !cand.is_file() {
            continue;
        }
        let text = fs::read_to_string(&cand).unwrap_or_default();
        let v = parse_vcmi_json(&text);
        if let Some(obj) = v.as_object() {
            for (k, val) in obj {
                if let Some(id) = val.get("id").and_then(|x| x.as_i64()) {
                    map.insert(k.clone(), id);
                }
            }
        }
        if !map.is_empty() {
            break;
        }
    }
    map
}

/// 用 VCMI 标准表校正建筑 bid（百科建筑图标按 bid 从 buildingsIcons 取帧）。
pub(crate) fn apply_building_id_library(configs: &mut [TownConfig], lib: &HashMap<String, i64>) {
    if lib.is_empty() {
        return;
    }
    for cfg in configs.iter_mut() {
        for d in cfg.buildings_detail.iter_mut() {
            let ident = d
                .get("id")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            if ident.is_empty() {
                continue;
            }
            if let Some(bid) = lib.get(&ident).copied() {
                if let Some(obj) = d.as_object_mut() {
                    obj.insert("bid".into(), json!(bid));
                }
            }
        }
    }
}

/// 解析建筑数字 bid：配置 `id` 字段 → 配置内顺序（保底，后续由标准表校正）。
pub(crate) fn resolve_building_bid(v: &Value, order_index: usize) -> i64 {
    v.get("id")
        .and_then(|x| x.as_i64())
        .unwrap_or(order_index as i64)
}

/// 从 mod 目录读取所有 town 配置。
/// VCMI 的城镇 faction 配置文件名不固定（town.json / faction.json / Faction.json，
/// 均在 `config/**/town/` 目录下），故扫描该目录下所有 `.json`，
/// 仅保留解析出 townBackground / creatureBackground 的配置，避免遗漏。
pub(crate) fn read_town_configs(mod_path: &Path) -> Vec<TownConfig> {
    let mut configs = Vec::new();
    let mut seen = std::collections::HashSet::new();

    // 0. 目录索引（两遍扫描第一遍）：磁盘 Content 全量 + zip 条目名 → `目录 → 条目名集合`，
    //    供**变体城镇目录**判定（`config/Abode/buildings.json` 无 `town/` 目录层的写法，
    //    见 is_town_config_path_in；单看路径字符串无法与普通数据目录区分）。
    let mut dir_idx: DirFileIndex = DirFileIndex::new();
    let content_dir = mod_path.join("Content");
    if content_dir.is_dir() {
        collect_dir_index(&content_dir, &mut dir_idx);
    }
    let zip_path = mod_path.join("content.zip");
    let mut zip_archive = None;
    if let Ok(file) = fs::File::open(&zip_path) {
        if let Ok(archive) = zip::ZipArchive::new(file) {
            zip_archive = Some(archive);
        }
    }
    if let Some(archive) = zip_archive.as_mut() {
        for i in 0..archive.len() {
            if let Ok(entry) = archive.by_index(i) {
                attach_index_entry(&mut dir_idx, entry.name());
            }
        }
    }

    // 1. Content 目录下的城镇配置（含变体目录）
    if content_dir.is_dir() {
        collect_town_jsons_dir(&content_dir, &dir_idx, &mut configs, &mut seen);
    }

    // 2. content.zip 内城镇配置（第二遍：按索引判定后读内容）
    if let Some(archive) = zip_archive.as_mut() {
        for i in 0..archive.len() {
            if let Ok(mut entry) = archive.by_index(i) {
                let name = entry.name().to_string();
                if is_town_config_path_in(&name, &dir_idx) {
                    let key = name.to_lowercase();
                    if seen.insert(key) {
                        let mut buf = Vec::new();
                        if std::io::Read::read_to_end(&mut entry, &mut buf).is_ok() {
                            // 不过滤：无背景配置可能携带建筑/生物数据（合并逻辑统一处理）
                            configs.push(parse_town_config(&buf, &name));
                        }
                    }
                }
            }
        }
    }
    // 合并：同一 faction_id 可能拆在多个文件（VCMI 1.x 常见 structures.json/faction.json/buildings.json），
    // 旧逻辑"先到先得"会丢掉另一半字段（如 buildings 只在 buildings.json、背景只在 faction.json）。
    // 改为字段级合并：非空优先，列表取并集。
    let mut merged: Vec<TownConfig> = Vec::new();
    let mut by_id: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for cfg in configs {
        match by_id.get(&cfg.faction_id).copied() {
            Some(idx) => {
                let dst = &mut merged[idx];
                if dst.town_background.is_none() {
                    dst.town_background = cfg.town_background;
                }
                if dst.creature_background.is_none() {
                    dst.creature_background = cfg.creature_background;
                }
                if dst.buildings_icons.is_none() {
                    dst.buildings_icons = cfg.buildings_icons;
                }
                // 城镇名：同一城镇拆多文件时 name 常只在 `faction.json`，而 `buildings.json`
                // 等可能先被解析成 merged[0]（无 name）。此前漏合并该字段 → 中文名被丢弃，
                // 回退成小写 factionId（实测 old-/new-chinese-mods 28 个城镇：
                // glade→禁猎地、xanadu→灵墟、shuHan→蜀汉 等全部只剩英文 id）。
                if dst.town_name.is_none() {
                    dst.town_name = cfg.town_name;
                }
                if dst.town_desc.is_none() {
                    dst.town_desc = cfg.town_desc;
                }
                // 阵营主题曲：同样可能只写在 `faction.json`（实测 hota.cove / hota.bulwark /
                // 沙堡都是多文件城镇），不并入就会被先解析到的 `buildings.json` 顶成 None。
                if dst.music_theme.is_none() {
                    dst.music_theme = cfg.music_theme;
                }
                // 结构位置与建筑原始对象：字段级并入（首次命中优先，保证多文件拆分时不全丢）
                if dst.structures.is_empty() {
                    dst.structures = cfg.structures;
                } else {
                    for (k, v) in &cfg.structures {
                        dst.structures.entry(k.clone()).or_insert_with(|| v.clone());
                    }
                }
                if dst.buildings_raw.is_empty() {
                    dst.buildings_raw = cfg.buildings_raw;
                } else {
                    for (k, v) in &cfg.buildings_raw {
                        dst.buildings_raw
                            .entry(k.clone())
                            .or_insert_with(|| v.clone());
                    }
                }
                if dst.alignment.is_empty() {
                    dst.alignment = cfg.alignment;
                }
                if dst.native_terrain.is_empty() {
                    dst.native_terrain = cfg.native_terrain;
                }
                for id in cfg.building_ids {
                    if !dst.building_ids.contains(&id) {
                        dst.building_ids.push(id);
                    }
                }
                for id in cfg.creature_ids {
                    if !dst.creature_ids.contains(&id) {
                        dst.creature_ids.push(id);
                    }
                }
                if dst.buildings_detail.is_empty() {
                    dst.buildings_detail = cfg.buildings_detail;
                }
                // 城镇界面建筑动画（`structures.json` 常独立成文件）同样按字段级并入，
                // 否则先解析到的无动画文件会让整座城镇丢失建筑动画引用。
                if dst.building_animations.is_empty() {
                    dst.building_animations = cfg.building_animations;
                } else {
                    for (k, v) in &cfg.building_animations {
                        dst.building_animations
                            .entry(k.clone())
                            .or_insert_with(|| v.clone());
                    }
                }
            }
            None => {
                by_id.insert(cfg.faction_id.clone(), merged.len());
                merged.push(cfg);
            }
        }
    }
    // 防御：空 faction_id = 顶层不是 `{ "<factionId>": {...} }` 包裹的非城镇文件
    // （如漏网的精灵/图形定义、扁平结构），直接丢弃，避免产出无名幽灵条目。
    // 注意：`<mod>:<id>` 跨 mod 引用补丁**有意保留**（前端标记 `ref:true` 展示，见 town_refs 用例），
    // 不要在此过滤——避免误采应靠采集端（如 `dwellings.json` 不纳入，见 is_town_config_path_in）。
    merged.retain(|c| !c.faction_id.is_empty());
    merged
}

/// 判断 zip 条目 / 磁盘路径是否为城镇 faction 配置文件。
///
/// VCMI 的城镇配置有三种形态：
///   - 旧式：`config/<faction>/town.json`（如 kremlin / abyss）；
///   - 新式：`config/<faction>/town/faction.json` + 同目录子文件
///     （`buildings.json` / `structures.json` / `puzzle.json` / `siege.json` / `spells.json`）；
///   - 变体：不遵循 `/town/` 目录约定，如 家园 `config/Abode/faction.json`、
///     沙堡 `townPavilion/faction.json`、发条城 `config/celestial/town/building.json`（单数）。
///
/// ⚠️ 不能按"路径含 `/town/` + 文件名以 town 开头"判定：图形定义目录
/// `sprites/<mod>/town/townhall.json`（内容 `{basepath, images}`）会被误判为城镇配置，
/// 产出 factionId 为空的幽灵条目（实测 hota.bulwark / hota.cove / new-chinese-mods.shuHan）。
///
/// 同时不能漏：`building.json`（单数）/ `structures.json` / `puzzle.json` / `siege.json` /
/// `spells.json` 是建筑与结构数据所在文件，漏读会导致城镇建筑列表为空
/// （实测发条城 / Pavilion 等）。
pub(crate) fn is_town_config_path(name: &str) -> bool {
    /// 城镇目录下允许的文件名（去掉 .json 后缀、小写）。
    ///
    /// 含单数 `structure`（无 s）：老中文包 `Celestial-master`（发条城）把结构写进
    /// `config/celestial/town/structure.json`（**单数**），此前漏入导致 `structures` 为空、
    /// 幕布图只剩裸底图。与 `structures`、`building`/`buildings`、`spell`/`spells` 等
    /// 单复数变体一并纳入，避免这类老 mod 漏读。
    const TOWN_SUB_STEMS: &[&str] = &[
        "building",
        "buildings",
        "structure",
        "structures",
        "puzzle",
        "siege",
        "spell",
        "spells",
    ];

    let n = name.replace('\\', "/");
    let lower = n.to_lowercase();
    if !lower.ends_with(".json") {
        return false;
    }
    let segs: Vec<&str> = lower.split('/').filter(|s| !s.is_empty()).collect();
    let Some(base) = segs.last().copied() else {
        return false;
    };
    let stem = base.trim_end_matches(".json");
    // 图形 / 精灵定义目录：其中也有 `<x>/town/townhall.json` 这类同名结构，但不是城镇配置。
    if segs.iter().any(|s| {
        matches!(
            *s,
            "sprites" | "sprite" | "townbuilding" | "sequences" | "images"
        )
    }) {
        return false;
    }
    // 阵营定义文件（不论所在目录层级）：config/<faction>/faction.json、townPavilion/faction.json。
    if matches!(stem, "faction" | "factions" | "town" | "towns") {
        return true;
    }
    // 城镇子文件：所在目录须是城镇目录（段以 town 开头，如 `town/`、`townPavilion/`；
    // 或段含 faction）。
    let in_town_dir = segs[..segs.len() - 1]
        .iter()
        .any(|s| s.starts_with("town") || s.contains("faction"));
    if !in_town_dir {
        return false;
    }
    TOWN_SUB_STEMS.contains(&stem)
}

/// 目录文件索引：目录路径（小写、`/` 分隔）→ 该目录下条目名（小写、去 `.json`；含子目录名）。
///
/// 用于**目录上下文**判定：`config/<Faction>/buildings.json` 这类不带 `town/` 目录层的
/// 变体写法（实测家园 `config/Abode/`、`config/Abode-creatures/`），单看路径字符串无法与
/// 普通数据目录区分，须看"同目录是否有 `faction.json` 或 `town/` 子目录"。
pub(crate) type DirFileIndex = std::collections::HashMap<String, std::collections::HashSet<String>>;

/// 把一个条目名（文件或目录，磁盘相对/绝对路径或 zip 条目）挂进目录索引。
///
/// - `config/Abode/buildings.json` → dir=`config/abode`，条目 `buildings`
/// - `sprites/hota/cove/town/townhall/`（目录条目）→ dir=`sprites/hota/cove/town`，条目 `townhall`
pub(crate) fn attach_index_entry(idx: &mut DirFileIndex, raw: &str) {
    let n = raw.replace('\\', "/").to_lowercase();
    let trimmed = n.trim_end_matches('/');
    let segs: Vec<&str> = trimmed.split('/').filter(|s| !s.is_empty()).collect();
    let Some(last) = segs.last() else { return };
    let dir = segs[..segs.len() - 1].join("/");
    let stem = last.trim_end_matches(".json");
    idx.entry(dir).or_default().insert(stem.to_string());
}

/// 建立磁盘目录索引（递归；不可读目录跳过）。
pub(crate) fn collect_dir_index(dir: &Path, out: &mut DirFileIndex) {
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if let Some(s) = p.to_str() {
                attach_index_entry(out, s);
            }
            if p.is_dir() {
                collect_dir_index(&p, out);
            }
        }
    }
}

/// 目录上下文版：[`is_town_config_path`] 之外，接受**变体城镇目录**——
/// 子文件（`building(s)` / `structures` / `dwellings` / `puzzle` / `siege` / `spells`）
/// 与 `faction.json` 或 `town/` 子目录**同级**时视为城镇配置。
///
/// 实测：家园 `config/Abode/{faction,buildings,structures,dwellings,puzzle,siege,spells}.json`
/// 无 `town/` 目录层（单文件规则只救了 `faction.json`）→ 建筑/结构整批漏读、建筑列表为空。
/// `sprites|townbuilding|sequences|images` 目录排除规则保持不变（防幽灵条目回归）。
pub(crate) fn is_town_config_path_in(name: &str, idx: &DirFileIndex) -> bool {
    if is_town_config_path(name) {
        return true;
    }
    // 注：`dwellings.json` **不纳入**——它常是"给别家生物生成器打补丁"的跨 mod 引用文件
    // （实测 xanadu 顶层为 `core:creatureGeneratorCommon`），采入会产出 `core:*` 伪 faction；
    // 城镇招募建筑数据由 `buildings.json` 承载（家园验证：buildings.json 已含全部建筑）。
    const VARIANT_STEMS: &[&str] = &[
        "building",
        "buildings",
        "structure",
        "structures",
        "puzzle",
        "siege",
        "spell",
        "spells",
    ];
    let n = name.replace('\\', "/");
    let lower = n.to_lowercase();
    if !lower.ends_with(".json") {
        return false;
    }
    let segs: Vec<&str> = lower.split('/').filter(|s| !s.is_empty()).collect();
    let Some(base) = segs.last().copied() else {
        return false;
    };
    let stem = base.trim_end_matches(".json");
    if !VARIANT_STEMS.contains(&stem) {
        return false;
    }
    if segs.iter().any(|s| {
        matches!(
            *s,
            "sprites" | "sprite" | "townbuilding" | "sequences" | "images"
        )
    }) {
        return false;
    }
    // 目录上下文：同目录存在 faction.json / town 子目录
    let dir = segs[..segs.len() - 1].join("/");
    idx.get(&dir)
        .map(|set| {
            set.iter().any(|s| {
                matches!(s.as_str(), "faction" | "factions" | "town" | "towns")
                    || s.starts_with("town")
            })
        })
        .unwrap_or(false)
}

/// 递归扫描 Content 目录，收集城镇 faction 配置文件（含变体目录，见 [`is_town_config_path_in`]）。
pub(crate) fn collect_town_jsons_dir(
    dir: &Path,
    idx: &DirFileIndex,
    out: &mut Vec<TownConfig>,
    seen: &mut std::collections::HashSet<String>,
) {
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                collect_town_jsons_dir(&p, idx, out, seen);
            } else if p.is_file() {
                let rel = p.to_string_lossy().replace('\\', "/");
                if is_town_config_path_in(&rel, idx) {
                    let key = rel.to_lowercase();
                    if seen.insert(key) {
                        // 不过滤：无背景的配置（如 buildings.json）可能携带建筑/生物数据，
                        // 由 read_town_configs 末尾的字段级合并统一处理
                        out.push(parse_town_config_file(&p));
                    }
                }
            }
        }
    }
}

/// 读取游戏根 `config/factions/*.json`：原版 9 城镇（castle/rampart/tower/inferno/necropolis/
/// dungeon/stronghold/fortress/conflux）的权威配置。它们不在任何 mod 的 town.json 中，
/// 而是位于游戏根 `config/factions/<id>.json`，且携带完整图片字段（townBackground /
/// creatureBackground / buildingsIcons 等）。`build_town_config` 已能解析该结构。
/// 仅在构建 core 伪 mod 时并入（见 build_mod_wiki_inner），避免跨 mod 重复注入原版城镇。
/// 跳过 neutral / random（非可玩城镇）。
pub(crate) fn read_game_root_factions(game_dir: &Path) -> Vec<TownConfig> {
    let mut configs = Vec::new();
    let dir = game_dir.join("config").join("factions");
    if !dir.is_dir() {
        return configs;
    }
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file()
                && path
                    .extension()
                    .map(|s| s.eq_ignore_ascii_case("json"))
                    .unwrap_or(false)
            {
                let cfg = parse_town_config_file(&path);
                if cfg.faction_id.is_empty()
                    || cfg.faction_id == "neutral"
                    || cfg.faction_id == "random"
                {
                    continue;
                }
                configs.push(cfg);
            }
        }
    }
    configs
}

/// 读取游戏根 `config/<sub>/*.json`（原版实体：creatures / heroes / ...）。
/// 解析口径与 `collect_config_jsons` 一致（顶层 `{ "<id>": {...} }` 或文件名作 id）。
/// 仅在构建 core 伪 mod 时并入，避免跨 mod 重复注入原版实体。
///
/// `core_faction`（来自 `core.<faction>` 的 mod id）决定作用域，规则镜像 VCMI 磁盘布局：
/// - `Some(f)`（子模组）：只取 `<f>.json` —— 原版生物/英雄本就按阵营分文件；
/// - `None`（父 core）：取**不属于任何可玩阵营**的文件（neutral / special 等无归属实体）。
pub(crate) fn read_game_root_configs(
    game_dir: &Path,
    sub: &str,
    core_faction: Option<&str>,
) -> Vec<(String, Value)> {
    let mut out = Vec::new();
    let dir = game_dir.join("config").join(sub);
    if !dir.is_dir() {
        return out;
    }
    let mut files = Vec::new();
    collect_json_files(&dir, sub, &mut files);
    for f in files {
        let stem = f
            .file_stem()
            .map(|s| s.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        // `portraits.json` / `portraitsChronicles.json` 是**头像定义**（texts.name 为空、
        // special=true），并非英雄实体；不排除会把 portraitXxx 之类伪英雄混进父 core。
        // 注意 `special.json` 是真实战役英雄（sirMullich 等），须保留。
        if sub == "heroes" && stem.starts_with("portrait") {
            continue;
        }
        match core_faction {
            Some(fac) => {
                if stem != fac {
                    continue;
                }
            }
            None => {
                if CORE_FACTIONS.contains(&stem.as_str()) {
                    continue;
                }
            }
        }
        if let Ok(text) = fs::read_to_string(&f) {
            let v = parse_vcmi_json(&text);
            push_config_object(&mut out, &f.to_string_lossy(), v, sub);
        }
    }
    out
}

pub(crate) fn parse_town_config_file(path: &Path) -> TownConfig {
    let v = read_vcmi_json(path);
    build_town_config(&v)
}

pub(crate) fn parse_town_config(bytes: &[u8], _name: &str) -> TownConfig {
    let v = read_vcmi_json_bytes(bytes);
    build_town_config(&v)
}

/// 取 `AudioPath` 数组的首个非空项（兼容裸字符串写法）。
///
/// VCMI 里 `musicTheme` / `ambientSounds` 之类字段在 schema 上是 `string[]`，
/// 但第三方模组常写成裸字符串 —— 只按 `as_str()` 取会永远落空。
fn first_audio_path(v: &Value) -> Option<String> {
    match v {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().to_string()),
        Value::Array(arr) => arr.iter().find_map(|x| {
            x.as_str()
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
        }),
        _ => None,
    }
}

pub(crate) fn build_town_config(v: &Value) -> TownConfig {
    // VCMI 1.x 的 config/<faction>/town.json 结构：
    // { "<factionId>": { "town": { "townBackground": ..., "creatureBackground": ... }, "guildBackground": ..., ... } }
    // 也可能更扁平（creatureBackground 在 faction 顶层）。
    let (faction_id, inner) = match v.as_object() {
        Some(obj) if obj.len() == 1 => {
            let (k, val) = obj.iter().next().unwrap();
            (k.clone(), val)
        }
        _ => ("".to_string(), v),
    };
    // town 相关字段优先在 inner.town 子对象，其次回退 inner 顶层
    let town_sub = inner.get("town");
    let town_background = town_sub
        .and_then(|t| t.get("townBackground"))
        .or_else(|| inner.get("townBackground"))
        // VCMI 1.x 标准结构：clientInfo.townScreen.background
        .or_else(|| {
            inner
                .get("clientInfo")
                .and_then(|c| c.get("townScreen"))
                .and_then(|t| t.get("background"))
        })
        .and_then(value_to_resource);
    // 城镇界面建筑图标 DEF（VCMI: clientInfo.buildingsIcons）
    let buildings_icons = inner
        .get("clientInfo")
        .and_then(|c| c.get("buildingsIcons"))
        .or_else(|| town_sub.and_then(|t| t.get("buildingsIcons")))
        .and_then(value_to_resource);
    let creature_background = inner
        .get("creatureBackground")
        .or_else(|| town_sub.and_then(|t| t.get("creatureBackground")))
        .and_then(|x| resolve_creature_background(x));
    // 关系链：town.json 的 `creatures` 数组（每级一组），展平收集该城镇可招募的生物 id
    let mut creature_ids = Vec::new();
    let creatures_val = inner
        .get("creatures")
        .or_else(|| town_sub.and_then(|t| t.get("creatures")));
    if let Some(Value::Array(levels)) = creatures_val {
        for lvl in levels {
            if let Value::Array(items) = lvl {
                for it in items {
                    if let Some(s) = it.as_str() {
                        creature_ids.push(s.to_string());
                    }
                }
            } else if let Some(s) = lvl.as_str() {
                creature_ids.push(s.to_string());
            }
        }
    }
    // 关系链：town.json / faction.json 的 `buildings` 对象（keys 为建筑 id）
    let mut building_ids = Vec::new();
    let mut buildings_raw: HashMap<String, Value> = HashMap::new();
    let bld_val = inner
        .get("buildings")
        .or_else(|| town_sub.and_then(|t| t.get("buildings")));
    if let Some(Value::Object(map)) = bld_val {
        for (k, v) in map {
            if !building_ids.iter().any(|x: &String| x == k) {
                building_ids.push(k.clone());
            }
            buildings_raw.insert(k.clone(), v.clone());
        }
    }
    // 阵营善恶 / 本族地形（顶层或 town 子对象）
    let alignment = inner
        .get("alignment")
        .or_else(|| town_sub.and_then(|t| t.get("alignment")))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let native_terrain = inner
        .get("nativeTerrain")
        .or_else(|| town_sub.and_then(|t| t.get("nativeTerrain")))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    // 阵营主题曲（town.musicTheme）：VCMI schema 是 string[]（实测 `["music/CstleTown"]`），
    // 个别模组写成裸字符串或已带扩展名（实测 hota.bulwark 的 `music/factions/BulwarkTown.mp3`）。
    let music_theme = town_sub
        .and_then(|t| t.get("musicTheme"))
        .or_else(|| inner.get("musicTheme"))
        .and_then(first_audio_path)
        .map(|s| s.to_string());
    // 建筑详情：从 buildings 对象取 id + cost + 招募生物 + 图标路径（后续再 extract）
    let mut buildings_detail = Vec::new();
    if let Some(Value::Object(map)) = bld_val {
        for (order_index, (k, v)) in map.iter().enumerate() {
            let k = k.as_str();
            let bid = resolve_building_bid(v, order_index);
            let mut cost = v.get("cost").cloned().unwrap_or(json!({}));
            // 原版城镇配置通常没有 cost；从 h3-core 的 BUILDING.TXT 提取回退
            if cost.as_object().map(|o| o.is_empty()).unwrap_or(true) {
                if let Some(c) = h3_building_cost(&faction_id, k) {
                    cost = c;
                }
            }
            // 建筑图标原始路径（字符串形式，如 "path/x.def"）
            let bimg = v
                .get("image")
                .or_else(|| v.get("graphics").and_then(|g| g.get("image")))
                .or_else(|| v.get("icon"))
                .and_then(|x| x.as_str())
                .map(|s| s.to_string());
            // 建筑图标（DEF 动画形式，如 {animation:".../x.def", group, index}）——城镇界面小图通常来自 town-screen 的 DEF 帧
            let banim = v
                .get("animation")
                .or_else(|| v.get("graphics").and_then(|g| g.get("animation")))
                .and_then(|x| x.as_str())
                .map(|s| s.to_string());
            let bgroup = v
                .get("group")
                .or_else(|| v.get("graphics").and_then(|g| g.get("group")))
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            let bindex = v
                .get("index")
                .or_else(|| v.get("graphics").and_then(|g| g.get("index")))
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            // building 招募的生物：creatures 数组 或 creature 单值
            let mut bc = Vec::new();
            if let Some(arr) = v.get("creatures").and_then(|x| x.as_array()) {
                for it in arr {
                    if let Some(s) = it.as_str() {
                        bc.push(s.to_string());
                    }
                }
            } else if let Some(s) = v.get("creature").and_then(|x| x.as_str()) {
                bc.push(s.to_string());
            }
            buildings_detail.push(json!({
                "id": k,
                // VCMI: bid = 建筑数字 id（CTownHandler::loadBuilding）；
                // 取 `id` 字段 → buildingsLibrary 标准表 → 配置顺序（保底）
                "bid": bid,
                "mode": v.get("mode").and_then(|x| x.as_str()).unwrap_or("normal"),
                // 翻译缺失时的英文原文（VCMI 翻译基准语言为英文）
                "rawName": v.get("name").and_then(|x| x.as_str()).unwrap_or(""),
                "rawDesc": v.get("description").and_then(|x| x.as_str()).unwrap_or(""),
                // 城镇界面结构动画（town.structures[id].animation）首帧 = 建筑表大图标（对齐 VCMI 效果图）
                "structureAnim": town_sub
                    .and_then(|t| t.get("structures"))
                    .and_then(|s| s.get(&k))
                    .and_then(|sv| sv.get("animation"))
                    .and_then(value_to_resource),
                "cost": cost,
                "creatures": bc,
                // 被本建筑升级的基础建筑 id（VCMI: `upgrades`）。
                // 客户端把「升级建筑」与「被升级建筑」视为**同一底座**绘制
                // （CBuilding::getBase() 沿 upgrade 链走到链尾；CCastleBuildings::
                // recreate 用 groups[building->getBase()] 分组），故建筑图标在
                // 本建筑四通道全失败时应回退到 base 的（见 data/town.rs）。
                "upgrades": v
                    .get("upgrades")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string(),
                "image": bimg,
                "animation": banim,
                "group": bgroup,
                "index": bindex,
            }));
        }
    }
    // 城镇内嵌名称（优先 inner.name，其次 town_sub.name；可能是字符串或多语言对象）。
    // 这是**阵营名的主来源**（VCMI 同样取 faction 配置的 name，而非 mod.json 的模组名），
    // 原样保留 JSON 形态，由 `name_obj_with_inline` 决定放进哪个语言键。
    let town_name: Option<Value> = inner
        .get("name")
        .or_else(|| town_sub.and_then(|t| t.get("name")))
        .filter(|v| match v {
            Value::String(s) => !s.trim().is_empty(),
            Value::Object(m) => !m.is_empty(),
            _ => false,
        })
        .cloned();

    // 城镇自带描述（`inner.description` / `town_sub.description`；字符串或多语言对象）。
    // 与 town_name 同理：部分模组没有 translation 目录，整段设定直接写在 town.json 里。
    let town_desc: Option<Value> = inner
        .get("description")
        .or_else(|| town_sub.and_then(|t| t.get("description")))
        .filter(|v| match v {
            Value::String(s) => !s.trim().is_empty(),
            Value::Object(m) => !m.is_empty(),
            _ => false,
        })
        .cloned();

    // 城镇界面建筑动画（town.structures：<bid> → { animation: "<file>.def", x, y }）。
    // 仅取资源名文本，不做任何解码——原版 town-screen 有 2452 个 .def，
    // 全量预解会显著拖慢构建；改为详情页打开时按需加载（设计 §11 / §13.1）。
    let mut building_animations: HashMap<String, String> = HashMap::new();
    // 建筑屏幕位置（完整结构对象，供叠加生成带建筑的背景图）。
    // 来源：town.structures 或 clientInfo.townScreen.structures（mod 的现代格式）。
    let mut structures: HashMap<String, Value> = HashMap::new();
    let mut add_structures = |obj: &serde_json::Map<String, Value>| {
        for (k, v) in obj {
            if v.get("animation")
                .and_then(|a| a.as_str())
                .map(|s| !s.is_empty())
                .unwrap_or(false)
            {
                building_animations.insert(
                    k.clone(),
                    v.get("animation")
                        .and_then(|a| a.as_str())
                        .unwrap()
                        .to_string(),
                );
                structures.entry(k.clone()).or_insert_with(|| v.clone());
            }
        }
    };
    if let Some(obj) = town_sub
        .and_then(|t| t.get("structures"))
        .and_then(|s| s.as_object())
    {
        add_structures(obj);
    }
    if let Some(obj) = inner
        .get("clientInfo")
        .and_then(|c| c.get("townScreen"))
        .and_then(|t| t.get("structures"))
        .and_then(|s| s.as_object())
    {
        add_structures(obj);
    }

    TownConfig {
        faction_id,
        town_background,
        creature_background,
        creature_ids,
        building_ids,
        alignment,
        native_terrain,
        town_name,
        town_desc,
        buildings_detail,
        buildings_icons,
        building_animations,
        structures,
        buildings_raw,
        music_theme,
    }
}

/// 将 JSON 值转为资源路径字符串（支持裸字符串或 {file/name} 对象）。
pub(crate) fn value_to_resource(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Object(o) => o
            .get("file")
            .or_else(|| o.get("name"))
            .and_then(|x| x.as_str())
            .map(|s| s.to_string()),
        _ => None,
    }
}

/// creatureBackground 可能是分辨率字典（如 {"120px": "...", "130px": "..."}）。
/// 优先取 130px，否则取首个值。
pub(crate) fn resolve_creature_background(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Object(o) => {
            if let Some(s) = o.get("130px").and_then(|x| x.as_str()) {
                return Some(s.to_string());
            }
            if let Some(s) = o.get("120px").and_then(|x| x.as_str()) {
                return Some(s.to_string());
            }
            for (_, val) in o {
                if let Some(s) = val.as_str() {
                    return Some(s.to_string());
                }
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 发条城（老中文包 `Celestial-master`）把结构写进 `config/celestial/town/structure.json`
    /// （**单数** `structure`），漏入会让 `structures` 为空、幕布图只剩裸底图（2026-10-14 回归）。
    #[test]
    fn singular_structure_json_is_town_config() {
        assert!(
            is_town_config_path("config/celestial/town/structure.json"),
            "单数 structure.json 必须被识别为城镇配置"
        );
        assert!(
            is_town_config_path("config/celestial/town/structures.json"),
            "复数 structures.json 仍应识别"
        );
        // 非城镇目录下的 structure.json 不应误识别（防幽灵条目）
        assert!(!is_town_config_path("config/other/structure.json"));
        // sprites 目录下的同名结构定义不应误识别
        assert!(!is_town_config_path(
            "sprites/celestial/town/structure.json"
        ));
    }
}
