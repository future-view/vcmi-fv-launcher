# 领域系统（Realm）

> 状态：已实现 ｜ 最后核对：2026-10-12 ｜ 代码：`src-tauri/src/realms/`、`src/features/realms/`、`src/lib/realm/`
> 编辑器见 [realms-editor](./realms-editor.zh-CN.md)；远程分发见 [realms-repository](./realms-repository.zh-CN.md)。

## 1. 目标与范围

**做**：把"模组组合 + 存档 + 设置 + 地图"整体隔离成多套互不干扰的 VCMI 环境，可进入 / 退出、可导出分发。

**不做**：修改 VCMI 源码。隔离完全靠改写 VCMI 官方的 `config/dirs.json` 实现。

## 2. 概念与权威数据

- 领域的**唯一权威**是 `realms/<id>/realm.json`；`launcher.json` 只保存 `realms.active`（**无迁移兼容**）。
- 领域根下另有：`lock`（编辑权）、`.introPlayedAt` / `.bgmMuted` / `.prevActivePreset`（用户态）、`assets/**`、`vcmi/`（隔离后的用户目录）。

## 3. 生命周期

| 转换 | 行为 |
|---|---|
| **创建** `create_realm` | id 必填，非法即报错（**不静默清洗**）；建隔离骨架、写 `lock`、补 `createdAt/updatedAt/presetId=__realm__/tags=[]/showcase.enabled=false` |
| **更新** `update_realm` | 深合并；`id` 不可改；**内容无变化则不写盘、不刷 `updatedAt`** |
| **激活** `activate_realm` | 进程守卫 → 校验 `VCMI_client.exe` → 备份并改写 `dirs.json` → 配置共享链接 → 播种 `setupCompleted` → 地图阀门 → 重建领域地图 junction → 写 `__realm__` 预设 → 落地借用 / 整合模组 → 记录 `realms.active` |
| **停用** `deactivate_realm` | 恢复进入前的 `activePreset` → 还原 `dirs.json` → 仅当"已整理过"才还原 Maps → 清激活态 |
| **删除** `delete_realm` | 激活中先停用 → `remove_realm_dir_safely`（**先摘借用联接**再 `remove_dir_all`） |
| **重命名** `rename_realm` | 新 id 未占用且领域未激活 → 整目录改名 + 同步 `realm.json.id` |

## 4. 隔离机制

`dirs.json` 的 5 个真实 key：`userDataPath` / `userConfigPath` / `userCachePath` / `userLogsPath` / `userSavePath`。`build_realm_dirs` 按 `dirs.{data,cache,config,logs,save}Path == "global"` 在「领域目录」与「全局目录」间二选一，落盘必须是**绝对路径**。

- 备份文件名 `config/dirs.json.vcmi-fv-backup`，**仅首次创建**，切换不覆盖；
- 还原用**备份原文**（不能写"官方默认路径"——增强版默认是 `My Games\vcmi`）；无备份则删除 `dirs.json` 回退默认。
- 隔离目录：`realms/<id>/vcmi/{config,cache,logs,Saves,Mods}`。
- **配置共享**：`settings.json` / `keyBindingsConfig.json` / `persistentStorage.json` / `mapeditor.ini` 默认硬链接共享，改为隔离时转私有副本；用 `.vcmi-fv-shared.json` 标记区分；删除只用 `remove_file`。
- **`lock`**：编辑权标记，仅本机创建时写；导入的领域不写 → 只读（`list_realms` 输出 `editable`）；导出时被排除。

### 进程守卫

`is_vcmi_running` 只判游戏本体（client / server / mapeditor），官方 `VCMI_launcher.exe` 可共存；实现用 `tasklist /FO CSV` 精确匹配第一列并排除自身 exe。`kill_vcmi` 只杀游戏本体。

拦截点：`activate_realm` / `deactivate_realm` / `set_realm_maps_blocked` / `repair_game_maps` / `apply_realm_maps_valve` / `set_realm_map_filter` / `realm_mod_ops(unlink|link)` / `realm_mod_{unref,bundle,delete,import}`。

## 5. 地图阀门（Maps）

VCMI 会把**游戏目录的 `Maps/`** 与用户目录的 `Maps/` 同时挂载，且无法通过 `dirs.json` 屏蔽游戏目录那一份——这是阀门存在的原因。

- 首次执行（懒执行）把真实 `Maps` 改名为 `Maps.vcmi-fv-native`；
- `blocked=true` 时 `remove_dir` 删 junction（**不能 `remove_dir_all`**：junction 被 `is_dir()` 判为真目录，递归会删掉真相源）；
- `false` 时重建透明态 junction，默认态对用户完全透明。
- 偏好存 `dirs.gameMaps`（`hide` / `show`）。

`repair_game_maps` 三动作：`unlink`（只删悬空联接）/ `merge`（新自带地图并入 native，重名 `-2` 递增）/ `revert`（删联接 + native 改名回 `Maps`）。

领域地图仓库在 `vcmi/Maps.vcmi-fv-realm/{maps,campaigns}/<lang>/…`（真相源，`index.json` 缓存按 size+mtime 复用），可视目录 `vcmi/Maps/` **全是 junction**，按 `maps.filterLang / filterLangTarget` 只挂目标语言 + `unknown`。

## 6. 领域模组管理

四种形态（`mod_manage.rs`）：

| 形态 | 语义 |
|---|---|
| `borrowed` | 借用清单，**纯台账、零物理动作**；激活时才由 `links::apply_realm_mods` 为 user 源建联接（game 源永不建链） |
| `bundled` | 整合副本：复制进领域 `Mods/`，随包分发 |
| `pinned` | 领域私有副本台账（下载 / 导入所得），永不被激活清理 |
| `realm_mod_ops` 的 `unlink` / `link` | 删 / 建私有副本或借用联接 |

- `list_realm_mods` 扫描 ∪ 台账补漏，附 `isLink / hasLedger / source / inBorrowed / inBundled / inPinned / metaOk`。
- `realm_mod_bundle` 复制本体，`force=false` 时按 `game` / `diverged` / `same` 返回判定。
- `realm_mod_import` 支持 zip 或含 `mod.json` 的目录，写 `.fv-install.json` 台账，覆盖走 `.fv-old` 无损交换。
- `check_realm_mod_conflicts` 判定整合重名是否真有风险（`game` 恒可见重复挂载 / `diverged` 目录指纹不一致）；`check_realm_mods_ready` 返回 `missingBundled / missingBorrowed / conflicts`。
- 依赖求解在前端 `lib/modDeps.ts`：`solveRealmDeps` 输出 `byRoot` 缺陷（`installed | repo | unavailable`），`solveDepClosure` 一键解析出 `refs / installs / deads`，`detectTranslationLang` 按领域语言默认勾选翻译包。

### 预设

`write_realm_preset` 确保 `presets.__realm__`（或 `realm.presetId`）存在并设为 `activePreset`，**已存在则不覆盖**（启用组合与携带清单解耦），并强制补 `vcmi` 与 `launcher.json` 的 `mods.required`。进入 / 退出时快照与恢复 `prevActivePreset`。

## 7. 自检与自愈

诊断码（`selfcheck.rs`）：

| 码 | 说明 | 可自动修复 |
|---|---|---|
| `ok` | 正常 | — |
| `realm_missing` | 领域目录缺失 | heal：还原 `dirs.json` + 清激活记录 |
| `dirs_mismatch` | dirs.json 与领域不一致 | heal |
| `active_mismatch` | 激活记录与实际不符 | heal |
| `maps_orphan` | 联接丢失 | heal：重建联接（App 启动期静默修复） |
| `maps_dangling` | 悬空联接 | 否（需 `unlink`） |
| `maps_realboth` | 真实目录与 native 并存 | 否（需 `merge` 或 `revert`） |

全 global 领域用 `realm_dirs_match` 比对 5 key，避免误判。

## 8. 导入导出（`.vcmirealm`）

- 包 = ZIP：包根 `realm.json`（含 `checksum`）+ `vcmi/{Mods,Maps,config}` + `assets/`（+ 可选 `showcase-cache/`）。
- 排除：`cache` / `logs` / `Saves` / `lock` / 用户态文件与**所有目录联接**（借用模组不打包）。
- ZIP 注释写魔数 `vcmirealm`；已压缩扩展名走 `Stored`；单遍流式求 `checksum`（按相对路径排序的 `(路径, 原始内容)` SHA-256）。
- **checksum ≠ sha256**：前者是包内条目集合的完整性校验和（导入侧复现比对），后者是整包文件哈希（写入索引供下载校验）。
- 导入：`conflict ∈ {error, overwrite, rename}`，冲突返回 `{code:"id_conflict", realmId, realmName}`；ZIP 路径净化只放行 `vcmi/` 与 `assets/`；导入**不写 `lock`**（只读）。
- 发布预览 `preview_realm_publish` 只读 `index.json` + `.git`，返回 `downloadUrl / prevVersion / nextVersion / versionRel(none|up|same|down) / entryExists`；`url_base` 推导优先级与版本降级阻断见 [repository-index](../guides/repository-index.zh-CN.md) §3。

## 9. 登场演出与素材（persona）

- `get_intro_state` 读 `.introPlayedAt`；`mark_intro_played` 写戳。
- 播放策略（`lib/realm/intro.ts`）：有内容且距上次 >24h 才播。
- `RealmIntroOverlay`：视频铺满（带声播放，被拦截则静音重试）> 壁纸 > 渐变；介绍音频**仅无视频时**播放；无视频则文本滚动（窄窗 <720px 改静态）；ESC / 右下角跳过（受 `intro.skipable`），淡出 200ms。
- `import_realm_asset` 复制素材到 `assets/<[sub/]>stem.<ext>`，`kind ∈ {wallpaper, cover, video, music, introMusic}`；`sub` / `stem` 做路径净化并清理同名异扩展名旧文件。
- BGM 由 `RealmShell` 用 `assets.music` 循环播放，`bgmMuted` 时 pause；巡礼复用同一开关。

## 10. 界面

`RealmShell` 页面枚举：`home / info / assets / switch / editor / settings / repository / showcase`。

| 页 | 内容 |
|---|---|
| `home` | 进入 / 信息 / 切换 / 退出 / 重播登场 / BGM 开关 / 巡礼入口 |
| `info` | 领域信息正文（复用百科详情展示层） |
| `assets` | 素材准备，就绪即自动放行 |
| `switch` | 领域列表（16:9 整卡封面 + 搜索）、新建 / 导入 / 远程 / 调整 / 删除（含进程守卫提示） |
| `editor` | 见 [realms-editor](./realms-editor.zh-CN.md) |
| `settings` | 复用 `SettingsView`（带作用域徽标） |
| `repository` | 见 [realms-repository](./realms-repository.zh-CN.md) |
| `showcase` | 见 [faction-showcase](./faction-showcase.zh-CN.md) |

## 11. 边界与容错

- 删除 junction **只能用 `remove_dir`**。
- 停用必须还原 `dirs.json` 原文备份。
- 激活失败会记录日志并提示，但仍进入领域（不把用户锁在外面）；自检失败同理不阻断。

## 12. 验证

```bash
npx tauri dev   # 真机：新建 → 激活（观察 dirs.json 与备份）→ 借用模组 → 屏蔽地图 → 停用 → 导出 → 导入
```

## 13. 开放问题

- 领域模式与主界面互斥（防争写），暂不支持"在领域内改全局模组"。
