# IPC 命令契约

> 状态：已实现 ｜ 最后核对：2026-10-17 ｜ 代码：`src-tauri/src/lib.rs`、`src/lib/tauri.ts`、`scripts/check-invoke-parity.mjs`
> 当前基线 **138 个命令**（以脚本输出为准）。改后端必跑 `npm run check:invoke-parity`。

## 1. 三方一致性

脚本比对三方，任一不一致退出码 1：

1. 前端 `src/lib/tauri.ts` 里的字面量 `invoke("x")`（前端 invoke 集中在此文件，脚本只扫它）；
2. Rust 侧 `#[tauri::command]` 定义（含**参数名**）；
3. `lib.rs` 的 `generate_handler!` 注册表。

**命令名 + 参数名即契约**：重命名或改参数会静默 404（前端拿不到命令），不会报错。

> 本机工具链对**新增** Tauri 命令可能触发 never-type 硬错误；能复用现有命令扩参数的，优先扩参数（如可选更新节流复用 `set_update_pref`）。

## 2. 命令清单（按域）

### VCMI 与模组（18）

`read_mod_settings`、`write_mod_settings`、`patch_preset_lang_mods`、`clear_mod_validation`、`gamedata_health_check`、`preview_mod_zip`、`install_local_mod`、`import_game_files`、`get_dirs`、`detect_vcmi_paths`、`suggest_user_data_dir`、`scan_mods`、`detect_edition`、`list_installed_mods`、`read_def_bytes`、`read_vcmi_settings`、`write_vcmi_settings`、`set_vcmi_language`

### 启动器配置（17）

`read_launcher_config`、`write_launcher_config`、`set_ui_lang`、`set_launcher_pref`、`set_ai_hd_enhance`、`set_ai_model`、`list_ai_models`、`verify_ai_model`、`set_hd_enhance`、`set_town_preview`、`set_realm_mod_auto_check`、`set_realm_publish_target`、`set_hd_edition_use`、`set_hd_edition_scale`、`set_network_proxy`、`set_network_mirror_rules`、`hd_edition_status`

### 应用与启动（14）

`launch_game`、`launch_editor`、`launch_native_launcher`、`log_frontend`、`open_in_explorer`、`get_primary_monitor`、`get_compile_info`、`get_runtime_info`、`list_monitors`、`list_resolutions`、`close_window`、`minimize_window`、`toggle_maximize_window`、`is_debug`

### 外挂组件（4）

`runtime_status`、`runtime_install`、`runtime_cancel`、`runtime_uninstall`

### 百科缓存（16）

`read_mod_cache`、`read_mod_metas`、`read_town_cards`、`read_showcase_slides`、`ensure_showcase_audio`、`generate_town_preview`、`cache_build_progress`、`cancel_cache_build`、`realm_cache_build_progress`、`cancel_realm_cache_build`、`build_realm_cache_urgent`、`check_realm_assets`、`build_mods_cache_batch`、`build_all_mod_cache`、`read_cache_index`、`get_mods_cache_dir`

### 领域（48）

`list_realms`、`get_active_realm`、`realm_config_dir`、`write_realm_presets`、`create_realm`、`check_realm_id`、`update_realm`、`delete_realm`、`activate_realm`、`deactivate_realm`、`set_realm_maps_blocked`、`kill_vcmi`、`realm_self_check`、`realm_self_heal`、`export_realm`、`cancel_realm_export`、`preview_realm_publish`、`import_realm`、`realm_dir`、`get_intro_state`、`mark_intro_played`、`get_bgm_muted`、`set_bgm_muted`、`import_realm_asset`、`check_realm_mod_conflicts`、`check_realm_mods_ready`、`rename_realm`、`list_realm_maps`、`upload_realm_map`、`delete_realm_map`、`native_maps_status`、`repair_game_maps`、`import_realm_maps_dir`、`move_realm_map`、`apply_realm_maps_valve`、`set_realm_map_filter`、`is_vcmi_running`、`list_vcmi_processes`、`list_vcmi_game_processes`、`realm_mod_ops`、`list_realm_mods`、`realm_mod_ref`、`realm_mod_unref`、`realm_mod_bundle`、`realm_mod_delete`、`realm_mod_import`、`ensure_realm_lang_visible`、`realm_lang_pack_candidates`

### 远程领域仓库（8）

`get_realm_repositories`、`add_realm_repository`、`remove_realm_repository`、`test_realm_repository`、`fetch_realm_index`、`install_remote_realm`、`cancel_realm_download`、`ensure_realm_covers`

### 模组仓库（6）

`fetch_mod_index`、`install_mod_from_repo`、`cancel_mod_download`、`auto_check_repos_enabled`、`list_mod_ledgers`、`delete_mod`

### 启动器更新（6）

`check_launcher_update`、`download_launcher_update`、`cancel_launcher_update`、`apply_launcher_update`、`set_update_pref`、`set_update_manifest_url`

## 3. 事件（后端 → 前端）

| 事件 | 来源 | 用途 |
|---|---|---|
| `wiki-build-progress` | 全局百科构建 | 进度推送（每 8 个 / 最后一个），前端另有 400ms 轮询兜底 |
| `realm-cache-build-progress` | 领域加急构建 | 独立通道，避免与全局构建互相覆盖 |
| `realm-export-progress` | 领域导出 | 打包进度 |
| `user-data-copy-progress` | 首启用户数据迁移 + 原版数据复制 | `{stage?, doneFiles, doneBytes, totalFiles, totalBytes}`，后端 200ms 节流 + 末帧；`stage` 仅原版数据复制带（`data` / `mp3` / `maps`，两段各有独立分母，前端据此换标题） |
| `runtime-install-progress` | 外挂组件安装 | 每 200ms |
| 更新下载进度 | 自更新 | 流式写盘进度 |

进度事件是**加速手段不是唯一通道**：前端一律有轮询兜底，事件丢失不会卡死。

## 4. 冻结契约（改动前必读）

| 契约 | 内容 |
|---|---|
| `launcher.json` 三层格式 | 磁盘 = 分组 snake_case；`read_launcher_config` 出参 = 手写 camelCase；`write_launcher_config` 入参 = camelCase。任一层被"顺手统一"→ 用户配置丢失 |
| `write_launcher_config` 扩展入参 | `redirectUserData` / `copyFrom` / `copyOnly` / `cancelCopy` / `copyH3Data` / `h3DataFrom` / `h3DataTo` 均为**可选**（旧调用不传 = 原行为）；`copyOnly` 只迁移用户数据、不写配置，出参 `{saved, copied, cancelled, files, bytes, h3Files, h3Bytes}`；`copyH3Data` 从外部数据目录复制 `.lod` 到 `h3DataTo`（必须是 `…/Data`）。语义与执行顺序见 [首启向导](../features/setup-wizard.zh-CN.md) §4 |
| `gamedata_health_check` 入参 | 可选 `game_dir` / `user_data_dir`（首启向导对"尚未保存的目录"体检）；不传 = 完全沿用已保存配置。返回加 `h3Data`（原版数据闸门，官方口径）与 `dataInstallCandidates`（注册表安装候选）。**必须 async + `spawn_blocking`**：LOD 头读取与 `reg query` ×4 是磁盘/进程 IO，同步命令占主线程会拖住文件夹选择框（2026-10-18 实测首次点「选择」卡数秒） |
| `check_launcher_update` 扩展入参 | `probe`（`Option<bool>`）：`Some(true)` 只做清单可达性探测，不触碰更新状态机，结果在 `UpdateInfo::probe`；`force` 语义不变 |
| `get_dirs` | 固定 6 个 key |
| realms 结构化错误 | `{code, msg?, realmId?, realmName?}` 字符串 |
| `check_realm_id` | 返回 `{available, reason}` |
| `BatchWikiMod` | serde flatten 字段，前端 `buildTargets.ts` 与之对齐 |
| 错误对象 | 后端 `be_err(code, arg)` → `{"code","arg"}`；前端按 `code` 查词典，缺译回退中文 |

## 5. 禁改（看似兼容、实为必需）

- `write_launcher_config` 的「宽容继承」回退：防首次向导清掉便携包预置项。
- `vcmi/paths.rs` 的 `%USERDOCUMENTS%` 映射：增强版的 `dirs.json` 用了这个变量，VCMI 源码里没有。
- `modcache::sanitize_id`（**允许点号**，`core.castle` 依赖）与 `VcmiPaths::sanitize_realm_id`（禁点号、空回退 `"realm"`）：**禁止合并**，两者语义不同。
