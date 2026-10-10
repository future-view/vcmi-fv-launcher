//! 引擎版本读取：`get_compile_info` 的 `vcmiVersion` / `coreModVersion` 来源。
//!
//! **三级来源，顺序不可颠倒**（全部为实测结论）：
//! 1. `VCMI_client.exe -v`（**权威**）：`clientapp/EntryPoint.cpp::prog_version()`
//!    在 SDL / 引擎初始化**之前**打印并 `return 0` —— 无窗口、无副作用、退出码 0。
//!    实测输出 `VCMI 1.7.4.652bb0b7c140…`（`VCMI_VERSION_FULL` = 版本号 + git sha）。
//! 2. PE 文件版本（若某发行版写入了 rc 资源）。
//! 3. 扫描 `VCMI_client.exe` / `VCMI_server.exe` 里**唯一**的 `x.y.z` ASCII 串
//!    （`Version.cpp` 里的 `const char VCMI_VERSION[]` 编译期常量；实测 client 与 server
//!    各只含一个候选且值一致；`VCMI_launcher.exe` 噪音太多不可用）。
//! 三级都失败 → `"Unknown"`，前端此时**不做**兼容性判定（宁可放过，不可误报）。
//!
//! ⚠️ **绝对不要**把 `Mods/vcmi/mod.json` 的 `version` 当作引擎版本：实测两个参考安装
//! 都是 `1.5`（核心 mod 的**资产版本**），而引擎实际是 1.7.4 / 1.7.5 —— 用它判 `compatibility`
//! 会把 66 个 `min >= 1.6` 的模组全部误报为不兼容。该值单独作为 `coreModVersion` 返回。
//!
//! 依赖方向：只依赖 `infra`（见 `docs/architecture/backend.zh-CN.md` §4.1）。

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use crate::infra::cache::{cfg_cache_get, fingerprint_of};

/// `-v` 探测的等待上限：实测即时返回，超时视为失败（不阻塞 IPC）。
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Windows `CREATE_NO_WINDOW`：避免 `-v` 探测闪出一个黑色控制台窗口。
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

static VERSION_CACHE: OnceLock<Mutex<Vec<(String, Arc<EngineVersionInfo>)>>> = OnceLock::new();

/// 版本探测结果（`source` 供日志排查"版本是怎么读出来的"）。
pub(crate) struct EngineVersionInfo {
    /// 引擎版本（`1.7.4` 形式；读不到为 `Unknown`）
    pub version: String,
    /// 内置核心 mod 的资产版本（`Mods/vcmi/mod.json` 的 `version`，**不是**引擎版本，仅供诊断）
    pub core_mod_version: String,
    /// 版本来源：`cli` / `pe` / `scan` / `none`
    pub source: &'static str,
}

/// 读取引擎版本（进程内按 exe 与内置 mod 指纹缓存；文件变化自动失效）。
pub(crate) fn engine_version(game_dir: &Path, mods_dirs: &[PathBuf]) -> Arc<EngineVersionInfo> {
    let key = fingerprint_of(&version_fingerprint_paths(game_dir, mods_dirs));
    cfg_cache_get(&VERSION_CACHE, key, || {
        let (version, source) = match detect_engine_version(game_dir) {
            Some((v, s)) => (v, s),
            None => ("Unknown".to_string(), "none"),
        };
        tracing::debug!(target: "vcmi", "引擎版本 = {version}（来源 {source}）");
        EngineVersionInfo {
            version,
            core_mod_version: read_core_mod_version(mods_dirs),
            source,
        }
    })
}

/// 缓存指纹覆盖全部会被读取的文件：改动其一即重新探测。
fn version_fingerprint_paths(game_dir: &Path, mods_dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut v = vec![
        game_dir.join("VCMI_client.exe"),
        game_dir.join("VCMI_server.exe"),
    ];
    for d in mods_dirs {
        v.push(d.join("vcmi").join("mod.json"));
    }
    v
}

fn detect_engine_version(game_dir: &Path) -> Option<(String, &'static str)> {
    let client = game_dir.join("VCMI_client.exe");
    if let Some(v) = probe_version_arg(&client) {
        return Some((v, "cli"));
    }
    if let Some(v) = pe_file_version(&client) {
        return Some((v, "pe"));
    }
    scan_exe_version(game_dir).map(|v| (v, "scan"))
}

/// 走 `VCMI_client.exe -v`：读 stdout 的版本行。
///
/// 在**独立线程**里 `wait_with_output` + 主线程 `recv_timeout`：进程万一不退出也不会卡住 IPC
/// （命令照常返回失败，探测线程随后自行结束）。
fn probe_version_arg(exe: &Path) -> Option<String> {
    if !exe.is_file() {
        return None;
    }
    let mut cmd = Command::new(exe);
    cmd.arg("-v")
        .current_dir(exe.parent()?)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let child = cmd.spawn().ok()?;
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(child.wait_with_output());
    });
    let out = rx.recv_timeout(PROBE_TIMEOUT).ok()?.ok()?;
    parse_prog_version(&String::from_utf8_lossy(&out.stdout))
}

/// 从 `-v` 输出里取版本行：`VCMI 1.7.4.652bb0b7c140…` → `1.7.4`。
fn parse_prog_version(stdout: &str) -> Option<String> {
    for line in stdout.lines() {
        let l = line.trim();
        if let Some(rest) = l.strip_prefix("VCMI ") {
            if let Some(v) = normalize_version(rest.split_whitespace().next().unwrap_or("")) {
                return Some(v);
            }
        }
    }
    None
}

/// 取版本串的前 3 段数字：`1.7.4.652bb0b7…` → `1.7.4`（第 4 段是 git sha，遇非数字即停）。
fn normalize_version(raw: &str) -> Option<String> {
    let mut out: Vec<&str> = Vec::new();
    for seg in raw.split('.') {
        if seg.is_empty() || !seg.chars().all(|c| c.is_ascii_digit()) {
            break;
        }
        out.push(seg);
        if out.len() == 3 {
            break;
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out.join("."))
    }
}

/// 扫 `client` / `server` 里**唯一**的 `x.y.z` 串；多于一个候选即放弃（宁可不判）。
fn scan_exe_version(game_dir: &Path) -> Option<String> {
    for name in ["VCMI_client.exe", "VCMI_server.exe"] {
        if let Some(v) = scan_unique_xyz(&game_dir.join(name)) {
            return Some(v);
        }
    }
    None
}

fn scan_unique_xyz(exe: &Path) -> Option<String> {
    let data = fs::read(exe).ok()?;
    let set = collect_xyz(&data);
    if set.len() == 1 {
        set.into_iter().next()
    } else {
        None
    }
}

/// 收集二进制里形如 `d{1,3}.d{1,3}.d{1,3}` 的 ASCII 候选（取每段数字点串的前 3 段）。
///
/// 语义要点：
/// - 起点左侧不得是数字 / 点（否则说明该串是从更长串中间截出来的）；
/// - 串末允许**更长**（`1.7.4.652bb0b7…` 是 `VCMI_VERSION_FULL` = 版本 + git sha）：
///   按段切分后**取前 3 段**即为版本号，第 4 段（sha 的十六进制数字前缀）忽略；
/// - 所有段必须是 1–3 位数字，否则整串丢弃（挡掉 `10.0.19045` 这类系统版本噪音）。
fn collect_xyz(data: &[u8]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let n = data.len();
    let mut i = 0usize;
    while i < n {
        if !data[i].is_ascii_digit()
            || (i > 0 && (data[i - 1].is_ascii_digit() || data[i - 1] == b'.'))
        {
            i += 1;
            continue;
        }
        let start = i;
        let mut j = i;
        while j < n && (data[j].is_ascii_digit() || data[j] == b'.') {
            j += 1;
        }
        // 去掉尾随点（`1.7.4.` 这类）
        let mut end = j;
        while end > start && data[end - 1] == b'.' {
            end -= 1;
        }
        let s = String::from_utf8_lossy(&data[start..end]);
        let segs: Vec<&str> = s.split('.').collect();
        let well_formed = segs.len() >= 3
            && segs
                .iter()
                .all(|x| !x.is_empty() && x.len() <= 3 && x.chars().all(|c| c.is_ascii_digit()));
        if well_formed {
            out.insert(segs[..3].join("."));
        }
        i = if j > i { j } else { i + 1 };
    }
    out
}

/// 内置核心 mod 的资产版本（诊断用；**不是**引擎版本，见模块头警告）。
fn read_core_mod_version(mods_dirs: &[PathBuf]) -> String {
    for dir in mods_dirs {
        let p = dir.join("vcmi").join("mod.json");
        let Ok(text) = fs::read_to_string(&p) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        if let Some(ver) = v.get("version").and_then(|x| x.as_str()) {
            let ver = ver.trim();
            if !ver.is_empty() {
                return ver.to_string();
            }
        }
    }
    "Unknown".into()
}

/* ===================== Windows PE 文件版本读取（FFI） ===================== */

#[cfg(windows)]
fn pe_file_version(exe: &Path) -> Option<String> {
    if !exe.is_file() {
        return None;
    }
    let v = read_pe_file_version(exe)?;
    let v = v.trim().to_string();
    if v.is_empty() || v == "0.0.0.0" {
        return None;
    }
    // PE 版本可能是 4 段（`1.7.4.0`），统一收敛到 3 段
    normalize_version(&v).or(Some(v))
}

#[cfg(not(windows))]
fn pe_file_version(_exe: &Path) -> Option<String> {
    None
}

#[cfg(windows)]
fn read_pe_file_version(path: &Path) -> Option<String> {
    use std::os::windows::ffi::OsStrExt;
    let wide: Vec<u16> = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let handle_size = GetFileVersionInfoSizeW(wide.as_ptr(), std::ptr::null_mut());
        if handle_size == 0 {
            return None;
        }
        let mut buffer: Vec<u8> = vec![0u8; handle_size as usize];
        if GetFileVersionInfoW(wide.as_ptr(), 0, handle_size, buffer.as_mut_ptr()) == 0 {
            return None;
        }
        let mut queries: Vec<String> = Vec::new();
        let mut lang_ptr: *mut u16 = std::ptr::null_mut();
        let mut lang_size: u32 = 0;
        if VerQueryValueW(
            buffer.as_ptr() as *mut _,
            encode_wide_null("\\VarFileInfo\\Translation").as_ptr(),
            &mut lang_ptr as *mut _ as *mut *mut _,
            &mut lang_size,
        ) != 0
            && lang_size >= 4
        {
            let lang = *lang_ptr;
            let code_page = *(lang_ptr.offset(1));
            queries.push(format!(
                "\\StringFileInfo\\{:04x}{:04x}\\FileVersion",
                lang, code_page
            ));
        }
        // 回退：常见英文（美国）编码
        queries.push("\\StringFileInfo\\040904b0\\FileVersion".to_string());

        for query in &queries {
            let mut value_ptr: *mut u16 = std::ptr::null_mut();
            let mut value_size: u32 = 0;
            if VerQueryValueW(
                buffer.as_ptr() as *mut _,
                encode_wide_null(query).as_ptr(),
                &mut value_ptr as *mut _ as *mut *mut _,
                &mut value_size,
            ) == 0
            {
                continue;
            }
            if value_size == 0 {
                continue;
            }
            let slice = std::slice::from_raw_parts(value_ptr, value_size as usize);
            let s = String::from_utf16_lossy(slice);
            let clean = s.trim_end_matches('\0').trim();
            if !clean.is_empty() {
                return Some(clean.to_string());
            }
        }
        None
    }
}

#[cfg(windows)]
fn encode_wide_null(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(windows)]
extern "system" {
    fn GetFileVersionInfoSizeW(lptstrfilename: *const u16, lpdwhandle: *mut u32) -> u32;
    fn GetFileVersionInfoW(lptstrfilename: *const u16, dwhandle: u32, dwlen: u32, lpdata: *mut u8) -> u32;
    fn VerQueryValueW(pblock: *mut u8, lpstrsubblock: *const u16, lplpbuffer: *mut *mut u8, pulen: *mut u32) -> u32;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_version_keeps_three_numeric_segments() {
        assert_eq!(normalize_version("1.7.4.652bb0b7c140").as_deref(), Some("1.7.4"));
        assert_eq!(normalize_version("1.7.5").as_deref(), Some("1.7.5"));
        assert_eq!(normalize_version("1.8").as_deref(), Some("1.8"));
        assert_eq!(normalize_version("abc"), None);
        assert_eq!(normalize_version("").as_deref(), None);
    }

    #[test]
    fn parse_prog_version_reads_cli_output() {
        let out = "Starting... \nVCMI 1.7.4.652bb0b7c140d8cdf5c4f3ac4892163fecb5cdf0\n  game data:\t\t.\n";
        assert_eq!(parse_prog_version(out).as_deref(), Some("1.7.4"));
        assert_eq!(parse_prog_version("no version here"), None);
    }

    #[test]
    fn collect_xyz_takes_first_three_segments() {
        // 真实形态：`VCMI_VERSION_FULL` = 版本 + git sha（十六进制）
        let mut data = b"\0VCMI 1.7.4.652bb0b7c140d8cdf5c4f3ac4892163fecb5cdf0\0".to_vec();
        data.extend_from_slice(b"1.2.3.4.5");
        let set = collect_xyz(&data);
        assert!(set.contains("1.7.4"), "应取出前三段：{set:?}");
        assert!(set.contains("1.2.3"), "长串也取前三段：{set:?}");
        assert!(!set.contains("2.3.4"), "整串一次性消费，不产生错位窗口：{set:?}");
    }

    #[test]
    fn collect_xyz_rejects_noise_patterns() {
        // 段超长（Windows 版本 `10.0.19045`）与少于 3 段（`1.7`）→ 不采信
        let set = collect_xyz(b"10.0.19045 1.7");
        assert!(set.is_empty(), "不应产生候选：{set:?}");
        // 多候选（如 Qt 版本串）：都收集，由调用方按"唯一"判定放弃
        let multi = collect_xyz(b"1.7.4 2.0.11");
        assert_eq!(multi.len(), 2, "两个候选都应可辨：{multi:?}");
    }

    /// 端到端：对参考安装真跑一次 `-v` 探测（默认 ignore，需 `references/`）。
    /// 断言"引擎版本不等于核心 mod 的 1.5"，这正是修复前会误报成不兼容的取值。
    #[test]
    #[ignore = "需要 references/ 参考游戏目录，仅人工验证"]
    fn real_installations_report_expected_engine_version() {
        let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("references");
        for (dir, expect) in [
            ("Heroes3-VCMI", "1.7.4"),
            ("Heroes3-VCMI", "1.7.5"),
        ] {
            let game = base.join(dir);
            if !game.is_dir() {
                eprintln!("跳过：{} 不存在", game.display());
                continue;
            }
            let mods = vec![game.join("userdata").join("Mods"), game.join("Mods")];
            let info = engine_version(&game, &mods);
            println!(
                "{dir}: engine={} source={} coreMod={}",
                info.version, info.source, info.core_mod_version
            );
            assert_eq!(info.version, expect, "{dir} 的引擎版本");
            assert_eq!(info.source, "cli", "{dir} 应走 -v 探测");
            assert_ne!(info.version, info.core_mod_version, "不得把核心 mod 版本当引擎版本");
        }
    }
}
