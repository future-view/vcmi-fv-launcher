//! 按 **VCMI modId** 在磁盘上解析模组目录（大小写不敏感 + 支持子模组嵌套）。
//!
//! ## 为什么需要单独一层
//!
//! VCMI 的 modId（`TModID`）**不等于**磁盘目录名，两者有两处系统性差异：
//!
//! 1. **大小写**：modId 恒为**小写**（`lib/modding/ModManager.cpp:54` 的
//!    `boost::algorithm::to_lower_copy`），而磁盘目录保留作者原本大小写
//!    （实测：`1-Knight-town`、`HuaXia`、`29- Jadelake-town`、`Forgotten Field---sw`）。
//! 2. **嵌套**：子模组 id 是 `a.b`，磁盘上是 `<root>/<a>/Mods/<b>`
//!    （`ModManager.cpp::getModDirectory` 把 `.` 替换成 `/MODS/`，
//!     而 `ModsState` 构造时正是沿 `MODS/` 逐层下钻拼出 `.` 分隔的 id）。
//!
//! 于是 `mods_dir.join(id)` 只在**大小写不敏感**的文件系统（Windows / macOS 默认）上
//! 恰好命中；在 Linux 上必然落空，而且**子模组在任何平台都找不到**。
//! 领域借用 / 整合（`realms/*`）、目录联接来源解析（`infra::links`）都依赖这一查找，
//! 落空会被当成"来源模组不存在"而**静默跳过**——不报错，只是功能悄悄失效。
//!
//! 放在 `infra` 而非 `vcmi`：`infra::links` 也要用，而依赖方向是 `vcmi → infra`。

use std::fs;
use std::path::{Path, PathBuf};

/// 在某个 Mods 根下按 modId 解析真实目录。
///
/// 快路径：逐段先精确 `join` 一次（Windows 命中即返回，**零额外开销**）；
/// 慢路径：遍历该层目录按小写名比对（只在未命中时发生）。
pub fn resolve_mod_dir(root: &Path, id: &str) -> Option<PathBuf> {
    if id.is_empty() {
        return None;
    }
    let mut cur = root.to_path_buf();
    for (i, seg) in id.split('.').enumerate() {
        // 子模组的中间层固定是 `Mods`（VCMI 的 `MODS/`，大小写容错）
        if i > 0 {
            cur = find_child_ci(&cur, "Mods")?;
        }
        cur = find_child_ci(&cur, seg)?;
    }
    Some(cur)
}

/// 在 `dir` 下找一个名为 `name` 的子**目录**（大小写不敏感）。
fn find_child_ci(dir: &Path, name: &str) -> Option<PathBuf> {
    let direct = dir.join(name);
    if direct.is_dir() {
        return Some(direct);
    }
    let want = name.to_lowercase();
    for e in fs::read_dir(dir).ok()?.flatten() {
        if e.path().is_dir() && e.file_name().to_string_lossy().to_lowercase() == want {
            return Some(e.path());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 建一棵临时目录树（`<t>/HuaXia`、`<t>/66expfixmod/Mods/New Pavilion`）。
    #[test]
    fn 大小写不敏感与嵌套解析() {
        let t = std::env::temp_dir().join("vcmi_fv_moddir_test");
        let _ = fs::remove_dir_all(&t);
        fs::create_dir_all(t.join("HuaXia")).unwrap();
        fs::create_dir_all(t.join("66expfixmod").join("Mods").join("New Pavilion")).unwrap();

        // 顶层：modId 小写，磁盘首字母大写
        assert!(resolve_mod_dir(&t, "huaxia").is_some());
        assert!(resolve_mod_dir(&t, "HuaXia").is_some());
        // 嵌套：点号 → `<父>/Mods/<子>`（且中间层大小写容错）
        assert!(resolve_mod_dir(&t, "66expfixmod.new pavilion").is_some());
        // 不存在
        assert!(resolve_mod_dir(&t, "nope").is_none());
        assert!(resolve_mod_dir(&t, "").is_none());

        let _ = fs::remove_dir_all(&t);
    }
}
