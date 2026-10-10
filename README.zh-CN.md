# VCMI-未来视觉-启动器

一个**不修改 VCMI 源码**的独立外部桌面启动器：以最小化基线为起点，保存为 VCMI 原生 `modSettings.json` 预设，并一键启动游戏/地图编辑器。

## 功能

- **模组视图**：左栏搜索/分类/画廊选择，右栏按类型分组已选；依赖自动勾选、冲突标红。
- **预设管理**：可搜索下拉切换、保存（遇变动激活）、更多菜单（新建含继承自 / 重命名 / 导入导出官方格式 / 删除 / Markdown 启用报告）。
- **百科**：9 类目离线浏览（城镇 / 生物 / 英雄 / 建筑 / 宝物 / 法术 / 技能 / 地形 / 模组），支持右键悬停浮层。
- **领域系统**：多套隔离的模组 + 存档 + 设置 + 地图环境，可导出为 `.vcmirealm` 分发，支持远程仓库安装。
- **设置面板**：官方 12 节 schema 加启动器自有设置（AI 超分 / HD 版素材 / 镜像加速 / 更新）。
- **多语言**：启动器 UI 中/英/俄/波/德，模组与百科文本按 VCMI 语言 identifier 取词，内置帮助 5 语。
- **底部区**：多语言下拉、游戏数据抽屉（结构化目录 + 健康检查）、帮助抽屉、地图编辑器入口、启用游戏。
- **目录识别**：读取 `dirs.json` 回退默认规则；启动器读取 VCMI 配置目录写预设。
- **容错**：定制版部分 `Content.zip` 带密码，捕获异常回退色块，绝不崩溃。

## 技术栈

- 前端：React 18 + TypeScript + Tailwind CSS 3.4（Vite 5，玻璃拟态暗色）
- 桌面壳：Tauri 2（Rust），spawn `VCMI_client.exe` / `VCMI_mapeditor.exe`
- zip：TS 端 yauzl；Rust 端 zip

## 开发

```bash
# 安装依赖
npm install

# 纯前端 mock 运行（浏览器）
npm run dev

# 验证真实 modSettings 读写与备份（使用临时副本，不改真实文件）
npm run verify:settings

# Tauri 桌面运行
npx tauri dev

# 打包web版
npm run build

# 打包 Windows exe
npx tauri build
```

## 打包产物

- Windows exe：`src-tauri/target/release/vcmi-fv-launcher.exe`
- （MSI 安装包需联网下载 wix 工具，网络受限时仅生成 exe）

## 配置说明

- **基线 mods**（`src/lib/minimapBase.ts`）：保留 scope（`vcmi`、`core`）加配置中的必需模组。
- **路径识别**（`src-tauri/src/vcmi/paths.rs`）：优先读 `<游戏目录>/config/dirs.json`，无则回退 Windows 默认用户目录。
- **启动/编辑器 exe**：默认在游戏目录（exe 同目录）。

## 设计文档

完整索引见 [docs/README.zh-CN.md](docs/README.zh-CN.md)（中文）/[docs/README.md](docs/README.md)（English）：

- `docs/architecture/` — 总览、前端架构、后端架构、IPC 契约、持久化布局
- `docs/features/` — 按功能域一功能一篇（模组 / 百科 / 领域 / 设置 / 高清 / 更新 …）
- `docs/guides/` — 构建运行、发行、便携部署、发行版定制构建、仓库索引、替换图标
- `docs/reference/` — VCMI 目录机制、错误码、预设格式
- `docs/decisions/` — 架构决策记录（ADR）
