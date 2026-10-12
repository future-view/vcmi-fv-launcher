//! 模组缓存版本常量（**唯一定义点**）。
//!
//! - [`WIKI_CACHE_VERSION`]：缓存结构 / 提取逻辑变更时递增 → 旧缓存整体失效重建；
//! - [`ASSETS_VERSION`]：仅素材提取逻辑变更时递增 → 可使旧缓存只重跑素材阶段。
//!
//! - [`WIKI_CACHE_VERSION`] 是缓存键的一部分（`v{WIKI_CACHE_VERSION}:{sourceHash}:{h3DataKey}:{h3CoreHash}:{schemaHash}`），
//!   命中判定为**字符串相等**（`build/mod.rs`）：它一变，算出的键就对不上旧文件 → 整体重建。
//!   `h3CoreHash` / `schemaHash` 自 v77/v85 起并入键，数据源/schema 变化会自动触发重建，
//!   与版本号解耦，开发者不必每次都手动 bump 版本号。
//! - [`ASSETS_VERSION`] 不在键内，作为 `cache.json` 的 `assetsVersion` 字段以 **`>=` 数值比较**判定
//!   （`build/mod.rs`）：它是**单调下限**，只能递增、不可回退——回退到更小值会让旧缓存被判「有效」而跳过重建。

/// 解析原版伪模组的 id 约定，返回 `(是否 core 系, 阵营作用域)`：
/// - `"core"`            → `(true, None)`：父模组，只承载**全局/无归属**数据（无城镇）；
/// - `"core.<faction>"`  → `(true, Some(faction))`：子模组，只承载该阵营的城镇/生物/英雄，
///   形态与第三方 town mod 完全一致；
/// - 其他                → `(false, None)`：普通第三方 mod。

/// 素材提取版本号：素材提取逻辑（编码策略/取帧规则/落盘位置等）变更时递增，
/// 使只需重跑阶段二补图，而不必重新解析配置。作为缓存失效下限（见模块头说明），只能递增、不可回退。
pub(crate) const ASSETS_VERSION: u32 = 1;

/// Wiki / 模组缓存版本号：缓存结构 / 提取逻辑变更时递增，使旧缓存整体失效重建。
///
/// `modPath` 采用相对路径编码（`gameMods:`/`userMods:`/`realm:<id>:`），base 取自
/// `launcher.json` 权威配置与 `.vcmi-fv-launcher` 根，不写死绝对路径、不依赖
/// `VcmiPaths::detect()` 重探测游戏目录（见 `encode_mod_path_for_cache`）。
pub(crate) const WIKI_CACHE_VERSION: u32 = 1;
