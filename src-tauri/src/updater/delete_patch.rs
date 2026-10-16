//! 删除清单：更新包携带的 `.vcmi-fv-launcher/launcher.delete.json`，
//! 在启动早期删除**白名单内**的文件 / 目录 —— 补上"更新只能覆盖、不能删除"的能力缺口。
//!
//! ## 为什么需要
//!
//! 更新包只能**覆盖**目标路径，无法"删掉"废弃文件 / 旧领域 / 过期缓存。
//! ⚠️ 先确认：**"缓存失效需要重建"不要用本机制** —— 那应当直接 bump
//! `WIKI_CACHE_VERSION` / `HD_SCHEME_VERSION` / `ASSETS_VERSION`（见 `migrate.rs` 模块头）。
//! 本机制只处理版本号表达不了的**删除**需求。
//!
//! ## 契约
//!
//! ```json5
//! // .vcmi-fv-launcher/launcher.delete.json（发行方提供；路径相对 **exe 目录**）
//! { "paths": [
//!   ".vcmi-fv-launcher/realms/old-realm",
//!   ".vcmi-fv-launcher/mods/abandoned-mod",
//!   ".vcmi-fv-launcher/runtime/legacy-component"
//! ] }
//! ```
//!
//! ## 安全约束（逐条都有原因，勿放宽）
//!
//! - **白名单前缀**：只允许 `.vcmi-fv-launcher/{realms,mods,runtime}/` 与 `runtime/`；
//!   配置（`launcher.json` / `state.json`）、`cache/`、`downloads/`、`logs/` 一律拒绝。
//! - **不许删白名单根**：至少要 `前缀/<一层>`（避免一条 `realms` 清空全部领域）。
//! - ⚠️ **目录联接（junction）只能用 `remove_dir`**：领域目录内 `vcmi/Mods/<mod>` 常是
//!   指向游戏目录的联接，`remove_dir_all` 会连带删掉**联接目标的真实文件**（rules 04 铁律）。
//! - 路径含 `..` / 空段 → 拒绝。
//! - **单项跳过而非整包拒绝**：删除是破坏性操作，一条写错不该让其它合法项也不执行；全部记日志。
//! - 一次性：处理完**删除清单文件**（下次启动不重复执行）。
//! - 任何失败只记日志，**绝不阻断启动**。

use crate::infra::berror::be_err;
use std::fs;
use std::path::Path;

use crate::infra::json::parse_json_lenient;
use crate::vcmi::paths::VcmiPaths;

/// 删除清单在更新包中的相对路径（白名单单独放行）
pub const DELETE_REL: &str = ".vcmi-fv-launcher/launcher.delete.json";

/// 启动时调用：执行 `<exe>/.vcmi-fv-launcher/launcher.delete.json`（不存在则什么都不做）
pub fn apply() {
    let Some(exe_dir) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
    else {
        return;
    };
    let Some(data) = VcmiPaths::launcher_data_dir() else {
        return;
    };
    match apply_at(&exe_dir, &data.join("launcher.delete.json")) {
        Ok(done) if !done.is_empty() => tracing::warn!(
            target: "updater",
            "删除清单已执行（{} 项）：{}",
            done.len(),
            done.join("、")
        ),
        Ok(_) => {}
        Err(e) => tracing::warn!(target: "updater", "删除清单未执行（可忽略）：{e}"),
    }
}

/// 执行实现（可测）：`exe_dir` 为相对路径基准；返回实际删除的相对路径。
pub(crate) fn apply_at(exe_dir: &Path, list_path: &Path) -> Result<Vec<String>, String> {
    if !list_path.is_file() {
        return Ok(Vec::new());
    }
    let text = fs::read_to_string(list_path)
        .map_err(|e| be_err("upd_delete_manifest_read_failed", &format!("{}", e)))?;
    let v = parse_json_lenient(&text)
        .map_err(|e| be_err("upd_delete_manifest_invalid_json", &format!("{}", e)))?;
    let paths = v
        .get("paths")
        .and_then(|x| x.as_array())
        .ok_or_else(|| be_err("updater_delete_manifest_no_paths", ""))?;

    let mut done = Vec::new();
    for item in paths {
        let Some(raw) = item.as_str() else {
            tracing::warn!(target: "updater", "删除清单项不是字符串，已跳过：{item}");
            continue;
        };
        let rel = raw.trim().replace('\\', "/");
        let rel = rel.trim_start_matches("./").to_string();
        if !rel_ok(&rel) {
            tracing::warn!(target: "updater", "删除清单路径不在白名单内，已跳过：{rel}");
            continue;
        }
        let target = exe_dir.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        if !target.exists() && fs::symlink_metadata(&target).is_err() {
            continue; // 本来就不存在（联接需用 symlink_metadata 判定）
        }
        match remove_any(&target) {
            Ok(()) => done.push(rel),
            Err(e) => tracing::warn!(target: "updater", "删除失败：{rel} —— {e}"),
        }
    }

    // 一次性：无论是否有项被执行，清单都视为已处理
    let _ = fs::remove_file(list_path);
    Ok(done)
}

/// 允许删除的相对路径（白名单 + 不许删白名单根）
fn rel_ok(rel: &str) -> bool {
    if rel.split('/').any(|s| s.is_empty() || s == "..") {
        return false;
    }
    // 至少要 3 段：`.vcmi-fv-launcher/<域>/<一层>`（或 `runtime/<id>/<一层>`）
    let segs: Vec<&str> = rel.split('/').collect();
    if segs.len() < 3 {
        return false;
    }
    const ROOTS: [&str; 3] = [
        ".vcmi-fv-launcher/realms/",
        ".vcmi-fv-launcher/mods/",
        // 组件只认一个位置（见 `runtime::dir_of`）
        ".vcmi-fv-launcher/runtime/",
    ];
    ROOTS.iter().any(|p| rel.starts_with(p))
}

/// 删除文件 / 目录；**联接（reparse point）只删链接自身，绝不递归**
fn remove_any(p: &Path) -> std::io::Result<()> {
    let meta = fs::symlink_metadata(p)?;
    if meta.is_dir() {
        if is_reparse_point(p) {
            fs::remove_dir(p)
        } else {
            fs::remove_dir_all(p)
        }
    } else {
        fs::remove_file(p)
    }
}

/// 是否为重解析点（Windows 目录联接 / 符号链接 / 挂载点）
///
/// ⚠️ 不能用 `FileType::is_symlink()` —— 目录联接（junction）在 Rust 里**不算 symlink**，
/// 必须查 `FILE_ATTRIBUTE_REPARSE_POINT`（0x400），否则 `remove_dir_all` 会毁掉联接目标。
#[cfg(windows)]
fn is_reparse_point(p: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    fs::symlink_metadata(p)
        .map(|m| m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn is_reparse_point(p: &Path) -> bool {
    fs::symlink_metadata(p)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vcmi-delpatch-test-{name}"));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn write(p: &Path, s: &str) {
        fs::write(p, s).unwrap();
    }

    #[test]
    fn deletes_files_and_dirs_in_whitelist() {
        let d = tmp("ok");
        let data = d.join(".vcmi-fv-launcher");
        fs::create_dir_all(data.join("realms/old-realm")).unwrap();
        write(&data.join("realms/old-realm/realm.json"), "{}");
        fs::create_dir_all(data.join("mods/abandoned")).unwrap();
        write(&data.join("mods/abandoned/cache.json"), "{}");
        let list = data.join("launcher.delete.json");
        write(
            &list,
            r#"{"paths":[".vcmi-fv-launcher/realms/old-realm",".vcmi-fv-launcher/mods/abandoned/cache.json"]}"#,
        );

        let done = apply_at(&d, &list).unwrap();
        assert_eq!(done.len(), 2, "两项都应执行：{done:?}");
        assert!(!data.join("realms/old-realm").exists());
        assert!(!list.is_file(), "清单应被删除（一次性）");
        let _ = fs::remove_dir_all(&d);
    }

    /// 白名单：用户配置 / 状态 / 缓存 / 下载一律拒绝
    #[test]
    fn refuses_forbidden_paths() {
        let d = tmp("refuse");
        let data = d.join(".vcmi-fv-launcher");
        fs::create_dir_all(data.join("cache")).unwrap();
        write(&data.join("launcher.json"), "{}");
        write(&data.join("state.json"), "{}");
        write(&data.join("cache/x.json"), "{}");
        let list = data.join("launcher.delete.json");
        write(
            &list,
            r#"{"paths":[
                ".vcmi-fv-launcher/launcher.json",
                ".vcmi-fv-launcher/state.json",
                ".vcmi-fv-launcher/cache/x.json",
                ".vcmi-fv-launcher/downloads/a.zip",
                ".vcmi-fv-launcher/logs/a.log",
                "vcmi-fv-launcher.exe",
                "other.txt",
                ".vcmi-fv-launcher/realms",
                "../escape.json",
                ".vcmi-fv-launcher/realms/../../escape.json"
            ]}"#,
        );

        let done = apply_at(&d, &list).unwrap();
        assert!(done.is_empty(), "全部应被拒绝，实际：{done:?}");
        assert!(data.join("launcher.json").is_file());
        assert!(data.join("state.json").is_file());
        assert!(data.join("cache/x.json").is_file());
        let _ = fs::remove_dir_all(&d);
    }

    /// 单项非法不阻断其它项（破坏性操作应"能做的先做"）
    #[test]
    fn one_bad_item_does_not_block_others() {
        let d = tmp("mixed");
        let data = d.join(".vcmi-fv-launcher");
        fs::create_dir_all(data.join("mods/dead")).unwrap();
        write(&data.join("mods/dead/cache.json"), "{}");
        let list = data.join("launcher.delete.json");
        write(
            &list,
            r#"{"paths":[".vcmi-fv-launcher/launcher.json",".vcmi-fv-launcher/mods/dead"]}"#,
        );

        let done = apply_at(&d, &list).unwrap();
        assert_eq!(done, vec![".vcmi-fv-launcher/mods/dead".to_string()]);
        assert!(!data.join("mods/dead").exists());
        let _ = fs::remove_dir_all(&d);
    }

    /// 缺 paths / 非对象 → 报错且不删任何东西
    #[test]
    fn invalid_list_is_ignored() {
        let d = tmp("invalid");
        let data = d.join(".vcmi-fv-launcher");
        fs::create_dir_all(&data).unwrap();
        let list = data.join("launcher.delete.json");
        write(&list, r#"{"nope":1}"#);
        assert!(apply_at(&d, &list).is_err());
        let _ = fs::remove_dir_all(&d);
    }
}
