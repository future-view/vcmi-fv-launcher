# 高清素材（HD）

> 状态：已实现 ｜ 最后核对：2026-10-20 ｜ 代码：`src-tauri/src/modcache/hd.rs`、`hd_map.rs`、`infra/{pak,upscale,sharpen,ai}.rs`、`vcmi/hd_edition.rs`、`src-tauri/src/runtime/`

## 1. 目标与范围

**做**：为百科素材与城镇幕布提供更高清的图源，三条链路可叠加：

1. **HD Edition 真高清源**（官方 HD 版 `.pak`）；
2. **传统放大 + 锐化**（xbrz 等像素画算法 + USM）；
3. **AI 超分**（ONNX Real-ESRGAN，仅 4x）。

**不做**：改动原图。所有高清产物都是**同目录的 `@Nx` 变体**，原图始终保留作回退。

**边界（易被误解）**：这条链路只产出**启动器 WebView 用的图**（百科、阵营巡礼、模组列表/详情的城镇预览、
领域编辑器展示图），游戏内渲染由 VCMI 自己读数据，**不受 `hd.enhance` / HD 素材影响**。
因此向导与设置页的文案统一表述为「生成启动器高清图」，不再用「高清画面」这种会被读成游戏画面的说法。

## 2. 链路 A：HD Edition（真高清源）

| 模块 | 职责 |
|---|---|
| `infra/pak.rs` | 只读 `.pak` 容器解析器（magic=4、`name[20]`、zlib sheet → DDS/DXT 图集，帧表为文本整数行）；DDS 解码在 `infra::image` |
| `vcmi/hd_edition.rs` | ① 探测：在用户 / 游戏模组目录找 `hd-edition`（判据是 pak 存在，不硬编码大小写）② 注入：构建批次开始 `set_source` 一次 ③ 取图：`lookup` / `lookup_def_frame` / `probe_def_frame` / `lookup_for(name, need)`，默认取可用最高档（x3 → x2） |

`MAX_HD_SCALE = 3`。命令：`hd_edition_status`、`set_hd_edition_use`（仅 `auto` / `off`）、`set_hd_edition_scale`（仅 `auto` / `x2`）。

**覆盖范围（2026-10-20 对真实 `hd-edition` 全量帧名实测）**——HD 版只重绘了原版（RoE）8 族的部分素材，**不是"8 族全有、扩展全无"的整齐边界**：

| 有 HD | 覆盖 |
|---|---|
| 城镇幕布底图 `TB*BACK` | 8/8 族（bitmap pak；无元素城 `TBELBACK`） |
| 兵种背景 `CRBKG*` / 幕布结构帧 `TPCAS*` | 9 帧（8 族+中立）/ 25 帧 |
| 生物小图 `CPRSMALL` | 帧编号 0-121（8 族 112 只 + 中立；122-145 元素城/SoD 无） |
| 生物大图 `TWCRPORT` | 124 帧 |
| 英雄头像 `HPL*` | 130 个 |
| 特长图标 `UN32/UN44` | 172 帧 |
| 宝物图标 `HRART*` | 0-126（127 个，组合宝物 127+ 无） |
| 资源图标 `SMALRES*` | 0-6（7/8） |
| 建筑图标 `HALL*`（帧名 `TH*/TP*`） | **8 族全有**（`HALLCSTL/RAMP/TOWR/INFR/NECR/DUNG/STRN/FORT` 均命中，实测 334 个图标） |
| 城镇描边 `TOCS*` | 仅 Castle（41 帧） |
| 英雄四主属性 `PSKIL42` | 4 帧（`PSKIL42A/D/P/S`） |
| 地形块 `*TL` | 10/10（`TDTS000`/`TGRD000`/`TSND000`/`ROCKTL01`…；水域走子帧 `WATRTL01_0`） |
| 士气 / 运气 `IMRL30` / `ILCK30` | 各 7 帧 |
| 资源大图 `RESOUR82` | 7 帧（`RSRC820`…；wiki 用小图，未用） |
| 二技能大图 `SECSK82` / `SECSKILL` | 各 85 帧（`SKL8201A` 246×279 / `SKILL01A` 132×132）；wiki 用 `SECSK32`，同样已有 HD（96×96 够 84px 显示） |
| 英雄小头像 `HPS*` | 140 个（144×96；wiki 用 `HPL` 大图，无小图槽位） |
| 视野符号 `VWSYMBOL` | 91 帧（60×57 级；地图对象页无图，未用） |

| 无 HD | 说明 |
|---|---|
| 法术 `SPELLBON`（`SB01SUMB`…）、表头 `stackwin` | 一帧未收录 |
| 战争机器 `CPRSMALL` 帧 146-150（`CPRS145M`…） | 生物帧 0-121 有，战争机器帧无 |
| 元素城（Conflux）建筑图标 `HALLELEM`、幕布 `TBELBACK` | 扩展阵营未重绘（其余 8 族建筑**有**） |
| AB / SoD 扩展内容 | 元素城生物、组合宝物（`HRART*` 127+）等均无 |

⚠️ **法术图标有"同图案别的套"**：`SPELLBON` 本身无 HD，但同一法术的 `SPELLINT`（`SI01SUMB`，x3 = 144×108）与 `SPELLS`（`SP01SUMB`，x3 = 195×162）在 HD 里都有，69/70 帧可按 `SB→SI/SP` 前缀映射。**未接入**——两套画幅比（4:3 / 6:5）与 `SPELLBON`（58×64）不同，替换会改变图标外观，需产品拍板。

⇒ 命中 HD 的走真高清源；未命中的回落原版 1x + 插值变体（既有设计）。**DEF 战斗动画播放链路不接 HD**（`read_def_bytes` 只读原版 DEF 字节，前端实时解码）。

⚠️ **"有 HD" ≠ "用上 HD"**：真高清源必须先由素材阶段登记进 `<mod>/hd-map.json`（`probe_def_frame` / `probe`），阶段三才用得上；漏登记会静默退化为 1x 放大。2026-10-20 修掉三处此类漏登记/漏生成：

1. DEF 首帧路径（地形）漏登记 → `terrain/` 在 `hd-map` 里恒空；
2. 动画化 DEF 的帧名是**动画根**（`WATRTL01`），HD 按**子帧**存（`WATRTL01_0`）→ `probe_def_frame` 加首子帧回退，水域地形才登记得上；
3. `pskil42` 的目标框错写 28（UI 是 32）→ 1080p 需求恰等于源帧 42px → 一个 `@Nx` 都不生成。

### 2.1 HD 是**底图**，不是事后补丁（2026-10-20 明确）

分三类，都是"先取 HD，再谈放大"：

| 路径 | 做法 |
|---|---|
| DEF 帧图标（生物/英雄/特长/宝物/技能/士气运气/资源/主属性/建筑/地形/法术书） | 素材阶段登记 `<mod>/hd-map.json`；阶段三 `upscale_file` **直接拿 HD 图当源**放大，1x 只在 HD 未命中时兜底 |
| 幕布底图 `town` / 兵种背景 | 构建期整图直取：`composite_structures_to_canvas_hd` 按 HD 档位合成、`save_hd_or_raw_image` 命中即写 HD 图（基准图本身就是 HD，无需变体） |
| 城镇预览 `townscreen` | 合成时逐结构帧 `lookup_with_offset`（HD 图 + margin 复刻），产物即 HD 合成图 |

**仍无法接 HD 的三处**（已核实，不是漏做）：
- `townborder`（巡礼描边高亮）：HD 有同名帧，但**画框不同**（`TOCSC101` x3 = 287×108 vs 结构记录 322×210），直接替换会让高亮错位；
- `spellbon`：见上节，HD 只有"同图案不同套"；
- `warmachine` / `ability` / `stackwin`：HD 未收录对应帧。

### 2.2 源档选择（`lookup_for`）：4x 及以上取最高档 + 单步重采样

| 需求 | 选源 | 结果 |
|---|---|---|
| `need ≤ 3` | 最小覆盖、其次最小放大倍数（旧规则） | `need=2` → x2 原图直出（不跨档解大盘） |
| `need > 3`（4x 起） | **一律取最高档 x3** | 由 `upscale_file` 单步 Lanczos3 重采样到 `need × 1x`（x3→4x = 1.33×） |

原因：旧规则在 `need=4` 时选 x2 再放大 2×（`h*k` 恰为 4，体积最小），但 x2 原图细节本就不足，2× 插值后明显糊（用户实测 `@4x`）。新规则用 x3 源重采样，**体积不变**（输出仍是 4× 1x），细节更多。

## 3. 链路 B：传统放大与锐化

- `infra/upscale.rs`：可插拔 `Upscaler` trait 注册表 —— `xbrz`（默认，像素画专用，与引擎同款）、`nearest`（无损基线，max 8）、`lanczos3` / `catmullrom` / `triangle`。新算法只需实现 trait 并在 `ALL` 登记，调用方无感；实现须 `Send + Sync`。
- `infra/sharpen.rs`：USM（`out = src + amount × (src − blur)`），`DEFAULT_SIGMA = 0.6`、`ILLUSTRATION_AMOUNT = 0.8`；只作用 RGB、alpha 保留；**像素画档不叠加**。另有 `mix_rgb` 供 AI 混合。

## 4. 链路 C：AI 超分

- `infra/ai.rs`：ONNX Real-ESRGAN，**仅 4x**（`AI_FACTOR = 4`）。
- ORT 以 `load-dynamic` 运行时加载 `onnxruntime.dll`；加载前须 `pin_vc_runtime`（ORT 1.28 需 MSVC 14.40+）。
- session 池上限 8；**任何失败返回 `None` → 静默回退传统算法**，并在整进程内永久禁用。
- 本模块**不选模型、无默认模型**：模型必须在 `hd.ai.model` 显式指定。

**为什么外挂为组件**（`runtime/`，见下）：`ort` 内部的 `setup_api()` 在 dylib 缺失时会 `panic`（`.expect("Failed to load ONNX Runtime dylib")`），与"AI 失败静默回退"语义冲突。因此必须先由 `runtime::is_ready` 校验存在性 + sha256，再用 `ort::init_from`（返回 Result）并确保它是**第一个被调用的 ort API**。

## 5. 配置（`launcher.json` 的 `hd` 分组）

| 字段 | 含义 |
|---|---|
| `enhance` | 总开关，默认 false。关 = 完全不生成 `@Nx` |
| `ai.enhance` / `ai.model` / `ai.mix` | AI 子链路；`mix` 默认 0.75（`α×AI + (1−α)×传统`） |
| `edition_use` | `auto` / `off` |
| `edition_scale` | `auto` / `x2` |
| `usm` | 锐化强度，默认 0.8，0 = 关闭 |

**AI 四重生效条件**：总开关 ∧ AI 开关 ∧ model 非空 ∧ 模型在组件目录中存在；任一不满足回落 `Lanczos3 + USM`。

**首启建议**：向导**第 ④ 步**（保存前）调用
`hd_edition_status({ gameDir, userDataDir })`（可选目录覆盖，配置尚未落盘），在
`<userData>/Mods` / `<gameDir>/Mods` 下命中 `hd-edition` 的 pak 时，就地显示开关；
点「保存并进入启动器」后按开关写 `hd.enhance = true` 与 `hd.edition_use = "auto"`
—— 避免"装了 HD 版却用不上"。见 [首启向导](./setup-wizard.zh-CN.md) §6。

> ⚠️ 两个易混命令：`set_hd_enhance` 写 `hd.enhance`（放大总开关），`set_ai_hd_enhance` 写 `hd.ai.enhance`（AI 超分开关），另有 `set_ai_model`。

## 6. 变体落地（`modcache/hd.rs`）

- 构建完成后对 `images/**` 做**幂等**后处理：`<name>@<rate>x.webp`，与原图同目录。
- `stem` 已含 `@` 的跳过（防二次放大）；`MAX_HD_RATE = 6`。
- 倍率来自用户显示器，按用途分级（`hd_target_box`）。
- `HD_SCHEME_VERSION = 6`：编码 / 分级 / 命名规则变更时递增，递增后清理全部旧变体重建。v5 的语义是**变体源优先用官方 HD 素材**；v6（2026-10-09）为**小尺寸抖动插画改判像素画**——源图长边 ≤ `SMALL_DITHER_EDGE` 的试算"插画"（creature/hero 58×64 抖动头像）在 AI 未接管时改走 xBRZ + 无损（实测 Lanczos3 把抖动磨成糊斑，xBRZ 可辨识度显著更高）。
- `hd_map.rs`：阶段三只有缓存图路径、反查不到原版资源名，故素材阶段把「缓存图 → HD 资源名」登记进 `<mod>/hd-map.json`（线程局部、按 owner 分组 flush），阶段三据此用 `hd_edition::lookup_for` 取真高清源，而不是从 1x 插值。
  - ⚠️ **登记键 = 磁盘文件 stem 口径**（`modcache::sanitize_asset_name`，保留 `~`）：产物名约定「资源标识`~`帧标识」（`cprsmall_def~g0f100`），阶段三按磁盘 stem 反查本表。曾误用 `sanitize_id`（modId 目录名归一，`~`→`_`）导致 140 条登记 0 命中、高清变体全部退化为插值（2026-10-20 修复，`ASSETS_VERSION = 3` 触发重建）。两个 sanitize 是**两套口径，禁止互用**。
  - 登记的**补齐依赖素材重跑**：图已存在会走 `wiki_image_exists` 短路、不补登记，故装/卸 hd-edition 靠 `.hd-src` 指纹变化触发 `clear_stale`（删除范围含本 mod 引用的 `core/images/` 共享区，见 `build/mod.rs::remove_recorded_assets`）重跑补齐。
  - ⚠️ **登记表是多 mod 共建的共享表**：`clear_stale` 只能按「本 mod cache.json 记录过的产物键」**精确摘除**（`hd_map::prune_recorded`），绝不能删整个 `hd-map.json` —— 父 core 重建时删整表会把 8 族 `core.<faction>` 累积的成百上千条登记一并抹掉，而重建只补回自己那一部分（2026-10-20 实测 1010 → 140，高清变体全军覆没；该事故还曾因 probe 在 NeedData/NeedAssets 间摇摆而表现为**非确定性**，时好时坏）。

## 7. 外挂运行时（`runtime/`）

管理"不随每次版本更新变化的外部依赖"。理由：ORT 静态链进 exe 会让体积 21.7MB → 42.8MB，而更新是整包替换，每次都要重下 30MB；外置后组件只装一次、常驻磁盘。

- 三种分发形态共用同一逻辑：便携内置（发行方放文件）/ 安装包内置（`tauri build -c tauri.ai-runtime.conf.json`）/ 远程下载（配 `package`）。
- 声明：`runtime.components.<id>.dir` 与 `.package{url, sha256, size, version}`。**文件清单不进配置**，由代码常量 `COMPONENTS` 定义（AI = `onnxruntime.dll` + `onnxruntime_providers_shared.dll`，各带 sha256；随包 2 个 `.onnx` 模型为**可选**）。
- 落点：三种形态**完全同一处** `<exe>/.vcmi-fv-launcher/runtime/ai`。
- 校验：`is_ready` = 必需文件齐备 **且** sha256 全中。
- 命令：`runtime_status` / `runtime_install`（阻塞：下载 → 整包 sha256 → 白名单解压 → 逐文件校验 → 原子替换；与 updater 非空闲阶段互斥，否则报 `runtime_install_busy`）/ `runtime_cancel` / `runtime_uninstall`（只删程序文件，不动用户数据）。

组件下载走统一网络出口，见 [network-egress](./network-egress.zh-CN.md)。

## 8. 边界与容错

- AI 全链路失败静默回退，绝不阻断启动或缓存构建。
- 变体是"锦上添花"：缺任一变体都回退原图，界面不会出现破图。
- 模型校验是本地 sha256 扫描（读目录），**不走网络**。

## 9. 验证

```bash
npm run hd:preview   # 放大效果预览
npm run hd:ai        # AI 超分实验
npx tauri dev        # 真机：设置页观察 HD 版状态卡与 AI 组件三态
```

## 10. 开放问题

- `hd_target_box` 的用途分级目前是内置表，未按显示器 DPI 动态推导。
