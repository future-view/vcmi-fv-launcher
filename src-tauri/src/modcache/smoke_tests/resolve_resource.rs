//! 资源定位（MOD → 游戏 LOD 回退）（冒烟测试，多数 `#[ignore]`，需真实游戏目录）。

use super::*;
use crate::vcmi::resource::{
    has_core_h3_lods, resolve_resource_fallback, resolve_resource_origin, set_wiki_mod_paths,
    set_wiki_user_data_dir, Origin,
};

#[test]
#[ignore = "需 references/Heroes3-VCMI 游戏 fixture"]
fn test_read_vcmi_json_tolerant() {
    let p = fixture("abyss-town");
    let configs = read_town_configs(&p);
    for c in &configs {
        eprintln!(
            "faction={} town_bg={:?} creature_bg={:?}",
            c.faction_id, c.town_background, c.creature_background
        );
    }
    let cfg = configs
        .into_iter()
        .find(|c| c.town_background.is_some())
        .expect("应找到含 townBackground 的 town 配置");
    let tb = cfg.town_background.clone().unwrap();
    assert!(!tb.is_empty(), "应能读到 townBackground 字段");
    println!("abyss faction={} townBackground = {}", cfg.faction_id, tb);
}

#[test]
#[ignore = "需 references/Heroes3-VCMI 游戏 fixture"]
fn test_resolve_town_background_resource() {
    let p = fixture("abyss-town");
    let cfg = read_town_configs(&p)
        .into_iter()
        .find(|c| c.town_background.is_some())
        .expect("应找到含 townBackground 的 town 配置");
    let res = cfg.town_background.unwrap();
    let found = resolve_resource(&p, &res);
    assert!(
        found.is_some(),
        "townBackground 资源 {} 应能在资源树中定位",
        res
    );
    let (buf, ext) = found.unwrap();
    println!(
        "townBackground {} -> {} 字节, 扩展名 {}",
        res,
        buf.len(),
        ext
    );
    assert!(!buf.is_empty());
}

#[test]
#[ignore = "需 references/Heroes3-VCMI 游戏 fixture"]
fn test_creature_background_resolved() {
    let p = fixture("abyss-town");
    let configs = read_town_configs(&p);
    let cfg = configs
        .into_iter()
        .find(|c| c.creature_background.is_some())
        .expect("应找到含 creatureBackground 的 town 配置");
    let res = cfg.creature_background.unwrap();
    println!("creatureBackground 解析为 {}", res);
    assert!(!res.is_empty());
}

#[test]
#[ignore]
fn read_def_bytes_creature_smoke() {
    let paths = VcmiPaths::detect();
    let mod_paths = collect_mod_paths(&paths);
    let resource = "abyss/battle/creatures/sandcrab.def";
    for (mp, _) in &mod_paths {
        if let Some((bytes, ext)) = resolve_resource(mp, resource) {
            println!("resolved {resource} ext={ext} len={}", bytes.len());
            assert!(bytes.len() > 100, "def 文件应非空");
            return;
        }
    }
    panic!("未找到 {resource}（VCMI_GAME_DIR 未设置或目录无效）");
}

/// 真机冒烟（2026-10-20）：生物战斗动画的**高清分组**。
///
/// 两条对照，**缺一不可**：
/// - `CGRIFF.DEF`（狮鹫）在 hd-edition 里整组收录 → 应返回整组高清帧；
/// - `CPEAS.DEF`（农民）未收录 → 必须返回 `None`（前端据此回退 1x 解码，行为不变）。
///
/// 运行：`VCMI_GAME_DIR=<游戏目录> cargo test --lib modcache::smoke_tests::resolve_resource::def_group_hd_creature_smoke -- --ignored --nocapture`
#[test]
#[ignore]
fn def_group_hd_creature_smoke() {
    let paths = VcmiPaths::detect();
    // 与详情链路同口径：`auto` 探测 hd-edition（未装则两条都应回退 None）
    crate::vcmi::hd_edition::setup_from_config(None, &paths.game_dir);
    let has_hd = crate::vcmi::hd_edition::source().is_some();
    println!("hd-edition 来源: {has_hd}");

    // need = 显示框 CSS(180) × phys(1080p=1.5) = 270
    for (res, expect_hd) in [("CGRIFF.DEF", true), ("CPEAS.DEF", false)] {
        let got = crate::vcmi::resource::read_def_group_hd_inner(
            &paths,
            "core.castle",
            res,
            2,    // HOLDING
            true, // doubleWide
            270,
        );
        match got {
            Some(v) => {
                let frames = v["frames"].as_array().map(|a| a.len()).unwrap_or(0);
                let scale = v["scale"].as_u64().unwrap_or(0);
                println!(
                    "{res} → 命中：窗口 {}x{} 档位 {} 帧数 {frames}",
                    v["w"], v["h"], scale
                );
                assert!(frames > 0, "命中时必须至少一帧");
                assert_eq!(v["w"].as_u64(), Some(100 * scale), "窗口宽 = 100 × 档位");
                assert_eq!(v["h"].as_u64(), Some(130 * scale), "窗口高 = 130 × 档位");
                if expect_hd {
                    assert!(
                        scale >= 2,
                        "{res} 被 HD 收录，应取高清档（need=270 → x3），实际 {scale}"
                    );
                } else {
                    assert_eq!(scale, 1, "{res} 未被 HD 收录，应走 1x 兜底");
                    assert_eq!(v["w"].as_u64(), Some(100), "1x 窗口应为 100×130");
                    assert_eq!(v["h"].as_u64(), Some(130));
                }
            }
            None => {
                println!("{res} → 无帧返回");
                // 只有"带 HD 且确有帧"的期望不该落空；未收录的也必须由 1x 兜底给出帧
                panic!("{res} 应返回帧（HD 或 1x 兜底）");
            }
        }
    }
    // 不存在的动作组 → None（前端据此把该项置灰）
    assert!(
        crate::vcmi::resource::read_def_group_hd_inner(&paths, "core.castle", "CGRIFF.DEF", 99, true, 270)
            .is_none(),
        "不存在的组应返回 None"
    );
    crate::vcmi::hd_edition::set_source(None);
}

/// 真机冒烟（2026-10-21）：**中立阵营必须有 creatureBackground**。
///
/// 中立不是可玩城镇（`read_game_root_factions` 明确过滤 neutral），所以它的
/// `creatureBackground`（原版 `CRBKGNEU`）不会进入 `factions` 列表 —— 中立生物在百科里
/// 只能显示黑底。此测试守住"能解析到 + 资源可读"，配合 `build_creatures` 的兜底提取。
///
/// 运行：`VCMI_GAME_DIR=<游戏目录> cargo test --lib modcache::smoke_tests::resolve_resource::neutral_faction_bg_smoke -- --ignored --nocapture`
#[test]
#[ignore]
fn neutral_faction_bg_smoke() {
    let game = game_dir();
    let cfg = crate::vcmi::town::parse_town_config_file(
        &game.join("config").join("factions").join("neutral.json"),
    );
    assert_eq!(cfg.faction_id, "neutral", "应解析出 neutral 阵营");
    let bg = cfg
        .creature_background
        .expect("neutral.json 应有 creatureBackground");
    println!("neutral creatureBackground = {bg}");
    let (buf, ext) =
        crate::vcmi::resource::resolve_resource_fallback(&game, &game, &bg).expect("资源应可读");
    println!("resolved {} bytes, ext={ext}", buf.len());
    assert!(buf.len() > 1000, "背景图不应为空");
}

/// 回归守卫：当游戏根 `Data/` 无原版 LOD（便携运行场景），原版数据放在 `user_data_dir/Data` 时，
/// `resolve_resource_fallback` 必须回退到用户数据目录取到精灵（此前"二选一"逻辑会漏掉）。
#[test]
#[ignore]
fn fallback_searches_user_data_dir_when_game_dir_lacks_data() {
    let fake_game = fixture("abyss-town"); // 无 Data/LODs 的 mod 目录，模拟"游戏根无原版数据"
    let user_dir = game_dir();
    assert!(
        has_core_h3_lods(&user_dir),
        "VCMI_GAME_DIR 应指向含 Data/*.lod 的游戏目录"
    );
    set_wiki_user_data_dir(&user_dir);
    // 游戏根无数据，但用户数据目录有原版 LOD：应回退 user_dir/Data 取到精灵
    let (buf, ext) = resolve_resource_fallback(&fake_game, &fake_game, "TBCSBACK.bmp")
        .expect("应经 user_data_dir/Data 回退取到 TBCSBACK");
    assert!(!buf.is_empty());
    println!(
        "user_dir 回退取到 TBCSBACK：{} 字节, ext={}",
        buf.len(),
        ext
    );
}

/// 跨模组资源回退回归：父模组配置引用了子模组（如 Fen）`Sprites/fen/creatures/...` 的精灵，
/// 而本模组自身不含该资源时，`resolve_resource_fallback` 应遍历其它已装模组找到它
/// （对齐 VCMI `findResourceOrigin` 的全局检索）。纯单测，不依赖真实游戏目录。
#[test]
fn crossmod_fallback_resolves_resource_in_foreign_mod() {
    let tmp = std::env::temp_dir().join(format!("vcmi-fv-crossmod-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);

    let current = tmp.join("current-mod");
    std::fs::create_dir_all(current.join("Content")).unwrap();

    let foreign = tmp.join("foreign-fen");
    let sprite_dir = foreign
        .join("Content")
        .join("Sprites")
        .join("fen")
        .join("creatures");
    std::fs::create_dir_all(&sprite_dir).unwrap();
    std::fs::write(sprite_dir.join("cross.bmp"), b"FAKEPNGDATA").unwrap();

    set_wiki_mod_paths(&[
        (current.clone(), "current-mod".to_string()),
        (foreign.clone(), "foreign-fen".to_string()),
    ]);

    // 资源引用形如 VCMI 配置：fen/creatures/cross.bmp（解析器去 Sprites/ 前缀并归一）
    let res = "fen/creatures/cross.bmp";
    let (buf, ext) = resolve_resource_fallback(&current, &current, res)
        .expect("应跨模组从 foreign-fen 的 Content/Sprites/fen/creatures/cross.bmp 取到");
    assert_eq!(buf, b"FAKEPNGDATA");
    assert_eq!(ext, "bmp");

    // 归属：跨模组命中必须报出**被引用方**的 modId。
    // 百科缓存据此把产物落到 foreign-fen 的缓存目录，而不是提取调用方 current-mod 的目录
    // —— 这是「跨模组素材相互依赖时不各存一份」的判定基础。
    let (_, _, origin) = resolve_resource_origin(&current, &current, res).expect("同上应命中");
    assert_eq!(origin, Origin::Mod("foreign-fen".to_string()));

    // 对照：本模组内命中 → SelfMod（由调用方填自己的 modId 归属到自己）
    let own = current.join("Content").join("own.bmp");
    std::fs::write(&own, b"OWNDATA").unwrap();
    let (_, _, origin_own) =
        resolve_resource_origin(&current, &current, "own.bmp").expect("本模组内应命中");
    assert_eq!(origin_own, Origin::SelfMod);

    let _ = std::fs::remove_dir_all(&tmp);
}

/// 同基名不同扩展名：请求带扩展名时必须命中**该扩展名**的文件（2026-10-13 回归）。
///
/// 背景：VCMI 1.6+ sprites-json 的定义是 `X.json`、其位图帧是 `X.png`，二者经
/// `norm_zip_key`「去扩展名 + 小写」归一后**撞成同一个 key**；旧实现只按该 key 取
/// 「索引里第一个」的文件（实测总是 `.json`）→ 请求 `X.png` 拿到 JSON 字节 →
/// `image::load_from_memory` 失败 → 城镇结构帧整批解析失败 → 幕布图只剩裸底图
/// （实测 shuHan / feydom / adeptorium / karmicCourt 四个城镇建筑全部消失）。
///
/// 现规则：**请求显式带扩展名时先精确命中**；无同扩展名文件时仍回落宽松匹配
/// （保留配置写的扩展名与实际文件不符时的兼容能力）。纯单测，不依赖真实游戏目录。
#[test]
fn same_basename_diff_ext_prefers_requested_ext() {
    let tmp = std::env::temp_dir().join(format!("vcmi-fv-extkey-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let dir = tmp
        .join("Content")
        .join("Sprites")
        .join("town")
        .join("buildings");
    std::fs::create_dir_all(&dir).unwrap();

    // 真实 PNG（`load_structure_frame` 会真的解码它，故不能用假字节）
    let mut png = Vec::new();
    image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
        2,
        2,
        image::Rgba([1, 2, 3, 255]),
    ))
    .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
    .unwrap();

    // ① 同基名定义 + 位图帧（VCMI 1.6+ sprites-json 的标准形态）
    std::fs::write(
        dir.join("mageGuild1.json"),
        br#"{"basepath":"town/buildings/","sequences":[{"group":0,"frames":["mageGuild1.png"]}]}"#,
    )
    .unwrap();
    std::fs::write(dir.join("mageGuild1.png"), &png).unwrap();
    // ② 只有 json、没有同名 .def（配置写错扩展名 → 应回落宽松匹配）
    std::fs::write(dir.join("onlyJson.json"), b"{}").unwrap();

    // ① 位图请求拿到位图，而非同基名定义 json
    let (buf, ext) = resolve_resource(&tmp, "town/buildings/mageGuild1.png").expect("应命中 png");
    assert_eq!(ext, "png", "带扩展名的请求应精确命中同扩展名文件");
    assert_eq!(buf, png);
    // 定义 json 请求仍拿 json
    let (_, ext) = resolve_resource(&tmp, "town/buildings/mageGuild1.json").expect("应命中 json");
    assert_eq!(ext, "json");
    // 定义文件按其自身引用可解出位图帧（端到端复现原有失败路径）
    let frame = load_structure_frame(&tmp, &tmp, &tmp, "town/buildings/mageGuild1.json");
    assert!(
        frame.is_some(),
        "sprites-json 定义 + 同基名位图必须能取出帧"
    );
    // ② 扩展名与实际不符 → 仍回落宽松匹配（不因新增精确命中而回归）
    let (_, ext) = resolve_resource(&tmp, "town/buildings/onlyJson.def").expect("应回落宽松匹配");
    assert_eq!(ext, "json");
    // ③ 无扩展名引用不受影响
    assert!(resolve_resource(&tmp, "town/buildings/mageGuild1").is_some());
    // ④ 大小写不敏感（mod 配置大小写与磁盘不一致是常态）
    assert!(resolve_resource(&tmp, "town/buildings/MAGEGUILD1.PNG").is_some());

    let _ = std::fs::remove_dir_all(&tmp);
}

/// 跨模组回退在未设置模组路径列表时退化为「仅本模组 + 游戏根」，不产生误命中或崩溃。
#[test]
fn crossmod_fallback_no_paths_is_safe() {
    let tmp = std::env::temp_dir().join(format!("vcmi-fv-crossmod-none-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("Content")).unwrap();
    // 不调用 set_wiki_mod_paths；引用一个不存在的资源应安全返回 None
    let got = resolve_resource_fallback(&tmp, &tmp, "fen/creatures/missing.bmp");
    assert!(got.is_none());
    let _ = std::fs::remove_dir_all(&tmp);
}
