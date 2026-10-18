//! h3-core 文案表与原版（core）伪模组识别。
//!
//! - 文案表：`data/h3-core/{english,chinese}.<category>.json`（由 `scripts/gen-h3-core-texts.mjs`
//!   生成并随启动器打包），转换为 VCMI i18n key 格式后供原版名称/描述解析。
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
/// `data/h3-core/*.json` 的**内容**（文件名 + 大小 + mtime 的哈希）。
/// 改 h3-core 文案（如给原版城镇补 `description`）后**无需手动 bump
/// `WIKI_CACHE_VERSION`** 即可触发静默重建——否则这类改动会"改了不生效"。
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

pub(crate) fn str_field(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string()
}

/// 读取 h3-core 文案表（english.*.json + chinese.*.json），转换为 VCMI i18n key 格式，
/// 以便 build_mod_wiki_inner 在提取原版（mod_id="core"）时通过 core_i18n 命中名称。
/// key 前缀与 build_mod_wiki_inner 内部 build_name_object 调用的格式一致。
pub(crate) fn load_h3_core_i18n(dir: &Path) -> Option<I18nMap> {
    let read = |lang: &str, cat: &str| -> Value {
        let p = dir.join(format!("{}.{}.json", lang, cat));
        fs::read_to_string(&p)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or(Value::Null)
    };
    let langs = ["english", "chinese"];
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
        // 资源名 / 法术派系名：VCMI 翻译池里没有对应键（实测 0 条），且 `config/resources.json`
        // 不含 name 字段，故由 H3 官方译名烘入 h3-core（见 scripts/gen-h3-core-texts.mjs）
        "resources",
        "spellSchools",
        // 数组文案（ARRAYTXT.TXT）：四维主属性的右键说明等，VCMI 翻译池里同样没有
        "arraytxt",
    ];
    let empty = Value::Object(serde_json::Map::new());
    let mut out: I18nMap = std::collections::HashMap::new();
    for cat in cats {
        for lang in langs {
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
                    "factions" => vec![
                        (format!("faction.core.{}.name", id), str_field(val, "name")),
                        // 原版城镇描述：VCMI 自身 faction 定义无此字段，由 `gen-h3-core-texts.mjs`
                        // 从 `data/h3-core-src/factionDescriptions.json`（百科，中英双语）烘入。
                        (
                            format!("faction.core.{}.description", id),
                            str_field(val, "description"),
                        ),
                    ],
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
                    // 法术派系名（气系/火系/…）：`spellSchool.core.<id>.name`
                    "spellSchools" => vec![(
                        format!("spellSchool.core.{}.name", id),
                        str_field(val, "name"),
                    )],
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
    if out.is_empty() {
        return None;
    }
    // **键统一小写**：VCMI 的翻译查找大小写不敏感（内部按小写存储/查询），
    // 而键里实体 id 的原始大小写与 mod 配置/查询键常有出入
    // （h3-core 的 `sn01BPoisonLeaf` vs 配置 id、`vampiresCowl` 等）。
    // 查询端 `wiki::i18n::lookup` 同样按小写回退——两端一致才能命中。
    Some(
        out.into_iter()
            .map(|(k, v)| (k.to_lowercase(), v))
            .collect(),
    )
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

    /// 手写源数据 `data/h3-core-src/factionDescriptions.json` 的守门测试。
    ///
    /// 该文件由 `scripts/gen-h3-core-texts.mjs` 按 id 合并进生成产物；
    /// 脚本对**未知 id 静默忽略**、缺语言**静默退 `english`** —— 写错了不会有任何提示，
    /// 只会在百科里"少一段描述"。这里把口子堵住：id 必须是原版 9 城镇之一，
    /// `english` / `chinese` 都必须非空且不同（确认真的翻译过，而不是复制英文）。
    ///
    /// 语言键必须是 VCMI 语言标识（`english`/`chinese`，见 `lib/texts/Languages.h`
    /// 与 `config/schemas/mod.json` 的 enum）；早期用 `en`/`zh` 短码，脚本按 `d[lang]`
    /// 取不到、中文分支静默落成英文。
    #[test]
    fn faction_descriptions_src_covers_core_factions() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("data")
            .join("h3-core-src")
            .join("factionDescriptions.json");
        let text = std::fs::read_to_string(&path).expect("缺少城镇描述源数据");
        let v: Value = serde_json::from_str(&text).expect("factionDescriptions.json 不是合法 JSON");
        let obj = v.as_object().expect("顶层应为对象（id → {english, chinese}）");

        for (id, entry) in obj {
            assert!(
                CORE_FACTIONS.contains(&id.as_str()),
                "未知阵营 id `{id}`（脚本会静默忽略，等于白写）"
            );
            let en = entry["english"].as_str().unwrap_or("").trim();
            let zh = entry["chinese"].as_str().unwrap_or("").trim();
            assert!(!en.is_empty(), "`{id}` 缺少 english 描述");
            assert!(!zh.is_empty(), "`{id}` 缺少 chinese 描述");
            assert_ne!(en, zh, "`{id}` 的 chinese 与 english 完全相同，疑似漏翻");
        }
        assert_eq!(
            obj.len(),
            CORE_FACTIONS.len(),
            "9 个原版阵营应全覆盖（当前 {} 条）",
            obj.len()
        );
    }

    /// 生成产物必须与手写源数据一致。
    ///
    /// 堵住两类历史问题：
    /// ① 源文件用短码 `en`/`zh` 而脚本按 `d[lang]`（`english`/`chinese`）取值 →
    ///    中文产物静默变成英文；
    /// ② 改了源数据却没重跑 `scripts/gen-h3-core-texts.mjs` → 产物与源脱节
    ///    （实测 9 个城镇 en/zh 全部不一致）。
    #[test]
    fn faction_descriptions_generated_match_src() {
        let data = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("data");
        let src_text = std::fs::read_to_string(
            data.join("h3-core-src").join("factionDescriptions.json"),
        )
        .expect("缺少城镇描述源数据");
        let src: Value = serde_json::from_str(&src_text).expect("源数据不是合法 JSON");

        for lang in ["english", "chinese"] {
            let gen_path = data.join("h3-core").join(format!("{lang}.factions.json"));
            let gen_text = std::fs::read_to_string(&gen_path)
                .unwrap_or_else(|e| panic!("缺少生成产物 {}: {e}", gen_path.display()));
            let gen: Value = serde_json::from_str(&gen_text).expect("生成产物不是合法 JSON");
            for (id, entry) in src.as_object().expect("源数据顶层应为对象") {
                let want = entry[lang].as_str().unwrap_or("").trim();
                let got = gen[id]["description"].as_str().unwrap_or("").trim();
                assert_eq!(
                    got, want,
                    "`{lang}.factions.json` 的 `{id}` 描述与源数据不一致（改完源文件需重跑 gen-h3-core-texts.mjs）"
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
