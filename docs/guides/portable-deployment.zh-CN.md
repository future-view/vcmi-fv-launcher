# 便携部署

> 状态：已实现 ｜ 最后核对：2026-10-12 ｜ 代码：`src-tauri/src/vcmi/paths.rs`、`src-tauri/tauri.conf.json`、`tauri.ai-runtime.conf.json`

## 1. 机制

数据目录由 `launcher_data_dir()` **按平台分派**（ADR 0008 D10）：

| 平台 | 数据根 | 说明 |
|---|---|---|
| Windows | **exe 同级的 `.vcmi-fv-launcher/`** | 便携语义的核心：按 exe 路径推导、**不用进程 cwd**，从任意工作目录启动都落在同一处，可直接整个文件夹拷走 |
| macOS | `~/Library/Application Support/com.vcmi.fv-launcher/` | 系统标准位置（`.app` 装进 `/Applications` 后包内不可写） |
| Linux | `${XDG_DATA_HOME:-~/.local/share}/vcmi-fv-launcher/` | XDG 规范（AppImage 挂载只读 squashfs） |

本文其余部分以 **Windows 便携形态**为主线（当前唯一日常发行平台）。

领域目录 = `<data>/realms/<id>/vcmi`，随数据目录一起走。

## 2. 随包预置

`tauri.conf.json` 的 `bundle.resources` 把随包文件映射进数据目录：

| 源 | 目标 |
|---|---|
| `resources/release-preset/launcher.patch.json` | `.vcmi-fv-launcher/launcher.patch.json` |
| `data/h3-core` | 原版文案表（供 core 伪模组） |
| `resources/vcmi-schemas` | VCMI schema |
| `resources/mod-cache-protocol.json` | 提取协议 |

`launcher.patch.json` 在首次启动时被 `config_patch` 合并进用户 `launcher.json`（合并后删除，天然幂等），用来预置语言、更新清单地址、高清开关、组件地址等。

`src-tauri/build.rs` 只调用 `tauri_build::build()`，**无便携相关定制**——便携语义全在 `paths.rs` + `bundle.resources` 映射。

## 3. AI 版变体

```bash
npx tauri build -c tauri.ai-runtime.conf.json
```

该变体：targets 限 `nsis`，额外把 `resources/runtime/ai` 映射到 `.vcmi-fv-launcher/runtime/ai`，并用 `launcher.patch.ai.json` 覆盖补丁（预置 `hd.enhance=true` 与 `hd.ai.{enhance,model}`）。

## 4. 便携包内容边界

便携 zip **只含程序文件**（exe + 运行时组件 + 预置补丁），**绝不包含** `.vcmi-fv-launcher/` 下的用户数据——把用户数据打进包会导致升级时覆盖。

## 5. 实测踩坑

1. **部署目录里 51 个 dll 带隐藏属性**：某些便携部署目录（如 `C:\Games\...`）下的 dll 被设了隐藏属性，按常规方式枚举会漏文件；处理部署目录时要注意枚举方式。
2. **自带 VC 运行库 14.29 会让 ONNX Runtime 1.28 加载失败（错误码 1114）**：ORT 1.28 需要 MSVC 14.40+，代码里用 `pin_vc_runtime` 处理。AI 版便携包不要附带旧版 VC 运行库。

## 6. 验证

```bash
npx tauri build                                  # 生成 exe
npm run release:pack -- --version <ver>          # 产出便携 zip
# 把 zip 解开到新目录，运行 exe，确认数据目录生成在 exe 同级、预置项生效
```
