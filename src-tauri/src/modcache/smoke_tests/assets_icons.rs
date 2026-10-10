//! DEF 帧解码 / webp 编码 / 图标与肖像提取（冒烟测试，多数 `#[ignore]`，需真实游戏目录）。

use super::*;
use crate::vcmi::resource::set_wiki_user_data_dir;
use crate::modcache::set_assets_enabled;

/// 游戏资源提取冒烟测试（依赖本机游戏目录，默认跳过）：
/// 覆盖 lod 解包（SMALRES/stackWindow 均在 H3sprite.lod 内）→ webp 产物落盘。
/// 运行：cargo test -- --ignored
/// 全量 batch 重建测试（ignored，依赖本机 mod + 游戏目录）：
/// 验证 v9 提取逻辑在全部城镇 mod 上的 preview 生成率 / 资源图标 / 名称多语言。
#[test]
#[ignore]
fn structure_frame_probe() {
    let game_dir = game_dir();
    let core = game_dir.join("Mods").join("vcmi");
    let p = fixture("HuaXia");
    let anims = [
        "huaXia/town-screen/buildings/0mag1.json",
        "Content/Sprites/huaXia/town-screen/buildings/0mag1.json",
    ];
    for a in anims {
        let r1 = resolve_resource(&p, a);
        println!("resolve({a}) -> {}", r1.is_some());
        let img = load_structure_frame(&p, &core, &game_dir, a);
        println!("load_structure_frame({a}) -> {}", img.is_some());
    }
    // compose 直调：强制重建 HuaXia 预览并打印结果
    {
        let cfgs = read_town_configs(&p);
        println!("read_town_configs n={}", cfgs.len());
        for c in &cfgs {
            println!(
                "  cfg faction={} bld_ids={} bld_detail={} creatures={} bg={}",
                c.faction_id,
                c.building_ids.len(),
                c.buildings_detail.len(),
                c.creature_ids.len(),
                c.town_background.is_some()
            );
        }
    }
    let mods_dir = test_mods_dir();
    let cache_root = default_cache_root(&game_dir);
    let towns = collect_all_raw_configs(&mods_dir.join("HuaXia"));
    let towns_v = Value::Object(towns);
    let _ = fs::remove_file(cache_root.join("HuaXia/images/townscreen/huaXia.webp"));
    let _ = fs::remove_file(cache_root.join("HuaXia/cache.json"));
    match compose_town_preview_for(&mods_dir.join("HuaXia"), &cache_root, "HuaXia", "huaXia", &towns_v) {
        Ok(rel) => println!("compose OK -> {rel}"),
        Err(e) => println!("compose ERR -> {e}"),
    }
}

#[test]
#[ignore]
fn game_resource_icons_extract_smoke() {
    let game_dir = game_dir();
    assert!(game_dir.is_dir(), "游戏目录不存在：{}", game_dir.display());
    let cache_root = default_cache_root(&game_dir);
    let _ = fs::create_dir_all(&cache_root);

    // 1) 资源图标（SMALRES，lod 内）
    extract_resource_icons_for_mod(&game_dir, &cache_root, "abyss-town");
    assert!(
        cache_root.join("core/images/resources/0.webp").is_file(),
        "resources/0.webp 应生成（SMALRES 提取，归位到 core/images）"
    );

    // 2) 生物表 stat 表头图标（stackWindow/*.def，lod 内）
    let stat_icons = extract_stat_icons_for_mod(&game_dir, &cache_root, "abyss-town");
    let got = stat_icons.as_object().map(|m| m.len()).unwrap_or(0);
    assert!(got >= 6, "statIcons 应提取到至少 6 个，实际 {}", got);

    // 3) 英雄特长 default 图标（UN44，lod 内）—— HuaXia 英雄 specialtyLarge 非 default 时此步为 0 属预期
    //    已由 mod 内自定义 sprite 覆盖（herospec/ 36 个），此处不强制断言

    // 4) 用新代码重建真实缓存（cache.json 含 statIcons/建筑图标/preview 字段 + raw.json 归一结构）
    //    先清缓存绕过 sourceHash 短路，保证走完整重建路径
    let _ = fs::remove_file(cache_root.join("abyss-town/cache.json"));
    let _ = fs::remove_file(cache_root.join("abyss-town/raw.json"));
    // 默认走仓库内 references 符号链接；若本机符号链接不可访问（Windows 权限），
    // 可用 VCMI_TEST_MOD_DIR 指向真实 mod 目录。
    let mod_dir = fixture("abyss-town");
    let built = build_mod_wiki_inner(&mod_dir.to_string_lossy(), "abyss-town", &cache_root, &game_dir);

    // 5) 自愈重建 mock index.json（清缓存后跑测试即可恢复浏览器自测数据源）
    // ⚠️ 仅在索引缺失/为空时兜底生成（本逻辑条目字段较简，name/type 为空）；
    //    已有非空索引（rebuild_all_with_core / 启动器构建产物）时**不得覆盖**，
    //    否则浏览器自测的名称检索（如「深渊」）会因 name 为空而失效。
    {
        let mut index_mods: Vec<Value> = Vec::new();
        if let Ok(entries) = fs::read_dir(&cache_root) {
            for e in entries.flatten() {
                let d = e.path();
                let mp = d.join("cache.json");
                if !mp.is_file() {
                    continue;
                }
                if let Ok(text) = fs::read_to_string(&mp) {
                    if let Ok(v) = serde_json::from_str::<Value>(&text) {
                        let mut counts = serde_json::Map::new();
                        // spells/skills 拆分 builtin（原版注入，全局只计一次）与自定义
                        let builtin_of = |key: &str| -> usize {
                            v.get(key)
                                .and_then(|x| x.as_array())
                                .map(|a| {
                                    a.iter()
                                        .filter(|s| {
                                            s.get("builtin").and_then(|b| b.as_bool()).unwrap_or(false)
                                        })
                                        .count()
                                })
                                .unwrap_or(0)
                        };
                        for key in ["factions", "creatures", "heroes", "spells", "artifacts", "skills"] {
                            let n = v.get(key).and_then(|x| x.as_array()).map(|a| a.len()).unwrap_or(0);
                            let n = if key == "spells" {
                                n - builtin_of("spells")
                            } else if key == "skills" {
                                n - builtin_of("skills")
                            } else {
                                n
                            };
                            counts.insert(key.to_string(), json!(n));
                        }
                        counts.insert("spellBuiltin".to_string(), json!(builtin_of("spells")));
                        counts.insert("skillBuiltin".to_string(), json!(builtin_of("skills")));
                        let mid = v.get("modId").and_then(|x| x.as_str()).unwrap_or("").to_string();
                        if mid.is_empty() {
                            continue;
                        }
                        let dname = d.file_name().and_then(|x| x.to_str()).unwrap_or("").to_string();
                        index_mods.push(json!({
                            "modId": mid,
                            "diskName": dname,
                            "parentId": mid,
                            "type": "",
                            "name": "",
                            "encrypted": false,
                            "cacheDir": format!("mods/{}", dname),
                            "contentCount": Value::Object(counts),
                        }));
                    }
                }
            }
        }
        let index = json!({ "generatedAt": crate::infra::time::now_millis(), "version": 2, "mods": index_mods });
        let existing_nonempty = fs::read_to_string(cache_root.join("index.json"))
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(&t).ok())
            .and_then(|v| v.get("mods").and_then(|x| x.as_array()).map(|a| !a.is_empty()))
            .unwrap_or(false);
        if !existing_nonempty {
            let _ = fs::write(cache_root.join("index.json"), serde_json::to_string_pretty(&index).unwrap());
        }
    }
    assert!(
        built.get("ok").and_then(|x| x.as_bool()).unwrap_or(false),
        "abyss-town 重建应成功"
    );

    // 5) 城镇预览合成（背景+structures 叠加 → images/townscreen/）
    let fac_id = built
        .get("factions")
        .and_then(|f| f.as_array())
        .and_then(|a| a.first())
        .and_then(|f| f.get("factionId"))
        .and_then(|x| x.as_str())
        .unwrap_or("abyss")
        .to_string();
    let prev = generate_town_preview_inner(&cache_root, "abyss-town", &fac_id);
    assert!(
        prev.get("ok").and_then(|x| x.as_bool()).unwrap_or(false),
        "城镇预览合成应成功: {}",
        prev
    );

    // 6) cache.json 自检：statIcons 非空、建筑图标无缺失、preview 已写入
    let mj: Value = serde_json::from_str(
        &fs::read_to_string(cache_root.join("abyss-town/cache.json")).expect("cache.json"),
    )
    .expect("cache.json parse");
    let stat_n = mj.get("statIcons").and_then(|s| s.as_object()).map(|m| m.len()).unwrap_or(0);
    assert!(stat_n >= 6, "cache.json statIcons 应 ≥6，实际 {}", stat_n);
    let f0 = mj.get("factions").and_then(|f| f.as_array()).and_then(|a| a.first()).cloned().unwrap_or(json!({}));
    let bld_empty = f0
        .get("buildings")
        .and_then(|b| b.as_array())
        .map(|a| a.iter().filter(|b| b.get("image").and_then(|x| x.as_str()).unwrap_or("").is_empty()).count())
        .unwrap_or(999);
    assert_eq!(bld_empty, 0, "建筑图标缺失数应为 0，实际 {}", bld_empty);
    assert!(
        f0.get("preview").and_then(|x| x.as_str()).unwrap_or("").len() > 0,
        "preview 应写入 cache.json"
    );

    // 7) 建筑 bid 应对齐 VCMI buildingsLibrary（buildingsIcons 按 bid 取帧）
    let bl = f0.get("buildings").and_then(|b| b.as_array()).cloned().unwrap_or_default();
    for (name, want) in [("mageGuild1", 0i64), ("tavern", 5), ("fort", 7), ("dwellingLvl1", 30), ("dwellingUpLvl1", 37)] {
        let got = bl
            .iter()
            .find(|b| b.get("id").and_then(|x| x.as_str()) == Some(name))
            .and_then(|b| b.get("bid"))
            .and_then(|x| x.as_i64())
            .unwrap_or(-1);
        assert_eq!(got, want, "建筑 {name} 的 bid 应为 {want}，实际 {got}");
    }
}

/// 回归守卫：原版 LOD 放在 `user_data_dir/Data`（便携场景，`game_dir/Data` 无 LOD）时，
/// `extract_resource_icons_for_mod` 必须能经 `wiki_user_data_dir` 回退取到 `SMALRES.DEF` 并写出
/// `core/images/resources/0.webp`。此前它用 `read_game_resource(game_dir, …)` 只搜 `game_dir/Data`，
/// 便携场景会静默无图（见 `references/Heroes3-VCMI` 实测日志缺失 `core/images/resources/0..6.webp`）。
#[test]
#[ignore]
fn resource_icons_extracted_from_user_data_dir() {
    set_assets_enabled(true);
    let lod_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../references/Heroes3-VCMI");
    assert!(
        lod_dir.join("Data").join("H3sprite.lod").is_file()
            || lod_dir.join("Data").join("H3Sprite.lod").is_file(),
        "references 增强版应含 Data/*.lod：{}",
        lod_dir.display()
    );
    set_wiki_user_data_dir(&lod_dir);
    // game_dir 传一个绝不带 Data/LOD 的 mod 目录（abyss-town）
    let game_dir = fixture("abyss-town");
    let cache_root = std::env::temp_dir().join("vcmi-fv-test-resicons");
    let _ = fs::create_dir_all(&cache_root);
    let _ = fs::remove_dir_all(cache_root.join("core").join("images").join("resources"));
    extract_resource_icons_for_mod(&game_dir, &cache_root, "abyss-town");
    assert!(
        cache_root.join("core/images/resources/0.webp").is_file(),
        "resources/0.webp 应经 user_data_dir/Data 回退生成（SMALRES 提取）"
    );
}

#[test]
#[ignore = "需 references/Heroes3-VCMI 游戏 fixture"]
fn test_encode_webp() {
    let p = fixture("abyss-town");
    let configs = read_town_configs(&p);
    assert!(!configs.is_empty(), "abyss 应能读城镇配置");
    let cfg = &configs[0];
    let res = cfg.town_background.as_deref().expect("abyss 应有背景");
    let (buf, ext) = resolve_resource(&p, res).expect("应能定位背景图");
    let webp = encode_webp(&buf, &ext).expect("应能编码为 webp");
    assert!(webp.starts_with(b"RIFF"), "webp 应以 RIFF 开头，实际前4字节: {:?}", &webp[..4.min(webp.len())]);
    assert!(webp.len() < buf.len(), "webp 应比原 bmp 小（压缩），原={} webp={}", buf.len(), webp.len());
    println!("{}: bmp={} webp={}", cfg.faction_id, buf.len(), webp.len());
}

#[test]
#[ignore = "需 references/Heroes3-VCMI 游戏 fixture"]
fn test_def_animation_frames() {
    // 评估动画模式：生物 .def 动画能否解出多方向组/多帧
    let p = fixture("abyss-town");
    let (buf, _ext) = resolve_resource(&p, "abyss/battle/creatures/abyssalcrab.def")
        .expect("应能定位生物战斗动画 .def");
    eprintln!("battle def 大小: {} 字节", buf.len());
    let groups = crate::infra::def::DefReader::new(&buf).decode_animation();
    match groups {
        Some(g) => {
            eprintln!("解码出 {} 个方向组", g.len());
            for (gi, w, h, _) in &g {
                eprintln!("  group[{}]: {}x{}", gi, w, h);
            }
            assert!(!g.is_empty(), "至少应解出 1 个方向组");
        }
        None => {
            eprintln!("动画模式评估：battle def 解码失败（可能组结构特殊）");
        }
    }
    // 地图动画
    if let Some((mapbuf, _)) = resolve_resource(&p, "abyss/map/creatures/AVWAbyssalCrab.def") {
        let mg = crate::infra::def::DefReader::new(&mapbuf).decode_animation();
        eprintln!("map def 方向组数: {}", mg.as_ref().map(|x| x.len()).unwrap_or(0));
    }
}

#[test]
#[ignore = "需 references/Heroes3-VCMI 游戏 fixture"]
fn test_def_first_frame_bmp() {
    // abyss-town 的 town-screen/buildings 下有 .def（建筑动画）
    let p = fixture("abyss-town");
    let zip_path = p.join("content.zip");
    let file = fs::File::open(&zip_path).expect("content.zip 应存在");
    let mut archive = zip::ZipArchive::new(file).expect("zip 可打开");
    let mut found: Option<Vec<u8>> = None;
    for i in 0..archive.len() {
        if let Ok(mut entry) = archive.by_index(i) {
            let name = entry.name().to_lowercase();
            if name.contains("town-screen") && name.ends_with(".def") {
                let mut buf = Vec::new();
                if std::io::Read::read_to_end(&mut entry, &mut buf).is_ok() {
                    found = Some(buf);
                    break;
                }
            }
        }
    }
    let buf = found.expect("应在 town-screen 找到 .def 文件");
    let bmp = crate::infra::def::DefReader::new(&buf).first_frame_bmp();
    assert!(bmp.is_some(), "DEF 解码应输出 BMP");
    let bmp = bmp.unwrap();
    assert_eq!(&bmp[0..2], b"BM");
    println!("DEF 解码为 BMP，{} 字节", bmp.len());
}

#[test]
#[ignore]
fn shared_ui_icons_portraits_smoke() {
    let game_dir = game_dir();
    let cache_root = default_cache_root(&game_dir);
    let icons = extract_shared_ui_icons(&game_dir, &cache_root, "vcmi");
    println!("sharedIcons keys = {:?}", icons.as_object().map(|o| o.keys().collect::<Vec<_>>()));
    // 英雄大头像已改为按模组抽取（core.<faction>/images/hero/），不再落入全局 sharedIcons，
    // 故此处不再断言 portraitsLargeFrames。
    // 生物弹窗属性图标 + 英雄四主属性图标
    for k in ["stackwin/shots", "stackwin/mana", "pskil42/attack", "pskil42/spellPower"] {
        assert!(icons.get(k).is_some(), "缺少公共 UI 图标 {k}");
    }
}
