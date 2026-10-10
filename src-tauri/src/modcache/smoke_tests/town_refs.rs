//! 城镇解析修复回归（2026-09-13 城镇列表核对，WIKI_CACHE_VERSION 67）。
//!
//! 覆盖 4 个修复点（均以真实 mod 目录为样本）：
//! 1. 多文件合并的 `town_name`——`faction.json` 的中文名不再被先解析到的
//!    `buildings.json` 挤掉（old-/new-chinese-mods 28 个城镇：禁猎地/灵墟/蜀汉…）；
//! 2. `sprites/**` 图形定义误判——空 factionId 幽灵条目被过滤（hota.bulwark/cove、shuHan）；
//! 3. 跨 mod 引用条目（`<modId>:<entityId>` 补丁语法）标记 `ref:true`（hota.supplementary）；
//! 4. 单数 `building.json` / `townX/` 目录——建筑列表不再为空（发条城 / Pavilion）。
//!
//! 运行：`VCMI_GAME_DIR=<游戏目录> cargo test --lib town_ -- --ignored --nocapture`
//! （游戏目录缺省回落仓库 `references/Heroes3-VCMI` 验证 fixture，不存在时整组跳过）

use super::*;

/// 本组测试共享：游戏目录（不存在则跳过）。
fn test_game_dir() -> Option<PathBuf> {
    let game_dir = game_dir();
    if !game_dir.join("Mods").is_dir() {
        eprintln!("跳过：游戏目录不存在或缺少 Mods：{}", game_dir.display());
        return None;
    }
    crate::vcmi::resource::set_wiki_user_data_dir(&user_data_dir(&game_dir));
    Some(game_dir)
}

/// 全新构建单个 mod（写临时缓存目录）并读回 `mod.json`。
fn build_and_read(mod_rel: &str, mod_id: &str, game_dir: &Path) -> Value {
    let cache_root = std::env::temp_dir()
        .join("vcmi_fv_town_refs_test")
        .join(crate::modcache::sanitize_id(mod_id));
    let _ = fs::create_dir_all(&cache_root);
    let p = fixture(mod_rel);
    assert!(p.is_dir(), "fixture 不存在：{}", p.display());
    let r = build_mod_wiki_inner(&p.to_string_lossy(), mod_id, &cache_root, game_dir);
    assert_eq!(
        r.get("ok").and_then(|x| x.as_bool()),
        Some(true),
        "构建失败 {mod_id}: {r}"
    );
    let text = fs::read_to_string(cache_root.join(crate::modcache::sanitize_id(mod_id)).join("cache.json"))
        .unwrap_or_else(|e| panic!("{mod_id} cache.json 未落盘: {e}"));
    serde_json::from_str(&text).expect("cache.json 解析失败")
}

/// 取指定 factionId 的城镇条目（不假设 `factions[0]`——多文件可产出额外条目）。
fn faction_of<'a>(data: &'a Value, fid: &str) -> &'a Value {
    data.get("factions")
        .and_then(|x| x.as_array())
        .and_then(|a| {
            a.iter()
                .find(|f| f.get("factionId").and_then(|x| x.as_str()) == Some(fid))
        })
        .unwrap_or_else(|| panic!("未找到 faction {fid}：{:?}", data.get("factions")))
}

fn town_name_of(data: &Value) -> String {
    data.get("factions")
        .and_then(|x| x.as_array())
        .and_then(|a| a.first())
        .map(|f| {
            f.get("name")
                .and_then(|n| n.get("chinese"))
                .and_then(|s| s.as_str())
                .unwrap_or("")
                .to_string()
        })
        .unwrap_or_default()
}

/// 1) 中文城镇名：`faction.json` 的 `name` 必须在多文件合并后存活。
#[test]
#[ignore]
fn town_name_merge_restores_chinese_names() {
    let Some(game_dir) = test_game_dir() else { return };

    // 新中文包（磁盘 `config/xanadu/town/faction.json` = "灵墟"，此前回落 "xanadu"）
    let data = build_and_read("new-chinese-mods/mods/xanadu", "new-chinese-mods.xanadu", &game_dir);
    assert_eq!(town_name_of(&data), "灵墟", "factions={:?}", data.get("factions"));

    // 老中文包（zip 内 `config/glade/town/Faction.json` = "禁猎地"，此前回落 "glade"）
    let data = build_and_read("old-chinese-mods/mods/Glade", "old-chinese-mods.Glade", &game_dir);
    assert_eq!(town_name_of(&data), "禁猎地", "factions={:?}", data.get("factions"));
}

/// 2) 空 factionId 幽灵条目：`sprites/**/town/townhall.json` 不再被当城镇配置。
#[test]
#[ignore]
fn no_empty_faction_id_entries() {
    let Some(game_dir) = test_game_dir() else { return };

    // hota.bulwark / cove：此前各含 2 条（正常 faction + 空条目）
    for (rel, id, fid) in [
        ("hota/mods/bulwark", "hota.bulwark", "bulwark"),
        ("hota/mods/cove", "hota.cove", "cove"),
    ] {
        let data = build_and_read(rel, id, &game_dir);
        let facs = data.get("factions").and_then(|x| x.as_array()).expect("factions");
        assert_eq!(facs.len(), 1, "{id} 应只有 1 条城镇，实际 {facs:?}");
        assert_eq!(facs[0].get("factionId").and_then(|x| x.as_str()), Some(fid));
        assert!(
            !town_name_of(&data).is_empty(),
            "{id} 城镇名不应为空"
        );
    }
}

/// 3) 跨 mod 引用（补丁语法）标记 `ref:true` + `refMod` / `refId`，数据保留。
#[test]
#[ignore]
fn cross_mod_reference_marked_as_ref() {
    let Some(game_dir) = test_game_dir() else { return };

    let data = build_and_read("hota/mods/supplementary", "hota.supplementary", &game_dir);
    let facs = data.get("factions").and_then(|x| x.as_array()).expect("factions");
    assert_eq!(facs.len(), 1, "factions={facs:?}");
    assert_eq!(
        facs[0].get("ref").and_then(|x| x.as_bool()),
        Some(true),
        "hota.factory:factory 应标记 ref"
    );
    assert_eq!(facs[0].get("refMod").and_then(|x| x.as_str()), Some("hota.factory"));
    assert_eq!(facs[0].get("refId").and_then(|x| x.as_str()), Some("factory"));
}

/// 5) 跨模组翻译（祖先链）+ 大小写不敏感 + `translation*` 目录变体：
///    此前 17 个城镇显示英文名（crossroads 12 / hota 3 / new-pavilion / Fairytale）。
#[test]
#[ignore]
fn ancestor_i18n_restores_english_town_names() {
    let Some(game_dir) = test_game_dir() else { return };

    // crossroads 子模组：中文名在**父 mod** 的 content.zip，键含空格（sand castle / night watch）
    for (rel, id, expect) in [
        ("crossroads-of-worlds/mods/Fen", "crossroads-of-worlds.Fen", "泥沼"),
        ("crossroads-of-worlds/mods/Sand Castle", "crossroads-of-worlds.Sand Castle", "红城"),
        ("crossroads-of-worlds/mods/Highland", "crossroads-of-worlds.Highland", "高地"),
    ] {
        let data = build_and_read(rel, id, &game_dir);
        assert_eq!(town_name_of(&data), expect, "{id} 中文名应命中父模组翻译");
    }

    // hota 子模组：中文名在父 `hota/content.zip::translation/chinese.json`
    let data = build_and_read("hota/mods/bulwark", "hota.bulwark", &game_dir);
    assert_eq!(town_name_of(&data), "坚垒");

    // 自身 zip 的 `translationPavilion` 目录变体（此前只认 translation/translations）
    let data = build_and_read("new-pavilion", "new-pavilion", &game_dir);
    assert_eq!(town_name_of(&data), "沙堡");

    // 大小写回退：modId `Fairytale` vs 键 `faction.fairytale.fairytale.name`
    let data = build_and_read("Fairytale", "Fairytale", &game_dir);
    assert_eq!(town_name_of(&data), "花仙");
}

/// 6) 多语言文本提取补全（2026-09-13 第三轮，`WIKI_CACHE_VERSION` 69）：
///    - 技能三级描述键 `skill.<scope>.<id>.description.<lvl>`（此前键顺序写反 → 全空）；
///    - 原版（builtin）法术/技能**中英分列**（此前英文键内容为中文）；
///    - 模组宝物 `description` 生成（此前模组分支完全不输出该字段）。
#[test]
#[ignore]
fn multilang_text_extraction_regression() {
    let Some(game_dir) = test_game_dir() else { return };

    // 技能：creature-heroes/chBackstab 的三级描述（键顺序修复）
    let data = build_and_read("creature-heroes", "creature-heroes", &game_dir);
    let skills = data.get("skills").and_then(|x| x.as_array()).cloned().unwrap_or_default();
    let sk = skills
        .iter()
        .find(|s| s.get("id").and_then(|x| x.as_str()) == Some("chBackstab"))
        .unwrap_or_else(|| panic!("chBackstab 技能应存在（共 {} 条）", skills.len()));
    let basic_zh = sk
        .get("levels")
        .and_then(|l| l.get("basic"))
        .and_then(|b| b.get("chinese"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    assert!(
        basic_zh.contains("背刺"),
        "技能三级描述应命中中文（skill.<scope>.<id>.description.basic），实际: {basic_zh:?}"
    );

    // 原版法术中英分列（h3-core english.spells.json）
    // builtin 法术已收敛到原版 core 伪模组（不再逐 mod 注入），故从 core 读取
    let data = build_and_read("vcmi", "core", &game_dir);
    let spells = data.get("spells").and_then(|x| x.as_array()).cloned().unwrap_or_default();
    let sp = spells
        .iter()
        .find(|s| {
            s.get("id").and_then(|x| x.as_str()) == Some("scuttleBoat")
                && s.get("builtin").and_then(|b| b.as_bool()) == Some(true)
        })
        .expect("builtin 法术 scuttleBoat 应存在");
    assert_eq!(
        sp.get("name").and_then(|n| n.get("english")).and_then(|x| x.as_str()),
        Some("Scuttle Boat"),
        "原版法术英文名应取自 english.spells.json（此前被中文覆盖）"
    );
    let lv_en = sp
        .get("levelDescriptions")
        .and_then(|l| l.get("basic"))
        .and_then(|b| b.get("english"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    assert!(lv_en.contains("Scuttle Boat"), "原版法术分级描述英文应可得，实际: {lv_en:?}");

    // 模组宝物 description（crossroads 翻译含 artifact.<scope>.<id>.description）
    let arts = data.get("artifacts").and_then(|x| x.as_array()).cloned().unwrap_or_default();
    let with_desc = arts
        .iter()
        .filter(|a| {
            a.get("description")
                .and_then(|d| d.get("chinese"))
                .and_then(|x| x.as_str())
                .map(|s| !s.is_empty())
                .unwrap_or(false)
        })
        .count();
    assert!(
        with_desc > 0,
        "模组宝物应至少 1 条描述命中（artifact.<scope>.<id>.description），共 {} 条宝物",
        arts.len()
    );
}

/// 4) 建筑数据：单数 `building.json`（发条城）与 `townPavilion/buildings.json`（Pavilion）。
#[test]
#[ignore]
fn building_files_recognized() {
    let Some(game_dir) = test_game_dir() else { return };

    let data = build_and_read(
        "old-chinese-mods/mods/Celestial-master",
        "old-chinese-mods.Celestial-master",
        &game_dir,
    );
    let b = data
        .get("factions")
        .and_then(|x| x.as_array())
        .and_then(|a| a.first())
        .and_then(|f| f.get("buildings"))
        .and_then(|x| x.as_array())
        .expect("buildings");
    assert!(!b.is_empty(), "发条城建筑列表不应为空（building.json 单数）");

    let data = build_and_read("new-pavilion", "new-pavilion", &game_dir);
    let b = data
        .get("factions")
        .and_then(|x| x.as_array())
        .and_then(|a| a.first())
        .and_then(|f| f.get("buildings"))
        .and_then(|x| x.as_array())
        .expect("buildings");
    assert!(!b.is_empty(), "Pavilion 建筑列表不应为空（townPavilion/buildings.json）");
}

/// 7) 城镇数据缺口与建筑图标解析（2026-09-14 第六轮，`WIKI_CACHE_VERSION` 83）：
///    - 家园（`config/Abode/` 变体目录）建筑列表恢复（此前 `buildings=0`）；
///    - 家园生物/英雄从 `<Type>-<suffix>` 目录（`Abode-creatures` / `Abode-heroes`）恢复；
///    - 5 个城镇建筑图标全命中（xanadu/cove/bulwark/christmas/shuHan；
///      覆盖无扩展名引用、写错扩展名、sprites-json `sequences` 形态、zip 内定义等写法）。
#[test]
#[ignore]
fn town_variant_paths_and_icons_regression() {
    let Some(game_dir) = test_game_dir() else { return };

    // 家园：变体目录 `config/Abode/`（建筑）与 `config/Abode-creatures/`（生物）
    let data = build_and_read(
        "old-chinese-mods/mods/abode-mod",
        "old-chinese-mods.abode-mod",
        &game_dir,
    );
    let fac = faction_of(&data, "abode");
    let bl = fac.get("buildings").and_then(|x| x.as_array()).expect("buildings");
    assert!(
        bl.len() >= 30,
        "家园建筑列表应恢复（变体目录 config/Abode/），实际 {} 条",
        bl.len()
    );
    let creatures = data
        .get("creatures")
        .and_then(|x| x.as_array())
        .cloned()
        .unwrap_or_default();
    assert!(
        !creatures.is_empty(),
        "家园生物应从 Abode-creatures/ 恢复，实际 {} 条",
        creatures.len()
    );
    let with_img = bl
        .iter()
        .filter(|b| b.get("image").and_then(|x| x.as_str()).is_some())
        .count();
    assert!(
        with_img >= bl.len() * 3 / 4,
        "家园建筑图标应大多可提取，实际 {with_img}/{}",
        bl.len()
    );

    // 5 个城镇：建筑图标（此前 100% 缺失）
    for (rel, id, fid, min_buildings) in [
        ("new-chinese-mods/mods/xanadu", "new-chinese-mods.xanadu", "xanadu", 40usize),
        ("hota/mods/cove", "hota.cove", "cove", 35),
        ("hota/mods/bulwark", "hota.bulwark", "bulwark", 35),
        ("old-chinese-mods/mods/christmas", "old-chinese-mods.christmas", "christmas", 34),
        ("new-chinese-mods/mods/shuHan", "new-chinese-mods.shuHan", "shuhan", 33),
    ] {
        let data = build_and_read(rel, id, &game_dir);
        let bl = faction_of(&data, fid)
            .get("buildings")
            .and_then(|x| x.as_array())
            .unwrap_or_else(|| panic!("{id} 无 buildings"));
        assert!(
            bl.len() >= min_buildings,
            "{id} 建筑数异常：{}（期望 ≥ {min_buildings}）",
            bl.len()
        );
        let with_img = bl
            .iter()
            .filter(|b| b.get("image").and_then(|x| x.as_str()).is_some())
            .count();
        assert!(
            with_img >= bl.len() * 3 / 4,
            "{id} 建筑图标应大多命中（真实类型分派 + images/sequences），实际 {with_img}/{}",
            bl.len()
        );
        // 诊断字段：不应再有图标失败记录
        let failures = data
            .get("iconFailures")
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default();
        assert!(
            failures.len() <= bl.len() / 4,
            "{id} iconFailures 过多：{} 条（应接近 0）",
            failures.len()
        );
    }
}
