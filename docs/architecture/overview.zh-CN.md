# VCMI-FV 启动器 · 总览

> 状态：已实现 ｜ 最后核对：2026-09-30 ｜ 代码：`src/`、`src-tauri/`、`package.json`
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

| 功能域 | 一句话 | 文档 |
|---|---|---|
| 外壳与导航 | 顶栏页签、底栏入口、窗口控件、启动编排 | [shell-and-navigation](../features/shell-and-navigation.zh-CN.md) |
| 首启向导 | 首次运行时确定游戏目录 / 用户目录 / 界面语言 | [setup-wizard](../features/setup-wizard.zh-CN.md) |
| 模组浏览与选择 | 左右分栏、城镇幕布、依赖自动勾选、冲突标红 | [mods](../features/mods.zh-CN.md) |
| 预设管理 | VCMI 原生预设的增删切换、导入导出、启用报告 | [presets](../features/presets.zh-CN.md) |
| 模组获取与更新 | 从 VCMI 官方仓库索引安装 / 更新 / 删除模组 | [mods-repository](../features/mods-repository.zh-CN.md) |
| 游戏文件导入 | 把地图 / 存档 / 模组 zip / HD 素材导入到正确位置 | [game-import](../features/game-import.zh-CN.md) |
| 百科 | 9 类目离线浏览、详情分发、右键悬停浮层 | [wiki](../features/wiki.zh-CN.md) |
| 百科缓存管线 | 两阶段构建、素材抽取、HD 后处理、进度与取消 | [modcache](../features/modcache.zh-CN.md) |
| 高清素材 | HD Edition `.pak` 真高清源 / 传统放大 / AI 超分 | [hd-assets](../features/hd-assets.zh-CN.md) |
| 设置面板 | VCMI 12 节设置 + 启动器自有设置 | [settings](../features/settings.zh-CN.md) |
| 领域系统 | 隔离环境的生命周期、激活、地图阀门、自检 | [realms](../features/realms.zh-CN.md) |
| 领域编辑器 | 六标签页编辑领域元数据 / 模组 / 地图 / 巡礼 | [realms-editor](../features/realms-editor.zh-CN.md) |
| 远程领域仓库 | 拉取领域索引、下载安装 `.vcmirealm` | [realms-repository](../features/realms-repository.zh-CN.md) |
| 阵营巡礼 | 全屏播放阵营幕布 / 英雄 / 主题曲 | [faction-showcase](../features/faction-showcase.zh-CN.md) |
| 启动器自动更新 | 清单比对、可选与强制更新、自替换 | [launcher-update](../features/launcher-update.zh-CN.md) |
| 多语言 | UI 五语、百科取词、帮助文档五语 | [i18n](../features/i18n.zh-CN.md) |

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
- **便携部署**：数据目录固定为 exe 同级的 `.vcmi-fv-launcher/`（按 exe 路径推导，不用进程 cwd），见 [persistence](./persistence.zh-CN.md)。
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
- 文档分四层：`architecture/`（跨功能全局）、`features/`（一功能一篇）、`guides/`（可照做的步骤）、`reference/`（契约与硬事实），另有 `decisions/` 记录拍板理由、`prototypes/` 存放历史 HTML 原型。

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
