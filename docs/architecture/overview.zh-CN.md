# VCMI-FV 启动器 · 总览

> 状态：已实现 ｜ 最后核对：2026-10-12 ｜ 代码：`src/`、`src-tauri/`、`package.json`
> 本文是整套文档的入口。按功能查 [README.zh-CN.md](../README.zh-CN.md) 索引表，按层查阅 [architecture/](./)。

## 1. 定位

一个**不修改 VCMI 源码**的独立外部桌面启动器。它站在 VCMI 之外，只做三件事：

1. **挑选**：可视化地挑选模组与子模组，自动拉齐依赖、标出冲突；
2. **落盘**：把选择写成 VCMI 原生的 `modSettings.json` 预设，VCMI 自身无需任何改动即可读取；
3. **启动**：一键拉起游戏本体或地图编辑器。

在此之上长出了两项"启动器独有"的能力：

- **百科（Wiki）**：把散落在 mod.json / LOD 归档 / `content.zip` 里的游戏数据抽成统一缓存，供离线浏览与悬停浮层使用；
- **领域（Realm）**：通过改写 VCMI 的 `dirs.json`，把"模组组合 + 存档 + 设置 + 地图"整体隔离成多套互不干扰的环境，并可导出为 `.vcmirealm` 包分发。

### 非目标

- 不修改、不重新分发 VCMI 引擎本体；
- 不接管 VCMI 官方启动器（可作为入口并存，但启动器不依赖它）；
- 不实现游戏内功能（战斗、地图编辑等一律交给 VCMI）。

## 2. 能力全景

启动器的能力分五组。**一功能一篇的文档索引与一句话说明以 [README.zh-CN.md](../README.zh-CN.md) 为唯一口径**（本文不再复制表格——两处各维护一份必然腐烂，2026-10 已实测过一次）：

- **模组线**：浏览勾选（mods）→ 预设落盘（presets）→ 仓库安装更新（mods-repository）→ 本地 / 拖放导入（game-import）→ 加密 zip 采集（encrypted-zip-handling）；
- **内容线**：百科展示（wiki）+ 缓存构建管线（modcache）+ 高清三链路（hd-assets）+ 阵营巡礼（faction-showcase）；
- **领域线**：领域运行时（realms）+ 编辑器（realms-editor）+ 远程仓库（realms-repository）；
- **设置与语言**：VCMI 设置面板（settings）+ 网络出口（network-egress）+ 界面 / 内容 / 帮助三套语言（i18n）+ 语言模组管理（lang-mods）；
- **外壳与运维**：外壳导航（shell-and-navigation）+ 首启向导（setup-wizard）+ 启动响应性（startup-responsiveness）+ 自动更新（launcher-update）。

## 3. 技术栈

| 层 | 技术 | 备注 |
|---|---|---|
| 前端 | React 18 + TypeScript + Tailwind CSS 3.4（Vite 5） | 玻璃拟态暗色主题 |
| 基础控件 | shadcn/ui（Radix 封装） | `src/shared/ui/` |
| 桌面壳 | Tauri 2（Rust） | spawn `VCMI_client.exe` / `VCMI_mapeditor.exe` |
| 归档解析 | TS 侧 yauzl；Rust 侧 `zip` + 自研 LOD / DEF / PAK | 见 [backend](./backend.zh-CN.md) |
| 图标 | lucide-react + 少量手写 SVG | — |

**版本冻结（勿随意升级）**：vite 5、typescript 5、tailwindcss 3.4.17、tailwind-merge ^2.5.5、tailwindcss-animate ^1.0.7、postcss 8.5、autoprefixer ^10.4.20；`tsconfig` 设 `verbatimModuleSyntax: false`。升级前先确认百科缓存与 DEF 解码链路不受影响。

## 4. 运行形态

- **两种互斥模式**：启动器主界面（模组 / 百科 / 设置）与领域模式（领域首页 / 信息 / 素材 / 切换 / 编辑器 / 仓库 / 巡礼）。同一时刻只渲染其一，避免两个模组管理器争写同一份配置。
- **便携部署**：Windows 数据目录固定为 exe 同级的 `.vcmi-fv-launcher/`（按 exe 路径推导，不用进程 cwd）；macOS / Linux 按平台标准目录分派（ADR 0008 D10），见 [persistence](./persistence.zh-CN.md)。
- **启动编排**（`src/App.tsx`）：目录探测 → 领域恢复 → 领域自检 → 百科缓存异步构建 → 更新静默检查。任一步失败都不阻断进入主界面（强制更新除外）。

## 5. 源码结构

```
src/                      前端
├── shell/                启动器外壳（顶栏/底栏/抽屉/错误边界）
├── features/             按功能域：mods / wiki / settings / realms / setup
├── shared/               跨域通用件 + ui/ 基础控件
├── lib/                  纯 TS 逻辑（禁止 import react）
│   ├── modcache/         百科缓存数据层与构建状态总线
│   ├── realm/            领域类型、store、登场、巡礼
│   └── def/              DEF 解码与动画缓存
├── i18n/                 UI 词典（5 语）+ 设置文案
├── assets/help/          内置帮助文档（5 语 Markdown）
└── generated/            生成物（settingDefaults / settingKeys，勿手改）

src-tauri/src/            后端
├── commands/             IPC 壳（唯一允许 tauri::command 的层，按域分文件）
├── infra/                无业务语义的设施（json/def/lod/pak/image/http/ai/...）
├── vcmi/                 游戏侧知识（paths/config/scan/town/gamedata/h3core/i18n/resource）
├── modcache/             百科缓存构建（原 wiki 域，已更名）
├── realms/               领域系统
├── repository/           远程索引与下载
├── runtime/              外挂原生组件
└── updater/              启动器自更新
```

> ⚠️ 命名沿革：`wiki` 域的数据层与后端目录已更名为 **`modcache`**（`src/lib/modcache/`、`src-tauri/src/modcache/`、`commands/cache.rs`），但用户可见的百科页面仍叫 Wiki（`src/features/wiki/`）。文档与代码一律以 `modcache` 指数据层。

## 6. 文档约定

- 中文文档用 `.zh-CN.md`，英文用 `.md`；文件名 kebab-case，**不加数字前缀**，顺序由索引表决定。
- 每篇头部标注「状态 / 最后核对日期 / 对应代码路径」。内容以**代码实测**为准，代码与文档冲突时改文档。
- **方案记录型**文档（`startup-responsiveness`、`lang-mods`）需在头部另标**体裁**，说明它是"问题诊断 / 方案 + 实施记录"而非功能规格——不套用"验证 / 开放问题"骨架，避免读者按规格类文档理解。
- 行尾统一 **LF**（`.gitattributes` 已声明 `*.md text eol=lf`）；跨行改写时注意文件实际行尾，CRLF 文件的多行替换需带 `\r\n` 才能命中。
- 文档分四层：`architecture/`（跨功能全局）、`features/`（一功能一篇）、`guides/`（可照做的步骤）、`reference/`（契约与硬事实），另有 `decisions/` 记录拍板理由。

## 7. 关键术语

| 术语 | 含义 |
|---|---|
| 预设（preset） | VCMI `modSettings.json` 里 `presets.<name>` 的一份 `{mods, settings}`，即"一套模组组合" |
| 基线（baseline） | 任何预设都强制包含的保留模组（`vcmi`、`core`）与配置里的 `mods.required` |
| scope | 模组 ID 的命名空间；`vcmi` / `core` 为 VCMI 保留 scope，前端列表隐藏但写盘时保留 |
| submod | 子模组，ID 归并为 `parent.child`，开关记在父模组的 `settings` 里 |
| 领域（Realm） | 一套隔离的 VCMI 运行环境，权威元数据是 `realms/<id>/realm.json` |
| 借用 / 整合 | 领域引用用户目录模组的两种方式：目录联接（borrowed）与复制进领域（bundled） |
| modcache | 百科数据缓存（原 wiki 缓存），两阶段构建，产物在 `.vcmi-fv-launcher/mods/` |
| `@Nx` | 高清变体图命名后缀（如 `x2`、`x4`），与原图同目录共存，原图保留作回退 |

## 8. 开放问题

- 领域模式与主界面互斥是"防争写"的硬约束，若未来要支持"领域内直接改全局模组"，需要重新设计写盘出口。
- 百科缓存的 `index.json` **只合并不删除**（见 [modcache](../features/modcache.zh-CN.md)），已卸载模组的条目靠前端实时扫描过滤，属于取舍而非缺陷。
