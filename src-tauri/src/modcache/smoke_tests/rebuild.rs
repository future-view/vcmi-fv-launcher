//! 全量/批量重建驱动（真机 + 缓存重建脚本的入口用例）。
//!
//! `rebuild_all_with_core` 是 `npm run rebuild:wiki-cache` 实际调用的用例，改动它等于
//! 改动重建脚本的行为，需同步核对 `scripts/rebuild-wiki-cache.mjs`。

use super::*;

/// **全量重建真实缓存**（镜像应用 `ensureWikiCacheBuilt` 的流程），作为 CI / 人工验收的
/// 「重建驱动」端，配合 `node scripts/verify-wiki-cache.mjs` 校验产物是否齐全。
///
/// 流程（**与启动器 `ensureWikiCacheBuilt` 同款分批**）：扫描「游戏目录 Mods + 用户数据
/// 目录 Mods」→ 展平子模组 → 注入父 `core` 与 9 个 `core.<faction>`
/// → ① 全部 mod 阶段一（仅数据）→ ② 城镇批阶段二（`type === "Town"`，补图）
/// → ③ 其余批阶段二。
///
/// 环境变量（**均可用，不硬编码游戏目录**）：
/// - `VCMI_GAME_DIR`：游戏目录（缺省回落仓库 `references/Heroes3-VCMI` 验证 fixture）
/// - `VCMI_MOCK_CACHE_DIR`：缓存根（缺省 `references/.../.vcmi-fv-launcher/mods`），
///   可指向任意目录用于核对（不会污染默认缓存）
/// - `VCMI_USER_DATA_DIR`：用户数据目录（缺省按 `game_dir/config/dirs.json` 解析）
/// - `VCMI_WIKI_REBUILD_FRESH=1`：先清空全部 `mod.json` 强制全量重建（CI 用）；
///   不设则为增量（只补缺/过期的，适合日常恢复）
/// - `VCMI_WIKI_THREADS`：并发线程数（`0` = 用满全部逻辑核）；缺省走验收档（全部核）
///
/// 推荐用跨平台包装 CLI（自动设置以上变量）：
/// `node scripts/rebuild-wiki-cache.mjs [--game <dir>] [--cache <dir>] [--user <dir>] [--fresh] [--jobs max|N]`
///
/// 直接运行：`cargo test rebuild_all_with_core -- --ignored --nocapture`
#[test]
#[ignore]
fn rebuild_all_with_core() {
    // 埋点观测：测试进程**默认没有 tracing subscriber**（应用侧由 `infra::logging` 初始化），
    // 故设 `VCMI_WIKI_LOG=1` 时把 DEBUG 及以上日志打到 stdout（配合 `--nocapture` 可见），
    // 用于分析缓存构建的耗时构成（每 mod 一行 + 每批 Top15 汇总）。
    if std::env::var("VCMI_WIKI_LOG").is_ok() {
        let _ = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::DEBUG)
            .with_target(true)
            .try_init();
    }
    fn flatten(list: &[Value], out: &mut Vec<Value>) {
        for m in list {
            out.push(m.clone());
            if let Some(subs) = m.get("subs").and_then(|x| x.as_array()) {
                flatten(subs, out);
            }
        }
    }

    let game_dir = game_dir();
    if !game_dir.join("Mods").is_dir() {
        eprintln!(
            "游戏目录不含 Mods 子目录：{}（请用 --game 或 VCMI_GAME_DIR 指定）",
            game_dir.display()
        );
        return;
    }
    // 缓存根默认 = `<游戏目录>/.vcmi-fv-launcher/mods`。
    // 应用侧是 `VcmiPaths::mods_cache_dir()` = `<启动器 exe 同级>/.vcmi-fv-launcher/mods`；
    // 便携部署（启动器 exe 放在游戏目录，见 portable_deployment 文档）下两者**同一个目录**。
    // 测试进程的 exe 在 target/ 下，故不能直接用 mods_cache_dir()，改为按游戏目录推导；
    // 需要精确指定（例如与运行中的启动器共用）时用 `VCMI_MOCK_CACHE_DIR` / `--cache`。
    let cache_root = default_cache_root(&game_dir);
    let _ = fs::create_dir_all(&cache_root);
    println!("游戏目录：{}", game_dir.display());
    println!("缓存根　：{}", cache_root.display());
    if std::env::var("VCMI_WIKI_REBUILD_FRESH").is_ok() {
        // 彻底清空：删除所有 mod 缓存子目录（含旧版全局 `_shared`，重建时原版共享素材统一归位到 `core/images/`），
        // 并删除旧 index.json。批量构建的索引是「合并已有索引」语义（见 batch.rs），
        // 不清目录/索引会把历史幽灵条目（如被保留作用域跳过的 `vcmi` 空目录）带回，使"全量重建"不干净。
        //
        // ⚠️ **高危护栏（修复误删 realms 事故）**：`cache_root` 可能因 `--cache` / `VCMI_MOCK_CACHE_DIR`
        // 指错成启动器数据根 `<x>/.vcmi-fv-launcher`（而非 `<x>/.vcmi-fv-launcher/mods`）。
        // 此前的「遍历子目录递归删除」会把 `realms/`、`launcher.json`、`runtime/`、`state.json` 一并删掉。
        // 因此这里：① 锚定 cache_root 必须是 `*.vcmi-fv-launcher/mods`；② 拒绝清单兜底，绝不删
        // 受保护兄弟项；③ 目录联结（junction）只用 `remove_dir`，避免连带删掉联接目标（rules 04）。
        if !is_wiki_cache_dir(&cache_root) {
            eprintln!(
                "❌ 拒绝执行 --fresh 清空：缓存根不是 `<...>/.vcmi-fv-launcher/mods`（实得 {}）。\n\
                 \t若指向了启动器数据根 `.vcmi-fv-launcher`，继续会误删 realms/launcher.json/runtime/state.json。\n\
                 \t请用 --cache 指向正确的缓存子目录（即以 `/mods` 结尾）。",
                cache_root.display()
            );
            return;
        }
        const PROTECTED: &[&str] = &[
            "realms",
            "runtime",
            "launcher.json",
            "state.json",
            "cache",
            "downloads",
            "logs",
        ];
        for e in fs::read_dir(&cache_root).into_iter().flatten().flatten() {
            let p = e.path();
            if let Some(name) = p.file_name().and_then(|s| s.to_str()) {
                if PROTECTED.contains(&name) {
                    eprintln!("⚠️ 跳过受保护项（绝不删除）：{}", p.display());
                    continue;
                }
            }
            if p.is_dir() {
                if is_reparse_point(&p) {
                    // 安全红线：目录联结只能用 remove_dir（rules 04），否则删除目标真实文件
                    let _ = fs::remove_dir(&p);
                } else {
                    let _ = fs::remove_dir_all(&p);
                }
            } else if p.file_name().and_then(|s| s.to_str()) == Some("index.json") {
                let _ = fs::remove_file(&p);
            }
        }
    }

    // 1) 收集全部模组——**精确镜像** `scan_mods` 命令 + 前端 `flattenTree`：
    //    用户数据目录优先，游戏目录补齐同名之外的（同名以用户目录为准）。
    fn id_of(m: &Value) -> String {
        m.get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    }
    let user_dir = user_data_dir(&game_dir);
    let game_tree = scan_user_mods_dir(&game_dir.join("Mods"));
    let user_tree = scan_user_mods_dir(&user_dir.join("Mods"));
    let merged_tree: Vec<Value> = if game_tree.is_empty() {
        user_tree
    } else if user_tree.is_empty() {
        game_tree
    } else {
        let user_ids: std::collections::HashSet<String> = user_tree.iter().map(id_of).collect();
        let mut merged = user_tree;
        for m in game_tree {
            let id = id_of(&m);
            if !id.is_empty() && !user_ids.contains(&id) {
                merged.push(m);
            }
        }
        merged
    };
    let mut flat: Vec<Value> = Vec::new();
    flatten(&merged_tree, &mut flat);
    let sub_count = flat
        .iter()
        .filter(|m| m.get("isSubmod").and_then(|x| x.as_bool()).unwrap_or(false))
        .count();
    println!(
        "扫描：顶层 mod = {}　展平（含子模组）= {}　其中子模组 = {}",
        merged_tree.len(),
        flat.len(),
        sub_count
    );
    assert!(
        sub_count > 0,
        "应包含子模组（全量重建必须覆盖所有子模组，否则与启动器效果不一致）"
    );

    let mut batch: Vec<BatchWikiMod> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for m in &flat {
        let id = m
            .get("id")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let mod_path = m
            .get("modPath")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        if id.is_empty() || mod_path.is_empty() || !seen.insert(id.clone()) {
            continue;
        }
        batch.push(BatchWikiMod {
            mod_id: id,
            mod_path,
            extra: json!({
                "diskName": m.get("diskName").and_then(|x| x.as_str()).unwrap_or(""),
                "parentId": m.get("parentId").and_then(|x| x.as_str()).unwrap_or(""),
                "type": m.get("type").and_then(|x| x.as_str()).unwrap_or(""),
                "name": m.get("name").and_then(|x| x.as_str()).unwrap_or(""),
            }),
        });
    }
    // 2) 注入原版伪模组（父 core + 9 子）——与 `src/lib/modcache/buildTargets.ts` 的 buildCoreEntries 对齐。
    //    `name` 留空：显示名由后端从 h3-core 文案表派生（多语言），与启动器构建路径同款。
    let core_path = game_dir
        .join("Mods")
        .join("vcmi")
        .to_string_lossy()
        .to_string();
    batch.push(BatchWikiMod {
        mod_id: "core".into(),
        mod_path: core_path.clone(),
        extra: json!({ "diskName": "vcmi", "parentId": "", "type": "Core", "name": "" }),
    });
    for f in crate::vcmi::h3core::CORE_FACTIONS {
        batch.push(BatchWikiMod {
            mod_id: format!("core.{f}"),
            mod_path: core_path.clone(),
            extra: json!({ "diskName": f, "parentId": "core", "type": "Town", "name": "" }),
        });
    }

    let game_dir_s = game_dir.to_string_lossy().to_string();
    // 验收场景不关心 ECO 限速：默认用满全部逻辑核（档位 3）。
    // 如需限速，设 `VCMI_WIKI_THREADS=<n>`；`=0` 同样表示用满全部核。
    //
    // `VCMI_WIKI_THROTTLE=eco` → 档位 None，走**应用默认档 + 空闲感知**路径
    // （配合 `VCMI_WIKI_IDLE_SECS` 可模拟空闲机器验证自动提档）；
    // `=1|2|3` → 显式档位。
    let throttle: Option<u8> = match std::env::var("VCMI_WIKI_THROTTLE").ok().as_deref() {
        Some("eco") => None,
        Some(v) => v.trim().parse::<u8>().ok().or(Some(3)),
        None => Some(3),
    };
    // 3) 与启动器 `ensureWikiCacheBuilt` **同款分批**：
    //    ① 全部 mod 阶段一（仅数据）→ ② 城镇批阶段二（补图，让城镇卡片尽快出大图）
    //    → ③ 其余批阶段二。分批口径与前端一致：`type === "Town"` 归城镇批。
    let town_batch: Vec<BatchWikiMod> = batch
        .iter()
        .filter(|m| m.extra.get("type").and_then(|x| x.as_str()) == Some("Town"))
        .map(|m| BatchWikiMod {
            mod_id: m.mod_id.clone(),
            mod_path: m.mod_path.clone(),
            extra: m.extra.clone(),
        })
        .collect();
    let rest_batch: Vec<BatchWikiMod> = batch
        .iter()
        .filter(|m| m.extra.get("type").and_then(|x| x.as_str()) != Some("Town"))
        .map(|m| BatchWikiMod {
            mod_id: m.mod_id.clone(),
            mod_path: m.mod_path.clone(),
            extra: m.extra.clone(),
        })
        .collect();
    println!(
        "重建模组数 = {}（城镇批 {} + 其余批 {}）　并发线程 = {}",
        batch.len(),
        town_batch.len(),
        rest_batch.len(),
        wiki_threads(throttle)
    );
    // ① 阶段一：全部 mod，仅数据
    let t0 = std::time::Instant::now();
    let r1 =
        build_mods_cache_batch_inner(None, &game_dir_s, &batch, &cache_root, throttle, Some(0))
            .expect("阶段一失败");
    println!(
        "①阶段一（全部 c0）：{}（{}ms）",
        r1,
        t0.elapsed().as_millis()
    );
    // ② 阶段二：城镇批优先
    let t1 = std::time::Instant::now();
    let r2 = build_mods_cache_batch_inner(
        None,
        &game_dir_s,
        &town_batch,
        &cache_root,
        throttle,
        Some(1),
    )
    .expect("阶段二（城镇批）失败");
    println!(
        "②阶段二（城镇批 c1）：{}（{}ms）",
        r2,
        t1.elapsed().as_millis()
    );
    // ③ 阶段二：其余批
    let t2 = std::time::Instant::now();
    let r3 = build_mods_cache_batch_inner(
        None,
        &game_dir_s,
        &rest_batch,
        &cache_root,
        throttle,
        Some(1),
    )
    .expect("阶段二（其余批）失败");
    println!(
        "③阶段二（其余批 c1）：{}（{}ms）",
        r3,
        t2.elapsed().as_millis()
    );
    assert_eq!(r3.get("ok").and_then(|x| x.as_bool()), Some(true));

    // 4) 产物一致性：索引必须覆盖本次提交的**全部** mod（含子模组），
    //    否则说明有模组未被构建/未入索引 —— 与「启动器构建效果一致」不符
    let idx: Value =
        serde_json::from_str(&fs::read_to_string(cache_root.join("index.json")).unwrap()).unwrap();
    let idx_ids: std::collections::HashSet<String> = idx
        .get("mods")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|m| {
                    m.get("modId")
                        .and_then(|x| x.as_str())
                        .map(|s| s.to_string())
                })
                .collect()
        })
        .unwrap_or_default();
    let missing: Vec<&str> = batch
        .iter()
        .map(|b| b.mod_id.as_str())
        .filter(|id| !idx_ids.contains(*id))
        .collect();
    println!("索引 mods = {}（本次提交 {}）", idx_ids.len(), batch.len());
    assert!(
        missing.is_empty(),
        "索引缺少 {} 个已构建模组（前 5：{:?}）",
        missing.len(),
        &missing[..missing.len().min(5)]
    );
    println!("缓存目录：{}", cache_root.display());
    println!("校验：node scripts/verify-wiki-cache.mjs");
}

/// 锚定校验：`cache_root` 必须是 `<...>/.vcmi-fv-launcher/mods`，否则 `remove_dir_all` 会误删
/// 启动器数据根下的 `realms` / `launcher.json` / `runtime` / `state.json`（见 --fresh 护栏）。
fn is_wiki_cache_dir(cache_root: &Path) -> bool {
    let leaf = match cache_root.file_name().and_then(|s| s.to_str()) {
        Some(n) => n,
        None => return false,
    };
    if leaf != "mods" {
        return false;
    }
    cache_root
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n == ".vcmi-fv-launcher")
        .unwrap_or(false)
}

/// 目录联结（junction）判定：Windows 上 `remove_dir_all` 会连带删掉联结指向的真实目录，
/// 故删除前必须识别并改用 `remove_dir`（rules 04）。
#[cfg(windows)]
fn is_reparse_point(p: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    std::fs::symlink_metadata(p)
        .map(|m| m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
        .unwrap_or(false)
}
#[cfg(not(windows))]
fn is_reparse_point(_p: &Path) -> bool {
    false
}

#[test]
#[ignore]
fn full_batch_rebuild_smoke() {
    let game_dir = game_dir();
    let cache_root = default_cache_root(&game_dir);
    let _ = fs::create_dir_all(&cache_root);
    // 强制全量重建：清掉全部 mod.json（v11 起由 WIKI_CACHE_VERSION 机制自动失效，此处保险）
    for e in fs::read_dir(&cache_root).unwrap() {
        let mp = e.unwrap().path().join("mod.json");
        let _ = fs::remove_file(&mp);
    }
    // 收集全部城镇 mod（含 mod.json 的 faction 声明）
    let mods_dir = test_mods_dir();
    let mut batch: Vec<BatchWikiMod> = Vec::new();
    if let Ok(entries) = fs::read_dir(&mods_dir) {
        for e in entries.flatten() {
            let p = e.path();
            let mp = p.join("mod.json");
            if !mp.is_file() {
                continue;
            }
            if let Some(disk) = p.file_name().and_then(|x| x.to_str()) {
                // 顶层 mod 的 id = 目录名（norm_mod_id 顶层规则）；非城镇 mod 由 batch 内 notown 快速跳过
                let id = disk;
                batch.push(BatchWikiMod {
                    mod_id: id.to_string(),
                    mod_path: p.to_string_lossy().to_string(),
                    extra: json!({"diskName": disk, "parentId": id, "type": "Town", "name": ""}),
                });
            }
        }
    }
    println!("batch mods: {}", batch.len());
    // 测试场景不关心 ECO 限速：用满全部逻辑核
    let res = build_mods_cache_batch_inner(
        None,
        &game_dir.to_string_lossy().as_ref(),
        &batch,
        &cache_root,
        Some(3),
        None,
    )
    .expect("batch ok");
    let failed = res
        .get("failed")
        .and_then(|x| x.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    let fps = res
        .get("failedPreviews")
        .and_then(|x| x.as_array())
        .map(|a| a.len())
        .unwrap_or(0);
    println!("failed mods: {} ; failedPreviews: {}", failed, fps);
    for f in res
        .get("failedPreviews")
        .and_then(|x| x.as_array())
        .unwrap_or(&vec![])
    {
        println!("  FP: {}", f.as_str().unwrap_or(""));
    }
    // 抽样断言：abyss-town 的 preview + 英文名 + cacheVersion
    let j = cache_root.join("abyss-town/cache.json");
    let v: Value = serde_json::from_str(&fs::read_to_string(&j).unwrap()).unwrap();
    assert_eq!(
        v.get("cacheVersion").and_then(|x| x.as_u64()),
        Some(WIKI_CACHE_VERSION as u64),
        "cacheVersion 应与 WIKI_CACHE_VERSION 一致"
    );
    // 回归锁：references 的 launcher.json 用相对 game_dir（"./"），构建必须把 modPath 写成
    // 相对编码串（gameMods:/userMods:/realm:），不得写死绝对路径（见 encode_mod_path_for_cache
    // 与 batch 层用 VcmiPaths::detect() 解析后的绝对 game_dir 做 strip_prefix）。
    let mp = v.get("modPath").and_then(|x| x.as_str()).unwrap_or("");
    assert!(
        mp.starts_with("gameMods:") || mp.starts_with("userMods:") || mp.starts_with("realm:"),
        "cache.json 的 modPath 必须为相对编码（gameMods:/userMods:/realm:），实际：{mp}"
    );
    assert!(
        v.get("factions").unwrap()[0].get("preview").is_some(),
        "abyss 应有 preview"
    );
    assert!(
        cache_root.join("core/images/resources/6.webp").is_file(),
        "core/images/resources/6.webp（gold）应存在"
    );
}
