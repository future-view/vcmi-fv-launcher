//! 游戏级共享素材提取（原版游戏资源，统一归位到父 `core` 模组的 `core/images/<sub>/` 下，
//! 收编进 core，使迁移只需拷贝 `core*` 目录）。各 mod 的 statIcons/sharedIcons 均引用此路径。
//!
//! 覆盖：属性图标（`stackWindow/*.png`）、共享 UI（`PortraitsLarge.DEF`、`SPELLBON.DEF` 等）、
//! 资源图标（`SMALRES.DEF`）。
//!
//! ⚠️ 这些是游戏级资源：各 mod 构建时调用会因幂等仅由首个 mod 实际付出提取代价，
//! 产物统一落在 `core/images/`（见 `vcmi-core-cache-restructure` §13.4/§16）。
//!
//! 依赖方向：可依赖 `infra` 与 `vcmi`（见 `docs/architecture/backend.zh-CN.md` §4.1）。

use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use crate::infra::image::{encode_webp, encode_webp_from_bgra};
use crate::modcache::assets::extract_game_def_frame_and_save;
use crate::modcache::assets_enabled;

/// 提取生物表 8 属性表头图标（游戏 Sprites/stackWindow/*.def 首帧），
/// 存 `cache_root/<mod>/images/stats/{key}.webp`，返回 {key: 相对路径}。
pub(crate) fn extract_stat_icons_for_mod(
    game_dir: &Path,
    cache_root: &Path,
    _mod_id: &str,
) -> Value {
    if !assets_enabled() {
        return Value::Object(serde_json::Map::new());
    }
    let defs: [(&str, &str); 8] = [
        ("level", "iconLevel"),
        ("attack", "iconAttack"),
        ("defense", "iconDefense"),
        ("damage", "iconDamage"),
        ("speed", "iconSpeed"),
        ("health", "iconHealth"),
        ("growth", "iconGrowth"),
        ("ai", "iconAI"),
    ];
    let mut out = serde_json::Map::new();
    // 原版游戏级资源统一归位到父 `core` 模组的 images/ 下（`_shared` 收编进 core，迁移只拷 core*）
    let dir = cache_root.join("core").join("images").join("stats");
    for (key, def_name) in defs {
        let rel = format!("core/images/stats/{}.webp", key);
        if dir.join(format!("{key}.webp")).is_file() {
            out.insert(key.to_string(), json!(rel));
            continue;
        }
        // stackWindow 图标为 png（VCMI 核心 mod 散文件）
        let bytes = match crate::vcmi::resource::read_game_resource_fallback(
            game_dir,
            &format!("stackWindow/{def_name}.png"),
        ) {
            Some(b) => b,
            None => continue,
        };
        if let Some(webp) = encode_webp(&bytes, "png") {
            let _ = fs::create_dir_all(&dir);
            if fs::write(dir.join(format!("{key}.webp")), &webp).is_ok() {
                out.insert(key.to_string(), json!(rel));
            }
        }
    }
    Value::Object(out)
}

/// 提取 VCMI 公共 UI 图标（原版游戏资源，各 mod 引用），统一存 `core/images/` 下各子目录。
/// 返回 { pskil42/attack: "core/images/pskil42/attack.webp", stackwin/shots: ..., spellbonDir: "core/images/spellbon", ... }
pub(crate) fn extract_shared_ui_icons(game_dir: &Path, cache_root: &Path, _mod_id: &str) -> Value {
    if !assets_enabled() {
        return Value::Object(serde_json::Map::new());
    }
    let mut out = serde_json::Map::new();

    // 1) stackWindow 弹窗属性图标：百科右键弹窗使用（shot/healthLeft/mana 为弹窗专用）
    let stackwin_dir = cache_root.join("core").join("images").join("stackwin");
    let stackwin_defs: [(&str, &str); 8] = [
        ("attack", "iconAttack"),
        ("defense", "iconDefense"),
        ("shots", "iconShots"),
        ("damage", "iconDamage"),
        ("health", "iconHealth"),
        ("healthLeft", "iconHealthLeft"),
        ("speed", "iconSpeed"),
        ("mana", "iconMana"),
    ];
    for (key, def_name) in stackwin_defs {
        let rel = format!("core/images/stackwin/{}.webp", key);
        if stackwin_dir.join(format!("{key}.webp")).is_file() {
            out.insert(format!("stackwin/{key}"), json!(rel));
            continue;
        }
        if let Some(bytes) = crate::vcmi::resource::read_game_resource_fallback(
            game_dir,
            &format!("stackWindow/{def_name}.png"),
        ) {
            if let Some(webp) = encode_webp(&bytes, "png") {
                let _ = fs::create_dir_all(&stackwin_dir);
                if fs::write(stackwin_dir.join(format!("{key}.webp")), &webp).is_ok() {
                    out.insert(format!("stackwin/{key}"), json!(rel));
                }
            }
        }
    }

    // 2) PSKIL42.DEF：英雄四主属性图标（group0 帧 0=攻 1=防 2=力 5=知）
    let pskil_dir = cache_root.join("core").join("images").join("pskil42");
    let pskil_frames: [(&str, usize); 4] = [
        ("attack", 0),
        ("defense", 1),
        ("spellPower", 2),
        ("knowledge", 5),
    ];
    for (key, frame) in pskil_frames {
        let rel = format!("core/images/pskil42/{}.webp", key);
        if pskil_dir.join(format!("{key}.webp")).is_file() {
            out.insert(format!("pskil42/{key}"), json!(rel));
            continue;
        }
        if let Some(rel2) = extract_game_def_frame_and_save(
            game_dir, cache_root, _mod_id, "PSKIL42", 0, frame, "pskil42", key,
        ) {
            out.insert(format!("pskil42/{key}"), json!(rel2));
        }
    }

    // 3) SPELLBON.DEF：法术列表小图标（group0 全部帧，按 spell.iconIndex 索引）
    let spellbon_dir = cache_root.join("core").join("images").join("spellbon");
    //
    // ⚠️ 完整性判定必须看**首帧在不在**，不能只看"目录非空"。
    // 曾经用 `read_dir().count() > 0`：一旦目录里剩下任意一帧（例如没有被任何法术条目
    // 引用过的 30.webp）就整体跳过，而其余帧若被精确清理（`remove_recorded_assets`
    // 按 cache.json 记录删除）删掉，就**永远不会补齐** → 引用它们的法术图标 404、列表全空。
    // 逐帧写入已有 `!path.is_file()` 保护，所以重跑时只补缺失帧，不会重复解码。
    let spellbon_complete = spellbon_dir.join("0.webp").is_file();
    if spellbon_complete {
        out.insert("spellbonDir".into(), json!("core/images/spellbon"));
        out.insert(
            "spellbonFrames".into(),
            json!(fs::read_dir(&spellbon_dir).map(|rd| rd.count()).unwrap_or(0)),
        );
    } else if let Some(bytes) =
        crate::vcmi::resource::read_game_resource_fallback(game_dir, "SPELLBON.DEF")
    {
        let def = crate::infra::def::DefReader::new(&bytes);
        if let Some(frames) = def.decode_animation() {
            let mut idx = 0usize;
            let mut max_frame = 0usize;
            for (gi, w, h, bgra) in frames {
                if gi != 0 {
                    continue;
                }
                if let Some(webp) = encode_webp_from_bgra(&bgra, w, h) {
                    let _ = fs::create_dir_all(&spellbon_dir);
                    let path = spellbon_dir.join(format!("{}.webp", idx));
                    if !path.is_file() {
                        let _ = fs::write(&path, &webp);
                    }
                    // 登记官方 HD 源（与其它共享图标同口径；本段此前手工解码落盘，漏了登记）。
                    // 实测 hd-edition 未收录 SPELLBON 帧（`SB01SUMB`/`SB16MBLT`… 全 miss），
                    // 故当前不产生登记；留着是为了将来某版 HD 收录后能自动接上。
                    if let Some(hd) = crate::vcmi::hd_edition::probe_def_frame(&bytes, 0, idx) {
                        crate::modcache::hd_map::record(
                            "core",
                            "spellbon",
                            &idx.to_string(),
                            &hd,
                        );
                    }
                    max_frame = idx;
                }
                idx += 1;
            }
            out.insert("spellbonDir".into(), json!("core/images/spellbon"));
            out.insert("spellbonFrames".into(), json!(max_frame + 1));
        }
    }

    // 4) ARTIFACT.DEF 第 0 帧：法术书图标（英雄弹窗魔法区首格）
    if let Some(rel) = extract_game_def_frame_and_save(
        game_dir,
        cache_root,
        _mod_id,
        "ARTIFACT",
        0,
        0,
        "spellbook",
        "0",
    ) {
        out.insert("spellbook".into(), json!(rel));
    }

    // 5) CPRSMALL 帧：VCMI 固定战争机器（catapult/ballista/firstAidTent/ammoCart）。
    //    英雄窗战争机器区首格恒为投石车（CHeroOverview.cpp:177 用 army.creature.CATAPULT），
    //    与英雄配置无关，故作为公共资源提取，mod 内无这些生物时也能显示。
    //    帧号 = iconIndex；CCreatureHandler.cpp:585 —— 原版生物未显式写 iconIndex 时
    //    `iconIndex = 生物索引 + 2`，catapult/ballista/tent/cart 索引为 145..148。
    let war_machines: [(&str, usize); 4] = [
        ("catapult", 147),
        ("ballista", 148),
        ("firstAidTent", 149),
        ("ammoCart", 150),
    ];
    for (key, frame) in war_machines {
        if let Some(rel) = extract_game_def_frame_and_save(
            game_dir,
            cache_root,
            _mod_id,
            "CPRSMALL",
            0,
            frame,
            "warmachine",
            key,
        ) {
            out.insert(format!("warmachine/{key}"), json!(rel));
        }
    }

    // 6) Morale / Luck 图标（CStackWindow::MainSection 的 MoraleLuckBox）。
    //    MiscWidgets.cpp:657-662 —— 小图用 IMRL30/ILCK30，帧号 = clamp(value + 3, 0, 6)。
    //    生物自身士气/运气通常为 0 → 帧 3。
    for (key, def_name) in [("morale", "IMRL30"), ("luck", "ILCK30")] {
        for frame in 0..=6usize {
            if let Some(rel) = extract_game_def_frame_and_save(
                game_dir,
                cache_root,
                _mod_id,
                def_name,
                0,
                frame,
                key,
                &frame.to_string(),
            ) {
                out.insert(format!("{key}/{frame}"), json!(rel));
            }
        }
    }

    Value::Object(out)
}

/// 提取 VCMI 资源图标（木/汞/矿/硫/晶/宝/金/魔），存到父 `core` 模组的 `core/images/resources/{0..7}.webp`。
/// 前端花费显示时按索引取（0=wood 1=mercury 2=ore 3=sulfur 4=crystal 5=gems 6=gold 7=mitril）。
pub(crate) fn extract_resource_icons_for_mod(game_dir: &Path, cache_root: &Path, _mod_id: &str) {
    if !assets_enabled() {
        return;
    }
    let res_dir = cache_root.join("core").join("images").join("resources");
    if res_dir.join("0.webp").is_file() {
        return; // 已提取，幂等
    }
    // SMALRES.DEF 在 H3sprite.lod 包内（磁盘无散文件），走游戏资源统一入口
    let bytes = match crate::vcmi::resource::read_game_resource_fallback(game_dir, "SMALRES.DEF") {
        Some(b) => b,
        None => return,
    };
    let def = crate::infra::def::DefReader::new(&bytes);
    let frames = match def.decode_animation() {
        Some(f) => f,
        None => return,
    };
    let _ = fs::create_dir_all(&res_dir);
    // SMALRES 单 group 多帧，顺序即资源顺序
    let mut idx = 0usize;
    for (gi, _w, _h, bgra) in frames {
        if gi != 0 {
            continue;
        }
        if idx >= 8 {
            break;
        }
        let w = _w;
        let h = _h;
        if let Some(webp) = encode_webp_from_bgra(&bgra, w, h) {
            let _ = fs::write(res_dir.join(format!("{}.webp", idx)), &webp);
            // 资源图标有官方 HD 版（pak 的 `SMALRES` 条目，按 DEF 帧名索引）→ 登记供变体取源。
            // 产物归属 `core`（共享图标），登记也必须归 core。
            if let Some(name) = def.frame_name(0, idx) {
                if let Some(hd) = crate::vcmi::hd_edition::probe(&name) {
                    crate::modcache::hd_map::record("core", "resources", &idx.to_string(), &hd);
                }
            }
        }
        idx += 1;
    }
}
