# 文档索引（中文）

> 中文文档用 `.zh-CN.md`，英文用 `.md`。本文件是 `docs/` 的入口，按功能查表即可。
> 正文以**代码实测**为准，代码与文档冲突时改文档。每篇头部标注状态与最后核对日期。

## 架构（跨功能）

> 入门先读 **overview → persistence → ipc** 三篇（项目定位 / 磁盘上到底写了什么 / 前后端接口契约），其余按需查。

| 文档 | 一句话 |
|---|---|
| [overview](./architecture/overview.zh-CN.md) | 定位、能力全景、技术栈、目录结构、文档约定 |
| [frontend](./architecture/frontend.zh-CN.md) | 前端四层、目录归属判定、依赖方向、启动编排 |
| [backend](./architecture/backend.zh-CN.md) | Rust 分层、infra 模块清单、并发节流、禁改清单 |
| [ipc](./architecture/ipc.zh-CN.md) | 136 个命令清单、事件、三方 parity、冻结契约 |
| [persistence](./architecture/persistence.zh-CN.md) | 数据目录、`launcher.json` 分组、领域 / 缓存 / 日志布局 |

## 功能

| 文档 | 一句话 |
|---|---|
| [shell-and-navigation](./features/shell-and-navigation.zh-CN.md) | 顶栏 / 底栏 / 游戏数据 / 帮助 / 窗口与渲染优先级 |
| [startup-responsiveness](./features/startup-responsiveness.zh-CN.md) | 首启模组页卡死：主线程阻塞根因与分级整改（2026-10-08 已实施两轮） |
| [setup-wizard](./features/setup-wizard.zh-CN.md) | 首启向导：触发条件、步骤、目录识别 |
| [mods](./features/mods.zh-CN.md) | 模组浏览与勾选：依赖拉齐、冲突标红、城镇幕布 |
| [presets](./features/presets.zh-CN.md) | 预设管理：原生格式互转、导入导出、启用报告 |
| [lang-mods](./features/lang-mods.zh-CN.md) | 语言模组按分类发现（不按名字）+ 启动前检查弹窗 |
| [mods-repository](./features/mods-repository.zh-CN.md) | 模组获取与更新：官方仓库索引、安装、台账 |
| [game-import](./features/game-import.zh-CN.md) | 游戏文件导入：分类规则、冲突策略、四态弹层 |
| [wiki](./features/wiki.zh-CN.md) | 百科页面：9 类目、详情分发、右键悬停浮层 |
| [modcache](./features/modcache.zh-CN.md) | 百科缓存管线：两阶段、产物、缓存键、领域加急 |
| [encrypted-zip-handling](./features/encrypted-zip-handling.zh-CN.md) | 加密 `content.zip` 密码解包与发行版喂密码两路线 |
| [faction-showcase](./features/faction-showcase.zh-CN.md) | 阵营巡礼：牌组编排、播放规格、音频来源 |
| [settings](./features/settings.zh-CN.md) | 设置面板：13 分组、作用域徽标、启动器自有卡 |
| [hd-assets](./features/hd-assets.zh-CN.md) | 高清三链路：HD pak / 传统放大 / AI 超分 + 外挂组件 |
| [network-egress](./features/network-egress.zh-CN.md) | 网络出口：直连 → 镜像 → 代理的回退策略 |
| [realms](./features/realms.zh-CN.md) | 领域系统：生命周期、隔离、地图阀门、自检、导入导出 |
| [realms-editor](./features/realms-editor.zh-CN.md) | 领域编辑器六标签页与保存语义 |
| [realms-repository](./features/realms-repository.zh-CN.md) | 远程领域仓库：索引、状态矩阵、安装校验 |
| [realms-sharded-package](./features/realms-sharded-package.zh-CN.md) | 领域包分片分发：>45 MiB 自动切分、`parts` 索引契约、下载拼回（已实施，真机待验） |
| [launcher-update](./features/launcher-update.zh-CN.md) | 自动更新：清单、强制 / 可选、自替换、配置补丁 |
| [i18n](./features/i18n.zh-CN.md) | 三套语言体系：界面 / VCMI 内容 / 帮助文档 |

## 指南（可照做）

| 文档 | 一句话 |
|---|---|
| [build-and-run](./guides/build-and-run.zh-CN.md) | 安装、运行、打包与各类校验脚本 |
| [release](./guides/release.zh-CN.md) | ChangeLog 语法、本地发包、发布到 GitHub、`index.json`、本地预演、端到端流程与验收清单 |
| [release-cross-platform](./guides/release-cross-platform.zh-CN.md) | 跨平台发行与更新实施计划（ADR 0008 配套）：平台描述符、per-target 清单、`updateMode` 分派、新用法演示 |
| [portable-deployment](./guides/portable-deployment.zh-CN.md) | 便携目录机制、随包预置、AI 版变体、实测踩坑 |
| [distribution-build](./guides/distribution-build.zh-CN.md) | 发行版定制构建：加密 zip 密码内置（路线 B）与 `launcher.json` 配置（路线 A） |
| [repository-index](./guides/repository-index.zh-CN.md) | 领域仓库 `index.json` 编写与发布 |
| [replace-icon](./guides/replace-icon.zh-CN.md) | 需要替换的图标文件清单 |

## 参考（查字典）

| 文档 | 一句话 |
|---|---|
| [vcmi-dirs](./reference/vcmi-dirs.zh-CN.md) | VCMI 目录机制硬事实（`dirs.json` 5 key、双重挂载、junction） |
| [error-codes](./reference/error-codes.zh-CN.md) | 错误契约 `be_err(code,arg)`、常见错误码、守卫分级 |
| [mod-settings-format](./reference/mod-settings-format.zh-CN.md) | `modSettings.json` 与预设格式 |

## 决策记录（ADR）

| 文档 | 决策 |
|---|---|
| [0001](./decisions/0001-realm-isolation-via-dirs-json.zh-CN.md) | 领域隔离靠改写 `dirs.json` 的 5 个 key |
| [0002](./decisions/0002-maps-junction-valve.zh-CN.md) | 地图屏蔽用 junction 阀门，懒执行 |
| [0003](./decisions/0003-town-backdrop-single-source.zh-CN.md) | 城镇幕布以 `build_towns` 产物为唯一来源 |
| [0004](./decisions/0004-modcache-index-no-prune.zh-CN.md) | 百科缓存索引只合并不删除 |
| [0005](./decisions/0005-ai-runtime-external.zh-CN.md) | AI 超分组件外挂，不静态链接 |
| [0006](./decisions/0006-guard-coexist-not-block.zh-CN.md) | 守卫分级：共存优先于硬阻断 |
| [0007](./decisions/0007-version-injection-at-pack.zh-CN.md) | 发行版本由 `release:pack --version` 单点注入 |
| [0008](./decisions/0008-cross-platform-release-and-update.zh-CN.md) | 跨平台分发与更新：平台描述符 + per-target 清单 + `updateMode` 分派（2026-10-12 已确认并实施） |

## 开发规则（给 AI / 协作者）

规则真相源在 `docs/ide/rules/`（`NN-<主题>.mdc`），**与具体 IDE 无关**。使用不同 IDE 的开发者从这里把规则拷贝到自己 IDE 的规则目录即可；拷贝产物不在仓库维护范围内，改规则一律改真相源。
