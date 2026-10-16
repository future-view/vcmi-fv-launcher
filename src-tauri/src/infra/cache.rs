//! 小容量内存缓存：文件指纹 + 带容量上限的进程内 LRU。
//!
//! 用途：VCMI 游戏根配置（h3-core 文案表、建筑 bid 标准表、初始部队概率等）解析开销大，
//! 但内容很少变化 —— 以「文件指纹」为键缓存解析结果，指纹变化即失效。

use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;

pub(crate) const CFG_CACHE_CAP: usize = 4;

/// 取 (mtime, len) 指纹：任一候选文件变化即视为失效。
pub(crate) fn fingerprint_of(paths: &[PathBuf]) -> String {
    let mut s = String::new();
    for p in paths {
        let (mt, len) = fs::metadata(p)
            .map(|m| {
                let mt = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_nanos())
                    .unwrap_or(0);
                (mt, m.len())
            })
            .unwrap_or((0, 0));
        s.push_str(&format!("{}|{}|{};", p.to_string_lossy(), mt, len));
    }
    s
}

/// 带容量上限的小缓存：命中直接返回 Arc（零拷贝克隆）；未命中则**持锁**构建。
/// 持锁构建是有意为之：并发下首个线程解析、其余线程等待后直接命中，避免惊群重复解析。
pub(crate) fn cfg_cache_get<T>(
    cache: &'static std::sync::OnceLock<Mutex<Vec<(String, std::sync::Arc<T>)>>>,
    key: String,
    make: impl FnOnce() -> T,
) -> std::sync::Arc<T> {
    let list = cache.get_or_init(|| Mutex::new(Vec::new()));
    let mut guard = list.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, v)) = guard.iter().find(|(k, _)| *k == key) {
        return v.clone();
    }
    let v = std::sync::Arc::new(make());
    guard.push((key, v.clone()));
    if guard.len() > CFG_CACHE_CAP {
        guard.remove(0);
    }
    v
}

/// 清空小缓存并**收缩容器**（构建结束释放常驻内存，见 [`crate::infra::lru::SizedCache::clear`]）。
pub(crate) fn cfg_cache_clear<T>(
    cache: &'static std::sync::OnceLock<Mutex<Vec<(String, std::sync::Arc<T>)>>>,
) {
    if let Some(list) = cache.get() {
        if let Ok(mut g) = list.lock() {
            g.clear();
            g.shrink_to_fit();
        }
    }
}
