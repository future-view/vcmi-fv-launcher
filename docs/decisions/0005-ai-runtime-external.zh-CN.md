# ADR 0005：AI 超分组件外挂，不静态链接

- 状态：已采纳
- 相关：[hd-assets](../features/hd-assets.zh-CN.md) §7

## 背景

AI 超分依赖 ONNX Runtime（ORT）。静态链进 exe 会让体积从 21.7MB 涨到 42.8MB，而启动器更新是**整包替换**，每次更新都要重下 30MB。

## 决策

ORT 作为**外挂组件**放在 `<exe>/.vcmi-fv-launcher/runtime/ai/`，三种分发形态（便携内置 / 安装包内置 / 远程下载）共用同一落点；组件声明在 `launcher.json` 的 `runtime.components`，文件清单由代码常量定义。

## 理由

- 体积与更新成本：组件只装一次、常驻磁盘；
- 语义冲突：`ort` 内部的 `setup_api()` 在 dylib 缺失时会 `panic`（`.expect("Failed to load ONNX Runtime dylib")`），与"AI 失败静默回退"的要求冲突。因此必须先由 `runtime::is_ready` 校验存在性 + sha256，再用 `ort::init_from`（返回 Result）并确保它是**第一个被调用的 ort API**。

## 后果

- AI 永远可选：未安装则回退传统算法，不影响任何其它功能；
- 远程安装与更新流程互斥（非空闲阶段报 `runtime_install_busy`）；
- ORT 1.28 需要 MSVC 14.40+，便携包**不要**附带旧版 VC 运行库（14.29 会导致加载失败，错误码 1114）。
