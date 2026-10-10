//! 游戏文件导入（底栏「导入游戏文件」弹层）：分类 / 预检 / 执行 / HD 素材生成。
//!
//! 对齐官方 `CModListView::installFiles`（`references/vcmi/launcher/modManager/cmodlistview_moc.cpp:1067`）
//! 与 `HdExtractor`（`hdextractor.cpp:36-101`）：
//!
//! | 类型 | 输入 | 目标 |
//! |---|---|---|
//! | 地图 / 战役 | `.h3m .vmap .h3c .vcmp` | `<userDataDir>/Maps/`（zip 内**保相对结构**） |
//! | 存档 | `.vsgm1` | `<userDataDir>/Saves/`（同理保结构） |
//! | 模组 | `.zip`（含 `mod.json`） | `<userDataDir>/Mods/<modId>`（装完由前端通知重扫 + 自动启用） |
//! | 配置 | `.json` | 用户配置目录（同名走冲突策略；重启启动器生效） |
//! | 压缩包 | `.zip` 自动分类 | `mod.json`→模组；含地图→地图包；含 `.vsgm1`→存档包；否则按模组尝试 |
//! | 历代记 | `.exe` | 本期仅**识别**（Inno Setup 特征串）+ 引导，解包二期 |
//! | HD 素材 | 目录（含 `HOMM3 2.0.exe`）**或该 exe 本体** | 生成 `<userDataDir>/Mods/hd-edition`（对齐官方产物结构） |
//! | 普通目录 | 递归收集受支持文件（`.h3m .vmap .h3c .vcmp .vsgm1 .json .zip`） | 按类型分流，**保留相对子目录结构**（不支持项计入忽略） |
//!
//! zip 分类复刻官方 `classifyZipArchive`（`:1036-1064`）：**mod.json > 地图 > 存档 > 兜底模组**；
//! 前端的 `overrides`（`path → mod|maps|saves`）可逐项改判。
//!
//! **有意不复制官方 bug**：`hdextractor.cpp:96-98` 对同一目标双 `QFile::copy`（第二次必失败弹
//! 错误框）；本实现逐文件只拷一次，源 pak 缺失时**容忍并计数**（预检显示 n/8，缺失项列出）。
//!
//! 依赖方向：`vcmi` 层只依赖 `infra`（复用 `infra::h3m` 元数据、`infra::links` 联接保护），
//! 无 tauri 类型，可单测。

use crate::infra::berror::be_err;
use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::infra::h3m;
use crate::infra::links::is_link;
use crate::vcmi::mod_install;
use crate::vcmi::paths::VcmiPaths;

/// 地图 / 战役扩展名（官方白名单口径：全部落 `Maps/`）。
const MAP_SUFFIXES: [&str; 4] = [".h3m", ".vmap", ".h3c", ".vcmp"];
/// 存档扩展名（VCMI 1.x 唯一格式）。
const SAVE_SUFFIX: &str = ".vsgm1";
/// HD 版安装目录校验标记（官方 `hdextractor.cpp:44`）。
const HD_MARKER: &str = "HOMM3 2.0.exe";
/// 目录递归上限（深度 / 项数）：防止误选整盘导致预检卡死。
const DIR_MAX_DEPTH: u32 = 8;
const DIR_MAX_FILES: usize = 2000;
/// HD 包语言目录 → VCMI 语言 identifier（顺序即官方 QMap 迭代序，多命中取后者）。
const HD_LANGUAGES: [(&str, &str); 9] = [
    ("CH", "chinese"),
    ("CZ", "czech"),
    ("DE", "german"),
    ("EN", "english"),
    ("ES", "spanish"),
    ("FR", "french"),
    ("IT", "italian"),
    ("PL", "polish"),
    ("RU", "russian"),
];
/// HD 版全部 8 个 pak：`(文件名, 是否本地化包, 档位)`（官方 `hdextractor.h:38-47`）。
const HD_PAKS: [(&str, bool, u32); 8] = [
    ("bitmap_DXT_com_x2.pak", false, 2),
    ("bitmap_DXT_com_x3.pak", false, 3),
    ("sprite_DXT_com_x2.pak", false, 2),
    ("sprite_DXT_com_x3.pak", false, 3),
    ("bitmap_DXT_loc_x2.pak", true, 2),
    ("bitmap_DXT_loc_x3.pak", true, 3),
    ("sprite_DXT_loc_x2.pak", true, 2),
    ("sprite_DXT_loc_x3.pak", true, 3),
];

// ── 基础类型 ────────────────────────────────────────────────────────────────

/// 冲突处理策略（与前端 `conflict` 参数、领域地图导入同口径）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Conflict {
    Skip,
    Overwrite,
    Rename,
}

impl Conflict {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s {
            "skip" => Ok(Self::Skip),
            "overwrite" => Ok(Self::Overwrite),
            "rename" => Ok(Self::Rename),
            _ => Err(be_err("vcmi_illegal_conflict_mode", "")),
        }
    }
}

/// zip 自动分类结果（复刻官方 `EZipType`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZipKind {
    Mod,
    Maps,
    Saves,
}

impl ZipKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Mod => "mod",
            Self::Maps => "maps",
            Self::Saves => "saves",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        match s {
            "mod" => Some(Self::Mod),
            "maps" => Some(Self::Maps),
            "saves" => Some(Self::Saves),
            _ => None,
        }
    }
}

/// 导入目标目录组（业务层不持 `VcmiPaths`，便于单测用临时目录构造）。
#[derive(Debug, Clone)]
pub struct ImportEnv {
    pub maps_dir: PathBuf,
    pub saves_dir: PathBuf,
    pub mods_dir: PathBuf,
    pub config_dir: PathBuf,
}

impl ImportEnv {
    pub fn from_paths(p: &VcmiPaths) -> Self {
        Self {
            maps_dir: p.user_maps_dir(),
            saves_dir: p.saves_dir.clone(),
            mods_dir: p.user_mods_dir(),
            config_dir: p.user_config_dir.clone(),
        }
    }
}

// ── 返回契约（前端 `src/lib/tauri.ts` 同构定义）────────────────────────────

/// 模组预览（zip 判为模组且探查成功时）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModPreview {
    pub mod_id: String,
    pub name: String,
    pub version: String,
    pub entry_count: usize,
    pub target: String,
    pub exists: bool,
}

/// zip 识别结论（透明化：`overridden` = 用户改判生效）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ZipPreview {
    /// `mod` | `maps` | `saves`
    pub detected: String,
    pub overridden: bool,
    /// 是否含 mod.json（false + detected=mod → 前端显示「按模组尝试」）
    pub has_mod_json: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub map_count: usize,
    pub save_count: usize,
}

/// 目录递归预览：各类型计数 + 忽略数 + 冲突数（一项多文件，与 zip 同口径）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirPreview {
    pub map_count: usize,
    pub campaign_count: usize,
    pub save_count: usize,
    pub config_count: usize,
    pub zip_count: usize,
    /// 目录内不支持（含嵌套 HD 标记 exe / `.exe` / 无关文件）的项数
    pub ignored: usize,
    /// 平铺文件目标已存在的项数（zip 子项需解压才能判定，不计入）
    pub conflict_count: usize,
}

/// HD 目录预览：校验 / 语言 / pak 命中数（缺失项列出）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HdPreview {
    pub valid: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub language_code: Option<String>,
    pub paks: PaksCount,
    pub missing: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct PaksCount {
    pub found: usize,
    pub total: usize,
}

/// 预检条目。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewItem {
    pub path: String,
    /// `map` | `campaign` | `zip` | `save` | `config` | `chronicles` | `hd` | `dir` | `unsupported`
    pub kind: String,
    pub ext: String,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<serde_json::Value>,
    #[serde(rename = "mod", skip_serializing_if = "Option::is_none")]
    pub mod_info: Option<ModPreview>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zip: Option<ZipPreview>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hd: Option<HdPreview>,
    /// 仅普通目录输入：递归收集结果统计。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dir: Option<DirPreview>,
    /// 仅 `.exe`：Inno Setup 特征串识别结论（GOG 离线包为 Inno 安装器）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub inno: Option<bool>,
    pub target: String,
    pub conflict: bool,
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewSummary {
    pub total: usize,
    pub by_kind: BTreeMap<String, usize>,
    pub conflicts: usize,
    pub unsupported: usize,
}

#[derive(Debug, Serialize)]
pub struct PreviewReport {
    pub items: Vec<PreviewItem>,
    pub summary: PreviewSummary,
}

/// 执行结果条目。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecItem {
    pub path: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dest: Option<String>,
    /// `added` | `replaced` | `skipped` | `failed` | `unsupported`
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// 本次装成（或替换）的模组清单（前端据此重扫 + 自动启用 + 加入当前预设）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModInstalled {
    pub mod_id: String,
    pub name: String,
    pub version: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecReport {
    pub total: usize,
    pub added: usize,
    pub skipped: usize,
    pub failed: usize,
    pub replaced: usize,
    pub unsupported: usize,
    pub items: Vec<ExecItem>,
    pub mods_installed: Vec<ModInstalled>,
}

// ── 顶层分类 ────────────────────────────────────────────────────────────────

/// 输入顶层分类（预检与执行共用，避免两处逻辑漂移）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum InputClass {
    Map,
    Campaign,
    Zip,
    Save,
    Config,
    Chronicles,
    Hd,
    /// 普通目录：递归收集受支持文件后按类型分流
    DirPack,
    Unsupported,
    Missing,
}

/// 文件名恰为 HD 标记本体（`HOMM3 2.0.exe`，大小写不敏感）。
///
/// 「选择文件」与「选择 HD 目录」合一后，用户可直接选中该 exe 作为 HD 素材入口，
/// 不必理解“该选哪一层目录”。
fn is_hd_marker_file(p: &Path) -> bool {
    p.file_name()
        .map(|f| f.to_string_lossy().eq_ignore_ascii_case(HD_MARKER))
        .unwrap_or(false)
}

/// HD 素材根目录：目录（含标记）→ 自身；标记 exe 本体 → 其父目录。
fn hd_root(p: &Path) -> Option<PathBuf> {
    if p.is_dir() {
        return find_child_file_ci(p, HD_MARKER).map(|_| p.to_path_buf());
    }
    if is_hd_marker_file(p) {
        return p.parent().map(|d| d.to_path_buf());
    }
    None
}

// ── 普通目录递归收集 ──────────────────────────────────────────────────────

/// 目录内可自动导入的文件分类（`.exe` / HD 标记不在此列：需显式选择，不随目录摊平）。
fn classify_dir_file(p: &Path) -> Option<InputClass> {
    match ext_lower(p).as_str() {
        "zip" => Some(InputClass::Zip),
        "h3m" | "vmap" => Some(InputClass::Map),
        "h3c" | "vcmp" => Some(InputClass::Campaign),
        "vsgm1" => Some(InputClass::Save),
        "json" => Some(InputClass::Config),
        _ => None,
    }
}

/// 目录内文件的相对路径（`/` 分隔；越界或异常 → None 计入忽略）。
fn rel_from_root(root: &Path, p: &Path) -> Option<PathBuf> {
    let rel = p.strip_prefix(root).ok()?;
    let s = rel.to_string_lossy().replace('\\', "/");
    safe_rel_path(&s)
}

/// 目录递归收集结果：受支持文件（相对路径 + 分类）与忽略计数。
struct DirScan {
    entries: Vec<(PathBuf, InputClass)>,
    ignored: usize,
}

fn scan_dir(root: &Path) -> DirScan {
    let mut entries = Vec::new();
    let mut ignored = 0usize;
    walk_dir(root, root, 0, &mut entries, &mut ignored);
    DirScan { entries, ignored }
}

fn walk_dir(
    root: &Path,
    dir: &Path,
    depth: u32,
    entries: &mut Vec<(PathBuf, InputClass)>,
    ignored: &mut usize,
) {
    if depth > DIR_MAX_DEPTH || entries.len() >= DIR_MAX_FILES {
        return;
    }
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        if entries.len() >= DIR_MAX_FILES {
            return;
        }
        let p = e.path();
        if p.is_dir() {
            // 联接保护 + 深度上限防环（与 `dir_size` 同口径）
            if !is_link(&p) {
                walk_dir(root, &p, depth + 1, entries, ignored);
            }
            continue;
        }
        match classify_dir_file(&p).and_then(|c| rel_from_root(root, &p).map(|r| (r, c))) {
            Some(pair) => entries.push(pair),
            None => *ignored += 1,
        }
    }
}

fn count_class(entries: &[(PathBuf, InputClass)], target: InputClass) -> usize {
    entries.iter().filter(|(_, c)| *c == target).count()
}

/// 分类对应的落盘目录（zip 由 `exec_zip` 自行决定去向）。
fn dest_dir_for(env: &ImportEnv, class: InputClass) -> &Path {
    match class {
        InputClass::Map | InputClass::Campaign => &env.maps_dir,
        InputClass::Save => &env.saves_dir,
        InputClass::Config => &env.config_dir,
        _ => &env.mods_dir,
    }
}

/// 目录项主目标：单一去向 → 该目录；跸多去向 → 用户数据根（表示“按类型分流”）。
fn dir_primary_target(env: &ImportEnv, scan: &DirScan) -> PathBuf {
    let Some(first) = scan.entries.first() else {
        return env.maps_dir.clone();
    };
    let base = dest_dir_for(env, first.1);
    let mixed = scan.entries.iter().any(|(_, c)| dest_dir_for(env, *c) != base);
    if mixed {
        env.maps_dir
            .parent()
            .map(PathBuf::from)
            .unwrap_or_else(|| env.maps_dir.clone())
    } else {
        base.to_path_buf()
    }
}

fn classify_input(path: &Path) -> InputClass {
    if path.is_dir() {
        return if find_child_file_ci(path, HD_MARKER).is_some() {
            InputClass::Hd
        } else {
            InputClass::DirPack
        };
    }
    if !path.is_file() {
        return InputClass::Missing;
    }
    // 标记 exe 本体等同 HD 安装目录（必须先于 `.exe` → 历代记 判定）
    if is_hd_marker_file(path) {
        return InputClass::Hd;
    }
    match ext_lower(path).as_str() {
        "zip" => InputClass::Zip,
        "h3m" | "vmap" => InputClass::Map,
        "h3c" | "vcmp" => InputClass::Campaign,
        "vsgm1" => InputClass::Save,
        "json" => InputClass::Config,
        "exe" => InputClass::Chronicles,
        _ => InputClass::Unsupported,
    }
}

fn kind_str(c: InputClass) -> &'static str {
    match c {
        InputClass::Map => "map",
        InputClass::Campaign => "campaign",
        InputClass::Zip => "zip",
        InputClass::Save => "save",
        InputClass::Config => "config",
        InputClass::Chronicles => "chronicles",
        InputClass::Hd => "hd",
        InputClass::DirPack => "dir",
        InputClass::Unsupported | InputClass::Missing => "unsupported",
    }
}

fn ext_lower(p: &Path) -> String {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default()
}

fn file_name_of(p: &Path) -> String {
    p.file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_else(|| "unnamed".to_string())
}

// ── 文件系统小工具（大小写不敏感解析；与 `hd_edition.rs` 同口径但独立，便于单测）──

fn find_child_ci(parent: &Path, name: &str) -> Option<PathBuf> {
    let rd = fs::read_dir(parent).ok()?;
    for e in rd.flatten() {
        if e.file_name().to_string_lossy().eq_ignore_ascii_case(name) {
            let p = e.path();
            if p.is_dir() {
                return Some(p);
            }
        }
    }
    None
}

fn find_child_file_ci(parent: &Path, name: &str) -> Option<PathBuf> {
    let rd = fs::read_dir(parent).ok()?;
    for e in rd.flatten() {
        if e.file_name().to_string_lossy().eq_ignore_ascii_case(name) {
            let p = e.path();
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

/// 逐级大小写不敏感解析路径（如 `["data","LOC","EN"]`）。
fn resolve_ci(base: &Path, parts: &[&str]) -> Option<PathBuf> {
    let mut cur = base.to_path_buf();
    for p in parts {
        cur = find_child_ci(&cur, p)?;
    }
    Some(cur)
}

/// 条目大小：文件取 metadata；目录递归累加（仅元数据，不读内容；不追联接防环）。
fn entry_size(p: &Path) -> u64 {
    if p.is_file() {
        return fs::metadata(p).map(|m| m.len()).unwrap_or(0);
    }
    if p.is_dir() {
        return dir_size(p, 0);
    }
    0
}

fn dir_size(p: &Path, depth: u32) -> u64 {
    if depth > 8 {
        return 0;
    }
    let mut total = 0u64;
    if let Ok(rd) = fs::read_dir(p) {
        for e in rd.flatten() {
            let path = e.path();
            if path.is_dir() {
                if !is_link(&path) {
                    total += dir_size(&path, depth + 1);
                }
            } else if let Ok(m) = e.metadata() {
                total += m.len();
            }
        }
    }
    total
}

// ── zip 探查 ────────────────────────────────────────────────────────────────

fn open_zip(path: &Path) -> Result<zip::ZipArchive<fs::File>, String> {
    let f = fs::File::open(path).map_err(|e| be_err("vcmi_zip_open_failed", &format!("{}", e)))?;
    zip::ZipArchive::new(f).map_err(|_| be_err("vcmi_zip_corrupted", ""))
}

/// 列出全部条目名（**保留原始名**：`by_name` 需精确匹配；路径归一化在使用处做）。
fn zip_file_names(zip_path: &Path) -> Result<Vec<String>, String> {
    let mut archive = open_zip(zip_path)?;
    let mut names = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        if let Ok(e) = archive.by_index(i) {
            names.push(e.name().to_string());
        }
    }
    Ok(names)
}

/// 复刻官方 `classifyZipArchive`（`cmodlistview_moc.cpp:1036-1064`）。
pub fn classify_zip_names(names: &[String]) -> ZipKind {
    let mut has_mod_json = false;
    let mut has_maps = false;
    let mut has_saves = false;
    for name in names {
        let lower = name.to_ascii_lowercase();
        if lower.ends_with("mod.json") {
            has_mod_json = true;
        }
        if MAP_SUFFIXES.iter().any(|s| lower.ends_with(s)) {
            has_maps = true;
        }
        if lower.ends_with(SAVE_SUFFIX) {
            has_saves = true;
        }
    }
    if has_mod_json {
        ZipKind::Mod
    } else if has_maps {
        ZipKind::Maps
    } else if has_saves {
        ZipKind::Saves
    } else {
        ZipKind::Mod
    }
}

/// zip 条目名 → 安全相对路径（zip-slip 防御：拒绝绝对路径 / `..` / 盘符与 ADS 冒号）。
pub fn safe_rel_path(name: &str) -> Option<PathBuf> {
    let norm = name.replace('\\', "/");
    let mut out = PathBuf::new();
    for seg in norm.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." || seg.contains(':') {
            return None;
        }
        out.push(seg);
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

/// zip 预检信息（分类 + 待解出条目）。
struct ZipScan {
    kind: ZipKind,
    overridden: bool,
    has_mod_json: bool,
    /// 地图包待解出条目：`(zip 内原始条目名, 安全相对路径)`
    map_entries: Vec<(String, PathBuf)>,
    save_entries: Vec<(String, PathBuf)>,
    /// 首个非法路径条目（zip-slip），有值则整体拒绝
    unsafe_entry: Option<String>,
}

fn scan_zip(zip_path: &Path, override_kind: Option<ZipKind>) -> Result<ZipScan, String> {
    let names = zip_file_names(zip_path)?;
    let has_mod_json = names
        .iter()
        .any(|n| n.to_ascii_lowercase().ends_with("mod.json"));
    let detected = classify_zip_names(&names);
    let mut map_entries = Vec::new();
    let mut save_entries = Vec::new();
    let mut unsafe_entry = None;
    for name in &names {
        let trimmed = name.trim_end_matches(['/', '\\']);
        if trimmed.is_empty() || trimmed.ends_with(':') {
            continue;
        }
        let lower = name.to_ascii_lowercase();
        let is_map = MAP_SUFFIXES.iter().any(|s| lower.ends_with(s));
        let is_save = lower.ends_with(SAVE_SUFFIX);
        if !is_map && !is_save {
            continue;
        }
        match safe_rel_path(name) {
            Some(rel) => {
                if is_map {
                    map_entries.push((name.clone(), rel));
                } else {
                    save_entries.push((name.clone(), rel));
                }
            }
            None => {
                if unsafe_entry.is_none() {
                    unsafe_entry = Some(name.clone());
                }
            }
        }
    }
    Ok(ZipScan {
        kind: override_kind.unwrap_or(detected),
        overridden: override_kind.is_some(),
        has_mod_json,
        map_entries,
        save_entries,
        unsafe_entry,
    })
}

/// zip 预检聚合（目标 / 冲突 / 可导入性）。
struct ZipOutcome {
    preview: ZipPreview,
    mod_info: Option<ModPreview>,
    target: String,
    conflict: bool,
    ok: bool,
    reason: Option<String>,
}

fn zip_outcome_for(env: &ImportEnv, zip_path: &Path, scan: &ZipScan) -> ZipOutcome {
    let mut out = ZipOutcome {
        preview: ZipPreview {
            detected: scan.kind.as_str().to_string(),
            overridden: scan.overridden,
            has_mod_json: scan.has_mod_json,
            name: None,
            version: None,
            map_count: scan.map_entries.len(),
            save_count: scan.save_entries.len(),
        },
        mod_info: None,
        target: String::new(),
        conflict: false,
        ok: true,
        reason: None,
    };
    if let Some(bad) = &scan.unsafe_entry {
        out.ok = false;
        out.reason = Some(format!("压缩包内含非法路径条目（zip-slip），已拒绝：{bad}"));
        return out;
    }
    match scan.kind {
        ZipKind::Mod => match mod_install::inspect(zip_path, &env.mods_dir) {
            Ok(info) => {
                out.conflict = info.exists;
                out.target = info.target.to_string_lossy().into_owned();
                out.preview.name = Some(info.name.clone());
                out.preview.version = Some(info.version.clone());
                out.mod_info = Some(ModPreview {
                    mod_id: info.mod_id,
                    name: info.name,
                    version: info.version,
                    entry_count: info.entry_count,
                    target: info.target.to_string_lossy().into_owned(),
                    exists: info.exists,
                });
            }
            Err(e) => {
                out.ok = false;
                out.reason = Some(e);
                out.target = env.mods_dir.to_string_lossy().into_owned();
            }
        },
        ZipKind::Maps => {
            out.target = env.maps_dir.to_string_lossy().into_owned();
            out.conflict = scan
                .map_entries
                .iter()
                .any(|(_, rel)| env.maps_dir.join(rel).exists());
        }
        ZipKind::Saves => {
            out.target = env.saves_dir.to_string_lossy().into_owned();
            out.conflict = scan
                .save_entries
                .iter()
                .any(|(_, rel)| env.saves_dir.join(rel).exists());
        }
    }
    out
}

// ── 历代记识别 / HD 目录 ────────────────────────────────────────────────────

/// 轻量识别 PE 内 Inno Setup 特征串（GOG 历代记离线包为 Inno Setup 安装器）。
///
/// 分块扫描（特征串跨块时保留尾部重叠），大包（数百 MB）也不会整读进内存。
pub fn inno_signature(path: &Path) -> bool {
    const NEEDLE: &[u8] = b"Inno Setup";
    let Ok(mut f) = fs::File::open(path) else {
        return false;
    };
    let mut buf = vec![0u8; 1 << 20];
    let mut carry: Vec<u8> = Vec::new();
    loop {
        let n = match f.read(&mut buf) {
            Ok(0) => return false,
            Ok(n) => n,
            Err(_) => return false,
        };
        let mut window = std::mem::take(&mut carry);
        window.extend_from_slice(&buf[..n]);
        if window.windows(NEEDLE.len()).any(|w| w == NEEDLE) {
            return true;
        }
        let keep = NEEDLE.len() - 1;
        carry = if window.len() > keep {
            window[window.len() - keep..].to_vec()
        } else {
            window
        };
    }
}

/// 探测 HD 版素材语言（`data/LOC/<CODE>` 目录；多命中取官方迭代序的最后者）。
fn detect_hd_language(dir: &Path) -> Option<(String, String)> {
    let loc = resolve_ci(dir, &["data", "LOC"])?;
    let mut found: Option<(String, String)> = None;
    for (code, name) in HD_LANGUAGES {
        if find_child_ci(&loc, code).is_some() {
            found = Some((code.to_string(), name.to_string()));
        }
    }
    found
}

/// 定位单个 pak 源文件（com 在 `data/`，loc 在 `data/LOC/<CODE>/`）。
fn hd_pak_source(dir: &Path, name: &str, is_loc: bool, lang_code: &Option<String>) -> Option<PathBuf> {
    let base = if is_loc {
        resolve_ci(dir, &["data", "LOC", lang_code.as_deref()?])?
    } else {
        resolve_ci(dir, &["data"])?
    };
    find_child_file_ci(&base, name)
}

struct HdScan {
    valid: bool,
    language: Option<(String, String)>,
    paks_found: usize,
    missing: Vec<String>,
}

fn scan_hd(dir: &Path) -> HdScan {
    let valid = find_child_file_ci(dir, HD_MARKER).is_some();
    let language = if valid { detect_hd_language(dir) } else { None };
    let code = language.as_ref().map(|(c, _)| c.clone());
    let mut paks_found = 0usize;
    let mut missing = Vec::new();
    for (name, is_loc, _scale) in HD_PAKS {
        if hd_pak_source(dir, name, is_loc, &code).is_some() {
            paks_found += 1;
        } else {
            missing.push(name.to_string());
        }
    }
    HdScan {
        valid,
        language,
        paks_found,
        missing,
    }
}

fn write_mod_json(dir: &Path, v: serde_json::Value) -> Result<(), String> {
    let text = serde_json::to_string(&v).map_err(|e| be_err("vcmi_mod_json_serialize_failed", &format!("{}", e)))?;
    fs::write(dir.join("mod.json"), text).map_err(|e| be_err("vcmi_mod_json_write_failed", &format!("{}", e)))
}

fn copy_pak_or_count(
    src_dir: &Path,
    name: &str,
    is_loc: bool,
    lang_code: &Option<String>,
    dst_dir: &Path,
    files: &mut usize,
    missing: &mut Vec<String>,
) -> Result<(), String> {
    match hd_pak_source(src_dir, name, is_loc, lang_code) {
        Some(src) => {
            fs::copy(&src, dst_dir.join(name)).map_err(|e| be_err("vcmi_copy_failed", &format!("{} {}", name, e)))?;
            *files += 1;
        }
        None => missing.push(name.to_string()),
    }
    Ok(())
}

/// HD 素材安装结果。
#[derive(Debug)]
pub struct HdInstallOutcome {
    pub dest: PathBuf,
    pub files: usize,
    pub replaced: bool,
    pub missing: Vec<String>,
}

/// 从 HD 版安装目录生成 `Mods/hd-edition`（复刻官方 `hdextractor.cpp:36-101` 产物结构）。
///
/// - 校验根目录含 `HOMM3 2.0.exe`；
/// - `overwrite=true` 且目标已存在 → 先清空重建（联接只删本体）；
/// - 产物：父 `mod.json` + `content/data/spriteFlagsInfo.txt` + `content/data/flags/*`；
///   子模组 `mods/x2`、`mods/x3`（Graphical）+ 有语言时 `mods/x{2,3}_loc_<CODE>`（Translation）；
/// - 源 pak 缺失时容忍并计入 `missing`（官方此处双 copy 必失败，不复制该 bug）。
pub fn install_hd_edition(
    env: &ImportEnv,
    src_dir: &Path,
    overwrite: bool,
) -> Result<HdInstallOutcome, String> {
    if find_child_file_ci(src_dir, HD_MARKER).is_none() {
        return Err(be_err("vcmi_not_hd_dir", &format!("{}", HD_MARKER)));
    }
    let target = env.mods_dir.join("hd-edition");
    let had = target.exists();
    if had && !overwrite {
        return Err(be_err(
            "vcmi_hd_mod_already_exists",
            &format!("{}", target.display()),
        ));
    }
    if had {
        if is_link(&target) {
            fs::remove_dir(&target).map_err(|e| be_err("vcmi_hd_cleanup_failed", &format!("{}", e)))?;
        } else {
            fs::remove_dir_all(&target).map_err(|e| be_err("vcmi_hd_cleanup_failed", &format!("{}", e)))?;
        }
    }
    fs::create_dir_all(&target).map_err(|e| be_err("realm_target_dir_create_failed", &format!("{}", e)))?;

    let mut files = 0usize;
    let language = detect_hd_language(src_dir);
    let code = language.as_ref().map(|(c, _)| c.clone());
    let full = language.as_ref().map(|(_, n)| n.clone());

    // 父模组（官方 createModJson 的 nullopt 分支）
    write_mod_json(
        &target,
        serde_json::json!({
            "modType": "Graphical",
            "name": "Heroes III HD Edition",
            "description": "Extracted resources from official Heroes HD to make it usable on VCMI",
            "author": "Ubisoft",
            "version": "1.0",
            "contact": "vcmi.eu",
        }),
    )?;
    files += 1;

    // content/data：spriteFlagsInfo.txt + flags/*（官方 :61-72）
    let data_dst = target.join("content").join("data");
    fs::create_dir_all(&data_dst).map_err(|e| be_err("realm_target_dir_create_failed", &format!("{}", e)))?;
    let src_data = resolve_ci(src_dir, &["data"]);
    if let Some(cfg) =
        src_data.as_deref().and_then(|d| find_child_file_ci(d, "spriteFlagsInfo.txt"))
    {
        fs::copy(&cfg, data_dst.join("spriteFlagsInfo.txt"))
            .map_err(|e| be_err("vcmi_sprite_flags_copy_failed", &format!("{}", e)))?;
        files += 1;
    }
    if let Some(flags_src) = src_data.as_deref().and_then(|d| find_child_ci(d, "flags")) {
        let flags_dst = data_dst.join("flags");
        fs::create_dir_all(&flags_dst).map_err(|e| be_err("realm_target_dir_create_failed", &format!("{}", e)))?;
        if let Ok(rd) = fs::read_dir(&flags_src) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_file() {
                    fs::copy(&p, flags_dst.join(e.file_name()))
                        .map_err(|e| be_err("vcmi_flags_copy_failed", &format!("{}", e)))?;
                    files += 1;
                }
            }
        }
    }

    let mut missing = Vec::new();
    // 基础子模组 x2 / x3（Graphical）
    for scale in [2u32, 3] {
        let subdir = target.join("mods").join(format!("x{scale}"));
        fs::create_dir_all(subdir.join("content").join("data"))
            .map_err(|e| be_err("realm_target_dir_create_failed", &format!("{}", e)))?;
        write_mod_json(
            &subdir,
            serde_json::json!({
                "modType": "Graphical",
                "name": format!("HD (x{scale})"),
                "description": format!("Resources (x{scale})"),
                "author": "Ubisoft",
                "version": "1.0",
                "contact": "vcmi.eu",
            }),
        )?;
        files += 1;
        let dst = subdir.join("content").join("data");
        for (name, is_loc, pak_scale) in HD_PAKS {
            if is_loc || pak_scale != scale {
                continue;
            }
            copy_pak_or_count(src_dir, name, false, &code, &dst, &mut files, &mut missing)?;
        }
    }
    // 本地化子模组（官方目录后缀用短码，如 `x2_loc_EN`；mod.json 的 language 用全名）
    match (code, full) {
        (Some(code), Some(full)) => {
            for scale in [2u32, 3] {
                let loc_paks: Vec<&str> = HD_PAKS
                    .iter()
                    .filter(|(_, is_loc, s)| *is_loc && *s == scale)
                    .map(|(n, _, _)| *n)
                    .collect();
                let has_any = loc_paks
                    .iter()
                    .any(|n| hd_pak_source(src_dir, n, true, &Some(code.clone())).is_some());
                if !has_any {
                    missing.extend(loc_paks.iter().map(|n| (*n).to_string()));
                    continue;
                }
                let subdir = target.join("mods").join(format!("x{scale}_loc_{code}"));
                let dst = subdir.join("content").join("data");
                fs::create_dir_all(&dst).map_err(|e| be_err("realm_target_dir_create_failed", &format!("{}", e)))?;
                write_mod_json(
                    &subdir,
                    serde_json::json!({
                        "modType": "Translation",
                        "name": format!("HD Localisation ({full}) (x{scale})"),
                        "description": format!("Translated Resources (x{scale})"),
                        "author": "Ubisoft",
                        "version": "1.0",
                        "contact": "vcmi.eu",
                        "language": full,
                    }),
                )?;
                files += 1;
                for name in loc_paks {
                    copy_pak_or_count(
                        src_dir,
                        name,
                        true,
                        &Some(code.clone()),
                        &dst,
                        &mut files,
                        &mut missing,
                    )?;
                }
            }
        }
        _ => {
            // 无语言：loc pak 无法定位，全部计入缺失（预检已按 n/8 展示）
            for (name, is_loc, _) in HD_PAKS {
                if is_loc {
                    missing.push(name.to_string());
                }
            }
        }
    }

    Ok(HdInstallOutcome {
        dest: target,
        files,
        replaced: had,
        missing,
    })
}

// ── 冲突落盘 ────────────────────────────────────────────────────────────────

enum DestPlan {
    Skip,
    Use(PathBuf),
}

fn resolve_dest(dest: &Path, conflict: Conflict) -> DestPlan {
    if !dest.exists() {
        return DestPlan::Use(dest.to_path_buf());
    }
    match conflict {
        Conflict::Skip => DestPlan::Skip,
        Conflict::Overwrite => DestPlan::Use(dest.to_path_buf()),
        Conflict::Rename => DestPlan::Use(rename_candidate(dest)),
    }
}

/// 重名避让（复刻领域地图口径）：`a.h3m` → `a-2.h3m` → `a-3.h3m`…
fn rename_candidate(dest: &Path) -> PathBuf {
    let stem = dest
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = dest
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    let mut n = 2;
    loop {
        let cand = dest.with_file_name(format!("{stem}-{n}{ext}"));
        if !cand.exists() {
            return cand;
        }
        n += 1;
    }
}

/// 单文件落盘（含冲突策略）。返回 `(status, 最终目标, 说明)`。
fn copy_file_with_conflict(
    src: &Path,
    dest_dir: &Path,
    conflict: Conflict,
) -> (String, PathBuf, Option<String>) {
    copy_rel_with_conflict(src, dest_dir, Path::new(&file_name_of(src)), conflict)
}

/// 按相对路径落盘（目录 / zip 共用：保留子目录结构，含冲突策略）。
fn copy_rel_with_conflict(
    src: &Path,
    dest_dir: &Path,
    rel: &Path,
    conflict: Conflict,
) -> (String, PathBuf, Option<String>) {
    let dest = dest_dir.join(rel);
    let dest = match resolve_dest(&dest, conflict) {
        DestPlan::Skip => return ("skipped".to_string(), dest, None),
        DestPlan::Use(d) => d,
    };
    if let Some(parent) = dest.parent() {
        if let Err(e) = fs::create_dir_all(parent) {
            return ("failed".to_string(), dest, Some(format!("创建目标目录失败: {e}")));
        }
    }
    let existed = dest.exists();
    match fs::copy(src, &dest) {
        Ok(_) => (
            if existed { "replaced" } else { "added" }.to_string(),
            dest,
            None,
        ),
        Err(e) => ("failed".to_string(), dest, Some(format!("复制失败: {e}"))),
    }
}

/// 从 zip 解出给定条目子集（保相对结构）。返回 `(聚合状态, 说明)`。
fn extract_zip_subset(
    zip_path: &Path,
    entries: &[(String, PathBuf)],
    dest_root: &Path,
    conflict: Conflict,
) -> (String, Option<String>) {
    let mut archive = match open_zip(zip_path) {
        Ok(a) => a,
        Err(e) => return ("failed".to_string(), Some(e)),
    };
    let mut added = 0usize;
    let mut replaced = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    for (entry_name, rel) in entries {
        let dest = dest_root.join(rel);
        let dest = match resolve_dest(&dest, conflict) {
            DestPlan::Skip => {
                skipped += 1;
                continue;
            }
            DestPlan::Use(d) => d,
        };
        let existed = dest.exists();
        if let Some(parent) = dest.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let copied = (|| -> Result<(), String> {
            let mut zf = archive
                .by_name(entry_name)
                .map_err(|e| be_err("vcmi_entry_read_failed", &format!("{}", e)))?;
            let mut out = fs::File::create(&dest).map_err(|e| be_err("rt_file_create_failed", &format!("{}", e)))?;
            std::io::copy(&mut zf, &mut out).map_err(|e| be_err("vcmi_extract_failed", &format!("{}", e)))?;
            Ok(())
        })();
        match copied {
            Ok(()) => {
                if existed {
                    replaced += 1;
                } else {
                    added += 1;
                }
            }
            Err(_) => failed += 1,
        }
    }
    let status = if failed > 0 && added + replaced + skipped == 0 {
        "failed"
    } else if added + replaced > 0 {
        if added == 0 {
            "replaced"
        } else {
            "added"
        }
    } else {
        "skipped"
    };
    let mut notes = Vec::new();
    if skipped > 0 {
        notes.push(format!("{skipped} 项跳过"));
    }
    if failed > 0 {
        notes.push(format!("{failed} 项失败"));
    }
    let reason = if notes.is_empty() {
        None
    } else {
        Some(notes.join("，"))
    };
    (status.to_string(), reason)
}

// ── 预检 ────────────────────────────────────────────────────────────────────

/// 预检（dry run）：分类 / 识别 / 冲突检查，**不写盘**。
pub fn preflight(
    env: &ImportEnv,
    paths: &[String],
    overrides: &BTreeMap<String, String>,
) -> PreviewReport {
    let mut items = Vec::new();
    let mut by_kind: BTreeMap<String, usize> = BTreeMap::new();
    let mut conflicts = 0usize;
    let mut unsupported = 0usize;

    for raw in paths {
        let p = Path::new(raw);
        let class = classify_input(p);
        let kind = kind_str(class);
        *by_kind.entry(kind.to_string()).or_insert(0) += 1;

        let mut item = PreviewItem {
            path: raw.clone(),
            kind: kind.to_string(),
            ext: ext_lower(p),
            size: entry_size(p),
            meta: None,
            mod_info: None,
            zip: None,
            hd: None,
            dir: None,
            inno: None,
            target: String::new(),
            conflict: false,
            ok: true,
            reason: None,
        };

        match class {
            InputClass::Missing => {
                item.ok = false;
                item.reason = Some("文件不存在".to_string());
            }
            InputClass::DirPack => {
                let scan = scan_dir(p);
                let conflict_count = scan
                    .entries
                    .iter()
                    .filter(|(rel, c)| match c {
                        InputClass::Zip => false,
                        other => dest_dir_for(env, *other).join(rel).exists(),
                    })
                    .count();
                if scan.entries.is_empty() {
                    item.ok = false;
                    item.reason = Some("目录内没有可导入的文件".to_string());
                } else {
                    item.target = dir_primary_target(env, &scan).to_string_lossy().into_owned();
                    item.conflict = conflict_count > 0;
                }
                item.dir = Some(DirPreview {
                    map_count: count_class(&scan.entries, InputClass::Map),
                    campaign_count: count_class(&scan.entries, InputClass::Campaign),
                    save_count: count_class(&scan.entries, InputClass::Save),
                    config_count: count_class(&scan.entries, InputClass::Config),
                    zip_count: count_class(&scan.entries, InputClass::Zip),
                    ignored: scan.ignored,
                    conflict_count,
                });
            }
            InputClass::Unsupported => {
                item.ok = false;
                item.reason = Some(format!("不支持的文件类型（.{}）", item.ext));
            }
            InputClass::Map | InputClass::Campaign => {
                item.target = env.maps_dir.to_string_lossy().into_owned();
                item.conflict = env.maps_dir.join(file_name_of(p)).exists();
                item.meta = h3m::parse_file(p)
                    .ok()
                    .and_then(|m| serde_json::to_value(m).ok());
            }
            InputClass::Save => {
                item.target = env.saves_dir.to_string_lossy().into_owned();
                item.conflict = env.saves_dir.join(file_name_of(p)).exists();
            }
            InputClass::Config => {
                item.target = env.config_dir.to_string_lossy().into_owned();
                item.conflict = env.config_dir.join(file_name_of(p)).exists();
            }
            InputClass::Chronicles => {
                item.inno = Some(inno_signature(p));
                item.target = env.mods_dir.join("chronicles").to_string_lossy().into_owned();
            }
            InputClass::Hd => match hd_root(p) {
                None => {
                    item.ok = false;
                    item.reason = Some(format!("无法定位 HD 版根目录（需含 {HD_MARKER}）"));
                }
                Some(root) => {
                    let scan = scan_hd(&root);
                    let target = env.mods_dir.join("hd-edition");
                    item.target = target.to_string_lossy().into_owned();
                    item.conflict = target.exists();
                    item.ok = scan.valid;
                    if !scan.valid {
                        item.reason = Some(format!("所选目录不含 {HD_MARKER}，不是 HD 版安装目录"));
                    }
                    item.hd = Some(HdPreview {
                        valid: scan.valid,
                        language: scan.language.as_ref().map(|(_, n)| n.clone()),
                        language_code: scan.language.as_ref().map(|(c, _)| c.clone()),
                        paks: PaksCount {
                            found: scan.paks_found,
                            total: HD_PAKS.len(),
                        },
                        missing: scan.missing,
                    });
                }
            },
            InputClass::Zip => {
                let override_kind = overrides.get(raw.as_str()).and_then(|s| ZipKind::parse(s));
                match scan_zip(p, override_kind) {
                    Err(e) => {
                        item.ok = false;
                        item.reason = Some(e);
                    }
                    Ok(scan) => {
                        let out = zip_outcome_for(env, p, &scan);
                        item.target = out.target;
                        item.conflict = out.conflict;
                        item.ok = out.ok;
                        item.reason = out.reason;
                        item.mod_info = out.mod_info;
                        item.zip = Some(out.preview);
                    }
                }
            }
        }

        if item.conflict {
            conflicts += 1;
        }
        if kind == "unsupported" {
            unsupported += 1;
        }
        items.push(item);
    }

    PreviewReport {
        items,
        summary: PreviewSummary {
            total: paths.len(),
            by_kind,
            conflicts,
            unsupported,
        },
    }
}

// ── 执行 ────────────────────────────────────────────────────────────────────

/// 执行导入（按 `conflict` 落盘；`overrides` 对 zip 生效）。
pub fn execute(
    env: &ImportEnv,
    paths: &[String],
    conflict: Conflict,
    overrides: &BTreeMap<String, String>,
) -> ExecReport {
    let mut items = Vec::new();
    let mut added = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    let mut replaced = 0usize;
    let mut unsupported = 0usize;
    let mut mods_installed: Vec<ModInstalled> = Vec::new();

    for raw in paths {
        let p = Path::new(raw);
        let class = classify_input(p);
        let mut exec = ExecItem {
            path: raw.clone(),
            kind: kind_str(class).to_string(),
            dest: None,
            status: "failed".to_string(),
            reason: None,
        };

        match class {
            InputClass::Map | InputClass::Campaign => {
                let (status, dest, reason) = copy_file_with_conflict(p, &env.maps_dir, conflict);
                exec.status = status;
                exec.dest = Some(dest.to_string_lossy().into_owned());
                exec.reason = reason;
            }
            InputClass::Save => {
                let (status, dest, reason) = copy_file_with_conflict(p, &env.saves_dir, conflict);
                exec.status = status;
                exec.dest = Some(dest.to_string_lossy().into_owned());
                exec.reason = reason;
            }
            InputClass::Config => {
                let (status, dest, reason) = copy_file_with_conflict(p, &env.config_dir, conflict);
                exec.status = status;
                exec.dest = Some(dest.to_string_lossy().into_owned());
                exec.reason = reason;
            }
            InputClass::Chronicles => {
                // 识别 + 引导（解包二期）：不落盘，状态 unsupported（前端按 kind 渲染引导）
                exec.status = "unsupported".to_string();
            }
            InputClass::Unsupported => {
                exec.status = "unsupported".to_string();
            }
            InputClass::DirPack => {
                exec_dir(env, p, conflict, overrides, &mut exec, &mut mods_installed);
            }
            InputClass::Missing => {
                exec.status = "failed".to_string();
                exec.reason = Some("文件不存在".to_string());
            }
            InputClass::Hd => {
                exec_hd(env, p, conflict, &mut exec, &mut mods_installed);
            }
            InputClass::Zip => {
                exec_zip(
                    env,
                    p,
                    conflict,
                    overrides,
                    &mut exec,
                    &mut mods_installed,
                );
            }
        }

        match exec.status.as_str() {
            "added" => added += 1,
            "replaced" => replaced += 1,
            "skipped" => skipped += 1,
            "unsupported" => unsupported += 1,
            _ => failed += 1,
        }
        items.push(exec);
    }

    ExecReport {
        total: paths.len(),
        added,
        skipped,
        failed,
        replaced,
        unsupported,
        items,
        mods_installed,
    }
}

fn exec_hd(
    env: &ImportEnv,
    src: &Path,
    conflict: Conflict,
    exec: &mut ExecItem,
    mods_installed: &mut Vec<ModInstalled>,
) {
    let Some(root) = hd_root(src) else {
        exec.status = "failed".to_string();
        exec.reason = Some(format!("无法定位 HD 版根目录（需含 {HD_MARKER}）"));
        return;
    };
    let target = env.mods_dir.join("hd-edition");
    exec.dest = Some(target.to_string_lossy().into_owned());
    if target.exists() && conflict == Conflict::Skip {
        exec.status = "skipped".to_string();
        return;
    }
    // rename 对 HD 项不适用 → 按覆盖处理（前端列表已逐项标注）
    match install_hd_edition(env, &root, true) {
        Ok(out) => {
            tracing::info!(
                target: "mods",
                "HD 素材已生成：{}（{} 个文件，缺失 {} 个 pak）",
                out.dest.display(),
                out.files,
                out.missing.len()
            );
            exec.status = if out.replaced { "replaced" } else { "added" }.to_string();
            exec.dest = Some(out.dest.to_string_lossy().into_owned());
            if !out.missing.is_empty() {
                exec.reason = Some(format!("缺少 {} 个 pak", out.missing.len()));
            }
            mods_installed.push(ModInstalled {
                mod_id: "hd-edition".to_string(),
                name: "Heroes III HD Edition".to_string(),
                version: "1.0".to_string(),
            });
        }
        Err(e) => {
            exec.status = "failed".to_string();
            exec.reason = Some(e);
        }
    }
}

/// 目录项执行：递归收集 → 按类型分流落盘（保留相对子目录结构）。
///
/// 一项多文件与 zip 同口径：`status` 为聚合结论，`reason` 给出计数明细。
/// 目录内 zip 复用 `exec_zip`（顶层 `overrides` 的 key 是输入路径，故子项 zip 只能走自动分类）。
fn exec_dir(
    env: &ImportEnv,
    root: &Path,
    conflict: Conflict,
    overrides: &BTreeMap<String, String>,
    exec: &mut ExecItem,
    mods_installed: &mut Vec<ModInstalled>,
) {
    let scan = scan_dir(root);
    if scan.entries.is_empty() {
        exec.status = "unsupported".to_string();
        exec.reason = Some("目录内没有可导入的文件".to_string());
        return;
    }
    let (mut added, mut skipped, mut replaced, mut failed) = (0usize, 0usize, 0usize, 0usize);
    for (rel, class) in &scan.entries {
        let src = root.join(rel);
        let status = match class {
            InputClass::Zip => {
                let mut sub = ExecItem {
                    path: rel.to_string_lossy().into_owned(),
                    kind: "zip".to_string(),
                    dest: None,
                    status: "failed".to_string(),
                    reason: None,
                };
                exec_zip(env, &src, conflict, overrides, &mut sub, mods_installed);
                sub.status
            }
            other => copy_rel_with_conflict(&src, dest_dir_for(env, *other), rel, conflict).0,
        };
        match status.as_str() {
            "added" => added += 1,
            "replaced" => replaced += 1,
            "skipped" => skipped += 1,
            _ => failed += 1,
        }
    }
    exec.dest = Some(dir_primary_target(env, &scan).to_string_lossy().into_owned());
    let mut detail = format!("新增 {added} · 跳过 {skipped} · 失败 {failed}");
    if replaced > 0 {
        detail.push_str(&format!(" · 覆盖 {replaced}"));
    }
    if scan.ignored > 0 {
        detail.push_str(&format!(" · 忽略 {}", scan.ignored));
    }
    exec.status = if failed > 0 {
        "failed"
    } else if added + replaced > 0 {
        if replaced > 0 { "replaced" } else { "added" }
    } else {
        "skipped"
    }
    .to_string();
    exec.reason = Some(detail);
}

fn exec_zip(
    env: &ImportEnv,
    zip_path: &Path,
    conflict: Conflict,
    overrides: &BTreeMap<String, String>,
    exec: &mut ExecItem,
    mods_installed: &mut Vec<ModInstalled>,
) {
    let override_kind = overrides.get(exec.path.as_str()).and_then(|s| ZipKind::parse(s));
    let scan = match scan_zip(zip_path, override_kind) {
        Ok(s) => s,
        Err(e) => {
            exec.status = "failed".to_string();
            exec.reason = Some(e);
            return;
        }
    };
    if let Some(bad) = &scan.unsafe_entry {
        exec.status = "failed".to_string();
        exec.reason = Some(format!("压缩包内含非法路径条目（zip-slip），已拒绝：{bad}"));
        return;
    }
    match scan.kind {
        ZipKind::Mod => {
            let info = match mod_install::inspect(zip_path, &env.mods_dir) {
                Ok(i) => i,
                Err(e) => {
                    exec.status = "failed".to_string();
                    exec.reason = Some(e);
                    return;
                }
            };
            if info.exists && conflict == Conflict::Skip {
                exec.status = "skipped".to_string();
                exec.dest = Some(info.target.to_string_lossy().into_owned());
                return;
            }
            // rename 对模组不适用 → 按覆盖处理（前端列表已逐项标注）
            match mod_install::install(zip_path, &env.mods_dir, true) {
                Ok(out) => {
                    exec.status = if out.replaced { "replaced" } else { "added" }.to_string();
                    exec.dest = Some(out.dest.to_string_lossy().into_owned());
                    mods_installed.push(ModInstalled {
                        mod_id: info.mod_id,
                        name: info.name,
                        version: info.version,
                    });
                }
                Err(e) => {
                    exec.status = "failed".to_string();
                    exec.reason = Some(e);
                }
            }
        }
        ZipKind::Maps => {
            exec.dest = Some(env.maps_dir.to_string_lossy().into_owned());
            let (status, notes) =
                extract_zip_subset(zip_path, &scan.map_entries, &env.maps_dir, conflict);
            exec.status = status;
            exec.reason = notes;
        }
        ZipKind::Saves => {
            exec.dest = Some(env.saves_dir.to_string_lossy().into_owned());
            let (status, notes) =
                extract_zip_subset(zip_path, &scan.save_entries, &env.saves_dir, conflict);
            exec.status = status;
            exec.reason = notes;
        }
    }
}

// ── 单测 ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write as _;

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vcmi-fv-game-import-{tag}"));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_zip(path: &Path, entries: &[(&str, &str)]) {
        let f = fs::File::create(path).unwrap();
        let mut w = zip::ZipWriter::new(f);
        let opt: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name, data) in entries {
            w.start_file(*name, opt).unwrap();
            w.write_all(data.as_bytes()).unwrap();
        }
        w.finish().unwrap();
    }

    fn env_at(root: &Path) -> ImportEnv {
        ImportEnv {
            maps_dir: root.join("Maps"),
            saves_dir: root.join("Saves"),
            mods_dir: root.join("Mods"),
            config_dir: root.join("config"),
        }
    }

    fn s(p: &Path) -> String {
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn classify_zip_prefers_mod_json_then_maps_then_saves() {
        let both = vec!["x/mod.json".to_string(), "x/map.h3m".to_string()];
        assert_eq!(classify_zip_names(&both), ZipKind::Mod, "mod.json 优先于地图");
        let maps = vec!["a.h3m".to_string(), "b/c.vmap".to_string()];
        assert_eq!(classify_zip_names(&maps), ZipKind::Maps);
        let saves = vec!["s.vsgm1".to_string()];
        assert_eq!(classify_zip_names(&saves), ZipKind::Saves);
        let none = vec!["readme.txt".to_string()];
        assert_eq!(
            classify_zip_names(&none),
            ZipKind::Mod,
            "三者都不含 → 按模组尝试（官方 fallback）"
        );
        // 大小写不敏感（官方 toLower 口径）
        assert_eq!(classify_zip_names(&["A\\B.H3M".to_string()]), ZipKind::Maps);
    }

    #[test]
    fn safe_rel_path_rejects_zip_slip() {
        assert_eq!(safe_rel_path("../evil.h3m"), None);
        assert_eq!(safe_rel_path("a/../../b.h3m"), None);
        assert_eq!(safe_rel_path("C:/win.h3m"), None);
        assert_eq!(safe_rel_path("a\\b\\c.h3m"), Some(PathBuf::from("a/b/c.h3m")));
        assert_eq!(safe_rel_path("./a.h3m"), Some(PathBuf::from("a.h3m")));
        assert_eq!(safe_rel_path(""), None);
    }

    #[test]
    fn conflict_strategies_on_single_file() {
        let d = tmp_dir("conflict");
        let env = env_at(&d);
        let src = d.join("new.h3m");
        fs::write(&src, b"new").unwrap();

        let rep = execute(&env, &[s(&src)], Conflict::Skip, &BTreeMap::new());
        assert_eq!(rep.added, 1);
        assert!(env.maps_dir.join("new.h3m").is_file());

        // skip：内容不变
        fs::write(&src, b"newer").unwrap();
        let rep = execute(&env, &[s(&src)], Conflict::Skip, &BTreeMap::new());
        assert_eq!(rep.skipped, 1);
        assert_eq!(fs::read(env.maps_dir.join("new.h3m")).unwrap(), b"new");

        // overwrite：替换
        let rep = execute(&env, &[s(&src)], Conflict::Overwrite, &BTreeMap::new());
        assert_eq!(rep.replaced, 1);
        assert_eq!(fs::read(env.maps_dir.join("new.h3m")).unwrap(), b"newer");

        // rename：落到 -2 后缀，原文件保留
        let rep = execute(&env, &[s(&src)], Conflict::Rename, &BTreeMap::new());
        assert_eq!(rep.added, 1);
        assert!(env.maps_dir.join("new-2.h3m").is_file());
        assert_eq!(fs::read(env.maps_dir.join("new.h3m")).unwrap(), b"newer");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn preflight_zip_maps_counts_and_conflicts() {
        let d = tmp_dir("prezip");
        let env = env_at(&d);
        let zip_path = d.join("pack.zip");
        write_zip(
            &zip_path,
            &[("sub/a.h3m", "a"), ("b.vmap", "b"), ("note.txt", "n")],
        );
        let rep = preflight(&env, &[s(&zip_path)], &BTreeMap::new());
        let it = &rep.items[0];
        assert_eq!(it.kind, "zip");
        assert!(it.ok);
        let zp = it.zip.as_ref().unwrap();
        assert_eq!(zp.detected, "maps");
        assert_eq!(zp.map_count, 2, "仅地图扩展名条目计入");
        assert!(!zp.overridden);
        assert!(!it.conflict);

        // 预置冲突文件 → conflict + summary 计数
        fs::create_dir_all(env.maps_dir.join("sub")).unwrap();
        fs::write(env.maps_dir.join("sub/a.h3m"), b"old").unwrap();
        let rep = preflight(&env, &[s(&zip_path)], &BTreeMap::new());
        assert!(rep.items[0].conflict);
        assert_eq!(rep.summary.conflicts, 1);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn execute_zip_maps_preserves_structure_and_skips_others() {
        let d = tmp_dir("zexe");
        let env = env_at(&d);
        let zip_path = d.join("maps.zip");
        write_zip(&zip_path, &[("sub/a.h3m", "a"), ("note.txt", "n")]);
        let rep = execute(&env, &[s(&zip_path)], Conflict::Skip, &BTreeMap::new());
        assert_eq!(rep.added, 1);
        assert_eq!(fs::read(env.maps_dir.join("sub/a.h3m")).unwrap(), b"a");
        assert!(!env.maps_dir.join("note.txt").exists(), "非地图条目不得解出");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn zip_slip_entry_rejected_in_preflight_and_execute() {
        let d = tmp_dir("zipslip");
        let env = env_at(&d);
        let zip_path = d.join("evil.zip");
        write_zip(
            &zip_path,
            &[("../escape.h3m", "boom"), ("ok.h3m", "ok")],
        );
        let rep = preflight(&env, &[s(&zip_path)], &BTreeMap::new());
        assert!(!rep.items[0].ok);
        assert!(rep.items[0].reason.as_deref().unwrap().contains("zip-slip"));

        let rep = execute(&env, &[s(&zip_path)], Conflict::Overwrite, &BTreeMap::new());
        assert_eq!(rep.failed, 1);
        assert!(!d.join("escape.h3m").exists(), "越界条目不得落地");
        assert!(!env.maps_dir.join("ok.h3m").exists(), "同批非法条目存在时整体拒绝");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn zip_override_reclassifies() {
        let d = tmp_dir("override");
        let env = env_at(&d);
        // 无 mod.json / 无地图 → 官方 fallback 按模组；改判为地图包
        let zip_path = d.join("mystery.zip");
        write_zip(&zip_path, &[("data/foo.bin", "x")]);
        let mut ov = BTreeMap::new();
        ov.insert(s(&zip_path), "maps".to_string());
        let rep = preflight(&env, &[s(&zip_path)], &ov);
        let zp = rep.items[0].zip.as_ref().unwrap();
        assert_eq!(zp.detected, "maps");
        assert!(zp.overridden);
        assert_eq!(zp.map_count, 0);
        assert!(!zp.has_mod_json);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn execute_installs_mod_zip_and_reports_overwrite() {
        let d = tmp_dir("modzip");
        let env = env_at(&d);
        let zip_path = d.join("mymod-1.0.zip");
        write_zip(
            &zip_path,
            &[
                (
                    "mymod-1.0/mod.json",
                    r#"{"name":"My Mod","version":"1.0","modType":"Other"}"#,
                ),
                ("mymod-1.0/config/x.json", "{}"),
            ],
        );
        let rep = execute(&env, &[s(&zip_path)], Conflict::Skip, &BTreeMap::new());
        assert_eq!(rep.added, 1);
        assert_eq!(rep.mods_installed.len(), 1);
        assert_eq!(rep.mods_installed[0].mod_id, "mymod");
        assert!(env.mods_dir.join("mymod/mod.json").is_file());

        let rep = execute(&env, &[s(&zip_path)], Conflict::Skip, &BTreeMap::new());
        assert_eq!(rep.skipped, 1, "已存在 + skip");
        let rep = execute(&env, &[s(&zip_path)], Conflict::Overwrite, &BTreeMap::new());
        assert_eq!(rep.replaced, 1, "已存在 + overwrite（rename 同口径）");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn preflight_covers_config_chronicles_and_unsupported() {
        let d = tmp_dir("misc");
        let env = env_at(&d);
        let cfg = d.join("settings.json");
        fs::write(&cfg, "{}").unwrap();
        let exe = d.join("chronicles.exe");
        fs::write(&exe, b"MZ ... Inno Setup Setup Data (6.2) ...").unwrap();
        let pdf = d.join("readme.pdf");
        fs::write(&pdf, "x").unwrap();
        let dir = d.join("plain-dir");
        fs::create_dir_all(&dir).unwrap();

        let rep = preflight(
            &env,
            &[s(&cfg), s(&exe), s(&pdf), s(&dir)],
            &BTreeMap::new(),
        );
        assert_eq!(rep.items[0].kind, "config");
        assert!(rep.items[0].ok);
        assert_eq!(rep.items[1].kind, "chronicles");
        assert_eq!(rep.items[1].inno, Some(true));
        assert_eq!(rep.items[2].kind, "unsupported");
        assert!(!rep.items[2].ok);
        assert_eq!(rep.items[3].kind, "dir", "普通目录 → 递归收集（不再一律判不支持）");
        assert!(!rep.items[3].ok, "空目录无可导入项");
        assert_eq!(rep.items[3].dir.as_ref().unwrap().map_count, 0);
        assert_eq!(rep.summary.unsupported, 1);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn inno_signature_scans_needle_across_chunks() {
        let d = tmp_dir("inno");
        let yes = d.join("a.exe");
        fs::write(&yes, b"MZ....Inno Setup Setup Data (6.2)....").unwrap();
        assert!(inno_signature(&yes));
        let no = d.join("b.exe");
        fs::write(&no, b"MZ...plain exe...").unwrap();
        assert!(!inno_signature(&no));
        // 特征串跨 1MB 分块边界也能命中
        let big = d.join("c.exe");
        let mut data = vec![0u8; (1 << 20) - 4];
        data.extend_from_slice(b"Inno Setup");
        fs::write(&big, data).unwrap();
        assert!(inno_signature(&big));
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn dir_pack_recurses_preserves_structure_and_splits_by_kind() {
        let d = tmp_dir("dirpack");
        let env = env_at(&d);
        let src = d.join("bundle");
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("a.h3m"), b"a").unwrap();
        fs::write(src.join("sub/b.vcmp"), b"b").unwrap();
        fs::write(src.join("c.vsgm1"), b"c").unwrap();
        fs::write(src.join("readme.txt"), b"t").unwrap();

        // 预检：聚合为一项，kind=dir，分类计数与忽略数正确
        let rep = preflight(&env, &[s(&src)], &BTreeMap::new());
        assert_eq!(rep.items.len(), 1);
        let it = &rep.items[0];
        assert_eq!(it.kind, "dir");
        assert!(it.ok);
        let dp = it.dir.as_ref().unwrap();
        assert_eq!(
            (dp.map_count, dp.campaign_count, dp.save_count, dp.ignored),
            (1, 1, 1, 1)
        );

        // 执行：按类型分流 + 保留子目录结构；汇总按“项”计数
        let ex = execute(&env, &[s(&src)], Conflict::Skip, &BTreeMap::new());
        assert_eq!(ex.added, 1);
        assert!(env.maps_dir.join("a.h3m").is_file());
        assert!(env.maps_dir.join("sub/b.vcmp").is_file(), "子目录结构应保留");
        assert!(env.saves_dir.join("c.vsgm1").is_file());

        // 二次预检：3 个平铺项全部已存在 → conflict_count=3
        let rep2 = preflight(&env, &[s(&src)], &BTreeMap::new());
        assert!(rep2.items[0].conflict);
        assert_eq!(rep2.items[0].dir.as_ref().unwrap().conflict_count, 3);
        let ex2 = execute(&env, &[s(&src)], Conflict::Skip, &BTreeMap::new());
        assert_eq!(ex2.skipped, 1);
        assert_eq!(ex2.items[0].status, "skipped");

        // 无受支持文件的目录 → 不可导入
        let empty = d.join("empty");
        fs::create_dir_all(&empty).unwrap();
        let rep3 = preflight(&env, &[s(&empty)], &BTreeMap::new());
        assert!(!rep3.items[0].ok);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn hd_marker_exe_alone_is_an_hd_input_with_parent_root() {
        let d = tmp_dir("hd-exe-entry");
        make_fake_hd(&d, false);
        let exe = d.join(HD_MARKER);
        // 合一入口：直接选中标记 exe 也判为 HD，根目录解析为其父目录
        assert!(matches!(classify_input(&exe), InputClass::Hd));
        assert_eq!(hd_root(&exe).as_deref(), Some(d.as_path()));
        // 目录入口保持等价
        assert!(matches!(classify_input(&d), InputClass::Hd));
        assert_eq!(hd_root(&d).as_deref(), Some(d.as_path()));
        // 其它 exe 仍归历代记引导
        let installer = d.join("gog_installer.exe");
        fs::write(&installer, b"MZ").unwrap();
        assert!(matches!(classify_input(&installer), InputClass::Chronicles));
        assert!(hd_root(&installer).is_none());
        let _ = fs::remove_dir_all(&d);
    }

    fn make_fake_hd(dir: &Path, with_loc: bool) {
        fs::create_dir_all(dir.join("data").join("flags")).unwrap();
        fs::write(dir.join(HD_MARKER), b"PE").unwrap();
        fs::write(dir.join("data/spriteFlagsInfo.txt"), b"flags").unwrap();
        fs::write(dir.join("data/flags/f1.bmp"), b"f").unwrap();
        for name in [
            "bitmap_DXT_com_x2.pak",
            "bitmap_DXT_com_x3.pak",
            "sprite_DXT_com_x2.pak",
            "sprite_DXT_com_x3.pak",
        ] {
            fs::write(dir.join("data").join(name), b"pak").unwrap();
        }
        if with_loc {
            let loc = dir.join("data").join("LOC").join("EN");
            fs::create_dir_all(&loc).unwrap();
            for name in [
                "bitmap_DXT_loc_x2.pak",
                "bitmap_DXT_loc_x3.pak",
                "sprite_DXT_loc_x2.pak",
                "sprite_DXT_loc_x3.pak",
            ] {
                fs::write(loc.join(name), b"pak").unwrap();
            }
        }
    }

    #[test]
    fn hd_install_full_structure_with_loc() {
        let d = tmp_dir("hd");
        let env = env_at(&d);
        let src = d.join("HeroesHD");
        make_fake_hd(&src, true);

        let rep = preflight(&env, &[s(&src)], &BTreeMap::new());
        let it = &rep.items[0];
        assert_eq!(it.kind, "hd");
        assert!(it.ok);
        let hd = it.hd.as_ref().unwrap();
        assert!(hd.valid);
        assert_eq!(hd.language.as_deref(), Some("english"));
        assert_eq!(hd.language_code.as_deref(), Some("EN"));
        assert_eq!((hd.paks.found, hd.paks.total), (8, 8));
        assert!(hd.missing.is_empty());
        assert!(!it.conflict);

        let rep = execute(&env, &[s(&src)], Conflict::Skip, &BTreeMap::new());
        assert_eq!(rep.added, 1);
        assert_eq!(rep.mods_installed.len(), 1);
        assert_eq!(rep.mods_installed[0].mod_id, "hd-edition");
        let hd_root = env.mods_dir.join("hd-edition");
        assert!(hd_root.join("mod.json").is_file());
        assert!(hd_root.join("content/data/spriteFlagsInfo.txt").is_file());
        assert!(hd_root.join("content/data/flags/f1.bmp").is_file());
        assert!(hd_root.join("mods/x2/content/data/bitmap_DXT_com_x2.pak").is_file());
        assert!(hd_root.join("mods/x3/content/data/sprite_DXT_com_x3.pak").is_file());
        assert!(hd_root
            .join("mods/x2_loc_EN/content/data/bitmap_DXT_loc_x2.pak")
            .is_file());
        let loc_json: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(hd_root.join("mods/x2_loc_EN/mod.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(loc_json["modType"], "Translation");
        assert_eq!(loc_json["language"], "english");
        let parent_json: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(hd_root.join("mod.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(parent_json["name"], "Heroes III HD Edition");

        // 已存在 + skip → skipped
        let rep = execute(&env, &[s(&src)], Conflict::Skip, &BTreeMap::new());
        assert_eq!(rep.skipped, 1);
        // 已存在 + overwrite → 清空重建（replaced）
        let rep = execute(&env, &[s(&src)], Conflict::Overwrite, &BTreeMap::new());
        assert_eq!(rep.replaced, 1);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn hd_without_loc_installs_base_only() {
        let d = tmp_dir("hd2");
        let env = env_at(&d);
        let src = d.join("HeroesHD");
        make_fake_hd(&src, false);

        let rep = preflight(&env, &[s(&src)], &BTreeMap::new());
        let hd = rep.items[0].hd.as_ref().unwrap();
        assert!(hd.valid);
        assert_eq!(hd.language, None);
        assert_eq!((hd.paks.found, hd.paks.total), (4, 8));

        let rep = execute(&env, &[s(&src)], Conflict::Skip, &BTreeMap::new());
        assert_eq!(rep.added, 1);
        assert_eq!(rep.mods_installed.len(), 1);
        let hd_root = env.mods_dir.join("hd-edition");
        assert!(hd_root.join("mods/x2/content/data/sprite_DXT_com_x2.pak").is_file());
        assert!(!hd_root.join("mods/x2_loc_EN").exists(), "无语言不建本地化子模组");
        let _ = fs::remove_dir_all(&d);
    }
}
