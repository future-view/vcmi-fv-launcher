# 百科（Wiki）页面

> 状态：已实现 ｜ 最后核对：2026-09-30 ｜ 代码：`src/features/wiki/`、`src/lib/modcache/index.ts`
> 数据层的构建管线见 [modcache](./modcache.zh-CN.md)。本文只讲**展示与交互**。

## 1. 目标与范围

**做**：把已构建的百科缓存以 9 个类目离线浏览，支持搜索、详情分发、右键悬停浮层与条目互跳。

**不做**：解析原始 mod / LOD（一律读缓存）；构建与素材抽取（见 [modcache](./modcache.zh-CN.md)）。

## 2. 类目与搜索

类目（9）：`factions` / `creatures` / `heroes` / `artifacts` / `spells` / `skills` / `buildings` / `terrains` / `mods`。

- 搜索按「中文名 + 英文名 + 条目 id」做小写子串匹配；
- 列表滚动分页窗口 300 条，避免长列表卡顿。

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
