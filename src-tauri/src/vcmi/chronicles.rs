//! 历代记（Heroes Chronicles）导入：**按官方产物结构生成 mod**。
//!
//! # 为什么是 mod 而不是"往 Data 里塞 LOD"
//!
//! 官方 `ChroniclesExtractor`（`references/vcmi/launcher/modManager/chroniclesextractor.cpp:90-244`）
//! 的产物是 `<userDataPath>/Mods/chronicles/` 这个 **Expansion 模组**：
//! - `mod.json` 里声明 `modType: Expansion` 与 `mapFormat.chronicles.supported = true`
//!   （引擎靠这个识别历代记地图格式）+ Tarnum 头像表；
//! - 每篇战役是它的子模组 `Mods/chronicles_<n>/`，素材放在 `content/` 下按 VCMI 命名空间
//!   分目录（`Data` / `Sprites` / `Video` / `Sounds` / `Maps`），战役文件 `Hc<n>_Main.h3c`
//!   落在 `Maps/Chronicles/`。
//! 直接把 `.lod` 拷进游戏 `Data` 会与原版数据混在一起，且引擎识别不到"这是历代记"，
//! 战役也不会出现在列表里 —— 这正是此前"复制 Data/Mp3/Maps"做法的偏差所在。
//!
//! 目录与文件命名逐项对齐官方。**实测**（GOG 合辑包 `setup_heroes_chronicles.exe`）：解包根下
//! **没有 `app/` 层**，直接是 `<篇名>/Data/` + 基础 `Data/`；`app/` 形态属官方 `getChronicleNo()`
//! 兼容的另一种布局，两种都要支持。几个容易搞错的点：
//! - 战役文件的来源是**解出的 Data**：`Main.h3c` 在 `xlBitmap.lod` 内（World Tree 例外，散在
//!   `<篇名>/Maps/`）。在**源** data 目录找 `Main.h3c` 只会让 8 篇里 7 篇缺战役；
//! - Tarnum 头像（`HPS137..142` / `HPL137..142`）与开场图 `INTRORIM` 都从**基础区**取
//!   （`bitmap.lod` / `lbitmap.lod`），不是历代记自己的 `xBitmap.lod`；
//! - 合辑包的篇目片头在基础区 `Hchron.vid` 里（`Intro/Intr2..4/Intro5/Intro6`）。
//!
//! `mod.json` 的文案（含 VCMI 多语言块）全部来自 `vcmi::modmeta`（`resources/mod-meta.json`），
//! 本模块不含任何译文：根字段英文兜底，语言块只覆盖有官方译文的字段。

use std::fs;
use std::path::{Path, PathBuf};

use crate::infra::berror::be_err;
use crate::infra::lod::LodArchive;

/// 历代记篇名（**1 起**；0 号留空对齐官方的 1-based 列表）
const CHRONICLES: [&str; 9] = [
    "",
    "Warlords of the Wasteland",
    "Conquest of the Underworld",
    "Masters of the Elements",
    "Clash of the Dragons",
    "The World Tree",
    "The Fiery Moon",
    "Revolt of the Beastmasters",
    "The Sword of Frost",
];

/// Tarnum 头像（官方 `extractFiles` 里写死的 12 个条目前缀）
const TARNUM_PORTRAITS: [&str; 12] = [
    "HPS137", "HPS138", "HPS139", "HPS140", "HPS141", "HPS142", "HPL137", "HPL138", "HPL139",
    "HPL140", "HPL141", "HPL142",
];

pub struct ChroniclesInstall {
    /// 基础模组目录（`<mods>/chronicles`）
    pub base_mod: PathBuf,
    /// 已生成的子模组名（如 `chronicles_1`）
    pub chronicles: Vec<String>,
    pub files: u64,
}

/// 从**已解包**的目录生成历代记模组。
///
/// `extracted_root`：已解包目录（实测 GOG 合辑包直接是 `<篇名>/Data` + 基础 `Data`；
/// 官方兼容的 `app/…` 布局也支持）。
/// `mods_dir`：模组根目录（`<用户数据目录或游戏目录>/Mods`）。
/// `game_dir`：游戏目录（用于取 `portraitsChronicles.json`）。
pub fn install(
    extracted_root: &Path,
    mods_dir: &Path,
    game_dir: &Path,
    overwrite: bool,
) -> Result<ChroniclesInstall, String> {
    let app =
        find_child_dir_ci(extracted_root, "app").unwrap_or_else(|| extracted_root.to_path_buf());
    let nos = detect_chronicles(&app);
    if nos.is_empty() {
        return Err("解包目录里没有识别到任何历代记篇目（缺少 app/<篇名>/data 结构）".to_string());
    }

    let base_mod = mods_dir.join("chronicles");
    if base_mod.exists() && !overwrite {
        return Err(be_err(
            "vcmi_hd_mod_already_exists",
            &format!("{}", base_mod.display()),
        ));
    }
    create_base_mod(&base_mod, game_dir, overwrite)?;

    let mut files = 0u64;
    let mut made: Vec<String> = Vec::new();
    // 基础游戏数据目录（Tarnum 头像 / INTRORIM 来源）：与篇目目录同级
    let base_data = find_child_dir_ci(&app, "data").unwrap_or_else(|| app.join("data"));
    for no in &nos {
        let dir_name = format!("chronicles_{}", no);
        let mod_dir = base_mod.join("Mods").join(&dir_name);
        if mod_dir.exists() {
            if overwrite {
                let _ = fs::remove_dir_all(&mod_dir);
            } else {
                continue;
            }
        }
        let n = create_chronicle_mod(&app, &base_data, &mod_dir, *no)?;
        files += n;
        made.push(dir_name);
    }
    // Tarnum 头像（基础模组共享，只需一次）
    files += extract_tarnum_portraits(&base_data, &base_mod, overwrite);

    Ok(ChroniclesInstall {
        base_mod,
        chronicles: made,
        files,
    })
}

/// 识别解包目录里包含哪些篇目（按 `app/<篇名>` 目录名匹配）。
fn detect_chronicles(app: &Path) -> Vec<usize> {
    let mut out = Vec::new();
    let Ok(rd) = fs::read_dir(app) else {
        return out;
    };
    let mut names: Vec<String> = Vec::new();
    for e in rd.flatten() {
        if e.path().is_dir() {
            names.push(e.file_name().to_string_lossy().to_string());
        }
    }
    for i in 1..CHRONICLES.len() {
        if names.iter().any(|n| n.eq_ignore_ascii_case(CHRONICLES[i])) {
            out.push(i);
        }
    }
    out
}

/// 基础模组：`mod.json` + `content/config/portraitsChronicles.json`（官方 `createBaseMod`）
fn create_base_mod(base_mod: &Path, game_dir: &Path, overwrite: bool) -> Result<(), String> {
    fs::create_dir_all(base_mod).map_err(|e| be_err("cmd_mkdir_failed", &format!("{}", e)))?;
    let json_path = base_mod.join("mod.json");
    if !json_path.exists() || overwrite {
        write_mod_json(&json_path, &base_mod_json())?;
    }
    // 头像表：来自游戏目录（VCMI 内置核心模组的 config/heroes）
    let candidates = [
        game_dir
            .join("Mods")
            .join("vcmi")
            .join("Content")
            .join("config")
            .join("heroes")
            .join("portraitsChronicles.json"),
        game_dir
            .join("config")
            .join("heroes")
            .join("portraitsChronicles.json"),
    ];
    for src in &candidates {
        if !src.is_file() {
            continue;
        }
        let dst_dir = base_mod.join("content").join("config");
        fs::create_dir_all(&dst_dir).map_err(|e| be_err("cmd_mkdir_failed", &format!("{}", e)))?;
        let dst = dst_dir.join("portraitsChronicles.json");
        if dst.exists() && !overwrite {
            break;
        }
        let _ = fs::remove_file(&dst);
        fs::copy(src, &dst).map_err(|e| be_err("infra_file_copy_failed", &format!("{}", e)))?;
        break;
    }
    Ok(())
}

/// 基础模组 `mod.json`（官方 `createBaseMod`）。
///
/// 文案（`name` / `description` / 作者 / 版本 / 语言块）来自 `vcmi::modmeta`，
/// 本模块只负责**结构**字段（`heroes` 与 `settings.mapFormat`）。
fn base_mod_json() -> serde_json::Value {
    use serde_json::json;
    let mut obj = serde_json::Map::new();
    obj.insert("modType".to_string(), json!("Expansion"));
    obj.extend(crate::vcmi::modmeta::chronicles_base_fields());
    obj.insert(
        "heroes".to_string(),
        json!(["config/portraitsChronicles.json"]),
    );
    // Tarnum 六种职业头像槽位（官方写死的 163..168）
    obj.insert(
        "settings".to_string(),
        json!({
            "mapFormat": {
                "chronicles": {
                    "supported": true,
                    "portraits": {
                        "portraitTarnumBarbarian": 163,
                        "portraitTarnumKnight": 164,
                        "portraitTarnumWizard": 165,
                        "portraitTarnumRanger": 166,
                        "portraitTarnumOverlord": 167,
                        "portraitTarnumBeastmaster": 168
                    }
                }
            }
        }),
    );
    serde_json::Value::Object(obj)
}

/// 2 空格缩进 + 结尾换行的 `mod.json`（与官方 `QJsonDocument::toJson` 的可读性对齐）。
fn write_mod_json(path: &Path, v: &serde_json::Value) -> Result<(), String> {
    let mut text = serde_json::to_string_pretty(v)
        .map_err(|e| be_err("vcmi_mod_json_serialize_failed", &format!("{}", e)))?;
    text.push('\n');
    fs::write(path, text).map_err(|e| be_err("infra_file_write_failed", &format!("{}", e)))
}

/// 单篇子模组：素材按 VCMI 命名空间分目录（官方 `createChronicleMod` + `extractFiles`）
///
/// `base_data`：基础游戏数据区（`<解包根>/Data`），提供 `bitmap.lod`（Tarnum）、
/// `lbitmap.lod`（开场图）与 `Hchron.vid`（合辑包的篇目片头）。
fn create_chronicle_mod(
    app: &Path,
    base_data: &Path,
    mod_dir: &Path,
    no: usize,
) -> Result<u64, String> {
    let dir_name = format!("chronicles_{}", no);
    // mod.json
    fs::create_dir_all(mod_dir).map_err(|e| be_err("cmd_mkdir_failed", &format!("{}", e)))?;
    // 命名逐字对齐官方 `createChronicleMod`：
    //   name        = "%1 - %2"（篇号 - 篇名）
    //   description = tr("Heroes Chronicles %1 - %2")
    // 并带上 VCMI 语言块（篇名前缀翻译），见 `vcmi::modmeta`。
    let mut meta = serde_json::Map::new();
    meta.insert("modType".to_string(), serde_json::json!("Expansion"));
    meta.extend(crate::vcmi::modmeta::chronicles_chapter_fields(
        no,
        CHRONICLES[no],
    ));
    write_mod_json(&mod_dir.join("mod.json"), &serde_json::Value::Object(meta))?;

    let content = mod_dir.join("content");
    let out_data = content.join("Data").join(&dir_name);
    let out_sprites = content.join("Sprites").join(&dir_name);
    let out_video = content.join("Video").join(&dir_name);
    let out_sounds = content.join("Sounds").join(&dir_name);
    let out_maps = content.join("Maps").join("Chronicles");
    for d in [&out_data, &out_sprites, &out_video, &out_sounds, &out_maps] {
        fs::create_dir_all(d).map_err(|e| be_err("cmd_mkdir_failed", &format!("{}", e)))?;
    }

    let mut files = 0u64;
    // 素材目录：`app/<篇名>/data`（GOG V2 安装包也可能把 data 放在篇名目录之外）
    let src_data = chronicle_data_dir(app, no);
    if let Some(dir) = src_data {
        files += extract_lod(&dir, "xBitmap.lod", &out_data, None);
        files += extract_lod(&dir, "xlBitmap.lod", &out_data, None);
        files += extract_lod(&dir, "xSprite.lod", &out_sprites, None);
        files += extract_lod(&dir, "xlSprite.lod", &out_sprites, None);
        files += extract_archive(&dir, "xVideo.vid", &out_video);
        files += extract_archive(&dir, "xSound.snd", &out_sounds);
        // "The World Tree" 特例：战役文件在篇目的 maps 目录而非 xlBitmap.lod 内。
        // 官方同样是**拷进 Data**（`chroniclesextractor.cpp:208-213`），随后统一步骤改名。
        if let Some(maps_dir) = dir.parent().and_then(|p| find_child_dir_ci(p, "maps")) {
            if let Ok(rd) = fs::read_dir(&maps_dir) {
                for e in rd.flatten() {
                    if e.path().is_file()
                        && fs::copy(e.path(), out_data.join(e.file_name())).is_ok()
                    {
                        files += 1;
                    }
                }
            }
        }
    }
    // 开场图：官方从**基础数据区**的 `lbitmap.lod` 只筛 INTRORIM 塞进本篇 Data（:219）
    files += extract_lod(base_data, "lbitmap.lod", &out_data, Some(&["INTRORIM"]));
    // 篇目片头：合辑安装包把它们收在基础数据区的 `Hchron.vid` 里（:221-238）
    files += copy_intro_video(base_data, no, &out_video);
    // 战役文件：必须从**解出的 Data** 取（`Main.h3c` 在 `xlBitmap.lod` 内，或上面 maps 目录拷来的），
    // 再改名为 `Hc<n>_Main.h3c` 放进 `Maps/Chronicles/`（:240-243）。
    // 曾经错误地在**源** data 目录找 Main.h3c，导致 8 篇里只有 World Tree 有战役文件。
    if let Some(main) = find_child_file_ci(&out_data, "Main.h3c") {
        let dst = out_maps.join(format!("Hc{}_Main.h3c", no));
        if fs::copy(&main, &dst).is_ok() {
            files += 1;
        }
    }
    Ok(files)
}

/// 合辑安装包的篇目片头：`Hchron.vid` 内的 `Intro*.bik/smk`。
///
/// 官方映射（`chroniclesextractor.cpp:224-237`）：`Intro→1`、`Intr2→2`、`Intr3→3`、
/// `Intr4→4`、`Intro5→7`、`Intro6→8`；7、8 篇落原名，其余统一落 `Intro`。
fn copy_intro_video(base_data: &Path, no: usize, out_video: &Path) -> u64 {
    let src_name = match no {
        1 => "Intro",
        2 => "Intr2",
        3 => "Intr3",
        4 => "Intr4",
        7 => "Intro5",
        8 => "Intro6",
        _ => return 0,
    };
    let Some(vid) = find_child_file_ci(base_data, "Hchron.vid") else {
        return 0;
    };
    let Some(entries) = crate::infra::archive::vid_entries(&vid) else {
        return 0;
    };
    let dst_name = if no == 7 || no == 8 {
        src_name
    } else {
        "Intro"
    };
    let want = src_name.to_ascii_lowercase();
    let mut n = 0u64;
    for e in entries {
        let lower = e.name.to_ascii_lowercase();
        let ext = if lower.ends_with(".bik") {
            ".bik"
        } else if lower.ends_with(".smk") {
            ".smk"
        } else {
            continue;
        };
        if lower != format!("{want}{ext}") {
            continue;
        }
        if let Some(bytes) = crate::infra::archive::read_entry(&vid, &e) {
            if fs::write(out_video.join(format!("{dst_name}{ext}")), bytes).is_ok() {
                n += 1;
            }
        }
    }
    n
}

/// 篇目的 data 目录（两种安装包布局都兼容，官方 `extractFiles` 开头的 cdUp 逻辑）
fn chronicle_data_dir(app: &Path, no: usize) -> Option<PathBuf> {
    let chron_dir = find_child_dir_ci(app, CHRONICLES[no])?;
    if let Some(d) = find_child_dir_ci(&chron_dir, "data") {
        return Some(d);
    }
    // GOG V2：data 与篇名目录同级
    find_child_dir_ci(app, "data")
}

/// 把 `lod` 的全部（或按名称子串过滤的）条目解到目标目录。
///
/// 过滤口径对齐官方 `extract()` 的 `find(item) != npos`，即**子串包含**而非前缀。
fn extract_lod(dir: &Path, lod_name: &str, out: &Path, only: Option<&[&str]>) -> u64 {
    let Some(lod_path) = find_child_file_ci(dir, lod_name) else {
        return 0;
    };
    let Some(arc) = LodArchive::open(&lod_path) else {
        return 0;
    };
    // 保留条目原始大小写：官方 `extractToFolder` 直接用条目名落盘（如 `CamBkHc.pcx`）
    let Some(names) = crate::infra::lod::lod_entry_names_raw(&lod_path) else {
        return 0;
    };
    let mut n = 0u64;
    for name in names {
        if let Some(keys) = only {
            let up = name.to_uppercase();
            if !keys.iter().any(|k| up.contains(&k.to_uppercase())) {
                continue;
            }
        }
        let Some(bytes) = arc.read(&name) else {
            continue;
        };
        let dst = out.join(&name);
        if fs::write(&dst, &bytes).is_ok() {
            n += 1;
        }
    }
    n
}

/// 把 `.vid` / `.snd` 归档的条目解到目标目录。
fn extract_archive(dir: &Path, name: &str, out: &Path) -> u64 {
    let Some(p) = find_child_file_ci(dir, name) else {
        return 0;
    };
    let entries = if name.to_lowercase().ends_with(".vid") {
        crate::infra::archive::vid_entries(&p)
    } else {
        crate::infra::archive::snd_entries(&p)
    };
    let Some(entries) = entries else { return 0 };
    let mut n = 0u64;
    for e in entries {
        if let Some(bytes) = crate::infra::archive::read_entry(&p, &e) {
            if fs::write(out.join(&e.name), &bytes).is_ok() {
                n += 1;
            }
        }
    }
    n
}

/// Tarnum 头像：从**基础游戏**的 `bitmap.lod` 取（官方同款写死前缀）
fn extract_tarnum_portraits(base_data: &Path, base_mod: &Path, overwrite: bool) -> u64 {
    let out = base_mod.join("content").join("Data");
    if let Err(e) = fs::create_dir_all(&out) {
        let _ = e;
        return 0;
    }
    if out.join("HPS137.pcx").exists() && !overwrite {
        return 0;
    }
    extract_lod(base_data, "bitmap.lod", &out, Some(&TARNUM_PORTRAITS))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 写"最小合法 lod"：目录表 + 条目数据（offset/fullSize 指向尾部数据区）。
    fn write_lod(path: &Path, entries: &[(&str, &[u8])]) {
        let mut buf = vec![0u8; 0x5c];
        buf[8..12].copy_from_slice(&(entries.len() as u32).to_le_bytes());
        let table_size = entries.len() * 32;
        for (name, _) in entries {
            let mut e = [0u8; 32];
            let b = name.as_bytes();
            let len = b.len().min(16);
            e[..len].copy_from_slice(&b[..len]);
            buf.extend_from_slice(&e);
        }
        let mut data_start = 0x5c + table_size;
        for (name, data) in entries {
            // 定位刚写入的目录项，回填 offset / fullSize（未压缩直存）
            let idx = entries.iter().position(|(n, _)| n == name).unwrap();
            let pos = 0x5c + idx * 32;
            buf[pos + 16..pos + 20].copy_from_slice(&(data_start as u32).to_le_bytes());
            buf[pos + 20..pos + 24].copy_from_slice(&(data.len() as u32).to_le_bytes());
            buf[pos + 28..pos + 32].copy_from_slice(&(data.len() as u32).to_le_bytes());
            buf.extend_from_slice(data);
            data_start += data.len();
        }
        fs::write(path, &buf).expect("write lod");
    }

    /// 官方产物结构回归：基础模组 + 单篇子模组 + 战役改名 + Tarnum 头像。
    #[test]
    fn install_builds_official_mod_layout() {
        let d = std::env::temp_dir().join("vcmi-fv-chronicles-test");
        let _ = fs::remove_dir_all(&d);
        let extracted = d.join("extracted");
        let chron_dir = extracted.join("app").join(CHRONICLES[1]).join("data");
        let base_data = extracted.join("app").join("data");
        fs::create_dir_all(&chron_dir).unwrap();
        fs::create_dir_all(&base_data).unwrap();
        // 历代记自己的素材：战役文件在 xlBitmap.lod 内（真实 GOG 包就是这样），
        // 而不是散落在 data 目录里
        // 条目名故意用混合大小写：落盘必须保留目录表原样（与官方 extractToFolder 一致）
        write_lod(&chron_dir.join("xBitmap.lod"), &[("CamBkHc.pcx", b"menu")]);
        write_lod(
            &chron_dir.join("xlBitmap.lod"),
            &[("Main.h3c", b"campaign"), ("Tutorial.h3c", b"tut")],
        );
        write_lod(&chron_dir.join("xSprite.lod"), &[("CHRSPR.DEF", b"spr")]);
        // 基础游戏数据区：bitmap.lod（Tarnum 头像，含一个不该被抽的普通条目）+ lbitmap.lod（开场图）
        write_lod(
            &base_data.join("bitmap.lod"),
            &[("HPS137.PCX", b"tarnum"), ("OTHER.PCX", b"nope")],
        );
        write_lod(
            &base_data.join("lbitmap.lod"),
            &[("INTRORIM.PCX", b"rim"), ("OTHER2.PCX", b"nope")],
        );

        let mods = d.join("Mods");
        fs::create_dir_all(&mods).unwrap();
        let out = install(&extracted, &mods, &d.join("game"), true).expect("install");

        // 基础模组：mod.json 声明战役地图格式 + VCMI 语言块
        let base_json = fs::read_to_string(out.base_mod.join("mod.json")).unwrap();
        assert!(base_json.contains("mapFormat"), "基础模组应声明 mapFormat");
        assert!(
            base_json.contains("portraitTarnumBarbarian"),
            "应带 Tarnum 头像表"
        );
        let base_parsed: serde_json::Value = serde_json::from_str(&base_json).unwrap();
        assert_eq!(base_parsed["chinese"]["name"], "英雄无敌历代记");
        assert_eq!(base_parsed["chinese"]["description"], "英雄无敌历代记");
        // Tarnum 头像进了基础模组的 content/Data，且只抽指定前缀
        assert!(out
            .base_mod
            .join("content")
            .join("Data")
            .join("HPS137.PCX")
            .is_file());
        assert!(
            !out.base_mod
                .join("content")
                .join("Data")
                .join("OTHER.PCX")
                .exists(),
            "只抽 Tarnum 前缀，其它条目不应混入"
        );
        // 单篇子模组：素材按命名空间分目录
        let sub = out.base_mod.join("Mods").join("chronicles_1");
        let sub_json = fs::read_to_string(sub.join("mod.json")).unwrap();
        // 官方 createChronicleMod：name = "<no> - <篇名>"，description = "Heroes Chronicles <no> - <篇名>"
        assert!(
            sub_json.contains(r#""name": "1 - Warlords of the Wasteland""#),
            "{}",
            sub_json
        );
        assert!(
            sub_json
                .contains(r#""description": "Heroes Chronicles 1 - Warlords of the Wasteland""#),
            "{}",
            sub_json
        );
        // 语言块：只覆盖 description（篇名保留英文），中文前缀按官方译名展开
        let sub_parsed: serde_json::Value = serde_json::from_str(&sub_json).unwrap();
        assert_eq!(
            sub_parsed["chinese"]["description"],
            "英雄无敌历代记 1 - Warlords of the Wasteland"
        );
        assert!(
            sub_parsed["chinese"].get("name").is_none(),
            "篇名不翻译，语言块不应覆盖 name"
        );
        let sub_data = sub.join("content").join("Data").join("chronicles_1");
        // Windows 文件系统大小写不敏感，必须比对**目录项字符串**才能确认没有强制大写
        let entries: Vec<String> = fs::read_dir(&sub_data)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert!(
            entries.contains(&"CamBkHc.pcx".to_string()),
            "条目名应保留原始大小写（官方同款），实际 {:?}",
            entries
        );
        assert!(
            sub_data.join("Main.h3c").is_file(),
            "xlBitmap.lod 条目应解到 Data"
        );
        assert!(
            sub_data.join("INTRORIM.PCX").is_file(),
            "开场图应来自基础 lbitmap.lod"
        );
        assert!(
            !sub_data.join("OTHER2.PCX").exists(),
            "lbitmap.lod 只抽 INTRORIM，其它条目不应混入"
        );
        assert!(sub
            .join("content")
            .join("Sprites")
            .join("chronicles_1")
            .join("CHRSPR.DEF")
            .is_file());
        // 战役文件：从解出的 Data 改名落位
        assert!(
            sub.join("content")
                .join("Maps")
                .join("Chronicles")
                .join("Hc1_Main.h3c")
                .is_file(),
            "Data 里的 Main.h3c 应改名为 Hc1_Main.h3c"
        );
        assert_eq!(out.chronicles, vec!["chronicles_1".to_string()]);
        let _ = fs::remove_dir_all(&d);
    }

    /// "The World Tree" 布局：战役文件在 `<篇名>/Maps/Main.h3c` 而不是 lod 内。
    /// 官方把它先拷进 Data 再改名（曾经直接拷进 Maps/Chronicles 留下未改名的 `Main.h3c`）。
    #[test]
    fn install_takes_campaign_from_maps_folder() {
        let d = std::env::temp_dir().join("vcmi-fv-chronicles-maps-test");
        let _ = fs::remove_dir_all(&d);
        let extracted = d.join("extracted");
        let chron_dir = extracted.join("app").join(CHRONICLES[5]);
        fs::create_dir_all(chron_dir.join("data")).unwrap();
        fs::create_dir_all(chron_dir.join("Maps")).unwrap();
        fs::write(chron_dir.join("Maps").join("Main.h3c"), b"worldtree").unwrap();

        let mods = d.join("Mods");
        fs::create_dir_all(&mods).unwrap();
        let out = install(&extracted, &mods, &d.join("game"), true).expect("install");

        let maps = out
            .base_mod
            .join("Mods")
            .join("chronicles_5")
            .join("content")
            .join("Maps")
            .join("Chronicles");
        assert!(
            maps.join("Hc5_Main.h3c").is_file(),
            "maps 目录里的战役文件也应改名"
        );
        assert!(!maps.join("Main.h3c").exists(), "不应残留未改名的 Main.h3c");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn install_rejects_dir_without_chronicles() {
        let d = std::env::temp_dir().join("vcmi-fv-chronicles-empty-test");
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(d.join("app")).unwrap();
        let err = install(&d, &d.join("Mods"), &d.join("game"), true);
        assert!(err.is_err(), "没有篇目目录应报错而不是生成空模组");
        let _ = fs::remove_dir_all(&d);
    }
}

fn find_child_dir_ci(parent: &Path, name: &str) -> Option<PathBuf> {
    let rd = fs::read_dir(parent).ok()?;
    for e in rd.flatten() {
        if e.file_name().to_string_lossy().eq_ignore_ascii_case(name) && e.path().is_dir() {
            return Some(e.path());
        }
    }
    None
}

fn find_child_file_ci(parent: &Path, name: &str) -> Option<PathBuf> {
    let rd = fs::read_dir(parent).ok()?;
    for e in rd.flatten() {
        if e.file_name().to_string_lossy().eq_ignore_ascii_case(name) && e.path().is_file() {
            return Some(e.path());
        }
    }
    None
}
