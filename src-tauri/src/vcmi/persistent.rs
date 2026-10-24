//! `persistentStorage.json` 清洗：剔除指向「当前生效预设未启用模组」的跨模组引用。
//!
//! 依赖方向（见 `docs/architecture/backend.zh-CN.md` §4.1）：
//! 只依赖 `infra`；**不得**出现 `tauri` 类型，不得依赖 `realms` / `commands`。
//!
//! ## 为什么需要它（VCMI 1.7.4 实测闪退链路）
//!
//! 「战斗模式」标签页把上一次的自定义对局（城镇 / 英雄 / 部队 / 技能）写进
//! `persistentStorage.json` 的 `battleModeSettings` 段，取值形如 `<modId>:<entityId>`。
//! 玩家切到**不含该模组**的预设后：
//!
//! 1. `BattleOnlyModeTab` 构造时反序列化该段，标识符解析失败；而
//!    `requestIdentifier` 是**回调式** API，失败时回调不触发
//!    ⇒ 字段停在构造函数默认值（`selectedTown = FactionID::ANY = -1`）。
//! 2. 客户端收到服务端广播的 `LobbySetBattleOnlyModeStartInfo`
//!    → `applyStartInfo()` → `update()` → `setTerrainButtonText()`，其中
//!    `selectedTerrain == TerrainId::NONE` 时走
//!    `selectedTown.toEntity(LIBRARY)->getNameTranslated()`，即以 **-1** 查 `CTownHandler`。
//! 3. `CHandlerBase::getObjectImpl`（`lib/IHandlerBase.h`）打日志后抛
//!    `std::runtime_error`，未捕获 ⇒ `std::terminate` ⇒ **闪退**。
//!
//! ⚠️ 崩溃发生在**进入 Lobby 时**，与选哪张地图无关，
//!    所以只能由启动器在**拉起 VCMI 之前**落盘清洗。
//!
//! ⚠️ 反直觉点：**只删 `selectedTown` 是无效修法** —— 键缺失时反序列化同样给
//!    `ANY(-1)`，与 `selectedTerrain == NONE` 组合仍然命中崩溃分支
//!    ⇒ 清洗时必须**同时**保证 `selectedTerrain` 为合法值。

use crate::infra::berror::be_err;
use crate::vcmi::paths::VcmiPaths;
use crate::vcmi::scan::RESERVED_SCOPES;
use serde_json::{Map, Value};
use std::collections::HashSet;
use std::fs;
use std::path::Path;

/// `selectedTerrain` 缺失 / 空值时的兜底地形（`TerrainId::DIRT` = 0）。
///
/// ⚠️ 为什么必须补它：引擎在键缺失时给默认值 `TerrainId::NONE(-1)`，
///    而 `BattleOnlyModeTab::setTerrainButtonText()` 在 `terrain == NONE` 时
///    改用 `selectedTown` 查城镇表。于是「terrain 缺失 + town 无效」正是崩溃组合，
///    仅删 `selectedTown` 无法解除它。
const FALLBACK_TERRAIN: &str = "core:dirt";

/// 清洗结果（供日志 / 前端提示）。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct PruneOutcome {
    /// 是否真的改写了文件（**无变化不落盘** —— 不刷 mtime、不产生备份）。
    pub changed: bool,
    /// 被剔除的引用，形如 `battleModeSettings.selectedTown=fairy:fairy`。
    pub removed: Vec<String>,
}

/// 取 `<scope>:<ident>` 里的 scope；不含 `:` 时返回 ""（= 内置 / core 标识符）。
fn scope_of(raw: &str) -> &str {
    match raw.split_once(':') {
        Some((scope, _)) => scope.trim(),
        None => "",
    }
}

/// 判定一个跨模组标识符的 scope 是否**存活**。
///
/// 存活条件（满足其一）：
/// - 无 `:` ⇒ 内置（core）标识符，恒存活；
/// - scope ∈ [`RESERVED_SCOPES`]（`core` / `vcmi`）⇒ 引擎运行时必有，不写入用户选择；
/// - scope 出现在**生效预设**的 `mods` 中；子模组写成 `parent.child` 时 `parent` 命中即可。
pub(crate) fn is_scope_alive(raw: &str, enabled: &HashSet<String>) -> bool {
    let scope = scope_of(raw);
    if scope.is_empty() {
        return true;
    }
    let lower = scope.to_lowercase();
    if RESERVED_SCOPES.iter().any(|s| *s == lower) {
        return true;
    }
    if enabled.contains(&lower) {
        return true;
    }
    match scope.split_once('.') {
        Some((parent, _)) => enabled.contains(&parent.trim().to_lowercase()),
        None => false,
    }
}

/// 缺失 / 空值即视为非法（引擎会落到 `TerrainId::NONE`，从而走崩溃分支）。
///
/// ⚠️ 只做**形态**校验：地形也可能来自未启用的模组，启动器无法判定其存在性，
///    故不把"无法判定的值"当失效值删掉 —— 误删比漏删代价大。
fn terrain_is_valid(raw: Option<&Value>) -> bool {
    match raw.and_then(Value::as_str).map(str::trim) {
        Some(t) => !t.is_empty(),
        None => false,
    }
}

/// 从对象里剔除一个失效的**标量标识符**字段。返回是否发生剔除。
fn prune_scalar_field(
    obj: &mut Map<String, Value>,
    key: &str,
    path: &str,
    enabled: &HashSet<String>,
    removed: &mut Vec<String>,
) -> bool {
    let dead = obj
        .get(key)
        .and_then(Value::as_str)
        .map(|v| v.to_string())
        .filter(|v| !is_scope_alive(v, enabled));
    match dead {
        Some(v) => {
            obj.remove(key);
            removed.push(format!("{path}={v}"));
            true
        }
        None => false,
    }
}

/// 从数组里剔除所有失效的标识符条目（原地保留其余元素顺序）。
fn prune_id_array(
    arr: &mut Vec<Value>,
    path: &str,
    enabled: &HashSet<String>,
    removed: &mut Vec<String>,
) -> bool {
    let items = std::mem::take(arr);
    let mut kept: Vec<Value> = Vec::with_capacity(items.len());
    let mut changed = false;
    for entry in items {
        if let Some(v) = entry.as_str() {
            if !is_scope_alive(v, enabled) {
                removed.push(format!("{path}={v}"));
                changed = true;
                continue;
            }
        }
        kept.push(entry);
    }
    *arr = kept;
    changed
}

/// 从「键为标识符」的映射里剔除失效条目。
fn prune_id_map(
    map: &mut Map<String, Value>,
    path: &str,
    enabled: &HashSet<String>,
    removed: &mut Vec<String>,
) -> bool {
    let dead: Vec<String> = map
        .keys()
        .filter(|k| !is_scope_alive(k, enabled))
        .cloned()
        .collect();
    for k in &dead {
        map.remove(k);
        removed.push(format!("{path}[{k}]"));
    }
    !dead.is_empty()
}

/// 清洗 `battleModeSettings` 段。返回（被剔除的引用列表, 城镇引用是否被剔除）。
///
/// ⚠️ 白名单驱动：只动下表列出的已知标识符路径，未知结构一律不碰（保守优先）。
///
/// | 路径 | 形态 | 失效处理 |
/// |---|---|---|
/// | `battleModeSettings.selectedTown` | 标量 id | 删除键 |
/// | `battleModeSettings.selectedTerrain` | 标量地形 id | 城镇被删时补兜底值 |
/// | `battleModeSettings.slots[].selectedHero` | 标量 id | 删除键 |
/// | `battleModeSettings.slots[].selectedArmy[].type` | 标量 id | 删除键 |
/// | `battleModeSettings.slots[].secSkillLevel[].skill` | 标量 id | 删除键 |
/// | `battleModeSettings.slots[].artifacts` | 键为 id | 删键 |
/// | `battleModeSettings.slots[].spells` | id 数组 | 删条目 |
fn prune_battle_settings(
    obj: &mut Map<String, Value>,
    enabled: &HashSet<String>,
    removed: &mut Vec<String>,
) -> bool {
    // ① 城镇：失效则删除键 ⇒ 引擎反序列化时落到默认 `FactionID::ANY`。
    //    ⚠️ 单独删它**不够**，必须同时处理 ②，理由见 `FALLBACK_TERRAIN` 注释。
    let town_pruned = prune_scalar_field(
        obj,
        "selectedTown",
        "battleModeSettings.selectedTown",
        enabled,
        removed,
    );

    // ② 地形：崩溃组合的另一半。仅当城镇被剔除时才写入，
    //    避免动到「地形已合法 / 玩家主动选了地形」的健康状态。
    if town_pruned && !terrain_is_valid(obj.get("selectedTerrain")) {
        obj.insert(
            "selectedTerrain".to_string(),
            Value::String(FALLBACK_TERRAIN.to_string()),
        );
        tracing::debug!(target: "persistent", "battleModeSettings.selectedTerrain 缺失，已补兜底值 {FALLBACK_TERRAIN}");
    }

    if let Some(slots) = obj.get_mut("slots").and_then(Value::as_array_mut) {
        for (i, slot) in slots.iter_mut().enumerate() {
            let Some(slot_obj) = slot.as_object_mut() else {
                continue;
            };
            let p = format!("battleModeSettings.slots[{i}]");

            prune_scalar_field(
                slot_obj,
                "selectedHero",
                &format!("{p}.selectedHero"),
                enabled,
                removed,
            );

            if let Some(army) = slot_obj
                .get_mut("selectedArmy")
                .and_then(Value::as_array_mut)
            {
                for (j, entry) in army.iter_mut().enumerate() {
                    if let Some(eo) = entry.as_object_mut() {
                        prune_scalar_field(
                            eo,
                            "type",
                            &format!("{p}.selectedArmy[{j}].type"),
                            enabled,
                            removed,
                        );
                    }
                }
            }

            if let Some(skills) = slot_obj
                .get_mut("secSkillLevel")
                .and_then(Value::as_array_mut)
            {
                for (j, entry) in skills.iter_mut().enumerate() {
                    if let Some(eo) = entry.as_object_mut() {
                        prune_scalar_field(
                            eo,
                            "skill",
                            &format!("{p}.secSkillLevel[{j}].skill"),
                            enabled,
                            removed,
                        );
                    }
                }
            }

            if let Some(spells) = slot_obj.get_mut("spells").and_then(Value::as_array_mut) {
                prune_id_array(spells, &format!("{p}.spells"), enabled, removed);
            }

            if let Some(artifacts) = slot_obj.get_mut("artifacts").and_then(Value::as_object_mut) {
                prune_id_map(artifacts, &format!("{p}.artifacts"), enabled, removed);
            }
        }
    }

    town_pruned
}

/// 清洗 `rmg.*.lastSettings.roads` 里的失效道路标识符。
fn prune_rmg_roads(root: &mut Value, enabled: &HashSet<String>, removed: &mut Vec<String>) {
    if let Some(roads) = root
        .get_mut("rmg")
        .and_then(|v| v.get_mut("rmg"))
        .and_then(|v| v.get_mut("lastSettings"))
        .and_then(|v| v.get_mut("roads"))
        .and_then(Value::as_array_mut)
    {
        prune_id_array(roads, "rmg.rmg.lastSettings.roads", enabled, removed);
    }
}

/// 清洗配置目录里的 `persistentStorage.json`（**纯函数**，便于单测）。
///
/// - 只在**发现失效引用**时落盘；无变化不碰文件。
/// - 解析失败 → `Err`，**绝不**降级为空对象覆写（否则清空玩家既有设置）。
/// - 写前备份 `persistentStorage.json.backup`（与 `write_mod_settings` 同款约定）。
pub(crate) fn prune_in_config_dir(
    config_dir: &Path,
    enabled: &HashSet<String>,
) -> Result<PruneOutcome, String> {
    let p = config_dir.join("persistentStorage.json");
    if !p.is_file() {
        return Ok(PruneOutcome::default());
    }
    let text =
        fs::read_to_string(&p).map_err(|e| be_err("persistent_read_failed", &format!("{}", e)))?;
    // ⚠️ 解析失败必须中止：降级为空对象再回写会清空玩家全部既有设置
    let mut root = crate::infra::json::parse_json_lenient(&text)
        .map_err(|e| be_err("persistent_parse_abort", &format!("{}", e)))?;

    let removed = collect_dead_refs(&mut root, enabled);
    if removed.is_empty() {
        return Ok(PruneOutcome::default());
    }

    let backup = p.with_extension("json.backup");
    let _ = fs::copy(&p, &backup);
    let out = serde_json::to_string_pretty(&root)
        .map_err(|e| be_err("persistent_serialize_failed", &format!("{}", e)))?;
    // ⚠️ **原地覆写**（走 trunc）：领域共享配置是硬链接，
    //    temp + rename 会断开共享（见 `realms/activate.rs` §配置共享）。
    fs::write(&p, out).map_err(|e| be_err("persistent_write_failed", &format!("{}", e)))?;

    tracing::info!(
        target: "persistent",
        "已清洗 persistentStorage.json：剔除 {} 项失效引用",
        removed.len()
    );
    Ok(PruneOutcome {
        changed: true,
        removed,
    })
}

/// 剔除根节点里全部已知路径上的失效引用（原地修改），返回被剔除项列表。
fn collect_dead_refs(root: &mut Value, enabled: &HashSet<String>) -> Vec<String> {
    let mut removed: Vec<String> = Vec::new();
    if let Some(battle) = root.get_mut("battleModeSettings").filter(|v| v.is_object()) {
        prune_battle_settings(
            battle.as_object_mut().expect("已过滤为对象"),
            enabled,
            &mut removed,
        );
    }
    prune_rmg_roads(root, enabled, &mut removed);
    removed
}

/// 读取生效预设的启用组合：`<config_dir>/modSettings.json → presets[activePreset].mods`。
///
/// 返回值统一 **小写**（VCMI 的 modId 恒为目录名小写，见 `scan::norm_mod_id`）。
///
/// ⚠️ 判定不出组合时返回 `Err`：**绝不猜测** ——
///    猜错会把玩家仍在使用的引用当成失效引用删掉。
pub(crate) fn enabled_mods(config_dir: &Path) -> Result<HashSet<String>, String> {
    let p = config_dir.join("modSettings.json");
    if !p.is_file() {
        return Err(be_err("persistent_no_modsettings", ""));
    }
    let text = fs::read_to_string(&p)
        .map_err(|e| be_err("cmd_modsettings_read_failed", &format!("{}", e)))?;
    // 解析失败即中止（与 write_mod_settings / patch 路径同口径：不降级为空对象）
    let root = crate::infra::json::parse_json_lenient(&text)
        .map_err(|e| be_err("cmd_modsettings_parse_abort", &format!("{}", e)))?;

    let preset_id = root
        .get("activePreset")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| be_err("persistent_no_active_preset", ""))?;

    let mut set: HashSet<String> = RESERVED_SCOPES.iter().map(|s| s.to_string()).collect();
    if let Some(mods) = root
        .get("presets")
        .and_then(|v| v.get(preset_id))
        .and_then(|v| v.get("mods"))
        .and_then(Value::as_array)
    {
        for entry in mods {
            if let Some(id) = entry.as_str() {
                let id = id.trim().to_lowercase();
                if !id.is_empty() {
                    set.insert(id);
                }
            }
        }
    }
    Ok(set)
}

/// 启动前清洗的**唯一出口**：按 `dirs.json` 解析生效配置目录后清洗
/// `persistentStorage.json`。
///
/// - `game_dir`：游戏数据目录（含 `VCMI_client.exe`）。配置目录取自
///   `<game_dir>/config/dirs.json → userConfigPath`（缺则回退 `<userDataPath>/config`），
///   **因此领域模式下自动命中领域隔离目录，调用方无需感知领域**。
/// - 幂等：无失效引用时不写盘；出错返回 `Err`，由调用方决定（当前策略：不阻断启动）。
pub(crate) fn prune_dead_mod_refs(game_dir: &Path) -> Result<PruneOutcome, String> {
    let Some(config_dir) = VcmiPaths::effective_config_dir(game_dir) else {
        return Ok(PruneOutcome::default());
    };
    let enabled = enabled_mods(&config_dir)?;
    prune_in_config_dir(&config_dir, &enabled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::path::PathBuf;

    fn tmp(tag: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("vcmi-fv-persistent-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).expect("临时目录应可创建");
        d
    }

    fn write_json(dir: &Path, name: &str, value: &Value) {
        let text = serde_json::to_string_pretty(value).expect("序列化应成功");
        fs::write(dir.join(name), text).expect("写入应成功");
    }

    fn read_json(dir: &Path, name: &str) -> Value {
        let text = fs::read_to_string(dir.join(name)).expect("文件应存在");
        crate::infra::json::parse_json_lenient(&text).expect("应为合法 JSON")
    }

    fn official_enabled() -> HashSet<String> {
        ["vcmi", "core", "chinesetruetypefonts"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    /// 真实崩溃场景：`selectedTown = "fairy:fairy"` 且缺 `selectedTerrain`
    #[test]
    fn prunes_dead_town_and_seeds_terrain() {
        let dir = tmp("dead-town");
        write_json(
            &dir,
            "persistentStorage.json",
            &json!({
                "battleModeSettings": {
                    "selectedTown": "fairy:fairy",
                    "slots": [
                        { "primSkillLevel": [2,2,1,1], "secSkillLevel": [], "selectedArmy": [], "selectedHero": "core:christian" },
                        { "primSkillLevel": [0,0,0,0], "secSkillLevel": [], "selectedArmy": [], "spellBook": true, "warMachines": false }
                    ]
                }
            }),
        );

        let out = prune_in_config_dir(&dir, &official_enabled()).expect("清洗应成功");
        assert!(out.changed, "存在失效引用时应落盘");
        assert_eq!(
            out.removed,
            vec!["battleModeSettings.selectedTown=fairy:fairy".to_string()]
        );

        let v = read_json(&dir, "persistentStorage.json");
        assert!(
            v["battleModeSettings"]["selectedTown"].is_null(),
            "失效城镇引用应被剔除"
        );
        // ⚠️ 关键：只删 town 不够，必须同时补合法地形，否则仍命中崩溃分支
        assert_eq!(
            v["battleModeSettings"]["selectedTerrain"],
            json!("core:dirt"),
            "缺失地形应补兜底值"
        );
        // 有效引用必须原样保留
        assert_eq!(
            v["battleModeSettings"]["slots"][0]["selectedHero"],
            json!("core:christian")
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// 全部引用有效 → **不写盘**（内容 + mtime 均不变）
    #[test]
    fn keeps_valid_refs_and_skips_write() {
        let dir = tmp("valid");
        let original = json!({
            "battleModeSettings": {
                "selectedTerrain": "grass",
                "selectedTown": "core:castle",
                "slots": [
                    {
                        "selectedHero": "core:christian",
                        "selectedArmy": [{"type": "core:archer", "amount": 100}],
                        "secSkillLevel": [{"skill": "core:leadership", "masteryLevel": "basic"}],
                        "spells": ["core:bless"],
                        "artifacts": {"core:boots": 1}
                    }
                ]
            }
        });
        write_json(&dir, "persistentStorage.json", &original);
        let before = fs::metadata(dir.join("persistentStorage.json"))
            .expect("metadata")
            .modified()
            .expect("mtime");

        let out = prune_in_config_dir(&dir, &official_enabled()).expect("清洗应成功");
        assert!(!out.changed, "无失效引用时不应写盘");
        assert!(out.removed.is_empty());

        let after = fs::metadata(dir.join("persistentStorage.json"))
            .expect("metadata")
            .modified()
            .expect("mtime");
        assert_eq!(before, after, "无变化时不应刷新 mtime");
        assert_eq!(read_json(&dir, "persistentStorage.json"), original);
        let _ = fs::remove_dir_all(&dir);
    }

    /// slots[] 内各类失效条目按类型剔除，有效条目保留
    #[test]
    fn prunes_dead_entries_in_slots() {
        let dir = tmp("slots");
        write_json(
            &dir,
            "persistentStorage.json",
            &json!({
                "battleModeSettings": {
                    "selectedTerrain": "grass",
                    "selectedTown": "core:rampart",
                    "slots": [
                        {
                            "selectedHero": "fairy:someHero",
                            "selectedArmy": [
                                {"type": "fairy:sprite", "amount": 10},
                                {"type": "core:archer", "amount": 100}
                            ],
                            "secSkillLevel": [{"skill": "fairy:unknown", "masteryLevel": "basic"}],
                            "spells": ["fairy:spell", "core:bless"],
                            "artifacts": {"fairy:ring": 5, "core:boots": 1}
                        }
                    ]
                }
            }),
        );

        let out = prune_in_config_dir(&dir, &official_enabled()).expect("清洗应成功");
        assert!(out.changed);
        assert_eq!(
            out.removed,
            vec![
                "battleModeSettings.slots[0].selectedHero=fairy:someHero".to_string(),
                "battleModeSettings.slots[0].selectedArmy[0].type=fairy:sprite".to_string(),
                "battleModeSettings.slots[0].secSkillLevel[0].skill=fairy:unknown".to_string(),
                "battleModeSettings.slots[0].spells=fairy:spell".to_string(),
                "battleModeSettings.slots[0].artifacts[fairy:ring]".to_string(),
            ]
        );

        let slot = &read_json(&dir, "persistentStorage.json")["battleModeSettings"]["slots"][0];
        assert!(slot.get("selectedHero").is_none(), "失效英雄应被剔除");
        // 部队槽位定长（7 格），失效项只清 `type`（→ CreatureID::NONE，等价空槽），不删槽位
        assert!(
            slot["selectedArmy"][0].get("type").is_none(),
            "失效部队应被清空"
        );
        assert_eq!(
            slot["selectedArmy"][1]["type"],
            json!("core:archer"),
            "有效部队应保留"
        );
        assert!(
            slot["secSkillLevel"][0].get("skill").is_none(),
            "失效技能应被剔除"
        );
        assert_eq!(slot["spells"], json!(["core:bless"]), "有效法术应保留");
        assert_eq!(
            slot["artifacts"],
            json!({ "core:boots": 1 }),
            "有效宝物应保留"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// 内置 scope / 裸标识符 / 子模组：均不得误删
    #[test]
    fn keeps_reserved_scopes_and_bare_identifiers() {
        let enabled: HashSet<String> = ["hota", "hota.neutralcreatures"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert!(is_scope_alive("core:castle", &enabled));
        assert!(is_scope_alive("vcmi:anything", &enabled));
        assert!(is_scope_alive("bareIdentifier", &enabled));
        assert!(is_scope_alive("hota:creature", &enabled));
        assert!(is_scope_alive("hota.neutralcreatures:creature", &enabled));
        assert!(!is_scope_alive("fairy:fairy", &enabled));
    }

    /// RMG 上次设置里失效的道路标识符应被剔除
    #[test]
    fn prunes_dead_roads_in_rmg_settings() {
        let dir = tmp("roads");
        write_json(
            &dir,
            "persistentStorage.json",
            &json!({
                "rmg": { "rmg": { "lastSettings": {
                    "width": 144,
                    "roads": ["core:dirtRoad", "nonexistentMod:fancyRoad", "core:cobblestoneRoad"]
                } } }
            }),
        );

        let out = prune_in_config_dir(&dir, &official_enabled()).expect("清洗应成功");
        assert!(out.changed);
        assert_eq!(
            out.removed,
            vec!["rmg.rmg.lastSettings.roads=nonexistentMod:fancyRoad".to_string()]
        );
        assert_eq!(
            read_json(&dir, "persistentStorage.json")["rmg"]["rmg"]["lastSettings"]["roads"],
            json!(["core:dirtRoad", "core:cobblestoneRoad"])
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// 文件损坏 → Err 且**不覆写**
    #[test]
    fn refuses_corrupt_file() {
        let dir = tmp("corrupt");
        let before = "{ not json";
        fs::write(dir.join("persistentStorage.json"), before).expect("写入应成功");

        assert!(prune_in_config_dir(&dir, &official_enabled()).is_err());
        assert_eq!(
            fs::read_to_string(dir.join("persistentStorage.json")).expect("文件应仍存在"),
            before,
            "损坏文件不应被覆写"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// 文件不存在 → 静默 no-op（不创建）
    #[test]
    fn missing_file_is_noop() {
        let dir = tmp("missing");
        let out = prune_in_config_dir(&dir, &official_enabled()).expect("不应报错");
        assert!(!out.changed);
        assert!(!dir.join("persistentStorage.json").exists(), "不应创建文件");
        let _ = fs::remove_dir_all(&dir);
    }

    /// 判定不出「生效组合」（缺 modSettings）→ Err，不猜不删
    #[test]
    fn enabled_mods_requires_modsettings() {
        let dir = tmp("no-modsettings");
        assert!(
            enabled_mods(&dir).is_err(),
            "缺 modSettings.json 时必须报错"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// 无 `activePreset` 时不猜默认组合（与领域预设「写入前要求已有预设」同口径）
    #[test]
    fn enabled_mods_requires_active_preset() {
        let dir = tmp("no-preset");
        write_json(
            &dir,
            "modSettings.json",
            &json!({ "presets": { "default": { "mods": ["hota"] } } }),
        );
        assert!(enabled_mods(&dir).is_err(), "无 activePreset 时必须报错");
        let _ = fs::remove_dir_all(&dir);
    }

    /// 生效组合 = presets[activePreset].mods ∪ 内置 scope
    #[test]
    fn enabled_mods_includes_reserved_scopes() {
        let dir = tmp("preset");
        write_json(
            &dir,
            "modSettings.json",
            &json!({
                "activePreset": "__official__",
                "presets": { "__official__": { "mods": ["vcmi", "core", "chinesetruetypefonts"], "settings": {} } }
            }),
        );
        let set = enabled_mods(&dir).expect("应解析成功");
        assert!(set.contains("vcmi"));
        assert!(set.contains("core"));
        assert!(set.contains("chinesetruetypefonts"));
        assert!(!set.contains("fairy"));
    }

    /// 写前生成备份（与 `write_mod_settings` 同款约定）
    #[test]
    fn creates_backup_before_rewrite() {
        let dir = tmp("backup");
        write_json(
            &dir,
            "persistentStorage.json",
            &json!({ "battleModeSettings": { "selectedTown": "fairy:fairy" } }),
        );

        prune_in_config_dir(&dir, &official_enabled()).expect("清洗应成功");
        assert!(
            dir.join("persistentStorage.json.backup").is_file(),
            "写前应备份"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// 护栏：只删 `selectedTown` 而**不补** `selectedTerrain` 时崩溃组合依旧成立。
    ///
    /// 这是本修复的反直觉点 —— 用来防止后人把「地形兜底」当成多余逻辑简化掉。
    #[test]
    fn partial_fix_still_crashes() {
        let mut root = json!({ "battleModeSettings": { "selectedTown": "fairy:fairy" } });
        let obj = root["battleModeSettings"].as_object_mut().expect("对象");
        obj.remove("selectedTown"); // 只删城镇，不动地形
        let battle = root.get("battleModeSettings").expect("存在");
        assert!(
            !terrain_is_valid(battle.get("selectedTerrain")),
            "terrain 仍缺失 ⇒ 引擎仍会以 -1 查城镇表"
        );
    }

    /// 真机冒烟（需真实游戏目录；**只写副本**）：
    /// 运行 `cargo test --lib -- vcmi::persistent::tests::real_ --ignored --nocapture`
    #[test]
    #[ignore]
    fn real_prune_is_idempotent() {
        let Some(game_dir) = std::env::var("VCMI_GAME_DIR")
            .ok()
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
        else {
            eprintln!("跳过：未设置 VCMI_GAME_DIR");
            return;
        };
        let effective = VcmiPaths::effective_config_dir(&game_dir).expect("应能解析生效配置目录");

        // ⚠️ 真实目录只读：把待清洗文件复制到临时目录，再把副本的 dirs.json 指向它
        let copy = tmp("real-copy");
        let cfg = copy.join("config");
        fs::create_dir_all(&cfg).expect("应可建目录");
        let mut seeded = 0;
        for name in ["persistentStorage.json", "modSettings.json"] {
            let src = effective.join(name);
            if src.is_file() {
                fs::copy(&src, cfg.join(name)).expect("应可复制");
                seeded += 1;
            }
        }
        if seeded < 2 {
            let _ = fs::remove_dir_all(&copy);
            eprintln!("跳过：生效配置目录缺少 persistentStorage.json / modSettings.json");
            return;
        }

        let first = prune_in_config_dir(&cfg, &enabled_mods(&cfg).expect("应能解析生效组合"))
            .expect("清洗应成功");
        let second = prune_in_config_dir(&cfg, &enabled_mods(&cfg).expect("应能解析生效组合"))
            .expect("二次清洗应成功");
        assert!(
            !second.changed && second.removed.is_empty(),
            "清洗后仍存在失效引用：{:?}",
            second.removed
        );
        println!("首次清洗剔除：{:?}", first.removed);
        let _ = fs::remove_dir_all(&copy);
    }
}
