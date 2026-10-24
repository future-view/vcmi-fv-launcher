//! 启动器本地日志模块。
//!
//! - **目录**：`<exe>/.vcmi-fv-launcher/logs/`（由 `VcmiPaths::launcher_data_dir()` 派生，
//!   **不依赖 `launcher.json`**，故可在进程极早期初始化）。
//! - **等级**：由 `launcher.json` 的 `runtime.log_level` 决定（`error|warn|info|debug|trace`），
//!   缺失或非法回退 `info`。
//! - **噪音过滤**：`tauri::protocol::asset` 的 ERROR 一律丢弃（见 [`NOISY_TARGETS`]），
//!   否则"前端试探不存在的 `@Nx` 素材"会把真正的报错淹掉。
//! - **轮转**：按天（UTC）分文件 + 单文件大小上限切分。
//!   `tracing-appender` 的 `RollingFileAppender` 不含「大小切分」与「保留清理」，
//!   故此处自实现 writer 与后台清理线程。
//! - **落盘**：`tracing_appender::non_blocking` 提供「内存缓冲 + 后台线程写出」，
//!   业务线程不因磁盘 IO 阻塞；`WorkerGuard` 存入进程级 `OnceLock`，存活到进程结束。

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::filter::filter_fn;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer as _;

/// 单文件大小上限（默认 5 MB），超出即切分序号分片。
const DEFAULT_MAX_BYTES: u64 = 5 * 1024 * 1024;
/// 日志保留天数（默认 7 天）。
const DEFAULT_KEEP_DAYS: u64 = 7;
/// 过期日志清理周期（默认每小时）。
const CLEAN_INTERVAL: Duration = Duration::from_secs(60 * 60);
/// 日志文件名前缀（清理线程据此识别本模块产物，避免误删其它文件）。
const FILE_PREFIX: &str = "vcmi-fv.";

/// **已知噪音来源**（tracing target）：其 **ERROR 级**一律丢弃，其余级别照常落盘。
///
/// ## `tauri::protocol::asset`
///
/// 图片走 Tauri 资产协议，而前端是**按候选链降序试探**高清变体的
/// （`lib/hd.ts::imgCandidates`：4x → 3x → 2x → 原图）——素材由后端按用途分级生成
/// （`modcache::hd`），大量素材本就没有 `@Nx`（源图已够大的一张都不生成）。
/// 于是"没生成"是**正常状态**，但 Tauri 内建 handler 每次未命中都以 `error!` 记一行，
/// 我们改不了它的级别，只能在这里挡掉。
///
/// 实测（2026-10-23，1.83 目录）：单次会话 286 行同款 ERROR、去重后 197 个 URL，
/// 把真正的报错完全淹掉。
const NOISY_TARGETS: &[&str] = &["tauri::protocol::asset"];

/// 事件是否落盘：**等级过滤** + **噪音来源的 ERROR 过滤**。
///
/// 等级比较方向见 `tracing_core::Level` 的 `Ord`（`ERROR` 最小、`TRACE` 最大）：
/// `*level > max` = 比配置更啰嗦 → 丢弃。
fn keep_event(meta: &tracing::Metadata<'_>, max: tracing::Level) -> bool {
    if *meta.level() > max {
        return false;
    }
    *meta.level() != tracing::Level::ERROR || !NOISY_TARGETS.contains(&meta.target())
}

/// 进程级 guard：**必须存活到进程结束**，否则缓冲日志不会落盘。
static GUARD: OnceLock<WorkerGuard> = OnceLock::new();

/// 日志等级，对应 `launcher.json` 的 `runtime.log_level`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl LogLevel {
    /// 解析配置字符串（大小写不敏感）；缺失或非法值回退 [`LogLevel::Info`]。
    pub fn parse(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "error" => Self::Error,
            "warn" | "warning" => Self::Warn,
            "debug" => Self::Debug,
            "trace" => Self::Trace,
            _ => Self::Info,
        }
    }

    fn as_tracing(self) -> tracing::Level {
        match self {
            Self::Error => tracing::Level::ERROR,
            Self::Warn => tracing::Level::WARN,
            Self::Info => tracing::Level::INFO,
            Self::Debug => tracing::Level::DEBUG,
            Self::Trace => tracing::Level::TRACE,
        }
    }
}

/// 初始化全局日志。**幂等**：重复调用直接返回。
///
/// 应在 `run()` 的 `.setup()` 中、`VcmiPaths::detect()` **之前**调用，
/// 以便覆盖目录初始化 / 领域激活等早期逻辑。
pub fn init(logs_dir: &Path, level: LogLevel) -> io::Result<()> {
    if GUARD.get().is_some() {
        return Ok(());
    }
    fs::create_dir_all(logs_dir)?;

    let (non_blocking, guard) = tracing_appender::non_blocking(RotatingFile::new(
        logs_dir.to_path_buf(),
        DEFAULT_MAX_BYTES,
    ));
    // 存入进程级 static（不 drop）：保证后台写出线程存活到进程结束。
    let _ = GUARD.set(guard);

    // 逐层过滤（`registry` + `filter_fn`）而非 `fmt().with_max_level(...)`：
    // 后者只有**全局等级**，无法按 target 挡掉 [`NOISY_TARGETS`]；
    // 而 `EnvFilter` 需要 `env-filter` feature（多拉 matchers/regex 依赖），此处不必。
    let max = level.as_tracing();
    let _ = tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(non_blocking)
                .with_ansi(false) // 文件日志不输出 ANSI 颜色
                .with_timer(tracing_subscriber::fmt::time::SystemTime)
                .with_filter(filter_fn(move |meta| keep_event(meta, max))),
        )
        .try_init();

    spawn_cleanup(logs_dir.to_path_buf(), DEFAULT_KEEP_DAYS);
    Ok(())
}

/* ------------------------------ 文件 writer ------------------------------ */

/// 按「天 + 单文件大小」轮转的文件 writer。
///
/// 命名：当天首个文件 `vcmi-fv.YYYY-MM-DD.log`，同日超限后为
/// `vcmi-fv.YYYY-MM-DD.1.log`、`.2.log` …
struct RotatingFile {
    dir: PathBuf,
    max_bytes: u64,
    file: Option<File>,
    bytes: u64,
    /// 当前文件所属日期（`YYYY-MM-DD`，UTC）
    date: String,
    /// 当日分片序号（0 = 首个）
    seq: u32,
}

impl RotatingFile {
    fn new(dir: PathBuf, max_bytes: u64) -> Self {
        Self {
            dir,
            max_bytes,
            file: None,
            bytes: 0,
            date: String::new(),
            seq: 0,
        }
    }

    fn open(&mut self, date: &str, seq: u32) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let name = if seq == 0 {
            format!("{FILE_PREFIX}{date}.log")
        } else {
            format!("{FILE_PREFIX}{date}.{seq}.log")
        };
        let f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join(name))?;
        // 续写已存在的文件时，从现有长度起算大小
        self.bytes = f.metadata().map(|m| m.len()).unwrap_or(0);
        self.file = Some(f);
        self.date = date.to_string();
        self.seq = seq;
        Ok(())
    }

    /// 判断是否需要换文件：首次打开 / 跨天（序号归零）/ 同日超限（序号 +1）。
    fn rotate_if_needed(&mut self, incoming: u64) -> io::Result<()> {
        let today = crate::infra::time::today_utc();
        if self.file.is_none() {
            return self.open(&today, 0);
        }
        if self.date != today {
            return self.open(&today, 0);
        }
        if self.bytes + incoming > self.max_bytes {
            let date = self.date.clone();
            let seq = self.seq + 1;
            return self.open(&date, seq);
        }
        Ok(())
    }
}

impl Write for RotatingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.rotate_if_needed(buf.len() as u64)?;
        let n = match self.file.as_mut() {
            Some(f) => f.write(buf)?,
            None => return Err(io::Error::new(io::ErrorKind::Other, "日志文件未打开")),
        };
        self.bytes += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.file.as_mut() {
            Some(f) => f.flush(),
            None => Ok(()),
        }
    }
}

/* ------------------------------ 过期清理 ------------------------------ */

/// 后台线程：定期清理超过保留期的日志文件（按文件修改时间判断）。
fn spawn_cleanup(dir: PathBuf, keep_days: u64) {
    std::thread::spawn(move || {
        let keep = Duration::from_secs(keep_days.saturating_mul(86_400));
        loop {
            cleanup_once(&dir, keep);
            std::thread::sleep(CLEAN_INTERVAL);
        }
    });
}

fn cleanup_once(dir: &Path, keep: Duration) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        // 只清理本模块产物，避免误删同目录其它文件
        if !name.starts_with(FILE_PREFIX) || !name.ends_with(".log") {
            continue;
        }
        let expired = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| now.duration_since(t).ok())
            .map(|age| age > keep)
            .unwrap_or(false);
        if expired && fs::remove_file(&path).is_ok() {
            tracing::info!(target: "logging", "已清理过期日志：{name}");
        }
    }
}

/* ------------------------------ 时间工具 ------------------------------ */

#[cfg(test)]
mod tests {
    use super::{filter_fn, keep_event};
    use std::sync::{Arc, Mutex};
    use tracing::Level;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::Layer as _;

    /// 内存 writer：把格式化后的日志行收集成 `String`（`Arc` 便于跨闭包取出）。
    #[derive(Clone, Default)]
    struct Buf(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Buf {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().write(b)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Buf {
        type Writer = Self;
        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    /// 用与生产同构的 subscriber（等级 `INFO`）跑一段代码，返回落盘内容。
    fn capture(f: impl FnOnce()) -> String {
        let buf = Buf::default();
        let out = buf.clone();
        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .with_writer(buf)
                .with_ansi(false)
                .with_filter(filter_fn(|meta| keep_event(meta, Level::INFO))),
        );
        tracing::subscriber::with_default(subscriber, f);
        let bytes = out.0.lock().map(|g| g.clone()).unwrap_or_default();
        String::from_utf8(bytes).unwrap_or_default()
    }

    /// 噪音来源的 ERROR 被丢弃 —— 本模块存在的唯一理由。
    #[test]
    fn asset_404_error_is_dropped() {
        let out = capture(|| {
            tracing::event!(target: "tauri::protocol::asset", Level::ERROR, "File does not exist at path: x");
        });
        assert!(!out.contains("File does not exist"), "噪音未拦住：{out}");
    }

    /// 同 target 的**非 ERROR** 仍要落盘（不能一刀切静音整个 target）。
    #[test]
    fn noisy_target_keeps_other_levels() {
        let out = capture(|| {
            tracing::event!(target: "tauri::protocol::asset", Level::WARN, "keep me");
        });
        assert!(out.contains("keep me"), "WARN 被误杀：{out}");
    }

    /// 等级过滤照常生效：`INFO` 配置下 DEBUG 不出、ERROR 必须出（非噪音 target）。
    #[test]
    fn level_filter_still_works() {
        let out = capture(|| {
            tracing::event!(target: "wiki", Level::ERROR, "real error");
            tracing::event!(target: "wiki", Level::INFO, "real info");
            tracing::event!(target: "wiki", Level::DEBUG, "noisy debug");
        });
        assert!(out.contains("real error"), "真正的 ERROR 被误杀：{out}");
        assert!(out.contains("real info"));
        assert!(!out.contains("noisy debug"), "等级过滤失效：{out}");
    }
}
