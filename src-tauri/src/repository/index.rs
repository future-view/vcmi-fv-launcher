//! 远程仓库索引：仓库 URL 增删、索引拉取（带 ETag/`INDEX_TTL_SECS` 缓存）、条目构建与本地比对。

use super::*;
use crate::infra::berror::be_err;

use serde_json::{json, Value};
use std::fs;

use crate::realms;

pub(crate) fn validate_repository_url(url: &str) -> Result<(), String> {
    if !url.starts_with("https://") {
        return Err(be_err("repo_index_not_https", ""));
    }
    let path = url.split('?').next().unwrap_or("");
    if !path.to_lowercase().ends_with(".json") {
        return Err(be_err("repo_url_not_json", ""));
    }
    Ok(())
}

pub(crate) struct IndexCache {
    pub(crate) etag: Option<String>,
    pub(crate) fetched_at: u64,
    pub(crate) index: Value,
}

pub(crate) fn load_cache(url: &str) -> Option<IndexCache> {
    let p = index_cache_dir()?.join(format!("{}.json", url_key(url)));
    let text = fs::read_to_string(&p).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    if v.get("url").and_then(|x| x.as_str()) != Some(url) {
        return None;
    }
    Some(IndexCache {
        etag: v
            .get("etag")
            .and_then(|x| x.as_str())
            .map(|s| s.to_string()),
        fetched_at: v.get("fetchedAt").and_then(|x| x.as_u64()).unwrap_or(0),
        index: v.get("index").cloned().unwrap_or(Value::Null),
    })
}

pub(crate) fn save_cache(url: &str, etag: Option<&str>, index: &Value) -> Result<(), String> {
    let dir = index_cache_dir().ok_or_else(|| be_err("common_data_dir_unknown", ""))?;
    fs::create_dir_all(&dir)
        .map_err(|e| be_err("repo_index_cache_dir_create_failed", &format!("{}", e)))?;
    let v = json!({
        "url": url,
        "etag": etag,
        "fetchedAt": crate::infra::time::now_secs(),
        "index": index,
    });
    let text = serde_json::to_string_pretty(&v)
        .map_err(|e| be_err("repo_index_cache_serialize_failed", &format!("{}", e)))?;
    fs::write(dir.join(format!("{}.json", url_key(url))), text)
        .map_err(|e| be_err("repo_index_cache_write_failed", &format!("{}", e)))
}

/// 本地已安装领域的比对信息（版本 + 内容指纹）。
/// `sha256` 来自安装时写入的 `origin.sha256`（远程安装必有；本地自建 / 旧数据为空）。
pub(crate) struct LocalRealm {
    pub(crate) version: String,
    pub(crate) sha256: String,
}

/// 判定单条的 `state`（判定矩阵，防降级 + 内容感知）：
/// - 双方 sha 已知且相同 → `installed`（内容一致，无论版本号怎么写）
/// - 索引缺 version（手工条目）→ `installed`（无基准不提示，宁缺勿错）
/// - 本地版本 < 索引版本 → `updatable`
/// - 本地版本 = 索引版本：双方 sha 已知且不同 → `updatable`（发布事故兜底：
///   同版换包，重装可取到新内容）；sha 不可比 → `installed`
/// - 本地版本 > 索引版本 → `installed`（本地领先，绝不提示降级）
fn realm_update_state(local: &LocalRealm, entry_version: &str, entry_sha: &str) -> &'static str {
    let ls = local.sha256.trim();
    let es = entry_sha.trim();
    let sha_known = !ls.is_empty() && !es.is_empty();
    if sha_known && ls.eq_ignore_ascii_case(es) {
        return "installed";
    }
    let ev = entry_version.trim();
    if ev.is_empty() {
        return "installed";
    }
    match crate::infra::version::compare(local.version.trim(), ev) {
        std::cmp::Ordering::Less => "updatable",
        std::cmp::Ordering::Equal => {
            if sha_known {
                "updatable"
            } else {
                "installed"
            }
        }
        std::cmp::Ordering::Greater => "installed",
    }
}

/// 把单个索引整理成前端可用列表：补齐 id / tags，按本地安装情况打 `state`，
/// 并给每条打上来源 `repositoryUrl`（聚合多源时用于定位下载 / 校验信息）。
pub(crate) fn build_items(
    local: &HashMap<String, LocalRealm>,
    url: &str,
    index: &Value,
) -> Vec<Value> {
    let mut items: Vec<Value> = Vec::new();
    if let Some(obj) = index.get("realms").and_then(|v| v.as_object()) {
        for (id, entry) in obj {
            let mut e = entry.clone();
            if let Some(o) = e.as_object_mut() {
                o.insert("id".to_string(), json!(id));
                if !o.contains_key("tags") {
                    o.insert("tags".to_string(), json!([]));
                }
                o.insert("repositoryUrl".to_string(), json!(url));
                let version = o
                    .get("version")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let entry_sha = o
                    .get("sha256")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let (state, installed) = match local.get(id) {
                    None => ("available", Value::Null),
                    Some(l) => (
                        realm_update_state(l, &version, &entry_sha),
                        json!(&l.version),
                    ),
                };
                o.insert("state".to_string(), json!(state));
                o.insert("installedVersion".to_string(), installed);
            }
            items.push(e);
        }
    }
    items.sort_by_key(|v| {
        v.get("id")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string()
    });
    items
}

/// 当前仓库源列表（不触发网络请求）。
#[tauri::command]
pub fn get_realm_repositories() -> Value {
    let urls = read_repository_urls();
    json!({ "urls": urls })
}

/// 新增一个仓库源地址（去重、校验 https + .json）。
#[tauri::command]
pub fn add_realm_repository(url: String) -> Result<Value, String> {
    let trimmed = url.trim().to_string();
    if trimmed.is_empty() {
        return Err(be_err("repo_url_empty", ""));
    }
    validate_repository_url(&trimmed)?;
    let mut cfg =
        VcmiPaths::read_launcher_config().ok_or_else(|| be_err("settings_not_init", ""))?;
    if !cfg
        .realms
        .repository_urls
        .iter()
        .any(|u| u.trim() == trimmed)
    {
        cfg.realms.repository_urls.push(trimmed);
    }
    VcmiPaths::write_launcher_config(&cfg)?;
    Ok(json!({ "urls": cfg.realms.repository_urls }))
}

/// 移除一个仓库源地址（按 trim 后相等匹配）。
#[tauri::command]
pub fn remove_realm_repository(url: String) -> Result<Value, String> {
    let trimmed = url.trim().to_string();
    let mut cfg =
        VcmiPaths::read_launcher_config().ok_or_else(|| be_err("settings_not_init", ""))?;
    cfg.realms.repository_urls.retain(|u| u.trim() != trimmed);
    VcmiPaths::write_launcher_config(&cfg)?;
    Ok(json!({ "urls": cfg.realms.repository_urls }))
}

/// 测试某个索引地址是否可用（不写入配置），返回解析到的领域数量。
/// P8：同步命令跑在主线程，http_get 会冻结窗口——移交阻塞线程池。
#[tauri::command]
pub async fn test_realm_repository(url: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || test_realm_repository_blocking(url))
        .await
        .map_err(|e| be_err("repo_source_test_task_failed", &format!("{}", e)))?
}

fn test_realm_repository_blocking(url: String) -> Result<Value, String> {
    let target = url.trim();
    if target.is_empty() {
        return Err(be_err("repo_test_empty", ""));
    }
    validate_repository_url(target)?;
    let final_url = target.to_string();
    let started = Instant::now();
    let (_, _, bytes) = http_get(&final_url, None)?;
    let index: Value =
        serde_json::from_slice(&bytes).map_err(|_| be_err("repo_index_parse_invalid", ""))?;
    if index.get("realms").and_then(|v| v.as_object()).is_none() {
        return Err(be_err("repo_index_no_realms", ""));
    }
    let count = index
        .get("realms")
        .and_then(|v| v.as_object())
        .map(|o| o.len())
        .unwrap_or(0);
    Ok(json!({
        "ok": true,
        "url": final_url,
        "realmCount": count,
        "elapsedMs": started.elapsed().as_millis() as u64,
    }))
}

/// 拉取远程领域列表。`force = true` 时忽略 TTL 强制拉取（刷新按钮 / 改源后）。
///
/// 失败且有缓存时**回退缓存**并在 `error` 字段带上原因，前端据此提示「离线」。
#[tauri::command]
pub async fn fetch_realm_index(force: bool) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || fetch_realm_index_blocking(force))
        .await
        .map_err(|e| be_err("repo_realm_index_task_failed", &format!("{}", e)))?
}

fn fetch_realm_index_blocking(force: bool) -> Result<Value, String> {
    let urls = read_repository_urls();
    // 本地已安装领域版本 + 内容指纹（一次算好，跨源复用）
    let mut local: HashMap<String, LocalRealm> = HashMap::new();
    for r in realms::realm_versions() {
        let Some(id) = r.get("id").and_then(|v| v.as_str()) else {
            continue;
        };
        let ver = r.get("version").and_then(|v| v.as_str()).unwrap_or("");
        let sha = r.get("sha256").and_then(|v| v.as_str()).unwrap_or("");
        local.insert(
            id.to_string(),
            LocalRealm {
                version: ver.to_string(),
                sha256: sha.to_string(),
            },
        );
    }

    let mut merged: std::collections::BTreeMap<String, Value> = std::collections::BTreeMap::new();
    let mut sources: Vec<Value> = Vec::new();
    let mut any_ok = false;
    let mut last_error: Option<String> = None;

    for url in &urls {
        let cached = load_cache(url);
        // P6 缓存加强（与 mod 侧同构）：验证时刻 = max(fetchedAt, 文件 mtime)；304 只 touch 不重写
        let cache_file = index_cache_dir().map(|d| d.join(format!("{}.json", url_key(url))));
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

        // TTL 内且已有缓存：直接用缓存，不触网。
        let (index, fetched_at, err) = if !force && age < INDEX_TTL_SECS {
            match cached {
                Some(c) => (c.index, cache_fetched.unwrap_or(c.fetched_at), None),
                None => (
                    Value::Null,
                    0u64,
                    Some("领域索引为空，请刷新重试".to_string()),
                ),
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
                    None => (
                        Value::Null,
                        0u64,
                        Some("领域索引为空，请刷新重试".to_string()),
                    ),
                },
                Ok((_, new_etag, bytes)) => {
                    // Gitee 的 Content-Type 是 text/plain，只能靠"解析成功"判定
                    let index: Value = serde_json::from_slice(&bytes)
                        .map_err(|_| be_err("repo_realm_index_parse_invalid", ""))?;
                    if index.get("realms").and_then(|v| v.as_object()).is_none() {
                        return Err(be_err("repo_index_missing_realms", ""));
                    }
                    let _ = save_cache(url, new_etag.as_deref(), &index);
                    (index, crate::infra::time::now_secs(), None)
                }
                Err(e) => match cached {
                    Some(c) => (c.index, c.fetched_at, Some(e)),
                    None => (Value::Null, 0u64, Some(e)),
                },
            }
        };

        let realm_count = index
            .get("realms")
            .and_then(|v| v.as_object())
            .map(|o| o.len())
            .unwrap_or(0);
        if err.is_none() {
            any_ok = true;
        } else if last_error.is_none() {
            last_error = err.clone();
        }
        sources.push(json!({
            "url": url,
            "ok": err.is_none(),
            "error": err,
            "fetchedAt": if err.is_none() {
                Some(fetched_at)
            } else {
                cache_fetched
            },
            "realmCount": realm_count,
        }));

        // 多源 realm 按 id 合并：靠前的仓库优先，靠后源的重复 id 被忽略。
        for it in build_items(&local, url, &index) {
            if let Some(id) = it.get("id").and_then(|v| v.as_str()) {
                merged.entry(id.to_string()).or_insert(it);
            }
        }
    }

    let mut realms: Vec<Value> = merged.into_values().collect();
    realms.sort_by_key(|v| {
        v.get("id")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string()
    });

    let error = if any_ok { None } else { last_error };
    Ok(json!({
        "realms": realms,
        "sources": sources,
        "error": error,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(version: &str, sha256: &str) -> LocalRealm {
        LocalRealm {
            version: version.to_string(),
            sha256: sha256.to_string(),
        }
    }

    #[test]
    fn state_matrix_covers_the_five_cases() {
        // 1) 内容一致（sha 相同）→ installed，即使版本写法不同
        assert_eq!(
            realm_update_state(&local("1.0.0", "aa"), "1.0.1", "AA"),
            "installed"
        );
        // 2) 索引缺 version → installed（无基准不提示）
        assert_eq!(realm_update_state(&local("1.0.0", ""), "", ""), "installed");
        // 3) 本地版本低于索引 → updatable
        assert_eq!(
            realm_update_state(&local("1.0.0", "aa"), "2.0.0", "bb"),
            "updatable"
        );
        // 4) 同版本但内容不同 → updatable（发布事故兜底：同版换包）
        assert_eq!(
            realm_update_state(&local("1.2.0", "aa"), "1.2.0", "bb"),
            "updatable"
        );
        // 5) 本地版本高于索引 → installed（本地领先，不提示降级）
        assert_eq!(
            realm_update_state(&local("2.0.0", "aa"), "1.0.0", "bb"),
            "installed"
        );
    }

    #[test]
    fn state_matrix_degrades_gracefully_without_sha() {
        // 同版本、sha 不可比（任一侧缺）→ installed（退回纯版本比较）
        assert_eq!(
            realm_update_state(&local("1.0.0", ""), "1.0.0", ""),
            "installed"
        );
        assert_eq!(
            realm_update_state(&local("1.0.0", ""), "1.0.0", "bb"),
            "installed"
        );
        assert_eq!(
            realm_update_state(&local("1.0.0", "aa"), "1.0.0", ""),
            "installed"
        );
        // 无 sha 也不影响版本高低判定
        assert_eq!(
            realm_update_state(&local("1.0.0", ""), "1.1.0", ""),
            "updatable"
        );
        // 后缀不参与：1.0.0-rc1 vs 1.0.0 视为同版
        assert_eq!(
            realm_update_state(&local("1.0.0-rc1", ""), "1.0.0", ""),
            "installed"
        );
    }
}
