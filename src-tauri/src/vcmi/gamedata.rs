//! 游戏根配置读取（与具体 mod 无关）：技能索引、法术表、地形条目、初始部队概率。
//!
//! 数据源：`<游戏目录>/config/*.json`（VCMI 官方配置）。
//!
//! 依赖方向：只依赖 `infra`（见 `docs/architecture/backend.zh-CN.md` §4.1）。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::Value;

use crate::infra::cache::{cfg_cache_get, fingerprint_of};
use crate::infra::json::parse_vcmi_json;

/// 读取次级技能标识 → 数字 index（`config/skills.json`，与 SSTRAITS.TXT 行序对应）。
pub(crate) fn read_skill_index_map(game_dir: &Path) -> HashMap<String, i64> {
    let mut map = HashMap::new();
    for path in [
        game_dir.join("config").join("skills.json"),
        game_dir
            .join("Mods")
            .join("vcmi")
            .join("Content")
            .join("config")
            .join("skills.json"),
    ] {
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        if let Value::Object(m) = parse_vcmi_json(&text) {
            for (id, obj) in m {
                if let Some(ix) = obj.get("index").and_then(|v| v.as_i64()) {
                    map.entry(id).or_insert(ix);
                }
            }
        }
    }
    map
}

/// VCMI 原版法术 identifier → 数字 index（法术书图标 SPELLBON 帧号 = index）。
/// 来源：lib/constants/EntityIdentifiers.h 的 SpellID 枚举（枚举名转驼峰即 identifier），
/// 生成脚本 scripts/gen-spell-ids.mjs。
pub(crate) const SPELL_ID_TABLE: &[(&str, i64)] = &[
    ("scuttleBoat", 1),
    ("visions", 2),
    ("viewEarth", 3),
    ("disguise", 4),
    ("viewAir", 5),
    ("fly", 6),
    ("waterWalk", 7),
    ("dimensionDoor", 8),
    ("townPortal", 9),
    ("quicksand", 10),
    ("landMine", 11),
    ("forceField", 12),
    ("fireWall", 13),
    ("earthquake", 14),
    ("magicArrow", 15),
    ("iceBolt", 16),
    ("lightningBolt", 17),
    ("implosion", 18),
    ("chainLightning", 19),
    ("frostRing", 20),
    ("fireball", 21),
    ("inferno", 22),
    ("meteorShower", 23),
    ("deathRipple", 24),
    ("destroyUndead", 25),
    ("armageddon", 26),
    ("shield", 27),
    ("airShield", 28),
    ("fireShield", 29),
    ("protectionFromAir", 30),
    ("protectionFromFire", 31),
    ("protectionFromWater", 32),
    ("protectionFromEarth", 33),
    ("antiMagic", 34),
    ("dispel", 35),
    ("magicMirror", 36),
    ("cure", 37),
    ("resurrection", 38),
    ("animateDead", 39),
    ("sacrifice", 40),
    ("bless", 41),
    ("curse", 42),
    ("bloodlust", 43),
    ("precision", 44),
    ("weakness", 45),
    ("stoneSkin", 46),
    ("disruptingRay", 47),
    ("prayer", 48),
    ("mirth", 49),
    ("sorrow", 50),
    ("fortune", 51),
    ("misfortune", 52),
    ("haste", 53),
    ("slow", 54),
    ("slayer", 55),
    ("frenzy", 56),
    ("titansLightningBolt", 57),
    ("counterstrike", 58),
    ("airElemental", 59),
    ("earthElemental", 60),
    ("fireElemental", 61),
    ("waterElemental", 62),
    ("stoneSkin", 63),
    ("magicMirror", 64),
    ("haste", 65),
    ("slow", 66),
    ("shield", 67),
    ("bloodlust", 68),
    ("precision", 69),
    ("disruptingRay", 70),
    ("prayer", 71),
    ("mirth", 72),
    ("sorrow", 73),
    ("fortune", 74),
    ("misfortune", 75),
    ("deathRipple", 76),
    ("destroyUndead", 77),
    ("resurrection", 78),
    ("deathStare", 79),
    ("acidBreathDefense", 80),
    ("acidBreathDamage", 81),
];

pub(crate) static STACK_CHANCES_CACHE: std::sync::OnceLock<Mutex<Vec<(String, std::sync::Arc<Vec<i64>>)>>> =
    std::sync::OnceLock::new();

/// 初始部队出现概率。
pub(crate) fn cached_starting_stack_chances(game_dir: &Path) -> std::sync::Arc<Vec<i64>> {
    let cands = starting_stack_chances_candidates(game_dir);
    let key = format!("chance:{}", fingerprint_of(&cands));
    cfg_cache_get(&STACK_CHANCES_CACHE, key, || read_starting_stack_chances(game_dir))
}

pub(crate) fn starting_stack_chances_candidates(game_dir: &Path) -> Vec<PathBuf> {
    vec![
        game_dir.join("config").join("gameConfig.json"),
        game_dir
            .join("Mods")
            .join("vcmi")
            .join("Content")
            .join("config")
            .join("gameConfig.json"),
    ]
}

// 英雄职业 identifier → 数字 index（`config/heroClasses.json`）。
// 用于把 hero.json 的 `class` 字符串映射到 HCTRAITS.TXT 行序，从而命中 h3-core 职业中文名。
pub(crate) static HERO_CLASS_INDEX_CACHE: std::sync::OnceLock<
    Mutex<Vec<(String, std::sync::Arc<HashMap<String, i64>>)>>,
> = std::sync::OnceLock::new();

pub(crate) fn cached_hero_class_index_map(game_dir: &Path) -> std::sync::Arc<HashMap<String, i64>> {
    let cands = hero_class_index_candidates(game_dir);
    let key = format!("hci:{}", fingerprint_of(&cands));
    cfg_cache_get(&HERO_CLASS_INDEX_CACHE, key, || read_hero_class_index_map(game_dir))
}

pub(crate) fn hero_class_index_candidates(game_dir: &Path) -> Vec<PathBuf> {
    vec![
        game_dir.join("config").join("heroClasses.json"),
        game_dir
            .join("Mods")
            .join("vcmi")
            .join("Content")
            .join("config")
            .join("heroClasses.json"),
    ]
}

pub(crate) fn read_hero_class_index_map(game_dir: &Path) -> HashMap<String, i64> {
    let mut map = HashMap::new();
    for path in hero_class_index_candidates(game_dir) {
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        if let Value::Object(m) = parse_vcmi_json(&text) {
            for (id, obj) in m {
                if let Some(ix) = obj.get("index").and_then(|v| v.as_i64()) {
                    map.entry(id).or_insert(ix);
                }
            }
        }
        if !map.is_empty() {
            break;
        }
    }
    map
}

pub(crate) fn read_starting_stack_chances(game_dir: &Path) -> Vec<i64> {
    let default = vec![100i64, 88, 25];
    for path in starting_stack_chances_candidates(game_dir) {
        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let v = parse_vcmi_json(&text);
        let chances: Vec<i64> = v
            .get("heroes")
            .and_then(|h| h.get("startingStackChances"))
            .and_then(|c| c.as_array())
            .map(|arr| arr.iter().filter_map(|x| x.as_i64()).collect())
            .unwrap_or_default();
        if !chances.is_empty() {
            return chances;
        }
    }
    default
}

/// 把单个配置文件的解析结果按 `{ "<id>": {...} }` 拆入 out；无包裹时用文件名作 id。
pub(crate) fn push_config_object(out: &mut Vec<(String, Value)>, path: &str, v: Value, _type: &str) {
    let fname = path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .trim_end_matches(".json")
        .to_string();
    match v {
        Value::Object(map) => {
            // 单 key 包裹：{ "<id>": {...} }
            if map.len() == 1 {
                if let Some((k, obj)) = map.iter().next() {
                    if obj.is_object() {
                        out.push((k.clone(), obj.clone()));
                        return;
                    }
                    out.push((fname, obj.clone()));
                    return;
                }
            }
            // 多 key 且每个值都是对象：该文件承载多个实体，**每个 key 即一个实体 id**。
            // 典型：`config/creatures/castle.json`、`config/heroes/castle.json`、
            // mod 的 `config/spells/*.json`。此前会被整体折叠成「以文件名为 id」的单条记录，
            // 导致原版生物/英雄/法术实体错乱（id 变成 castle / heroes 这类文件名）。
            if !map.is_empty() && map.values().all(|x| x.is_object()) {
                for (k, obj) in map {
                    out.push((k, obj));
                }
                return;
            }
            // 无包裹或字段平铺：整个对象体，用文件名作 id
            out.push((fname, Value::Object(map)));
        }
        other => {
            out.push((fname, other));
        }
    }
}
