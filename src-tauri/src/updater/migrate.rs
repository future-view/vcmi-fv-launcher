//! 版本迁移骨架：**启动时**检查「上次运行版本 → 当前版本」，执行必要的迁移动作。
//!
//! ## 设计原则（重要，新增迁移前必读）
//!
//! 1. **能靠 bump 版本常量解决的，绝不写进这里。**
//!    项目已有三套声明式机制：`WIKI_CACHE_VERSION`（百科缓存）、`HD_SCHEME_VERSION`（高清变体方案）、
//!    `ASSETS_VERSION`（素材阶段产物）。格式一变就改那个数字，代码会自动检测并重建 ——
//!    零维护、不会堆积、不关心用户从哪个版本升级。
//!    "某版本导致缓存失效，需要清缓存重建" **正是它们的职责**，不是本模块的。
//!
//! 2. 本模块只处理**版本号表达不了**的非常规动作：删废弃文件 / 目录搬家 / 配置字段改名等。
//!    设计成**断点 + 动作枚举**（不是"每版本一个函数"），一年通常也就 1~2 条。
//!
//! 3. **生命周期由 `minSupported` 界定**：迁移只需覆盖 `[minSupported, current)`。
//!    低于 [`MIGRATION_FLOOR`] 的旧版本**不重放历史迁移**，直接清空派生数据（反正官方已不支持，
//!    重建比逐条重放更可靠）；随着门槛前移，旧断点即可删除。
//!
//! 4. 迁移必须**幂等、可重入**：中途失败下次启动继续；任何失败都只记日志，**绝不阻断启动**。

use std::cmp::Ordering;
use std::fs;
use std::path::PathBuf;

use serde_json::{json, Value};

use super::manifest::{self, semver_cmp};
use crate::vcmi::paths::VcmiPaths;

/// 支持门槛：低于它的版本不重放历史迁移，直接清空派生数据。
///
/// ⚠️ 这是**编译期常量**（不读远端清单）：迁移逻辑不能依赖网络。与清单的 `minSupported`
/// 由人工保持一致 —— 清单一侧负责"强制用户更新"，本常量负责"更新后怎么迁移"。
const MIGRATION_FLOOR: &str = "0.1.0";

/// 非常规迁移动作（**枚举而非闭包**：可枚举、可单测、可审计、不会塞进业务代码）
#[allow(dead_code)] // 断点表当前为空；新增第一条时删掉本行
enum Break {
    /// 清空所有**可重建**的派生数据（百科缓存 / 高清变体），保留用户配置与领域
    ClearDerived,
}

/// 迁移断点表：`lastRunVersion < 断点 <= current` 时执行。
///
/// 新增一条即 `("0.3.0", Break::ClearDerived)`；⚠️ 先读模块头第 1 条 ——
/// 能靠 `WIKI_CACHE_VERSION` 等常量解决的就不要加在这里。
#[allow(dead_code)] // 当前为空表（见上）
const BREAKS: &[(&str, Break)] = &[];

/// 启动时调用：执行版本迁移并记录本次运行版本。
///
/// 幂等：`lastRunVersion == current` 时直接返回（同一版本重入不做任何事）。
pub fn run() {
    let last = state_str("lastRunVersion");
    let cur = manifest::CURRENT_VERSION;
    if last.as_deref() == Some(cur) {
        return;
    }

    match last.as_deref() {
        Some(v) => tracing::info!(target: "updater", "版本迁移：{v} → {cur}"),
        None => tracing::info!(target: "updater", "首次记录运行版本：{cur}（不执行迁移）"),
    }

    if let Some(v) = last.as_deref() {
        if is_below_floor(v) {
            tracing::warn!(
                target: "updater",
                "版本 {v} 低于支持门槛 {MIGRATION_FLOOR}：不重放历史迁移，改为清空派生数据后重建"
            );
            clear_derived();
        } else {
            for (target, br) in BREAKS {
                if should_run(v, cur, target) {
                    tracing::info!(target: "updater", "执行迁移断点 {target}");
                    apply_break(br);
                }
            }
        }
    }

    set_state_str("lastRunVersion", cur);
}

/// `last < target <= current`：用户跨过了该断点且尚未在其之后运行过
fn should_run(last: &str, cur: &str, target: &str) -> bool {
    matches!(semver_cmp(last, target), Some(Ordering::Less))
        && !matches!(semver_cmp(cur, target), Some(Ordering::Less))
}

fn is_below_floor(last: &str) -> bool {
    matches!(semver_cmp(last, MIGRATION_FLOOR), Some(Ordering::Less))
}

fn apply_break(br: &Break) {
    match br {
        Break::ClearDerived => clear_derived(),
    }
}

/// 清空**可重建**的派生数据：百科缓存 + 高清变体（都在 `<data>/mods`）。
///
/// ⚠️ 只删这一处：用户配置（`launcher.json`）、领域（`realms/`）、更新缓存（`cache/`）
/// 都不在 `mods/` 下，**绝不触碰**。
fn clear_derived() {
    let Some(data) = VcmiPaths::launcher_data_dir() else {
        return;
    };
    let mods = data.join("mods");
    if !mods.exists() {
        return;
    }
    match fs::remove_dir_all(&mods) {
        Ok(()) => {
            tracing::warn!(target: "updater", "已清空派生数据（下次启动将重建）：{}", mods.display())
        }
        Err(e) => tracing::error!(target: "updater", "清空派生数据失败：{e}"),
    }
}

/* ----------------------------- state.json ----------------------------- */
//
// `.vcmi-fv-launcher/state.json`：启动器的**运行时状态**（与用户配置 `launcher.json` 分开）。
// 目前承载两类字段：
// - `lastRunVersion`：版本迁移用（见上）
// - `appliedUpdateSha`：**已应用的清单条目 sha256** —— 供"纯配置更新包"去重
//   （这种包不改 exe 版本，若不记录会被反复提示更新）

fn state_path() -> Option<PathBuf> {
    VcmiPaths::launcher_data_dir().map(|d| d.join("state.json"))
}

fn read_state() -> Value {
    state_path()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}))
}

fn write_state(v: &Value) {
    let Some(p) = state_path() else { return };
    if let Some(parent) = p.parent() {
        let _ = fs::create_dir_all(parent);
    }
    if let Err(e) = fs::write(&p, serde_json::to_string_pretty(v).unwrap_or_default()) {
        tracing::warn!(target: "updater", "写入状态文件失败（可忽略）：{e}");
    }
}

/// 读状态字符串字段（缺失 / 空串 → None）
pub(crate) fn state_str(key: &str) -> Option<String> {
    read_state()
        .get(key)
        .and_then(|x| x.as_str())
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.to_string())
}

/// 写状态字符串字段（**读-改-写**，保留其它字段；失败只记日志）
pub(crate) fn set_state_str(key: &str, val: &str) {
    let mut v = read_state();
    if let Some(m) = v.as_object_mut() {
        m.insert(key.to_string(), json!(val));
        m.insert("updatedAt".to_string(), json!(now_secs()));
    }
    write_state(&v);
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 断点只在**跨越**时执行一次（跨过即执行；同版本重入不再执行；未来断点不执行）
    #[test]
    fn break_runs_only_when_crossing() {
        assert!(should_run("0.1.0", "0.3.0", "0.2.0"), "跨过断点应执行");
        assert!(!should_run("0.3.0", "0.3.0", "0.2.0"), "同版本重入不应执行");
        assert!(!should_run("0.3.0", "0.4.0", "0.5.0"), "断点在未来不应执行");
        assert!(
            !should_run("0.2.0", "0.3.0", "0.2.0"),
            "断点已在起点之前（含相等）不应执行"
        );
    }

    /// 门槛判定：低于 floor 走"清派生数据"兜底，而不是重放历史迁移
    #[test]
    fn floor_decision() {
        assert!(is_below_floor("0.0.9"));
        assert!(!is_below_floor(MIGRATION_FLOOR), "等于 floor 不算低于");
        assert!(!is_below_floor("9.9.9"));
    }
}
