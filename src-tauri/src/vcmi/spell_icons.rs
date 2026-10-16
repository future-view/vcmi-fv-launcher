//! 法术自身的图标表：`spells` 配置的 `graphics.iconImmune`（`SPELL_IMMUNITY` 特例）。
//!
//! ## 为什么需要
//!
//! VCMI 的 `bonusToGraphics` 有一处**先于**常规链的硬编码特例
//! （`lib/CBonusTypeHandler.cpp:105-122`，原文照录）：
//!
//! ```cpp
//! ImagePath CBonusTypeHandler::bonusToGraphics(const std::shared_ptr<Bonus> & bonus) const
//! {
//! 	const CBonusType & bt = *bonusTypes.at(vstd::to_underlying(bonus->type));
//!
//! 	if (bonus->type == BonusType::SPELL_IMMUNITY && bonus->subtype.as<SpellID>().hasValue())
//! 	{
//! 		const CSpell * sp = bonus->subtype.as<SpellID>().toSpell();
//! 		return sp->getIconImmune();
//! 	}
//!
//! 	if (bt.subtypeIcons.count(bonus->subtype.getNum()))
//! 		return bt.subtypeIcons.at(bonus->subtype.getNum());
//! 	...
//! ```
//!
//! 即「免疫某法术」这类能力的图标 = **被免疫法术自己**的 `graphics.iconImmune`
//! （`lib/spells/CSpellHandler.cpp:379` 读入），而不是 `bonuses.json` 里 `SPELL_IMMUNITY`
//! 的图形段。实测 `vcmi-extras` 的 Bonus Icons 子模组正是这么写的：
//!
//! ```json
//! "core:SPELL_IMMUNITY" : { "graphics" : { // Uses icon from spell config
//! } },
//! ```
//!
//! 于是「能力图标只做 `bonuses.json` 常规链」时，所有 `SPELL_IMMUNITY` 能力**必然无图**
//! （图标不在 bonuses.json 里，而在法术配置里）。
//!
//! ## 数据在哪
//!
//! 原版法术本身（SPTRAITS.TXT legacy，无 JSON）没有 `iconImmune`，图标由
//! `Mods/vcmi-extras/Mods/bonusIcons/mods/Immunity Icons` 子模组补上。它的两个文件
//! **不在 `Content/config/` 下**，而是放在 `Content/` 根，靠 `mod.json` 的清单声明：
//!
//! ```json
//! "spells" : [ "spellImmunityIcons.json", "abilityImmunityIcons.json" ]
//! ```
//!
//! （`ContentTypeHandler::preloadModData` 读的就是 `mod.getLocalValue("spells")`，
//! 路径相对模组的 `Content/` 根；键写成 `core:blind` 形式表示**补丁**到 core 的那个法术上，
//! 见同文件 59-88 与 103-113 的 `modData[scope].patches`。）多数模组则把附加法术文件放在
//! `Content/config/<自己的目录>/xxx.json`（如 PurpleDragon 的 `abilityandspellsiImunityIcons.json`）
//! ——同样是**任意文件名**，只有 `mod.json` 声明能定位。
//!
//! 这与 [`crate::vcmi::bonus`] 的表是**同一条链的两段**：那里按 bonus type 查
//! `subtypeIcons → valueIcons → icon`，这里补上先执行的 `SPELL_IMMUNITY` 特例。
//!
//! ## 键
//!
//! 键统一取**末段**：`core:blind` / `spell.blind` / `blind` 都归到 `blind`
//! （法术 id 是全局的；`modcache` 侧的 `core_spellbon_image` 用同一口径取 SPELLBON 帧号）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::Value;

/// 法术 id 末段 → `graphics.iconImmune` 资源路径。
type IconMap = HashMap<String, String>;

/// 缓存：键为「游戏目录 + 模组集合大小」，与 [`crate::vcmi::bonus`] 同口径。
///
/// 这是热路径（每个生物的每条 `SPELL_IMMUNITY` 能力都会查一次），键必须廉价——
/// 详见 `bonus.rs` 中 `bonus_graphics_map` 的性能注记（算键本身不能变成一次目录遍历）。
static SPELL_ICON_CACHE: OnceLock<Mutex<Vec<(String, Arc<IconMap>)>>> = OnceLock::new();

/// 跨模组标识符的末段：`core:blind` / `spell.blind` / `blind` → `blind`。
pub(crate) fn id_leaf(id: &str) -> String {
    id.rsplit([':', '.']).next().unwrap_or(id).to_string()
}

/// 全部法术的 `graphics.iconImmune`（跨全部模组 + 游戏根合并）。
///
/// 合并口径：`or_insert` 保留**首个**非空值（与 [`crate::vcmi::bonus::bonus_graphics_map`] 一致）。
pub(crate) fn spell_immune_icons(game_dir: &Path) -> Arc<IconMap> {
    let key = format!(
        "{}:{}",
        game_dir.display(),
        crate::vcmi::resource::wiki_mod_paths()
            .map(|v| v.len())
            .unwrap_or(0)
    );
    crate::infra::cache::cfg_cache_get(&SPELL_ICON_CACHE, key, || {
        let mut map: IconMap = HashMap::new();
        for root in crate::vcmi::bonus::config_scan_roots(game_dir) {
            collect_root(&root, &mut map);
        }
        map
    })
}

/// 单个法术的 `iconImmune`：`subtype` 可为 `spell.blind` / `core:blind` / `blind`。
pub(crate) fn spell_immune_icon(game_dir: &Path, subtype: &str) -> Option<String> {
    let leaf = id_leaf(subtype);
    if leaf.is_empty() {
        return None;
    }
    spell_immune_icons(game_dir).get(&leaf).cloned()
}

/// 释放图标表缓存（构建全部结束后调用；见 [`crate::infra::cache_gc`]）。
pub(crate) fn release_caches() {
    crate::infra::cache::cfg_cache_clear(&SPELL_ICON_CACHE);
}

/// 收一个扫描根的 `iconImmune`。
///
/// [`crate::vcmi::bonus::config_scan_roots`] 返回的是两类形态：游戏根 `config`（本身就是
/// 配置目录）与模组根（配置在 `Content/config/` 下），这里按有无 `Content/` 分流。
fn collect_root(root: &Path, map: &mut IconMap) {
    let Some(content) = crate::vcmi::bonus::content_dir_of(root) else {
        // 游戏根 `config`：法术在 `config/spells/**`
        let files = spell_config_files(root);
        for f in &files {
            merge_spell_file(f, map);
        }
        return;
    };
    // 模组（含 `Mods/vcmi`）：常规法术配置在 `Content/config/**/spells/**`。
    //
    // 走 [`crate::infra::json::collect_json_files`] 而不是自己递归：口径与本项目其余
    // 配置收集完全一致（大小写 / 单复数 / `<前缀>-spells` 目录容错、聚合文件
    // `config/spells.json`、排除 `town/spells.json` 的魔法行会概率表），且复用
    // `infra::dirscan` 的目录清单缓存，零额外 IO。
    let mut files = Vec::new();
    let cfg = content.join("config");
    if cfg.is_dir() {
        files.extend(spell_config_files(&cfg));
    }
    // `mod.json` 声明的附加 `spells` 文件：这些**不在** `config/**/spells/` 下
    // （`Content/` 根、或 `config/<任意名>/<任意名>.json`），只有声明清单能找到。
    files.extend(declared_spell_files(root, &content));
    // 排序 + 去重：同一文件可能既命中聚合规则又被 mod.json 声明（如 `config/spells.json`）；
    // 排序让「首个非空值胜出」的合并顺序跨运行稳定（与 `collect_config_jsons_merged` 同做法）。
    files.sort();
    files.dedup();
    for f in &files {
        merge_spell_file(f, map);
    }
}

/// `config/**/spells/**` 下的法术配置文件。
fn spell_config_files(cfg_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    crate::infra::json::collect_json_files(cfg_dir, "spells", &mut files);
    files
}

/// 读 `mod.json` 的 `"spells"` 清单并解析成实际文件。
///
/// VCMI 侧的同款逻辑是 `ContentTypeHandler::preloadData` →
/// `mod.getLocalValue("spells")`（`lib/modding/ContentTypeHandler.cpp:279-282`），
/// 每项都是**相对 `Content/` 根**的路径，经 `JsonPath::builtinTODO` 定位。
fn declared_spell_files(mod_root: &Path, content: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    // VCMI 的 `getLocalConfig()` 是 `<mod>/mod.json`；个别打包方式把它放进 `Content/`。
    let root_mj = mod_root.join("mod.json");
    let mj = if root_mj.is_file() {
        root_mj
    } else {
        let c = content.join("mod.json");
        if !c.is_file() {
            return out;
        }
        c
    };
    let v = crate::infra::json::read_vcmi_json(&mj);
    let Some(list) = v.get("spells").and_then(|x| x.as_array()) else {
        return out;
    };
    for e in list {
        let Some(rel) = e.as_str() else {
            continue;
        };
        let rel = rel.replace('\\', "/");
        let p = content.join(rel.trim_start_matches('/'));
        if p.is_file() {
            out.push(p);
            continue;
        }
        // 实测声明常省略扩展名（`Mods/vcmi` 的 `"config/spells"` → `config/spells.json`）
        let pj = content.join(format!("{}.json", rel.trim_start_matches('/')));
        if pj.is_file() {
            out.push(pj);
            continue;
        }
        // 目录形态：整棵收（裸 `read_dir` 递归，只在这条少数分支上发生）
        if p.is_dir() {
            crate::infra::json::collect_all_json_files(&p, &mut out);
        }
    }
    out
}

/// 把一份法术 JSON 的 `graphics.iconImmune` 并入表。
///
/// 顶层是 `{ "<id>": { ... } }`（id 可带模组作用域前缀，如 `core:blind`）：这是 VCMI
/// 唯一认得的形态——`preloadModData` 正是按顶层键拆成"本模组对象 / 补丁到别的模组"。
fn merge_spell_file(path: &Path, map: &mut IconMap) {
    let v = crate::infra::json::read_vcmi_json(path);
    let Some(obj) = v.as_object() else {
        return;
    };
    for (id, body) in obj {
        if let Some(res) = immune_of(body) {
            map.entry(id_leaf(id)).or_insert(res);
        }
    }
}

/// 取一份法术配置的 `graphics.iconImmune`（空值返回 `None`）。
fn immune_of(spell: &Value) -> Option<String> {
    crate::vcmi::bonus::res_of(spell.pointer("/graphics/iconImmune"))
}
