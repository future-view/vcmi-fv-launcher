//! 阵营主题曲（`town.musicTheme`）解析回归。
//!
//! 背景（用户报「阵营巡礼里 棱堡 / 工厂 / 沙堡 / 港口 没有背景音乐」，实测音乐文件都在盘上）：
//!
//! 1. **多文件城镇**：`read_town_configs` 会把同一阵营拆在多个 json（`faction.json` +
//!    `buildings.json` / `structures.json` …）的配置**字段级合并**。合并分支漏掉哪个字段，
//!    哪个字段就被先解析到的无该字段的文件顶成空 —— 主题曲只写在 `faction.json` 里，
//!    于是 `hota.cove` / `hota.bulwark` / 沙堡 的 `musicTheme` 恒为 `null`。
//! 2. **配置不在 `config/**` 树**：沙堡写在 `Content/townPavilion/faction.json`（变体目录），
//!    进不了 `raw.json` 的 `rawConfigs.towns`，只认 raw.json 就永远读不到。
//! 3. **theme 自带扩展名**：`hota.bulwark` 写作 `music/factions/BulwarkTown.mp3`，
//!    只按「拼接扩展名」找会拼出 `BulwarkTown.mp3.mp3`，全部落空。
//!
//! 本组验证两条不变量：① 缓存阵营条目带 `musicTheme`；② 该 theme 能解析到**真实存在**的音频文件。
//!
//! 运行：`VCMI_GAME_DIR=<游戏目录> cargo test --lib town_music -- --ignored --nocapture`
//! （游戏目录缺省回落仓库 `references/Heroes3-VCMI` fixture，未装对应 mod 的用例跳过）

use super::*;

/// 取 `<游戏目录>/Mods/<相对路径>`，不存在返回 None（用例跳过）。
fn game_mod(mod_rel: &str) -> Option<PathBuf> {
    let p = game_dir().join("Mods").join(mod_rel);
    p.is_dir().then_some(p)
}

/// 全新构建单个 mod 的**阶段一（仅数据）** 并读回 `cache.json`。
///
/// 只跑阶段一：本组断言全在数据层（配置解析），跑素材阶段会白等 DEF/webp 提取。
/// 每次先删掉本 mod 的缓存目录 —— 构建逻辑的改动不在缓存键里，残留缓存会让断言读到旧产物。
fn build_and_read(mod_dir: &Path, mod_id: &str, game_dir: &Path) -> Value {
    let cache_root = std::env::temp_dir().join("vcmi_fv_town_music_test");
    let mod_cache = cache_root.join(crate::modcache::sanitize_id(mod_id));
    let _ = fs::remove_dir_all(&mod_cache);
    let _ = fs::create_dir_all(&cache_root);
    let r = build_mod_wiki_phase(&mod_dir.to_string_lossy(), mod_id, &cache_root, game_dir, 0);
    assert_eq!(
        r.get("ok").and_then(|x| x.as_bool()),
        Some(true),
        "构建失败 {mod_id}: {r}"
    );
    let text = fs::read_to_string(mod_cache.join("cache.json"))
        .unwrap_or_else(|e| panic!("{mod_id} cache.json 未落盘: {e}"));
    serde_json::from_str(&text).expect("cache.json 解析失败")
}

/// 端到端断言：某阵营的 `musicTheme` 非空，且能解析到磁盘上真实存在的音频文件。
fn assert_music_resolves(mod_dir: &Path, mod_id: &str, faction_id: &str, game_dir: &Path) {
    let data = build_and_read(mod_dir, mod_id, game_dir);
    let faction = data
        .get("factions")
        .and_then(|x| x.as_array())
        .and_then(|a| {
            a.iter()
                .find(|f| f.get("factionId").and_then(|x| x.as_str()) == Some(faction_id))
        })
        .unwrap_or_else(|| panic!("{mod_id} 缓存里没有阵营 {faction_id}"));
    let theme = faction
        .get("musicTheme")
        .and_then(|x| x.as_str())
        .unwrap_or_else(|| panic!("{mod_id}/{faction_id} 的 musicTheme 为空（多文件合并 / 变体目录漏读）"))
        .to_string();
    let (music, music_zip) = crate::modcache::showcase::resolve_music_from_theme(
        game_dir,
        mod_id,
        &theme,
        Some(&mod_dir.to_string_lossy()),
    );
    let path = music.as_str().map(String::from).unwrap_or_default();
    assert!(
        !path.is_empty() && Path::new(&path).is_file(),
        "{mod_id}/{faction_id} 的 theme `{theme}` 没解析到音频文件（music={music}, musicZip={music_zip}）"
    );
}

/// 沙堡（New Pavilion）：配置在**变体目录** `Content/townPavilion/faction.json`，
/// 不在 `config/**` 树 → 只认 `raw.json` 的旧实现永远读不到 musicTheme。
#[test]
#[ignore]
fn pavilion_music_theme_from_variant_dir() {
    let Some(mod_dir) = game_mod("New Pavilion/Mods/New Pavillion") else {
        eprintln!("跳过：未安装 New Pavilion Town");
        return;
    };
    let game_dir = game_dir();
    crate::vcmi::resource::set_wiki_user_data_dir(&user_data_dir(&game_dir));
    assert_music_resolves(
        &mod_dir,
        "new pavilion.new pavillion",
        "pavilion",
        &game_dir,
    );
}

/// **全量审计**（诊断用，非断言）：扫游戏目录所有 mod，逐个对比「旧口径」与「新口径」的主题曲。
///
/// - 旧口径 = `raw.json` 的 `rawConfigs.towns[<fid>].town.musicTheme`（只聚合 `config/**` 树，
///   且没有多文件城镇的字段合并）；
/// - 新口径 = [`read_town_configs`] 解析 + 字段级合并（含变体目录与 `content.zip`）。
///
/// 输出四类：**旧逻辑漏**（新有旧无 = 本次修复覆盖到的"这种情况"）、**缺音乐文件**
/// （有 theme 但解析不到音频）、**正常**、**本就没有主题曲**。
///
/// 运行：`VCMI_GAME_DIR=<游戏目录> cargo test --lib town_music::audit -- --ignored --nocapture`
#[test]
#[ignore]
fn audit_town_music_all_mods() {
    let game_dir = game_dir();
    crate::vcmi::resource::set_wiki_user_data_dir(&user_data_dir(&game_dir));

    /// 枚举 mod 目录（顶层 + 子 mod，最多两层 `Mods/` 嵌套）+ 规范 modId。
    fn walk_mods(root: &Path, parent: &str, out: &mut Vec<(PathBuf, String)>) {
        let Ok(entries) = fs::read_dir(root) else { return };
        let mut dirs: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        for d in dirs {
            let disk = d
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            if disk.starts_with('.') {
                continue;
            }
            let id = if parent.is_empty() {
                disk.to_lowercase()
            } else {
                format!("{parent}.{}", disk.to_lowercase())
            };
            out.push((d.clone(), id.clone()));
            for sub in ["Mods", "mods"] {
                let p = d.join(sub);
                if p.is_dir() {
                    walk_mods(&p, &id, out);
                }
            }
        }
    }

    let mut mods: Vec<(PathBuf, String)> = Vec::new();
    walk_mods(&game_dir.join("Mods"), "", &mut mods);
    walk_mods(&user_mods_dir(), "", &mut mods);

    let t0 = std::time::Instant::now();
    let mut lost: Vec<String> = Vec::new();
    let mut missing_file: Vec<String> = Vec::new();
    let mut ok = 0usize;
    let mut no_theme = 0usize;

    for (i, (dir, mod_id)) in mods.iter().enumerate() {
        if i % 50 == 0 {
            eprintln!("… 已扫 {}/{}（{}s）", i, mods.len(), t0.elapsed().as_secs());
        }
        let configs = read_town_configs(dir);
        if configs.is_empty() {
            continue;
        }
        let raw = crate::modcache::collect::collect_all_raw_configs(dir);
        let towns = raw.get("towns").and_then(|x| x.as_object());
        for cfg in configs {
            let new_theme = cfg.music_theme.clone().unwrap_or_default();
            // 旧口径：`towns.<fid>`（可能带 `town` 子对象）
            let old_theme = towns
                .and_then(|t| t.get(&cfg.faction_id).or_else(|| t.get(&cfg.faction_id.to_lowercase())))
                .and_then(|v| Some(v.get("town").unwrap_or(v)))
                .and_then(|v| v.get("musicTheme"))
                .and_then(|v| v.as_str().or_else(|| v.get(0).and_then(|x| x.as_str())))
                .unwrap_or("")
                .to_string();

            let line = |tag: &str| {
                format!(
                    "{tag} · {mod_id}/{} · theme={new_theme}",
                    cfg.faction_id.clone()
                )
            };
            if new_theme.is_empty() {
                no_theme += 1;
                continue;
            }
            let (music, music_zip) = crate::modcache::showcase::resolve_music_from_theme(
                &game_dir,
                mod_id,
                &new_theme,
                Some(&dir.to_string_lossy()),
            );
            let path = music.as_str().unwrap_or("");
            if path.is_empty() && music_zip.is_null() {
                missing_file.push(line("缺音乐文件"));
                continue;
            }
            if old_theme.is_empty() {
                lost.push(line("旧逻辑漏"));
            } else {
                ok += 1;
            }
        }
    }

    eprintln!("\n──────── 阵营主题曲审计 · {} ────────", game_dir.display());
    eprintln!("扫描 mod {} 个，耗时 {}s", mods.len(), t0.elapsed().as_secs());
    eprintln!("正常（旧口径也能取到）: {ok}");
    eprintln!("本就没有主题曲: {no_theme}");
    // 同一个 mod 可能同时装在游戏目录与用户目录（VCMI 两边都挂载）→ 报告去重
    let report = |title: &str, list: &Vec<String>| {
        let mut seen: std::collections::HashSet<&String> = std::collections::HashSet::new();
        let uniq: Vec<&String> = list.iter().filter(|l| seen.insert(l)).collect();
        eprintln!("\n【{title}】共 {} 个：", uniq.len());
        for l in uniq {
            eprintln!("  {l}");
        }
    };
    report("旧逻辑漏（新有旧无 = 本次修复覆盖）", &lost);
    report("缺音乐文件（有 theme 但找不到音频）", &missing_file);
}

/// HotA 三镇（港口 / 工厂 / 棱堡）：**多文件城镇**，主题曲只写在 `faction.json` —
/// 合并分支漏掉 `music_theme` 时会被先解析到的 `buildings.json` 顶成 null。
/// 棱堡额外覆盖「theme 自带扩展名」这一支。
#[test]
#[ignore]
fn hota_multi_file_towns_keep_music_theme() {
    let game_dir = game_dir();
    crate::vcmi::resource::set_wiki_user_data_dir(&user_data_dir(&game_dir));
    for (rel, faction) in [
        ("hota/Mods/cove", "cove"),
        ("hota/Mods/Factory", "factory"),
        ("hota/Mods/bulwark", "bulwark"),
    ] {
        let Some(mod_dir) = game_mod(rel) else {
            eprintln!("跳过：未安装 {rel}");
            continue;
        };
        assert_music_resolves(&mod_dir, &format!("hota.{faction}"), faction, &game_dir);
    }
}
