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
            None,
        )
        .map(|g| g.to_value());
        match got {
            Some(v) => {
                let frames = v["frames"].as_array().map(|a| a.len()).unwrap_or(0);
                let scale = v["scale"].as_u64().unwrap_or(0);
                println!(
                    "{res} → 命中：窗口 {}x{} 档位 {} 帧数 {frames}",
                    v["w"], v["h"], scale
                );
                assert!(frames > 0, "命中时必须至少一帧");
                // 2026-10-21 起窗口是"两种布局"之一：基准 100×130，或左右对称外扩的宽幕布
                // （高度恒 130、宽 200，见 `resource::two_layout_window`）。
                // `w/h` 是**裁剪矩形**（幕布窗口 ∪ 全部组包围盒）× 档位，只会 ≥ 幕布窗口。
                let win_w = v["winW"].as_u64().unwrap_or(0);
                let win_h = v["winH"].as_u64().unwrap_or(0);
                assert!(
                    v["w"].as_u64().unwrap_or(0) >= win_w * scale,
                    "裁剪宽应 ≥ 幕布窗口宽 × 档位"
                );
                assert!(
                    v["h"].as_u64().unwrap_or(0) >= win_h * scale,
                    "裁剪高应 ≥ 幕布窗口高 × 档位"
                );
                if expect_hd {
                    assert!(
                        scale >= 2,
                        "{res} 被 HD 收录，应取高清档（need=270 → x3），实际 {scale}"
                    );
                } else {
                    assert_eq!(scale, 1, "{res} 未被 HD 收录，应走 1x 兜底");
                    assert!(
                        win_w == 100 || win_w == 200,
                        "1x 幕布窗口只有两种布局（100 或 200 宽），实际 {win_w}"
                    );
                    assert_eq!(win_h, 130, "幕布高度两种布局都恒为 130");
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
        crate::vcmi::resource::read_def_group_hd_inner(&paths, "core.castle", "CGRIFF.DEF", 99, true, 270, None)
            .is_none(),
        "不存在的组应返回 None"
    );
    crate::vcmi::hd_edition::set_source(None);
}

/// 真机冒烟（2026-10-21）：**显示窗口双口径**。
///
/// - `game`：默认游戏口径（窗口 = 基准 ∪ 待机帧），悬停越界就裁 —— 构图与游戏一致；
/// - `wing`：悬停帧横向越界 ≥24px → 左右**对称**扩成宽幕布（黑底），构图仍居中。
///
/// 运行：`VCMI_GAME_DIR=<游戏目录> cargo test --lib modcache::smoke_tests::resolve_resource::adaptive_window_smoke -- --ignored --nocapture`
#[test]
#[ignore]
fn adaptive_window_smoke() {
    let paths = VcmiPaths::detect();
    // (资源, 双格, 期望) —— 判定看"除死亡外的全部组"，所以宽体龙/天使都走宽幕布
    for (res, dw, expect) in [
        ("CPHX.DEF", true, "wing"),    // 凤凰：全组 315×239
        ("CFBIRD.DEF", true, "wing"),  // 火鸟
        ("CNDRGN.DEF", true, "wing"),  // 骨龙：全组 240×187
        ("CHDRGN.DEF", true, "wing"),  // 鬼龙
        ("CDDRAG.DEF", true, "wing"),  // 金龙：全组 294×226
        ("CRANGL.DEF", true, "wing"),  // 大天使：全组 181×150
        ("CBDRGN.DEF", true, "wing"),  // 黑龙：全组 273×221
        ("CCHAMP.DEF", true, "game"),  // 骑士
        ("CPEAS.DEF", false, "game"),  // 农民（单格）
    ] {
         let Some(v) = crate::vcmi::resource::read_def_group_hd_inner(
            &paths, "core", res, 2, dw, 270, None,
        ).map(|g| g.to_value()) else {
            println!("{res} → 无帧（跳过）");
            continue;
        };
        let s = v["scale"].as_u64().unwrap_or(1);
        let (w, h) = (
            v["w"].as_u64().unwrap_or(0) / s,
            v["h"].as_u64().unwrap_or(0) / s,
        );
        let (winx, winy) = (v["winW"].as_u64().unwrap_or(0), v["winH"].as_u64().unwrap_or(0));
        let (fitw, fith) = (v["fitW"].as_u64().unwrap_or(0), v["fitH"].as_u64().unwrap_or(0));
        println!("{res} → 裁剪 {w}x{h} / 幕布 {winx}x{winy} / 适配 {fitw}x{fith}（{expect}）");
        match expect {
            // 两种布局：高度**恒为 130**；宽幕布只左右扩
            // 幕布只有两种尺寸；裁剪矩形可以更大（含超出窗口的素材）
            "wing" => assert!(
                winx == 200 && winy == 130 && w >= 200 && h >= 130,
                "{res} 应走宽幕布 200×130 且裁剪≥幕布（实测 裁剪 {w}x{h} / 幕布 {winx}x{winy}）"
            ),
            _ => assert!(
                winx == 100 && winy == 130 && w >= 100 && h >= 130,
                "{res} 应走默认 100×130 且裁剪≥幕布（实测 裁剪 {w}x{h} / 幕布 {winx}x{winy}）"
            ),
        }
    }
}

/// 真机冒烟（2026-10-21）：**sprites-json（png 动画模式）能出帧**。
///
/// 实测 `35- Yokai-town` / `HuaXia` 全部生物走 VCMI 1.6 的 png 动画（`xxx.json` +
/// 450×400 整画布帧），此前只支持 DEF → 这些生物永远静态图。
///
/// 2026-10-21 补充 **`images` 写法**（HotA Factory 的沙虫 / 死亡沙虫 / 重型机甲 /
/// 无畏机甲）：这 4 个生物**没有 `.def`**，配置里的 `graphics.animation` 指向一个不存在
/// 的 `.def` 基名，动画只在 `X.json` 的 `images[]` 里。请求必须写**配置里的 `.def` 名**
/// （解析层按去扩展名 key 命中同名 json），这正是详情页真实入参。
///
/// 同时守住反向红线：`fix_h3creatures` 也写 `images`（对 core 生物的逐帧覆盖），
/// **不得**跨模组顶替 core 自己的 DEF 动画。
///
/// 运行：`VCMI_GAME_DIR=<游戏目录> cargo test --lib modcache::smoke_tests::resolve_resource::json_anim_group_smoke -- --ignored --nocapture`
#[test]
#[ignore]
fn json_anim_group_smoke() {
    let game = game_dir();
    let paths = crate::vcmi::paths::VcmiPaths::detect();
    // `required`：组必须出帧（缺帧 = 百科回退静态图）。
    // `images` 方言那 4 条是 2026-10-21 的回归红线：它们没有 `.def` 兜底，只靠 json。
    // `mod_id` 用**真实 wiki 条目归属**：`hota.factory` 的自身路径 = `Mods/hota/Mods/factory`。
    let candidates = [
        ("35- Yokai-town", "x.yokai-town", "yokai/creatures/battle/lv7up2.json", 2u32, false),
        ("35- Yokai-town", "x.yokai-town", "yokai/creatures/battle/lv7up2.json", 11, false),
        ("HuaXia", "x.huaxia", "huaXia/creatures/battle/lv1.json", 2, false),
        // 请求名 = 生物配置里的 `graphics.animation`（那个 `.def` 文件并不存在）
        ("hota/Mods/factory", "hota.factory", "hota/factory/creatures/battle/CSANDWX.def", 0, true),
        ("hota/Mods/factory", "hota.factory", "hota/factory/creatures/battle/CDREAD.def", 30, true),
        ("hota/Mods/factory", "hota.factory", "hota/factory/creatures/battle/COLGOIX.def", 5, true),
        ("hota/Mods/factory", "hota.factory", "hota/factory/creatures/battle/CJUGGER.def", 2, true),
    ];
    let mut hit = false;
    for (mod_dir, mod_id, res, group, required) in candidates {
        if !game.join("Mods").join(mod_dir).is_dir() {
            continue;
        }
        let got = crate::vcmi::resource::read_def_group_hd_inner(
            &paths,
            mod_id,
            res,
            group as usize,
            true,
            270,
            None,
        )
        .map(|g| g.to_value());
        match got {
            Some(v) => {
                let frames = v["frames"].as_array().map(|a| a.len()).unwrap_or(0);
                println!(
                    "{mod_dir} {res} g{group} → 窗口 {}x{} 档位 {} 帧数 {frames}",
                    v["w"], v["h"], v["scale"]
                );
                assert!(frames > 0, "png 动画应至少一帧");
                assert_eq!(v["scale"].as_u64(), Some(1), "png 动画无官方 HD 档位，应恒 1x");
                hit = true;
            }
            None => {
                println!("{mod_dir} {res} g{group} → 无帧（该组可能不存在）");
                assert!(!required, "images 方言的动画组必须能解析出帧（不得回退静态图）");
            }
        }
    }
    if !hit {
        println!("跳过：未安装 Yokai / HuaXia / hota");
    }

    // 反向红线：跨模组的 `images` 定义（fix_h3creatures 对 core 生物的覆盖）不得被采用。
    // 采用与否直接看 json 通道：必须返回 None，让上层落到 core 自己的 DEF。
    // 色键 / 阴影（2026-10-21）：帧是 **8bpp 索引 PNG**（0=青、1=浅洋红、4=洋红），
    // 必须按**索引**还原成"0 透明、1/4 半透明黑"，否则百科里就是一块天蓝幕布 + 脚下洋红。
    if game.join("Mods").join("hota").is_dir() {
        let res = "hota/factory/creatures/battle/CSANDWX.def";
        if let Some(anim) =
            crate::vcmi::anim_json::load_group(&paths, "hota.factory", res, 0, true)
        {
            let f = anim.frames.first().expect("至少一帧");
            let (mut cyan, mut magenta, mut shadow) = (0usize, 0usize, 0usize);
            for p in f.pixels() {
                let (r, g, b, a) = (p.0[0], p.0[1], p.0[2], p.0[3]);
                if a != 0 && (r, g, b) == (0, 255, 255) {
                    cyan += 1;
                }
                if a == 255 && r > 200 && g < 80 && b > 200 {
                    magenta += 1;
                }
                if a > 0 && a < 255 {
                    shadow += 1;
                }
            }
            println!("{res} 首帧：残留青 {} / 残留洋红 {} / 半透明阴影 {}", cyan, magenta, shadow);
            assert_eq!(cyan, 0, "色键索引 0 必须还原成透明，不得留下天蓝幕布");
            assert_eq!(magenta, 0, "阴影索引 1/4 必须是半透明黑，不得画成洋红");
            assert!(shadow > 0, "生物帧应带脚底阴影（索引 1/4）");
        }
    }

    // 反向红线：跨模组的 `images` 定义（fix_h3creatures 对 core 生物的覆盖）不得被采用。
    let cross_json = game
        .join("Mods")
        .join("fix_h3creatures")
        .join("content")
        .join("sprites")
        .join("CAELEM.json");
    if cross_json.is_file() {
        let hijack = crate::vcmi::anim_json::load_group(&paths, "core.conflux", "CAELEM.DEF", 2, false);
        assert!(
            hijack.is_none(),
            "跨模组的 images 定义不得顶替 core 生物的 DEF 动画"
        );
        let def = crate::vcmi::resource::read_def_group_hd_inner(
            &paths, "core.conflux", "CAELEM.DEF", 2, false, 270, None,
        );
        assert!(def.is_some(), "core 生物必须仍走 DEF 路径出帧");
    } else {
        println!("跳过跨模组守卫：未安装 fix_h3creatures");
    }
}

/// 真机冒烟（2026-10-21）：**mod 自带资源必须被归类为 mod 来源**（HD 守卫的前提）。
///
/// 背景：HD 素材按**原版命名空间**匹配。若 mod 自带资源被误判成 `Origin::Game`，
/// 帧名/名字探针就会把原版高清图塞进 mod 的美术（实测 1-knight-town 的 `CPrS005C.bmp`
/// 里是长枪兵，pak 的 `CPRS005C` 却是另一种生物 → @4x 整张错图）。
///
/// 运行：`VCMI_GAME_DIR=<游戏目录> cargo test --lib modcache::smoke_tests::resolve_resource::mod_bitmap_origin_smoke -- --ignored --nocapture`
#[test]
#[ignore]
fn mod_bitmap_origin_smoke() {
    let game = game_dir();
    let mod_dir = game.join("Mods").join("1-knight-town");
    if !mod_dir.is_dir() {
        println!("跳过：未安装 1-knight-town（{mod_dir:?}）");
        return;
    }
    let (buf, ext, origin) = crate::vcmi::resource::resolve_resource_origin(
        &mod_dir,
        &game,
        "Sprites/Knight/creatures/iconsSmall/CPrS005C.bmp",
    )
    .expect("应能解析 mod 自带位图");
    println!("origin={origin:?} ext={ext} bytes={}", buf.len());
    assert!(
        !matches!(origin, crate::vcmi::resource::Origin::Game),
        "mod 自带资源不得被判为原版来源，否则 HD 替换会张冠李戴"
    );
    // 对照组：原版资源必须判为 Game（守卫不能把原版高清也一起挡掉）
    let (_b, _e, game_origin) = crate::vcmi::resource::resolve_resource_origin(
        &game.join("Mods").join("vcmi"),
        &game,
        "CRBKGCAS.bmp",
    )
    .expect("应能解析原版兵种背景");
    assert_eq!(
        game_origin,
        crate::vcmi::resource::Origin::Game,
        "原版资源必须判为 Game，否则 HD 守卫会误伤原版高清"
    );
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






