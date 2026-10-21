//! HD 素材（`hd-edition` 的 `.pak`）端到端冒烟：**素材登记 → HD 变体生成**。
//!
//! 依赖真实游戏目录（`Data/*.lod`）、用户模组目录（含 `hd-edition`），故 `#[ignore]`。
//! 运行：`cargo test --lib modcache::smoke_tests::hd_edition -- --ignored --nocapture`

use super::*;
use crate::modcache::hd::{generate_for_mod, HdOptions};

// 游戏目录 / 用户模组目录统一走 `mod.rs` 的 `game_dir()` / `user_mods_dir()`（未设置环境变量时
// 回落仓库内 `references/Heroes3-VCMI` 验证 fixture，跨机器一致）。

/// **高清素材导出**：把 `hd-edition` 各档收录的素材导出到 `references/h3-hd-assets`。
///
/// 产物（供开发者核对"HD 到底有哪些可用素材"）：
/// - `index.csv`：`档位,条目,帧名` 全量清单（每档约 2.4 万行）；
/// - `x<N>/<pak类>/<条目>.webp`：**每个条目的首帧**可视样本（每档 2731 张）。
///   不导出全部帧的原因：两档合计约 4.9 万帧，全量导出会到 GB 级；核对"有哪些素材"
///   用条目首帧即可，具体帧名查 `index.csv`。
///
/// 运行：`VCMI_USER_MODS_DIR=<含 hd-edition 的目录> cargo test --lib modcache::smoke_tests::hd_edition::hd_assets_dump -- --ignored --nocapture`
#[test]
#[ignore]
fn hd_assets_dump() {
    use std::collections::BTreeMap;
    use std::io::Write;

    let _g = crate::vcmi::hd_edition::test_guard();
    // CARGO_MANIFEST_DIR = <repo>/src-tauri → 退一级即仓库根
    let out_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("references")
        .join("h3-hd-assets");
    fs::create_dir_all(&out_root).unwrap();
    // 探针：先确认目标目录可写（避免"跑了半天、一个文件都没落盘"）
    match fs::write(out_root.join(".probe"), b"x") {
        Ok(()) => println!("探针写入 OK：{}", out_root.canonicalize().unwrap_or_default().display()),
        Err(e) => println!("探针写入失败：{e}（目标 {}）", out_root.display()),
    }
    let mut csv = fs::File::create(out_root.join("index.csv")).expect("清单文件");
    writeln!(csv, "scale,pak,entry,frame").unwrap();

    for forced in [3u32, 2u32] {
        let src = crate::vcmi::hd_edition::detect(&[user_mods_dir()], Some(forced));
        let Some(src) = src else {
            println!("x{forced}: 未检测到 hd-edition");
            continue;
        };
        crate::vcmi::hd_edition::set_source(Some(src));
        let src = crate::vcmi::hd_edition::source().expect("已注入");

        // (条目 → 帧名列表)；pak 类用条目首个字母粗分（pak 本身不暴露文件名，按用途归类）
        let mut by_entry: BTreeMap<String, (String, Vec<String>)> = BTreeMap::new();
        for (_scale, pak_name, entry, frame) in src.dump_frames() {
            by_entry
                .entry(entry.clone())
                .or_insert_with(|| (pak_name.clone(), Vec::new()))
                .1
                .push(frame.clone());
            writeln!(csv, "x{forced},{pak_name},{entry},{frame}").unwrap();
        }

        let scale_dir = out_root.join(format!("x{forced}"));
        let mut written = 0usize;
        for (entry, (pak_name, frames)) in &by_entry {
            frames.iter().find_map(|f| src.lookup(f)).map(|(img, _)| {
                // 目录用真实 pak 名（`sprite_...` / `bitmap_...`），不用条目名猜
                let dir = scale_dir.join(pak_name);
                fs::create_dir_all(&dir).unwrap();
                if let Some(bytes) =
                    crate::infra::image::encode_webp_lossy(img.as_raw(), img.width(), img.height())
                {
                    match fs::write(dir.join(format!("{entry}.webp")), bytes) {
                        Ok(()) => written += 1,
                        Err(e) if written == 0 => println!("首个写入失败：{e}"),
                        Err(_) => {}
                    }
                }
            });
        }
        println!("x{forced}: 条目 {}，导出首帧样本 {written} → {}", by_entry.len(), scale_dir.display());
    }
    println!("清单 {}", out_root.join("index.csv").display());
}

/// **全量帧导出**：把每个条目的**每一帧**都导出成 webp（GB 级，仅供人工核对）。
///
/// 产物：`x<N>/<pak>/<条目>/<帧名>.webp`。跳过已存在的文件，可中断后续跑。
///
/// 运行：`VCMI_USER_MODS_DIR=<含 hd-edition 的目录> cargo test --lib modcache::smoke_tests::hd_edition::hd_assets_dump_all -- --ignored --nocapture`
#[test]
#[ignore]
fn hd_assets_dump_all() {
    let _g = crate::vcmi::hd_edition::test_guard();
    let out_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("references")
        .join("h3-hd-assets");
    fs::create_dir_all(&out_root).unwrap();

    for forced in [3u32, 2u32] {
        let Some(src) = crate::vcmi::hd_edition::detect(&[user_mods_dir()], Some(forced)) else {
            println!("x{forced}: 未检测到 hd-edition");
            continue;
        };
        crate::vcmi::hd_edition::set_source(Some(src));
        let src = crate::vcmi::hd_edition::source().expect("已注入");

        let mut n = 0usize;
        let mut entries = 0usize;
        let mut cur_dir: Option<std::path::PathBuf> = None;
        // dump_frames 已按 (档位, pak, 条目) 有序产出 → 目录只需在条目变化时建一次
        for (_scale, pak_name, entry, frame) in src.dump_frames() {
            let need_dir = match &cur_dir {
                Some(d) => !d.ends_with(&entry),
                None => true,
            };
            if need_dir {
                let d = out_root.join(format!("x{forced}")).join(&pak_name).join(&entry);
                fs::create_dir_all(&d).unwrap();
                entries += 1;
                cur_dir = Some(d);
            }
            let dir = cur_dir.as_ref().unwrap();
            let path = dir.join(format!("{frame}.webp"));
            if path.is_file() {
                n += 1;
                continue;
            }
            if let Some((img, _)) = src.lookup(&frame) {
                if let Some(bytes) =
                    crate::infra::image::encode_webp_lossy(img.as_raw(), img.width(), img.height())
                {
                    let _ = fs::write(&path, bytes);
                    n += 1;
                }
            }
        }
        println!("x{forced}: 条目 {entries}，帧 {n}");
    }
    println!("全量导出完成：{}", out_root.display());
}

/// 真机冒烟（2026-10-22）：**法术图标的 HD 兄弟帧**。
///
/// HD 版没有 `SPELLBON` 条目，但同一套法术图标以卷轴版收录在 `SPELLSCR`
/// （帧名 `SB01SumB` → `SM01SUMB`，前缀 SB→SM、其余一致）。验证映射与 pak 命中。
///
/// 运行：`VCMI_USER_MODS_DIR=<含 hd-edition 的目录> cargo test --lib modcache::smoke_tests::hd_edition::spellbon_hd_sibling_smoke -- --ignored --nocapture`
#[test]
#[ignore]
fn spellbon_hd_sibling_smoke() {
    let _g = crate::vcmi::hd_edition::test_guard();
    let Some(src) = crate::vcmi::hd_edition::detect(&[user_mods_dir()], None) else {
        println!("跳过：未检测到 hd-edition");
        return;
    };
    crate::vcmi::hd_edition::set_source(Some(src));
    let game = game_dir();
    let bytes = crate::vcmi::resource::read_game_resource_fallback(&game, "SPELLBON.DEF")
        .expect("应能读到 SPELLBON.DEF");
    let def = crate::infra::def::DefReader::new(&bytes);
    let raw = def.frame_name(0, 0).expect("帧名");
    let stem = raw.trim().rsplit_once('.').map_or(raw.trim(), |(s, _)| s);
    let sibling = format!("SM{}", stem[2..].to_ascii_uppercase());
    let hit = crate::vcmi::hd_edition::lookup(&sibling);
    println!("SPELLBON g0f0 = {raw} → 兄弟帧 {sibling} → {}", if hit.is_some() { "命中" } else { "未命中" });
    let img = hit.expect("SPELLSCR 应有同图案高清帧");
    println!("高清尺寸 {}x{}（1x 约 {}x{}）", img.0.width(), img.0.height(), img.0.width() / img.1, img.0.height() / img.1);
    assert!(img.0.width() > 100, "应为真高清帧");
}

/// 真机冒烟（2026-10-22）：**别名帧**（DEF 帧名与 pak 命名不同体系）。
///
/// - 法术图标：`SB01SUMB` → `SPELLSCR` 的 `SM01SUMB`；
/// - 战争机器图标：`CPRSMALL` 的 147..150 帧 → pak 同条目的 **118..121**
///   （`CPRS118M`..`CPRS121M`，HD 版只保留这 4 个 `M` 帧；轮廓 IoU 已验证顺序一致）。
///
/// 运行：`VCMI_USER_MODS_DIR=<含 hd-edition 的目录> cargo test --lib modcache::smoke_tests::hd_edition::hd_alias_frames_smoke -- --ignored --nocapture`
#[test]
#[ignore]
fn hd_alias_frames_smoke() {
    let _g = crate::vcmi::hd_edition::test_guard();
    let Some(src) = crate::vcmi::hd_edition::detect(&[user_mods_dir()], None) else {
        println!("跳过：未检测到 hd-edition");
        return;
    };
    crate::vcmi::hd_edition::set_source(Some(src));
    for (label, name, min_w) in [
        ("法术图标 SM 兄弟帧", "SM01SUMB", 100u32),
        ("投石车图标 CPRS118M", "CPRS118M", 80),
        ("弩车图标 CPRS119M", "CPRS119M", 80),
        ("帐篷图标 CPRS120M", "CPRS120M", 60),
        ("补给车图标 CPRS121M", "CPRS121M", 60),
    ] {
        match crate::vcmi::hd_edition::lookup(name) {
            Some((img, scale)) => {
                println!("{label}: {name} 命中 x{scale} {}x{}", img.width(), img.height());
                assert!(
                    img.width() >= min_w,
                    "{name} 应为高清帧（实测宽 {}）",
                    img.width()
                );
            }
            None => panic!("{label}: {name} 未命中"),
        }
    }
}

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
    println!(
        "hd-map 登记 {} 项，样例: {:?}",
        map.len(),
        map.iter().take(5).collect::<Vec<_>>()
    );
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
    assert!(
        hd_variants > 0,
        "变体应由 HD 源生成（hd_src.variant 埋点 > 0）"
    );

    // 4) HD 源本体：登记名应可直接取到，且尺寸 = 基准 × 档位（证明"x2/x3 真素材"而非缩放件）
    let (base_key, hd_name) = map
        .iter()
        .find(|(k, _)| k.starts_with("creature/"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .expect("应有 creature 登记");
    let base_img = image::load_from_memory(
        &fs::read(
            cache_root
                .join("core.castle")
                .join("images")
                .join(format!("{base_key}.webp")),
        )
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
        .chain(
            crate::vcmi::h3core::CORE_FACTIONS
                .iter()
                .map(|f| format!("core.{f}")),
        )
        .collect();
    let opt = HdOptions::new(Some(4), Some(1.5), None, Some(false)).expect("rate>=2 应有选项");

    let t0 = std::time::Instant::now();
    let (mut town_b, mut town_n) = (0u64, 0usize);
    let (mut var_b, mut total_b) = (0u64, 0u64);
    for m in &mods {
        let r = build_mod_wiki_phase(
            mod_path.to_string_lossy().as_ref(),
            m,
            &cache_root,
            &game,
            1,
        );
        assert_eq!(
            r.get("ok").and_then(|x| x.as_bool()),
            Some(true),
            "{m} 阶段二应成功: {r}"
        );
        let stats = generate_for_mod(&cache_root, m, &opt);
        println!("  {m}: 变体 {:?}", stats);
        let dir = cache_root
            .join(crate::modcache::sanitize_id(m))
            .join("images");
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

/// 定位冒烟（2026-10-20）：CPRSMALL 的 DEF 帧 probe 为何 miss（位图路径 HRART 却 127 条命中）。
/// 打印读到的字节数 / frame_name / probe 结果做对照。运行：
/// `VCMI_GAME_DIR=<游戏目录> cargo test --lib modcache::smoke_tests::hd_edition::debug_probe_def_frame_cprsmall -- --ignored --nocapture`
#[test]
#[ignore]
fn debug_probe_def_frame_cprsmall() {
    let _g = crate::vcmi::hd_edition::test_guard();
    let game = game_dir();
    // 与构建批次同口径：auto 探测（game/Mods 兜底）
    crate::vcmi::hd_edition::setup_from_config(None, &game);

    println!("== 位图路径对照（已知 127 条命中）");
    println!("probe(HRART098) = {:?}", crate::vcmi::hd_edition::probe("HRART098"));

    println!("== CPRSMALL 路径（实测仅 2 条命中）");
    let bytes = crate::vcmi::resource::read_game_resource_fallback(&game, "CPRSMALL.DEF")
        .expect("应读到 CPRSMALL.DEF");
    println!("CPRSMALL.DEF bytes = {}（LOD 原版为 153231）", bytes.len());
    let def = crate::infra::def::DefReader::new(&bytes);
    for idx in [2usize, 100, 118] {
        println!(
            "  frame_name(0,{idx}) = {:?}  probe_def_frame = {:?}",
            def.frame_name(0, idx),
            crate::vcmi::hd_edition::probe_def_frame(&bytes, 0, idx)
        );
    }

    println!("== ARTIFACT 对照（同走 probe_def_frame，127 条命中）");
    let ab = crate::vcmi::resource::read_game_resource_fallback(&game, "ARTIFACT.DEF")
        .expect("应读到 ARTIFACT.DEF");
    println!("ARTIFACT.DEF bytes = {}（LOD 原版为 191046）", ab.len());
    println!(
        "  probe_def_frame(0,103) = {:?}",
        crate::vcmi::hd_edition::probe_def_frame(&ab, 0, 103)
    );

    println!("== resolve 侧：无扩展名引用 CPRSMALL 会命中什么");
    let core = game.join("Mods").join("vcmi");
    println!(
        "  resolve_resource(core, \"CPRSMALL\") = {:?}",
        crate::vcmi::resource::resolve_resource(&core, "CPRSMALL").map(|(b, e)| (b.len(), e))
    );
}

/// 定位冒烟（2026-10-20）：复刻 core.castle 阶段二，看 hd-map 登记实际落盘内容。
#[test]
#[ignore]
fn debug_rebuild_core_castle_registration() {
    let _g = crate::vcmi::hd_edition::test_guard();
    let game = game_dir();
    crate::vcmi::hd_edition::setup_from_config(None, &game);
    let cache_root = std::env::temp_dir().join("vcmi-fv-hd-debug-castle");
    let _ = fs::remove_dir_all(&cache_root);
    fs::create_dir_all(&cache_root).unwrap();
    let mod_path = game.join("Mods").join("vcmi");
    let r = build_mod_wiki_phase(
        mod_path.to_string_lossy().as_ref(),
        "core.castle",
        &cache_root,
        &game,
        1,
    );
    println!("build ok={:?}", r.get("ok"));
    for owner in ["core", "core.castle"] {
        let map = crate::modcache::hd_map::load(&cache_root, owner);
        let by_sub: std::collections::BTreeMap<String, usize> = {
            let mut m: std::collections::BTreeMap<String, usize> = Default::default();
            for k in map.keys() {
                *m.entry(k.split('/').next().unwrap_or("").to_string()).or_default() += 1;
            }
            m
        };
        println!("{owner}/hd-map.json 共 {} 条，按 sub: {:?}", map.len(), by_sub);
        let creature: Vec<_> = map.iter().filter(|(k, _)| k.starts_with("creature/")).take(6).collect();
        println!("  creature 样例: {:?}", creature);
    }
    let dir = cache_root.join("core").join("images").join("creature");
    let n = fs::read_dir(&dir).map(|d| d.flatten().filter(|e| {
        e.file_name().to_string_lossy().contains("g0f2") || e.file_name().to_string_lossy().contains("g0f10")
    }).count()).unwrap_or(0);
    println!("core/images/creature 中 g0f2/g0f10 相关文件数: {n}");

    // 3) HD 变体验证：generate_for_mod 应按 hd-map 登记取 HD 真源（而非 1x 插值）
    let opt = HdOptions {
        rate: 4,
        phys: 1.5,
        upscaler: "xbrz".into(),
        ai: false,
        ai_model: None,
        usm: 0.0,
        ai_mix: 0.75,
    };
    let st = generate_for_mod(&cache_root, "core", &opt);
    println!("generate_for_mod(core): generated={} skipped={} failed={}", st.generated, st.skipped, st.failed);
    for name in ["cprsmall_def~g0f2", "artifact_def~g0f103"] {
        for sub in ["creature", "artifact"] {
            let base = cache_root.join("core").join("images").join(sub).join(format!("{name}.webp"));
            let variant = cache_root.join("core").join("images").join(sub).join(format!("{name}@4x.webp"));
            let dim = |p: &std::path::Path| -> Option<(u32, u32)> {
                let b = fs::read(p).ok()?;
                image::load_from_memory(&b).ok().map(|i| (i.width(), i.height()))
            };
            let v = if variant.is_file() { Some(dim(&variant)) } else { None };
            println!("  {sub}/{name}: 1x={:?} @4x={:?}", dim(&base), v);
        }
    }

    // 4) 城镇描边：登记 HD 后按用途目标框生成变体（观察倍率与体积）
    let st2 = generate_for_mod(&cache_root, "core.castle", &opt);
    println!("generate_for_mod(core.castle): generated={} skipped={} failed={}", st2.generated, st2.skipped, st2.failed);
    let mut base_b = 0u64;
    let mut var_b = 0u64;
    let mut var_n = 0usize;
    let mut sample = String::new();
    if let Ok(rd) = fs::read_dir(cache_root.join("core.castle").join("images").join("townborder")) {
        for f in rd.flatten() {
            let n = f.metadata().map(|m| m.len()).unwrap_or(0);
            let name = f.file_name().to_string_lossy().to_string();
            if name.contains('@') {
                var_b += n;
                var_n += 1;
                if sample.is_empty() {
                    sample = name;
                }
            } else {
                base_b += n;
            }
        }
    }
    print!("townborder 变体 {var_n} 个 共 {} KB（基准 {} KB）样例 {sample}", var_b / 1024, base_b / 1024);
    if !sample.is_empty() {
        let p = cache_root.join("core.castle").join("images").join("townborder").join(&sample);
        if let Ok(b) = fs::read(&p) {
            if let Ok(i) = image::load_from_memory(&b) {
                println!(" → {}x{}", i.width(), i.height());
            }
        }
    } else {
        println!();
    }
}

/// 共享 UI 图标的**官方 HD 覆盖度**诊断（2026-10-20 两次复核成立，留档避免重复排查）。
///
/// 结论（本机 hd-edition x2/x3 pak，全量 48824 帧名）：
/// - `PSKIL42` 帧 0/1/2/5 → `PSKIL42A/D/P/S` **有**（曾因目标框错写 28 而不用，已修）；
/// - 地形 tiles 首帧 **有**（`DIRTTL`→`TDTS000`、`ROCKTL`→`ROCKTL01`；曾因 DEF 首帧路径
///   漏登记而白丢）；水域 `WATRTL` 帧名是动画根，HD 存子帧 `WATRTL01_0`，靠回退命中；
/// - 建筑图标 `HALL*` **8 族全有**（`THCSMAG1`/`TPRMAG1`/`TPDMAGE1`…），仅元素城
///   `HALLELEM` 无；`TOCS*`（城镇描边）只有 Castle；
/// - `SPELLBON` 全部帧（`SB01SUMB` 等）**本身 miss**，但同图案的 `SPELLINT`(`SI..`, 144×108)
///   与 `SPELLS`(`SP..`, 195×162) **有** —— 69/70 帧可按 `SB→SI/SP` 映射（未接入，画幅不同）；
/// - `CPRSMALL` 战争机器帧 147..150（`CPRS145M`…`CPRS148M`）**miss** —— 生物帧 2/99 有。
///
/// 运行：`VCMI_GAME_DIR=<游戏目录> cargo test --lib modcache::smoke_tests::hd_edition::debug_probe_shared_icon_hd_coverage -- --ignored --nocapture`
#[test]
#[ignore]
fn debug_probe_shared_icon_hd_coverage() {
    let _g = crate::vcmi::hd_edition::test_guard();
    let game = game_dir();
    crate::vcmi::hd_edition::setup_from_config(None, &game);
    let probe_def = |name: &str, frames: &[usize]| {
        let Some(bytes) = crate::vcmi::resource::read_game_resource_fallback(&game, name) else {
            println!("  {name}: 读不到");
            return;
        };
        let def = crate::infra::def::DefReader::new(&bytes);
        println!("  {name}: {} bytes", bytes.len());
        for &i in frames {
            println!(
                "    f{i}: frame_name={:?} hd={:?}",
                def.frame_name(0, i),
                crate::vcmi::hd_edition::probe_def_frame(&bytes, 0, i)
            );
        }
    };
    println!("== SPELLBON（法术小图标，预期全 miss）");
    probe_def("SPELLBON.DEF", &[0, 15, 60]);
    println!("== CPRSMALL 战争机器帧 147..150（预期 miss；2/99 对照命中）");
    probe_def("CPRSMALL.DEF", &[2, 99, 147, 148, 149, 150]);
    println!("== PSKIL42 四主属性帧（预期命中）");
    probe_def("PSKIL42.DEF", &[0, 1, 2, 5]);
    println!("== 地形 tiles DEF 首帧（预期命中）");
    for t in ["DIRTTL", "ROCKTL"] {
        probe_def(&format!("{t}.DEF"), &[0]);
    }
    println!("== SPELLBON 帧名 → SPELLINT(SI)/SPELLS(SP) 兄弟命中（法术图标 HD 候选）");
    if let Some(bytes) = crate::vcmi::resource::read_game_resource_fallback(&game, "SPELLBON.DEF")
    {
        let d = crate::infra::def::DefReader::new(&bytes);
        let mut si = 0;
        let mut sp = 0;
        for i in 0..70 {
            let Some(n) = d.frame_name(0, i) else { break };
            let rest = n.strip_prefix("SB").unwrap_or(&n);
            let a = crate::vcmi::hd_edition::probe(&format!("SI{rest}"));
            let b = crate::vcmi::hd_edition::probe(&format!("SP{rest}"));
            if a.is_some() {
                si += 1;
            }
            if b.is_some() {
                sp += 1;
            }
            if i < 4 || i == 69 {
                println!("  f{i} {n} -> SI{rest}={a:?} SP{rest}={b:?}");
            }
        }
        println!("  汇总：SI 命中 {si}/70，SP 命中 {sp}/70");
    }
    println!("== 各族建筑图标 DEF 帧命名（核对文档「仅 Castle 有 HD」）");
    for f in [
        "HALLCSTL", "HALLRAMP", "HALLTOWR", "HALLINFR", "HALLNECR", "HALLDUNG", "HALLSTRN",
        "HALLFORT", "HALLELEM",
    ] {
        probe_def(&format!("{f}.DEF"), &[0, 5]);
    }
    println!("== 城镇描边（townborder）：原版 BMP 与 HD 帧的宽高比对照");
    for n in ["TOCSMAG1", "TOCSTAV1", "TOCSDKMS", "TOCSC101"] {
        let core = game.join("Mods").join("vcmi");
        let bmp = crate::vcmi::resource::resolve_resource_fallback(
            &core,
            &game,
            &format!("{n}.bmp"),
        )
        .and_then(|(b, _e)| {
            image::load_from_memory(&b)
                .ok()
                .map(|i| (i.width(), i.height()))
        });
        let hd = crate::vcmi::hd_edition::lookup(n).map(|(i, s)| (i.width(), i.height(), s));
        println!("  {n}: bmp={bmp:?} hd={hd:?}");
    }
    println!("== townborder：产物尺寸 vs HD 帧 ÷ 档位（判定能否直接接 HD）");
    {
        let cache_root = std::env::temp_dir().join("vcmi-fv-hd-border-probe");
        let _ = fs::remove_dir_all(&cache_root);
        fs::create_dir_all(&cache_root).unwrap();
        crate::modcache::set_assets_enabled(true);
        let core = game.join("Mods").join("vcmi");
        let mut same = 0;
        let mut diff = 0;
        for cfg in crate::vcmi::town::read_game_root_factions(&game) {
            if cfg.faction_id != "castle" {
                continue;
            }
            for (sid, v) in &cfg.structures {
                let Some(res) = v.get("border").and_then(|x| x.as_str()) else {
                    continue;
                };
                let Some((_rel, w, h)) = crate::modcache::assets::extract_border_and_save(
                    &core,
                    &game,
                    &cache_root,
                    "core.castle",
                    &cfg.faction_id,
                    sid,
                    res,
                ) else {
                    continue;
                };
                let hd = crate::vcmi::hd_edition::lookup(res);
                let matched = hd.as_ref().map(|(i, s)| {
                    let (ew, eh) = (i.width() as f64 / *s as f64, i.height() as f64 / *s as f64);
                    (ew - w as f64).abs() <= 2.0 && (eh - h as f64).abs() <= 2.0
                });
                match matched {
                    Some(true) => same += 1,
                    Some(false) => diff += 1,
                    None => {}
                }
                println!(
                    "  {sid:18} {res:14} out={w}x{h} hd={:?} 1x_match={matched:?}",
                    hd.as_ref().map(|(i, s)| (i.width(), i.height(), *s))
                );
            }
        }
        println!("  汇总：尺寸一致 {same}，不一致 {diff}");
        let _ = fs::remove_dir_all(&cache_root);
    }
    println!("== 水域 tiles（WATRTL 帧名带子帧后缀，疑似 miss）");
    probe_def("WATRTL.DEF", &[0, 1]);
    for n in ["WATRTL01", "WATRTL01_0", "WATRTL00", "WATRTL00_0"] {
        println!("  probe({n}) = {:?}", crate::vcmi::hd_edition::probe(n));
    }
}

/// 回归（2026-10-20）：**地形图标必须登记官方 HD 源**。
///
/// 两个断点先后被修掉：
/// 1. 地形走 `extract_icon_and_save`（DEF 首帧分支），该分支漏了 `probe_def_frame`
///    → 即使 HD 素材里有 `TDTS000`，`hd-map.json` 也永远没有 `terrain/...` 条目；
/// 2. 水域 tiles 是**动画化 DEF**（帧名 `WATRTL01`），HD 按子帧存（`WATRTL01_0`）→
///    只查主名必 miss，故 `probe_def_frame` 加了首子帧回退。
#[test]
#[ignore]
fn terrain_registers_hd_source() {
    let _g = crate::vcmi::hd_edition::test_guard();
    let game = game_dir();
    crate::vcmi::hd_edition::setup_from_config(None, &game);
    let core = game.join("Mods").join("vcmi");
    // (地形 id, tiles DEF, 期望登记到的 HD 键)
    let cases = [
        ("dirt", "DIRTTL.DEF", "TDTS000"),
        ("water", "WATRTL.DEF", "WATRTL01_0"),
    ];
    for (id, def, want) in cases {
        if crate::vcmi::hd_edition::probe(want).is_none() {
            println!("跳过 {id}：本机 HD 素材未收录 {want}");
            continue;
        }
        let cache_root = std::env::temp_dir().join(format!("vcmi-fv-hd-terrain-{id}"));
        let _ = fs::remove_dir_all(&cache_root);
        fs::create_dir_all(&cache_root).unwrap();
        crate::modcache::set_assets_enabled(true);
        crate::modcache::hd_map::reset();
        let rel = crate::modcache::assets::extract_icon_and_save(
            &core, &game, &cache_root, "core", "terrain", id,
            Some(def),
        )
        .unwrap_or_else(|| panic!("{id} 应提取出地形图标"));
        crate::modcache::hd_map::flush(&cache_root);
        let map = crate::modcache::hd_map::load(&cache_root, "core");
        let key = rel
            .trim_start_matches("core/images/terrain/")
            .trim_end_matches(".webp");
        assert_eq!(
            map.get(&format!("terrain/{key}")).map(String::as_str),
            Some(want),
            "{id} 地形必须登记 HD 源（键与产物 stem 同口径）"
        );
        let _ = fs::remove_dir_all(&cache_root);
    }
}

/// 全量导出 HD 素材的帧名清单（覆盖度审计用）：`<temp>/hd-index.tsv`，列为 `档位\t条目\t帧名`。
///
/// 运行：`VCMI_GAME_DIR=<游戏目录> cargo test --lib modcache::smoke_tests::hd_edition::dump_hd_index -- --ignored --nocapture`
#[test]
#[ignore]
fn dump_hd_index() {
    let _g = crate::vcmi::hd_edition::test_guard();
    let game = game_dir();
    crate::vcmi::hd_edition::setup_from_config(None, &game);
    let Some(src) = crate::vcmi::hd_edition::source() else {
        println!("未探测到 HD 来源");
        return;
    };
    let frames = src.dump_frames();
    let mut lines = String::new();
    for (scale, _pak, entry, name) in &frames {
        lines.push_str(&format!("{scale}\t{entry}\t{name}\n"));
    }
    let out = std::env::temp_dir().join("hd-index.tsv");
    fs::write(&out, &lines).unwrap();
    println!("写出 {} 行 → {}", frames.len(), out.display());
}

/// 回归（2026-10-20）：**4x 变体必须取 x3 源再重采样到 4x 精确尺寸**。
///
/// 旧规则在 `need=4` 时选 x2 档再放大 2×（`h*k` 恰好 4，体积最小），但 x2 原图细节不足，
/// 2× 插值后明显糊（用户实测）。新规则：`need > 最高档` → 取 x3，再单步 Lanczos3
/// 重采样到 `4 × 1x`。断言用**尺寸**区分两条路径：x2×2 得 124×128（x2 帧 62px 宽），
/// x3→4x 得 120×128（4×30）。
#[test]
#[ignore]
fn four_x_variant_resamples_top_scale() {
    use crate::modcache::hd::{generate_for_mod, HdOptions};
    let _g = crate::vcmi::hd_edition::test_guard();
    let game = game_dir();
    crate::vcmi::hd_edition::setup_from_config(None, &game);
    if crate::vcmi::hd_edition::probe("CPRS000C").is_none() {
        println!("跳过：本机 HD 素材未收录 CPRS000C");
        return;
    }
    let cache_root = std::env::temp_dir().join("vcmi-fv-hd-fourx");
    let _ = fs::remove_dir_all(&cache_root);
    fs::create_dir_all(&cache_root).unwrap();
    crate::modcache::set_assets_enabled(true);
    crate::modcache::hd_map::reset();
    crate::modcache::assets::extract_game_def_frame_and_save(
        &game, &cache_root, "core", "CPRSMALL", 0, 2, "creature", "c000",
    )
    .expect("应提取 CPRS000C");
    crate::modcache::hd_map::flush(&cache_root);
    let opt = HdOptions {
        rate: 4,
        phys: 1.5,
        upscaler: "xbrz".into(),
        ai: false,
        ai_model: None,
        usm: 0.0,
        ai_mix: 0.75,
    };
    generate_for_mod(&cache_root, "core", &opt);
    // 关键行为：need=4 必须取 **x3** 源（旧规则取 x2 再 2× 放大 → 细节不足）
    let (src_img, src_scale, _) =
        crate::vcmi::hd_edition::lookup_for("CPRS000C", 4).expect("HD 源应可取");
    assert_eq!(src_scale, 3, "4x 需求必须取最高档 x3 源");
    println!("源 {src_scale} 档 {}x{}", src_img.width(), src_img.height());
    let dir = cache_root.join("core").join("images").join("creature");
    let dim = |p: &std::path::Path| -> Option<(u32, u32)> {
        image::load_from_memory(&fs::read(p).ok()?)
            .ok()
            .map(|i| (i.width(), i.height()))
    };
    let base = dim(&dir.join("cprsmall_def~g0f2.webp")).expect("1x 基准应存在");
    let v4 = dim(&dir.join("cprsmall_def~g0f2@4x.webp")).expect("应生成 @4x");
    println!("1x={base:?} @4x={v4:?}");
    // 目标=4×基准；实测允许 pak 帧矩形 DXT(4×4) 对齐带来的几像素偏差
    let (ew, eh) = (base.0 * 4, base.1 * 4);
    assert!(
        (v4.0 as i64 - ew as i64).abs() <= 4 && (v4.1 as i64 - eh as i64).abs() <= 4,
        "4x 变体尺寸应 ≈ 4× 基准 {ew}×{eh}（允许 DXT 对齐偏差 ≤4px），实得 {v4:?}"
    );
    let _ = fs::remove_dir_all(&cache_root);
}

/// 回归（2026-10-20）：`pskil42` 的 `@2x` 必须**取官方 HD 源**，而不是 1x 放大。
///
/// 两个前置条件缺一不可：①`hd_target_box("pskil42")` = 32（UI 是 `h-8 w-8`），
/// 使 1080p 下 `target_rate` = 2；②素材阶段登记了 `PSKIL42A`。此前 ① 错写 28 →
/// 需求 42px 恰等于源帧 42×42 → 连变体都不生成（用户报「pskil42 没用高清素材」）。
#[test]
#[ignore]
fn pskil42_variant_uses_hd_source() {
    use crate::modcache::hd::{generate_for_mod, HdOptions};
    let _g = crate::vcmi::hd_edition::test_guard();
    let game = game_dir();
    crate::vcmi::hd_edition::setup_from_config(None, &game);
    if crate::vcmi::hd_edition::probe("PSKIL42A").is_none() {
        println!("跳过：本机 HD 素材未收录 PSKIL42A");
        return;
    }
    let cache_root = std::env::temp_dir().join("vcmi-fv-hd-pskil42");
    let _ = fs::remove_dir_all(&cache_root);
    fs::create_dir_all(&cache_root).unwrap();
    crate::modcache::set_assets_enabled(true);
    crate::modcache::hd_map::reset();
    // 素材阶段：1x 基准图 + HD 登记（与生产同函数）
    crate::modcache::assets::extract_game_def_frame_and_save(
        &game, &cache_root, "core", "PSKIL42", 0, 0, "pskil42", "attack",
    )
    .expect("应提取 PSKIL42 首帧");
    crate::modcache::hd_map::flush(&cache_root);
    let map = crate::modcache::hd_map::load(&cache_root, "core");
    assert_eq!(
        map.get("pskil42/pskil42_def~g0f0").map(String::as_str),
        Some("PSKIL42A"),
        "PSKIL42 首帧必须登记 HD 源"
    );
    // 高清阶段：1080p（phys=1.5）→ 应得 2x
    let opt = HdOptions {
        rate: 4,
        phys: 1.5,
        upscaler: "xbrz".into(),
        ai: false,
        ai_model: None,
        usm: 0.0,
        ai_mix: 0.75,
    };
    let st = generate_for_mod(&cache_root, "core", &opt);
    let dir = cache_root.join("core").join("images").join("pskil42");
    let variant = dir.join("pskil42_def~g0f0@2x.webp");
    let dim = |p: &std::path::Path| -> Option<(u32, u32)> {
        let b = fs::read(p).ok()?;
        image::load_from_memory(&b)
            .ok()
            .map(|i| (i.width(), i.height()))
    };
    println!(
        "generated={} skipped={} 1x={:?} @2x={:?}",
        st.generated,
        st.skipped,
        dim(&dir.join("pskil42_def~g0f0.webp")),
        dim(&variant)
    );
    assert!(variant.is_file(), "1080p 下 pskil42 必须出 @2x 变体");
    let (w, h) = dim(&variant).expect("变体应可解码");
    assert!(w >= 84 && h >= 84, "变体尺寸应 ≥ 84×84，实际 {w}×{h}");
    let _ = fs::remove_dir_all(&cache_root);
}
