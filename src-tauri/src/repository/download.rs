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

    // 远程来源必须可校验：没有 sha256 直接拒绝（本地导入允许为空，这里不允许）。
    // ⚠️ 条目级 sha256 恒为**整包**哈希——分片条目也不例外（拼回后才比对，语义不变）。
    let expected = entry
        .get("sha256")
        .and_then(|v| v.as_str())
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| be_err("repo_index_entry_no_sha", ""))?;
    // 分片判定必须**先于** `download`：分片条目不带 download（两者互斥，见索引规范）
    let parts = parse_parts(entry.get("parts"))?;
    let version = entry
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let dir = downloads_dir().ok_or_else(|| be_err("common_download_dir_unknown", ""))?;
    fs::create_dir_all(&dir).map_err(|e| be_err("repo_download_dir_create_failed", &format!("{}", e)))?;
    let tmp = dir.join(format!("{}-{}.vcmirealm", realm_id, version));

    if parts.is_empty() {
        let download = entry
            .get("download")
            .and_then(|v| v.as_str())
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| be_err("repo_index_entry_no_download", ""))?;
        download_to_file(&app, &realm_id, download, &tmp, None)?;
    } else {
        // 分片：逐片下载 + 逐片校验 → 拼回整包 → 再走统一的整包校验与导入。
        // 分片目录用完即清（拼回成败都不留），磁盘不留多余副本。
        let parts_dir = dir.join(format!("{}-{}.parts", realm_id, version));
        download_parts(&app, &realm_id, &parts, &parts_dir)?;
        // 拼回失败要如实报错（拼不回就没有「整包」可校验），但分片目录照样清场
        let assembled = assemble_parts(&parts, &parts_dir, &tmp);
        let _ = fs::remove_dir_all(&parts_dir);
        assembled?;
    }

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

/* ----------------------------- 分片下载（Sharding） ----------------------------- */

/// 索引条目 `parts[]` 的单条（**数组序即拼接顺序**）。
pub(crate) struct PartSpec {
    pub(crate) url: String,
    /// 分片字节数：只用于进度分母与体积 sanity；缺失按 0 计（不影响正确性）
    pub(crate) size: u64,
    pub(crate) sha256: String,
}

/// 解析索引条目的 `parts`：缺省 / 空 = 单包分发（返回空列表）；有值但结构不合法 → 拒装。
///
/// 校验从严（`url` 与 `sha256` 缺一不可）：分片是「逐片校验后拼回」的链条，
/// 任何一片缺校验信息，最终那次整包校验就只能事后发现问题——白下载几百 MB。
fn parse_parts(v: Option<&Value>) -> Result<Vec<PartSpec>, String> {
    let arr = match v.and_then(|x| x.as_array()) {
        Some(a) if !a.is_empty() => a,
        _ => return Ok(Vec::new()),
    };
    let mut out: Vec<PartSpec> = Vec::with_capacity(arr.len());
    for (i, p) in arr.iter().enumerate() {
        let url = p
            .get("url")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| be_err("repo_realm_parts_invalid", &format!("{}", i + 1)))?;
        let sha256 = p
            .get("sha256")
            .and_then(|x| x.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| be_err("repo_realm_parts_invalid", &format!("{}", i + 1)))?;
        out.push(PartSpec {
            url: url.to_string(),
            size: p.get("size").and_then(|x| x.as_u64()).unwrap_or(0),
            sha256: sha256.to_string(),
        });
    }
    Ok(out)
}

/// 本地分片文件名：`part001`——**按数组序命名**，不取远端文件名（避免路径穿越与命名冲突）。
fn part_file_name(index: usize) -> String {
    format!("part{:03}", index + 1)
}

/// 分片进度上下文：把「本片进度」换算回「整包进度」再推事件。
pub(crate) struct ShardProgress {
    /// 之前各片已收字节数
    pub(crate) received_before: u64,
    /// 全部片字节数之和（进度分母）
    pub(crate) total_all: u64,
    pub(crate) index: usize,
    pub(crate) count: usize,
}

/// 逐片下载：每片落盘即验 sha256（快速失败），进度按**整包**聚合推送。
///
/// 与单包流程只差两处——分片目录与聚合进度；下载、取消、体积护栏全部复用 `download_to_file`。
/// 任一片失败 / 被取消 → 整个分片目录清掉（不留半成品，与「取消不留半成品」同口径）。
fn download_parts(
    app: &AppHandle,
    realm_id: &str,
    parts: &[PartSpec],
    dir: &Path,
) -> Result<(), String> {
    let total_all: u64 = parts.iter().map(|p| p.size).sum();
    // 总量 sanity 上限：分片让「整包 > 100 MB」成为合法形态，但仍要挡住索引误填的天文数字
    if total_all > SHARDED_TOTAL_MAX_BYTES {
        return Err(be_err(
            "repo_realm_pack_too_large",
            &format!("{}", SHARDED_TOTAL_MAX_BYTES / 1024 / 1024),
        ));
    }
    if dir.is_dir() {
        let _ = fs::remove_dir_all(dir);
    }
    fs::create_dir_all(dir).map_err(|e| be_err("repo_download_dir_create_failed", &format!("{}", e)))?;
    let mut received_before = 0u64;
    for (i, p) in parts.iter().enumerate() {
        if is_cancelled(realm_id) {
            let _ = fs::remove_dir_all(dir);
            clear_cancelled(realm_id);
            return Err(REALM_DOWNLOAD_CANCELLED.to_string());
        }
        let dest = dir.join(part_file_name(i));
        let shard = ShardProgress {
            received_before,
            total_all,
            index: i,
            count: parts.len(),
        };
        if let Err(e) = download_to_file(app, realm_id, &p.url, &dest, Some(&shard)) {
            let _ = fs::remove_dir_all(dir);
            return Err(e);
        }
        let actual = crate::infra::hash::sha256_file(&dest)?;
        if !actual.eq_ignore_ascii_case(&p.sha256) {
            let _ = fs::remove_dir_all(dir);
            return Err(be_err("repo_realm_part_sha", &format!("{}", i + 1)));
        }
        // 用**实际落盘字节**累加：索引里的 size 只作分母，写错也不该让进度漂移失真
        received_before += fs::metadata(&dest).map(|m| m.len()).unwrap_or(0);
    }
    Ok(())
}

/// 按 `parts` 数组序拼回整包（`download_parts` 的逆操作）。
/// 拼回结果必须与分片前的整包逐字节一致——随后由整包 sha256 校验兜底。
fn assemble_parts(parts: &[PartSpec], dir: &Path, dest: &Path) -> Result<(), String> {
    let mut out =
        fs::File::create(dest).map_err(|e| be_err("repo_temp_file_create_failed", &format!("{}", e)))?;
    for i in 0..parts.len() {
        let src = dir.join(part_file_name(i));
        let mut f = fs::File::open(&src).map_err(|e| be_err("infra_file_read_failed", &format!("{}", e)))?;
        std::io::copy(&mut f, &mut out)
            .map_err(|e| be_err("repo_temp_file_write_failed", &format!("{}", e)))?;
    }
    Ok(())
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
///
/// `shard` 非空时表示「这是分片之一」：进度按**整包**聚合推送（否则进度条每片归零重来），
/// 事件附加 `partIndex` / `partCount`；体积护栏仍按**单片**计（分片本身 ≤ 45 MiB）。
pub(crate) fn download_to_file(
    app: &AppHandle,
    realm_id: &str,
    url: &str,
    dest: &Path,
    shard: Option<&ShardProgress>,
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
                progress_payload(realm_id, shard, received, total, false),
            );
            last_emit = Instant::now();
        }
    }
    let _ = app.emit(
        "realm-download-progress",
        progress_payload(realm_id, shard, received, total, true),
    );
    Ok(())
}

/// 下载进度事件体。分片场景把「本片进度」加上**之前各片**再作为整包进度，
/// 并附加 `partIndex` / `partCount`（附加字段，旧监听忽略即可）。
fn progress_payload(
    realm_id: &str,
    shard: Option<&ShardProgress>,
    received: u64,
    total: u64,
    done: bool,
) -> Value {
    match shard {
        Some(s) => json!({
            "realmId": realm_id,
            "received": s.received_before + received,
            "total": s.total_all,
            "partIndex": s.index + 1,
            "partCount": s.count,
            // 只有最后一片的收尾帧才报「整包下载完成」
            "done": done && s.index + 1 == s.count,
        }),
        None => json!({ "realmId": realm_id, "received": received, "total": total, "done": done }),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn part(url: &str, size: u64, sha: &str) -> Value {
        json!({ "url": url, "size": size, "sha256": sha })
    }

    #[test]
    fn parts_parse_accepts_absent_and_empty_as_single_package() {
        // 缺省 / 空数组 = 单包分发（旧索引与绝大多数领域走这条）
        assert!(parse_parts(None).unwrap().is_empty());
        assert!(parse_parts(Some(&json!([]))).unwrap().is_empty());
        assert!(parse_parts(Some(&json!("not-an-array"))).unwrap().is_empty());
        // 分片条目：数组序即拼接序
        let parsed = parse_parts(Some(&json!([
            part("https://x/a.part001", 10, "aa"),
            part("https://x/a.part002", 20, "bb"),
        ])))
        .unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].sha256, "aa");
        assert_eq!(parsed[1].size, 20);
    }

    #[test]
    fn parts_parse_rejects_entries_without_url_or_sha() {
        // 缺 url：拼不回来也定位不到
        assert!(parse_parts(Some(&json!([{ "size": 1, "sha256": "aa" }]))).is_err());
        // 缺 sha256：整包校验会变成「下完几百 MB 才发现不对」
        assert!(parse_parts(Some(&json!([{ "url": "https://x/1", "size": 1 }]))).is_err());
        // 空串同缺
        assert!(parse_parts(Some(&json!([part("", 1, "aa")]))).is_err());
        assert!(parse_parts(Some(&json!([part("https://x/1", 1, " ")]))).is_err());
    }

    #[test]
    fn sharded_progress_is_aggregated_and_done_only_on_last_part() {
        let shard = ShardProgress {
            received_before: 100,
            total_all: 300,
            index: 1,
            count: 3,
        };
        let p = progress_payload("demo", Some(&shard), 50, 100, false);
        assert_eq!(p["received"], json!(150), "本片进度要加上之前各片");
        assert_eq!(p["total"], json!(300), "分母恒为整包");
        assert_eq!(p["partIndex"], json!(2));
        assert_eq!(p["partCount"], json!(3));
        assert_eq!(p["done"], json!(false), "非末片不得报完成");
        // 末片收尾帧才报完成
        let last = ShardProgress { index: 2, ..shard };
        assert_eq!(progress_payload("demo", Some(&last), 100, 100, true)["done"], json!(true));
        // 单包：字段与旧契约一致（无 partIndex / partCount）
        let single = progress_payload("demo", None, 7, 9, true);
        assert_eq!(single, json!({ "realmId": "demo", "received": 7, "total": 9, "done": true }));
    }

    /// 拼回必须与分片前逐字节一致：整包 sha256 校验依赖这一点。
    #[test]
    fn assemble_parts_reproduces_the_original_bytes() {
        use std::fs;
        let dir = std::env::temp_dir().join("vcmi-fv-assemble-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let specs = vec![
            PartSpec { url: String::new(), size: 3, sha256: String::new() },
            PartSpec { url: String::new(), size: 2, sha256: String::new() },
        ];
        fs::write(dir.join(part_file_name(0)), b"abc").unwrap();
        fs::write(dir.join(part_file_name(1)), b"de").unwrap();
        let out = dir.join("joined.vcmirealm");
        assemble_parts(&specs, &dir, &out).unwrap();
        assert_eq!(fs::read(&out).unwrap(), b"abcde");
        let _ = fs::remove_dir_all(&dir);
    }
}
