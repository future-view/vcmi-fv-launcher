# 百科（Wiki）页面

> 状态：已实现 ｜ 最后核对：2026-09-30 ｜ 代码：`src/features/wiki/`、`src/lib/modcache/index.ts`
> 数据层的构建管线见 [modcache](./modcache.zh-CN.md)。本文只讲**展示与交互**。

## 1. 目标与范围

**做**：把已构建的百科缓存以 9 个类目离线浏览，支持搜索、详情分发、右键悬停浮层与条目互跳。

**不做**：解析原始 mod / LOD（一律读缓存）；构建与素材抽取（见 [modcache](./modcache.zh-CN.md)）。

## 2. 类目与搜索

类目（9）：`factions` / `creatures` / `heroes` / `artifacts` / `spells` / `skills` / `buildings` / `terrains` / `mods`。

- 搜索按**条目的全部语言名**（`name` 多语言对象的每个值：chinese / tchinese / english / …）+ 条目 `id` 做小写子串匹配，**不枚举具体语言**（新增界面语言无需改这里）；
- **列表排序**：按当前界面语言的显示名升序（`Intl.Collator`，对齐 VCMI `WikiWindow.cpp` 各分类的 `std::sort(a.name < b.name)`）。`buildings` 额外保留「中立置顶」；
- 列表滚动分页窗口 300 条，避免长列表卡顿。

### 2.1 筛选

中栏头部右侧是一个**固宽工具槽**（图标按钮，带已启用维度计数徽标），点击弹出筛选浮层。维度按类目声明在 `src/features/wiki/filters.ts` 的 `WIKI_FILTERS`：

| 类目 | 维度 |
|---|---|
| `factions` | 阵营类型（`alignment`） |
| `heroes` | 英雄职业（`class`，标签用条目自带 `className`）、英雄技能（`secondarySkills[].skill`，**多值**）、阵营名称（`factionId`，由 `heroClass.faction` 推导） |
| `creatures` | 阵营名称（`faction`）、生物级别（`level`，0 = 战争机器）、生物形态（`upgradedFrom`：基础/升级）、生物能力（`abilities`，**多值**） |
| `artifacts` | 宝物等级（`class`，标签与列表副标题共用 `artifactClassLabel`）、装备位（`slots`，**多值**）、是否组合宝物（`isCombined`）、来源模组 |
| `spells` | 法术派系（`schools`，**多值**：命中即属于该学派）、法术等级（`level`）、来源模组 |
| `buildings` | 阵营名称（`factionId`）、建筑类型（是否为招募建筑，按 id 前缀 `dwelling` 判定） |
| `mods` | 模组类型（`type`，标签复用 `t.mods.typeLabels`） |
| `skills` / `terrains` | 无（按钮保持**禁用态但占位**） |

约定：

- **新增一个筛选维度 = 往 `WIKI_FILTERS` 加一项**，UI 与布局零改动；选项与计数从当前类目**未筛选**的全量条目聚合；
- 浮层内**一次只展开一个维度**（维度 ≥2 时显示 Tab 行，已选值的维度带小圆点）——否则多个大列表各带搜索框与独立滚动区，会叠成「面板滚动套列表滚动」的嵌套滚动。当前维度的控件形态由选项数决定（≤6 用 chip 行、>6 用「搜索 + 单选列表」），浮层整体限高 70vh；
- 同一维度单选、不同维度取交集（AND），与搜索叠加；切类目时清空筛选（对齐 VCMI 切类目清空搜索框）；
- **多值维度的语义是「命中即属于」**（法术学派、英雄技能、宝物装备位、生物能力），且同一实体内重复值只计一次：装备位要先经 `slotKeyOf` 归并 —— VCMI 用 `MISC1`~`MISC5` 表达「可放任一杂物槽」，直接展开会得到 5 个同义选项；
- 显示名映射两条：阵营名取 `cache.json` 顶层 `factionNames`（后端按 `factions[].name` 输出，前端跨 mod 合并）；来源模组名取 `index.json` 条目名（前端在 `applyIndex` 建 `modId → 多语言名`，无需额外 IPC）。两者缺失时均回退 id 本身；
- 「来源模组」**只挂在 `artifacts` / `spells`**（这两个类目没有阵营维度，且原版内容全部收敛到 `core`，故该维度等价于「原版 vs 某个 mod」）。不挂载的理由：`factions`（1 个城镇 mod 恰好提供 1 个阵营 → 筛选结果与列表一一对应、计数恒为 1）、`heroes` / `creatures`（条目按 `core.<faction>` 伪模组分散，与「阵营名称」重复）、`buildings`（已被「阵营」覆盖）、`skills`（原版技能全收敛到 core）、`terrains`（游戏级数据、跨 mod 去重后不归属单一模组）、`mods`（本身即模组清单）；
- **布局不变量**：工具槽宽度、浮层都不参与列表布局 —— 任何类目下输入框宽度与列表上沿均恒定（列表上方不新增任何行）。

## 3. 数据来源

- `loadWikiIndex()` 读 `mods/index.json`，`loadModCache(modId)` 读 `mods/<modId>/cache.json`。
- 图片经 `fullImgUrl` / `fullImgCandidates` + `convertFileSrc` 拼绝对 URL；高清变体 `@Nx` 由 `SafeImg` 自动回退。
- **幽灵条目过滤**：索引条目先与 `visibleModKeySet()`（实时 `scan_mods` ∪ core 伪模组）求交，剔除已卸载或属于其它领域的条目。缓存 `index.json` 只合并不删除，过滤发生在读侧。

## 4. 详情分发

`detail/EntryDetail.tsx` 按 category switch；各类目组件自带标题，跳过通用大图 header。

| 组件 | 内容 |
|---|---|
| `FactionDetail` | 城镇幕布大图 + 生物表（statIcons 表头）+ 建筑表 + 英雄表 |
| `CreatureDetail` | 阵营 `creatureBackground` 底图 + canvas 播放战斗 DEF 动画（只解当前动作分组）+ 能力格右键气泡 |
| `HeroDetail` | 头像、传记、特长、初始部队（排除战争机器，按 `startingStackChances` 取概率）、副技能 / 法术行 |
| `BuildingDetail` | 标题 + 大图 + 描述 + `formatCost` 花费 |
| `SpellDetail` | 派系 + 等级 / 魔法值 / 造价 + 初 / 中 / 高级分级描述 |
| `SkillDetail` | basic / advanced / expert 三级描述 |
| `ArtifactDetail` | 效果行 + 价格 |
| `TerrainDetail` | 地形图块 + 移动消耗 + `nativeTowns` |
| `ModDetail` | 读 `cache.json` 顶层 `modMeta`（多语言名 / 描述 / 版本）+ 按类目分节列出可跳转条目 |

公共件 `detailShared.tsx`：`ANIM` 动画组号 / `CREA_PREVIEW`、`SafeImg`、`DetailProps`、`DetailHeader`、`StatRow`、`formatCost` / `formatResourceAmount`（资源图标优先 `resources` 段，回退 `core/images/resources/<idx>.webp`）、`WikiLink`。

> **已知数据限制**：原版（core）建筑的**花费没有任何 JSON 数据源**（`town.buildings` 为空、`buildingsLibrary.json` 只有 id/bonuses，VCMI 把 H3 建筑花费硬编码在引擎里），因此原版建筑 cost 必然为空。这是数据限制，不是缺陷。

## 5. 悬停浮层（WikiPopup）

`WikiPopup.tsx` 提供 Provider，两种形态：

- **文本气泡** `CRClickPopup`：居中于光标，`fitToScreen(10)`，z-200；
- **完整窗** `CStackWindow` / `CHeroOverview`：屏幕居中，creatures 460px、heroes 620px。

触发（`WikiPopupContext.ts`）对齐 VCMI 的 EventDispatcher：**右键按下即显示、右键松开即关**；触屏为 500ms 长按（移动 >12px 取消）。`usePopupTriggers()` 返回可复用 props 工厂，供行级元素挂载。

## 6. 多语言取词

- UI 语言 → VCMI 语言标识：`contentLangKey`（chinese / tchinese / english）。
- 条目名：`pickName`（当前语言 → english 严格回退）。
- 详见 [i18n](./i18n.zh-CN.md)。

## 7. 边界与容错

- 缓存缺失时详情返回占位，不报错；巡礼依赖的 slide 缺缓存返回 `missing_slide("nocache")`。
- 图片加载失败由 `SafeImg` 逐级回退（高清 → 原图 → 隐藏），不出现破图。

## 8. 验证

```bash
npm run build
npx tauri dev   # 真机：9 类目浏览、右键悬停、条目互跳
npm run verify:wiki-cache   # 缓存校验（对着刚产出的目录）
```

## 9. 开放问题

- 百科条目只支持缓存内已有类目；新增类目需同时改后端采集与前端分发。
