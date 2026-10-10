//! 阶段二（素材）：游戏级共享图标的提取入口。
//!
//! 这些是**游戏级**资源（原版游戏素材，统一归位到父 `core` 模组的 `core/images/...`：`SMALRES.DEF`、`PortraitsLarge.DEF` 等），与具体 mod 无关，
//! 但产物写入各自 mod 的缓存目录以保持相对路径自洽（见 `vcmi-core-cache-restructure` §13.4）。
//!
//! 单 mod 的素材提取（城镇背景、生物小图、英雄头像等）由 `data/*` 各节在 `assets_enabled()`
//! 为真时直接完成 —— 本模块只承载「跨 mod 复用的共享图标」这一横向关注点。

use std::path::Path;

use serde_json::Value;

use crate::modcache::global::{
    extract_resource_icons_for_mod, extract_shared_ui_icons, extract_stat_icons_for_mod,
};

/// 提取游戏级共享素材到该 mod 的缓存目录：资源图标（`SMALRES` 等）。
///
/// 在构建早期调用（城镇 / 生物等条目可能引用这些图标）。
pub(crate) fn extract_shared_resource_icons(game_dir: &Path, cache_root: &Path, mod_id: &str) {
    extract_resource_icons_for_mod(game_dir, cache_root, mod_id);
}

/// 提取属性图标（`stackWindow/*.png`）与共享 UI（头像 / 法术书 / 战争机器等）。
///
/// 返回 `(statIcons, sharedIcons)`，由骨架写入 `mod.json`。
pub(crate) fn build_shared_icons(
    game_dir: &Path,
    cache_root: &Path,
    mod_id: &str,
) -> (Value, Value) {
    let stat_icons = extract_stat_icons_for_mod(game_dir, cache_root, mod_id);
    let shared_icons = extract_shared_ui_icons(game_dir, cache_root, mod_id);
    (stat_icons, shared_icons)
}
