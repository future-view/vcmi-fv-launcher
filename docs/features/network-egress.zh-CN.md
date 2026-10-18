# 网络出口（代理与镜像）

> 状态：已实现 ｜ 最后核对：2026-09-30 ｜ 代码：`src-tauri/src/infra/http.rs`、`commands/settings.rs`、`src/features/settings/MirrorRulesCard.tsx`

## 1. 目标与范围

**做**：为所有出站请求提供统一出口，在国内网络环境下用镜像与代理提升可达性。

**不做**：SOCKS 代理（reqwest 0.12+ 已移除）；对非 GitHub 主机应用代理。

## 2. 出口策略

默认**直连**；遇连接类失败按序回退：

```
直连  →  逐条镜像规则（声明序 = 优先级，L2）  →  代理（L1，仅 http/https、仅 GitHub）
```

- 镜像返回 **403 / 429 / 5xx** = 镜像坏 → 推进下一条；**404** = 资源真不存在 → 停止。
- 代理仅支持 http/https，且只对 GitHub 主机参与（`is_github_url`：github.com / githubusercontent.com 及子域）；镜像规则的 `hosts` 可填任意主机。
- **出口粘性** `Sticky`：一轮内命中后不再白等直连；直连冷却 30 分钟；`reset_sticky()` 每轮重置。
- 成功时返回 `Egress::{Direct, Mirror{host}, Proxy}`，供日志指名来源。

## 3. 配置

| 命令 | 行为 |
|---|---|
| `set_network_proxy(proxy)` | 归一化：`None` / 空 / `"off"` = 直连；非 http(s) 丢弃；改动即时更新进程级 `PROXY` 并 `reset_sticky` |
| `set_network_mirror_rules(rules)` | 上限 10 条，逐条 `normalize_rule` 预校验，超限报 `settings_mirror_max` |

`MirrorRule` 字段：

| 字段 | 说明 |
|---|---|
| `enabled` | 默认 true，可临时停用而不删除 |
| `note` | 备忘 / 展示 |
| `hosts` | 精确域名或子域匹配 |
| `mode` | `"prefix"`（拼原 URL）/ `"template"`（占位换形） |
| `prefix` / `template` | 模板占位符 `{owner} {repo} {ref} {path} {url} {host}`，需 http(s) 且至少一个占位符 |

默认**空**，不内置任何镜像。

### 3.1 首启向导的「一键 GitHub 镜像」

镜像域名会腐化（N10），因此**不内置任何默认规则**；但首启阶段新用户不知道该填什么，
故在向导里提供**一个开关**：

- 判据：`check_launcher_update(probe=true)` 的短超时直连探测（`infra::http::probe_url`，
  **连接 5s / 总 8s、不走回退链**）失败，且清单地址属 GitHub 主机；
- 开关 = 追加一条 prefix 规则：hosts = GitHub 全家桶 5 域，前缀默认
  `https://v4.gh-proxy.org/`（常量与设置页预填同源：`src/lib/mirrorRules.ts`）；
- 失败不影响正确性：镜像坏掉（403/429/5xx）由回退链推进到下一个出口，不会比不开更糟；
- 展示受界面语言能力表 `UI_LANG_FEATURES.githubMirrorHint` 约束（**探测始终执行**，
  只有建议卡片受控），新增语言必须在该表补一行。

细节见 [首启向导](./setup-wizard.zh-CN.md) §5。

## 4. 哪些功能走网络

启动器更新、领域仓库、模组仓库、外挂组件远程安装——全部经 `infra::http`。

**不走网络**：AI 模型校验是本地 sha256 扫描（读组件目录）。

## 5. 边界与容错

- 必须**跟随 302**（很多下载直链是跳转）。
- **禁止**按 Content-Type 判断成功（部分镜像返回 HTML 错误页但状态码 200）；领域封面下载会显式排除 HTML 错误页。
- 索引类请求失败时回退任意年龄缓存，无缓存才报错。

## 6. 验证

```bash
npx tauri dev   # 真机：设置页添加镜像规则 → 观察实时预览 → 断网验证回退与报错
```

## 7. 开放问题

- 代理仅覆盖 GitHub；若未来需要代理其它主机，需放宽 `is_github_url` 并评估鉴权泄露风险。
