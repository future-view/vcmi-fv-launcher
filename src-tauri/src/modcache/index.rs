//! 缓存索引（`mods/index.json`）重建与单 mod 缓存读取。
//!
//! - 索引条目由各 mod 的 `mod.json` 摘要 + 扫描期携带的 `diskName`/`parentId`/`type`/`name` 组装；
//! - 索引**以小写归一的 modId 为 key 合并**：分多批构建时互不覆盖（见 `batch.rs`）。
//!
//! 依赖方向：可依赖 `infra` 与 `vcmi`（见 `docs/architecture/backend.zh-CN.md` §4.1）。

use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use crate::modcache::build::build_mod_wiki_inner;
use crate::modcache::global::extract_resource_icons_for_mod;
use crate::modcache::sanitize_id;
use crate::vcmi::paths::VcmiPaths;
use crate::vcmi::resource::{set_wiki_mod_paths, set_wiki_user_data_dir};
use crate::vcmi::scan::cached_mod_paths;

/// 归一 `index.json` 的 `mods` 数组：**按小写归一的 modId 去重**，返回丢弃的条数。
///
/// ## 为什么需要
///
/// 索引是「已缓存清单」，**只增不删**（见 `batch.rs` 的合并说明），而 modId 口径在 2026-10-16
/// 从"磁盘原名"（`1-Knight-town`）改成了"VCMI 口径小写"（`1-knight-town`，
/// 见 [`crate::vcmi::scan::norm_mod_id`]）。索引过去按 **modId 原文** 做 key，口径一变，
/// 同一个 mod 就在索引里留下**两条**（实测本机 652 条 = 573 个真实 mod + 79 条旧口径幽灵条目）。
///
/// 这不是"多一行"那么轻：
/// - 前端把索引与实时扫描求交时用的是小写键（`visibleModKeySet` / `modIdKey`），
///   两种大小写**都能命中** → 城镇 / 英雄 / 建筑（**不做**跨 mod 去重的类目）同一 mod 列两遍；
/// - 每条都要多读一份 cache.json（冷缓存下是 HDD 上实打实的 I/O）。
///
/// ## 规则
///
/// - key = `modId.to_lowercase()`（与 VCMI `to_lower_copy` 同口径，modId 本就是小写形态）；
/// - 同 key 有多条时**保留 modId 已是规范小写的那条**（新口径数据），丢掉旧口径的；两条同级
///   则保留先出现的（位置与相对顺序稳定，不引入随机性）；
/// - 只在**读侧**归一返回值，不改磁盘文件（文件由下一次 `batch.rs` 合并时自然归一）。
pub(crate) fn dedup_index_mods(v: &mut Value) -> usize {
    let Some(mods) = v.get_mut("mods").and_then(|x| x.as_array_mut()) else {
        return 0;
    };
    let before = mods.len();
    let mut keep: Vec<Value> = Vec::with_capacity(before);
    let mut pos: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for m in mods.drain(..) {
        let Some(id) = m.get("modId").and_then(|x| x.as_str()) else {
            continue;
        };
        let key = id.to_lowercase();
        match pos.get(&key) {
            None => {
                pos.insert(key, keep.len());
                keep.push(m);
            }
            // 唯一的替换分支：旧口径（含大写）条目被同 key 的规范小写条目顶掉
            Some(&i) => {
                let prev_id = keep[i].get("modId").and_then(|x| x.as_str()).unwrap_or("");
                if prev_id != prev_id.to_lowercase() && id == key {
                    keep[i] = m;
                }
            }
        }
    }
    let removed = before - keep.len();
    *mods = keep;
    removed
}

/// 读取单个 mod 的缓存 JSON **原文**（带文本缓存，见 `modcache::read_cached_text`）。
///
/// 为什么返回原文而不是 `Value`：`Value` 的深拷贝（1.7 MB 的 cache.json 实测约 33 ms）
/// 与重新解析等价 —— 缓存解析结果毫无收益。返回原文后后端零解析、零深拷贝，
/// 前端 `JSON.parse` 是本来就必需的一步（它要拿对象）。
pub(crate) fn read_mod_wiki_text(
    cache_root: &Path,
    mod_id: &str,
) -> Option<std::sync::Arc<String>> {
    crate::modcache::read_cached_text(
        &cache_root.join(format!("{}/cache.json", sanitize_id(mod_id))),
    )
}

/// [`read_mod_wiki_text`] 的 `{ok, data}` 包装版（**会解析整份 JSON**）。
///
/// 仅供低频调用方（展示页 `showcase`）与测试使用；详情页命令走文本版，
/// 避免为 1.7 MB 的 cache.json 付一次 33 ms 解析。
pub(crate) fn read_mod_wiki_inner(cache_root: &Path, mod_id: &str) -> Value {
    let Some(text) = read_mod_wiki_text(cache_root, mod_id) else {
        return json!({ "ok": false, "error": "notcached" });
    };
    match serde_json::from_str::<Value>(&text) {
        Ok(v) => json!({ "ok": true, "data": v }),
        Err(_) => json!({ "ok": false, "error": "parse" }),
    }
}

/// 原版伪模组（`core` / `core.<faction>`）的**多语言显示名**；非 core 模组返回 `None`。
///
/// 这类伪模组没有 `mod.json`（名称本应由模组作者以多语言块提供），因此名称一律从**既有数据**派生，
/// 不在任何一侧写死语言标签（早期前端 `CORE_FACTION_NAMES` 只写中文 → 英文界面也显示中文，
/// 且中英文同值）：
/// - `core.<faction>`：城镇名优先取**已构建缓存里的阵营名**（`town_names`：游戏
///   `config/factions/*.json` + 各翻译 mod，语言覆盖最全，可含 tchinese/russian/…），
///   缓存没有时回退随包 h3-core 文案表的 `faction.core.<id>.name`（english + chinese，
///   见 [`crate::vcmi::h3core::load_h3_core_i18n`]），前缀语言中立的引擎名；
/// - `core`：引擎名本身（承载原版全局数据：法术/技能/宝物/地形/中立生物）。
///
/// 两个来源都不可用时回退 `factionId` —— **必须返回非空名**：空名会让 wiki 模组列表出现空白行。
pub(crate) fn core_mod_display_name(
    mod_id: &str,
    town_names: Option<&serde_json::Map<String, Value>>,
) -> Option<Value> {
    /// 归属前缀 / 父模组名：语言中立的引擎名（游戏数据里没有"原版"这类可翻译文案）
    const ENGINE: &str = "VCMI";
    let (is_core, faction) = crate::vcmi::h3core::parse_core_mod(mod_id);
    if !is_core {
        return None;
    }
    let Some(faction) = faction else {
        // 引擎名是语言中立的专名、不是译文：只落 `english`，其它语言由前端回退
        return Some(json!({ "english": ENGINE }));
    };
    // 回退来源：随包 h3-core 文案表（只有 en/zh 两种语言）
    let h3_names: Option<serde_json::Map<String, Value>> = crate::vcmi::h3core::h3_core_data_dir()
        .and_then(|d| crate::vcmi::h3core::load_h3_core_i18n(&d))
        .and_then(|table| {
            table
                .get(&format!("faction.core.{}.name", faction.to_lowercase()))
                .cloned()
        })
        .map(|m| m.into_iter().map(|(k, v)| (k, Value::String(v))).collect());
    let mut names = serde_json::Map::new();
    if let Some(src) = town_names.filter(|m| !m.is_empty()).or(h3_names.as_ref()) {
        for (lang, v) in src {
            let s = v.as_str().unwrap_or("");
            if !s.trim().is_empty() {
                names.insert(lang.clone(), json!(format!("{ENGINE} · {s}")));
            }
        }
    }
    if names.is_empty() {
        // 兜底是「引擎名 · factionId」，语言中立（不是中文/英文译文）→ 只落 english
        names.insert(
            "english".to_string(),
            json!(format!("{ENGINE} · {faction}")),
        );
    }
    Some(Value::Object(names))
}

/// 把模组绝对路径拆成 `(来源根标识, 相对该根的路径)` —— 索引落盘用，保证**可迁移**。
///
/// 根标识的取值与语义：
/// - `"game"`：`<game_dir>/Mods`（另含 `launcher.json` 的 `paths.game_mods_dir` 备用项）
/// - `"user"`：`<user_data_dir>/Mods`
/// - `"realm"`：`<领域目录>/vcmi/Mods`（领域内整合/借用的模组，路径段特征 `realms/<id>/vcmi/Mods`）
/// - `"abs"`：未命中任何已知根 —— 相对路径退化为**绝对路径**，信息不丢（只有跨机器迁移时失效）
///
/// 相对路径统一用 `/` 分隔（跨平台可移植）；匹配时取**最长前缀**，嵌套根更具体者胜。
/// `extra_game_mods`：`launcher.json` 的 `paths.game_mods_dir`（便携/单目录安装可能与
/// `game_dir/Mods` 不同）。**由调用方在循环外读一次后传入** —— 该函数对每个模组都要调用，
/// 若在内部读配置，就是每个模组一次「读文件 + 两趟 JSON 解析」（实测 572 次），
/// 属典型的把 O(1) 摆到 O(n) 热路径里。
fn modpath_root_rel(
    mod_path: &Path,
    paths: &VcmiPaths,
    extra_game_mods: Option<&Path>,
) -> (String, String) {
    let mut roots: Vec<(&str, std::path::PathBuf)> = vec![
        ("game", paths.game_mods_dir()),
        ("user", paths.user_mods_dir()),
    ];
    if let Some(gm) = extra_game_mods {
        roots.push(("game", gm.to_path_buf()));
    }
    // ⚠️ 不用 `Path::strip_prefix`：它是**逐组件按 `OsStr` 比较**（大小写敏感），
    // 而这里的两个输入来自不同来源 —— `mod_path` 是 `fs::read_dir` 返回的**真实**路径，
    // 根则来自 `launcher.json` 的配置文本。用户在配置里把盘符/目录名写成另一种大小写
    // 完全正常（`d:/games/H3` vs `D:\Games\h3`），此时 `strip_prefix` 一律失败，
    // 所有模组都会退化成 `abs`。故用"归一化后比前缀 + 按原串切片"的方式，
    // 既能不敏感匹配，又能保留相对部分的**原始大小写**。
    let orig = mod_path.to_string_lossy().replace('\\', "/");
    let lower = orig.to_lowercase();
    let mut best: Option<(usize, &str, String)> = None;
    for (tag, root) in &roots {
        let r = root.to_string_lossy().replace('\\', "/");
        let r = r.trim_end_matches('/');
        if r.is_empty() {
            continue;
        }
        if !lower.starts_with(&r.to_lowercase()) {
            continue;
        }
        // 必须落在组件边界上（`.../Mods` 不能匹配 `.../ModsExtra`）
        let rest = &lower[r.len()..];
        if !rest.is_empty() && !rest.starts_with('/') {
            continue;
        }
        // 小写化在个别 Unicode 字符上会改变字节长度（如 `İ`），此时下面按 `orig` 的同位切片
        // 会落在字符中间 → 直接跳过该根（退化为 `abs`，不影响正确性，只丢可迁移性）。
        if !orig.is_char_boundary(r.len()) {
            continue;
        }
        let rel = orig[r.len()..].trim_start_matches('/').to_string();
        if best.as_ref().map(|(l, _, _)| r.len() > *l).unwrap_or(true) {
            best = Some((r.len(), tag, rel));
        }
    }
    if let Some((_, tag, rel)) = best {
        return (tag.to_string(), rel);
    }
    // 领域内模组：路径段特征 `.../realms/<id>/vcmi/Mods/<相对>`
    if let Some(pos) = lower.find("/vcmi/mods/") {
        if lower[..pos].contains("/realms/") {
            let start = pos + "/vcmi/mods/".len();
            if orig.is_char_boundary(start) {
                return ("realm".to_string(), orig[start..].to_string());
            }
        }
    }
    ("abs".to_string(), orig)
}

/// 批量构建多个城镇 mod 的 Wiki 缓存，并汇总生成 `mods/index.json` 索引（核心逻辑，可测试）。
pub(crate) fn build_all_wiki_inner(mods: &Value, cache_root: &Path) -> Value {
    let paths = VcmiPaths::detect();
    let game_dir = paths.game_dir.clone();
    // 让资源回退解析也能搜到 `user_data_dir/Data`（便携运行原版数据常在此处）
    set_wiki_user_data_dir(&paths.user_dir);
    // HD 素材来源：与 batch 入口同口径注入（否则这个遗留入口的产物缺 HD 幕布/兵种背景）
    //
    // `launcher.json` 只读**一次**并复用：它下面还要供 `modpath_root_rel` 取
    // `paths.game_mods_dir`（每个模组都要），而 `read_launcher_config` 是
    // 「读文件 + 两趟 JSON 解析」——放进逐模组循环就是 572 次徒劳 IO。
    let launcher_cfg = VcmiPaths::read_launcher_config();
    crate::vcmi::hd_edition::setup_from_config(launcher_cfg.as_ref(), &paths.game_dir);
    let extra_game_mods = launcher_cfg
        .as_ref()
        .and_then(|c| c.paths.game_mods_dir.clone());
    // 让资源回退解析也能搜到其它已装模组内的素材（跨模组引用，如子模组 Fen 的精灵被父模组引用）
    set_wiki_mod_paths(&cached_mod_paths(&paths));
    let mut index_mods: Vec<Value> = Vec::new();
    let mut built = 0;
    let mut cached = 0;
    let skipped = 0;
    let mut encrypted = 0;

    // 递归展开 mod 树（含嵌套子 mod），收集所有节点
    fn flatten(node: &Value, out: &mut Vec<Value>) {
        out.push(node.clone());
        if let Some(subs) = node.get("subs").and_then(|x| x.as_array()) {
            for s in subs {
                flatten(s, out);
            }
        }
    }
    let mut nodes: Vec<Value> = Vec::new();
    if let Some(arr) = mods.as_array() {
        for m in arr {
            flatten(m, &mut nodes);
        }
    }
    // 注入原版（Core）伪 mod：从内置 vcmi 目录提取原版 9 城镇/生物/英雄等。
    // `name` 留空 —— 显示名由 `core_mod_display_name` 从数据派生（多语言，见该函数注释）。
    let core_mod_path = game_dir.join("Mods").join("vcmi");
    if core_mod_path.is_dir() {
        nodes.push(json!({
            "id": "core",
            "modPath": core_mod_path.to_string_lossy().to_string(),
            "diskName": "core",
            "parentId": "core",
            "type": "Core",
            "name": "",
            "subs": []
        }));
    }

    for m in &nodes {
        let id = m
            .get("id")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        let mod_path = m
            .get("modPath")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .to_string();
        // 跳过内置 vcmi mod（与原版 core 同源，避免重复构建与空名称条目）
        let norm = mod_path.replace('\\', "/");
        let is_vcmi = id == "vcmi" || norm.ends_with("Mods/vcmi") || norm.ends_with("Mods/vcmi/");
        if id.is_empty() || mod_path.is_empty() || is_vcmi {
            continue;
        }
        // 提取 VCMI 资源图标到该 mod 缓存目录，供前端花费显示使用
        extract_resource_icons_for_mod(&game_dir, cache_root, &id);
        let type_str = m
            .get("type")
            .and_then(|x| x.as_str())
            .unwrap_or("Other")
            .to_string();
        // 尝试为所有 mod 构建（含生物/英雄/法术等类型；无内容时返回 notown/notfound 属预期）
        let res = build_mod_wiki_inner(&mod_path, &id, cache_root, &game_dir);
        if id == "core" {
            tracing::debug!(target: "wiki-core", "core 构建结果：{res}");
        }
        let cc = res.get("contentCount").cloned().unwrap_or(json!({}));
        if res.get("cached").and_then(|x| x.as_bool()).unwrap_or(false) {
            cached += 1;
        } else if res.get("ok").and_then(|x| x.as_bool()).unwrap_or(false) {
            built += 1;
        } else {
            let err = res
                .get("error")
                .and_then(|x| x.as_str())
                .unwrap_or("?")
                .to_string();
            if err == "encrypted" {
                encrypted += 1;
            }
        }
        // 多语言模组名：① 原版伪模组无 mod.json，名称由数据派生（见 `core_mod_display_name`）；
        // ② 其余优先取本 mod 缓存的 modMeta.name（与 mods/realms 页同源）；
        // ③ 缺失时只写 `english`（前端 `pickName` 回退 english）——**不伪造 `chinese`**
        //    副本：那等于把英文冒充成中文译文，也让"哪些语言真的有翻译"失真。
        let town_names = res.pointer("/factions/0/name").and_then(|x| x.as_object());
        let mod_name_val: Value = if let Some(core_name) = core_mod_display_name(&id, town_names) {
            core_name
        } else {
            let meta_name = res
                .get("modMeta")
                .and_then(|x| x.get("name"))
                .and_then(|x| x.as_object());
            let fallback = || {
                let s = m.get("name").and_then(|x| x.as_str()).unwrap_or("");
                json!({ "english": s })
            };
            if let Some(n) = meta_name {
                if !n.is_empty() {
                    Value::Object(n.clone())
                } else {
                    fallback()
                }
            } else {
                fallback()
            }
        };
        // 模组磁盘位置拆成「来源根 + 相对路径」（见 `modpath_root_rel`）——**不存绝对路径**。
        let (mp_root, mp_rel) = modpath_root_rel(
            std::path::Path::new(&mod_path),
            &paths,
            extra_game_mods.as_deref(),
        );
        index_mods.push(json!({
            "modId": id,
            "diskName": m.get("diskName").and_then(|x| x.as_str()).unwrap_or(""),
            "parentId": m.get("parentId").and_then(|x| x.as_str()).unwrap_or(""),
            // 模组在磁盘上的位置：**来源根标识 + 相对该根的路径**。
            //
            // 为什么不存绝对路径、也不存"相对 launcher.json"：
            // - `diskName` + `parentId` 只能表达"名字与层级"，表达不了**来源根**——
            //   游戏目录 `Mods/` 与 userDataPath 下的 `Mods/` 会同时挂载且可能有同名模组；
            //   子模组的布局（`<父>/Mods/<子>`）也无法从名字推出（实测：`66expfixmod.8-Dervish-town`
            //   被猜成 `Mods/66expfixmod.8-Dervish-town`，该目录不存在）。
            // - 但**绝对路径会随缓存迁移而失效**：`.vcmi-fv-launcher/mods/` 常被整体复制
            //   复用（换机器 / 换路径），此时绝对路径指向旧位置。
            // - "相对 `launcher.json`"也不成立：缓存与它虽同根，但它与**游戏目录**可能
            //   分属不同盘符，Windows 上跨盘符根本无法表示相对路径（`..` 不能跨盘）。
            // ⇒ 模组的"根"是**有限且已知的枚举**（游戏 `Mods/` / 用户 `Mods/` / 领域 `vcmi/Mods`），
            //   所以「根标识 + 相对路径」既完全可迁移、又无跨盘问题，还能直接看出来源。
            //   消费方按当前 `launcher.json` 解出对应根，拼上 `modPathRel` 即得最终路径。
            "modPathRoot": mp_root,
            "modPathRel": mp_rel,
            "type": type_str,
            "name": mod_name_val,
            "encrypted": m.get("contentZipEncrypted").and_then(|x| x.as_bool()).unwrap_or(false),
            "h3DataMissing": res.get("h3DataMissing").and_then(|x| x.as_bool()).unwrap_or(false),
            "cacheDir": format!("mods/{}", sanitize_id(&id)),
            "contentCount": cc,
        }));
    }

    // 与批量入口（batch.rs）统一顺序口径：按 modId 排序，使 index.json 的 mods 数组确定
    // （扫描顺序受目录枚举影响，不该外泄到产物里）
    index_mods.sort_by(|a, b| {
        a.get("modId")
            .and_then(|x| x.as_str())
            .unwrap_or("")
            .cmp(b.get("modId").and_then(|x| x.as_str()).unwrap_or(""))
    });
    let index = json!({
        "generatedAt": crate::infra::time::now_millis(),
        "version": 2,
        "mods": index_mods,
    });
    let idx_path = cache_root.join("index.json");
    if fs::write(&idx_path, serde_json::to_string_pretty(&index).unwrap()).is_err() {
        return json!({ "ok": false, "error": "io", "built": built, "skipped": skipped });
    }
    json!({
        "ok": true,
        "built": built,
        "cached": cached,
        "skipped": skipped,
        "encrypted": encrypted,
        "total": nodes.len(),
        "indexPath": idx_path.to_string_lossy(),
        "totalIndexed": index_mods.len(),
    })
}

#[cfg(test)]
mod modpath_tests {
    use super::*;

    /// 构造一个只剩路径字段的最小 `VcmiPaths`（`empty` 后手动填 `game_dir`/`user_dir`）。
    fn paths() -> VcmiPaths {
        let mut p = VcmiPaths::empty(std::env::temp_dir());
        p.game_dir = std::path::PathBuf::from("D:/Games/h3");
        p.user_dir = std::path::PathBuf::from("C:/Users/u/Documents/My Games/vcmi");
        p
    }

    #[test]
    fn 拆出来源根与相对路径() {
        let p = paths();
        let f = |s: &str| modpath_root_rel(std::path::Path::new(s), &p, None);

        // 游戏目录 Mods
        assert_eq!(
            f("D:/Games/h3/Mods/1-Knight-town"),
            ("game".into(), "1-Knight-town".into())
        );
        // 用户数据 Mods
        assert_eq!(
            f("C:/Users/u/Documents/My Games/vcmi/Mods/HuaXia"),
            ("user".into(), "HuaXia".into())
        );
        // 子模组：相对根要连中间层一起带上
        assert_eq!(
            f("D:/Games/h3/Mods/66expfixmod/Mods/8-Dervish-town"),
            ("game".into(), "66expfixmod/Mods/8-Dervish-town".into())
        );
        // 领域内模组
        assert_eq!(
            f("C:/App/.vcmi-fv-launcher/realms/r1/vcmi/Mods/m1"),
            ("realm".into(), "m1".into())
        );
    }

    #[test]
    fn 配置与真实路径大小写不一致也能匹配() {
        // 真实场景：`launcher.json` 写 `d:/games/H3`，而 `read_dir` 返回 `D:\Games\h3`
        let mut p = paths();
        p.game_dir = std::path::PathBuf::from("d:/games/H3");
        let (root, rel) = modpath_root_rel(
            std::path::Path::new("D:/games/h3/Mods/1-Knight-town"),
            &p,
            None,
        );
        assert_eq!(root, "game");
        // 相对部分保留**原始**大小写（不能把小写化的结果写进去）
        assert_eq!(rel, "1-Knight-town");
    }

    #[test]
    fn 组件边界与未命中回退() {
        let p = paths();
        let f = |s: &str| modpath_root_rel(std::path::Path::new(s), &p, None);
        // `ModsExtra` 不能被 `/Mods` 前缀吞掉
        let (root, rel) = f("D:/Games/h3/ModsExtra/x");
        assert_eq!(root, "abs");
        assert_eq!(rel, "D:/Games/h3/ModsExtra/x");
        // 完全无关的路径
        assert_eq!(f("E:/elsewhere/m").0, "abs");
    }
}
