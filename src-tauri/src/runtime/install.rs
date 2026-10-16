//! 组件安装：下载 zip → sha256 校验 → **白名单解压** → 逐文件校验 → 原子替换。
//!
//! 与 `repository::download`（远程领域包）同一套模式：同步阻塞执行 + 进度回调 +
//! 全局取消标记；事件 emit 由 `commands` 层负责（本层无 tauri 类型）。

use crate::infra::berror::be_err;
use std::collections::HashSet;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::infra::hash::sha256_file;
use crate::infra::http::get_send;
use crate::vcmi::paths::VcmiPaths;

use super::{def, dir_of, package_of, ComponentDef};

/// 单个组件包的大小上限（100MB：ORT + 模型目前不到 20MB，留足余量）
const PACKAGE_MAX_BYTES: u64 = 100 * 1024 * 1024;

static CANCELLED: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn cancelled_set() -> &'static Mutex<HashSet<String>> {
    CANCELLED.get_or_init(|| Mutex::new(HashSet::new()))
}

/// 请求取消某个组件的安装
pub fn cancel(id: &str) {
    if let Ok(mut s) = cancelled_set().lock() {
        s.insert(id.to_string());
    }
}

pub(crate) fn is_cancelled(id: &str) -> bool {
    cancelled_set()
        .lock()
        .map(|s| s.contains(id))
        .unwrap_or(false)
}

fn clear_cancelled(id: &str) {
    if let Ok(mut s) = cancelled_set().lock() {
        s.remove(id);
    }
}

// ── 安装中标记（供更新器互斥）──────────────────────────────────────────────

static BUSY: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();

fn busy_set() -> &'static Mutex<HashSet<String>> {
    BUSY.get_or_init(|| Mutex::new(HashSet::new()))
}

/// 是否有组件正在安装 / 卸载。
///
/// ⚠️ 供**启动器更新**互斥使用：更新包也可能写 `runtime/**`（打包预置组件），
/// 与设置页的组件安装并发会互相踩踏同一目录。
pub fn any_busy() -> bool {
    busy_set().lock().map(|s| !s.is_empty()).unwrap_or(false)
}

fn set_busy(id: &str, on: bool) {
    if let Ok(mut s) = busy_set().lock() {
        if on {
            s.insert(id.to_string());
        } else {
            s.remove(id);
        }
    }
}

/// 安装（或覆盖安装）组件。
///
/// `progress(received, total)` 每 200ms 回调一次；`total` 可能为 0（服务端未给长度）。
///
/// 全程持有 [`any_busy`] 标记（含提前返回路径），便于与更新器互斥。
pub fn install(id: &str, progress: impl Fn(u64, u64)) -> Result<(), String> {
    set_busy(id, true);
    let r = install_inner(id, progress);
    set_busy(id, false);
    r
}

fn install_inner(id: &str, progress: impl Fn(u64, u64)) -> Result<(), String> {
    let d = def(id).ok_or_else(|| be_err("runtime_unknown_component", id))?;
    let pkg = package_of(id).ok_or_else(|| {
        format!("组件 {id} 未配置远程包（launcher.json 的 runtime.components.{id}.package），无法在线安装")
    })?;
    let dest = dir_of(id).ok_or_else(|| be_err("common_component_dir_unknown", ""))?;
    let tmp = tmp_root()?;
    clear_cancelled(id);

    let zip_path = tmp.join(format!("{id}.zip"));
    download(&pkg.url, &zip_path, &progress, id)?;

    // 整包 sha256：远程来源必须可校验
    let got = sha256_file(&zip_path)?;
    if !got.eq_ignore_ascii_case(pkg.sha256.trim()) {
        let _ = fs::remove_file(&zip_path);
        return Err(be_err("runtime_install_sha", ""));
    }
    if is_cancelled(id) {
        let _ = fs::remove_file(&zip_path);
        return Err(be_err("runtime_install_cancelled", ""));
    }

    let extract = tmp.join(format!("{id}_extract"));
    let _ = fs::remove_dir_all(&extract);
    fs::create_dir_all(&extract)
        .map_err(|e| be_err("rt_extract_dir_create_failed", &format!("{}", e)))?;

    extract_whitelisted(&zip_path, d, &extract)?;

    // 逐文件校验：任何一项不匹配 → 整包作废（不留下半截组件）
    // 必需文件 + 随包模型都要在包里（模型的"可选"指**使用**可选，不是"包可以不带"）
    for f in d.files.iter().chain(d.models.iter()) {
        let p = extract.join(f.name);
        if !p.is_file() {
            return Err(be_err("rt_pack_missing_file", &format!("{}", f.name)));
        }
        let got = sha256_file(&p)?;
        if !got.eq_ignore_ascii_case(f.sha256) {
            return Err(be_err("rt_file_verify_failed", &format!("{}", f.name)));
        }
    }

    replace_dir(&extract, &dest)?;

    let _ = fs::remove_file(&zip_path);
    let _ = fs::remove_dir_all(&extract);
    Ok(())
}

/// 卸载组件：删除整个组件目录（不影响用户数据）
pub fn uninstall(id: &str) -> Result<(), String> {
    set_busy(id, true);
    let r = (|| {
        let dest = dir_of(id).ok_or_else(|| be_err("common_component_dir_unknown", ""))?;
        if dest.exists() {
            fs::remove_dir_all(&dest)
                .map_err(|e| be_err("rt_component_dir_delete_failed", &format!("{}", e)))?;
        }
        Ok(())
    })();
    set_busy(id, false);
    r
}

fn tmp_root() -> Result<PathBuf, String> {
    let d = VcmiPaths::launcher_data_dir()
        .ok_or_else(|| be_err("common_data_dir_unknown", ""))?
        .join(".tmp-runtime");
    fs::create_dir_all(&d).map_err(|e| be_err("rt_temp_dir_create_failed", &format!("{}", e)))?;
    Ok(d)
}

fn download(url: &str, dest: &Path, progress: &impl Fn(u64, u64), id: &str) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| be_err("repo_download_dir_create_failed", &format!("{}", e)))?;
    }
    let (resp, _egress) =
        get_send(url).map_err(|e| be_err("repo_download_failed", &format!("{}", e)))?;
    let status = resp.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(be_err(
            "repo_download_server_returned",
            &format!("{}", status),
        ));
    }
    let total = resp.content_length().unwrap_or(0);

    let mut reader = resp;
    let mut file =
        fs::File::create(dest).map_err(|e| be_err("rt_file_create_failed", &format!("{}", e)))?;
    let mut buf = vec![0u8; 64 * 1024];
    let mut received: u64 = 0;
    let mut last_emit = Instant::now();

    loop {
        if is_cancelled(id) {
            return Err(be_err("runtime_install_cancelled", ""));
        }
        let n = reader
            .read(&mut buf)
            .map_err(|e| be_err("repo_download_interrupted", &format!("{}", e)))?;
        if n == 0 {
            break;
        }
        received += n as u64;
        if received > PACKAGE_MAX_BYTES {
            return Err(be_err(
                "rt_pack_too_large",
                &format!("{}", PACKAGE_MAX_BYTES / 1024 / 1024),
            ));
        }
        file.write_all(&buf[..n])
            .map_err(|e| be_err("realm_file_write_failed", &format!("{}", e)))?;
        if last_emit.elapsed() >= Duration::from_millis(200) {
            progress(received, total);
            last_emit = Instant::now();
        }
    }
    progress(received, total);
    Ok(())
}

/// **白名单解压**：只提取组件清单里声明的文件（按 basename 匹配），其它条目一律跳过。
///
/// ⚠️ 安全性：目标路径固定为 `<out>/<白名单文件名>`，**不使用压缩包内的目录结构**，
/// 因此 `../`、绝对路径、盘符等 zip slip 手法在这里全部无效。
fn extract_whitelisted(zip_path: &Path, d: &ComponentDef, out: &Path) -> Result<(), String> {
    let file =
        fs::File::open(zip_path).map_err(|e| be_err("rt_pack_open_failed", &format!("{}", e)))?;
    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| be_err("rt_pack_invalid_zip", &format!("{}", e)))?;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| be_err("rt_pack_entry_read_failed", &format!("{}", e)))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        let base = name.rsplit(['/', '\\']).next().unwrap_or(&name).to_string();
        // 非清单文件（README、无关的 dll…）跳过，不落盘。
        // 白名单 = 必需文件 + 随包模型（模型要在包里，否则上面的逐文件校验会判失败）
        if !d
            .files
            .iter()
            .chain(d.models.iter())
            .any(|f| f.name == base.as_str())
        {
            continue;
        }
        let target = out.join(&base);
        let mut out_f = fs::File::create(&target)
            .map_err(|e| be_err("rt_write_failed", &format!("{} {}", base, e)))?;
        std::io::copy(&mut entry, &mut out_f)
            .map_err(|e| be_err("rt_extract_failed", &format!("{} {}", base, e)))?;
    }
    Ok(())
}

/// 原子替换目录：旧目录先改名为 `.old`，新目录 rename 到位成功后才删除备份；失败则还原。
fn replace_dir(src: &Path, dest: &Path) -> Result<(), String> {
    if let Some(parent) = dest.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| be_err("rt_component_dir_create_failed", &format!("{}", e)))?;
    }
    let backup = PathBuf::from(format!("{}.old", dest.display()));
    let has_old = dest.exists();

    if has_old {
        let _ = fs::remove_dir_all(&backup);
        fs::rename(dest, &backup)
            .map_err(|e| be_err("rt_old_component_backup_failed", &format!("{}", e)))?;
    }
    match fs::rename(src, dest) {
        Ok(()) => {
            let _ = fs::remove_dir_all(&backup);
            Ok(())
        }
        Err(e) => {
            // 还原，避免"旧组件没了、新组件也没装上"
            if has_old {
                let _ = fs::rename(&backup, dest);
            }
            Err(be_err("rt_component_dir_replace_failed", &format!("{}", e)))
        }
    }
}
