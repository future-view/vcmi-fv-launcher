//! 更新清单：拉取（ETag + TTL 缓存）、解析校验、semver 比较与强制更新判定。
//!
//! 防误伤底线：**清单不可达（未配置 / 404 / 断网）且无任何缓存 → 一律放行**
//! （返回 `hasUpdate=false`），绝不因服务端故障锁死全体用户；强制判定允许
//! 使用任意年龄的缓存（离线场景也能阻断）。

use crate::infra::berror::be_err;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::infra::http::http_get;
use crate::infra::time::now_secs;
use crate::vcmi::paths::VcmiPaths;

use super::{cleanup_stale, set_phase, target, UpdatePhase, MANIFEST_TTL_SECS};

/// 当前启动器版本（与 `tauri.conf.json` 的 `version` 同源，发版两处必须一起改）。
pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// 可选更新的自动提醒间隔：同一版本距上次自动弹窗满 7 天后才再次自动弹（`shouldPrompt`）。
/// 只约束"自动弹窗"这一主动提醒；红点 / 菜单徽标等**被动感知不受限**。
/// 「跳过此版本」仍是永久忽略（该版本）。
const PROMPT_INTERVAL_SECS: u64 = 7 * 24 * 60 * 60;

/* ----------------------------- 清单结构（契约见设计文档 §3） ----------------------------- */

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub version: String,
    pub download: String,
    pub sha256: String,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default, rename = "publishedAt")]
    pub published_at: Option<String>,
    #[serde(default)]
    pub critical: bool,
    /// 多语言更新说明（语言 id → 条目列表）；取词由前端按 ui_lang 做，english 兜底。
    #[serde(default)]
    pub notes: HashMap<String, Vec<String>>,
    /// 更新包形态：`exe`（缺省，兼容旧清单）或 `zip`（便携包）。
    ///
    /// zip 形态的价值：① 一次可替换**多个文件**（含 `.vcmi-fv-launcher/runtime/**` 组件）；
    /// ② 压缩确定生效（实测约 43%），不依赖 CDN 是否开 gzip。
    #[serde(default)]
    pub format: Option<String>,
    /// 包类型提示（**可选**）：`full`（缺省，含 exe）/ `config`（纯配置，不含 exe）。
    ///
    /// ⚠️ 仅用于「应用前」的 UI 预判（"无需关闭启动器"）与日志；**实际行为以包内容为准**
    /// —— 解压后由 `download::staged_has_exe()` 判定，声明错也不会做错事。
    #[serde(default)]
    pub kind: Option<String>,
}

impl ManifestEntry {
    /// 是否为 zip 形态（缺省 / 其它值一律按单 exe 处理，保证旧清单不被改变行为）
    pub fn is_zip(&self) -> bool {
        matches!(self.format.as_deref(), Some("zip"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LauncherManifest {
    #[serde(default)]
    pub latest: Option<String>,
    /// 强制门槛：`current < minSupported` → 强制更新。缺省 = 无强制。
    #[serde(default, rename = "minSupported")]
    pub min_supported: Option<String>,
    #[serde(default)]
    pub versions: Vec<ManifestEntry>,
    /// 放行**明文 http** 的下载地址（缺省 false = 只认 https）。
    ///
    /// 只认两种自担风险的场景（都必须显式声明，默认一律不放行）：
    /// ① 本地预演：`release:preview` 生成的派生索引会写 `allowHttp: true`（回环静态服务）；
    /// ② 自建 CDN 只有 http：由发行方在索引里声明。
    #[serde(default, rename = "allowHttp")]
    pub allow_http: bool,
}

/// IPC 出参 `UpdateInfo`（camelCase 序列化）。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub has_update: bool,
    pub forced: bool,
    pub critical: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes_i18n: Option<HashMap<String, Vec<String>>>,
    /// idle | available | downloading | downloaded | error
    pub state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cached: Option<bool>,
    /// 清单是否已配置（未配置时前端手动检查提示「未配置更新源」）。
    pub configured: bool,
    /// 包类型提示：`full`（缺省，含 exe，需关闭启动器替换）/ `config`（纯配置，可直接应用）
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// 是否应**自动弹出**更新弹窗（可选更新感知 L4）：
    /// 有可选更新且未跳过时，每版本首次发现立即为 true；同版本距上次自动弹窗满 7 天才再次为 true。
    /// 手动检查（`force=true`）与强制更新恒为 false（前者由用户主动打开弹窗，后者走 `ForcedUpdateOverlay`）。
    pub should_prompt: bool,
    /// 是否已**下载完成、待用户应用**（"更新包已就绪"）。与 `has_update` 不同：
    /// 它来自持久化的"待应用版本"（`state.json::pendingApplyVersion`），跨进程 / 跨会话恢复，
    /// **不受 7 天自动弹窗节流**——每次启动都应提示用户「点击重启并应用」，直到被应用或跳过。
    pub ready_to_apply: bool,
}

/* ----------------------------- semver 比较 ----------------------------- */

/// 解析 `major.minor.patch…`（≥1 段纯数字），`-pre` 后缀记为 pre-release。
fn parse_ver(v: &str) -> Option<(Vec<u64>, bool)> {
    let v = v.trim();
    // 去掉构建元数据（+build）
    let core = v.split_once('+').map(|(c, _)| c).unwrap_or(v);
    let (core, pre) = match core.split_once('-') {
        Some((c, _)) => (c, true),
        None => (core, false),
    };
    let mut nums = Vec::new();
    for seg in core.split('.') {
        if seg.is_empty() || !seg.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        nums.push(seg.parse::<u64>().ok()?);
    }
    if nums.is_empty() {
        return None;
    }
    Some((nums, pre))
}

/// 逐段数值比较（缺段按 0）；同号时 pre-release 低于正式版。任一侧无法解析 → None。
pub(crate) fn semver_cmp(a: &str, b: &str) -> Option<Ordering> {
    let (na, pa) = parse_ver(a)?;
    let (nb, pb) = parse_ver(b)?;
    let len = na.len().max(nb.len());
    for i in 0..len {
        let x = na.get(i).copied().unwrap_or(0);
        let y = nb.get(i).copied().unwrap_or(0);
        if x != y {
            return Some(x.cmp(&y));
        }
    }
    Some(pa.cmp(&pb))
}

/* ----------------------------- 磁盘缓存 ----------------------------- */

struct ManifestCache {
    etag: Option<String>,
    fetched_at: u64,
    manifest: Value,
}

fn cache_path() -> Option<PathBuf> {
    crate::vcmi::paths::VcmiPaths::launcher_data_dir()
        .map(|d| d.join("cache").join("updateManifest").join("manifest.json"))
}

fn load_cache(url: &str) -> Option<ManifestCache> {
    let text = fs::read_to_string(cache_path()?).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    if v.get("url").and_then(|x| x.as_str()) != Some(url) {
        return None;
    }
    Some(ManifestCache {
        etag: v.get("etag").and_then(|x| x.as_str()).map(|s| s.to_string()),
        fetched_at: v.get("fetchedAt").and_then(|x| x.as_u64()).unwrap_or(0),
        manifest: v.get("manifest").cloned().unwrap_or(Value::Null),
    })
}

fn save_cache(url: &str, etag: Option<&str>, manifest: &Value) {
    let Some(p) = cache_path() else { return };
    if let Some(dir) = p.parent() {
        if fs::create_dir_all(dir).is_err() {
            return;
        }
    }
    let v = serde_json::json!({
        "url": url,
        "etag": etag,
        "fetchedAt": now_secs(),
        "manifest": manifest,
    });
    if let Ok(text) = serde_json::to_string_pretty(&v) {
        let _ = fs::write(p, text);
    }
}

/* ----------------------------- 拉取与解析 ----------------------------- */

/// 拉取清单：`force=false` 时 TTL 内直接用缓存；失败时任意年龄缓存兜底。
/// 返回 `(清单, 是否来自缓存)`。
fn fetch_manifest(url: &str, force: bool) -> Result<(LauncherManifest, bool), String> {
    let cache = load_cache(url);
    if !force {
        if let Some(c) = &cache {
            if now_secs().saturating_sub(c.fetched_at) < MANIFEST_TTL_SECS {
                if let Ok(m) = serde_json::from_value::<LauncherManifest>(c.manifest.clone()) {
                    return Ok((m, true));
                }
            }
        }
    }
    let etag = cache.as_ref().and_then(|c| c.etag.clone());
    match http_get(url, etag.as_deref()) {
        Ok((304, new_etag, _)) => {
            if let Some(c) = cache {
                save_cache(url, new_etag.or(c.etag.clone()).as_deref(), &c.manifest);
                if let Ok(m) = serde_json::from_value::<LauncherManifest>(c.manifest.clone()) {
                    return Ok((m, true));
                }
            }
            Err(be_err("upd_server_304_no_cache", ""))
        }
        Ok((_, new_etag, bytes)) => {
            let manifest = parse_manifest(&bytes)?;
            let v = serde_json::to_value(&manifest)
                .map_err(|e| be_err("updater_manifest_serialize_failed", &format!("{}", e)))?;
            save_cache(url, new_etag.as_deref(), &v);
            Ok((manifest, false))
        }
        Err(e) => {
            // 拉取失败：任意年龄缓存兜底（强制判定离线可用）；无缓存才把错误抛出
            if let Some(c) = cache {
                if let Ok(m) = serde_json::from_value::<LauncherManifest>(c.manifest.clone()) {
                    return Ok((m, true));
                }
            }
            Err(e)
        }
    }
}

fn parse_manifest(bytes: &[u8]) -> Result<LauncherManifest, String> {
    let text = String::from_utf8_lossy(bytes);
    let v: Value = serde_json::from_str(text.trim())
        .map_err(|e| be_err("upd_manifest_invalid_json", &format!("{}", e)))?;
    let m: LauncherManifest = serde_json::from_value(v)
        .map_err(|e| be_err("upd_manifest_schema_invalid", &format!("{}", e)))?;
    if m.versions.is_empty() {
        return Err(be_err("updater_manifest_empty_versions", ""));
    }
    Ok(m)
}

/// 下载地址协议校验：https 恒放行；http 需**显式放行**（索引 `allowHttp` 或环境变量
/// `VCMI_FV_UPDATER_ALLOW_HTTP=1`）；其它协议一律拒绝。
///
/// 为什么要显式放行：`sha256` 只能防损坏、**防不了调包**（能改索引的人同时能改 sha）。
/// 放行 http 即接受"更新包可被中间人替换"，故默认绝不放行（见 `LauncherManifest::allow_http`）。
fn download_scheme_ok(url: &str, allow_http: bool) -> bool {
    if url.starts_with("https://") {
        return true;
    }
    if !url.starts_with("http://") {
        return false;
    }
    allow_http || std::env::var("VCMI_FV_UPDATER_ALLOW_HTTP").map(|v| v == "1").unwrap_or(false)
}

/// 定位目标版本条目：`latest` 优先，否则取 semver 最大者；校验下载协议 + sha256 必填。
fn resolve_target(m: &LauncherManifest) -> Result<ManifestEntry, String> {
    let entry = m
        .latest
        .as_ref()
        .and_then(|lv| m.versions.iter().find(|e| e.version == *lv))
        .or_else(|| {
            m.versions
                .iter()
                .max_by(|a, b| semver_cmp(&a.version, &b.version).unwrap_or(Ordering::Equal))
        })
        .ok_or_else(|| be_err("updater_manifest_no_version", ""))?;
    if !download_scheme_ok(&entry.download, m.allow_http) {
        return Err(be_err("updater_manifest_dl_not_https", ""));
    }
    if entry.download.starts_with("http://") {
        tracing::warn!(target: "updater", "更新包走明文 http（已显式放行）：{}", entry.download);
    }
    if entry.sha256.trim().is_empty() {
        return Err(be_err("updater_manifest_no_sha", ""));
    }
    Ok(entry.clone())
}

/* ----------------------------- 入口：检查更新 ----------------------------- */

/// 检查更新（异常一律折叠进 `UpdateInfo`，不返回 Err）：
/// `force=false` 为启动静默检查（任何失败只记日志、不回显）；
/// `force=true` 为手动检查（失败填 `error` 字段供界面回显）。
pub fn check_update(force: bool) -> UpdateInfo {
    // 调试构建不参与版本契约：跳过清单拉取与版本比较，避免开发者本机误报「有更新」。
    // 与 apply.rs 的 `cfg!(debug_assertions)` 守卫一致（debug 包既不检查、也不应用）。
    // 设 `VCMI_FV_UPDATER_TEST=1` 可临时绕过，用于在 debug 中联调 updater UI。
    #[cfg(debug_assertions)]
    if std::env::var("VCMI_FV_UPDATER_TEST").map(|v| v != "1").unwrap_or(true) {
        return UpdateInfo {
            current_version: CURRENT_VERSION.to_string(),
            latest_version: String::new(),
            has_update: false,
            forced: false,
            critical: false,
            size: None,
            published_at: None,
            notes_i18n: None,
            state: "idle",
            error: None,
            cached: None,
            configured: false,
            kind: None,
            should_prompt: false,
            ready_to_apply: false,
        };
    }
    let mut info = UpdateInfo {
        current_version: CURRENT_VERSION.to_string(),
        latest_version: String::new(),
        has_update: false,
        forced: false,
        critical: false,
        size: None,
        published_at: None,
        notes_i18n: None,
        state: "idle",
        error: None,
        cached: None,
        configured: false,
        kind: None,
        should_prompt: false,
        ready_to_apply: false,
    };
    let cfg = VcmiPaths::read_launcher_config();
    let url = cfg
        .as_ref()
        .and_then(|c| c.update.manifest_url.clone())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let Some(url) = url else {
        return info; // 未配置清单：完全不检查（零网络行为）
    };
    info.configured = true;
    if !force && !cfg.as_ref().map(|c| c.update.auto_check).unwrap_or(true) {
        return info; // 用户关闭了自动检查
    }
    cleanup_stale();
    match fetch_manifest(&url, force) {
        Ok((manifest, cached)) => {
            info.cached = Some(cached);
            match resolve_target(&manifest) {
                Ok(entry) => {
                    info.latest_version = entry.version.clone();
                    info.critical = entry.critical;
                    info.size = entry.size;
                    info.published_at = entry.published_at.clone();
                    info.notes_i18n = Some(entry.notes.clone()).filter(|m| !m.is_empty());
                    info.kind = entry.kind.clone();
                    // 强制判定：current < minSupported（缓存命中同样生效 → 离线可阻断）
                    info.forced = manifest
                        .min_supported
                        .as_deref()
                        .map(|s| !s.trim().is_empty())
                        .unwrap_or(false)
                        .then(|| {
                            manifest
                                .min_supported
                                .as_deref()
                                .and_then(|ms| semver_cmp(CURRENT_VERSION, ms.trim()))
                                .map(|o| o == Ordering::Less)
                                .unwrap_or(false)
                        })
                        .unwrap_or(false);
                    // skip 只对普通更新生效（强制更新忽略）
                    let skip = if force || info.forced {
                        None
                    } else {
                        cfg.as_ref().and_then(|c| c.update.skip_version.clone())
                    };
                    let skipped = skip.as_deref() == Some(entry.version.as_str());
                    // 纯配置更新包**不改 exe 版本**：若该条目（按 sha256 + exe 版本）已应用过，
                    // 视为已最新 —— 否则会每次启动都提示同一版本可更新。
                    let has = semver_cmp(CURRENT_VERSION, &entry.version)
                        .map(|o| o == Ordering::Less)
                        .unwrap_or(false)
                        && !super::is_applied(&entry.sha256);
                    // 待应用（已下载完成、待用户重启生效）：跨进程 / 跨会话恢复点。
                    // 这是**未竟事务**而非"新版本提醒"，故**不受 7 天节流**：
                    // 每次启动都应提示用户「更新包已就绪」，直到被应用 / 跳过。
                    if skip.as_deref().is_some() {
                        // 用户已显式跳过该版本 → 清掉待应用标记，避免跨会话仍提示
                        super::set_pending_apply(None);
                    }
                    let pending = super::pending_apply().as_deref() == Some(entry.version.as_str());
                    if pending && !skipped && !super::is_applied(&entry.sha256) && super::download::staged_ready(&entry) {
                        info.has_update = true;
                        info.state = "downloaded";
                        info.ready_to_apply = true;
                        // 同步内部阶段：跨会话恢复的「待应用」也必须把 phase 拉到 Downloaded，
                        // 否则「关闭时自动应用」的判定（phase==Downloaded）在本会话永不成立
                        // （实测踩坑：重启后只填 info 不设 phase，关闭时静默跳过，pending 永远悬着）。
                        set_phase(UpdatePhase::Downloaded);
                        // 仅当下方 `has && !skipped` 分支未接管 target 时补设（避免移动 entry 造成借用冲突）
                        if super::target().is_none() {
                            *super::target() = Some(entry.clone());
                        }
                        // 不覆盖 should_prompt：待应用走启动期 NoticeBar，与 7 天节流无关
                    }
                    if has && !skipped {
                        // 可选更新的**自动弹窗节流（7 天）**：仅静默检查参与；
                        // 版本号变化（新版本首次发现）→ 立即提醒，不受 7 天限制。
                        info.should_prompt = !force && {
                            let (pv, pa, pev) = cfg
                                .as_ref()
                                .map(|c| {
                                    (
                                        c.update.prompt_version.as_deref(),
                                        c.update.prompt_at,
                                        c.update.prompt_exe_version.as_deref(),
                                    )
                                })
                                .unwrap_or((None, None, None));
                            // 用户「应用后回退」（如 0.1.0→0.2.0→手工换回 0.1.0）：
                            // 此时 `appliedUpdateVersion`（0.2.0）!= 当前 exe 版本（0.1.0），
                            // 但 `prompt_exe_version` 因回环又等于当前版本 → 上面的节流会误判"未过期"。
                            // 这里显式识别回退，让节流重新生效、重新提示（见 `updater::applied_version`）。
                            let reverted = super::applied_version()
                                .map(|v| v != CURRENT_VERSION)
                                .unwrap_or(false);
                            match pv {
                                // 节流只在「同一目标版本 + 同一 exe 版本 + 未回退」下生效：
                                // exe 版本变了（升级 / 降级 / 手工换包 / 应用后回退）→ 旧记录失效，重新提醒。
                                // `prompt_exe_version` 缺失（存量配置）同样视为失效 → 天然自愈。
                                Some(v)
                                    if v == entry.version.as_str()
                                        && pev == Some(CURRENT_VERSION)
                                        && !reverted =>
                                {
                                    now_secs().saturating_sub(pa.unwrap_or(0)) >= PROMPT_INTERVAL_SECS
                                }
                                _ => true,
                            }
                        };
                        info.has_update = true;
                        *target() = Some(entry);
                        // 仅在非下载/已下载阶段才重置为 Available，避免重查打断下载进度
                        if matches!(
                            super::phase(),
                            UpdatePhase::Idle | UpdatePhase::Available | UpdatePhase::Error
                        ) {
                            set_phase(UpdatePhase::Available);
                        }
                    } else if super::phase() == UpdatePhase::Available {
                        // 无更新且此前只是"可更新"提示 → 清理；下载中/已下载状态不动
                        *target() = None;
                        set_phase(UpdatePhase::Idle);
                    }
                    // 阶段同步：下载中/已下载时重开弹窗能从 info.state 恢复现场
                    match super::phase() {
                        UpdatePhase::Downloading => info.state = "downloading",
                        UpdatePhase::Downloaded => info.state = "downloaded",
                        _ => {}
                    }
                }
                Err(e) => {
                    if force {
                        info.error = Some(e);
                        info.state = "error";
                    } else {
                        tracing::warn!(target: "updater", "清单解析失败（静默跳过）: {e}");
                    }
                }
            }
        }
        Err(e) => {
            if force {
                info.error = Some(e);
                info.state = "error";
            } else {
                tracing::warn!(target: "updater", "更新清单拉取失败（静默跳过，不阻塞使用）: {e}");
            }
        }
    }
    info
}
