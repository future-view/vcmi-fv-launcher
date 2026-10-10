//! HD 素材（`hd-edition` 的 `.pak`）端到端冒烟：**素材登记 → HD 变体生成**。
//!
//! 依赖真实游戏目录（`Data/*.lod`）、用户模组目录（含 `hd-edition`），故 `#[ignore]`。
//! 运行：`cargo test --lib modcache::smoke_tests::hd_edition -- --ignored --nocapture`

use super::*;
use crate::modcache::hd::{generate_for_mod, HdOptions};

// 游戏目录 / 用户模组目录统一走 `mod.rs` 的 `game_dir()` / `user_mods_dir()`（未设置环境变量时
// 回落仓库内 `references/Heroes3-VCMI` 验证 fixture，跨机器一致）。

/// 端到端：`core.castle` 阶段二应产出 HD 登记；`generate_for_mod` 应据 HD 源生成 `@Nx` 变体。
///
/// 断言链（对应 2026-09-23 拍板的两条策略）：
/// 1. **基准图仍是原版 1x**（阶段二产物尺寸不变）；
/// 2. **变体由 HD 源生成**：`hd-map.json` 有登记，且变体尺寸 ≈ HD 档 × 整数放大（≥ 3× 基准）。
#[test]
#[ignore]
fn core_castle_hd_registration_and_variants() {
    // 与 `vcmi::hd_edition` 的测试共用互斥：全局来源注入在并行测试间会互相干扰
    let _g = crate::vcmi::hd_edition::test_guard();
    let game = game_dir();
    let cache_root = std::env::temp_dir().join("vcmi-fv-hd-e2e");
    let _ = fs::remove_dir_all(&cache_root);
    fs::create_dir_all(&cache_root).unwrap();

    // 注入 HD 来源（与批量构建前的注入口径一致）
    let src = crate::vcmi::hd_edition::detect(&[user_mods_dir()], None);
    if src.is_none() {
        eprintln!("跳过：未检测到 hd-edition（用 VCMI_USER_MODS_DIR 指向含 hd-edition 的目录）");
        return;
    }
    crate::vcmi::hd_edition::set_source(src);

    // 阶段二构建 core.castle（core 伪模组的 modPath = <game>/Mods/vcmi，同前端 buildCoreEntries）
    let mod_path = game.join("Mods").join("vcmi");
    let r = build_mod_wiki_phase(
        mod_path.to_string_lossy().as_ref(),
        "core.castle",
        &cache_root,
        &game,
        1,
    );
    assert_eq!(
        r.get("ok").and_then(|x| x.as_bool()),
        Some(true),
        "core.castle 阶段二应成功: {r}"
    );

    // 1) 登记表：应含生物/建筑/英雄类（实测至少 creature + building）
    let map = crate::modcache::hd_map::load(&cache_root, "core.castle");
    assert!(!map.is_empty(), "应产出 hd-map.json 登记");
    // 来源指纹标记：用于"装/卸 hd-edition 后自动补登记"（见 `hd_map::source_mark_matches`）
    assert!(
        cache_root.join("core.castle").join(".hd-src").is_file(),
        "应写下 .hd-src 来源指纹"
    );
    assert!(
        crate::modcache::hd_map::source_mark_matches(&cache_root, "core.castle"),
        "写下后指纹应与当前来源一致"
    );
    let has = |prefix: &str| map.keys().any(|k| k.starts_with(prefix));
    println!("hd-map 登记 {} 项，样例: {:?}", map.len(), map.iter().take(5).collect::<Vec<_>>());
    assert!(has("creature/"), "应有生物登记");
    assert!(has("building/") || has("hero/"), "应有建筑或英雄登记");

    // 1b) 共享 UI 图标（SMALRES 资源图标）应登记到 **core**（产物归 core，登记也必须归 core）
    let core_map = crate::modcache::hd_map::load(&cache_root, "core");
    println!(
        "core 登记 {} 项，样例: {:?}",
        core_map.len(),
        core_map.iter().take(4).collect::<Vec<_>>()
    );
    assert!(
        core_map.keys().any(|k| k.starts_with("resources/")),
        "SMALRES 资源图标应登记到 core（实得 {:?}）",
        core_map.keys().take(5).collect::<Vec<_>>()
    );

    // 2) 幕布（**选项 A**）：命中 HD 时应为"HD 底图 + 结构"的合成图（800×374 的 2×/3×）
    let town_path = cache_root
        .join("core.castle")
        .join("images")
        .join("town")
        .join("castle.webp");
    let town = image::load_from_memory(&fs::read(&town_path).expect("幕布应已产出")).unwrap();
    println!("幕布 castle: {}x{}", town.width(), town.height());
    assert!(
        town.width() == 1600 || town.width() == 2400,
        "幕布应为 HD 档合成图（800×374 × 2/3），实得 {}x{}",
        town.width(),
        town.height()
    );

    // 2c) **结构帧绘制偏移**：命中 HD ≠ 能沿用结构自身的 `x/y`。
    //     DEF 帧自带 `lm/tm`，pak 帧另声明 `spriteOffset`，VCMI 的定位公式是
    //     `(defMargins - spriteOffset) * scale`（`HdImageLoader::getImage`）。
    //     回归：2026-09-24 前 HD 路径恒返回 `(0,0)` —— 城堡船坞 `TBCSDKNN`
    //     （DEF `lm=0 tm=53`、pak `spriteOffsetY=2`）因此上移 51px，压在战神像（`z` 更低）身上。
    let (dock_hd, dock_hd_scale, dock_off) =
        crate::vcmi::hd_edition::lookup_with_offset("TBCSDKNN").expect("HD 应收录城堡船坞帧");
    let core_dir = game.join("Mods").join("vcmi");
    let (dock_img, dock_lm, dock_tm) = crate::modcache::preview::load_structure_frame_hd(
        &core_dir,
        &core_dir,
        &game,
        "TBCSDOCK.def",
        dock_hd_scale,
    )
    .expect("城堡船坞应能取到帧");
    println!(
        "船坞帧: {}x{} 绘制偏移 ({dock_lm},{dock_tm})；HD 原始 {}x{}（档 x{dock_hd_scale}）spriteOffset {:?}",
        dock_img.width(),
        dock_img.height(),
        dock_hd.width(),
        dock_hd.height(),
        dock_off
    );
    assert_eq!(
        (dock_lm, dock_tm),
        (-(dock_off.0 as i64), 53 - dock_off.1 as i64),
        "HD 结构帧偏移应为 `DEF lm/tm - pak spriteOffset`（本机 DEF 实测 lm=0/tm=53）"
    );
    assert_ne!(
        (dock_lm, dock_tm),
        (0, 0),
        "偏移恒为 (0,0) = 退回「丢掉 DEF margin」的旧口径"
    );

    // 2b) 兵种背景：`creatureBackground` 是**尺寸分档对象**
    //     （`{120px: TPCASCAS, 130px: CRBKGCAS}`），配置解析取哪一档由 `value_to_resource` 决定
    //     （实测本机取 130px 档 `CRBKGCAS`）——故对候选逐个比对，只断言"产物 = 某个 HD 档的尺寸"。
    let cb_path = cache_root
        .join("core.castle")
        .join("images")
        .join("town")
        .join("castle-creature-bg.webp");
    let got = image::load_from_memory(&fs::read(&cb_path).expect("兵种背景应已产出")).unwrap();
    let mut matched = false;
    for name in ["CRBKGCAS", "TPCASCAS"] {
        if let Some((hd_cb, h_cb)) = crate::vcmi::hd_edition::lookup(name) {
            println!(
                "兵种背景候选 {name}: HD {}x{}（档 x{}）；产物 {}x{}",
                hd_cb.width(),
                hd_cb.height(),
                h_cb,
                got.width(),
                got.height()
            );
            if (got.width(), got.height()) == (hd_cb.width(), hd_cb.height()) {
                matched = true;
            }
        }
    }
    assert!(
        matched,
        "兵种背景尺寸应等于某个 HD 档（证明未回落 1x 放大），实得 {}x{}",
        got.width(),
        got.height()
    );

    // 3) HD 变体生成（rate=4, phys=1.5：1080p 口径）
    let opt = HdOptions::new(Some(4), Some(1.5), None, Some(false)).expect("rate>=2 应有选项");
    let stats = generate_for_mod(&cache_root, "core.castle", &opt);
    println!("HD 变体统计: {:?}", stats);
    assert!(stats.generated > 0, "应生成变体: {stats:?}");

    // 3) **证伪"其实是从 1x 插值"**：变体尺寸与纯 1x 放大可能相同（need=4 时 HD x2×2 恰好也是 4x），
    //    只有埋点能区分取源 —— `hd_src.variant` 必须 > 0。
    let items = crate::infra::metrics::drain_local();
    let hd_variants: u64 = items
        .iter()
        .filter(|(k, _, _, _)| *k == "hd_src.variant")
        .map(|(_, _, c, _)| *c)
        .sum();
    println!("埋点: {}", crate::infra::metrics::format_items(&items, 12));
    assert!(hd_variants > 0, "变体应由 HD 源生成（hd_src.variant 埋点 > 0）");

    // 4) HD 源本体：登记名应可直接取到，且尺寸 = 基准 × 档位（证明"x2/x3 真素材"而非缩放件）
    let (base_key, hd_name) = map
        .iter()
        .find(|(k, _)| k.starts_with("creature/"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .expect("应有 creature 登记");
    let base_img = image::load_from_memory(
        &fs::read(cache_root.join("core.castle").join("images").join(format!("{base_key}.webp")))
            .expect("基准图应存在"),
    )
    .unwrap();
    let (hd_img, h, k) = crate::vcmi::hd_edition::lookup_for(&hd_name, 4).expect("HD 源应可取");
    println!(
        "HD 源 {hd_name}: {}x{}（档 x{h}, 放大 k={k}）；基准 {base_key} = {}x{}",
        hd_img.width(),
        hd_img.height(),
        base_img.width(),
        base_img.height()
    );
    // 允许少量偏差：pak 帧矩形按 DXT 块（4×4）对齐，实测 `CPRS000C` = 62×64 而 1x 是 30×32
    // （严格 2× 应为 60×64）——尺寸不参与坐标计算，只影响图像右/下边缘几个像素。
    let expect_w = base_img.width() * h;
    assert!(
        (hd_img.width() as i64 - expect_w as i64).abs() <= 4,
        "HD 源宽度应 ≈ 基准宽度 × 档位（允许 DXT 对齐偏差 ≤4px），实得 {} vs {}×{}={}",
        hd_img.width(),
        base_img.width(),
        h,
        expect_w
    );

    crate::vcmi::hd_edition::set_source(None);

    // 5) 缓存体积观测（为"HD 变体是否值得 / 是否需禁止二次放大"提供实测数据）
    let images = cache_root.join("core.castle").join("images");
    let (mut base_b, mut var_b) = (0u64, 0u64);
    if let Ok(rd) = fs::read_dir(&images) {
        for sub in rd.flatten() {
            if let Ok(rd2) = fs::read_dir(sub.path()) {
                for f in rd2.flatten() {
                    let n = f.metadata().map(|m| m.len()).unwrap_or(0);
                    if f.file_name().to_string_lossy().contains('@') {
                        var_b += n;
                    } else {
                        base_b += n;
                    }
                }
            }
        }
    }
    println!(
        "core.castle 缓存体积: 基准图 {:.2} MB + @Nx 变体 {:.2} MB = {:.2} MB",
        base_b as f64 / 1048576.0,
        var_b as f64 / 1048576.0,
        (base_b + var_b) as f64 / 1048576.0
    );

    let _ = fs::remove_dir_all(&cache_root);
}

/// 规模观测：构建 `core` + 9 个 `core.<faction>`（**原版全量**），统计 HD 接入后的
/// 城镇类体积、`@Nx` 变体体积与耗时量级。
///
/// 用途：为"HD 幕布 2400×1122 在 4K 下是否还要二次放大"之类的取舍提供实测数据；
/// 用临时缓存目录，不污染真实缓存。
///
/// 运行：`cargo test --lib core_family_hd_volume_probe -- --ignored --nocapture`
#[test]
#[ignore]
fn core_family_hd_volume_probe() {
    let _g = crate::vcmi::hd_edition::test_guard();
    let game = game_dir();
    let cache_root = std::env::temp_dir().join("vcmi-fv-hd-probe");
    let _ = fs::remove_dir_all(&cache_root);
    fs::create_dir_all(&cache_root).unwrap();
    let src = crate::vcmi::hd_edition::detect(&[user_mods_dir()], None);
    if src.is_none() {
        eprintln!("跳过：未检测到 hd-edition（用 VCMI_USER_MODS_DIR 指向含 hd-edition 的目录）");
        return;
    }
    println!(
        "HD 来源：{:?}（档位 {:?}）",
        src.as_ref().map(|s| s.dir().display().to_string()),
        src.as_ref().map(|s| s.scales())
    );
    crate::vcmi::hd_edition::set_source(src);

    let mod_path = game.join("Mods").join("vcmi");
    let mods: Vec<String> = std::iter::once("core".to_string())
        .chain(crate::vcmi::h3core::CORE_FACTIONS.iter().map(|f| format!("core.{f}")))
        .collect();
    let opt = HdOptions::new(Some(4), Some(1.5), None, Some(false)).expect("rate>=2 应有选项");

    let t0 = std::time::Instant::now();
    let (mut town_b, mut town_n) = (0u64, 0usize);
    let (mut var_b, mut total_b) = (0u64, 0u64);
    for m in &mods {
        let r = build_mod_wiki_phase(mod_path.to_string_lossy().as_ref(), m, &cache_root, &game, 1);
        assert_eq!(
            r.get("ok").and_then(|x| x.as_bool()),
            Some(true),
            "{m} 阶段二应成功: {r}"
        );
        let stats = generate_for_mod(&cache_root, m, &opt);
        println!("  {m}: 变体 {:?}", stats);
        let dir = cache_root.join(crate::modcache::sanitize_id(m)).join("images");
        walk_images(&dir, &mut |path, len| {
            total_b += len;
            let s = path.replace('\\', "/");
            if s.contains("/town/") {
                town_b += len;
                town_n += 1;
            }
            if s.contains('@') {
                var_b += len;
            }
        });
    }
    let secs = t0.elapsed().as_secs_f64();
    println!(
        "core 家族（1+9）HD 构建耗时 {:.1}s | 城镇类（幕布/兵种背景）{town_n} 个文件 {:.2} MB | @Nx 变体 {:.2} MB | 缓存合计 {:.2} MB",
        secs,
        town_b as f64 / 1048576.0,
        var_b as f64 / 1048576.0,
        total_b as f64 / 1048576.0
    );
    crate::vcmi::hd_edition::set_source(None);
    let _ = fs::remove_dir_all(&cache_root);
}

/// 递归统计 `images/**`（测试内辅助；回调参数 = 路径字符串与字节数）
fn walk_images(dir: &Path, f: &mut impl FnMut(String, u64)) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk_images(&p, f);
        } else {
            let len = e.metadata().map(|m| m.len()).unwrap_or(0);
            f(p.to_string_lossy().to_string(), len);
        }
    }
}
