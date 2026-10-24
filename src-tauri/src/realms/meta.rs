//! 领域元数据与生命周期：`realm.json` 读写、列举/版本、创建/更新/删除/重命名。
//!
//! ⚠️ 本模块的命令出参与错误串（`{code,msg?,realmId?,realmName?}`）是**前端契约**
//! （见 `docs/architecture/backend.zh-CN.md` §7.1），保持逐字不变。
//! `read_realm_meta` / `write_realm_meta` / `realm_versions` / `now_date` 同时被
//! `repository` 复用，故保持 `pub`。

use super::activate::deactivate_realm_inner;
use super::base::{
    ensure_realm_dirs, now_date, persist, read_config, realm_lock_path, realm_meta_path,
    realm_root, realm_root_of, realms_root, write_realm_lock, REALM_PRESET_ID,
};
use crate::infra::berror::be_err;

use serde_json::{json, Value};
use std::fs;
use tauri::State;

use crate::infra::links::{self, is_link};
use crate::state::AppPaths;
use crate::vcmi::paths::VcmiPaths;

/// 语言无关的**媒体**素材字段（壁纸 / 封面 / 音乐 / 介绍音频 / 介绍视频）。
///
/// 其余（如 `introText`）属**文本**，按语言覆盖，留在 `i18n.<lang>.assets`。
/// 媒体只认顶层 `assets`，两条硬理由：
/// - 存储层就存不下分语言：`import_realm_asset` 按 kind 固定文件名（`assets/wallpaper.<ext>`），
///   后传的会覆盖先传的，异扩展名还会删掉旧文件 → 另一语言块留下悬空引用；
/// - 发布推导封面（`plan_publish`）只读顶层 `assets`，配在非默认语言的素材会**静默丢失**。
const REALM_MEDIA_ASSET_KEYS: [&str; 5] =
    ["wallpaper", "cover", "music", "introMusic", "introVideo"];

pub fn read_realm_meta(id: &str) -> Option<Value> {
    let p = realm_meta_path(id)?;
    if !p.is_file() {
        return None;
    }
    let text = fs::read_to_string(&p).ok()?;
    // 宽容解析（JSON5 子集）：realm.json 可能被手工编辑（注释 / 尾逗号）
    let mut realm = crate::infra::json::parse_json_lenient(&text).ok()?;
    normalize_realm_assets(&mut realm);
    Some(realm)
}

/// 把旧形状（媒体写在 `i18n.<lang>.assets`）归一成「媒体只在顶层 `assets`」。
///
/// **只改内存、不写盘**：归并在读取入口做，UI / 导出 / 发布共用同一份数据，
/// 因此无需强制迁移写盘；真正落盘发生在下次保存（`update_realm`）。导入的只读领域
/// （无 `lock`、不会保存）磁盘上仍是旧形状，但读取已归一，导出与展示不受影响。
///
/// 取值优先级：顶层已有值（已是新形状，唯一真相源）> 默认语言 > 其余语言（按语言名字典序，稳定）。
/// 各语言块里的媒体字段一律剥离，空掉的 `assets` 键一并移除。
fn normalize_realm_assets(realm: &mut Value) {
    let Some(obj) = realm.as_object_mut() else {
        return;
    };
    let default_lang = obj
        .get("defaultLang")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();

    // 先只读收集，再一次性改写（避免借用冲突）
    let mut has_top: Vec<String> = Vec::new();
    if let Some(a) = obj.get("assets").and_then(|v| v.as_object()) {
        for (k, v) in a {
            if v.as_str().map(|s| !s.trim().is_empty()).unwrap_or(false) {
                has_top.push(k.clone());
            }
        }
    }
    let mut langs: Vec<(String, serde_json::Map<String, Value>)> = Vec::new();
    if let Some(m) = obj.get("i18n").and_then(|v| v.as_object()) {
        for (lang, entry) in m {
            if let Some(a) = entry.get("assets").and_then(|v| v.as_object()) {
                langs.push((lang.clone(), a.clone()));
            }
        }
    }
    langs.sort_by(|a, b| a.0.cmp(&b.0)); // 语言顺序稳定，保证同一份数据归并结果唯一

    // 1) 上提：顶层缺该媒体字段时，从语言块里找回（默认语言优先）
    let mut lifted: Vec<(String, String)> = Vec::new();
    for key in REALM_MEDIA_ASSET_KEYS {
        if has_top.iter().any(|k| k == key) {
            continue;
        }
        let mut found: Option<String> = None;
        if let Some((_, a)) = langs.iter().find(|(l, _)| l == &default_lang) {
            found = a.get(key).and_then(|v| v.as_str()).map(|s| s.to_string());
        }
        if found.is_none() {
            for (_, a) in &langs {
                if let Some(s) = a.get(key).and_then(|v| v.as_str()) {
                    found = Some(s.to_string());
                    break;
                }
            }
        }
        if let Some(s) = found.filter(|s| !s.trim().is_empty()) {
            lifted.push((key.to_string(), s));
        }
    }
    if !lifted.is_empty() {
        let top = obj
            .entry("assets")
            .or_insert_with(|| Value::Object(serde_json::Map::new()));
        if let Some(top) = top.as_object_mut() {
            for (k, v) in lifted {
                top.insert(k, Value::String(v));
            }
        }
    }

    // 2) 剥离：语言块不再承载媒体字段（空掉的 assets 键一并移除）
    if let Some(m) = obj.get_mut("i18n").and_then(|v| v.as_object_mut()) {
        for (_, entry) in m.iter_mut() {
            let mut drop_assets = false;
            if let Some(a) = entry.get_mut("assets").and_then(|v| v.as_object_mut()) {
                for key in REALM_MEDIA_ASSET_KEYS {
                    a.remove(key);
                }
                drop_assets = a.is_empty();
            }
            if drop_assets {
                if let Some(o) = entry.as_object_mut() {
                    o.remove("assets");
                }
            }
        }
    }
}

/// 写入领域元数据：始终补齐 `id`，剥离导出包专用的 `checksum`。
pub fn write_realm_meta(id: &str, realm: &Value) -> Result<(), String> {
    let root = realm_root(id)?;
    fs::create_dir_all(&root).map_err(|e| be_err("realm_dir_create_failed", &format!("{}", e)))?;
    let mut meta = realm.clone();
    if let Some(o) = meta.as_object_mut() {
        o.insert("id".to_string(), json!(VcmiPaths::sanitize_realm_id(id)));
        o.remove("checksum"); // 校验和只对导出包有意义
    }
    let text = serde_json::to_string_pretty(&meta)
        .map_err(|e| be_err("realm_json_serialize_failed", &format!("{}", e)))?;
    fs::write(root.join("realm.json"), text)
        .map_err(|e| be_err("realm_json_write_failed", &format!("{}", e)))
}

/// 列出全部领域 id（扫描 `realms/` 子目录，按 id 升序）。
pub(crate) fn list_realm_ids() -> Vec<String> {
    let mut out = Vec::new();
    if let Some(root) = realms_root() {
        if let Ok(rd) = fs::read_dir(&root) {
            for e in rd.flatten() {
                if !e.path().is_dir() {
                    continue;
                }
                let name = e.file_name().to_string_lossy().to_string();
                // 跳过非法目录名（sanitize 后不一致的一律不是领域目录）
                if name.is_empty() || name != VcmiPaths::sanitize_realm_id(&name) {
                    continue;
                }
                out.push(name);
            }
        }
    }
    out.sort();
    out
}

/// 读取单个领域元数据；不存在返回 Err（命令侧统一报错文案）。
pub(crate) fn get_realm(id: &str) -> Result<Value, String> {
    read_realm_meta(id).ok_or_else(|| be_err("realm_not_found", id))
}

/// 读取全部领域元数据（按 `createdAt` 升序；无创建时间的排最前，其内按 id 升序）。
pub(crate) fn load_realms() -> Vec<Value> {
    let mut out: Vec<Value> = list_realm_ids()
        .into_iter()
        .filter_map(|id| read_realm_meta(&id))
        .collect();
    out.sort_by_key(|r| {
        r.get("createdAt")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string()
    });
    out
}

/// 供 `repository` 模块做「已安装 / 有更新」比对用的领域版本摘要。
/// 版本优先取 `origin.version`（远程安装时写入），本地领域回退到顶层 `version`；
/// `sha256` 取 `origin.sha256`（安装时索引记录的整包哈希；本地导入无此记录 → 空串，
/// 接收侧判定会退化为纯版本比较）。
pub fn realm_versions() -> Vec<Value> {
    load_realms()
        .into_iter()
        .map(|r| {
            let id = r.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let origin_version = r
                .get("origin")
                .and_then(|o| o.get("version"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let version = r.get("version").and_then(|v| v.as_str()).unwrap_or("");
            let sha256 = r
                .get("origin")
                .and_then(|o| o.get("sha256"))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            json!({
                "id": id,
                "version": if origin_version.is_empty() { version } else { origin_version },
                "sha256": sha256,
            })
        })
        .collect()
}

/// 领域 id 是否已被占用：领域目录存在即占用（不再依赖 launcher.json）。
pub(crate) fn realm_id_exists(id: &str) -> bool {
    realm_root_of(id).map(|p| p.is_dir()).unwrap_or(false)
}

/// 安全删除领域目录（§10.6）：**先逐个摘掉借用联接，再 `remove_dir_all` 真实内容**。
/// 领域目录内 `vcmi/Mods/<id>` 可能是借用联接（指向玩家真实模组），直接 `remove_dir_all`
/// 有跟随联接删除目标文件的风险，故必须先摘联接。
pub(crate) fn remove_realm_dir_safely(id: &str) -> Result<(), String> {
    let root = realm_root(id)?;
    if !root.is_dir() {
        return Ok(());
    }
    let mods = root.join("vcmi").join("Mods");
    if mods.is_dir() {
        if let Ok(entries) = fs::read_dir(&mods) {
            for e in entries.flatten() {
                let p = e.path();
                if is_link(&p) {
                    let _ = links::remove_link(&p);
                }
            }
        }
    }
    fs::remove_dir_all(&root)
        .map_err(|e| be_err("realm_realm_dir_delete_failed", &format!("{}", e)))
}

/// 列出全部领域 + 当前激活 id。
///
/// 每个领域附带 `editable`（§6.2 编辑权）：按 `{realm-dir}/lock` 是否存在判定；
/// 导入的领域不写 lock，故只读。
#[tauri::command]
pub fn list_realms() -> Value {
    // 领域列表直接来自各领域目录的 realm.json（§3.1）；launcher.json 只提供 realms.active。
    let realms: Vec<Value> = load_realms()
        .into_iter()
        .map(|r| {
            let id = r.get("id").and_then(|v| v.as_str()).unwrap_or("");
            // 编辑权（§6.2）：本机创建写有 `{realm-dir}/lock`；导入的领域无 lock → 只读
            let editable = realm_lock_path(id).map(|p| p.is_file()).unwrap_or(false);
            let mut m = r;
            if let Some(o) = m.as_object_mut() {
                o.insert("editable".to_string(), json!(editable));
            }
            m
        })
        .collect();
    let active = read_config().and_then(|c| c.realms.active);
    json!({
        "realms": realms,
        "activeRealm": active,
    })
}

/// 当前激活的领域 id（无则 null）。
#[tauri::command]
pub fn get_active_realm() -> Value {
    match read_config().and_then(|c| c.realms.active) {
        Some(id) => json!({ "activeRealm": id }),
        None => json!({ "activeRealm": Value::Null }),
    }
}

/// 返回某领域设置读写所用的 `config` 绝对路径（作为前端 `config_dir`）。
/// 尊重领域 `dirs.configPath`：
/// - `global`：返回全局用户配置目录，与 VCMI 实际读取位置一致（非隔离领域如 vcmi-realm 的
///   设置/模组预设才能真正生效，也才能读到便携包内置的 `./userdata/config/settings.json` 预设）；
/// - 其它（默认 `realm`）：返回领域隔离目录的 config（隔离核心，保持原行为）。
#[tauri::command]
pub fn realm_config_dir(realm_id: String) -> Result<String, String> {
    let id = VcmiPaths::sanitize_realm_id(&realm_id);
    let realm = get_realm(&id).map_err(|e| be_err("realm_read_failed", &format!("{}", e)))?;
    let config_path_is_global = realm
        .get("dirs")
        .and_then(|d| d.get("configPath"))
        .and_then(|v| v.as_str())
        == Some("global");
    if config_path_is_global {
        let global = read_config()
            .map(|c| c.paths.user_config_dir)
            .unwrap_or_default();
        return Ok(global.to_string_lossy().to_string());
    }
    let base =
        VcmiPaths::realm_vcmi_dir(&id).ok_or_else(|| be_err("common_data_dir_unknown", ""))?;
    Ok(base.join("config").to_string_lossy().to_string())
}

/// 新建领域：前端传入完整领域 JSON（**`id` 必填**），立即建目录并落盘 `realm.json`。
/// 返回创建后的领域（含 id / createdAt）。
#[tauri::command]
pub fn create_realm(data: Value) -> Result<Value, String> {
    // 前置：启动器必须已初始化（领域根目录依赖 launcher 数据目录）
    read_config().ok_or_else(|| be_err("common_not_initialized", ""))?;
    let mut realm = if data.is_object() {
        data.clone()
    } else {
        json!({})
    };
    // ID 必填（新建领域先定 ID 再进编辑器）。字符集由前端实时校验，
    // 这里再兜底一次：非法字符直接报错，绝不静默清洗或生成。
    let raw = realm
        .get("id")
        .and_then(|v| v.as_str())
        .map(|s| s.trim())
        .unwrap_or("");
    if raw.is_empty() {
        return Err(be_err("realm_id_empty", ""));
    }
    let id = VcmiPaths::sanitize_realm_id(raw);
    if id != raw {
        return Err(be_err("realm_id_invalid", ""));
    }
    if realm_id_exists(&id) {
        return Err(be_err("realm_id_taken", &format!("{}", id)));
    }
    let obj = realm
        .as_object_mut()
        .ok_or_else(|| be_err("common_realm_data_corrupt", ""))?;
    obj.insert("id".to_string(), json!(id));
    if !obj.contains_key("createdAt") {
        obj.insert("createdAt".to_string(), json!(now_date()));
    }
    obj.insert("updateAt".to_string(), json!(now_date()));
    // 领域激活时自动应用的模组预设 id（设计 §6；默认固定 __realm__，见 §3.3）
    if !obj.contains_key("presetId") {
        obj.insert("presetId".to_string(), json!(REALM_PRESET_ID));
    }
    // 标签（设计 §6）：新建领域即建立空数组——编辑器保存恒写、发布侧「键存在即声明」
    // （空数组 = 显式清空），确保 realm.json 始终有 tags 字段。
    if !obj.contains_key("tags") {
        obj.insert("tags".to_string(), json!([]));
    }
    // 阵营巡礼（faction-showcase §6.1）：新建领域默认**关闭**——巡礼入口在作者配好
    // 牌组 / 音乐前不应暴露给玩家，由维护人员在编辑器「阵营巡礼」页手工开启。
    if !obj.contains_key("showcase") {
        obj.insert("showcase".to_string(), json!({ "enabled": false }));
    }
    // 确保隔离目录骨架存在（config/cache/logs/Saves/Mods），便于后续写入
    ensure_realm_dirs(&id)?;
    // §6.2：写入编辑权标记 lock（本机创建 → 可编辑）。导入的领域不写 lock，故只读。
    write_realm_lock(&id)?;
    // 领域元信息落盘到 `{realm-dir}/realm.json`（§3.1：目录即领域）
    write_realm_meta(&id, &realm)?;
    Ok(realm)
}

/// 领域 ID 是否可用（新建弹窗实时校验占用）。返回 `{ available, reason }`。
#[tauri::command]
pub fn check_realm_id(id: String) -> Value {
    let raw = id.trim();
    if raw.is_empty() {
        return json!({ "available": false, "reason": "empty" });
    }
    if VcmiPaths::sanitize_realm_id(raw) != raw {
        return json!({ "available": false, "reason": "invalid" });
    }
    if realm_id_exists(raw) {
        return json!({ "available": false, "reason": "occupied" });
    }
    json!({ "available": true, "reason": "" })
}

/// 更新已存在领域（按 id 深合并到 `realm.json`）。
///
/// 「保存无实质变化」不写盘、不刷新 `updateAt`（设计 §6）：否则每次点保存都会让
/// realm.json 变一个字节，导出侧据此报「同版包变」，仓库 diff 也全是噪声。
#[tauri::command]
pub fn update_realm(realm_id: String, data: Value) -> Result<(), String> {
    let id = VcmiPaths::sanitize_realm_id(&realm_id);
    let mut target = get_realm(&id)?;
    let before = target.clone();
    // 深合并：入参覆盖同名键，保留未提供的字段
    if let (Some(to), Some(from)) = (target.as_object_mut(), data.as_object()) {
        for (k, v) in from {
            if k == "id" {
                continue; // id 不可改
            }
            to.insert(k.clone(), v.clone());
        }
    }
    if realm_update_is_noop(&before, &target) {
        return Ok(()); // 内容无变化：磁盘保持原样（updateAt 不动）
    }
    if let Some(to) = target.as_object_mut() {
        // 有实质变化才刷新 `updateAt`（设计 §6）
        to.insert("updateAt".to_string(), json!(now_date()));
    }
    write_realm_meta(&id, &target)
    // 注：这里**不再**用 realm.json 的 mods 覆盖 modSettings.json 的预设。
    // 预设（启用组合）与 borrowed / bundled（携带清单）已解耦，预设由
    // `write_realm_presets` 单独维护，保存领域不会覆盖玩家调整过的启用组合。
}

/// 保存是否没有实质变化（忽略 `updateAt` 这一时间戳噪声字段）。
/// 前端保存会回传本地状态里的旧 `updateAt`（或完全不传），因此两侧都先剔除再全量比较。
fn realm_update_is_noop(before: &Value, after: &Value) -> bool {
    let mut a = before.clone();
    let mut b = after.clone();
    if let (Some(ao), Some(bo)) = (a.as_object_mut(), b.as_object_mut()) {
        ao.remove("updateAt");
        bo.remove("updateAt");
    }
    a == b
}

/// 删除领域：若正在激活则先停用（还原 dirs.json），再移除整个领域目录（含 realm.json）。
#[tauri::command]
pub fn delete_realm(realm_id: String, state: State<AppPaths>) -> Result<(), String> {
    let id = VcmiPaths::sanitize_realm_id(&realm_id);
    let mut cfg = read_config().ok_or_else(|| be_err("common_not_initialized", ""))?;
    if cfg.realms.active.as_deref() == Some(id.as_str()) {
        deactivate_realm_inner(&state)?;
    }
    // 领域目录即全部：删掉 `{realm-dir}` 就删掉了 realm.json / vcmi / assets / lock
    remove_realm_dir_safely(&id)?;
    // 清理可能残留的激活记录
    if cfg.realms.active.as_deref() == Some(id.as_str()) {
        cfg.realms.active = None;
        persist(&cfg)?;
    }
    Ok(())
}

/// 重命名领域（§9.6.1 / B8）：把整个领域目录重命名为新 ID，并同步改写目录内 `realm.json`
/// 的 `id`。仅当 ID 未被占用且领域未激活时允许；返回错误信息给前端提示。
#[tauri::command]
pub fn rename_realm(old_id: String, new_id: String) -> Result<(), String> {
    let old = VcmiPaths::sanitize_realm_id(&old_id);
    let new = VcmiPaths::sanitize_realm_id(&new_id);
    if new.is_empty() {
        return Err(be_err("realm_id_empty", ""));
    }
    if realm_id_exists(&new) {
        return Err(be_err("realm_id_taken", &format!("{}", new)));
    }
    let mut cfg = read_config().ok_or_else(|| be_err("common_not_initialized", ""))?;
    if cfg.realms.active.as_deref() == Some(old.as_str()) {
        return Err(be_err("realm_rename_in_use", ""));
    }
    let old_root = realm_root(&old)?;
    let new_root = realm_root(&new)?;
    if !old_root.is_dir() {
        return Err(be_err("realm_dir_not_found", &format!("{}", old)));
    }
    fs::rename(&old_root, &new_root)
        .map_err(|e| be_err("realm_rename_failed", &format!("{}", e)))?;
    // 目录已改名：同步改写 realm.json 内的 id（它随目录一起搬走）
    if let Some(mut meta) = read_realm_meta(&new) {
        if let Some(o) = meta.as_object_mut() {
            o.insert("id".to_string(), json!(new));
        }
        write_realm_meta(&new, &meta)?;
    }
    // 激活记录理论上不会指向旧 id（上面已拒绝改名），兜底同步一次
    if cfg.realms.active.as_deref() == Some(old.as_str()) {
        cfg.realms.active = Some(new);
        persist(&cfg)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{normalize_realm_assets, realm_update_is_noop};
    use serde_json::json;

    #[test]
    fn save_is_noop_when_only_update_at_differs() {
        let before = json!({ "id": "demo", "name": "A", "updateAt": "2026-09-01" });
        let after = json!({ "id": "demo", "name": "A", "updateAt": "2026-09-29" });
        assert!(realm_update_is_noop(&before, &after));
        // 前端回传本地状态里的旧 updateAt（与磁盘不同）同样视为无变化
        let after_old_ts = json!({ "id": "demo", "name": "A", "updateAt": "2026-09-01" });
        assert!(realm_update_is_noop(&before, &after_old_ts));
    }

    /// 旧形状（媒体写在非默认语言块）→ 读取时上提到顶层，并从语言块剥离
    #[test]
    fn legacy_media_assets_are_lifted_to_top_level() {
        let mut realm = json!({
            "id": "heroes-chronicles",
            "defaultLang": "english",
            "assets": { "introText": "top text" },
            "i18n": {
                "english": { "assets": { "introText": "en text" } },
                "chinese": { "assets": { "wallpaper": "assets/wallpaper.webp", "music": "assets/music.ogg", "introText": "中文" } }
            }
        });
        normalize_realm_assets(&mut realm);
        assert_eq!(realm["assets"]["wallpaper"], json!("assets/wallpaper.webp"));
        assert_eq!(realm["assets"]["music"], json!("assets/music.ogg"));
        // 文本不动：仍是分语言，顶层只保留原有兜底值
        assert_eq!(realm["assets"]["introText"], json!("top text"));
        assert_eq!(
            realm["i18n"]["chinese"]["assets"]["introText"],
            json!("中文")
        );
        // 媒体字段已从语言块剥离
        assert!(realm["i18n"]["chinese"]["assets"]["wallpaper"].is_null());
        assert!(realm["i18n"]["chinese"]["assets"]["music"].is_null());
    }

    /// 新形状（顶层已有值）优先：不被语言块的同名值覆盖
    #[test]
    fn top_level_media_wins_over_legacy_language_entries() {
        let mut realm = json!({
            "id": "demo",
            "defaultLang": "english",
            "assets": { "wallpaper": "assets/new.webp" },
            "i18n": { "chinese": { "assets": { "wallpaper": "assets/old.webp" } } }
        });
        normalize_realm_assets(&mut realm);
        assert_eq!(realm["assets"]["wallpaper"], json!("assets/new.webp"));
        // 语言块被清空的 assets 键一并移除
        assert!(realm["i18n"]["chinese"]["assets"].is_null());
    }

    /// 幂等：归并两次结果一致（读时归一化会被反复调用）
    #[test]
    fn normalize_is_idempotent() {
        let mut realm = json!({
            "id": "demo",
            "defaultLang": "english",
            "i18n": { "chinese": { "assets": { "cover": "assets/cover.webp" } } }
        });
        normalize_realm_assets(&mut realm);
        let once = realm.clone();
        normalize_realm_assets(&mut realm);
        assert_eq!(once, realm);
    }

    #[test]
    fn save_is_not_noop_when_content_differs() {
        let before = json!({ "id": "demo", "name": "A", "updateAt": "2026-09-01" });
        let changed = json!({ "id": "demo", "name": "B", "updateAt": "2026-09-01" });
        assert!(!realm_update_is_noop(&before, &changed));
        // 新增字段也是实质变化
        let added = json!({ "id": "demo", "name": "A", "updateAt": "2026-09-01", "author": "x" });
        assert!(!realm_update_is_noop(&before, &added));
    }
}
