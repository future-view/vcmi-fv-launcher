//! 全局共享状态（Tauri `State<AppPaths>` 的注入点）。
//!
//! 独立成模块的原因：`AppPaths` 原定义在 `commands.rs`，导致 `realms` / `repository`
//! 需要反向依赖命令层才能拿到它。它本质是「运行期路径状态」，与命令无关，
//! 故提升到 crate 根（见 `docs/architecture/backend.zh-CN.md` §1.3 / §4.1）。

use std::sync::Mutex;

use crate::vcmi::paths::VcmiPaths;

/// 运行期路径状态：目录识别的唯一权威，命令层通过 `State<AppPaths>` 访问。
///
/// 用 `Mutex` 而非 `RwLock`：读远多于写，但临界区极短（克隆若干 `PathBuf`），
/// 保持与既有实现一致的锁语义，避免重构期间改变行为。
pub struct AppPaths(pub Mutex<VcmiPaths>);
