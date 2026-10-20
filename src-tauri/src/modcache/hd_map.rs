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
//! —— key 与 `assets::wiki_image_rel` 同口径（`<sub>/<sanitize(name)>`），无 HD 覆盖的图不出现在表里。

use std::cell::RefCell;
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::Mutex;

use crate::modcache::sanitize_id;

thread_local! {
    /// 本线程累计的登记项：`owner(产物归属 mod id) -> { "<sub>/<sanitize(name)>": HD 资源名 }`
    static ENTRIES: RefCell<HashMap<String, HashMap<String, String>>> = RefCell::new(HashMap::new());
}

/// 清空本线程登记项（每个 mod 构建开始前调用）。
pub(crate) fn reset() {
    ENTRIES.with(|m| m.borrow_mut().clear());
}

/// 登记一张缓存图对应的 HD 资源名。
///
/// - `owner`：产物**实际归属**的 mod id（落 `core/` 的共享图标传 `"core"`）；
/// - `name`：与 `wiki_image_rel` 同口径的图名（内部会 sanitize）；
/// - 调用点必须**在产物确实落盘之后**（否则会出现"登记了但没图"，阶段三取到 HD 源却覆盖不到基准图）。
pub(crate) fn record(owner: &str, sub: &str, name: &str, hd_name: &str) {
    if owner.is_empty() || sub.is_empty() || name.is_empty() || hd_name.is_empty() {
        return;
    }
    let key = format!("{}/{}", sub, sanitize_id(name));
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
}
