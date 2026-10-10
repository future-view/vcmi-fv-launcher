//! 远程领域包下载与封面缓存。
//!
//! 安装流程 = 下载到临时目录（**强制 sha256 校验**，见 `PACKAGE_MAX_BYTES` 上限）→
//! 复用 `realms::import_realm` 落库 → 回填 `installedAt`。支持按 realmId 取消。

use crate::infra::berror::be_err;
use super::*;
use super::index::load_cache;

use serde_json::{json, Value};
use std::fs;
use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;
use tauri::{AppHandle, Emitter, State};

use crate::realms;
use crate::state::AppPaths;
use crate::vcmi::paths::VcmiPaths;

pub(crate) fn is_cancelled(realm_id: &str) -> bool {
    CANCELLED
        .lock()
        .map(|v| v.iter().any(|x| x == realm_id))
        .unwrap_or(false)
}

pub(crate) fn clear_cancelled(realm_id: &str) {
    if let Ok(mut v) = CANCELLED.lock() {
        v.retain(|x| x != realm_id);
    }
}

/// 下载并安装远程领域：下载 → **强制 sha256 校验** → 复用本地导入流程 → 写入来源标记。
///
/// 进度通过 `realm-download-progress` 事件推送 `{realmId, received, total, done?}`。
/// P8：同步命令跑在主线程，下载+解包会冻结窗口——主体移交阻塞线程池。
#[tauri::command]
pub async fn install_remote_realm(
    app: AppHandle,
    state: State<'_, AppPaths>,
    realm_id: String,
    conflict: String,
) -> Result<Value, String> {
    let paths = state.0.lock().expect("paths lock poisoned").clone();
    tauri::async_runtime::spawn_blocking(move || {
        install_remote_realm_blocking(app, paths, realm_id, conflict)
    })
    .await
    .map_err(|e| be_err("repo_realm_install_task_failed", &format!("{}", e)))?
}

fn install_remote_realm_blocking(
    app: AppHandle,
    paths: VcmiPaths,
    realm_id: String,
    conflict: String,
) -> Result<Value, String> {
    // 跨所有仓库源定位该 realm 的索引条目（任一缓存命中即可）。
    let located = (|| -> Option<(String, Value)> {
        for u in read_repository_urls() {
            if let Some(c) = load_cache(&u) {
                if let Some(e) = c
                    .index
                    .get("realms")
                    .and_then(|v| v.get(&realm_id))
                    .cloned()
                {
                    return Some((u, e));
                }
            }
        }
        None
    })();
    let (url, entry) = located
        .ok_or_else(|| be_err("repo_realm_index_not_loaded", ""))?;

    let download = entry
        .get("download")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| be_err("repo_index_entry_no_download", ""))?;
    // 远程来源必须可校验：没有 sha256 直接拒绝（本地导入允许为空，这里不允许）
    let expected = entry
        .get("sha256")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| be_err("repo_index_entry_no_sha", ""))?;
    // 一期不实现分片：契约保留，遇到则提示升级（真需要时无需迁移数据）
    if entry
        .get("parts")
        .and_then(|v| v.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false)
    {
        return Err(be_err("repo_realm_sharded", ""));
    }
    let version = entry
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let dir = downloads_dir().ok_or_else(|| be_err("common_download_dir_unknown", ""))?;
    fs::create_dir_all(&dir).map_err(|e| be_err("repo_download_dir_create_failed", &format!("{}", e)))?;
    let tmp = dir.join(format!("{}-{}.vcmirealm", realm_id, version));

    download_to_file(&app, &realm_id, download, &tmp)?;

    let actual = crate::infra::hash::sha256_file(&tmp)?;
    if !actual.eq_ignore_ascii_case(expected) {
        let _ = fs::remove_file(&tmp);
        return Err(be_err("repo_realm_install_sha", ""));
    }

    // 复用本地导入：解包 → 建骨架 → 写 __realm__ 预设 → 校验借用模组
    let res = realms::import_realm_impl(paths, tmp.to_string_lossy().to_string(), conflict)?;

    // 写入来源标记，供后续「有更新」判定
    if let Some(rid) = res.get("realmId").and_then(|v| v.as_str()) {
        if let Some(mut meta) = realms::read_realm_meta(rid) {
            if let Some(o) = meta.as_object_mut() {
                o.insert(
                    "origin".to_string(),
                    json!({
                        "repositoryUrl": url,
                        "version": version,
                        "sha256": expected,
                        "installedAt": realms::now_date(),
                    }),
                );
            }
            let _ = realms::write_realm_meta(rid, &meta);
        }
    }
    let _ = fs::remove_file(&tmp);
    Ok(res)
}

/// 取消正在进行的领域下载（按领域 id）。
#[tauri::command]
pub fn cancel_realm_download(realm_id: String) -> Result<(), String> {
    let mut v = CANCELLED
        .lock()
        .map_err(|_| be_err("realm_cancel_lock_corrupt", ""))?;
    if !v.contains(&realm_id) {
        v.push(realm_id);
    }
    Ok(())
}

/// 下载并缓存远程领域的封面图，返回 `领域 id -> 本地图片绝对路径`。
///
/// 封面一律由**后端下载**：Gitee raw 对带 Referer 的请求返回 403，
/// 前端 `<img>` 直连会被拦，后端请求不带 Referer 则正常。
/// 封面批量下载（逐 realm http_get）——P8：同样移出主线程。
#[tauri::command]
pub async fn ensure_realm_covers() -> Value {
    tauri::async_runtime::spawn_blocking(ensure_realm_covers_blocking)
        .await
        .unwrap_or_else(|_| json!({}))
}

fn ensure_realm_covers_blocking() -> Value {
    let dir = match covers_dir() {
        Some(d) => d,
        None => return json!({}),
    };
    if fs::create_dir_all(&dir).is_err() {
        return json!({});
    }

    // 遍历所有仓库源，合并 id -> 本地封面路径（同 id 取首个命中的源）。
    let mut out: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for url in read_repository_urls() {
        let cache = match load_cache(&url) {
            Some(c) => c,
            None => continue,
        };
        if let Some(obj) = cache.index.get("realms").and_then(|v| v.as_object()) {
            for (id, entry) in obj {
                if out.contains_key(id) {
                    continue;
                }
                let Some(cover) = entry.get("cover").and_then(|v| v.as_str()) else {
                    continue;
                };
                if cover.trim().is_empty() {
                    continue;
                }
                let version = entry.get("version").and_then(|v| v.as_str()).unwrap_or("");
                let path = dir.join(format!("{}-{}.{}", id, version, cover_ext(cover)));
                if !path.is_file() {
                    if let Ok((_, _, bytes)) = http_get(cover, None) {
                        // 排除 HTML 错误页 + 体积护栏
                        let ok = !bytes.starts_with(b"<")
                            && bytes.len() > 512
                            && bytes.len() as u64 <= COVER_MAX_BYTES;
                        if ok {
                            let _ = fs::write(&path, &bytes);
                        }
                    }
                }
                if path.is_file() {
                    out.insert(id.clone(), path.to_string_lossy().to_string());
                }
            }
        }
    }
    Value::Object(out.into_iter().map(|(k, v)| (k, json!(v))).collect())
}

/// 流式下载到文件，每 200ms 推一次进度；支持取消与体积护栏。
/// 网络出口（直连→镜像规则→代理回退）由 `get_send` 统一处理（领域包多在 Gitee，通常恒直连）。
pub(crate) fn download_to_file(
    app: &AppHandle,
    realm_id: &str,
    url: &str,
    dest: &Path,
) -> Result<(), String> {
    let (resp, _egress) = get_send(url).map_err(|e| be_err("repo_download_failed", &format!("{}", e)))?;
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
        if received > PACKAGE_MAX_BYTES {
            drop(file);
            let _ = fs::remove_file(dest);
            return Err(be_err("repo_realm_pack_too_large", &format!("{}", PACKAGE_MAX_BYTES / 1024 / 1024)));
        }
        file.write_all(&buf[..n])
            .map_err(|e| be_err("repo_temp_file_write_failed", &format!("{}", e)))?;

        if is_cancelled(realm_id) {
            drop(file);
            let _ = fs::remove_file(dest);
            clear_cancelled(realm_id);
            return Err(REALM_DOWNLOAD_CANCELLED.to_string());
        }

        if last_emit.elapsed() >= Duration::from_millis(200) {
            let _ = app.emit(
                "realm-download-progress",
                json!({ "realmId": realm_id, "received": received, "total": total }),
            );
            last_emit = Instant::now();
        }
    }
    let _ = app.emit(
        "realm-download-progress",
        json!({ "realmId": realm_id, "received": received, "total": total, "done": true }),
    );
    Ok(())
}

pub(crate) fn cover_ext(url: &str) -> String {
    let base = url.to_lowercase();
    let base = base.split('?').next().unwrap_or("");
    for e in ["png", "jpg", "jpeg", "webp", "gif"] {
        if base.ends_with(&format!(".{e}")) {
            return e.to_string();
        }
    }
    "png".to_string()
}
