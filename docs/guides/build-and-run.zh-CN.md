# 构建、运行与验证

> 状态：已实现 ｜ 最后核对：2026-09-30 ｜ 代码：`package.json`、`scripts/`

## 1. 常用命令

```bash
npm install          # 安装依赖

npm run dev          # 纯前端 mock 运行（浏览器，不走 Tauri）
npm run build        # tsc && vite build（类型检查 + 打包）
npm run preview      # 预览构建产物

npx tauri dev        # Tauri 桌面运行（真机）
npx tauri build      # 打包 Windows exe

npm run verify:settings       # 验证真实 modSettings 读写与备份（临时副本，不改真实文件）
npm run check:invoke-parity   # IPC 命令三方一致性（改后端必跑）
npm run verify:wiki-cache     # 百科缓存校验（退出码 0 = 通过）
npm run rebuild:wiki-cache    # 百科缓存全量重建
```

## 2. 全量脚本清单

| 脚本 | 用途 |
|---|---|
| `dev` / `build` / `preview` / `tauri` | 前端开发常规流程 |
| `dev:tauri` / `build:debug` / `build:console` / `build:console:release` | 桌面运行与带控制台的调试构建 |
| `version:sync` / `version:bump` | 版本号同步与递增 |
| `release:pack` / `release:publish` / `release:verify` / `release:status` | 发行流程，见 [release](./release.zh-CN.md) |
| `verify:settings` / `verify:wiki-cache` / `rebuild:wiki-cache` / `verify:html-text` / `verify:maps` / `test:maps` | 各类校验 |
| `check:invoke-parity` | IPC 契约校验 |
| `gen:settings` / `gen:setting-keys` | 从官方 schema 生成设置默认值与键常量 |
| `realms:index` | 生成 / 更新领域仓库 index.json，见 [repository-index](./repository-index.zh-CN.md) |
| `hd:preview` / `hd:ai` | 高清放大与 AI 超分实验 |

## 3. 验证闭环

改动后按影响面跑对应校验，**不要只跑 build**：

| 改动面 | 必跑 |
|---|---|
| 逻辑层（`src/lib/**`） | `npm run build` + 对应 verify 脚本 |
| 后端命令 | `npm run check:invoke-parity` + `npx tauri dev` 真机 |
| 百科缓存 | `npm run verify:wiki-cache` |
| 设置 schema | `npm run gen:settings` + `npm run build` |
| 发行相关 | `npm run release:verify` |
