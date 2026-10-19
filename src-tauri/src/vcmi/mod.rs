//! VCMI 领域层（vcmi）：读取游戏目录 / 模组 / VCMI 设置（无 Tauri、无 UI）。
//!
//! 依赖方向（见 `docs/architecture/backend.zh-CN.md` §4.1）：
//! 只依赖 `infra`；**不得**依赖 `wiki` / `realms` / `repository` / `commands`，
//! 也不得出现 `tauri` 类型（`State` / `AppHandle` / `Window`）。
//!
//! 模块职责：
//! - `paths`：目录识别（`dirs.json` → `VcmiPaths`）、缓存/日志/领域目录派生、环境变量展开。
//! - `persistent`：引擎持久化（`persistentStorage.json`）清洗，剔除指向未启用模组的引用。
//! - `config`：`launcher.json` 的磁盘结构（`LauncherConfig` / `WindowSize` / 路径 serde）。
//! - `resource`：模组资源解析（mod 目录 / `content.zip` / 游戏 LOD 三来源）。
//! - `scan`：模组扫描与分类（`Mods/` 遍历、`mod.json` 解析、submod 归并）。
//! - `h3core`：h3-core 文案表与原版（core）伪模组识别。
//! - `game_import`：游戏文件导入（地图/战役/存档/模组/配置/历代记识别/HD 素材生成）。
//! - `health`：游戏数据完整性体检（lod 条目 / 核心 mod / 资源容器，只读）。
//! - `h3data`：原版 H3 数据闸门（官方口径检测 / 注册表定位 / 复制 `.lod`/`.snd`/`.vid` 补齐）。
//! - `chronicles`：历代记导入（解包后生成官方同款 `Mods/chronicles` 模组）。
//! - `modmeta`：生成模组的 `mod.json` 文案表（外部 JSON + 编译期嵌入，含 VCMI 语言块）。
//! - `hd_edition`：官方 HD 版素材 mod（`hd-edition`）的 `.pak` 探测 / 注入 / 按名取图（真高清来源）。
//! - `setup`：用户配置播种（官方启动器首次运行向导状态 `launcher.setupCompleted`）。
//! - `bonus`：bonus type 的图标表（`bonuses.json` 的 `graphics` 段，百科能力图标来源）。
//! - `spell_icons`：法术自身的 `graphics.iconImmune`（`SPELL_IMMUNITY` 能力的图标来源）。
//! - `inno`：Inno Setup 安装器的只读格式探测（版本 / 头部位置 / 数据区 / 压缩算法）。

pub mod bonus;
/// 历代记（Chronicles）：按官方产物结构生成 Expansion 模组（`chroniclesextractor.cpp` 同款）。
pub mod chronicles;
pub mod config;
pub mod game_import;
pub mod gamedata;
pub mod h3core;
pub mod modmeta;
/// 原版 H3 数据文件：检测（官方口径）/ 定位（注册表）/ 补齐（复制 .lod）。
pub mod h3data;
pub mod health;
pub mod i18n;
/// Inno Setup 安装器的**只读**探测（版本 / 头部位置 / 数据区起点 / 压缩算法）。
pub mod inno;
pub mod mod_install;
// HD 高清素材来源（`.pak`）：接线期间部分接口尚未被提取链路调用，先放宽 dead_code。
#[allow(dead_code)]
pub mod hd_edition;
pub mod paths;
/// 引擎持久化清洗（`persistentStorage.json` 失效跨模组引用）。
pub mod persistent;
pub mod resource;
pub mod scan;
pub mod setup;
pub mod spell_icons;
pub mod town;
pub mod version;
