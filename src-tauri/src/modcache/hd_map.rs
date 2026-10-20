//! HD 素材登记表（`<mod>/hd-map.json`）：记录"哪些缓存图有官方 HD 真高清源"。
//!
//! ## 为什么需要它
//!
//! 素材阶段（阶段二）知道每张图对应的**原版资源名 / DEF 帧号**；而 HD 变体阶段（阶段三，
//! `modcache::hd`）只面对 `images/<sub>/<name>.webp` 这样的路径 —— **反查不到资源名**
//! （如 `pikeman` → `CPRS097S` 要靠生物配置的 `iconIndex`）。
//!
//! 因此素材阶段命中 HD 时登记「缓存图 → HD 资源名」，阶段三据此用
//! `vcmi::hd_edition::lookup_for` 取真高清源（而不是从 1x 图插值放大）。
//!
//! ## 线程模型与"归属"（改前必读）
//!
//! 登记表是**线程局部**（构建期 rayon 多线程），与 `infra::metrics` 同一套路数：
//! rayon 线程会被复用，必须**每个 mod 开始时 reset、结束时 flush**，否则登记项会串到下一个 mod。
//!
//! ⚠️ 还有一层**归属**：部分提取（`extract_game_def_frame_and_save`、`global.rs` 的共享 UI 图标）
//! 的产物固定落 **`core/`**，尽管它可能是在 `core.castle`、`abyss-town` 等任意 mod 的构建中被执行的。
//! 因此登记项按 `owner`（产物实际归属的 mod id）分组，flush 时**把所有 owner 的表分别落盘**。
//!
//! ## 文件格式
//!
//! `{"creature/pikeman": "CPRS097S", "building/castle-mageGuild1": "THCSMAG1"}`
//! —— key 与 `assets::wiki_image_rel` 同口径（`<sub>/<sanitize_asset_name(name)>`），
//! 无 HD 覆盖的图不出现在表里。
//!
//! ⚠️ **键口径 = 磁盘文件名口径**（`sanitize_asset_name`，保留 `~`）：阶段三按**文件 stem**
//! 反查本表，两者差一个字符就会全线落空 —— 2026-10-20 此前误用 `sanitize_id`（`~`→`_`），
//! 实测 140 条登记 0 命中，高清变体全部退化为 1x 插值。改键口径必须同步这里。

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use crate::modcache::{sanitize_asset_name, sanitize_id};

thread_local! {
    /// 本线程累计的登记项：`owner(产物归属 mod id) -> { "<sub>/<sanitize_asset_name(name)>": HD 资源名 }`
    static ENTRIES: RefCell<HashMap<String, HashMap<String, String>>> = RefCell::new(HashMap::new());
}

/// 清空本线程登记项（每个 mod 构建开始前调用）。
pub(crate) fn reset() {
    ENTRIES.with(|m| m.borrow_mut().clear());
}

/// 登记一张缓存图对应的 HD 资源名。
///
/// - `owner`：产物**实际归属**的 mod id（落 `core/` 的共享图标传 `"core"`）；
/// - `name`：与 `wiki_image_rel` 同口径的图名（内部按 `sanitize_asset_name` 归一，
///   **保留 `~`** —— 阶段三按磁盘文件 stem 反查，见模块头警告）；
/// - 调用点必须**在产物确实落盘之后**（否则会出现"登记了但没图"，阶段三取到 HD 源却覆盖不到基准图）。
pub(crate) fn record(owner: &str, sub: &str, name: &str, hd_name: &str) {
    if owner.is_empty() || sub.is_empty() || name.is_empty() || hd_name.is_empty() {
        return;
    }
    let key = format!("{}/{}", sub, sanitize_asset_name(name));
    ENTRIES.with(|m| {
        m.borrow_mut()
            .entry(owner.to_string())
            .or_default()
            .insert(key, hd_name.to_string());
    });
}

/// 保护"读-改-写"登记表的互斥锁（见下方 flush 说明）。
static FLUSH_LOCK: std::sync::OnceLock<Mutex<()>> = std::sync::OnceLock::new();

/// 把本线程的登记项**按 owner 分别落盘**到 `<cache>/<sanitize(owner)>/hd-map.json`，随后清空。
///
/// mod 构建结束时调用一次（rayon 线程复用的前提）。
///
/// ⚠️ 必须**与已有内容合并**再整体写回，不能整文件覆盖：产物归属改为「素材来源方」后，
/// 多个模组会**同时**向同一个共享 owner（典型是 `core`）登记（124 个模组的原版地形同一份
/// HD 源）。直接 `fs::write` 会让后 flush 的模组把先 flush 的条目整片抹掉 —— 表现为
/// 高清阶段随机取不到真高清源、退回 1x 插值。故加锁保护读-改-写，保证不丢条目。
///
/// ⚠️ 空表**既不写也不删**：缓存命中（`wiki_image_exists` 短路）的 mod 不会产生登记项，
/// 若此时删文件就会把"上一次重建写好的登记"误清空。
pub(crate) fn flush(cache_root: &Path) {
    let all = ENTRIES.with(|m| std::mem::take(&mut *m.borrow_mut()));
    if all.is_empty() {
        return;
    }
    let lock = FLUSH_LOCK.get_or_init(|| Mutex::new(()));
    let _guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    for (owner, entries) in all {
        if entries.is_empty() {
            continue;
        }
        let path = cache_root.join(sanitize_id(&owner)).join("hd-map.json");
        // 埋点只计**本次新增**的条目（merged 含历史条目，计它会重复计数）
        let added = entries.len();
        let mut merged: HashMap<String, String> = fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<HashMap<String, String>>(&t).ok())
            .unwrap_or_default();
        merged.extend(entries);
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if let Ok(text) = serde_json::to_string(&merged) {
            let _ = fs::write(&path, text);
            crate::infra::metrics::count("hd_src.recorded", added as u64);
        }
    }
}

/// 读回某 mod 的登记表（HD 变体阶段用；不存在 / 损坏 → 空表，行为与"无 HD 覆盖"一致）。
pub(crate) fn load(cache_root: &Path, mod_id: &str) -> HashMap<String, String> {
    let path = cache_root.join(sanitize_id(mod_id)).join("hd-map.json");
    fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<HashMap<String, String>>(&t).ok())
        .unwrap_or_default()
}

/// 从各 owner 的登记表中**精确移除**某 mod 的 `cache.json` 记录过的产物键
/// （`clear_stale` 时调用，替代"删整个 hd-map.json"）。
///
/// ## 为什么不能删整个文件（2026-10-20 事故）
///
/// 产物归属改为「素材来源方」后，`core/<owner>/hd-map.json` 是**多 mod 共建的共享表**：
/// 8 族的生物/英雄/特长/建筑图标由 `core.<faction>` 提取并登记到 **core** 的表里。
/// 父 core（或任何 owner=core 的共享提取者）`clear_stale` 时若删掉整个文件，
/// 会把其它 mod 累积的成百上千条登记一并抹掉，而重建只补回自己那一部分
/// （实测：1010 条 → 140 条，高清变体随之全军覆没）。
///
/// 本函数按「本 mod cache.json 记录的产物路径」反推键（`<sub>/<磁盘stem>`，与
/// [`record`] / 阶段三查询同口径）逐个移除：删的正是"本 mod 重建后会重新登记"的那些，
/// 别人的键原样保留。移除后表为空则删文件（保持"无登记不落盘"的旧语义）。
pub(crate) fn prune_recorded(cache_json: &Path, cache_root: &Path, _mod_id: &str) {
    let text = match fs::read_to_string(cache_json) {
        Ok(t) => t,
        Err(_) => return,
    };
    let v: serde_json::Value = match serde_json::from_str(&text) {
        Ok(v) => v,
        Err(_) => return,
    };
    // rel → (owner, key)：rel 形如 `<owner>/images/<sub>/<name>.webp`
    let mut targets: std::collections::HashMap<String, Vec<String>> = Default::default();
    fn walk(v: &serde_json::Value, out: &mut std::collections::HashMap<String, Vec<String>>) {
        match v {
            serde_json::Value::String(s) => {
                let t = s.trim();
                if !(t.ends_with(".webp") && t.contains("/images/")) {
                    return;
                }
                let Some((owner, rest)) = t.split_once("/images/") else {
                    return;
                };
                let Some((sub, file)) = rest.split_once('/') else {
                    return;
                };
                let Some(stem) = file.strip_suffix(".webp") else {
                    return;
                };
                out.entry(owner.to_string())
                    .or_default()
                    .push(format!("{sub}/{stem}"));
            }
            serde_json::Value::Array(a) => a.iter().for_each(|x| walk(x, out)),
            serde_json::Value::Object(o) => o.values().for_each(|x| walk(x, out)),
            _ => {}
        }
    }
    walk(&v, &mut targets);
    for (owner, keys) in targets {
        let path = cache_root.join(sanitize_id(&owner)).join("hd-map.json");
        let Some(mut map) = fs::read_to_string(&path)
            .ok()
            .and_then(|t| serde_json::from_str::<HashMap<String, String>>(&t).ok())
        else {
            continue;
        };
        let before = map.len();
        map.retain(|k, _| !keys.contains(k));
        if map.len() == before {
            continue; // 无交集：不动文件（避免无谓写盘）
        }
        if map.is_empty() {
            let _ = fs::remove_file(&path);
        } else if let Ok(text) = serde_json::to_string(&map) {
            let _ = fs::write(&path, text);
        }
    }
}

/// HD 来源指纹标记（`<mod>/.hd-src`）是否与当前来源一致。
///
/// **为什么需要**：登记表在素材提取时写；若图已存在（`wiki_image_exists` 短路）就不会补登记。
/// 用户装 / 卸 / 升级 `hd-edition` 或切换档位后，靠"标记不匹配 → 删 images 重跑素材阶段"补齐
/// （与 `ASSETS_VERSION` 落后时的处置同款，见 `build::build_mod_wiki_phase_inner`）。
///
/// 标记缺失时：来源为 `off`（无 HD）→ 视为匹配（无需重跑）；否则视为不匹配（补一次）。
pub(crate) fn source_mark_matches(cache_root: &Path, mod_id: &str) -> bool {
    let now = crate::vcmi::hd_edition::source_fingerprint();
    let path = cache_root.join(sanitize_id(mod_id)).join(".hd-src");
    match fs::read_to_string(&path) {
        Ok(t) => t == now,
        Err(_) => now == "off",
    }
}

/// 记录本次构建所依据的 HD 来源指纹。
///
/// ⚠️ 调用点必须在**素材确实跑过之后**（`build_mod_wiki_phase` 里判 `phase != 0 && ok`），
/// 否则阶段一会抢先写下标记，导致阶段二误判"登记已完成"。
pub(crate) fn write_source_mark(cache_root: &Path, mod_id: &str) {
    let now = crate::vcmi::hd_edition::source_fingerprint();
    let path = cache_root.join(sanitize_id(mod_id)).join(".hd-src");
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(&path, now);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn record_and_load_roundtrip() {
        let dir = std::env::temp_dir().join("vcmi-fv-hdmap-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        reset();
        // 产物归属 core.castle 的两项 + 归属 core 的一项（共享图标）
        record("core.castle", "creature", "pikeman", "CPRS097S");
        record("core.castle", "building", "castle-mageGuild1", "THCSMAG1");
        record("core", "resources", "0", "SMALRES0");
        flush(&dir);
        let got = load(&dir, "core.castle");
        assert_eq!(
            got.get("creature/pikeman").map(String::as_str),
            Some("CPRS097S")
        );
        assert_eq!(
            got.get("building/castle-mageGuild1").map(String::as_str),
            Some("THCSMAG1")
        );
        assert_eq!(
            load(&dir, "core").get("resources/0").map(String::as_str),
            Some("SMALRES0")
        );
        // 空表不删文件（缓存命中的 mod 不应清掉上一轮登记）
        reset();
        flush(&dir);
        assert_eq!(load(&dir, "core.castle").len(), 2);
        let _ = fs::remove_dir_all(&dir);
    }

    /// 回归（2026-10-20）：登记键必须与**磁盘文件 stem** 同口径（保留 `~`）。
    ///
    /// 曾误用 `sanitize_id`（`~`→`_`）→ 阶段三按磁盘 stem（`cprsmall_def~g0f100`）反查
    /// 全线落空 → 高清变体全部退化为 1x 插值。本用例锁住「登记键 = 磁盘名」。
    #[test]
    fn record_key_keeps_tilde_separator() {
        let dir = std::env::temp_dir().join("vcmi-fv-hdmap-tilde");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        reset();
        record("core", "creature", "cprsmall_def~g0f100", "CPRS097S");
        flush(&dir);
        let got = load(&dir, "core");
        assert_eq!(
            got.get("creature/cprsmall_def~g0f100").map(String::as_str),
            Some("CPRS097S"),
            "登记键必须保留 `~`（与 wiki_image_rel / 磁盘 stem 同口径）"
        );
        assert!(
            !got.keys().any(|k| k.contains("_g0f")),
            "不应出现 `~g0f` 被 `_` 化的键（那是对不齐磁盘的旧 sanitize_id 口径）"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// 回归（2026-10-20 事故）：`clear_stale` 摘除登记必须**按产物键精确删**，
    /// 不能删整个 `hd-map.json` —— 它是多 mod 共建的共享表（8 族生物/英雄/特长/建筑
    /// 都登记在 core 名下），父 core 重建时删整表会把别人的成百上千条一并抹掉
    /// （实测 1010 → 140，高清变体全军覆没）。
    #[test]
    fn prune_recorded_removes_only_own_keys() {
        let dir = std::env::temp_dir().join("vcmi-fv-hdmap-prune");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("core")).unwrap();
        // 共享表：core.castle 提的生物（键A）+ 父 core 提的宝物（键B）
        let shared = r#"{"creature/cprsmall_def~g0f2":"CPRS000C","artifact/artifact_def~g0f103":"HRART098"}"#;
        fs::write(dir.join("core").join("hd-map.json"), shared).unwrap();
        // core.castle 的 cache.json：只记录生物图
        let cj = dir.join("core.castle").join("cache.json");
        fs::create_dir_all(dir.join("core.castle")).unwrap();
        fs::write(
            &cj,
            r#"{"creatures":[{"image":"core/images/creature/cprsmall_def~g0f2.webp"}]}"#,
        )
        .unwrap();
        prune_recorded(&cj, &dir, "core.castle");
        let got = load(&dir, "core");
        assert!(
            !got.contains_key("creature/cprsmall_def~g0f2"),
            "本 mod 记录过的键应被摘除（重建后会重新登记）"
        );
        assert_eq!(
            got.get("artifact/artifact_def~g0f103").map(String::as_str),
            Some("HRART098"),
            "别人的键必须保留——共享表删整表会把 8 族登记全部抹掉"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
