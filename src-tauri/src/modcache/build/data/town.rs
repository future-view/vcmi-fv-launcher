//! 城镇（faction）缓存构建：城镇条目 + 建筑列表 + 背景 / 预览字段。
//!
//! 由 `build_mod_wiki_once` 的「城镇」节原样搬出（S3）；参数与原局部变量同名，节内容零改动。

use std::collections::HashMap;
use std::path::Path;

use serde_json::{json, Value};

use crate::modcache::assets::{
    extract_border_and_save, extract_def_frame_and_save, extract_icon_and_save,
    save_hd_or_raw_image, save_wiki_image,
};
use crate::modcache::i18n::{build_name_object, i18n_mod_scope, name_obj_with_inline};
use crate::modcache::preview::{composite_structures_to_canvas_hd, load_structure_frame};
use crate::vcmi::h3core::I18nMap;
use crate::vcmi::resource::resolve_resource_fallback;
use crate::vcmi::town::{value_to_resource, TownConfig};

/// 城镇自带 `description` → 多语言对象。
///
/// 两件必须做的事：
/// 1. **字符串形态同时写入 chinese / english**。配置里的裸字符串只可能是作者写的原文
///    （这些模组没有翻译文件），若只塞进 `english`，界面语言为中文时 `pickName(...,"chinese")`
///    会取空 → 描述整段消失。两端都写，任何界面语言都取得到。
/// 2. **去掉 VCMI MetaString 的 `{标题}` 花括号**。VCMI 把 `{堡垒}` 渲染成高亮标题，
///    我们是纯文本渲染，保留花括号会原样显示 `{堡垒}`。只剥一层括号、保留内容。
fn normalize_town_desc(v: &Value) -> Option<Value> {
    let strip = |s: &str| -> String {
        let t = s.trim();
        if let Some(rest) = t.strip_prefix('{') {
            if let Some((head, tail)) = rest.split_once('}') {
                return format!("{head}{tail}");
            }
        }
        t.to_string()
    };
    match v {
        // **裸字符串原样返回**（只剥花括号），不在这里写任何语言键：
        // 交给共用的 `name_obj_with_inline` 按"VCMI 翻译基准语言"单键注入，
        // 前端 `pickName` 的 `当前语言 → english` 回退链会兜住中文界面。
        // 若在这里枚举具体语言（chinese/english/…），就等于把语言集合写死 ——
        // 界面语言会继续增加（当前 5 种），写死的那天就是新语言静默失效的那天。
        Value::String(s) if !s.trim().is_empty() => Some(json!(strip(s))),
        Value::Object(m) if !m.is_empty() => {
            let mut out = serde_json::Map::new();
            for (k, x) in m {
                if let Some(s) = x.as_str() {
                    out.insert(k.clone(), json!(strip(s)));
                }
            }
            if out.is_empty() {
                None
            } else {
                Some(Value::Object(out))
            }
        }
        _ => None,
    }
}

/// `mod.json` 的模组名是否只是**标识符**（不能当名字用）。
///
/// 很多 H3 时代的城镇模组把 `mod.json` 的 `name` 直接写成了阵营 id（或它的变体）
/// —— 实测 `132mod v6.0` 发行版 123 个阵营里有 83 个如此（`csknight` / `blazers` /
/// `oldpavilion` / `stundertaker`）。把它们当英文名填进 `english` 槽，英文界面就会显示
/// 一串 id，比配置里的中文原文更难读（这正是 2.x 缓存的表现：`骑士` 的英文列是 `csknight`）。
///
/// 判据两条**同时**成立（两条都不涉及“猜测语言”）：
/// - 归一化（只留字母数字、转小写）后与 faction id 相同 —— 不含任何新信息；
/// - 原串没有大写字母 —— 真正以 id 为名的英文名常带首字母大写（`Bulwark`），不能误杀。
fn mod_name_is_identifier(name: &str, faction_id: &str) -> bool {
    let norm = |s: &str| -> String {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .map(|c| c.to_ascii_lowercase())
            .collect()
    };
    norm(name) == norm(faction_id) && !name.chars().any(|c| c.is_uppercase())
}

/// 阵营名的**双源合并**：faction 配置的 `name`（VCMI 的 `CFaction::getName()`）
/// + `mod.json` 的模组名（多语言块 / `name`）。
///
/// 为什么要合并：城镇模组常只写一种语言的 faction 名（`1.83` 发行版 13 座城镇只写了中文），
/// 而英文名常常只存在于 `mod.json` 的**模组名**里（`Asylum Town` / `Cove town` / `Pavilion Town`）。
/// 两边各填对方的空槽，中英界面才能都读到可用文本：
/// - `chinese`：`mod.json` 中文块 → faction 配置名
/// - `english`：`mod.json` 英文名（非 id 形态时）→ faction 配置的英文键
///
/// 优先级“真实翻译 > 模组名 > 配置原文”而不是“配置一律优先”：配置里的裸字符串是
/// **语言未知的原文**（VCMI 里它就是个字面量，任何语言界面都原样显示），只补**空槽**，
/// 不会顶掉 `mod.json` 已给出的中文名。反过来（模组名压制配置名）则会让中文界面只剩
/// 英文（实测 Cove / Factory / Bulwark / Pavilion → “Cove town” / “Pavilion Town”）。
fn merge_faction_name_inline(
    town: Option<&Value>,
    mod_name: Option<&Value>,
    faction_id: &str,
) -> Option<Value> {
    let mut out = serde_json::Map::new();
    let mut put = |k: &str, s: &str, only_empty: bool| {
        if s.trim().is_empty() || (only_empty && out.contains_key(k)) {
            return;
        }
        out.insert(k.to_string(), json!(s));
    };
    // 1) mod.json 语言块：真实多语言名，优先；但**形如 id 的模组名不能当名字**
    //    （否则英文列会变成 `csknight`，见 `mod_name_is_identifier`）
    let usable_mod_name = |k: &str, s: &str| -> bool {
        !(matches!(k, "english" | "auto") && mod_name_is_identifier(s, faction_id))
    };
    match mod_name {
        Some(Value::Object(o)) => {
            for (k, v) in o {
                if let Some(s) = v.as_str() {
                    if usable_mod_name(k, s) {
                        put(k, s, false);
                    }
                }
            }
        }
        Some(Value::String(s)) => {
            if usable_mod_name("english", s) {
                put("english", s, false);
            }
        }
        _ => {}
    }
    // 2) faction 配置名补空槽
    match town {
        Some(Value::Object(o)) => {
            for (k, v) in o {
                if let Some(s) = v.as_str() {
                    put(k, s, true);
                }
            }
        }
        Some(Value::String(s)) => {
            // 原文语言未知：先补中文槽（否则中文界面会回退到英文模组名），再补英文槽
            put("chinese", s, true);
            put("english", s, true);
        }
        _ => {}
    }
    if out.is_empty() {
        None
    } else {
        Some(Value::Object(out))
    }
}

pub(crate) fn build_towns(
    p: &Path,
    mod_id: &str,
    cache_root: &Path,
    game_dir: &Path,
    configs: &[TownConfig],
    i18n: &I18nMap,
    creature_dwellings: &mut HashMap<String, Value>,
    creature_jsons: &HashMap<String, Value>,
    // mod.json 多语言文本块（见 `build::mod_lang_texts`）：**单城镇 mod** 的城镇名/描述兜底
    mod_lang_name: Option<&Value>,
    mod_lang_desc: Option<&Value>,
) -> (Vec<Value>, Vec<Value>) {
    // 只有单城镇 mod 才能用 mod.json 的元数据文本兜底——
    // 多城镇 mod 的 mod 描述与具体某个城镇无关，混用会张冠李戴。
    let single_town = configs.len() == 1;
    let mod_name_inline = if single_town { mod_lang_name } else { None };
    let mod_desc_inline = if single_town { mod_lang_desc } else { None };
    let mut factions = Vec::new();
    // 建筑图标提取失败诊断（见下方 buildings 循环；随 cache.json 的 `iconFailures` 输出）
    let mut icon_failures: Vec<Value> = Vec::new();
    for cfg in configs {
        // 城镇自带描述 → 多语言对象（字符串形态同时写入 chinese/english，见 normalize_town_desc）
        let town_desc_norm = cfg.town_desc.as_ref().and_then(normalize_town_desc);
        let mut bg_img = None;
        if let Some(res) = &cfg.town_background {
            // 原版城镇背景（如 castle 的 TBCSBACK.bmp）位于游戏根资源包，需回退游戏根解析
            if let Some((buf, ext)) = resolve_resource_fallback(&p, game_dir, res) {
                // 有建筑结构（原版城镇：townBackground 仅为裸底图）→ 叠加建筑精灵生成合成背景；
                // 无结构（多数 mod 城镇：townBackground 已预合成，含建筑）→ 直接用作背景。
                // 走 `_hd` 变体：底图命中官方 HD 素材时，整张幕布按 HD 档位合成（选项 A），
                // 未命中则内部逐字节回退原版 1x 路径。
                bg_img = if !cfg.structures.is_empty() {
                    composite_structures_to_canvas_hd(
                        &p,
                        game_dir,
                        res,
                        &buf,
                        &ext,
                        &cfg.structures,
                        &cfg.buildings_raw,
                    )
                    .and_then(|webp| {
                        save_wiki_image(cache_root, mod_id, "town", &cfg.faction_id, &webp, "webp")
                    })
                    .or_else(|| {
                        save_wiki_image(cache_root, mod_id, "town", &cfg.faction_id, &buf, &ext)
                    })
                } else {
                    save_wiki_image(cache_root, mod_id, "town", &cfg.faction_id, &buf, &ext)
                };
            }
        }
        let mut cr_img = None;
        if let Some(res) = &cfg.creature_background {
            // 原版兵种背景（如 TPCASCAS / CRBKGCAS）位于游戏根资源包，需回退游戏根解析；
            // 命中官方 HD 版（`TPCAS*` 均在 pak 内）时改用 HD 图（有损 q85）
            if let Some((buf, ext)) = resolve_resource_fallback(&p, game_dir, res) {
                cr_img = save_hd_or_raw_image(
                    cache_root,
                    mod_id,
                    "town",
                    &format!("{}-creature-bg", cfg.faction_id),
                    res,
                    &buf,
                    &ext,
                );
            }
        }
        // 建筑表（id + 多语言名 + 花费 + 描述 + 图标）
        let scope = i18n_mod_scope(mod_id);
        let mut buildings = Vec::new();
        for bid in &cfg.building_ids {
            // 建筑多语言名称：直接采用 VCMI 语言 key；
            // 英文缺失时回退 town.json 原文（VCMI 翻译基准语言为英文，原文即英文）
            let detail = cfg
                .buildings_detail
                .iter()
                .find(|d| d.get("id").and_then(|x| x.as_str()) == Some(bid.as_str()));
            let raw_name_v = detail.and_then(|d| d.get("rawName")).cloned();
            let bname = build_name_object(
                &i18n,
                &format!("building.{}.{}.{}.name", scope, cfg.faction_id, bid),
                raw_name_v.as_ref(),
            );
            let raw_desc_v = detail.and_then(|d| d.get("rawDesc")).cloned();
            let bdesc = build_name_object(
                &i18n,
                &format!("building.{}.{}.{}.description", scope, cfg.faction_id, bid),
                raw_desc_v.as_ref(),
            );
            let cost = detail
                .and_then(|d| d.get("cost").cloned())
                .unwrap_or(json!({}));
            // 建筑图标提取优先级（对齐 VCMI 百科建筑表 WikiTownContent.cpp:262-265）：
            // 1) clientInfo.buildingsIcons 帧[bid]（VCMI 百科口径，与 VCMI 内百科截图一致）
            // 2) town.structures[id].animation 首帧（城镇界面大图，带背景场景）
            // 3) 建筑自带 animation / image
            // 4) **升级链回退**：本建筑四通道全失败且声明了 `upgrades` 时，取被升级建筑
            //    （base）的图标 —— 见下方 `bimg_path` 的收尾分支。
            let bimg_path = {
                // 单建筑的图标提取（structureAnim / animation / image 三条通道）。
                // 抽成闭包：第 4 级回退要复用同一套逻辑，只是换成 base 建筑的 detail。
                let extract_one = |detail_v: &Value, icon_name: &str| -> Option<String> {
                    let st_anim = detail_v
                        .get("structureAnim")
                        .and_then(|x| x.as_str())
                        .map(|s| s.to_string());
                    let banim = detail_v
                        .get("animation")
                        .and_then(|x| x.as_str())
                        .map(|s| s.to_string());
                    let bgroup =
                        detail_v.get("group").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
                    let bindex =
                        detail_v.get("index").and_then(|x| x.as_u64()).unwrap_or(0) as usize;
                    let bimg = detail_v
                        .get("image")
                        .and_then(|x| x.as_str())
                        .map(|s| s.to_string());
                    if let Some(sa) = st_anim {
                        // structureAnim 支持 sprites-json（*.json 整画布帧）与 def 双格式
                        let core = game_dir.join("Mods").join("vcmi");
                        match load_structure_frame(&p, &core, &game_dir, &sa) {
                            Some((img, _lm, _tm)) => {
                                let mut webp = Vec::new();
                                let enc = image::codecs::webp::WebPEncoder::new_lossless(&mut webp);
                                if enc
                                    .encode(
                                        img.as_raw(),
                                        img.width(),
                                        img.height(),
                                        image::ExtendedColorType::Rgba8,
                                    )
                                    .is_ok()
                                {
                                    save_wiki_image(
                                        cache_root, mod_id, "building", icon_name, &webp, "webp",
                                    )
                                } else {
                                    None
                                }
                            }
                            None => extract_def_frame_and_save(
                                &p, game_dir, cache_root, mod_id, "building", icon_name, &sa, 0, 0,
                            ),
                        }
                    } else if let Some(a) = banim {
                        extract_def_frame_and_save(
                            &p, game_dir, cache_root, mod_id, "building", icon_name, &a, bgroup,
                            bindex,
                        )
                    } else if let Some(i) = bimg {
                        extract_icon_and_save(
                            &p,
                            game_dir,
                            cache_root,
                            mod_id,
                            "building",
                            icon_name,
                            Some(&i),
                        )
                    } else {
                        None
                    }
                };
                let bid_num = detail
                    .and_then(|d| d.get("bid"))
                    .and_then(|x| x.as_i64())
                    .unwrap_or(-1);
                let icon_name = format!("{}-{}", cfg.faction_id, bid);
                // 1) buildingsIcons：按**资源真实类型**分派（sprites-json / DEF）——
                //    支持无扩展名引用、zip 内定义、images/sequences 两种 sprites-json 形态、
                //    file 自带扩展名（bmp/png）与 basepath 缺省（见 assets::extract_sprite_frame_and_save）。
                //    旧实现按"配置字符串后缀"选解析器：实测 xanadu（无扩展名→实际 json）、
                //    cove/bulwark（无扩展名）、christmas（写 .def 实际 json）→ 5 个城镇建筑图标全空。
                //
                //    该通道本身已含"帧号取不到 → 沿升级链换 bid 重试"：VCMI 客户端对升级
                //    建筑按 **base** 的 bid 取帧（`CBuilding::getBase()` / `CCastleBuildings::
                //    recreate` 的 base 分组），而 mod 常只给升级建筑一个**越界的 bid**
                //    （如 lianjin 的 dwellingUpLvl2 = 38，buildingsIcons 里没有该帧）。
                let mut from_icons = cfg.buildings_icons.as_deref().and_then(|icons_ref| {
                    crate::modcache::assets::extract_sprite_frame_and_save(
                        &p,
                        game_dir,
                        cache_root,
                        mod_id,
                        "building",
                        &icon_name,
                        icons_ref,
                        bid_num.max(0) as usize,
                    )
                });
                if from_icons.is_none() {
                    if let Some(icons_ref) = cfg.buildings_icons.as_deref() {
                        // 沿 `upgrades` 链走到链尾（= VCMI 的 getBase()），逐级换 bid 重试。
                        // 上限 8 级防配置写环时死循环。
                        let mut cur = detail
                            .and_then(|d| d.get("upgrades"))
                            .and_then(|x| x.as_str())
                            .map(|s| s.to_string());
                        for _ in 0..8 {
                            let Some(up_id) = cur.filter(|s| !s.is_empty()) else {
                                break;
                            };
                            let Some(up) = cfg.buildings_detail.iter().find(|d| {
                                d.get("id").and_then(|x| x.as_str()) == Some(up_id.as_str())
                            }) else {
                                break;
                            };
                            let up_bid = up.get("bid").and_then(|x| x.as_i64()).unwrap_or(-1);
                            if let Some(hit) =
                                crate::modcache::assets::extract_sprite_frame_and_save(
                                    &p,
                                    game_dir,
                                    cache_root,
                                    mod_id,
                                    "building",
                                    &icon_name,
                                    icons_ref,
                                    up_bid.max(0) as usize,
                                )
                            {
                                from_icons = Some(hit);
                                break;
                            }
                            cur = up
                                .get("upgrades")
                                .and_then(|x| x.as_str())
                                .map(|s| s.to_string());
                        }
                    }
                }
                if from_icons.is_some() {
                    from_icons
                } else if let Some(d) = detail {
                    extract_one(d, &icon_name)
                } else {
                    None
                }
                .or_else(|| {
                    // 4) 升级链回退：本建筑（含 buildingsIcons 通道）全失败，
                    //    且声明了 `upgrades` → 用**被升级建筑**的图标。
                    //    依据：VCMI `CBuilding::getBase()`（lib/entities/building/CBuilding.cpp:61）
                    //    沿 `upgrade` 链走到链尾；`CCastleBuildings::recreate`
                    //    （client/windows/CCastleInterface.cpp:771）按 base 分组绘制建筑
                    //    —— 即升级建筑与底座在客户端是同一组图形。
                    //    实测：lianjin 的 dwellingUpLvl2（高级魁地奇）无 image 且
                    //    buildingsIcons 无 bid=38 帧，而其底座 dwellingLvl2（魁地奇）两者都有。
                    let mut cur = detail
                        .and_then(|d| d.get("upgrades"))
                        .and_then(|x| x.as_str())
                        .map(|s| s.to_string());
                    for _ in 0..8 {
                        let Some(up_id) = cur.filter(|s| !s.is_empty()) else {
                            break;
                        };
                        let Some(up) = cfg
                            .buildings_detail
                            .iter()
                            .find(|d| d.get("id").and_then(|x| x.as_str()) == Some(up_id.as_str()))
                        else {
                            break;
                        };
                        // 产物文件名沿用**本建筑**的 icon_name：前端按
                        // `<mod>/images/building/<faction>-<bid>.webp` 取值，改名会静默丢图。
                        if let Some(hit) = extract_one(up, &icon_name) {
                            return Some(hit);
                        }
                        cur = up
                            .get("upgrades")
                            .and_then(|x| x.as_str())
                            .map(|s| s.to_string());
                    }
                    None
                })
            };
            // 该建筑招募的生物 → 记录建筑名（多语言），供生物页面 Dwellings 显示
            let bc = detail
                .and_then(|d| d.get("creatures"))
                .and_then(|x| x.as_array())
                .cloned()
                .unwrap_or_default();
            for c in bc {
                if let Some(cid) = c.as_str() {
                    let entry = creature_dwellings
                        .entry(cid.to_string())
                        .or_insert_with(|| json!({}));
                    // creature_dwellings 用 VCMI 语言 key 组织，与 build_name_object 对齐
                    if let Some(map) = entry.as_object_mut() {
                        for (lang, text) in bname.as_object().unwrap_or(&serde_json::Map::new()) {
                            if text.is_string() {
                                map.entry(lang.clone())
                                    .or_insert_with(|| json!([]))
                                    .as_array_mut()
                                    .map(|arr| arr.push(text.clone()));
                            }
                        }
                    }
                }
            }
            // 诊断：声明了 buildingsIcons 但多通道（icons → structureAnim → 建筑 animation → image）
            // 全部失败 → 记录（此前完全静默：运行日志 ERROR/WARN=0，用户只能肉眼发现缺图）。
            // 产出到 cache.json 的 `iconFailures` 并由批次 WARN 汇总。
            if bimg_path.is_none()
                && cfg.buildings_icons.is_some()
                && crate::modcache::assets_enabled()
            {
                icon_failures.push(json!({
                    "factionId": cfg.faction_id,
                    "buildingId": bid,
                    "bid": detail.and_then(|d| d.get("bid")).and_then(|x| x.as_i64()).unwrap_or(-1),
                    "ref": cfg.buildings_icons.as_deref(),
                }));
            }
            buildings.push(json!({
                "id": bid,
                // VCMI building mode：normal/auto/special/grail（wiki 表只显示 normal）
                "mode": detail.and_then(|d| d.get("mode")).and_then(|x| x.as_str()).unwrap_or("normal"),
                // 建筑数字 id（VCMI bid，用于排序）
                "bid": detail.and_then(|d| d.get("bid")).and_then(|x| x.as_i64()).unwrap_or(-1),
                "name": bname,
                "description": bdesc,
                "cost": cost,
                "image": bimg_path,
            }));
        }
        // 扩展 creatureList：把基础生物的 upgrades 也加入，便于百科显示升级链
        let mut creature_list = cfg.creature_ids.clone();
        for base_id in &cfg.creature_ids {
            if let Some(obj) = creature_jsons.get(base_id) {
                if let Some(upgs) = obj.get("upgrades").and_then(|x| x.as_array()) {
                    for u in upgs {
                        if let Some(uid) = u.as_str() {
                            if !creature_list.iter().any(|x| x == uid) {
                                creature_list.push(uid.to_string());
                            }
                        }
                    }
                }
            }
        }
        // 阵营善恶颜色映射
        let alignment = cfg.alignment.clone();
        // 城镇结构 + 金色描边（hover 高亮罩住整栋建筑，设计 §5.9）：
        // 只输出**有 border 图**的结构，携带幕布坐标 (x, y) 与描边图原始 (w, h)。
        // 前端把 border 图按 (x, y) + (w, h) 叠加到同尺寸幕布（原版 800×374）即精确贴合；
        // 无 border 的结构（装饰/部分 extraAnimation）不产出——原版 likewise 不高亮。
        // 坐标非 0 的（原版）与 x/y=0 的（新式 sprites-json 整画布帧）在这里是统一的，
        // 前者 border 图小、后者 border 图整画布，前端一律「原样叠在原位」即可。
        let mut structures_out: Vec<Value> = Vec::new();
        {
            let mut skeys: Vec<&String> = cfg.structures.keys().collect();
            skeys.sort();
            for skey in skeys {
                let sv = &cfg.structures[skey];
                let Some(bres) = sv.get("border").and_then(value_to_resource) else {
                    continue;
                };
                if let Some((img, w, h)) = extract_border_and_save(
                    &p,
                    game_dir,
                    cache_root,
                    mod_id,
                    &cfg.faction_id,
                    skey,
                    &bres,
                ) {
                    structures_out.push(json!({
                        "id": skey,
                        "x": sv.get("x").and_then(|v| v.as_i64()).unwrap_or(0),
                        "y": sv.get("y").and_then(|v| v.as_i64()).unwrap_or(0),
                        "z": sv.get("z").and_then(|v| v.as_i64()).unwrap_or(0),
                        "w": w,
                        "h": h,
                        "image": img,
                    }));
                }
            }
        }
        let native_terrain = cfg.native_terrain.clone();
        // 阵营名：faction 配置名 + mod.json 模组名的双源合并（见 `merge_faction_name_inline`）
        let name_inline =
            merge_faction_name_inline(cfg.town_name.as_ref(), mod_name_inline, &cfg.faction_id);
        factions.push(json!({
            "factionId": cfg.faction_id,
            // 名称优先级：mod 翻译（i18n）→ 模组名 / 配置名合并体 → faction id。
            //
            // 两侧各有所长，缺一边中文或英文就有一边读不到：
            // - faction 配置名：VCMI 真正显示的名字（`CFaction::getName()`），
            //   本发行版的 13 座第三方城镇只在这里写了中文（港口 / 工厂 / 棱堡 / 沙堡…）；
            // - mod.json 模组名：英文常只在这里（`Asylum Town` / `Cove town` / `Pavilion Town`）。
            // 合并规则见 `merge_faction_name_inline` 的文档注释。
            "name": name_obj_with_inline(
                &i18n,
                &format!("faction.{}.{}.name", scope, cfg.faction_id),
                name_inline.as_ref(),
                &cfg.faction_id,
            ),
            // 阵营描述（VCMI: faction.<mod>.<id>.description，合成图下方段落）。
            // 兜底链：翻译键 → **城镇自带 description** → 单城镇 mod 的 mod.json 语言块 → 空。
            // ⚠️ 城镇自带必须优先：中文城镇模组常把「种族阵营 / 种族地形 / 所在地 + 正文」
            // 整段写进 town.json 的 description（实测 bastille 有 4 行设定），而 mod.json 的
            // description 只是一句简介（"EXP中的堡垒种族"）—— 用后者会把整段顶掉。
            "description": name_obj_with_inline(
                &i18n,
                &format!("faction.{}.{}.description", scope, cfg.faction_id),
                town_desc_norm.as_ref().or(mod_desc_inline),
                "",
            ),
            "background": bg_img,
            "creatureBackground": cr_img,
            "creatureList": creature_list,
            "buildings": buildings,
            "alignment": alignment,
            "nativeTerrain": native_terrain,
            // 城镇界面建筑动画 DEF 资源名（<bid> → "<file>.def"）。
            // 仅文本引用，前端在详情页按需 `getDefGroup` 懒加载播放（设计 §11）；
            // 构建期不解码，避免 2452 个原版 .def 拖慢后台任务。
            "buildingAnimations": cfg.building_animations,
            // 城镇结构定位 + 金色描边图（hover 高亮罩住整栋建筑，设计 §5.9）：
            // [{ id, x, y, z, w, h, image }]，坐标为原版幕布像素（800×374）
            "structures": structures_out,
        }));
    }
    (factions, icon_failures)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 形如 id 的模组名不得当英文名（实测 132mod 发行版 83/123 个阵营如此：
    /// `csknight` / `blazers` / `oldpavilion`），否则英文界面显示一串 id。
    #[test]
    fn identifier_like_mod_name_is_rejected() {
        assert!(mod_name_is_identifier("csknight", "csknight"));
        assert!(mod_name_is_identifier("stundertaker", "stundertaker"));
        assert!(mod_name_is_identifier("oldpavilion", "oldpavilion"));
        // 带首字母大写的同名是真英文名（Expansion 的 Bulwark），不能误杀
        assert!(!mod_name_is_identifier("Bulwark", "bulwark"));
        assert!(!mod_name_is_identifier("Cemetary", "cemetary"));
        // 不是 id → 保留（Cove town / Pavilion Town / Abyss town）
        assert!(!mod_name_is_identifier("Cove town", "cove"));
        assert!(!mod_name_is_identifier("Abyss town", "abyss"));
    }

    /// 合并口径：中文取配置名、英文取可用的模组名；模组名是 id 时两边都是配置名。
    #[test]
    fn merge_prefers_usable_mod_name_for_english() {
        let town = json!("沙堡");
        let merged = merge_faction_name_inline(
            Some(&town),
            Some(&json!({"english": "Pavilion Town"})),
            "pavilion",
        )
        .unwrap();
        assert_eq!(merged["chinese"], "沙堡");
        assert_eq!(merged["english"], "Pavilion Town");

        // 模组名 == 阵营 id → 丢弃，英文退回配置原文（与 VCMI 一致：英文界面显示配置名）
        let merged = merge_faction_name_inline(
            Some(&town),
            Some(&json!({"english": "oldpavilion"})),
            "oldpavilion",
        )
        .unwrap();
        assert_eq!(merged["chinese"], "沙堡");
        assert_eq!(merged["english"], "沙堡");
    }
}
