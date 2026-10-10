# 设置面板

> 状态：已实现 ｜ 最后核对：2026-09-30 ｜ 代码：`src/features/settings/`、`src/lib/settingsSchema.ts`、`src-tauri/src/commands/vcmi_settings.rs`、`commands/settings.rs`

## 1. 目标与范围

**做**：以等高卡片 + 全量描述呈现 VCMI 官方设置（12 节），并承载启动器自有设置（AI 组件、HD 版状态、镜像规则、更新）。

**不做**：VCMI 设置项的语义解释（文案取自官方 schema 与 `settingsText.*`）。

## 2. 分组与控件

`SETTING_TAGS` 共 13 个标签：VCMI 的 `general / video / adventure / battle / gameTweaks / ai / input / launcher / mods / server / lobby / logging`，外加 `fvLauncher`（启动器自身，刻意排最后，与 VCMI 第 8 个 `launcher` 段区分）。

控件类型（`SettingCtl`）：`switch` / `select` / `slider` / `sliderPct` / `number` / `text` / `btnList` / `toggleGroup` / `resolutionCombo` / `scalingGroup` / `json` / `fullscreenGroup` / `creatureDisplay`。

- 跨键互斥项配 `altKey`；支持 `disabledWhen` 依赖禁用；
- `dynamic: "scaling" | "resolution" | "aiModels"` 表示候选项运行时生成。

## 3. 显示层级

| 配置项 | 位置 | 说明 |
|---|---|---|
| `settings_level` | `launcher.json` 的 `ui` | `common` / `advanced` / `all`，默认 `common` |
| `settings_show_desc` | 同上 | 是否显示详细描述 |
| `show_all_settings` | 同上 | **保留兼容字段**，已不再驱动过滤 |

入口命令 `set_launcher_pref(show_all_settings, settings_level, settings_show_desc)`。界面顶部有 sticky 搜索。

## 4. 作用域徽标（ScopeBadge）

`variant: "global" | "realm"`：标识该配置目录是**全域共享**（琥珀 `Share2`）还是**仅本领域独立**（靛蓝 `Lock`）。hover 展开、点击钉住、Esc / 外点关闭；面板显示作用域说明、完整配置目录与「复制路径」。

## 5. 读写 VCMI 设置

| 命令 | 行为 |
|---|---|
| `read_vcmi_settings` | 传 `config_dir` 就读领域目录下的 settings.json，否则全局；文件不存在返回 `{}` |
| `write_vcmi_settings` | **读-合并-写**（非整体覆盖），写前备份 `.json.backup`；支持 `__vcmi_fv_delete__` 扁平键列表删除并回退官方默认；对象递归合并、数组 / 标量替换 |
| `set_vcmi_language` | 把启动器语言同步写入 `general.language`，幂等 |

> `set_vcmi_language` 是**必需**的：否则 VCMI 会把 Translation 类模组判为 BROKEN 并回写移除。解析失败直接报错，**不降级覆写**。

**通道区分**：schema 中键名以 `fv.` 开头的项走 `launcher.json` 通道，不进 VCMI 路径。

## 6. 启动器自有设置卡

| 卡片 | 能力 | 命令 |
|---|---|---|
| `AiRuntimeCard` | 三态：已安装（版本 + 二次确认移除）/ 未装且有包（安装 + 进度 + 取消）/ 未装且无包（仅提示）。组件永远可选，未装则 AI 回退传统算法 | `runtime_status` / `runtime_install` / `runtime_cancel` / `runtime_uninstall` |
| `HdEditionCard` | HD 版 `.pak` **只读**状态：是否检测到、来源目录、可用档位、是否已关闭、改动需重建缓存；监听 `vcmi-fv:mods-installed` 刷新 | `hd_edition_status` |
| `MirrorRulesCard` | L2 镜像规则表编辑：启用 / 备注 / 排序 / 删除（二次确认）/ hosts 标签（GitHub 五域预填）/ mode 下拉 / 前缀或模板 + 实时预览。上限 10 条，默认空 | `set_network_mirror_rules` |
| `UpdateDialog` | 可选更新弹窗，可稍后 / 跳过此版本；关闭不中断下载，重开从后端阶段恢复 | `check_launcher_update` / `set_update_pref` |
| `ForcedUpdateOverlay` | 强制更新全屏阻断（无关闭 / 无跳过） | 见 [launcher-update](./launcher-update.zh-CN.md) |

## 7. 运行信息弹窗（RuntimeInfoDialog）

只读键值表，按当前 `configDir` 取数（`get_runtime_info(config_dir)`）：

- **本配置目录**：lastMap / lastSave / gameDataLanguage / modsEnabled / setupCompleted / mainWindow.geometry；
- **环境（全局）**：vcmiVersion / modsInstalled / recommendedResolution。

## 8. 复用关系

`SettingsView` 被两处复用：主界面（`App.tsx`）与领域设置页（`RealmShell`），靠 props 表达差异：`configDir` / `scope` / `hideTitle`。

## 9. 边界与容错

- 任何写操作先备份；`write_vcmi_settings` 是合并而非覆盖，避免清掉 VCMI 新增字段。
- 设置项文案缺失时回退英文原值，不空白。

## 10. 验证

```bash
npm run gen:settings   # 从官方 schema 重新生成默认值（改动 schema 后）
npm run build
npx tauri dev          # 真机：三档等级切换、领域作用域徽标、运行信息
```

## 11. 开放问题

- `show_all_settings` 已无实际作用，是否从配置里彻底移除待定（目前为兼容保留）。
