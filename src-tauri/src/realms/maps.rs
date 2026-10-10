//! 地图管理：官方 `Maps` 目录阀门（封锁/解锁）、领域地图收纳仓库
//! （`Maps.vcmi-fv-realm/`：VCMI 不挂载的真相源，含元数据缓存 `index.json`）、
//! 引入/移动/删除与游戏内语言隔离（按语言重建 `Maps/` 下的 junction 集合）。

use crate::infra::berror::be_err;
use super::base::{
    MAPS_NATIVE_SUFFIX, game_dir_of, guard_vcmi_not_running, now_date, read_config, realm_root,
};
use super::meta::{get_realm, write_realm_meta};

use serde_json::{Value, json};
use std::fs;
use std::path::{Path, PathBuf};

use crate::infra::h3m::{self, MapMeta};
use crate::infra::links::{ensure_link, is_link, remove_link};
use crate::state::AppPaths;
use crate::vcmi::paths::VcmiPaths;
use tauri::State;

/// 领域地图收纳仓库目录名（VCMI 不挂载 `.vcmi-fv-realm` 后缀目录，真相源）。
pub(crate) const MAPS_REALM_SUFFIX: &str = "vcmi-fv-realm";

/// 领域地图支持的扩展名（其余文件一律忽略）。
const MAP_EXTS: [&str; 3] = ["h3m", "h3c", "vmap"];

// ── 游戏自带地图阀门（§8.3，原有逻辑不变）────────────────────────────

/// 切换「屏蔽游戏自带地图」阀门（§8.3）。仅对当前激活领域即时生效，并持久化偏好。
#[tauri::command]
pub fn set_realm_maps_blocked(realm_id: String, blocked: bool, state: State<AppPaths>) -> Result<(), String> {
    let id = VcmiPaths::sanitize_realm_id(&realm_id);
    let mut realm = get_realm(&id)?;
    {
        let obj = realm
            .as_object_mut()
            .ok_or_else(|| be_err("common_realm_data_corrupt", ""))?;
        // 屏蔽偏好的唯一权威是 `dirs.gameMaps`（设计 §6）
        let dirs = obj
            .entry("dirs")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or_else(|| be_err("common_realm_data_corrupt", ""))?;
        dirs.insert(
            "gameMaps".to_string(),
            json!(if blocked { "hide" } else { "show" }),
        );
        obj.insert("updateAt".to_string(), json!(now_date()));
    }
    // 偏好落盘到领域目录内的 realm.json（§3.1）
    write_realm_meta(&id, &realm)?;
    // 仅在当前激活时即时切换阀门
    let cfg = read_config().ok_or_else(|| be_err("common_not_initialized", ""))?;
    if cfg.realms.active.as_deref() == Some(id.as_str()) {
        guard_vcmi_not_running()?;
        let game_dir = game_dir_of(&state);
        apply_maps_valve(&game_dir, blocked)?;
    }
    Ok(())
}

/// 地图阀门状态查询（§8.3.4）：`organized` 表示游戏自带 `Maps` 已被收纳进
/// `Maps.vcmi-fv-native`（即「整理」已执行过）；`blocked` 表示当前处于屏蔽态
/// （`Maps` 联接不存在、仅见领域自带地图）。供前端决定是否弹出「整理」确认弹窗。
#[tauri::command]
pub fn native_maps_status(state: State<AppPaths>) -> Value {
    let game_dir = game_dir_of(&state);
    let maps = game_dir.join("Maps");
    let native = game_dir.join(format!("Maps.{MAPS_NATIVE_SUFFIX}"));
    let organized = native.is_dir();
    let blocked = organized && !maps.exists();
    json!({ "organized": organized, "blocked": blocked })
}

/// 地图阀门（§8.3）：懒执行。
/// - blocked=false（默认态，完全透明）：确保 `Maps` 是 junction → `Maps.vcmi-fv-native`。
/// - blocked=true：移除 `Maps` junction，使 VCMI 仅见领域自带地图。
/// 真实地图目录始终为 `Maps.vcmi-fv-native`；junction 的删除**只能用 remove_dir**。
pub(crate) fn apply_maps_valve(game_dir: &Path, blocked: bool) -> Result<(), String> {
    let maps = game_dir.join("Maps");
    let native = game_dir.join(format!("Maps.{MAPS_NATIVE_SUFFIX}"));

    // 首次：把真实 Maps 改名为 native（仅当真实 Maps 存在且 native 不存在）
    if maps.is_dir() && !native.exists() {
        fs::rename(&maps, &native).map_err(|e| be_err("realm_maps_collect_failed", &format!("{}", e)))?;
    }

    if blocked {
        // 移除 junction 暴露（不删真实文件：remove_dir 仅删目录联接本身）
        if maps.is_dir() {
            fs::remove_dir(&maps).map_err(|e| be_err("realm_maps_block_failed", &format!("{}", e)))?;
        }
    } else {
        // 透明态：确保 junction 存在
        if !maps.exists() && native.is_dir() {
            ensure_link(&maps, &native)?;
        }
    }
    Ok(())
}

/// 修复崩坏的地图阀门形态（§8.3.3 状态机 / §4.5 自检表）。
/// 三个动作各自只处理一种坏形态，形态不匹配直接报错（不做猜测），都由玩家显式触发：
///
/// - `"unlink"`：联接悬空（`Maps` 是联接但 `Maps_native` 已不在）→ 只删联接本体，
///   回到「无自带地图」基线；
/// - `"merge"`：`Maps` 与 `Maps_native` **都是真实目录**（VCMI 更新又装了地图）→
///   新出现的自带地图并入 `Maps_native` → 删掉真实 `Maps` → 重建联接；
/// - `"revert"`：彻底还原游戏目录 → 删联接 → `Maps_native` 改名回 `Maps`。
///
/// 安全红线：**删除联接只能用 `remove_link`（内部 `remove_dir`）**，`Maps_native` 是真相源，
/// 任何分支都不得对它 `remove_dir_all`。
#[tauri::command]
pub fn repair_game_maps(action: String, state: State<AppPaths>) -> Result<(), String> {
    guard_vcmi_not_running()?;
    let game_dir = game_dir_of(&state);
    let maps = game_dir.join("Maps");
    let native = game_dir.join(format!("Maps.{MAPS_NATIVE_SUFFIX}"));
    match action.as_str() {
        "unlink" => {
            if !is_link(&maps) {
                return Err(be_err("realm_maps_repair_not_link", ""));
            }
            remove_link(&maps)
        }
        "merge" => {
            if !native.is_dir() || !maps.is_dir() || is_link(&maps) {
                return Err(be_err("realm_maps_repair_not_merged", ""));
            }
            move_dir_contents(&maps, &native)?;
            // 已搬空：此处 `maps` 是真实目录（形态已校验），删除它不会波及真相源
            fs::remove_dir_all(&maps).map_err(|e| be_err("realm_temp_maps_clear_failed", &format!("{}", e)))?;
            ensure_link(&maps, &native)
        }
        "revert" => {
            if !native.is_dir() {
                return Err(be_err("realm_maps_repair_nothing", ""));
            }
            if maps.exists() && !is_link(&maps) {
                return Err(be_err("realm_maps_repair_exists", ""));
            }
            if is_link(&maps) {
                remove_link(&maps)?;
            }
            fs::rename(&native, &maps).map_err(|e| be_err("realm_maps_restore_failed", &format!("{}", e)))
        }
        _ => Err(be_err("realm_unsupported_repair_action", "")),
    }
}

/// 把 `src` 目录下的内容搬入 `dst`（递归；同名以 `-2` / `-3` 递增后缀避让）。
/// 文件先试 `rename`（同卷零拷贝），跨卷失败回退「复制 + 删源」；
/// 目录联接**不许当作目录递归**（会顺着联接跑到真相源），改为在新位置重建同名联接。
fn move_dir_contents(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| be_err("realm_target_dir_create_failed", &format!("{}", e)))?;
    let entries: Vec<_> = fs::read_dir(src)
        .map_err(|e| be_err("realm_map_dir_read_failed", &format!("{}", e)))?
        .flatten()
        .collect();
    for e in entries {
        let from = e.path();
        let name = e.file_name().to_string_lossy().into_owned();
        let mut target = dst.join(&name);
        if target.exists() {
            let stem = Path::new(&name)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            let ext = Path::new(&name)
                .extension()
                .map(|s| format!(".{}", s.to_string_lossy()))
                .unwrap_or_default();
            let mut n = 2;
            loop {
                target = dst.join(format!("{stem}-{n}{ext}"));
                if !target.exists() {
                    break;
                }
                n += 1;
            }
        }
        // 先判联接：目录联接被 `is_dir()` 判为真目录（Windows 跟随联接），放前面会误递归
        if is_link(&from) {
            let tgt = fs::read_link(&from).map_err(|e| be_err("realm_junction_read_failed", &format!("{}", e)))?;
            ensure_link(&target, &tgt)?;
            remove_link(&from)?;
        } else if from.is_dir() {
            move_dir_contents(&from, &target)?;
            fs::remove_dir_all(&from).map_err(|e| be_err("realm_src_dir_clear_failed", &format!("{}", e)))?;
        } else {
            fs::rename(&from, &target)
                .or_else(|_| fs::copy(&from, &target).and_then(|_| fs::remove_file(&from)))
                .map_err(|e| be_err("realm_move_failed", &format!("{} {}", name, e)))?;
        }
    }
    Ok(())
}

/// 判定目录是否为重解析点（复用 links::is_link）。
pub(crate) fn is_reparse_point(p: &Path) -> bool {
    is_link(p)
}

/// 取领域的「屏蔽游戏自带地图」偏好（读 `{realm-dir}/realm.json` 的 `dirs.gameMaps`）。
pub(crate) fn realm_blocked(id: &str) -> bool {
    get_realm(id)
        .ok()
        .and_then(|r| {
            r.get("dirs")
                .and_then(|d| d.get("gameMaps"))
                .and_then(|v| v.as_str())
                .map(|s| s == "hide")
        })
        .unwrap_or(false)
}

// ── 领域地图收纳仓库（Maps.vcmi-fv-realm）────────────────────────────

/// 收纳仓库根：`{realm}/vcmi/Maps.vcmi-fv-realm/`。
fn repo_dir(id: &str) -> Result<PathBuf, String> {
    Ok(realm_root(id)?.join("vcmi").join(format!("Maps.{MAPS_REALM_SUFFIX}")))
}

/// VCMI 可视目录：`{realm}/vcmi/Maps/`（内容全部为指向仓库的 junction）。
fn visible_dir(id: &str) -> Result<PathBuf, String> {
    Ok(realm_root(id)?.join("vcmi").join("Maps"))
}

fn is_map_file(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| MAP_EXTS.iter().any(|x| x.eq_ignore_ascii_case(e)))
        .unwrap_or(false)
}

/// 按扩展名决定入库分区（h3c → campaigns，其余 → maps）。
fn kind_by_ext(ext: &str) -> &'static str {
    if ext.eq_ignore_ascii_case("h3c") {
        "campaigns"
    } else {
        "maps"
    }
}

/// MapMeta → 仓库条目 JSON（`file` 为仓库内相对路径，前端据此分区）。
fn meta_to_entry(file: &str, abs: &Path, meta: MapMeta) -> Value {
    let size = fs::metadata(abs).map(|m| m.len()).unwrap_or(0);
    let modified = fs::metadata(abs)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| (d.as_secs() * 1000 + u64::from(d.subsec_millis())) as i64)
        .unwrap_or(0);
    json!({
        "file": file,
        "kind": meta.kind,
        "lang": meta.lang,
        "encoding": meta.encoding,
        "name": meta.name,
        "description": meta.description,
        "version": meta.version,
        "difficulty": meta.difficulty,
        "players": meta.players,
        "humans": meta.humans,
        "sizeW": meta.size_w,
        "sizeH": meta.size_h,
        "sizeClass": meta.size_class,
        "victory": meta.victory,
        "loss": meta.loss,
        "scenarioCount": meta.scenario_count,
        "scenarios": meta.scenarios,
        "size": size,
        "modifiedAt": modified,
    })
}

/// 解析失败时的兜底条目（lang=unknown，不阻塞引入）。
fn fallback_entry(file: &str, abs: &Path, kind_dir: &str) -> Value {
    let ext = abs
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("h3m")
        .to_lowercase();
    let mut e = meta_to_entry(
        file,
        abs,
        MapMeta {
            kind: (if kind_dir == "campaigns" { "campaign" } else { "map" }).to_string(),
            lang: "unknown".into(),
            encoding: String::new(),
            name: String::new(),
            description: String::new(),
            version: String::new(),
            difficulty: -1,
            players: -1,
            humans: -1,
            size_w: 0,
            size_h: 0,
            size_class: String::new(),
            victory: String::new(),
            loss: String::new(),
            scenario_count: 0,
            scenarios: Vec::new(),
        },
    );
    if let Some(o) = e.as_object_mut() {
        o.insert("parseError".to_string(), json!(format!("unsupported .{ext}")));
    }
    e
}

/// 解析单个地图文件（含 vmap/h3c ZIP 分流），失败回退兜底条目。
fn parse_map_entry(file: &str, abs: &Path, kind_dir: &str) -> Value {
    match h3m::parse_file(abs) {
        Ok(meta) => meta_to_entry(file, abs, meta),
        Err(_) => fallback_entry(file, abs, kind_dir),
    }
}

// ── index.json 元数据缓存 ────────────────────────────────────────────

fn index_path(id: &str) -> Result<PathBuf, String> {
    Ok(repo_dir(id)?.join("index.json"))
}

/// index.json 缓存格式版本：解析产物结构 / 语义变化时递增（不匹配视为无缓存，全量重建）。
/// - v1 → v2：H3C 场景表结构化（regionName 走编码判决 + mapFileName / difficulty）；
///   v1 条目的 `scenarios` 为纯字符串数组（且 GBK 场景名乱码），必须重解析。
const INDEX_CACHE_VERSION: u32 = 2;

/// 读缓存：`{ "version": N, "entries": [entry...] }`；以 `file` 为键。
/// 版本不匹配（含旧文件无 `version` 字段）→ 返回空，触发全量重建。
fn load_index(id: &str) -> Vec<Value> {
    let Ok(path) = index_path(id) else {
        return Vec::new();
    };
    let Ok(text) = fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return Vec::new();
    };
    if v.get("version").and_then(|x| x.as_u64()) != Some(u64::from(INDEX_CACHE_VERSION)) {
        return Vec::new();
    }
    v.get("entries")
        .and_then(|e| e.as_array())
        .cloned()
        .unwrap_or_default()
}

/// 递归收集目录下的地图文件（返回仓库内相对路径形式的前缀+绝对路径）。
/// `rel` 始终相对最初传入的根目录（保留子目录结构，如 `chinese/结盟/x.h3m`）。
fn collect_map_files(root: &Path, out: &mut Vec<(String, PathBuf)>) {
    collect_map_files_in(root, root, out);
}

fn collect_map_files_in(base: &Path, dir: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() && !is_link(&p) {
            collect_map_files_in(base, &p, out);
        } else if p.is_file() && is_map_file(&p) {
            let rel = p
                .strip_prefix(base)
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .unwrap_or_default();
            out.push((rel, p));
        }
    }
}

/// 全量重建 index.json：目录为真相源；缓存中 path+size+mtime 一致的条目直接复用
/// （避免重复解析 h3m 头），新文件/变更文件才重新解析。
fn rebuild_index(id: &str) -> Result<Vec<Value>, String> {
    let repo = repo_dir(id)?;
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    for kind in ["maps", "campaigns"] {
        let d = repo.join(kind);
        if d.is_dir() {
            let mut found: Vec<(String, PathBuf)> = Vec::new();
            collect_map_files(&d, &mut found);
            // 统一加上分区前缀：`maps/chinese/x.h3m` / `campaigns/chinese/y.h3c`
            files.extend(found.into_iter().map(|(rel, p)| (format!("{kind}/{rel}"), p)));
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));

    // 旧缓存索引（file → entry），命中（size+mtime 相同）则复用
    let mut cache: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
    for e in load_index(id) {
        if let Some(f) = e.get("file").and_then(|x| x.as_str()) {
            cache.insert(f.to_string(), e);
        }
    }
    let sig = |p: &Path| -> (u64, i64) {
        let md = fs::metadata(p).ok();
        (
            md.as_ref().map(|m| m.len()).unwrap_or(0),
            md.and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0),
        )
    };

    let mut entries: Vec<Value> = Vec::with_capacity(files.len());
    for (rel, abs) in &files {
        let kind_dir = rel.split('/').next().unwrap_or("maps");
        let cached = cache.get(rel).filter(|c| {
            let cs = (
                c.get("size").and_then(|x| x.as_u64()).unwrap_or(0),
                c.get("modifiedAt").and_then(|x| x.as_i64()).unwrap_or(0),
            );
            cs == sig(abs)
        });
        let entry = match cached {
            Some(c) => c.clone(),
            None => parse_map_entry(rel, abs, kind_dir),
        };
        entries.push(entry);
    }

    let repo = repo_dir(id)?;
    fs::create_dir_all(&repo).map_err(|e| be_err("realm_map_repo_create_failed", &format!("{}", e)))?;
    let text = serde_json::to_string_pretty(&json!({ "version": INDEX_CACHE_VERSION, "entries": entries }))
        .map_err(|e| be_err("realm_index_serialize_failed", &format!("{}", e)))?;
    fs::write(repo.join("index.json"), text).map_err(|e| be_err("realm_index_write_failed", &format!("{}", e)))?;
    Ok(entries)
}

/// 迁移 + 重建 index.json + 回写 realm.json 快照（领域导出/解包后调用）。
pub(crate) fn sync_realm_maps_meta(id: &str) -> Vec<Value> {
    if migrate_legacy_maps(id).is_ok() {
        if let Ok(entries) = rebuild_index(id) {
            sync_realm_maps_snapshot(id, &entries);
            return entries;
        }
    }
    Vec::new()
}

/// 把 entries 快照回写 realm.json 的 `maps.entries`（保持既有分发快照契约），
/// 同时保留 `maps.filterLang` / `maps.filterLangTarget` 配置。
fn sync_realm_maps_snapshot(id: &str, entries: &[Value]) {
    if let Ok(mut realm) = get_realm(id) {
        if let Some(o) = realm.as_object_mut() {
            let mut maps = o
                .get("maps")
                .cloned()
                .filter(|m| m.is_object())
                .unwrap_or_else(|| json!({}));
            if let Some(m) = maps.as_object_mut() {
                m.insert("entries".to_string(), json!(entries));
            }
            o.insert("maps".to_string(), maps);
            o.insert("updateAt".to_string(), json!(now_date()));
            let _ = write_realm_meta(id, &realm);
        }
    }
}

/// 旧版迁移：`{realm}/vcmi/Maps/` 下的存量地图搬入收纳仓库对应语言子目录，
/// 随后清空可视目录（为 junction 重建做准备）。
fn migrate_legacy_maps(id: &str) -> Result<(), String> {
    let vis = visible_dir(id)?;
    if !vis.is_dir() {
        return Ok(());
    }
    // 可视目录若已是纯 junction 集合（新布局），无需迁移
    let mut legacy: Vec<(String, PathBuf)> = Vec::new();
    collect_map_files(&vis, &mut legacy);
    if legacy.is_empty() {
        return Ok(());
    }
    let repo = repo_dir(id)?;
    for (_, abs) in legacy {
        let ext = abs
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("h3m")
            .to_lowercase();
        let kind_dir = kind_by_ext(&ext);
        let meta = h3m::parse_file(&abs).ok();
        let lang = meta.map(|m| m.lang).unwrap_or_else(|| "unknown".into());
        let name = abs.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let dest = repo.join(kind_dir).join(&lang).join(&name);
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).map_err(|e| be_err("realm_repo_dir_create_failed", &format!("{}", e)))?;
        }
        if abs.is_file() {
            fs::rename(&abs, &dest).or_else(|_| fs::copy(&abs, &dest).map(|_| ()))
                .map_err(|e| be_err("realm_map_migrate_failed", &format!("{} {}", name, e)))?;
        }
    }
    // 搬空后清掉可视目录残留（空目录树），交由 junction 重建
    prune_empty_dirs(&vis)?;
    Ok(())
}

/// 自底向上删除空目录（含根），非空则原样保留。
fn prune_empty_dirs(root: &Path) -> Result<(), String> {
    if !root.is_dir() {
        return Ok(());
    }
    for e in fs::read_dir(root).map_err(|e| be_err("realm_generic_arg", &format!("{}", e)))?.flatten() {
        let p = e.path();
        if p.is_dir() && !is_link(&p) {
            prune_empty_dirs(&p)?;
        }
    }
    // 目录已空才删除；junction 交给上层按链接处理
    if fs::read_dir(root).map(|mut rd| rd.next().is_none()).unwrap_or(false) {
        if is_link(root) {
            remove_link(root)?;
        } else {
            let _ = fs::remove_dir(root);
        }
    }
    Ok(())
}

// ── 游戏内语言隔离：junction 集合重建（§0.4）─────────────────────────

/// 读领域语言过滤配置：`(enabled, target_lang)`；target 为空时回退启动器 `ui_lang`。
fn realm_map_filter(id: &str) -> (bool, String) {
    let realm = get_realm(id).unwrap_or_else(|_| json!({}));
    let maps = realm.get("maps").cloned().unwrap_or_else(|| json!({}));
    let enabled = maps
        .get("filterLang")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let target = maps
        .get("filterLangTarget")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .or_else(|| {
            VcmiPaths::read_launcher_config()
                .and_then(|c| c.ui.lang)
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_else(|| "english".into());
    (enabled, target)
}

/// 重建 `{realm}/vcmi/Maps/` 的 junction 集合（maps + campaigns 两组）：
/// - 过滤开：仅 junction「目标语言 + unknown」子目录；
/// - 过滤关：全部语言子目录。
/// junction 命名：`<lang>` 与 `<lang>-campaigns`。
pub(crate) fn rebuild_realm_maps_valve(id: &str) -> Result<(), String> {
    migrate_legacy_maps(id)?;
    let repo = repo_dir(id)?;
    let vis = visible_dir(id)?;
    let (filter, target) = realm_map_filter(id);

    // 可视目录：清掉旧 junction（真实目录留给迁移逻辑处理）
    if vis.is_dir() {
        for e in fs::read_dir(&vis).map_err(|e| be_err("realm_maps_dir_read_failed", &format!("{}", e)))?.flatten() {
            let p = e.path();
            if is_link(&p) {
                remove_link(&p)?;
            }
        }
        if !vis.exists() {
            fs::create_dir_all(&vis).map_err(|e| be_err("realm_maps_dir_create_failed", &format!("{}", e)))?;
        }
    } else {
        fs::create_dir_all(&vis).map_err(|e| be_err("realm_maps_dir_create_failed", &format!("{}", e)))?;
    }

    for kind in ["maps", "campaigns"] {
        let src_root = repo.join(kind);
        if !src_root.is_dir() {
            continue;
        }
        let Ok(langs) = fs::read_dir(&src_root) else {
            continue;
        };
        for l in langs.flatten() {
            let lang_dir = l.path();
            if !lang_dir.is_dir() || is_link(&lang_dir) {
                continue;
            }
            let lang = l.file_name().to_string_lossy().into_owned();
            if filter && lang != target && lang != "unknown" {
                continue;
            }
            let link_name = if kind == "campaigns" {
                format!("{lang}-campaigns")
            } else {
                lang.clone()
            };
            let link = vis.join(&link_name);
            // 目标目录可能为空语言壳，仍建 junction 保持结构一致
            let _ = fs::create_dir_all(&lang_dir);
            ensure_link(&link, &lang_dir)?;
        }
    }
    Ok(())
}

/// 命令版重建：仅当领域为当前激活时执行（前端在过滤开关/UI 语言变更时调用）。
#[tauri::command]
pub fn apply_realm_maps_valve(realm_id: String) -> Result<(), String> {
    let id = VcmiPaths::sanitize_realm_id(&realm_id);
    let cfg = read_config().ok_or_else(|| be_err("common_not_initialized", ""))?;
    if cfg.realms.active.as_deref() == Some(id.as_str()) {
        guard_vcmi_not_running()?;
        rebuild_realm_maps_valve(&id)?;
    }
    Ok(())
}

/// 设置语言过滤（开关 + 目标语言）并持久化；激活领域即时重建阀门。
#[tauri::command]
pub fn set_realm_map_filter(
    realm_id: String,
    enabled: bool,
    target: Option<String>,
) -> Result<(), String> {
    let id = VcmiPaths::sanitize_realm_id(&realm_id);
    let mut realm = get_realm(&id)?;
    {
        let obj = realm
            .as_object_mut()
            .ok_or_else(|| be_err("common_realm_data_corrupt", ""))?;
        let maps = obj
            .entry("maps")
            .or_insert_with(|| json!({}))
            .as_object_mut()
            .ok_or_else(|| be_err("common_realm_data_corrupt", ""))?;
        maps.insert("filterLang".to_string(), json!(enabled));
        if let Some(t) = target {
            maps.insert("filterLangTarget".to_string(), json!(t));
        }
        obj.insert("updateAt".to_string(), json!(now_date()));
    }
    write_realm_meta(&id, &realm)?;
    let cfg = read_config().ok_or_else(|| be_err("common_not_initialized", ""))?;
    if cfg.realms.active.as_deref() == Some(id.as_str()) {
        guard_vcmi_not_running()?;
        rebuild_realm_maps_valve(&id)?;
    }
    Ok(())
}

// ── 列举 / 引入 / 上传 / 移动 / 删除 ────────────────────────────────

/// 列出领域专属地图：重建 index.json 并返回条目 + 语言过滤配置。
#[tauri::command]
pub fn list_realm_maps(realm_id: String) -> Value {
    let id = VcmiPaths::sanitize_realm_id(&realm_id);
    let entries = rebuild_index(&id).unwrap_or_default();
    let snapshot: Vec<Value> = entries
        .iter()
        .map(|e| {
            json!({
                "file": e.get("file").cloned().unwrap_or(Value::Null),
                "kind": e.get("kind").cloned().unwrap_or(json!("map")),
                "lang": e.get("lang").cloned().unwrap_or(json!("unknown")),
                "size": e.get("size").cloned().unwrap_or(json!(0)),
            })
        })
        .collect();
    sync_realm_maps_snapshot(&id, &snapshot);
    let (filter_lang, filter_target) = realm_map_filter(&id);
    json!({
        "entries": entries,
        "filterLang": filter_lang,
        "filterLangTarget": filter_target,
    })
}

/// 引入报告（dry_run 预检 / 确认引入共用）。
fn import_report(
    total: usize,
    added: usize,
    skipped: usize,
    failed: usize,
    by_kind: Value,
    by_lang: Value,
    conflicts: Vec<String>,
    files: Vec<Value>,
) -> Value {
    json!({
        "total": total,
        "added": added,
        "skipped": skipped,
        "failed": failed,
        "byKind": by_kind,
        "byLang": by_lang,
        "conflicts": conflicts,
        "files": files,
    })
}

/// 从目录（或含单文件的目录）引入地图：两段式。
/// - `dry_run=true`：只读预检，返回数量/类型/语言分布/冲突清单，不改任何文件；
/// - `dry_run=false`：按 `conflict`（skip/overwrite/rename）实际复制入库。
/// 保留原始相对目录结构（`keep_structure=false` 时拍平进语言目录根）。
#[tauri::command]
pub fn import_realm_maps_dir(
    realm_id: String,
    src_dir: String,
    keep_structure: bool,
    conflict: String,
    dry_run: bool,
) -> Result<Value, String> {
    let id = VcmiPaths::sanitize_realm_id(&realm_id);
    let src = PathBuf::from(&src_dir);
    if !src.is_dir() {
        return Err(be_err("realm_maps_import_src_missing", ""));
    }
    if !matches!(conflict.as_str(), "skip" | "overwrite" | "rename") {
        return Err(be_err("realm_maps_import_conflict", ""));
    }
    let repo = repo_dir(&id)?;

    // 收集源目录内全部地图文件
    let mut found: Vec<(String, PathBuf)> = Vec::new();
    collect_map_files(&src, &mut found);
    found.sort_by(|a, b| a.0.cmp(&b.0));

    let mut by_kind: std::collections::BTreeMap<String, u32> = Default::default();
    let mut by_lang: std::collections::BTreeMap<String, u32> = Default::default();
    let mut conflicts: Vec<String> = Vec::new();
    let mut plan: Vec<(PathBuf, PathBuf, Value)> = Vec::new();

    for (rel, abs) in &found {
        let ext = abs
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("h3m")
            .to_lowercase();
        let kind_dir = kind_by_ext(&ext);
        let meta = h3m::parse_file(abs).ok();
        let lang = meta
            .as_ref()
            .map(|m| m.lang.clone())
            .unwrap_or_else(|| "unknown".into());
        *by_kind.entry(kind_dir.to_string()).or_insert(0) += 1;
        *by_lang.entry(lang.clone()).or_insert(0) += 1;

        let rel_in_dest = if keep_structure {
            rel.clone()
        } else {
            abs.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| rel.clone())
        };
        let dest = repo.join(kind_dir).join(&lang).join(&rel_in_dest);
        let dest_repo_rel = format!("{kind_dir}/{lang}/{rel_in_dest}");
        let is_conflict = dest.exists();
        if is_conflict {
            conflicts.push(dest_repo_rel.clone());
        }
        let dest_meta = match &meta {
            Some(m) => meta_to_entry(&dest_repo_rel, &dest, m.clone()),
            None => fallback_entry(&dest_repo_rel, &dest, kind_dir),
        };
        plan.push((abs.clone(), dest, dest_meta));
    }

    if dry_run {
        let files = plan
            .iter()
            .map(|(s, _, m)| {
                json!({
                    "rel": s.file_name().and_then(|n| n.to_str()).unwrap_or(""),
                    "dest": m.get("file").cloned().unwrap_or(Value::Null),
                    "kind": m.get("kind").cloned().unwrap_or(json!("map")),
                    "lang": m.get("lang").cloned().unwrap_or(json!("unknown")),
                    "conflict": m.get("file").and_then(|f| f.as_str()).map(|f| conflicts.iter().any(|c| c == f)).unwrap_or(false),
                })
            })
            .collect::<Vec<_>>();
        let by_kind_v = Value::Object(by_kind.into_iter().map(|(k, v)| (k, json!(v))).collect());
        let by_lang_v = Value::Object(by_lang.into_iter().map(|(k, v)| (k, json!(v))).collect());
        return Ok(import_report(
            plan.len(),
            0,
            0,
            0,
            by_kind_v,
            by_lang_v,
            conflicts,
            files,
        ));
    }

    // 实际复制
    let mut added = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;
    for (src_abs, dest, _meta) in &plan {
        let mut target = dest.clone();
        if dest.exists() {
            match conflict.as_str() {
                "skip" => {
                    skipped += 1;
                    continue;
                }
                "rename" => {
                    // dest-2.ext / dest-3.ext
                    let stem = dest
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    let ext = dest
                        .extension()
                        .map(|e| format!(".{}", e.to_string_lossy()))
                        .unwrap_or_default();
                    let mut n = 2;
                    loop {
                        let cand = dest.with_file_name(format!("{stem}-{n}{ext}"));
                        if !cand.exists() {
                            target = cand;
                            break;
                        }
                        n += 1;
                    }
                }
                _ => {} // overwrite：直接覆盖
            }
        }
        if let Some(parent) = target.parent() {
            if fs::create_dir_all(parent).is_err() {
                failed += 1;
                continue;
            }
        }
        match fs::copy(src_abs, &target) {
            Ok(_) => added += 1,
            Err(_) => failed += 1,
        }
    }

    let entries = rebuild_index(&id)?;
    sync_realm_maps_snapshot(&id, &entries);
    // 激活领域即时重建可视目录
    let cfg = read_config();
    if cfg.as_ref().and_then(|c| c.realms.active.as_deref()) == Some(id.as_str()) {
        let _ = rebuild_realm_maps_valve(&id);
    }
    let by_kind_v = Value::Object(by_kind.into_iter().map(|(k, v)| (k, json!(v))).collect());
    let by_lang_v = Value::Object(by_lang.into_iter().map(|(k, v)| (k, json!(v))).collect());
    Ok(import_report(
        plan.len(),
        added,
        skipped,
        failed,
        by_kind_v,
        by_lang_v,
        conflicts,
        Vec::new(),
    ))
}

/// 上传单个地图文件：解析 → 归档语言目录 → 同步索引（自动避免重名）。
#[tauri::command]
pub fn upload_realm_map(realm_id: String, src: String) -> Result<Value, String> {
    let id = VcmiPaths::sanitize_realm_id(&realm_id);
    let src_path = PathBuf::from(&src);
    if !src_path.is_file() {
        return Err(be_err("realm_import_src_missing", ""));
    }
    let ext = src_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("h3m")
        .to_lowercase();
    if !MAP_EXTS.iter().any(|x| x.eq_ignore_ascii_case(&ext)) {
        return Err(be_err("realm_map_format_unsupported", &format!("{}", ext)));
    }
    let kind_dir = kind_by_ext(&ext);
    let meta = h3m::parse_file(&src_path).ok();
    let lang = meta.map(|m| m.lang).unwrap_or_else(|| "unknown".into());
    let stem = src_path
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty() && !s.contains(".."))
        .unwrap_or("map");

    let repo = repo_dir(&id)?;
    let mut rel = format!("{kind_dir}/{lang}/{stem}.{ext}");
    let mut target = repo.join(kind_dir).join(&lang).join(format!("{stem}.{ext}"));
    let mut n = 2;
    while target.exists() {
        rel = format!("{kind_dir}/{lang}/{stem}-{n}.{ext}");
        target = repo.join(kind_dir).join(&lang).join(format!("{stem}-{n}.{ext}"));
        n += 1;
    }
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).map_err(|e| be_err("realm_repo_dir_create_failed", &format!("{}", e)))?;
    }
    fs::copy(&src_path, &target).map_err(|e| be_err("realm_map_copy_failed", &format!("{}", e)))?;

    let entries = rebuild_index(&id)?;
    sync_realm_maps_snapshot(&id, &entries);
    let cfg = read_config();
    if cfg.as_ref().and_then(|c| c.realms.active.as_deref()) == Some(id.as_str()) {
        let _ = rebuild_realm_maps_valve(&id);
    }
    let entry = entries
        .into_iter()
        .find(|e| e.get("file").and_then(|f| f.as_str()) == Some(rel.as_str()))
        .unwrap_or_else(|| json!({ "file": rel, "kind": kind_dir, "lang": lang }));
    Ok(entry)
}

/// 删除领域地图：`file` 为仓库内相对路径（如 `maps/chinese/x/a.h3m`），路径穿越防护。
#[tauri::command]
pub fn delete_realm_map(realm_id: String, file: String) -> Result<(), String> {
    if file.is_empty() || file.contains("..") || file.contains('\\') {
        return Err(be_err("realm_map_delete_invalid", ""));
    }
    if !file.starts_with("maps/") && !file.starts_with("campaigns/") {
        return Err(be_err("realm_map_delete_partition", ""));
    }
    let id = VcmiPaths::sanitize_realm_id(&realm_id);
    let repo = repo_dir(&id)?;
    let p = repo.join(&file);
    // 规范化校验：解析后仍须位于仓库内
    let canon = p
        .canonicalize()
        .map_err(|_| be_err("realm_map_delete_not_found", ""))?;
    let repo_canon = repo.canonicalize().map_err(|e| be_err("realm_generic_arg", &format!("{}", e)))?;
    if !canon.starts_with(&repo_canon) {
        return Err(be_err("realm_map_delete_invalid", ""));
    }
    if !canon.is_file() {
        return Err(be_err("realm_map_delete_not_found", ""));
    }
    fs::remove_file(&canon).map_err(|e| be_err("realm_map_delete_failed", &format!("{}", e)))?;
    // 清掉变空的父目录（保留仓库根结构）
    let mut dir = canon.parent().map(|d| d.to_path_buf());
    while let Some(d) = dir {
        if d == repo || d == repo.join("maps") || d == repo.join("campaigns") {
            break;
        }
        if fs::read_dir(&d).map(|mut rd| rd.next().is_none()).unwrap_or(false) {
            let _ = fs::remove_dir(&d);
            dir = d.parent().map(|x| x.to_path_buf());
        } else {
            break;
        }
    }

    let entries = rebuild_index(&id)?;
    sync_realm_maps_snapshot(&id, &entries);
    let cfg = read_config();
    if cfg.as_ref().and_then(|c| c.realms.active.as_deref()) == Some(id.as_str()) {
        let _ = rebuild_realm_maps_valve(&id);
    }
    Ok(())
}

/// 移动领域地图到其他目录：`file` 为仓库内相对路径，`dest_dir` 为仓库内目标目录
/// （如 `maps/english` 或 `maps/english/子目录`）。禁止地图 ↔ 战役跨类型移动。
#[tauri::command]
pub fn move_realm_map(realm_id: String, file: String, dest_dir: String) -> Result<Value, String> {
    let clean = |s: &str| -> Result<String, String> {
        let s = s.trim_matches('/');
        if s.is_empty() || s.contains("..") || s.contains('\\') || s.contains("//") {
            return Err(be_err("realm_map_move_invalid", ""));
        }
        Ok(s.to_string())
    };
    let file = clean(&file)?;
    let dest_dir = clean(&dest_dir)?;
    let src_kind = file.split('/').next().unwrap_or("maps");
    let dst_kind = dest_dir.split('/').next().unwrap_or("maps");
    if src_kind != dst_kind || !matches!(src_kind, "maps" | "campaigns") {
        return Err(be_err("realm_map_move_cross_kind", ""));
    }

    let id = VcmiPaths::sanitize_realm_id(&realm_id);
    let repo = repo_dir(&id)?;
    let src = repo.join(&file);
    let name = src
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .ok_or_else(|| be_err("realm_map_move_invalid", ""))?;
    let dest_parent = repo.join(&dest_dir);
    let mut dest = dest_parent.join(&name);
    if !src.is_file() {
        return Err(be_err("realm_map_delete_not_found", ""));
    }
    if dest.exists() {
        let stem = src
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let ext = src
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        let mut n = 2;
        loop {
            let cand = dest_parent.join(format!("{stem}-{n}{ext}"));
            if !cand.exists() {
                dest = cand;
                break;
            }
            n += 1;
        }
    }
    fs::create_dir_all(&dest_parent).map_err(|e| be_err("realm_target_dir_create_failed", &format!("{}", e)))?;
    fs::rename(&src, &dest).map_err(|e| be_err("realm_map_move_failed", &format!("{}", e)))?;
    let dest_rel = dest
        .strip_prefix(&repo)
        .map(|r| r.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();

    let entries = rebuild_index(&id)?;
    sync_realm_maps_snapshot(&id, &entries);
    let cfg = read_config();
    if cfg.as_ref().and_then(|c| c.realms.active.as_deref()) == Some(id.as_str()) {
        let _ = rebuild_realm_maps_valve(&id);
    }
    Ok(json!({ "file": dest_rel }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `merge` 动作的核心搬运逻辑：递归搬子目录 + 重名 `-2` 递增避让 + 搬空源目录。
    /// 这是本次唯一会**搬动玩家真实地图文件**的新代码，必须用测试锁住行为。
    #[test]
    fn move_dir_contents_merges_and_renames() {
        let root = std::env::temp_dir().join(format!("vcmi_fv_move_dir_contents_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let src = root.join("src");
        let dst = root.join("dst");
        fs::create_dir_all(src.join("nested")).unwrap();
        fs::create_dir_all(&dst).unwrap();
        fs::write(src.join("a.h3m"), b"new").unwrap();
        fs::write(dst.join("a.h3m"), b"old").unwrap();
        fs::write(src.join("nested").join("b.h3m"), b"b").unwrap();

        move_dir_contents(&src, &dst).unwrap();

        // 重名避让：目标原有内容必须保留，源落在 `-2` 后缀上
        assert_eq!(fs::read(dst.join("a.h3m")).unwrap(), b"old");
        assert_eq!(fs::read(dst.join("a-2.h3m")).unwrap(), b"new");
        // 子目录递归搬过去
        assert!(dst.join("nested").join("b.h3m").is_file());
        // 源被搬空（`merge` 随后据此删除真实 Maps 目录）
        assert!(fs::read_dir(&src).unwrap().next().is_none());

        let _ = fs::remove_dir_all(&root);
    }
}
