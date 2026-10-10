# 错误契约与错误码

> 状态：已实现 ｜ 最后核对：2026-09-30 ｜ 代码：`src-tauri/src/infra/berror.rs`、`src/lib/backendError.ts`

## 1. 契约

后端不抛本地化文案，只抛**结构化错误**：

```rust
be_err("some_code", Some("arg"))   // → {"code":"some_code","arg":"arg"}
```

前端 `lib/backendError.ts`：

1. 取 `Error.message` 并尝试解析为 JSON；
2. 命中 → 用 `code` 查当前语言的 `beErr` 词典，**缺译回退中文**，用 `{arg}` 插值；
3. 未命中（非结构化串）→ 原样显示。

**分支判断必须用 `backendErrorCode(e)`，禁止**：

- `String(e)` / `e.message` 直接展示给用户；
- **按译文文本匹配**（多语言下必然失效）。

新增错误码时，后端加码 + 前端各语言词典补文案，两边一起改。

## 2. 常见错误码

| code | 含义 | 出现场景 |
|---|---|---|
| `common_data_dir_unknown` | 数据目录未知 | 未初始化时访问数据 |
| `settings_not_init` | 设置未初始化 | 首启向导未完成 |
| `cmd_settings_parse_abort` | 设置解析失败中止 | `vcmi/setup.rs` 解析失败（禁止降级覆写） |
| `realm_guard_vcmi_running` | VCMI 正在运行，操作被守卫拦截 | 激活 / 停用 / 地图阀门 / 领域模组操作 |
| `id_conflict` | 领域 id 冲突 | 导入领域（附 `realmId` / `realmName`） |
| `repo_url_empty` | 仓库地址为空 | 源管理 |
| `repo_index_not_https` | 索引地址非 https | 仓库校验 |
| `repo_download_failed` | 下载失败 | 仓库安装 |
| `repo_realm_install_sha` | 领域包 sha256 校验失败 | 远程领域安装 |
| `settings_mirror_max` | 镜像规则超过上限（10） | 网络设置 |
| `runtime_unknown_component` | 未知外挂组件 | runtime |
| `runtime_install_busy` | 与更新流程冲突，组件安装忙 | runtime install |
| `upd_launcher_dir_not_writable` | 启动器目录不可写（无法自替换） | 应用更新 |

> 完整清单以后端 `be_err(...)` 调用点为准；本表只列高频项。

## 3. 结构化错误串（另一类）

领域相关错误除了 `be_err` 外，还有一类**结构化错误串** `{code, msg?, realmId?, realmName?}`（如 `id_conflict`），由前端按 `realmId` 做上下文提示。格式已冻结，见 [ipc](../architecture/ipc.zh-CN.md) §4。

## 4. 守卫分级原则

> **共存优先于硬阻断**：只有"数据损坏"才阻断操作；"改动不生效"一类应放行 + 提示。

进程守卫（`realm_guard_vcmi_running`）已按此实现——VCMI 运行时阻断会改 `dirs.json` 的操作，但不阻断只读浏览。
