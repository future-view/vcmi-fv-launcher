# 模组获取与更新

> 状态：已实现 ｜ 最后核对：2026-09-30 ｜ 代码：`src-tauri/src/repository/mods.rs`、`src/lib/modRepo.ts`、`src/features/mods/ModManagerPanel.tsx`、`ModSourcePopover.tsx`

## 1. 目标与范围

**做**：从 VCMI 官方模组仓库拉取索引，安装 / 更新 / 删除模组，并把"可更新"状态回显到界面。

**不做**：自建模组仓库协议（复用 VCMI 官方格式）；把安装结果自动写进预设（**只落盘不写预设**，交由用户勾选）。

## 2. 数据源

复用 VCMI 官方设置键，读自 `{user_config}/settings.json`（`repository/mods.rs`）：

| 键 | 默认 |
|---|---|
| `launcher.defaultRepositoryEnabled` | 启用 |
| `launcher.defaultRepositoryURL` | 缺省回退 `OFFICIAL_REPO_FALLBACK` |
| `launcher.extraRepositoryEnabled` | 禁用 |
| `launcher.extraRepositoryURL` | 空 |

前端 `ModSourcePopover` 就地编辑这四个键，`DEFAULT_REPO_URL` 与后端回退值保持一致。

## 3. 索引抓取

- 命令 `fetch_mod_index`；缓存目录 `cache/modIndex/<urlHash>.json`。
- **TTL 30 分钟** + ETag 条件请求；支持 SWR：`force` 强制刷新、`manual` 手动刷新。
- 网络出口统一走 `infra/http.rs`（直连 → 镜像 → 代理），见 [hd-assets 的同源出口说明](./hd-assets.zh-CN.md)之外的网络策略简述：代理仅对 GitHub 生效。

## 4. 版本比较

- 后端收敛到 `infra::version::compare`（逐段数值、缺段补 0、pre-release 更低）。
- 前端 `lib/modRepo.ts` 提供**同口径**的 `compareModVersions` / `isModUpdatable` / `modUpdateMap`，避免前后端判定不一致。
- **子模组不参与更新比较**，只比顶层模组。

## 5. 安装流程

`install_mod_from_repo`：

1. 定位索引条目（支持 `scope: "realm:<id>"` 装进领域）；
2. 流式下载，进度事件 + 可取消（`cancel_mod_download`）；
3. **体积护栏**：`MOD_PACKAGE_MAX_BYTES = 2GB`；包缓存 2GB LRU（`cache/modPackages/`）；
4. 校验分两级：
   - 条目带 `sha256` → **强校验**，不符拒装；
   - 条目无 `sha256` → 弱校验，比对解压后 mod.json 的 name / id 并告警；
5. 只落盘，返回 `replaced` 区分"更新"与"新装"。

**进程守卫**：安装 / 删除前检查 VCMI 是否在运行（只判游戏本体 client / server / mapeditor，官方 `VCMI_launcher.exe` 可共存）。

## 6. 台账与删除

- `list_mod_ledgers`：批量读安装台账（`.fv-install.json`），记录来源与版本。
- `delete_mod`：物理删除模组目录。
- `auto_check_repos_enabled`：读自动检查开关（对应 `launcher.json` 的 `realms.mod_auto_check`）。

## 7. 界面

| 组件 | 能力 |
|---|---|
| `ModManagerPanel` | 状态筛选（all / installed / downloadable / updatable）+ 类型筛选；行级安装队列；打开所在目录；删除；自动检查开关；模组源弹层 |
| `ModRowDrawer` | 行详情：名称 / 类型 / 版本 / 描述 / 作者 / 大小 / stars / 仓库地址 / 最近 15 版 changelog |
| `ModSourcePopover` | 四个仓库键的就地编辑 |

领域内也有同源入口：`features/realms/editor/RealmModManagerPanel.tsx`。

## 8. 边界与容错

- 安装失败不写预设，也不会留下半成品目录（先解到临时目录再就位）。
- 无 `sha256` 的条目仍允许安装但告警，避免第三方源被一刀切拒绝。
- 索引抓取失败时回退任意年龄缓存；无缓存才报错。

## 9. 验证

```bash
npx tauri dev   # 真机：配置额外源 → 拉取索引 → 安装 → 更新 → 删除
```

## 10. 开放问题

- 目前没有"模组依赖自动安装"的批量编排（领域侧有 `solveDepClosure`，全局侧未接通）。
