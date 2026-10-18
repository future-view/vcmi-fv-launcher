//! 带进度 / 可取消的目录复制 —— 首启「用户数据迁移」的通用底座。
//!
//! 刻意做成 infra（**无 VCMI 业务语义**）：`vcmi::paths::migrate_user_data_dir` 负责
//! 「目标已存在则不复制」「staging + rename」等业务规则，本模块只做纯粹的递归复制。
//!
//! 取消哨兵与 `realms::transfer::EXPORT_CANCELLED` 同款（**进程级单任务**）：进程内同时
//! 只会跑一次用户数据迁移，故不必按 id 区分。
//!
//! ⚠️ 中断语义：复制**不保证原子**（文件逐个落盘）。原子性由调用方的 staging + rename
//! 保证 —— 本模块只负责在取消时**尽早**返回，残留清理交给调用方。

use crate::infra::berror::be_err;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// 复制取消哨兵。
static COPY_CANCELLED: AtomicBool = AtomicBool::new(false);

/// 请求取消当前复制（下一个检查点生效）。
pub fn cancel_dir_copy() {
    COPY_CANCELLED.store(true, Ordering::SeqCst);
}

/// 复位取消哨兵（**每次复制开始前必须调用**，否则上一次的取消会污染本轮）。
pub fn reset_dir_copy() {
    COPY_CANCELLED.store(false, Ordering::SeqCst);
}

fn copy_cancelled() -> bool {
    COPY_CANCELLED.load(Ordering::SeqCst)
}

/// 进度回调节流间隔（与领域导出 `ExportReporter` 同款；末帧强制补发）。
const PROGRESS_THROTTLE: Duration = Duration::from_millis(200);

/// 复制进度快照（`total_*` 来自复制前的整树预扫，故首帧即有分母）。
#[derive(Debug, Clone, Copy, Default)]
pub struct CopyProgress {
    pub done_files: u64,
    pub done_bytes: u64,
    pub total_files: u64,
    pub total_bytes: u64,
}

/// 复制结局：`Cancelled` = 用户中途跳过（**残留由调用方清理**）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CopyOutcome {
    Done,
    Cancelled,
}

/// 统计目录体量：`(文件数, 总字节)`。
///
/// 迭代式（显式栈）而非递归：用户目录可能上千层深（模组嵌套），递归会爆栈。
pub fn dir_stats(path: &Path) -> Result<(u64, u64), String> {
    let mut files = 0u64;
    let mut bytes = 0u64;
    let mut stack: Vec<PathBuf> = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let rd = std::fs::read_dir(&dir)
            .map_err(|e| be_err("infra_src_dir_read_failed", &format!("{}", e)))?;
        for entry in rd.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if meta.is_dir() {
                stack.push(entry.path());
            } else if meta.is_file() {
                files += 1;
                bytes += meta.len();
            }
        }
    }
    Ok((files, bytes))
}

/// 递归复制 `src` → `dst`（`dst` 由本函数创建，父目录需已存在）。
///
/// 取消检查点：每个目录出栈时 + 每个文件落盘前（大文件复制本身不中断，
/// 与领域导出「每 4MB 检查」的取舍一致——单文件通常远小于 4MB）。
pub fn copy_dir_progress<F: FnMut(CopyProgress)>(
    src: &Path,
    dst: &Path,
    on_progress: &mut F,
) -> Result<CopyOutcome, String> {
    let (total_files, total_bytes) = dir_stats(src)?;
    let mut p = CopyProgress {
        total_files,
        total_bytes,
        ..Default::default()
    };
    std::fs::create_dir_all(dst).map_err(|e| be_err("cmd_mkdir_failed", &format!("{}", e)))?;

    let mut last = Instant::now();
    let mut stack: Vec<(PathBuf, PathBuf)> = vec![(src.to_path_buf(), dst.to_path_buf())];
    while let Some((from, to)) = stack.pop() {
        if copy_cancelled() {
            return Ok(CopyOutcome::Cancelled);
        }
        let rd = std::fs::read_dir(&from)
            .map_err(|e| be_err("infra_src_dir_read_failed", &format!("{}", e)))?;
        for entry in rd.flatten() {
            if copy_cancelled() {
                return Ok(CopyOutcome::Cancelled);
            }
            let Ok(meta) = entry.metadata() else { continue };
            let target = to.join(entry.file_name());
            if meta.is_dir() {
                std::fs::create_dir_all(&target)
                    .map_err(|e| be_err("cmd_mkdir_failed", &format!("{}", e)))?;
                stack.push((entry.path(), target));
            } else if meta.is_file() {
                std::fs::copy(entry.path(), &target)
                    .map_err(|e| be_err("infra_file_copy_failed", &format!("{}", e)))?;
                p.done_files += 1;
                p.done_bytes += meta.len();
                if last.elapsed() >= PROGRESS_THROTTLE {
                    last = Instant::now();
                    on_progress(p);
                }
            }
        }
    }
    on_progress(p); // 末帧必发（否则前端永远停在 99%）
    Ok(CopyOutcome::Done)
}
