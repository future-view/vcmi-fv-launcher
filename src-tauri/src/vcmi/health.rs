//! 游戏数据完整性体检（`gamedata_health_check` 的实现）：**只读**，不改动任何文件。
//!
//! 判定依据（全部来自实测 + VCMI 源码，不做臆测）：
//! - 引擎启动前硬性检查（`references/vcmi/clientapp/EntryPoint.cpp:294-298`）：
//!   `DATA/HELP.TXT`、`MODS/VCMI/MOD.JSON`、`DATA/NOTOSERIF-MEDIUM.TTF`、
//!   `DATA/PLAYERS.PAL`、`SPRITES/DEFAULT.DEF`。
//! - 官方启动器的四种缺失诊断（`references/vcmi/launcher/startGame/StartGameTab.cpp:153-176`）：
//!   9 个 `Data/TpThBk*.bmp`、AB 战役、音轨、开场视频。
//!
//! ⚠️ **实测形态**（决定检查方式）：
//! - H3 数据不是散文件，而是打包在 `Data/*.lod` / `.snd` / `.vid` 容器里：
//!   `H3bitmap.lod` 含 `HELP.TXT`、`PLAYERS.PAL`、`TPTHBK*.PCX`；
//!   `H3sprite.lod` 含 `DEFAULT.DEF`；`H3ab_bmp.lod` 含 `AB.H3C` 等 6 个 AB 战役。
//!   故容器内的项目用 `infra::lod::lod_entry_names` 检查条目名，**而不是判散文件**
//!   （判散文件会把正常的完整安装全判成缺失）。
//! - 资源根有两处：**游戏目录**与**用户数据目录**（VCMI 同时挂载，实测官方版把 H3 数据
//!   放在 userDataPath、增强版放游戏目录），任一命中即算通过。
//! - 文件名大小写混用（如内置字体实际是 `NotoSerif-Medium.ttf`，而 VCMI 虚拟路径写作
//!   `DATA/NOTOSERIF-MEDIUM.TTF`），故一律大小写不敏感匹配。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::infra::lod::lod_entry_names;
use crate::vcmi::paths::VcmiPaths;

/// 单个体检项：`id` 供前端 i18n 取文案，`level` 决定缺失时的严重度。
struct Item {
    id: &'static str,
    /// `fatal` = 缺了游戏无法启动；`warn` = 缺了影响体验（缺音轨/视频/扩展素材）。
    level: &'static str,
    ok: bool,
    /// 命中的路径（存在时）或缺失说明（缺失时），供 UI 显示细节。
    detail: String,
}

/// 数据体检入口（纯逻辑，不依赖 Tauri）。
pub(crate) fn health_check(paths: &VcmiPaths) -> Value {
    health_check_with_roots(&resource_roots(paths))
}

/// 体检主体：直接指定资源根（供单测 / 端到端验证使用，避免构造 `VcmiPaths`）。
pub(crate) fn health_check_with_roots(roots: &[PathBuf]) -> Value {
    // lod 条目表按需读取并缓存（每个 lod 只读文件头几百 KB，非整个容器）
    let mut lod_cache: HashMap<PathBuf, Vec<String>> = HashMap::new();
    let mut items: Vec<Item> = Vec::new();

    // ── 1) 内置核心 mod（VCMI 自带，缺了引擎直接报 installation corrupted）
    let core_mod = find_ci(&roots, "Mods/vcmi/mod.json");
    items.push(Item {
        id: "coreMod",
        level: "fatal",
        ok: core_mod.is_some(),
        detail: core_mod
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "Mods/vcmi/mod.json".into()),
    });

    // ── 2) 内置字体（VCMI 虚拟路径 DATA/NOTOSERIF-MEDIUM.TTF）
    let font = find_ci(&roots, "Mods/vcmi/Content/Data/NotoSerif-Medium.ttf");
    items.push(Item {
        id: "builtinFont",
        level: "fatal",
        ok: font.is_some(),
        detail: font
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "Mods/vcmi/Content/Data/NotoSerif-Medium.ttf".into()),
    });

    // ── 3) 位图数据 H3bitmap.lod（内含 HELP.TXT / PLAYERS.PAL）
    let bitmap = find_ci(&roots, "Data/H3bitmap.lod");
    let bitmap_entries = bitmap
        .as_ref()
        .and_then(|p| lod_entries(&mut lod_cache, p));
    let bitmap_ok = bitmap_entries
        .as_ref()
        .map(|e| e.iter().any(|x| x == "HELP.TXT") && e.iter().any(|x| x == "PLAYERS.PAL"))
        .unwrap_or(false);
    items.push(Item {
        id: "bitmapLod",
        level: "fatal",
        ok: bitmap_ok,
        detail: match (&bitmap, bitmap_ok) {
            (Some(p), true) => p.to_string_lossy().to_string(),
            (Some(p), false) => format!("{}（缺少 HELP.TXT / PLAYERS.PAL）", p.to_string_lossy()),
            (None, _) => "Data/H3bitmap.lod".into(),
        },
    });

    // ── 4) 精灵数据 H3sprite.lod（内含 SPRITES/DEFAULT.DEF）
    let sprite = find_ci(&roots, "Data/H3sprite.lod");
    let sprite_entries = sprite
        .as_ref()
        .and_then(|p| lod_entries(&mut lod_cache, p));
    let sprite_ok = sprite_entries
        .as_ref()
        .map(|e| e.iter().any(|x| x == "DEFAULT.DEF"))
        .unwrap_or(false);
    items.push(Item {
        id: "spriteLod",
        level: "fatal",
        ok: sprite_ok,
        detail: match (&sprite, sprite_ok) {
            (Some(p), true) => p.to_string_lossy().to_string(),
            (Some(p), false) => format!("{}（缺少 DEFAULT.DEF）", p.to_string_lossy()),
            (None, _) => "Data/H3sprite.lod".into(),
        },
    });

    // ── 5) 城镇大厅位图（官方诊断的"残缺盗版"特征；实测为 lod 内 TPTHBK*.PCX）
    const TOWN_BG: &[&str] = &[
        "TPTHBKCS.PCX",
        "TPTHBKDG.PCX",
        "TPTHBKFR.PCX",
        "TPTHBKIN.PCX",
        "TPTHBKNC.PCX",
        "TPTHBKRM.PCX",
        "TPTHBKST.PCX",
        "TPTHBKTW.PCX",
    ];
    let town_ok = bitmap_entries
        .as_ref()
        .map(|e| TOWN_BG.iter().all(|n| e.iter().any(|x| x == n)))
        .unwrap_or(false);
    items.push(Item {
        id: "townBackgrounds",
        level: "warn",
        ok: town_ok,
        detail: if town_ok {
            "TPTHBK*.PCX ×8".into()
        } else {
            "H3bitmap.lod 内 TPTHBK*.PCX 不齐".into()
        },
    });

    // ── 6) AB（末日之刃）扩展战役：H3ab_bmp.lod 内含 6 个战役脚本
    const AB_CAMPAIGNS: &[&str] = &[
        "AB.H3C",
        "BLOOD.H3C",
        "SLAYER.H3C",
        "FESTIVAL.H3C",
        "FIRE.H3C",
        "FOOL.H3C",
    ];
    let ab_bmp = find_ci(&roots, "Data/H3ab_bmp.lod");
    let ab_entries = ab_bmp.as_ref().and_then(|p| lod_entries(&mut lod_cache, p));
    let ab_campaign_ok = ab_entries
        .as_ref()
        .map(|e| AB_CAMPAIGNS.iter().all(|n| e.iter().any(|x| x == n)))
        .unwrap_or(false);
    items.push(Item {
        id: "abCampaigns",
        level: "warn",
        ok: ab_campaign_ok,
        detail: match (&ab_bmp, ab_campaign_ok) {
            (Some(p), true) => p.to_string_lossy().to_string(),
            (Some(p), false) => format!("{}（战役脚本不全）", p.to_string_lossy()),
            (None, _) => "Data/H3ab_bmp.lod".into(),
        },
    });

    // ── 7) AB 扩展素材（精灵包 + 音效/视频容器）
    let ab_spr = find_ci(&roots, "Data/H3ab_spr.lod");
    let ab_snd = find_ci(&roots, "Data/H3ab_ahd.snd");
    let ab_vid = find_ci(&roots, "Data/H3ab_ahd.vid");
    let ab_assets_ok = ab_spr.is_some() && ab_snd.is_some() && ab_vid.is_some();
    items.push(Item {
        id: "abAssets",
        level: "warn",
        ok: ab_assets_ok,
        detail: if ab_assets_ok {
            "H3ab_spr.lod / H3ab_ahd.snd / H3ab_ahd.vid".into()
        } else {
            "Data/H3ab_spr.lod、H3ab_ahd.snd|.vid".into()
        },
    });

    // ── 8) 音轨（Music/MainMenu 对应的散文件 mp3，官方诊断项之一）
    let mp3_dir = find_has_mp3(&roots);
    items.push(Item {
        id: "soundtrack",
        level: "warn",
        ok: mp3_dir.is_some(),
        detail: mp3_dir
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "Mp3/*.mp3".into()),
    });

    // ── 9) 开场视频（Video/H3Intro）
    let video = find_ci(&roots, "Data/Heroes3.vid").or_else(|| find_ci(&roots, "Data/VIDEO.VID"));
    items.push(Item {
        id: "video",
        level: "warn",
        ok: video.is_some(),
        detail: video
            .as_ref()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|| "Data/Heroes3.vid | VIDEO.VID".into()),
    });

    let fatal_ok = items
        .iter()
        .filter(|i| i.level == "fatal")
        .all(|i| i.ok);

    json!({
        "ok": fatal_ok,
        "roots": roots.iter().map(|p| p.to_string_lossy().to_string()).collect::<Vec<_>>(),
        "items": items
            .iter()
            .map(|i| json!({ "id": i.id, "level": i.level, "ok": i.ok, "detail": i.detail }))
            .collect::<Vec<_>>(),
    })
}

/// 资源根候选：VCMI 同时挂载**游戏目录**与**用户数据目录**（实测两种发行方式各用其一）。
fn resource_roots(paths: &VcmiPaths) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for p in [&paths.game_dir, &paths.user_dir] {
        if !p.as_os_str().is_empty() && !out.iter().any(|x| x == p) {
            out.push(p.clone());
        }
    }
    out
}

/// 读取 lod 条目名并按路径缓存（同一份 lod 在一次体检里只读一次头部）。
fn lod_entries(cache: &mut HashMap<PathBuf, Vec<String>>, path: &Path) -> Option<Vec<String>> {
    if let Some(hit) = cache.get(path) {
        return Some(hit.clone());
    }
    let names = lod_entry_names(path)?;
    cache.insert(path.to_path_buf(), names.clone());
    Some(names)
}

/// 大小写不敏感的相对路径查找：逐段枚举目录命中，多个根依次尝试。
fn find_ci(roots: &[PathBuf], rel: &str) -> Option<PathBuf> {
    let segs: Vec<String> = rel.split('/').map(|s| s.to_string()).collect();
    for root in roots {
        let mut cur = root.clone();
        let mut ok = true;
        for seg in &segs {
            let mut hit: Option<PathBuf> = None;
            if let Ok(rd) = fs::read_dir(&cur) {
                for e in rd.flatten() {
                    if e.file_name().to_string_lossy().eq_ignore_ascii_case(seg) {
                        hit = Some(e.path());
                        break;
                    }
                }
            }
            match hit {
                Some(p) => cur = p,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            return Some(cur);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_ci_is_case_insensitive_and_falls_back_across_roots() {
        let base = std::env::temp_dir().join("vcmi-fv-health-find-test");
        let _ = fs::remove_dir_all(&base);
        let game = base.join("game");
        let user = base.join("user");
        fs::create_dir_all(game.join("Data")).expect("mkdir");
        fs::create_dir_all(&user).expect("mkdir");
        // 真实磁盘上大小写混用（内置字体即为 NotoSerif-Medium.ttf）
        fs::write(game.join("Data").join("H3Bitmap.LOD"), b"x").expect("write");

        let roots = vec![user.clone(), game.clone()];
        let hit = find_ci(&roots, "Data/H3bitmap.lod").expect("应命中（大小写不敏感）");
        assert_eq!(
            hit.file_name().map(|s| s.to_string_lossy().to_string()),
            Some("H3Bitmap.LOD".to_string())
        );
        // 目录名同样大小写不敏感，且命中属于**第一个**含该文件的根
        fs::create_dir_all(user.join("data")).expect("mkdir");
        fs::write(user.join("data").join("h3sprite.LOD"), b"x").expect("write");
        let hit2 = find_ci(&roots, "Data/H3Sprite.lod").expect("应命中 user 根");
        assert!(hit2.starts_with(&user), "应按 roots 顺序取第一个命中：{hit2:?}");
        // 完全不存在 → None（不误报）
        assert!(find_ci(&roots, "Data/NoSuchFile.lod").is_none());
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn find_has_mp3_detects_directory() {
        let base = std::env::temp_dir().join("vcmi-fv-health-mp3-test");
        let _ = fs::remove_dir_all(&base);
        let root = base.join("game");
        fs::create_dir_all(root.join("MP3")).expect("mkdir");
        assert!(find_has_mp3(&[root.clone()]).is_none(), "空目录不算命中");
        fs::write(root.join("MP3").join("mainmenu.mp3"), b"x").expect("write");
        assert!(find_has_mp3(&[root.clone()]).is_some());
        let _ = fs::remove_dir_all(&base);
    }

    /// 用参考目录跑**端到端**判定（默认 ignore：依赖本机 `references/` 存在）。
    /// 断言"完整安装不应有任何一项被判缺失" —— 这正是最容易出错的方向（误报）。
    fn assert_all_items_ok(roots: &[PathBuf], label: &str) {
        let v = health_check_with_roots(roots);
        let items = v["items"].as_array().cloned().unwrap_or_default();
        assert!(!items.is_empty(), "{label}: 未产出任何体检项");
        let failed: Vec<String> = items
            .iter()
            .filter(|i| i["ok"] != true)
            .map(|i| format!("{} ({})", i["id"].as_str().unwrap_or("?"), i["detail"].as_str().unwrap_or("")))
            .collect();
        assert!(failed.is_empty(), "{label}: 误报缺失 → {failed:?}");
        assert_eq!(v["ok"], true, "{label}: 顶层结论应为通过");
    }

    /// 增强版形态：H3 数据在**游戏目录**（`Data/` 与 `MP3/` 同级）。
    #[test]
    #[ignore = "需要 references/Heroes3-VCMI，仅人工验证"]
    fn real_enhanced_edition_is_reported_complete() {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("references")
            .join("Heroes3-VCMI");
        if !root.is_dir() {
            eprintln!("跳过：{} 不存在", root.display());
            return;
        }
        assert_all_items_ok(std::slice::from_ref(&root), "增强版");
    }

    /// 官方版形态：H3 数据在**用户数据目录**（游戏目录只有程序与内置 mod）。
    #[test]
    #[ignore = "需要 references/Heroes3-VCMI，仅人工验证"]
    fn real_official_edition_finds_data_in_userdata_root() {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("references")
            .join("Heroes3-VCMI");
        if !base.is_dir() {
            eprintln!("跳过：{} 不存在", base.display());
            return;
        }
        let roots = vec![base.clone(), base.join("userdata")];
        assert_all_items_ok(&roots, "官方版");
    }
}

/// 任一根下的 `Mp3`（大小写不敏感）目录内是否存在 `*.mp3`。
fn find_has_mp3(roots: &[PathBuf]) -> Option<PathBuf> {
    for root in roots {
        let dir = match find_ci(std::slice::from_ref(root), "Mp3") {
            Some(d) if d.is_dir() => d,
            _ => continue,
        };
        if let Ok(rd) = fs::read_dir(&dir) {
            let has = rd.flatten().any(|e| {
                e.path()
                    .extension()
                    .map(|x| x.eq_ignore_ascii_case("mp3"))
                    .unwrap_or(false)
            });
            if has {
                return Some(dir);
            }
        }
    }
    None
}
