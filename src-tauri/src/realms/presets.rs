//! 预设（`__realm__`）写入：为领域生成 modSettings 预设、注入必需 mod、维护上一次激活的预设。
//!
//! 预设文件位于领域根的 `modSettings.json`；`inject_required_mods` 负责把领域声明的
//! 必需 mod 强制置为启用，避免用户误关导致领域不可用。

use crate::infra::berror::be_err;
use super::base::{PREV_ACTIVE_PRESET_FILE, REALM_PRESET_ID, read_config, realm_root};
use super::meta::get_realm;

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};

use crate::vcmi::paths::VcmiPaths;

/// 写入领域隔离目录下的 `modSettings.json`：确保 `presets.__realm__` 存在，
/// 并携带领域的模组组合（borrowed + bundled 引用）。仅当组合非空时才把
/// `activePreset` 设为 `__realm__`，避免空预设意外禁用全部模组。
/// 计算某领域 `modSettings.json` 的实际落点：
/// - `dirs.configPath=global`：落在全局用户配置目录（`read_config().paths.user_config_dir`），
///   即 VCMI 真正读取的位置；
/// - 其它（默认 `realm`）：落在领域隔离目录的 `config/modSettings.json`（隔离核心）。
pub(crate) fn realm_mod_settings_path(id: &str) -> Result<PathBuf, String> {
    let realm = get_realm(id)?;
    let global = realm
        .get("dirs")
        .and_then(|d| d.get("configPath"))
        .and_then(|v| v.as_str())
        == Some("global");
    if global {
        return Ok(read_config()
            .map(|c| c.paths.user_config_dir)
            .unwrap_or_default()
            .join("modSettings.json"));
    }
    let base = VcmiPaths::realm_vcmi_dir(id)
        .ok_or_else(|| be_err("common_data_dir_unknown", ""))?;
    Ok(base.join("config").join("modSettings.json"))
}

/// 领域自带的 modSettings.json 完全由启动器写入（见 §3.3），故这里直接覆盖预设段。
///
/// **共享配置（`dirs.configPath = "global"`）也必须写**：`realm_mod_settings_path` 已把它
/// 解析到全局用户配置目录（`userdata/config/modSettings.json`），即 VCMI 真正读取的那份。
/// 顶级页面重构后领域模式不再挂载 Mods 视图（消除写盘冲突），若此处跳过，
/// 共享配置下将**没有任何路径**创建该文件（实测问题）。
/// 写入采用**合并**语义：保留既有其它预设，仅确保 `preset_id` 存在并把 `activePreset` 指向它。
pub(crate) fn write_realm_preset(id: &str) -> Result<(), String> {
    let realm = get_realm(id)?;
    // 预设 id 取自 `realm.presetId`（设计 §6），缺省回落固定值 `__realm__`
    let preset_id = realm
        .get("presetId")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(REALM_PRESET_ID)
        .to_string();
    let mods = realm.get("mods").cloned().unwrap_or(Value::Null);
    let borrowed = mods
        .get("borrowed")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let bundled = mods
        .get("bundled")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    // 领域是否携带自有模组（决定是否注入 `__realm__` 预设）。
    let has_realm_mods = !borrowed.is_empty() || !bundled.is_empty();

    let mut list: Vec<Value> = Vec::new();
    for m in borrowed.iter().chain(bundled.iter()) {
        if let Some(s) = m.as_str() {
            // VCMI 读取预设用的是 `convertTo<std::vector<TModID>>()`
            // （`ModsPresetState::getRootMods`，references/vcmi/lib/modding/ModManager.cpp:218-225），
            // 而 `TModID = std::string`（ModManager.h:19）——因此这里**必须写字符串数组**，
            // 写成 `{id, enabled}` 对象数组会导致转换后为空、领域模组一个都不启用。
            list.push(json!(s));
        }
    }
    // 内置模组 `vcmi` 必须启用：字体等内置资源来自 `Mods/vcmi/Content`，只有它被激活
    // 才会注册进资源系统（`ModScope::scopeBuiltin()` 是 "core"，不会自动带上 vcmi）。
    if !list.iter().any(|v| v.as_str() == Some("vcmi")) {
        list.insert(0, json!("vcmi"));
    }
    // 全局强制模组：领域预设种子也须包含（realm.json 不携带此特性，随发行版 launcher.json 生效）
    if let Some(cfg) = VcmiPaths::read_launcher_config() {
        for r in &cfg.mods.required {
            if !list.iter().any(|v| v.as_str() == Some(r.as_str())) {
                list.push(json!(r));
            }
        }
    }

    let p = realm_mod_settings_path(id)?;
    let mut settings: Value = if p.is_file() {
        let text = fs::read_to_string(&p).map_err(|e| be_err("realm_modsettings_read_failed", &format!("{}", e)))?;
        // **解析失败必须报错**：静默降级为空对象会在回写时清空官方已有的全部预设。
        crate::infra::json::parse_json_lenient(&text)
            .map_err(|e| be_err("realm_modsettings_parse_abort", &format!("{}", e)))?
    } else {
        json!({})
    };
    if settings.get("presets").is_none() {
        settings["presets"] = json!({});
    }
    let has_any_preset = settings["presets"]
        .as_object()
        .map(|o| !o.is_empty())
        .unwrap_or(false);

    // 空清单领域（未携带任何自有模组）且**已有预设**：不注入 `__realm__`、不改 `activePreset`，
    // 保持共享（全局）配置原样——其启用组合交由既有预设决定（旧设计语义）。
    // 仅当目标文件尚无任何预设（全新环境）时才注入，否则将没有任何配置可用。
    if !has_realm_mods && has_any_preset {
        let changed = inject_required_mods(&mut settings);
        tracing::info!(
            target: "realm",
            "领域未携带模组：跳过注入 {preset_id}（保持既有 activePreset 不变，required_mods 变更={changed}）"
        );
        if changed {
            write_settings_file(&p, &settings)?;
        }
        return Ok(());
    }
    // §3.3：进入领域时**确保领域预设存在**。但若预设**已存在则不覆盖**——
    // 启用组合由编辑器 / 玩家独立维护（可包含游戏目录的模组、可有多个预设），
    // 而 realm.json 的 `borrowed` / `bundled` 只是「携带清单」（决定联接 / 复制 / 导出），
    // 不再用于推导启用组合，以免保存领域时把玩家调整过的启用组合覆盖掉。
    // 进入领域前记录原 `activePreset`（仅首次），供退出领域时恢复
    record_prev_active_preset(id, settings.get("activePreset").and_then(|v| v.as_str()))?;

    let created = if settings["presets"].get(preset_id.as_str()).is_none() {
        // 官方 preset 结构为 { mods, settings }；settings 是子模组开关，缺失时给空对象
        settings["presets"][preset_id.as_str()] = json!({ "mods": list, "settings": {} });
        true
    } else {
        false
    };
    // activePreset 指向领域当前预设（§3.3）
    settings["activePreset"] = json!(preset_id);
    write_settings_file(&p, &settings)?;
    tracing::info!(target: "realm", "领域预设已就绪：preset={preset_id} 新建={created}");
    Ok(())
}

/// 写入 `modSettings.json`（自动建父目录）。
pub(crate) fn write_settings_file(p: &Path, settings: &Value) -> Result<(), String> {
    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).map_err(|e| be_err("realm_realm_config_dir_create_failed", &format!("{}", e)))?;
    }
    let text = serde_json::to_string_pretty(settings)
        .map_err(|e| be_err("realm_modsettings_serialize_failed", &format!("{}", e)))?;
    fs::write(p, text).map_err(|e| be_err("realm_modsettings_write_failed", &format!("{}", e)))
}

/// 确保启动器级强制模组（`launcher.json` 的 `mods.required`）存在于**所有**预设中。
/// 返回是否发生变更（无变更时调用方可跳过写盘）。
pub(crate) fn inject_required_mods(settings: &mut Value) -> bool {
    let Some(cfg) = VcmiPaths::read_launcher_config() else {
        return false;
    };
    let req = &cfg.mods.required;
    if req.is_empty() {
        return false;
    }
    let mut changed = false;
    if let Some(presets) = settings.get_mut("presets").and_then(|v| v.as_object_mut()) {
        for (_k, v) in presets.iter_mut() {
            if let Some(m) = v.get_mut("mods").and_then(|x| x.as_array_mut()) {
                for r in req {
                    if !m.iter().any(|e| e.as_str() == Some(r.as_str())) {
                        m.push(json!(r));
                        changed = true;
                    }
                }
            }
        }
    }
    changed
}

/// 记录进入领域前的 `activePreset`（**仅首次**）。
///
/// 已存在记录时保持不变——重复激活不能把 `__realm__` 记成"上次值"，否则退出时无法还原。
/// 记录为空字符串表示「原文件没有 activePreset」，恢复时按兜底链解析。
pub(crate) fn record_prev_active_preset(id: &str, prev: Option<&str>) -> Result<(), String> {
    let root = realm_root(id)?;
    let f = root.join(PREV_ACTIVE_PRESET_FILE);
    if f.is_file() {
        return Ok(());
    }
    fs::create_dir_all(&root).map_err(|e| be_err("realm_dir_create_failed", &format!("{}", e)))?;
    fs::write(&f, prev.unwrap_or("")).map_err(|e| be_err("realm_preset_snapshot_write_failed", &format!("{}", e)))
}

/// 退出领域时恢复进入前的 `activePreset`，并删除快照点文件。
///
/// 兜底链：快照值（**须确实存在**）→ `__official__`（须存在）→ 首个非领域预设 → 保持不动。
/// 必须校验存在性，否则会把 `activePreset` 指向不存在的预设（VCMI 行为不可预期）。
/// 无快照文件（如空清单领域从未注入）时直接返回。
pub(crate) fn restore_prev_active_preset(id: &str) -> Result<(), String> {
    let root = realm_root(id)?;
    let f = root.join(PREV_ACTIVE_PRESET_FILE);
    if !f.is_file() {
        return Ok(());
    }
    let prev = fs::read_to_string(&f).unwrap_or_default().trim().to_string();
    let realm_preset_id = get_realm(id)
        .ok()
        .and_then(|r| {
            r.get("presetId")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| REALM_PRESET_ID.to_string());

    let p = realm_mod_settings_path(id)?;
    if p.is_file() {
        let text = fs::read_to_string(&p).map_err(|e| be_err("cmd_modsettings_read_failed", &format!("{}", e)))?;
        let mut settings = crate::infra::json::parse_json_lenient(&text)
            .map_err(|e| be_err("realm_modsettings_parse_skip", &format!("{}", e)))?;
        let target = {
            let presets = settings.get("presets").and_then(|v| v.as_object());
            let has = |k: &str| presets.map(|o| o.contains_key(k)).unwrap_or(false);
            if !prev.is_empty() && has(&prev) {
                Some(prev.clone())
            } else if has("__official__") {
                Some("__official__".to_string())
            } else {
                presets.and_then(|o| {
                    o.keys()
                        .find(|k| k.as_str() != realm_preset_id && k.as_str() != REALM_PRESET_ID)
                        .cloned()
                })
            }
        };
        match target {
            Some(t) => {
                settings["activePreset"] = json!(t);
                write_settings_file(&p, &settings)?;
                tracing::info!(target: "realm", "已恢复退出领域前的 activePreset：{t}");
            }
            None => tracing::warn!(target: "realm", "无可用预设可恢复，保持 activePreset 不变"),
        }
    }

    let _ = fs::remove_file(&f);
    Ok(())
}

/// 写入领域隔离目录下 `modSettings.json` 的**全部预设**（多预设支持，§3.3）。
///
/// - `presets`：完整预设字典 `{ <presetId>: { mods: string[], settings?: {...} } }`。
/// - `active_preset`：当前启用的预设 id（写入 `activePreset`）。
///
/// 与 `write_realm_preset` 的区别：后者只在激活时**确保领域预设存在**（缺失才初始化）；
/// 本命令由编辑器调用、整体接管预设内容，是「启用组合」的权威来源。
/// 每个预设都会补齐内置模组 `vcmi`——否则 VCMI 会因缺少内置字体无法启动
/// （`ModScope::scopeBuiltin()` 是 "core"，不会自动带上 vcmi）。
#[tauri::command]
pub fn write_realm_presets(
    realm_id: String,
    presets: Value,
    active_preset: String,
) -> Result<(), String> {
    let id = VcmiPaths::sanitize_realm_id(&realm_id);
    let p = realm_mod_settings_path(&id)?;
    let mut settings: Value = if p.is_file() {
        let text = fs::read_to_string(&p).map_err(|e| be_err("realm_modsettings_read_failed", &format!("{}", e)))?;
        // 解析失败必须报错：静默降级为空对象会在回写时清空既有预设
        crate::infra::json::parse_json_lenient(&text)
            .map_err(|e| be_err("realm_modsettings_parse_abort", &format!("{}", e)))?
    } else {
        json!({})
    };

    // 规范为官方 preset 结构 { mods, settings }，并给每个预设补齐内置模组 vcmi
    let mut out = serde_json::Map::new();
    if let Some(obj) = presets.as_object() {
        for (pid, val) in obj {
            let mut mods: Vec<Value> = val
                .get("mods")
                .and_then(|m| m.as_array())
                .cloned()
                .unwrap_or_default()
                .into_iter()
                .filter(|v| v.as_str().map(|s| !s.is_empty()).unwrap_or(false))
                .collect();
            if !mods.iter().any(|v| v.as_str() == Some("vcmi")) {
                mods.insert(0, json!("vcmi"));
            }
            let sub = val.get("settings").cloned().unwrap_or_else(|| json!({}));
            out.insert(pid.clone(), json!({ "mods": mods, "settings": sub }));
        }
    }
    // 全局强制模组：确保落盘的每个预设都包含 required_mods（用户不可关闭）
    if let Some(cfg) = VcmiPaths::read_launcher_config() {
        let req = &cfg.mods.required;
        if !req.is_empty() {
            for (_pid, val) in out.iter_mut() {
                if let Some(m) = val.get_mut("mods").and_then(|x| x.as_array_mut()) {
                    for r in req {
                        if !m.iter().any(|e| e.as_str() == Some(r.as_str())) {
                            m.push(json!(r));
                        }
                    }
                }
            }
        }
    }
    settings["presets"] = Value::Object(out);
    settings["activePreset"] = json!(active_preset);

    if let Some(parent) = p.parent() {
        fs::create_dir_all(parent).map_err(|e| be_err("realm_realm_config_dir_create_failed", &format!("{}", e)))?;
    }
    let text = serde_json::to_string_pretty(&settings)
        .map_err(|e| be_err("realm_modsettings_serialize_failed", &format!("{}", e)))?;
    fs::write(&p, text).map_err(|e| be_err("realm_modsettings_write_failed", &format!("{}", e)))
}
