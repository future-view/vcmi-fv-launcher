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

## 4. 缓存键与失效

键格式：`v{WIKI_CACHE_VERSION}:{sourceHash}:{h3DataKey}:{h3CoreHash}:{schemaHash}`，**字符串相等**判定命中。

当前值：`WIKI_CACHE_VERSION = 1`、`ASSETS_VERSION = 4`（另 `HD_SCHEME_VERSION = 6`）。

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
