# 远程领域仓库

> 状态：已实现 ｜ 最后核对：2026-09-30 ｜ 代码：`src-tauri/src/repository/`、`src/features/realms/repository/RealmRepositoryPage.tsx`、`src-tauri/src/realms/transfer.rs`
> 索引文件的编写规范见 [repository-index 指南](../guides/repository-index.zh-CN.md)。

## 1. 目标与范围

**做**：从远端索引拉取领域清单，下载安装 `.vcmirealm` 包，并在有更新时提示。

**不做**：托管服务（仓库只是一个静态索引 + 包目录）；上传发布（走手工或脚本写入仓库目录）。

## 2. 常量

| 常量 | 值 |
|---|---|
| `INDEX_TTL_SECS` | 30 分钟 |
| `PACKAGE_MAX_BYTES` | 100 MB |
| `COVER_MAX_BYTES` | 2 MB |

缓存位置：索引 `cache/realmIndex/<urlHash>.json`，封面 `cache/realmCovers/<id>-<version>.<ext>`。

## 3. 索引格式

```jsonc
{
  "urlBase": "...",          // 可选：包与封面的相对基准
  "updatedAt": "...",
  "realms": {
    "<id>": {
      "version": "1.0.0",
      "author": "...",
      "defaultLang": "chinese",
      "name": "...", "description": "...", "tags": [], "i18n": {},
      "requiresMods": [],
      "sha256": "...",       // 远程安装强制校验
      "size": 123456,
      "download": "https://.../xxx.vcmirealm",
      "cover": "...",
      "gameEdition": "enhanced"   // 可选
    }
  }
}
```

## 4. 源管理

命令：`get_realm_repositories` / `add_realm_repository` / `remove_realm_repository` / `test_realm_repository`。源列表存 `launcher.json` 的 `realms.repository_urls`，**支持多源**，按 id 合并时**靠前的源优先**。

## 5. 抓取与状态判定

`fetch_realm_index`：ETag 条件请求；TTL 内零请求；304 只 touch；失败回退缓存；多源按 id 合并。

安装状态五态矩阵（`realm_update_state`）：

| 条件 | 状态 |
|---|---|
| sha256 相同 | installed |
| 索引缺 version | installed |
| 本地版本更低 | updatable |
| 同版本但 sha 不同 | updatable |
| 本地版本更高 | installed |

## 6. 安装流程

`install_remote_realm`：

1. 定位索引条目；
2. 流式下载（200ms 进度节流）+ 体积护栏 + 可取消（`cancel_realm_download`）；
3. **强制 sha256 校验**（不像模组那样允许弱校验）；
4. 复用 `import_realm_impl` 落盘；
5. 写 `origin.{repositoryUrl, version, sha256, installedAt}`。

冲突策略：`"error" | "overwrite" | "rename"`。

## 7. 封面

`ensure_realm_covers` 由**后端**下载并缓存，规避部分站点（如 Gitee）的 Referer 403；会排除 HTML 错误页。

## 8. 界面

`RealmRepositoryPage`：

- 源管理弹窗（add / remove / test）；
- 三组列表：**有更新 / 未安装 / 已安装**（已安装整卡禁用）；
- 关键词 + 多标签 chips 筛选；
- 封面本地化；
- 安装：冲突弹窗 + 进度 + 取消。

## 9. 边界与容错

- 索引抓取失败回退任意年龄缓存；无缓存才报错。
- sha256 不符直接拒装（领域包会改写 `dirs.json`，风险高于模组）。
- 下载可取消；取消不留半成品目录。

## 10. 验证

```bash
npm run realms:index         # 由本地 .vcmirealm 目录生成 / 更新 index.json
npx tauri dev                # 真机：加源 → 测连通 → 拉取 → 安装 → 覆盖安装
```

## 11. 开放问题

- 索引没有签名机制，第三方源的可信度靠用户自行判断。
