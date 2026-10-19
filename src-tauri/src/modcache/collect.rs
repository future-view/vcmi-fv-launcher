//! 缓存构建的数据收集：配置 JSON 合并、原始配置快照、派生数值计算。
//!
//! 依赖方向：可依赖 `infra` 与 `vcmi`（见 `docs/architecture/backend.zh-CN.md` §4.1）。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{json, Value};

use crate::infra::hash::config_tree_fingerprint;
use crate::infra::json::{collect_json_files, parse_vcmi_json};
use crate::vcmi::gamedata::push_config_object;

/// JSON 值的近似堆占用（字节）——用于给解析缓存记"体积"。
///
/// 只走一遍树（不序列化），单 mod 约数千节点，代价可忽略；精度够用即可
/// （缓存按字节上界淘汰，估偏一点只影响命中率，不影响正确性）。
fn value_weight(v: &Value) -> usize {
    match v {
        Value::Object(m) => {
            let mut n = 48 * m.len();
            for (k, x) in m {
                n += k.len() + value_weight(x);
            }
            n
        }
        Value::Array(a) => {
            let mut n = 24 * a.len();
            for x in a {
                n += value_weight(x);
            }
            n
        }
        Value::String(s) => 16 + s.len(),
        _ => 0,
    }
}

/// 配置收集结果缓存：`(mod 路径, 类型, 配置树指纹) → 解析产物`。
///
/// ⚠️ 必要性：`phase=0`（读取游戏数据）与 `phase=1`（提取游戏素材）会对**同一个 mod
/// 各解析一遍全部配置**——实测批 2 里 `cfg.raw` 29.3s + `cfg.creatures` 12.1s +
/// `cfg.town` 8.0s ≈ 整批 CPU 的 5%，纯重复。缓存后 phase=1 直接复用 phase=0 的结果。
///
/// ⚠️ 失效键用 [`config_tree_fingerprint`]（配置目录的相对路径 + 字节数）：
/// 只用 mtime 不够——改文件内容不改目录 mtime，会让"改了配置再重建"读到旧解析结果。
/// 配置收集缓存的内部结构（`clippy::type_complexity` 要求拆出别名）。
///
/// 容量按**字节上界**（`mem::cache_budget().config_bytes`，随机器内存分档）：
/// 单条从几十 KB 到数 MB（大模组）不等，按条目数设限无法约束真实占用；
/// 且淘汰改为 FIFO —— "满了整体 clear()" 会让下一批全部重新解析（雪崩）。
type CfgMemo =
    Mutex<crate::infra::lru::SizedCache<(PathBuf, String, u64), Arc<Vec<(String, Value)>>>>;
static CFG_MEMO: OnceLock<CfgMemo> = OnceLock::new();

/// 原始配置快照缓存：同 [`CFG_MEMO`]，值为 `serde_json::Map`。
type RawMemo =
    Mutex<crate::infra::lru::SizedCache<(PathBuf, u64), Arc<serde_json::Map<String, Value>>>>;
static RAW_MEMO: OnceLock<RawMemo> = OnceLock::new();

/// 会话级解析缓存：按内存分档取字节上界并建（首次调用时定档，之后复用）。
fn cfg_memo() -> &'static CfgMemo {
    CFG_MEMO.get_or_init(|| {
        Mutex::new(crate::infra::lru::SizedCache::new(
            crate::infra::mem::cache_budget().config_bytes,
        ))
    })
}
fn raw_memo() -> &'static RawMemo {
    RAW_MEMO.get_or_init(|| {
        Mutex::new(crate::infra::lru::SizedCache::new(
            crate::infra::mem::cache_budget().config_bytes,
        ))
    })
}

/// 释放配置解析缓存（构建全部结束后调用；见 [`crate::infra::cache_gc`]）。
///
/// 这两份是**最大的常驻持有者**：值为整份 mod 配置的 `Value` 树，按机器内存分档可达
/// 近百 MB，且 `OnceLock` 常驻到进程退出 —— 只靠 FIFO 淘汰上限控制不住"构建完仍占着"。
pub(crate) fn release_caches() {
    crate::infra::lru::clear_sized(CFG_MEMO.get());
    crate::infra::lru::clear_sized(RAW_MEMO.get());
}

/// 深度合并两个 JSON：`b` 的字段并入 `a`（对象递归合并，其余类型以 `b` 覆盖）。
///
/// 对应 VCMI 的模组配置合并语义：**同一实体可以分散在多个文件中**
/// （典型：`creatures/xxx.json` 定义名称/属性/图形，`creatures/stack_exp.json` 只补
/// `stackExperience`），必须合并而非整条覆盖，否则后者会把完整定义冲成残缺定义
/// （表现为生物名回落 id、属性全 0、无图）。
pub(crate) fn merge_json_in_place(a: &mut Value, b: &Value) {
    match (a, b) {
        (Value::Object(am), Value::Object(bm)) => {
            for (k, bv) in bm {
                match am.get_mut(k) {
                    Some(av) => merge_json_in_place(av, bv),
                    None => {
                        am.insert(k.clone(), bv.clone());
                    }
                }
            }
        }
        (a_slot, bv) => *a_slot = bv.clone(),
    }
}

/// 收集指定类型配置并按实体 id **深度合并**（同 id 多文件合并为一条）。
/// 文件按路径排序，保证合并顺序确定（跨运行稳定，避免缓存抖动）。
pub(crate) fn collect_config_jsons_merged(
    mod_path: &Path,
    type_subdir: &str,
) -> Vec<(String, Value)> {
    // 会话级缓存（见 `CFG_MEMO`）：phase=0 与 phase=1 对同一 mod 的重复解析只做一次。
    let key = (
        mod_path.to_path_buf(),
        type_subdir.to_string(),
        config_tree_fingerprint(mod_path),
    );
    if let Ok(g) = cfg_memo().lock() {
        if let Some(v) = g.get(&key) {
            return v.as_ref().clone();
        }
    }
    let v = Arc::new(collect_config_jsons_merged_uncached(mod_path, type_subdir));
    let size: usize = v.iter().map(|(k, x)| k.len() + value_weight(x)).sum();
    if let Ok(mut g) = cfg_memo().lock() {
        g.insert(key, size, v.clone());
    }
    v.as_ref().clone()
}

/// 未缓存版（真正的收集 + 合并逻辑）。
fn collect_config_jsons_merged_uncached(
    mod_path: &Path,
    type_subdir: &str,
) -> Vec<(String, Value)> {
    let mut raw = collect_config_jsons(mod_path, type_subdir);
    raw.sort_by(|x, y| x.0.cmp(&y.0));
    let mut map: HashMap<String, Value> = HashMap::new();
    for (id, v) in raw {
        match map.get_mut(&id) {
            Some(existing) => merge_json_in_place(existing, &v),
            None => {
                map.insert(id, v);
            }
        }
    }
    map.into_iter().collect()
}

/// 读取 `<mod>/mod.json` 里某类型的显式文件清单，返回**绝对路径**（相对 `content/` 解析）。
///
/// VCMI 允许 mod 元数据按类型列出配置（`"spells": [...]`、`"heroes": [...]` ...），
/// 与 `content/config/` 下的目录名无关；这是唯一能覆盖 `NewOldSpells` 这类
/// "目录名不是标准类型"的模组的办法（见 `collect_config_jsons` 的说明）。
fn mod_json_type_files(mod_path: &Path, type_subdir: &str) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for name in ["mod.json", "Mod.json"] {
        let p = mod_path.join(name);
        let Ok(text) = fs::read_to_string(&p) else {
            continue;
        };
        let Value::Object(m) = parse_vcmi_json(&text) else {
            continue;
        };
        let Some(arr) = m.get(type_subdir).and_then(|v| v.as_array()) else {
            continue;
        };
        for x in arr {
            let Some(rel) = x.as_str() else { continue };
            let rel = rel.trim_start_matches('/').replace('\\', "/");
            // VCMI 的 mod 文件系统以 `content/` 为根；大小写按磁盘实况兜底（Windows 无关，
            // 但打包/迁移到大小写敏感文件系统时会用得上）。
            for root in ["Content", "content"] {
                let f = mod_path.join(root).join(&rel);
                if f.is_file() {
                    out.push(f);
                    break;
                }
            }
        }
        break;
    }
    out
}

/// 收集 mod 配置目录下指定类型子目录的所有 JSON（含 Content 目录 + content.zip）。
/// 返回 `(对象 id, 对象根 Value)`。类型如 "creatures"/"heroes"/"spells"/"artifacts"/"skills"。
/// 每个 JSON 顶层 `{ "<id>": { ... } }`，也可能直接是对象体（无包裹），此时 id 用文件名。
pub(crate) fn collect_config_jsons(mod_path: &Path, type_subdir: &str) -> Vec<(String, Value)> {
    let mut out: Vec<(String, Value)> = Vec::new();
    let type_subdir = type_subdir.trim_matches('/');

    // 0) `mod.json` 里该类型的**显式文件清单**。
    //
    // VCMI 的 mod.json 支持按类型列出配置文件路径（`"spells": ["config/NewOldSpells/x.json"]`），
    // 路径相对于该 mod 的 `content/`，**与目录名无关**。实测两个真实模组都靠这个机制装载：
    //   · `Mods/NewOldSpells+`：法术放在 `Content/config/NewOldSpells/`（目录名不是 spells）
    //   · `Mods/tartarus`：法术放在 `Content/config/tartarus/NewOldSpells/`
    // 只按目录名扫会整批漏掉 → 百科里这些法术没有名字/图标（表现为裸 id、空格子）。
    let mut files: Vec<std::path::PathBuf> = mod_json_type_files(mod_path, type_subdir);

    // 1) Content/config/**/<type>/*.json
    let content_dir = mod_path.join("Content").join("config");
    if content_dir.is_dir() {
        let mut scanned = Vec::new();
        collect_json_files(&content_dir, type_subdir, &mut scanned);
        // 去重：显式清单与目录扫描可能指向同一个文件（不合并会重复 push 同一 id）
        let known: std::collections::HashSet<String> =
            files.iter().map(|p| p.to_string_lossy().to_lowercase()).collect();
        files.extend(
            scanned
                .into_iter()
                .filter(|p| !known.contains(&p.to_string_lossy().to_lowercase())),
        );
    }
    for f in files {
        if let Ok(text) = fs::read_to_string(&f) {
            let v = parse_vcmi_json(&text);
            push_config_object(&mut out, &f.to_string_lossy(), v, type_subdir);
        }
    }

    // 2) content.zip 内 config/**/<type>/*.json
    let zip_path = mod_path.join("content.zip");
    if let Ok(file) = fs::File::open(&zip_path) {
        if let Ok(mut archive) = zip::ZipArchive::new(file) {
            for i in 0..archive.len() {
                if let Ok(mut entry) = archive.by_index(i) {
                    let name = entry.name().to_string();
                    let nl = name.replace('\\', "/");
                    let nl_lower = nl.to_lowercase();
                    // config/<faction>/<type>/xxx.json 或 config/<type>/xxx.json；
                    // 含大小写/单复数容错与聚合文件（`<mod>/heroes.json`），
                    // 排除 `town/`（魔法行会概率表）与 translation（见 is_type_config_path）
                    let is_target = nl_lower.ends_with(".json")
                        && nl_lower.split('/').any(|s| s == "config")
                        && crate::infra::json::is_type_config_path(&nl_lower, type_subdir);
                    if is_target {
                        let mut buf = Vec::new();
                        if std::io::Read::read_to_end(&mut entry, &mut buf).is_ok() {
                            let text = String::from_utf8_lossy(&buf);
                            let v = parse_vcmi_json(&text);
                            push_config_object(&mut out, &name, v, type_subdir);
                        }
                    }
                }
            }
        }
    }
    drop_cross_mod_patches(out)
}

/// `<modId>:<id>` 形式的**跨模组纯补丁键**：键含 `:`（跨 mod 标识符语法）且
/// 条目里**没有任何实体身份/数值字段**。
///
/// VCMI 的 `<modId>:<id>` 指向**另一个 mod** 的实体
/// （`LIBRARY->identifiers()->requestIdentifier`）；VCMI 的 JSON 合并会把这种键的字段
/// 并到目标实体上，而不是新建实体。启动器的配置收集只看本 mod，够不到目标 mod 的那份
/// 定义，于是这类纯补丁被当成独立实体输出 → 无 name、无 graphics、无任何属性 →
/// `image` / `animation` 恒 null、名称回落 id，在列表里堆成一片空壳。
///
/// 实测（572 mod 发行版）两类：
/// - `66expfixmod` 系列 148 条 `{ "upgrades": [...] }`（给别的 mod 的生物加升级目标）→ 占无图生物 1433/1517；
/// - `tears-of-ashan.toa-ashan-sounds` 142 条 `{ "sound": {...} }`（给原版/mod 生物换音效）
///   → 全部 `image` 与 `animation` 为空（用户报"某些生物只有静态图片没有动态动画"，实为这些空壳）。
///
/// 判定用**实体字段白名单**而非"只允许 upgrades"：补丁的覆盖字段种类是开放的
/// （`upgrades` / `sound` / …），白名单外的新补丁字段会漏；而实体身份字段是封闭的
/// —— 任何真实生物定义都必然带 `name` / `graphics` / `level` 等之一。
/// 不带冒号的正常定义（含建筑的 `{name, description, upgrades, cost}`）一律不碰。
fn is_cross_mod_patch_only(id: &str, v: &Value) -> bool {
    if !id.contains(':') {
        return false;
    }
    let Some(m) = v.as_object() else {
        return false;
    };
    if m.is_empty() {
        return true;
    }
    // 命中任一「实体身份/数值字段」即认定为真正的实体定义，保留。
    // 取 VCMI creature schema 的核心字段：任何真实生物定义都至少有 `name`（或多语言名）
    // 与 `graphics` / `level` 等之一；而纯补丁只带单点覆盖字段
    // （`upgrades` 加升级目标、`sound` 换音效、`abilities` 加能力）。
    const ENTITY_FIELDS: &[&str] = &[
        "name",
        "graphics",
        "level",
        "attack",
        "defense",
        "damage",
        "hitPoints",
        "speed",
        "growth",
        "aiValue",
        "fightValue",
        "cost",
        "faction",
        "doubleWide",
        "advMapAmount",
        "horde",
    ];
    !ENTITY_FIELDS.iter().any(|k| m.contains_key(*k))
}

/// 收集出口：丢掉跨模组纯补丁键（见 [`is_cross_mod_patch_only`]）。
///
/// 放在**收集完**而不是各 `build_*` 里：`build_creatures` 走 `collect_config_jsons`（未合并版）
/// 拿 `creature_jsons`，`build_*` 其余走 `collect_config_jsons_merged`，两个入口都要生效。
fn drop_cross_mod_patches(mut out: Vec<(String, Value)>) -> Vec<(String, Value)> {
    out.retain(|(id, v)| !is_cross_mod_patch_only(id, v));
    out
}

/// 全量收集 mod 的所有 JSON 配置并做**多文件归一**（保底数据层，写入独立 raw.json）。
/// 归一规则（完全兼容原始数据接口，仅聚合、不改写原始字段）：
/// - 已知类型（路径含 config/<type>/）：`{ "<type>": { "<id>": 原始对象 } }`，同类型多文件按键合并
///   类型：creatures/heroes/heroClasses/towns(town|towns|faction|factions)/spells/artifacts/skills/
///        terrains/rivers/roads/objects/obstacles/battlefields/resources/abilities/bonuses
/// - translation/*.json：`{ "translation": { "<文件名>": 原始内容 } }`
/// - 根 mod.json / Content/mod.json：`{ "mod": { ...原始字段 } }`
/// - 其余无法归类：`{ "files": { "<相对路径>": 原始内容 } }` 兜底
pub(crate) fn collect_all_raw_configs(mod_path: &Path) -> serde_json::Map<String, Value> {
    // 会话级缓存（见 `RAW_MEMO`）：raw.json 的解析同样在 phase=0 / phase=1 各做一遍。
    let key = (mod_path.to_path_buf(), config_tree_fingerprint(mod_path));
    if let Ok(g) = raw_memo().lock() {
        if let Some(v) = g.get(&key) {
            return v.as_ref().clone();
        }
    }
    let v = Arc::new(collect_all_raw_configs_uncached(mod_path));
    let size: usize = v.iter().map(|(k, x)| k.len() + value_weight(x)).sum();
    if let Ok(mut g) = raw_memo().lock() {
        g.insert(key, size, v.clone());
    }
    v.as_ref().clone()
}

/// 该 JSON 是否其实是**英雄职业定义**（而不是英雄）。
///
/// 为什么需要：不少 H3 时代 mod 把职业定义写在 `heroes/` 目录里（`moonspire` 的
/// `moonPriest.json`、`10-blazers--town` 的 `honorWarrior.json`），只按目录回退分类会把它们
/// 当英雄读 → 城镇页多出"无头像、无职业、特长只有一个 ✦"的幽灵行；而 VCMI 的百科城镇页
/// 恰恰会跳过 `heroClass` 为空的英雄（`WikiTownContent.cpp:455`）。
///
/// 判据：**有职业特征**（`affinity` / `lowLevelChance` / `tavern` / `commander` / 技能表…）
/// **且完全没有英雄特征**（`army` / `texts` / `images` / `class` / `biography`）。
/// 后者兜住写得精简的合法英雄文件（VCMI 的 hero schema 必填 `class`+`army`+`skills`+`texts`）。
pub(crate) fn looks_like_hero_class(v: &Value) -> bool {
    const HERO_MARKS: &[&str] = &["army", "startingArmy", "texts", "images", "class", "biography"];
    const CLASS_MARKS: &[&str] = &[
        "affinity",
        "lowLevelChance",
        "tavern",
        "commander",
        "primarySkills",
        "secondarySkills",
    ];
    !HERO_MARKS.iter().any(|k| v.get(*k).is_some())
        && CLASS_MARKS.iter().any(|k| v.get(*k).is_some())
}

/// 未缓存版（真正的收集 + 归一逻辑）。
fn collect_all_raw_configs_uncached(mod_path: &Path) -> serde_json::Map<String, Value> {
    // 先收集 (归一类型 or None, 原始 Value, 相对路径)
    enum Slot {
        Type(&'static str),
        Translation(String),
        Mod,
        File(String),
    }
    let mut items: Vec<(Slot, Value)> = Vec::new();

    fn known_type(rel_lower: &str) -> Option<&'static str> {
        const TYPES: &[&str] = &[
            "creatures",
            "heroes",
            "heroclasses",
            "spells",
            "artifacts",
            "skills",
            "terrains",
            "rivers",
            "roads",
            "objects",
            "obstacles",
            "battlefields",
            "resources",
            "abilities",
            "bonuses",
        ];
        let norm = |t: &'static str| match t {
            "heroclasses" => "heroClasses",
            other => other,
        };
        // 1) **文件名优先**：`config/terrain/obstacles.json` 归 obstacles 而不是 terrains
        //    （否则障碍物/河流/道路定义会混进地形池）；`terrains.json` / `rivers.json` 同理。
        let fname = rel_lower
            .rsplit('/')
            .next()
            .unwrap_or("")
            .trim_end_matches(".json");
        for &t in TYPES {
            if crate::infra::json::type_seg_match(fname, t) {
                return Some(norm(t));
            }
        }
        // 2) 目录段回退：**段优先、从路径末段往前**（越靠后越具体）——
        //    容错大小写、单复数与 `<前缀>-<类型>` 变体（`artifact/`、`Creatures/`、`Abode-creatures/`）。
        //    必须"段优先"而非"类型优先"：mod 目录名可能以类型名收尾（`creature-heroes/`），
        //    类型优先会让 `config/creature-heroes/skills/x.json` 被判成 heroes（实测回归）。
        let segs: Vec<&str> = rel_lower.split('/').filter(|s| !s.is_empty()).collect();
        for seg in segs.iter().rev() {
            for &t in TYPES {
                if crate::infra::json::type_seg_match(seg, t) {
                    return Some(norm(t));
                }
            }
        }
        // HuaXia 等把职业配置放 config/<mod>/classes/（分段 "classes"）
        if rel_lower.split('/').any(|s| s == "classes") {
            return Some("heroClasses");
        }
        // 城镇/阵营配置：town|towns|faction|factions 归入 towns（兼容 town.json 这类带扩展名的文件名）
        if rel_lower.split('/').any(|s| {
            s.starts_with("town") || s.starts_with("faction") || matches!(s, "towns" | "factions")
        }) {
            return Some("towns");
        }
        None
    }

    /// 归一化「mod.json 里写的相对路径」：小写、正斜杠、去掉开头 `Content/`。
    ///
    /// mod.json 的内容路径以 **Content/** 为基准（VCMI 规则），而磁盘上目录名可能是
    /// 小写 `content/`（实测 `moonspire` 就是），故统一按「相对 Content 的小写路径」比较。
    fn norm_rel(rel: &str) -> String {
        let s = rel.trim().replace('\\', "/").to_lowercase();
        let s = s.strip_prefix("./").unwrap_or(&s).to_string();
        let s = s.trim_start_matches('/').to_string();
        s.strip_prefix("content/").unwrap_or(&s).to_string()
    }
    // 已收集的配置文件：`相对 Content 的小写路径` → 原始 JSON。
    // 用于把 mod.json 声明的内容清单**映射回实际文件**（磁盘与 content.zip 都覆盖）。
    let mut by_rel: std::collections::HashMap<String, Value> = std::collections::HashMap::new();

    let mut collect = |rel: String, v: Value| {
        let rel_lower = rel.to_lowercase();
        by_rel.insert(norm_rel(&rel), v.clone());
        let slot = if rel == "mod.json" || rel == "Content/mod.json" {
            Slot::Mod
        } else if let Some(t) = known_type(&rel_lower) {
            Slot::Type(t)
        } else if rel_lower.contains("translation") {
            let stem = rel
                .rsplit('/')
                .next()
                .unwrap_or(&rel)
                .trim_end_matches(".json")
                .to_string();
            Slot::Translation(stem)
        } else {
            // 兜底：内容嗅探 town 配置（HuaXia 等路径不含 town/faction 分段，如 config/<mod>/xxx.json）
            // 同时检查对象自身与一级子对象（兼容平级结构 {town, buildings, clientInfo} 与包裹结构 {<fid>: {...}}）
            let has_town_marks = |x: &Value| {
                x.get("clientInfo").is_some()
                    || x.get("townScreen").is_some()
                    || x.get("buildings").is_some()
            };
            let looks_town = has_town_marks(&v)
                || v.as_object()
                    .map(|obj| obj.values().any(|val| has_town_marks(val)))
                    .unwrap_or(false);
            if looks_town {
                items.push((Slot::Type("towns"), v));
                return;
            }
            items.push((Slot::File(rel.clone()), v));
            return;
        };
        items.push((slot, v));
    };

    // 1) 根 mod.json / Content/mod.json
    for rel in ["mod.json", "Content/mod.json"] {
        let fp = mod_path.join(rel);
        if fp.is_file() {
            if let Ok(text) = fs::read_to_string(&fp) {
                collect(rel.to_string(), parse_vcmi_json(&text));
            }
        }
    }
    // 2) Content/config/**/*.json（磁盘）
    let cfg_dir = mod_path.join("Content").join("config");
    let mut files = Vec::new();
    fn walk(dir: &Path, out_files: &mut Vec<PathBuf>) {
        if let Ok(entries) = fs::read_dir(dir) {
            for e in entries.flatten() {
                let fp = e.path();
                if fp.is_dir() {
                    walk(&fp, out_files);
                } else if fp
                    .extension()
                    .and_then(|x| x.to_str())
                    .map(|x| x.eq_ignore_ascii_case("json"))
                    .unwrap_or(false)
                {
                    out_files.push(fp);
                }
            }
        }
    }
    if cfg_dir.is_dir() {
        walk(&cfg_dir, &mut files);
        for f in files {
            let rel = f
                .strip_prefix(mod_path.join("Content"))
                .unwrap_or(&f)
                .to_string_lossy()
                .replace('\\', "/");
            if let Ok(text) = fs::read_to_string(&f) {
                collect(rel, parse_vcmi_json(&text));
            }
        }
    }
    // 3) content.zip 内全部 .json
    let zip_path = mod_path.join("content.zip");
    if let Ok(file) = fs::File::open(&zip_path) {
        if let Ok(mut archive) = zip::ZipArchive::new(file) {
            for i in 0..archive.len() {
                if let Ok(mut entry) = archive.by_index(i) {
                    let name = entry.name().to_string();
                    let nl = name.replace('\\', "/");
                    if !nl.to_lowercase().ends_with(".json") || nl.contains("__MACOSX") {
                        continue;
                    }
                    let mut buf = Vec::new();
                    if std::io::Read::read_to_end(&mut entry, &mut buf).is_ok() {
                        let text = String::from_utf8_lossy(&buf);
                        collect(nl, parse_vcmi_json(&text));
                    }
                }
            }
        }
    }

    // 2.5) **mod.json 声明的 `heroClasses` 清单优先**（内容类型以声明为准，与所在目录无关）。
    //
    // VCMI 只加载 mod.json 里列出的文件，且按**声明的内容类型**解析：实测本发行版
    // `moonspire` 把 2 个职业定义放在 `config/moonspire/heroes/` 目录下、却在 mod.json 的
    // `heroClasses` 键里声明 —— 目录回退把它们当**英雄**读，造成：
    //   ① 多出幽灵英雄（`moonPriest` / `groveKeeper` / `daoSage` / `kungFuMaster`：无头像、
    //      无职业，正是城镇页那 4 行空图标）；
    //   ② 真正的职业表为空 → 17 个英雄的 `className` 回落成 id（城镇页分组标题显示
    //      "MOONPRIEST // GROVEKEEPER"）、`affinity` 为空（力量/魔法分组失效）。
    // 这里把**声明为职业的文件**补进职业桶（幽灵英雄的剔除由下方 2.6 的形状嗅探统一负责）。
    // 只做这一步（不改动其它类型的目录回退语义），把爆炸半径限制在已取证的场景上。
    let mut declared_classes: Vec<Value> = Vec::new();
    for mj in [mod_path.join("mod.json"), mod_path.join("Content").join("mod.json")] {
        let Ok(text) = fs::read_to_string(&mj) else {
            continue;
        };
        let mv = parse_vcmi_json(&text);
        let Some(arr) = mv.get("heroClasses").and_then(|x| x.as_array()) else {
            continue;
        };
        for rel in arr.iter().filter_map(|x| x.as_str()) {
            if let Some(cv) = by_rel.get(&norm_rel(rel)) {
                declared_classes.push(cv.clone());
            }
        }
    }
    for cv in declared_classes {
        items.push((Slot::Type("heroClasses"), cv));
    }

    // 归一合并（深合并：VCMI 常把 town 节点拆到多个文件，如 structures.json/faction.json，
    // 浅覆盖会丢失兄弟字段——如先并入 town.buildings 再并入 town.clientInfo 会互删）
    fn deep_merge(dst: &mut serde_json::Map<String, Value>, src: &serde_json::Map<String, Value>) {
        for (k, val) in src {
            match dst.get_mut(k) {
                Some(Value::Object(d)) if val.is_object() => {
                    if let Some(s) = val.as_object() {
                        deep_merge(d, s);
                    }
                }
                _ => {
                    dst.insert(k.clone(), val.clone());
                }
            }
        }
    }
    let mut out = serde_json::Map::new();
    for (slot, v) in items {
        match slot {
            Slot::Mod => {
                let dst = out.entry("mod".to_string()).or_insert_with(|| json!({}));
                if let (Some(d), Some(s)) = (dst.as_object_mut(), v.as_object()) {
                    deep_merge(d, s);
                }
            }
            Slot::Type(t) => {
                let type_map = out.entry(t.to_string()).or_insert_with(|| json!({}));
                if let Some(d) = type_map.as_object_mut() {
                    if v.is_object() {
                        // 类型下按实体 id 深合并（同 id 跨文件补全字段）
                        if let Some(s) = v.as_object() {
                            for (eid, ev) in s {
                                match d.get_mut(eid) {
                                    Some(Value::Object(dd)) if ev.is_object() => {
                                        if let Some(s2) = ev.as_object() {
                                            deep_merge(dd, s2);
                                        }
                                    }
                                    _ => {
                                        d.insert(eid.clone(), ev.clone());
                                    }
                                }
                            }
                        }
                    } else {
                        d.insert("__raw__".to_string(), v);
                    }
                }
            }
            Slot::Translation(stem) => {
                let tr = out
                    .entry("translation".to_string())
                    .or_insert_with(|| json!({}));
                if let Some(d) = tr.as_object_mut() {
                    d.insert(stem, v);
                }
            }
            Slot::File(rel) => {
                let fl = out.entry("files".to_string()).or_insert_with(|| json!({}));
                if let Some(d) = fl.as_object_mut() {
                    d.insert(rel, v);
                }
            }
        }
    }
    // 2.6) **英雄桶里的职业定义不是英雄**（内容嗅探，兜住未在 mod.json 里声明的残留文件）。
    //
    // 依据：VCMI 百科的城镇页会跳过 `heroClass` 为空的英雄（`WikiTownContent.cpp:455`
    // `if(!h || h->special || !h->heroClass) continue;`），而这类文件按**职业**写就
    // （`affinity` / `levels` / `primarySkills` / `lowLevelChance` …，没有 `army`/`texts`/`images`），
    // 落在英雄桶里就是"无头像、无职业、特长只有一个 ✦"的幽灵行（截图里的武将/谋士/月之祭司/丛林守护者）。
    // 既然它们确是职业定义，就搬进职业桶 —— 顺带让引用了它们的英雄能把职业名解析出来。
    //
    // 判据要求"有职业特征 **且** 完全没有英雄特征"，避免误伤写得精简的合法英雄文件。
    let looks_like_class = looks_like_hero_class;
    let moved: Vec<(String, Value)> = out
        .get("heroes")
        .and_then(|x| x.as_object())
        .map(|heroes| {
            heroes
                .iter()
                .filter(|(_, v)| looks_like_class(v))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
        .unwrap_or_default();
    if !moved.is_empty() {
        if let Some(heroes) = out.get_mut("heroes").and_then(|x| x.as_object_mut()) {
            for (id, _) in &moved {
                heroes.remove(id);
            }
        }
        let classes = out
            .entry("heroClasses".to_string())
            .or_insert_with(|| json!({}));
        if let Some(cm) = classes.as_object_mut() {
            for (id, cv) in moved {
                cm.entry(id).or_insert(cv);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// 造一个"职业定义写在 `heroes/` 目录里"的 mod（实测 `moonspire` / `10-blazers--town` 的写法）。
    fn write_mod(root: &Path, mod_json: &str, files: &[(&str, &str)]) {
        fs::create_dir_all(root.join("Content/config/x/heroes")).unwrap();
        fs::write(root.join("mod.json"), mod_json).unwrap();
        for (rel, body) in files {
            let p = root.join("Content").join(rel);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(p, body).unwrap();
        }
    }

    /// 职业定义必须进职业桶、且**不得**以英雄身份出现（内容类型以 mod.json 声明为准，
    /// 与所在目录无关）。回归背景：城镇页出现 4 行"无头像、无职业"的幽灵英雄，
    /// 同时真英雄的 `className` 回落成 id（分组标题显示 MOONPRIEST // GROVEKEEPER）。
    #[test]
    fn declared_hero_classes_move_out_of_heroes_bucket() {
        let root = std::env::temp_dir().join("vcmi_fv_collect_decl_test_declared");
        let _ = fs::remove_dir_all(&root);
        write_mod(
            &root,
            r#"{"name":"x","heroClasses":["config/x/heroes/moonPriest.json","config/x/heroes/groveKeeper.json"],
                "heroes":["config/x/heroes/hero1.json"]}"#,
            &[
                (
                    "config/x/heroes/moonPriest.json",
                    r#"{"moonPriest":{"name":"月之祭司","affinity":"magic","faction":"moon",
                        "primarySkills":{"attack":0,"defence":0,"spellpower":2,"knowledge":3}}}"#,
                ),
                (
                    "config/x/heroes/groveKeeper.json",
                    r#"{"groveKeeper":{"name":"丛林守护者","affinity":"might","faction":"moon"}}"#,
                ),
                (
                    "config/x/heroes/hero1.json",
                    r#"{"hero1":{"name":"英雄一","class":"moonPriest","army":[{"creature":"pikeman"}],
                        "texts":{"name":"英雄一"}}}"#,
                ),
            ],
        );
        let raw = collect_all_raw_configs(&root);
        let classes = raw.get("heroClasses").and_then(|x| x.as_object()).unwrap();
        assert!(
            classes.contains_key("moonPriest") && classes.contains_key("groveKeeper"),
            "声明的职业必须进职业桶：{classes:?}"
        );
        let heroes = raw.get("heroes").and_then(|x| x.as_object()).unwrap();
        assert!(heroes.contains_key("hero1"), "真英雄必须保留");
        assert!(
            !heroes.contains_key("moonPriest") && !heroes.contains_key("groveKeeper"),
            "职业文件不得再作为英雄出现：{heroes:?}"
        );
        let _ = fs::remove_dir_all(&root);
    }

    /// **未声明**的残留职业文件（`daoSage.json` / `kungFuMaster.json` 就是这种）同样不能算英雄：
    /// 它们没有任何英雄特征（army/texts/images/class），却有 affinity/primarySkills。
    #[test]
    fn class_shaped_leftover_is_not_a_hero() {
        let root = std::env::temp_dir().join("vcmi_fv_collect_decl_test_leftover");
        let _ = fs::remove_dir_all(&root);
        write_mod(
            &root,
            r#"{"name":"x","heroes":["config/x/heroes/hero1.json"]}"#,
            &[
                (
                    "config/x/heroes/daoSage.json",
                    r#"{"daoSage":{"name":"谋士","affinity":"might","lowLevelChance":5,"tavern":true}}"#,
                ),
                (
                    "config/x/heroes/hero1.json",
                    r#"{"hero1":{"name":"英雄一","class":"daoSage","texts":{"name":"英雄一"}}}"#,
                ),
            ],
        );
        let raw = collect_all_raw_configs(&root);
        let heroes = raw.get("heroes").and_then(|x| x.as_object()).unwrap();
        assert!(heroes.contains_key("hero1"));
        assert!(!heroes.contains_key("daoSage"), "职业形状的残留文件不得算英雄：{heroes:?}");
        // 顺手搬进职业桶，让引用了它的英雄能解析出职业名
        let classes = raw.get("heroClasses").and_then(|x| x.as_object()).unwrap();
        assert!(classes.contains_key("daoSage"));
        let _ = fs::remove_dir_all(&root);
    }
}

/// 递归累加升级链完整招募成本（对齐 VCMI getFullRecruitCost）：
/// full = 自身 cost + 所有升级来源（沿 upgradedFrom 链）cost 之和。
pub(crate) fn accumulate_full_cost(
    creature_jsons: &std::collections::HashMap<String, Value>,
    upgraded_from: &std::collections::HashMap<String, Vec<String>>,
    id: &str,
    seen: &mut std::collections::HashSet<String>,
) -> Value {
    let mut result = serde_json::Map::new();
    if !seen.insert(id.to_string()) {
        return Value::Object(result); // 防环
    }
    let add_cost = |dst: &mut serde_json::Map<String, Value>, c: &Value| {
        if let Some(cost) = c.get("cost").and_then(|x| x.as_object()) {
            for (k, v) in cost {
                let n = v.as_i64().unwrap_or(0);
                if n != 0 {
                    let cur = dst.get(k).and_then(|x| x.as_i64()).unwrap_or(0);
                    dst.insert(k.clone(), json!(cur + n));
                }
            }
        }
    };
    if let Some(obj) = creature_jsons.get(id) {
        add_cost(&mut result, obj);
    }
    if let Some(parents) = upgraded_from.get(id) {
        for pid in parents {
            let sub = accumulate_full_cost(creature_jsons, upgraded_from, pid, seen);
            if let Some(m) = sub.as_object() {
                for (k, v) in m {
                    let cur = result.get(k).and_then(|x| x.as_i64()).unwrap_or(0);
                    result.insert(k.clone(), json!(cur + v.as_i64().unwrap_or(0)));
                }
            }
        }
    }
    seen.remove(id);
    Value::Object(result)
}
