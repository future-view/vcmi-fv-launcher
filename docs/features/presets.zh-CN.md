# 预设管理

> 状态：已实现 ｜ 最后核对：2026-09-30 ｜ 代码：`src/features/mods/PresetBar.tsx`、`PresetSaveDialog.tsx`、`ModView.tsx`、`src/lib/vcmiPreset.ts`、`builtinPresets.ts`、`langMods.ts`

## 1. 目标与范围

**做**：把用户的模组组合保存为 **VCMI 原生** `modSettings.json` 预设，支持切换、新建（含继承）、重命名、导入导出官方格式、删除、生成启用报告。

**不做**：领域隔离环境下的预设写入（走 `write_realm_presets`，见 [realms](./realms.zh-CN.md)）。

## 2. 数据模型

`modSettings.json` 顶层字段：

| 字段 | 含义 |
|---|---|
| `activePreset` | 当前激活的预设名 |
| `presets` | `{ <name>: { mods, settings } }` |
| `activeMods` | 旧格式字段，写入时保留（兼容 VCMI 1.5） |
| `validatedMods` | VCMI 的校验缓存，启动器只清理不生成 |

`settings` 结构：`父modID -> { 子modID: bool }`。

**保留 scope**：`core` 是 VCMI 内置 scope，运行时自动加载，**不写入用户选择**。

## 3. 格式互转（`lib/vcmiPreset.ts`）

| 方向 | 规则 |
|---|---|
| `toVcmiPreset` | 扁平选中 → `{mods, settings}`；子模组键取父前缀后**小写**；冲突子模组显式钉 `false` |
| `fromVcmiPreset` | 反向；大小写不敏感；未设置时按 `!keepDisabled` 推断默认开 |
| `excludeBaseSubmods` | 剔除基线子模组，避免污染用户选择 |

ID 校正走 `resolvePresetIds` 三级匹配（id → diskName → 归一化），解决历史预设里 ID 大小写 / 形态不一致的问题。

## 4. 内置预设与基线

- 内置预设仅 `__official__`：`vcmi` + `core`。**不再写死任何按语言的模组名字**——发行版碎片化使中文包名字无规范（增强版 `simplified-chinese-translation` / Expansion `ChineseTrueTypeFonts` / 官方 `chinese-translation-vcmi`）。语言包改用 `lib/langMods.ts` 按「已装模组的 `modType==Translation && language==当前语言`」分类发现（详见 `docs/features/lang-mods.zh-CN.md`）。
- `launcher.json` 的 `mods.builtin_presets` 仍可按 `id` **字段级合并**覆盖内置项（`mergeBuiltinPresets`），项结构 `{id, label, mods, modsByLang?, locked}`；`modsByLang` 字段保留供按需按 id 覆盖，但默认不再使用。
- 基线 = `RESERVED_MOD_IDS`（`vcmi`、`core`）+ `launcher.json` 的 `mods.required`；UI 上标 `locked` 不可取消。

## 5. 操作与命令走向

| 操作 | 路径 |
|---|---|
| 保存 / 切换 / 新建继承 / 重命名 / 删除 / 导入 / 导出 | **全部只走 `write_mod_settings`**（`ModView.persistPresets` → `writePresetsSafe`），保存前经 `toVcmiPreset` 转换 |
| 语言模组增删 | `patch_preset_lang_mods`（`lib/langMods.ts`），返回磁盘缺失项 |
| 领域隔离预设 | `write_realm_presets` |
| 清除 VCMI 校验缓存 | `clear_mod_validation` |

**写前备份**：后端写 `modSettings.json` 前自动生成 `.json.backup`（`commands/mod_settings.rs`），patch / clear 路径同款。

## 6. 启用报告

`ModView` 的 `onReport` 生成 Markdown：`# 标题 + 生成时间 + 基线 + 当前预设 + 模组名列表`，在 `PresetBar` 的弹窗中展示，便于用户把组合贴到社区。

## 7. 保存时机

- `writePresetsSafe`（防抖合并）与 `writePresetsNow`（立即）是**唯一写盘出口**，避免多处散落写入互相覆盖。
- 预设内容无变化时 `update_realm` 一类接口不写盘、不刷新时间戳；预设同理只在真正变动时落盘。

## 8. 实现映射

| 关注点 | 位置 |
|---|---|
| 预设下拉 + 更多菜单（新建 / 重命名 / 导入导出 / 删除 / 报告） | `features/mods/PresetBar.tsx` |
| 新建 / 另存命名弹窗（重名与空名禁用确认） | `features/mods/PresetSaveDialog.tsx` |
| 页面容器、唯一写盘出口、报告生成 | `features/mods/ModView.tsx` |
| 原生格式互转 | `lib/vcmiPreset.ts` |
| 内置预设与配置合并 | `lib/builtinPresets.ts` |
| 语言模组同步 | `lib/langMods.ts` |
| 类型基线 | `lib/minimapBase.ts` |

## 9. 边界与容错

- 导入单预设格式为 `{name, mods, settings}`；导入时读 `name` 存入 `presets` 后**移除 `name` 字段**。
- 预设引用的模组在本地不存在时，右栏标红并归入 `Missing` 组；启动前模态列出原因。
- VCMI 的 presets 迁移只在 `presets` 为空时发生一次，不能反复写 1.5 的 `activeMods` 格式去"触发迁移"。

## 10. 验证

```bash
npm run verify:settings   # 结构解析、子 ID 命名规律、官方单预设导出、写入 + 备份（临时副本）
npx tauri dev             # 真机：新建继承 → 切换 → 导出 → 导入 → 删除
```

## 11. 开放问题

- 导出目前只支持单预设；多预设批量导出未实现。

## 12. 跨预设残留引用清洗（启动前唯一出口）

**问题**：`persistentStorage.json` 由引擎自写，会保留**别的预设**里模组的标识符；
切预设后引擎拿它去查表会崩（详见 [vcmi-dirs](../reference/vcmi-dirs.zh-CN.md) §10）。

**实现**：`src-tauri/src/vcmi/persistent.rs`（`vcmi/` 领域层，**无 tauri 类型**）

| 关注点 | 落点 |
|---|---|
| 生效配置目录 | `VcmiPaths::effective_config_dir()` —— 按 `<game_dir>/config/dirs.json → userConfigPath` 解析，**领域模式自动命中领域隔离目录**；**不得**用全局快照 `AppPaths.user_config_dir`（领域激活后它不更新） |
| 启用组合 | `presets[activePreset].mods` ∪ `RESERVED_SCOPES`（`core` / `vcmi`），比较时大小写不敏感；子模组 `parent.child` 按父级判定 |
| 失效判定 | 值含 `:` 且 scope 不在白名单 ⇒ 剔除；不含 `:`（= 内置 / core）恒保留 |
| 清洗范围（白名单） | `battleModeSettings.selectedTown` / `.selectedTerrain` / `.slots[].{selectedHero, selectedArmy[].type, secSkillLevel[].skill, artifacts, spells}`、`rmg.rmg.lastSettings.roads` |
| 时机 | `launch_game` / `launch_editor` / `launch_native_launcher`：spawn **之前** |

⚠️ 安全红线（既有容错底线，勿移除）：

1. 只在**发现失效引用**时写盘；无变化不碰文件（不刷 mtime、不产生备份）。
2. 写前备份 `persistentStorage.json.backup`（与 `write_mod_settings` 同款约定）。
3. 解析失败 → **跳过清洗**，只记日志；绝不降级为空对象覆写（会清空玩家既有设置）。
4. **原地覆写**（走 `trunc`），**禁止** temp + rename —— 领域共享配置是硬链接，
   rename 会断开共享（见 [realms](./realms.zh-CN.md) §5）。
5. 清洗失败**不阻断启动**（与既有「自检失败只记日志」同口径）。
