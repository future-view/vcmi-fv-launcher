# 首启向导

> 状态：已实现 ｜ 最后核对：2026-10-17 ｜ 代码：`src/features/setup/SetupWizard.tsx`、`src/App.tsx`、
> `src-tauri/src/vcmi/paths.rs`、`src-tauri/src/commands/{settings,paths,updater}.rs`、
> `src-tauri/src/infra/{fs,http}.rs`、`src/lib/mirrorRules.ts`、`src/i18n/langs.ts`

## 1. 触发条件

`detect_vcmi_paths()` 返回 `ready === false` 时独占渲染（此时不进入主界面，也不渲染强制更新）。

## 2. 步骤（**固定四步**，没有时隐时现的步骤）

步骤条恒定四项：`① 游戏目录` `② 用户数据目录` `③ 游戏数据文件` `④ 汇总确认` +「第 n / 4 步」。
**「保存并进入启动器」只在最后一步（④）出现**；中途前进由各步自身条件把关
（③ 的「下一步」在原版数据未就绪前禁用）。

**① 游戏目录与界面语言**（只放必填项）

- 界面语言（下拉，默认跟随系统）；
- 游戏目录（自动探测回填，或手动选择，命令 `suggest_user_data_dir` 顺带推导用户目录）；
- `[自动探测]` `[下一步]`（未选游戏目录时禁用）。

**② 用户数据目录**

- 「使用启动器专属用户数据目录」开关（默认关）
  - 关：`<VCMI 官方目录>`（由 `dirs.json` / 默认规则解析）；
  - 开：`<exe 同目录>/userdata`，数据随启动器走；
- **数据同步**：显式按钮 `[同步既有用户数据]`，**仅在目标目录 ≠ 同步源时显示**（见 §4）；
  目标即来源时显示一句"当前目录即数据来源，无需复制"（避免开关开了却什么都没有的困惑）；
- `[上一步]` `[下一步]`。

**③ 游戏数据文件**（固定步骤：检测 + 导入，硬闸门）

进入本步即检测（此时用户数据目录已在 ② 定稿，判定根不会再变）；
就绪 = 一行绿字（含命中根）；缺失 = 就地展开数据目录选择 + 复制 + 重试；探测中 = 占位。
`[下一步]` 在数据未就绪前禁用。详见 §4.1。

**④ 确认并保存**（只读信息 + 保存入口）

- 游戏目录 / 用户数据目录两行只读汇总；
- 派生目录卡片（"由用户数据目录自动推导（无需手动设置）" + `config` / `Mods` 各一行）；
- **可选内容汇总**：高清版素材 / 历代记战役的当前选择（无论装不装都显示，见 §4.2）；
- **（条件）清单不可达 → 建议开启 GitHub 镜像**（见 §5）；
- `[上一步]` `[保存并进入启动器]`。

**第 ④ 步内**：若探测到 `hd-edition` → 就地显示建议面板（见 §6），开关随「保存并进入启动器」一起写入。

> 设计取舍：曾把数据检测内联进 ②、并让每步都显示保存按钮——实际更难用（② 被同步区和
> 数据面板挤爆、复制能力的入口被淹没、① 还要为"不可信的检测"背提示）。固定四步 +
> 末尾汇总，是"每步只做一件事 + 最后确认"的常规向导形态。

> 分步的动机：单屏版本在 889px 窗口下必然出现滚动条（内容 ~780px > 88vh），
> 且开关展开后一屏同时有 7 个纵向块，信息密度过高。分步后每屏 ~300px，
> 任何界面语言都不会滚动。**不做双列**：双列会让单屏信息量进一步翻倍。
> 第 3 步同理：**不能放在第 2 步检测**——数据来源有三处（游戏目录 / 用户数据目录 /
> 自定义导入），且第 2 步的「专属目录 + 同步」本身会改变判定结果（同步可能顺带带来
> `Data/*.lod`，也可能仍是空目录），只有在目录尘埃落定后检测才不出误判。

## 3. 落盘

写入 `launcher.json`：

- `paths.game_dir`
- `paths.user_data_dir`
- `paths.redirect_user_data`（= 界面上的「启动器专属目录」开关）
- `ui.lang`

「启动器专属目录」另经既有机制接管 `<游戏目录>/config/dirs.json` 的 5 个真实 key
（`VcmiPaths::ensure_user_data_dir` → `apply_user_data_redirect`），并播种
`settings.json` 的 `launcher.setupCompleted`（让官方启动器不再弹首次运行向导）。

## 4. 数据同步（第 2 步的**显式**动作）

**动机**：多个发行版共用 `文档\My Games\vcmi` 会互相踩踏（预设 / Mods / 存档混杂）。

**交互定案**：同步是独立动作，**不由保存隐式触发**——

- 第 2 步有独立按钮 `[同步既有用户数据]`（**通用名，不耦合路径**；来源以"来源：…"单列一行小字，
  源 = 解析出的用户目录，无 `dirs.json` 时为官方默认目录）；
- 三种状态（待同步 / 进行中 / 已同步）在**同一个框内原地切换**，不推移页面其它内容；
- 点击即开始，进度**就地**展示（按钮原地换成进度条 + `[跳过复制]`），保存按钮不被劫持；
- **可见性联动**：目标目录 = 同步源（维持默认目录）时整个同步区隐藏 —— 数据本来就在那里，
  同步无从谈起；开了专属目录或手动改选其它目录时才出现；
- 未点同步就保存 = **使用空目录**，不做兜底隐式复制（不在界面常驻说明，节省文案）。

规则：

| 情形 | 行为 |
|---|---|
| 目标目录已存在 | **不复制**（绝不覆盖已有数据），按钮旁提示"目标目录已存在，不会复制" |
| 源目录不存在（全新安装） | 按钮禁用 + 提示"未检测到既有用户数据，将创建空目录" |
| 源 = 目标 / 目标在源内部 | 拒绝（`setup_userdata_invalid`） |
| 用户点「跳过复制」 | 置取消哨兵 → 删除 staging → 回到未同步态（可用空目录继续保存） |
| 复制中途关闭启动器 | 进程退出、复制线程被强杀；**此时配置尚未落盘** ⇒ 下次启动原样重入向导 |
| 同步完成后改动路径/开关 | **锁定**：防止复制到旧路径留下孤儿目录；改回默认目录会作废同步结果 |

**顺序是契约**：

```
① 同步（staging → rename，copyOnly 不写配置）→ ② 保存：写 launcher.json → ③ 接管 dirs.json
```

- 复制进**同级 staging** `<目标>.fv-copy-tmp`，完成后 `rename` 就位（同卷瞬时）；
  staging 残留会在下次迁移开始前清理。
- 复制实现：`infra::fs::copy_dir_progress`（显式栈递归、200ms 节流进度、可取消哨兵，
  与 `realms::transfer::EXPORT_CANCELLED` 同款）；体量指标来自复制前的整树预扫。
- 进度经事件 `user-data-copy-progress` 推送 `{doneFiles, doneBytes, totalFiles, totalBytes}`。
- **实测体量**（`references/`）：增强版用户目录 `Mods` 1.2 GB（其余项合计 ~20 MB），
  SSD 约 3–12 s、HDD 约 20–40 s ⇒ 走向导内同步进度条，不做主界面后台任务。
  `cache/` 与 `logs/` 只占 1.7%，**不值得**为它们引入排除逻辑，整目录复制。

## 4.1 原版数据文件（第 3 步硬闸门）

**动机**：缺 `H3Bitmap.lod` / `H3sprite.lod` 时 **VCMI 根本起不来**（`EntryPoint.cpp:294-303`：
`DATA/HELP.TXT`、`DATA/PLAYERS.PAL`、`SPRITES/DEFAULT.DEF` 是硬前置；ROE-only 数据含 HD 版直接致命），
百科图片反而是次要问题。旧流程只在百科构建后弹横幅，用户已填完向导才发现游戏跑不起来。

**检测口径 = 官方两份逻辑取严**（`src-tauri/src/vcmi/h3data.rs`）：

| 官方 | 位置 | 本模块 |
|---|---|---|
| 首启：`ROE(GENRLTXT.TXT) && (SOD(TENTCOLR.TXT) \|\| demo(H3DEMO.H3M))` | `firstlaunch_moc.cpp:331-335` | `roe && (sod \|\| demo)` |
| 运行期 fatal：HELP.TXT / PLAYERS.PAL / DEFAULT.DEF | `EntryPoint.cpp:294-298` | `help_txt && players_pal && default_def` |

`found = roe && (sod || demo) && help_txt && players_pal && default_def` —— 比官方首启严一点，
换取"初始化通过 ⇒ 游戏一定能跑"，不在启动时才炸。

**资源解析按挂载表而非文件存在**（`config/filesystem.json`）：`DATA/` = `H3ab_bmp.lod` /
`h3abp_bm.lod` / `H3bitmap.lod` / `H3pbitma.lod` 的条目 + 散目录 `Data`；`SPRITES/` =
`H3ab_spr.lod` / `h3abp_sp.lod` / `H3sprite.lod` 的条目 + 散目录 `Sprites`（**LOD 一律从
`Data/` 读**——`Data/H3sprite.lod` 挂的是 `SPRITES/`，此前按挂载点找 LOD 会把 sprite 包漏掉，
该 bug 由真实安装 fixture 单测抓住）。全部大小写不敏感。

**第 ③ 步交互**（`H3DataGate`，固定步骤）：

```
③ 游戏数据文件
  原版数据：○ 检测中…                ← 占位，不展开
  原版数据：● ✓ 已就绪（游戏目录）    ← 一行绿字，说明命中根
  原版数据：● ⚠ 未检测到（H3Bitmap.lod、H3Sprite.lod）
            缺少这些数据文件 VCMI 将无法启动（百科图片也会缺失）。
            数据目录 [ 注册表候选/手动选择 ] [选择…]
            将复制到：<游戏目录>\Data
            [复制数据文件]  [重新检测]
底部：[上一步]  [下一步（未就绪禁用，hover 说明原因）]
```

- **没有跳过按钮**：唯一出路是复制 / 手动放置后重新检测；未就绪时「下一步」禁用
  （`handleSave` 里还有一次双保险：缺数据则踢回本步）；
- **探测时机**：进入 ③ / 用户数据目录变化 / 同步结束。③ 是固定步骤，用户数据目录
  在 ② 已定稿，不存在"检测完又变"的时序问题；
- **注册表候选按需查**：常态探测 `with_candidates=false`（省 `reg query` ×4 约百毫秒），
  确认缺失后才补一次 `true` 用于预填数据目录；
- **未确认即阻断（fail-closed）**：探测失败（命令异常 / 后端未返回 `h3Data`）时同样禁用
  「下一步」并提示“无法确认原版数据状态”+`[重试]`。
- **复制目标固定 `<游戏目录>/Data`**：领域切换时该目录被所有领域共享；放进用户数据目录
  会"切领域即失效"。
- **来源**：任意外部目录（`resolve_source` 同时接受 `Data` 本身与含 `Data` 的游戏根目录，
  并给出源根用于定位兄弟目录 `Mp3` / `Maps`）；
- **复制范围**（**全部跟随 radio 目标根**——`.lod` / `Mp3` / `Maps` 同一个根，用户实测纠偏）：
  - `.lod` / `.snd` / `.vid` → `<目标根>/Data`（判定根之一）。**`.snd`/`.vid` 不可省**：
    只搬 `.lod` 时 VCMI 启动会弹「未检测到视频文件」（缺 `VIDEO.VID` / `Heroes3.vid`），
    官方 `classifyTargetByExt` 同样把这两类归到 Data（`.pak` 除外——VCMI 拒绝 HD 版数据）；
  - `Mp3` → `<目标根>/Mp3`（**默认勾选**，对齐官方 `copyHeroesData`；缺了只是没音乐）；
  - `Maps` → `<目标根>/Maps`（**可选**，默认不勾；且无论勾不勾，`Maps` 目录都会被创建
    ——VCMI 只自建 `cache/config/logs/Saves`，不建 `Mods`/`Maps`，见 vcmi-dirs 硬事实 7；
    `ensure_user_data_dir` 的创建清单已补上 `Maps`）；
  - 一律**不覆盖**已存在文件。
- **`.lod` 目标 radio**：默认「游戏目录 Data」——**领域只隔离用户数据目录，游戏目录被所有领域共享**，
  放进用户数据目录会导致切换领域后数据不可见、对应领域直接起不来。仍保留「用户数据目录 Data」
  选项（游戏目录只读等场景），选中时给 amber 警示。

## 4.2 可选内容（第 ③ 步配置 / 第 ④ 步汇总）

| 选项 | 交互 | 落地方式 |
|---|---|---|
| **高清版素材** | 第 ③ 步开关（默认关）→ 展开后：目录选择 + `[安装]` | 走 `import_game_files`：HD 安装目录（含 `HOMM3 2.0.exe` 的目录或该 exe 本体）→ `<目标根>/Mods/hd-edition` |
| **历代记战役** | 第 ③ 步开关（默认关）→ 展开后：安装器选择（`.exe`）+ `[安装]` | 同一命令：Inno Setup 特征串校验 → **内建解包**（`vcmi::inno`，零外部依赖）→ 生成 `<目标根>/Mods/chronicles` + 8 个 `chronicles_<n>` 子模组（官方同款结构） |

两项共用一个 **`安装到` radio**（游戏目录 Data / 用户数据目录 Data，默认游戏目录，同 LOD radio
的领域语义）；安装进行中**临时禁用 `上一步` / `下一步`**（避免没处理完就切页）。

- **点击即装，不等保存**：`import_game_files` 加了可选 `userDataDir` 覆盖（配置未落盘时也能
  按用户选的目录写），并改为**异步 + `spawn_blocking`**——HD 要拷几百 MB、历代记要内建解包
  675 MiB（实测 ≈57s），同步命令会把 WebView 冻住；
- **历代记解包是内建实现**（2026-10-18 起，零外部依赖）：`vcmi::inno`
  （PE/设置头探测 + 纯 Rust LZMA1 + MD5 自校验）+ `vcmi::chronicles`（官方模组结构生成）。
  官方启动器把 innoextract 编成 GPL 库、`vcmibuilder --gog` 也要求 PATH 里有它，
  但本仓库 `references/vcmi/launcher/lib/innoextract` 子模块为空，无法照搬 ⇒ 自研复刻。
  真实包（`setup_heroes_chronicles.exe`）实测：全量 218 文件 / 675 MiB / MD5 全等；
  端到端导入 8 篇齐全（见 `chronicles_real_installer_end_to_end` ignored 测试）。
  **不再需要 `.bin`**、不需要任何外部程序；
- **临时目录跟随所选目标**：`<目标根>/tmp/chronicles-extract`，导入结束即删；
- **已安装即锁定**：安装成功后该行的输入框 / 选择 / 安装按钮全部禁用，避免二次操作；
- **失败语义**：HD 失败 → 红字（可重试）；历代记失败（非 Inno 包 / 安装包不完整）→ 琥珀字 + 指引（不阻断）；
- **历代记产物不是"复制 Data/Mp3/Maps"，而是官方同款模组**（见下节）；
- **装好后不再占说明行**：第 ③ 步两项各自安装成功（`hdInstalled` / `chrInstalled === true`）后，
  `hdInstallHint` / `chrInstallHint` 两段说明隐藏，只保留绿色结论行（减少滚动与信息重复）；
- 第 ④ 步恒定显示两项：`高清版素材: 已启用|未启用` / `历代记战役: 已启用|未启用`（只显示启用状态，不显示来源路径）。
  其中 HD 行按「本次装好 **或** 探测到既有 `hd-edition` 且建议开关打开」显示已启用。
- **注册表自动定位**（`locate_installs`，Windows）：GOG `1207658787` 与
  `New World Computing\…\1.0`（对齐官方 `getHeroesInstallDir`），命中即预填数据目录。
- **时机**：点第 2 步「保存并进入启动器」时用最终 `(游戏目录, 用户数据目录)` 复核；
  缺 → 转第 3 步；齐 → 直接保存。因此"同步刚带来 Data"与"用户改过目录"两种时序都正确。
- **GOG 离线安装器提取已实现**：走内建解包（`vcmi::inno`），不外挂依赖、不捆绑二进制；
  注册表定位（`locate_installs`）仍用于预填游戏目录。

**后端接口**（全部扩参，零新增命令）：

| 命令 | 扩展 |
|---|---|
| `gamedata_health_check` | 可选 `game_dir` / `user_data_dir`（向导可对未保存目录体检）；返回加 `h3Data` 与 `dataInstallCandidates` |
| `write_launcher_config` | `copyOnly` + `copyH3Data` + `h3DataFrom` / `h3DataTo`（目标必须是 `…/Data`） |

## 5. 清单可达性探测 → 一键 GitHub 镜像

- 探测：复用 `check_launcher_update(force, probe=true)`（**未新增命令**）→
  `updater::manifest::probe_manifest()` → `infra::http::probe_url`；
  **连接 5s / 总 8s、直连、不走镜像与代理回退链**（否则自我循环），收到任何 HTTP 状态都算通。
- 触发建议的条件：探测失败 **且** 清单地址属 GitHub 主机（`is_github_url`）**且**
  当前界面语言在 `UI_LANG_FEATURES` 中开了 `githubMirrorHint`。
  > 探测本身**始终执行**（与界面语言无关）；只有"建议卡片"受语言能力表控制 ——
  > 镜像是区域性便利设施，不该向所有语言用户推销。
- 一键开关 = 写一条 **prefix 模式**规则（hosts = GitHub 全家桶 5 域，前缀默认
  `https://v4.gh-proxy.org/`，见 `src/lib/mirrorRules.ts`），**追加**而非覆盖既有规则；
  写坏/写失败不阻断初始化。之后可在设置页「镜像规则」卡编辑（与向导同源常量）。
- 文案必须弱承诺：镜像是社区服务，不保证可用；失败仍回退直连。

## 6. HD 素材建议（已并入第 ④ 步）

进入第 ④ 步时调 `hd_edition_status({ gameDir, userDataDir })`——`gameDir` / `userDataDir` 是
**可选目录覆盖**（向导阶段 `launcher.json` 还没写，不能只靠已保存配置；设置页仍按原方式不传参）。
探测范围：`<userData>/Mods` 与 `<gameDir>/Mods` 下定位 `hd-edition` 的 pak。

命中则在「派生目录」卡片下方就地显示建议面板，默认勾选「启用生成高清素材」；
点「保存并进入启动器」时**先写配置**（`write_launcher_config`），再按开关写：

- `hd.enhance = true`（`set_hd_enhance`）
- `hd.edition_use = "auto"`（`set_hd_edition_use`）

关掉开关 → 只写配置，不改这两项（之后仍可在设置页开启）。

> 2026-10-18 之前这里是「保存后再单独开一屏建议面板」，会打断流程、丢失向导上下文；
> 现已合并进第 ④ 步，随保存一次完成。

## 7. 目录识别规则

优先读 `<游戏目录>/config/dirs.json` 的 5 个真实 key（支持环境变量展开，含增强版特有的
`%USERDOCUMENTS%` 映射），缺失则回退 Windows 默认用户目录；未显式指定的 4 个 key 由
`userDataPath` 派生。完整硬事实（双重挂载、junction、预设迁移等）见
[vcmi-dirs](../reference/vcmi-dirs.zh-CN.md)。VCMI **不会**自动创建 `Mods` 与 `Maps` 子目录，需要启动器自建。

**专属目录放在游戏目录内也是安全的**：VCMI 的初始加载器只把 `dataPaths()` 与 `userDataPath`
各自根下的 `MODS` / `CONFIG` / `DATA` 挂进虚拟命名空间
（`references/vcmi/lib/filesystem/Filesystem.cpp:147-156`），不会递归扫到兄弟目录。

## 8. 边界与容错

- `vcmi/setup.rs` 解析失败**必须报错**，禁止降级为空对象覆写。
- `write_launcher_config` 走「宽容继承」回退，防止向导清掉便携包预置项。
- 写入 `dirs.json` 一律用**绝对路径**；**请求接管却写失败必须上报**（否则界面显示的目录
  与 VCMI 实际使用的目录不一致，属静默错配）；未请求接管时沿用"非致命、只记日志"。
- 用户数据目录 = 游戏目录 → 拒绝（`setup_userdata_invalid`）。
- 新增能力一律**复用既有命令扩参数**，不新增 Tauri 命令（本机工具链 never-type 硬错误，
  见 [ipc](../architecture/ipc.zh-CN.md) §1）：`write_launcher_config` 增加
  `redirectUserData` / `copyFrom` / `copyOnly` / `cancelCopy`，`check_launcher_update` 增加 `probe`。
  - `copyOnly: true` = 只迁移、不写任何配置（第 2 步同步按钮），出参
    `{saved, copied, cancelled, files, bytes}`；保存按钮自身**不传 `copyFrom`**。

## 8.1 首启标题栏（与主界面**同一布局模板**，必须有）

初始化未完成时 `App` 只渲染向导、**不挂主界面**（含 `TopBar`），窗口因此没有任何
最小化 / 最大化 / 关闭入口（只能 Alt+F4 或任务管理器）—— 不可接受。

「初始化 / 模组 / 百科」统一标题栏的实现方式：**共享原子 + 各自组合层**，
而不是让完整 `TopBar` 传空 props（那会渲染一排点不动的灰页签，误导用户）：

| 层 | 文件 | 内容 |
|---|---|---|
| 原子 | `src/shared/AppBrand.tsx` | 品牌块（logo + 名称），主界面 `TopBar` 同源 |
| 原子 | `src/shared/WindowControls.tsx` | 窗口三键（首启不传 `onSettings`，设置此时不可用） |
| 组合 | `src/shell/InitTopBar.tsx` | 60px 高、同底色/分隔线/拖拽区；**无**页签 / 设置 / 多语言 |
| 组合 | `src/shell/TopBar.tsx` | 主界面：页签 + 设置 + 多语言 + 窗口控件 |

- `App.tsx` 的 `!ready` 分支 = `InitTopBar` + 垂直居中的向导卡片（卡片自身不再 `fixed` 全屏遮罩）；
- 多语言不放首启标题栏：向导第 1 步已有语言选择，标题栏再放一份是重复入口；
- 与「复制中途关闭」的语义一致：关闭只是进程退出，**配置尚未落盘** ⇒ 下次启动重入向导。

## 9. 多语言布局

- 弹窗宽度 `max-w-2xl`（672px）：加宽是为让四项步骤条在**长语言**（俄/德）下仍单行显示；
  步骤标签 `text-[11px]` + `max-w-[8.5rem] truncate` + `title` 全称，四步恒定一行不换行。

- **分步 + 弹窗固定高度**：单屏版本（7 个纵向块）中文 ~780px 即超出 88vh（889px 窗口）。
  弹窗高度固定 `min(520px, 88vh)` = **最长语言（俄/德）在第 2 步常规态**（专属目录 + 同步区 +
  派生卡片）的高度；切步骤时标题区 / 底部功能区**不跳动**。叠加态（再加 GitHub 镜像建议卡、
  或同步进行中）由内容区滚动兜底，底部按钮始终可见。
  第 1 步内容**顶部对齐**（不做垂直居中——居中会让表单位置显得随意，用户已否决）。
- 三段结构：**顶部标题区**（标题 + 副标题 + 步骤条，固定）/ **中部内容区**（唯一随步骤
  切换与局部刷新的区域，独立 `overflow-y-auto`）/ **底部功能区**（`自动探测`、`上一步`、
  `下一步`、`保存并进入启动器` 固定在同一行位置，HD 阶段右侧按钮即"继续"）。
- 长文案语言（俄语 / 德语）的安全阀：开关标签与提示行一律 `min-w-0` + 独立成行换行，
  路径输入 `min-w-0` + 选择按钮 `shrink-0`，进度数字走"键 + 值"两段式（`{arg}` 单一占位）。
- 新增文案落在 `setup.*` 段，5 本词典 + `i18n/types.ts` 同步（`LangDict` 穷举，缺译编译失败）。

## 10. 验证

```bash
npx tauri dev   # 真机：清空 launcher.json 的 paths 后重启，走完整向导
# 1) ① 选目录 → ② 默认路径 → ③ 数据就绪 → ④ 保存 → 无 dirs.json 改写
# 2) ② 开专属目录 + 「同步数据」→ 观察进度条 / 「跳过复制」/ 关闭窗口中断后重入；
#    目标即来源时应显示"无需复制"而不是空白
# 3) 同步完成后确认路径与开关已锁定；「上一步」改游戏目录后同步结果作废
# 4) ③ 把 Data/*.lod 临时改名 → 数据面板展开且「下一步」禁用；复制 / 重检后恢复
# 5) ④ 断网（GitHub 清单）→ 中文界面出现镜像建议；切英文界面消失
# 6) 各语言（俄/德长文案）下确认四屏都不出现滚动条
# 7) 排查"闸门没拦"：日志（.vcmi-fv-launcher/logs）应有
#    「原版数据探测：found=… roe=… sod=… missing=… game=… user=…」一行，
#    据此判断是真·有数据、还是探测/IPC 失败（后者会看到「无法确认原版数据状态」）
cargo test --lib h3data   # 真实原版安装（references/Heroes3-EN）上的官方口径回归
```

## 11. 开放问题

- 同步仅在首启提供：已初始化后想再迁移数据，暂无入口。

### 4.3 历代记产物结构（对齐官方 `chroniclesextractor.cpp`）

历代记**不是**"把 Data/Mp3/Maps 拷进用户数据目录"，而是生成一个 Expansion 模组
（`vcmi/chronicles.rs`）：

```
<目标>/Mods/chronicles/
  mod.json                       # modType=Expansion + mapFormat.chronicles.supported + Tarnum 头像表
  content/config/portraitsChronicles.json
  content/Data/HPS137.pcx …      # 从基础游戏 bitmap.lod 抽的 Tarnum 头像
  Mods/chronicles_<n>/
    mod.json
    content/Data/chronicles_<n>/     # xBitmap.lod / xlBitmap.lod 条目
    content/Sprites/chronicles_<n>/  # xSprite.lod / xlSprite.lod 条目
    content/Video/chronicles_<n>/    # xVideo.vid 条目
    content/Sounds/chronicles_<n>/   # xSound.snd 条目
    content/Maps/Chronicles/Hc<n>_Main.h3c   # 战役（原 Main.h3c 改名）
```

- 篇目按 `app/<篇名>` 目录识别（8 篇全量，支持 All-in-one 与分卷）；
- `data` 目录可能在篇名目录内、也可能在其外（GOG V2 安装包），两种布局都兼容；
- `.vid` / `.snd` 条目解析见 `infra/archive.rs`，格式取自官方
  `lib/filesystem/CArchiveLoader.cpp:103-169`（`.vid` 无长度字段，按相邻 offset 推导）；
- **mod.json 文案与多语言**：全部来自 `src-tauri/resources/mod-meta.json`（经
  `vcmi/modmeta.rs` 用 `include_str!` 编译期嵌入，`.rs` 不含任何译文）。
  遵循 VCMI 的 mod 翻译方案：根字段英文兜底，语言块（键取 VCMI 语言全名，如 `chinese` /
  `polish` / `russian`）只覆盖有官方译文的字段；篇名保持英文。译文取自官方启动器
  `launcher/translation/*.ts`（12 种），俄语取核心模组的 `vcmi.campaignSet.chronicles`。
  两个模组都**不引入游戏内字符串**，故不生成 `content/translation/<lang>.json`
  （官方也不生成；空表无意义）；
- 回归用例：`cargo test --lib vcmi::chronicles vcmi::modmeta`（后者校验语言键 ∈
  VCMI 29 项枚举、模板占位符、中/波/英渲染；前者断言官方目录/文件/改名与语言块）。
