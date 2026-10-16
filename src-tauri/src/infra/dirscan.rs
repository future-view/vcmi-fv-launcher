//! 目录清单缓存：**一次递归的产物**，供「资源索引」与「配置收集」共用。
//!
//! ⚠️ 必要性（2026-10-16）：此前 `Content/` 的全量递归在每个 mod 的**每个阶段**被重复
//! 执行约 9 次 —— `creatures` / `heroes` / `spells` / `skills` / `artifacts`×2 /
//! 城镇目录索引 / `town.rs` 的类型收集 / `raw` 全量归一，各自 `fs::read_dir` 一遍。
//! 实测单次全量遍历本发行版（12.6 万文件 + 1.4 万目录 ≈ 14 万条目）约 **17.7s，
//! 其中 sys 9.7s**（冷缓存；条目访问还各带 `e.path()` 分配 + `is_dir()` stat）。
//! 缓存后每个目录只递归一次，其余调用退化为纯内存过滤。
//!
//! 依赖方向：本层属 `infra`，不依赖 `vcmi` / `modcache` / `tauri`；
//! `vcmi::resource` 与 `infra::json` 都基于它构建各自的索引。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// 单个目录的文件清单。
pub(crate) struct DirListing {
    /// 目录前缀长度：`abs` 切掉它（及其后的 `/`）即得相对路径
    prefix_len: usize,
    /// (绝对路径小写归一（正斜杠）, 绝对路径)
    items: Vec<(String, Arc<Path>)>,
}

impl DirListing {
    /// 条目数。
    pub(crate) fn len(&self) -> usize {
        self.items.len()
    }

    #[allow(dead_code)]
    pub(crate) fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// 第 `i` 项的**绝对路径小写归一**（`infra::json` 的类型匹配沿用此口径，见 [`crate::infra::json::is_type_config_path`]）。
    pub(crate) fn abs(&self, i: usize) -> &str {
        &self.items[i].0
    }

    /// 第 `i` 项**相对本目录**的路径（小写、正斜杠）——资源索引的归一口径。
    ///
    /// 与原 `walk_index` 的 `p.strip_prefix(base).to_string_lossy()` 等价
    /// （两者都只用于喂给会自行 lower + 归一分隔符的 `norm_zip_key` / `norm_key_with_ext`）。
    pub(crate) fn rel(&self, i: usize) -> &str {
        let a = &self.items[i].0;
        let s = self.prefix_len.min(a.len());
        a[s..].trim_start_matches('/')
    }

    /// 第 `i` 项的绝对路径（`Arc` 共享，供资源索引与配置收集复用同一份分配）。
    pub(crate) fn path(&self, i: usize) -> &Arc<Path> {
        &self.items[i].1
    }
}

/// 清单缓存：`目录路径 → (mtime, 清单)`。
type ListingCache = Mutex<HashMap<PathBuf, (u128, Arc<DirListing>)>>;
static DIR_LISTING_CACHE: OnceLock<ListingCache> = OnceLock::new();
/// 条目上限（按目录数；正常发行版数百个 mod 目录，远低于此）
const DIR_LISTING_CAP: usize = 8192;

/// 取（并缓存）目录清单：每个目录只递归一次。
pub(crate) fn dir_listing(dir: &Path) -> Arc<DirListing> {
    let mt = dir_mtime(dir);
    let cache = DIR_LISTING_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    {
        if let Ok(g) = cache.lock() {
            if let Some((m, v)) = g.get(dir) {
                if *m == mt {
                    return v.clone();
                }
            }
        }
    }
    let v = Arc::new(scan_dir(dir));
    if let Ok(mut g) = cache.lock() {
        if g.len() >= DIR_LISTING_CAP {
            g.clear();
        }
        g.insert(dir.to_path_buf(), (mt, v.clone()));
    }
    v
}

/// 释放目录清单缓存（构建结束调用；见 [`crate::infra::cache_gc`]）。
pub(crate) fn release_caches() {
    if let Some(c) = DIR_LISTING_CACHE.get() {
        if let Ok(mut g) = c.lock() {
            g.clear();
            g.shrink_to_fit();
        }
    }
}

fn dir_mtime(dir: &Path) -> u128 {
    std::fs::metadata(dir)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

/// 递归收集目录下的**全部文件**（目录本身不进清单）。
fn scan_dir(dir: &Path) -> DirListing {
    let prefix_len = norm_lower(dir).len();
    let mut items: Vec<(String, Arc<Path>)> = Vec::new();
    walk(dir, &mut items);
    DirListing { prefix_len, items }
}

/// 深度优先遍历。
///
/// ⚠️ **必须与原 `read_dir` 递归同序**（遇到子目录立即深入，父目录剩余条目排在其后）：
/// 资源索引用 `or_insert` 保留首个匹配，两个文件归一后撞 key 时**遍历顺序决定谁胜出**，
/// 换序会静默改变解析结果。
fn walk(d: &Path, items: &mut Vec<(String, Arc<Path>)>) {
    const MAX_ITEMS: usize = 200_000;
    let Ok(entries) = std::fs::read_dir(d) else {
        return;
    };
    for e in entries.flatten() {
        if items.len() >= MAX_ITEMS {
            return;
        }
        let p = e.path();
        if p.is_dir() {
            walk(&p, items);
        } else {
            items.push((norm_lower(&p), Arc::from(p.as_path())));
        }
    }
}

/// 路径 → 小写 + 正斜杠归一（与 [`crate::infra::json::collect_json_files`] 原口径一致）。
fn norm_lower(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/").to_lowercase()
}
