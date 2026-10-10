//! 模组仓库（获取与更新）：索引拉取/缓存、远程下载、校验、无损就位、安装台账。
//!
//! 设计见 `docs/features/mods-repository.zh-CN.md`（C1/C2/D1 等决策已拍板）：
//! - 仓库源**共用官方 vcmiSettings 键**（C1）：`launcher.defaultRepository*` / `extraRepository*`
//!   （`{user_config}/settings.json`，与官方启动器/原生配置互通）；
//! - 校验（C2）：条目带 `sha256` 一律强校验拒装不符；不带则**弱校验**——以安装后
//!   `mod.json` 的 name/id 比对索引条目，不符仅告警放行；UI 以 `verified|weak` 标注；
//! - 安装语义（D1）：本模块只负责**落盘**（返回 `replaced` 区分「更新」与「新装」），
//!   启用决策恒为玩家显式勾选，此处不写预设。
//!
//! 与领域仓库（`index.rs`/`download.rs`）同构：ETag + TTL 缓存 + 多源合并 + 取消 + 进度事件；
//! 但缓存目录独立（`cache/modIndex`），条目结构是官方 `availableMods` 而非 `realms`。

use crate::infra::berror::be_err;
use super::*;

use serde_json::{json, Value};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};

use crate::realms::{self, base::guard_vcmi_not_running};
use crate::state::AppPaths;
use crate::vcmi::mod_install;

/// mod 下载包体积上限：官方 hota 完整包 ~700MB，领域包的 100MB 上限不适用（方案 C8）。
const MOD_PACKAGE_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// 取消下载的哨兵错误（前端识别后不弹错误提示；与 realm 契约同款）。
pub const MOD_DOWNLOAD_CANCELLED: &str = "MOD_DOWNLOAD_CANCELLED";
/// 官方默认仓库兜底地址（settings.json 缺失该键时；与 `settingsSchema.ts` 默认值一致）。
const OFFICIAL_REPO_FALLBACK: &str =
    "https://raw.githubusercontent.com/vcmi/vcmi-mods-repository/develop/vcmi-1.7.json";

/// 被取消的模组下载（mod id 列表）。
static MOD_CANCELLED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

fn is_mod_cancelled(mod_id: &str) -> bool {
    MOD_CANCELLED
        .lock()
        .map(|v| v.iter().any(|x| x == mod_id))
        .unwrap_or(false)
}

fn clear_mod_cancelled(mod_id: &str) {
    if let Ok(mut v) = MOD_CANCELLED.lock() {
        v.retain(|x| x != mod_id);
    }
}

/* ----------------------------- 版本比较 ----------------------------- */

/// 点分数字版本逐段比较。**权威实现已收敛到 `infra::version::compare`**
/// （模组与领域共用同一口径；前端镜像 `modRepo.compareModVersions`、
/// 仓库索引脚本 `scripts/build-realms-index.mjs` 的 `cmpVersion`）。
/// 本函数保留为模组侧调用别名与历史单测锚点（M3 后端预筛可接入）。
#[allow(dead_code)]
pub(crate) fn cmp_mod_versions(a: &str, b: &str) -> std::cmp::Ordering {
    crate::infra::version::compare(a, b)
}

/// 更新判定：仅当**新版本更高**才算可更新（本地版本领先仓库时不提示降级）。
/// 任一端为空（已装模组未声明版本 / 索引缺 version）无法可靠判定新旧，一律不提示，
/// 与前端 `isModUpdatable` 同一口径。
#[allow(dead_code)]
pub(crate) fn is_mod_update(current: &str, latest: &str) -> bool {
    let (c, l) = (current.trim(), latest.trim());
    !c.is_empty()
        && !l.is_empty()
        && c != l
        && cmp_mod_versions(c, l) == std::cmp::Ordering::Less
}

/* ----------------------------- 仓库源与缓存 ----------------------------- */

fn mod_index_cache_dir() -> Option<PathBuf> {
    VcmiPaths::launcher_data_dir().map(|d| d.join("cache").join("modIndex"))
}

fn mod_downloads_dir() -> Option<PathBuf> {
    VcmiPaths::launcher_data_dir().map(|d| d.join("downloads").join("mods"))
}

/// P6（包缓存复用）：下载过的模组 zip 缓存目录——全局与领域安装共用（替代 C8「校验即删」）。
fn mod_packages_dir() -> Option<PathBuf> {
    VcmiPaths::launcher_data_dir()
        .map(|d| d.join("cache").join("modPackages"))
}

/// 包缓存总容量上限（2GB）：超限按 mtime 从旧到新淘汰（当前包除外）。
const MOD_PKG_CACHE_MAX_BYTES: u64 = 2 * 1024 * 1024 * 1024;

/// 缓存包文件名：`<sanitized modId>__<urlKey 前 16 位>.zip`。
/// URL 变化（新版本发布）→ 文件名变化；安装成功后按 modId 前缀清旧文件 = 「老版本清理」。
fn pkg_file_name(mod_id: &str, url: &str) -> String {
    let id: String = mod_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '*'
            }
        })
        .collect();
    let key = url_key(url);
    format!("{}__{}.zip", id, &key[..key.len().min(16)])
}

/// 安装成功后的包缓存清理：①同 mod 旧版本包（同前缀不同文件）直删；②总容量超限按 LRU mtime 淘汰。
fn prune_mod_packages(keep: &Path) {
    let Some(dir) = mod_packages_dir() else { return };
    let Ok(rd) = fs::read_dir(&dir) else { return };
    let keep_name = keep
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let mod_prefix = keep_name.split("__").next().unwrap_or("").to_string();
    let mut files: Vec<(PathBuf, u64, u64)> = Vec::new();
    for de in rd.flatten() {
        let p = de.path();
        let name = p
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        if name == keep_name {
            continue;
        }
        if !mod_prefix.is_empty() && name.starts_with(&format!("{mod_prefix}__")) {
            let _ = fs::remove_file(&p); // ① 同模组的旧版本
            continue;
        }
        if let Ok(md) = p.metadata() {
            if md.is_file() {
                let mt = crate::infra::time::file_mtime_secs(&p);
                files.push((p, md.len(), mt));
            }
        }
    }
    // ② LRU：总量超限时从最旧开始淘汰
    let keep_len = fs::metadata(keep).map(|m| m.len()).unwrap_or(0);
    let mut total = keep_len + files.iter().map(|(_, s, _)| s).sum::<u64>();
    files.sort_by_key(|(_, _, mt)| *mt);
    for (p, size, _) in files {
        if total <= MOD_PKG_CACHE_MAX_BYTES {
            break;
        }
        let _ = fs::remove_file(&p);
        total = total.saturating_sub(size);
    }
}

/// 读 `{user_config}/settings.json`（VCMI 游戏设置，launcher 键也在其中）。
fn read_vcmi_settings_json(paths: &VcmiPaths) -> Value {
    let p = paths.vcmi_settings_path();
    let Ok(text) = fs::read_to_string(&p) else {
        return Value::Null;
    };
    crate::infra::json::parse_json_lenient(&text).unwrap_or(Value::Null)
}

/// 生效的 mod 仓库源（按声明顺序，先到先得去重）。
/// 返回 `(url, is_default)`——`is_default` 供 `fetch_mod_index` 合并时标源（默认源 / 额外源）。
/// C1：与官方共用 vcmiSettings 键；读不到配置（未初始化）时兜底官方默认仓库。
fn mod_repository_urls(paths: &VcmiPaths) -> Vec<(String, bool)> {
    let s = read_vcmi_settings_json(paths);
    let mut out: Vec<(String, bool)> = Vec::new();
    let launcher = s.get("launcher");
    if let Some(l) = launcher {
        // 官方启动器习惯不写这两个键：**默认源缺失视为启用**（与官方语义/schema 默认 val:true 一致，
        // 曾因 unwrap_or(false) 导致只拉 extra 源）；extra 缺失视为禁用。
        let default_on = l
            .get("defaultRepositoryEnabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(true);
        let extra_on = l
            .get("extraRepositoryEnabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let url = |k: &str| {
            l.get(k)
                .and_then(|v| v.as_str())
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
        };
        if default_on {
            out.push((
                url("defaultRepositoryURL").unwrap_or_else(|| OFFICIAL_REPO_FALLBACK.to_string()),
                true,
            ));
        }
        if extra_on {
            if let Some(u) = url("extraRepositoryURL") {
                out.push((u, false));
            }
        }
    }
    if out.is_empty() {
        out.push((OFFICIAL_REPO_FALLBACK.to_string(), true));
    }
    let mut seen: Vec<String> = Vec::new();
    out.retain(|(u, _)| {
        let keep = !seen.iter().any(|s| s == u);
        if keep {
            seen.push(u.clone());
        }
        keep
    });
    out
}

pub(crate) struct ModIndexCache {
    pub(crate) etag: Option<String>,
    pub(crate) fetched_at: u64,
    pub(crate) index: Value,
}

fn mod_cache_path(url: &str) -> Option<PathBuf> {
    Some(mod_index_cache_dir()?.join(format!("{}.json", url_key(url))))
}

pub(crate) fn load_mod_cache(url: &str) -> Option<ModIndexCache> {
    let p = mod_cache_path(url)?;
    let text = fs::read_to_string(&p).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    if v.get("url").and_then(|x| x.as_str()) != Some(url) {
        return None;
    }
    Some(ModIndexCache {
        etag: v.get("etag").and_then(|x| x.as_str()).map(|s| s.to_string()),
        fetched_at: v.get("fetchedAt").and_then(|x| x.as_u64()).unwrap_or(0),
        index: v.get("index").cloned().unwrap_or(Value::Null),
    })
}

fn save_mod_cache(url: &str, etag: Option<&str>, index: &Value) {
    let Some(dir) = mod_index_cache_dir() else {
        return;
    };
    let _ = fs::create_dir_all(&dir);
    let v = json!({ "url": url, "etag": etag, "fetchedAt": crate::infra::time::now_secs(), "index": index });
    if let Ok(text) = serde_json::to_string(&v) {
        // 原子写（临时文件 + rename）：SWR 后台校验写缓存与前台读缓存视图并发，避免半读文件
        let target = dir.join(format!("{}.json", url_key(url)));
        let tmp = dir.join(format!("{}.json.tmp", url_key(url)));
        if fs::write(&tmp, text).is_ok() {
            let _ = fs::rename(&tmp, &target);
        }
    }
}

/// 条目来源主机名（展示用）：官方索引条目不带 sha256，来源标注是弱校验策略（C2）的可见性配套。
fn repo_host(url: &str) -> String {
    url.split("://")
        .nth(1)
        .unwrap_or(url)
        .split('/')
        .next()
        .unwrap_or("")
        .to_string()
}

/// 规范化单个索引条目（官方 `availableMods` 结构 + FV 扩展 `fv`）。
fn normalize_mod_entry(id: &str, entry: &Value, source_url: &str) -> Value {
    let fv = entry.get("fv");
    json!({
        "id": id,
        "modJsonUrl": entry.get("mod").and_then(|v| v.as_str()).unwrap_or(""),
        "downloadUrl": entry.get("download").and_then(|v| v.as_str()).unwrap_or(""),
        "downloadSize": entry.get("downloadSize").cloned().unwrap_or(Value::Null),
        "githubStars": entry.get("githubStars").cloned().unwrap_or(Value::Null),
        "screenshots": entry.get("screenshots").cloned().unwrap_or(json!([])),
        "version": fv.and_then(|f| f.get("version")).and_then(|v| v.as_str()).unwrap_or(""),
        "name": fv
            .and_then(|f| f.get("name"))
            .and_then(|v| v.as_str())
            .or_else(|| entry.get("name").and_then(|v| v.as_str()))
            .unwrap_or(id),
        // 官方索引条目不带 name/description/modType——这三样靠 enrich_mod_versions 逐条拉 mod.json 回填
        //（与官方启动器行为一致：未下载模组的名称/描述/类型信息同样来自 mod.json）。
        "description": fv
            .and_then(|f| f.get("description"))
            .and_then(|v| v.as_str())
            .or_else(|| entry.get("description").and_then(|v| v.as_str()))
            .unwrap_or(""),
        "modType": fv
            .and_then(|f| f.get("modType"))
            .and_then(|v| v.as_str())
            .or_else(|| entry.get("modType").and_then(|v| v.as_str()))
            .unwrap_or(""),
        // P11：翻译类基准语言（mod.json 的 language）；官方索引条目不带，靠 enrich 回填。
        // 前端据此 + modType 按类型（而非名字）发现语言包。
        "language": fv
            .and_then(|f| f.get("language"))
            .and_then(|v| v.as_str())
            .or_else(|| entry.get("language").and_then(|v| v.as_str()))
            .unwrap_or(""),
        "author": fv
            .and_then(|f| f.get("author"))
            .and_then(|v| v.as_str())
            .or_else(|| entry.get("author").and_then(|v| v.as_str()))
            .unwrap_or(""),
        // P10.1：多语言块（mod.json 的 chinese/polish/... 顶层字段）随 enrich 回填，
        // 前端用 localizedName/localizedDescription 按启动器语言选词（与已装扫描同口径）
        "i18n": fv
            .and_then(|f| f.get("i18n"))
            .cloned()
            .unwrap_or_else(|| entry.get("i18n").cloned().unwrap_or(json!({}))),
        "changelog": fv.and_then(|f| f.get("changelog")).cloned().unwrap_or(Value::Null),
        "sha256": fv.and_then(|f| f.get("sha256")).and_then(|v| v.as_str()).filter(|s| !s.is_empty()),
        "repositoryUrl": source_url,
        "repositoryHost": repo_host(source_url),
    })
}

/* ----------------------------- 命令 ----------------------------- */

/// 拉取模组仓库索引（多源合并，id 先到先得）。TTL 内用缓存；网络失败回退缓存并附 `error`。
/* ----------------------------- 元数据补全（P4 建立 / P7 扩展：与官方更新检测对齐） ----------------------------- */

/// mod.json 元数据缓存 TTL（T2 定案：结果缓存，避免每次检查都批量重拉）。
/// 更新判定不需要高实时性；TTL 内命中不重拉，新条目/过期条目才拉。
const MOD_VERSION_TTL_SECS: u64 = 24 * 3600;
/// 批量拉 mod.json 的并发上限（网络 IO 密集，用独立 rayon 池，不占全局计算池）。
const MOD_VERSION_CONCURRENCY: usize = 8;

#[derive(serde::Serialize, serde::Deserialize, Clone, Default)]
struct CachedModVersion {
    /// 空串 = 拉取成功但 mod.json 无 version（确认无版本，TTL 内不再重拉）
    version: String,
    /// P7：官方索引条目不带名称/描述/类型，一并从 mod.json 提取缓存
    #[serde(default)]
    name: String,
    #[serde(default)]
    description: String,
    /// P10：抽屉详情展示作者（mod.json 的 author，同样回填）
    #[serde(default)]
    author: String,
    /// P10.1：mod.json 多语言块（collect_i18n 产物 {lang:{name,description}}），供抽屉按语言选词
    #[serde(default)]
    i18n: Value,
    /// classify_mod 结果（与已安装扫描同一口径；空 = 旧缓存无此字段，触发一次重拉）
    #[serde(default)]
    mod_type: String,
    /// P11：mod.json 的 `language`（翻译类基准语言）；空 = 旧缓存无此字段，触发一次重拉
    #[serde(default)]
    language: String,
    /// 缓存格式版本（P10=2：新增 author；P10.1 加 i18n = 3；P11 加 language = 4）；
    /// 低于当前值的条目重拉一次升级
    #[serde(default)]
    v: u8,
    fetched_at: u64,
}

/// 当前 modVersions 缓存格式版本（P10 加 author = 2；P10.1 加 i18n = 3；P11 加 language = 4）。
const MOD_VERSION_CACHE_V: u8 = 4;

fn mod_versions_cache_path() -> Option<PathBuf> {
    VcmiPaths::launcher_data_dir()
        .map(|d| d.join("cache").join("modVersions.json"))
}

fn load_mod_version_cache() -> std::collections::HashMap<String, CachedModVersion> {
    mod_versions_cache_path()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_mod_version_cache(map: &std::collections::HashMap<String, CachedModVersion>) {
    if let Some(p) = mod_versions_cache_path() {
        if let Some(dir) = p.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(s) = serde_json::to_string(map) {
            // 原子写（临时文件 + rename）：SWR 秒回视图读缓存与后台 enrich 写盘并发，避免半读
            let tmp = p.with_extension("json.tmp");
            if fs::write(&tmp, s).is_ok() {
                let _ = fs::rename(&tmp, &p);
            }
        }
    }
}

/// 从 mod.json 内容提取展示元数据（官方语义：版本/名称/描述/类型以 mod.json 为准，索引条目不带）。
/// 类型走 `classify_mod`——与已安装扫描（vcmi/scan.rs）同一判定口径，保证分类 chips 两边一致。
#[derive(Clone, Default)]
struct ModJsonMeta {
    version: String,
    name: String,
    description: String,
    author: String,
    i18n: Value,
    mod_type: String,
    /// mod.json 的 `language`：翻译类模组基准语言（如 "chinese"）。
    /// 供前端按类型+语言发现语言包（语言模组自动管理特性），不依赖模组 id 名字。
    language: String,
}

fn parse_mod_json_meta(id: &str, bytes: &[u8]) -> ModJsonMeta {
    let Ok(v) = serde_json::from_slice::<Value>(bytes) else {
        return Default::default();
    };
    let text = |k: &str| -> String {
        v.get(k)
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let factions: Vec<String> = v
        .get("factions")
        .and_then(|f| f.as_object())
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    let name = text("name");
    let description = text("description");
    ModJsonMeta {
        version: text("version"),
        author: text("author"),
        i18n: crate::vcmi::scan::collect_i18n(&v),
        mod_type: crate::vcmi::scan::classify_mod(
            id,
            v.get("modType").and_then(|x| x.as_str()),
            &name,
            &description,
            &factions,
        ),
        language: text("language"),
        name,
        description,
    }
}

/// 条目某字符串字段缺失（无 / 空串）判定（enrich 收集与回填共用）。
fn field_missing(entry: &Value, k: &str) -> bool {
    entry
        .get(k)
        .and_then(|v| v.as_str())
        .map(|s| s.is_empty())
        .unwrap_or(true)
}

/// 给缺元数据的条目补 version/name/description/modType：官方启动器的更新检测与详情展示
/// 就是逐条拉 mod.json，我们同构对齐；FV 聚合索引已预聚合 `fv.*` 的条目直接跳过
///（聚合源退化为加速+sha256 增强）。单条失败静默（保持空=不提示，与官方一致）；
/// 出口粘性（T2）保证批量拉取不逐条白等直连。
/// P9（D1/D2 定案）：拉取阶段向前端推 `mod-index-progress` {phase:"meta", done, total}（200ms 节流），
/// 只报进度不动列表——列表仍在命令 resolve 后一次性刷新（避免逐条重排抖动）。
fn enrich_mod_versions(
    app: &AppHandle,
    merged: &mut std::collections::BTreeMap<String, Value>,
    index_changed: bool,
    prune_orphans: bool,
) {
    let mut cache = load_mod_version_cache();
    let now = crate::infra::time::now_secs();
    // 1) 收集任务：版本/名称/类型任一缺失 + 有 modJsonUrl + 缓存过期/缺失（同 URL 去重）。
    //    A2 指纹联动：`index_changed=false`（本轮全源 304 / TTL 内，索引未变）时命中条目忽略
    //    24h TTL 并续期——远端没变就永不触发全量重拉（时间驱动 → 事件驱动）。
    let mut jobs: Vec<(String, String)> = Vec::new();
    for (id, entry) in merged.iter() {
        let name_is_id = entry
            .get("name")
            .and_then(|v| v.as_str())
            .map(|s| s.is_empty() || s == id)
            .unwrap_or(true);
        if !field_missing(entry, "version") && !name_is_id && !field_missing(entry, "modType") {
            continue; // 聚合源已带齐元数据，不拉
        }
        let url = match entry.get("modJsonUrl").and_then(|v| v.as_str()) {
            Some(u) if !u.is_empty() => u.to_string(),
            _ => continue,
        };
        // 旧格式缓存（无 name 或版本低于当前）视为不过期，升级重拉一次
        let valid = cache
            .get(&url)
            .map(|c| c.v >= MOD_VERSION_CACHE_V && !c.name.is_empty())
            .unwrap_or(false);
        let within_ttl = cache
            .get(&url)
            .map(|c| now.saturating_sub(c.fetched_at) < MOD_VERSION_TTL_SECS)
            .unwrap_or(false);
        if valid && (within_ttl || !index_changed) {
            // A2：索引未变 → 顺带续期（把「最后一次验证」推进到本轮，滚动 24h 窗口）
            if !index_changed {
                if let Some(c) = cache.get_mut(&url) {
                    c.fetched_at = now;
                }
            }
        } else if !jobs.iter().any(|(_, u)| u == &url) {
            jobs.push((id.clone(), url));
        }
    }
    // 2) 并发拉取（独立小池；失败条目不入 results，下次检查重试）
    if !jobs.is_empty() {
        use rayon::prelude::*;
        // P9 进度：total = 本轮真需拉取数（缓存命中高时很小，秒完属正常）
        let meta_total = jobs.len() as u64;
        let meta_done = std::sync::atomic::AtomicU64::new(0);
        let meta_throttle = std::sync::Mutex::new(Instant::now());
        let _ = app.emit(
            "mod-index-progress",
            json!({ "phase": "meta", "done": 0u64, "total": meta_total }),
        );
        let results: std::sync::Mutex<std::collections::HashMap<String, ModJsonMeta>> =
            Default::default();
        if let Ok(pool) = rayon::ThreadPoolBuilder::new()
            .num_threads(MOD_VERSION_CONCURRENCY)
            .build()
        {
            pool.install(|| {
                jobs.par_iter().for_each(|(id, url)| {
                    if let Ok((_, _, bytes)) = http_get(url, None) {
                        // 全空 = 确认无元数据（也缓存，避免反复拉坏条目；fresh 判 name 兜底重拉）
                        let meta = parse_mod_json_meta(id, &bytes);
                        if let Ok(mut m) = results.lock() {
                            m.insert(url.clone(), meta);
                        }
                    }
                    // 无论成败都计入 done（失败静默继续，但进度不能卡住）；200ms 节流（照搬下载进度模式）
                    let done = meta_done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                    let due = meta_throttle
                        .lock()
                        .map(|mut t| {
                            if done >= meta_total || t.elapsed() >= Duration::from_millis(200) {
                                *t = Instant::now();
                                true
                            } else {
                                false
                            }
                        })
                        .unwrap_or(true);
                    if due {
                        let _ = app.emit(
                            "mod-index-progress",
                            json!({ "phase": "meta", "done": done, "total": meta_total }),
                        );
                    }
                });
            });
        }
        let entries: Vec<(String, ModJsonMeta)> = results
            .lock()
            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
            .unwrap_or_default();
        for (url, meta) in entries {
            cache.insert(
                url,
                CachedModVersion {
                    version: meta.version,
                    name: meta.name,
                    description: meta.description,
                    author: meta.author,
                    i18n: meta.i18n,
                    mod_type: meta.mod_type,
                    language: meta.language,
                    v: MOD_VERSION_CACHE_V,
                    fetched_at: now,
                },
            );
        }
    }
    // 3) 回写（过期缓存兜底：本轮拉失败的条目仍显示上次已知值，判新旧恒走 is_mod_update 空值语义）
    fill_merged_from_versions(&cache, merged);
    // 4) A2 孤儿清理：索引本轮真变且全部源成功 → 剪除 merged 不再引用的缓存条目
    //    （旧索引/旧 URL 遗留；部分源失败时跳过，避免误删失败源的条目）
    if prune_orphans {
        let live: std::collections::HashSet<String> = merged
            .values()
            .filter_map(|e| e.get("modJsonUrl").and_then(|v| v.as_str()))
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
        cache.retain(|url, _| live.contains(url));
    }
    save_mod_version_cache(&cache);
}

/// 用 modVersions 缓存回填 merged 条目缺失字段（enrich step 3；无网络、无写盘——
/// 完整 enrich 与 SWR 秒回视图 `cached_index_view` 共用）。
fn fill_merged_from_versions(
    cache: &std::collections::HashMap<String, CachedModVersion>,
    merged: &mut std::collections::BTreeMap<String, Value>,
) {
    for (id, entry) in merged.iter_mut() {
        let Some(url) = entry.get("modJsonUrl").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(c) = cache.get(url) else { continue };
        let mut fill = |key: &str, val: &str| {
            if field_missing(entry, key) && !val.is_empty() {
                entry[key] = Value::String(val.to_string());
            }
        };
        fill("version", &c.version);
        fill("description", &c.description);
        fill("author", &c.author);
        fill("modType", &c.mod_type);
        // P11：翻译类基准语言回填（供前端按类型+语言发现语言包）
        fill("language", &c.language);
        // P10.1：多语言块回写（条目自身无 i18n 且缓存有 → 填入，供前端按启动器语言选词）
        let cache_has_i18n = c.i18n.as_object().map_or(false, |o| !o.is_empty());
        if cache_has_i18n {
            let entry_no_i18n = entry
                .get("i18n")
                .and_then(|v| v.as_object())
                .map(|o| o.is_empty())
                .unwrap_or(true);
            if entry_no_i18n {
                entry["i18n"] = c.i18n.clone();
            }
        }
        if !c.name.is_empty() {
            let stale_name = entry
                .get("name")
                .and_then(|v| v.as_str())
                .map(|s| s.is_empty() || s == id)
                .unwrap_or(true);
            if stale_name {
                entry["name"] = Value::String(c.name.clone());
            }
        }
    }
}

/// 索引结构兼容（真机踩坑）：官方主索引是 `{"availableMods": {…}}`，而 **1.4-archive 索引是裸
/// `{id: {…}}` 映射**（无包裹层）。缺 availableMods 但顶层确为条目集合时包一层，下游只看 availableMods。
fn coerce_mod_index(v: Value) -> Value {
    if v.get("availableMods").and_then(|x| x.as_object()).is_some() {
        return v;
    }
    let Some(map) = v.as_object() else { return v };
    let looks_like_entries = map
        .values()
        .next()
        .map(|e| e.get("mod").is_some() || e.get("download").is_some())
        .unwrap_or(false);
    if map.is_empty() || !looks_like_entries {
        return v;
    }
    json!({ "availableMods": map.clone() })
}

/* ----------------- A1（SWR）辅助：探测 / 秒回视图 / 后台校验调度 ----------------- */

/// SWR 后台校验去重标记：同一时刻至多一个后台 full 在跑（多入口连点不重复触发）。
static SWR_RUNNING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// 前后台 full 路径串行锁：SWR 后台校验可能与 force 阻塞路径并发，共用锁避免缓存写竞争。
static FETCH_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 逐源探测缓存状态（只读，不发网络）：(是否有任一缓存, 是否全部源 TTL 内)。
fn probe_index_cache(urls: &[(String, bool)]) -> (bool, bool) {
    let mut any_cache = false;
    let mut all_fresh = true;
    for (url, _) in urls {
        let cached = load_mod_cache(url);
        let cache_file = mod_cache_path(url);
        let cache_fetched = cached.as_ref().map(|c| {
            let mt = cache_file
                .as_ref()
                .map(|p| crate::infra::time::file_mtime_secs(p))
                .unwrap_or(0);
            c.fetched_at.max(mt)
        });
        let fresh = cache_fetched
            .map(|f| crate::infra::time::now_secs().saturating_sub(f) < INDEX_TTL_SECS)
            .unwrap_or(false);
        any_cache |= cached.is_some();
        all_fresh &= fresh;
    }
    (any_cache, all_fresh)
}

/// 缓存视图（零网络）：用各源本地缓存构造返回体，元数据仅从 modVersions 缓存回填——
/// 供 SWR 秒回与「TTL 内纯缓存」两条路径共用。
fn cached_index_view(urls: &[(String, bool)]) -> Value {
    let mut merged: std::collections::BTreeMap<String, Value> = std::collections::BTreeMap::new();
    let mut sources: Vec<Value> = Vec::new();
    let mut any_ok = false;
    let mut last_error: Option<String> = None;
    for (url, is_default) in urls {
        let cached = load_mod_cache(url);
        let cache_file = mod_cache_path(url);
        let cache_fetched = cached.as_ref().map(|c| {
            let mt = cache_file
                .as_ref()
                .map(|p| crate::infra::time::file_mtime_secs(p))
                .unwrap_or(0);
            c.fetched_at.max(mt)
        });
        let (index, fetched_at, err) = match cached {
            Some(c) => (c.index, cache_fetched.unwrap_or(c.fetched_at), None),
            None => (Value::Null, 0u64, Some("尚未拉取过模组索引".to_string())),
        };
        if err.is_none() {
            any_ok = true;
        } else if last_error.is_none() {
            last_error = err.clone();
        }
        let count = index
            .get("availableMods")
            .and_then(|v| v.as_object())
            .map(|o| o.len())
            .unwrap_or(0);
        sources.push(json!({
            "url": url,
            "ok": err.is_none(),
            "error": err,
            "fetchedAt": if err.is_none() { Some(fetched_at) } else { cache_fetched },
            "modCount": count,
        }));
        if let Some(obj) = index.get("availableMods").and_then(|v| v.as_object()) {
            let source_kind = if *is_default { "default" } else { "extra" };
            for (id, entry) in obj {
                merged.entry(id.to_lowercase()).or_insert_with(|| {
                    let mut v = normalize_mod_entry(id, entry, url);
                    v["sourceKind"] = json!(source_kind);
                    v["sourceUrl"] = json!(url);
                    v
                });
            }
        }
    }
    // 元数据回填（无网络）：只读 modVersions 缓存，不续期、不写盘
    fill_merged_from_versions(&load_mod_version_cache(), &mut merged);
    let mods: Vec<Value> = merged.into_values().collect();
    let error = if any_ok { None } else { last_error };
    json!({ "mods": mods, "sources": sources, "error": error })
}

/// 调度 SWR 后台校验（去重）：force 全量校验所有源；索引真变时发 `mod-index-updated`
/// 事件通知前端重拉（此时缓存已刷新，重拉为 TTL 内秒回）。
fn schedule_background_index_check(app: AppHandle, urls: Vec<(String, bool)>) {
    use std::sync::atomic::Ordering;
    if SWR_RUNNING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return; // 已有后台校验在跑
    }
    std::thread::spawn(move || {
        if let Ok((_, true)) = full_index_fetch(&app, &urls, true) {
            let _ = app.emit("mod-index-updated", json!({}));
        }
        SWR_RUNNING.store(false, Ordering::SeqCst);
    });
}

/// 拉取模组仓库索引（多源合并，id 先到先得）。TTL 内用缓存；网络失败回退缓存并附 `error`。
///
/// P8（真机窗口卡死修复）：Tauri **同步命令在主线程执行**，索引 + enrich 批量拉
/// mod.json（blocking reqwest + rayon 等待）会把消息循环冻结几分钟。命令壳只留
/// 读源配置（本地快 IO），重活整体移交 `spawn_blocking` 专用阻塞线程池。
/// `manual`（A1）：手动「检查更新」路径忽略 TTL 新鲜度，总是触发后台条件校验。
#[tauri::command]
pub async fn fetch_mod_index(
    app: AppHandle,
    state: State<'_, AppPaths>,
    force: bool,
    manual: bool,
) -> Result<Value, String> {
    let urls = {
        let paths = state.0.lock().expect("paths lock poisoned");
        mod_repository_urls(&paths)
    };
    tauri::async_runtime::spawn_blocking(move || fetch_mod_index_blocking(app, urls, force, manual))
        .await
        .map_err(|e| be_err("repo_mod_index_task_failed", &format!("{}", e)))?
}

fn fetch_mod_index_blocking(
    app: AppHandle,
    urls: Vec<(String, bool)>,
    force: bool,
    manual: bool,
) -> Result<Value, String> {
    // A1（SWR 秒回）：!force 时先看缓存——TTL 内且非手动直接返回（零请求）；有缓存但过期
    // （或手动点检）则先返回缓存视图、后台静默校验，远端真变才经 `mod-index-updated`
    // 事件通知前端刷新；首次（无任何缓存）落入下方阻塞全量。
    if !force {
        let (any_cache, all_fresh) = probe_index_cache(&urls);
        if all_fresh && !manual {
            return Ok(cached_index_view(&urls));
        }
        if any_cache {
            schedule_background_index_check(app.clone(), urls.clone());
            return Ok(cached_index_view(&urls));
        }
    }
    full_index_fetch(&app, &urls, force).map(|(v, _)| v)
}

/// 全量路径（首次 / force / SWR 后台校验）：逐源 TTL 判定或条件请求 → 合并 → enrich
/// （A2 指纹联动 + 孤儿清理）。返回 `(IPC 返回体, 索引是否真变)`。
fn full_index_fetch(
    app: &AppHandle,
    urls: &[(String, bool)],
    force: bool,
) -> Result<(Value, bool), String> {
    // 串行化：SWR 后台校验可能与前台 force 阻塞路径并发，共用锁避免缓存写竞争
    let _serial = FETCH_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // 新一轮检查开始：重探直连（T2 粘性生命周期 = 一轮检查；本轮内后续请求复用已验证出口）
    crate::infra::http::reset_sticky();
    let mut merged: std::collections::BTreeMap<String, Value> = std::collections::BTreeMap::new();
    let mut sources: Vec<Value> = Vec::new();
    let mut any_ok = false;
    let mut last_error: Option<String> = None;
    let mut index_changed = false;

    // P9 两阶段进度（D2 定案）：阶段一 = 逐源拉索引（失败也计入 done，卡在哪源可见）；
    // 阶段二 = enrich 批量补元数据。事件 mod-index-progress {phase, done, total}。
    let idx_total = urls.len() as u64;
    for (i, (url, is_default)) in urls.iter().enumerate() {
        let _ = app.emit(
            "mod-index-progress",
            json!({ "phase": "index", "done": (i as u64) + 1, "total": idx_total }),
        );
        let cached = load_mod_cache(url);
        let cache_file = mod_cache_path(url);
        // P6 缓存加强：「最后验证时刻」= max(存的 fetchedAt, 文件 mtime)。
        // 304 命中只 touch mtime、不重写内容——远端无新内容时缓存文件内容零写入。
        let cache_fetched = cached.as_ref().map(|c| {
            let mt = cache_file
                .as_ref()
                .map(|p| crate::infra::time::file_mtime_secs(p))
                .unwrap_or(0);
            c.fetched_at.max(mt)
        });
        let age = cache_fetched
            .map(|f| crate::infra::time::now_secs().saturating_sub(f))
            .unwrap_or(u64::MAX);

        let (index, fetched_at, err) = if !force && age < INDEX_TTL_SECS {
            match cached {
                Some(c) => (c.index, cache_fetched.unwrap_or(c.fetched_at), None),
                None => (Value::Null, 0u64, Some("尚未拉取过模组索引".to_string())),
            }
        } else {
            let etag = cached.as_ref().and_then(|c| c.etag.clone());
            match http_get(url, etag.as_deref()) {
                Ok((304, _, _)) => match cached {
                    Some(c) => {
                        // 304：远端无新内容——touch mtime 标记新鲜，不重写缓存文件（P6）
                        if let Some(p) = &cache_file {
                            crate::infra::time::touch_file_mtime(p);
                        }
                        (
                            c.index,
                            cache_fetched.unwrap_or_else(crate::infra::time::now_secs),
                            None,
                        )
                    }
                    None => (Value::Null, 0u64, Some("尚未拉取过模组索引".to_string())),
                },
                Ok((_, new_etag, bytes)) => {
                    // Gitee Content-Type 不可信：只以「2xx + JSON 可解析 + 有 availableMods（含裸映射兼容）」判定
                    match serde_json::from_slice::<Value>(&bytes) {
                        Ok(raw) => {
                            let index = coerce_mod_index(raw);
                            if index.get("availableMods").and_then(|v| v.as_object()).is_some() {
                                save_mod_cache(url, new_etag.as_deref(), &index);
                                index_changed = true; // A2：索引真变（供补全层续期/清理决策）
                                (index, crate::infra::time::now_secs(), None)
                            } else {
                                (
                                    Value::Null,
                                    0u64,
                                    Some(format!("索引缺少 availableMods 字段（不是模组仓库索引）：{url}")),
                                )
                            }
                        }
                        Err(_) => (Value::Null, 0u64, Some(format!("索引 JSON 解析失败：{url}"))),
                    }
                }
                Err(e) => match cached {
                    Some(c) => (c.index, c.fetched_at, Some(e)),
                    None => (Value::Null, 0u64, Some(e)),
                },
            }
        };

        if err.is_none() {
            any_ok = true;
        } else if last_error.is_none() {
            last_error = err.clone();
        }
        let count = index
            .get("availableMods")
            .and_then(|v| v.as_object())
            .map(|o| o.len())
            .unwrap_or(0);
        sources.push(json!({
            "url": url,
            "ok": err.is_none(),
            "error": err,
            "fetchedAt": if err.is_none() { Some(fetched_at) } else { cache_fetched },
            "modCount": count,
        }));

        if let Some(obj) = index.get("availableMods").and_then(|v| v.as_object()) {
            // 源标记（realm-mods-management §9.4）：默认源 / 额外源——「可下载」页与筛选需要
            // 区分条目来源；`enrich_mod_versions` 只 fill 缺失字段，不会覆盖这两个标记。
            let source_kind = if *is_default { "default" } else { "extra" };
            for (id, entry) in obj {
                merged.entry(id.to_lowercase()).or_insert_with(|| {
                    let mut v = normalize_mod_entry(id, entry, url);
                    v["sourceKind"] = json!(source_kind);
                    v["sourceUrl"] = json!(url);
                    v
                });
            }
        }
    }

    // P4（T1/T2 定案）：与官方启动器同构的更新检测——索引无版本的条目逐条拉 mod.json 补 version
    // （24h 结果缓存 + 并发限流 + 出口粘性）。仅在有源成功时执行，离线降级不触发批量拉取。
    // A2 指纹联动：`index_changed=false`（全源 304/TTL 内）时补全缓存整体续期；孤儿清理仅在
    // 「索引真变且全源成功」时执行（部分源失败时跳过，避免误删失败源的条目）。
    let all_ok = !sources.is_empty()
        && sources
            .iter()
            .all(|s| s.get("ok").and_then(|v| v.as_bool()).unwrap_or(false));
    if any_ok {
        enrich_mod_versions(app, &mut merged, index_changed, all_ok && index_changed);
    }

    let mods: Vec<Value> = merged.into_values().collect();
    let error = if any_ok { None } else { last_error };
    Ok((json!({ "mods": mods, "sources": sources, "error": error }), index_changed))
}

/// T1 定案：机制与官方一致（消费 `launcher.autoCheckRepositories`），但 **FV 默认关**——
/// 缺失/false = 纯手工更新；显式 true 时前端在启动后自动跑一轮静默检查（不打扰弹窗）。
///
/// ⚠️ `async` + `spawn_blocking`：读 settings.json 属磁盘 IO，且本命令在 `boot === "ready"`
/// 后**立刻**被调用（模组页的 T1 自动检查开关）——同步执行会落在页面刚可用的那一帧上。
#[tauri::command]
pub async fn auto_check_repos_enabled(state: State<'_, AppPaths>) -> Result<bool, String> {
    let paths = {
        let guard = state.0.lock().expect("paths lock poisoned");
        guard.clone()
    };
    tauri::async_runtime::spawn_blocking(move || {
        read_vcmi_settings_json(&paths)
            .get("launcher")
            .and_then(|l| l.get("autoCheckRepositories"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    })
    .await
    .map_err(|e| be_err("cmd_blocking_task_failed", &format!("{}", e)))
}

/// 模组管理页「来源/校验」列：批量读全局模组根目录下各模组的 `.fv-install.json` 安装台账。
/// 返回 `{ "<目录名小写>": { source, repositoryUrl, via, check, installedAt } }`；
/// 无台账的模组不出现在结果里（前端展示为「随包/本地」）。
///
/// ⚠️ `async` + `spawn_blocking`：`read_dir` + 每个模组目录一次 `read_to_string`（N≈数百），
/// 同步执行会在「打开模组管理页」时冻结整个窗口。
#[tauri::command]
pub async fn list_mod_ledgers(state: State<'_, AppPaths>) -> Result<Value, String> {
    let dir = {
        let paths = state.0.lock().expect("paths lock poisoned");
        paths.user_mods_dir()
    };
    tauri::async_runtime::spawn_blocking(move || {
        let mut out = serde_json::Map::new();
        let Ok(rd) = fs::read_dir(&dir) else {
            return Ok(Value::Object(out));
        };
        for de in rd.flatten() {
            let p = de.path();
            if !p.is_dir() {
                continue;
            }
            let Ok(s) = fs::read_to_string(p.join(".fv-install.json")) else {
                continue;
            };
            let Ok(v) = serde_json::from_str::<Value>(&s) else {
                continue;
            };
            let str_of = |k: &str| {
                v.get(k)
                    .and_then(|x| x.as_str())
                    .unwrap_or_default()
                    .to_string()
            };
            out.insert(
                de.file_name().to_string_lossy().to_ascii_lowercase(),
                json!({
                    "source": str_of("source"),
                    "repositoryUrl": str_of("repositoryUrl"),
                    "via": str_of("via"),
                    "check": str_of("check"),
                    "installedAt": str_of("installedAt"),
                }),
            );
        }
        Ok(Value::Object(out))
    })
    .await
    .map_err(|e| be_err("cmd_blocking_task_failed", &format!("{}", e)))?
}

/// 物理删除模组目录（模组管理页，P6）。路径护栏：canonicalize 后必须是
/// **用户模组目录或游戏模组目录的直接子目录**（领域私有副本在 realms/ 下，天然越界拒绝）。
/// 二次确认（输入目录名 + 游戏目录额外警示）在前端完成，本命令只守住边界。
///
/// ⚠️ `async` + `spawn_blocking`：`remove_dir_all` 对大模组目录可能很重，
/// 同步执行会在删除时把整个窗口冻结。
#[tauri::command]
pub async fn delete_mod(state: State<'_, AppPaths>, path: String) -> Result<(), String> {
    let (user_mods, game_mods) = {
        let paths = state.0.lock().expect("paths lock poisoned");
        (paths.user_mods_dir(), paths.game_dir.join("Mods"))
    };
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let target =
            fs::canonicalize(&path).map_err(|e| be_err("repo_mod_dir_not_found", &format!("{}", e)))?;
        if !target.is_dir() {
            return Err(be_err("repo_mod_delete_not_dir", ""));
        }
        let ok = [user_mods, game_mods].iter().any(|root| {
            fs::canonicalize(root)
                .map(|r| target.parent() == Some(r.as_path())) // 仅允许直接子目录，防误删根/嵌套异常
                .unwrap_or(false)
        });
        if !ok {
            return Err(be_err("repo_mod_delete_outside", ""));
        }
        let name = target
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        fs::remove_dir_all(&target).map_err(|e| be_err("repo_delete_failed", &format!("{} {}", name, e)))?;
        tracing::info!(target: "mods", "模组管理页物理删除：{}", target.display());
        Ok(())
    })
    .await
    .map_err(|e| be_err("cmd_blocking_task_failed", &format!("{}", e)))?
}

/// 从仓库安装/更新模组（远程获取闭环）。
///
/// - `scope`：`None`/`"global"` = 写全局 `user_data/Mods`（mods 页）；`"realm:<id>"` = 写领域
///   独立空间 `{realm}/vcmi/Mods`（**断链私有副本**，全局与其它领域物理上不受影响，§4.4）。
/// - 下载 → 校验（C2）→ 台账随暂存目录原子就位；失败不动既有安装。
/// - 不写任何预设（D1：启用必须玩家显式勾选；更新保留原启用状态由前端 `replaced` 区分保障）。
#[tauri::command]
pub async fn install_mod_from_repo(
    app: AppHandle,
    state: State<'_, AppPaths>,
    module: String,
    scope: Option<String>,
) -> Result<Value, String> {
    guard_vcmi_not_running()?;

    let (urls, user_mods, realm_mods) = {
        let paths = state.0.lock().expect("paths lock poisoned");
        let realm_mods = match scope.as_deref() {
            Some(s) => {
                let realm_id = s
                    .strip_prefix("realm:")
                    .ok_or_else(|| be_err("repo_scope_invalid", ""))?;
                let id = VcmiPaths::sanitize_realm_id(realm_id);
                realms::base::ensure_realm_dirs(&id)?;
                let base = VcmiPaths::realm_vcmi_dir(&id)
                    .ok_or_else(|| be_err("common_data_dir_unknown", ""))?;
                Some(base.join("Mods"))
            }
            None => None,
        };
        (
            mod_repository_urls(&paths)
                .into_iter()
                .map(|(u, _)| u)
                .collect::<Vec<String>>(),
            paths.user_mods_dir(),
            realm_mods,
        )
    };
    let mods_dir = realm_mods.unwrap_or_else(|| user_mods.clone());

    // P8：下载几十 MB + 解包安装是长阻塞 IO，留在主线程会冻结窗口——移交阻塞线程池。
    tauri::async_runtime::spawn_blocking(move || {
        install_mod_from_repo_blocking(app, module, urls, mods_dir)
    })
    .await
    .map_err(|e| be_err("repo_mod_install_task_failed", &format!("{}", e)))?
}

fn install_mod_from_repo_blocking(
    app: AppHandle,
    module: String,
    urls: Vec<String>,
    mods_dir: PathBuf,
) -> Result<Value, String> {
    // 跨全部源缓存定位条目（与 install_remote_realm 同款：安装不依赖再联网拉索引）
    let located = (|| -> Option<(String, Value)> {
        for u in &urls {
            let Some(c) = load_mod_cache(u) else {
                continue;
            };
            if let Some(obj) = c.index.get("availableMods").and_then(|v| v.as_object()) {
                for (id, e) in obj {
                    if id.eq_ignore_ascii_case(&module) {
                        return Some((u.clone(), e.clone()));
                    }
                }
            }
        }
        None
    })();
    let (repo_url, entry) = located.ok_or_else(|| {
        "尚未加载模组索引，请先在模组页点击「检查更新」或刷新仓库".to_string()
    })?;
    let download = entry
        .get("download")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| be_err("repo_index_entry_no_download", ""))?;

    // P6（包缓存复用）：先查 cache/modPackages，命中即免下载（全局与领域安装共用）；
    // 未命中则下载→校验→**入包缓存**（替代 C8「校验即删」），安装直接以包文件为源。
    let pkg_dir = mod_packages_dir().ok_or_else(|| be_err("common_data_dir_unknown", ""))?;
    fs::create_dir_all(&pkg_dir).map_err(|e| be_err("repo_pack_cache_dir_create_failed", &format!("{}", e)))?;
    let pkg = pkg_dir.join(pkg_file_name(&module, download));
    let expected_sha = entry
        .get("fv")
        .and_then(|f| f.get("sha256"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string());

    let mut sha = String::new();
    let mut via_label = String::new();
    let mut from_cache = false;
    if pkg.is_file() {
        // 命中：重算 sha256（防缓存文件损坏/与强校验源不符）；不符则删缓存重下一次
        match crate::infra::hash::sha256_file(&pkg) {
            Ok(s) => {
                let ok = expected_sha
                    .as_deref()
                    .map(|e| s.eq_ignore_ascii_case(e))
                    .unwrap_or(true);
                if ok {
                    sha = s;
                    via_label = "cache".to_string();
                    from_cache = true;
                } else {
                    let _ = fs::remove_file(&pkg);
                }
            }
            Err(_) => {
                let _ = fs::remove_file(&pkg);
            }
        }
    }
    if !from_cache {
        let dir = mod_downloads_dir().ok_or_else(|| be_err("common_data_dir_unknown", ""))?;
        fs::create_dir_all(&dir).map_err(|e| be_err("repo_download_dir_create_failed", &format!("{}", e)))?;
        let tmp = dir.join(format!("{module}.zip"));
        let _ = fs::remove_file(&tmp);
        via_label = match download_mod_to_file(&app, &module, download, &tmp) {
            Ok(v) => v.label(),
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                return Err(e);
            }
        };
        sha = match crate::infra::hash::sha256_file(&tmp) {
            Ok(s) => s,
            Err(e) => {
                let _ = fs::remove_file(&tmp);
                return Err(e);
            }
        };
        // C2 强校验：仅当条目带 sha256（FV 聚合索引补录后生效）
        if let Some(expected) = &expected_sha {
            if !sha.eq_ignore_ascii_case(expected) {
                let _ = fs::remove_file(&tmp);
                return Err(be_err("repo_mod_install_sha", ""));
            }
        }
        // 入包缓存（rename 优先，跨卷失败退化为 copy+删）
        if fs::rename(&tmp, &pkg).is_err() {
            if fs::copy(&tmp, &pkg).is_err() {
                let _ = fs::remove_file(&tmp);
                return Err(be_err("repo_mod_install_cache", ""));
            }
            let _ = fs::remove_file(&tmp);
        }
    }
    let mut check = "weak".to_string();
    if expected_sha.is_some() {
        check = "verified".to_string();
    }

    // 弱校验（安装前探查）：zip 内 mod.json 的 id/name 与索引条目比对，不符仅告警
    let info = mod_install::inspect(&pkg, &mods_dir)?;
    let mut warn: Vec<String> = Vec::new();
    if !info.mod_id.eq_ignore_ascii_case(&module) {
        warn.push(format!(
            "包内模组 id（{}）与索引条目（{module}）不一致",
            info.mod_id
        ));
    }
    let entry_name = entry.get("name").and_then(|v| v.as_str()).unwrap_or("");
    if !entry_name.is_empty() && !info.name.is_empty() && entry_name != info.name {
        warn.push(format!("包内名称（{}）与索引（{entry_name}）不一致", info.name));
    }

    // N12：实际经镜像下载且无 sha256 强校验 → 明确告警（**指名具体镜像域名**，§3.2.6 精化；包缓存命中不重复告警）
    if check == "weak" {
        if let Some(host) = via_label.strip_prefix("mirror:") {
            warn.push(format!(
                "内容经第三方镜像（{host}）下载且未做 sha256 校验，无法排除镜像篡改，请仅信任熟悉来源。"
            ));
        }
    }

    // 安装台账（mod 根内 .fv-install.json；同时作为 realm 私有副本的保护标记）
    let ledger = json!({
        "source": "repo",
        "repositoryUrl": repo_url,
        "downloadUrl": download,
        "sha256": sha,
        "check": check,
        "via": via_label, // 实际出口：direct / mirror:<域名> / proxy / cache（包缓存命中，台账可追溯）
        "installedAt": realms::now_date(),
        "modJsonUrl": entry.get("mod").and_then(|v| v.as_str()).unwrap_or(""),
        "changelog": entry.get("changelog").cloned().unwrap_or(Value::Null),
    });
    // 安装直接以包缓存文件为源；失败保留包（下次安装可复用/重验），成功后清理老版本+LRU
    let out = mod_install::install_with_ledger(&pkg, &mods_dir, true, Some(&ledger))?;
    prune_mod_packages(&pkg); // P6：自动清理（同模组旧版本包 + 容量超限 LRU）

    tracing::info!(
        target: "mods",
        "仓库安装模组 {}（{}，校验 {check}）→ {}",
        out.mod_id,
        if out.replaced { "更新" } else { "新装" },
        out.dest.display()
    );
    Ok(json!({
        "ok": true,
        "modId": out.mod_id,
        "version": info.version,
        "modName": info.name,
        "installedFiles": out.installed_files,
        "replaced": out.replaced,
        "sha256": sha,
        "check": check,
        "warn": warn,
        "dest": out.dest.to_string_lossy(),
    }))
}

/// 取消正在进行的模组下载（按 mod id）。
#[tauri::command]
pub fn cancel_mod_download(module: String) -> Result<(), String> {
    let mut v = MOD_CANCELLED
        .lock()
        .map_err(|_| be_err("realm_cancel_lock_corrupt", ""))?;
    if !v.contains(&module) {
        v.push(module);
    }
    Ok(())
}

/// 流式下载到文件：每 200ms 推 `mod-download-progress`；支持取消与体积护栏。
/// 网络出口（直连→镜像规则→代理回退）由 `get_send` 统一处理；返回**实际出口**供 N12 告警/台账。
fn download_mod_to_file(
    app: &AppHandle,
    mod_id: &str,
    url: &str,
    dest: &Path,
) -> Result<Egress, String> {
    let (resp, egress) = get_send(url).map_err(|e| be_err("repo_download_failed", &format!("{}", e)))?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(be_err("repo_download_server_returned", &format!("{}", status)));
    }
    let total = resp.content_length().unwrap_or(0);

    let mut reader = resp;
    let mut file = fs::File::create(dest).map_err(|e| be_err("repo_temp_file_create_failed", &format!("{}", e)))?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut received: u64 = 0;
    let mut last_emit = Instant::now();

    loop {
        let n = reader.read(&mut buf).map_err(|e| be_err("repo_download_interrupted", &format!("{}", e)))?;
        if n == 0 {
            break;
        }
        received += n as u64;
        if received > MOD_PACKAGE_MAX_BYTES {
            drop(file);
            let _ = fs::remove_file(dest);
            return Err(be_err("repo_mod_pack_too_large", &format!("{}", MOD_PACKAGE_MAX_BYTES / 1024 / 1024)));
        }
        file.write_all(&buf[..n])
            .map_err(|e| be_err("repo_temp_file_write_failed", &format!("{}", e)))?;

        if is_mod_cancelled(mod_id) {
            drop(file);
            let _ = fs::remove_file(dest);
            clear_mod_cancelled(mod_id);
            return Err(MOD_DOWNLOAD_CANCELLED.to_string());
        }

        if last_emit.elapsed() >= Duration::from_millis(200) {
            let _ = app.emit(
                "mod-download-progress",
                json!({ "modId": mod_id, "received": received, "total": total }),
            );
            last_emit = Instant::now();
        }
    }
    let _ = app.emit(
        "mod-download-progress",
        json!({ "modId": mod_id, "received": received, "total": total, "done": true }),
    );
    Ok(egress)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkg_file_name_sanitize() {
        // modId 非法字符替换为 '*'；URL 取 key 前 16 位（版本变化 URL 变 → 文件名变 → 老版本可识别）
        let n1 = pkg_file_name("vcmi-extras", "https://github.com/x/y/releases/download/1/a.zip");
        let n2 = pkg_file_name("vcmi-extras", "https://github.com/x/y/releases/download/2/a.zip");
        assert!(n1.starts_with("vcmi-extras__"));
        assert!(n1.ends_with(".zip"));
        assert_ne!(n1, n2);
        assert!(pkg_file_name("a b/c", "https://x/y").starts_with("a*b*c__"));
    }

    #[test]
    fn mod_json_version_parse() {
        // 官方语义：版本以 mod.json 为准（索引条目不带）
        assert_eq!(
            parse_mod_json_meta("x", br#"{"name":"x","version":"1.7.4"}"#).version,
            "1.7.4"
        );
        assert_eq!(parse_mod_json_meta("x", br#"{"name":"x"}"#).version, "");
        assert_eq!(parse_mod_json_meta("x", br#"{"version":""}"#).version, "");
        assert_eq!(parse_mod_json_meta("x", b"not json").version, "");
    }

    #[test]
    fn mod_json_meta_full_fields() {
        // P7：名称/描述/类型一并提取；modType 显式值原样透传（与扫描侧 classify_mod 同口径）
        let meta = parse_mod_json_meta(
            "vcmi-extras",
            br#"{"name":"VCMI extras","description":"Collection","modType":"Graphical","version":"3.9.4"}"#,
        );
        assert_eq!(meta.name, "VCMI extras");
        assert_eq!(meta.description, "Collection");
        assert_eq!(meta.mod_type, "Graphical");
        assert_eq!(meta.version, "3.9.4");
        // 无 modType 声明时走启发式：id 含 translation → Translation
        let heur = parse_mod_json_meta("belarusian-translation", br#"{"name":"Belarusian","description":""}"#);
        assert_eq!(heur.mod_type, "Translation");
    }

    #[test]
    fn normalize_keeps_index_meta_fields() {
        // 官方索引条目不带 name/description/modType → 规范化后为空串占位（enrich 回填）；
        // FV 聚合源 fv.* 优先；裸 entry.name 也能透传（兼容未来索引带名称）
        let bare = normalize_mod_entry("some-mod", &json!({"download": "u"}), "https://h/x.json");
        assert_eq!(bare["name"], "some-mod");
        assert_eq!(bare["description"], "");
        assert_eq!(bare["modType"], "");
        let fv = normalize_mod_entry(
            "a",
            &json!({"download": "u", "fv": {"name": "N", "description": "D", "modType": "T"}}),
            "https://h/x.json",
        );
        assert_eq!(fv["name"], "N");
        assert_eq!(fv["description"], "D");
        assert_eq!(fv["modType"], "T");
        let idx = normalize_mod_entry("b", &json!({"name": "Idx Name"}), "https://h/x.json");
        assert_eq!(idx["name"], "Idx Name");
    }

    #[test]
    fn archive_bare_index_coercion() {
        // 官方主索引：有 availableMods → 原样直通
        let main = json!({"availableMods": {"a": {"download": "u"}}});
        assert!(coerce_mod_index(main.clone())
            .get("availableMods")
            .is_some());
        // 1.4-archive：裸 {id:{mod,download}} 映射 → 包裹后可用
        let archive = json!({
            "horde-town": {"mod": "https://…/mod.json", "download": "https://…/x.zip"},
            "other": {"download": "https://…/y.zip"}
        });
        let wrapped = coerce_mod_index(archive);
        let mods = wrapped.get("availableMods").and_then(|v| v.as_object()).unwrap();
        assert_eq!(mods.len(), 2);
        // 非索引结构（realm 索引/空对象）不乱包
        let realms = json!({"realms": {"a": {}}});
        assert!(coerce_mod_index(realms.clone()).get("availableMods").is_none());
        assert!(coerce_mod_index(json!({})).get("availableMods").is_none());
    }

    #[test]
    fn mod_version_update_detection() {
        assert!(is_mod_update("1.7.3", "1.7.4"));
        assert!(is_mod_update("1.7", "1.7.1"));
        assert!(is_mod_update("1.9", "2.0"));
        // 本地领先 / 相等 / 空版本 → 不提示降级
        assert!(!is_mod_update("1.8", "1.7"));
        assert!(!is_mod_update("1.7.4", "1.7.4"));
        assert!(!is_mod_update("1.7.4", ""));
        assert!(!is_mod_update("", "1.0"));
        // 非数字段起按 0 处理（与前端 parseVcmiVersion 口径一致）
        assert_eq!(cmp_mod_versions("1.7.4-beta", "1.7.5"), std::cmp::Ordering::Less);
        assert_eq!(cmp_mod_versions("v2.0", "1.9.9"), std::cmp::Ordering::Greater);
    }
}
