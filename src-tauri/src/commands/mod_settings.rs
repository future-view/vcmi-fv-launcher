//! `modSettings.json` 读写与预设补丁：`read_mod_settings` / `write_mod_settings` /
//! `patch_preset_lang_mods`。
//!
//! ⚠️ 本文件的**出参/入参字段名是前端契约**（见 `docs/architecture/backend.zh-CN.md` §7.1），
//! 重构时保持逐字不变。

use crate::infra::berror::be_err;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::State;

use crate::state::AppPaths;
use crate::vcmi::paths::VcmiPaths;

/// `modSettings.json` 读写串行锁。
///
/// 原同步命令在**主线程天然串行**；异步化后并发调用会「读-改-写」相互覆盖
/// （最坏情况：写盘与备份交叉，把用户预设写坏），故 read / write / patch 三条路径共用这把锁。
/// ⚠️ 锁只在 `spawn_blocking` 的阻塞线程内持有，**不跨 `.await`**。
static MOD_SETTINGS_LOCK: Mutex<()> = Mutex::new(());

/// 读取 modSettings.json 全文。
/// `config_dir` 可选：传入领域配置目录时读取该领域隔离目录下的 modSettings.json
/// （与 `read_vcmi_settings` 的 `config_dir` 参数同一模式，见设计 §3.5.1）。
/// 文件不存在时返回空对象（而非报错），便于前端区分「无预设」与「读取失败」。
///
/// ⚠️ `async` + `spawn_blocking`：文件读取与解析下沉阻塞线程池；路径解析留在命令壳内
/// 同步完成（需读受管状态，且不得跨 `.await` 持锁）。
#[tauri::command]
pub async fn read_mod_settings(state: State<'_, AppPaths>, config_dir: Option<String>) -> Result<Value, String> {
    let p = match &config_dir {
        Some(dir) => PathBuf::from(dir).join("modSettings.json"),
        None => {
            let mut paths = state.0.lock().expect("paths lock poisoned");
            // 防御：若全局路径尚未就绪（如 user_config_dir 为空），重新从 launcher.json 解析，
            // 确保读到的就是用户真实配置目录，避免读到错误/空路径而返回 null 后被前端误判为「无配置」覆盖清空。
            if paths.user_config_dir.as_os_str().is_empty() {
                if let Some(cfg) = VcmiPaths::read_launcher_config() {
                    paths.update_from_config(&cfg);
                }
            }
            paths.mod_settings_path()
        }
    };
    tauri::async_runtime::spawn_blocking(move || -> Result<Value, String> {
        if !p.is_file() {
            return Ok(json!({}));
        }
        let text = fs::read_to_string(&p).map_err(|e| be_err("cmd_read_failed", &format!("{}", e)))?;
        // 宽容解析（JSON5 子集）：modSettings.json 可能被手工 / 外部工具编辑（注释、尾逗号）
        crate::infra::json::parse_json_lenient(&text).map_err(|e| be_err("cmd_json_parse_failed", &format!("{}", e)))
    })
    .await
    .map_err(|e| be_err("cmd_blocking_task_failed", &format!("{}", e)))?
}

/// 写 modSettings.json（写前备份）
#[tauri::command]
pub fn write_mod_settings(state: State<AppPaths>, mut data: Value) -> Result<(), String> {
    let mut paths = state.0.lock().expect("paths lock poisoned");
    // 与 read_mod_settings 一致：路径未就绪时先从 launcher.json 重新解析，避免写到错误目录。
    if paths.user_config_dir.as_os_str().is_empty() {
        if let Some(cfg) = VcmiPaths::read_launcher_config() {
            paths.update_from_config(&cfg);
        }
    }
    let p = paths.mod_settings_path();
    if p.exists() {
        let backup = p.with_extension("json.backup");
        fs::copy(&p, &backup).map_err(|e| be_err("cmd_backup_failed", &format!("{}", e)))?;
    }
    if let Some(dir) = p.parent() {
        fs::create_dir_all(dir).map_err(|e| be_err("cmd_mkdir_failed", &format!("{}", e)))?;
    }
    // 全局强制模组：确保写盘后的每个预设都包含 mods.required（用户不可关闭）。
    if let Some(cfg) = VcmiPaths::read_launcher_config() {
        let req = &cfg.mods.required;
        if !req.is_empty() {
            if let Some(presets) = data.get_mut("presets").and_then(|v| v.as_object_mut()) {
                for (_k, v) in presets.iter_mut() {
                    if let Some(m) = v.get_mut("mods").and_then(|x| x.as_array_mut()) {
                        for r in req {
                            if !m.iter().any(|e| e.as_str() == Some(r.as_str())) {
                                m.push(serde_json::json!(r));
                            }
                        }
                    }
                }
            }
        }
    }
    let text = serde_json::to_string_pretty(&data).map_err(|e| be_err("cmd_serialize_failed", &format!("{}", e)))?;
    fs::write(&p, text).map_err(|e| be_err("cmd_write_failed", &format!("{}", e)))
}

/// 按启动器语言同步某预设的「语言相关模组」（语言层，与语言无关的 `mods.required` 互补）：
/// 从目标预设的 `mods` 中移除 `remove`、加入 `add`（去重、保序）。
///
/// - `config_dir`：传入时作用于**该领域隔离目录**的 `modSettings.json`（§3.5.1），
///   否则作用于全局用户配置目录。
/// - `preset_id`：目标预设 id（领域取 `realm.json` 的 `presetId`，全局取 `activePreset`）。
/// - 返回 `missing`：`add` 中「VCMI 可见模组目录里实际不存在」的模组 id，仅用于前端提示使用者。
///   按既定决策：模组由开发者保证置于游戏 `Mods/` 或便携 `./userdata/Mods`，
///   真缺漏**不阻断写入**，交由 VCMI 自身检测。
#[tauri::command]
pub async fn patch_preset_lang_mods(
    state: State<'_, AppPaths>,
    config_dir: Option<String>,
    preset_id: String,
    add: Vec<String>,
    remove: Vec<String>,
) -> Result<Value, String> {
    if preset_id.is_empty() {
        return Err(be_err("mod_settings_preset_empty", ""));
    }

    // 一次性取「目标 modSettings.json 路径」与「存在性检查目录」快照（不得跨 .await 持锁）。
    let (p, mods_dirs) = {
        let mut paths = state.0.lock().expect("paths lock poisoned");
        // 与 read/write_mod_settings 一致：路径未就绪时先从 launcher.json 重新解析。
        if paths.user_config_dir.as_os_str().is_empty() {
            if let Some(cfg) = VcmiPaths::read_launcher_config() {
                paths.update_from_config(&cfg);
            }
        }
        match &config_dir {
            Some(dir) => {
                let d = PathBuf::from(dir);
                // 领域 config 目录形如 `{realm}/vcmi/config`，其同级 `Mods` 即领域可见模组目录
                let mut dirs = vec![paths.game_mods_dir(), paths.user_mods_dir()];
                if let Some(rm) = d.parent().map(|x| x.join("Mods")) {
                    dirs.push(rm);
                }
                (d.join("modSettings.json"), dirs)
            }
            None => (
                paths.mod_settings_path(),
                vec![paths.game_mods_dir(), paths.user_mods_dir()],
            ),
        }
    };

    tauri::async_runtime::spawn_blocking(move || -> Result<Value, String> {
    let _guard = MOD_SETTINGS_LOCK.lock().expect("mod settings lock poisoned");
    // 仅在文件不存在时用最小结构；**解析失败必须报错**——
    // 回退成空对象再覆写会清空玩家预设。
    let mut settings: Value = if p.is_file() {
        let text = fs::read_to_string(&p).map_err(|e| be_err("cmd_modsettings_read_failed", &format!("{}", e)))?;
        // 宽容解析（JSON5 子集）；解析失败即报错，绝不降级为空对象覆写
        crate::infra::json::parse_json_lenient(&text)
            .map_err(|e| be_err("cmd_modsettings_json_parse_failed", &format!("{}", e)))?
    } else {
        json!({})
    };
    if settings.get("presets").is_none() {
        settings["presets"] = json!({});
    }

    {
        let presets = settings
            .get_mut("presets")
            .and_then(|v| v.as_object_mut())
            .ok_or_else(|| be_err("mod_settings_presets_corrupt", ""))?;
        if !presets.contains_key(&preset_id) {
            presets.insert(preset_id.clone(), json!({ "mods": [], "settings": {} }));
        }
        let entry = presets
            .get_mut(&preset_id)
            .ok_or_else(|| be_err("mod_settings_preset_write_failed", ""))?;
        if entry.get("mods").is_none() {
            entry["mods"] = json!([]);
        }
        let mods = entry
            .get_mut("mods")
            .and_then(|v| v.as_array_mut())
            .ok_or_else(|| be_err("mod_settings_preset_mods_corrupt", ""))?;
        // 先移除其它语言的模组，再加入当前语言的模组（去重、保持原有顺序）
        mods.retain(|v| v.as_str().map(|s| !remove.iter().any(|r| r == s)).unwrap_or(true));
        for a in &add {
            if !mods.iter().any(|v| v.as_str() == Some(a.as_str())) {
                mods.push(json!(a));
            }
        }
        // 与 write_realm_presets 一致：预设必须含内置模组 vcmi，否则 VCMI 缺内置字体无法启动
        if !mods.iter().any(|v| v.as_str() == Some("vcmi")) {
            mods.insert(0, json!("vcmi"));
        }
    }

    // 与既有写盘命令一致：确保语言无关的 mods.required 仍在每个预设中，避免本命令把它写丢。
    if let Some(cfg) = VcmiPaths::read_launcher_config() {
        let req = &cfg.mods.required;
        if !req.is_empty() {
            if let Some(presets) = settings.get_mut("presets").and_then(|v| v.as_object_mut()) {
                for (_k, v) in presets.iter_mut() {
                    if let Some(m) = v.get_mut("mods").and_then(|x| x.as_array_mut()) {
                        for r in req {
                            if !m.iter().any(|e| e.as_str() == Some(r.as_str())) {
                                m.push(json!(r));
                            }
                        }
                    }
                }
            }
        }
    }

    // 存在性检查：仅用于「告知使用者」，不阻断写入（目录来自命令壳快照 `mods_dirs`）。
    let mut missing: Vec<String> = Vec::new();
    if !add.is_empty() {
        for a in &add {
            if !mods_dirs.iter().any(|d| d.join(a).is_dir()) {
                missing.push(a.clone());
            }
        }
    }

    if p.exists() {
        let backup = p.with_extension("json.backup");
        let _ = fs::copy(&p, &backup);
    }
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).map_err(|e| be_err("cmd_mkdir_failed", &format!("{}", e)))?;
    }
    let text = serde_json::to_string_pretty(&settings).map_err(|e| be_err("cmd_serialize_failed", &format!("{}", e)))?;
    fs::write(&p, text).map_err(|e| be_err("cmd_write_failed", &format!("{}", e)))?;
    Ok(json!({ "missing": missing }))
    })
    .await
    .map_err(|e| be_err("cmd_blocking_task_failed", &format!("{}", e)))?
}

/// 清除指定模组在 `validatedMods`（VCMI 的内容校验缓存）中的条目，**强制引擎重新校验**。
///
/// 为什么需要它：`validatedMods` 的 checksum 只覆盖 `mod.json` + 各 config json
/// （`references/vcmi/lib/modding/ModManager.cpp` 的 ModsStorage checksum 计算），
/// 玩家在游戏外**替换过模组的素材 / 增删文件**时 checksum 往往不变 ⇒ 引擎沿用旧校验结果、
/// 不再重查文件完整性（缺文件也不报错）。删掉条目即可让引擎下次启动重跑校验。
///
/// - `config_dir`：传入时作用于该领域隔离目录的 `modSettings.json`（§3.5.1），否则全局。
/// - `mod_ids`：要清除的模组 id（大小写不敏感；**同时清除其子模组** `id.xxx` 条目）。
/// - 返回实际清除的条目数（0 = 本来就没有缓存条目，属正常）。
/// - 文件不存在 / 无 `validatedMods` 段时直接返回 0（不创建文件、不报错）。
#[tauri::command]
pub async fn clear_mod_validation(
    state: State<'_, AppPaths>,
    config_dir: Option<String>,
    mod_ids: Vec<String>,
) -> Result<usize, String> {
    if mod_ids.is_empty() {
        return Ok(0);
    }
    let p = {
        let mut paths = state.0.lock().expect("paths lock poisoned");
        match &config_dir {
            Some(dir) => PathBuf::from(dir).join("modSettings.json"),
            None => {
                if paths.user_config_dir.as_os_str().is_empty() {
                    if let Some(cfg) = VcmiPaths::read_launcher_config() {
                        paths.update_from_config(&cfg);
                    }
                }
                paths.mod_settings_path()
            }
        }
    };
    tauri::async_runtime::spawn_blocking(move || -> Result<usize, String> {
    let _guard = MOD_SETTINGS_LOCK.lock().expect("mod settings lock poisoned");
    if !p.is_file() {
        return Ok(0);
    }
    let text = fs::read_to_string(&p).map_err(|e| be_err("cmd_modsettings_read_failed", &format!("{}", e)))?;
    // 解析失败即报错：**绝不**把坏文件覆盖成空对象
    let mut settings: Value = crate::infra::json::parse_json_lenient(&text)
        .map_err(|e| be_err("cmd_modsettings_json_parse_failed", &format!("{}", e)))?;
    let targets: Vec<String> = mod_ids
        .iter()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();

    let removed = {
        let validated = settings
            .get_mut("validatedMods")
            .and_then(|v| v.as_object_mut());
        match validated {
            Some(map) => {
                let before = map.len();
                map.retain(|k, _| {
                    let key = k.trim().to_lowercase();
                    !targets
                        .iter()
                        .any(|t| key == *t || key.starts_with(&format!("{t}.")))
                });
                before - map.len()
            }
            None => 0,
        }
    };
    if removed == 0 {
        return Ok(0);
    }

    let backup = p.with_extension("json.backup");
    let _ = fs::copy(&p, &backup);
    let out = serde_json::to_string_pretty(&settings).map_err(|e| be_err("cmd_serialize_failed", &format!("{}", e)))?;
    fs::write(&p, out).map_err(|e| be_err("cmd_write_failed", &format!("{}", e)))?;
    Ok(removed)
    })
    .await
    .map_err(|e| be_err("cmd_blocking_task_failed", &format!("{}", e)))?
}
