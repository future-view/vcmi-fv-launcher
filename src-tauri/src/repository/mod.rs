//! 远程领域仓库（设计文档 `docs/plan/todo/远程领域分发方案（供核对）.md`）。
//!
//! 职责：
//! - 领域仓库源地址（多源）的读写（持久化在 `launcher.json` 的 `realms.repository_urls`）
//! - 拉取 / 缓存索引 `index.json`（ETag 条件请求 + TTL + 离线回退缓存）
//! - 下载并安装远程领域（**强制 sha256 校验**后复用 `realms::import_realm`）
//! - 封面图下载与本地缓存（**后端下载**规避 Gitee raw 的 Referer 防盗链）
//!
//! 关键实测约束（勿违背）：
//! - Gitee raw / 附件都是 **302 跳转**，客户端必须跟随重定向
//! - Gitee 返回的 `Content-Type` 是 `text/plain`，**不能靠它判定成功**，只能"2xx + JSON 可解析"
//! - Gitee 附件**不支持 Range**（请求分片会返回全量）：HTTP Range 续传不可用，
//!   但**分片分发**（索引 `parts[]`：整包切成 ≤45 MiB 的多文件，逐片下载后拼回）已实现，
//!   规格见 `docs/features/realms-sharded-package.zh-CN.md`

use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Instant;

use crate::vcmi::paths::VcmiPaths;

/// 索引缓存有效期（秒）：超过则触发重新校验（带 ETag 的条件请求）；期间直接用缓存（零请求）。
/// 30 分钟：官方源实际低频变动（一周数次）；过期后走 SWR（先返回缓存、后台校验）不再阻塞，
/// 手动「检查更新」另有 manual 强制校验路径不受此限（A1/A2 + TTL 调大，2026-09-27）。
const INDEX_TTL_SECS: u64 = 30 * 60;
/// 单个领域包硬上限（Gitee 附件单文件上限 100 MB，见方案 §1.0）。
/// 分片场景下该护栏作用于**单片**（单片 ≤ 45 MiB，故只有当索引被写脏时才会触发）。
const PACKAGE_MAX_BYTES: u64 = 100 * 1024 * 1024;
/// 分片**总量** sanity 上限：分片让「整包 > 100 MB」成为合法形态，
/// 但仍要挡住索引误填的天文数字（否则会一路下载到磁盘满）。
const SHARDED_TOTAL_MAX_BYTES: u64 = 1024 * 1024 * 1024;
/// 单个封面体积上限：防止索引误填大文件拖垮列表。
const COVER_MAX_BYTES: u64 = 2 * 1024 * 1024;
/// 取消下载的哨兵错误：前端识别后不弹错误提示。
pub const REALM_DOWNLOAD_CANCELLED: &str = "REALM_DOWNLOAD_CANCELLED";

/// 被取消的下载任务（领域 id 列表）。
static CANCELLED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/* ----------------------------- 基础工具 ----------------------------- */

/// URL → 缓存文件名（避免把整条 URL 当文件名，Windows 下含非法字符）。
fn url_key(url: &str) -> String {
    let mut h = DefaultHasher::new();
    url.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn index_cache_dir() -> Option<PathBuf> {
    VcmiPaths::launcher_data_dir().map(|d| d.join("cache").join("realmIndex"))
}

fn covers_dir() -> Option<PathBuf> {
    VcmiPaths::launcher_data_dir().map(|d| d.join("cache").join("realmCovers"))
}

fn downloads_dir() -> Option<PathBuf> {
    VcmiPaths::launcher_data_dir().map(|d| d.join("downloads"))
}

/// 当前配置的全部仓库索引地址（多源）。空列表表示未配置任何仓库。
fn read_repository_urls() -> Vec<String> {
    let mut out: Vec<String> = VcmiPaths::read_launcher_config()
        .map(|c| c.realms.repository_urls)
        .unwrap_or_default();
    out.retain(|s| !s.trim().is_empty());
    out
}

/* ----------------------------- HTTP ----------------------------- */

/// HTTP 能力已抽到 `infra/http.rs`（启动器更新 `updater` 与本模块共用）。
/// 子模块经 `use super::*;` 取用：`http_get`（索引/封面）与 `get_send`（流式下载，均内置直连→镜像→代理回退，
/// 并返回实际出口 `Egress` 供台账/N12 告警指名镜像域名）。
pub(crate) use crate::infra::http::{get_send, http_get, Egress};

mod download;
mod index;
mod mods;

pub use download::{cancel_realm_download, ensure_realm_covers, install_remote_realm};
pub use index::{
    add_realm_repository, fetch_realm_index, get_realm_repositories, remove_realm_repository,
    test_realm_repository,
};
pub use mods::{
    auto_check_repos_enabled, cancel_mod_download, delete_mod, fetch_mod_index,
    install_mod_from_repo, list_mod_ledgers,
};
