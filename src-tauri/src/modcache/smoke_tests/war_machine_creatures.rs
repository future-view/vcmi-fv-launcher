//! 战争机器生物回归（2026-10-24，`WIKI_CACHE_VERSION` 4）。
//!
//! 背景：VCMI 的百科只放行「非 special」与**战争机器**（`WikiWindow.cpp:553-557`：
//! `if(!creature->special || isWM)`）。原版 `catapult` / `ballista` / `firstAidTent` /
//! `ammoCart` 全部定义在游戏根 `config/creatures/special.json` 且都标了 `special: true`，
//! 而旧实现把 `special: true` 一刀切丢掉 —— 整个发行版**没有任何生物记录**带 `warMachine`：
//!
//! - 英雄「初始部队」里的弩车 / 急救帐篷（实测该发行版 11 个 `ballista` + 9 个
//!   `firstAidTent`）取不到生物记录 → 图标空白、右键弹不出生物窗；
//! - 同一批槽位还会被前端按「拿不到 `warMachine` 就不是战争机器」错分到「初始部队」格里
//!   （实测艾拉切第 2 格：数量 1、概率 88%、无图）。
//!
//! 本组验证修复后**产物**（`core/cache.json`）的不变量：战争机器必须作为生物条目存在且
//! 带 `warMachine`；非同战争机器的 `special` 生物（`arrowTower`）仍须被排除。
//!
//! 运行：`VCMI_GAME_DIR=<游戏目录> cargo test --lib war_machine_creatures -- --ignored --nocapture`
//! （游戏目录缺省回落仓库 `references/Heroes3-VCMI` fixture，未装对应数据时整组跳过）

use super::*;

/// 原版四个战争机器（VCMI 生物索引 145-148，CPRSMALL 帧 147-150）。
const WAR_MACHINES: [&str; 4] = ["catapult", "ballista", "firstAidTent", "ammoCart"];

/// 取（首次调用时构建）原版伪模组的数据产物；同一 mod 在整组用例内**只构建一次**。
///
/// 战争机器在父 `core`（游戏根 `config/creatures/special.json` 无阵营归属），
/// 而英雄在各阵营子模组 `core.<faction>`：两侧都得建，否则看不到「英雄引用 → 生物记录」的关系。
///
/// 用例是并行跑的（同一进程多线程），各自去删同一个缓存目录会把别人的 `cache.json`
/// 读成半截（实测 `stream did not contain valid UTF-8`）—— 故用进程级缓存 + 锁串行化。
fn build_core_and_read(game_dir: &Path, mod_id: &str) -> Option<Value> {
    type Built = std::sync::Mutex<std::collections::HashMap<String, Option<Value>>>;
    static BUILT: std::sync::OnceLock<Built> = std::sync::OnceLock::new();
    let mut guard = BUILT.get_or_init(Default::default).lock().unwrap();
    if let Some(v) = guard.get(mod_id) {
        return v.clone();
    }
    let v = build_once(game_dir, mod_id);
    guard.insert(mod_id.to_string(), v.clone());
    v
}

/// 全新构建`core` / `core.<faction>` 的**阶段一（仅数据）** 并读回 `cache.json`。
///
/// 只跑阶段一：本组断言全在数据层（生物条目 / `warMachine`），跑素材阶段会白等
/// HDD 上的 DEF 提取。先删缓存目录 —— 构建逻辑的改动不在缓存键里，残留缓存会读到旧产物。
fn build_once(game_dir: &Path, mod_id: &str) -> Option<Value> {
    let core_mod = game_dir.join("Mods").join("vcmi");
    if !core_mod.is_dir() {
        eprintln!("跳过：{} 内没有 Mods/vcmi", game_dir.display());
        return None;
    }
    let cache_root = std::env::temp_dir().join("vcmi_fv_war_machine_test");
    let mod_cache = cache_root.join(mod_id);
    let _ = fs::remove_dir_all(&mod_cache);
    let _ = fs::create_dir_all(&cache_root);
    let r = build_mod_wiki_phase(
        &core_mod.to_string_lossy(),
        mod_id,
        &cache_root,
        game_dir,
        0,
    );
    assert_eq!(
        r.get("ok").and_then(|x| x.as_bool()),
        Some(true),
        "{mod_id} 构建失败: {r}"
    );
    let text = fs::read_to_string(mod_cache.join("cache.json"))
        .unwrap_or_else(|e| panic!("{mod_id} cache.json 未落盘: {e}"));
    Some(serde_json::from_str(&text).expect("cache.json 解析失败"))
}

/// 取生物表为 id → 条目。
fn creatures_by_id(data: &Value) -> std::collections::HashMap<String, Value> {
    data.get("creatures")
        .and_then(|x| x.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|c| {
                    let id = c.get("id").and_then(|x| x.as_str())?.to_string();
                    Some((id, c.clone()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// 战争机器必须进生物表，且 `warMachine`（绑定的神器 id）非空 —— 前端靠它把槽位从
/// 「初始部队」挪到「战争机器」区；同时 `iconIndex` 必须落在 CPRSMALL 的战争机器帧上。
#[test]
#[ignore]
fn core_war_machines_are_creatures_with_binding() {
    let game_dir = game_dir();
    crate::vcmi::resource::set_wiki_user_data_dir(&user_data_dir(&game_dir));
    let Some(data) = build_core_and_read(&game_dir, "core") else {
        return;
    };
    let by_id = creatures_by_id(&data);
    for id in WAR_MACHINES {
        let Some(c) = by_id.get(id) else {
            panic!("core 生物表缺少战争机器 {id}（`special: true` 被一刀切丢弃）");
        };
        let wm = c.get("warMachine").and_then(|x| x.as_str()).unwrap_or("");
        assert_eq!(wm, id, "{id} 的 warMachine 应为同名神器 id，实际 {wm:?}");
        let icon = c.get("iconIndex").and_then(|x| x.as_i64()).unwrap_or(-1);
        assert!(
            (147..=150).contains(&icon),
            "{id} 的小图标应取 CPRSMALL 战争机器帧 147-150，实际 {icon}"
        );
        // 战争机器标注（VCMI `CCreature::isSiegeWeapon()`）：配置写在 `abilities.siegeWeapon`
        // 而不是 `bonuses` —— 前端靠它选择“开炮预览”编排（见 `animScenario::buildSiegeSteps`）。
        assert_eq!(
            c.get("siegeWeapon").and_then(|x| x.as_bool()),
            Some(true),
            "{id} 的 siegeWeapon 应为 true（`abilities.siegeWeapon.type = SIEGE_WEAPON`）"
        );
    }
}

/// 反例：`special: true` 且**没有**战争机器绑定（`arrowTower`）仍不得进百科 ——
/// 否则会退回到「一排无归属的 special 生物」噪音。
#[test]
#[ignore]
fn non_war_machine_special_creature_still_excluded() {
    let game_dir = game_dir();
    crate::vcmi::resource::set_wiki_user_data_dir(&user_data_dir(&game_dir));
    let Some(data) = build_core_and_read(&game_dir, "core") else {
        return;
    };
    let by_id = creatures_by_id(&data);
    assert!(
        !by_id.contains_key("arrowTower"),
        "arrowTower 是 special 且非战争机器，不应进生物表"
    );
}

/// 端到端：英雄初始部队里引用的战争机器**必须能被生物表解析** ——
/// 这是前端出图 / 分类的唯一依据（解析不到 ⇒ 空格子 + 错分类，即用户实测的艾拉切第 2 格）。
///
/// 生物表跨模组合并（父 `core` 的战争机器 + 各阵营子模组的普通生物），与运行期的
/// 「全类目索引」兜底同口径（`detailShared.tsx::makeRefResolver`）。
#[test]
#[ignore]
fn hero_army_war_machines_resolve_to_records() {
    let game_dir = game_dir();
    crate::vcmi::resource::set_wiki_user_data_dir(&user_data_dir(&game_dir));
    let Some(core) = build_core_and_read(&game_dir, "core") else {
        return;
    };
    let Some(dungeon) = build_core_and_read(&game_dir, "core.dungeon") else {
        return;
    };
    let mut by_id = creatures_by_id(&dungeon);
    by_id.extend(creatures_by_id(&core));
    let heroes = dungeon
        .get("heroes")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(!heroes.is_empty(), "core.dungeon 未产出英雄");
    let mut checked = 0usize;
    for h in &heroes {
        let hid = h.get("id").and_then(|x| x.as_str()).unwrap_or("?");
        for u in h
            .get("army")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default()
        {
            let cid = u.get("creature").and_then(|x| x.as_str()).unwrap_or("");
            if !WAR_MACHINES.contains(&cid) {
                continue;
            }
            let c = by_id
                .get(cid)
                .unwrap_or_else(|| panic!("英雄 {hid} 的 {cid} 在生物表里不存在（无图 / 错分类）"));
            let wm = c.get("warMachine").and_then(|x| x.as_str()).unwrap_or("");
            assert_eq!(wm, cid, "英雄 {hid} 的 {cid} 缺 warMachine 绑定，会被错分到初始部队");
            checked += 1;
        }
    }
    assert!(checked > 0, "父 core 英雄初始部队里应至少出现 1 个战争机器");
    eprintln!("已核对 {checked} 个英雄初始部队战争机器槽位");
}

/// 素材事实（前端 `animScenario::buildSiegeSteps` 的设计依据）：战争机器战斗 DEF 的**待机组
/// 只有 1 帧**、`MOVING`/`MOUSEON` 是 0 帧，动作帧全在射击组 —— 所以普通生物的状态机在它们
/// 身上看起来就是“没有动画”（用户实测投石车/弩车）。
///
/// 素材一旦变了（出现多帧待机）本用例会失败，提醒重新评估前端编排。
#[test]
#[ignore]
fn war_machine_defs_have_no_idle_loop() {
    use crate::infra::def::DefReader;

    let paths = VcmiPaths::detect();
    crate::vcmi::resource::set_wiki_user_data_dir(&user_data_dir(&paths.game_dir));
    // (DEF, HOLDING 帧数上限, 是否必须带射击组)：
    // 投石车 / 弩车 / 补给车只有 1 帧待机；急救帐篷的待机本身有 6 帧（帐篖会拂动）。
    let cases: [(&str, usize, bool); 4] = [
        ("SMCATA.DEF", 1, true),
        ("SMBAL.DEF", 1, true),
        ("SMTENT.DEF", 6, false),
        ("SMCART.DEF", 1, false),
    ];
    for (res, max_holding, has_shoot) in cases {
        let Ok(bytes) = crate::vcmi::resource::read_def_bytes_inner(&paths, "core", res) else {
            eprintln!("跳过：未找到 {res}");
            continue;
        };
        let def = DefReader::new(&bytes);
        let count = |g: usize| (0..256).filter(|i| def.frame_name(g, *i).is_some()).count();
        let (holding, moving, mouseon) = (count(2), count(0), count(1));
        eprintln!(
            "{res}: HOLDING={holding} MOVING={moving} MOUSEON={mouseon} SHOOT(14)={}",
            count(14)
        );
        assert!(
            holding <= max_holding,
            "{res} 的 HOLDING 组出现了 {holding} 帧（预期 ≤{max_holding}）—— 待机编排可重新评估"
        );
        assert_eq!(moving, 0, "{res} 不应有 MOVING 帧（战争机器不会走路）");
        assert_eq!(mouseon, 0, "{res} 不应有 MOUSEON 帧");
        if has_shoot {
            assert!(count(14) > 0, "{res} 应有射击组（前端用它做动画）");
        }
    }
}

/// 帧到达前的**布局基线**（`animWindow`）必须与运行期帧携带的几何完全一致 ——
/// 否则前端会先按基线摆一次、帧到了又改一次，照样跳。
///
/// 历史现象：投石车待机预览先按基准 100×130 摆（标准幕布 180×234），帧一到算出宽幕布
/// （200×130 → 320×208），画框整块“跳”一下。
#[test]
#[ignore]
fn anim_window_matches_runtime_group_meta() {
    let game_dir = game_dir();
    crate::vcmi::resource::set_wiki_user_data_dir(&user_data_dir(&game_dir));
    let Some(data) = build_core_and_read(&game_dir, "core") else {
        return;
    };
    let by_id = creatures_by_id(&data);
    let paths = VcmiPaths::detect();
    let mut checked = 0usize;
    for id in WAR_MACHINES {
        let Some(c) = by_id.get(id) else { continue };
        let win = c
            .get("animWindow")
            .and_then(|x| x.as_object())
            .unwrap_or_else(|| panic!("{id} 缺 animWindow（前端会先按 100×130 摆一次再跳）"));
        let anim = c.get("anim").and_then(|x| x.as_object()).expect("anim 指针");
        let owner = anim.get("owner").and_then(|x| x.as_str()).unwrap_or("core");
        let res = anim
            .get("resource")
            .and_then(|x| x.as_str())
            .expect("anim.resource");
        let double_wide = c.get("doubleWide").and_then(|x| x.as_bool()).unwrap_or(false);
        let out = crate::vcmi::resource::read_def_group_hd_inner(
            &paths,
            owner,
            res,
            2, // HOLDING
            double_wide,
            270,
            Some("def"),
        )
        .unwrap_or_else(|| panic!("{id} 的 {res} 取不到 HOLDING 组"));
        let meta = out.meta.as_object().expect("meta");
        let pick = |k: &str| meta.get(k).and_then(|x| x.as_i64());
        for (k, want) in [
            ("winX", win.get("winX")),
            ("winY", win.get("winY")),
            ("winW", win.get("winW")),
            ("winH", win.get("winH")),
            ("fitW", win.get("fitW")),
            ("fitH", win.get("fitH")),
        ] {
            assert_eq!(
                pick(k),
                want.and_then(|x| x.as_i64()),
                "{id} 的 animWindow.{k} 与运行期 {res} 的几何不一致"
            );
        }
        checked += 1;
    }
    assert!(checked > 0, "没有可校验的战争机器");
    // 具体锚点：投石车的幕布确实是**宽幕布**（200×130）—— 正是缺基线时会跳的那一档
    let catapult = by_id.get("catapult").expect("catapult");
    assert_eq!(
        catapult
            .get("animWindow")
            .and_then(|x| x.get("winW"))
            .and_then(|x| x.as_i64()),
        Some(200),
        "投石车应为宽幕布（200）—— 缺 animWindow 时前端会从 100 跳到这里"
    );
}

/// 具体个案锚点：艾拉切（arlach）第 2 格是 `ballista` 1 个（88%），
/// 修复后它必须落进「战争机器」区而不是「初始部队」。
#[test]
#[ignore]
fn arlach_second_slot_is_ballista_war_machine() {
    let game_dir = game_dir();
    crate::vcmi::resource::set_wiki_user_data_dir(&user_data_dir(&game_dir));
    let Some(core) = build_core_and_read(&game_dir, "core") else {
        return;
    };
    let Some(dungeon) = build_core_and_read(&game_dir, "core.dungeon") else {
        return;
    };
    let mut by_id = creatures_by_id(&dungeon);
    by_id.extend(creatures_by_id(&core));
    let hero = dungeon
        .get("heroes")
        .and_then(|x| x.as_array())
        .and_then(|arr| arr.iter().find(|h| h.get("id").and_then(|x| x.as_str()) == Some("arlach")))
        .cloned();
    let Some(hero) = hero else {
        eprintln!("跳过：core.dungeon 未产出 arlach");
        return;
    };
    let army = hero.get("army").and_then(|x| x.as_array()).cloned().unwrap_or_default();
    let slot = army.get(1).and_then(|u| u.get("creature")).and_then(|x| x.as_str());
    assert_eq!(slot, Some("ballista"), "arlach 第 2 格应为 ballista: {army:?}");
    let wm = by_id
        .get("ballista")
        .and_then(|c| c.get("warMachine"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    assert_eq!(wm, "ballista", "ballista 缺 warMachine 绑定（前端会错分到初始部队）");
}
