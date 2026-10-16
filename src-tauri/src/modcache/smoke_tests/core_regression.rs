//! 原版 core / h3-core 数据回归（冒烟测试，多数 `#[ignore]`，需真实游戏目录）。

use super::*;

/// 原版 core 拆分回归：父 `core` 不持有城镇；`core.<faction>` 子模组只持有本阵营的
/// 1 座城镇 + 本阵营生物/英雄，形态与第三方 town mod 一致。
/// 依赖真实游戏目录（含 Mods/vcmi），故默认忽略，需实机时显式运行：
/// `cargo test -p vcmi-fv-launcher core_split -- --ignored --nocapture`
#[test]
#[ignore]
fn core_faction_split_regression() {
    // 与既有实机测试一致：优先 VCMI_GAME_DIR，未设置时回落仓库 references 验证 fixture
    let game_dir = game_dir();
    let core_mod = game_dir.join("Mods").join("vcmi");
    if !core_mod.is_dir() {
        eprintln!("未检测到 {} 内的 Mods/vcmi，跳过", game_dir.display());
        return;
    }
    let cache = std::env::temp_dir().join("vcmi_fv_core_split_test");
    let _ = fs::remove_dir_all(&cache);
    let _ = fs::create_dir_all(&cache);

    // 父 core：全局数据，不应含城镇
    let parent = build_mod_wiki_inner(&core_mod.to_string_lossy(), "core", &cache, &game_dir);
    assert_eq!(
        parent.get("ok").and_then(|x| x.as_bool()),
        Some(true),
        "父 core 应构建成功: {parent}"
    );
    let parent_txt = fs::read_to_string(cache.join("core/cache.json")).unwrap();
    let parent_json: Value = serde_json::from_str(&parent_txt).unwrap();
    let parent_factions = parent_json
        .get("factions")
        .and_then(|x| x.as_array())
        .unwrap();
    assert!(
        parent_factions.is_empty(),
        "父 core 不应持有城镇，实际 {} 座",
        parent_factions.len()
    );
    // 头像定义文件（portraits*.json）不是英雄实体，不应混入父 core
    let parent_heroes = parent_json
        .get("heroes")
        .and_then(|x| x.as_array())
        .unwrap();
    let bogus = parent_heroes
        .iter()
        .filter_map(|h| h.get("id").and_then(|x| x.as_str()))
        .filter(|id| id.starts_with("portrait"))
        .count();
    assert_eq!(
        bogus, 0,
        "父 core 不应包含 portrait* 伪英雄，实际 {bogus} 个"
    );

    // 子模组 core.castle：应恰好 1 座城镇，且生物/英雄非空
    let child = build_mod_wiki_inner(
        &core_mod.to_string_lossy(),
        "core.castle",
        &cache,
        &game_dir,
    );
    assert_eq!(
        child.get("ok").and_then(|x| x.as_bool()),
        Some(true),
        "core.castle 应构建成功: {child}"
    );
    let child_txt = fs::read_to_string(cache.join("core.castle/cache.json")).unwrap();
    let child_json: Value = serde_json::from_str(&child_txt).unwrap();
    let factions = child_json
        .get("factions")
        .and_then(|x| x.as_array())
        .unwrap();
    assert_eq!(factions.len(), 1, "core.castle 应恰好 1 座城镇");
    assert_eq!(
        factions[0].get("factionId").and_then(|x| x.as_str()),
        Some("castle"),
        "城镇应归属 castle"
    );
    let creatures = child_json
        .get("creatures")
        .and_then(|x| x.as_array())
        .unwrap();
    let heroes = child_json.get("heroes").and_then(|x| x.as_array()).unwrap();
    assert!(!creatures.is_empty(), "core.castle 应有本阵营生物");
    assert!(!heroes.is_empty(), "core.castle 应有本阵营英雄");
    // 原版生物属性必须非 0（数值来自 h3-core 烘焙的 CRTRAITS；配置本身不含）
    let with_stats = creatures
        .iter()
        .filter(|c| c.get("attack").and_then(|x| x.as_i64()).unwrap_or(0) > 0)
        .count();
    assert_eq!(
        with_stats,
        creatures.len(),
        "core.castle 全部生物都应有属性（实际 {with_stats}/{}）",
        creatures.len()
    );
    // 抽查 pikeman 已知原版数值：攻 4 / 防 5 / HP 10 / 速 4
    let pik = creatures
        .iter()
        .find(|c| c.get("id").and_then(|x| x.as_str()) == Some("pikeman"))
        .expect("应有 pikeman");
    assert_eq!(
        pik.get("attack").and_then(|x| x.as_i64()),
        Some(4),
        "pikeman 攻击应为 4"
    );
    assert_eq!(
        pik.get("defense").and_then(|x| x.as_i64()),
        Some(5),
        "pikeman 防御应为 5"
    );
    assert_eq!(
        pik.get("hitPoints").and_then(|x| x.as_i64()),
        Some(10),
        "pikeman HP 应为 10"
    );
    // 建筑动画资源名（仅文本引用，供前端按需懒加载 —— 设计 §11）
    let anims = factions[0]
        .get("buildingAnimations")
        .and_then(|x| x.as_object())
        .map(|m| m.len())
        .unwrap_or(0);
    assert!(
        anims > 0,
        "core.castle 应带建筑动画资源名（town.structures）"
    );
    println!(
        "core.castle: factions={} creatures={} heroes={}",
        factions.len(),
        creatures.len(),
        heroes.len()
    );
    // 抽样确认生物 id 真实存在（多 key 文件已正确展开，而非折叠成文件名）
    let sample = creatures[0]
        .get("id")
        .and_then(|x| x.as_str())
        .unwrap_or("");
    assert!(
        !sample.is_empty() && sample != "castle",
        "生物 id 不应是折叠后的文件名，实际: {sample}"
    );
}

/// 原版英雄四维回归：原版 `config/heroClasses.json` 不含 `primarySkills`，
/// 旧代码在 heroClass 循环里无条件 `unwrap_or(0)`，会把 h3-core（HCTRAITS.TXT）
/// 桥接得到的职业基础四维覆盖成 0/0/0/0（约克等原版英雄四维全 0 的根因）。
/// 修复后野蛮人约克应显示 4/0/1/1。
///
/// 遍历父 `core` 与 9 个 `core.<faction>`，断言所有原版英雄四维均不全为 0。
/// 默认写入临时目录；`VCMI_MOCK_CACHE_DIR` 可指向真实缓存以就地修复（与
/// `rebuild_all_with_core` 同款约定）。
#[test]
#[ignore]
fn core_hero_primary_skills_regression() {
    let game_dir = game_dir();
    let core_mod = game_dir.join("Mods").join("vcmi");
    if !core_mod.is_dir() {
        eprintln!("未检测到 {} 内的 Mods/vcmi，跳过", game_dir.display());
        return;
    }
    let cache = match std::env::var("VCMI_MOCK_CACHE_DIR") {
        Ok(d) => PathBuf::from(d),
        Err(_) => {
            let d = std::env::temp_dir().join("vcmi_fv_core_hero_prim_test");
            let _ = fs::remove_dir_all(&d);
            d
        }
    };
    let _ = fs::create_dir_all(&cache);

    let mod_ids = [
        "core",
        "core.castle",
        "core.rampart",
        "core.tower",
        "core.inferno",
        "core.necropolis",
        "core.dungeon",
        "core.stronghold",
        "core.fortress",
        "core.conflux",
    ];
    for mod_id in mod_ids {
        let r = build_mod_wiki_inner(&core_mod.to_string_lossy(), mod_id, &cache, &game_dir);
        assert_eq!(
            r.get("ok").and_then(|x| x.as_bool()),
            Some(true),
            "{mod_id} 应构建成功: {r}"
        );
        let v: Value = serde_json::from_str(
            &fs::read_to_string(cache.join(format!("{mod_id}/cache.json"))).unwrap(),
        )
        .unwrap();
        let heroes = v.get("heroes").and_then(|x| x.as_array()).unwrap();
        // 原版英雄四维不得全 0（数值来自 h3-core 烘焙的 HCTRAITS；配置本身不含）
        let zero = heroes
            .iter()
            .filter(|h| {
                ["attack", "defense", "spellPower", "knowledge"]
                    .iter()
                    .all(|k| h.get(k).and_then(|x| x.as_i64()).unwrap_or(0) == 0)
            })
            .count();
        assert_eq!(zero, 0, "{mod_id} 不应有英雄四维全 0，实际 {zero} 个");
    }

    // 抽查：野蛮人约克 = 4/0/1/1（h3-core class12）
    let v: Value = serde_json::from_str(
        &fs::read_to_string(cache.join("core.stronghold/cache.json")).unwrap(),
    )
    .unwrap();
    let heroes = v.get("heroes").and_then(|x| x.as_array()).unwrap();
    let yog = heroes
        .iter()
        .find(|h| h.get("id").and_then(|x| x.as_str()) == Some("yog"))
        .expect("应有约克 yog");
    assert_eq!(
        yog.get("attack").and_then(|x| x.as_i64()),
        Some(4),
        "约克攻击应为 4"
    );
    assert_eq!(
        yog.get("defense").and_then(|x| x.as_i64()),
        Some(0),
        "约克防御应为 0"
    );
    assert_eq!(
        yog.get("spellPower").and_then(|x| x.as_i64()),
        Some(1),
        "约克力量应为 1"
    );
    assert_eq!(
        yog.get("knowledge").and_then(|x| x.as_i64()),
        Some(1),
        "约克知识应为 1"
    );
    println!(
        "core 英雄四维回归通过：yog=4/0/1/1，缓存根={}",
        cache.display()
    );
}

/// 回归：同一实体分散在多个配置文件时须**合并**而非覆盖。
/// 典型病例 `new-chinese-mods.nightelf-town`：`creatures/*.json` 定义完整生物，
/// 而 `creatures/stack_exp.json` 又用同名键只补 `stackExperience`——
/// 若整条覆盖，生物会变成「名回落 id、属性全 0、无图」。
/// 同时校验城镇名兜底（该模组无 translation 目录，中文名写在 town.json 的 name）。
#[test]
#[ignore]
fn merge_config_and_town_name_regression() {
    let game_dir = game_dir();
    let mod_dir = game_dir
        .join("Mods")
        .join("new-chinese-mods")
        .join("Mods")
        .join("nightelf-town");
    if !mod_dir.is_dir() {
        eprintln!("未找到 {}，跳过", mod_dir.display());
        return;
    }
    let cache = std::env::temp_dir().join("vcmi_fv_merge_regression");
    let _ = fs::remove_dir_all(&cache);
    let _ = fs::create_dir_all(&cache);
    let mod_id = "new-chinese-mods.nightelf-town";
    let r = build_mod_wiki_inner(&mod_dir.to_string_lossy(), mod_id, &cache, &game_dir);
    assert_eq!(
        r.get("ok").and_then(|x| x.as_bool()),
        Some(true),
        "构建应成功: {r}"
    );

    let v: Value = serde_json::from_str(
        &fs::read_to_string(cache.join(format!("{mod_id}/cache.json"))).unwrap(),
    )
    .unwrap();
    let creatures = v.get("creatures").and_then(|x| x.as_array()).unwrap();
    assert!(!creatures.is_empty(), "应有生物");
    let with_stats = creatures
        .iter()
        .filter(|c| c.get("attack").and_then(|x| x.as_i64()).unwrap_or(0) > 0)
        .count();
    assert!(
        with_stats > 0,
        "合并后应有生物带属性（整条覆盖会全部为 0），实际带属性 {} / {}",
        with_stats,
        creatures.len()
    );
    // 城镇名不应回落为英文 faction id
    let fname = v
        .get("factions")
        .and_then(|x| x.as_array())
        .and_then(|a| a.first())
        .and_then(|f| f.get("name"))
        .and_then(|n| n.get("chinese"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    assert_ne!(fname, "nightelf", "城镇名应使用配置自带中文名，而非回落 id");
    println!(
        "合并/名称回归通过：生物带属性 {with_stats}/{}，城镇名={fname}",
        creatures.len()
    );
}

/// h3-core 文案表必须能被定位并加载（不依赖游戏目录）。
/// 曾因用*运行时* `CARGO_MANIFEST_DIR` 定位，开发运行时找不到 → 日志 `h3 keys=0`，
/// 原版名称/描述全部回落成 id。此测试即为该缺陷的守门测试。
#[test]
fn h3_core_data_loads() {
    let dir =
        h3_core_data_dir().expect("应能定位 h3-core 数据目录（开发期为编译期 CARGO_MANIFEST_DIR）");
    let m = load_h3_core_i18n(&dir).expect("应能加载 h3-core 文案表");
    assert!(
        m.len() > 100,
        "h3-core 文案条目过少：{}（疑似未加载）",
        m.len()
    );
    for k in [
        "skill.core.pathfinding.name",
        "skill.core.pathfinding.expert",
        "spell.core.fireball.name",
        "spell.core.fireball.level",
        "terrain.core.dirt.name",
        "faction.core.castle.name",
        "building.core.castle.grail.name",
        "building.core.tower.blacksmith.name",
        "building.core.tower.mageGuild1.name",
        "building.core.tower.dwellingLvl1.name",
        "building.core.tower.townHall.name",
    ] {
        // 键在加载时**统一小写**（对齐 VCMI 的大小写不敏感翻译查找，
        // 见 `load_h3_core_i18n` 尾部注释），查询侧同样小写化
        assert!(m.contains_key(&k.to_lowercase()), "h3-core 应包含键 {k}");
    }
    eprintln!(
        "DEBUG building.core.tower.blacksmith.name = {:?}",
        m.get("building.core.tower.blacksmith.name")
    );
}

/// 原版伪模组（`core` / `core.<faction>`）的显示名必须**从数据派生且多语言**。
/// 守门背景：此前前端写死中文枚举表 → 英文界面也显示中文（且 chinese/english 同值）。
/// 纯逻辑：只依赖随包 h3-core 文案表，不需要游戏目录。
#[test]
fn core_mod_display_name_is_multilingual() {
    // ① 无缓存阵营名 → 回退随包 h3-core 文案表（en/zh）
    let castle = crate::modcache::index::core_mod_display_name("core.castle", None)
        .expect("core.<faction> 应派生显示名");
    assert_eq!(
        castle.get("chinese").and_then(|x| x.as_str()),
        Some("VCMI · 城堡")
    );
    assert_eq!(
        castle.get("english").and_then(|x| x.as_str()),
        Some("VCMI · Castle")
    );

    // ② 有缓存阵营名 → 优先用它（可含翻译 mod 的 tchinese/russian 等语言，与城镇实体名一致）
    let town = serde_json::json!({ "chinese": "城堡", "tchinese": "城堡", "russian": "Замок" });
    let from_cache = crate::modcache::index::core_mod_display_name("core.castle", town.as_object())
        .expect("core.<faction> 应派生显示名");
    assert_eq!(
        from_cache.get("russian").and_then(|x| x.as_str()),
        Some("VCMI · Замок")
    );
    assert_eq!(
        from_cache.get("tchinese").and_then(|x| x.as_str()),
        Some("VCMI · 城堡")
    );

    // ③ 父 core 用语言中立的引擎名；常规模组不参与派生（名称来自 modMeta.name / 扫描期 name）
    let core =
        crate::modcache::index::core_mod_display_name("core", None).expect("core 应派生显示名");
    assert_eq!(core.get("english").and_then(|x| x.as_str()), Some("VCMI"));
    assert!(crate::modcache::index::core_mod_display_name("hota", None).is_none());
}

#[test]
#[ignore]
fn core_town_background_resolves_regression() {
    // 原版 9 城镇的 townBackground/creatureBackground 在配置里写作 ".bmp" 甚至无扩展名，
    // 实际是 H3bitmap.lod 内的 .pcx（如 TBCSBACK.bmp→TBCSBACK.PCX、CRBKGCAS→CRBKGCAS.PCX）。
    // 验证 resolve_resource_fallback（经 read_game_resource_ext 的「基名+多扩展名」回退）能取到
    // 正确字节并带上 pcx 扩展名，供 encode_webp 解码——否则 wiki 原版城镇背景/兵种背景为 null。
    let game_dir = game_dir();
    let core_mod = game_dir.join("Mods").join("vcmi");
    let configs = crate::vcmi::town::read_game_root_factions(&game_dir);
    assert_eq!(configs.len(), 9, "应解析出 9 个原版城镇");
    for c in &configs {
        let tb = c.town_background.as_deref().unwrap_or("");
        let cb = c.creature_background.as_deref().unwrap_or("");
        assert!(!tb.is_empty(), "{} 应有 town_background", c.faction_id);
        assert!(!cb.is_empty(), "{} 应有 creature_background", c.faction_id);
        let (b, e) = crate::vcmi::resource::resolve_resource_fallback(&core_mod, &game_dir, tb)
            .unwrap_or_else(|| panic!("{} town_background {} 应可解析", c.faction_id, tb));
        assert_eq!(e, "pcx", "{} town_background 应为 pcx", c.faction_id);
        assert!(
            b.len() > 1000,
            "{} town_background 字节应非空",
            c.faction_id
        );
        let (_, ce) = crate::vcmi::resource::resolve_resource_fallback(&core_mod, &game_dir, cb)
            .unwrap_or_else(|| panic!("{} creature_background {} 应可解析", c.faction_id, cb));
        assert_eq!(ce, "pcx", "{} creature_background 应为 pcx", c.faction_id);
    }
}

#[test]
#[ignore]
fn core_town_background_composites_buildings() {
    // 原版城镇背景（如 castle 的 TBCSBACK）只是裸底图，须把建筑精灵按位置叠加，
    // 才能与其它（背景已预合成）的城镇视觉一致。回归：合成图尺寸=底图、且显著区别于裸底图。
    let game_dir = game_dir();
    let configs = crate::vcmi::town::read_game_root_factions(&game_dir);
    let castle = configs
        .iter()
        .find(|c| c.faction_id == "castle")
        .expect("应有原版 castle 配置");
    assert!(
        !castle.structures.is_empty(),
        "原版 castle 应含建筑结构位置"
    );
    let (buf, ext) =
        crate::vcmi::resource::resolve_resource_fallback(&game_dir, &game_dir, "TBCSBACK.bmp")
            .expect("应解析到 TBCSBACK 资源");
    let base_webp = crate::infra::image::encode_webp(&buf, &ext).expect("底图应可解码");
    let base = image::load_from_memory(&base_webp).unwrap().to_rgba8();
    let comp_webp = composite_structures_to_canvas(
        &game_dir,
        &game_dir,
        &buf,
        &ext,
        &castle.structures,
        &castle.buildings_raw,
    )
    .expect("合成应成功");
    let comp = image::load_from_memory(&comp_webp).unwrap().to_rgba8();
    assert_eq!(comp.dimensions(), base.dimensions(), "合成图应保持底图尺寸");
    let diff = base.iter().zip(comp.iter()).filter(|(a, b)| a != b).count();
    assert!(
        diff > 100_000,
        "合成图应与裸底图有显著像素差异（建筑叠加生效），实得 {}",
        diff
    );
    let tl = comp.get_pixel(0, 0);
    assert!(tl[2] >= tl[0], "顶部应仍是天空蓝（背景未被破坏）");
}

#[test]
#[ignore]
fn core_town_background_cached_is_composited() {
    // 长效守卫：缓存产物 core.castle/images/town/castle.webp 必须是"底图 + 建筑"合成图，
    // 而非裸底图。读缓存 webp 与裸 TBCSBACK 底图对比，应有显著像素差异。
    let game_dir = game_dir();
    let cache_root = default_cache_root(&game_dir)
        .join("core.castle")
        .join("images")
        .join("town")
        .join("castle.webp");
    let bytes = fs::read(&cache_root).expect("缓存 webp 应存在（需先 --fresh 重建）");
    let comp = image::load_from_memory(&bytes)
        .expect("缓存 webp 应可解码")
        .to_rgba8();
    let (buf, ext) =
        crate::vcmi::resource::resolve_resource_fallback(&game_dir, &game_dir, "TBCSBACK.bmp")
            .expect("应解析到 TBCSBACK 资源");
    let base = image::load_from_memory(&crate::infra::image::encode_webp(&buf, &ext).unwrap())
        .unwrap()
        .to_rgba8();
    assert_eq!(comp.dimensions(), base.dimensions(), "缓存图应保持底图尺寸");
    let diff = base.iter().zip(comp.iter()).filter(|(a, b)| a != b).count();
    assert!(
        diff > 100_000,
        "缓存背景应与裸底图有显著像素差异（建筑已叠加），实得 {}",
        diff
    );
}

#[test]
#[ignore]
fn core_original_creature_hero_images_present() {
    // 长效守卫：原版生物/英雄图片经 iconIndex/imageIndex 回退补齐后，
    // references 缓存 core.castle/cache.json 的 creature.imageLarge / hero.image / hero.specialtyIcon
    // 应非空且指向的文件确实存在（原版此前这些字段为 null）。
    let game_dir = game_dir();
    let cache_root = default_cache_root(&game_dir);
    let bytes = fs::read(cache_root.join("core.castle").join("mod.json"))
        .expect("缓存应存在（需先 --fresh 重建）");
    let j: Value = serde_json::from_slice(&bytes).unwrap();
    // 生物大图（原版 iconIndex → TWCRPORT.DEF）
    let champ = j
        .get("creatures")
        .and_then(|a| a.as_array())
        .unwrap()
        .iter()
        .find(|c| c.get("id").and_then(|x| x.as_str()) == Some("champion"))
        .expect("应有 champion");
    let img_large = champ
        .get("imageLarge")
        .and_then(|x| x.as_str())
        .expect("原版生物 imageLarge 应非空");
    assert!(
        fs::metadata(cache_root.join(img_large)).is_ok(),
        "imageLarge 文件应存在: {img_large}"
    );
    // 英雄头像（原版 imageIndex → PortraitsLarge.DEF）
    let orrin = j
        .get("heroes")
        .and_then(|a| a.as_array())
        .unwrap()
        .iter()
        .find(|h| h.get("id").and_then(|x| x.as_str()) == Some("orrin"))
        .expect("应有 orrin");
    let img = orrin
        .get("image")
        .and_then(|x| x.as_str())
        .expect("原版英雄 image 应非空");
    assert!(
        fs::metadata(cache_root.join(img)).is_ok(),
        "image 文件应存在: {img}"
    );
    let spec = orrin
        .get("specialtyIcon")
        .and_then(|x| x.as_str())
        .expect("原版英雄 specialtyIcon 应非空");
    assert!(
        fs::metadata(cache_root.join(spec)).is_ok(),
        "specialtyIcon 文件应存在: {spec}"
    );
}

#[test]
#[ignore]
fn core_town_background_pcx_decode_regression() {
    // 原版城镇背景 TBCSBACK.PCX 是 H3 专有 24-bit PCX（非标准 PCX、无 0x0A 魔数），
    // 数据按 BGR888 存储。曾因两处 bug 解出乱码：①8bpp 分支判定 `len>=12+px+768`
    // 对 24bpp 也成立 → 误入 8bpp 把 BGR 像素当索引查文件尾部 → 乱码；
    // ②24bpp 按 RGB 读 → R/B 互换（红蓝颠倒）。
    // 此处断言 decode_h3_pcx 正确产出 800x374、顶部像素偏蓝（天空），且标准 PCX 解码器不误命中。
    let game_dir = game_dir();
    let raw = crate::infra::lod::read_game_resource_ext(&game_dir, "TBCSBACK.bmp")
        .unwrap()
        .0;
    assert!(
        crate::infra::image::decode_pcx(&raw).is_none(),
        "H3 专有 PCX 不应被标准 PCX 解码器误解"
    );
    let (w, h, rgba) =
        crate::infra::image::decode_h3_pcx(&raw).expect("TBCSBACK 应被 decode_h3_pcx 解出");
    assert_eq!((w, h), (800, 374));
    assert_eq!(rgba.len(), 800 * 374 * 4);
    let tl = (rgba[0], rgba[1], rgba[2]);
    // 顶部应为天空蓝：B 通道高于 R（BGR 已正确交换）
    assert!(tl.2 > tl.0, "背景顶部应偏蓝（BGR 修复生效），实得 {:?}", tl);
}
