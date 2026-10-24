//! 轻量性能埋点：线程局部累计 + 全局聚合。
//!
//! 用途：定位 wiki 缓存构建的真实热点（首次全量约 22~30 分钟，见运行日志）。
//! 已埋的关键指标（按"每调用一次的代价"排序，越靠前越可疑）：
//! - `lod.open`：打开并解析 `.lod` 目录表——当前每次都 `fs::read` 整个文件（数百 MB 级）；
//! - `res.dir_scan`：`Content/` 目录递归扫描——每解析一个资源都全量走一遍目录树；
//! - `res.zip_read`：从 `content.zip` 解压单个条目；
//! - `def.decode`：DEF 动画解码（含逐帧像素还原）；
//! - `img.encode` / `img.rgba`：webp 编码（有损小图 / 无损大图）；
//! - `town.preview`：城镇预览图合成；
//! - `cfg.*` / `ent.*`：单 mod 构建的配置解析与实体构建阶段；
//! - `io.write`：`cache.json` / `raw.json` 落盘。
//!
//! 设计约束：
//! - **线程局部累计**：构建跑在 rayon 工作线程上，不能每次埋点都抢全局锁；
//! - **必须 drain**：rayon 池内线程复用，故每个 mod 构建结束时调用 [`drain_local`]
//!   取走并清零，再 [`merge_global`] 汇总，否则不同 mod 的耗时会互相串味；
//! - 指标名固定为 `&'static str`，热路径零分配。
//!
//! 依赖方向：本层属 `infra`，不依赖 `vcmi` / `wiki` / `tauri`。

use std::cell::RefCell;
use std::time::Instant;

/// 单个指标的累计值：耗时（微秒）/ 次数 / 字节数。
#[derive(Default, Clone, Copy)]
struct Entry {
    us: u128,
    count: u64,
    bytes: u64,
}

thread_local! {
    static LOCAL: RefCell<Vec<(&'static str, Entry)>> = const { RefCell::new(Vec::new()) };
}

static GLOBAL: std::sync::OnceLock<std::sync::Mutex<Vec<(&'static str, Entry)>>> =
    std::sync::OnceLock::new();

fn local_add(key: &'static str, us: u128, count: u64, bytes: u64) {
    LOCAL.with(|c| {
        let mut v = c.borrow_mut();
        if let Some(i) = v.iter().position(|(k, _)| *k == key) {
            let e = &mut v[i].1;
            e.us += us;
            e.count += count;
            e.bytes += bytes;
        } else {
            v.push((key, Entry { us, count, bytes }));
        }
    });
}

/// 计时 + 计数 1：包裹一段逻辑（推荐用法）。
pub fn scope<T>(key: &'static str, f: impl FnOnce() -> T) -> T {
    let t = Instant::now();
    let r = f();
    local_add(key, t.elapsed().as_micros(), 1, 0);
    r
}

/// 仅计数（如资源命中 / 未命中次数）。
pub fn count(key: &'static str, n: u64) {
    local_add(key, 0, n, 0);
}

/// 累计字节（次数同时 +1，便于看"读了几次 / 共多少"）。
pub fn bytes(key: &'static str, n: u64) {
    local_add(key, 0, 1, n);
}

/// 取走本线程累计并清零（**必须**在每个 mod 构建结束时调用）。
pub fn drain_local() -> Vec<(&'static str, u128, u64, u64)> {
    LOCAL.with(|c| {
        let mut v = c.borrow_mut();
        if v.is_empty() {
            return Vec::new();
        }
        let out: Vec<_> = v
            .iter()
            .map(|(k, e)| (*k, e.us, e.count, e.bytes))
            .collect();
        v.clear();
        out
    })
}

/// 把一份 `drain_local` 的结果并入全局（供批次结束时汇总打印）。
pub fn merge_global(items: &[(&'static str, u128, u64, u64)]) {
    if items.is_empty() {
        return;
    }
    let list = GLOBAL.get_or_init(|| std::sync::Mutex::new(Vec::new()));
    let mut g = list.lock().unwrap_or_else(|e| e.into_inner());
    for (k, us, count, bytes) in items {
        if let Some(i) = g.iter().position(|(n, _)| *n == *k) {
            let e = &mut g[i].1;
            e.us += *us;
            e.count += *count;
            e.bytes += *bytes;
        } else {
            g.push((
                k,
                Entry {
                    us: *us,
                    count: *count,
                    bytes: *bytes,
                },
            ));
        }
    }
}

/// 全局快照：按累计耗时降序取前 `n` 项。
pub fn global_top(n: usize) -> Vec<(&'static str, u128, u64, u64)> {
    let list = GLOBAL.get_or_init(|| std::sync::Mutex::new(Vec::new()));
    let g = list.lock().unwrap_or_else(|e| e.into_inner());
    let mut v: Vec<_> = g
        .iter()
        .map(|(k, e)| (*k, e.us, e.count, e.bytes))
        .collect();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    v.truncate(n);
    v
}

/// 清空全局累计（每批构建开始时调用，避免跨批次相加）。
pub fn reset_global() {
    if let Some(list) = GLOBAL.get() {
        let mut g = list.lock().unwrap_or_else(|e| e.into_inner());
        g.clear();
    }
}

/// 毫秒（保留一位小数）。
fn fmt_ms(us: u128) -> String {
    format!("{:.1}ms", us as f64 / 1000.0)
}

/// 字节数：超过 1MB 用 MB，否则用 KB。
fn fmt_bytes(b: u64) -> String {
    if b >= 1_048_576 {
        format!("{:.1}MB", b as f64 / 1_048_576.0)
    } else if b >= 1024 {
        format!("{:.1}KB", b as f64 / 1024.0)
    } else {
        format!("{b}B")
    }
}

/// 格式化一份指标：按耗时降序取前 `top` 项，形如 `lod.open=12345ms×652(1.2GB)`。
pub fn format_items(items: &[(&'static str, u128, u64, u64)], top: usize) -> String {
    let mut v = items.to_vec();
    v.sort_by(|a, b| b.1.cmp(&a.1));
    v.truncate(top);
    v.iter()
        .map(|(k, us, c, b)| {
            if *b > 0 {
                format!("{k}={}×{c}({})", fmt_ms(*us), fmt_bytes(*b))
            } else {
                format!("{k}={}×{c}", fmt_ms(*us))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// 全局汇总一行（供批次结束的 info 日志）。
pub fn format_global_top(n: usize) -> String {
    format_items(&global_top(n), n)
}
