//! 时间工具：UTC 日期与时间戳（不引入 chrono，纯 `std`）。
//!
//! 汇总原先散落在 `logging` / `realms` / `repository` / `commands` 的四份实现
//! （见 `docs/architecture/backend.zh-CN.md` §4.4）。

use std::time::{SystemTime, UNIX_EPOCH};

/// 当前 Unix 时间戳（秒）。
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 文件 mtime（秒）；不存在/无权限返回 0（调用方按「无时间信息」处理）。
pub fn file_mtime_secs(path: &std::path::Path) -> u64 {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 仅刷新文件 mtime 为当前时间，**不碰内容**（`FileTimes`，Rust 1.75+ stable）。
/// 用途：HTTP 304 命中时标记「已验证新鲜」，避免重写整个缓存文件（P6 缓存加强）。
pub fn touch_file_mtime(path: &std::path::Path) {
    if let Ok(f) = std::fs::OpenOptions::new().write(true).open(path) {
        let now = SystemTime::now();
        let _ = f.set_times(std::fs::FileTimes::new().set_modified(now));
    }
}

/// 当前 Unix 时间戳（毫秒）。
pub fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// 自 1970-01-01 的天数 → 公历 `(年, 月, 日)`。
///
/// 采用 Howard Hinnant 的 `civil_from_days` 算法（纯整数运算，无外部依赖）。
pub fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m as u32, d)
}

/// `SystemTime` → UTC 日期串 `YYYY-MM-DD`。
pub fn ymd_utc(t: SystemTime) -> String {
    let secs = t
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// 当前 UTC 日期串 `YYYY-MM-DD`。
///
/// 用 UTC 而非本地时区：避免依赖时区数据（跨平台取值不可靠），
/// 且日志行内时间戳本身即 UTC（`tracing_subscriber` 默认格式）。
pub fn today_utc() -> String {
    ymd_utc(SystemTime::now())
}
