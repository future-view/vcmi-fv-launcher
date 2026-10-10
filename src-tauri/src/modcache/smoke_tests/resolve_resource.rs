//! 资源定位（MOD → 游戏 LOD 回退）（冒烟测试，多数 `#[ignore]`，需真实游戏目录）。

use super::*;
use crate::vcmi::resource::{
    has_core_h3_lods, resolve_resource_fallback, set_wiki_mod_paths, set_wiki_user_data_dir,
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
    println!("townBackground {} -> {} 字节, 扩展名 {}", res, buf.len(), ext);
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
    for mp in &mod_paths {
        if let Some((bytes, ext)) = resolve_resource(mp, resource) {
            println!("resolved {resource} ext={ext} len={}", bytes.len());
            assert!(bytes.len() > 100, "def 文件应非空");
            return;
        }
    }
    panic!("未找到 {resource}（VCMI_GAME_DIR 未设置或目录无效）");
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
    println!("user_dir 回退取到 TBCSBACK：{} 字节, ext={}", buf.len(), ext);
}

/// 跨模组资源回退回归：父模组配置引用了子模组（如 Fen）`Sprites/fen/creatures/...` 的精灵，
/// 而本模组自身不含该资源时，`resolve_resource_fallback` 应遍历其它已装模组找到它
/// （对齐 VCMI `findResourceOrigin` 的全局检索）。纯单测，不依赖真实游戏目录。
#[test]
fn crossmod_fallback_resolves_resource_in_foreign_mod() {
    let tmp = std::env::temp_dir()
        .join(format!("vcmi-fv-crossmod-{}", std::process::id()));
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

    set_wiki_mod_paths(&[current.clone(), foreign.clone()]);

    // 资源引用形如 VCMI 配置：fen/creatures/cross.bmp（解析器去 Sprites/ 前缀并归一）
    let res = "fen/creatures/cross.bmp";
    let (buf, ext) = resolve_resource_fallback(&current, &current, res)
        .expect("应跨模组从 foreign-fen 的 Content/Sprites/fen/creatures/cross.bmp 取到");
    assert_eq!(buf, b"FAKEPNGDATA");
    assert_eq!(ext, "bmp");

    let _ = std::fs::remove_dir_all(&tmp);
}

/// 跨模组回退在未设置模组路径列表时退化为「仅本模组 + 游戏根」，不产生误命中或崩溃。
#[test]
fn crossmod_fallback_no_paths_is_safe() {
    let tmp = std::env::temp_dir()
        .join(format!("vcmi-fv-crossmod-none-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(tmp.join("Content")).unwrap();
    // 不调用 set_wiki_mod_paths；引用一个不存在的资源应安全返回 None
    let got = resolve_resource_fallback(&tmp, &tmp, "fen/creatures/missing.bmp");
    assert!(got.is_none());
    let _ = std::fs::remove_dir_all(&tmp);
}
