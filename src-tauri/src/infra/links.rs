//! 目录联接（junction）的统一实现（设计 §10.6）。
//!
//! 同时服务于「借用模组」（§3.4）与「Maps 阀门」（§8.3），避免两处各写一遍
//! 高危的联接创建 / 删除代码。核心安全约束：**删除联接只能用 `remove_dir`，
//! 绝不能 `remove_dir_all`，否则会误删联接目标（真实目录）里的文件**。

use crate::infra::berror::be_err;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 判断路径是否为重解析点（Windows 目录联接 / 符号链接）。
#[cfg(windows)]
pub fn is_link(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    if let Ok(meta) = fs::symlink_metadata(path) {
        (meta.file_attributes() & 0x400) != 0 // FILE_ATTRIBUTE_REPARSE_POINT
    } else {
        false
    }
}
#[cfg(not(windows))]
pub fn is_link(path: &Path) -> bool {
    fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
}

/// 幂等创建目录联接（已存在且指向正确目标则跳过；否则先安全移除再重建）。
/// `link` / `target` 均应为绝对路径。
///
/// ⚠️ **判存必须用 `symlink_metadata`**：悬空联接（目标已删除）的 `exists()` 为 false，
/// 但目录项仍然存在——若在此时直接 `mklink` 会报「已存在」，悬空链接永远无法自愈。
///
/// - Windows：目录联接（junction，`mklink /J`，免管理员权限）。
/// - 非 Windows：符号链接（`symlink`，VCMI 同样会跟随联接 / 软链）。
pub fn ensure_link(link: &Path, target: &Path) -> Result<(), String> {
    if fs::symlink_metadata(link).is_ok() {
        if is_link(link) {
            remove_link(link)?;
        } else {
            // 陈旧的真实目录占位，强制清除
            fs::remove_dir_all(link).map_err(|e| be_err("infra_stale_dir_clear_failed", &format!("{}", e)))?;
        }
    }
    if let Some(parent) = link.parent() {
        fs::create_dir_all(parent).map_err(|e| be_err("cmd_mkdir_failed", &format!("{}", e)))?;
    }
    ensure_dir_link(link, target)
}

#[cfg(windows)]
fn ensure_dir_link(link: &Path, target: &Path) -> Result<(), String> {
    let status = Command::new("cmd")
        .arg("/C")
        .arg("mklink")
        .arg("/J")
        .arg(safe_win_path(link))
        .arg(safe_win_path(target))
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| be_err("infra_junction_create_failed", &format!("{}", e)))?;
    if !status.success() {
        return Err(be_err("links_create_failed", ""));
    }
    Ok(())
}

#[cfg(not(windows))]
fn ensure_dir_link(link: &Path, target: &Path) -> Result<(), String> {
    std::os::unix::fs::symlink(target, link)
        .map_err(|e| be_err("infra_symlink_create_failed", &format!("{}", e)))
}

/// 安全删除目录联接：仅当为重解析点时删除；真实目录不会被误删。
pub fn remove_link(link: &Path) -> Result<(), String> {
    if !is_link(link) {
        return Ok(());
    }
    fs::remove_dir(link).map_err(|e| be_err("infra_junction_remove_failed", &format!("{}", e)))
}

/// 把路径转成 Windows `mklink` / `cmd` 可安全接受的字符串。
///
/// 修复（增强版 + 领域切换报 `无效开关 - "userdata"` 的根因）：若路径是
/// 「无盘符的盘根相对路径」（如 `/userdata`，`Component::RootDir` 开头），直接传给
/// `mklink` 会被 `cmd` 把前导 `/` 当成命令开关，进而报 `无效开关 - "userdata"`。
/// 这里用启动器 exe 所在盘符补全为 `C:\userdata` 之类，既消除开关歧义，也保证 VCMI 按预期加载。
/// 用 `current_exe()`（必然带盘符）而非进程 CWD——GUI 进程 CWD 可能不可达，会导致补盘失败、原样透传。
#[cfg(windows)]
pub(crate) fn safe_win_path(path: &Path) -> String {
    let needs_drive = path.is_absolute()
        && matches!(path.components().next(), Some(Component::RootDir));
    if needs_drive {
        if let Some(root) = current_drive_root() {
            return root
                .join(path.strip_prefix("/").unwrap_or(path))
                .to_string_lossy()
                .to_string();
        }
    }
    path.to_string_lossy().to_string()
}

/// 启动器 exe 所在盘的根（如 `D:\`）——用 `current_exe()`，比 `current_dir()` 可靠。
#[cfg(windows)]
fn current_drive_root() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .ancestors()
        .find(|a| a.parent().is_none())
        .map(|a| a.to_path_buf())
}

/// 建立文件级链接（配置文件共享阀门用）：硬链接优先（同卷、免权限）→
/// 符号链接（跨卷 / 硬链接失败回退）→ 复制快照（最终降级，非实时但保证可用）。
/// `link` / `target` 均应为绝对路径；`target` 必须存在（调用方负责确保全局源存在）。
/// **安全红线**：删除链接须用 `fs::remove_file`，绝不能用 `remove_dir` / `remove_dir_all`，
/// 否则会破坏全局源文件。
pub fn create_file_link(link: &Path, target: &Path) -> Result<(), String> {
    if let Some(parent) = link.parent() {
        fs::create_dir_all(parent).map_err(|e| be_err("cmd_mkdir_failed", &format!("{}", e)))?;
    }
    ensure_file_link(link, target)
}

#[cfg(windows)]
fn ensure_file_link(link: &Path, target: &Path) -> Result<(), String> {
    // 1) 硬链接（同卷、无需管理员）
    let hl = Command::new("cmd")
        .arg("/C")
        .arg("mklink")
        .arg("/H")
        .arg(safe_win_path(link))
        .arg(safe_win_path(target))
        .creation_flags(CREATE_NO_WINDOW)
        .status();
    if let Ok(s) = hl {
        if s.success() {
            return Ok(());
        }
    }
    // 2) 符号链接（跨卷 / 硬链接失败回退，可能需要管理员）
    let sl = Command::new("cmd")
        .arg("/C")
        .arg("mklink")
        .arg(safe_win_path(link))
        .arg(safe_win_path(target))
        .creation_flags(CREATE_NO_WINDOW)
        .status();
    if let Ok(s) = sl {
        if s.success() {
            return Ok(());
        }
    }
    // 3) 复制快照（降级，非实时共享但保证可用）
    fs::copy(target, link)
        .map(|_| ())
        .map_err(|e| be_err("infra_file_link_fallback_copy", &format!("{}", e)))
}

#[cfg(not(windows))]
fn ensure_file_link(link: &Path, target: &Path) -> Result<(), String> {
    // 1) 硬链接（同卷）
    if std::fs::hard_link(target, link).is_ok() {
        return Ok(());
    }
    // 2) 符号链接（跨卷 / 硬链接失败回退）
    if std::os::unix::fs::symlink(target, link).is_ok() {
        return Ok(());
    }
    // 3) 复制快照（降级，非实时共享但保证可用）
    fs::copy(target, link)
        .map(|_| ())
        .map_err(|e| be_err("infra_file_link_fallback_copy", &format!("{}", e)))
}

/// 递归复制目录（用于「整合」模组快照：把来源目录复制到领域目录）。
pub fn copy_dir_all(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| be_err("cmd_mkdir_failed", &format!("{}", e)))?;
    let entries = fs::read_dir(src).map_err(|e| be_err("infra_src_dir_read_failed", &format!("{}", e)))?;
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let dst_path = dst.join(entry.file_name());
        let ft = entry
            .file_type()
            .map_err(|e| be_err("infra_file_type_read_failed", &format!("{}", e)))?;
        if ft.is_dir() {
            copy_dir_all(&path, &dst_path)?;
        } else {
            fs::copy(&path, &dst_path).map_err(|e| be_err("infra_file_copy_failed", &format!("{}", e)))?;
        }
    }
    Ok(())
}

/// 解析某个 mod id 的来源目录（先在全局用户 Mods 找，再在游戏目录 Mods 找）。
fn mod_source(id: &str, user_mods: &Path, game_mods: &Path) -> Option<PathBuf> {
    let u = user_mods.join(id);
    if u.is_dir() {
        return Some(u);
    }
    let g = game_mods.join(id);
    if g.is_dir() {
        return Some(g);
    }
    None
}

/// 将领域模组组合落地到 `{realm_vcmi_dir}/Mods`（§3.4 / realm-mods-management §8.1）：
/// - `borrowed` = **引用清单**：按来源解析——**user 源建目录联接**（领域隔离后 VCMI 读不到
///   用户目录，必须链接）；**game 源仅登记校验、不建链**（游戏模组目录天然可见），
///   并清理历史遗留的冗余/悬空链接（自愈）。
/// - `bundled`：把来源目录复制到 `realm/Mods/<id>`（随领域分发）。
/// - `pinned`：**领域私有副本**（维护者在领域沙箱内从仓库更新得到，见
///   `docs/features/mods-repository.zh-CN.md` §4.4）：不参与缺失校验、
///   **永不被清理**（试验现场不得被激活流程抹掉）；若同时出现在 borrowed/bundled
///   也一律跳过，防止联接/复制覆盖私有副本。
///
/// 行为：先清理不再被引用的条目（联接用 `remove_dir`、真实目录用 `remove_dir_all`），
/// 再建立借用联接、复制整合模组（已存在的真实目录跳过，避免重复大体积复制）。
/// 返回 `Err` 当任一被引用模组在来源中不存在（§9.2.2 缺失校验）。
pub fn apply_realm_mods(
    realm_vcmi_dir: &Path,
    borrowed: &[String],
    bundled: &[String],
    pinned: &[String],
    user_mods: &Path,
    game_mods: &Path,
) -> Result<(), String> {
    let realm_mods = realm_vcmi_dir.join("Mods");
    fs::create_dir_all(&realm_mods).map_err(|e| be_err("infra_realm_mods_dir_create_failed", &format!("{}", e)))?;

    // 计算有效引用集合（pinned 是领域私有副本，不参与联接/复制）
    let borrowed_set: std::collections::HashSet<String> = borrowed
        .iter()
        .cloned()
        .filter(|id| !pinned.contains(id))
        .collect();
    let bundled_set: std::collections::HashSet<String> = bundled
        .iter()
        .cloned()
        .filter(|id| !pinned.contains(id))
        .collect();

    let desired_names: std::collections::HashSet<String> = borrowed_set
        .iter()
        .cloned()
        .chain(bundled_set.iter().cloned())
        .chain(pinned.iter().cloned())
        .collect();

    // 缺失校验（pinned 不参与：私有副本活在领域目录里，全局没有来源是正常态）
    let mut missing = Vec::new();
    for id in desired_names.iter().filter(|id| !pinned.contains(id)) {
        if mod_source(id, user_mods, game_mods).is_none() {
            missing.push(id.clone());
        }
    }
    if !missing.is_empty() {
        return Err(be_err(
            "realm_mods_missing_in_installed",
            &missing.join("、"),
        ));
    }

    // 清理不再被引用的条目（pinned 永不参与清理：领域沙箱的试验现场必须保留）
    if realm_mods.is_dir() {
        if let Ok(entries) = fs::read_dir(&realm_mods) {
            for e in entries.filter_map(|e| e.ok()) {
                let name = e.file_name().to_string_lossy().to_string();
                if !desired_names.contains(&name) && !pinned.contains(&name) {
                    let p = e.path();
                    if is_link(&p) {
                        let _ = remove_link(&p);
                    } else if p.is_dir() {
                        let _ = fs::remove_dir_all(&p);
                    }
                }
            }
        }
    }

    // 建立借用联接（来源感知）：user 源建目录联接；game 源天然可见不建链。
    for id in &borrowed_set {
        let src = mod_source(id, user_mods, game_mods).unwrap();
        let dst = realm_mods.join(id);
        // user 优先解析：来源不在 user 目录（即 game 目录）时为 game 源
        let src_is_game = !user_mods.join(id).is_dir();
        if src_is_game {
            // game 源：无需链接（游戏目录恒可见）；清理历史遗留的冗余/悬空链接
            if is_link(&dst) {
                remove_link(&dst)?;
            }
            continue;
        }
        // user 源：重建联接（幂等；悬空链接自愈）
        ensure_link(&dst, &src)?;
    }

    // 复制整合模组（已存在真实目录则跳过，避免重复大体积复制）
    for id in &bundled_set {
        let src = mod_source(id, user_mods, game_mods).unwrap();
        let dst = realm_mods.join(id);
        if is_link(&dst) {
            remove_link(&dst)?;
        }
        if dst.is_dir() && !is_link(&dst) {
            continue; // 已复制，跳过
        }
        copy_dir_all(&src, &dst)?;
    }

    Ok(())
}
