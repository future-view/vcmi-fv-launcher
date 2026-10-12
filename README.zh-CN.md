# VCMI-未来视觉-启动器

一个**兼容 VCMI 配置**的独立外部桌面启动器，采用「未来视觉」界面风格。

更好的模组管理面板（支持可视化挑选城镇模组）。

更好的游戏设置面板（支持分级别查看VCMI配置）。

提供**领域**系统，实现把（模组组合 + 存档 + 设置 + 地图）整体隔离成多套互不干扰的游戏环境，可打包分发给他人。

提供**百科**系统，无需运行游戏即可查看游戏图鉴与资料。

## 功能

### 领域系统

实现多套隔离的（模组组合 + 存档 + 设置 + 地图）游戏环境。

#### 领域登场演出

运行启动器就进入领域的登场演出（壁纸 / 视频 / 主题曲 / 剧情描述），提供更好的剧情体验。

![领域主页面](./static/img/screenshots/realm_vcmi-realm-main.webp)

#### 阵营巡礼

进入领域后可播放一段全屏轮播演出，依次展示各阵营的幕布、生物巢穴、英雄与主题曲。

![阵营巡礼页面](./static/img/screenshots/realm_vcmi-realm-faction-showcase.webp)

#### 本地领域切换

可以切换本地已安装的领域，也可以直接导入其他人分发`.vcmirealm`的领域。

![切换领域页面](./static/img/screenshots/realm_switch-realm.webp)

#### 远程领域仓库

像订阅模组一样订阅别人做好的领域，一键下载安装，有更新会提示。

![远程领域仓库列表页面](./static/img/screenshots/realm_online-realm.webp)

#### 领域编辑器

像编辑一份「游戏方案」一样定制你的领域，然后打包发布。

- 基本信息：多语言名称、描述、标签、作者、版本
- 登场演出：壁纸、封面、主题曲、介绍视频 / 音乐、时长、是否可跳过
- 模组组合：复用主界面的勾选方式，自动求解依赖、一键补齐
- 地图管理：引入、上传、移动、删除，按语言过滤
- 阵营巡礼配置：自动编排或手动编排牌组
- 高级选项：五个目录可单独设为「跟随全局」或「领域独立」，配置文件可选共享或私有
- 导出为 `.vcmirealm` 包
- 预览当前领域

![编辑领域页面](./static/img/screenshots/realm_edit-realm-intro.webp)

### 模组视图

#### 模组预设

把一套满意的模组组合存成预设，随时一键切换，也可分享给他人。

![模组预设主页面](./static/img/screenshots/mods_main.webp)

支持显示各个城镇背景，提高模组预设配置的友好度。

![模组预设主页面-城镇分类](./static/img/screenshots/mods_main-town.webp)

#### 模组管理

直接从 VCMI 官方模组仓库浏览、安装、更新、卸载模组，不再需要手动下载解压。

![模组管理页面-已安装](./static/img/screenshots/mods_manager-installed.webp)

### 百科

9 类目离线浏览（城镇 / 生物 / 英雄 / 建筑 / 宝物 / 法术 / 技能 / 地形 / 模组），支持右键悬停浮层。

- 按中文名、英文名、条目编号搜索
- 每类目有定制呈现：阵营含幕布与生物 / 建筑 / 英雄总览，生物带战斗动画，英雄带传记与特长
- 鼠标悬停任意条目名即弹出资料浮层，条目之间可互相跳转
- 资料随模组变动自动更新，全程离线可用

![百科页面-城镇](./static/img/screenshots/wiki_factions.webp)

**高清素材**：百科图片与城镇幕布会自动以更清晰的方式呈现。支持`官方高清版素材`、`图片高清放大`、`AI超分放大`。

### 设置面板

完整收录 VCMI 官方设置项，以卡片呈现并附带作用说明，改完立即生效。

- 多个配置分组：常规、视频、冒险地图、战斗、游戏调整、AI、输入、启动器、模组、服务器、联机、日志，以及启动器自身设置
- 三档显示等级：常用 / 高级 / 全部，可开关详细描述
- 顶部搜索快速定位设置项
- 设置显示「全局共享 / 仅本领域」徽标

![设置面板页面-全部](./static/img/screenshots/settings_all.webp)

### 其他能力

#### 多语言

界面、百科条目、内置帮助都跟随你选择的语言。

- 界面支持多种语言
- 百科条目按当前语言取词，缺失自动回退英文
- 切换界面语言即同步游戏内语言
- 切换语言后，对应的语言包会被自动识别并提示启用

#### 游戏文件导入

把散落的地图、存档、模组压缩包、高清素材直接拖进窗口，启动器自动判断类型并放到正确位置。

![导入游戏文件页面](./static/img/screenshots/common_import-game-files.webp)

#### 首次启动向导

第一次打开时引导你完成必要的准备：选择界面语言、定位`英雄无敌三`的游戏目录（自动探测，失败可手动选择）、确认用户数据目录，校验通过后即可进入主界面。

#### 启动器自动更新

新版发布后自动检查并提示，更新过程自动完成，不影响你的数据。

#### 便携分发

任何发行人员可以通过预设`launcher.json`相关配置，实现更多**便携运行**效果。

- 预设`paths.*`路径配置（支持相对路径），实现跳过`首次启动向导`。
- 预设`realms.active`实现内置默认领域，实现首次打开就进行领域登场演出。
- 预设`realms.repository_urls`实现内置领域仓库。
- 预设`network.*`实现网络代理，提供不同网络环境下的访问成功率。

## 技术栈

- 前端：React 18 + TypeScript + Tailwind CSS 3.4（Vite 5，玻璃拟态暗色）
- 桌面壳：Tauri 2（Rust），spawn `VCMI_client.exe` / `VCMI_mapeditor.exe`
- zip：TS 端 yauzl；Rust 端 zip

## 开发

```bash
# 安装依赖
npm install

# 纯前端 mock 运行（浏览器，不走 Tauri）
npm run dev

# 类型检查 + 打包 web 版
npm run build

# 预览构建产物（需先 build）
npm run preview

# Tauri 桌面运行（真机）
npx tauri dev

# 打包 Windows exe
npx tauri build
```

> 更多命令（校验、调试构建、百科缓存、版本/发行等）见 [docs/guides/build-and-run.zh-CN.md](docs/guides/build-and-run.zh-CN.md)。

## 打包产物

- Windows exe：`src-tauri/target/release/vcmi-fv-launcher.exe`
- Windows msi: `src-tauri/target/release/bundle/msi/*.exe`
- Windows nsis: `src-tauri/target/release/bundle/nsis/*.exe`

## 设计文档

**完整索引:**

- [docs/README.zh-CN.md](docs/README.zh-CN.md)（中文）
- [docs/README.md](docs/README.md)（English）

**文档类型:**

- `docs/architecture/` — 总览、前端架构、后端架构、IPC 契约、持久化布局
- `docs/features/` — 按功能域一功能一篇（模组 / 百科 / 领域 / 设置 / 高清 / 更新 …）
- `docs/guides/` — 构建运行、发行、便携部署、发行版定制构建、仓库索引、替换图标
- `docs/reference/` — VCMI 目录机制、错误码、预设格式
- `docs/decisions/` — 架构决策记录（ADR）
