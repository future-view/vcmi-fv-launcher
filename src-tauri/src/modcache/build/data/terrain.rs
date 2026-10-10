//! 地形（terrain）缓存构建：名称 / 移动消耗 / 图标 / 归属城镇。
//!
//! 由 `build_mod_wiki_once` 的「地形」节原样搬出（S3）；返回 `(terrains, failed_previews)`。

use std::path::Path;

use serde_json::{json, Value};

use crate::vcmi::h3core::I18nMap;
use crate::vcmi::town::TownConfig;
use crate::modcache::assets::{extract_game_def_frame_and_save, extract_icon_and_save};
use crate::modcache::preview::compose_town_preview_for;
use crate::modcache::assets_enabled;

use crate::modcache::build::read_terrain_types;

pub(crate) fn build_terrains(
    p: &Path,
    mod_id: &str,
    cache_root: &Path,
    game_dir: &Path,
    i18n: &I18nMap,
    raw_cfgs: &serde_json::Map<String, Value>,
    configs: &[TownConfig],
    factions: &mut [Value],
) -> (Vec<Value>, Vec<String>) {
    // 本族地形 → 城镇名（VCMI 在地形条目描述里列出以该地形为本族的城镇）
    let mut terrain_towns: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
    for c in configs {
        let fname = factions
            .iter()
            .find(|f| f.get("factionId").and_then(|x| x.as_str()) == Some(c.faction_id.as_str()))
            .and_then(|f| f.get("name"))
            .and_then(|n| n.get("chinese").or_else(|| n.get("auto")).or_else(|| n.get("english")))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if c.native_terrain.is_empty() || fname.is_empty() {
            continue;
        }
        terrain_towns
            .entry(c.native_terrain.clone())
            .or_default()
            .push(fname);
    }
    let scope = crate::modcache::i18n::i18n_mod_scope(mod_id);
    let mut terrains = read_terrain_types(game_dir, raw_cfgs, &i18n, scope, &terrain_towns);
    // 地形图标：tiles DEF 首帧（VCMI 用 terrainViewPatterns 的 "n1" 帧，此处取首帧近似）。
    // mod 地形（hota.highlands / dunes terrain 等）的 tiles 在 mod 自己的 resources 里，
    // 先走 mod 资源查找，再回退游戏资源。
    for t in terrains.iter_mut() {
        let id = t.get("id").and_then(|x| x.as_str()).unwrap_or("").to_string();
        let tiles = t.get("tiles").and_then(|x| x.as_str()).unwrap_or("").to_string();
        if id.is_empty() || tiles.is_empty() {
            continue;
        }
        let def = format!("{tiles}.DEF");
        let img = extract_icon_and_save(&p, game_dir, cache_root, mod_id, "terrain", &id, Some(&def))
            .or_else(|| {
                extract_game_def_frame_and_save(game_dir, cache_root, mod_id, &def, 0, 0, "terrain", &id)
            });
        if let Some(rel) = img {
            if let Some(o) = t.as_object_mut() {
                o.insert("image".into(), json!(rel));
            }
        }
    }
    // 城镇预览图：构建期合成（不再查看时实时生成）；失败列表进返回值便于诊断
    let mut failed_previews: Vec<String> = Vec::new();
    {
        let towns_v = Value::Object(raw_cfgs.clone());
        for f in factions.iter_mut() {
            let fid = f.get("factionId").and_then(|x| x.as_str()).unwrap_or("").to_string();
            if fid.is_empty() {
                continue;
            }
            // 城镇预览合成是最重的一步（位图解码 + 建筑图层叠加），阶段一跳过
            if !assets_enabled() {
                continue;
            }
            // `build_towns` 已产出「底图 + 建筑」合成图（`images/town/<fid>.webp`），
            // 其 structures/buildings 取自**跨文件并集**，比 `compose_town_preview_for`
            // 的"单文件优先命中"口径更完整 → 直接复用为 preview，两字段指向同一文件。
            //
            // 此前两条路径各合成一遍、各落一份盘：实测 84 个城镇重复约 42 MB，
            // 且多花一遍最重的合成（建筑 DEF 解码 + 拼合 + 无损编码）。
            let bg = f.get("background").and_then(|x| x.as_str()).unwrap_or("").to_string();
            if !bg.is_empty() {
                if let Some(obj) = f.as_object_mut() {
                    obj.insert("preview".to_string(), json!(bg));
                }
                continue;
            }
            // 兜底：无合成背景（既无 townBackground 也无 clientInfo.townScreen.background）
            // 才独立合成到 `images/townscreen/`
            match compose_town_preview_for(&p, cache_root, mod_id, &fid, &towns_v) {
                Ok(rel) => {
                    if let Some(obj) = f.as_object_mut() {
                        obj.insert("preview".to_string(), json!(rel));
                    }
                }
                Err(stage) => {
                    failed_previews.push(format!("{}:{}", fid, stage));
                }
            }
        }
    }
    (terrains, failed_previews)
}
