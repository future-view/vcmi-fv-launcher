# 启动器自动更新

> 状态：已实现 ｜ 最后核对：2026-10-09 ｜ 代码：`src-tauri/src/updater/`、`commands/updater.rs`、`src/lib/updater.ts`、`src/features/settings/updateFlow.ts`
> 发行流程与 ChangeLog 见 [release 指南](../guides/release.zh-CN.md)。

## 1. 目标与范围

**做**：静默检查新版本、可选更新提示、强制更新阻断、下载校验、自替换。

**不做**：增量更新（更新是**整包替换**，这正是外挂组件存在的原因，见 [hd-assets](./hd-assets.zh-CN.md) §7）。

## 2. 三段流水线

```
manifest（清单比对）  →  download（下载校验）  →  apply（自替换）
```

`updater/` 内**禁止出现 tauri 类型**，事件 emit 只在 `commands/updater.rs`。

## 3. 清单（manifest）

- 结构：`LauncherManifest{latest, minSupported, versions[]}`；条目 `ManifestEntry{version, download, sha256, size, publishedAt, critical, notes{i18n}, format(exe|zip), kind(full|config)}`。
- `download` 必须 https、`sha256` 必填。
- 缓存 `cache/updateManifest/manifest.json`，**TTL 24 小时** + ETag（304 复用）；拉取失败用任意年龄缓存兜底，无缓存才报错。
- semver：`parse_ver` + `semver_cmp`（逐段数值、缺段补 0、pre-release 更低）。

**强制 vs 可选**：

| 判定 | 结果 |
|---|---|
| `current < minSupported` | `forced = true`（可由任意年龄缓存判定，**离线也能阻断**） |
| 条目标记 `critical` | 清单侧的紧急标记 |
| 其余 | 可选更新 |

**7 天节流**：`PROMPT_INTERVAL_SECS`，配 `update.prompt_version` / `update.prompt_at`——同一版本 7 天内不再自动弹窗（设置红点等被动感知不受限）。`update.skip_version` 是永久忽略该版本（**强制更新不受其影响**）。出口 `UpdateInfo` 含 `should_prompt`。

> ⚠️ **节流必须认 exe 版本 + 已应用版本**：`prompt_version` / `prompt_at` 只记**目标版本号**，不记"当时是哪个 exe 版本"；
> 仅认 `prompt_exe_version == CURRENT_VERSION` 会在**回环**场景失效：已升到 0.2.0（期间弹过窗）→ 手工换回 0.1.0
> → `prompt_version` 仍是 0.2.0、`prompt_exe_version` 又等于 0.1.0 且未满 7 天 → 误判"未过期"。
> 故额外用 `appliedUpdateVersion`（exe 应用后记录**目标版本**）识别「应用后回退」：`appliedUpdateVersion != CURRENT_VERSION` ⇒
> 视为回退，节流重新生效、重新提示。exe 更新在 `apply_update` 内写 `appliedUpdateVersion`（非 CURRENT，因替换尚未发生）。
>
> 故配套记录 `prompt_exe_version`：**节流只在 `prompt_exe_version == CURRENT_VERSION` 时生效**。
> exe 版本一变（升级 / 降级 / 手工换包）→ 旧节流记录自动失效、重新评估；
> 存量配置没有该字段 → 同样视为失效，**天然自愈**（无需额外迁移）。
>
> ⚠️ **不要**用"检测到 `lastRunVersion` 变化就清空记录"这种做法：版本切换的"过渡"可能先被
> **不含该逻辑的旧构建**消费掉，新构建此后永远看到 `last == cur`，重置再也不会触发（已实测踩过）。
>
> `skip_version` 语义是「永久忽略该版本」（换包后仍然有效），不参与这套失效 ——
> 且手动「检查更新」(`force`) 恒忽略 skip，用户始终有出路。

## 4. 下载

- 流式写盘 + sha256 强校验 + 进度回调 + 取消哨兵；硬上限 `UPDATE_MAX_BYTES = 200MB`。
- zip 落 `VCMI-FV-Launcher_<ver>.zip`，exe 落 `.exe.update`（同版本可复用免重下）。
- **解压白名单** `allowed_rel`：主 exe、`.vcmi-fv-launcher/{runtime,mods,realms}/`、配置补丁；**拒绝** `launcher.json`、`realms/`、`cache/`、`downloads/`、`mods/`（用户数据）。

## 5. 应用（自替换）

1. **可写性探测**：写 `.vcmi-fv-write-probe`；不可写报 `upd_launcher_dir_not_writable`，走手动替换降级。
2. 生成**纯 ASCII** 的 `update-apply.ps1`（`apply.rs` 的 `APPLY_PS1` 常量），由 `powershell.exe -File` 静默承载；
   **路径只经命令行参数传入**（Rust `.arg()` 负责引号），脚本体不含任何路径 → 非 ASCII 安装目录同样安全。
   - 参数顺序：`<TargetPid> <Staged> <InstalledExe> <LogPath>`。
   - **`Staged` 的含义随形态而变，务必对应**：单 exe 形态 = 暂存的 `.exe.update` **文件**（脚本把它移动到原位）；
     **zip 多文件形态 = 已解压的暂存目录** `downloads/staged_<ver>/`（脚本递归逐文件移动）。
   - ⚠️ 脚本体**必须纯 ASCII**：PowerShell 5.1 对无 BOM 的 `.ps1` 按 ANSI（中文 Windows = GBK）解码，
     非 ASCII 会乱码，甚至**把下一行代码吞进注释**。单测 `apply_ps1_must_be_ascii` 守这条。
   - ⚠️ PS 5.1 **不支持** `Split-Path -LiteralPath X -Leaf/-Parent`（`AmbiguousParameterSet`，静默返回空串）→
     取父目录 / 文件名必须用 `[System.IO.Path]::GetDirectoryName` / `GetFileName`。
3. `CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP` detached spawn：**不闪任何控制台窗口**，
   且脚本在启动器进程退出后仍存活（已实测）。
4. Windows 不能覆盖运行中的 exe 但**可以重命名** → `Move-Item 旧 → .old` + `Move-Item 新 → 原位`（不用覆盖式拷贝）。
5. **必须原位替换**：数据目录在 exe 同级，换目录会丢数据。
6. 脚本先按 PID 等进程退出（`WaitForExit(30s)`，超时则 `Stop-Process -Force`），再替换、
   最后 `Start-Process` 拉起新版本；主程序排在文件列表**最后**替换，失败回滚并**保留暂存包**便于重试。
7. 每次尝试写日志 `<data>/update-apply.log` —— 全程无窗口，日志是唯一现场；启动清理会删残留 `.ps1`。

⚠️ **不要退回 `cmd` / 批处理**（历史教训）：`cmd /C` + `start` + 批处理会同时踩「引号剥离、`%~dp1` 字符串拼接、
ANSI 代码页」三重坑，且 `start` 会闪出黑框并报「文件名、目录名或卷标语法不正确。」，
表现为**窗口关了但新启动器从未被拉起**。

成功后前端**延迟 800ms 关窗**；`apply` 函数本身绝不调 `app.exit`。

## 6. 配置补丁与删除清单

| 机制 | 说明 |
|---|---|
| `config_patch` | 一次性把更新包里的 `launcher.patch.json` 合并进用户 `launcher.json` 后删除（天然幂等）。对象递归合并（未提及的键保留）/ 数组与标量以补丁为准；仅在确有改动时写回并备份 `launcher.json.bak`。**必须早于日志初始化执行** |
| `delete_patch` | `launcher.delete.json` 补上"更新只能覆盖不能删除"的缺口。白名单前缀只允许 `.vcmi-fv-launcher/{realms,mods,runtime}/` 与 `runtime/`；不许删白名单根；含 `..` 拒绝；**junction 只能 `remove_dir`**；单项跳过而非整包拒绝 |

## 7. 迁移（migrate）

- `MIGRATION_FLOOR = "0.1.0"`：低于门槛不重放历史迁移，直接清空派生数据。
- 原则：**能靠 bump `WIKI_CACHE_VERSION` / `HD_SCHEME_VERSION` / `ASSETS_VERSION` 解决的绝不写迁移**；迁移必须幂等且不阻断启动。
- `state.json` 按 **sha256 而非版本号**记录已应用（纯配置包不改 exe 版本）。

## 8. 前端

| 模块 | 职责 |
|---|---|
| `lib/updater.ts` | 无副作用的展示逻辑：`pickNotes`（按 uiLang 取说明，english 兜底）、`formatBytes`、`progressPercent` |
| `features/settings/updateFlow.ts` | 共用状态机 hook：`idle → downloading → downloaded → applying → restarting / applied / error`；`autoApply=true` 时下载完即应用；`restarted===false`（纯配置包）不关窗 |
| `UpdateDialog` | 可选更新弹窗，可稍后 / 跳过此版本；关闭不中断下载，重开从后端阶段恢复 |
| `ForcedUpdateOverlay` | 全屏阻断，只有「立即更新」与「退出」 |

### 待应用（跨会话未竟事务）与自动应用

下载完成即写入 `state.json::pendingApplyVersion`，**与是否点过「重启并应用」无关**。该标记表示"用户已表达升级意图"，驱动两类自动应用：

1. **纯配置包（`kind=config`，应用后无需重启）**：下次启动静默就地应用（`App.tsx::checkUpdateSilently` 在启动期调用 `applyLauncherUpdate`，走 `apply_inplace`），无需任何用户操作；应用成功即清 `pending` 与 `applied` 记录。
   - 若启动期应用失败（个别文件被占用），回退为启动期 NoticeBar，用户手动点击兜底。
2. **含 exe 的更新包**：不自动应用，但**关闭启动器时自动完成替换**——
   `close_window` 命令 / `lib.rs::on_window_event(CloseRequested)` 在关闭前检查 `should_auto_apply_on_close()`（阶段 `Downloaded` + `pending` 匹配 + 暂存就绪 + 含 exe），满足则 spawn 替换脚本后照常关闭；脚本等本进程退出即原地替换。
   - **关键区别（按场景）**：`apply_update(relaunch)` 的 `relaunch` 开关决定替换后是否拉起新版本：
     - `重启并应用`（手动）：`relaunch=true` → 替换**并重新拉起** 0.2.0（前端随后 `close_window` 退出旧进程）；
     - `直接关闭`（关闭时自动应用）：`relaunch=false` → **只替换二进制、不拉起**，用户下次手动打开才是新版。
   - 判定用**包内容**（`staged_has_exe`）而非 `kind`，避免 kind 标错导致对纯配置包误走 exe 替换路径。
   - 失败（如目录不可写）只记日志、照常关闭，`pending` 保留，下次启动仍提示。

两种自动应用都依赖"下载完成即记 `pending`"这一前置；「跳过此版本」会清掉 `pending`，因此不再触发自动应用。

## 9. 边界与容错

- 更新包与用户数据严格隔离（白名单双向生效）。
- 自替换失败可手动替换：exe 与数据目录同级，用户可自行拷贝。
- 强制更新由任意年龄缓存判定，保证离线也能阻断过旧版本。

## 10. 验证

```bash
npm run release:pack -- --version <ver>   # 本地发包
npm run release:verify                    # 索引契约自检
npx tauri dev                             # 真机：改 manifest_url 指向本地清单验证三段流程
```

## 11. 开放问题

- 更新包目前是整包替换，未来若要减小体积需引入差分更新。
