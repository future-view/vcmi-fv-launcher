# ADR 0007：版本注入 —— 由 `release:pack` 决定本次发行版本

- 状态：**已实施**（2026-10-01）
- 相关：[release](../guides/release.zh-CN.md) §3/§6/§8、[launcher-update](../features/launcher-update.zh-CN.md)、`docs/ide/rules/05-build-run-commands.mdc`

## 背景

三个"版本"此前混在一起：

| 层面 | 谁决定 | 现状 |
|---|---|---|
| 源码版本 | git tag / commit | 自由（`v0.2.0`） |
| **产物版本**（契约版本） | 构建期常量 = `Cargo.toml` = `tauri.conf.json` | 必须**先手工** `version:bump`，再 `release:pack` |
| 发行标识（tag / Release title / 索引键） | `release:pack --version` | 自由，但必须与产物版本一致，否则脚本报错 |

痛点：发版要"改两处、走两步"，且 `--version` 与 `Cargo.toml` 不一致的类别错误只能靠校验拦截。开发者诉求是：**标签（tag）决定源码版本；本次发行版本由 `release:pack` 单点决定；换机器（云主机/他人电脑）从分支或 tag 构建时不必再传参数。**

## 决策

1. **默认持久注入**：`release:pack --version <semver>` 在构建前把该版本写入 `src-tauri/Cargo.toml` + `src-tauri/tauri.conf.json`（**幂等**：值相同不写），写入结果**保留**，随发行一起提交。`--temp-version` 切换为"构建期写入、构建后还原"的零源码改动模式。
2. **不拆参数**：`--version` 同时是契约版本与发行标识（缺省 = 文件里的版本）。发行标识可由索引 `naming` 块的模板生成，CLI(`--version`/`--tag`/`--title`) > 模板 > 内置缺省。
3. **失败/中断**：持久模式**不还原**（版本本就是本次发行要落地的），但打印可复制的撤回命令；临时模式**必须还原**（`finally` + 启动时残留自检）。

## 理由

- **为什么默认持久**：提交后，`git checkout <tag> && npm run release:pack` **零参数**即可产出与维护机一致的产物；tag 也能直接复现该 exe。临时模式不持久化 ⇒ 换机器必须带参数，违背诉求。
- **为什么保留临时模式**：给"不希望打包改动源码"的场景留出口（代价是 tag 不能直接复现，需按发行记录里的参数重跑）。
- **为什么不拆 `--set-version`**：一个旋钮、少的出错面；"只想改展示名"用 `--tag`/`--title` 即可。
- **为什么不能"只改清单里的版本"**：exe 必须自带"我是谁"的版本（更新器以此判定 `current < latest`；便携部署语义是"换 exe 即换版本"，手动替换/回滚场景下外部文件不可信）。清单与 exe 脱节 = 死循环提示或永远收不到更新。

## 不可绕过的约束（实施时必须校验）

- **合法 semver**（Cargo 强制）：`1.0.0` / `2026.10.1` / `0.2.0-rc1` 可用；`v1.0.0`、`2026-10-01` 不可用。
- **严格单调递增**：更新器只在 `current < version` 时提示 ⇒ 同号重发对已装用户无效、版本号不可回退。
- **`Cargo.toml` ↔ `tauri.conf.json` 必须一起改**（Tauri 硬校验，不一致构建直接失败）。
- **prerelease 后缀不能逐轮升级**：更新器把 `-rc1`/`-rc2` 判为**相等** ⇒ 多轮预发行只能每轮递增 patch，或用 `--no-update-chain` 不进更新链。
- `--no-build` 与注入**互斥**（不构建就不需要改版本）。

## 实现记录（2026-10-01）

落地开关：`--version`（定版+注入）、`--temp-version`（临时注入）、`--test`（= 临时注入 + 不进更新链 + 不出安装包 +
自动版本 `<现版本>-dev.<YYYYMMDD>`）、`--no-bundle`、`--allow-downgrade`、`--allow-tag-mismatch`；
`publish --version` 缺省 = 文件里的版本；索引支持可选 `naming.{release,tag,title}` 模板。

实施中发现并修掉的问题（**前两条是既有隐患，不是本次引入**）：

1. **`sync-version.mjs::setCargoVersion` 一直是静默空操作**：正则 `/(^\s*\[package\s*\])([\s\S]*?)(?=\n\s*\[|$)/m`
   在 `/m` 下 `$` 会在第一个行尾即满足，非贪婪量词立刻停下 ⇒ `body` 为空 ⇒ 只打印"版本改了"、文件没变
   （`version:bump` 因此从未真正生效；Cargo.toml 版本自建仓起一直是 0.1.0）。改为**行级扫描**并加断言：
   注入后立刻回读 `Cargo.toml` + `tauri.conf.json`，不一致即 `die`。
2. **`tauri.conf.json` 被 JSON 往返重写**（`["**"]` → 多行）造成无意义 diff ⇒ 改为**定向替换顶层 version 行**，
   保留原格式，改完仍做 `JSON.parse` 校验。
3. **Windows 安装包不支持 prerelease 版本**（实测 WiX 报
   `optional pre-release identifier in app version must be numeric-only and cannot be greater than 65535 for msi target`）
   ⇒ `--test` 默认 `--no-bundle`；带 `-`/`+` 的版本在不加 `--no-bundle` 时**提前拒绝并说明**。
4. `--test`/`--no-update-chain` 缺 ChangeLog 节时**降级为告警**（这类包不进更新链，空说明不影响用户）。
5. 防"测试 exe 当正式包发"：临时注入构建后落标记 `target/<profile>/.fv-test-build.json`，
   `--no-build` 命中标记**直接拒绝**（fail-closed）。
6. 索引 `updatedAt` 只在**内容真变**时刷新（否则每天跑一次 pack 都会产生"只改日期"的 diff）。
7. 便携 zip 的条目顺序按名称排序（`fs.readdirSync` 无顺序保证）⇒ 不同机器/CI 产出的包结构一致。

## 后果

- `scripts/release.mjs`：新增注入/还原、semver 与单调性校验、`naming` 模板展开、`--temp-version`、`publish --version` 缺省 = 文件版本；`scripts/sync-version.mjs` 暴露"写指定版本"的复用入口。
- `releases/index.json`：新增可选 `naming` 块；`verify` 增加"模板合法 + `versions[]` 单调"校验。
- ⚠️ **行为变更**：`--version` 从"纯发行标识"变为"契约版本 + 发行标识"（按"一功能一命令、不做老兼容"处理）。
- 持久模式下 `Cargo.toml` / `Cargo.lock` / `tauri.conf.json` 三处会被改写，**必须随发行提交**，否则 tag 复现不出该 exe。
- 需同步的文档：`guides/release.zh-CN.md` §6、`ide/rules/05-build-run-commands.mdc`（两份副本）、`features/launcher-update.zh-CN.md`（版本语义来源）。
