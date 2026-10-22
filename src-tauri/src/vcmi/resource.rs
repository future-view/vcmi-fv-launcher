//! 模组资源解析：按资源名从 mod 目录 / `content.zip` / 游戏 LOD 取字节。
//!
//! 统一处理 VCMI 的三种资源来源（优先级从高到低）：
//! 1. mod 目录内的 `Content/**`（未打包，直接文件）；
//! 2. mod 的 `content.zip`（带缓存索引 `ZipIndex`；支持经 `launcher.json` 的 `archives.zip_passwords` 密码解包）；
//! 3. 游戏根 `Data/*.lod`（`resolve_resource_fallback` 的回退路径）。
//!
//! 依赖方向：只依赖 `infra`（见 `docs/architecture/backend.zh-CN.md` §4.1）。
//!
//! **命中规则（2026-10-13 起）**：请求**显式带扩展名**时优先命中间目录/zip 内**同扩展名**的
//! 文件，未命中才回落到"去扩展名"的宽松匹配（保留 VCMI 配置写错扩展名时的兼容能力）。
//! 原因见 [`ZipIndex::by_ext`]：sprites-json 的 `X.json` 定义与 `X.png` 位图帧同基名，
//! 去扩展名口径下会撞 key，只按宽松 key 取会拿到定义 JSON 的字节、使位图解码失败。

use serde_json::{json, Value};

use crate::infra::berror::be_err;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::vcmi::paths::VcmiPaths;
use crate::vcmi::scan::ModEntry;

/// 单个 content.zip 的条目索引。
#[derive(Default)]
pub(crate) struct ZipIndex {
    /// 归一 key（去前缀 + 去扩展名 + 小写）→ 条目序号（同 key 多文件时保留首个）
    by_key: HashMap<String, usize>,
    /// 归一 key **含扩展名**（去前缀 + 小写）→ 条目序号。
    ///
    /// 存在的理由：同基名不同扩展名的文件（VCMI 1.6+ sprites-json 的标准写法是
    /// 定义 `X.json` + 位图帧 `X.png`）经 `norm_zip_key` 去扩展名后会**撞成同一 key**，
    /// 只按 `by_key` 命中会取到先遍历到的那个（实测总是 `.json`）→ 位图解码失败。
    by_ext: HashMap<String, usize>,
    /// 条目序号 → 扩展名（小写，与原实现的取值口径一致）
    ext_of: HashMap<usize, String>,
    /// 是否含加密条目（只读中央目录元数据即可判定，无需密码）
    has_encrypted: bool,
}

pub(crate) static ZIP_INDEX_CACHE: std::sync::OnceLock<
    Mutex<Vec<(String, std::sync::Arc<ZipIndex>)>>,
> = std::sync::OnceLock::new();

/// 索引条目多（单 mod 可达数千），容量按 mod 数量级给，避免超线性增长
pub(crate) const ZIP_INDEX_CACHE_CAP: usize = 64;

/// `Content/` 目录索引：`by_key` 用于宽松匹配（配置写的扩展名与实际不符时仍能命中），
/// `by_ext` 用于请求显式带扩展名时精确命中——见 [`ZipIndex::by_ext`] 的说明（同一撞 key 问题）。
///
/// ⚠️ 2026-10-16 内存瘦身：
/// - 两张表改为共享 `Arc<Path>`：原实现各存一份 `PathBuf` 副本，12.6 万条目 × 2 = 两份路径分配；
/// - `by_ext` **只登记"同基名存在多个扩展名"的条目**：只有一个文件时 `by_ext` 命中与
///   `by_key` 命中本就指向同一路径，登记它纯属白占一份内存（实测可省约 25~30 MB）。
#[derive(Default)]
pub(crate) struct DirIndex {
    by_key: HashMap<String, Arc<Path>>,
    by_ext: HashMap<String, Arc<Path>>,
    /// `by_key` 当前存的是 `config/` 下**定义文件**的 stem（见 [`build_dir_index`]）。
    /// 这些 stem 一旦后来出现真正的素材文件，就要被它顶掉。
    cfg_key: HashSet<String>,
}

/// `Content/` 目录索引缓存（目录路径 → (指纹, 索引)）。
///
/// 原实现每解析一个资源就**全量递归一次 `Content/`**（实测 10,419 次 × 45ms ≈ 360s CPU）；
/// 改为"每个目录只建一次索引"。
///
/// ⚠️ 2026-10-15 修订（首次全量构建实测 103 分钟的根因之一）：
/// 旧结构是 `Vec<(指纹串, Arc<DirIndex>)>` —— **线性查找 + 容量 64**，而单个发行版常有
/// 500+ 个模组（`resolve_resource_fallback` 的跨模组回退会遍历全部），容量远小于工作集
/// → 索引被反复淘汰重建（实测 291,358 次 `res.dir_scan` × 21.6ms ≈ 6,286s CPU，占整批
/// CPU 的 76%）。改为 `HashMap`（O(1) 命中）+ 容量按模组数量级给。
/// 指纹**只取 mtime**：顶层条目数需 `read_dir` 枚举，热路径每次命中都做会明显拖慢
/// （目录 mtime 在条目增删改名时即变化，对本用途已足够）。
/// 目录索引缓存的内部结构（`clippy::type_complexity` 要求拆出别名）。
type DirIndexCache = Mutex<HashMap<PathBuf, (u128, std::sync::Arc<DirIndex>)>>;
pub(crate) static DIR_INDEX_CACHE: std::sync::OnceLock<DirIndexCache> = std::sync::OnceLock::new();
/// 容量按「模组数量级」给（实测发行版 572 个模组 + 子模组），远超旧值 64。
pub(crate) const DIR_INDEX_CACHE_CAP: usize = 4096;

/// 目录指纹：mtime（纳秒）。取不到时返回 0（等价于"总是当作已变化"）。
fn dir_mtime(dir: &Path) -> u128 {
    fs::metadata(dir)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// 模组「内容载体」缓存：`Content/` 目录 / `content.zip` 是否存在。
///
/// 跨模组回退**每解析一个资源都要遍历全部模组**，若每次都 `is_dir()` / `exists()`
/// 就是「资源数 × 模组数」次 stat（实测 29 万次）。缓存后每个模组只探测一次。
static MOD_KIND_CACHE: OnceLock<Mutex<HashMap<PathBuf, (bool, bool)>>> = OnceLock::new();

/// 返回 `(有 Content 目录, 有 content.zip)`。
fn mod_content_kind(mod_path: &Path) -> (bool, bool) {
    let list = MOD_KIND_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(g) = list.lock() {
        if let Some(v) = g.get(mod_path) {
            return *v;
        }
    }
    let v = (
        mod_path.join("Content").is_dir(),
        mod_path.join("content.zip").is_file(),
    );
    if let Ok(mut g) = list.lock() {
        if g.len() >= DIR_INDEX_CACHE_CAP {
            g.clear();
        }
        g.insert(mod_path.to_path_buf(), v);
    }
    v
}

/// 资源文件字节缓存（按字节上界淘汰，上界由 [`crate::infra::mem::cache_budget`] 按机器内存分档）。
///
/// ⚠️ 必要性（2026-10-16）：一座城镇约 30 个建筑共引用同一个 `town-hall.def`，而**每个**
/// 建筑图标提取都会 `resolve_resource` → `fs::read` 整份文件（DEF 常几百 KB~数 MB）
/// → 同一份字节被反复读 30 次。全库放大后是几十 GB 级的重复读，以及同样次数的 DEF 头解析。
///
/// key 含 `mtime + 长度`：文件内容变了自然失效（不读内容即可判定，无需哈希）。
/// 超过 [`FILE_BYTES_MAX_ITEM`] 的单文件**不进缓存**，免得一条吃掉整个预算。
type FileBytesKey = (PathBuf, u128, u64);
static FILE_BYTES_CACHE: OnceLock<
    Mutex<crate::infra::lru::SizedCache<FileBytesKey, Arc<Vec<u8>>>>,
> = OnceLock::new();
/// 单条超过此体积不缓存（LOD 内的大动画、整包位图等）
const FILE_BYTES_MAX_ITEM: u64 = 8 * 1024 * 1024;

/// 读文件内容（带缓存）；缓存未命中 / 不缓存时直接读盘。
fn read_file_cached(p: &Path) -> std::io::Result<Vec<u8>> {
    let (len, mt) = match fs::metadata(p) {
        Ok(m) => (
            m.len(),
            m.modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_nanos())
                .unwrap_or(0),
        ),
        Err(_) => (0, 0),
    };
    let cap = crate::infra::mem::cache_budget().file_bytes;
    let cache =
        FILE_BYTES_CACHE.get_or_init(|| Mutex::new(crate::infra::lru::SizedCache::new(cap)));
    if len == 0 || len > FILE_BYTES_MAX_ITEM {
        return fs::read(p);
    }
    let key = (p.to_path_buf(), mt, len);
    if let Ok(g) = cache.lock() {
        if let Some(v) = g.get(&key) {
            crate::infra::metrics::count("res.bytes.cached", 1);
            return Ok(v.as_ref().clone());
        }
    }
    let data = Arc::new(fs::read(p)?);
    if let Ok(mut g) = cache.lock() {
        g.insert(key, data.len(), data.clone());
    }
    Ok(data.as_ref().clone())
}

/// 释放资源解析侧的进程级缓存（构建全部结束后调用；见 [`crate::infra::cache_gc`]）。
///
/// 只清**可重建**的派生缓存：zip 索引 / 文件字节 / 正负结果 / 载体探测。
/// 它们都按「路径 + 指纹」自失效，清掉只会让下次构建多付一次扫描代价，不影响正确性。
///
/// ⚠️ **刻意保留目录索引**（`DIR_INDEX_CACHE`）：它是"每个模组 `Content/` 的递归清单"，
/// 569 模组 / 14 万文件。`anim.kind = def` 只覆盖"资源在自己模组里"的常规生物；
/// 跨模组引用 / 旧缓存 / 完全未命中的资源仍会走 `resolve_core` 的兜底链，
/// 索引一清就要重建全部清单 —— 实测完全未命中的资源首次 9.05s（其中 7.9s 是重建索引）。
/// 索引常驻约 20~40MB，这个交换明显划算。
pub(crate) fn release_caches() {
    // 通用：清空「Mutex<集合>」并收缩底层容器
    macro_rules! clear_and_shrink {
        ($c:expr) => {
            if let Some(c) = $c.get() {
                if let Ok(mut g) = c.lock() {
                    g.clear();
                    g.shrink_to_fit();
                }
            }
        };
    }
    clear_and_shrink!(ZIP_INDEX_CACHE);
    clear_and_shrink!(MOD_KIND_CACHE);
    clear_and_shrink!(CROSSMOD_MISS);
    clear_and_shrink!(CROSSMOD_HIT);
    clear_and_shrink!(H3_DATA_PRESENT_CACHE);
    // 字节缓存是 `SizedCache`（自建结构，`clear()` 内部已收缩）
    if let Some(c) = FILE_BYTES_CACHE.get() {
        if let Ok(mut g) = c.lock() {
            g.clear();
        }
    }
}

/// 跨模组回退的**负结果缓存**：已确认「全部其它模组都没有」的资源 key。
///
/// 典型场景：建筑图标声明了 `buildingsIcons`，但帧号在文件里根本不存在（实测 299 个），
/// 每个都要把几百个模组扫一遍，而且**永远扫不出结果**。记下后同类请求直接跳过。
///
/// 失效：随 [`set_wiki_mod_paths`]（模组集合变化——新装模组 / 切换领域）整体清空，
/// 否则新装的模组里的资源会被永久误判为"不存在"。
static CROSSMOD_MISS: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

/// 跨模组回退的**命中结果缓存**：资源 key → 上次命中的 `(模组磁盘路径, modId)`。
///
/// 为什么必须有：跨模组检索是「按 modId 字典序遍历，首个命中即返回」。对"资源确实躺在某个
/// 模组里"的请求（第三方兵种 DEF 全是这种），每次解析都要重新扫到那个模组才命中；而
/// `anim_json::locate` 又会为"这是不是 sprites-json 定义"逐个模组调解析 → 命中不写负缓存
/// ⇒ O(N²)（实测 569 模组 = 单次动画分组请求 24.8 万次解析 / 16 秒）。
///
/// 失效口径与 [`CROSSMOD_MISS`] 完全一致（模组集合变化时清空）：命中归属由遍历顺序决定，
/// 只有集合本身变了才可能改变。
static CROSSMOD_HIT: OnceLock<Mutex<HashMap<String, (PathBuf, String)>>> = OnceLock::new();

/// 构建目录索引：走 [`crate::infra::dirscan::dir_listing`]（每个目录只递归一次，
/// 与配置收集共用同一份清单），不再自己 `read_dir`。
///
/// 遍历顺序与命中语义与原实现一致：深度优先 + `or_insert` 保留首个匹配
/// （两个文件归一后撞 key 时由遍历顺序决定胜出者，换序会静默改变解析结果）。
fn build_dir_index(dir: &Path) -> DirIndex {
    let listing = crate::infra::dirscan::dir_listing(dir);
    let mut out = DirIndex::default();
    // ① 先建 `by_key` 并统计每个基名下的条目数
    let mut stem_count: HashMap<String, usize> = HashMap::new();
    // (含扩展名 key, 去扩展名 key, 路径)
    let mut pending: Vec<(String, String, Arc<Path>)> = Vec::with_capacity(listing.len());
    for i in 0..listing.len() {
        let rel = listing.rel(i);
        let kx = norm_key_with_ext(rel);
        let k = norm_zip_key(rel).unwrap_or_else(|| kx.clone());
        // ⚠️ 2026-10-16 修正：**`config/` 下的定义文件不得抢占素材的 stem**。
        // 索引根是 `<mod>/Content`，而 `config/<...>/<id>.json` 与 `sprites/<...>/<id>.png`
        // 经 `strip_resource_prefix` 去掉各自根前缀后**撞成同一个 key**
        // （实测 `lianjin/artifacts/ringOfTheCobrasEye` 同时命中 config 的 .json、
        // sprites 的 .bmp 与 .def）。遍历顺序是 `config` 在 `sprites` 之前 → 定义 JSON 胜出
        // → 调用方拿到的是**配置文件的字节**，`encode_webp` 必然失败 → `image` 恒为 null
        // （实测：法术 8 条、宝物 8 条"资源明明在盘却无图"）。
        // 处理：config 条目照旧登记（sprites-json 定义也可能放在 config 下），但**后来者**
        // 只要不是 config 条目就把前者的位置顶掉。
        let is_cfg = rel.to_lowercase().starts_with("config/");
        match out.by_key.entry(k.clone()) {
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(listing.path(i).clone());
                if is_cfg {
                    out.cfg_key.insert(k.clone());
                }
            }
            std::collections::hash_map::Entry::Occupied(mut e) => {
                if !is_cfg && out.cfg_key.contains(&k) {
                    e.insert(listing.path(i).clone());
                    out.cfg_key.remove(&k);
                }
            }
        }
        *stem_count.entry(k.clone()).or_insert(0) += 1;
        pending.push((kx, k, listing.path(i).clone()));
    }
    // ② 只为"同基名多个扩展名"的条目登记 `by_ext`（单扩展名时 by_key 命中已等价）
    for (kx, k, p) in pending {
        if stem_count.get(&k).copied().unwrap_or(0) > 1 {
            out.by_ext.entry(kx).or_insert(p);
        }
    }
    out
}

/// 取（并缓存）目录索引。
///
/// ⚠️ **不得持锁构建索引**：`walk_index` 是对整个 `Content/` 的全量递归（大模组数千文件），
/// 持着全局 Mutex 做会把所有工作线程串行化（旧实现的隐性瓶颈，与容量失配叠加后
/// 表现为「并发度越高越没收益」）。故：先查（持锁，极短）→ 未命中则**释放锁**再建 → 再插回。
fn dir_index_of(dir: &Path) -> std::sync::Arc<DirIndex> {
    let mt = dir_mtime(dir);
    let list = DIR_INDEX_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    {
        let guard = list.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((m, v)) = guard.get(dir) {
            if *m == mt {
                return v.clone();
            }
        }
    }
    let v = std::sync::Arc::new(build_dir_index(dir));
    if let Ok(mut guard) = list.lock() {
        if guard.len() >= DIR_INDEX_CACHE_CAP {
            guard.clear();
        }
        guard.insert(dir.to_path_buf(), (mt, v.clone()));
    }
    v
}

/// 条目名归一：去资源根前缀 + 去扩展名 + 小写（zip 索引与目录索引**共用同一口径**）。
pub(crate) fn norm_zip_key(entry_name: &str) -> Option<String> {
    strip_ext(&strip_resource_prefix(
        &entry_name.replace('\\', "/").to_lowercase(),
    ))
}

/// 条目名归一但**保留扩展名**：去资源根前缀 + 小写。
///
/// 与 [`norm_zip_key`] 只差最后一步 `strip_ext`，用于「请求显式带扩展名」时的精确命中
/// （同基名的 `X.json` / `X.png` 在去扩展名口径下不可区分，见 [`ZipIndex::by_ext`]）。
fn norm_key_with_ext(entry_name: &str) -> String {
    strip_resource_prefix(&entry_name.replace('\\', "/").to_lowercase())
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
    let Ok(file) = fs::File::open(&zip_path) else {
        return false;
    };
    let Ok(mut archive) = zip::ZipArchive::new(file) else {
        return false;
    };
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
///
/// 命中顺序与 [`resolve_resource`] 一致：先"含扩展名精确命中"（`X.json`/`X.png` 同基名撞 key
/// 时唯一可靠），未命中再回落去扩展名的宽松匹配。
pub(crate) fn zip_resource_index(mod_path: &Path, resource: &str) -> Option<usize> {
    let full = format!("Content/{}", resource);
    let idx = zip_index_of(mod_path)?;
    if let Some(i) = idx.by_ext.get(&norm_key_with_ext(&full)) {
        return Some(*i);
    }
    idx.by_key.get(&norm_zip_key(&full)?).copied()
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
        idx.by_ext.entry(norm_key_with_ext(&name)).or_insert(i);
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

/// 只解析资源的**实际扩展名**，不读文件字节（构建期登记动画指针用）。
///
/// 与 [`resolve_resource`] 同口径（同一套目录 / zip 索引与命中顺序），只是跳过读字节：
/// 构建期为数千个生物登记 `graphics.animation` 指针时，若每个都读几百 KB 的 DEF，
/// 会额外付出 GB 级 IO —— 而这里只需要回答"有没有、是什么扩展名"。
pub(crate) fn resolve_resource_ext(mod_path: &Path, resource: &str) -> Option<String> {
    let resource = resource.trim();
    if resource.is_empty() {
        return None;
    }
    let (key, key_with_ext) = resource_keys(resource);
    let content_dir = mod_path.join("Content");
    if content_dir.is_dir() {
        if let Ok(found) = find_in_dir(&content_dir, &key, &key_with_ext) {
            return Some(
                found
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase(),
            );
        }
    }
    let i = zip_resource_index(mod_path, resource)?;
    zip_entry_ext(mod_path, i)
}

/// 资源引用 → `(去扩展名 key, 含扩展名 key)`，与磁盘/zip 索引**同一归一口径**。
///
/// 抽出来是为了让 `resolve_resource_fallback` 的负结果缓存能用**完全相同**的 key 记账，
/// 避免两处各写一份归一逻辑而漂移（漂移会让负缓存误伤命中）。
fn resource_keys(resource: &str) -> (String, String) {
    let norm = resource.trim().trim_start_matches('/').replace('\\', "/");
    // 资源名 key：去常见根前缀 + 去扩展名 + 小写（保证非 None）
    let key = strip_resource_prefix(&norm);
    let key = strip_ext(&key)
        .unwrap_or_else(|| key.clone())
        .to_lowercase();
    // 含扩展名的归一 key：请求**本身带扩展名**时优先按它精确命中。
    //
    // 必要性（2026-10-13 定位）：VCMI 1.6+ sprites-json 的定义与位图帧同基名不同扩展名
    // （`X.json` + `X.png`），二者经 `norm_zip_key` 去扩展名后**撞成同一个 key**
    // （`by_key.entry(k).or_insert(...)` 保留遍历到的第一个，实测总是 `.json`）→
    // 请求 `X.png` 却拿到定义 JSON 的字节 → `image::load_from_memory` 失败 →
    // `load_structure_frame` 对每个结构都返回 None → 城镇幕布图**只剩裸底图**、建筑全部消失。
    // 请求不带扩展名时该 key 与 `key` 同形（`by_ext` 中不存在该 key → 零副作用）。
    let key_with_ext = norm_key_with_ext(&norm);
    (key, key_with_ext)
}

/// 资源的**文件名安全标识**（供百科缓存给产物命名）。
///
/// 取 `resource_keys` 的 `key_with_ext`（去虚拟前缀、含扩展名、小写）再做文件名安全化，
/// 于是**命名键 == 解析键**：
/// - 不同资源必不同名 —— 不会重演 basename 撞名（实测 hota 的
///   `artifactsCampaignBonuses/capeOfSilence.png` 与 `artifactsIcons/capeOfSilence.png`
///   内容完全不同却同名，共 173 组）；
/// - 同一资源必同名 —— 被 N 个条目引用也只产出一个文件。
///
/// ⚠️ 安全化把 `/` 与 `.` 都映射成 `_`，理论上存在「`a/b` 与 `a_b` 撞名」的单射性瑕疵。
/// 实测资源名中 `~` 极少见，故保留 `-`/`_`/字母数字，其余一律 `_`；撞名由落盘哨兵告警兜住
/// （见 `modcache::assets::note_asset_collision`），不会静默错图。
pub(crate) fn resource_asset_key(res: &str) -> String {
    let (_, key_with_ext) = resource_keys(res);
    key_with_ext
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn resolve_resource_inner(mod_path: &Path, resource: &str) -> Option<(Vec<u8>, String)> {
    let resource = resource.trim();
    if resource.is_empty() {
        return None;
    }
    let (key, key_with_ext) = resource_keys(resource);

    // Content 目录（磁盘）
    let content_dir = mod_path.join("Content");
    if content_dir.is_dir() {
        if let Ok(found) = find_in_dir(&content_dir, &key, &key_with_ext) {
            let ext = found
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_string();
            // 走字节缓存：同一份 DEF/位图被同一 mod 的多个条目引用时只读盘一次
            if let Ok(data) = read_file_cached(&found) {
                crate::infra::metrics::count("res.hit.dir", 1);
                return Some((data, ext));
            }
        }
    }

    // content.zip：走条目索引（每个 mod 只建一次索引，替代原来的全量遍历）
    // 命中顺序同磁盘分支：含扩展名精确命中 → 去扩展名宽松匹配。
    let idx = zip_index_of(mod_path)?;
    let i = idx
        .by_ext
        .get(&key_with_ext)
        .or_else(|| idx.by_key.get(&key))
        .copied()?;
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
/// 素材来源归属：这份素材**真正属于谁**（而非谁在提取它）。
///
/// 用途：百科缓存据此把产物归属到来源方，使同一份素材只落一份盘 ——
/// 游戏级素材 → 归属 `core`；跨模组引用 → 归属被引用方；本模组素材 → 归属自己。
///
/// ⚠️ 刻意用 `SelfMod` 而不是直接带 id：解析层只持有**磁盘路径**，由路径反查 modId
/// 要遍历整张模组表（热路径白付代价）。"归属请求方自己"这层语义留给调用方填它自己的
/// modId —— 调用方本来就知道自己是谁。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    /// 游戏根 `Data/*.lod` 或用户数据 `Data/`（原版 H3 素材）
    Game,
    /// 请求方模组自己（调用方即归属方）
    SelfMod,
    /// 其它已装模组（跨模组引用命中），携带该模组的规范 modId
    Mod(String),
}

pub(crate) fn resolve_resource_fallback(
    mod_path: &Path,
    game_dir: &Path,
    res: &str,
) -> Option<(Vec<u8>, String)> {
    resolve_core(mod_path, game_dir, res).map(|(b, e, _)| (b, e))
}

/// 与 [`resolve_resource_fallback`] 同结果，但额外给出**素材来源归属**（见 [`Origin`]）。
///
/// 供百科缓存的落盘入口使用：产物归属由来源决定，而非由提取调用方决定。
pub(crate) fn resolve_resource_origin(
    mod_path: &Path,
    game_dir: &Path,
    res: &str,
) -> Option<(Vec<u8>, String, Origin)> {
    resolve_core(mod_path, game_dir, res)
}

/// 资源解析的**唯一实现**（三级回退：本模组 → 游戏根 → 跨模组），附带来源归属。
///
/// ⚠️ 单一实现是硬要求：两个对外入口（[`resolve_resource_fallback`] /
/// [`resolve_resource_origin`]）共用它。两处各写一份归一口径必然漂移 ——
/// [`resource_keys`] 的文档记过这类漂移的后果（负缓存误伤命中）。
fn resolve_core(
    mod_path: &Path,
    game_dir: &Path,
    res: &str,
) -> Option<(Vec<u8>, String, Origin)> {
    if let Some((b, e)) = resolve_resource(mod_path, res) {
        return Some((b, e, Origin::SelfMod));
    }
    // 埋点：mod 内未命中 → 回退游戏根（会走到 `lod.open` 这条高代价路径）
    crate::infra::metrics::count("res.fallback", 1);
    // VCMI 同时把游戏根 `Data/` 与用户数据 `Data/` **都**挂载进虚拟 `Data/` 命名空间，二者都应参与
    // 资源解析（便携运行原版数据常在 `user_data_dir/Data`）。故两个目录都试，而非"二选一"——
    // 否则游戏根仅有部分/HD 版 LOD 时，会漏掉真正的原版精灵。
    // 命中 mod 内资源时根本不走这里；且任一目录命中即返回，正常（游戏根已含数据）构建零额外开销。
    if let Some((b, e)) = crate::infra::lod::read_game_resource_ext(game_dir, res) {
        return Some((b, e, Origin::Game));
    }
    if let Some(ud) = wiki_user_data_dir() {
        if !ud.as_os_str().is_empty() {
            if let Some((b, e)) = crate::infra::lod::read_game_resource_ext(&ud, res) {
                return Some((b, e, Origin::Game));
            }
        }
    }
    // 跨模组检索：对齐 VCMI `findResourceOrigin` —— 全局遍历所有已装模组，首个命中即返回。
    // 仅当本模组与游戏根均未命中才进入（常见图标都在前面短路），对正常构建零额外开销；
    // 真正跨模组引用的素材（如子模组 Fen 的 `Sprites/fen/creatures/...` 被父模组配置引用）由此补到。
    //
    // 历史（2026-10-15 已解决，留档避免重复踩坑）：本节曾是全量构建的头号开销
    // （291,358 次 `res.dir_scan` ≈ 6,286s CPU，占整批 76%）—— 根因是 `DIR_INDEX_CACHE`
    // 当时是 `Vec` + 容量 64，容量远小于 500+ 模组的工作集 → 索引反复淘汰重建。
    // 改为 `HashMap` + 容量 4096 后单次已是 O(1)，本节不再是瓶颈。
    // 现存的两道闸门保留（仍防"扫多少次都扫不出结果"的那 299 个不存在帧号）：
    //   ① 负结果缓存 —— 扫空过一次的 key 直接跳过（同类请求在同一批里大量重复）；
    //   ② 模组载体缓存 —— 没有 `Content/` 也没有 `content.zip` 的模组连 stat 都不做。
    if let Some(all) = wiki_mod_paths() {
        let (key, key_with_ext) = resource_keys(res);
        let miss_key = format!("{key_with_ext}\u{1}{key}");
        {
            let set = CROSSMOD_MISS.get_or_init(|| Mutex::new(HashSet::new()));
            if let Ok(g) = set.lock() {
                if g.contains(&miss_key) {
                    crate::infra::metrics::count("res.miss.cached", 1);
                    return None;
                }
            }
        }
        // 正缓存：上次命中的模组直接复用。**必须**：命中时下面不会写负缓存，而详情页
        // 每个动画分组请求都会为"这是不是 json 定义"逐个模组做解析（见 `CROSSMOD_HIT`）。
        {
            let hit = CROSSMOD_HIT.get_or_init(|| Mutex::new(HashMap::new()));
            let cached = hit
                .lock()
                .ok()
                .and_then(|g| g.get(&miss_key).cloned());
            if let Some((mp, mid)) = cached {
                if let Some((b, e)) = resolve_resource(&mp, res) {
                    crate::infra::metrics::count("res.hit.cached", 1);
                    return Some((b, e, Origin::Mod(mid)));
                }
            }
        }
        for (mp, mid) in all.iter() {
            if mp.as_path() == mod_path {
                continue; // 本模组已查过
            }
            let (has_dir, has_zip) = mod_content_kind(mp);
            if !has_dir && !has_zip {
                continue;
            }
            if let Some((b, e)) = resolve_resource(mp, res) {
                crate::infra::metrics::count("res.hit.crossmod", 1);
                // 记下命中归属：同一资源的后续解析（如逐分组取动画帧）直接短路到该模组
                if let Ok(mut g) = CROSSMOD_HIT
                    .get_or_init(|| Mutex::new(HashMap::new()))
                    .lock()
                {
                    g.insert(miss_key.clone(), (mp.clone(), mid.clone()));
                }
                // 归属直接取命中模组的 modId（无需调用方事后反查路径→id）
                return Some((b, e, Origin::Mod(mid.clone())));
            }
        }
        let set = CROSSMOD_MISS.get_or_init(|| Mutex::new(HashSet::new()));
        if let Ok(mut g) = set.lock() {
            g.insert(miss_key);
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

/// 批量/详情构建期**全部已装模组条目**（用于跨模组资源搜索，对齐 VCMI `findResourceOrigin` 的全局遍历）。
///
/// 条目形如 `(磁盘路径, modId)`（见 [`ModEntry`]）：跨模组命中时靠 `modId` 直接答出素材归属，
/// 无需调用方事后反查路径→id。
///
/// 由 wiki 构建编排入口（batch / index）与详情页 `read_def_bytes` 在每批/每次开始前设置一次；
/// `resolve_resource_fallback` 在「本模组 + 游戏根 + 用户数据」都未命中后，迭代这些条目做跨模组检索。
/// 用 `Arc<Vec<ModEntry>>` 存储：读取时仅原子增引用，避免每次解析都克隆整条列表。
static WIKI_MOD_PATHS: Mutex<Option<Arc<Vec<ModEntry>>>> = Mutex::new(None);

/// 模组集合版本号：集合**实际变化**（见 [`set_wiki_mod_paths`]）时 +1。
///
/// 供上层派生缓存自失效：`anim_json::locate` 的结果只取决于 (modId, resource) 与模组集合，
/// 用本版本号做缓存键即可在换模组 / 切领域后立即失效，无需让 `resource` 层反向依赖上层。
static WIKI_MOD_PATHS_GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// 当前模组集合版本号（见 [`WIKI_MOD_PATHS_GEN`]）。
pub(crate) fn wiki_mod_paths_gen() -> u64 {
    WIKI_MOD_PATHS_GEN.load(std::sync::atomic::Ordering::Relaxed)
}

/// 设置当前构建的全部模组条目（见 `WIKI_MOD_PATHS`）。
///
/// **集合未变时直接返回**（不清任何缓存、不重建 Arc）：详情页每取一个动画分组都会调用本函数
/// （`read_def_bytes_inner` / `anim_json::locate` 各一次），若每次都清 [`CROSSMOD_MISS`] /
/// [`CROSSMOD_HIT`]，跨模组检索的结论永远留不住 —— 实测 569 模组下单次分组请求会因此
/// 重复解析 24.8 万次资源（16 秒）。集合按 `(磁盘路径, modId)` 逐项比较，569 项比较为微秒级。
///
/// 集合变了（新装模组 / 切换领域 / 重扫目录）才清空负缓存与命中缓存：之前"全部模组都没有"
/// 和"命中归属某模组"的结论都不再成立。
pub(crate) fn set_wiki_mod_paths(paths: &[ModEntry]) {
    if let Ok(g) = WIKI_MOD_PATHS.lock() {
        if let Some(prev) = g.as_ref() {
            if prev.len() == paths.len()
                && prev.iter().zip(paths.iter()).all(|(a, b)| a.0 == b.0 && a.1 == b.1)
            {
                return;
            }
        }
    }
    if let Ok(mut g) = WIKI_MOD_PATHS.lock() {
        *g = Some(Arc::new(paths.to_vec()));
    }
    WIKI_MOD_PATHS_GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    clear_crossmod_caches();
    // `bonus_graphics_map` / `spell_immune_icons` 的缓存键只含「游戏目录 + 参与扫描的目录数」，
    // 模组集合变化（数量恰好相同时）无法自失效 → 在此显式清空，与 [`CROSSMOD_MISS`] 同理。
    crate::vcmi::bonus::release_caches();
    crate::vcmi::spell_icons::release_caches();
}

/// 清空跨模组**正/负结果**缓存（[`CROSSMOD_MISS`] / [`CROSSMOD_HIT`]）。
///
/// 模组集合变化时（安装 / 卸载 / 手动重扫，见 `scan::invalidate_mod_paths`）必须清：
/// "全部模组都没有"与"命中归属某模组"的两个结论都以**当时那份模组表**为前提。
pub(crate) fn clear_crossmod_caches() {
    if let Some(c) = CROSSMOD_MISS.get() {
        if let Ok(mut g) = c.lock() {
            g.clear();
            g.shrink_to_fit();
        }
    }
    if let Some(c) = CROSSMOD_HIT.get() {
        if let Ok(mut g) = c.lock() {
            g.clear();
            g.shrink_to_fit();
        }
    }
}

/// 读取当前构建的全部模组条目（未设置则返回 None，跨模组搜索退化为仅本模组 + 游戏根）。
pub(crate) fn wiki_mod_paths() -> Option<Arc<Vec<ModEntry>>> {
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
            return if *v {
                "present".to_string()
            } else {
                "missing".to_string()
            };
        }
    }
    let present = h3_data_present(game_dir, user_data_dir);
    if let Ok(mut guard) = list.lock() {
        guard.push((key, present));
        if guard.len() > H3_DATA_PRESENT_CACHE_CAP {
            guard.remove(0);
        }
    }
    if present {
        "present".to_string()
    } else {
        "missing".to_string()
    }
}

/// 去除 VCMI 资源根前缀（Data/Sprites/Maps/...），保留相对路径。
pub(crate) fn strip_resource_prefix(p: &str) -> String {
    let prefixes = [
        "data/", "sprites/", "maps/", "config/", "fonts/", "video/", "sound/",
    ];
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
/// 去除 Data/Sprites/ 等根前缀后，**先按"含扩展名"的 `key_with_ext` 精确命中**
/// （同基名的 `X.json` 定义与 `X.png` 位图帧在去扩展名口径下不可区分，见
/// [`ZipIndex::by_ext`]），未命中再按去扩展名、转小写的 `key` 宽松匹配（见 `norm_zip_key`）。
///
/// 走**目录索引**（`dir_index_of`，每个目录只递归一次）：原实现每次调用都全量遍历一遍
/// `Content/`，代价随「资源数 × 文件数」增长（实测 ≈360s CPU）。
pub(crate) fn find_in_dir(dir: &Path, key: &str, key_with_ext: &str) -> Result<Arc<Path>, ()> {
    crate::infra::metrics::scope("res.dir_scan", || find_in_dir_inner(dir, key, key_with_ext))
}

fn find_in_dir_inner(dir: &Path, key: &str, key_with_ext: &str) -> Result<Arc<Path>, ()> {
    // 走**目录索引**（每个目录只递归一次）：原实现每次调用都全量遍历一遍 `Content/`，
    // 而 `Content/` 常有数千文件，代价随「资源数 × 文件数」增长（实测 ≈360s CPU）。
    let idx = dir_index_of(dir);
    // ① 请求带扩展名 → 先精确命中（`X.json` / `X.png` 同基名撞 key 时唯一可靠）
    if let Some(p) = idx.by_ext.get(key_with_ext) {
        return Ok(p.clone());
    }
    // ② 回退：去扩展名的宽松匹配（VCMI 允许配置写的扩展名与实际文件不一致）
    idx.by_key.get(key).cloned().ok_or(())
}

/// 去掉**末段**的扩展名（`a/b.png` → `a/b`；`a/b.res/c.png` → `a/b.res/c`；
/// `a/b.res/c` → `None`）。返回 `None` 表示末段本就没有扩展名。
///
/// ⚠️ 2026-10-16 修正：旧实现用 `rfind('.')` 找"最后一个点"，会把**中间段**的点当扩展名 ——
/// `zvs/Lib1.res/E_DEATH`（VCMI `ImagePath` 的 `<file>/<sub>` 写法，资源实体是
/// `Content/data/zvs/Lib1.res/E_DEATH.png`）被截成 `zvs/lib1`，于是**所有 bonus 图标**
/// （真实发行版由 `vcmi-extras` 的 Bonus Icons 子模组提供，`bonuses.json` 的 `graphics.icon`）
/// 恒解析失败：实测 19,376 条能力里只有 1,042 条（5.4%）有图标，且全是有内联
/// `ability["icon"]` 的那批（用户报"所有技能图片均为空图"）。
///
/// 索引侧（[`norm_zip_key`] / `build_dir_index`）与请求侧（[`resource_keys`]）**共用**本函数，
/// 故两侧同时修正、归一口径不漂移。实测该发行版配置里 `<file>.def/<帧号>` 这类
/// "靠旧截断碰巧命中"的引用 **0 条**，改语义不会造成命中回退。
pub(crate) fn strip_ext(p: &str) -> Option<String> {
    let start = p.rfind(['/', '\\']).map(|i| i + 1).unwrap_or(0);
    match p[start..].rfind('.') {
        Some(pos) => Some(p[..start + pos].to_string()),
        None => None,
    }
}

/// 跨 mod 定位并返回 `.def` 动画文件的原始字节，供前端直接解码绘制。
/// 取消落盘缓存方案后，动画改为"按需读取 + 前端 TS 直解"，零磁盘占用、无预生成耗时。
/// Tauri 会把 `Vec<u8>` 作为 `Uint8Array` 传给前端。
/// 读取 `.def` 资源的原始字节（供前端详情页按需解码播放动画）。
/// 资源检索的**优先顺序**：请求方 mod 自己 → 其它已装 mod → （调用方再拼游戏根回退）。
///
/// - `core` / `core.<faction>` → 游戏目录 `Mods/vcmi`（原版 DEF 实际所在）；
/// - 其它 → 与 modId 末段同名的 mod 目录（嵌套子 mod 的 id 末段即磁盘目录名）。
///
/// 不这么做会从几百个 mod 里逐个开 `content.zip` 找同一个资源，单次悬停即明显卡顿。
pub(crate) fn ordered_mod_paths(paths: &VcmiPaths, mod_id: &str) -> Vec<PathBuf> {
    let all = crate::vcmi::scan::cached_mod_paths(paths);
    let leaf = mod_id.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let mut ordered: Vec<PathBuf> = Vec::with_capacity(2);
    if mod_id == "core" || mod_id.starts_with("core.") {
        ordered.push(paths.game_dir.join("Mods").join("vcmi"));
    }
    if !leaf.is_empty() {
        for (p, _) in all.iter() {
            if p.file_name()
                .map(|n| n.to_string_lossy().to_ascii_lowercase() == leaf)
                .unwrap_or(false)
            {
                ordered.push(p.clone());
            }
        }
    }
    ordered
}

pub(crate) fn read_def_bytes_inner(
    paths: &VcmiPaths,
    mod_id: &str,
    resource: &str,
) -> Result<Vec<u8>, String> {
    let all = crate::vcmi::scan::cached_mod_paths(paths);
    // 详情页资源回退同款：让 `resolve_resource_fallback` 也能搜到 `user_data_dir/Data`
    set_wiki_user_data_dir(&paths.user_dir);
    // 详情页跨模组资源检索：让 `resolve_resource_fallback` 也能搜到其它已装模组内的素材
    set_wiki_mod_paths(&all);
    // 只从「请求方自身」开始：
    // - `core` / `core.<faction>` → 游戏目录 `Mods/vcmi`（原版 DEF 实际所在）
    // - 其他 → 与 modId 末段同名的模组目录（嵌套子 mod 的 id 末段即磁盘目录名）
    //
    // ⚠️ **不得再 `chain(all)`**：`resolve_resource_fallback` 内部已经是
    // 「本 mod → 游戏根/用户数据 LOD → 跨模组全遍历」。外层再遍历全表是语义冗余 ——
    // 每个未命中资源会因此多付 568 次 `is_dir` stat + 568 次解析（实测 808ms，
    // 缓存刚释放时首次还要重建 870 个目录索引 ≈ 9s）。跨模组能力由内层提供，不会丢。
    let own = ordered_mod_paths(paths, mod_id);
    // 目录名与 modId 末段不一致（人工改名）时 `own` 为空 → 用全表首个作为起点，
    // 让 `resolve_core` 的跨模组段照常覆盖全表（能命中，只是不享受"自己优先"的短路）。
    let start: Vec<PathBuf> = if own.is_empty() {
        all.iter().take(1).map(|(p, _)| p.clone()).collect()
    } else {
        own
    };
    for mp in start.iter() {
        // resolve_resource 只搜 mod 自身；原版生物战斗动画（如 CPKMAN.DEF）实际在
        // 游戏根 Data/*.lod 内，故加游戏目录回退，与背景/图标资源一致。
        if let Some((bytes, _ext)) = resolve_resource_fallback(mp, &paths.game_dir, resource) {
            return Ok(bytes);
        }
    }
    Err(be_err("vcmi_resource_not_found", &format!("{}", resource)))
}

/// 单个动作分组的帧数上限（防御坏文件：原版生物战斗 DEF 单组最多几十帧）。
const MAX_GROUP_FRAMES: usize = 512;

/// VCMI 生物窗的**显示窗口**（源像素）：`CCreaturePic` 从 DEF 全画布裁出的固定矩形。
///
/// 依据 `client/widgets/MiscWidgets.cpp:703-704`：
/// `anim->clipRect(doubleWide ? 170 : 150, 155, bg.w, bg.h)`，
/// 且 `Images.cpp::blitImage` 用它作 `src` 矩形 1:1 画进背景框 ——
/// 即"窗口左上角 (150|170, 155)、尺寸 = **背景图尺寸**"。
///
/// 背景 = 阵营 `creatureBackground["130px"]`（原版城堡 `CRBKGCAS`），**1x 为 100×130**
/// （mod 里 "130px" 指的是高度；我们缓存的 300×390 webp 是 HD x3）。
/// 窗口尺寸**与屏幕无关**，屏幕只决定取哪一档高清（见 `need` 参数）。
const WIN_W: i64 = 100;
const WIN_H: i64 = 130;
const WIN_X_SINGLE: i64 = 150;
const WIN_X_DOUBLE: i64 = 170;
const WIN_Y: i64 = 155;

/// 窗口布局**只有两种**（用户 2026-10-21 拍板）：
///
/// - **A 默认**：`100×130`，位置与 VCMI 完全一致（`clipRect(150|170, 155, 100, 130)`）；
///   素材越界就**正常裁剪**（游戏本来就这么干）。
/// - **B 宽幕布**：`(100 + 2·pad) × 130` —— 只在**左右对称**加黑幕，**高度恒定为 130**；
///   超出幕布的仍然裁掉。
///
/// 为什么不做纵向扩展：纵向扩出来的一圈没有背景像素可填（素材本身只有 100×130），
/// 补天会泛白/条纹、镜像会克隆建筑、模糊会毁细节 —— 三种都被实机否决。
/// 横向扩之所以可以，是因为两侧本来就是"幕布黑"，不需要伪造地形。
const WIN_WING_TRIGGER: i64 = 24;

/// 宽幕布档位：**只有一种尺寸**（用户要求"只有两种布局"）。
///
/// 100 + 2×50 = 200 宽、高度恒 130；只对"悬停帧横向越出 ≥ [`WIN_WING_TRIGGER`]"的生物启用。
/// 不再按生物各算各的宽度 —— 那会让每个生物一套黑幕，观感不统一。
const WIN_WING_WIDTH: i64 = 200;

/// 计算显示窗口：**两种布局之一**（见上方说明）。
///
/// `art` = 该生物"最宽姿态"（DEF 用 `MOUSEON` 悬停帧、png 动画用帧的 alpha 包围盒）的
/// 包围盒；横向越出基准窗口 ≥ [`WIN_WING_TRIGGER`] 时切宽幕布，否则原样返回基准窗口。
pub(crate) fn two_layout_window(
    double_wide: bool,
    canvas: Option<(u32, u32)>,
    art: Option<(i64, i64, i64, i64)>,
) -> (i64, i64, i64, i64) {
    let bx0 = if double_wide { WIN_X_DOUBLE } else { WIN_X_SINGLE };
    let base = (bx0, WIN_Y, WIN_W, WIN_H);
    let Some((ax0, _, ax1, _)) = art else {
        return base;
    };
    // 触发条件看"悬停帧横向越界"，但**宽度取固定档位**（只有两种布局）
    let over = (bx0 - ax0).max(ax1 - (bx0 + WIN_W));
    if over < WIN_WING_TRIGGER {
        return base;
    }
    // 以基准窗口中心为轴、左右对称展开；夹到画布内（越界部分本来也没有像素）
    let cw = canvas.map(|(w, _)| w as i64).unwrap_or(450);
    let center = bx0 + WIN_W / 2;
    let x0 = (center - WIN_WING_WIDTH / 2).max(0);
    let x1 = (center + WIN_WING_WIDTH / 2).min(cw);
    (x0, WIN_Y, (x1 - x0).max(1), WIN_H)
}

/// 计算若干组的并集包围盒。
fn union_bounds(
    def: &crate::infra::def::DefReader,
    keep: impl Fn(usize) -> bool,
) -> Option<(i64, i64, i64, i64)> {
    let mut acc: Option<(i64, i64, i64, i64)> = None;
    for (g, r) in def.group_frame_bounds() {
        if !keep(g) {
            continue;
        }
        acc = Some(match acc {
            Some((x0, y0, x1, y1)) => (x0.min(r.0), y0.min(r.1), x1.max(r.2), y1.max(r.3)),
            None => r,
        });
    }
    acc
}

/// 决定"是否需要宽幕布"的组：**移动/悬停/待机/防御** —— 也就是待机与移动状态里
/// 会反复播放、用户最常看到的姿态。
///
/// 实测只看悬停帧会漏掉金龙（悬停不越界，但**防御**帧左探 84px）、大天使/黑龙
/// （**移动**帧左探 29px），于是它们拿不到宽幕布、一动就被裁。
/// 攻击/死亡不计入：那些是"前扑/倒地"的瞬间，越界属正常，且算进来会让几乎所有生物都变宽幕布。
const WING_PROBE_GROUPS: &[usize] = &[0, 1, 2, 4];

/// 显示几何的三层矩形（都是 1x 画布坐标）。
///
/// - `win`：**幕布窗口**（两种布局之一，100×130 或 200×130）—— 前端按它确定画框比例，
///   框外区域就是黑幕；
/// - `tex`：**裁剪矩形**（`win ∪ 全部组的包围盒`）—— 后端按它裁帧。
///   必须比 `win` 大：否则头部等超出窗口的像素在**进入前端之前**就被切掉了，
///   前端再怎么缩放也补不回来（这正是"凤凰头还是被裁"的根因）；
/// - `fit`：**适配参考矩形**（`win ∪ 除死亡外的组`）—— 前端按它算统一缩放倍数，
///   让生物完整落在幕布里；死亡姿态摊得很开，不参与缩放计算（越界就正常裁）。
pub(crate) struct WindowPlan {
    pub win: (i64, i64, i64, i64),
    pub tex: (i64, i64, i64, i64),
    pub fit: (i64, i64, i64, i64),
}

fn anim_window(def: &crate::infra::def::DefReader, double_wide: bool) -> WindowPlan {
    let canvas = def.canvas_size();
    let wing_probe = union_bounds(def, |g| WING_PROBE_GROUPS.contains(&g));
    // 适配参考只看"站着/走动"的姿态（0/1/2/4）：攻击前扑、死亡摊开的越界属正常裁剪
    let fit_probe = union_bounds(def, |g| WING_PROBE_GROUPS.contains(&g));
    let all_probe = union_bounds(def, |_| true);
    let win = two_layout_window(double_wide, canvas, wing_probe);

    // ⚠️ 包围盒口径是 `(x0, y0, x1, y1)`，窗口口径是 `(x, y, w, h)` —— 必须先统一再并
    let win_bbox = (win.0, win.1, win.0 + win.2, win.1 + win.3);
    let union = |a: Option<(i64, i64, i64, i64)>, b: (i64, i64, i64, i64)| -> (i64, i64, i64, i64) {
        match a {
            Some((x0, y0, x1, y1)) => (x0.min(b.0), y0.min(b.1), x1.max(b.2), y1.max(b.3)),
            None => b,
        }
    };
    let clamp = |r: (i64, i64, i64, i64)| -> (i64, i64, i64, i64) {
        let (mut x0, mut y0, mut x1, mut y1) = r;
        if let Some((cw, ch)) = canvas {
            x0 = x0.clamp(0, cw as i64);
            x1 = x1.clamp(0, cw as i64);
            y0 = y0.clamp(0, ch as i64);
            y1 = y1.clamp(0, ch as i64);
        }
        (x0, y0, (x1 - x0).max(1), (y1 - y0).max(1))
    };
    WindowPlan {
        win,
        tex: clamp(union(all_probe, win_bbox)),
        fit: clamp(union(fit_probe, win_bbox)),
    }
}

/// 取某 `.def` 指定分组的动画帧，**高清优先、1x 兜底**，两者都合成进 VCMI 显示窗口。
///
/// **为什么由后端统一出图**（2026-10-21）：1x 与 HD 的构图必须完全一致，
/// 而前端 TS 解码器只产出"裁剪帧"，拿不到 `lm/tm`，无法摆到全画布上 ——
/// 让前端自己拼窗口会与后端两条口径，迟早漂移。故 1x 也在此合成。
///
/// `need`（目标物理像素）= 显示框 CSS × 屏幕物理倍率；档位取**最小满足 `档位×130 ≥ need`**
/// 者，否则最高档（1080p 大图 x3 / 弹窗 x2；4K 封顶 x3）。
///
/// 返回 `None` = 该分组无法播放（资源找不到 / 该组没有可用帧），前端回退静态图或旧 TS 解码。
///
/// ## 出参结构
///
/// ```json
/// { "w": 390, "h": 390, "scale": 3, "frames": [ { "x": 21, "y": 44, "w": 336, "h": 300, "data": [<webp 字节>] } ] }
/// ```
///
/// - `w/h`：窗口尺寸 × 档位（130×`scale`）；
/// - `x/y`：该帧在窗口内的位置 = `(lm/tm − spriteOffset − 窗口左上角) × 档位`；
///   超出窗口的部分已裁掉（大生物被裁边是**原版行为**，`clipRect` 同样会裁）；
/// - `data`：webp 字节（HD 的独立阴影切片已合成进这一张）。
pub(crate) fn read_def_group_hd_inner(
    paths: &VcmiPaths,
    mod_id: &str,
    resource: &str,
    group: usize,
    double_wide: bool,
    need: u32,
    known_kind: Option<&str>,
) -> Option<GroupOut> {
    // 详情链路（非构建批次）没有注入 HD 来源 → 按配置探测一次。
    // `off` / 未装 hd-edition 时注入结果为 None，下方 `source()` 判空即走回退。
    if crate::vcmi::hd_edition::source().is_none() {
        crate::vcmi::hd_edition::setup_from_config(
            VcmiPaths::read_launcher_config().as_ref(),
            &paths.game_dir,
        );
    }
    // ── ⓿ sprites-json 动画（VCMI 1.6 png 模式）：帧即整画布，套同一窗口裁剪 ──
    // 必须先试：这类资源的"字节"是 json，`DefReader` 解析不了。实测 Yokai / HuaXia
    // 两个 mod 全部生物走这条路径（frames 450×400，与原版 DEF 同坐标空间）。
    //
    // 构建期登记的指针（`anim.kind`）已确认是 DEF 时**跳过**这次探测：`locate` 要在
    // 模组索引里按资源名定位，省掉它对每个动作分组的固定开销（见 `anim_json::LOCATE_CACHE`）。
    let probe_json = known_kind != Some("def");
    if probe_json {
        if let Some(anim) =
            crate::vcmi::anim_json::load_group(paths, mod_id, resource, group, double_wide)
        {
        let (jx, jy, jw, jh) = anim.window;
        // png 动画的帧即整画布：win = 基准窗口，tex = 窗口 ∪ 帧包围盒（实测这类素材多在窗口内）
        let plan = crate::vcmi::resource::WindowPlan {
            win: (
                if double_wide { WIN_X_DOUBLE } else { WIN_X_SINGLE },
                WIN_Y,
                WIN_W,
                WIN_H,
            ),
            tex: (jx, jy, jw, jh),
            fit: (jx, jy, jw, jh),
        };
        let _ = &plan;
        let mut frames: Vec<(i64, i64, u32, u32, Vec<u8>)> = Vec::new();
        for img in &anim.frames {
            let (w, h) = (img.width(), img.height());
            if let Some(f) = encode_window_frame(img.as_raw(), w, h, -jx, -jy, jw, jh) {
                frames.push(f);
            }
        }
        if frames.is_empty() {
            return None;
        }
        // png 动画没有官方 HD 档位（HD 包只收原版 DEF 的帧名）→ 恒 1x，由前端平滑放大
        return Some(build_window_json(frames, (jx, jy, jw, jh), 1, &plan));
        }
    }

    let bytes = read_def_bytes_inner(paths, mod_id, resource).ok()?;
    let def = crate::infra::def::DefReader::new(&bytes);
    // 目标档位：窗口是 130 源像素，`need` 是这块区域实际要占的物理像素
    let need_scale = need.div_ceil(WIN_W as u32).max(1);
    // 三层矩形（win 幕布窗口 / tex 裁剪矩形 / fit 适配参考），见 `anim_window`
    let plan = anim_window(&def, double_wide);
    let (wx, wy, ww, wh) = plan.tex;

    // ── ① 高清：逐帧查 pak，任一帧未命中 → 整组回退 1x（不混档） ──
    let hd_ok = crate::vcmi::hd_edition::source().is_some();
    if hd_ok {
        let mut frames: Vec<(i64, i64, u32, u32, Vec<u8>)> = Vec::new();
        let mut scale: Option<u32> = None;
        let mut ok = true;
        for idx in 0..MAX_GROUP_FRAMES {
            let Some(name) = def.frame_name(group, idx) else {
                break;
            };
            let Some((img, s, off)) =
                crate::vcmi::hd_edition::lookup_with_offset_for(&name, need_scale)
            else {
                ok = false;
                break;
            };
            if let Some(known) = scale {
                if known != s {
                    ok = false;
                    break;
                }
            } else {
                scale = Some(s);
            }
            let (w, h) = (img.width(), img.height());
            if w == 0 || h == 0 {
                ok = false;
                break;
            }
            // 阴影：HD 版是独立切片、本体图里不含 → 合成进同一张（左上角对齐），否则"悬空"。
            let (rgba, w, h) = match crate::vcmi::hd_edition::lookup_shadow(&name, s) {
                Some(sh) if sh.width() > 0 && sh.height() > 0 => {
                    composite_shadow(img.as_raw(), w, h, &sh)
                }
                _ => (img.as_raw().clone(), w, h),
            };
            let (lm, tm) = def.frame_margins(group, idx).unwrap_or((0, 0));
            // 全画布位置 − 窗口左上角，再按档位放大
            let x = ((lm as i64 - off.0 as i64) - wx) * s as i64;
            let y = ((tm as i64 - off.1 as i64) - wy) * s as i64;
            if let Some(f) =
                encode_window_frame(&rgba, w, h, x, y, ww * s as i64, wh * s as i64)
            {
                frames.push(f);
            }
        }
        if ok && !frames.is_empty() {
            let s = scale.unwrap_or(1);
            return Some(build_window_json(frames, (wx, wy, ww * s as i64, wh * s as i64), s, &plan));
        }
    }

    // ── ② 1x 兜底：同一套窗口与定位公式，只是档位恒为 1 ──
    let mut frames: Vec<(i64, i64, u32, u32, Vec<u8>)> = Vec::new();
    for idx in 0..MAX_GROUP_FRAMES {
        // 动画专用渲染：处理"索引 1-4=阴影、5-7=选择层"的特殊调色板（否则阴影画成洋红实体色）
        let Some((w, h, bgra, lm, tm)) = def.decode_frame_at_anim(group, idx) else {
            break;
        };
        if w == 0 || h == 0 {
            continue;
        }
        // DEF 解码产出 BGRA（见 `infra::def`），webp 编码要 RGBA
        let mut rgba = bgra.clone();
        for px in rgba.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        let (x, y) = (lm as i64 - wx, tm as i64 - wy);
        if let Some(f) = encode_window_frame(&rgba, w, h, x, y, ww, wh) {
            frames.push(f);
        }
    }
    if frames.is_empty() {
        return None;
    }
    Some(build_window_json(frames, (wx, wy, ww, wh), 1, &plan))
}

/// 把一帧**裁进显示窗口**并编码：与窗口无交集的帧直接丢弃（原版 `clipRect` 同样裁掉）。
///
/// 返回 `(窗口内 x, 窗口内 y, 宽, 高, webp)`。
fn encode_window_frame(
    rgba: &[u8],
    w: u32,
    h: u32,
    x: i64,
    y: i64,
    win_w: i64,
    win_h: i64,
) -> Option<(i64, i64, u32, u32, Vec<u8>)> {
    // 与窗口 [0,win) 求交
    let sx = (-x.min(0)).max(0) as usize;
    let sy = (-y.min(0)).max(0) as usize;
    let ex = ((win_w - x).min(w as i64)).max(0) as usize;
    let ey = ((win_h - y).min(h as i64)).max(0) as usize;
    if sx >= ex || sy >= ey {
        return None;
    }
    let (cw, ch) = (ex - sx, ey - sy);
    let mut crop = Vec::with_capacity(cw * ch * 4);
    for yy in sy..ey {
        let base = (yy * w as usize + sx) * 4;
        crop.extend_from_slice(&rgba[base..base + cw * 4]);
    }
    let webp = crate::infra::image::encode_webp_lossy_q(
        &crop,
        cw as u32,
        ch as u32,
        crate::infra::image::LOSSY_QUALITY,
    )?;
    Some((x.max(0), y.max(0), cw as u32, ch as u32, webp))
}

/// 一个动作分组的出参（**二进制封装前的中间形态**）。
///
/// 帧字节不再塞进 `serde_json::Value`：那样会以"数字数组"过 IPC（载荷膨胀 3~4 倍，
/// 前端还要逐元素写回 `Uint8Array`）。这里保留原始 `Vec<u8>`，由命令层拼成二进制载荷。
pub(crate) struct GroupOut {
    /// 元数据（`w/h/wx/wy/scale/win*/fit*`），不含帧
    pub meta: Value,
    pub frames: Vec<FrameOut>,
}

pub(crate) struct FrameOut {
    pub x: i64,
    pub y: i64,
    pub w: u32,
    pub h: u32,
    pub data: Vec<u8>,
}

impl GroupOut {
    /// 旧结构的 JSON 形态（**仅供测试**；会把帧字节展开成数字数组）。
    #[cfg(test)]
    pub(crate) fn to_value(&self) -> Value {
        let mut v = self.meta.clone();
        let frames: Vec<Value> = self
            .frames
            .iter()
            .map(|f| {
                json!({ "x": f.x, "y": f.y, "w": f.w, "h": f.h, "data": f.data })
            })
            .collect();
        v["frames"] = Value::Array(frames);
        v
    }
}

/// 组装出参：`w/h` = 窗口尺寸 × 档位；`wx/wy` = 窗口左上角在**动画画布**（450×400）里的坐标。
///
/// `wx/wy` 给前端合成宽窗口背景用：生物背景图（`CRBKGCAS` 等）原生就是基准窗口那一块
/// （100×130，对应画布 `150|170..250|270 × 155..285`），窗口一旦外扩，前端得知道往哪些方向补。
fn build_window_json(
    frames: Vec<(i64, i64, u32, u32, Vec<u8>)>,
    tex: (i64, i64, i64, i64),
    scale: u32,
    plan: &crate::vcmi::resource::WindowPlan,
) -> GroupOut {
    let (wx, wy, tex_w, tex_h) = tex;
    let (winx, winy, winw, winh) = plan.win;
    let (_, _, fitw, fith) = plan.fit;
    let meta = json!({
        // 裁剪矩形（帧位图就是这块）—— 前端按 1x 尺寸换算成像素布局
        "w": tex_w,
        "h": tex_h,
        "wx": wx,
        "wy": wy,
        "scale": scale,
        // 幕布窗口（两种布局之一）：前端据此定画框比例、黑幕范围
        "winX": winx,
        "winY": winy,
        "winW": winw,
        "winH": winh,
        // 适配参考尺寸（未含死亡姿态）：前端据此算等比缩放
        "fitW": fitw,
        "fitH": fith
    });
    GroupOut {
        meta,
        frames: frames
            .into_iter()
            .map(|(x, y, w, h, data)| FrameOut { x, y, w, h, data })
            .collect(),
    }
}

/// 打包成 IPC 二进制载荷：`[4B LE 元数据长度][元数据 JSON][帧体...]`。
///
/// 元数据里 `frames[i]` 只含 `{x,y,w,h,len}`，第 i 帧字节紧跟在元数据之后、按 `len` 依次排布。
/// 返回空 `Vec`（0 字节）= 该组没有可播放帧（前端据此回退）。
pub(crate) fn pack_group_binary(out: Option<GroupOut>) -> Vec<u8> {
    let Some(out) = out else {
        return Vec::new();
    };
    let frames_meta: Vec<Value> = out
        .frames
        .iter()
        .map(|f| {
            json!({ "x": f.x, "y": f.y, "w": f.w, "h": f.h, "len": f.data.len() })
        })
        .collect();
    let mut meta = out.meta;
    meta["frames"] = Value::Array(frames_meta);
    let meta_bytes = serde_json::to_vec(&meta).unwrap_or_default();
    let body_len: usize = out.frames.iter().map(|f| f.data.len()).sum();
    let mut buf = Vec::with_capacity(4 + meta_bytes.len() + body_len);
    buf.extend_from_slice(&(meta_bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(&meta_bytes);
    for f in out.frames {
        buf.extend_from_slice(&f.data);
    }
    buf
}

/// 把 HD 的**独立阴影切片**合成到本体图里，返回 `(RGBA, 宽, 高)`。
///
/// 对齐 VCMI `HdImageLoader::getImage(shadow=true)` 的语义：阴影与本体**共用同一 margins**
/// （`HdImageLoader.cpp:176` 对两者一视同仁），只是各自尺寸不同 ⇒ 二者**左上角对齐**叠加，
/// 画布取二者较大者。绝不自己发明"底边/居中对齐" —— 那会让阴影相对生物漂移。
fn composite_shadow(
    body: &[u8],
    w: u32,
    h: u32,
    shadow: &image::RgbaImage,
) -> (Vec<u8>, u32, u32) {
    let (sw, sh) = (shadow.width(), shadow.height());
    if w == 0 || h == 0 || sw == 0 || sh == 0 {
        return (body.to_vec(), w, h);
    }
    let (ow, oh) = (w.max(sw), h.max(sh));
    let mut out = image::RgbaImage::new(ow, oh);
    // 1) 阴影在下（左上角对齐）
    for yy in 0..sh {
        for xx in 0..sw {
            let px = shadow.get_pixel(xx, yy).0;
            if px[3] == 0 {
                continue;
            }
            let dst = out.get_pixel_mut(xx, yy);
            dst.0 = blend_over(px, dst.0);
        }
    }
    // 2) 本体在上
    for yy in 0..h.min(oh) {
        for xx in 0..w.min(ow) {
            let i = ((yy * w + xx) * 4) as usize;
            let px = [body[i], body[i + 1], body[i + 2], body[i + 3]];
            if px[3] == 0 {
                continue;
            }
            let dst = out.get_pixel_mut(xx, yy);
            dst.0 = blend_over(px, dst.0);
        }
    }
    (out.into_raw(), ow, oh)
}

/// src-over 合成（`dst` 为底，`src` 在上）。
fn blend_over(src: [u8; 4], dst: [u8; 4]) -> [u8; 4] {
    let a = src[3] as f32 / 255.0;
    let mut out = [0u8; 4];
    for c in 0..3 {
        out[c] = (src[c] as f32 * a + dst[c] as f32 * (1.0 - a)).round().min(255.0) as u8;
    }
    out[3] = (a * 255.0 + dst[3] as f32 * (1.0 - a)).round().min(255.0) as u8;
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 二进制分组载荷的格式自洽：`[4B 元数据长度][元数据][帧体...]`，
    /// 帧体按元数据里的 `len` 依次排布，总长度必须严丝合缝。
    #[test]
    fn pack_group_binary_layout_is_self_consistent() {
        let out = GroupOut {
            meta: json!({ "w": 300, "h": 390, "scale": 3 }),
            frames: vec![
                FrameOut { x: 0, y: 0, w: 300, h: 390, data: vec![1, 2, 3] },
                FrameOut { x: 1, y: 2, w: 10, h: 20, data: vec![4; 5] },
            ],
        };
        let buf = pack_group_binary(Some(out));
        let meta_len = u32::from_le_bytes(buf[0..4].try_into().unwrap()) as usize;
        let meta: Value = serde_json::from_slice(&buf[4..4 + meta_len]).unwrap();
        let frames = meta["frames"].as_array().expect("frames 必须是数组");
        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0]["len"].as_u64(), Some(3));
        assert_eq!(frames[1]["len"].as_u64(), Some(5));
        assert_eq!(meta["scale"].as_u64(), Some(3));
        assert_eq!(buf.len(), 4 + meta_len + 3 + 5, "帧体必须紧跟在元数据之后");
        assert!(pack_group_binary(None).is_empty(), "无帧必须是空载荷");
    }

    /// `strip_ext` 只剥**末段**的扩展名（中间段的点不是扩展名）。
    #[test]
    fn strip_ext_only_strips_last_segment_extension() {
        assert_eq!(strip_ext("a/b.png").as_deref(), Some("a/b"));
        assert_eq!(strip_ext("a/b.res/e.png").as_deref(), Some("a/b.res/e"));
        assert_eq!(strip_ext("dir.d/e/f.bmp").as_deref(), Some("dir.d/e/f"));
        assert_eq!(strip_ext("a/b.res/e"), None, "中间段的点不得当扩展名");
        assert_eq!(strip_ext("e_death"), None);
    }

    /// VCMI `<file>/<sub>` 引用（如 bonus 图标的 `zvs/Lib1.res/E_DEATH`）必须与磁盘清单
    /// 条目（`data/zvs/Lib1.res/E_DEATH.png`）归一到**同一个 key**。
    ///
    /// 回归背景：旧 `strip_ext` 用 `rfind('.')`，把它截成 `zvs/lib1` → 全部 bonus 图标解析失败
    /// （实测 19,376 条能力只有 5.4% 有图标）。两侧共用 `strip_ext`，本用例锁住口径一致。
    #[test]
    fn subpath_resource_normalizes_like_listing_entry() {
        let req = resource_keys("zvs/Lib1.res/E_DEATH");
        let entry = resource_keys("data/zvs/Lib1.res/E_DEATH.png");
        assert_eq!(req.0, "zvs/lib1.res/e_death");
        assert_eq!(req.0, entry.0, "请求与清单条目的去扩展名 key 必须一致");
        assert_eq!(entry.1, "zvs/lib1.res/e_death.png", "含扩展名 key 保留末段扩展");
    }

    /// **素材不得被 `config/` 下的同名定义文件顶掉**（2026-10-16 修复）。
    ///
    /// 实测形态：`config/lianjin/artifacts/ringOfTheCobrasEye.json` 与
    /// `sprites/lianjin/artifacts/ringOfTheCobrasEye.bmp` 经前缀剥离后撞成同一个 key，
    /// 而 `config` 在 `sprites` 之前被遍历到 → 解析返回的是**配置 JSON 的字节** →
    /// `encode_webp` 失败 → 法术/宝物 `image` 恒为 null（资源明明在盘）。
    #[test]
    fn asset_wins_over_same_stem_config_json() {
        let root = std::env::temp_dir().join("vcmi_fv_res_cfg_collision");
        let _ = fs::remove_dir_all(&root);
        let cfg = root.join("Content").join("config").join("lianjin").join("artifacts");
        let sp = root.join("Content").join("sprites").join("lianjin").join("artifacts");
        fs::create_dir_all(&cfg).unwrap();
        fs::create_dir_all(&sp).unwrap();
        fs::write(cfg.join("ringOfTheCobrasEye.json"), b"{ \"x\": 1 }").unwrap();
        fs::write(sp.join("ringOfTheCobrasEye.bmp"), b"BMP-BYTES").unwrap();

        let got = resolve_resource(&root, "lianjin/artifacts/ringOfTheCobrasEye").unwrap();
        assert_eq!(got.1, "bmp", "命中的应是 sprites 下的素材，而不是 config 的 JSON");
        assert_eq!(got.0, b"BMP-BYTES".to_vec());

        // 请求带（不存在于盘的）扩展名时同样命中素材，而非定义文件
        let got2 = resolve_resource(&root, "lianjin/artifacts/ringOfTheCobrasEye.bmp").unwrap();
        assert_eq!(got2.1, "bmp");

        // 只有定义文件时不回归（仍按原样返回，调用方自行判空）
        let only = std::env::temp_dir().join("vcmi_fv_res_cfg_only");
        let _ = fs::remove_dir_all(&only);
        let d = only.join("Content").join("config").join("m");
        fs::create_dir_all(&d).unwrap();
        fs::write(d.join("solo.json"), b"{}").unwrap();
        let got3 = resolve_resource(&only, "m/solo").unwrap();
        assert_eq!(got3.1, "json", "只有定义文件时行为不变");

        let _ = fs::remove_dir_all(&root);
        let _ = fs::remove_dir_all(&only);
    }
}
