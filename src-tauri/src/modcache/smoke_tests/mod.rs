//! 实机冒烟与回归测试（2026-09-12 从 `commands.rs` 移出，见设计文档 `rust-backend-refactor` §6 S5）。
//!
//! 绝大多数用例依赖真实游戏目录（`VCMI_GAME_DIR`，缺省回落仓库内 `references/Heroes3-VCMI`
//! 验证 fixture），故标注 `#[ignore]`；纯逻辑用例（如 `source_hash_is_content_based`）可直接跑。
//!
//! **路径解析统一约定（见本文件底部的助手函数）**：所有用例的游戏目录 / 缓存根 / 用户目录 /
//! mod 目录 / HD 样本根都从助手取，不得在各子模块散落写死绝对路径
//! （如 `D:/Games/Heroes3-VCMI`、`D:/Developer/Gitee/...`）——否则换机器后全部失效。
//! 未设置环境变量时一律回落 `<crate>/../references/Heroes3-VCMI`，这是随仓库携带、
//! 跨机器一致的"固定验证地址"。
//!
//! 运行方式：
//! - `cargo test -p vcmi-fv-launcher`：纯逻辑用例；
//! - `cargo test -p vcmi-fv-launcher -- --ignored`：全部（需游戏目录）；
//! - `cargo test -p vcmi-fv-launcher rebuild_all_with_core -- --ignored --nocapture`：缓存全量重建
//!   （`npm run rebuild:wiki-cache` 亦调用此用例）。
//!
//! 按域拆分子模块（共享 imports 与辅助函数在本文件，子模块以 `use super::*` 取用）：
//! - `core_regression`：原版 core / h3-core 数据回归
//! - `build_phase`　 ：两阶段构建、缓存版本与索引
//! - `assets_icons`　 ：DEF 帧 / webp 编码 / 图标与肖像提取
//! - `resolve_resource`：资源定位（MOD → 游戏 LOD 回退）
//! - `hd_edition`　　 ：HD 素材端到端（阶段二登记 → HD 源生成 `@Nx` 变体）

use std::fs;
use std::path::{Path, PathBuf};
use serde_json::{json, Value};
use crate::vcmi::paths::VcmiPaths;
use crate::vcmi::resource::resolve_resource;
use crate::vcmi::scan::{collect_mod_paths, scan_user_mods_dir};
use crate::modcache::batch::{
    build_mods_cache_batch_inner, wiki_threads, BatchWikiMod,
};
use crate::modcache::build::build_mod_wiki_phase;
use crate::modcache::index::{build_all_wiki_inner, read_mod_wiki_inner};
use crate::modcache::preview::generate_town_preview_inner;
use crate::infra::hash::compute_source_hash;
use crate::infra::image::encode_webp;
use crate::vcmi::h3core::{h3_core_data_dir, load_h3_core_i18n};
use crate::vcmi::town::read_town_configs;
use crate::modcache::build::build_mod_wiki_inner;
use crate::modcache::collect::collect_all_raw_configs;
use crate::modcache::global::{
    extract_resource_icons_for_mod, extract_shared_ui_icons, extract_stat_icons_for_mod,
};
use crate::modcache::preview::{composite_structures_to_canvas, compose_town_preview_for, load_structure_frame};
use crate::modcache::version::WIKI_CACHE_VERSION;

mod assets_icons;
mod ai_audit;
mod build_phase;
mod rebuild;
mod core_regression;
mod resolve_resource;
mod town_refs;
mod protocol;
mod hd_bench;
mod hd_compare;
mod hd_edition;

fn fixture(mod_name: &str) -> PathBuf {
    references_game_dir().join("Mods").join(mod_name)
}

// ─────────────────────────────────────────────────────────────────────────────
// 统一的测试环境 / 路径解析助手
//
// 所有用例的"游戏目录 / 缓存根 / 用户目录 / mod 目录 / HD 样本根"都从这里取，避免在子模块里
// 散落写死的绝对路径（换机器后全部失效）。未设置环境变量时一律回落仓库内随包携带的
// `references/Heroes3-VCMI` 验证 fixture——它经 `CARGO_MANIFEST_DIR` 定位，跨机器一致。
//
// 历史遗留的 `D:/Games/Heroes3-VCMI`、`D:/Developer/Gitee/...` 等写死绝对路径一律不允许，
// 必须经由下方助手回落到 fixture，或经环境变量显式指定。
// ─────────────────────────────────────────────────────────────────────────────

/// 仓库内随包携带的游戏目录验证 fixture（跨机器一致）。
fn references_game_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../references/Heroes3-VCMI")
}

/// 游戏目录：优先 `VCMI_GAME_DIR`，否则回落 references fixture。
pub(super) fn game_dir() -> PathBuf {
    std::env::var("VCMI_GAME_DIR")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(references_game_dir)
}

/// 缓存根：优先 `VCMI_MOCK_CACHE_DIR`，否则 `<game_dir>/.vcmi-fv-launcher/mods`。
pub(super) fn default_cache_root(game_dir: &Path) -> PathBuf {
    std::env::var("VCMI_MOCK_CACHE_DIR")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| game_dir.join(".vcmi-fv-launcher").join("mods"))
}

/// 用户模组目录：优先 `VCMI_USER_MODS_DIR`，否则 `<game_dir>/userdata/Mods`。
pub(super) fn user_mods_dir() -> PathBuf {
    std::env::var("VCMI_USER_MODS_DIR")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| game_dir().join("userdata").join("Mods"))
}

/// 用户数据目录：优先 `VCMI_USER_DATA_DIR`，否则经 `VcmiPaths::resolve_user_dir` 按游戏目录推导。
pub(super) fn user_data_dir(game_dir: &Path) -> PathBuf {
    std::env::var("VCMI_USER_DATA_DIR")
        .ok()
        .map(PathBuf::from)
        .unwrap_or_else(|| VcmiPaths::resolve_user_dir(game_dir))
}

/// 测试 mod 目录：优先 `VCMI_TEST_MODS_DIR`（兼容旧 `VCMI_TEST_MOD_DIR`），否则 `<references>/Mods`。
pub(super) fn test_mods_dir() -> PathBuf {
    std::env::var("VCMI_TEST_MODS_DIR")
        .ok()
        .or_else(|| std::env::var("VCMI_TEST_MOD_DIR").ok())
        .map(PathBuf::from)
        .unwrap_or_else(|| references_game_dir().join("Mods"))
}

/// HD bench / compare / audit 的样本根（缓存 mods 根）：优先 `HD_BENCH_MODS`，
/// 否则回落 references fixture 的缓存目录。
pub(super) fn sample_root() -> PathBuf {
    if let Some(p) = std::env::var_os("HD_BENCH_MODS") {
        let p = PathBuf::from(p);
        if p.is_dir() {
            return p;
        }
        println!("⚠️ HD_BENCH_MODS 指向的目录不存在：{}（回退默认）", p.display());
    }
    references_game_dir().join(".vcmi-fv-launcher").join("mods")
}
