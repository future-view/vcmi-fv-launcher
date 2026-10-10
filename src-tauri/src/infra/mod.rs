//! 纯基础设施层（infra）：无 Tauri、无 VCMI 业务语义。
//!
//! 依赖方向（见 `docs/architecture/backend.zh-CN.md` §4.1）：
//! 所有上层模块（`vcmi` / `wiki` / `realms` / `repository` / `commands`）均可依赖本层，
//! 本层**不得**反向依赖它们，也不得出现 `tauri` 类型（`State` / `AppHandle` / `Window`）。
//!
//! 模块职责：
//! - `json`：VCMI 宽容 JSON 解析（JSON5 子集：注释 / 尾逗号 / 单引号转义）。
//! - `def` ：H3 DEF 动画/帧解码（含 BMP/BGRA 输出）。
//! - `lod` ：H3 LOD 归档按名取资源（`H3bitmap.lod` 等）。
//! - `pak` ：H3 高清版（HD Edition）`.pak` 容器按名取图集切片（只读；业务编排在 `vcmi::hd_edition`）。
//! - `links`：目录联接/符号链接的创建与安全移除（领域借用模组、Maps 阀门）。
//! - `logging`：tracing 日志初始化、按天轮转与过期清理。
//! - `time`：UTC 日期与时间戳（`now_secs` / `now_millis` / `today_utc`）。
//! - `hash`：SHA-256 摘要与字节 → 十六进制。
//! - `version`：点分版本逐段比较（去 `v` 前缀、每段取前导数字；模组与领域共用口径）。
//! - `h3m`：H3M/H3C/VMAP 地图头部解析与语言检测（领域集成地图元数据）。
//! - `http`：统一 blocking HTTP 客户端（跟随 302 + ETag 条件请求）。
//! - `idle`：系统空闲检测（Win32 `GetLastInputInfo`），供后台构建动态提档。
//! - `metrics`：轻量性能埋点（线程局部累计 + 全局聚合），用于定位缓存构建热点。
//! - `upscale`：高清素材放大（**可插拔算法层**：xBRZ / 最近邻 / Lanczos3…）。
//! - `window`：主窗口初始尺寸推导（窗口显示前定几何，消除启动可见跳变）。

pub mod berror;
pub mod cache;
pub mod def;
// 高清放大算法层：`modcache::hd` 已在素材阶段实际调用
// （`default_upscaler` / `upscaler_by_id` / `upscale_with`）。
// 仍保留 allow 的原因：`all_upscalers` 与 `upscale_rgba(id,…)` 目前只被 `hd_bench`（选型对比）使用。
#[allow(dead_code)]
pub mod upscale;
/// AI 超分（Real-ESRGAN ONNX，模型内嵌进二进制）。
/// 仅 `modcache::hd` 的**插画档**调用；任何失败都返回 `None`，由调用方回退传统算法。
pub mod ai;
pub mod http;
pub mod idle;
pub mod image;
pub mod hash;
pub mod h3m;
pub mod json;
pub mod links;
pub mod lod;
pub mod logging;
pub mod metrics;
/// 锐化算子（USM）：传统插画档放大后叠加 —— AI 超分不可用（组件缺失 / 倍率不符 / 类型不适用）
/// 时的锐度补偿，见 `modcache::hd::upscale_file` 的编码前处理。
pub mod sharpen;
// H3 高清版素材容器（`.pak`）：解析索引 + 帧表 + sheet 解压。
// 目前仅 `vcmi::hd_edition` 消费；接线过程中先放宽 dead_code（方法会逐步被用上）。
#[allow(dead_code)]
pub mod pak;
pub mod time;
pub mod version;
pub mod window;
