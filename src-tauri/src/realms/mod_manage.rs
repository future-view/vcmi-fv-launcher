//! 领域模组管理（`realm-mods-management` §10.1）：已实装列表 / 引用 / 取消引用 / 整合 /
//! 删除 / 导入。
//!
//! 语义（v3 定稿，`docs/features/realms.zh-CN.md` §3/§7.8）：
//! - `borrowed` = **领域引用清单**：只申明依赖哪些「非领域目录」的模组（user/game 源混合）。
//!   引用动作**纯台账申报、零物理动作**；实际挂载（user 源建目录联接，领域隔离后 VCMI
//!   读不到用户目录）只发生在**激活领域**时（`links::apply_realm_mods`）；game 源天然可见、
//!   永不建链。
//! - `bundled` = 整合副本（复制到领域 `Mods/`，随 `.vcmirealm` 分发）。
//! - `pinned` = 领域私有副本台账（下载 / 导入所得，**永不被激活清理**）。
//!
//! 落盘模型（§7.8）：**引用 / 取消引用 = 纯台账申报**（随「保存领域」落盘）；内容实体
//! 动作（整合 / 删除副本 / 导入）即时执行。本模块不写 `realm.json`
//! （`pinned/bundled/borrowed` 的最终值由前端随 `update_realm` 写入）。
//!
//! 安全红线（`infra::links`）：删除联接只能用 `remove_dir`（经 `remove_link`），
//! 绝不 `remove_dir_all`；命令入参目录名一律经 `safe_disk_name` 护栏（防目录穿越）。

use crate::infra::berror::be_err;
use super::base::{guard_vcmi_not_running, now_date, read_config};
use super::conflicts::dir_fingerprint;
use super::meta::get_realm;

use serde_json::{json, Value};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use tauri::State;

use crate::infra::links::{copy_dir_all, ensure_link, is_link, remove_link};
use crate::state::AppPaths;
use crate::vcmi::paths::VcmiPaths;
use crate::vcmi::{mod_install, scan};

/// 命令公共前奏：领域 Mods 目录 + 全局来源目录（user / game）。
struct RealmModEnv {
    realm_mods: PathBuf,
    user_mods: PathBuf,
    game_mods: PathBuf,
}

impl RealmModEnv {
    fn resolve(state: &State<AppPaths>, realm_id: &str) -> Result<Self, String> {
        let id = VcmiPaths::sanitize_realm_id(realm_id);
        let base = VcmiPaths::realm_vcmi_dir(&id)
            .ok_or_else(|| be_err("common_data_dir_unknown", ""))?;
        let (user_mods, game_mods) = {
            let p = state.0.lock().expect("paths lock poisoned");
            (p.user_mods_dir(), p.game_mods_dir())
        };
        Ok(Self {
            realm_mods: base.join("Mods"),
            user_mods,
            game_mods,
        })
    }

    /// 解析模组来源（user 优先，对齐 `listInstalledMods` / `apply_realm_mods` 口径）。
    fn source_of(&self, id: &str) -> Option<(&'static str, PathBuf)> {
        let u = self.user_mods.join(id);
        if u.is_dir() {
            return Some(("user", u));
        }
        let g = self.game_mods.join(id);
        if g.is_dir() {
            return Some(("game", g));
        }
        None
    }
}

fn str_list(v: Option<&Value>) -> Vec<String> {
    v.and_then(|a| a.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// `pinned` 台账取 id（条目形态 `[{id,…}]`，宽容纯字符串数组的手工写法）。
fn pinned_ids(mods: &Value) -> Vec<String> {
    match mods.get("pinned") {
        Some(Value::Array(items)) => items
            .iter()
            .map(|x| match x {
                Value::String(s) => s.clone(),
                obj => obj
                    .get("id")
                    .and_then(|i| i.as_str())
                    .unwrap_or_default()
                    .to_string(),
            })
            .filter(|s| !s.is_empty())
            .collect(),
        _ => Vec::new(),
    }
}

fn has_id(list: &[String], id: &str) -> bool {
    list.iter().any(|x| x.eq_ignore_ascii_case(id))
}

/// 目录名护栏：拒绝路径分隔符 / 盘符 / 通配符 / `..` / 点开头（防穿越 + 防扫描穿透）。
fn safe_disk_name(name: &str) -> Result<String, String> {
    let t = name.trim();
    let bad = t.is_empty()
        || t.starts_with('.')
        || t.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|'])
        || t.contains("..");
    if bad {
        return Err(be_err("realm_illegal_mod_dir_name", &format!("{}", name)));
    }
    Ok(t.to_string())
}

/// 从模组目录读 `mod.json` 的展示元数据（`{name, version}`）。
fn dir_mod_meta(dir: &Path) -> (String, String) {
    let text = fs::read_to_string(dir.join("mod.json")).unwrap_or_default();
    let v: Value = crate::infra::json::parse_json_lenient(&text).unwrap_or(Value::Null);
    let ver = v
        .get("version")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    // name 可能是多语言块对象（VCMI 1.7）：回退链 english → chinese
    let name = v
        .get("name")
        .and_then(|x| x.as_str())
        .map(str::to_string)
        .or_else(|| {
            v.get("name").and_then(|x| x.as_object()).and_then(|o| {
                o.get("english")
                    .or_else(|| o.get("chinese"))
                    .and_then(|x| x.as_str())
                    .map(str::to_string)
            })
        })
        .unwrap_or_default();
    (name, ver)
}

/// 把领域扫描节点（含 `subs` 子树）递归展开为**扁平条目**，并为整棵树注入 realm 字段
/// （`isLink` / `hasLedger` / `source` / `in*` / `metaOk`）。
///
/// - 展开口径与 `push_installed_mod`（全局已安装列表）一致：子模组各自成条，
///   携 `id`（`parent.child`）/`parentId`/`isSubmod`，前端据此建树、勾选到子模组粒度。
/// - realm 字段是**根目录级事实**（子模组随整个目录一起联接 / 复制，台账以根为单位登记），
///   因此整棵子树共享同一份字段。
/// - `seen` 按小写 id 去重（跨大小写）；条目去掉 `subs` 嵌套负载，避免重复传输子树。
fn push_realm_mod_flat(
    m: &Value,
    fields: &serde_json::Map<String, Value>,
    seen: &mut HashSet<String>,
    out: &mut Vec<Value>,
) {
    let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string();
    if id.is_empty() || !seen.insert(id.to_lowercase()) {
        return;
    }
    let mut obj = m.as_object().cloned().unwrap_or_default();
    obj.remove("subs");
    for (k, v) in fields {
        obj.insert(k.clone(), v.clone());
    }
    out.push(Value::Object(obj));
    if let Some(subs) = m.get("subs").and_then(|v| v.as_array()) {
        for s in subs {
            push_realm_mod_flat(s, fields, seen, out);
        }
    }
}

/* ----------------------------- 已实装 ----------------------------- */

/// 领域已实装模组列表（`realm-mods-management` §5.2.1）：领域目录扫描 ∪ 台账补漏。
///
/// 每条在 `scan_mod_dir` 完整元数据之外附加：
/// - `isLink`：目录联接（user 源引用）/ 真实目录（领域位置）；
/// - `hasLedger`：`.fv-install.json` 安装台账（下载 / 导入所得）；
/// - `source`：`realm`（领域位置）/ `user` / `game`（引用来源）；
/// - `inBorrowed` / `inBundled` / `inPinned`：是否在 `realm.json` 对应台账中；
/// - `metaOk`：能否解析到 `mod.json`（false = 引用缺失，仅有 id，供「缺失 N」展示）。
///
/// **子模组递归展开为扁平条目**（同 `push_installed_mod` 口径）：预设视图的父子树、
/// 启用组合（领域 modSettings）的子模组粒度都依赖它——只返回顶层时前端拿不到子模组，
/// 右侧栏树永远缺子节点、保存预设也无法为子模组写 `settings`。
#[tauri::command]
pub fn list_realm_mods(state: State<AppPaths>, realm_id: String) -> Result<Value, String> {
    let env = RealmModEnv::resolve(&state, &realm_id)?;
    let id = VcmiPaths::sanitize_realm_id(&realm_id);
    let mods_val = get_realm(&id)
        .map(|r| r.get("mods").cloned().unwrap_or(Value::Null))
        .unwrap_or(Value::Null);
    let borrowed = str_list(mods_val.get("borrowed"));
    let bundled = str_list(mods_val.get("bundled"));
    let pinned = pinned_ids(&mods_val);

    let mut out: Vec<Value> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new(); // 小写 id 集合（含子模组）

    // 1) 领域目录物理条目：真实目录 = 领域位置；目录联接 = user 源引用
    for m in scan::scan_user_mods_dir(&env.realm_mods) {
        let disk = m
            .get("diskName")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let path = env.realm_mods.join(&disk);
        let link = is_link(&path);
        let fields = json!({
            "isLink": link,
            "hasLedger": path.join(mod_install::LEDGER_FILE).is_file(),
            "source": if link { "user" } else { "realm" },
            "inBorrowed": has_id(&borrowed, &disk),
            "inBundled": has_id(&bundled, &disk),
            "inPinned": has_id(&pinned, &disk),
            "metaOk": true,
        });
        push_realm_mod_flat(&m, fields.as_object().expect("fields object"), &mut seen, &mut out);
    }

    // 2) 台账补漏：game 源引用（天然可见不建链，不在领域目录）；未激活 / 未同步的
    //    user 源引用（v3 纯登记模型下引用无物理痕迹，靠台账呈现）；
    //    以及已丢失的引用（metaOk=false，供缺失展示与取消引用清理）
    let mut rest: Vec<String> = Vec::new();
    for group in [&borrowed, &bundled, &pinned] {
        for gid in group {
            if !seen.contains(&gid.to_lowercase()) && !has_id(&rest, gid) {
                rest.push(gid.clone());
            }
        }
    }
    for gid in rest {
        let src = env.source_of(&gid);
        let mut m = match &src {
            Some((_, dir)) => scan::scan_mod_dir(dir, None),
            None => json!({
                "id": gid, "diskName": gid, "name": gid, "desc": "", "type": "Other",
                "rawType": Value::Null, "version": Value::Null, "author": Value::Null,
                "depends": [], "softDepends": [], "conflicts": [], "factions": [],
                "subs": [], "isSubmod": false, "keepDisabled": false,
            }),
        };
        // id 以台账为准（与 realm.json 的 id 匹配，防磁盘名大小写/空格差异）
        m["id"] = json!(gid);
        m["diskName"] = json!(gid);
        let source = if has_id(&bundled, &gid) || has_id(&pinned, &gid) {
            "realm"
        } else {
            src.as_ref().map(|(s, _)| *s).unwrap_or("user")
        };
        let fields = json!({
            "isLink": false,
            "hasLedger": false,
            "source": source,
            "inBorrowed": has_id(&borrowed, &gid),
            "inBundled": has_id(&bundled, &gid),
            "inPinned": has_id(&pinned, &gid),
            "metaOk": src.is_some(),
        });
        push_realm_mod_flat(&m, fields.as_object().expect("fields object"), &mut seen, &mut out);
    }

    Ok(json!({ "mods": out }))
}

/* ----------------------------- 引用 / 取消引用 ----------------------------- */

/// 批量引用（§5.2.2 `引用` / `全部引用`）：**纯台账申报**——只校验来源存在性，
/// 不建任何链接、不碰领域目录。
///
/// v3 模型（§7.8）：user 源的目录联接由**激活领域**时 `links::apply_realm_mods`
/// 按台账统一建立（未激活时领域目录零物理痕迹）；game 源天然可见、永不建链。
///
/// 返回 `{results: [{id, ok, source?, reason?}]}`；缺失（user/game 均无）时
/// `ok=false, reason="missing"`（不中断其余条目）。
#[tauri::command]
pub fn realm_mod_ref(
    state: State<AppPaths>,
    realm_id: String,
    mod_ids: Vec<String>,
) -> Result<Value, String> {
    let env = RealmModEnv::resolve(&state, &realm_id)?;

    let mut results: Vec<Value> = Vec::new();
    for raw in mod_ids {
        let mid = raw.trim().to_string();
        if mid.is_empty() {
            continue;
        }
        let Some((source, _)) = env.source_of(&mid) else {
            results.push(json!({ "id": mid, "ok": false, "reason": "missing" }));
            continue;
        };
        results.push(json!({ "id": mid, "ok": true, "source": source }));
    }
    Ok(json!({ "results": results }))
}

/// 批量取消引用（§7.1）：台账移除（由前端随保存落盘）；**仅当领域处于激活状态**时
/// 才同步删除对应目录联接（v3 模型：物理挂载只属于激活域）。
///
/// 未激活时领域目录本无联接（引用纯登记），无物理动作；game 源同样无物理动作。
/// 真实目录**绝不删除**（防御：若该 id 已转为领域位置副本，取消引用不应动副本）。
#[tauri::command]
pub fn realm_mod_unref(
    state: State<AppPaths>,
    realm_id: String,
    mod_ids: Vec<String>,
) -> Result<Value, String> {
    guard_vcmi_not_running()?;
    let env = RealmModEnv::resolve(&state, &realm_id)?;
    let id = VcmiPaths::sanitize_realm_id(&realm_id);
    // 激活态判定：只有当前领域激活中（物理联接真实存在）才需要删链
    let active = read_config()
        .and_then(|c| c.realms.active)
        .map(|a| a == id)
        .unwrap_or(false);

    let mut results: Vec<Value> = Vec::new();
    for raw in mod_ids {
        let mid = raw.trim().to_string();
        if mid.is_empty() {
            continue;
        }
        let dst = env.realm_mods.join(&mid);
        let mut removed = false;
        if active && is_link(&dst) {
            remove_link(&dst)?;
            removed = true;
        }
        results.push(json!({ "id": mid, "ok": true, "removed": removed }));
    }
    Ok(json!({ "results": results }))
}

/// 确保领域在当前语言下**能看到**匹配的语言包（§语言层：启动器自有，不进领域数据）。
///
/// 语言包是**玩家本地的显示语言需求**，不属于领域内容，故本命令：
/// - **不写 `realm.json`**、不登记 `borrowed`/`bundled`（只读领域因此也不再有特殊分支）；
/// - 只在必要时于领域隔离目录建一条**临时目录联接**。
///
/// 可见性判定（按优先级，基础语言包 `baseTranslation` 优先）：
/// - 领域目录内**真实目录** → 领域自带（`bundled` 复制 / `pinned` 私有副本），已可见，`source="realm"`；
/// - 领域目录内**目录联接** → 既有引用，已可见，`source="user"`；
/// - 游戏目录 → 天然可见（永不建链），`source="game"`；
/// - 领域**未隔离**（`dirs.dataPath=global`）→ 全局 user 目录即其可见来源，零动作。
/// - 领域**隔离**且只在全局 user 目录 → 建 junction（`{realm}/vcmi/Mods/<id>`），`linked=true`。
///
/// **必须先扫领域自身 Mods**：`scan_mods` 按 `AppPaths` 只扫「全局 user + 游戏目录」，看不到领域
/// 自身的 Mods；漏掉会把领域自带的包误判为"缺失"、重复装一份全局包。
///
/// ⚠️ 该临时联接**不落台账**，会被下次激活的 `apply_realm_mods` 清理——这是刻意的：语言层与
/// 领域数据完全解耦，每次「启动前把关」重建即可（幂等、毫秒级）。
///
/// **只确认可见、不注入预设**：生效预设可能是 `__realm__` 也可能是玩家自建的其它预设（领域支持
/// 多预设），解析口径由前端 `syncLanguageMods` 统一负责（`activePreset → presetId → __realm__`）。
///
/// 返回 `{found, id, source, linked}`；`linked=true` 表示本次**新建立**了联接（此前对该领域不可见），
/// 前端据此把它自动并入生效预设（此前不可见 = 不存在"用户显式关闭"一说）。
#[tauri::command]
pub fn ensure_realm_lang_visible(
    state: State<AppPaths>,
    realm_id: String,
    vcmi_lang: String,
) -> Result<Value, String> {
    let id = VcmiPaths::sanitize_realm_id(&realm_id);
    let env = RealmModEnv::resolve(&state, &id)?;

    // 数据隔离是否生效（`dirs.dataPath=realm`，默认）：未隔离时全局 user 目录即领域可见来源，无需建链
    let isolated = get_realm(&id)?
        .get("dirs")
        .and_then(|d| d.get("dataPath"))
        .and_then(|v| v.as_str())
        != Some("global");

    // 1) 收集候选：**必须先看领域自身 Mods**——领域自带（bundled 复制 / pinned 私有副本）的语言包
    //    对全局扫描不可见，漏掉会把自带的包误判成"缺失"并重复安装一份全局包。
    //    rank：0 = 无需借用（领域自带真实目录 / 已借用联接 / game 源天然可见）；1 = 需借用（全局 user）。
    struct Cand {
        id: String,
        base: bool,
        rank: u8,
        source: &'static str,
    }
    let mut cands: Vec<Cand> = Vec::new();
    let collect = |dir: &Path, src: &'static str, rank: u8, in_realm: bool, out: &mut Vec<Cand>| {
        for m in scan::scan_user_mods_dir(dir) {
            if m.get("rawType").and_then(|v| v.as_str()) != Some("Translation") {
                continue;
            }
            if m.get("language").and_then(|v| v.as_str()).unwrap_or("") != vcmi_lang {
                continue;
            }
            let Some(mid) = m.get("id").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) else {
                continue;
            };
            // 领域目录内：联接 = 借用（user 源，已可见）；真实目录 = 领域自带（已可见）
            let (source, rank) = if in_realm {
                if is_link(&dir.join(mid)) {
                    ("user", 0u8)
                } else {
                    ("realm", 0u8)
                }
            } else {
                (src, rank)
            };
            out.push(Cand {
                id: mid.to_string(),
                base: m.get("baseTranslation").and_then(|v| v.as_bool()).unwrap_or(false),
                rank,
                source,
            });
        }
    };
    collect(&env.realm_mods, "realm", 0, true, &mut cands);
    collect(&env.game_mods, "game", 0, false, &mut cands);
    collect(&env.user_mods, "user", 1, false, &mut cands);

    if cands.is_empty() {
        return Ok(json!({ "found": false }));
    }
    // 优先基础语言包（真正翻译游戏本体的那个）；其次"无需借用"的来源，避免不必要的安装/借用
    cands.sort_by_key(|c| (!c.base, c.rank));
    let chosen = &cands[0];
    let chosen_id = chosen.id.clone();
    let source = chosen.source;
    // 2) 需要建链的唯一情形：领域隔离 + 命中全局 user + 领域目录里尚无该 id。
    //    ⚠️ 只在"目录项不存在或已是联接"时动手：若领域目录里是**真实目录**（领域自带副本），
    //    `ensure_link` 会先 remove_dir_all 再重建，会毁掉自带的包。
    //    联接不落台账，会被下次激活的 `apply_realm_mods` 清理——刻意如此，启动前重建即可。
    //    VCMI 运行中不建链（与 `realm_mod_unref` 同护栏）：本命令的调用点在启动前把关。
    let dst = env.realm_mods.join(&chosen_id);
    let mut linked = false;
    if isolated && source == "user" {
        let occupied = fs::symlink_metadata(&dst).is_ok();
        if (!occupied || is_link(&dst)) && guard_vcmi_not_running().is_ok() {
            fs::create_dir_all(&env.realm_mods)
                .map_err(|e| be_err("infra_realm_mods_dir_create_failed", &format!("{}", e)))?;
            ensure_link(&dst, &env.user_mods.join(&chosen_id))?;
            linked = true;
        }
    }

    Ok(json!({
        "found": true,
        "id": chosen_id,
        "source": source,
        "linked": linked,
    }))
}

/* ----------------------------- 整合 / 删除 ----------------------------- */

/// 整合（§5.2.2 `整合` / §9.3.3）：复制模组本体到领域 `Mods/`（入 `bundled`）。
///
/// 冲突检测（`force=false` 时返回 `{ok:false, reason}`，前端提示后带 `force=true` 重试）：
/// - `game`：来源在游戏目录（恒可见）→ 进入领域后与领域副本重复挂载同 ID；
/// - `diverged`：领域内已有副本且目录指纹与来源不一致（同名不同内容）→ 百科缓存会串味；
/// - 指纹一致 → `{ok:true, skipped:"same"}`（已整合过，无需重复复制）。
#[tauri::command]
pub fn realm_mod_bundle(
    state: State<AppPaths>,
    realm_id: String,
    mod_id: String,
    force: bool,
) -> Result<Value, String> {
    guard_vcmi_not_running()?;
    let env = RealmModEnv::resolve(&state, &realm_id)?;
    let mid = safe_disk_name(&mod_id)?;
    let Some((source, src)) = env.source_of(&mid) else {
        return Err(be_err("realm_mod_not_in_dirs", &format!("{}", mid)));
    };
    let dst = env.realm_mods.join(&mid);
    fs::create_dir_all(&env.realm_mods).map_err(|e| be_err("infra_realm_mods_dir_create_failed", &format!("{}", e)))?;
    let dst_is_link = is_link(&dst);
    let dst_is_real = !dst_is_link && dst.symlink_metadata().is_ok();

    if !force {
        if source == "game" {
            return Ok(json!({ "ok": false, "modId": mid, "source": source, "reason": "game" }));
        }
        if dst_is_real {
            let same = matches!(
                (dir_fingerprint(&src), dir_fingerprint(&dst)),
                (Ok(a), Ok(b)) if a == b
            );
            if same {
                return Ok(json!({ "ok": true, "modId": mid, "source": source, "skipped": "same" }));
            }
            return Ok(json!({ "ok": false, "modId": mid, "source": source, "reason": "diverged" }));
        }
    }

    // 执行：清链接 → 清旧副本 → 复制本体
    if dst_is_link {
        remove_link(&dst)?;
    }
    if dst_is_real {
        fs::remove_dir_all(&dst)
            .map_err(|e| be_err("realm_old_copy_clear_failed", &format!("{}", e)))?;
    }
    copy_dir_all(&src, &dst)?;
    tracing::info!(target: "realm", "领域整合模组：{mid}（来源 {source}）");
    Ok(json!({ "ok": true, "modId": mid, "source": source, "replaced": dst_is_real }))
}

/// 删除领域位置副本（§7.2）：`pinned` / `bundled` 的删除动作。
///
/// `kind` 仅作战日志与弱校验（`bundled` / `pinned` / `local`）；实际删除按物理形态：
/// 联接 → `remove_link`（红线：绝不递归）；真实目录 → `remove_dir_all`。
#[tauri::command]
pub fn realm_mod_delete(
    state: State<AppPaths>,
    realm_id: String,
    mod_id: String,
    kind: String,
) -> Result<Value, String> {
    guard_vcmi_not_running()?;
    if !matches!(kind.as_str(), "bundled" | "pinned" | "local") {
        return Err(be_err("realm_unknown_mod_location", &format!("{}", kind)));
    }
    let env = RealmModEnv::resolve(&state, &realm_id)?;
    let mid = safe_disk_name(&mod_id)?;
    let dst = env.realm_mods.join(&mid);
    if dst.symlink_metadata().is_err() {
        return Ok(json!({ "ok": true, "removed": false })); // 已不存在（幂等）
    }
    if is_link(&dst) {
        remove_link(&dst)?;
    } else if dst.is_dir() {
        fs::remove_dir_all(&dst)
            .map_err(|e| be_err("realm_mod_copy_delete_failed", &format!("{}", e)))?;
    } else {
        return Err(be_err("repo_mod_delete_not_dir", ""));
    }
    tracing::info!(target: "realm", "领域删除模组副本：{mid}（{kind}）");
    Ok(json!({ "ok": true, "removed": true }))
}

/* ----------------------------- 导入 ----------------------------- */

/// 把源目录无损替换到目标目录（`.fv-old` 暂存交换；成功删旧、失败回滚）。返回是否替换。
fn install_dir_replace(src: &Path, dest: &Path) -> Result<bool, String> {
    let parent = dest.parent().ok_or_else(|| be_err("common_target_no_parent", ""))?;
    let fname = dest
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let old = parent.join(format!(".fv-old-{fname}"));
    // 清理上次残留
    if is_link(&old) {
        let _ = fs::remove_dir(&old);
    } else if old.is_dir() {
        let _ = fs::remove_dir_all(&old);
    }
    let had = dest.symlink_metadata().is_ok();
    if had {
        fs::rename(dest, &old)
            .map_err(|e| be_err("realm_old_version_stage_failed", &format!("{}", e)))?;
    }
    match copy_dir_all(src, dest) {
        Ok(()) => {
            if had {
                if is_link(&old) {
                    let _ = fs::remove_dir(&old);
                } else {
                    let _ = fs::remove_dir_all(&old);
                }
            }
            Ok(had)
        }
        Err(e) => {
            if had {
                let _ = fs::rename(&old, dest);
            }
            Err(e)
        }
    }
}

/// 导入模组文件到领域（§5.2.1 `导入模组文件` / §7.4）：zip（自动识别 mod 根）或
/// 「含 `mod.json` 的目录」；落 `{realm}/vcmi/Mods/<id>` 并写导入台账
/// （`.fv-install.json` 的 `source: "import"`，`pinned` 台账条目由前端随保存落盘）。
///
/// 冲突策略：目标已存在且 `overwrite=false` → 该项返回 `{ok:false, exists:true, modId, name, version}`，
/// 前端二次确认（替换 / 跳过）后对需要替换的项以 `overwrite=true` 重调；
/// 替换走无损重装（§11-4）：zip 走 `install_with_ledger` 的 `.fv-old` 交换、目录走 `install_dir_replace`。
#[tauri::command]
pub fn realm_mod_import(
    state: State<AppPaths>,
    realm_id: String,
    paths: Vec<String>,
    overwrite: bool,
) -> Result<Value, String> {
    guard_vcmi_not_running()?;
    let env = RealmModEnv::resolve(&state, &realm_id)?;
    fs::create_dir_all(&env.realm_mods).map_err(|e| be_err("infra_realm_mods_dir_create_failed", &format!("{}", e)))?;

    let mut results: Vec<Value> = Vec::new();
    for raw in paths {
        let p = Path::new(&raw);
        if p.is_dir() {
            // 目录导入：必须含 mod.json（非模组目录拒绝）
            if !p.join("mod.json").is_file() {
                results.push(json!({ "path": raw, "ok": false, "reason": "not-mod" }));
                continue;
            }
            let disk = match p
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .map(|s| safe_disk_name(&s))
            {
                Some(Ok(d)) => d,
                _ => {
                    results.push(json!({ "path": raw, "ok": false, "reason": "bad-name" }));
                    continue;
                }
            };
            let dst = env.realm_mods.join(&disk);
            let (name, version) = dir_mod_meta(p);
            if dst.symlink_metadata().is_ok() && !overwrite {
                results.push(json!({
                    "path": raw, "ok": false, "exists": true,
                    "modId": disk, "diskName": disk, "name": name, "version": version,
                }));
                continue;
            }
            match install_dir_replace(p, &dst) {
                Ok(replaced) => {
                    let ledger = json!({
                        "source": "import",
                        "name": name,
                        "version": version,
                        "importedAt": now_date(),
                    });
                    let text = serde_json::to_string_pretty(&ledger)
                        .map_err(|e| be_err("realm_ledger_serialize_failed", &format!("{}", e)))?;
                    if let Err(e) = fs::write(dst.join(mod_install::LEDGER_FILE), text) {
                        tracing::warn!(target: "realm", "写入导入台账失败（模组已就位）：{e}");
                    }
                    tracing::info!(target: "realm", "领域导入模组目录：{disk}（替换={replaced}）");
                    results.push(json!({
                        "path": raw, "ok": true, "modId": disk, "diskName": disk,
                        "name": name, "version": version, "replaced": replaced,
                    }));
                }
                Err(e) => results.push(json!({ "path": raw, "ok": false, "reason": e })),
            }
        } else if p.is_file() {
            // zip 导入：自动识别 mod 根 + 校验 mod.json
            let info = match mod_install::inspect(p, &env.realm_mods) {
                Ok(i) => i,
                Err(e) => {
                    results.push(json!({ "path": raw, "ok": false, "reason": e }));
                    continue;
                }
            };
            if info.exists && !overwrite {
                results.push(json!({
                    "path": raw, "ok": false, "exists": true,
                    "modId": info.mod_id, "diskName": info.mod_id,
                    "name": info.name, "version": info.version,
                }));
                continue;
            }
            let ledger = json!({
                "source": "import",
                "name": info.name,
                "version": info.version,
                "importedAt": now_date(),
            });
            match mod_install::install_with_ledger(p, &env.realm_mods, true, Some(&ledger)) {
                Ok(out) => {
                    tracing::info!(
                        target: "realm",
                        "领域导入模组包：{}（替换={}）",
                        out.mod_id, out.replaced
                    );
                    results.push(json!({
                        "path": raw, "ok": true, "modId": out.mod_id, "diskName": out.mod_id,
                        "name": info.name, "version": info.version,
                        "replaced": out.replaced, "files": out.installed_files,
                    }));
                }
                Err(e) => results.push(json!({ "path": raw, "ok": false, "reason": e })),
            }
        } else {
            results.push(json!({ "path": raw, "ok": false, "reason": "missing" }));
        }
    }
    Ok(json!({ "results": results }))
}


