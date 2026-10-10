# `modSettings.json` 与预设格式

> 状态：已实现 ｜ 最后核对：2026-09-30 ｜ 代码：`src/lib/vcmiPreset.ts`、`src-tauri/src/commands/mod_settings.rs`
> 读写流程见 [presets](../features/presets.zh-CN.md)。本文是**格式参考**。

## 1. 顶层结构

```jsonc
{
  "activePreset": "我的组合",
  "presets": {
    "我的组合": {
      "mods":    { "mod.id": true, "other.mod": false },
      "settings": { "parent.mod": { "child": true, "other-child": false } }
    }
  },
  "activeMods":  { ... },   // 旧格式，写入时保留（VCMI 1.5 兼容）
  "validatedMods": { ... }  // VCMI 的校验缓存，启动器只清理不生成
}
```

| 字段 | 说明 |
|---|---|
| `activePreset` | 当前激活的预设名 |
| `presets` | `名称 → { mods, settings }` |
| `activeMods` | 旧格式字段，保留以兼容 VCMI 1.5 |
| `validatedMods` | VCMI 校验缓存，由 `clear_mod_validation` 清理 |

## 2. `settings` 的子模组键

结构：`父modID -> { 子modID: bool }`。

- 子模组 ID 形态为 `parent.child`；
- 写入 VCMI 时子键取父前缀后**小写**（`toVcmiPreset`）；读回时大小写不敏感（`fromVcmiPreset`）；
- 未显式设置时，按模组的 `keepDisabled` 取反推断默认开关。

## 3. 保留 scope

`core` 与 `vcmi` 是 VCMI 保留 scope：

- 运行时自动加载，**不写入用户选择**；
- 前端列表隐藏（`RESERVED_MOD_IDS`），但写盘时保留在 `mods` 中。

## 4. 单预设导入导出

导出格式：

```json
{ "name": "我的组合", "mods": { ... }, "settings": { ... } }
```

导入时读 `name` 存入 `presets`，然后**移除 `name` 字段**再落盘。

## 5. 领域预设

领域隔离目录下同样有一份 `modSettings.json`，预设名固定 `__realm__`（可由 `realm.presetId` 覆盖）。写入走 `write_realm_presets` / `write_realm_preset`，**已存在则不覆盖**（启用组合与携带清单解耦）。

## 6. 写盘纪律

- 写前自动生成 `modSettings.json.backup`（`commands/mod_settings.rs`），patch / clear 路径同款。
- 预设 ID 校正走 `resolvePresetIds` 三级匹配（id → diskName → 归一化）。
- **VCMI 的 presets 迁移只在 `presets` 为空时发生一次**，不要靠反复写旧格式去触发。
