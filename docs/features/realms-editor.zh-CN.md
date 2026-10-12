# 领域编辑器

> 状态：已实现 ｜ 最后核对：2026-10-12 ｜ 代码：`src/features/realms/editor/`、`src-tauri/src/realms/meta.rs`、`mod_manage.rs`、`maps.rs`
> 领域运行时行为见 [realms](./realms.zh-CN.md)。

## 1. 目标与范围

**做**：编辑 `realm.json` 的全部内容——元数据、登场演出、模组组合、地图、巡礼、目录隔离策略，并可导出为 `.vcmirealm`。

**不做**：运行时状态（激活 / 自检）的编辑。

## 2. 结构

`RealmEditor` 负责状态编排（表单 state / 校验 / 保存 / 导出）与标签栏、弹窗；六个标签页经 `XxxTabProps` 接收扁平 props。

| 标签 | 内容 |
|---|---|
| **basic** | id（只读）、版本、作者、多语言名称与描述、标签（每语言 ≤6 个，单标签 ≤16 字） |
| **intro** | 壁纸 / 封面 / 主题曲 / 介绍视频 / 介绍音乐 / 时长 / 可跳过 / 显示领域名 / `showGameName` / `loopMusic` / 遮罩 |
| **mods** | 视图 A：预设条 + `ModBrowser`；视图 B：`RealmModManagerPanel`（已实装 / 可引用 / 可下载 / 可更新） |
| **maps** | 三栏：目录树 / 列表 / 详情；引入 / 上传 / 移动 / 删除 / 屏蔽阀门 / 语言过滤 |
| **showcase** | 牌组（自动 / 手动）、城镇与英雄幻灯片、音频 |
| **adv** | `dirs` 五路径（每项可设 `global`）、`configShare` 四个开关、`info.showCore` |

弹窗：`ExportRealmDialog`（导出）、`DepResolveDialog`（依赖缺陷）、`DepClosureDialog`（一键解析预览）。

## 3. 模组标签页

- 复用 `features/mods/ModBrowser`（与主界面同源），勾选结果写回领域预设。
- 依赖求解能力（`solveRealmDeps` / `solveDepClosure` / `detectTranslationLang`）的口径见 [realms](./realms.zh-CN.md) §6；编辑器侧只负责消费——徽标 / 角标 / 「去处理」与一键解析弹窗。
- `RealmModManagerPanel` 区分四种来源视图，对应 borrowed / bundled / pinned / 可下载，操作走 `realm_mod_{ref,unref,bundle,delete,import}`。

## 4. 地图标签页

- 真相源 / junction / 语言过滤的运行时语义见 [realms](./realms.zh-CN.md) §5；编辑器操作：`import_realm_maps_dir`（`dry_run` 预检 + `skip|overwrite|rename`）、`upload_realm_map`、`move_realm_map`（禁止跨 kind）、`delete_realm_map`（路径穿越防护）、`list_realm_maps`；阀门 `apply_realm_maps_valve`（仅激活时生效）与语言过滤 `set_realm_map_filter`。

## 5. 保存语义

- 保存走 `update_realm`：**深合并**，`id` 不可改，内容无变化则不写盘、不刷 `updatedAt`。
- 预设由编辑器整体接管时走 `write_realm_presets`。

## 6. 边界与容错

- `id` 非法在**创建**时就报错（不静默清洗），避免后续路径问题。
- 依赖未满足不阻断保存，只在 UI 标缺陷（"共存优先于硬阻断"的取向）。
- 导出排除联接与用户态文件，避免把借用模组打进包。

## 7. 验证

```bash
npx tauri dev   # 真机：新建领域 → 六标签逐项编辑 → 保存 → 导出 → 导入校验
```

## 8. 开放问题

- 巡礼标签页目前只能配置牌组与音频，不能自定义单站版式。
