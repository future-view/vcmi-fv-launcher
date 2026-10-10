//! 模组资源解析：按资源名从 mod 目录 / `content.zip` / 游戏 LOD 取字节。
//!
//! 统一处理 VCMI 的三种资源来源（优先级从高到低）：
//! 1. mod 目录内的 `Content/**`（未打包，直接文件）；
//! 2. mod 的 `content.zip`（带缓存索引 `ZipIndex`；支持经 `launcher.json` 的 `archives.zip_passwords` 密码解包）；
//! 3. 游戏根 `Data/*.lod`（`resolve_resource_fallback` 的回退路径）。
//!
//! 依赖方向：只依赖 `infra`（见 `docs/architecture/backend.zh-CN.md` §4.1）。

use crate::infra::berror::be_err;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::vcmi::paths::VcmiPaths;
use crate::vcmi::scan::cached_mod_paths;

/// 单个 content.zip 的条目索引。
#[derive(Default)]
pub(crate) struct ZipIndex {
    /// 归一 key（去前缀 + 去扩展名 + 小写）→ 条目序号
    by_key: HashMap<String, usize>,
    /// 条目序号 → 扩展名（小写，与原实现的取值口径一致）
    ext_of: HashMap<usize, String>,
    /// 是否含加密条目（只读中央目录元数据即可判定，无需密码）
    has_encrypted: bool,
}

pub(crate) static ZIP_INDEX_CACHE: std::sync::OnceLock<Mutex<Vec<(String, std::sync::Arc<ZipIndex>)>>> =
    std::sync::OnceLock::new();

/// 索引条目多（单 mod 可达数千），容量按 mod 数量级给，避免超线性增长
pub(crate) const ZIP_INDEX_CACHE_CAP: usize = 64;

/// `Content/` 目录索引缓存（归一 key → 绝对路径）。
///
/// 原实现每解析一个资源就**全量递归一次 `Content/`**（实测 10,419 次 × 45ms ≈ 360s CPU）；
/// 改为"每个目录只建一次索引"。索引同样按 mod 数量级给容量。
pub(crate) static DIR_INDEX_CACHE: std::sync::OnceLock<
    Mutex<Vec<(String, std::sync::Arc<HashMap<String, PathBuf>>)>>,
> = std::sync::OnceLock::new();
pub(crate) const DIR_INDEX_CACHE_CAP: usize = 64;

/// 目录指纹：路径 + mtime + **顶层条目数**（文件增减时顶层计数会变，足以让缓存失效）。
fn dir_fingerprint(dir: &Path) -> String {
    let mt = fs::metadata(dir)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let n = fs::read_dir(dir).map(|rd| rd.count()).unwrap_or(0);
    format!("{}|{}|{}", dir.to_string_lossy(), mt, n)
}

/// 递归收集：`归一 key -> 绝对路径`。遍历顺序与原 `walk` 一致（read_dir 顺序 + 深度优先），
/// 且用 `entry().or_insert()` 保留首个匹配，保证命中结果与原实现相同。
fn walk_index(d: &Path, base: &Path, out: &mut HashMap<String, PathBuf>) {
    if let Ok(entries) = fs::read_dir(d) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk_index(&p, base, out);
            } else if let Some(k) = norm_zip_key(&p.strip_prefix(base).unwrap_or(&p).to_string_lossy()) {
                out.entry(k).or_insert(p);
            }
        }
    }
}

/// 取（并缓存）目录索引。
fn dir_index_of(dir: &Path) -> std::sync::Arc<HashMap<String, PathBuf>> {
    let key = dir_fingerprint(dir);
    let list = DIR_INDEX_CACHE.get_or_init(|| Mutex::new(Vec::new()));
    let mut guard = list.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, v)) = guard.iter().find(|(k, _)| *k == key) {
        return v.clone();
    }
    let mut map: HashMap<String, PathBuf> = HashMap::new();
    walk_index(dir, dir, &mut map);
    let v = std::sync::Arc::new(map);
    guard.push((key, v.clone()));
    if guard.len() > DIR_INDEX_CACHE_CAP {
        guard.remove(0);
    }
    v
}

/// 条目名归一：去资源根前缀 + 去扩展名 + 小写（zip 索引与目录索引**共用同一口径**）。
pub(crate) fn norm_zip_key(entry_name: &str) -> Option<String> {
    strip_ext(&strip_resource_prefix(&entry_name.replace('\\', "/").to_lowercase()))
}

// 每线程 LRU：已打开的 `content.zip`。
//
// ⚠️ 原实现**每次读取一个条目都 `ZipArchive::new` 重建中央目录**（mod 的 zip 常有数千条目），
// 实测 `res.zip_read` 累计约 525s CPU（10,508+ 次 × ~38ms），优化后成为最大单项。
// `ZipArchive` 不是 `Sync`，故放线程局部（线程内复用，无锁竞争）。
thread_local! {
    static ZIP_ARCHIVES: RefCell<Vec<(String, zip::ZipArchive<fs::File>)>> =
        const { RefCell::new(Vec::new()) };
}
const ZIP_ARCHIVE_CAP: usize = 4;

/// 读取 zip 的第 `index` 个条目：优先复用本线程已打开的 zip。
///
/// 若 `launcher.json` 的 `archives.zip_passwords` 为该 mod 配了密码，则走 `by_index_decrypt`
/// 解密读取（AES 需 `aes-crypto` 特性，传统 ZipCrypto 仅需密码）；否则按原行为 `by_index`
/// （加密条目会失败 → 返回 None，与历史跳过行为一致）。
fn read_zip_entry_cached(zip_path: &Path, index: usize) -> Option<Vec<u8>> {
    let key = format!(
        "{}|{}",
        zip_path.to_string_lossy(),
        crate::infra::cache::fingerprint_of(&[zip_path.to_path_buf()])
    );
    // 解密密码按 mod 目录解析（zip_path 即 mod_path/content.zip）
    let password = zip_path.parent().and_then(zip_password_for);
    ZIP_ARCHIVES.with(|c| {
        let mut v = c.borrow_mut();
        if let Some(pos) = v.iter().position(|(k, _)| *k == key) {
            let arch = &mut v[pos].1;
            // 限定 `entry` 的作用域：它借用 arch，必须在 arch 归还/移动前结束
            let buf = {
                let mut entry = match password.as_deref() {
                    Some(p) => arch.by_index_decrypt(index, p.as_bytes()).ok()?,
                    None => arch.by_index(index).ok()?,
                };
                let mut buf = Vec::new();
                std::io::Read::read_to_end(&mut entry, &mut buf).ok()?;
                buf
            };
            return Some(buf);
        }
        let f = fs::File::open(zip_path).ok()?;
        // 不再包 BufReader：zip 条目多为大块顺序读，8KB 缓冲反而多一次内存拷贝（实测 P1 变慢）
        let mut arch = zip::ZipArchive::new(f).ok()?;
        let buf = {
            let mut entry = match password.as_deref() {
                Some(p) => arch.by_index_decrypt(index, p.as_bytes()).ok()?,
                None => arch.by_index(index).ok()?,
            };
            let mut buf = Vec::new();
            std::io::Read::read_to_end(&mut entry, &mut buf).ok()?;
            buf
        };
        v.push((key, arch));
        if v.len() > ZIP_ARCHIVE_CAP {
            v.remove(0);
        }
        Some(buf)
    })
}

/// 运行时密码表（`launcher.json` 的 `archives.zip_passwords`），进程内固定（单游戏目录），
/// 用 OnceLock 缓存一次，避免每次读档。取不到时为 `None`（走内置密码兜底）。
fn zip_passwords_map() -> Option<&'static HashMap<String, String>> {
    static CACHE: OnceLock<Option<HashMap<String, String>>> = OnceLock::new();
    CACHE
        .get_or_init(|| VcmiPaths::read_launcher_config().map(|c| c.archives.zip_passwords))
        .as_ref()
}

/// 发行版内置密码表（路线 B，编译进 FV 二进制）。
///
/// **发行版开发者：在此常量数组填入你们发行版加密 zip 的密码**，然后自行 `cargo build` 出你们的发行版 FV。
/// 每行一条：`(mod 目录名, 密码)`，例如 `("adeptorium", "你们的密码")`。
/// 也可用 `("_default", 密码)` 作全局兜底——**任何没有单独列出的 mod 都用它**，适用于你们所有加密包共用同一密码的情况。
///
/// public FV 源码此处为空数组 → 构建出的二进制**不携带任何发行版密码**，通用 FV 无法解开任一发行版的加密包；
/// 只有发行版自行修改此处并构建出的二进制才携带对应密码。
///
/// 注意：密码最终会编译进二进制，技术层面仍可被逆向还原（与把密码硬编码进 `VCMI_client.exe` 同级），
/// 仅能挡小白，不提供强保密。若需更强隔离，应把密码埋在发行版自有可信组件、`launcher.json` 只走配置。
pub(crate) const BUILTIN_ZIP_PASSWORDS: &[(&str, &str)] = &[
    // ("adeptorium", "发行版自设密码"),
    // ("_default", "所有未单独列出的 mod 共用的密码"),
];

/// 把 `BUILTIN_ZIP_PASSWORDS` 常量表转成可查的 `HashMap`（进程内只构建一次）。
pub(crate) fn builtin_zip_passwords() -> &'static HashMap<String, String> {
    static CACHE: OnceLock<HashMap<String, String>> = OnceLock::new();
    CACHE.get_or_init(|| {
        BUILTIN_ZIP_PASSWORDS
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    })
}

/// 取某 mod 的解密密码，路由优先级：
/// 1. `launcher.json` 配置（`archives.zip_passwords`）中按 mod 目录名精确匹配（路线 A）；
/// 2. 配置缺失时，回退到 rs 源码内置密码表 `builtin_zip_passwords` 按目录名匹配（路线 B）；
/// 3. 两者都无具体匹配时，再依次尝试配置 / 内置的 `_default` 全局兜底。
///
/// 这样：stock FV（无内置密码、无配置）解不开任何加密包；发行版既可改 `launcher.json`（免构建），
/// 也可改 rs 源码内置密码后自行构建（密码进二进制、不进公开仓库）。
fn zip_password_for(mod_path: &Path) -> Option<String> {
    let leaf = mod_path.file_name()?.to_string_lossy().to_string();
    let config_map = zip_passwords_map();
    let builtin_map = builtin_zip_passwords();
    let specific = config_map
        .and_then(|m| m.get(&leaf))
        .or_else(|| builtin_map.get(&leaf));
    let default = config_map
        .and_then(|m| m.get("_default"))
        .or_else(|| builtin_map.get("_default"));
    specific.or(default).cloned()
}

/// 判断 mod 的 `content.zip` 是否含加密条目（不需要密码，只读中央目录元数据）。
///
/// 供扫描阶段标记 `contentZipEncrypted`，让前端明示"该 mod 资源已加密 / 未采集"，
/// 取代此前"静默丢资源"的不可见行为。
pub(crate) fn content_zip_is_encrypted(mod_path: &Path) -> bool {
    let zip_path = mod_path.join("content.zip");
    let Ok(file) = fs::File::open(&zip_path) else { return false; };
    let Ok(mut archive) = zip::ZipArchive::new(file) else { return false; };
    for i in 0..archive.len() {
        if let Ok(entry) = archive.by_index_raw(i) {
            if entry.encrypted() {
                return true;
            }
        }
    }
    false
}

/// 按资源名定位 mod `content.zip` 内条目序号（资源名形如 `music/CstleTown`，
/// 内部补 `Content/` 前缀后归一为 zip 索引 key，与 `norm_zip_key` 同一口径）。
pub(crate) fn zip_resource_index(mod_path: &Path, resource: &str) -> Option<usize> {
    let key = norm_zip_key(&format!("Content/{}", resource))?;
    let idx = zip_index_of(mod_path)?;
    idx.by_key.get(&key).copied()
}

/// 按条目序号读取 `content.zip` 内字节（复用线程局部已打开的 zip，避免重建中央目录）。
pub(crate) fn read_zip_entry_bytes(mod_path: &Path, index: usize) -> Option<Vec<u8>> {
    read_zip_entry_cached(mod_path, index)
}

/// 条目序号 → 扩展名（小写），用于落盘命名。
pub(crate) fn zip_entry_ext(mod_path: &Path, index: usize) -> Option<String> {
    zip_index_of(mod_path).and_then(|idx| idx.ext_of.get(&index).cloned())
}

/// 取（并缓存）mod 的 zip 索引；无 content.zip 时返回 None。
pub(crate) fn zip_index_of(mod_path: &Path) -> Option<std::sync::Arc<ZipIndex>> {
    let zip_path = mod_path.join("content.zip");
    let key = {
        let m = fs::metadata(&zip_path).ok()?;
        let mt = m
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        format!("{}|{}|{}", zip_path.to_string_lossy(), mt, m.len())
    };

    // 先查缓存（持锁时间极短）
    {
        let list = ZIP_INDEX_CACHE.get_or_init(|| Mutex::new(Vec::new()));
        let guard = list.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, v)) = guard.iter().find(|(k, _)| *k == key) {
            crate::infra::metrics::count("zip.idx.hit", 1);
            return Some(v.clone());
        }
    }
    crate::infra::metrics::count("zip.idx.build", 1);

    let file = fs::File::open(&zip_path).ok()?;
    let mut archive = zip::ZipArchive::new(file).ok()?;
    let mut idx = ZipIndex::default();
    for i in 0..archive.len() {
        // 用 `by_index_raw`：不解密也能拿到条目名与 `encrypted()` 标志，
        // 从而把加密条目也纳入索引（缺密码时读取会失败 → 跳过），并标记 `has_encrypted`。
        let entry = match archive.by_index_raw(i) {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.encrypted() {
            idx.has_encrypted = true;
        }
        let name = entry.name().to_string();
        if let Some(k) = norm_zip_key(&name) {
            idx.by_key.entry(k).or_insert(i);
        }
        let ext = name.rsplit('.').next().unwrap_or("").to_lowercase();
        idx.ext_of.insert(i, ext);
    }
    let arc = std::sync::Arc::new(idx);

    let list = ZIP_INDEX_CACHE.get_or_init(|| Mutex::new(Vec::new()));
    let mut guard = list.lock().unwrap_or_else(|e| e.into_inner());
    guard.push((key, arc.clone()));
    if guard.len() > ZIP_INDEX_CACHE_CAP {
        guard.remove(0);
    }
    Some(arc)
}

pub(crate) fn resolve_resource(mod_path: &Path, resource: &str) -> Option<(Vec<u8>, String)> {
    crate::infra::metrics::scope("res.resolve", || resolve_resource_inner(mod_path, resource))
}

fn resolve_resource_inner(mod_path: &Path, resource: &str) -> Option<(Vec<u8>, String)> {
    let resource = resource.trim();
    if resource.is_empty() {
        return None;
    }
    let norm = resource.trim_start_matches('/').replace('\\', "/");
    // 资源名 key：去常见根前缀 + 去扩展名 + 小写（保证非 None）
    let key = strip_resource_prefix(&norm);
    let key = strip_ext(&key).unwrap_or_else(|| key.clone()).to_lowercase();

    // Content 目录（磁盘）
    let content_dir = mod_path.join("Content");
    if content_dir.is_dir() {
        if let Ok(found) = find_in_dir(&content_dir, &key) {
            let ext = found
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_string();
            if let Ok(data) = fs::read(&found) {
                crate::infra::metrics::count("res.hit.dir", 1);
                return Some((data, ext));
            }
        }
    }

    // content.zip：走条目索引（每个 mod 只建一次索引，替代原来的全量遍历）
    let idx = zip_index_of(mod_path)?;
    let &i = idx.by_key.get(&key)?;
    crate::infra::metrics::count("res.hit.zip", 1);
    let zip_path = mod_path.join("content.zip");
    crate::infra::metrics::scope("res.zip_read", || {
        // 复用本线程已打开的 zip（避免每次读取都重建中央目录）
        let buf = read_zip_entry_cached(&zip_path, i)?;
        // 原实现取条目名末段作扩展名，此处等价（索引里已按同一口径存好）
        let ext = idx.ext_of.get(&i).cloned().unwrap_or_default();
        Some((buf, ext))
    })
}

/// 解析资源：优先在当前 mod 的 Content/content.zip 中查找，
/// 失败则回退到游戏根资源（原版 H3 的 `Data/*.lod`、游戏 `Sprites/` 等）。
/// 原版 9 城镇的背景/建筑图标/兵种背景均位于游戏根资源包内（如 `TBCSBACK.bmp`、
/// `HALLCSTL.DEF`、`TPCASCAS`），`resolve_resource` 只搜 mod 自身故必须回退。
/// 返回**实际命中的扩展名**（经 `read_game_resource_ext` 的基名+多扩展名回退，
/// 配置写 `.bmp`/无扩展名也能命中 lod 内的 `.pcx`），供下游 `encode_webp` 正确解码。
pub(crate) fn resolve_resource_fallback(mod_path: &Path, game_dir: &Path, res: &str) -> Option<(Vec<u8>, String)> {
    if let Some(r) = resolve_resource(mod_path, res) {
        return Some(r);
    }
    // 埋点：mod 内未命中 → 回退游戏根（会走到 `lod.open` 这条高代价路径）
    crate::infra::metrics::count("res.fallback", 1);
    // VCMI 同时把游戏根 `Data/` 与用户数据 `Data/` **都**挂载进虚拟 `Data/` 命名空间，二者都应参与
    // 资源解析（便携运行原版数据常在 `user_data_dir/Data`）。故两个目录都试，而非"二选一"——
    // 否则游戏根仅有部分/HD 版 LOD 时，会漏掉真正的原版精灵。
    // 命中 mod 内资源时根本不走这里；且任一目录命中即返回，正常（游戏根已含数据）构建零额外开销。
    if let Some(r) = crate::infra::lod::read_game_resource_ext(game_dir, res) {
        return Some(r);
    }
    if let Some(ud) = wiki_user_data_dir() {
        if !ud.as_os_str().is_empty() {
            if let Some(r) = crate::infra::lod::read_game_resource_ext(&ud, res) {
                return Some(r);
            }
        }
    }
    // 跨模组检索：对齐 VCMI `findResourceOrigin` —— 全局遍历所有已装模组，首个命中即返回。
    // 仅当本模组与游戏根均未命中才进入（常见图标都在前面短路），对正常构建零额外开销；
    // 真正跨模组引用的素材（如子模组 Fen 的 `Sprites/fen/creatures/...` 被父模组配置引用）由此补到。
    if let Some(all) = wiki_mod_paths() {
        for mp in all.iter() {
            if mp.as_path() == mod_path {
                continue; // 本模组已查过
            }
            if let Some(r) = resolve_resource(mp, res) {
                crate::infra::metrics::count("res.hit.crossmod", 1);
                return Some(r);
            }
        }
    }
    None
}

/// 读取游戏级资源字节（同 `lod::read_game_resource`），但额外回退到 `user_data_dir/Data` 的
/// LOD/散文件——VCMI 把游戏根与用户数据 `Data/` 都挂载进虚拟 `Data/`，便携运行原版 LOD 常在
/// `user_data_dir/Data`。原版资源图标（SMALRES/SPELLBON/PortraitsLarge/stackWindow/UN32/UN44…）
/// 都在原版 LOD 内，若只搜 `game_dir/Data` 会在便携场景下全部落空、静默无图。
pub(crate) fn read_game_resource_fallback(game_dir: &Path, name: &str) -> Option<Vec<u8>> {
    if let Some(r) = crate::infra::lod::read_game_resource_ext(game_dir, name) {
        return Some(r.0);
    }
    if let Some(ud) = wiki_user_data_dir() {
        if !ud.as_os_str().is_empty() {
            if let Some(r) = crate::infra::lod::read_game_resource_ext(&ud, name) {
                return Some(r.0);
            }
        }
    }
    None
}

/// 原版 H3 数据存在性判定缓存（见 `h3_data_present` / `h3_data_key`）。
///
/// 结果按 (game_dir, user_data_dir) 缓存，整批构建只计算一次；
/// 之后每次资源回退解析仅一次哈希表查询，对正常（数据已就位）构建路径零额外开销。
/// 批量/详情构建期当前的用户数据目录（用于扩展原版数据回退搜索到 `user_data_dir/Data`）。
/// 由 wiki 构建编排入口在每批开始前设置一次；详情页 `read_def_bytes` 从 `VcmiPaths` 取同值设置。
static WIKI_USER_DATA_DIR: Mutex<Option<PathBuf>> = Mutex::new(None);

/// 设置当前构建的用户数据目录（见 `WIKI_USER_DATA_DIR`）。
pub(crate) fn set_wiki_user_data_dir(dir: &Path) {
    if let Ok(mut g) = WIKI_USER_DATA_DIR.lock() {
        *g = Some(dir.to_path_buf());
    }
}

/// 读取当前构建的用户数据目录（未设置则返回空路径，回退逻辑退化到仅游戏根）。
pub(crate) fn wiki_user_data_dir() -> Option<PathBuf> {
    WIKI_USER_DATA_DIR.lock().ok().and_then(|g| g.clone())
}

/// 批量/详情构建期**全部已装模组路径**（用于跨模组资源搜索，对齐 VCMI `findResourceOrigin` 的全局遍历）。
///
/// 由 wiki 构建编排入口（batch / index）与详情页 `read_def_bytes` 在每批/每次开始前设置一次；
/// `resolve_resource_fallback` 在「本模组 + 游戏根 + 用户数据」都未命中后，迭代这些路径做跨模组检索。
/// 用 `Arc<Vec<PathBuf>>` 存储：读取时仅原子增引用，避免每次解析都克隆整条路径列表。
static WIKI_MOD_PATHS: Mutex<Option<Arc<Vec<PathBuf>>>> = Mutex::new(None);

/// 设置当前构建的全部模组路径（见 `WIKI_MOD_PATHS`）。
pub(crate) fn set_wiki_mod_paths(paths: &[PathBuf]) {
    if let Ok(mut g) = WIKI_MOD_PATHS.lock() {
        *g = Some(Arc::new(paths.to_vec()));
    }
}

/// 读取当前构建的全部模组路径（未设置则返回 None，跨模组搜索退化为仅本模组 + 游戏根）。
pub(crate) fn wiki_mod_paths() -> Option<Arc<Vec<PathBuf>>> {
    WIKI_MOD_PATHS.lock().ok().and_then(|g| g.clone())
}

/// 判定某数据目录是否含有原版 H3 核心 LOD（bitmap + sprite 是 wiki 图标 / 建筑图 / 生物小图 /
/// 英雄头像的主要来源）。缺 bitmap 则图标与建筑图无源；缺 sprite 则生物小图与英雄头像无源。
pub(crate) fn has_core_h3_lods(data_dir: &Path) -> bool {
    let d = data_dir.join("Data");
    if !d.is_dir() {
        return false;
    }
    d.join("H3Bitmap.lod").is_file() && d.join("H3Sprite.lod").is_file()
}

/// 原版数据是否存在：游戏根 `Data/` 或用户数据 `Data/` 任一含有核心 LOD 即算存在。
///
/// VCMI 把两者**都**挂载进虚拟 `Data/` 命名空间（见 `resolve_resource_fallback`），故二者都应
/// 参与资源解析，不能"二选一"只取其一——否则当原版数据放在 `user_data_dir/Data`、而
/// `game_dir/Data` 仅有部分/HD 版 LOD 时，会误判游戏根"有数据"而漏掉真正的原版精灵。
pub(crate) fn h3_data_present(game_dir: &Path, user_data_dir: &Path) -> bool {
    has_core_h3_lods(game_dir)
        || (!user_data_dir.as_os_str().is_empty() && has_core_h3_lods(user_data_dir))
}

/// 原版数据状态指纹（缓存，避免正常构建每资源重复探测）：用于缓存失效判定。
/// - 任一处含核心 LOD → `"present"`；
/// - 两处皆无 → `"missing"`。
/// 用户补放 / 移除数据文件时状态变化，触发一次缓存重建（无需人工清缓存）；
/// 数据稳定时指纹不变，零重建——对正常构建无任何额外开销。
static H3_DATA_PRESENT_CACHE: OnceLock<Mutex<Vec<(String, bool)>>> = OnceLock::new();
const H3_DATA_PRESENT_CACHE_CAP: usize = 16;

pub(crate) fn h3_data_key(game_dir: &Path, user_data_dir: &Path) -> String {
    let key = format!(
        "{}|{}",
        game_dir.to_string_lossy(),
        user_data_dir.to_string_lossy()
    );
    let list = H3_DATA_PRESENT_CACHE.get_or_init(|| Mutex::new(Vec::new()));
    if let Ok(guard) = list.lock() {
        if let Some((_, v)) = guard.iter().find(|(k, _)| k == &key) {
            return if *v { "present".to_string() } else { "missing".to_string() };
        }
    }
    let present = h3_data_present(game_dir, user_data_dir);
    if let Ok(mut guard) = list.lock() {
        guard.push((key, present));
        if guard.len() > H3_DATA_PRESENT_CACHE_CAP {
            guard.remove(0);
        }
    }
    if present { "present".to_string() } else { "missing".to_string() }
}

/// 去除 VCMI 资源根前缀（Data/Sprites/Maps/...），保留相对路径。
pub(crate) fn strip_resource_prefix(p: &str) -> String {
    let prefixes = ["data/", "sprites/", "maps/", "config/", "fonts/", "video/", "sound/"];
    let lower = p.to_lowercase();
    for pre in prefixes {
        if let Some(stripped) = lower.strip_prefix(pre) {
            return stripped.to_string();
        }
    }
    p.to_lowercase()
}

/// 在目录树中查找与资源 key 匹配的条目（返回绝对路径）。
///
/// 匹配口径：相对 `dir` 的路径（`Data/...` 位于开头）经 `strip_resource_prefix`
/// 去除 Data/Sprites/ 等根前缀，再去扩展名、转小写后与 key 比较（见 `norm_zip_key`）。
///
/// ⚠️ 已知热点：本函数**每次资源解析都全量递归一次 `Content/`**（无缓存），
/// 而 `Content/` 常有数千个文件，代价随「资源数 × 文件数」增长。
/// `res.dir_scan` 埋点用于量化其在整批构建中的占比（见 `infra::metrics`）。
pub(crate) fn find_in_dir(dir: &Path, key: &str) -> Result<PathBuf, ()> {
    crate::infra::metrics::scope("res.dir_scan", || find_in_dir_inner(dir, key))
}

fn find_in_dir_inner(dir: &Path, key: &str) -> Result<PathBuf, ()> {
    // 走**目录索引**（每个目录只递归一次）：原实现每次调用都全量遍历一遍 `Content/`，
    // 而 `Content/` 常有数千文件，代价随「资源数 × 文件数」增长（实测 ≈360s CPU）。
    dir_index_of(dir).get(key).cloned().ok_or(())
}

pub(crate) fn strip_ext(p: &str) -> Option<String> {
    // 返回去掉最后一个扩展名后的路径（含点）
    if let Some(pos) = p.rfind('.') {
        Some(p[..pos].to_string())
    } else {
        None
    }
}

/// 跨 mod 定位并返回 `.def` 动画文件的原始字节，供前端直接解码绘制。
/// 取消落盘缓存方案后，动画改为"按需读取 + 前端 TS 直解"，零磁盘占用、无预生成耗时。
/// Tauri 会把 `Vec<u8>` 作为 `Uint8Array` 传给前端。
/// 读取 `.def` 资源的原始字节（供前端详情页按需解码播放动画）。
pub(crate) fn read_def_bytes_inner(
    paths: &VcmiPaths,
    mod_id: &str,
    resource: &str,
) -> Result<Vec<u8>, String> {
    let all = cached_mod_paths(paths);
    // 详情页资源回退同款：让 `resolve_resource_fallback` 也能搜到 `user_data_dir/Data`
    set_wiki_user_data_dir(&paths.user_dir);
    // 详情页跨模组资源检索：让 `resolve_resource_fallback` 也能搜到其它已装模组内的素材
    set_wiki_mod_paths(&all);
    // 优先请求方自身：详情页动画总是取本模组的资源。
    // - `core` / `core.<faction>` → 游戏目录 `Mods/vcmi`（原版 DEF 实际所在）
    // - 其他 → 与 modId 末段同名的模组目录（嵌套子 mod 的 id 末段即磁盘目录名）
    // 否则会从 600+ 个模组里逐个打开 content.zip 找同一个资源，单次悬停即明显卡顿。
    let leaf = mod_id.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let mut ordered: Vec<PathBuf> = Vec::with_capacity(2);
    if mod_id == "core" || mod_id.starts_with("core.") {
        ordered.push(paths.game_dir.join("Mods").join("vcmi"));
    }
    if !leaf.is_empty() {
        for p in all.iter() {
            if p.file_name()
                .map(|n| n.to_string_lossy().to_ascii_lowercase() == leaf)
                .unwrap_or(false)
            {
                ordered.push(p.clone());
            }
        }
    }
    for mp in ordered.iter().chain(all.iter()) {
        // resolve_resource 只搜 mod 自身；原版生物战斗动画（如 CPKMAN.DEF）实际在
        // 游戏根 Data/*.lod 内，故加游戏目录回退，与背景/图标资源一致。
        if let Some((bytes, _ext)) = resolve_resource_fallback(mp, &paths.game_dir, resource) {
            return Ok(bytes);
        }
    }
    Err(be_err("vcmi_resource_not_found", &format!("{}", resource)))
}
