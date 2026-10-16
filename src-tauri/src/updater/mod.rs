//! 启动器自动更新：清单检查 → 下载校验 → Windows 就地替换。
//!
//! 分层硬约束：本模块**禁 `tauri` 类型**（`AppHandle` / `State`）——
//! 下载进度用回调上报，事件 emit 只在 `commands/updater.rs`。
//!
//! 三段流水线（设计文档 `docs/features/launcher-update.zh-CN.md`）：
//! - [`manifest`]：清单拉取（ETag + TTL 24h + 磁盘缓存）、semver 比较、`minSupported` 强制判定；
//! - [`download`]：流式下载到 `downloads/` + sha256 强校验 + 取消哨兵；
//! - [`apply`]：可写性探测 → ASCII `.cmd` 脚本（路径经命令行参数传入）→ detached spawn。

use crate::infra::berror::be_err;
pub mod apply;
pub mod config_patch;
pub mod delete_patch;
pub mod download;
pub mod manifest;
pub mod migrate;
pub mod platform;

use std::fs;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

/// 清单缓存有效期（秒）：静默检查超期才重新拉取（手动 `force` 检查忽略 TTL）。
pub(crate) const MANIFEST_TTL_SECS: u64 = 24 * 60 * 60;

/// 更新包体积硬上限：无论清单是否声明 `size`，超限即中止。
pub(crate) const UPDATE_MAX_BYTES: u64 = 200 * 1024 * 1024;

/// 取消下载的哨兵错误：前端识别后不弹错误提示（同 `REALM_DOWNLOAD_CANCELLED` 约定）。
pub const UPDATE_DOWNLOAD_CANCELLED: &str = "UPDATE_DOWNLOAD_CANCELLED";

/* ----------------------------- 取消哨兵 ----------------------------- */

static CANCELLED: Mutex<bool> = Mutex::new(false);

pub(crate) fn is_cancelled() -> bool {
    CANCELLED.lock().map(|v| *v).unwrap_or(false)
}

pub(crate) fn set_cancelled(v: bool) {
    if let Ok(mut c) = CANCELLED.lock() {
        *c = v;
    }
}

/* ----------------------------- 状态机 ----------------------------- */

/// 更新流程状态（跨命令共享，供前端弹窗重开时恢复现场）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdatePhase {
    Idle,
    Available,
    Downloading,
    Downloaded,
    Error,
}

static PHASE: Mutex<UpdatePhase> = Mutex::new(UpdatePhase::Idle);

pub(crate) fn phase() -> UpdatePhase {
    PHASE.lock().map(|p| *p).unwrap_or(UpdatePhase::Idle)
}

pub(crate) fn set_phase(p: UpdatePhase) {
    if let Ok(mut v) = PHASE.lock() {
        *v = p;
    }
}

/// 最近一次检查定位到的目标版本条目（下载 / 应用阶段消费）。
static TARGET: Mutex<Option<manifest::ManifestEntry>> = Mutex::new(None);

pub(crate) fn target() -> MutexGuard<'static, Option<manifest::ManifestEntry>> {
    TARGET.lock().expect("updater target lock poisoned")
}

/* ----------------------------- 已应用记录 ----------------------------- */

/// 标记「该清单条目已应用」——按**清单条目的 sha256** 记录（存 `state.json`）。
///
/// 为什么不用版本号：**纯配置更新包不改 exe 版本**（不含 exe 时 `CURRENT_VERSION` 不变），
/// 只记版本会导致"每次启动都提示同一版本可更新"。而记 sha256 与实际下载内容一一对应，
/// 将来真发新 exe（新 sha）时依旧能正常提示。
pub(crate) fn mark_applied(sha: &str) {
    migrate::set_state_str("appliedUpdateSha", sha.trim());
    // 同时记录"应用时的 exe 版本"：一旦 exe 版本变化（装了新版 / 手动换包），
    // 旧记录自动失效，不会被误判成"已最新"而卡住更新。
    migrate::set_state_str("appliedUpdateVersion", manifest::CURRENT_VERSION);
}

/// 已应用的清单条目 sha256（无记录 → None）
pub(crate) fn applied_sha() -> Option<String> {
    migrate::state_str("appliedUpdateSha")
}

/// 已应用时的 exe 版本号（无记录 → None）。
///
/// 与 `applied_sha` 配合识别「用户已应用某版本后又回退」：回退后
/// `appliedUpdateVersion != CURRENT_VERSION` → 7 天节流应重新生效（见 manifest.rs）。
pub(crate) fn applied_version() -> Option<String> {
    migrate::state_str("appliedUpdateVersion")
}

/// 记录「该条目已应用」，但写入**目标版本号**（`version`，而非 `CURRENT_VERSION`）。
///
/// `mark_applied` 用于纯配置包（应用后 exe 版本不变，记 CURRENT 即可）；
/// exe 更新后版本会变，必须记**目标版本**，否则降级回到旧版本时
/// `appliedUpdateVersion` 与 `CURRENT_VERSION` 仍相等，无法识别「已回退」。
pub(crate) fn mark_applied_version(version: &str, sha: &str) {
    migrate::set_state_str("appliedUpdateSha", sha.trim());
    migrate::set_state_str("appliedUpdateVersion", version.trim());
}

/// 该清单条目是否**已应用过**（sha256 相同 **且** exe 版本未变）
pub(crate) fn is_applied(sha: &str) -> bool {
    applied_sha()
        .map(|s| s.eq_ignore_ascii_case(sha.trim()))
        .unwrap_or(false)
        && migrate::state_str("appliedUpdateVersion").as_deref() == Some(manifest::CURRENT_VERSION)
}

/* ----------------------------- 待应用（已暂存未应用）持久化 ----------------------------- */

/// 记录「已下载完成、待用户应用」的版本号到 `state.json`。
///
/// 用于跨进程 / 跨会话恢复：用户下载后未点「重启并应用」就关掉了启动器，
/// 下次启动据此在启动期直接提示「更新包已就绪」，**不受 7 天自动弹窗节流**约束
/// （这是未竟事务，不是新版本提醒）。
///
/// `None` / 空串 → 清掉（应用成功 / 跳过 / 下载失败 / 取消时调用）。
pub(crate) fn set_pending_apply(version: Option<&str>) {
    let v = version
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .unwrap_or("");
    migrate::set_state_str("pendingApplyVersion", v);
}

/// 当前是否有「已下载待应用」的版本（未设置 / 空串 → None）。
pub(crate) fn pending_apply() -> Option<String> {
    migrate::state_str("pendingApplyVersion")
}

/* ----------------------------- 路径与清理 ----------------------------- */

/// 当前启动器 exe 的绝对路径（**必须原位替换**：数据目录 = exe 同级 `.vcmi-fv-launcher`）。
pub(crate) fn exe_path() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|e| be_err("upd_exe_locate_failed", &format!("{}", e)))
}

/// 启动器数据目录（`<exe>/.vcmi-fv-launcher`）。
pub(crate) fn data_dir() -> Result<PathBuf, String> {
    crate::vcmi::paths::VcmiPaths::launcher_data_dir()
        .ok_or_else(|| be_err("common_data_dir_unknown", ""))
}

/// 更新下载/暂存目录（`<exe>/.vcmi-fv-launcher/downloads`，与 exe 同盘 → `move` 原子）。
pub(crate) fn downloads_dir() -> Result<PathBuf, String> {
    let d = data_dir()?.join("downloads");
    fs::create_dir_all(&d)
        .map_err(|e| be_err("repo_download_dir_create_failed", &format!("{}", e)))?;
    Ok(d)
}

/// 启动期 stale 清理：上次替换残留的 `<exe>.old`、自删失败的 `update-apply.cmd`、
/// 以及 zip 更新包的解压目录（`downloads/staged_*`）。
///
/// **不删下载下来的暂存包本身**（`.exe.update` / `.zip`）：同版本可直接复用免下载
/// —— 下载层会重新校验 sha256 并**按需补解压**（`ensure_staged`），不匹配自动重下。
///
/// ⚠️ `staged_*` 不能**无条件**全删：用户下载后**未应用就关闭**、下次才打开时，
/// `state.json::pendingApplyVersion` 仍指向该版本而暂存目录是唯一可恢复现场 ——
/// 若每次启动都清掉，跨会话的「更新包已就绪」恢复会失效，表现为"下了更新却永远不生效"。
/// 故只删**不与当前待应用版本匹配**的 `staged_*`。
pub(crate) fn cleanup_stale() {
    if let Ok(exe) = exe_path() {
        if let (Some(dir), Some(name)) = (exe.parent(), exe.file_name()) {
            let _ = fs::remove_file(dir.join(format!("{}.old", name.to_string_lossy())));
        }
    }
    if let Ok(d) = data_dir() {
        let _ = fs::remove_file(d.join("update-apply.cmd"));
        let _ = fs::remove_file(d.join("update-apply.ps1"));
        let pending = pending_apply();
        if let Ok(dl) = downloads_dir() {
            if let Ok(rd) = fs::read_dir(&dl) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        if let Some(name) = p.file_name().and_then(|n| n.to_str()) {
                            let is_staged = name.starts_with("staged_");
                            // 与「待应用」标记匹配 → 保留（跨会话恢复用）；其余 stale 全清
                            let is_pending = is_staged
                                && name
                                    .strip_prefix("staged_")
                                    .map(|ver| pending.as_deref() == Some(ver))
                                    .unwrap_or(false);
                            if is_staged && !is_pending {
                                let _ = fs::remove_dir_all(&p);
                            }
                        }
                    }
                }
            }
        }
    }
}
