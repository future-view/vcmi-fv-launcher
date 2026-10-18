//! 原版《英雄无敌 III》数据文件：检测 / 定位 / 补齐（首启向导第 3 步的硬闸门）。
//!
//! # 口径必须贴合官方（否则"向导说能跑、一启动就崩"）
//!
//! 官方**首启**检测（`references/vcmi/launcher/firstLaunch/firstlaunch_moc.cpp:331-335`）：
//! ```text
//! ROE  = DATA/GENRLTXT.TXT 必须存在
//! SOD  = DATA/TENTCOLR.TXT；缺 SOD 时允许 demo（MAPS/H3DEMO.H3M）
//! 通过 = ROE && (SOD || demo)
//! ```
//! 官方**运行期**硬前置（`references/vcmi/clientapp/EntryPoint.cpp:294-303`）：
//! `DATA/HELP.TXT`、`MODS/VCMI/MOD.JSON`、`DATA/NOTOSERIF-MEDIUM.TTF`、
//! `DATA/PLAYERS.PAL`、`SPRITES/DEFAULT.DEF`；且 **ROE-only 数据（含 HD 版）直接致命**
//! （`GameDataMode::ROE` → `handleFatalError`）。
//!
//! 本模块把两者合并成向导闸门：`found = ROE && (SOD || demo) && 运行期 fatal 项齐全` ——
//! 比官方首启更严一点，换取"初始化通过 ⇒ 游戏一定能跑"，不在启动时才炸。
//!
//! # 资源解析必须按挂载表，不能只判文件存在
//!
//! `references/vcmi/config/filesystem.json`：`DATA/` 由 `H3ab_bmp.lod` / `h3abp_bm.lod` /
//! `H3bitmap.lod` / `H3pbitma.lod` + 散目录 `Data` 合成；`SPRITES/` 由 `H3ab_spr.lod` /
//! `h3abp_sp.lod` / `H3sprite.lod` + 散目录 `Sprites` 合成；`MAPS/` 只有散目录 `Maps`。
//! `CArchiveLoader` 把 LOD 条目挂成 `mountPoint + 条目名`（`CArchiveLoader.cpp:80`），
//! 故"文件在盘"≠"资源可达"（LOD 内条目才算），判定走 LOD 条目名（`infra::lod::lod_entry_names`）。
//! 全部大小写不敏感（官方 filesystem.json 头注即声明 case-insensitive）。

use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::infra::berror::be_err;
use crate::infra::lod::lod_entry_names;

/// 挂到 `DATA/` 的 LOD 名（`config/filesystem.json:10-17`，大小写不敏感）。
const DATA_LODS: [&str; 4] = ["H3ab_bmp.lod", "h3abp_bm.lod", "H3bitmap.lod", "H3pbitma.lod"];
/// 挂到 `SPRITES/` 的 LOD 名（`config/filesystem.json:18-25`）。
const SPRITES_LODS: [&str; 4] = ["H3ab_spr.lod", "h3abp_sp.lod", "H3sprite.lod", "H3psprit.lod"];

/// 数据状态（首启向导第 3 步的唯一判据；同时进体检响应供主界面复用）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct H3DataStatus {
    /// 向导闸门：`ROE && (SOD || demo) && 运行期 fatal 项齐全`。
    pub found: bool,
    /// `DATA/GENRLTXT.TXT`（恢复帝国之战标记）
    pub roe: bool,
    /// `DATA/TENTCOLR.TXT`（末日之刃 / 历代记标记）
    pub sod: bool,
    /// `MAPS/H3DEMO.H3M`（demo 数据）
    pub demo: bool,
    /// 运行期 fatal：`DATA/HELP.TXT`
    pub help_txt: bool,
    /// 运行期 fatal：`DATA/PLAYERS.PAL`
    pub players_pal: bool,
    /// 运行期 fatal：`SPRITES/DEFAULT.DEF`
    pub default_def: bool,
    /// 命中根含**游戏目录**
    pub in_game: bool,
    /// 命中根含**用户数据目录**
    pub in_user: bool,
    /// 用户可操作的缺失文件（`H3Bitmap.lod` / `H3Sprite.lod`），供界面列出。
    pub missing: Vec<String>,
}

/// 单个资源根的挂载索引（一次扫描，多次查询，避免每个资源都重读 LOD 头）。
struct RootIndex {
    kind: &'static str,
    data: Vec<String>,
    sprites: Vec<String>,
    maps: Vec<String>,
    /// **该根本身**能否解析出 `GENRLTXT.TXT`——"数据在这个根"的准确判据。
    /// ⚠️ 不能用全局 roe 充当：全局标记会让 in_game / in_user 同时为真，
    /// 状态行"已就绪（游戏目录|用户数据目录）"随之失真（用户实测报过）。
    has_roe: bool,
}

/// 大小写不敏感查找单层目录项。
fn find_child_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    let rd = fs::read_dir(dir).ok()?;
    for e in rd.flatten() {
        if e.file_name().to_string_lossy().eq_ignore_ascii_case(name) {
            return Some(e.path());
        }
    }
    None
}

/// 目录下所有匹配（大小写不敏感）后缀的文件。
fn children_with_ext_ci(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            let n = e.file_name().to_string_lossy().to_string();
            if n.len() >= ext.len() && n[n.len() - ext.len()..].eq_ignore_ascii_case(ext) {
                if e.path().is_file() {
                    out.push(e.path());
                }
            }
        }
    }
    out
}

/// 某挂载点的资源名集合：**散文件**（`<root>/<loose_subdir>`）+ **挂载表内 LOD 的条目名**。
///
/// ⚠️ LOD 一律从 `<root>/Data/` 读——`config/filesystem.json` 里 `DATA/` 与 `SPRITES/`
/// 两个挂载点的 lod 都指向 `Data/` 下的文件（`Data/H3sprite.lod` 挂的是 `SPRITES/`），
/// 只有散文件目录不同（`Data` vs `Sprites`）。此前按挂载点找 LOD 会把 sprite 包漏掉。
fn mounted_names(root: &Path, loose_subdir: &str, lods: &[&str]) -> Vec<String> {
    let mut names: Vec<String> = Vec::new();
    let dir = root.join(loose_subdir);
    if dir.is_dir() {
        if let Ok(rd) = fs::read_dir(&dir) {
            for e in rd.flatten() {
                if e.path().is_file() {
                    names.push(e.file_name().to_string_lossy().to_string());
                }
            }
        }
    }
    for lod in children_with_ext_ci(&root.join("Data"), ".lod") {
        let fname = lod
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        if !lods.iter().any(|l| fname.eq_ignore_ascii_case(l)) {
            continue; // 不在挂载表内的 lod 不进虚拟命名空间
        }
        if let Some(entries) = lod_entry_names(&lod) {
            names.extend(entries);
        }
    }
    names
}

fn index_root(root: &Path, kind: &'static str) -> RootIndex {
    let data = mounted_names(root, "Data", &DATA_LODS);
    let sprites = mounted_names(root, "Sprites", &SPRITES_LODS);
    let maps = mounted_names(root, "Maps", &[]);
    let has_roe = data.iter().any(|n| n.eq_ignore_ascii_case("GENRLTXT.TXT"));
    RootIndex {
        kind,
        has_roe,
        data,
        sprites,
        maps,
    }
}

/// 检测两个资源根（用户数据目录优先列，与 VCMI「后者覆盖」无关——这里只看存在性）。
pub fn detect(game_dir: &Path, user_data_dir: &Path) -> H3DataStatus {
    let mut roots: Vec<RootIndex> = Vec::new();
    if !user_data_dir.as_os_str().is_empty() {
        roots.push(index_root(user_data_dir, "user"));
    }
    if !game_dir.as_os_str().is_empty() {
        roots.push(index_root(game_dir, "game"));
    }
    let has = |names: &dyn Fn(&RootIndex) -> &Vec<String>, name: &str| {
        roots
            .iter()
            .any(|r| names(r).iter().any(|n| n.eq_ignore_ascii_case(name)))
    };

    let roe = has(&|r| &r.data, "GENRLTXT.TXT");
    let sod = has(&|r| &r.data, "TENTCOLR.TXT");
    let demo = !sod && has(&|r| &r.maps, "H3DEMO.H3M");
    let help_txt = has(&|r| &r.data, "HELP.TXT");
    let players_pal = has(&|r| &r.data, "PLAYERS.PAL");
    let default_def = has(&|r| &r.sprites, "DEFAULT.DEF");

    // 缺失映射到"用户能操作的文件"：bitmap 包缺 → GENRLTXT / HELP / PLAYERS 全无源
    let mut missing: Vec<String> = Vec::new();
    if !roe || !help_txt || !players_pal {
        missing.push("H3Bitmap.lod".to_string());
    }
    if !default_def {
        missing.push("H3Sprite.lod".to_string());
    }

    // 命中根按**每根自己的 GENRLTXT** 判定（全局 roe 会让两者同时为真，状态行失真）
    let in_game = roots.iter().any(|r| r.kind == "game" && r.has_roe);
    let in_user = roots.iter().any(|r| r.kind == "user" && r.has_roe);

    H3DataStatus {
        found: roe && (sod || demo) && help_txt && players_pal && default_def,
        roe,
        sod,
        demo,
        help_txt,
        players_pal,
        default_def,
        in_game,
        in_user,
        missing,
    }
}

/// 识别「数据目录」来源：返回 `(源根目录, Data 目录)`。
///
/// 用户可能选三种东西：游戏根目录（含 `Data`）、`Data` 目录本身、或一个散装 `.lod` 的目录。
/// 源根用于定位 `Mp3` / `Maps`（它们是 `Data` 的兄弟目录）。
pub fn resolve_source(picked: &Path) -> Option<(PathBuf, PathBuf)> {
    if let Some(data) = find_child_ci(picked, "Data") {
        if data.is_dir() {
            return Some((picked.to_path_buf(), data));
        }
    }
    if children_with_ext_ci(picked, ".lod").is_empty() {
        return None;
    }
    // 选的就是 Data 目录本身：源根取其父目录（Mp3/Maps 的兄弟位置）
    let is_data_dir = picked
        .file_name()
        .map(|n| n.eq_ignore_ascii_case("Data"))
        .unwrap_or(false);
    if is_data_dir {
        let parent = picked.parent()?;
        Some((parent.to_path_buf(), picked.to_path_buf()))
    } else {
        Some((picked.to_path_buf(), picked.to_path_buf()))
    }
}

/// 搬进 `Data/` 的文件扩展名（官方 `Helper::classifyTargetByExt` 的 Data 分支）。
///
/// ⚠️ `.snd` / `.vid` **必须搬**：VCMI 启动时会检查视频（`VIDEO.VID` / `Heroes3.vid`），
/// 缺了会弹「未检测到视频文件」并提示用户手动复制（`clientapp` 的 video check）。
/// 早先只搬 `.lod`，用户实测在 `Heroes3-VCMI` 上必现该告警。
/// 官方还会搬 `.pak`（HD 版素材）；VCMI 明确拒绝 HD 版数据，故这里不搬。
const DATA_EXTS: [&str; 3] = [".lod", ".snd", ".vid"];

/// 复制源 `Data` 目录下的全部 `.lod` / `.snd` / `.vid` 到目标 `Data` 目录（**不覆盖**已存在文件）。
///
/// 口径对齐官方 `FirstLaunchView::performCopyFlow` + `Helper::classifyTargetByExt`
/// （官方还会把 `.pak` 归到 Data，但 VCMI 拒绝 HD 版，故不纳入）。
/// 先预扫总体量，使进度条**首帧即有分母**（含 490 MB 的 `Heroes3.vid`）。
/// 回调 `(已完成文件数, 已复制字节, 总文件数, 总字节)`。
pub fn copy_data_files(
    from_data_dir: &Path,
    to_data_dir: &Path,
    mut on_progress: impl FnMut(u64, u64, u64, u64),
) -> Result<(u64, u64), String> {
    let mut picked: Vec<PathBuf> = Vec::new();
    for ext in DATA_EXTS {
        picked.extend(children_with_ext_ci(from_data_dir, ext));
    }
    if picked.is_empty() {
        return Ok((0, 0));
    }
    let total_files = picked.len() as u64;
    let total_bytes: u64 = picked
        .iter()
        .map(|p| fs::metadata(p).map(|m| m.len()).unwrap_or(0))
        .sum();
    fs::create_dir_all(to_data_dir).map_err(|e| be_err("cmd_mkdir_failed", &format!("{}", e)))?;
    let mut files = 0u64;
    let mut bytes = 0u64;
    for src in &picked {
        let name = src
            .file_name()
            .ok_or_else(|| be_err("cmd_bad_path", &src.to_string_lossy()))?;
        let dst = to_data_dir.join(name);
        if dst.exists() {
            files += 1; // 已存在：计入进度但不复制（不覆盖用户数据）
            on_progress(files, bytes, total_files, total_bytes);
            continue;
        }
        fs::copy(src, &dst).map_err(|e| be_err("infra_file_copy_failed", &format!("{}", e)))?;
        let size = fs::metadata(&dst).map(|m| m.len()).unwrap_or(0);
        files += 1;
        bytes += size;
        on_progress(files, bytes, total_files, total_bytes);
    }
    Ok((files, bytes))
}

/// 复制 `Mp3` / `Maps` 目录到用户数据目录（官方首启 `copyHeroesData` 同口径：
/// 把 Maps/Data/Mp3 一并搬进 userDataPath）。
///
/// - `Mp3`：音轨（warn 级，缺了没音乐但不影响启动）；
/// - `Maps`：用户地图目录。**VCMI 不会自建 `Mods` / `Maps`**（只自建
///   `userDataPath/cache/config/logs/Saves`），故无论是否复制都要保证目录存在；
/// - 只复制**文件**（不递归子目录），已存在则跳过，不覆盖。
pub fn copy_extra(
    from_root: &Path,
    to_user_dir: &Path,
    mp3: bool,
    maps: bool,
    mut on_progress: impl FnMut(u64, u64, u64, u64),
) -> Result<(u64, u64), String> {
    let mut files = 0u64;
    let mut bytes = 0u64;
    let jobs: [(&str, bool); 2] = [("Mp3", mp3), ("Maps", maps)];
    for (name, enabled) in jobs {
        let dst = to_user_dir.join(name);
        // 目录始终创建：VCMI 不自建 Mods/Maps，缺目录会影响地图扫描
        fs::create_dir_all(&dst).map_err(|e| be_err("cmd_mkdir_failed", &format!("{}", e)))?;
        if !enabled {
            continue;
        }
        let src = from_root.join(name);
        if !src.is_dir() {
            continue;
        }
        let entries: Vec<PathBuf> = match fs::read_dir(&src) {
            Ok(rd) => rd.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect(),
            Err(_) => continue,
        };
        let total_files = entries.len() as u64;
        let total_bytes: u64 = entries
            .iter()
            .map(|p| fs::metadata(p).map(|m| m.len()).unwrap_or(0))
            .sum();
        for f in &entries {
            let Some(fname) = f.file_name() else { continue };
            let target = dst.join(fname);
            if target.exists() {
                files += 1;
                on_progress(files, bytes, total_files, total_bytes);
                continue;
            }
            fs::copy(f, &target)
                .map_err(|e| be_err("infra_file_copy_failed", &format!("{}", e)))?;
            files += 1;
            bytes += fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
            on_progress(files, bytes, total_files, total_bytes);
        }
    }
    Ok((files, bytes))
}

/// 本机已安装的原版游戏目录候选（Windows 注册表，对齐官方
/// `FirstLaunchView::getHeroesInstallDir`，`firstlaunch_moc.cpp:340-345`）。
///
/// 非 Windows 或无命中返回空。注册表读取走 `reg query`（不引入 `winreg` 依赖）。
pub fn locate_installs() -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        const KEYS: [(&str, &str); 4] = [
            (r"HKEY_LOCAL_MACHINE\SOFTWARE\GOG.com\Games\1207658787", "path"),
            (
                r"HKEY_LOCAL_MACHINE\SOFTWARE\WOW6432Node\GOG.com\Games\1207658787",
                "path",
            ),
            (
                r"HKEY_LOCAL_MACHINE\SOFTWARE\New World Computing\Heroes of Might and Magic® III\1.0",
                "AppPath",
            ),
            (
                r"HKEY_LOCAL_MACHINE\SOFTWARE\WOW6432Node\New World Computing\Heroes of Might and Magic® III\1.0",
                "AppPath",
            ),
        ];
        let mut out: Vec<PathBuf> = Vec::new();
        for (key, value) in KEYS {
            // reg query <key> /v <value>：输出行形如 "    path    REG_SZ    D:\Games\Heroes3"
            let Ok(res) = std::process::Command::new("reg")
                .args(["query", key, "/v", value])
                .output()
            else {
                continue;
            };
            if !res.status.success() {
                continue;
            }
            let text = String::from_utf8_lossy(&res.stdout);
            for line in text.lines() {
                let mut it = line.split_whitespace();
                let _key = it.next();
                let Some(val_name) = it.next() else { continue };
                if !val_name.eq_ignore_ascii_case(value) {
                    continue;
                }
                let _type = it.next();
                let Some(path) = it.next() else { continue };
                // reg 对含空格路径不会加引号；类型字段之后的所有内容即路径
                let rest: Vec<&str> = line.splitn(4, ' ').collect();
                let p = if rest.len() == 4 { rest[3].trim() } else { path };
                if p.is_empty() {
                    continue;
                }
                let pb = PathBuf::from(p);
                if pb.is_dir() && !out.iter().any(|x| x == &pb) {
                    out.push(pb);
                }
            }
        }
        out
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 仓库随携的原版安装 fixture（`references/Heroes3-EN`：H3bitmap.lod / H3sprite.lod / AB）。
    fn reference_install() -> Option<PathBuf> {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../references/Heroes3-EN");
        base.join("Data").is_dir().then_some(base)
    }

    /// 官方口径在**真实原版安装**上必须全绿：ROE + SOD + 运行期 fatal 三项。
    /// 这条同时验证「资源在 LOD 内而不在散文件」的挂载解析是否正确。
    #[test]
    fn detects_reference_install() {
        let Some(base) = reference_install() else {
            eprintln!("跳过：仓库未携带 references/Heroes3-EN");
            return;
        };
        let st = detect(&PathBuf::new(), &base);
        assert!(st.roe, "DATA/GENRLTXT.TXT 应在 LOD 内可解析");
        assert!(st.sod, "DATA/TENTCOLR.TXT 应在 LOD 内可解析");
        assert!(st.help_txt, "DATA/HELP.TXT（EntryPoint fatal）");
        assert!(st.players_pal, "DATA/PLAYERS.PAL（EntryPoint fatal）");
        assert!(st.default_def, "SPRITES/DEFAULT.DEF（EntryPoint fatal）");
        assert!(st.found, "完整原版安装应通过闸门");
        assert!(st.in_user && !st.in_game, "数据在用户目录根");
        assert!(st.missing.is_empty());
    }

    /// 空目录必须判缺，且缺失清单映射到用户可操作的文件名。
    #[test]
    fn missing_on_empty_dirs() {
        let base = std::env::temp_dir().join("vcmi-fv-h3data-empty-test");
        let _ = fs::remove_dir_all(&base);
        let game = base.join("game");
        let user = base.join("user");
        fs::create_dir_all(game.join("Data")).expect("mkdir");
        fs::create_dir_all(&user).expect("mkdir");
        let st = detect(&game, &user);
        assert!(!st.found);
        assert_eq!(st.missing, vec!["H3Bitmap.lod", "H3Sprite.lod"]);
        let _ = fs::remove_dir_all(&base);
    }

    /// `resolve_source`：既接受游戏根目录（含 `Data`），也接受 `Data` 本身，并给出源根
    /// （供定位兄弟目录 Mp3 / Maps）。
    #[test]
    fn resolve_source_both_forms() {
        let base = std::env::temp_dir().join("vcmi-fv-h3data-resolve-test");
        let _ = fs::remove_dir_all(&base);
        let root = base.join("Heroes3");
        let data = root.join("Data");
        fs::create_dir_all(&data).expect("mkdir");
        fs::write(data.join("H3Bitmap.lod"), b"x").expect("write");
        assert_eq!(
            resolve_source(&root),
            Some((root.clone(), data.clone())),
            "游戏根目录应解析到其 Data 子目录"
        );
        assert_eq!(
            resolve_source(&data),
            Some((root.clone(), data.clone())),
            "Data 目录本身应被接受，源根取其父目录"
        );
        assert_eq!(resolve_source(&base), None, "无关目录应拒绝");
        let _ = fs::remove_dir_all(&base);
    }

    /// `copy_extra`：Mp3 / Maps 复制 + **目录始终创建**（VCMI 不自建 Maps）。
    #[test]
    fn copy_extra_creates_dirs_even_when_disabled() {
        let base = std::env::temp_dir().join("vcmi-fv-h3data-extra-test");
        let _ = fs::remove_dir_all(&base);
        let src_root = base.join("src");
        let dst_user = base.join("user");
        fs::create_dir_all(src_root.join("Mp3")).expect("mkdir");
        fs::create_dir_all(src_root.join("Maps")).expect("mkdir");
        fs::write(src_root.join("Mp3").join("a.mp3"), b"123").expect("write");
        fs::write(src_root.join("Maps").join("m.h3m"), b"1234").expect("write");

        // 都不复制：目录仍必须建（VCMI 只自建 cache/config/logs/Saves）
        let (f, b) = copy_extra(&src_root, &dst_user, false, false, |_, _, _, _| {}).expect("copy");
        assert_eq!((f, b), (0, 0));
        assert!(dst_user.join("Mp3").is_dir(), "Mp3 目录应被创建");
        assert!(dst_user.join("Maps").is_dir(), "Maps 目录应被创建");
        assert!(!dst_user.join("Mp3").join("a.mp3").exists(), "未勾选不应复制文件");

        // 勾选 Maps：只搬 Maps
        let (f2, b2) = copy_extra(&src_root, &dst_user, false, true, |_, _, _, _| {}).expect("copy2");
        assert_eq!((f2, b2), (1, 4));
        assert!(dst_user.join("Maps").join("m.h3m").is_file());
        assert!(!dst_user.join("Mp3").join("a.mp3").exists(), "Mp3 未勾选仍不复制");
        let _ = fs::remove_dir_all(&base);
    }

    /// 复制 `.lod` / `.snd` / `.vid`（含 `VIDEO.VID`）且不覆盖已存在文件；
    /// 其它类型（`.bmp`）不搬。
    #[test]
    fn copy_data_files_includes_video_and_sound() {
        let base = std::env::temp_dir().join("vcmi-fv-h3data-copy-test");
        let _ = fs::remove_dir_all(&base);
        let src = base.join("src");
        let dst = base.join("dst");
        fs::create_dir_all(&src).expect("mkdir");
        fs::write(src.join("H3Bitmap.lod"), b"12345").expect("write");
        fs::write(src.join("H3Sprite.lod"), b"123").expect("write");
        fs::write(src.join("Heroes3.snd"), b"sound").expect("write");
        fs::write(src.join("Heroes3.vid"), b"video").expect("write");
        fs::write(src.join("VIDEO.VID"), b"vid2").expect("write");
        fs::write(src.join("mumaz.bmp"), b"bmp").expect("write"); // 非数据文件：不搬
        let (files, bytes) = copy_data_files(&src, &dst, |_, _, _, _| {}).expect("copy");
        assert_eq!(files, 5);
        assert_eq!(bytes, 5 + 3 + 5 + 5 + 4);
        assert!(dst.join("H3Bitmap.lod").is_file());
        assert!(dst.join("Heroes3.snd").is_file(), ".snd 应被复制（音效）");
        assert!(dst.join("Heroes3.vid").is_file(), ".vid 应被复制（过场动画）");
        assert!(dst.join("VIDEO.VID").is_file(), "VIDEO.VID 是 VCMI 启动检查项");
        assert!(!dst.join("mumaz.bmp").exists(), ".bmp 不在复制范围");
        // 第二次复制：目标已存在 ⇒ 0 字节
        let (files2, bytes2) = copy_data_files(&src, &dst, |_, _, _, _| {}).expect("copy2");
        assert_eq!((files2, bytes2), (5, 0), "已存在应跳过且不覆盖");
        let _ = fs::remove_dir_all(&base);
    }
}
