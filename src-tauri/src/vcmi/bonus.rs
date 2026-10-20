//! bonus type 的图标表：`bonuses.json` 的 `graphics` 段。
//!
//! ## 为什么需要
//!
//! VCMI 百科生物页的能力图标是**两级**来源
//! （`client/windows/wiki/WikiCreatureContent.cpp:307-309`）：
//!
//! ```cpp
//! const ImagePath icon = !b->customIconPath.empty()
//!     ? b->customIconPath                      // ① 生物配置里 ability["icon"]
//!     : LIBRARY->bth->bonusToGraphics(b);      // ② 按 bonus type 查 bonuses.json
//! ```
//!
//! ② 的实现见 `lib/CBonusTypeHandler.cpp:105-122`：按
//! `SPELL_IMMUNITY 特例 → subtypeIcons[subtype] → valueIcons[val] → icon` 四级取。
//! 数据由 `CBonusType::load`（同文件 165-183）从**各 mod 的 `bonuses.json`** 读入，
//! 键形如 `core:ADDITIONAL_ATTACK` / `<mod>:<TYPE>`（跨 mod 标识符语法）。
//!
//! 实测（572 mod 发行版）：提供图标的正是 `vcmi-extras` 的 **Bonus Icons** 子 mod
//! （`Mods/vcmi-extras/Mods/bonusIcons/mods/Bonus Icons/Content/config/bonusIcons/bonuses.json`，
//! 为每个 `core:<TYPE>` 定义 `graphics.icon`）。
//! 启动器此前只实现了 ① —— 而绝大多数能力**不写** `icon`（那是作者给单个生物定制时才用），
//! 于是百科能力表**所有图标恒为空**（用户报"所有技能图片均为空图"）。
//!
//! ## 键与子类型
//!
//! - type 键统一**取末段并转大写**：`core:ADDITIONAL_ATTACK` 与
//!   `<mod>:ADDITIONAL_ATTACK` 都归到 `ADDITIONAL_ATTACK`（bonus type 名是全局的）。
//! - VCMI 用 `requestIdentifier` 把 `subtypeIcons` 的键解析成数字标识符；启动器没有
//!   标识符表，故按**名字字符串**匹配配置里的 `subtype`（取末段，如 `water` / `fire`）。
//!   这覆盖绝大多数写法；解析不到时自然落到 `valueIcons` / `icon`。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::Value;

/// 单个 bonus type 的图标候选（对齐 VCMI `CBonusType` 的 `icon`/`subtypeIcons`/`valueIcons`）。
#[derive(Debug, Default, Clone)]
pub(crate) struct BonusGraphics {
    /// 通用图标（`graphics.icon`）
    pub(crate) icon: Option<String>,
    /// 子类型名（末段，如 `water`）→ 图标资源
    pub(crate) subtype_icons: HashMap<String, String>,
    /// `val` → 图标资源
    pub(crate) value_icons: HashMap<i64, String>,
}

impl BonusGraphics {
    /// 按 VCMI `bonusToGraphics` 的优先级取图标资源：
    /// `subtypeIcons[subtype] → valueIcons[val] → icon`。
    ///
    /// ⚠️ VCMI 的 `SPELL_IMMUNITY → spell->getIconImmune()` 特例**先于**本链执行
    /// （`CBonusTypeHandler.cpp:109-113`，命中即 return，不回落）。它由调用方在查表前处理：
    /// 见 [`crate::vcmi::spell_icons`]——本函数既拿不到游戏目录，也不知道 subtype 是哪个法术。
    pub(crate) fn resolve(&self, subtype: &str, val: i64) -> Option<&String> {
        if !subtype.is_empty() {
            if let Some(p) = self.subtype_icons.get(subtype) {
                return Some(p);
            }
        }
        if let Some(p) = self.value_icons.get(&val) {
            return Some(p);
        }
        self.icon.as_ref()
    }
}

type BonusMap = HashMap<String, BonusGraphics>;

/// 缓存：键为「模组集合指纹」，避免同一批次里重复扫 570+ 个目录。
static BONUS_GRAPHICS_CACHE: OnceLock<Mutex<Vec<(String, Arc<BonusMap>)>>> = OnceLock::new();

/// 全部 bonus type 的图标表（跨全部模组 + 游戏根合并）。
///
/// 合并口径：`or_insert` 保留**首个**非空值（与 `read_town_configs` 的字段级合并一致）。
/// 原版 `config/bonuses.json` 通常只有 `hidden` / `description`（实测 0 个 `graphics`），
/// 真正的图标来自 `vcmi-extras` 的 Bonus Icons 等模组。
pub(crate) fn bonus_graphics_map(game_dir: &Path) -> Arc<BonusMap> {
    // ⚠️ 缓存键必须**廉价**——这是热路径：每个"能力图标二级回退"都会调一次
    // （实测 5591 个生物，每个若干能力 = 数千~数万次）。
    //
    // 曾经用 `bonus_scan_dirs(game_dir).len()` 当键，而**算 dirs 本身**要遍历全部已装
    // 模组（每个模组 1~2 次 `is_dir`）。虽然键在同一批内稳定、缓存能命中，但**每次调用
    // 都要为算键而白扫一遍模组目录**。实测（174 个模组、目录已进系统缓存）：
    //   - 旧方案 `bonus_scan_dirs` ：3.73 ms / 次
    //   - 新方案（廉价键）        ：228 ns / 次        ← 相差约 16000 倍
    // 真机 572 个模组时旧方案约 12 ms/次，数千次调用即**数十秒~数分钟**纯浪费
    // （这是"缓存构建时间又变长"的主因）。
    //
    // 改用「游戏目录 + 模组数」：一次 `format!` + 一次 `Arc::len`（`wiki_mod_paths`
    // 只是 Mutex lock + Arc clone）。模组集合真正变化时不靠这个键自失效 ——
    // 由 `set_wiki_mod_paths` 显式调用 [`release_caches`] 清空（模组数恰好相同时也能覆盖）。
    let key = format!(
        "{}:{}",
        game_dir.display(),
        crate::vcmi::resource::wiki_mod_paths()
            .map(|v| v.len())
            .unwrap_or(0)
    );
    crate::infra::cache::cfg_cache_get(&BONUS_GRAPHICS_CACHE, key, || {
        // 昂贵的目录发现与递归解析**只在缓存未命中时执行一次**。
        let mut map: BonusMap = HashMap::new();
        for d in &config_scan_roots(game_dir) {
            collect_bonus_files(d, &mut map);
        }
        map
    })
}

/// 释放图标表缓存（构建全部结束后调用；见 [`crate::infra::cache_gc`]）。
pub(crate) fn release_caches() {
    crate::infra::cache::cfg_cache_clear(&BONUS_GRAPHICS_CACHE);
}

/// 参与扫描的目录：游戏根 `config` + `Mods/vcmi` + 全部已装模组。
///
/// 只收 `Content/config`（或其大小写变体）存在的模组 —— 没有 `Content/` 的模组
/// 连 stat 都不必做（与 `resolve_resource_fallback` 的载体缓存同理）。
///
/// 与 [`crate::vcmi::spell_icons`] 共用：法术的 `graphics.iconImmune` 表同样要跨全部模组合并。
pub(crate) fn config_scan_roots(game_dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    let root_cfg = game_dir.join("config");
    if root_cfg.is_dir() {
        out.push(root_cfg);
    }
    let core = game_dir.join("Mods").join("vcmi");
    if core.is_dir() {
        out.push(core);
    }
    if let Some(all) = crate::vcmi::resource::wiki_mod_paths() {
        for (p, _) in all.iter() {
            if content_dir_of(p).is_some() {
                out.push(p.clone());
            }
        }
    }
    out
}

/// 取模组的 `Content` 目录（大小写容错；`Content` / `content`）。
///
/// 与 [`crate::vcmi::spell_icons`] 共用：法术配置同样挂在 `Content/` 下。
pub(crate) fn content_dir_of(mod_path: &Path) -> Option<PathBuf> {
    for name in ["Content", "content"] {
        let d = mod_path.join(name);
        if d.is_dir() {
            return Some(d);
        }
    }
    None
}

/// 在一个目录下找 `bonuses.json` / `bonus.json` 并合并进表。
///
/// 只按**文件名**判定（VCMI 把它们放在 `config/<mod>/bonuses/` 或 `config/` 下，
/// 也有 `config/bonusIcons/` 这类自定义目录名，按目录约定判定会漏）。
///
/// ⚠️ 走 [`crate::infra::dirscan::dir_listing`] 而**不是**自己 `read_dir` 递归：
/// 构建期已对同一个 `Content/` 目录建过清单（`collect_json_files`），复用即零额外 IO。
/// 此前用 `collect_all_json_files`（裸 `fs::read_dir` 递归）等于把 570 个模组的配置树
/// **再全量扫一遍**，只为捞出 4 个 `bonuses.json` —— 这是本次性能回归的第二个来源。
fn collect_bonus_files(base: &Path, map: &mut BonusMap) {
    // `base` 要么是模组根（取其 `Content`），要么是游戏根的 `config` 目录本身
    let scan_root = if base.file_name().map(|n| n.eq_ignore_ascii_case("config")) == Some(true) {
        base.to_path_buf()
    } else {
        match content_dir_of(base) {
            Some(c) => c,
            None => return,
        }
    };
    let listing = crate::infra::dirscan::dir_listing(&scan_root);
    for i in 0..listing.len() {
        let abs = listing.abs(i); // 已小写归一、正斜杠
        if !abs.ends_with("bonuses.json") && !abs.ends_with("bonus.json") {
            continue;
        }
        // 务必限定在 `config/` 下：清单根是 `Content/`，其中 `sprites/` 等目录也可能有同名文件
        if !abs.contains("/config/") {
            continue;
        }
        let v = crate::infra::json::read_vcmi_json(listing.path(i).as_ref());
        merge_bonus_json(&v, map);
    }
}

/// 把一份 `bonuses.json` 的 `graphics` 段并入表。
fn merge_bonus_json(v: &Value, map: &mut BonusMap) {
    let Some(obj) = v.as_object() else {
        return;
    };
    for (k, val) in obj {
        let ty = k.rsplit(':').next().unwrap_or(k).to_ascii_uppercase();
        let Some(g) = val.get("graphics") else {
            continue;
        };
        if g.is_null() {
            continue;
        }
        let e = map.entry(ty).or_default();
        if e.icon.is_none() {
            e.icon = res_of(g.get("icon"));
        }
        if let Some(m) = g.get("subtypeIcons").and_then(|x| x.as_object()) {
            for (sk, sv) in m {
                if let Some(r) = res_of(Some(sv)) {
                    let leaf = sk.rsplit(':').next().unwrap_or(sk).to_string();
                    e.subtype_icons.entry(leaf).or_insert(r);
                }
            }
        }
        if let Some(m) = g.get("valueIcons").and_then(|x| x.as_object()) {
            for (vk, vv) in m {
                let Ok(val) = vk.trim().parse::<i64>() else {
                    continue;
                };
                if let Some(r) = res_of(Some(vv)) {
                    e.value_icons.entry(val).or_insert(r);
                }
            }
        }
    }
}

/// 取资源路径（裸字符串或 `{file/name}` 对象），空值返回 `None`。
///
/// 与 [`crate::vcmi::spell_icons`] 共用：两侧读的都是 `graphics.*` 里的资源引用。
pub(crate) fn res_of(v: Option<&Value>) -> Option<String> {
    crate::vcmi::town::value_to_resource(v?).filter(|s| !s.is_empty())
}


