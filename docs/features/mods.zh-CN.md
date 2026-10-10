# 模组浏览与选择

> 状态：已实现 ｜ 最后核对：2026-09-30 ｜ 代码：`src/features/mods/`、`src/lib/modScanner.ts`、`modScope.ts`、`modHelpers.ts`、`modCompat.ts`

## 1. 目标与范围

**做**：把游戏目录与用户目录下的全部模组（含子模组）以可视化方式呈现，让用户勾选组合，并在勾选时自动拉齐依赖、拦截冲突、标出缺失。

**不做**：模组的下载 / 更新 / 删除（见 [mods-repository](./mods-repository.zh-CN.md) 与领域编辑器）；预设的持久化（见 [presets](./presets.zh-CN.md)）。

## 2. 术语

| 术语 | 含义 |
|---|---|
| root mod | 顶层模组，一个目录 + 一个 `mod.json` |
| submod | 子模组，ID 归并为 `parent.child`，开关记在父模组的 `settings` 中 |
| 保留 scope | `vcmi` / `core`，VCMI 运行时自动加载，前端列表隐藏但写盘时保留 |
| 基线 | 任何预设都强制包含的模组：保留 scope + `launcher.json` 的 `mods.required` |
| FilterMode | 左栏三态过滤：`unjoined`（未加入）/ `partial`（部分子项）/ `all` |

## 3. 用户可见行为

### 左栏（浏览）

- **搜索**：按名称 / 描述 / 作者匹配。
- **类型标签**：按 `TYPE_ORDER` 排序的类型过滤；未声明类型的模组由 `classifyMod` 兜底分类。
- **三态过滤**：`unjoined` / `partial` / `all`。
- **父项展开 / 收起**：子模组挂在父项下；城镇类模组另有独立 tab 分组。
- 排序一律按**本地化名称** `localeCompare`。

### 右栏（已选）

- 双视图：`tree`（可交互）与 `grouped`（只读聚合）。
- 分组顺序按 `TYPE_ORDER`，`Missing` 组**永远排最后**。
- 支持全部展开 / 折叠。

### 卡片

| 元素 | 含义 |
|---|---|
| `townCard` | 城镇类模组的大卡片幕布图 |
| `updateTip` | 仓库中存在更新版本 |
| `depTip` | 被其他模组依赖（禁用会破坏他人） |
| `hasConflict` | 与已选模组冲突，标红 |
| `locked` | 基线必需，强制启用 |

## 4. 城镇幕布图与 hover 预览

- 取图：`ModCard` 的 `useTownCacheImage` 从百科缓存读图——小图标取 `creatureBackground`，大图**优先 `preview`，为空回退 `background`**。
- 真城镇判定：`type === "Town" && factions.length > 0`。
- 开关：`set_town_preview` 落 `launcher.json` 的 `ui.town_preview`；前端 `lib/townPreview.ts` 先更新内存再落盘，`useSyncExternalStore` 订阅。
- 关闭开关时紧凑行**直接不读大图**（省 IO）。
- hover 三态 `hidden → in → out`：150ms hover intent + 150ms 淡入淡出。

> 幕布图唯一来源是 `build_towns` 产物（`images/town/<fid>.webp`）；`preview` 仅在 `background` 为空时才回退合成到 `townscreen/`。详见 [modcache](./modcache.zh-CN.md)。

## 5. 依赖与冲突（核心逻辑）

勾选一个模组时（`ModBrowser.toggle`）：

1. **递归闭包拉齐依赖**：沿 `depends` 递归（含依赖的依赖、父模组、默认子项）；`keepDisabled` 子项跳过；与已启用项冲突的子项单独排除。
2. **必需依赖守卫**：若该模组被他人 `depends`，禁止禁用。
3. **冲突守卫**：双向判定（正向 + 反向），精确 ID 匹配、大小写不敏感，**不按根模糊匹配**。
4. **引擎版本守卫**：`compatibility` 不满足当前引擎版本时拦截。
5. 拦截结果经 `onDependencyBlocked` 回调抛出（领域编辑器复用同一组件时用它做提示）。

冲突集合 `conflictSet` 传给 `ModCard` 决定标红。

## 6. 数据模型

`mod.json` 关键字段（后端 `vcmi/scan.rs` 解析）：

`name` / `description` / `modType` / `version` / `author` / `depends` / `softDepends` / `conflicts` / `keepDisabled` / `language` / `compatibility{min,max}` / `factions`。

- 未声明 `factions` 时探测 `Content/config/factions`。
- submod 归并为 `parent.child`；子目录名 `Mods`（或 `mods`）。
- `RESERVED_SCOPES = ["core","vcmi"]` 与点开头目录在顶层扫描中跳过。
- `NON_I18N_KEYS` 之外的对象块收为多语言文案。
- 同 ID 时**用户目录优先**于游戏目录。

本地化取值回退链：`tchinese → chinese → english`（`modScanner.ts`）。版本兼容复刻 VCMI 的 `CModVersion::compatible`：major 必须相等，只比较双方都写到的段（`modCompat.ts`）。

## 7. 实现映射

| 关注点 | 位置 |
|---|---|
| 页面容器与唯一数据源（selected / presets / activePreset / rootMods） | `features/mods/ModView.tsx` |
| 通用浏览 / 勾选组件（不感知预设） | `features/mods/ModBrowser.tsx` |
| 卡片渲染与幕布图 | `features/mods/ModCard.tsx` |
| 通用详情抽屉（富文本描述 / 兼容区间 / 大图） | `features/mods/DetailDrawer.tsx` |
| 管理页行详情抽屉（版本 / 大小 / stars / changelog） | `features/mods/ModRowDrawer.tsx` |
| 模组管理页（安装 / 更新 / 删除 / 模组源） | `features/mods/ModManagerPanel.tsx` |
| 仓库源就地编辑 | `features/mods/ModSourcePopover.tsx` |
| 类型基线与兜底分类 | `lib/minimapBase.ts` |
| 扫描结果与本地化 | `lib/modScanner.ts` |
| 保留 scope 过滤 | `lib/modScope.ts` |
| 预设 ID 三级校正（id → diskName → 归一化） | `lib/modHelpers.ts` |
| 版本兼容判定 | `lib/modCompat.ts` |
| 领域依赖求解器 | `lib/modDeps.ts` |

## 8. 边界与容错

- 定制版部分 `Content.zip` **带密码**：zip 提取捕获异常回退色块，绝不崩溃（这是项目容错底线，不要移除）。
- 缺失模组：与本地实际扫描比对，右栏标红并归入 `Missing` 组。
- 依赖未满足 / 冲突存在时禁止启动，并在模态中列出原因（缺失 / 依赖 / 冲突）。

## 9. 验证

```bash
npm run build          # 类型通过
npm run verify:settings # 真实 modSettings 结构解析与写入（临时副本）
npx tauri dev          # 真机：勾选依赖链、制造冲突、观察标红
```

## 10. 开放问题

- `grouped` 视图只读是刻意取舍（避免两处编辑态），若用户反馈需要交互可再评估。
