# 百科缓存管线（modcache）

> 状态：已实现 ｜ 最后核对：2026-10-12 ｜ 代码：`src-tauri/src/modcache/`、`commands/cache.rs`、`src/lib/modcache/`
> 展示层见 [wiki](./wiki.zh-CN.md)。本文讲**缓存怎么建、产物长什么样**。

## 1. 目标与范围

**做**：把散落在 mod.json、`Content/config/**`、LOD 归档、`content.zip` 里的游戏数据，抽成**统一、可增量、可离线**的缓存，供百科页、城镇幕布、阵营巡礼、悬停浮层消费。

**不做**：在运行时解析原始资源（一律读缓存）；修改任何游戏文件。

## 2. 两阶段构建

| phase | 含义 |
|---|---|
| 0 | 仅数据（解析配置出文本，秒级） |
| 1 | 仅素材（解码 DEF / 编码 WebP / 合成预览，耗时） |
| 2 | 两者 |

前端编排三批（`lib/modcache/status.ts`）：

1. **phase 0 全量**（turbo 档 2）—— 先让文本可用；
2. **phase 1 标清素材** —— Town 优先、其余后置，保证幕布图先出；
3. **phase 3 高清**（eco 档 0，可选）—— 生成 `@Nx` 变体。

后端按 `mods.chunks(8)` 分块，**每块重建 rayon 局部池并重估空闲档位**：空闲 ≥5min 取 `cpus/2`、≥30s 取 `cpus/4`、否则 `cpus/8`。

**进度分两段**（`batch::BuildProgress`，事件 payload 的 `prep` 字段区分）：

| 段 | 内容 | 为什么必须单独可见 |
|---|---|---|
| `prep`（扫描模组目录） | 排序前对每个 mod 目录**递归枚举估成本**（LPT 调度用），并行度=本批线程数 | 冷缓存下是**分钟级**（机械盘实测 14,144 目录冷遍历 ≈ 652 s / 热 9.6 s），且发生在**第一个 mod 开工之前**。原实现既不并行也不上报 → 用户看到"0% 静止 63 s"（2026-10-16 真机实测，是"卡在 0%"投诉的来源） |
| `build`（读取游戏数据 / 提取游戏素材 / 生成高清素材） | 逐 mod 构建，**每 mod 完成都上报**（100ms 时间节流） | 原先是"每 8 个 / 最后一个"：首块恰是 LPT 选出的最大 8 个 mod（实测各 11~12 s），UI 在这段里停在 0% |

`prep` 不属于任何页面的 `deps`（不参与撤除判定），只要后端报过就始终显示；多批调用（阶段二/三）只对**未命中成本缓存**的 mod 上报，故不会出现"572/572 → 20/20"的倒退。

### 启动预检期的就绪判定（首次运行不再"先空页、后跳蒙层"）

启动编排第 4 步 `ensureWikiCacheBuilt()` 先做**预检**（`readLauncherConfig` → `scanRealMods` → `probeWikiCacheState`，与真实构建同源判定），**刻意不置 `busy`** —— 既定设计是"全部命中时页面用现有 `index.json` 立即渲染，不被蒙层整页阻塞"。

但**无索引可渲染**时（首次运行 / 缓存被清空）该设计会暴露体验问题：预检可达数秒~数十秒（首次 `scan_mods` 全量扫 572 个 mod 目录 + 逐 mod 算 source hash），期间 `busy=false` 被 `useWikiStagesDone` 判为"已完成" → 百科页直接渲染，而 `index.json` 不存在 → **分类全 0 + "未找到匹配条目"**（用户读作"没有数据"），直到预检结束置 `busy` 才跳进"正在生成模组缓存"蒙层 —— 即"卡在空页，过一会才显示正在生成"。

**判定规则**（`status.ts` + `shared/WikiBuildOverlay.tsx`，宿主零改动）：

| 场景 | `preparing` | `indexReady` | 页面行为 |
|---|---|---|---|
| 老用户增量重建（有索引） | true | true | **放行** —— 用现有索引立即渲染（设计 C 不变） |
| 首次运行 / 缓存被清空 | true | false | **等待态** —— 预检期即显示蒙层（spinner + 标题 + 提示，无 0% 三条），预检结束置 `busy` 无缝接力真实进度条 |
| 预检完成（无需构建 / 开始构建 / 异常） | false | — | 恢复原语义（`busy` 按阶段完成度判定） |

`indexReady` 由前端在预检开始时**直读一次索引**得出（~ms 级，与后端 `wiki_cache_state` 出参同口径），不拖长预检；预检标志在 `finally` 无条件清除（否则残留 `preparing + indexReady=false` 会令蒙层永驻）。`StageGate`（领域编辑器区块）同步按 `preparing` 隐藏 0% 进度行。

## 3. 产物

```
mods/
├── index.json                 已缓存清单（只合并不删除）
├── .hd-scheme                 HD 方案指纹
└── <modId>/
    ├── cache.json             投影数据（modMeta / resources / objects / 各类目条目）
    ├── raw.json               全量原始配置聚合（幕布合成等复用）
    ├── audit.json             素材缺口审计（无缺口时删除）
    ├── hd-map.json            缓存图 → HD 资源名映射
    ├── .hd-src                HD 来源指纹
    └── images/<sub>/*.webp    creature / hero / herospec / building / artifact /
                               spell / skill / terrain / town / townscreen /
                               townborder / stats / pskil42 / spellbon / resources ...
```

**原版数据以伪模组入缓存**：`core`（承载全局 / 无归属实体）与 `core.<faction>`（9 个原版城镇，形态等同第三方 town mod）。前端 `buildTargets.ts` 构造它们，`modPath` 指向 `<gameDir>/Mods/vcmi`。

**产物归属 = 素材来源方**（`assets::owner_of`，`Origin::{Game,SelfMod,Mod}`）：

- 游戏根资源（`Data/*.lod`、游戏级 DEF/位图）→ 一律归 `core`：9 个原版城镇的
  `building` / `creature` / `hero` / `herospec` 图标**只落一份**在 `core/images/`，
  `core.<faction>/images/` 只留该族专有的 `town` / `townscreen` / `townborder`；
- 跨模组引用 → 归被引用方；
- 第三方 mod 自有素材 → 归自己（保持自包含）。
- `core` / `core.<faction>` 是本 VCMI 核心模组的**同一份磁盘目录**，故其 `SelfMod` 命中
  同样归 `core` —— 否则同一张图会在父 core + 9 个城镇目录各写一份（2026-10-20 修复前
  `spell/battle_actiongenie_png.webp` 实测 10 份）。

**判定标准不是"是否与城镇有关"，而是"是否跨阵营共用"**（2026-10-20 复核，结论：现有分布正确）：

| 用途 | 归属 | 理由 |
|---|---|---|
| `building` / `creature` / `hero` / `herospec` / `artifact` / `skill` / `terrain` / `pskil42` / `luck` / `morale` / `resources` / `spellbon` | `core/images/`（一份） | 游戏根素材，9 族页面**共用同一批帧**；按族复制 = 纯浪费（实测 334 个建筑图标、236 个生物小图、130 个头像都只有一份原始数据） |
| `town`（幕布底图 / 兵种背景）、`townscreen`（合成预览）、`townborder`（结构描边高亮） | `core.<faction>/images/` | **每族独有的整幅画面**，不存在跨族共享；与第三方 town mod"自带幕布/描边"的布局一致（`hota/images/town/...`） |

⇒ 因此 `core.<faction>/images/` 只剩这三类（实测 `core.castle` 37 张描边 + 2 张幕布），
不是遗留重复。若把它们也搬进 `core/images/`，`core.<faction>` 将不剩任何图片，
反而破坏"城镇模组自包含"的对称性（第三方城镇仍然自带幕布），故**不做调整**。

**取名与职业口径（2026-10-24，缓存版本 4）**：

- **阵营名 = faction 配置的 `name`（VCMI `CFaction::getName()`）+ `mod.json` 模组名的双源合并**：`chinese` 取「`mod.json` 中文块 → 配置名」，`english` 取「`mod.json` 英文名 → 配置的英文键」。两边各填对方的空槽，因为两批真实发行版各缺一半：`1.83` 版 13 座城镇只在 faction 配置里写了中文（港口 / 工厂 / 棱堡 / 沙堡），英文只在 `mod.json`（Cove town / Factory Town / …）；而 `132mod v6.0` 版 19/123 个阵营的配置名本身就是英文（`Abyss` / `Ark` / `Cemetary`），中文反而靠翻译文件或 `mod.json` 中文块。**只取任一边都会坏一半语言**：模组名压制配置名 → 中文界面变英文；配置名压制模组名 → 英文界面变中文。配置里的**裸字符串**是语言未知的原文（VCMI 里任何语言界面都原样显示），因此只补**空槽**，不顶掉已有中文；多语言对象按语言键原样透传。
- **形如 id 的模组名不当英文名**：`132mod v6.0` 里 83/123 个阵营的 `mod.json.name` 就是阵营 id（`csknight` / `blazers` / `oldpavilion`），拿它填 `english` 槽只会把 id 当名字显示（2.x 缓存的表现：`骑士` 的英文列是 `csknight`）。判据 = 归一化后与 faction id 相同**且**原串无大写字母（保护 `Bulwark` 这类真名）；不成立时才轮到配置原文。
- **英雄职业表两源合并**：归一 `rawConfigs.heroClasses`（`config/heroClasses/`、`config/<mod>/classes/`、从 `heroes/` 目录按形状嗅探出的职业文件）+ **`mod.json` 的 `heroClasses` 清单**。清单里的文件可以放在 `Content/` 的任意目录，只要不在 `Content/config/` 树里就收不进 `rawConfigs`；只认前者会让该 mod 的**全部**英雄被 `class_known` 判负而整批丢弃（实测 `New Pavilion`：职业在 `Content/classesPavilion/`、英雄在 `Content/heroesPavilion/`，城镇页英雄区空白）。

## 4. 缓存键与失效

键格式：`v{WIKI_CACHE_VERSION}:{sourceHash}:{h3DataKey}:{h3CoreHash}:{schemaHash}`，**字符串相等**判定命中。

当前值：`WIKI_CACHE_VERSION = 4`、`ASSETS_VERSION = 3`（另 `HD_SCHEME_VERSION = 6`）。`WIKI_CACHE_VERSION = 4`（2026-10-24）：阵营名改为「faction 配置名 + mod.json 模组名」双源合并（3 版只取配置名，英文界面退回中文；id 形态的模组名不采用），连同英雄职业表补上 `mod.json` 的 `heroClasses` 清单 —— 都会改变 `factions[].name` / `heroes[]` 的内容，旧缓存必须整体重建（见 §3 取名与职业口径）。同版还把 `special: true` 的生物改为「只排除非战争机器」，原版弩车 / 急救帐篷 / 补给车 / 投石车（均在游戏根 `config/creatures/special.json`）因此重新进入 `creatures[]`（见 [wiki](./wiki.zh-CN.md) §3 生物收录口径）。`ASSETS_VERSION = 3`（2026-10-20）：HD 登记表键口径由 `sanitize_id`（`~`→`_`）修正为 `sanitize_asset_name`（保留 `~` 与磁盘 stem 一致），旧登记全线失效必须重建，见 [hd-assets](./hd-assets.zh-CN.md) §6。

失效条件：源哈希变化、游戏数据指纹变化、h3-core 文案表变化、schema 变化；素材未就绪或 `assetsVersion < ASSETS_VERSION` 或 HD 来源指纹变化时，清 `images/` 与 `hd-map.json` 重跑。

**增量粒度是 mod 级**：每个 mod 构建完成才落盘 `cache.json` / `raw.json`。进程被关闭时已完成的保留、未完成的丢失；半截文件下次解析失败会被视为无效缓存重建，不会读到坏数据。

## 5. 模块职责（后端）

| 模块 | 职责 |
|---|---|
| `build/` | 单 mod 两阶段构建主体；`build/data/*` 按实体类型采集 |
| `collect.rs` | 配置 JSON 深度合并 + 原始快照 |
| `i18n.rs` | 生成 name / description 多语言对象、注入 h3-core 文案 |
| `assets.rs` | 图标 / DEF 帧 / 特长图标落盘 |
| `global.rs` | 游戏级共享素材归位 `core/images/`（stats、pskil42、spellbon、resources） |
| `preview.rs` | 城镇幕布合成 `images/townscreen/` |
| `index.rs` | `index.json` 重建与单 mod 读取 |
| `batch.rs` | 批量并发 / 进度 / 取消 / 索引合并 |
| `audit.rs` | 提取协议审计（`audit.json`） |
| `schema.rs` | 内置 schema 与协议表 |
| `hd.rs` / `hd_map.rs` | `@Nx` 高清后处理与资源名映射 |
| `showcase.rs` | 巡礼 slide 与阵营音乐解析 |
| `version.rs` | 版本常量 |

## 6. 素材抽取链路

```
LOD (infra/lod.rs)  →  DEF 解码 (infra/def.rs)  →  PCX/BMP 解码 + WebP 编码 (infra/image.rs)
```

三来源解析（`vcmi/resource.rs`）：`Content/**` 目录 → `content.zip`（ZipIndex + 线程局部已开 archive 缓存）→ 游戏 `Data/*.lod` 回退。

**城镇幕布口径**：唯一来源是 `build_towns` 产物（`images/town/<fid>.webp`）。历史上 `preview` 与 `background` 各合成一份，实测 84 个城镇中 70 张不同（两条路径取数口径分叉），已统一为以 `build_towns` 为准；只有 `background` 为空时才兜底调 `preview` 合成到 `townscreen/`。

## 7. HD 后处理

素材阶段由 `hd_map.rs` 登记「缓存图 → HD 资源名」；阶段三 `hd.rs` 幂等生成 `<name>@<rate>x.webp`（`MAX_HD_RATE = 6`，倍率按显示器用途分级）。详见 [hd-assets](./hd-assets.zh-CN.md)。

## 8. 领域加急

`lib/modcache/realmAssets.ts`：

1. `realmAssetTargets` 算出 `core` ∪ 启用集合 ∪ bundled / borrowed；
2. `checkRealmAssets` 抽样判定是否就绪；
3. 未就绪 → `pauseWikiBuild()` → `buildRealmCacheUrgent`（**刻意不带 hd**，只出标清）→ 恢复全局构建并二次校验。

UI 对应领域 `assets` 页（进度页 + 返回按钮走 `cancel_realm_cache_build`），就绪即自动放行。

## 9. 命令清单

百科缓存共 16 个命令，清单见 [ipc](../architecture/ipc.zh-CN.md) §2「百科缓存」段（冻结契约同篇 §4）。

## 10. 边界与容错

- 进度事件是加速手段，前端一律有 400ms 轮询兜底，事件丢失不会卡死 —— **兜底也必须推进可见的阶段条**（`prep` 与 `data`，蒙层渲染的是 `status.stages` 而非 `status.progress`；曾出现"只写没人渲染的 progress → 事件真丢时仍停在 0%"）。
- `index.json` **只合并不删除**（领域切换可见性的取舍），已卸载模组靠读侧 `visibleModKeySet()` 过滤；不要加自动 GC。
- 带密码的 `Content.zip` 提取失败回退色块，绝不崩溃。

## 11. 验证

```bash
npm run verify:wiki-cache     # 对着刚产出的目录校验（退出码 0 = 通过）
npm run rebuild:wiki-cache    # 全量重建
npx tauri dev                 # 真机：观察三阶段进度与领域加急
```

## 12. 开放问题

- 缓存键是字符串全等判定，任一组成哈希变化即全量重跑该 mod；更细粒度的增量未实现。
