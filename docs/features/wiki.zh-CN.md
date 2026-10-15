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
| `heroes` | 阵营名称（`factionId`，由 `heroClass.faction` 推导）、英雄职业（`class`，标签用条目自带 `className`）、英雄技能（`secondarySkills[].skill`，**多值**） |
| `creatures` | 阵营名称（`faction`）、生物级别（`level`，0 = 战争机器）、生物形态（`upgradedFrom`：基础/升级）、生物能力（`abilities`，**多值**） |
| `artifacts` | 宝物等级（`class`，标签与列表副标题共用 `artifactClassLabel`）、装备位（`slots`，**多值**）、是否组合宝物（`isCombined`）、来源模组 |
| `spells` | 法术派系（`schools`，**多值**：命中即属于该学派）、法术等级（`level`）、来源模组 |
| `buildings` | 阵营名称（`factionId`）、建筑类型（是否为招募建筑，按 id 前缀 `dwelling` 判定） |
| `mods` | 模组类型（`type`，标签复用 `t.mods.typeLabels`） |
| `skills` / `terrains` | 无（按钮保持**禁用态但占位**） |

约定：

- **新增一个筛选维度 = 往 `WIKI_FILTERS` 加一项**，UI 与布局零改动；
- **维度联动（faceted search）**：每个维度的选项与计数基于「**搜索结果** ∩ **其它维度已选值**」，**排除自身** —— 不排除的话，选中「城堡」后再看阵营维度，其余阵营计数全变 0，就没法改选了。由此得到一条硬性质：**任何组合都选不出 0 条结果**（计数 0 的选项置灰不可点）。代价是当前展开维度的计数不再反映全量分布，要看全量分布把其它维度清掉即可。计数 0 一律**置灰而非隐藏**：隐藏会让选项列表随选择跳动，"刚才还在的选项突然没了"比灰色更难理解；「全部」永远可点，它正是把该维度从联动条件里摘掉的那一步；
- 搜索被纳入联动基准（浮层拿到的 `entries` 是 `searched` 而非全量 `entries`）：否则「全部 (12)」指全量 12 条、而列表只剩搜索命中的 3 条，数字对不上。搜索 = 先定范围，维度在其之上联动；
- **数组顺序 = Tab 顺序，`[0]` 是打开浮层时默认展开的维度**。一律「先粗后细」（生物/英雄/建筑：阵营在前）：联动会让后置维度被前置维度的选择收窄，粗粒度先选才能用完整分布收窄人群，反过来则一开始就面对被裁过的列表；
- 浮层内**一次只展开一个维度**（维度 ≥2 时显示 Tab 行，已选值的维度带小圆点）——否则多个大列表各带搜索框与独立滚动区，会叠成「面板滚动套列表滚动」的嵌套滚动。当前维度的控件形态由选项数决定（≤6 用 chip 行、>6 用「搜索 + 单选列表」），浮层整体限高 70vh；
- 同一维度单选、不同维度取交集（AND）；切类目时清空筛选（对齐 VCMI 切类目清空搜索框）；
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
| `CreatureDetail` | 阵营 `creatureBackground` 底图 + canvas 播放战斗 DEF 动画（只解当前动作分组）+ 统计表 / 招募花费 / 升级链 / 招募建筑 / 能力格（详见 §4.1） |
| `HeroDetail` | 头像、传记、特长、初始部队（排除战争机器，按 `startingStackChances` 取概率）、副技能 / 法术行 |
| `BuildingDetail` | 标题 + 大图 + 描述 + `formatCost` 花费 |
| `SpellDetail` | 派系 + 等级 / 魔法值 / 造价 + 初 / 中 / 高级分级描述 |
| `SkillDetail` | basic / advanced / expert 三级描述 |
| `ArtifactDetail` | 效果行 + 价格 |
| `TerrainDetail` | 地形图块 + 移动消耗 + `nativeTowns` |
| `ModDetail` | 读 `cache.json` 顶层 `modMeta`（多语言名 / 描述 / 版本）+ 按类目分节列出可跳转条目 |

公共件 `detailShared.tsx`：`ANIM` 动画组号 / `CREA_PREVIEW`、`SafeImg`、`DetailProps`、`DetailHeader`、`StatRow`、`StatTableRow`、`RelatedRow`、`costText`、`formatCost` / `formatResourceAmount`（资源图标优先 `resources` 段，回退 `core/images/resources/<idx>.webp`）、`WikiLink`。

### 4.1 生物详情页（CreatureDetail）

**布局**：内容容器 `max-w-[960px]`（超过不再拉伸，右侧留白而不是把横线拉断），**宽态与弹窗态是两套不同的列结构**：

- **宽态（wiki 右栏）= 两列**，`minmax(280px,1fr)`：左列 = 生物图（180×180）+ 士气/运气 + 招募花费 + 升级自/升级到；右列 = 统计表 + 招募建筑。分配原则是**经济/成长跟图走**、**表格式的归右列**，两列估高接近，不会一边空一边满。能力区是两列之下的**全宽网格**（`auto-fit minmax(260px,1fr)`，列数随宽度 1→3），描述在末段。
- **弹窗态（右键弹窗，880px）= 三列**，`minmax(240px,1fr)`：左列 = 图（130×130）+ 招募花费 + 升级链；中列 = 统计表 + 招募建筑（compact 不渲染统计表脚注）；右列 = **能力列**。三列均分 ≈274px，窗口更窄时自动退化成 2 列。
- **窄态（<572px）**：退化为单列，顺序回到 VCMI 的原始顺序。

**为什么弹窗要把能力立成第三列**：能力数是弹窗最大的高度变数（原版实测 0–15 项，中位数 2）。铺在下面时高度随项数线性涨到 1200px+；立为第三列并**限高 5 行（`max-h-[385px]`）**后，弹窗总高被压成常量：

```
40(标题) + max(左264, 中238–326, 能力列 18+385=403) + 34 = 477px
  N≤3 → 338px   N=4 → 400px   N≥5 → 477px（能力列内滚动）
```

- **83.5% 的兵种（≤5 项）不出现任何滚动条**；只有 16.5% 在能力列内部滚，滚到底时底部渐隐提示自动消失。
- 滚动语义与官方一致：VCMI `CCreatureWindow.cpp:336-355` 的 BonusesSection 就是"固定高度 + `CListBox` 内滚"（只露 3 行）。
- **N=0 时第三列留空**（不渲染、不收窄弹窗）—— 宽度跳动比一点留白更难接受。
- 弹窗宽 880 是刻意比 VCMI `CStackWindow`（430px）宽：这个窗口"按住右键才存在"，用户无法同时拖滚动条，只能用宽度换高度。

**统计表**：行序与行名对齐 VCMI `WikiCreatureContent.cpp:130-172`
（等级 → 攻击力 → 防御力 → 杀伤力 → 速度 → 生命值 → 生长率 → 弹药量 → 法力 → 囤兵增长 → 双格单位 → 地图部队 → AI价值 → 战斗价值）。
标签列固定 **144px**、值列左对齐 —— 两处刻意偏离 VCMI：① VCMI 的值列起点是 `tableW/2`（表格中线），只适用于它约 470px 的 viewport，我们右栏可到 900+，两端对齐会把标签与值拉到数百 px 外；② 144px 的依据是 5 语言字段名实测最长 110px（pl `Podwójna szerokość`）+ padding 与余量。

**行可见性**（h3-core 150 条实测）：前 7 行常驻；弹药量 23%、囤兵增长 15%、双格单位 37%、地图部队 96.6%。

**招募花费**：主显 **fullRecruitCost**（含升级链总投入，VCMI `getFullRecruitCost()`），tooltip 补"本阶花费"差额。原版 cost 组合最多 2 种资源（128 只纯金币 + 18 只 金币+一种稀有资源），但 **mod 兵种可到 9 种**（实测阿古菇奥特朗 原初：60000 金 + 8 种稀有资源）—— 因此渲染成"**每个资源一枚 chip**"的 `flex-wrap` 行，而不是 `formatCost` 的一行式排版（后者靠父级 flex 收缩，资源多时不换行、直接溢出容器）。

**士气/运气**：**非 0 才渲染**。原版 146 只里只有 9 只非 0（天使/大天使/牛头怪/牛头王 +1、骨龙/鬼龙 −1、恶魔/大恶魔 运气 −1、矮妖精 +1），恒显只会得到两个 0。VCMI 新版 wiki 页干脆不显示这两项（只有战斗弹窗 `CStackWindow` 显示）。

**升级链 / 招募建筑**：整行框（`RelatedRow`，36px 图标 + 名称 + 整行可点），对齐 VCMI `WikiCreatureContent.cpp:215-275 / 411-450`。名称与图标从**当前 mod 的缓存**按 id 取（同 mod 是常态，原版生物在 `core.<faction>` 内闭环）；跨 mod 升级取不到名字时回退 id，跳转由 `WikiView.navigateTo` 的"同分类按 id 兜底"接住（creatures/artifacts/spells/skills/terrains 均已跨 mod 按 id 去重，同 id 只有一条）。招募建筑后端只给「语言 → 建筑名数组」（`town.rs:165-178`），没有 id/图标，故渲染为不可点的名称条，不做伪链接。

**升级链去重**：同一个基础生物可能被多处注册（mod 覆盖 core 时，缓存里 core 的原版条目与 mod 自己的条目都指向它），二者 id 不同但**显示名与图标完全相同**，不去重就会在「升级自」里出现两行一模一样的条目（实测阿尔法龙）。口径 = 先按 id，再按「显示名 + 图标」；仅同名而图不同仍视为不同生物。

**跳转后中栏同步**：`navigateTo` 会置 `pendingListScrollRef`，于是点升级链 / 模组条目后，中栏会滚动到目标行并高亮（此前只有"返回"才滚，链接跳转后右栏换了、中栏停在原地）。

**能力区**：每格 = 52px 图标框（描边色按来源区分，对齐 VCMI `CCreatureWindow.cpp:277-290` 的 bonusColors）+ 名称（1 行）+ 描述（`line-clamp-3`，`title` 给全文，右键气泡保留）。原版实测中位数 2 个能力、≤3 个占 73%、15% 为 0（此时整块含标题隐藏）。

**描述段**放末段：原版描述 = h3-core 的 `abilityText`（`h3core.rs:137-140`），113/150 非空、平均 11 字、最长 25 字，而英文平均 27 字符、最长 66 字符 —— **跨语言高度差约 2.5 倍**。放前面会让统计表位置随界面语言上下跳，放末段则长度不推动任何其它块。

> **已知数据限制**：原版（core）建筑的**花费没有任何 JSON 数据源**（`town.buildings` 为空、`buildingsLibrary.json` 只有 id/bonuses，VCMI 把 H3 建筑花费硬编码在引擎里），因此原版建筑 cost 必然为空。这是数据限制，不是缺陷。

## 5. 悬停浮层（WikiPopup）

`WikiPopup.tsx` 提供 Provider，两种形态：

- **文本气泡** `CRClickPopup`：居中于光标，`fitToScreen(10)`，z-200；
- **完整窗** `CStackWindow` / `CHeroOverview`：屏幕居中，creatures **880px（三列）**、heroes 620px。creatures 刻意比 VCMI 的 430px 宽 —— 弹窗内容是三列（图+经济｜统计+招募建筑｜能力），能力列限高 5 行、超出列内滚动，弹窗总高因此压成常量 ≤477px。

触发（`WikiPopupContext.ts`）对齐 VCMI 的 EventDispatcher：**右键按下即显示、右键松开即关**；触屏为 500ms 长按（移动 >12px 取消）。`usePopupTriggers()` 返回可复用 props 工厂，供行级元素挂载。

**中栏条目行的气泡文本**（`WikiView` 的 `EntryRow`）：VCMI `WikiListItem::showPopupWindow` 显示的是副标题，我们**换成描述类字段**——副标题（T6 / 狂化人 / 中级宝物 / 北境）是分类信息，作为气泡没有信息量，用户右键一行想知道的是"这东西是什么"：

| 类目 | 气泡字段 | 回退 |
|---|---|---|
| `factions` / `creatures` / `artifacts` / `buildings` | `description` | 条目名 |
| `heroes` | `biography` | 条目名 |
| `spells` | `levelDescriptions.basic`（原版是纯字符串，需剥 `{标题}` 前缀，与 `SpellDetail` 同口径） | 条目名 |
| `skills` | `levels.basic` | 条目名 |
| `terrains` / `mods` | 不挂触发器（无描述类字段） | — |

行的副标题本身不变（仍是 T6 / 狂化人 / 中级宝物 / 北境）——它在列表里是有用的分类信息，只是不该当气泡文本。

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
