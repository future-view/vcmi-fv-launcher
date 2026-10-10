# 领域仓库索引编写

> 状态：已实现 ｜ 最后核对：2026-09-30 ｜ 代码：`npm run realms:index`、`src-tauri/src/realms/transfer.rs`、`src-tauri/src/repository/index.rs`
> 消费端行为见 [realms-repository](../features/realms-repository.zh-CN.md)。

## 1. 索引结构

```jsonc
{
  "urlBase": "https://example.com/vcmi-realms/",   // 可选：download / cover 的相对基准
  "updatedAt": "2026-09-30",
  "realms": {
    "<realmId>": {
      "version": "1.0.0",
      "author": "someone",
      "defaultLang": "chinese",
      "name": "领域名",
      "description": "一句话说明",
      "tags": ["剧情", "难度"],
      "i18n": { "english": { "name": "...", "description": "..." } },
      "requiresMods": ["some.mod.id"],
      "sha256": "<整包文件哈希，必填>",
      "size": 12345678,
      "download": "https://.../xxx.vcmirealm",       // 必须 https
      "cover": "https://.../xxx.png",                // ≤2MB
      "gameEdition": "enhanced"                      // 可选：official / enhanced
    }
  }
}
```

**必填与强约束**：`sha256` 必填（远程安装强制校验，不符拒装）；`download` 必须 https；包体积 ≤100MB、封面 ≤2MB。

## 2. 用脚本生成（推荐）

```bash
# 整个目录：扫描目录下所有 *.vcmirealm，一次性生成 / 更新整份 index.json
npm run realms:index -- --dir <含有 .vcmirealm 的目录> --out <index.json>

# 单个包
npm run realms:index -- --pkg <xxx.vcmirealm>

# 带封面（会自动拼 <cover-base>/<id>.png）
npm run realms:index -- --dir <dir> --cover-base <url>

# 只预览不写盘
npm run realms:index -- --dir <dir> --dry-run
```

脚本产出的 `realms` 片段可直接复制进已有 `index.json` 的 `realms` 块（手工合并方式）。

## 3. 发布到仓库目录

编辑器里「发布预览」走 `preview_realm_publish`（只读，返回 `downloadUrl / prevVersion / nextVersion / versionRel(none|up|same|down) / entryExists`），实际发布 `publish_realm_to_repo` 直写仓库：

```
<repo>/realms/<id>/<id>.vcmirealm
<repo>/realms/<id>/<cover>
<repo>/index.json    ← 合并条目（只覆盖 DERIVED_INDEX_FIELDS；空值视为未声明；tags 键存在即声明）
```

`url_base` 推导优先级：**手工覆盖 > 仓库声明 > git 元数据 > 既有条目多数派**。发布前会做 `check_version_downgrade`，阻断降级发布。

配置项：`realms.publish_repo_dir`、`realms.publish_url_base`、`realms.publish_mode`。

## 4. 多源合并

用户可配多个源（`realms.repository_urls`）。索引按 **realm id 合并，靠前的源优先**；同名条目以后不会覆盖靠前源，因此公共源应放在个人源之前。

## 5. 状态判定

本地与索引条目的五态矩阵（决定是否显示「有更新」）：

| 条件 | 状态 |
|---|---|
| sha256 相同 | installed |
| 索引缺 version | installed |
| 本地版本更低 | updatable |
| 同版本但 sha 不同 | updatable |
| 本地版本更高 | installed |

## 6. 注意事项

- 索引**没有签名机制**，第三方源的可信度由用户自行判断。
- 封面建议由后端 `ensure_realm_covers` 下载缓存（规避部分站点的 Referer 403），源站不必做防盗链特殊处理，但**不要返回 HTML 错误页**（会被识别并跳过）。
