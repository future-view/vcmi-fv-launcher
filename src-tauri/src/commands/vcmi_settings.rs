//! VCMI 游戏设置（`config/settings.json`）读写：`read_vcmi_settings` /
//! `write_vcmi_settings` / `set_vcmi_language`，以及深合并辅助 `merge_json` / `remove_path`。

use crate::infra::berror::be_err;
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::State;

use crate::state::AppPaths;

/// `settings.json` 读写串行锁。
///
/// 原同步命令在**主线程天然串行**；异步化后并发调用会「读-改-写」相互覆盖（丢失更新），
/// 故 read / write / set_vcmi_language 三条路径共用这把锁。
/// ⚠️ 锁只在 `spawn_blocking` 的阻塞线程内持有，**不跨 `.await`**。
static VCMI_SETTINGS_LOCK: Mutex<()> = Mutex::new(());

/// 读取 VCMI 的 settings.json。
/// `config_dir` 可选：传入领域配置目录时读取该领域隔离目录下的 settings.json
/// （领域设置页复用同一组件、按 configDir 读写不同目录，见设计 §3.5.1）。
///
/// ⚠️ `async` + `spawn_blocking`：读整份 settings.json + 解析（模组页 `loadVcmiLanguage` 的路径）。
#[tauri::command]
pub async fn read_vcmi_settings(
    state: State<'_, AppPaths>,
    config_dir: Option<String>,
) -> Result<Value, String> {
    let p = {
        let paths = state.0.lock().expect("paths lock poisoned");
        match &config_dir {
            Some(dir) => PathBuf::from(dir).join("settings.json"),
            None => paths.vcmi_settings_path(),
        }
    };
    tauri::async_runtime::spawn_blocking(move || -> Result<Value, String> {
        let _guard = VCMI_SETTINGS_LOCK.lock().expect("settings lock poisoned");
        if !p.is_file() {
            return Ok(json!({}));
        }
        let text = fs::read_to_string(&p)
            .map_err(|e| be_err("cmd_settings_read_failed", &format!("{}", e)))?;
        serde_json::from_str(&text)
            .map_err(|e| be_err("cmd_settings_parse_failed", &format!("{}", e)))
    })
    .await
    .map_err(|e| be_err("cmd_blocking_task_failed", &format!("{}", e)))?
}

/// 写回 VCMI 的 settings.json（写前备份）。
/// 采用「读-合并-写」而非整体覆盖：入参 data 为需要更新的部分（嵌套对象），
/// 与磁盘现有内容深度合并后再整体写回，避免丢失调用户未改动的其它配置。
/// `config_dir` 可选：传入领域配置目录时写入该领域隔离目录（见设计 §3.5.1）。
///
/// ⚠️ `async` + `spawn_blocking` + 共享锁：读-改-写整份 settings.json，必须与读路径串行。
#[tauri::command]
pub async fn write_vcmi_settings(
    state: State<'_, AppPaths>,
    data: Value,
    config_dir: Option<String>,
) -> Result<(), String> {
    let p = {
        let paths = state.0.lock().expect("paths lock poisoned");
        match &config_dir {
            Some(dir) => PathBuf::from(dir).join("settings.json"),
            None => paths.vcmi_settings_path(),
        }
    };
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let _guard = VCMI_SETTINGS_LOCK.lock().expect("settings lock poisoned");
        // 备份现有文件
        if p.exists() {
            let backup = p.with_extension("json.backup");
            let _ = fs::copy(&p, &backup);
        }
        if let Some(dir) = p.parent() {
            fs::create_dir_all(dir).map_err(|e| be_err("cmd_mkdir_failed", &format!("{}", e)))?;
        }
        // 读出现有内容（不存在则为空对象）
        let mut existing = if p.is_file() {
            let text = fs::read_to_string(&p)
                .map_err(|e| be_err("cmd_settings_read_failed", &format!("{}", e)))?;
            serde_json::from_str::<Value>(&text).unwrap_or(json!({}))
        } else {
            json!({})
        };
        if !existing.is_object() {
            existing = json!({});
        }
        // 提取删除指令（扁平键列表），该键本身不应写入文件
        let to_delete: Vec<String> = data
            .get("__vcmi_fv_delete__")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|x| x.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        // 深度合并：入参覆盖现有同名键（数组/标量直接替换，对象递归）
        merge_json(&mut existing, &data);
        // 合并后剔除指令键本身，避免污染配置文件
        if let Some(obj) = existing.as_object_mut() {
            obj.remove("__vcmi_fv_delete__");
        }
        // 执行删除：将这些官方键从文件中移除，回退到 VCMI 内置默认
        for k in &to_delete {
            let parts: Vec<&str> = k.split('.').collect();
            remove_path(&mut existing, &parts);
        }
        let text = serde_json::to_string_pretty(&existing)
            .map_err(|e| be_err("cmd_serialize_failed", &format!("{}", e)))?;
        fs::write(&p, text).map_err(|e| be_err("cmd_settings_write_failed", &format!("{}", e)))
    })
    .await
    .map_err(|e| be_err("cmd_blocking_task_failed", &format!("{}", e)))?
}

/// 把启动器语言写入 VCMI 生效的 `settings.json` 的 `general.language`（语言设置层）。
///
/// **为什么必须写**：VCMI 的 `ModDependenciesResolver` 有一条硬规则
/// （`references/vcmi/lib/modding/ModManager.cpp:829-832`）——
/// `modType == "Translation"` 的模组，若其 `language`（`getBaseLanguage()`）与
/// `settings.general.language`（`CGeneralTextHandler::getPreferredLanguage()`，默认 `english`）
/// 不一致，直接判为 `BROKEN`，随后由 `updatePreset()` 从激活预设中**移除并回写**。
///
/// 启动器按界面语言注入翻译模组（见 `src/langMods.ts`），若不先把该设置同步一致，
/// 注入的翻译模组会被 VCMI 在启动瞬间删掉，表现为"写进去了又没了"。
///
/// - `config_dir`：传入时作用于该领域配置目录的 `settings.json`（§3.5.1），
///   否则作用于全局用户配置目录。
/// - **幂等**：值已一致时不写盘（避免无谓改动与备份轮换）。
/// - **安全**：文件存在但解析失败直接报错，绝不降级为空对象覆写（否则清空用户全部设置）。
///
/// ⚠️ `async` + `spawn_blocking` + 共享锁：读-改-写 settings.json，与 read / write 串行。
#[tauri::command]
pub async fn set_vcmi_language(
    state: State<'_, AppPaths>,
    lang: String,
    config_dir: Option<String>,
) -> Result<(), String> {
    let lang = lang.trim().to_string();
    if lang.is_empty() {
        return Err(be_err("settings_lang_empty", ""));
    }
    let p = {
        let paths = state.0.lock().expect("paths lock poisoned");
        match &config_dir {
            Some(dir) => PathBuf::from(dir).join("settings.json"),
            None => paths.vcmi_settings_path(),
        }
    };
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let _guard = VCMI_SETTINGS_LOCK.lock().expect("settings lock poisoned");
        let mut settings: Value = if p.is_file() {
            let text = fs::read_to_string(&p)
                .map_err(|e| be_err("cmd_settings_read_failed", &format!("{}", e)))?;
            crate::infra::json::parse_json_lenient(&text)
                .map_err(|e| be_err("cmd_settings_parse_abort", &format!("{}", e)))?
        } else {
            json!({})
        };
        if !settings.is_object() {
            return Err(be_err("vcmi_settings_struct", ""));
        }
        // 幂等：已是目标语言则不动
        if settings
            .get("general")
            .and_then(|g| g.get("language"))
            .and_then(|v| v.as_str())
            == Some(lang.as_str())
        {
            return Ok(());
        }
        if p.exists() {
            let _ = fs::copy(&p, p.with_extension("json.backup"));
        }
        if let Some(dir) = p.parent() {
            fs::create_dir_all(dir).map_err(|e| be_err("cmd_mkdir_failed", &format!("{}", e)))?;
        }
        if !settings
            .get("general")
            .map(|v| v.is_object())
            .unwrap_or(false)
        {
            settings["general"] = json!({});
        }
        if let Some(g) = settings.get_mut("general").and_then(|v| v.as_object_mut()) {
            g.insert("language".to_string(), json!(lang));
        }
        let text = serde_json::to_string_pretty(&settings)
            .map_err(|e| be_err("cmd_serialize_failed", &format!("{}", e)))?;
        fs::write(&p, text).map_err(|e| be_err("cmd_settings_write_failed", &format!("{}", e)))?;
        tracing::info!(target: "lang", "已同步 VCMI 语言设置 general.language={lang}");
        Ok(())
    })
    .await
    .map_err(|e| be_err("cmd_blocking_task_failed", &format!("{}", e)))?
}

/// 把 b 深合并进 a（b 覆盖 a）。
fn merge_json(a: &mut Value, b: &Value) {
    if let (Some(ao), Some(bo)) = (a.as_object_mut(), b.as_object()) {
        for (k, v) in bo {
            if let Some(av) = ao.get(k) {
                if av.is_object() && v.is_object() {
                    let mut merged = av.clone();
                    merge_json(&mut merged, v);
                    ao.insert(k.clone(), merged);
                    continue;
                }
            }
            ao.insert(k.clone(), v.clone());
        }
    }
}

/// 在嵌套 Value 中按扁平键路径删除末端节点。
/// 路径段为数字时视为数组索引；删除数组元素后若数组变空则一并删除该数组。
/// 重置设置：复用 write_vcmi_settings 的读-合并-写流程（见其 data 中的
/// "__vcmi_fv_delete__" 指令），不再单独注册 tauri 命令，以规避本机工具链对
/// 新增命令宏的 never-type 兼容性错误。此处仅保留 remove_path 工具函数供其调用。

fn remove_path(value: &mut Value, parts: &[&str]) {
    if parts.is_empty() {
        return;
    }
    if let Some(obj) = value.as_object_mut() {
        if parts.len() == 1 {
            obj.remove(parts[0]);
            return;
        }
        if let Some(child) = obj.get_mut(parts[0]) {
            remove_path(child, &parts[1..]);
            // 若子对象变空则删除以保持整洁
            if let Some(c) = obj.get(parts[0]) {
                if c.is_object() && c.as_object().map(|m| m.is_empty()).unwrap_or(false) {
                    obj.remove(parts[0]);
                }
            }
        }
    } else if let Some(arr) = value.as_array_mut() {
        if let Ok(idx) = parts[0].parse::<usize>() {
            if idx < arr.len() {
                if parts.len() == 1 {
                    arr.remove(idx);
                } else {
                    remove_path(&mut arr[idx], &parts[1..]);
                }
            }
        }
    }
}
