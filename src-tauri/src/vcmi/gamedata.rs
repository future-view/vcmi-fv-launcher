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
/// 来源：lib/constants/EntityIdentifiers.h 的 SpellID 枚举（枚举名转驼峰即 identifier）。
///
/// ⚠️ 2026-10-16 修正表尾：此前 59 及之后的段来自**别的口径**（`airElemental=59` …
/// `stoneSkin=63`、`haste=65`，且 `haste/slow/shield/bloodlust/…` 与 53+ 段重复），
/// 与 1.8 枚举完全对不上，直接后果是 **`clone` / `blind` / `teleport` / `berserk` /
/// `hypnotize` / `forgetfulness` / `removeObstacle` / 四系召唤元素 / 生物能力伪法术
/// 全部查不到 index** —— 英雄页「魔法」里这些法术既没有图标（index=-1 → 无 SPELLBON 帧）
/// 也拿不到名字（实测该发行版 1,749 条英雄法术里有 147 条因此空白）。
/// 索引唯一性由本文件末尾的单测锁住，防止再次出现重复/错位的段。
pub(crate) const SPELL_ID_TABLE: &[(&str, i64)] = &[
    ("summonBoat", 0),
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
    // 4~5 级控制系与召唤系（59-69）：此前整段缺失
    ("berserk", 59),
    ("hypnotize", 60),
    ("forgetfulness", 61),
    ("blind", 62),
    ("teleport", 63),
    ("removeObstacle", 64),
    ("clone", 65),
    ("summonFireElemental", 66),
    ("summonEarthElemental", 67),
    ("summonWaterElemental", 68),
    ("summonAirElemental", 69),
    // 70 起是**生物能力伪法术**（枚举 `FIRST_NON_SPELL = 70`）：它们在 SPELLBON 里没有帧，
    // 列出只为让 index 语义与枚举一致（`firstNonSpell` 自身不是法术，故不入表）。
    ("stoneGaze", 70),
    ("poison", 71),
    ("bind", 72),
    ("disease", 73),
    ("paralyze", 74),
    ("age", 75),
    ("deathCloud", 76),
    ("thunderbolt", 77),
    ("dispelHelpfulSpells", 78),
    ("deathStare", 79),
    ("acidBreathDefense", 80),
    ("acidBreathDamage", 81),
];

#[cfg(test)]
mod tests {
    use super::SPELL_ID_TABLE;

    /// 表必须与 `lib/constants/EntityIdentifiers.h` 的 SpellID 枚举一致，且 **index 唯一**。
    ///
    /// 回归背景（2026-10-16）：表尾 59+ 段曾来自别的口径（`airElemental=59`…`haste=65`），
    /// 与枚举冲突且与 53+ 段重复 —— 英雄页 `clone`/`blind`/`teleport` 等法术因此拿不到
    /// SPELLBON 帧号（无图标）与名字，实测 1,749 条英雄法术里 147 条空白。
    #[test]
    fn spell_id_table_matches_vcmi_enum_and_is_unique() {
        let idx = |id: &str| SPELL_ID_TABLE.iter().find(|(k, _)| *k == id).map(|(_, i)| *i);
        // 抽样覆盖：冒险法术 / 表尾曾缺失段 / 生物能力伪法术
        assert_eq!(idx("summonBoat"), Some(0));
        assert_eq!(idx("townPortal"), Some(9));
        assert_eq!(idx("berserk"), Some(59));
        assert_eq!(idx("blind"), Some(62));
        assert_eq!(idx("teleport"), Some(63));
        assert_eq!(idx("clone"), Some(65));
        assert_eq!(idx("summonAirElemental"), Some(69));
        assert_eq!(idx("poison"), Some(71));
        assert_eq!(idx("acidBreathDamage"), Some(81));

        let mut seen = std::collections::HashMap::new();
        for (id, ix) in SPELL_ID_TABLE {
            if let Some(prev) = seen.insert(*ix, *id) {
                panic!("index {ix} 同时属于 {prev} 与 {id}（表尾曾有重复/错位段）");
            }
        }
        assert!(
            SPELL_ID_TABLE.len() >= 80,
            "条目数骤减（{}），疑似误删",
            SPELL_ID_TABLE.len()
        );
    }
}

pub(crate) static STACK_CHANCES_CACHE: std::sync::OnceLock<
    Mutex<Vec<(String, std::sync::Arc<Vec<i64>>)>>,
> = std::sync::OnceLock::new();

/// 初始部队出现概率。
pub(crate) fn cached_starting_stack_chances(game_dir: &Path) -> std::sync::Arc<Vec<i64>> {
    let cands = starting_stack_chances_candidates(game_dir);
    let key = format!("chance:{}", fingerprint_of(&cands));
    cfg_cache_get(&STACK_CHANCES_CACHE, key, || {
        read_starting_stack_chances(game_dir)
    })
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
    cfg_cache_get(&HERO_CLASS_INDEX_CACHE, key, || {
        read_hero_class_index_map(game_dir)
    })
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
pub(crate) fn push_config_object(
    out: &mut Vec<(String, Value)>,
    path: &str,
    v: Value,
    _type: &str,
) {
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
