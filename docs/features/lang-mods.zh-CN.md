# 语言模组自动管理（按类型发现，非按名字）

> 状态：实现中（决策 B/C 已拍板） ｜ 最后核对：2026-10-10 ｜ 代码：`src/lib/builtinPresets.ts`、`src/lib/langMods.ts`、`src/features/mods/ModView.tsx`、`src-tauri/src/repository/mods.rs`、`src-tauri/src/vcmi/scan.rs`、`src/features/prelaunch/PreLaunchCheckDialog.tsx`(新)、`src/features/prelaunch/LangPackCheckWidget.tsx`(新)、`src/shell/BottomBar.tsx`、`src/features/realms/RealmShell.tsx`

## 1. 背景与问题

- 现状 `builtinPresets.ts:21` 写死 `chinese: ["vcmi","core","simplified-chinese-translation"]`。
- 实测发行版碎片化：增强版 `simplified-chinese-translation`、Expansion 1.83 `ChineseTrueTypeFonts`、官方 VCMI `chinese-translation-vcmi`。**模组名字完全无规范**。
- 现有 `langMods.ts` 是「语言 → 唯一模组 id」一对一模型，对多数发行版失效：缺失模组只 `onMissing` 提示，中文永不自动开启。

## 2. 核心决策

- **按分类不按名字**：VCMI 标准化的是 Translation 模组的**分类口径**（`modType` + `language`），不是 id。三种中文包本质都是 `modType="Translation", language="chinese"`。
- 删除 `modsByLang` 的名字映射；`__official__` 退回 `mods:["vcmi","core"]`。
- 语言包的发现 / 启用全部基于「已装模组中 `rawType=="Translation" && language==当前语言`」。
- **【决策 B · 已拍板】只认父模组本身（基础语言包），绝不递归子模组。** VCMI 模组设计过于自由：一个父模组类型标 `Translation`，其真正的其它类型能力（字体 / UI / 地图等）往往由子模组提供。因此我们只取「父模组自身的 `rawType`/`language`」，把那一个父 id 写进预设，VCMI 的依赖解析会把子模组一并带起；不把子模组枚举为独立语言包，父模组自身没声明 `language` 时也直接排除（不回退到子模组去"挖"语言）。`translationModsForLang` 额外叠 `!isSubmod` 防御（仓库 `repoTranslationMods` 同理，索引只列顶层）。
- **【决策 C · 已拍板】`modType=="Translation"` 只是"这是个翻译模组"的必要条件，不是"这是基础语言包"的充分条件。** 中文社区常把内容模组的汉化打包成独立模组（`new-chinese-mods` / `old-chinese-mods`），它们同样声明 `modType=="Translation"` + `language=="chinese"`，但只翻译内容模组、不翻译游戏本体。真正的"基础游戏语言包"用两个正交信号判定（任一命中即认，见 `translationModsForLang`）：① 后端目录结构标记 `baseTranslation`——探测到 `content/translation/{game,campaigns,maps,chronicles}.json`（VCMI 把游戏/战役/地图/历代记文本翻译出来的硬签名）；② VCMI 官方命名契约 `id` 形如 `<language>-translation`（`simplified-chinese-translation` / `chinese-translation-vcmi`）。两者都不中（非规范发行版如 Expansion 的 `ChineseTrueTypeFonts`）时退回"任意满足 `rawType+language` 的顶层模组"宽松集合兜底，保证不失效。

## 3. VCMI 底层依据

- VCMI 给 Translation 模组算 language trait；依赖解析（`ModDependenciesResolver::tryAddMods`，`ModManager.cpp:829-832`）在 `general.language` 与模组 `language` 不符时判 BROKEN 并剔出生效集（本项目 `scan.rs:201-203` 已记录）。
- 推导 1：**英文模式自动过滤中文包**（反之亦然）；不匹配包不会生效也不致乱码。**中文模式无中文包才会乱码**（实测）——这是 Tier 3 保底的硬理由。
- 推导 2：VCMI 可能把不匹配包从预设剥离 → 切回中文时须由启动器**幂等重新确保**匹配包启用（Tier 2 的必要性）。
- 由推导 1 推出：**不再主动移除其它语言包**（VCMI 已按语言特征过滤），避免与 VCMI 写盘打架。

## 4. 分层方案

### Tier 1 — 分类发现（替代名字映射）
- `builtinPresets.ts`：删 `modsByLang` 名字项；`builtinPresetMods` 去掉语言名单推导。
- `langMods.ts`：新增 `translationModsForLang(lang, installedMods)`，从已装模组筛 `rawType=="Translation" && language==lang`；`languageModsFor`/`staleLanguageModsFor` 改写为此口径，**移除 stale 移除其它语言包的逻辑**。

### Tier 2 — 主动启用已装匹配包
- 触发点同现状（切语言 / 激活领域 / 启动恢复，`App.tsx` 的 `syncLanguageMods`）：若匹配包已装未启 → 加回（幂等，仅当缺失时加）。
- 入参改为「已装模组清单」，只传命中的那一个给 `patchPresetLangMods`。
- **只在「语言真正切换」时静默启用**：对比切换前 `settings.json` 的 `general.language`，
  相同则说明语言未变（典型是每次启动恢复）——此时**不强行加回**被用户显式关闭的语言包，
  返回 `respected: true`；否则用户在 ModView 里关掉的语言包会在下次启动被悄悄撤销。
  该情况改由 Tier 3 启动弹窗给出「已安装但未启用，是否启用并启动？」的建议。

### Tier 3 — 启动前预检弹窗（阻断式确认 + 逃生口）

> ⚠️ 本 Tier 是"语言包"这一个 step 的语义。检查项会扩展（依赖完整性 / 版本匹配…），
> 通用编排、加载归属、步骤条、四场景与未来 step 路线见 **§10 启动前检测通用框架（step 编排）**。
> 其中**联网时机已修订**：原"在 `handleLaunch` 同步 await 仓库索引、弹窗延迟出现"改为
> "弹窗内异步 resolve + 报告先行"（详见 §10.3）。

**形态定稿**：`launchGame()` 前弹**引导式确认弹窗**。选它是因为它**一套实现同时覆盖**
「模组 / 全局」（`BottomBar`）与「领域」（`RealmShell`）两个启动场景，且不侵入任何既有布局
（曾评估过"常驻 chip + popover"方案，因领域页不渲染 BottomBar、且需改造两处既有页面布局而放弃）。

**架构：通用外壳 + 检查项 widget（可按需扩展）**。弹窗**不认识任何具体检查项**，只提供
「遮罩 + 卡片 + 标题 + 显式关闭按钮 + 内容槽」（`features/prelaunch/PreLaunchCheckDialog.tsx`）；
每个检查项各自实现为一个 widget（当前为 `LangPackCheckWidget`），经 `children` 并列展示。
后续新增检查类型（缺依赖 / 文件完整性 / 版本不匹配……）只需再写一个 widget，**外壳与已有 widget 均不改**。

- 定位是**阻断式确认、不是硬阻断**：始终提供逃生口「跳过并启动」（警告色 `variant="warning"`；通用词，不限"英文/中文"，未来其它语言发行版同样适用），点了照常启动。
- **三条显式关闭路径**：右上角 ✕、点遮罩、Esc。此前只有"点遮罩"，用户难以发现 → 已补 ✕ 按钮；检查项忙碌（下载/启用中）时禁用关闭，避免中断。
- **动作即关闭**：点「启用并启动 / 下载并启用 / 跳过并启动 / 前往模组管理」都会**先关弹窗再执行**。否则：① 游戏拉起后弹窗仍停留在最上层（关掉游戏回来还停在"启动前检查"）；②「前往模组管理」切页后被 z-150 遮罩盖住，表现为"点了没反应"。
- 时点在"用户刚决定启动"这一刻——问题正是在此刻产生后果，因而是最Relevant 的提示时机。
- 内容三段式引导（解决"用户看不懂为什么弹这个"）：外壳标题「启动前检查」+ widget 结论行；正文 **① 检测到了什么 → ② 不管会怎样 → ③ 建议怎么做**。
- 语言名一律本地化（`vcmiLangName`），不露 `chinese` 这类内部码。
- 领域模式额外显示「当前领域：{名}」，避免用户误以为在改全局。

三态：

| 状态 | 正文要点 | 动作 |
|---|---|---|
| `installed-disabled`（已装未启） | 将启用 {名称（id）}；不启用则英文/乱码；建议启用后启动 | 「启用并启动」+ ☐以后自动启用（不再提示）+ 逃生口 |
| `missing-repo`（未装·仓库可达） | 未安装匹配翻译模组；可从仓库获取 | 候选逐条「下载并启用」+ 逃生口 |
| `missing-no-repo`（未装·仓库不可用） | 无法连仓库，无法自动下载 | 「前往模组管理」引导 + 逃生口 |

**防唠叨（会话级，不落盘）**：预检每次启动都跑，若用户每次都选「跳过并启动」却仍被反复提示会很烦。
两项选择都记在**进程内存**（`langMods.ts` 的 `sessionSkipCheck` / `sessionAutoEnable`），
本次会话生效、重启复位。刻意**不持久化**：语言包状态随时会变（装/卸模组），
持久化的"不再提示"会变成长期静默失效，反而更难排查。

原型（状态 A）：

```
┌──────────────────────────────────────────────────────────┐
│ 启动前检查                                        ✕  ← 显式关闭 │
│ 未启用简体中文语言包                    ← widget 结论（警告色） │
│                                                          │
│ 当前游戏语言：简体中文                                    │
│ 当前领域：光明陨落                            ← 仅领域模式 │
│ 将启用：简体中文翻译（simplified-chinese-translation）  ← 已装未启才显示 │
│                                                          │
│ 现在启动的话，游戏内文字会以英文显示（中文版发行版还会     │
│ 出现乱码）。                                              │
│ 建议：启用后再启动（启用后本提示不再出现）。               │
│                                                          │
│ ☐ 以后自动启用简体中文语言包（不再提示）                   │
│                                                          │
│          [ 跳过并启动（警告色） ]    [ 启用并启动 ]        │
└──────────────────────────────────────────────────────────┘
```

## 5. 仓库候选发现（Tier 3 落地）

- 调 `fetch_mod_index`（已有 `enrich_mod_versions` 回填 `mod.json`），过滤 `modType=="Translation" && language==当前语言`。
- **后端需补**：`normalize_mod_entry` 与 `parse_mod_json_meta` 当前**不提取 `language`**（仅 `modType`）。从 `mod.json` 的 `language` 字段补提取，随 enrich 回填，**零额外网络**（enrich 本就逐条拉 mod.json）。
- 候选列表（通常 1 项）展示在弹窗；按钮「下载并启用」→ `install_mod_from_repo(id, scope)`（`scope`：全局 `None` / 领域 `"realm:<id>"`）→ 再 `patchPresetLangMods` 启用 → 继续 `launchGame()`。
- **绝不写死 `chinese-translation-vcmi`**：官方不同版本若改名，新 id 自动出现。
- enrich 纯缓存未回填 `language` 时，退化为按 `modType` 过滤让用户选，或仅警示。

## 6. 可见性

- Translation 模组本就是普通 `ModInfo`，`ModView` 已可见可切换（对齐 vcmiqt）。
- 加分项：对 `language==当前语言` 的 Translation 模组加「与当前语言匹配」角标。

## 7. 改动落点

| 文件 | 改动 |
|---|---|
| `src/lib/builtinPresets.ts` | 删 `modsByLang` 名字项；`builtinPresetMods` 去语言名单依赖 |
| `src/lib/langMods.ts` | 分类发现版；删 stale 移除；`syncLanguageMods` 入参加已装清单；`checkLaunchLangMod` 启动前四态检查；会话级（不落盘）`sessionSkipCheck` / `sessionAutoEnable`；`translationModsForLang` 用 `baseTranslation`（结构）+ `<language>-translation` 命名契约（二选一）识别**基础游戏语言包**，排除内容汉化包；`LaunchLangModCheck.matchingLabels` 回填「名称（id）」供弹窗展示将启用的语言包 |
| `src-tauri/src/vcmi/scan.rs` | `scan_mod_dir` 新增 `baseTranslation` 字段：仅对 `modType=="Translation"` 模组探测 `content/translation/{game,campaigns,maps,chronicles}.json`（散文件 + `content.zip`），区分基础语言包与内容汉化包 |
| `src/features/mods/ModCard.tsx` | 「语言匹配」正向角标（与既有「语言不符」警告对称） |
| `src-tauri/src/repository/mods.rs` | `parse_mod_json_meta` + `normalize_mod_entry` + `CachedModVersion` 补提取 `language`；缓存版本 `3→4` |
| `src/features/prelaunch/PreLaunchCheckDialog.tsx`(新) | **通用外壳**：遮罩 + 卡片 + 标题 + 显式关闭按钮（✕ / 遮罩 / Esc）+ 内容槽；与具体检查项解耦，可按需并列多个 widget |
| `src/features/prelaunch/LangPackCheckWidget.tsx`(新) | 语言包**检查项 widget**：三态结论 + 正文 + 操作（启用并启动 / 下载并启用 / 跳过并启动（警告色）/ 前往模组管理）+ 会话级不再提示 |
| `src/shared/ui/button.tsx` | 新增 `warning` 变体（琥珀/橙，供"跳过并启动"这类带后果的非破坏性动作） |
| `src/shell/BottomBar.tsx` | 启动前检查 + 通用弹窗 + 语言包 widget + `scope` + `onGoToMods`（切「模组」页）+ **动作即关闭** |
| `src/features/realms/RealmShell.tsx` | 启动前检查 + 通用弹窗 + 语言包 widget + `scope` + 领域名 + `onGoToMods`（退出领域并切「模组」页）+ **动作即关闭** |
| `src/App.tsx` | 语言同步 effect 去掉按名字的 `onMissing`；向两处下发 `onGoToMods` |
| `src/i18n/*`（5 语） | `launch.langMod*` 引导式文案键 + 泛化逃生口键 `launch.preLaunchSkipAndLaunch`（跳过并启动）；`mods.langMatch` 角标 |
| `docs/features/presets.zh-CN.md` | 更新第 4 节描述 |
| `docs/features/i18n.zh-CN.md` | 可选补充 |

## 8. 验证

- `cargo check` / `npx tsc --noEmit`
- 真机三发行版各测：切语言自动开匹配包；启动缺包弹窗；一键下载启用后正常中文。

## 9. 开放问题

- `fetch_mod_index` 纯缓存命中时 `enrich` 是否仍回填（需确认，否则 Tier 3 退化路径兜底）。
- 领域模式 `scope` 与 `presetId` 的回退链在 Tier 3 下载后启用时的口径（复用 `syncLanguageMods` 既有逻辑）。

## 10. 启动前检测通用框架（step 编排）

### 10.1 动机
Tier 3 解决了"语言包"这一种检查项；但检查项会扩展（依赖完整性、版本匹配…）。
若每加一项就改 `BottomBar` / `RealmShell` 的状态结构（当前是单一 `langCheck`），耦合会迅速恶化。
故把"启动前检查"抽象为**通用编排**：检查项 = 一个 `PreLaunchStep`，新增只需追加一项，外壳与两入口零改。

### 10.2 两层加载归属（避免"点了没反应"）
弹窗**只在"有问题"时出现**，不能为了 loading 而每次都弹。Loading 按"是否需要弹窗"切换归属：
1. **本地扫描阶段（毫秒级，纯本地）**：两个入口的「启动游戏」按钮统一显示 `检查中…` 并禁用——保证"点了有反应"。
2. **扫描完成后分流**：
   - **全部 step = ok → 不弹窗**：按钮文案切 `启动中…` 并持续 loading，直到游戏窗口拉起（或失败恢复）。
   - **存在 warn/error → 弹窗出现**：按钮恢复常态（取消 loading），弹窗接管后续进度（报告已出 + 异步方案查询）。

### 10.3 Step 模型
```ts
interface PreLaunchStep {
  id: string;                                  // 'lang-pack' | 未来 'dep-integrity' | 'version-match'
  label: string;                               // 步骤条节点标签：语言包 / 依赖完整性 / 版本匹配
  Component: React.FC<StepViewProps>;          // 该项卡片渲染器（状态图标 / 报告 / 方案 / 按钮）
  scan: () => Promise<StepScan> | StepScan;     // ① 本地扫描：必须快、纯本地
  resolve?: (s: StepScan) => Promise<StepResolve>; // ② 方案解析：可联网/重 IO，独立 spinner，失败降级
}
interface StepScan { severity: "ok" | "warn" | "error"; title: string; details?: ReactNode }
interface StepResolve { severity: "ok" | "warn" | "error"; node: ReactNode }
```
- `PreLaunchCheckDialog` 升级为**编排器**：`open` 时并行跑所有 `scan` → 每项显示 spinner→结论（**报告先行**）；
  `scan` 完成后对有 `resolve` 的项异步跑 → 回填方案区（超时/失败降级为"仓库不可用"等提示）。
- **全 ok → 不弹窗、直接启动**（自动放行）；任一 warn/error → 显示「跳过并启动」+ 各 step 处置按钮。
- 两入口共用 `usePreLaunchChecks` hook：全局 `steps=[语言包]`；领域先做硬校验（bundled/borrowed/conflict）
  → 通过后再接 `steps=[语言包]`。

> 关键修订（相对 §4 旧形态）：**联网不阻塞报告**。旧实现在 `handleLaunch` 同步 `await checkLaunchLangMod`
> （内含 `fetchModIndex`，无缓存时阻塞全量拉取 + enrich，最坏 120s 超时，`src-tauri/.../http.rs` 连接超时 12s、
> 总超时 120s）→ 弹窗延迟出现。新实现 `scan` 纯本地、`resolve`（联网）移到弹窗内异步，并配前端
> `Promise.race` 短超时（10s）兜底，超时当"仓库不可用"。

### 10.4 步骤条（Stepper）
节点 = 检测项，让用户一眼理解"本次检测了哪些项、各自结果"，并天然支持扩展（加项 = 多一个节点）。
- 节点主状态 = `scan` 的 `severity`（`pending` 灰 / `ok` 绿✓ / `warn` 琥珀⚠ / `error` 红✕）；`scan` 进行中显示天蓝脉冲环。
- `resolve` 进行中**不改变主状态色**，仅在节点外加一圈脉冲环（避免 `⚠→⟳→⚠` 抖动）。
- 连接线：已完成段着色，未到达段灰。
- **单项降级**：`steps.length >= 2` 才画连接线；单项保留节点 + 标签、无连线（避免"孤圈 + 长线"）。
- **预留滚动兼容**：节点点击 → `scrollIntoView` 聚焦对应卡片。props/ref 结构现在就预留（即便当前项少未启用交互），项多时一键开启。

### 10.5 交互文本（四场景）
（文案 key 待填 `src/i18n/*`）

| key | 文本 |
|---|---|
| `launch.preLaunchTitle` | 启动前检测 |
| `launch.checking` | 检查中… |
| `launch.launching` | 启动中… |
| `launch.stepLangPack` | 语言包 |
| `launch.reportLangOk` | 已启用与游戏语言匹配的语言包（{lang}）。 |
| `launch.reportLangDisabled` | 检测到 {lang} 语言包，但当前未启用。游戏将以英文运行。 |
| `launch.reportLangMissing` | 未检测到与游戏语言（{lang}）匹配的语言包。游戏将以英文运行。 |
| `launch.resolvingRepo` | 正在查询可下载的语言包… |
| `launch.repoUnreachable` | 暂时无法连接模组仓库，无法自动下载。可前往模组管理手动安装，或直接以英文启动。 |

- **场景 A（全 ok）**：不弹窗，按钮 `检查中…`→`启动中…`→ 拉起（可选 toast「正在启动 VCMI…」）。
- **场景 B（已装未启）**：步骤条 `① 语言包 ⚠`；卡片 = 报告 + ☐以后自动启用 + [启用并启动][跳过并启动]。
- **场景 C（未装+仓库可达）**：步骤条 `① 语言包 ⚠(外环脉冲)` + 卡片内 `⟳ 正在查询可下载的语言包…`
  → 查到后列候选 [下载并启用] + [前往模组管理][跳过并启动]。
- **场景 D（未装+仓库不可用）**：步骤条 `① 语言包 ⚠`；卡片 = 报告 + `✕ 暂时无法连接模组仓库…可前往模组管理手动安装，或直接以英文启动。` + [前往模组管理][跳过并启动]。

### 10.6 未来 step 路线

#### 10.6.1 依赖完整性（必要，本期设计、后续实现）
- **调研结论（运行前 vs 运行后）**：VCMI 官方启动器有配置项 `launcher.validateMods`
  （bool，默认 true，描述 "Validates enabled mods before launching"，见 `docs/prototypes/prototype.html:700`）
  → **运行前检测**。另 VCMI 引擎层还有 `general.validationLevel`（运行**时**加载校验严格度），二者是两层；本 step 对应**启动器层** `validateMods`。
- **结论**：依赖完整性做成**启动前 step**，对齐官方语义——校验**已启用**模组的依赖是否满足 + 文件完整性。
- severity 偏硬（缺依赖 / 文件损坏 → `error`/`warn`），但保留「仍要启动」逃生口：VCMI 运行时自身也会禁用坏 mod 后继续，与逃生口一致。
- 实现时再对齐 `validateMods` 具体算法（依赖解析 + checksum）；后端可复用 `scan_mods` 已解析的 `dependencies`。

#### 10.6.2 版本匹配（待评估，本期不实现）
某些领域对 VCMI 引擎版本有要求（依赖特定版本 API）。需评估：① 领域元数据声明最低 vcmi 版本；
② 检测当前引擎版本（权威来源 `VCMI_client.exe -v`，见项目 memory「VCMI 引擎版本怎么读」）；
③ 不匹配时给出提示/阻断。属独立一期，本期只预留 step 槽位，不实现。

### 10.7 与 §4 Tier 3 的偏差（有意）
- 联网时机：从"弹窗前同步 await"改为"弹窗内异步 resolve + 报告先行 + 10s 兜底"。
- 状态结构：从单一 `langCheck` 改为多 step 数组 + 步骤条编排。
- 加载归属：明确"按钮检查中→弹窗/启动中"双层（§10.2）。
