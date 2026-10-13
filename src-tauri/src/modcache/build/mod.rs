//! 单 mod 缓存构建：两阶段（数据 / 素材）+ 增量判定 + `mod.json` 落盘。
//!
//! ⚠️ 本文件目前是**整体迁移**（`build_mod_wiki_once` 仍是 1300+ 行的大函数）。
//! S3 将按「阶段 × 条目类型」二维拆分为 `wiki/build/{data,assets}/**`（设计文档 §4.2）。
//!
//! 依赖方向：可依赖 `infra` 与 `vcmi`（见 `docs/architecture/backend.zh-CN.md` §4.1）。

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::infra::hash::compute_source_hash;
use crate::infra::json::parse_vcmi_json;
use crate::vcmi::gamedata::{
    cached_hero_class_index_map, cached_starting_stack_chances, read_skill_index_map,
};
use crate::vcmi::h3core::{
    cached_core_i18n, h3_core_data_dir, h3_creature_stats, load_h3_core_i18n, merge_i18n_into,
    parse_core_mod, I18nMap,
};
use crate::vcmi::i18n::read_i18n_map;
use crate::vcmi::paths::VcmiPaths;
use crate::vcmi::town::{
    apply_building_id_library, cached_building_id_library, read_game_root_configs,
    read_game_root_factions, read_town_configs,
};
use crate::modcache::collect::{
    collect_all_raw_configs, collect_config_jsons_merged,
    merge_json_in_place,
};
use crate::modcache::i18n::{
    name_obj_en_zhcn,
    set_core_i18n,
};
use crate::modcache::version::{ASSETS_VERSION, WIKI_CACHE_VERSION};
use crate::vcmi::resource::{has_core_h3_lods, h3_data_key, h3_data_present, wiki_user_data_dir};
use crate::vcmi::h3core::h3_core_data_hash;
use crate::modcache::{sanitize_id, set_assets_enabled};

pub(crate) mod assets;
pub(crate) mod data;

/// 核心 i18n 合并结果缓存（key = 核心模组目录指纹）。
///
/// 原实现**每个 mod 都重新构造一次**核心翻译表（读 18 个 h3-core JSON + 全表 clone/merge），
/// 而 `set_core_i18n` 是 `OnceLock`，只有首次真正生效——后 N-1 次的构造结果被直接丢弃
/// （实测 652+556 次，`cfg.i18n.core` 累计约 150s CPU）。
static CORE_I18N_MERGED_CACHE: std::sync::OnceLock<
    std::sync::Mutex<Vec<(String, std::sync::Arc<I18nMap>)>>,
> = std::sync::OnceLock::new();

/// 构建单个城镇 mod 的 Wiki 缓存核心逻辑（可测试）。
/// 构建单个 mod 的 wiki 缓存（**两阶段编排**）。
///
/// 目标：首次启动不被图片/素材提取堵塞——阶段一**只解析配置出数据**（廉价），
/// 立刻落盘 `mod.json`（`assetsDone=false`、图片字段为 null），wiki/realm 页面即可显示
/// 文本数据；阶段二再补图并改写 `mod.json`（`assetsDone=true`）。
///
/// 判定：
/// - 缓存命中且 `assetsDone && assetsVersion` 达标 → 整体跳过；
/// - 缓存命中但素材未就绪 → **只补跑阶段二**（不重新解析配置）；
/// - 无缓存/数据过期 → 阶段一 → 阶段二。
/// 把 cache.json 的 `modPath` 写为**相对路径编码串**，避免写死绝对路径使缓存随包迁移后失效。
///
/// 编码规则（运行时由 `resolve_cache_mod_path` 还原）：
/// - `gameMods:<rel>`  → 相对 `{game_dir}/Mods`（游戏自带 Mods）
/// - `userMods:<rel>`  → 相对 `user_mods_dir`（launcher.json 配置的用户 Mods，默认 `{user_data_dir}/Mods`）
/// - `realm:<id>:<rel>`→ 相对 `{launcher_data_dir}/realms/<id>/vcmi/Mods`（领域整合模组）
/// - 其余（落在三者之外，如实 symlink 目录）→ 维持绝对路径兜底，不阻塞构建。
///
/// base 全部来自 `launcher.json` 所在目录（`.vcmi-fv-launcher`，即 `cache_root` 的父目录）或其权威配置，
/// 不依赖 `VcmiPaths::detect()` 重新探测游戏目录，消除「探测到别的游戏目录」的歧义。
/// 缓存整目录可随包迁移，相对串依旧能定位 mod 源。
fn encode_mod_path_for_cache(
    mod_path: &str,
    game_dir: &Path,
    user_mods_dir: &Path,
    launcher_data_dir: &Path,
) -> String {
    let p = PathBuf::from(mod_path);
    // 领域整合模组：launcher_data_dir/realms/<id>/vcmi/Mods/<mod...>
    if let Ok(rel) = p.strip_prefix(launcher_data_dir) {
        let comps: Vec<String> = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect();
        if comps.len() >= 5
            && comps[0].eq_ignore_ascii_case("realms")
            && comps[2].eq_ignore_ascii_case("vcmi")
            && comps[3].eq_ignore_ascii_case("Mods")
        {
            let id = &comps[1];
            let mod_rel = comps[4..].join("/");
            return format!("realm:{}:{}", id, mod_rel);
        }
    }
    if let Ok(rel) = p.strip_prefix(game_dir.join("Mods")) {
        return format!("gameMods:{}", rel.to_string_lossy().replace('\\', "/"));
    }
    if !user_mods_dir.as_os_str().is_empty() {
        if let Ok(rel) = p.strip_prefix(user_mods_dir) {
            return format!("userMods:{}", rel.to_string_lossy().replace('\\', "/"));
        }
    }
    mod_path.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_mod_path_schemes() {
        let game_dir = PathBuf::from("C:/game");
        let user_mods = PathBuf::from("C:/user/Mods");
        let ldd = PathBuf::from("C:/exe/.vcmi-fv-launcher");
        // 游戏自带 Mods
        assert_eq!(
            encode_mod_path_for_cache("C:/game/Mods/abyss-town", &game_dir, &user_mods, &ldd),
            "gameMods:abyss-town"
        );
        // 嵌套 mod
        assert_eq!(
            encode_mod_path_for_cache("C:/game/Mods/sub/mod", &game_dir, &user_mods, &ldd),
            "gameMods:sub/mod"
        );
        // 用户目录 Mods
        assert_eq!(
            encode_mod_path_for_cache("C:/user/Mods/foo", &game_dir, &user_mods, &ldd),
            "userMods:foo"
        );
        // 领域整合模组
        assert_eq!(
            encode_mod_path_for_cache(
                "C:/exe/.vcmi-fv-launcher/realms/rl/vcmi/Mods/bundled",
                &game_dir,
                &user_mods,
                &ldd
            ),
            "realm:rl:bundled"
        );
        // 三者之外 → 绝对兜底
        assert_eq!(
            encode_mod_path_for_cache("D:/other/mod", &game_dir, &user_mods, &ldd),
            "D:/other/mod"
        );
    }
}

pub(crate) fn build_mod_wiki_inner(mod_path: &str, mod_id: &str, cache_root: &Path, game_dir: &Path) -> Value {
    build_mod_wiki_phase(mod_path, mod_id, cache_root, game_dir, 2)
}

/// `phase`：0=仅数据（阶段一）；1=仅素材（阶段二）；2=两阶段（缺省/单次调用）。
///
/// 批量构建按「全部 mod 阶段一 → 全部 mod 阶段二」分批调用，使**所有 mod 的文本数据
/// 先整体就绪**，避免第 N 个 mod 的数据被前 N-1 个 mod 的素材提取阻塞
/// （后台默认 eco 档仅 1~2 线程，若每 mod 内部串接两阶段，队尾 mod 的数据会被长时间延迟）。
pub(crate) fn build_mod_wiki_phase(
    mod_path: &str,
    mod_id: &str,
    cache_root: &Path,
    game_dir: &Path,
    phase: u8,
) -> Value {
    // HD 素材登记同样是线程局部的（见 `modcache::hd_map`）：rayon 线程复用，
    // **必须**每 mod 开始前重置、结束时按 owner 落盘，否则登记项会串到下一个 mod。
    crate::modcache::hd_map::reset();
    let r = build_mod_wiki_phase_inner(mod_path, mod_id, cache_root, game_dir, phase);
    // 取走本线程埋点并并入全局：rayon 池内线程复用，**必须**每 mod 清一次，否则串味。
    let items = crate::infra::metrics::drain_local();
    if !items.is_empty() {
        crate::infra::metrics::merge_global(&items);
        tracing::debug!(
            target: "wiki",
            "mod={} phase={} 耗时构成: {}",
            mod_id,
            phase,
            crate::infra::metrics::format_items(&items, 10)
        );
    }
    crate::modcache::hd_map::flush(cache_root);
    // 记录本次构建所依据的 HD 来源指纹。仅在真正产出素材后写（phase 0 不产图、不登记，
    // 若此时就写标记，随后的阶段二会误判"登记已完成"而不补齐）。
    if phase != 0 && r.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        crate::modcache::hd_map::write_source_mark(cache_root, mod_id);
    }
    r
}

/// 增量短路兜底（问题 1-B）：缓存键命中不代表素材文件仍在磁盘上——可能被外部删除 /
/// 杀软清理 / 构建中途被杀。若本就无图的 mod（faction/creature/hero/building 全 0）直接命中即可；
/// 但只要该 mod 本来产出了图像类素材、而缓存目录里的图却没了，就必须放弃命中、重提取，否则
/// `check_realm_assets` 永远判缺失 → 每次打开都重跑进度页（与问题 1-A 同一类症状的不同成因）。
fn assets_dir_nonempty(cache_root: &Path, mod_id: &str) -> bool {
    let dir = cache_root.join(sanitize_id(mod_id));
    let Ok(rd) = fs::read_dir(&dir) else {
        return false;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_file() {
            // cache.json 本身不算素材（每个 mod 都有）
            if e.file_name().to_string_lossy() != "cache.json" {
                return true;
            }
        } else if p.is_dir() {
            if let Ok(mut sub) = fs::read_dir(&p) {
                if sub.next().is_some() {
                    return true;
                }
            }
        }
    }
    false
}

/// 单个 mod 的缓存状态（**预检与构建共用同一判定**，避免两处口径漂移）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ModCacheState {
    /// 数据缓存失效（版本键不符 / 无缓存文件）→ 需要阶段一重建（连带阶段二）
    NeedData,
    /// 数据就绪，仅素材需补跑：`assetsVersion` 落后 / HD 来源变化 / 图已不在磁盘
    NeedAssets,
    /// 完全命中，无需任何工作
    Cached,
}

/// [`probe_mod_cache`] 的结果：状态 + 旧缓存内容计数（免构建命中分支二次读盘）+ 是否需清旧产物。
pub(crate) struct ModCacheProbe {
    pub state: ModCacheState,
    /// 旧 `cache.json` 的 `contentCount`（无缓存时为 `{}`）
    pub content_count: Value,
    /// `NeedAssets` 且需先清空 `images/` 与 `hd-map.json`（assets 版本落后 / HD 来源变化）
    pub clear_stale: bool,
}

/// 判定单个 mod 的缓存状态 —— **构建与预检的唯一判定入口**。
///
/// 缓存键 = `v{WIKI_CACHE_VERSION}:{sourceHash}:{h3DataKey}:{h3CoreHash}:{schemaHash}`，
/// 任一变化即整体失效（见 `modcache/version.rs`）。调用方：
/// - [`build_mod_wiki_phase_inner`]（真实构建，据状态决定跳过 / 补素材 / 重建）；
/// - `commands::cache::wiki_cache_state`（启动预检，据状态决定是否弹进度蒙层）。
///
/// ⚠️ 成本 = 一次 [`compute_source_hash`]（`mod.json` 全文 + `content.zip` 首尾 64 KiB 采样），
/// 与真实构建同源，故预检不会引入"第二次不同口径的判断"。
pub(crate) fn probe_mod_cache(
    mod_path: &str,
    mod_id: &str,
    cache_root: &Path,
    game_dir: &Path,
) -> ModCacheProbe {
    let need_data = ModCacheProbe {
        state: ModCacheState::NeedData,
        content_count: json!({}),
        clear_stale: false,
    };
    let p = PathBuf::from(mod_path);
    if !p.is_dir() {
        // 目录不存在：交由构建路径报 notfound，预检按"需重建"处理（保守：宁可多建不少建）
        return need_data;
    }
    let source_hash = compute_source_hash(&p);
    // 以下三个指纹与构建入口完全一致（原版数据状态 / h3-core 文案 / 内置 schema）
    let h3_key = h3_data_key(game_dir, &wiki_user_data_dir().unwrap_or_default());
    let schema_key = crate::modcache::schema::schema_content_hash();
    let h3_core_hash = h3_core_data_hash();
    let version_key = format!("v{WIKI_CACHE_VERSION}:{source_hash}:{h3_key}:{h3_core_hash}:{schema_key}");
    let out_path = cache_root.join(format!("{}/cache.json", sanitize_id(mod_id)));
    if let Ok(text) = fs::read_to_string(&out_path) {
        if let Ok(v) = serde_json::from_str::<Value>(&text) {
            let h3_core_hash_old = v.get("h3CoreHash").and_then(|x| x.as_str()).unwrap_or("");
            let old_key = format!(
                "v{}:{}:{}:{}:{}",
                v.get("cacheVersion").and_then(|x| x.as_u64()).unwrap_or(0),
                v.get("sourceHash").and_then(|x| x.as_str()).unwrap_or(""),
                v.get("h3DataKey").and_then(|x| x.as_str()).unwrap_or(""),
                h3_core_hash_old,
                v.get("schemaHash").and_then(|x| x.as_str()).unwrap_or("")
            );
            let cc = v.get("contentCount").cloned().unwrap_or(json!({}));
            if v.get("sourceHash").is_some() && old_key == version_key {
                let assets_ver_ok = v.get("assetsDone").and_then(|x| x.as_bool()).unwrap_or(false)
                    && v.get("assetsVersion").and_then(|x| x.as_u64()).unwrap_or(0)
                        >= ASSETS_VERSION as u64;
                // HD 来源（`hd-edition`）指纹：装 / 卸 / 升级或切档位后需重跑素材阶段补登记，
                // 即使 `assetsVersion` 未变（见 `modcache::hd_map::source_mark_matches`）。
                let hd_ok = crate::modcache::hd_map::source_mark_matches(cache_root, mod_id);
                if assets_ver_ok && hd_ok {
                    // 方案 B 兜底：若该 mod 本应产出图像素材，但磁盘上的图已不在，则放弃命中、强制重提取
                    // （前线 `check_realm_assets` 会据此再次判缺失并重跑，否则陷入"每次打开都重跑"死循环）。
                    let image_bearing = ["faction", "creature", "hero", "building"].iter().any(|k| {
                        v.get("contentCount")
                            .and_then(|c| c.get(k))
                            .and_then(|x| x.as_u64())
                            .unwrap_or(0)
                            > 0
                    });
                    if !image_bearing || assets_dir_nonempty(cache_root, mod_id) {
                        return ModCacheProbe { state: ModCacheState::Cached, content_count: cc, clear_stale: false };
                    }
                    // 图不在磁盘 → 仅需重提取（无需清理：本来就没有）
                    return ModCacheProbe { state: ModCacheState::NeedAssets, content_count: cc, clear_stale: false };
                }
                // 素材版本落后 / HD 来源变化 → 需补跑并清旧产物：
                // 前者避免"改了编码策略但旧图还在"；后者保证 `hd-map.json` 被重新写入
                // （图已存在时会走短路、不再登记，只有强制重提取才能补上）。
                return ModCacheProbe { state: ModCacheState::NeedAssets, content_count: cc, clear_stale: true };
            }
        }
    }
    need_data
}

fn build_mod_wiki_phase_inner(
    mod_path: &str,
    mod_id: &str,
    cache_root: &Path,
    game_dir: &Path,
    phase: u8,
) -> Value {
    let p = PathBuf::from(mod_path);
    if !p.is_dir() {
        return json!({ "ok": false, "error": "notfound" });
    }
    let probe = probe_mod_cache(mod_path, mod_id, cache_root, game_dir);
    let cached_json = |cc: &Value| {
        json!({
            "ok": true,
            "cached": true,
            "cachePath": cache_root.join(format!("{}/cache.json", sanitize_id(mod_id))).to_string_lossy(),
            "contentCount": cc,
        })
    };
    match probe.state {
        // 完全命中
        ModCacheState::Cached => return cached_json(&probe.content_count),
        ModCacheState::NeedAssets => {
            // 阶段一只要数据：数据已命中 → 直接视为命中（素材留到阶段二）
            if phase == 0 {
                return cached_json(&probe.content_count);
            }
            if probe.clear_stale {
                let dir = cache_root.join(sanitize_id(mod_id));
                let _ = fs::remove_dir_all(dir.join("images"));
                let _ = fs::remove_file(dir.join("hd-map.json"));
            }
            return build_mod_wiki_once(mod_path, mod_id, cache_root, game_dir, true);
        }
        ModCacheState::NeedData => { /* 落到下方完整构建 */ }
    }
    // 要求"只补素材"却连数据都没有（首次构建直接跑阶段二）→ 退化为完整构建
    if phase == 1 {
        let d = build_mod_wiki_once(mod_path, mod_id, cache_root, game_dir, false);
        if d.get("ok").and_then(|x| x.as_bool()) != Some(true) {
            return d;
        }
        return build_mod_wiki_once(mod_path, mod_id, cache_root, game_dir, true);
    }
    // 阶段一：只出数据（不解码 DEF、不编码 webp、不合成预览），让页面尽快可显示
    let data = build_mod_wiki_once(mod_path, mod_id, cache_root, game_dir, false);
    if data.get("ok").and_then(|x| x.as_bool()) != Some(true) {
        // 阶段一就失败（notown/notfound 等）→ 不必再跑阶段二
        return data;
    }
    if phase == 0 {
        return data;
    }
    // 阶段二：补齐图片后改写 mod.json
    build_mod_wiki_once(mod_path, mod_id, cache_root, game_dir, true)
}

/// 单次构建主体。`with_assets=false` 时跳过一切素材提取（阶段一），
/// `=true` 时补齐素材（阶段二）。
///
/// 缓存失效策略见 `modcache/version.rs`：版本号 + 源哈希 + h3 数据/文案指纹 + schema 哈希
/// 拼成缓存键，任一变化即整体失效重建。版本号本身只作失效标记（不参与数值比较），
/// 改后端逻辑后旧缓存会失效、不会再出现"修复了但表现没变"的假象。

pub(crate) fn build_mod_wiki_once(
    mod_path: &str,
    mod_id: &str,
    cache_root: &Path,
    game_dir: &Path,
    with_assets: bool,
) -> Value {
    set_assets_enabled(with_assets);
    // 原版 H3 数据是否就位（缓存命中，对正常构建零额外开销）：缺失则百科图片无源，
    // 供 `mod.json` 的 `h3DataMissing` 与构建结果标记，前端据此提示用户补放数据文件。
    let ud_for_check = wiki_user_data_dir().unwrap_or_default();
    let h3_missing = !h3_data_present(game_dir, &ud_for_check);
    // 诊断：原版数据到底在游戏根还是用户数据目录（便携运行常在 user_data_dir/Data）。
    // 出问题（有数据却无图 / 误报缺失）时据此定位用户是否把数据放错了目录。
    if !matches!(std::env::var("VCMI_WIKI_QUIET").as_deref(), Ok("1") | Ok("true")) {
        let in_game = has_core_h3_lods(game_dir);
        let in_user = if ud_for_check.as_os_str().is_empty() {
            false
        } else {
            has_core_h3_lods(&ud_for_check)
        };
        tracing::info!(
            target: "wiki",
            "原版 H3 数据探测：mod={} game_dir/Data 有核心LOD={} user_dir={:?} user_dir/Data 有核心LOD={} → 缺失={}",
            mod_id, in_game, ud_for_check, in_user, h3_missing
        );
    }
    let p = PathBuf::from(mod_path);
    if !p.is_dir() {
        return json!({ "ok": false, "error": "notfound" });
    }
    let source_hash = compute_source_hash(&p);
    // 仅在阶段一（数据重建）清空图片产物目录：save_wiki_image 对已存在产物是幂等跳过的，
    // 若不清理，改编码策略或改提取逻辑后旧产物会被原样保留，真机表现"修了但没变"。
    if !with_assets {
        let _ = fs::remove_dir_all(cache_root.join(sanitize_id(mod_id)).join("images"));
    }

    let (is_core, core_faction) = parse_core_mod(mod_id);
    // 埋点：城镇配置解析 + 原版 factions 并入 + 建筑 bid 校正
    let configs = crate::infra::metrics::scope("cfg.town", || {
        let mut configs = read_town_configs(&p);
        // 原版 9 城镇不在任何 mod 的 town.json，而在游戏根 config/factions/*.json（携带完整图片字段）。
        // 子模组 `core.<faction>` 只并入本阵营的那一座城镇（父 core 不持有城镇，factions 保持空数组），
        // 使每个子模组的形态与第三方 town mod 完全一致，且不重复注入。
        if let Some(f) = core_faction {
            for c in read_game_root_factions(game_dir) {
                if c.faction_id == f {
                    configs.push(c);
                }
            }
        }
        // 建筑 bid 校正：百科建筑图标 = clientInfo.buildingsIcons 的第 bid 帧，
        // bid 必须取自 VCMI 标准表（多数 mod 的建筑只写名称不写 id）。
        // 走跨 mod 缓存：原本每个 mod 都重新解析一次该表（380 次）。
        let building_id_lib = cached_building_id_library(game_dir);
        apply_building_id_library(&mut configs, &building_id_lib);
        configs
    });
    // 初始部队出现概率（VCMI 游戏设置，非英雄配置）
    // 取切片：Arc<Vec<i64>> 本身不实现 Serialize（serde 未开 rc feature），切片可以
    let stack_chances = cached_starting_stack_chances(game_dir);
    let stack_chances: &[i64] = stack_chances.as_slice();
    let i18n = crate::infra::metrics::scope("cfg.i18n", || read_i18n_map(&p));
    // 并入 vcmi 核心模组翻译（原版建筑/生物/英雄等名称的中文等翻译由核心提供）；
    // mod 自身翻译优先（覆盖同名语言）。
    // 原实现把整张核心表逐 key 合并进每个 mod 的 i18n map（380 次全表合并 + 380 次解析），
    // 改为全局持有一次、由 i18n_get_multi 按 key 两级查询。
    //
    // ⚠️ 埋点 `cfg.i18n.core` 用于暴露一处已知浪费：这段**每个 mod 都会完整执行一次**
    // （读 18 个 h3-core JSON + clone/merge 整张表），而 `set_core_i18n` 是 OnceLock，
    // 只有第一次真正生效——后 N-1 次的构造结果被直接丢弃。
    crate::infra::metrics::scope("cfg.i18n.core", || {
        // 核心模组目录优先取本次构建使用的 game_dir（测试/多安装场景可能与全局 detect 不同）
        let detected = VcmiPaths::detect().game_dir;
        for core_mod in [
            game_dir.join("Mods").join("vcmi"),
            detected.join("Mods").join("vcmi"),
        ] {
            if !core_mod.is_dir() {
                continue;
            }
            // 合并结果按目录指纹缓存：跨 mod / 跨批次只构造一次
            let key = crate::infra::cache::fingerprint_of(&[core_mod.clone()]);
            let merged = crate::infra::cache::cfg_cache_get(&CORE_I18N_MERGED_CACHE, key, || {
                let core_i18n_base = cached_core_i18n(&core_mod);
                let mut merged: I18nMap = (*core_i18n_base).clone();
                let h3_dir = h3_core_data_dir();
                let h3 = h3_dir.as_ref().and_then(|d| load_h3_core_i18n(d));
                tracing::debug!(
                    target: "wiki-core",
                    "core i18n 合并：h3 keys={} base keys={}",
                    h3.as_ref().map(|m| m.len()).unwrap_or(0),
                    merged.len()
                );
                if let Some(h3) = h3 {
                    merge_i18n_into(&mut merged, &h3);
                }
                merged
            });
            set_core_i18n(merged);
            break;
        }
    });
    // 游戏公共素材（SMALRES 资源图标等）——batch 路径此前缺失调用导致前端资源图标全部缺失
    assets::extract_shared_resource_icons(game_dir, cache_root, mod_id);
    // creature -> 招募建筑名（多语言）映射，用于生物页面 Dwellings 区块
    let mut creature_dwellings: std::collections::HashMap<String, serde_json::Value> = std::collections::HashMap::new();
    // 预收集生物配置，便于城镇 creatureList 扩展升级生物以及标记 upgradedFrom
    let creature_jsons: std::collections::HashMap<String, Value> =
        crate::infra::metrics::scope("cfg.creatures", || {
            let mut m: std::collections::HashMap<String, Value> =
                collect_config_jsons_merged(&p, "creatures").into_iter().collect();
            // 原版生物不在任何 mod 内，位于游戏根 config/creatures；仅对 core 系并入。
            // 使原版城镇的 creatureList 能命中 modCache.creatures（生物小图标经 CPRSMALL 游戏根回退出图）。
            // 子模组只取本阵营文件；父 core 取 neutral / special 等无归属文件。
            if is_core {
                // 原版配置缺数值属性 → 用 h3-core（源自 CRTRAITS.TXT）的 stats 补齐，
                // 否则 Wiki 里原版生物属性/花费全为 0（实测：castle.json 的 pikeman 无 attack 等字段）。
                let h3_stats = h3_creature_stats();
                for (id, mut v) in read_game_root_configs(game_dir, "creatures", core_faction) {
                    if let Some(st) = h3_stats.as_ref().and_then(|m| m.get(&id)) {
                        merge_json_in_place(&mut v, st);
                    }
                    m.entry(id).or_insert(v);
                }
            }
            m
        });
    // 扫描升级关系：升级生物 id -> 基础生物 id 列表
    let mut upgraded_from: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for (base_id, obj) in &creature_jsons {
        if let Some(upgs) = obj.get("upgrades").and_then(|x| x.as_array()) {
            for u in upgs {
                if let Some(uid) = u.as_str() {
                    upgraded_from.entry(uid.to_string()).or_default().push(base_id.clone());
                }
            }
        }
    }

    // ---- 归一原始配置（raw_cfgs）----
    // 提前到城镇构建之前：城镇名/描述需要 mod.json 语言块（mod_lang_texts），
    // 英雄 / 地形 / mod-raw 落盘等后续步骤共用同一份。
    let mut raw_cfgs = crate::infra::metrics::scope("cfg.raw", || collect_all_raw_configs(&p));
    // 原版 heroClasses 在游戏根 config/heroClasses.json（vcmi mod 不含），
    // 英雄表按 affinity（might/magic）分组、职业名与阵营推导均依赖它，core 分支须并入。
    if is_core {
        let hc_path = game_dir.join("config").join("heroClasses.json");
        if let Value::Object(obj) =
            parse_vcmi_json(&fs::read_to_string(&hc_path).unwrap_or_default())
        {
            let entry = raw_cfgs
                .entry("heroClasses".to_string())
                .or_insert_with(|| Value::Object(Default::default()));
            if let Value::Object(m) = entry {
                for (k, v) in obj {
                    m.entry(k).or_insert(v);
                }
            }
        }
    }

    // ---- 城镇 ----
    // mod.json 语言块（单城镇 mod 的名称/描述兜底来源，见 mod_lang_texts）。
    // core / core.<faction> 伪 mod 的 mod.json 是 **VCMI 核心自身**（描述为
    // "Essential files required for VCMI to run correctly"），与具体城镇无关，必须排除。
    let (mod_lang_name, mod_lang_desc) = if is_core || core_faction.is_some() {
        (None, None)
    } else {
        mod_lang_texts(&raw_cfgs)
    };
    let (mut factions, icon_failures) = crate::infra::metrics::scope("ent.towns", || {
        data::town::build_towns(
            &p,
            mod_id,
            cache_root,
            game_dir,
            &configs,
            &i18n,
            &mut creature_dwellings,
            &creature_jsons,
            mod_lang_name.as_ref(),
            mod_lang_desc.as_ref(),
        )
    });

    // ---- 提取协议审计（常驻，见 modcache/audit.rs）----
    // 1) 扫描真实配置找"配置里有、协议未登记"的文本字段（发现某个 mod 用了新写法而提取没覆盖）；
    // 2) 城镇"声明 vs 提取"缺口：配置声明了 buildings / buildingsIcons 但提取为 0 / 0 图标。
    // 放在城镇构建**之后**（需要已构建的 factions 比对）；结果写 `<mod>/audit.json`（无问题则不写），
    // 无需手工跑 `scripts/mod-cache-audit.mjs --game-dir`。
    let audit_report = crate::infra::metrics::scope("cfg.audit", || {
        crate::modcache::audit::audit_raw_configs(&raw_cfgs, &factions)
    });

    // ---- 建筑：把各城镇的建筑平铺为独立条目（供百科"建筑"分类检索）----
    // TODO：中立/地图建筑暂未纳入，需后续从游戏根 config/objects/*.json 提取。
    let mut buildings = Vec::new();
    for faction in factions.iter() {
        let fid = faction.get("factionId").and_then(|x| x.as_str()).unwrap_or("");
        let fname = faction.get("name").cloned().unwrap_or(Value::Null);
        if let Some(arr) = faction.get("buildings").and_then(|x| x.as_array()) {
            for b in arr {
                if let Some(obj) = b.as_object() {
                    let mut bo = obj.clone();
                    let bid = obj.get("id").and_then(|x| x.as_str()).unwrap_or("");
                    let unique_id = if fid.is_empty() || bid.is_empty() {
                        bid.to_string()
                    } else {
                        format!("{}.{}", fid, bid)
                    };
                    bo.insert("id".to_string(), json!(unique_id));
                    bo.insert("factionId".to_string(), json!(fid));
                    bo.insert("factionName".to_string(), fname.clone());
                    buildings.push(Value::Object(bo));
                }
            }
        }
    }
    // 中立建筑（原版 h3-core `neutral.*`）：只挂在父 core 上（不隶属任何城镇），
    // 名称/描述来自 BUILDING.TXT（`building.core.neutral.<bid>.name/description`）。
    // 图标与花费暂缺——中立建筑在 H3 里是冒险地图对象，无城镇图标、BUILDING.TXT 也无成本列。
    if is_core && core_faction.is_none() {
        for bid in crate::vcmi::h3core::h3_building_ids("neutral.") {
            let short = bid.trim_start_matches("neutral.").to_string();
            let name = name_obj_en_zhcn(
                &i18n,
                &format!("building.core.neutral.{}.name", short),
                &short,
            );
            let desc = name_obj_en_zhcn(
                &i18n,
                &format!("building.core.neutral.{}.description", short),
                "",
            );
            buildings.push(json!({
                "id": format!("neutral.{}", short),
                "mode": "neutral",
                "bid": -1,
                "name": name,
                "description": desc,
                "cost": {},
                "image": Value::Null,
                "factionId": "neutral",
                "factionName": { "chinese": "中立", "english": "Neutral" },
            }));
        }
    }

    // ---- 生物 ----
    let mut creatures = crate::infra::metrics::scope("ent.creatures", || {
        data::creature::build_creatures(
            &p, mod_id, cache_root, game_dir, &i18n, &factions, &mut creature_dwellings, &creature_jsons,
            &upgraded_from,
        )
    });
    // ---- 英雄 ----
    let skill_index_map = read_skill_index_map(game_dir);
    let hero_class_index_map = cached_hero_class_index_map(game_dir);
    let mut heroes = crate::infra::metrics::scope("ent.heroes", || {
        data::hero::build_heroes(
            &p, mod_id, cache_root, game_dir, &configs, &i18n, is_core, core_faction, &raw_cfgs,
            &skill_index_map, &hero_class_index_map,
        )
    });
    // ---- 法术 ----
    let mut spells = crate::infra::metrics::scope("ent.spells", || {
        data::spell::build_spells(&p, mod_id, cache_root, game_dir, &i18n)
    });
    // ---- 宝物 ----
    let mut artifacts = crate::infra::metrics::scope("ent.artifacts", || {
        data::artifact::build_artifacts(&p, mod_id, cache_root, game_dir, &i18n)
    });
    // ---- 技能 ----
    let mut skills = crate::infra::metrics::scope("ent.skills", || {
        data::skill::build_skills(&p, mod_id, cache_root, game_dir, &i18n, &skill_index_map)
    });
    // ---- 跨 mod 引用标记（VCMI `<modId>:<entityId>` 语法）----
    // 语义是"对目标 mod 实体的补丁/覆盖"（如 `hota.factory:factory`、`core:dimensionDoor`），
    // 多数条目无独立数据（名称回落 id 原文、属性为空），不应作为独立实体进入百科列表。
    // 统一打 `ref:true` 标记（数据保留，便于跳转与诊断），前端列表隐藏、contentCount 不计。
    let ref_count = mark_ref_entries(&mut factions)
        + mark_ref_entries(&mut buildings)
        + mark_ref_entries(&mut creatures)
        + mark_ref_entries(&mut heroes)
        + mark_ref_entries(&mut spells)
        + mark_ref_entries(&mut artifacts)
        + mark_ref_entries(&mut skills);
    let content_count = json!({
        "faction": count_non_ref(&factions),
        "creature": count_non_ref(&creatures),
        "hero": count_non_ref(&heroes),
        "spell": count_non_ref(&spells),
        "artifact": count_non_ref(&artifacts),
        "skill": count_non_ref(&skills),
        "building": count_non_ref(&buildings),
        // 跨 mod 引用条目数（诊断用：前端可知该 mod 有多少补丁引用被隐藏）
        "ref": ref_count,
    });

    // 属性表头图标（游戏 stackWindow def）与公共 UI 图标（游戏级资源，与 mod 无关，供弹窗使用），
    // 缺游戏资源时为空对象。统一由 `assets` 维度入口提取（见 `assets/mod.rs`）。
    let (stat_icons, shared_icons) = crate::infra::metrics::scope("shared.icons", || {
        assets::build_shared_icons(game_dir, cache_root, mod_id)
    });
    // ---- 地形（游戏级数据，VCMI WikiWindow.cpp:660-696）----
    let (terrains, failed_previews) = crate::infra::metrics::scope("ent.terrains", || {
        data::terrain::build_terrains(
            &p, mod_id, cache_root, game_dir, &i18n, &raw_cfgs, &configs, &mut factions,
        )
    });
    // ---- 资源类型（游戏级数据：金币/木材/水银/…，名称来自 h3-core 官方译名）----
    let resources = crate::infra::metrics::scope("ent.resources", || {
        data::resource::build_resources(game_dir, &i18n, mod_id)
    });
    // ---- 地图对象（VCMI `object.<scope>.<id>.name` 类名 + `mapObject.*` 子类型名/访问事件）----
    let objects = crate::infra::metrics::scope("ent.objects", || {
        data::object::build_objects(&i18n, raw_cfgs.get("objects").and_then(|x| x.as_object()), mod_id)
    });
    let mod_json = json!({
        "modId": mod_id,
        "sourceHash": source_hash,
        // 缓存代码版本（后端逻辑升级时递增，用于强制重建）
        "cacheVersion": WIKI_CACHE_VERSION,
        // 内置 VCMI 配置定义（schema）的 contentHash：schema 更新 → 缓存失效重建（静默、用户无感）
        "schemaHash": crate::modcache::schema::schema_content_hash(),
        // 原版数据状态（供前端提示 + 缓存失效）：缺失时百科图片将无源，用户补放后下次自动重建
        "h3DataKey": h3_data_key(game_dir, &wiki_user_data_dir().unwrap_or_default()),
        // h3-core 数据内容指纹（见 `h3_core_data_hash`）：改其文案后自动触发静默重建
        "h3CoreHash": h3_core_data_hash(),
        "h3DataMissing": h3_missing,
        // 素材是否已提取完毕（两阶段构建：阶段一 false，阶段二补齐后置 true）
        "assetsDone": with_assets,
        // 素材产物版本（提取逻辑变更时递增，可使仅重跑阶段二）
        "assetsVersion": if with_assets { ASSETS_VERSION } else { 0 },
        // 原始 mod 目录：存为相对 `launcher.json` 权威配置 / `.vcmi-fv-launcher` 根的编码串
        // （`gameMods:`/`userMods:`/`realm:<id>:` 前缀），运行时经 `resolve_cache_mod_path` 还原。
        // 避免写死绝对路径，使缓存可随包迁移、零歧义。
        "modPath": encode_mod_path_for_cache(
            mod_path,
            game_dir,
            &VcmiPaths::detect().user_mods_dir(),
            &VcmiPaths::launcher_data_dir()
                .unwrap_or_else(|| cache_root.to_path_buf()),
        ),
        // 生物表表头属性图标 {level/attack/...: 相对路径}
        "statIcons": stat_icons,
        // 游戏公共 UI 图标（弹窗属性/英雄主属性/法术/头像）
        "sharedIcons": shared_icons,
        // 游戏设置（VCMI gameConfig.json，供英雄弹窗显示初始部队概率）
        "gameSettings": { "startingStackChances": stack_chances },
        "encrypted": false,
        // 模组元数据（**启动器 mods / realms 页面**直接消费：多语言名/描述 + 作者/版本/类型/依赖）。
        // 此前投影缓存无 mod 级信息，两页面只能走实时扫描且拿不到多语言描述
        // （见 `src/lib/tauri.ts` 的 InstalledMod 注释）。
        "modMeta": build_mod_meta(&raw_cfgs, mod_id),
        // 建筑图标提取失败诊断（声明了 buildingsIcons 但多通道全失败）：空数组 = 全部成功。
        // 阶段一不产图（assets 关闭）→ 恒为空，仅阶段二有意义；批次结束按汇总 WARN 提示。
        "iconFailures": icon_failures,
        "factions": factions,
        "buildings": buildings,
        "creatures": creatures,
        "heroes": heroes,
        "spells": spells,
        "artifacts": artifacts,
        "skills": skills,
        // 资源类型（游戏级数据：木材/水银/矿石/硫磺/水晶/宝石/黄金/秘银，
        // 名称走 h3-core 官方译名 + mod 翻译键；供价格/花费等场景按 id/index 取用）
        "resources": resources,
        // 地图对象（adventure map objects：类名/子类型名/访问事件文本；
        // 名称来自 `object.*` / `mapObject.*` 翻译键，见 build/data/object.rs）
        "objects": objects,
        // 地形（游戏级数据，VCMI WikiWindow.cpp:660-696）
        "terrains": terrains,
        // 预览合成失败的 faction（诊断用；成功者 preview 字段已内嵌）
        "failedPreviews": failed_previews,
    });
    // 先确保输出目录存在——此前目录创建在 raw 落盘**之后**，导致 raw.json（无损层）
    // 在首次构建时因目录不存在而静默写失败（`let _ =` 忽略了错误），
    // 无损层数据从未真正落盘（城镇预览合成等依赖它的功能只能回退重新解析）。
    let out = cache_root.join(format!("{}/cache.json", sanitize_id(mod_id)));
    if fs::create_dir_all(out.parent().unwrap()).is_err() {
        return json!({ "ok": false, "error": "io" });
    }
    // 数据源保底层：全量原始配置归一聚合，写独立 raw.json（首次初始化时聚合，
    // 供城镇预览合成等后台功能直接读取，避免多次解析复杂 mod 目录）
    let raw_json = json!({
        "version": 1,
        "modId": mod_id,
        "sourceHash": source_hash,
        "rawConfigs": Value::Object(raw_cfgs),
    });
    let raw_out = cache_root.join(format!("{}/raw.json", sanitize_id(mod_id)));
    // 埋点：落盘序列化 + 写入（含字节数，`raw.json` 为全量原始配置聚合，体积可疑）
    crate::infra::metrics::scope("io.write", || {
        let text = serde_json::to_string(&raw_json).unwrap();
        crate::infra::metrics::bytes("io.bytes.raw", text.len() as u64);
        let _ = fs::write(&raw_out, text);
    });
    let ok = crate::infra::metrics::scope("io.write", || {
        let text = serde_json::to_string_pretty(&mod_json).unwrap();
        crate::infra::metrics::bytes("io.bytes.mod", text.len() as u64);
        fs::write(&out, text).is_ok()
    });
    if !ok {
        return json!({ "ok": false, "error": "io" });
    }
    // 提取协议审计结果落盘（仅发现未登记字段/缺口时产出，供人工复查；见 modcache/audit.rs）
    if let Some(rep) = audit_report {
        let audit_out = cache_root.join(format!("{}/audit.json", sanitize_id(mod_id)));
        if let Ok(text) = serde_json::to_string_pretty(&rep) {
            let _ = fs::write(&audit_out, text);
        }
        tracing::debug!(target: "wiki", "mod={} 协议审计：存在未登记文本字段或提取缺口", mod_id);
    } else {
        // 无缺口：清理残留（阶段一的 audit.json 见到的 image 全为 null，会误报 icons-missing；
        // 阶段二无缺口时若不删除，旧报告会一直留在缓存目录里）
        let _ = fs::remove_file(cache_root.join(format!("{}/audit.json", sanitize_id(mod_id))));
    }
    json!({
        "ok": true,
        "cachePath": out.to_string_lossy(),
        "factions": factions.len(),
        "contentCount": content_count,
        "h3DataMissing": h3_missing,
        // 建筑图标失败数（批次层按此汇总 WARN；明细见 cache.json 的 iconFailures）
        "failedIcons": icon_failures.len(),
    })
}

/// 标记跨 mod 引用条目（VCMI `<modId>:<entityId>` 语法）：写入 `ref: true` + `refMod` / `refId`。
/// 返回被标记的条目数。
fn mark_ref_entries(arr: &mut [Value]) -> usize {
    let mut n = 0;
    for v in arr.iter_mut() {
        let eid = v
            .get("id")
            .or_else(|| v.get("factionId"))
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if !eid.contains(':') {
            continue;
        }
        n += 1;
        if let Some(o) = v.as_object_mut() {
            o.insert("ref".to_string(), json!(true));
            if let Some((m, id)) = eid.split_once(':') {
                o.insert("refMod".to_string(), json!(m));
                o.insert("refId".to_string(), json!(id));
            }
        }
    }
    n
}

/// 非跨 mod 引用条目数（contentCount 口径：补丁引用不计入）。
fn count_non_ref(arr: &[Value]) -> usize {
    arr.iter()
        .filter(|v| !v.get("ref").and_then(|x| x.as_bool()).unwrap_or(false))
        .count()
}

/// 构建**模组元数据**（投影给启动器 mods / realms 页面）。
///
/// 名称/描述取多语言（VCMI 语言块 `<lang>.name/description` + 顶层原文），
/// 其余字段（作者/版本/类型/依赖/兼容性）原样透传。
/// 此前投影缓存无 mod 级信息 → mods/realms 只能走实时扫描（`list_installed_mods`）且无多语言描述。
pub(crate) fn build_mod_meta(raw_cfgs: &serde_json::Map<String, Value>, mod_id: &str) -> Value {
    let Some(m) = raw_cfgs.get("mod").and_then(|x| x.as_object()) else {
        return json!({ "modId": mod_id });
    };
    let mut names = serde_json::Map::new();
    let mut descs = serde_json::Map::new();
    // 语言块：`"chinese": { "name": ..., "description": ... }`
    for (lang, v) in m {
        let Some(o) = v.as_object() else { continue };
        if let Some(n) = o.get("name").and_then(|x| x.as_str()) {
            if !n.trim().is_empty() {
                names.insert(lang.clone(), json!(n));
            }
        }
        if let Some(d) = o.get("description").and_then(|x| x.as_str()) {
            if !d.trim().is_empty() {
                descs.insert(lang.clone(), json!(d));
            }
        }
    }
    // 顶层原文（VCMI 基准语言为英文）→ english 候选
    if let Some(n) = m.get("name").and_then(|x| x.as_str()) {
        if !n.trim().is_empty() {
            names.entry("english".to_string()).or_insert(json!(n));
        }
    }
    if let Some(d) = m.get("description").and_then(|x| x.as_str()) {
        if !d.trim().is_empty() {
            descs.entry("english".to_string()).or_insert(json!(d));
        }
    }
    json!({
        "modId": mod_id,
        "name": Value::Object(names),
        "description": Value::Object(descs),
        "author": m.get("author").cloned().unwrap_or(json!("")),
        "version": m.get("version").cloned().unwrap_or(json!("")),
        "modType": m.get("modType").cloned().unwrap_or(json!("")),
        "depends": m.get("depends").cloned().unwrap_or(json!([])),
        "conflicts": m.get("conflicts").cloned().unwrap_or(json!([])),
        "compatibility": m.get("compatibility").cloned().unwrap_or(json!({})),
    })
}

/// 从 mod.json 提取**多语言文本块**，供单城镇 mod 的城镇名/描述兜底。
///
/// VCMI 允许把 mod 元数据文本写在语言块里：
/// ```json
/// { "name": "Preserve Town", "description": "New town made by Ben Yan",
///   "chinese": { "name": "禁林", "description": "禁林生物的设计源自魔戒的世界观……", "translations": [...] } }
/// ```
/// 实测 630 个 mod.json 中 **485 个带多语言 description（454 个是单城镇 mod）**——
/// 许多城镇 mod 不写 translation 文件，城镇描述只存在于此，此前完全不读 → 城镇描述仅 6/93 可显示。
///
/// 返回 `(name 对象, description 对象)`（键为语言名，值均非空字符串）；无则 None。
pub(crate) fn mod_lang_texts(
    raw_cfgs: &serde_json::Map<String, Value>,
) -> (Option<Value>, Option<Value>) {
    let Some(m) = raw_cfgs.get("mod").and_then(|x| x.as_object()) else {
        return (None, None);
    };
    let mut names = serde_json::Map::new();
    let mut descs = serde_json::Map::new();
    for (k, v) in m {
        // 语言块（chinese/english/russian/...）：值为对象且含 name/description 字符串
        if let Some(o) = v.as_object() {
            if let Some(n) = o.get("name").and_then(|x| x.as_str()) {
                if !n.trim().is_empty() && k != "name" {
                    names.insert(k.clone(), json!(n));
                }
            }
            if let Some(d) = o.get("description").and_then(|x| x.as_str()) {
                if !d.trim().is_empty() && k != "description" {
                    descs.insert(k.clone(), json!(d));
                }
            }
        }
    }
    // 顶层 name/description（VCMI 基准语言为英文）→ english 候选
    if let Some(n) = m.get("name").and_then(|x| x.as_str()) {
        if !n.trim().is_empty() {
            names.entry("english".to_string()).or_insert(json!(n));
        }
    }
    if let Some(d) = m.get("description").and_then(|x| x.as_str()) {
        if !d.trim().is_empty() {
            descs.entry("english".to_string()).or_insert(json!(d));
        }
    }
    let to_v = |mp: serde_json::Map<String, Value>| {
        if mp.is_empty() {
            None
        } else {
            Some(Value::Object(mp))
        }
    };
    (to_v(names), to_v(descs))
}

/// 读取地形条目（VCMI WikiWindow.cpp:660-696）：
/// 名称 = 原版走 h3-core 的 `terrain.core.<id>.name`，mod 地形走 `terrain.<scope>.<id>.name`（含配置内嵌 name）；
/// 图标 = `tiles` DEF 的首帧；
/// 展示内容 = 移动消耗（`moveCost`，负数为不可通行）+ 以该地形为本族的城镇。
///
/// 数据来源 = 游戏根 `config/terrains.json`（原版 10 个）+ mod 归一配置的 `terrains` / `rivers`
/// （实测 hota.highlands、cradle、asphalt terrain 等 49 个地形只在 mod 内定义——
/// 此前只读游戏根，百科地形页永远只有原版 10 个）。
pub(crate) fn read_terrain_types(
    game_dir: &Path,
    raw_cfgs: &serde_json::Map<String, Value>,
    i18n: &I18nMap,
    scope: &str,
    native_towns: &std::collections::HashMap<String, Vec<String>>,
) -> Vec<Value> {
    let mut out = Vec::new();
    // id → out 索引：同名地形取"名称更完整"的一份（中文优先），
    // 避免因 mod 遍历顺序不同导致同一地形有时有中文、有时回退成 id
    let mut seen: std::collections::HashMap<String, usize> = std::collections::HashMap::new();

    // 1) 游戏根地形（原版 10 个，所有 mod 共用）：
    //    名称固定走 h3-core 的 `terrain.core.<id>.name`（不能用 mod 的 scope，
    //    否则第三方 mod 页面里原版地形名会回落成 dirt/grass 这类 id）
    let path = game_dir.join("config").join("terrains.json");
    if let Ok(text) = fs::read_to_string(&path) {
        if let Value::Object(m) = parse_vcmi_json(&text) {
            for (id, obj) in m {
                push_terrain(&mut out, &mut seen, &id, &obj, i18n, "core", native_towns);
            }
        }
    }

    // 2) mod 新增地形：归一配置的 `terrains`（`config/**/terrains.json`）。
    //    不取 `rivers`/`obstacles`/`roads`——它们是河流/障碍物/道路定义，会被地形页误列。
    if let Some(map) = raw_cfgs.get("terrains").and_then(|x| x.as_object()) {
        for (id, obj) in map {
            // 容器下钻：`{ "terrains": { "<id>": {...} } }`（cradle 等写法）
            if matches!(id.as_str(), "terrains" | "terrain") {
                if let Some(inner) = obj.as_object() {
                    for (id2, obj2) in inner {
                        push_terrain(&mut out, &mut seen, id2, obj2, i18n, scope, native_towns);
                    }
                }
                continue;
            }
            push_terrain(&mut out, &mut seen, id, obj, i18n, scope, native_towns);
        }
    }
    out.sort_by_key(|v| v.get("index").and_then(|x| x.as_i64()).unwrap_or(i64::MAX));
    out
}

/// 追加一条地形条目（id 去重，**中文名优先保留**；`index` -1 排到末尾）。
fn push_terrain(
    out: &mut Vec<Value>,
    seen: &mut std::collections::HashMap<String, usize>,
    id: &str,
    obj: &Value,
    i18n: &I18nMap,
    scope: &str,
    native_towns: &std::collections::HashMap<String, Vec<String>>,
) {
    // 跨 mod 引用（`core:0` / `core:mine`）不是地形自身定义，跳过
    if !obj.is_object() || id.is_empty() || id.contains(':') {
        return;
    }
    let key = if scope == "core" {
        format!("terrain.core.{}.name", id)
    } else {
        format!("terrain.{}.{}.name", scope, id)
    };
    let name = crate::modcache::i18n::name_obj_with_inline(i18n, &key, obj.get("name"), id);
    let index = obj.get("index").and_then(|v| v.as_i64()).unwrap_or(-1);
    let move_cost = obj.get("moveCost").and_then(|v| v.as_i64()).unwrap_or(-1);
    let tiles = obj.get("tiles").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let towns = native_towns.get(id).cloned().unwrap_or_default();
    let entry = json!({
        "id": id,
        "index": index,
        "name": name,
        "moveCost": move_cost,
        "tiles": tiles,
        "nativeTowns": towns,
    });
    // 同名地形可能被多个 mod 定义（父 mod + 子 mod）：**多语言合并**而非二选一——
    // 不同定义可能各带部分语言（A 有中文、B 有英文），合并后任意界面语言都能取到；
    // 不做"某语言优先"（会偏袒中文/外文，对以外文为主的模组不公平）。
    let merge_names = |a: &Value, b: &Value| -> Value {
        let mut m = serde_json::Map::new();
        if let Some(ao) = a.as_object() {
            for (k, v) in ao {
                m.insert(k.clone(), v.clone());
            }
        }
        if let Some(bo) = b.as_object() {
            for (k, v) in bo {
                // 已有值为空或就是实体 id 原文时，用新值替换（语言中立的质量规则）
                let replace = m
                    .get(k)
                    .and_then(|x| x.as_str())
                    .map(|old| {
                        let v_str = v.as_str().unwrap_or("");
                        (old.trim().is_empty() || old == id) && !v_str.trim().is_empty() && v_str != id
                    })
                    .unwrap_or(false);
                if replace || !m.contains_key(k) {
                    m.insert(k.clone(), v.clone());
                }
            }
        }
        Value::Object(m)
    };
    match seen.get(id) {
        Some(&ix) => {
            let merged = merge_names(&out[ix]["name"], &entry["name"]);
            if let Some(o) = out[ix].as_object_mut() {
                o.insert("name".to_string(), merged);
                // 版本更完整者补 index / tiles / moveCost（-1 / 空视为缺失）
                if o.get("index").and_then(|x| x.as_i64()).unwrap_or(-1) < 0 {
                    if let Some(v) = entry.get("index") {
                        o.insert("index".to_string(), v.clone());
                    }
                }
                if o.get("moveCost").and_then(|x| x.as_i64()).unwrap_or(-1) < 0 {
                    if let Some(v) = entry.get("moveCost") {
                        o.insert("moveCost".to_string(), v.clone());
                    }
                }
                if o.get("tiles").and_then(|x| x.as_str()).unwrap_or("").is_empty() {
                    if let Some(v) = entry.get("tiles") {
                        o.insert("tiles".to_string(), v.clone());
                    }
                }
            }
        }
        None => {
            seen.insert(id.to_string(), out.len());
            out.push(entry);
        }
    }
}
