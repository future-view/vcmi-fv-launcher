//! 更新包下载：流式写盘 + sha256 强校验 + 进度回调 + 取消哨兵。
//!
//! 照搬 `repository/download.rs` 的既有模式（sync 阻塞下载 + 周期回调），
//! 事件 emit 由 `commands/updater.rs` 在回调里完成（本层无 tauri 类型）。

use crate::infra::berror::be_err;
use std::collections::HashSet;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::infra::hash::sha256_file;
use crate::infra::http::get_send;
use crate::vcmi::paths::VcmiPaths;

use super::manifest::ManifestEntry;
use super::{
    downloads_dir, is_cancelled, set_cancelled, set_phase, target, UpdatePhase,
    UPDATE_DOWNLOAD_CANCELLED, UPDATE_MAX_BYTES,
};

/// zip 更新包内的主程序相对路径（**按平台**；与 `release.mjs::TARGETS[].bin` 镜像，
/// 见 `platform::binary_name`）。未知三元组 → Linux 通用名。
pub(crate) fn staged_binary_name() -> &'static str {
    super::platform::binary_name(super::platform::target_triple().unwrap_or(""))
}

/// 暂存路径（`downloads/` 内，与 exe 同盘 → `move` 原子）。
/// zip 形态落 `.zip`；exe 形态保持 `.exe.update`（旧约定，同版本可复用免重下）。
pub(crate) fn staged_path(entry: &ManifestEntry) -> Result<PathBuf, String> {
    let name = if entry.is_zip() {
        format!("VCMI-FV-Launcher_{}.zip", entry.version)
    } else {
        format!("VCMI-FV-Launcher_{}.exe.update", entry.version)
    };
    Ok(downloads_dir()?.join(name))
}

/// zip 更新包的解压目录（apply 阶段从这里取文件；与 exe 同盘 → `move` 原子）
pub(crate) fn staged_dir(version: &str) -> Result<PathBuf, String> {
    Ok(downloads_dir()?.join(format!("staged_{version}")))
}

/// zip 更新包**允许写入的相对路径**（白名单）。
///
/// ⚠️ 绝不触碰用户数据：`realms` / `cache` / `downloads` / `mods` / `launcher.json`
/// 一律拒绝 —— 否则"解压覆盖"会清空用户的领域与缓存。
fn allowed_rel(rel: &str) -> bool {
    rel == staged_binary_name()
        // 组件**只认一个位置**（`.vcmi-fv-launcher/runtime/`）——
        // 与打包预置、远程安装、便携内置三者统一（见 `runtime::dir_of`）
        || rel.starts_with(".vcmi-fv-launcher/runtime/")
        // 模组缓存（可重建；用于"带少量关键 mod 的缓存提速"）
        || rel.starts_with(".vcmi-fv-launcher/mods/")
        // 领域（新增或替换发行方领域）—— **是否覆盖完全由开发者决定**（拍板口径）。
        // ⚠️ 覆盖时只能逐文件写，**绝不能删目录**：领域内可能有借用联接（junction），
        //    `remove_dir_all` 会连带删掉联接指向的真实文件（见 rules 04）。
        || rel.starts_with(".vcmi-fv-launcher/realms/")
        // 配置补丁（**一次性合并**，见 `updater/config_patch.rs`）
        // ⚠️ 仍拒绝 `.vcmi-fv-launcher/launcher.json`：直接覆盖用户的配置就是灾难
        || rel == super::config_patch::PATCH_REL
        // 删除清单（**一次性执行**，见 `updater/delete_patch.rs`）：补上"更新只能覆盖、不能删除"的缺口
        || rel == super::delete_patch::DELETE_REL
}

/// 解压结果是否含主程序 → 决定走"退出后脚本替换"还是"进程内直接落地"
pub(crate) fn staged_has_exe(version: &str) -> bool {
    staged_dir(version)
        .map(|d| d.join(staged_binary_name()).is_file())
        .unwrap_or(false)
}

/// 路径是否可安全落盘：既要在白名单内，也不得含空段或 `..`（挡 `runtime/../../x`）
fn rel_ok(rel: &str) -> bool {
    allowed_rel(rel) && !rel.split('/').any(|s| s.is_empty() || s == "..")
}

/// 需要**整目录跳过**的模组缓存 id 集合（`.vcmi-fv-launcher/mods/<id>/`）。
///
/// 两条规则（该 mod 的任何文件都不落地）：
/// 1. **包内缓存版本与当前 exe 不匹配** —— 落地后立刻被判无效并全量重建，白付下载与磁盘 IO；
/// 2. **用户侧已有该 mod 且版本匹配** —— 包里的缓存是给"缺失 / 失效"填坑用的，
///    覆盖用户已重建好的缓存毫无收益（还可能更旧）。
///
/// ⚠️ 只影响 `mods/**`（可重建的派生数据），**不碰**程序文件 / 组件 / 领域。
fn skip_mod_ids(archive: &mut zip::ZipArchive<fs::File>) -> HashSet<String> {
    let want = crate::modcache::version::WIKI_CACHE_VERSION as u64;
    let user_mods = VcmiPaths::mods_cache_dir();
    let mut skip = HashSet::new();
    for i in 0..archive.len() {
        let Ok(mut e) = archive.by_index(i) else { continue };
        let rel = e.name().replace('\\', "/");
        let rel = rel.trim_start_matches("./").to_string();
        let Some(rest) = rel.strip_prefix(".vcmi-fv-launcher/mods/") else {
            continue;
        };
        let mut segs = rest.split('/');
        let Some(id) = segs.next().filter(|s| !s.is_empty()) else {
            continue;
        };
        // 每个 mod 只需读它的 cache.json（判版本），其余条目跳过
        if segs.next() != Some("cache.json") {
            continue;
        }
        let mut text = String::new();
        if e.read_to_string(&mut text).is_err() {
            continue;
        }
        let pkg_ver = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v.get("cacheVersion").and_then(Value::as_u64));
        if pkg_ver != Some(want) {
            skip.insert(id.to_string()); // 规则 1
            continue;
        }
        if let Some(dir) = &user_mods {
            if user_cache_version(&dir.join(id)) == Some(want) {
                skip.insert(id.to_string()); // 规则 2
            }
        }
    }
    skip
}

/// 用户侧某 mod 目录的缓存版本（缺失 / 不可解析 → None）
fn user_cache_version(dir: &Path) -> Option<u64> {
    let text = fs::read_to_string(dir.join("cache.json")).ok()?;
    serde_json::from_str::<Value>(&text)
        .ok()?
        .get("cacheVersion")?
        .as_u64()
}

/// 该相对路径是否属于被跳过的模组目录
fn mod_id_in_skip(rel: &str, skip: &HashSet<String>) -> bool {
    rel.strip_prefix(".vcmi-fv-launcher/mods/")
        .and_then(|r| r.split('/').next())
        .map(|id| skip.contains(id))
        .unwrap_or(false)
}

/// 解压 zip 更新包到 `dest`，返回解出的相对路径列表（`/` 分隔，已校验）。
///
/// 任一文件不在白名单 → **整包拒绝**（不做"跳过坏文件继续"）。
/// `dest` 由调用方给出（生产走 [`staged_dir`]；单测用临时目录）。
pub(crate) fn extract_zip(zip_path: &Path, dest: &Path) -> Result<Vec<String>, String> {
    let _ = fs::remove_dir_all(dest);
    fs::create_dir_all(dest).map_err(|e| be_err("rt_extract_dir_create_failed", &format!("{}", e)))?;

    let file = fs::File::open(zip_path).map_err(|e| be_err("upd_pack_open_failed", &format!("{}", e)))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| be_err("upd_pack_invalid_zip", &format!("{}", e)))?;

    // 模组缓存：整目录跳过（见 [`skip_mod_ids`]）—— 不落盘无效缓存、也不覆盖用户已有缓存
    let skip = skip_mod_ids(&mut archive);
    if !skip.is_empty() {
        tracing::info!(
            target: "updater",
            "跳过 {} 个模组缓存（版本不匹配或本地已有且匹配）：{:?}",
            skip.len(),
            skip
        );
    }

    let mut out: Vec<String> = Vec::new();
    for i in 0..archive.len() {
        let mut e = archive
            .by_index(i)
            .map_err(|e| be_err("rt_pack_entry_read_failed", &format!("{}", e)))?;
        if e.is_dir() {
            continue;
        }
        let raw = e.name().replace('\\', "/");
        let rel = raw.trim_start_matches("./").to_string();
        if !rel_ok(&rel) {
            return Err(be_err("upd_pack_path_rejected", &format!("{}", rel)));
        }
        if mod_id_in_skip(&rel, &skip) {
            continue;
        }
        let target = dest.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        if let Some(p) = target.parent() {
            fs::create_dir_all(p).map_err(|e| be_err("cmd_mkdir_failed", &format!("{}", e)))?;
        }
        let mut of = fs::File::create(&target).map_err(|e| be_err("rt_write_failed", &format!("{} {}", rel, e)))?;
        std::io::copy(&mut e, &mut of).map_err(|e| be_err("rt_extract_failed", &format!("{} {}", rel, e)))?;
        out.push(rel);
    }
    // 允许**纯配置更新包**（不含 exe）：只要求非空，避免"空包"这种无意义更新。
    if out.is_empty() {
        return Err(be_err("updater_pkg_empty", ""));
    }
    Ok(out)
}

/// 确保暂存可用（可直接进 apply 阶段）：sha256 匹配；zip 还要求**已解压出主程序**。
///
/// zip 的解压目录会在启动清理时被删（`cleanup_stale`），这里**按需补解压** ——
/// 只要 `.zip` 还在且校验通过，就不必重新下载。
fn ensure_staged(entry: &ManifestEntry, dest: &Path) -> bool {
    if !dest.is_file() || verify(dest, &entry.sha256).is_err() {
        return false;
    }
    if !entry.is_zip() {
        return true;
    }
    match staged_dir(&entry.version) {
        Ok(d) if d.join(staged_binary_name()).is_file() => true,
        Ok(d) => extract_zip(dest, &d).is_ok(),
        Err(_) => false,
    }
}

/// 重查用：暂存是否可用（跨进程 / 跨会话重启后 `.zip` 仍在则按需补解压）。
/// 供 `manifest::check_update` 在启动期判断"已下载待应用"是否仍有效。
pub(crate) fn staged_ready(entry: &ManifestEntry) -> bool {
    match staged_path(entry) {
        Ok(dest) => ensure_staged(entry, &dest),
        Err(_) => false,
    }
}

/// 下载并校验更新包，返回暂存文件路径。`progress(received, total)` 每 200ms 回调一次。
///
/// 幂等：`Downloaded` 且暂存仍在时直接复用（重开弹窗 / 重试不重复下载）。
pub fn download_and_verify(progress: impl Fn(u64, u64)) -> Result<PathBuf, String> {
    let entry = { target().clone().ok_or_else(|| be_err("updater_no_checked_update", ""))? };
    let dest = staged_path(&entry)?;
    // 已有完整暂存（sha256 匹配）→ 直接复用并补做解压准备；跨进程重启同样适用
    if ensure_staged(&entry, &dest) {
        set_phase(UpdatePhase::Downloaded);
        super::set_pending_apply(Some(&entry.version));
        return Ok(dest);
    }

    set_phase(UpdatePhase::Downloading);
    set_cancelled(false);
    match download_inner(&entry, &dest, &progress) {
        Ok(()) => match verify(&dest, &entry.sha256) {
            Ok(()) => match prepare(&entry, &dest) {
                Ok(()) => {
                    set_phase(UpdatePhase::Downloaded);
                    super::set_pending_apply(Some(&entry.version));
                    Ok(dest)
                }
                Err(e) => {
                    let _ = fs::remove_file(&dest);
                    set_phase(UpdatePhase::Error);
                    super::set_pending_apply(None);
                    Err(e)
                }
            },
            Err(e) => {
                let _ = fs::remove_file(&dest);
                set_phase(UpdatePhase::Error);
                super::set_pending_apply(None);
                Err(e)
            }
        },
        Err(e) => {
            let _ = fs::remove_file(&dest);
            if e == UPDATE_DOWNLOAD_CANCELLED {
                set_phase(UpdatePhase::Available); // 可重试
            } else {
                set_phase(UpdatePhase::Error);
            }
            super::set_pending_apply(None);
            Err(e)
        }
    }
}

/// 下载完成后的准备：zip 形态解压到 `staged_<ver>/`（走白名单），exe 形态无需处理。
fn prepare(entry: &ManifestEntry, dest: &Path) -> Result<(), String> {
    if entry.is_zip() {
        extract_zip(dest, &staged_dir(&entry.version)?).map(|_| ())?;
    } else {
        // 最后一道防线：清单漏写 `format:"zip"` 时，绝不能把 zip 当 exe 替换上去
        // （那会让用户得到一个名为 .exe 的压缩包 → 启动失败）
        guard_exe_not_zip(dest)?;
    }
    Ok(())
}

/// 单 exe 形态的暂存不允许是 zip（按魔数判定，不依赖扩展名）
fn guard_exe_not_zip(path: &Path) -> Result<(), String> {
    use std::io::Read;
    let mut f = fs::File::open(path).map_err(|e| be_err("upd_pack_read_failed", &format!("{}", e)))?;
    let mut magic = [0u8; 4];
    let n = f.read(&mut magic).map_err(|e| be_err("upd_pack_read_failed", &format!("{}", e)))?;
    if n == 4 && &magic == b"PK\x03\x04" {
        return Err(be_err("updater_pack_is_zip", ""));
    }
    Ok(())
}

fn download_inner(
    entry: &super::manifest::ManifestEntry,
    dest: &Path,
    progress: &impl Fn(u64, u64),
) -> Result<(), String> {
    let (resp, _egress) = get_send(&entry.download).map_err(|e| be_err("repo_download_failed", &format!("{}", e)))?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(be_err("repo_download_server_returned", &format!("{}", status)));
    }
    let total = resp.content_length().unwrap_or(entry.size.unwrap_or(0));

    let mut reader = resp;
    let mut file = fs::File::create(dest).map_err(|e| be_err("upd_staging_file_create_failed", &format!("{}", e)))?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut received: u64 = 0;
    let mut last_emit = Instant::now();

    loop {
        if is_cancelled() {
            return Err(UPDATE_DOWNLOAD_CANCELLED.to_string());
        }
        let n = reader.read(&mut buf).map_err(|e| be_err("repo_download_interrupted", &format!("{}", e)))?;
        if n == 0 {
            break;
        }
        received += n as u64;
        if received > UPDATE_MAX_BYTES {
            return Err(be_err("upd_pack_too_large", &format!("{}", UPDATE_MAX_BYTES / 1024 / 1024)));
        }
        if total > 0 && received > total {
            return Err(be_err("updater_dl_oversize", ""));
        }
        use std::io::Write;
        file.write_all(&buf[..n])
            .map_err(|e| be_err("upd_staging_file_write_failed", &format!("{}", e)))?;

        if last_emit.elapsed() >= Duration::from_millis(200) {
            progress(received, total);
            last_emit = Instant::now();
        }
    }
    progress(received, total);
    Ok(())
}

/// sha256 强校验（比较不区分大小写）。
fn verify(path: &Path, expected: &str) -> Result<(), String> {
    let actual = sha256_file(path)?;
    if !actual.eq_ignore_ascii_case(expected.trim()) {
        return Err(be_err("updater_pkg_sha", ""));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;
    use zip::write::SimpleFileOptions;

    fn tmp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vcmi-updater-test-{name}"));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    /// 写一个测试用 zip（deflate）
    fn write_zip(path: &Path, entries: &[(&str, &[u8])]) {
        let f = fs::File::create(path).unwrap();
        let mut w = zip::ZipWriter::new(f);
        let opt = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (name, data) in entries {
            w.start_file(*name, opt).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap();
    }

    /// 白名单：程序文件 / 组件 / 领域 / 模组缓存 / 配置补丁全部放行
    #[test]
    fn zip_extract_accepts_allowed_paths() {
        let d = tmp_dir("ok");
        let z = d.join("p.zip");
        // ⚠️ 模组缓存的 `cache.json` 必须是合法 JSON 且版本与当前 exe 一致，
        //    否则会被 `skip_mod_ids` 规则 1 整目录跳过（另见 stale 用例）
        let cache = format!(
            r#"{{"cacheVersion":{}}}"#,
            crate::modcache::version::WIKI_CACHE_VERSION
        );
        write_zip(
            &z,
            &[
                (staged_binary_name(), b"EXE"),
                (".vcmi-fv-launcher/runtime/ai/onnxruntime.dll", b"DLL"),
                (".vcmi-fv-launcher/runtime/ai/model.onnx", b"MODEL"),
                (".vcmi-fv-launcher/realms/newrealm/realm.json", b"REALM"),
                (".vcmi-fv-launcher/mods/some-mod/cache.json", cache.as_bytes()),
                (".vcmi-fv-launcher/launcher.patch.json", b"{}"),
                (".vcmi-fv-launcher/launcher.delete.json", b"{\"paths\":[]}"),
            ],
        );
        let out = d.join("out");
        let files = extract_zip(&z, &out).unwrap();
        assert_eq!(files.len(), 7, "实际解出：{files:?}");
        assert!(out.join(".vcmi-fv-launcher/runtime/ai/onnxruntime.dll").is_file());
        assert!(out.join(".vcmi-fv-launcher/runtime/ai/model.onnx").is_file());
        assert!(out.join(".vcmi-fv-launcher/realms/newrealm/realm.json").is_file());
        assert!(out.join(".vcmi-fv-launcher/mods/some-mod/cache.json").is_file());
        assert!(staged_has_exe_from(&out));
        let _ = fs::remove_dir_all(&d);
    }

    /// 模组缓存**版本不匹配** → 整个 mod 目录跳过（不落盘无效缓存：落地也立刻会被判失效重建）
    #[test]
    fn zip_extract_skips_stale_mod_cache() {
        let d = tmp_dir("stalemod");
        let z = d.join("p.zip");
        let stale = b"{\"cacheVersion\":0}";
        write_zip(
            &z,
            &[
                (staged_binary_name(), b"EXE"),
                (".vcmi-fv-launcher/mods/stale/cache.json", stale),
                (".vcmi-fv-launcher/mods/stale/other.txt", b"X"),
            ],
        );
        let out = d.join("out");
        let files = extract_zip(&z, &out).unwrap();
        assert_eq!(files, vec![staged_binary_name().to_string()], "过期缓存应整目录跳过");
        assert!(!out.join(".vcmi-fv-launcher/mods/stale").exists());
        let _ = fs::remove_dir_all(&d);
    }

    /// 白名单：**缓存 / 下载 / 配置本体一律拒绝**
    /// （`launcher.json` 直接覆盖会丢用户配置 —— 要改配置只能走 `launcher.patch.json`）
    #[test]
    fn zip_extract_rejects_forbidden_paths() {
        let d = tmp_dir("reject");
        for bad in [
            ".vcmi-fv-launcher/launcher.json",
            ".vcmi-fv-launcher/state.json",
            ".vcmi-fv-launcher/cache/updateManifest/manifest.json",
            ".vcmi-fv-launcher/downloads/x.zip",
            ".vcmi-fv-launcher/logs/launcher.log",
            // 组件只认 `.vcmi-fv-launcher/runtime/`：旧的平铺位置必须拒绝，避免语义分叉
            "runtime/ai/onnxruntime.dll",
            "other.txt",
            ".gitignore",
        ] {
            let z = d.join("bad.zip");
            write_zip(&z, &[(staged_binary_name(), b"EXE"), (bad, b"X")]);
            let out = d.join("out");
            assert!(extract_zip(&z, &out).is_err(), "应当拒绝：{bad}");
        }
        let _ = fs::remove_dir_all(&d);
    }

    /// 路径守卫：白名单前缀通过不代表安全（`runtime/../../x` 必须挡掉）
    #[test]
    fn rel_guard_blocks_traversal_and_empties() {
        assert!(rel_ok(staged_binary_name()));
        assert!(rel_ok(".vcmi-fv-launcher/runtime/ai/onnxruntime.dll"));
        assert!(rel_ok(".vcmi-fv-launcher/runtime/ai/model.onnx"));
        // 旧的平铺位置（`<exe>/runtime/`）已废弃 —— 组件只认数据目录下那一处
        assert!(!rel_ok("runtime/ai/onnxruntime.dll"));
        assert!(rel_ok(".vcmi-fv-launcher/realms/a/realm.json"));
        assert!(rel_ok(".vcmi-fv-launcher/mods/m/cache.json"));
        assert!(!rel_ok("runtime/../../escape.txt"));
        assert!(!rel_ok(".vcmi-fv-launcher/realms/../../escape.json"));
        assert!(!rel_ok("runtime//x.dll"));
        assert!(!rel_ok(".vcmi-fv-launcher/launcher.json"));
        assert!(!rel_ok(".vcmi-fv-launcher/cache/x.json"));
    }

    /// **纯配置更新包**（不含 exe）允许 —— 由应用层改走"进程内就地落地"
    #[test]
    fn zip_extract_allows_config_only_package() {
        let d = tmp_dir("cfgonly");
        let z = d.join("p.zip");
        write_zip(
            &z,
            &[
                (".vcmi-fv-launcher/realms/r1/realm.json", b"R"),
                (".vcmi-fv-launcher/launcher.patch.json", b"{}"),
            ],
        );
        let out = d.join("out");
        let files = extract_zip(&z, &out).unwrap();
        assert_eq!(files.len(), 2);
        assert!(!staged_has_exe_from(&out), "不含主程序 → 走就地落地分支");
        let _ = fs::remove_dir_all(&d);
    }

    /// 空包拒绝（避免"无意义更新"）
    #[test]
    fn zip_extract_rejects_empty_package() {
        let d = tmp_dir("empty");
        let z = d.join("p.zip");
        write_zip(&z, &[]);
        let out = d.join("out");
        assert!(extract_zip(&z, &out).is_err());
        let _ = fs::remove_dir_all(&d);
    }

    /// 测试用：判断某个解压目录里有没有主程序（生产走 [`staged_has_exe`]）
    fn staged_has_exe_from(dir: &Path) -> bool {
        dir.join(staged_binary_name()).is_file()
    }

    /// 清单漏写 `format:"zip"` 时的防线：zip 内容不得被当 exe 接受
    #[test]
    fn guard_rejects_zip_content_in_exe_format() {
        let d = tmp_dir("guard");
        let z = d.join("p.zip");
        write_zip(&z, &[(staged_binary_name(), b"EXE")]);
        assert!(guard_exe_not_zip(&z).is_err(), "zip 内容必须被拒绝");

        let exe = d.join("plain.exe");
        fs::write(&exe, b"MZ\x90\x00fake-pe").unwrap();
        assert!(guard_exe_not_zip(&exe).is_ok(), "普通内容应放行");
        let _ = fs::remove_dir_all(&d);
    }

    /// **闭环验收**：用 `npm run release:pack -- --version <v> --no-build` 真实打出的便携包跑一遍解压，
    /// 确认白名单不会误拒我们自己的产物（否则线上更新会失败）。
    ///
    /// 跑法：`VCMI_TEST_UPDATE_ZIP=../releases/<dir>/portable_*.zip cargo test --lib extract_real_portable_zip -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn extract_real_portable_zip() {
        let p = std::env::var("VCMI_TEST_UPDATE_ZIP")
            .expect("需设 VCMI_TEST_UPDATE_ZIP 指向 portable_*.zip");
        let out = tmp_dir("real");
        let files = extract_zip(Path::new(&p), &out).unwrap();
        println!("解出 {} 个文件：{files:?}", files.len());
        assert!(files.iter().any(|f| f == staged_binary_name()), "必须含主程序");
        assert!(out.join(staged_binary_name()).is_file());
        let _ = fs::remove_dir_all(out.parent().unwrap_or(&out));
    }
}
