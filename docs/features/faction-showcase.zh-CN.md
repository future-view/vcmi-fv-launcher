# 阵营巡礼（Faction Showcase）

> 状态：已实现 ｜ 最后核对：2026-09-30 ｜ 代码：`src/features/realms/showcase/ShowcaseOverlay.tsx`、`src/lib/realm/showcase.ts`、`src-tauri/src/modcache/showcase.rs`

## 1. 目标与范围

**做**：全屏轮播领域内各阵营的幕布、生物巢穴、英雄与主题曲，作为领域的"登场后演出"或独立展示页。

**不做**：编辑巡礼内容（编辑在领域编辑器的 Showcase 标签页，见 [realms-editor](./realms-editor.zh-CN.md)）。

## 2. 播放规格

- 画布 **1280×720 定尺等比缩放**；幕布 frame 按实测行高推算（`FRAME_RATIO = 800/374`）。
- 调度节奏：单阵营 `FACTION_DURATION_MS = 26000`、焦点步进 `3000ms`、英雄轮换 `6000ms`。
- 两路音频**交叉淡入**：`MUSIC_FADE_MS = 800`、提前淡出 `MUSIC_PRE_FADE_MS = 2000`。
- 可接管领域主界面的 BGM 元素，实现主题曲接续播放（复用 `bgmMuted` 开关）。

## 3. 牌组编排（纯逻辑 `lib/realm/showcase.ts`）

| 函数 | 行为 |
|---|---|
| `resolveTownIds` | 取索引里 `type === "Town"` 且非 `vcmi` 的模组；`core.*` 由 `includeCore` 单独旁路启用过滤；`enabledIds` 三态语义 |
| `buildShowcaseDeck` | manual 牌组优先，否则 auto：**先洗牌再截断**，种子 `autoSeed` / `fallbackSeed(realm.id)`，`DEFAULT_MAX_SLIDES = 12` |
| `resolveDwellingId` | 巢穴建筑按**名称**匹配，真值来源 `src-tauri/data/h3-core/*.buildings.json` |
| `estimateStayMs` | 估算单站停留时长 |
| `defaultHeroIds` / `heroClassKey` | 默认英雄与英雄职业键 |

## 4. 内容来源

全部来自 `cache.json` 的 `factions` / `creatures.dwellings` / `heroes` / `buildings`。**缺缓存返回 `missing_slide("nocache")`**，因此进入前必须由 `ensureRealmAssets` 保证本领域素材就绪（未就绪则"先建再进"，见 [modcache](./modcache.zh-CN.md) §8）。

音频（`modcache/showcase.rs`）：按 `musicTheme` 解析，先查磁盘，再查 `content.zip`（`AUDIO_EXTS` = mp3/ogg/wav/flac，单条 ≤25MB，超限跳过）。前端拿到 `music` 直接播；拿到 `musicZip` 则调 `ensure_showcase_audio` 抽取后播；两者皆无则该站安静——**不串主界面曲子**，除非配置 `musicUseRealmBgm`。

## 5. 边界与容错

- 单条音频超限只是跳过该站音乐，不影响画面。
- 缓存未就绪不静默降级为空白，而是先构建（领域 `assets` 页带进度与取消）。

## 6. 验证

```bash
npx tauri dev   # 真机：领域首页 → 巡礼入口；观察洗牌、英雄轮换、音乐交叉淡入
```

## 7. 开放问题

- 巡礼内容目前只覆盖城镇 / 生物巢穴 / 英雄三类，未纳入宝物与法术。
