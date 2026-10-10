//! 本地 zip 安装模组：`preview_mod_zip` / `install_local_mod` 的实现。
//!
//! 复刻官方 `ModStateController::doInstallMod` + `detectModArchive`
//! （`references/vcmi/launcher/modManager/modstatecontroller.cpp:115-149, 276-335`），
//! 并在四处**有意改进**（都以"不破坏既有安装"为准）：
//!
//! 1. **先解压到临时目录再整体就位**（`Mods/.fv-install-<id>/`，点开头 → `scan_user_mods_dir` 会跳过）：
//!    解压中断/失败不会留下半截 mod，也不会动到已有安装。
//! 2. **只解压 mod 根内的条目**并剥离该前缀：官方按原路径解压到 `Mods/` 再改名，
//!    包根部的散文件（readme / LICENSE）会散落到 `Mods/` 里污染目录。
//! 3. **支持"根目录即 mod"的包**（官方 `detectModArchive` 只认 `X/mod.json` 与 `X/Y/mod.json`，
//!    根目录直接放 `mod.json` 的包会被判为无效）。
//! 4. **覆盖时旧目录改名保留而非直接删**：删除失败也无妨；对目录联接（junction）只 `remove_dir`，
//!    否则会毁掉联接指向的真实文件（见 `infra::links::is_link`）。
//!
//! mod id 取「包内 mod 目录名」（根目录包取 zip 文件名），并**去掉尾随版本号**
//! （`hota-1.7.3` → `hota`）—— 否则 `depends: ["hota"]` 之类依赖声明会全部失配。
//!
//! 依赖方向：只依赖 `infra`（见 `docs/architecture/backend.zh-CN.md` §4.1）。

use crate::infra::berror::be_err;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::infra::links::is_link;

/// 安装台账文件名（写在 mod 根目录内，见方案 `mod-acquisition-update.zh-CN.md` §3.2）：
/// 记录远程来源 / 校验等级 / changelog，dot 前缀使目录扫描天然跳过；
/// 领域私有副本的清理保护另由 `realm.json` 的 `pinned` 台账驱动（见 `infra::links::apply_realm_mods`）。
pub(crate) const LEDGER_FILE: &str = ".fv-install.json";

/// 读取模组安装台账（不存在 / 解析失败 → None）。
/// 台账随 `install_with_ledger` 写入；回读供 M3 抽屉展示来源/校验等级，当前仅单测消费。
#[allow(dead_code)]
pub(crate) fn read_ledger(mod_dir: &Path) -> Option<Value> {
    let text = fs::read_to_string(mod_dir.join(LEDGER_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// 是否带安装台账（= 远程仓库安装 / 领域私有副本）。
/// 领域清理的保护依据是 `realm.json` 的 `pinned` 台账（bundled 副本也带此标记，
/// 不能用它作清理守卫），故当前仅诊断 / M3 消费。
#[allow(dead_code)]
pub(crate) fn has_ledger(mod_dir: &Path) -> bool {
    mod_dir.join(LEDGER_FILE).is_file()
}

/// 包探查结果（安装前预览；`target` / `exists` 供前端确认覆盖）。
#[derive(Debug)]
pub(crate) struct ZipInspection {
    pub mod_id: String,
    pub name: String,
    pub version: String,
    pub mod_type: String,
    /// 将写入的文件数（不含目录条目）
    pub entry_count: usize,
    pub target: PathBuf,
    pub exists: bool,
}

/// 安装结果。
#[derive(Debug)]
pub(crate) struct InstallOutcome {
    pub mod_id: String,
    pub dest: PathBuf,
    pub installed_files: usize,
    /// 是否替换了已存在的同名模组
    pub replaced: bool,
}

/// 探查 zip：识别 mod 根、读取 `mod.json` 元数据、推导 mod id 与目标目录（**不写盘**）。
pub(crate) fn inspect(zip_path: &Path, mods_dir: &Path) -> Result<ZipInspection, String> {
    let (info, _root) = archive_info(zip_path, mods_dir)?;
    Ok(info)
}

/// 安装 zip 到 `mods_dir/<mod_id>`。
///
/// - `overwrite=false` 且目标已存在 → 返回错误（由前端确认后改传 `true`）。
/// - 失败时保证：临时目录被清理、既有安装**原样保留**。
pub(crate) fn install(
    zip_path: &Path,
    mods_dir: &Path,
    overwrite: bool,
) -> Result<InstallOutcome, String> {
    install_with_ledger(zip_path, mods_dir, overwrite, None)
}

/// 同 `install`，但可在安装前向暂存目录写入安装台账 `ledger`（落盘即随目录原子就位，不污染 mod.json）。
pub(crate) fn install_with_ledger(
    zip_path: &Path,
    mods_dir: &Path,
    overwrite: bool,
    ledger: Option<&Value>,
) -> Result<InstallOutcome, String> {
    let (info, root) = archive_info(zip_path, mods_dir)?;
    let target = mods_dir.join(&info.mod_id);
    if target.exists() && !overwrite {
        return Err(be_err(
            "vcmi_mod_already_exists",
            &format!("{}（{}）", info.mod_id, target.display()),
        ));
    }

    fs::create_dir_all(mods_dir).map_err(|e| be_err("vcmi_mods_dir_create_failed", &format!("{}", e)))?;
    let temp = mods_dir.join(format!(".fv-install-{}", info.mod_id));
    let _ = fs::remove_dir_all(&temp);

    let file = fs::File::open(zip_path).map_err(|e| be_err("vcmi_zip_open_failed", &format!("{}", e)))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|_| be_err("vcmi_zip_invalid", ""))?;

    let mut installed = 0usize;
    let outcome_files = extract_into(&mut archive, &root, &temp, &mut installed);
    if let Err(e) = outcome_files {
        let _ = fs::remove_dir_all(&temp);
        return Err(e);
    }
    if let Some(led) = ledger {
        let text = serde_json::to_string_pretty(led).map_err(|e| be_err("realm_ledger_serialize_failed", &format!("{}", e)))?;
        if let Err(e) = fs::write(temp.join(LEDGER_FILE), text) {
            let _ = fs::remove_dir_all(&temp);
            return Err(be_err("vcmi_install_ledger_write_failed", &format!("{}", e)));
        }
    }

    // 就位：旧目录先改名保留（可回滚），再把临时目录改名到目标
    let old = mods_dir.join(format!(".fv-old-{}", info.mod_id));
    let _ = fs::remove_dir_all(&old);
    let had_target = target.exists();
    if had_target {
        fs::rename(&target, &old).map_err(|e| {
            let _ = fs::remove_dir_all(&temp);
            format!("暂存旧版本失败（可能有进程占用）: {e}")
        })?;
    }
    match fs::rename(&temp, &target) {
        Ok(()) => {
            if had_target {
                // 联接只删链接自身，绝不递归（否则毁掉联接目标）
                if is_link(&old) {
                    let _ = fs::remove_dir(&old);
                } else {
                    let _ = fs::remove_dir_all(&old);
                }
            }
            Ok(InstallOutcome {
                mod_id: info.mod_id,
                dest: target,
                installed_files: installed,
                replaced: had_target,
            })
        }
        Err(e) => {
            // 回滚：把旧版本改回去，清理临时目录
            if had_target {
                let _ = fs::rename(&old, &target);
            }
            let _ = fs::remove_dir_all(&temp);
            Err(be_err("vcmi_install_failed", &format!("{}", e)))
        }
    }
}

/// 打开 zip、识别 mod 根、读取 `mod.json`。返回探查信息与 mod 根前缀（`""` = 根目录即 mod）。
fn archive_info(zip_path: &Path, mods_dir: &Path) -> Result<(ZipInspection, String), String> {
    let file = fs::File::open(zip_path).map_err(|e| be_err("vcmi_zip_open_failed", &format!("{}", e)))?;
    let mut archive = zip::ZipArchive::new(file).map_err(|_| be_err("vcmi_zip_invalid", ""))?;

    let mut names: Vec<String> = Vec::with_capacity(archive.len());
    for i in 0..archive.len() {
        if let Ok(e) = archive.by_index(i) {
            names.push(e.name().replace('\\', "/"));
        }
    }
    let root = detect_mod_root(&names).ok_or_else(|| {
        "压缩包里找不到 mod.json（可能不是 VCMI 模组包，或需要先解压一层目录）".to_string()
    })?;

    let mod_json_rel = if root.is_empty() {
        "mod.json".to_string()
    } else {
        format!("{root}/mod.json")
    };
    let mod_json = read_entry_json(&mut archive, &mod_json_rel)?;

    let candidate = candidate_id(&root, zip_path);
    let mod_id = sanitize_mod_id(&candidate);
    let target = mods_dir.join(&mod_id);
    let entry_count = count_files(&names, &root);

    Ok((
        ZipInspection {
            mod_id,
            name: str_field(&mod_json, "name"),
            version: str_field(&mod_json, "version"),
            mod_type: str_field(&mod_json, "modType"),
            entry_count,
            exists: target.exists(),
            target,
        },
        root,
    ))
}

/// 识别 mod 根：`""`（根即 mod，官方不支持）→ `X` → `X/Y`。
fn detect_mod_root(names: &[String]) -> Option<String> {
    let is_mod_json = |s: &str| s.eq_ignore_ascii_case("mod.json");
    if names.iter().any(|n| is_mod_json(n)) {
        return Some(String::new());
    }
    for level in 0..=1usize {
        for n in names {
            let parts: Vec<&str> = n.split('/').collect();
            if parts.len() == level + 2 && is_mod_json(parts[level + 1]) {
                return Some(parts[..=level].join("/"));
            }
        }
    }
    None
}

/// mod id 候选：包内 mod 目录名（根目录包用 zip 文件名），再去掉尾随版本号。
fn candidate_id(root: &str, zip_path: &Path) -> String {
    let raw = if root.is_empty() {
        zip_path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default()
    } else {
        root.rsplit('/').next().unwrap_or(root).to_string()
    };
    strip_version_suffix(&raw)
}

/// 去掉尾随版本号：`hota-1.7.3` / `hota_v1.7.3` / `hota 1.7.3` → `hota`。
///
/// 保守规则：尾部必须是**至少两段**的纯数字点串（含 `.`），且前面是分隔符（`- _ 空格`）或
/// 可选的 `v` —— 这样 `forge2k`、`my-mod-2` 这类正常 id 不会被误剪。
fn strip_version_suffix(s: &str) -> String {
    let t = s.trim();
    let chars: Vec<char> = t.chars().collect();
    let n = chars.len();
    let mut i = n;
    while i > 0 && (chars[i - 1].is_ascii_digit() || chars[i - 1] == '.') {
        i -= 1;
    }
    if i == n {
        return t.to_string();
    }
    let tail: String = chars[i..].iter().collect();
    let tail = tail.trim_end_matches('.');
    let tail_ok = tail.contains('.')
        && tail
            .split('.')
            .all(|x| !x.is_empty() && x.chars().all(|c| c.is_ascii_digit()));
    if !tail_ok {
        return t.to_string();
    }
    let mut j = i;
    if j > 0 && matches!(chars[j - 1], '-' | '_' | ' ') {
        j -= 1;
    }
    if j > 0 && (chars[j - 1] == 'v' || chars[j - 1] == 'V') {
        let before_ok = j == 1 || matches!(chars[j - 2], '-' | '_' | ' ');
        if before_ok {
            j -= 1;
            if j > 0 && matches!(chars[j - 1], '-' | '_' | ' ') {
                j -= 1;
            }
        }
    }
    let out: String = chars[..j].iter().collect();
    let out = out.trim().to_string();
    if out.is_empty() {
        t.to_string()
    } else {
        out
    }
}

/// 净化 mod id：去掉路径分隔符等非法字符，防止 `../` 之类的目录穿越。
fn sanitize_mod_id(raw: &str) -> String {
    let s: String = raw
        .chars()
        .filter(|c| !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        .filter(|c| !c.is_control())
        .collect();
    let s = s.trim().trim_matches('.').trim().to_string();
    if s.is_empty() || s == ".." {
        "unnamed-mod".to_string()
    } else {
        s
    }
}

/// 解压 `root` 内的文件到 `dest`（剥离前缀），返回写入文件数。
///
/// 安全约束：条目路径不得为绝对路径 / 含 `..`；文件名若含乱码替换字符（U+FFFD，
/// 说明 zip 用了本库无法解码的编码）→ 直接失败，避免写出乱码目录。
fn extract_into(
    archive: &mut zip::ZipArchive<fs::File>,
    root: &str,
    dest: &Path,
    counter: &mut usize,
) -> Result<(), String> {
    let prefix = if root.is_empty() {
        String::new()
    } else {
        format!("{root}/")
    };
    for i in 0..archive.len() {
        let mut entry = match archive.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = entry.name().replace('\\', "/");
        let Some(rel) = name.strip_prefix(prefix.as_str()) else {
            continue; // mod 根之外的条目（readme 等）忽略
        };
        if rel.is_empty() {
            continue;
        }
        if !rel_ok(rel) {
            return Err(be_err("vcmi_zip_unsafe_path", &format!("{}", name)));
        }
        if rel.contains('\u{FFFD}') {
            return Err(be_err("vcmi_zip_filename_encoding", ""));
        }
        let out_path = dest.join(rel.trim_end_matches('/'));
        if entry.is_dir() {
            fs::create_dir_all(&out_path).map_err(|e| be_err("cmd_mkdir_failed", &format!("{}", e)))?;
            continue;
        }
        if let Some(parent) = out_path.parent() {
            fs::create_dir_all(parent).map_err(|e| be_err("cmd_mkdir_failed", &format!("{}", e)))?;
        }
        let mut buf = Vec::new();
        entry
            .read_to_end(&mut buf)
            .map_err(|e| be_err("vcmi_zip_entry_read_failed", &format!("{}", e)))?;
        let mut f = fs::File::create(&out_path).map_err(|e| be_err("cmd_write_failed", &format!("{}", e)))?;
        f.write_all(&buf).map_err(|e| be_err("cmd_write_failed", &format!("{}", e)))?;
        *counter += 1;
    }
    Ok(())
}

/// 相对路径安全校验：非空、非绝对、无盘符、无 `.` / `..` 段。
fn rel_ok(rel: &str) -> bool {
    let trimmed = rel.trim_end_matches('/');
    if trimmed.is_empty() || trimmed.starts_with('/') || trimmed.contains(':') {
        return false;
    }
    !trimmed.split('/').any(|s| s.is_empty() || s == "." || s == "..")
}

/// 统计 `root` 内的文件条目数（不含目录）。
fn count_files(names: &[String], root: &str) -> usize {
    let prefix = if root.is_empty() {
        String::new()
    } else {
        format!("{root}/")
    };
    names
        .iter()
        .filter(|n| {
            n.strip_prefix(prefix.as_str())
                .map(|rel| !rel.is_empty() && !rel.ends_with('/'))
                .unwrap_or(false)
        })
        .count()
}

fn read_entry_json(
    archive: &mut zip::ZipArchive<fs::File>,
    entry_name: &str,
) -> Result<serde_json::Value, String> {
    let mut found = None;
    for i in 0..archive.len() {
        let mut entry = match archive.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.name().replace('\\', "/").eq_ignore_ascii_case(entry_name) {
            let mut buf = String::new();
            entry
                .read_to_string(&mut buf)
                .map_err(|e| be_err("vcmi_mod_json_read_failed", &format!("{}", e)))?;
            found = Some(buf);
            break;
        }
    }
    let text = found.ok_or_else(|| be_err("vcmi_zip_entry_missing", entry_name))?;
    crate::infra::json::parse_json_lenient(&text)
        .or_else(|_| serde_json::from_str::<serde_json::Value>(&text))
        .map_err(|e| be_err("vcmi_mod_json_parse_failed", &format!("{}", e)))
}

fn str_field(v: &serde_json::Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vcmi-fv-install-{tag}"));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn write_zip(path: &Path, entries: &[(&str, &str)]) {
        let f = fs::File::create(path).unwrap();
        let mut w = zip::ZipWriter::new(f);
        let opt: zip::write::FileOptions<'_, ()> =
            zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Stored);
        for (name, data) in entries {
            w.start_file(*name, opt).unwrap();
            w.write_all(data.as_bytes()).unwrap();
        }
        w.finish().unwrap();
    }

    const MOD_JSON: &str = r#"{"name":"Horn of the Abyss","version":"1.7.3","modType":"Expansion"}"#;

    #[test]
    fn strip_version_suffix_keeps_normal_ids() {
        assert_eq!(strip_version_suffix("hota-1.7.3"), "hota");
        assert_eq!(strip_version_suffix("hota_v1.7.3"), "hota");
        assert_eq!(strip_version_suffix("tides of war 1.7"), "tides of war");
        assert_eq!(strip_version_suffix("hota"), "hota");
        assert_eq!(strip_version_suffix("forge2k"), "forge2k");
        // 单段数字（无点）不视为版本号，避免误剪
        assert_eq!(strip_version_suffix("my-mod-2"), "my-mod-2");
    }

    #[test]
    fn inspect_detects_root_level_and_nested_mods() {
        let d = tmp_dir("inspect");
        let zip_path = d.join("hota-1.7.3.zip");
        write_zip(
            &zip_path,
            &[
                ("hota-1.7.3/mod.json", MOD_JSON),
                ("hota-1.7.3/config/creatures.json", "{}"),
                ("readme.txt", "hi"),
            ],
        );
        let info = inspect(&zip_path, &d).unwrap();
        assert_eq!(info.mod_id, "hota", "应取包内目录名并去掉版本号");
        assert_eq!(info.name, "Horn of the Abyss");
        assert_eq!(info.version, "1.7.3");
        assert_eq!(info.mod_type, "Expansion");
        assert_eq!(info.entry_count, 2, "readme 在 mod 根之外，不计入");

        // 根目录即 mod（官方 detectModArchive 不支持，我们支持）：id 取 zip 文件名
        let zip2 = d.join("my-town_v2.0.zip");
        write_zip(&zip2, &[("mod.json", MOD_JSON), ("config/x.json", "{}")]);
        let info2 = inspect(&zip2, &d).unwrap();
        assert_eq!(info2.mod_id, "my-town");
        assert_eq!(info2.entry_count, 2);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn inspect_rejects_archive_without_mod_json() {
        let d = tmp_dir("nomodjson");
        let zip_path = d.join("broken.zip");
        write_zip(&zip_path, &[("docs/readme.txt", "x")]);
        let err = inspect(&zip_path, &d).unwrap_err();
        assert!(err.contains("mod.json"), "错误应说明缺 mod.json：{err}");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn install_extracts_only_mod_root_and_strips_prefix() {
        let d = tmp_dir("install");
        let mods = d.join("Mods");
        let zip_path = d.join("ark-1.0.0.zip");
        write_zip(
            &zip_path,
            &[
                ("ark-1.0.0/mod.json", MOD_JSON),
                ("ark-1.0.0/config/a.json", "{}"),
                ("ark-1.0.0/Content/data.bin", "binary"),
                ("readme.txt", "should be ignored"),
            ],
        );
        let out = install(&zip_path, &mods, false).unwrap();
        assert_eq!(out.mod_id, "ark");
        assert_eq!(out.installed_files, 3);
        assert!(!out.replaced);
        assert!(mods.join("ark/mod.json").is_file());
        assert!(mods.join("ark/config/a.json").is_file());
        assert!(!mods.join("readme.txt").exists(), "mod 根之外的条目不得落地");
        assert!(!mods.join(".fv-install-ark").exists(), "临时目录应清理");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn install_requires_overwrite_and_keeps_old_on_failure() {
        let d = tmp_dir("overwrite");
        let mods = d.join("Mods");
        let zip_path = d.join("m-1.0.0.zip");
        write_zip(
            &zip_path,
            &[("m-1.0.0/mod.json", MOD_JSON), ("m-1.0.0/config/a.json", "{}")],
        );
        install(&zip_path, &mods, false).unwrap();
        let marker = mods.join("m/marker.txt");
        fs::write(&marker, "old").unwrap();

        // 未授权覆盖 → 报错且旧内容原样保留。
        // 错误文本为结构化码（`{"code","arg"}`，见 `infra::berror`），断言码而非中文文案。
        let err = install(&zip_path, &mods, false).unwrap_err();
        let v: serde_json::Value = serde_json::from_str(&err).expect("应为结构化错误 JSON");
        assert_eq!(v["code"], "vcmi_mod_already_exists", "{err}");
        assert!(v["arg"].as_str().unwrap_or("").contains("m（"), "应带模组 id 与目标路径：{err}");
        assert!(marker.is_file(), "旧安装不得被破坏");

        // 授权覆盖 → 成功，旧文件（marker）随之被替换掉
        let out = install(&zip_path, &mods, true).unwrap();
        assert!(out.replaced);
        assert!(mods.join("m/mod.json").is_file());
        assert!(!marker.exists(), "覆盖后应是新解压的内容");
        assert!(!mods.join(".fv-old-m").exists(), "旧目录应清理");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn install_rejects_path_traversal() {
        let d = tmp_dir("traversal");
        let mods = d.join("Mods");
        let zip_path = d.join("evil.zip");
        write_zip(
            &zip_path,
            &[("evil/mod.json", MOD_JSON), ("evil/../../escape.txt", "boom")],
        );
        let err = install(&zip_path, &mods, false).unwrap_err();
        let v: serde_json::Value = serde_json::from_str(&err).expect("应为结构化错误 JSON");
        assert_eq!(v["code"], "vcmi_zip_unsafe_path", "{err}");
        assert!(
            v["arg"].as_str().unwrap_or("").contains("escape.txt"),
            "应带被拒绝的条目名：{err}"
        );
        assert!(!d.join("escape.txt").exists());
        assert!(!mods.join("evil").exists(), "失败后不得留下半截安装");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn sanitize_mod_id_blocks_escape() {
        assert_eq!(sanitize_mod_id("../etc"), "etc");
        assert_eq!(sanitize_mod_id(".."), "unnamed-mod");
        assert_eq!(sanitize_mod_id("a/b\\c"), "abc");
        assert_eq!(sanitize_mod_id("  "), "unnamed-mod");
    }

    fn walk_files(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(rd) = fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                walk_files(&p, out);
            } else {
                out.push(p);
            }
        }
    }

    /// 端到端：把一个**真实 mod 目录**打包成 zip 再安装（默认 ignore，需 `references/`）。
    /// 覆盖大包、深层路径与包内非 ASCII 文件名（验证解压链路不写乱码、不中断）。
    #[test]
    #[ignore = "需要 references/ 真实 mod，仅人工验证"]
    fn installs_real_mod_package() {
        let src_mod = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("references")
            .join("Heroes3-VCMI")
            .join("Mods")
            .join("hota");
        if !src_mod.is_dir() {
            eprintln!("跳过：{} 不存在", src_mod.display());
            return;
        }
        let d = tmp_dir("realpkg");
        let zip_path = d.join("hota-1.7.3.zip");
        {
            let mut files = Vec::new();
            walk_files(&src_mod, &mut files);
            let f = fs::File::create(&zip_path).unwrap();
            let mut w = zip::ZipWriter::new(f);
            let opt: zip::write::FileOptions<'_, ()> = zip::write::FileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated);
            for p in &files {
                let rel = p.strip_prefix(&src_mod).unwrap().to_string_lossy().replace('\\', "/");
                w.start_file(format!("hota-1.7.3/{rel}"), opt).unwrap();
                w.write_all(&fs::read(p).unwrap()).unwrap();
            }
            w.finish().unwrap();
            eprintln!("打包 {} 个文件 → {}", files.len(), zip_path.display());
        }

        let mods_dir = d.join("Mods");
        let info = inspect(&zip_path, &mods_dir).unwrap();
        assert_eq!(info.mod_id, "hota");
        assert!(info.entry_count > 100, "真实 mod 条目数：{}", info.entry_count);

        let out = install(&zip_path, &mods_dir, false).unwrap();
        assert_eq!(out.mod_id, "hota");
        assert_eq!(out.installed_files, info.entry_count);
        assert!(mods_dir.join("hota/mod.json").is_file());
        assert!(mods_dir.join("hota").join("content.zip").is_file() || mods_dir.join("hota/Content").is_dir());
        assert!(!mods_dir.join(".fv-install-hota").exists());
        let _ = fs::remove_dir_all(&d);
    }
}
