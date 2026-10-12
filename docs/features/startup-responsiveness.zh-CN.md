# 启动期响应性（首次启动模组页卡死）

> 状态：**已实施**（2026-10-08；方案第 2 版为「蒙层对齐 wiki 页」路线，第 3 版补 §8 实施记录）｜ **体裁：方案记录**（问题诊断 + 方案 + 实施记录，非功能规格，不套用"验证 / 开放问题"骨架）｜ 代码：`src/App.tsx`、`src/features/mods/**`、`src/lib/{tauri,modcache/status}.ts`、`src/shared/{LoadingOverlay,useInViewport,WikiBuildOverlay}.tsx`、`src-tauri/src/commands/{mods,cache,mod_settings,settings,paths,app}.rs`

## 1. 问题现象

首次打开启动器（默认页签 = 模组，`App.tsx:88`）：

- 「分析模组列表」期间**整个窗口无响应** —— 点击「领域」「⚙ 设置」等都不生效；
- 期间**没有任何进度反馈**：wiki 页未就绪时是「早返回 + 只渲染蒙层 + 进度条」（`WikiView.tsx:401-411`），mods 页同期只有右栏一行小字（`ModBrowser.tsx:1232`）——**两页观感明显不一致**；
- 缓存已建成的**第二次启动基本无感** —— 问题只在「无缓存 + 首次全量」时暴露；
- **切走再切回从头再来**：视图互斥卸载（`App.tsx:655-659`）导致 ModView 重挂载 + 重跑扫描（见 R5）。

## 2. 根因（R1 决定"是否卡死"，R2–R5 决定"多久看到内容"）

### R1（决定性）后端同步命令在主线程执行 → 冻结窗口消息循环

Tauri 的**同步命令**（`#[tauri::command] pub fn`）在**主线程**执行，执行期间 WebView 消息循环停转 —— 点击事件既不派发也不渲染，表现为「整窗卡死」而不是「只有列表卡」。

本仓库已有同类事故并已修复：`src-tauri/src/repository/mods.rs:733-753`（P8，`fetch_mod_index` 由同步改 `async + spawn_blocking`，注释原文：「同步命令在主线程执行…会把消息循环冻结几分钟」）。

而**启动路径上的命令目前仍是同步的**：

| 命令 | 触发者 | 首次启动调用量 | 单次重量 |
|---|---|---|---|
| `scan_mods`（`commands/mods.rs:16`） | ModView 装载 + `ensureWikiCacheBuilt` | **2 次全量** | 遍历全部 mod 目录 + 逐个解析 `mod.json` + factions 目录列举 |
| `read_mod_cache`（`commands/cache.rs:21`） | 每张城镇卡片的 2 个 hook | **数百次** | 读**整份** `cache.json`（首次=文件不存在，全部走失败路径） |
| `get_mods_cache_dir`（`commands/cache.rs:345`） | 同上，每 hook 一次 | 与上行同量级 | 每次都 `create_dir_all(cache_root)` |
| `read_mod_settings` / `read_launcher_config` / `detect_vcmi_paths` / `read_cache_index` | 启动编排、ModView 阶段 1 | 数次 | 小文件，影响小 |

`useTownCacheImage`（`ModCard.tsx:546-616`）在读取失败且 `busy` 时**每 5–10s 重试**（`:600-606`），首次启动会在整个构建期间持续向主线程投递命令。

### R2 前端列表渲染 O(N²) 且不可中断

- `ModView.tsx:270-276`：每 15 个模组 `setRootMods` 一次，每次触发 `ModBrowser` 的 8 个 `useMemo` 重算（含全量 `localeCompare` 排序）+ **已挂载行全部重渲染** → 渲染量 ≈ (N/15) × (N/2) 行；
- 行组件 `ModCard` 每次渲染执行 `richToPlainText`（10 次正则，`lib/html.ts:297-312`）；
- 全仓**无** `React.memo`、无列表虚拟化、无 `startTransition` / `useDeferredValue` / `Suspense` / Web Worker → 渲染一旦开始**不可被打断**，点击只能等渲染队列排空。

### R3 卡片 IPC 风暴

`ModCard.tsx:91-92`：每行挂 `useTownIcon` + `useTownBackground` 两个 hook，每个 hook 各发 `Promise.all([getModsCacheDir(), readModWikiCache(id)])` = **2 次 IPC**。数百模组 → IPC 数量级为**千**。

批量最小字段命令**已经存在**（`read_town_cards`，`commands/cache.rs:46-57`，注释实测「86 城镇 31 MB → 数十 KB」）—— 领域信息页已改用，**mods 页没改**。

### R4 后台构建与首屏争抢（叠加项）

`App.tsx:262` 启动即 fire-and-forget `ensureWikiCacheBuilt()`；data 阶段用 `DATA_BUILD_THROTTLE = 2`（turbo = cpus/2，`status.ts:61`）与 mods 首屏扫描/渲染/卡片 IPC 重叠，CPU 争用放大掉帧。

### R5 切走再切回 = 重新全量扫描（结构性问题）

`App.tsx:655-659` 视图是**条件渲染、互斥卸载**：`view !== "mods"` 时 ModView 被卸载，切回「模组」时**重新挂载** → 重跑 `scan_mods` + 重新分块渲染 + 卡片 IPC 重来。

即：即使 R1 修好、用户能随时切走，**切回来仍要再等一次**（首次启动尤其明显）。修复方案见 §3 P4（模块级扫描缓存）。

### 时间轴（首次启动，示意）

```
t=0    boot: detect_vcmi_paths → restoreRealm → selfCheck       [checking 整屏占位]
t≈1s   主界面挂载 → ModView 装载
t≈1s   scan_mods #1（主线程，同步）                              ← 窗口已卡
t≈1s   ensureWikiCacheBuilt 启动 → scan_mods #2（主线程）→ data 批 turbo 构建开始
t≈1~2s 分块渲染列表：N/15 次 setState × 平均 N/2 行重渲染        ← 主线程持续满载
t≈2s   数百张卡片 ×4 次 IPC（read_mod_cache 全部失败）+ 5~10s 重试轮询
        ↓ 整个构建期（分钟级）mods 页无任何提示，点击标签无响应
```

## 3. 方案（分层，P0 是唯一必选项）

**总原则**

- R1 修好后**任何时刻点标签页都立即响应**（视图切换是纯前端 state），"卡死"消失；其余各项只影响"多久看到内容"。
- **不重开 mods 页蒙层**（2026-09-22 已撤除，理由见 `ModBrowser.tsx:563-571`）—— 阻塞蒙层与本次目标相反。
- 进度反馈遵守既有布局，不新增容器。

### P0 消除主线程阻塞（后端 async 化，必做）

把启动关键路径上的同步命令改为 `async fn + spawn_blocking`（与 `fetch_mod_index` 同款）：

```rust
#[tauri::command]
pub async fn scan_mods(state: State<'_, AppPaths>) -> Result<Value, String> {
    // ⚠️ MutexGuard 非 Send：必须先取路径快照，不得跨 .await 持锁
    let (game_mods_dir, user_mods_dir) = {
        let p = state.0.lock().expect("paths lock poisoned");
        (p.game_mods_dir(), p.user_mods_dir())
    };
    tauri::async_runtime::spawn_blocking(move || scan_impl(game_mods_dir, user_mods_dir))
        .await
        .map_err(|e| be_err("...", &format!("{}", e)))?
}
```

- **命令名 / 参数 / 出参不变** → 前端零改动、`check:invoke-parity` 基线（135 命令）不动、不需要新命令。
- 建议范围（第一批，只读）：`scan_mods`、`read_mod_cache`、`read_mod_metas`、`read_town_cards`、`read_cache_index`、`get_mods_cache_dir`、`list_installed_mods`、`read_launcher_config`、`read_mod_settings`、`detect_vcmi_paths`。
- 第二批（写路径）：`write_mod_settings` 等；需评估 async 化后的**并发语义变化**（同步命令在主线程串行排队，async 会并发），必要时加写锁。
- **无替代方案**：把前端重活挪进 Web Worker 无效 —— 阻塞点在 IPC 命令执行侧，不在 JS 计算侧。

### P1 前端渲染可中断（低成本、零新依赖）

1. `startTransition` 包裹列表数据写入（`setRootMods` / `setSelected`）→ 渲染可中断，点击优先响应；
2. `React.memo` 行组件（`ModCard` / `ModListItem`）+ `useCallback` 稳定回调、`Set` 引用稳定；
3. CSS `content-visibility: auto` + `contain-intrinsic-size` 给列表行 → 视口外跳过布局与绘制（"廉价虚拟化"，零依赖）；
4. 搜索框 `useDeferredValue`。

> 虚拟化库（react-window 等）列为**备选**：效果更好但要新增依赖，建议先不做。

### P2 卡片图片 IPC 收敛

1. `getModsCacheDir()` 加模块级缓存（一次 IPC 复用），顺带消除每卡片的 `create_dir_all`；
2. 批量取卡片图路径：一次 IPC 拿全部城镇卡片所需字段（复用/扩展 `read_town_cards`，需补 `creatureBackground` 字段 → **出参兼容扩展**）；
3. 重试风暴收敛：失败且 `busy` 时改指数退避，或只在 `assets` 阶段完成事件触发时重试（`useWikiStageDone("assets")` 已存在），不再固定 5s 轮询；
4. （可选）视口懒加载：IntersectionObserver，卡片进入视口才请求。

### P3 后台构建错峰（可选，与既有决策有冲突）

- `ensureWikiCacheBuilt` 推迟到首屏列表渲染完成后（如 +500ms 或 `requestIdleCallback`）；
- 或 data 阶段 throttle 由 turbo(2) 降为 normal(1)。

> ⚠️ 与既有决策冲突：`status.ts:54-61` 明确「data 是关键路径，不能沿用 ECO」。需拍板（Q7）。

### P4 扫描结果复用（模块级缓存，建议升级为必做）

`lib/tauri.ts` 加模块级 promise 记忆（如 `scanRealModsOnce()`），一次进程内只扫一遍：

- 解决 §2 **R5**（切走再切回重扫）—— 直接影响"切页"体验，故建议由"可选"升为必做；
- 顺带消除启动期 `ensureWikiCacheBuilt` 的**第二次全量扫描**（`status.ts:117`）；
- 需提供显式失效入口 `invalidateModsScan()`（安装 / 删除 / 更新模组后调用），调用点待梳理（`ModManagerPanel` 安装流程、导入流程等）。

### U1 蒙层（与 wiki 页对齐；本次修订新增，替换原"骨架行 + 后台 pill"方案）

**形态 = 早返回 + 只渲染蒙层**，完整复刻 wiki 页的既定范式（`WikiView.tsx:401-411`）：

```tsx
// ModView：列表未就绪时只渲染蒙层 —— PresetBar / ModBrowser 一律不渲染
if (boot !== "ready") {
  return (
    <div className="relative flex h-full p-3">
      <ModsBootOverlay />      {/* 视觉壳与 WikiBuildOverlay 一致 */}
    </div>
  );
}
```

**为什么不是"在左右分栏上盖一层 absolute 蒙层"** —— wiki 注释里已实测（`WikiView.tsx:401-404`）：被遮住的内容照样渲染、照样跑 effect、照样挂载卡片（`useTownCacheImage` 的 IPC 与 5s 重试不停），纯浪费。早返回一并把 **R3 的卡片 IPC 在加载期归零**，且**左右分栏零改动**（正是"降低设计复杂度"的诉求）。

蒙层内容随**等待边界**（Q2 拍板）分两种形态：

**形态 ① 只等列表就绪（推荐）** —— 标题「正在分析模组列表…」（新增 i18n key）+ 不确定进度条（后端一次性返回，无进度可报）+ 提示「列表与依赖关系就绪后即可开始选择」；无出口（等待以秒计）。

**形态 ②/③ 等到 data（或 data+assets）段** —— 标题复用 `t.wiki.generating`、进度复用 `StageRows`（真实三段进度条）、提示复用 `stageReadyHint`；**必须**提供「先进入列表」出口（`onExit`，点后本会话不再显示蒙层），否则用户被锁几分钟。

> ⚠️ 事实前提：mods 列表字段（名称 / 依赖 / 冲突 / 子模组层级）**全部来自 `scan_mods`，与 wiki 缓存无关**（2026-09-22 已核实，见 `ModBrowser.tsx:563-571`）—— 等 data 对 mods 页**零增益**；等 assets 也只影响城镇卡片图（缺图有占位，本就设计为随后自动补齐）。**"与 wiki 一致"建议取"视觉一致"，不取"等待语义一致"。**

#### 原型 ①：只等列表就绪

```
┌──────────────────────────────────────────────────────────────┐
│ VCMI-FV    领域   ▣模组   百科              🌐  ⚙  ─  □  ✕   │
├──────────────────────────────────────────────────────────────┤
│                                                              │
│                    正在分析模组列表…                          │
│        ████████████░░░░░░░░░░░░░░░░░░░░  ← 不确定进度条      │
│              列表与依赖关系就绪后即可开始选择                  │
│                                                              │
├──────────────────────────────────────────────────────────────┤
│ [导入游戏文件][游戏数据][帮助]      [快速启动][编辑器][启动游戏]│
└──────────────────────────────────────────────────────────────┘
顶栏 / 底栏保持可见可点 → 任何时候都能切走（与 wiki 页行为一致）
```

#### 原型 ②/③：等到 data（或 data+assets）段

```
│                    正在生成模组缓存…                          │
│  读取游戏数据    ██████████████░░░░░░  412/650（63%）          │
│  提取游戏图     ░░░░░░░░░░░░░░░░░░░░  0/650（0%）             │
│        列表与依赖已就绪，可以等待图片或先开始挑选              │
│                    [ 先进入列表 → ]                           │
```

**组件形态**（Q3 拍板）：

- ① 抽 `shared/LoadingOverlay`（纯展示壳：标题 + 进度区 + 提示 + 可选出口），`WikiBuildOverlay` 改为其特例 → 一套视觉，避免将来分叉（推荐）；
- ② ModView 内联复刻样式（改动最小，但两套蒙层各自演化）。

**防闪烁**（Q5）：scan 很快时蒙层会一闪而过 —— 建议**延迟 200ms 显示**（`boot !== "ready"` 持续 ≥200ms 才渲染蒙层）。

## 4. 预期效果

| 落地项 | 效果 |
|---|---|
| P0 | **点击标签页永远有响应**（即使列表仍在加载）——核心诉求达成 |
| U1（形态 ①） | mods 页与 wiki 页**观感一致**（同一套蒙层 + 进度 + 提示）；加载期不再"空白 + 无反馈" |
| U1（形态 ②/③） | 蒙层自带真实构建进度，但首次启动多等数分钟、且对列表无实际增益（见 Q2 取舍） |
| P4 | 切走再切回**不再重扫**；启动期只扫一次 |
| P0+P1+P2 | 首屏就绪时间明显提前、滚动/输入不掉帧、IPC 数量从千级降到十级 |

## 5. 拍板结论（2026-10-08）

九个问题的完整选项清单从略（决策过程留档于 git 历史），结论：

| # | 问题 | 结论 |
|---|---|---|
| Q1 | P0 async 化范围 | ① 10 个只读命令 |
| Q2 ★ | 蒙层等待边界 | ① 只等列表就绪；蒙层用 **loading 图标**，不用假进度条 |
| Q3 ★ | 蒙层组件形态 | ① 抽 `shared/LoadingOverlay` 展示壳，`WikiBuildOverlay` 改为其特例 |
| Q4 ★ | 切页重扫（R5） | ① 扫描结果模块级缓存（升级为必做） |
| Q5 ★ | 蒙层防闪烁 | ① 延迟 200ms 显示 |
| Q6 | P1+P2 是否同批 | ① 一起做 |
| Q7 | 后台构建错峰 | ③ 都不做（与 `status.ts` 的"data 优先"冲突） |
| Q8 | 虚拟化方式 | ① CSS `content-visibility`（后实测放弃，见 §8 偏差） |
| Q9 | `read_town_cards` 扩展 | ③ 保持现状走懒加载 |

> 编号映射：旧 Q2（pill 位置）已随蒙层路线**取消**；旧 Q3/Q4/Q5 → 新 Q7/Q8/Q9；旧 Q6 → 新 Q4（升级为必做）。

## 6. 验证方式

- `npm run check:invoke-parity`（P0 后必跑，确认三方契约不变）；
- `npm run build`（类型）；
- 真机实测（关键）：清空 `.vcmi-fv-launcher/mods` 缓存目录 → `npx tauri dev` 首启 →
  1. 加载期点「领域」「⚙」应**立即**切换（P0）；
  2. 蒙层不闪烁（P4/U1 防闪烁）；
  3. 切到「百科」再切回「模组」，应**不再出现蒙层**（P4 生效后列表瞬间就绪）；
  4. 蒙层期间底栏「启动游戏」保持禁用（既有行为不回退）；
- 日志：`infra::metrics` 埋点对比改造前后 `scan_mods` 耗时与 IPC 次数。

## 7. 风险

- **async 化并发语义**：同步命令原为主线程串行；改 async 后并发执行，同一文件的读写可能竞争（`read_mod_settings` / `write_mod_settings`）→ 写路径放第二批并加锁；
- `spawn_blocking` 与构建期 rayon 争线程池：`scan_mods` 可能与构建排队，但**不再冻结窗口**（这正是目标）；
- 冻结消息循环的另一个隐性来源：`VcmiPaths::mods_cache_dir()` 等静态调用链若有重 IO，同样在命令线程 —— P0 时应一并检查；
- **蒙层会"掩盖"真实冻结**：CSS 动画由合成器线程驱动，主线程被同步命令冻结时蒙层仍在转 —— 用户无法区分"正在加载"与"死机"。故 **U1 必须与 P0 同批落地**，否则问题从"看起来卡了"变成"看起来在转但点不动"；
- 早返回的副作用：加载期 PresetBar（预设切换）一并不可见 —— 与现状（右栏加载提示、左侧可点）相比属于视觉变化，无功能回退；须确认 scan 失败路径仍能走到 `boot === "ready"`（现有 catch 分支已保证），否则蒙层永驻。

## 8. 实施记录（2026-10-08）

拍板：**Q1 ①**（10 个只读命令）／**Q2 ①**（只等列表就绪；蒙层**用 loading 图标**，不用进度条——无真实进度就不假装有）／**Q3 ①**／**Q4 ①**／**Q5 ①**／**Q6 ①**（P1+P2 同批）／Q7 ③（不错峰）／Q8 ①（不引入虚拟化库）／Q9 ③（不扩展 `read_town_cards`）。

### 已落地

| 项 | 落点 |
|---|---|
| P0 | `commands/mods.rs`（`scan_mods` / `list_installed_mods` / `read_def_bytes`）、`commands/cache.rs`（`read_mod_cache` / `read_mod_metas` / `read_town_cards` / `read_cache_index` / `get_mods_cache_dir`）、`commands/settings.rs`（`read_launcher_config`）、`commands/mod_settings.rs`（`read_mod_settings`）、`commands/paths.rs`（`detect_vcmi_paths`）→ 全部 `async + spawn_blocking`（命令名 / 参数 / 出参不变）。**顺带修复**：`commands/app.rs::get_runtime_info` 原先在**同步**命令里直调 `scan_mods`（设置面板「运行信息」一打开就冻结主线程），已改 async 并 await 子命令 |
| 容错 | 新增错误码 `cmd_blocking_task_failed`（5 语言 beErr）；前端 `readLauncherConfig` / `detectVcmiPaths` 失败改为返回 `null`（"未就绪"语义，不再把异常抛进启动编排） |
| U1 | 新增 `shared/LoadingOverlay.tsx`（spinner + 标题 + 可选进度区 / 提示 / 出口）；`WikiBuildOverlay` 收编为其特例（对外 `deps` / `blocking` / `onExit` 契约不变）；`ModView` 未就绪时**早返回只渲染蒙层** + 延迟 200ms 防闪烁；i18n 新增 `mods.analyzing` / `mods.analyzingHint`（5 语言） |
| P4 | `lib/tauri.ts`：`scanRealMods` 进程内记忆 + `invalidateModsScan()` / `rescanRealMods()`；`ModView` 四处"安装 / 更新 / 补装 / 删除后重扫"改走 `rescanRealMods`；领域 `activateRealm` / `deactivateRealm` 成功后自动失效（借用集合会变） |
| P1 | `ModView` 去掉"15 个一组 + 25ms"的分块渐进渲染 → `startTransition` 一次性写入（消除 O(N²) 重渲染）；`ModBrowser` 搜索接入 `useDeferredValue` |
| P2 | `getModsCacheDir` 模块级记忆（原每卡片 1 次 IPC + `create_dir_all`）；新增 `shared/useInViewport`（rootMargin 300px）→ `ModCard` **只对可见行**发起图片 IPC；未取到图的重试改**指数退避**（5→10→20s，上限 30s + 抖动，替代固定 5s 轮询） |

### 与方案的偏差（有意）

- **未用 CSS `content-visibility`**：紧凑行内含 `position: fixed` 的 hover 预览层，`content-visibility` 建立的 containment 会让 fixed 层相对行定位（错位）。要启用须先把预览层 portal 出去 —— 属独立改造，本轮放弃。
- **未做 `React.memo`**：行组件函数 props（`onToggle` 等）目前是每次渲染新建的内联闭包，memo 需先把 props 契约改为"稳定回调 + id 传参"（涉及 `ModBrowser` 十余处调用点）。去分块后 O(N²) 已消除，收益有限，单独一期更稳。
- **`read_def_bytes` 一并 async 化**（超出 Q1 清单）：同属"含重 IO 的只读命令"，mod 详情页每次解码 DEF 都调它。

### 验证

- `cargo check` ✅ ｜ `npm run check:invoke-parity` ✅（135 命令不变）｜ `npm run build` ✅；
- **真机首启手测待执行**（需人工按 §6 步骤：清空缓存目录 → `npx tauri dev`）。

## 9. 第二轮修复（2026-10-08 晚，实测反馈「蒙层消失后仍堵一会」）

### 根因（三类，全部落在 `boot === "ready"` 那一刻）

1. **列表首帧渲染落在蒙层撤除之后**：`setRootMods` 在 `startTransition`（低优先级）里，而 `setBoot("ready")` 走紧急更新 → React **先**提交"就绪"（蒙层撤除、列表仍为空），数百行渲染随后单独跑（`ModView` 装载 effect）。
2. **两个同步命令恰好落在 ready 帧**（主线程冻结）：
   - `get_compile_info` —— `loadEngineVersion()` 的首个调用点就在 `ModBrowser`/`ModCard` 挂载时；冷缓存会 **spawn `VCMI_client.exe -v`**；
   - `read_vcmi_settings` —— `loadVcmiLanguage()`。
3. **三个模块级订阅在 ready 之后才返回**（`loadEngineVersion` / `loadVcmiLanguage` / `loadTownPreview`）→ store 通知**数百行整体重渲染一次**（角标 / 语言提示"补齐"）。
4. 次要：`auto_check_repos_enabled`（ready 后立刻调用）、`write_mod_settings`（ready 前一行，序列化 + 备份 + 写盘）、`list_mod_ledgers`（进入管理页，N≈数百次文件 IO）也仍是同步命令。

### 修复

- **`ModView` 同批提交**：列表数据与 `setBoot("ready")` 共用**同一个 transition** → 蒙层与列表同时出现，几百行的 render 全程发生在蒙层内（不再出现"空列表帧"）。
- **`ModView` 预热**：阶段 1 与扫描并行 `loadEngineVersion()` / `loadVcmiLanguage()` / `loadTownPreview()` → ready 时它们已是终值，消除二次全量重渲染。
- **第二批 async 化**（同 P0 范式）：`get_compile_info`、`read_vcmi_settings`、`write_vcmi_settings`、`set_vcmi_language`（`settings.json` 读写共用 `VCMI_SETTINGS_LOCK`）、`write_mod_settings`、`patch_preset_lang_mods`、`clear_mod_validation`（`modSettings.json` 读写共用 `MOD_SETTINGS_LOCK`）、`list_mod_ledgers`、`auto_check_repos_enabled`、`delete_mod`。
  - ⚠️ **写路径并发语义**：同步命令原在主线程天然串行；异步化后必须显式加锁，否则「读-改-写」相互覆盖（丢更新 / 备份与写盘交叉）。
  - `set_town_preview` 等**小文件低频写**仍保持同步（几 ms 级，加锁改造面大于收益）。
- **前端兜底**：`autoCheckReposEnabled` / `listModLedgers` / `readLauncherConfig` / `detectVcmiPaths` 失败返回默认值，不把异常抛进交互流程。

### 仍未处理（建议单独一期）

**勾选 / 交互路径的全量重渲染**：`selected` 变化会连锁重算 `conflictSet`、`leftTree`（含 `localeCompare` 全树排序、`matchesQuery` 逐项正则）、右栏 `groups` / `allItems` / `forest`（两次排序），且 **600 行 `ModListItem` 无 `memo`**（其函数 props 是内联闭包，需先把 props 契约改为"稳定回调 + id 传参"；或改为 `portal` 预览层 + `content-visibility` / 虚拟化）。搜索已由 `useDeferredValue` 缓解；勾选路径暂无并发 / 切片保护 —— 若实测仍有"勾选一下卡一下"，从这里入手。
