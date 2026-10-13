//! 两阶段构建、缓存版本与索引（冒烟测试，多数 `#[ignore]`，需真实游戏目录）。

use super::*;

/// 两阶段构建回归：阶段一只出数据（`assetsDone=false`、无图），阶段二补图后置
/// `assetsDone=true`；此后整体命中缓存不再重建。
#[test]
#[ignore]
fn two_phase_build_regression() {
    let game_dir = game_dir();
    let core_mod = game_dir.join("Mods").join("vcmi");
    if !core_mod.is_dir() {
        eprintln!("未检测到 {} 内的 Mods/vcmi，跳过", game_dir.display());
        return;
    }
    let cache = std::env::temp_dir().join("vcmi_fv_two_phase_test");
    let _ = fs::remove_dir_all(&cache);
    let _ = fs::create_dir_all(&cache);
    let mp = core_mod.to_string_lossy().to_string();

    // 阶段一：只出数据
    let r0 = build_mod_wiki_phase(&mp, "core.castle", &cache, &game_dir, 0);
    assert_eq!(
        r0.get("ok").and_then(|x| x.as_bool()),
        Some(true),
        "阶段一应成功: {r0}"
    );
    let v0: Value =
        serde_json::from_str(&fs::read_to_string(cache.join("core.castle/cache.json")).unwrap())
            .unwrap();
    assert_eq!(
        v0.get("assetsDone").and_then(|x| x.as_bool()),
        Some(false),
        "阶段一 assetsDone 应为 false"
    );
    let f0 = v0.get("factions").and_then(|x| x.as_array()).unwrap();
    assert_eq!(f0.len(), 1, "阶段一也应有城镇数据（仅缺图）");
    assert!(
        f0[0].get("preview").map(|x| x.is_null()).unwrap_or(true),
        "阶段一不应生成预览图"
    );

    // 阶段二：补图
    let r1 = build_mod_wiki_phase(&mp, "core.castle", &cache, &game_dir, 1);
    assert_eq!(
        r1.get("ok").and_then(|x| x.as_bool()),
        Some(true),
        "阶段二应成功: {r1}"
    );
    let v1: Value =
        serde_json::from_str(&fs::read_to_string(cache.join("core.castle/cache.json")).unwrap())
            .unwrap();
    assert_eq!(
        v1.get("assetsDone").and_then(|x| x.as_bool()),
        Some(true),
        "阶段二 assetsDone 应为 true"
    );
    assert!(
        v1.get("assetsVersion").and_then(|x| x.as_u64()).unwrap_or(0) >= 1,
        "阶段二应写入 assetsVersion"
    );

    // 再跑一次（两阶段）应整体命中缓存
    let r2 = build_mod_wiki_phase(&mp, "core.castle", &cache, &game_dir, 2);
    assert_eq!(
        r2.get("cached").and_then(|x| x.as_bool()),
        Some(true),
        "素材就绪后应整体命中缓存: {r2}"
    );
    println!("两阶段构建回归通过：阶段一出数据 → 阶段二补图 → 后续命中缓存");
}

/// 回归：`sourceHash` 必须**只由内容决定**——这是「缓存随包携带到其他设备仍能命中」的前提。
/// 同内容不同路径 / 不同 mtime 必须得到相同哈希；内容变化必须得到不同哈希。
#[test]
fn source_hash_is_content_based() {
    fn copy_dir(from: &Path, to: &Path) {
        let _ = fs::create_dir_all(to);
        let Ok(entries) = fs::read_dir(from) else { return };
        for e in entries.flatten() {
            let p = e.path();
            let dst = to.join(e.file_name());
            if p.is_dir() {
                copy_dir(&p, &dst);
            } else {
                let _ = fs::copy(&p, &dst);
            }
        }
    }
    let src = fixture("abyss-town");
    if !src.is_dir() {
        eprintln!("fixtures 不存在，跳过");
        return;
    }
    let tmp = std::env::temp_dir().join("vcmi_fv_sourcehash_test");
    let _ = fs::remove_dir_all(&tmp);
    copy_dir(&src, &tmp);

    let h_src = compute_source_hash(&src);
    let h_copy = compute_source_hash(&tmp);
    assert_eq!(h_src, h_copy, "同内容不同路径应得到相同哈希");

    // 改 mtime（模拟跨设备拷贝 / ZIP 解压）：哈希不得变化
    let mj = tmp.join("mod.json");
    if let Ok(f) = fs::OpenOptions::new().write(true).open(&mj) {
        let t = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_000_000_000);
        if f.set_modified(t).is_ok() {
            assert_eq!(
                compute_source_hash(&tmp),
                h_copy,
                "mtime 变化不得影响哈希（否则便携缓存必然失效）"
            );
        }
    }

    // 内容变化：哈希必须变化
    if let Ok(mut text) = fs::read_to_string(&mj) {
        text.push('\n');
        if fs::write(&mj, text).is_ok() {
            assert_ne!(compute_source_hash(&tmp), h_copy, "内容变化应改变哈希");
        }
    }
    println!("sourceHash 内容相关性回归通过：{h_copy}");
}

#[test]
fn test_build_mod_wiki_cache() {
    let root = fixture("");
    if !root.is_dir() {
        eprintln!("fixtures 不存在，跳过");
        return;
    }
    let cache = std::env::temp_dir().join("vcmi_fv_test_wiki_cache");
    let _ = fs::remove_dir_all(&cache);
    let _ = fs::create_dir_all(&cache);
    let mut total = 0;
    let mut ok = 0;
    for entry in fs::read_dir(&root).unwrap().flatten() {
        let mp = entry.path();
        if !mp.is_dir() {
            continue;
        }
        let mod_id = mp.file_name().unwrap().to_string_lossy().to_string();
        let res = build_mod_wiki_inner(&mp.to_string_lossy(), &mod_id, &cache, Path::new(""));
        if res.get("ok").and_then(|x| x.as_bool()).unwrap_or(false) {
            ok += 1;
        }
        total += 1;
    }
    println!("所有 mod 构建 wiki 缓存: {}/{} 成功", ok, total);
    // abyss-town 是测试专用城镇 mod，需存在于游戏目录 Mods 下；缺失则跳过（不视为失败），
    // 避免换机器/未安装该 mod 时整套用例失效。
    let abyss_path = fixture("abyss-town");
    if !abyss_path.is_dir() {
        eprintln!(
            "跳过：abyss-town 测试 mod 不存在（{}），装好该 mod 后再跑",
            abyss_path.display()
        );
        return;
    }
    // 有城镇配置的 mod 应全部成功；非城镇 mod 返回 notown 属预期
    let abyss = build_mod_wiki_inner(
        &abyss_path.to_string_lossy(),
        "abyss-town",
        &cache,
        Path::new(""),
    );
    assert_eq!(abyss.get("ok").and_then(|x| x.as_bool()).unwrap_or(false), true, "abyss 应构建成功");
    let mod_json = cache.join("abyss-town/cache.json");
    assert!(mod_json.is_file(), "应生成 abyss-town/cache.json");
    let text = fs::read_to_string(&mod_json).unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    let factions = v.get("factions").and_then(|x| x.as_array()).unwrap();
    assert!(!factions.is_empty(), "应有 faction");
    let bg = factions[0].get("background").and_then(|x| x.as_str()).unwrap_or("");
    println!("abyss 背景图路径: {}", bg);
    // 若生成了图片，验证 webp 文件存在且为 RIFF
    if !bg.is_empty() {
        let img = cache.join(bg);
        assert!(img.is_file(), "背景图文件应存在: {}", bg);
        let data = fs::read(&img).unwrap();
        assert!(data.starts_with(b"RIFF"), "应为 webp");
    }
}

#[test]
#[ignore = "需 references/Heroes3-VCMI 游戏 fixture"]
fn test_build_all_mod_cache_index() {
    let cache = std::env::temp_dir().join("vcmi_fv_test_wiki_index");
    let _ = fs::remove_dir_all(&cache);
    let _ = fs::create_dir_all(&cache);
    // 构造两个 Town mod 的 scan_mods 风格节点
    let mods = json!([
        {
            "id": "abyss-town",
            "modPath": fixture("abyss-town").to_string_lossy(),
            "diskName": "abyss-town",
            "parentId": "abyss-town",
            "type": "Town",
            "name": "Abyss",
            "contentZipEncrypted": false,
            "subs": []
        },
        {
            "id": "some-hero",
            "modPath": fixture("crossroads-of-worlds").to_string_lossy(),
            "diskName": "crossroads-of-worlds",
            "parentId": "crossroads-of-worlds",
            "type": "Heroes",
            "name": "CoW",
            "subs": []
        }
    ]);
    let res = build_all_wiki_inner(&mods, &cache);
    assert_eq!(res.get("ok").and_then(|x| x.as_bool()).unwrap_or(false), true);
    assert!(res.get("built").and_then(|x| x.as_i64()).unwrap_or(0) >= 1, "abyss-town 应被构建");
    // 所有 mod（含非 Town）都应入索引，且带 contentCount
    assert!(res.get("totalIndexed").and_then(|x| x.as_i64()).unwrap_or(0) >= 2, "所有 mod 都应入索引");
    let idx = cache.join("index.json");
    assert!(idx.is_file(), "应生成 index.json");
    let text = fs::read_to_string(&idx).unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    let mods_arr = v.get("mods").and_then(|x| x.as_array()).unwrap();
    assert!(mods_arr.iter().any(|m| m.get("modId").and_then(|x| x.as_str()) == Some("abyss-town")));
    // index 条目应带 contentCount
    let abyss_entry = mods_arr.iter().find(|m| m.get("modId").and_then(|x| x.as_str()) == Some("abyss-town")).unwrap();
    let cc = abyss_entry.get("contentCount").and_then(|x| x.get("creature")).and_then(|x| x.as_i64()).unwrap_or(0);
    assert!(cc > 0, "abyss 的 index 应记录生物数，实际 {}", cc);
    // abyss-town 的 mod.json 应已生成
    assert!(cache.join("abyss-town/cache.json").is_file(), "应生成 abyss-town/cache.json");
    println!("index.json 已生成，收录 {} 个 mod，abyss 生物数 {}", mods_arr.len(), cc);
}

#[test]
#[ignore = "需 references/Heroes3-VCMI 游戏 fixture"]
fn test_read_wiki_cache_roundtrip() {
    // 验证免递归读取：build 生成缓存后，read 只读 mod.json（不重新解析原始 mod）
    let cache = std::env::temp_dir().join("vcmi_fv_test_wiki_roundtrip");
    let _ = fs::remove_dir_all(&cache);
    let _ = fs::create_dir_all(&cache);
    let mp = fixture("abyss-town").to_string_lossy().to_string();
    let b = build_mod_wiki_inner(&mp, "abyss-town", &cache, Path::new(""));
    assert_eq!(b.get("ok").and_then(|x| x.as_bool()).unwrap_or(false), true);

    let r = read_mod_wiki_inner(&cache, "abyss-town");
    assert_eq!(r.get("ok").and_then(|x| x.as_bool()).unwrap_or(false), true, "应能命中缓存读取");
    let data = r.get("data").unwrap();
    let factions = data.get("factions").and_then(|x| x.as_array()).unwrap();
    assert_eq!(factions.len(), 1, "abyss 应有 1 个 faction");
    let f = &factions[0];
    assert_eq!(f.get("factionId").and_then(|x| x.as_str()).unwrap_or(""), "abyss");
    let bg = f.get("background").and_then(|x| x.as_str()).unwrap_or("");
    assert!(!bg.is_empty(), "背景图应已写入缓存并在 JSON 中记录路径");
    // 缓存命中后，再次读取同样成功（幂等）
    let r2 = read_mod_wiki_inner(&cache, "abyss-town");
    assert_eq!(r2.get("ok").and_then(|x| x.as_bool()).unwrap_or(false), true);
    println!("免递归读取验证通过：{} faction={} bg={}", data.get("modId").and_then(|x| x.as_str()).unwrap_or(""), f.get("factionId").unwrap(), bg);
}

#[test]
#[ignore = "需 references/Heroes3-VCMI 游戏 fixture"]
fn test_wiki_all_types() {
    // 验证全类型解析：abyss-town 应解析出生物/英雄/法术/宝物，并提取图标 webp
    let cache = std::env::temp_dir().join("vcmi_fv_test_wiki_types");
    let _ = fs::remove_dir_all(&cache);
    let _ = fs::create_dir_all(&cache);
    let mp = fixture("abyss-town").to_string_lossy().to_string();
    let b = build_mod_wiki_inner(&mp, "abyss-town", &cache, Path::new(""));
    assert_eq!(b.get("ok").and_then(|x| x.as_bool()).unwrap_or(false), true);
    let cc = b.get("contentCount").unwrap();
    println!("abyss-town contentCount: {}", cc);
    let creatures = cc.get("creature").and_then(|x| x.as_i64()).unwrap_or(0);
    let heroes = cc.get("hero").and_then(|x| x.as_i64()).unwrap_or(0);
    assert!(creatures > 0, "abyss 应有生物，实际 {}", creatures);
    assert!(heroes > 0, "abyss 应有英雄，实际 {}", heroes);
    // 读取缓存确认 creatures 里 image 路径
    let r = read_mod_wiki_inner(&cache, "abyss-town");
    let data = r.get("data").unwrap();
    let cs = data.get("creatures").and_then(|x| x.as_array()).unwrap();
    if let Some(first) = cs.first() {
        let img = first.get("image").and_then(|x| x.as_str()).unwrap_or("");
        if !img.is_empty() {
            let img_path = cache.join(img);
            assert!(img_path.is_file(), "生物图标应已生成: {}", img);
            let bytes = fs::read(&img_path).unwrap();
            assert!(bytes.starts_with(b"RIFF"), "应为 webp");
        }
    }
    // 验证 i18n：abyss 的生物中文名应存在（translation/abyss/chinese.json）
    println!("abyss 生物示例: {:?}", cs.first().map(|c| c.get("name")));

    // 关系链验证
    let facs = data.get("factions").and_then(|x| x.as_array()).unwrap();
    let cl = facs[0].get("creatureList").and_then(|x| x.as_array()).unwrap();
    assert!(!cl.is_empty(), "城镇应提取 creatureList 关系链，实际 {}", cl.len());
    println!("abyss faction creatureList 数量: {}", cl.len());
    // 生物升级链/动画引用
    let has_any = cs.iter().any(|c| {
        c.get("upgrades").and_then(|u| u.as_array()).map(|a| !a.is_empty()).unwrap_or(false)
    });
    println!("abyss 有生物带 upgrades: {}", has_any);
    let anim_refs = cs.iter().filter(|c| {
        c.get("animation").and_then(|a| a.as_str()).map(|s| !s.is_empty()).unwrap_or(false)
            || c.get("mapAnimation").and_then(|a| a.as_str()).map(|s| !s.is_empty()).unwrap_or(false)
    }).count();
    println!("abyss 生物带动画引用数: {}", anim_refs);
}

/// 回归：预检 `probe_mod_cache` 必须与真实构建**同源判定**（防两处口径漂移 —— 预检与构建
/// 若分叉，就会出现"预检说不用建、构建却重建"或反之，直接表现为进度条行为错乱）。
///
/// 覆盖四种状态：无缓存 → `NeedData`；全量构建后 → `Cached`；图目录被删 → `NeedAssets`；
/// 内容变化 → `NeedData`。全程在**临时副本**上操作，绝不改动仓库 fixture。
#[test]
#[ignore = "需真实游戏目录（Mods/vcmi 的 core.castle）"]
fn probe_mod_cache_matches_build() {
    use crate::modcache::build::{probe_mod_cache, ModCacheState};
    let game_dir = game_dir();
    let core_mod = game_dir.join("Mods").join("vcmi");
    if !core_mod.is_dir() {
        eprintln!("未检测到 {} 内的 Mods/vcmi，跳过", game_dir.display());
        return;
    }
    let cache = std::env::temp_dir().join("vcmi_fv_test_probe");
    let _ = fs::remove_dir_all(&cache);
    let _ = fs::create_dir_all(&cache);
    let mp = core_mod.to_string_lossy().to_string();
    let cache_json = cache.join("core.castle").join("cache.json");
    let state = |c: &Path| probe_mod_cache(&mp, "core.castle", c, &game_dir).state;

    // 1) 无缓存 → 需要数据重建
    assert_eq!(state(&cache), ModCacheState::NeedData, "无缓存应判 NeedData");

    // 2) 两阶段全量构建 → 完全命中
    let b = build_mod_wiki_inner(&mp, "core.castle", &cache, &game_dir);
    assert_eq!(b.get("ok").and_then(|x| x.as_bool()), Some(true), "构建应成功: {b}");
    assert_eq!(state(&cache), ModCacheState::Cached, "全量构建后应判 Cached");

    // 3) 素材版本落后（篡改 `assetsVersion`）→ 仅素材缺失，强制重提取
    let images = cache.join("core.castle").join("images");
    assert!(images.is_dir(), "全量构建应产出 images/");
    let mut v: Value = serde_json::from_str(&fs::read_to_string(&cache_json).unwrap()).unwrap();
    v["assetsVersion"] = json!(0);
    fs::write(&cache_json, serde_json::to_string(&v).unwrap()).unwrap();
    assert_eq!(state(&cache), ModCacheState::NeedAssets, "assetsVersion 落后应判 NeedAssets");
    // 重建补素材 → 回到命中
    let _ = build_mod_wiki_inner(&mp, "core.castle", &cache, &game_dir);
    assert_eq!(state(&cache), ModCacheState::Cached, "补素材后应回到 Cached");

    // 4) 篡改缓存里的 sourceHash（等价于内容变化，且**不动游戏目录**）→ 缓存键失效
    let mut v: Value = serde_json::from_str(&fs::read_to_string(&cache_json).unwrap()).unwrap();
    v["sourceHash"] = json!("deadbeefdeadbeef");
    fs::write(&cache_json, serde_json::to_string(&v).unwrap()).unwrap();
    assert_eq!(state(&cache), ModCacheState::NeedData, "缓存键失效应判 NeedData");

    println!("probe_mod_cache 与构建同源判定回归通过（NeedData→Cached→NeedAssets→Cached→NeedData）");
}

#[test]
#[ignore = "需 references/Heroes3-VCMI 游戏 fixture"]
fn test_wiki_incremental_cache() {
    // 验证增量：第一次构建（cached=false），第二次哈希一致（cached=true 跳过）
    let cache = std::env::temp_dir().join("vcmi_fv_test_wiki_incr");
    let _ = fs::remove_dir_all(&cache); // 确保从干净状态开始（避免上次残留）
    let _ = fs::create_dir_all(&cache);
    let mp = fixture("abyss-town").to_string_lossy().to_string();
    let r1 = build_mod_wiki_inner(&mp, "abyss-town", &cache, Path::new(""));
    assert_eq!(r1.get("ok").and_then(|x| x.as_bool()).unwrap_or(false), true);
    assert_eq!(r1.get("cached").and_then(|x| x.as_bool()).unwrap_or(false), false, "首次应真正构建");
    // 读取已生成 mod.json 应有 sourceHash
    let text = fs::read_to_string(cache.join("abyss-town/cache.json")).unwrap();
    let v: Value = serde_json::from_str(&text).unwrap();
    assert!(v.get("sourceHash").and_then(|x| x.as_str()).is_some(), "应写入 sourceHash");
    // 第二次：哈希一致 → cached=true
    let r2 = build_mod_wiki_inner(&mp, "abyss-town", &cache, Path::new(""));
    assert_eq!(r2.get("cached").and_then(|x| x.as_bool()).unwrap_or(false), true, "第二次应命中缓存跳过");
    println!("增量验证通过：首次 built，二次 cached");
}
