//! h3-core 文案表与原版（core）伪模组识别。
//!
//! - **LOD 文案**：`data/h3-core/<lang>.<category>.json`（由 `scripts/gen-h3-core-texts.mjs`
//!   从游戏 LOD 提取并随启动器打包）。本目录的约定是：**只放能从 LOD 重放的数据**。
//! - **人工补充文案**：`resources/h3-core-texts.json`（编译期内嵌），只放 LOD / VCMI 翻译池
//!   都没有的原版文本（城镇描述、中立阵营标签、法术派系译名、扩展资源名）。
//!   两者合并后转换为 VCMI i18n key，供原版名称/描述解析。
//! - `parse_core_mod`：按 modId 约定识别 `core`（父，全局数据）与 `core.<faction>`（子，城镇数据）。
//!
//! 依赖方向：只依赖 `infra`（见 `docs/architecture/backend.zh-CN.md` §4.1）。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::Value;

use crate::infra::cache::cfg_cache_get;

/// i18n 文案表：`key -> { locale -> text }`。
pub(crate) type I18nMap = HashMap<String, HashMap<String, String>>;

// 原版 9 城镇 id 顺序（参考 references/vcmi/lib/constants/StringConstants.h:74 的 NFaction::names 前 9 个）。
// 通用建筑（common.*）需展开到每个城镇，因 VCMI 中该建筑对所有 town 都存在且 key 带 town id。
pub(crate) const CORE_FACTIONS: &[&str] = &[
    "castle",
    "rampart",
    "tower",
    "inferno",
    "necropolis",
    "dungeon",
    "stronghold",
    "fortress",
    "conflux",
];

/// 之所以用 id 而非新增函数参数：批量/单次/测试等多处调用签名无需改动，
/// 且 `modId` 本身已是路由键，天然表达作用域。
pub(crate) fn parse_core_mod(mod_id: &str) -> (bool, Option<&str>) {
    if mod_id == "core" {
        return (true, None);
    }
    match mod_id.strip_prefix("core.") {
        Some(f) if !f.is_empty() => (true, Some(f)),
        _ => (false, None),
    }
}

/// 定位 h3-core 文案数据目录（由 scripts/gen-h3-core-texts.mjs 生成 english.*.json / chinese.*.json）。
///
/// 查找顺序（**开发期必须用编译期常量**）：
/// 1. `env!("CARGO_MANIFEST_DIR")/data/h3-core` —— 编译期常量，开发运行（tauri dev）必定可用。
///    注意：早先用的是*运行时* `std::env::var("CARGO_MANIFEST_DIR")`，开发运行时不保证存在，
///    导致日志里 `h3 keys=0`（文案表从未加载），原版名称/描述全部回落成 id。
/// 2. exe 同级 `data/h3-core`（打包后 bundle.resources 的目录项落点）。
/// 3. exe 同级 `resources/h3-core`（另一种打包布局）。
pub(crate) fn h3_core_data_dir() -> Option<PathBuf> {
    let dev = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("data")
        .join("h3-core");
    if dev.is_dir() {
        return Some(dev);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let p = parent.join("data").join("h3-core");
            if p.is_dir() {
                return Some(p);
            }
            let p2 = parent.join("resources").join("h3-core");
            if p2.is_dir() {
                return Some(p2);
            }
        }
    }
    None
}

/// h3-core 数据内容的**指纹**（用于缓存失效，仿 `schema_content_hash`）。
///
/// 与 `resource::h3_data_key`（只表示原版 LOD 在不在）不同，本指纹覆盖
/// `data/h3-core/*.json` 的**内容**（文件名 + 大小 + mtime）以及
/// `resources/h3-core-texts.json` 的编译期内容。
/// 改文案（LOD 产物或人工补充表）后**无需手动 bump `WIKI_CACHE_VERSION`**
/// 即可触发静默重建——否则这类改动会"改了不生效"。
pub(crate) fn h3_core_data_hash() -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let dir = h3_core_data_dir();
    let Some(dir) = dir else {
        return "missing".to_string();
    };
    let mut files: Vec<PathBuf> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("json") {
                files.push(p);
            }
        }
    }
    files.sort();
    let mut hasher = DefaultHasher::new();
    // 人工补充表是编译期内嵌的，不在目录里，必须单独混入
    CORE_TEXTS_JSON.hash(&mut hasher);
    for f in files {
        if let Ok(meta) = std::fs::metadata(&f) {
            f.file_name().hash(&mut hasher);
            meta.len().hash(&mut hasher);
            if let Ok(t) = meta.modified() {
                let dur = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
                dur.as_nanos().hash(&mut hasher);
            }
        }
    }
    format!("{:016x}", hasher.finish())
}

/// 人工补充文案（编译期内嵌）：只放**不能从 LOD 重放**的原版文本。
///
/// 结构（每个 section 独立决定语言覆盖，键用 VCMI 语言标识）：
/// - `factionDescriptions` → `faction.core.<id>.description`
/// - `factionLabels` → `faction.core.<id>.name`（如 `neutral`）
/// - `resourceNames` → `resource.core.<id>.name`（仅 LOD 之外的扩展资源，如 HotA `mitril`）
/// - `spellSchoolNames` → `spellSchool.core.<id>.name`
const CORE_TEXTS_JSON: &str = include_str!("../../resources/h3-core-texts.json");

/// 解析后的补充文案表（进程内只解析一次）。
pub(crate) fn core_texts() -> &'static Value {
    static TEXTS: std::sync::OnceLock<Value> = std::sync::OnceLock::new();
    TEXTS.get_or_init(|| {
        serde_json::from_str(CORE_TEXTS_JSON).expect("resources/h3-core-texts.json 解析失败")
    })
}

pub(crate) fn str_field(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string()
}

/// 读取 h3-core 文案表（`<lang>.<category>.json`，语言按目录内容发现），转换为
/// VCMI i18n key 格式，以便 build_mod_wiki_inner 在提取原版（mod_id="core"）时命中名称；
/// 随后合并 `resources/h3-core-texts.json` 的人工补充文案（只补缺失语言）。
/// key 前缀与 build_mod_wiki_inner 内部 build_name_object 调用的格式一致。
pub(crate) fn load_h3_core_i18n(dir: &Path) -> Option<I18nMap> {
    let read = |lang: &str, cat: &str| -> Value {
        let p = dir.join(format!("{}.{}.json", lang, cat));
        fs::read_to_string(&p)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or(Value::Null)
    };
    // 语言集合**由目录内容决定**（`<lang>.<category>.json`），不在代码里列举：
    // 生成器多跑一种语言 = 自动多加载一种，无需改 Rust。
    let langs = discover_langs(dir);
    let cats = [
        "creatures",
        "factions",
        "heroes",
        "heroClasses",
        "artifacts",
        "spells",
        "skills",
        "terrains",
        "buildings",
        // 资源名：LOD 的 RESTYPES.TXT（行号 = config/resources.json 的 index）
        "resources",
        // 数组文案（ARRAYTXT.TXT）：四维主属性的右键说明等，VCMI 翻译池里同样没有
        "arraytxt",
    ];
    let empty = Value::Object(serde_json::Map::new());
    let mut out: I18nMap = std::collections::HashMap::new();
    for cat in cats {
        for lang in &langs {
            let data = read(lang, cat);
            let map = match &data {
                Value::Object(m) => m,
                _ => continue,
            };
            for (id, val) in map {
                let entries: Vec<(String, String)> = match cat {
                    "creatures" => vec![
                        (
                            format!("creatures.core.{}.name.singular", id),
                            str_field(val, "name"),
                        ),
                        (
                            format!("creature.core.{}.description", id),
                            str_field(val, "abilityText"),
                        ),
                    ],
                    // 原版城镇描述不在 LOD 里（TOWNTYPE.TXT 只有名字），
                    // 由 `resources/h3-core-texts.json` 的 `factionDescriptions` 合并进来。
                    "factions" => {
                        vec![(format!("faction.core.{}.name", id), str_field(val, "name"))]
                    }
                    "heroes" => {
                        let spec = val.get("specialty").unwrap_or(&empty);
                        vec![
                            (format!("hero.core.{}.name", id), str_field(val, "name")),
                            (
                                format!("hero.core.{}.biography", id),
                                str_field(val, "biography"),
                            ),
                            (
                                format!("hero.core.{}.specialty.name", id),
                                str_field(spec, "name"),
                            ),
                            (
                                format!("hero.core.{}.specialty.description", id),
                                str_field(spec, "description"),
                            ),
                            (
                                format!("hero.core.{}.specialty.tooltip", id),
                                str_field(spec, "tooltip"),
                            ),
                        ]
                    }
                    "heroClasses" => vec![(
                        format!("heroClass.core.{}.name", id),
                        str_field(val, "name"),
                    )],
                    // 数组文案（`core.arraytxt.<行号>`）：VCMI `CGeneralTextHandler::arraytxt`。
                    // 英雄窗口四维主属性的右键说明就是 `arraytxt[2 + v]`（`CHeroWindow.cpp:131-132`），
                    // 文本形如 `{名称}\n\n描述`（与 bonus 文案同款）。
                    "arraytxt" => {
                        vec![(format!("core.arraytxt.{}", id), str_field(val, "text"))]
                    }
                    // 资源名（木材/水银/…）：`resource.core.<id>.name`
                    "resources" => {
                        vec![(format!("resource.core.{}.name", id), str_field(val, "name"))]
                    }
                    "artifacts" => vec![
                        (format!("artifact.core.{}.name", id), str_field(val, "name")),
                        (
                            format!("artifact.core.{}.description", id),
                            str_field(val, "description"),
                        ),
                    ],
                    "spells" => {
                        let lv = val.get("levelDescriptions").unwrap_or(&empty);
                        // level 为数值（非法术文本），但同样来自 SPTRAITS，随文案一起烘进
                        // h3-core 后即可彻底免去运行时的 SPTRAITS.TXT 解码。
                        let level = val
                            .get("level")
                            .and_then(|x| x.as_i64())
                            .map(|n| n.to_string())
                            .unwrap_or_default();
                        // 派系为数组（earth/water/fire/air），以逗号连接后由使用方拆分
                        let schools = val
                            .get("schools")
                            .and_then(|x| x.as_array())
                            .map(|a| {
                                a.iter()
                                    .filter_map(|s| s.as_str())
                                    .collect::<Vec<_>>()
                                    .join(",")
                            })
                            .unwrap_or_default();
                        vec![
                            (format!("spell.core.{}.name", id), str_field(val, "name")),
                            (
                                format!("spell.core.{}.description", id),
                                str_field(lv, "basic"),
                            ),
                            (format!("spell.core.{}.basic", id), str_field(lv, "basic")),
                            (
                                format!("spell.core.{}.advanced", id),
                                str_field(lv, "advanced"),
                            ),
                            (format!("spell.core.{}.expert", id), str_field(lv, "expert")),
                            (format!("spell.core.{}.level", id), level),
                            (format!("spell.core.{}.schools", id), schools),
                        ]
                    }
                    "skills" => vec![
                        (format!("skill.core.{}.name", id), str_field(val, "name")),
                        (format!("skill.core.{}.basic", id), str_field(val, "basic")),
                        (
                            format!("skill.core.{}.advanced", id),
                            str_field(val, "advanced"),
                        ),
                        (
                            format!("skill.core.{}.expert", id),
                            str_field(val, "expert"),
                        ),
                    ],
                    "terrains" => {
                        vec![(format!("terrain.core.{}.name", id), str_field(val, "name"))]
                    }
                    "buildings" => {
                        let name = str_field(val, "name");
                        let desc = str_field(val, "description");
                        let parts: Vec<&str> = id.splitn(2, '.').collect();
                        if parts.len() == 2 {
                            let (faction, bid) = (parts[0], parts[1]);
                            if faction == "common" {
                                CORE_FACTIONS
                                    .iter()
                                    .flat_map(|f| {
                                        vec![
                                            (
                                                format!("building.core.{}.{}.name", f, bid),
                                                name.clone(),
                                            ),
                                            (
                                                format!("building.core.{}.{}.description", f, bid),
                                                desc.clone(),
                                            ),
                                        ]
                                    })
                                    .collect()
                            } else {
                                vec![
                                    (format!("building.core.{}.{}.name", faction, bid), name),
                                    (
                                        format!("building.core.{}.{}.description", faction, bid),
                                        desc,
                                    ),
                                ]
                            }
                        } else {
                            vec![]
                        }
                    }
                    _ => vec![],
                };
                for (k, v) in entries {
                    if v.is_empty() {
                        continue;
                    }
                    out.entry(k)
                        .or_insert_with(std::collections::HashMap::new)
                        .insert(lang.to_string(), v);
                }
            }
        }
    }
    // **键统一小写**：VCMI 的翻译查找大小写不敏感（内部按小写存储/查询），
    // 而键里实体 id 的原始大小写与 mod 配置/查询键常有出入
    // （h3-core 的 `sn01BPoisonLeaf` vs 配置 id、`vampiresCowl` 等）。
    // 查询端 `wiki::i18n::lookup` 同样按小写回退——两端一致才能命中。
    let mut out: I18nMap = out
        .into_iter()
        .map(|(k, v)| (k.to_lowercase(), v))
        .collect();
    merge_core_texts(&mut out);
    if out.is_empty() {
        return None;
    }
    Some(out)
}

/// 目录里的 `<lang>.<category>.json` 文件名 → 语言集合（排除 `meta.*`）。
fn discover_langs(dir: &Path) -> Vec<String> {
    let mut set = std::collections::BTreeSet::new();
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            let Some((lang, _)) = name.split_once('.') else {
                continue;
            };
            if lang == "meta" || !name.ends_with(".json") {
                continue;
            }
            set.insert(lang.to_string());
        }
    }
    set.into_iter().collect()
}

/// 把 `resources/h3-core-texts.json` 的补充文案合并进 i18n 表。
///
/// **只补缺失语言**（mod 翻译 / 未来上游译文优先），缺失的语言键不写副本。
fn merge_core_texts(out: &mut I18nMap) {
    let Some(root) = core_texts().as_object() else {
        return;
    };
    for (section, prefix, suffix) in [
        ("factionDescriptions", "faction.core.", ".description"),
        ("factionLabels", "faction.core.", ".name"),
        ("resourceNames", "resource.core.", ".name"),
        ("spellSchoolNames", "spellSchool.core.", ".name"),
    ] {
        let Some(Value::Object(items)) = root.get(section) else {
            continue;
        };
        for (id, langs) in items {
            let Value::Object(langs) = langs else {
                continue;
            };
            let entry = out
                .entry(format!("{prefix}{id}{suffix}").to_lowercase())
                .or_default();
            for (lang, text) in langs {
                let Some(t) = text
                    .as_str()
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                else {
                    continue;
                };
                entry.entry(lang.clone()).or_insert_with(|| t.to_string());
            }
        }
    }
}

/// 原版英雄**初始部队**表（`<lang>.heroes.json` 的 `army` 字段）。
///
/// 增强版 HOTRAITS.TXT 仅含 `Name` + 3 组 `Low/High/Type`（初始部队），**不含** 四维主属性；
/// 四维主属性由 `h3_hero_class_stats()` 从 HCTRAITS.TXT 提供。
/// 初始部队已随 h3-core 一并烘入。
/// 数值与语言无关，读中文文件即可（回退英文）。
pub(crate) static H3_HERO_STATS_CACHE: std::sync::OnceLock<
    Mutex<Vec<(String, std::sync::Arc<HashMap<String, Value>>)>>,
> = std::sync::OnceLock::new();

pub(crate) fn h3_hero_stats() -> Option<std::sync::Arc<HashMap<String, Value>>> {
    let dir = h3_core_data_dir()?;
    let key = dir.to_string_lossy().to_string();
    let map = cfg_cache_get(&H3_HERO_STATS_CACHE, key, || {
        let mut m: HashMap<String, Value> = HashMap::new();
        for lang in ["chinese", "english"] {
            let p = dir.join(format!("{lang}.heroes.json"));
            let Ok(text) = fs::read_to_string(&p) else {
                continue;
            };
            let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            for (id, v) in obj {
                if let Some(army) = v.get("army") {
                    if army.is_array() && !army.as_array().unwrap().is_empty() {
                        let mut entry = serde_json::Map::new();
                        entry.insert("army".to_string(), army.clone());
                        m.entry(id).or_insert_with(|| Value::Object(entry));
                    }
                }
            }
            if !m.is_empty() {
                break;
            }
        }
        m
    });
    if map.is_empty() {
        None
    } else {
        Some(map)
    }
}

/// 原版英雄职业**初始四维主属性**表（`<lang>.heroClasses.json` 的 `stats` 字段）。
///
/// HCTRAITS.TXT 按职业顺序给出 Attack / Defense / Spell Power / Knowledge，
/// VCMI `config/heroClasses.json` 再按 index 把职业名映射到 class_id；
/// 下游用 `class_id -> index -> stats` 链命中。
/// 数值与语言无关，读中文文件即可（回退英文）。
pub(crate) static H3_HERO_CLASS_STATS_CACHE: std::sync::OnceLock<
    Mutex<
        Vec<(
            String,
            std::sync::Arc<HashMap<String, (i64, i64, i64, i64)>>,
        )>,
    >,
> = std::sync::OnceLock::new();

pub(crate) fn h3_hero_class_stats() -> Option<std::sync::Arc<HashMap<String, (i64, i64, i64, i64)>>>
{
    let dir = h3_core_data_dir()?;
    let key = dir.to_string_lossy().to_string();
    let map = cfg_cache_get(&H3_HERO_CLASS_STATS_CACHE, key, || {
        let mut m: HashMap<String, (i64, i64, i64, i64)> = HashMap::new();
        for lang in ["chinese", "english"] {
            let p = dir.join(format!("{lang}.heroClasses.json"));
            let Ok(text) = fs::read_to_string(&p) else {
                continue;
            };
            let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            for (id, v) in obj {
                let Some(Value::Object(s)) = v.get("stats") else {
                    continue;
                };
                let get = |k: &str| s.get(k).and_then(|x| x.as_i64()).unwrap_or(0);
                m.insert(
                    id,
                    (
                        get("attack"),
                        get("defense"),
                        get("spellPower"),
                        get("knowledge"),
                    ),
                );
            }
            if !m.is_empty() {
                break;
            }
        }
        m
    });
    if map.is_empty() {
        None
    } else {
        Some(map)
    }
}

/// 原版城镇建筑**花费**表（`<lang>.buildingCosts.json`）。
///
/// VCMI 在 `CTownHandler::loadLegacyData()` 中从 `DATA/BUILDING.TXT` 读取原版建筑花费；
/// 官方/增强版 `config/factions/<faction>.json` 的 `buildings.<id>` 通常不写 `cost`。
/// 因此把 BUILDING.TXT 的成本按 `<faction>.<bid>` / `common.<bid>` 烘入 h3-core，
/// 供 `build_town_config` 在配置缺省时回退。
pub(crate) static H3_BUILDING_COSTS_CACHE: std::sync::OnceLock<
    Mutex<Vec<(String, std::sync::Arc<HashMap<String, Value>>)>>,
> = std::sync::OnceLock::new();

pub(crate) fn h3_building_costs() -> Option<std::sync::Arc<HashMap<String, Value>>> {
    let dir = h3_core_data_dir()?;
    let key = dir.to_string_lossy().to_string();
    let map = cfg_cache_get(&H3_BUILDING_COSTS_CACHE, key, || {
        let mut m: HashMap<String, Value> = HashMap::new();
        for lang in ["chinese", "english"] {
            let p = dir.join(format!("{lang}.buildingCosts.json"));
            let Ok(text) = fs::read_to_string(&p) else {
                continue;
            };
            let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            for (id, v) in obj {
                if let Some(cost) = v.as_object() {
                    m.entry(id).or_insert_with(|| Value::Object(cost.clone()));
                }
            }
            if !m.is_empty() {
                break;
            }
        }
        m
    });
    if map.is_empty() {
        None
    } else {
        Some(map)
    }
}

/// 查询指定城镇某建筑的花费。优先 `<faction>.<bid>`，未命中则回退 `common.<bid>`。
pub(crate) fn h3_building_cost(faction_id: &str, bid: &str) -> Option<Value> {
    let costs = h3_building_costs()?;
    costs
        .get(&format!("{faction_id}.{bid}"))
        .or_else(|| costs.get(&format!("common.{bid}")))
        .cloned()
}

/// 原版生物**数值属性**表（`<lang>.creatures.json` 的 `stats` 字段）。
///
/// 原版 `config/creatures/*.json` **不含** attack / defense / hitPoints / speed /
/// damage / cost 等数值字段（实测仅 index/level/faction/upgrades/abilities/graphics/sound），
/// 这些数值只存在于 H3 legacy `CRTRAITS.TXT`，已随 h3-core 一并烘入。
/// 数值与语言无关，读中文文件即可（回退英文）。
pub(crate) static H3_CREATURE_STATS_CACHE: std::sync::OnceLock<
    Mutex<Vec<(String, std::sync::Arc<HashMap<String, Value>>)>>,
> = std::sync::OnceLock::new();

pub(crate) fn h3_creature_stats() -> Option<std::sync::Arc<HashMap<String, Value>>> {
    let dir = h3_core_data_dir()?;
    let key = dir.to_string_lossy().to_string();
    let map = cfg_cache_get(&H3_CREATURE_STATS_CACHE, key, || {
        let mut m: HashMap<String, Value> = HashMap::new();
        for lang in ["chinese", "english"] {
            let p = dir.join(format!("{lang}.creatures.json"));
            let Ok(text) = fs::read_to_string(&p) else {
                continue;
            };
            let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            for (id, v) in obj {
                if let Some(stats) = v.get("stats") {
                    if stats.is_object() {
                        m.entry(id).or_insert_with(|| stats.clone());
                    }
                }
            }
            if !m.is_empty() {
                break;
            }
        }
        m
    });
    if map.is_empty() {
        None
    } else {
        Some(map)
    }
}

/// 原版宝物**元数据**表（`<lang>.artifacts.json` 的 value/slots/class）。
///
/// 原版 `config/artifacts.json` 只声明 index/type（+ 组合宝物的 components/bonuses），
/// 售价 / 装备位 / 等级只存在于 H3 legacy `ARTRAITS.TXT`，已随 h3-core 烘入；
/// 这三个字段语言无关，读中文文件即可（回退英文）。
pub(crate) static H3_ARTIFACT_META_CACHE: std::sync::OnceLock<
    Mutex<Vec<(String, std::sync::Arc<HashMap<String, Value>>)>>,
> = std::sync::OnceLock::new();

pub(crate) fn h3_artifact_meta() -> Option<std::sync::Arc<HashMap<String, Value>>> {
    let dir = h3_core_data_dir()?;
    let key = dir.to_string_lossy().to_string();
    let map = cfg_cache_get(&H3_ARTIFACT_META_CACHE, key, || {
        let mut m: HashMap<String, Value> = HashMap::new();
        for lang in ["chinese", "english"] {
            let p = dir.join(format!("{lang}.artifacts.json"));
            let Ok(text) = fs::read_to_string(&p) else {
                continue;
            };
            let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            for (id, v) in obj {
                m.entry(id).or_insert_with(|| {
                    serde_json::json!({
                        "value": v.get("value").cloned().unwrap_or(Value::from(0)),
                        "slots": v.get("slots").cloned().unwrap_or_else(|| Value::Array(vec![])),
                        "class": v.get("class").cloned().unwrap_or_else(|| Value::from("")),
                    })
                });
            }
            if !m.is_empty() {
                break;
            }
        }
        m
    });
    if map.is_empty() {
        None
    } else {
        Some(map)
    }
}

/// 原版建筑 id 列表（`<lang>.buildings.json` 的键；形如 `castle.mageGuild1` / `neutral.grail`）。
///
/// 用于把 `neutral.*`（中立建筑：宝物商店/生物巢穴/船坞/神器…）补进百科"建筑"分类——
/// 这些条目不隶属于任何城镇，VCMI 战斗/城镇界面也不展示，只能从 legacy `BUILDING.TXT` 取。
pub(crate) static H3_BUILDING_IDS_CACHE: std::sync::OnceLock<
    Mutex<Vec<(String, std::sync::Arc<Vec<String>>)>>,
> = std::sync::OnceLock::new();

pub(crate) fn h3_building_ids(prefix: &str) -> Vec<String> {
    let Some(dir) = h3_core_data_dir() else {
        return Vec::new();
    };
    let key = dir.to_string_lossy().to_string();
    let all = cfg_cache_get(&H3_BUILDING_IDS_CACHE, key, || {
        let mut ids: Vec<String> = Vec::new();
        for lang in ["chinese", "english"] {
            let p = dir.join(format!("{lang}.buildings.json"));
            let Ok(text) = fs::read_to_string(&p) else {
                continue;
            };
            let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            ids = obj.keys().cloned().collect();
            if !ids.is_empty() {
                break;
            }
        }
        ids.sort();
        ids
    });
    all.iter()
        .filter(|k| k.starts_with(prefix))
        .cloned()
        .collect()
}

/// 将 h3-core 文案表合并进核心翻译表（h3-core 的同名 key 覆盖 VCMI 内置翻译的缺失项）。
pub(crate) fn merge_i18n_into(base: &mut I18nMap, extra: &I18nMap) {
    for (k, v) in extra {
        let e = base
            .entry(k.clone())
            .or_insert_with(std::collections::HashMap::new);
        for (lk, lv) in v {
            e.insert(lk.clone(), lv.clone());
        }
    }
}

pub(crate) static CORE_I18N_CACHE: std::sync::OnceLock<
    Mutex<Vec<(String, std::sync::Arc<I18nMap>)>>,
> = std::sync::OnceLock::new();

/// 核心模组翻译表（按 mod 自身的 sourceHash 作缓存键，与构建增量判定同源）。
pub(crate) fn cached_core_i18n(core_mod: &Path) -> std::sync::Arc<I18nMap> {
    let key = format!("i18n:{}", crate::infra::hash::compute_source_hash(core_mod));
    cfg_cache_get(&CORE_I18N_CACHE, key, || {
        crate::vcmi::i18n::read_i18n_map(core_mod)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 启动器 UI 支持的语言（VCMI 语言标识）。补充表只允许这些键，
    /// 防止 `en`/`zh` 短码混入。
    const ALLOWED_LABEL_LANGS: &[&str] = &[
        "english", "chinese", "tchinese", "japanese", "german", "polish",
    ];

    fn assert_lang_entries(section: &str, id: &str, entry: &Value) {
        let langs = entry
            .as_object()
            .unwrap_or_else(|| panic!("`{section}.{id}` 应为语言对象"));
        assert!(!langs.is_empty(), "`{section}.{id}` 没有任何语言");
        for (lang, text) in langs {
            assert!(
                ALLOWED_LABEL_LANGS.contains(&lang.as_str()),
                "`{section}.{id}` 非法语言键 `{lang}`（应取 VCMI 语言标识）"
            );
            assert!(
                text.as_str().map(|s| !s.trim().is_empty()).unwrap_or(false),
                "`{section}.{id}.{lang}` 为空"
            );
        }
        // 不允许把英文抄成中文：两者同时存在时必须不同
        if let (Some(en), Some(zh)) = (
            langs.get("english").and_then(|v| v.as_str()),
            langs.get("chinese").and_then(|v| v.as_str()),
        ) {
            assert_ne!(en.trim(), zh.trim(), "`{section}.{id}` 的中英文本相同，疑似漏翻");
        }
    }

    /// 人工补充文案表的守门测试。
    ///
    /// `resources/h3-core-texts.json` 只放**不能从 LOD 重放**的原版文本：
    /// 城镇描述、中立阵营标签、LOD 之外的资源名、法术派系译名。
    /// 键必须是 VCMI 语言标识；缺语言不补副本（前端 `pickName` 回退 english）。
    #[test]
    fn core_texts_are_valid_and_vcmi_keyed() {
        let root = core_texts().as_object().expect("补充表顶层应为对象");

        let factions = root
            .get("factionDescriptions")
            .and_then(|v| v.as_object())
            .expect("缺少 factionDescriptions");
        for (id, entry) in factions {
            assert!(
                CORE_FACTIONS.contains(&id.as_str()),
                "未知阵营 id `{id}`"
            );
            assert_lang_entries("factionDescriptions", id, entry);
        }
        assert_eq!(
            factions.len(),
            CORE_FACTIONS.len(),
            "9 个原版阵营应全覆盖（当前 {} 条）",
            factions.len()
        );

        for (section, ids) in [
            ("factionLabels", vec!["neutral"]),
            ("spellSchoolNames", vec!["air", "fire", "earth", "water"]),
        ] {
            let map = root
                .get(section)
                .and_then(|v| v.as_object())
                .unwrap_or_else(|| panic!("缺少 {section}"));
            for id in ids {
                let entry = map
                    .get(id)
                    .unwrap_or_else(|| panic!("{section} 缺少 `{id}`"));
                assert_lang_entries(section, id, entry);
            }
        }

        let resources = root
            .get("resourceNames")
            .and_then(|v| v.as_object())
            .expect("缺少 resourceNames");
        for (id, entry) in resources {
            assert_lang_entries("resourceNames", id, entry);
        }
    }

    /// 补充表必须在加载期合并进 i18n（锁住“数据在但没接线”）。
    #[test]
    fn core_texts_merge_into_i18n() {
        let dir = h3_core_data_dir().expect("开发期应能定位 data/h3-core");
        let i18n = load_h3_core_i18n(&dir).expect("h3-core i18n 应可加载");

        let desc = i18n
            .get("faction.core.castle.description")
            .expect("城镇描述应来自补充表");
        assert_eq!(desc.get("chinese").map(String::as_str), Some("人类王国埃拉西亚的堡垒，骑士与牧师在此守护正义而法度严明的国度。凭借复活法术与全阵营最高的士气，长枪兵、狮鹫与天使组成了安塔瑞亚最均衡的军队。"));
        assert!(desc.contains_key("english"));

        let neutral = i18n
            .get("faction.core.neutral.name")
            .expect("中立阵营标签应来自补充表");
        assert_eq!(neutral.get("chinese").map(String::as_str), Some("中立"));
        assert_eq!(neutral.get("english").map(String::as_str), Some("Neutral"));

        // 加载器会把键统一小写（查询端 `wiki::i18n::lookup` 同样按小写回退）
        let school = i18n
            .get("spellschool.core.air.name")
            .expect("法术派系名应来自补充表");
        assert_eq!(school.get("chinese").map(String::as_str), Some("气系"));

        // 官方 7 资源来自 LOD 的 RESTYPES.TXT；HotA 的 mitril 来自补充表
        let wood = i18n
            .get("resource.core.wood.name")
            .expect("官方资源名应来自 LOD");
        assert_eq!(wood.get("chinese").map(String::as_str), Some("木材"));
        let mitril = i18n
            .get("resource.core.mitril.name")
            .expect("扩展资源名应来自补充表");
        assert_eq!(mitril.get("chinese").map(String::as_str), Some("秘银"));
    }

    /// `data/h3-core` 的契约：只放 LOD 可重放数据。
    /// 城镇描述属于人工文案，不得再出现在生成产物里。
    #[test]
    fn generated_factions_carry_only_lod_data() {
        let dir = h3_core_data_dir().expect("开发期应能定位 data/h3-core");
        for lang in discover_langs(&dir) {
            let p = dir.join(format!("{lang}.factions.json"));
            let Ok(text) = std::fs::read_to_string(&p) else {
                continue;
            };
            let v: Value = serde_json::from_str(&text).expect("factions.json 不是合法 JSON");
            for (id, entry) in v.as_object().expect("factions 顶层应为对象") {
                assert!(
                    entry.get("description").is_none(),
                    "`{lang}.factions.json` 的 `{id}` 不应包含 description（人工文案在 resources/h3-core-texts.json）"
                );
            }
        }
    }

    #[test]
    fn hero_class_stats_has_core_values() {
        let stats = h3_hero_class_stats().expect("h3-core heroClasses.json should be available");
        let knight = stats
            .get("class0")
            .expect("class0 (Knight) stats should exist");
        assert_eq!(
            *knight,
            (2, 2, 1, 1),
            "Knight base primary skills should be 2/2/1/1"
        );
        let wizard = stats
            .get("class5")
            .expect("class5 (Wizard) stats should exist");
        assert_eq!(
            *wizard,
            (0, 0, 2, 3),
            "Wizard base primary skills should be 0/0/2/3"
        );
    }

    #[test]
    fn building_cost_fallback_works() {
        // 通用建筑：城堡的 mageGuild1 与 common.mageGuild1 应一致
        let castle =
            h3_building_cost("castle", "mageGuild1").expect("castle.mageGuild1 cost should exist");
        let common =
            h3_building_cost("castle", "mageGuild1").expect("common.mageGuild1 cost should exist");
        assert_eq!(
            castle, common,
            "castle.mageGuild1 should equal common.mageGuild1"
        );
        let obj = castle.as_object().expect("cost should be an object");
        assert_eq!(obj.get("wood").and_then(|v| v.as_i64()), Some(5));
        assert_eq!(obj.get("ore").and_then(|v| v.as_i64()), Some(5));
        assert_eq!(obj.get("gold").and_then(|v| v.as_i64()), Some(2000));

        // 巢穴：城堡 1 级兵舍（pikeman）花费 10 ore + 500 gold
        let dwell = h3_building_cost("castle", "dwellingLvl1")
            .expect("castle.dwellingLvl1 cost should exist");
        let dwell_obj = dwell
            .as_object()
            .expect("dwelling cost should be an object");
        assert_eq!(dwell_obj.get("ore").and_then(|v| v.as_i64()), Some(10));
        assert_eq!(dwell_obj.get("gold").and_then(|v| v.as_i64()), Some(500));

        // 仅存在于 common 的建筑（如 villageHall）对任一城镇都应回退命中
        let hall = h3_building_cost("inferno", "villageHall")
            .expect("villageHall should be resolved via common.<bid>");
        assert!(hall.as_object().unwrap().contains_key("gold"));
    }
}
