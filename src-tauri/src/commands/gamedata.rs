//! 游戏数据完整性体检：`gamedata_health_check`。
//!
//! 只做 `State<AppPaths>` 解包，判定逻辑在 `vcmi::health` / `vcmi::h3data`
//! （可单测、无 Tauri 依赖）。

use serde_json::{json, Value};
use tauri::State;

use crate::state::AppPaths;

/// 游戏数据完整性体检（只读）：
/// 返回 `{ ok, roots, items: [{ id, level, ok, detail }], h3Data, dataInstallCandidates }`。
///
/// `id` 供前端 i18n 取文案（`data.health.items.<id>`），`level` ∈ `fatal|warn`。
///
/// `game_dir` / `user_data_dir` 为**可选覆盖**（首启向导用：目录还没保存进 launcher.json，
/// `State<AppPaths>` 里没有有效路径）。不传 = 完全沿用已保存配置（主界面「游戏数据」抽屉
/// 的行为与扩展前逐字节一致）。
///
/// - `h3Data`：原版数据闸门（官方口径，见 `vcmi::h3data`），向导据此禁用保存按钮；
/// - `dataInstallCandidates`：注册表里发现的本机原版安装路径（Windows；用于预填数据目录）。
///   仅在 `with_candidates = Some(true)` 时查询 —— `reg query` ×4 约百毫秒，
///   常态状态探测不必付这个开销（确认缺失时前端会补一次带候选的调用）。
///
/// ⚠️ **必须 async + `spawn_blocking`**：LOD 头读取与（可选的）`reg query` ×4 都是
/// 磁盘 / 进程 IO。同步命令跑在**主线程**上（Tauri 的 `ExecutionContext::Blocking`），
/// 实测首次点第 ③ 步「选择」会卡住几秒才弹出文件夹对话框——对话框要在主线程创建，
/// 被同时进行的体检（尤其进入步骤时补查注册表候选的那次）顶住了。
/// 2026-10-18 用户实测反馈后改为线程池执行。
#[tauri::command]
pub async fn gamedata_health_check(
    state: State<'_, AppPaths>,
    game_dir: Option<String>,
    user_data_dir: Option<String>,
    with_candidates: Option<bool>,
) -> Result<Value, String> {
    let has_override = game_dir.is_some() || user_data_dir.is_some();
    let want_candidates = with_candidates.unwrap_or(true);
    // ⚠️ MutexGuard 非 Send：先取快照，**不得跨 `.await` 持锁**
    let paths = {
        let guard = state.0.lock().expect("paths lock poisoned");
        guard.clone()
    };

    tauri::async_runtime::spawn_blocking(move || -> Result<Value, String> {
        if !has_override {
            // 无覆盖：走既有路径（VcmiPaths 自带的资源根推导）
            let mut out = crate::vcmi::health::health_check(&paths);
            append_h3(&mut out, &paths.game_dir, &paths.user_dir, want_candidates);
            return Ok(out);
        }

        let game = game_dir
            .map(std::path::PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| paths.game_dir.clone());
        let user = user_data_dir
            .map(std::path::PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| paths.user_dir.clone());

        // 资源根：用户数据目录优先（与 VCMI 双根挂载一致；存在性判定与顺序无关）
        let mut roots: Vec<std::path::PathBuf> = Vec::new();
        for p in [&user, &game] {
            if !p.as_os_str().is_empty() && !roots.iter().any(|x| x == p) {
                roots.push(p.clone());
            }
        }
        let mut out = crate::vcmi::health::health_check_with_roots(&roots);
        append_h3(&mut out, &game, &user, want_candidates);
        Ok(out)
    })
    .await
    // 线程池 panic：按 fail-closed 返回（前端会当作"无法确认原版数据状态"并给重试）
    .map_err(|e| crate::infra::berror::be_err("cmd_blocking_task_failed", &e.to_string()))?
}

/// 附上原版数据闸门与注册表安装候选（两个分支共用）。
fn append_h3(
    out: &mut Value,
    game: &std::path::Path,
    user: &std::path::Path,
    want_candidates: bool,
) {
    let h3 = crate::vcmi::h3data::detect(game, user);
    // 诊断日志：闸门不放过任何"数据其实在但被判缺"的情形（用户报障时第一手证据）
    tracing::info!(
        target: "paths",
        "原版数据探测：found={} roe={} sod={} demo={} missing={:?} game={} user={}",
        h3.found,
        h3.roe,
        h3.sod,
        h3.demo,
        h3.missing,
        game.display(),
        user.display()
    );
    if let Some(obj) = out.as_object_mut() {
        obj.insert("h3Data".into(), json!(h3));
        obj.insert("optionalMods".into(), detect_optional_mods(game, user));
        let candidates: Vec<String> = if want_candidates {
            crate::vcmi::h3data::locate_installs()
                .iter()
                .map(|p| p.to_string_lossy().to_string())
                .collect()
        } else {
            Vec::new()
        };
        obj.insert("dataInstallCandidates".into(), json!(candidates));
    }
}

/// 可选内容是否**已经**就绪（`hd-edition` / `chronicles`）。
///
/// 首启向导第 ③ 步据此把已存在的模组显示成「已就绪」并收起安装控件
/// （此前只看"本次会话是否点过安装"，已有模组仍显示空开关，用户实测反馈）。
/// 两个根都查：向导的安装目标 radio 可能选游戏目录或用户数据目录。
fn detect_optional_mods(game: &std::path::Path, user: &std::path::Path) -> Value {
    let mut hd_edition = false;
    let mut chronicles = false;
    let mut chronicles_chapters = 0u32;
    for root in [user, game] {
        let mods = root.join("Mods");
        if !hd_edition
            && crate::vcmi::hd_edition::detect(&[mods.clone()], None).is_some()
        {
            hd_edition = true;
        }
        let base = mods.join("chronicles");
        if !chronicles && base.join("mod.json").is_file() {
            chronicles = true;
        }
        if chronicles_chapters == 0 {
            // 官方结构：8 篇是**基础模组内嵌**的子模组 `chronicles/Mods/chronicles_<n>`
            // （`vcmi::chronicles::install` 产物）；容忍同级布局作为兜底。
            let nested = base.join("Mods");
            let nested = if nested.is_dir() {
                Some(nested)
            } else {
                let lower = base.join("mods");
                lower.is_dir().then_some(lower)
            };
            let dir = nested.unwrap_or_else(|| mods.clone());
            chronicles_chapters = (1..=8)
                .filter(|n| dir.join(format!("chronicles_{n}")).join("mod.json").is_file())
                .count() as u32;
        }
    }
    json!({
        "hdEdition": hd_edition,
        "chronicles": chronicles,
        "chroniclesChapters": chronicles_chapters,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 两处 Mods 任一命中：`chronicles` 只看 `mod.json`，章节数按 `chronicles_<n>` 统计。
    #[test]
    fn optional_mods_detects_chronicles_in_any_root() {
        let d = std::env::temp_dir().join("vcmi-fv-optional-mods-test");
        let _ = std::fs::remove_dir_all(&d);
        let game = d.join("game");
        let user = d.join("user");
        // 官方结构：`Mods/chronicles/Mods/chronicles_<n>`（子模组内嵌）
        let base = game.join("Mods").join("chronicles");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("mod.json"), "{}").unwrap();
        for n in 1..=3 {
            let sub = base.join("Mods").join(format!("chronicles_{n}"));
            std::fs::create_dir_all(&sub).unwrap();
            std::fs::write(sub.join("mod.json"), "{}").unwrap();
        }
        let v = detect_optional_mods(&game, &user);
        assert_eq!(v["chronicles"], true);
        assert_eq!(v["chroniclesChapters"], 3);
        assert_eq!(v["hdEdition"], false);

        // 用户数据目录同样命中（向导的安装目标可切到用户目录）
        std::fs::create_dir_all(user.join("Mods").join("chronicles")).unwrap();
        std::fs::write(user.join("Mods").join("chronicles").join("mod.json"), "{}").unwrap();
        let empty_game = d.join("empty");
        let v2 = detect_optional_mods(&empty_game, &user);
        assert_eq!(v2["chronicles"], true);
        assert_eq!(v2["chroniclesChapters"], 0);
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 真实 `hd-edition`（`references/Heroes3-VCMI/Mods/hd-edition`）：pak 存在才算就绪。
    #[test]
    fn optional_mods_detects_real_hd_edition() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../references/Heroes3-VCMI");
        if !root.join("Mods").join("hd-edition").is_dir() {
            eprintln!("跳过：缺少 references/Heroes3-VCMI/Mods/hd-edition");
            return;
        }
        let v = detect_optional_mods(&root, &root.join("userdata"));
        assert_eq!(v["hdEdition"], true, "真实 hd-edition 应被判定为已就绪");
    }
}
