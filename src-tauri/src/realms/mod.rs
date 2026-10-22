//! 领域（Realm）系统后端引擎（设计文档 §3 / §8.3 / §9.7.2）。
//!
//! 核心职责：
//! - 领域元信息的持久化：**唯一权威是各领域目录内的 `realm.json`**（§3.1「目录即领域」）；
//!   `launcher.json` 只保留 `realms.active` 一项（**无任何迁移兼容**：旧数据由开发者手工清理）。
//! - 激活领域：把 `<游戏目录>/config/dirs.json` 的 5 个真实 key 改写为指向领域隔离目录
//!   （`<exe>/.vcmi-fv-launcher/realms/<id>/vcmi`），并**从 VCMI 全局用户设置提取**
//!   `settings.json` 作为领域初始值（优先既有设定、不覆盖，见 §9.7.2）。
//! - 停用领域：用启动时保留的原文备份还原 `dirs.json`。
//! - 地图阀门（§8.3）：懒执行地把游戏自带 `Maps` 改名为 `Maps.vcmi-fv-native` 并建 junction，
//!   开关只决定 junction 是否存在。
//!
//! 注意（实现边界）：
//! - 进程守卫（§4.4 **分级**）：`activate_realm` / `deactivate_realm` / `set_realm_maps_blocked`
//!   在改写 `dirs.json` 或 Maps 联接前调用 `guard_vcmi_not_running()`（只挡**游戏本体**
//!   client / server / mapeditor；官方启动器 `VCMI_launcher.exe` 可共存），
//!   并通过 `kill_vcmi` 提供「结束游戏」（同样只结束游戏本体，不误伤官方启动器）。
//! - 崩溃自检自愈（§4.5）：`realm_self_check` / `realm_self_heal` 由前端在启动后调用。
//! - 领域模组组合（`modSettings` 的 `__realm__` 预设）由 `write_realm_preset` 在激活时按
//!   `realm.mods`（borrowed + bundled）写入领域隔离目录，并设为 `activePreset`。
//!
//! 文件划分（依赖自上而下）：
//! - [`base`]　    进程守卫 / 配置读写 / 领域根路径与锁 / 日期（无业务命令）
//! - [`meta`]　    元数据与生命周期（列举、创建、更新、删除、重命名）
//! - [`presets`]　 `__realm__` 预设写入与必需 mod 注入
//! - [`activate`]　激活 / 停用（dirs.json 改写、链接、停用回滚）
//! - [`maps`]　    地图阀门 + 领域地图管理
//! - [`selfcheck`] 自检 / 自愈
//! - [`transfer`]　导出 / 导入（zip）
//! - [`conflicts`] mod 冲突检测与就绪检查
//! - [`mod_manage`] 领域模组管理（已实装列表 / 引用 / 整合 / 删除 / 导入，
//!   见 `docs/features/realms.zh-CN.md`）
//! - [`persona`]　 intro 状态 / BGM / 素材导入
//!
//! ⚠️ **命令名 + 参数名 + 错误串格式 = 前端契约**：改动后跑 `npm run check:invoke-parity`。

mod activate;
pub(crate) mod base;
mod conflicts;
mod maps;
mod meta;
mod mod_manage;
mod persona;
mod presets;
mod realm_mods;
mod selfcheck;
mod transfer;

pub use activate::{activate_realm, deactivate_realm, kill_vcmi};
pub use base::{is_vcmi_running, list_vcmi_game_processes, list_vcmi_processes, now_date};
pub use conflicts::{check_realm_mod_conflicts, check_realm_mods_ready};
pub use maps::{
    apply_realm_maps_valve, delete_realm_map, import_realm_maps_dir, list_realm_maps,
    move_realm_map, native_maps_status, repair_game_maps, set_realm_map_filter,
    set_realm_maps_blocked, upload_realm_map,
};
pub use meta::{
    check_realm_id, create_realm, delete_realm, get_active_realm, list_realms, read_realm_meta,
    realm_config_dir, realm_versions, rename_realm, update_realm, write_realm_meta,
};
pub use mod_manage::{
    ensure_realm_lang_visible, list_realm_mods, realm_lang_pack_candidates, realm_mod_bundle,
    realm_mod_delete, realm_mod_import, realm_mod_ref, realm_mod_unref,
};
pub use persona::{
    get_bgm_muted, get_intro_state, import_realm_asset, mark_intro_played, realm_dir, set_bgm_muted,
};
pub use presets::write_realm_presets;
pub use realm_mods::realm_mod_ops;
pub use selfcheck::{realm_self_check, realm_self_heal};
pub use transfer::{
    cancel_realm_export, export_realm, import_realm, import_realm_impl, preview_realm_publish,
};
