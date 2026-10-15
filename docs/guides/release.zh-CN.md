# 发行流程

> 状态：已实现 ｜ 最后核对：2026-10-12 ｜ 代码：`ChangeLog.md`、`releases/index.json`、`scripts/`（`release:*`）

## 1. 总览

```
写 ChangeLog  →  release:pack（本地发包）  →  release:publish（发到 GitHub + 更新索引）
```

`ChangeLog.md` 是**发行说明的唯一手写编辑点**：更新弹窗、GitHub Release 正文、`releases/index.json` 的说明全部由它派生。

> 想直接照做：端到端命令 + 发布前后验收清单见 [§10](#10-端到端发行流程照做清单)。

<a id="changelog-syntax"></a>
## 2. ChangeLog 语法（脚本严格解析）

`ChangeLog.md` 是**发行说明的唯一手写编辑点**：启动器的更新弹窗、GitHub Release 正文、
`releases/index.json` 的更新说明都由它派生。脚本**严格解析**该文件，写错会直接报错并给出行号。

```
## <版本>          # 一个发行一节，版本号与 release:pack --version 一致（v 前缀可省略）
### <lang>         # 每种语言一块：VCMI 标识（chinese/english/tchinese/…），也接受 中文/英文/繁体中文
- 一行一条         # 更新弹窗按行渲染
```

规则：

- `## <版本>`：一个发行一节（版本号与 `release:pack --version` 一致，`v` 前缀可省略）；
- `### <lang>`：节内每种语言一块，`<lang>` 用 VCMI 语言标识（`chinese` / `english` / `tchinese` / …），
  也接受别名 `中文` / `英文` / `繁体中文`；
- `- `：一条说明一行（更新弹窗按行渲染）；
- ⚠️ 版本节内**只允许** `### <lang>` 块与列表项；自由文本、表格、链接等富文本请用
  `release:publish --body-file <file.md>` 单独提供；
- 新版本请加在文件顶部（顺序不影响解析，按版本号查找）。

`ChangeLog.md` **以英文为主**：每个版本节内 `### english` 块放在最前，其他语言（如 `### chinese`）作为同版本节内的额外块补充。该文件本身不放编写说明，详情见本节。

## 3. 第一步：本地发包

```bash
npm run release:pack -- --version 0.2.0     # 正式版：写入版本 → 构建 → 出包（默认只出二进制 + 便携包）
npm run release:pack -- --test              # 测试包：不改源码、不进更新链、不出安装包
npm run release:pack                        # 零参数：用文件里的版本（CI / 换机器，见 §8）
```

`--version` 是**本次发行的契约版本**（ADR 0007）：与 `Cargo.toml` 不同时，会在**构建之前**写入
`Cargo.toml` + `tauri.conf.json`（持久注入，`Cargo.lock` 由 cargo 更新）⇒ 这些改动要**随发行提交**。
`--temp-version` 换成"构建后自动还原"的临时注入（零源码改动）。

构建 → 提取产物 → 生成随包预置补丁 → 写发行目录与条目片段。**不联网、不改 index.json 的版本条目**。

> **说明校验前移（fail-fast）**：`ChangeLog.md` 的更新说明只写进 `release.json` / 条目片段，**不影响二进制与便携 zip 内容**，
> 因此校验排在**编译之前**——没写 ChangeLog 会在编译前直接报错，不会白烧一次构建。若产物已构建好、只是改了说明或代码外
> 元数据，加 `--no-build` 直接重打发行包（复用 `src-tauri/target` 里的现有产物），无需重新编译。

> **跨平台（ADR 0008）**：`--platform` 缺省 = 当前主机推断（Windows/mac/Linux 各自出各自平台的包，
> 推断不出直接报错、**绝不回退 Windows**）；安装包从"默认全出"改为**按需 `--bundles` 透传**。
> 完整平台矩阵、工具链现实与产物树见 [release-cross-platform](./release-cross-platform.zh-CN.md)。

| 开关 | 用途 |
|---|---|
| `--test` | 一键测试包 = `--temp-version` + `--no-update-chain` + `--no-bundle` + 自动版本 `<现版本>-dev.<YYYYMMDD>` |
| `--temp-version` | 临时注入：构建后还原版本文件（零源码改动；tag 复现需带 `--version`） |
| `--platform <triple>` | 目标平台：`windows-x86_64` / `darwin-aarch64` / `darwin-x86_64` / `linux-x86_64`；缺省 = 宿主推断 |
| `--bundles <csv>` | 安装包类型（透传 `tauri build --bundles`）：windows=`nsis,msi`｜darwin=`dmg,app`｜linux=`deb,appimage`。**缺省 = 不出安装包** |
| `--no-bundle` | 不出安装包（现已等同默认；`--test` 内部仍会带上） |
| `--no-build` | 只提取 `target/` 现有产物（**不能**与版本注入/`--test` 同用；上次是 `--test` 构建会被拒绝） |
| `--flavor ai` | AI 版（自带组件 + AI 预置），作同一 Release 的附件；**仅 windows-x86_64**（原生组件当前只有 Windows 库） |
| `--tag` / `--title` | 覆盖 tag / 标题（缺省可由索引 `naming` 模板生成） |
| `--allow-downgrade` / `--allow-tag-mismatch` | 放行"版本不高于索引最新版" / "HEAD 的 tag 与本 tag 不一致"（默认都拒绝） |

AI 版用 `tauri.ai-runtime.conf.json` 变体构建。⚠️ 只有请求 `msi` 时才校验"版本必须纯数字"（WiX 限制）；
默认不出安装包后，`0.2.0-rc1` 不再被误伤。

**常见错误速查**：

| 报错 | 原因 | 出路 |
|---|---|---|
| `平台 xxx 不支持 bundles：dmg` | 该平台描述符没声明 | 查 [release-cross-platform](./release-cross-platform.zh-CN.md) §1.1 平台表，换平台或去掉 |
| `版本 0.2.0-rc1 带后缀且请求 msi` | WiX 只收纯数字版本 | 用纯数字版本，或不请求 msi |
| `--flavor ai 仅支持 windows-x86_64` | AI 原生组件只有 Windows 库 | 换平台或去掉 flavor |

产物落在 `releases/<version>_<stamp>[_flavor]_<platform>/`（**目录名带平台后缀**：同一小时连打两个平台不会互删）：

| 产物 | 说明 |
|---|---|
| `<bin>` | 主程序（Windows `vcmi-fv-launcher.exe` / macOS `VCMI-FV-Launcher.app` / Linux `vcmi-fv-launcher`；app 形态只进便携包不单独落盘） |
| `portable_<ver>_<stamp>_<platform>[_flavor].zip` | 便携包 = **更新资产**：主程序 + 组件 + 预置补丁，**只含程序文件**，绝不含 `.vcmi-fv-launcher/` 下的用户数据；Unix 权限位随包落盘（主程序 0755） |
| 安装包（按 `--bundles`） | `VCMI-FV-Launcher_<ver>_x64-setup.exe`（NSIS）/ `_x64_en-US.msi`（MSI，需联网下载 wix）/ `.dmg` + `.app.tar.gz`（darwin）/ `.deb` + `.AppImage`（linux） |
| `release.json` | 各产物 hash / size，另记 `platform` / `updateMode` / `bin` |
| `index.entry.json` | 写入索引的条目片段 |

## 4. 第二步：发布到 GitHub

```bash
npm run release:publish [-- --version <ver>] [-- --platform <triple>] [-- --body-file <file.md>]
```

`--version` 缺省 = 文件里的版本（与 `release:pack` 对称：定版之后发布也可零参数）。
`--platform` 指定只发布某个平台；缺省时按本地发行目录匹配——该版本只有**单平台**目录时取之
（权威平台 = 目录里 `release.json` 的记录，pack 时写入），**多平台目录共存时直接拒绝**并要求显式
`--platform`（否则只能按打包时间猜，容易发错平台）；指定的平台没有对应目录同样拒绝并列出已有平台。

建 tag / release → 流式传资产 → 更新 `releases/index.json`（浏览数据）**并写 per-target 更新清单**
`releases/update/<platform>.json`（启动器的唯一更新源，见 §5）。**不同平台各跑一次 publish，
各写各的清单文件**——天然无写争用、可并行、可单独灰度/回滚。

### 4.1 重新发布同一版本（资产过时 / 需要覆盖）

同一 tag 的 Release 已存在时，按"要对既有资产做什么"选开关：

| 开关 | GitHub 上既有资产的处置 | 适用 |
|---|---|---|
| `--resume` | 保留，只补缺失/不一致的（按 sha256 比对，一致则跳过） | 多平台发同一 Release；只补新资产 |
| `--replace` | **同名**资产先删再传 | 同名但内容变了（如 `vcmi-fv-launcher.exe`） |
| `--replace-release` | **整个 Release 删除后重建** ⇒ 旧资产（nsis/msi/旧 portable zip）一并清除 | 发行包整体过时，要"只剩本次的资产" |

不受阻的守卫（逐条确认过）：publish **没有** tag 守卫（`--exact-match` 只在 `pack`）、**没有**单调性守卫
（`checkMonotonic` 只在 `pack`）；台账按 `(release, platform)` 直接 upsert 覆盖，同 releaseId 不触发重复版本检查
⇒ **不需要 `--replace-version`**；清单 upsert 是"同版本号整体替换" ⇒ 该版本的 sha256 / URL 会被刷新。

⚠️ **git tag 不会被脚本移动**：`--replace-release` 只删 Release 对象，**tag 保留**；重建时 `tag_name` 已存在，
GitHub **复用旧 tag**（`target_commitish` 被忽略）⇒ 会出现"新资产 + 旧 tag 指向"。要修正必须手工动 tag：

```bash
git log -1 --format=%H v0.1.0     # 先看旧 tag 指向
git tag -f v0.1.0 <目标commit>     # ⚠️ 破坏性：他人/CI 已拉取的 tag 会分叉
git push -f origin v0.1.0         # 强制推送 tag
```

⚠️ **先把工作区提交干净**：publish 会告警"工作区有未提交改动"，且台账记的是 `HEAD` 的 commit——
tag 指向提交、上传的二进制却由脏工作区构建，tag 的 source zipball 无法复现这批资产。

⚠️ **创建发行报 422 `target_commitish`**：tag 不存在时 GitHub 会**在该提交上新建 tag**，因此
`target_commitish`（缺省 = `HEAD`）指向的提交**必须已推到远端**——本地未推送的提交 GitHub 不认识 ⇒ 422，
而 API 文案只说 `"tag_name is not a valid tag"`（误导，真正字段是 `target_commitish`）。解法：先 `git push`，
或 `--target <已推送到远端的 sha|分支名>`，或先手工建好 tag 再发（tag 已存在时 GitHub 忽略 `target_commitish`）。
脚本已内置该诊断提示。

⚠️ **对已装用户无效**：更新器只在 `current < version` 时提示，重发同一版本号不会让已装用户收到更新。
"让新下载者拿到正确包" → `--replace-release`；"让已装用户升级" → 必须 bump 版本。

```bash
# 干跑（不联网、不写盘、不需要 token）
npm run release:publish -- --version 0.1.0 --platform windows-x86_64 --replace-release --dry-run
# 正式重发（需 GITHUB_TOKEN）
npm run release:publish -- --version 0.1.0 --platform windows-x86_64 --replace-release
```

辅助：

```bash
npm run release:preview  # 本地预演：per-target 本地清单 + 静态服务（见 §9）
npm run release:verify   # 索引契约自检
npm run release:status   # 索引 vs 本地目录比对
```

## 5. `releases/index.json` 与 per-target 更新清单

### `releases/index.json`（浏览数据；schema `vcmi-fv-launcher-release-index/1`）

关键字段：

- `repo`、`download{base, pathTemplate}`；
- `download.allowHttp` —— 放行**明文 http** 下载地址（本地预演 / 只有 http 的自建 CDN）；缺省 false = 只认 https；
- `preset.base.update.manifest_url` —— 便携包预置的更新清单地址，**带 `{platform}` 占位符的模板**（如 `.../releases/update/{platform}.json`；ADR 0008 D5）；
- `preset.flavors.ai` —— 预置 `hd.enhance=true` + `hd.ai{enhance,model}`；
- `product` / `channel` / `latest` / `releases[]`。

`releases[]` 是**发行台账**（启动器不读它），键 = **`release` + `platform`**：

- 同一版本的多个平台发布**各占一条**（如 `0.2.0/windows-x86_64` 与 `0.2.0/darwin-aarch64`），
  资产在 GitHub 上合入**同一个 Release / tag**（publish 第二平台用 `--resume`），台账互不覆盖；
- 记录 tag / commit / Release 页 / **全部资产指纹**（nsis/msi/dmg/deb 等安装包的 hash 只在这里）/
  preset / pack——是"离线可查的发行史"；git tag 与 GitHub Release 页是权威源，这里是本地副本；
- 与 per-target 清单**有意重叠**的字段：`updateVersion`↔`version`、`publishedAt`、`critical`、`notes`、
  update 资产的 name/url/sha256/size（台账 = 镜像 + 附加管理信息；清单 = 客户端生效源）。

⚠️ **`versions[]` 投影与 `minSupported` 已退役**（ADR 0008 D5）：更新源不再是本文件，而是下面的
per-target 清单；本文件只回答"整体最新是哪版 / 有哪些发行"（人、`release:status`、文档站看它）。

⚠️ **台账只由 `publish` 写**：`release:pack` 只写**本地片段** `releases/<dir>/index.entry.json`，**从不改台账**
（自举时也只补 repo/download/preset 配置区块）。所以**本地重新打包某个版本**——包括用 `--allow-downgrade`
重打旧版本——**不会**更新台账里的既有条目：台账记录的是 GitHub 上**实际已上传**的资产（url/sha256/size 来自
上传 API 的返回值），而本地重打包必然产出不同资产（新时间戳、zip 名带平台、默认不出安装包）。若 pack 也改台账，
就会出现"索引声称可下载、GitHub 上并没有该文件"的假记录 ⇒ 用户点更新拿到 404。两者不一致时 `verify` 会提示
"片段与索引条目不一致 / 有已打包但未发布的版本"——那是**正常信号**（跑 publish 即对齐）。

### `releases/update/<platform>.json`（启动器唯一更新源；提交进 git）

```jsonc
{ "schema": "vcmi-fv-launcher-update-manifest/2", "platform": "windows-x86_64",
  "latest": "0.2.0", "minSupported": "0.1.0",
  "versions": [{ "version": "0.2.0", "download": "https://…/portable_0.2.0_<ts>_windows-x86_64.zip",
    "sha256": "…", "size": 1, "format": "zip", "kind": "full", "updateMode": "inplace",
    "critical": false, "publishedAt": "2026-10-12", "notes": { } }] }
```

- 启动器把 preset 里的 `manifest_url` 模板按当前三元组解析后**只拉自己平台那一份**；
- **目标平台没有清单（404）= 无更新，静默**；非 200/非 JSON = 检查失败，走既有 TTL+缓存兜底——完整容错规则见 [launcher-update](../features/launcher-update.zh-CN.md) §3；
- 条目带 `updateMode`：`inplace`（Windows PS 脚本 / Linux sh 脚本就地替换）/ `appimage` / `bundle`（macOS `.app.tar.gz` 整包替换+重签，待签名基建）/ `manual`（不应用内更新，只提示手动下载）；
- 自建 CDN 只有 http 时：在清单顶层手工加 `allowHttp: true`（缺省 false = 只认 https；本地预演由 `release:preview` 自动写入）；

`.gitignore` 只放行 `releases/index.json`、`releases/update/*.json` 与初始模板 `releases/index.json.init`（`*.local.json` 本地预演产物不入库）。

### 初始模板（`.init`）与冷启动契约

仓库提交两份**初始模板**（任何代码路径都不会读它们——`loadIndex`/`loadTargetManifest` 只认精确文件名，`publishedVersions` 与 verify 只收 `.json` 结尾；ADR 0008 D11）：

- `releases/index.json.init` —— 空台账 + 真实仓库值（`repo`/`download`/`preset` 配置块 + `flavors.ai` + `{platform}` 模板）；
- `releases/update/windows-x86_64.json.init` —— 空清单（`latest`/`minSupported`/`versions` 均空）；其他平台照此改 `platform` 字段。

两份模板的形状（首次 pack 会自举出同形态内容，也可手工写成这样）：

```jsonc
// releases/index.json
{
  "schema": "vcmi-fv-launcher-release-index/1", "schemaVersion": 1,
  "repo": { "provider": "github", "owner": "<账号>", "name": "<仓库>", "defaultBranch": "main", … },
  "download": { "base": "https://github.com/<账号>/<仓库>/releases", "pathTemplate": "{base}/download/{tag}/{name}" },
  "preset": { "base": { "update": { "manifest_url": "https://raw.githubusercontent.com/<账号>/<仓库>/main/releases/update/{platform}.json" } } },
  "product": "VCMI-FV-Launcher", "channel": "stable",
  "latest": null,
  "releases": []            // ← 空台账
}

// releases/update/<platform>.json（每个要支持的平台一份；空清单 = 尚未发布）
{
  "schema": "vcmi-fv-launcher-update-manifest/2", "platform": "windows-x86_64",
  "latest": null, "minSupported": null,
  "versions": []            // ← 空清单
}
```

**哪些文件何时必需**（"版本构建"= `release:pack`）：

| 场景 | `index.json` | `update/<platform>.json` |
|---|---|---|
| `release:pack` | **不必需**——缺失时首次 pack 自举配置块 | **不必需**——pack 不读不写它 |
| `release:publish` / `preview` | 必需（缺失即 die 并提示先跑 pack；pack 本就先于它们） | **不必需**——缺失时 publish 自动新建 |
| `release:verify` / `status` | 缺失 = 友好提示，不算失败 | 缺失 = 告警 |
| 启动器 | 完全不读 | 404 / 空清单 = **静默无更新**（ADR 0008 D6） |

**仓库信息解析回退链**（自举与 publish 共用；2026-10-12 实测修正）：

- owner/repo：CLI `--owner`/`--repo` > 索引 `repo` 块 > `git remote get-url origin`（**任意主机**——自定义 SSH 别名如 `git@github-future-view:…`、GHE 域名都能解析；旧正则只认字面 `github.com`，会静默失败）；
- branch：CLI `--branch` > 索引 `defaultBranch` > **`origin/HEAD`** > 当前 HEAD > `main`。
  ⚠️ 绝不猜 `master`：raw 更新地址指错分支 = 用户侧 404（本仓库默认分支是 `main`）。

重置到初始态：`cp releases/index.json.init releases/index.json` + `cp releases/update/<platform>.json.init releases/update/<platform>.json`（或直接删掉让系统自建，两者等价）。

## 6. 版本号

三个"版本"先分清（ADR 0007）：

| 层面 | 谁决定 | 约束 |
|---|---|---|
| 源码版本 | git tag / commit | 完全自由 |
| **产物版本（契约版本）** | 构建期常量 `CURRENT_VERSION` = `Cargo.toml` = `tauri.conf.json` | 合法 semver + **严格单调递增** |
| 发行标识 | `--version`（缺省 = 契约版本），可由索引 `naming` 模板生成 | 自由（`[A-Za-z0-9._-]`） |

- **定版**：`release:pack --version <semver>` 写入两处（`Cargo.lock` 由 cargo 更新）→ 随发行提交；
  `npm run version:bump -- patch|minor|major` 仍可用于"只改版本不打包"，`version:sync` 把 `tauri.conf.json` 对齐 `Cargo.toml`。
- **零参数构建**（CI / 换机器）：`git checkout <tag> && npm run release:pack` —— 版本、仓库、说明、预置全部来自已提交的文件。
- 更新比较用它做 semver 判定（见 [launcher-update](../features/launcher-update.zh-CN.md)）：只在 `current < version` 时提示 ⇒
  **同号重发对已装用户无效、版本号不可回退**；`0.2.0-rc1` 与 `0.2.0-rc2` 被判**相等**（更新器不比较 prerelease 内容）。
- ⚠️ **Windows 安装包只支持纯数字版本**：`0.2.0-rc1` 这类会让 bundler 直接失败（实测
  `optional pre-release identifier in app version must be numeric-only ... for msi target`）⇒ 加 `--no-bundle`
  或改用纯 `x.y.z`。正式发行建议始终用纯数字版本。

## 7. 边界与容错

- `minSupported` 决定强制更新；发布前务必确认它与 `latest` 的关系。
- 纯配置包（`kind: config`）不改 exe 版本，`state.json` 按 sha256 记录已应用。

## 8. CI / 其他机器构建

```bash
git checkout <tag>          # 或分支
npm ci
npm run release:pack        # 零参数：版本取自 Cargo.toml
npm run release:verify      # 无需 token
npm run release:publish     # 仅在需要发布时；token 由 GITHUB_TOKEN 环境变量提供
```

- **标签守卫**：HEAD 正好停在某个 tag 上、却与本次 tag 不一致 → `release:pack` **直接拒绝**
  （防"打了标签但忘了定版"把 `v0.2.0` 标签下的 0.1.0 代码发成 `v0.1.0` 发行）；确属有意可加 `--allow-tag-mismatch`。
- **确定性边界**：便携 zip 的条目顺序已按名称排序、条目时间戳固定 1980-01-01；但 exe 内嵌构建期绝对路径、
  NSIS/MSI 内嵌时间戳 ⇒ **不保证逐字节可复现**。CI 与本地产物哈希不同属正常，以 `release.json` / 索引里的 `sha256` 为准。
- **临时注入残留**：`--test` / `--temp-version` 若被中断，下次 `release:pack` 启动时自动还原；
  `--no-build` 遇到"上次是测试构建"的标记会拒绝（`target/<profile>/.fv-test-build.json`）。

## 9. 本地预演（`release:preview`）：publish 之前自测升级链

`release:pack` **不写** `releases/index.json` 的版本条目（索引只由 publish 写），所以光靠 pack 测不出更新提示。
`release:preview` 补上这一环：写 **per-target 本地清单** `releases/update/<platform>.local.json`（更新源的本地副本），
**永不碰真索引与线上清单**：

```bash
npm run release:preview -- --version 0.2.0 --serve     # --port 8765（缺省）/ --base <url> 换自带服务
```

它做四件事：

1. 以本地待发布片段生成 **per-target 本地清单** `releases/update/<platform>.local.json`
   （与线上同款形状；ADR 0008 D5）；
2. 把下载地址改写到本地：模板 `{base}/{dir}/{name}`；
3. 写 `allowHttp: true` —— **明文 http 必须显式放行**：更新器默认只认 https
   （`src-tauri/src/updater/manifest.rs::download_scheme_ok`，读的是清单**顶层** `allowHttp`）；
4. `--serve` 顺带起一个零依赖静态服务（根 = `releases/`），打印该填进设置页的地址。

`.gitignore` 已忽略 `releases/*`（含 `*.local.json`）⇒ 本地清单**天然不入库**，用完无需清理。
⚠️ 不再生成 `releases/index.local.json`（派生索引已退役：更新源是 per-target 清单，它没有消费者）。

被测端（一个**旧版本**的 release 包，且是 release 构建）：

1. 把「更新源」指向本地预演。**设置页默认拒绝 http**（安全起见），最稳的做法是**直接编辑 `launcher.json`**：
   - 文件位置（Windows 便携版）：`<启动器目录>/.vcmi-fv-launcher/launcher.json`（exe 同级；解压目录里就有；macOS / Linux 走平台标准目录，见 ADR 0008 D10）
   - 配置键：`update.manifest_url`，把下面这段**合并进现有文件**（不要整份替换）：
     ```json
     {
       "update": { "manifest_url": "http://127.0.0.1:8765/update/<platform>.local.json" }
     }
     ```
   - 该文件是 JSON5（允许 `//` 注释、尾逗号），手工编辑很友好。
2. 想用设置页也行：先给**启动器进程**设 `VCMI_FV_UPDATER_ALLOW_HTTP=1`（该环境变量同时放行「填 http 地址」与「下载 http 包」），再在「更新源」填上面的地址；注意要**重启启动器**让环境变量生效。
3. 点「检查更新」（force，绕过 24h TTL 缓存）。

前置条件与已知坑：

| # | 条件 | 不满足会怎样 |
|---|---|---|
| ① | 被测 exe 是 **release 构建** | debug 构建既不检查也不应用更新（`cfg!(debug_assertions)` 守卫） |
| ② | 被测 exe 版本 **低于** 清单里的版本 | 更新器只在 `current < version` 时提示；**不能改清单版本造假**（应用后版本对不上 ⇒ 反复提示） |
| ③ | 重测前删 `state.json` 里的 `appliedUpdateSha` / `appliedUpdateVersion` | 同一 sha 装过一次即视为已最新，不再提示 |
| ④ | 更新后重新填一次更新源 | 随包预置（`preset.base.update.manifest_url`）会把更新源改回官方地址 |
| ⑤ | 环境有代理时 `NO_PROXY=127.0.0.1,localhost` | 本地地址被系统代理吞掉，表现为"拉不到清单" |

其它：

- `--no-update-chain` / `--test` 的条目 `updateVersion` 为空 ⇒ 不在 `versions[]` 里，**测不到更新提示**（脚本会告警）。
- 只生成清单不起服务：`--base http://127.0.0.1:8080`（清单里下载地址的基址），再用任意静态服务以 `releases/` 为根暴露。
- 自建 CDN 只有 http 时同理：索引里写顶层 `allowHttp: true`（或让使用者设 `VCMI_FV_UPDATER_ALLOW_HTTP=1`）；若要从设置页填 http 地址，还需给启动器进程设同一个环境变量（放行 UI 的 https 校验）。
- **清单缓存会"钉住"旧清单**：启动器把拉到的清单缓存在 `<启动器目录>/.vcmi-fv-launcher/cache/updateManifest/manifest.json`（24h TTL；**拉取失败时任意年龄缓存兜底**）。改了本地清单后，静默检查与弹窗核对（force=false）读到的仍是缓存 —— 用设置页「检查更新」（force=true，绕过 TTL 并回写缓存）或直接删掉该缓存文件；且预演服务必须保持运行，否则 force 重拉失败又会回退到旧缓存。

### 9.1 本地验收流程（部署旧版 → 验证升级）

> 前提：两个版本都已 `release:pack` 完成。**无需清理 `releases/`、无需重打** —— 旧目录只是被 gitignore 的本地产物，清理反而让本地服务 404；`release:pack` 每次用新时间戳建新目录（不覆盖旧目录），重跑只会累积目录。

把"旧版用户"在本地仿真出来，走完整升级链：

**① 起本地预演服务**（写 per-target 本地清单 + 起静态服务，根 = `releases/`）

```bash
npm run release:preview -- --version 0.2.0 --serve --port 8765
```

本地清单 `releases/update/<platform>.local.json` 只含 `--version` 指定的 0.2.0 条目（resolver 只比对"当前版本 < 目标版本"，**不要求当前版本进索引**，故无需把 0.1.0 也装进本地清单）。保持服务运行。

**② 部署旧版 0.1.0 作为被测端**

0.1.0 的 release 构建就在本地，无需重新编译：

```bash
mkdir D:\test\launcher && cd D:\test\launcher
copy <repo>\releases\0.1.0_20261013-14_windows-x86_64\vcmi-fv-launcher.exe .
```

> 若旧版本的本地目录已被清理、需要重新生成一个：用 **`--temp-version`**（临时注入，构建后自动还原版本文件，
> 零源码改动）——否则 `--version 0.1.0` 会按 ADR 0007 **持久**把 `Cargo.toml`/`tauri.conf.json` 写回 0.1.0，
> 让进行中的新版发行在源码里"丢掉了定版"：
>
> ```bash
> npm run release:pack -- --version 0.1.0 --allow-downgrade --temp-version
> ```
>
> （`--allow-downgrade` 是单调性守卫的放行：更新器只在 `current < version` 时提示，重发同号/更低版本对已装用户无效；
> 该守卫只看 per-target 清单里的已发布版本，与台账无关。）

首次运行会生成 `.vcmi-fv-launcher\launcher.json`。更新器要求其中 `paths.game_dir` / `paths.user_data_dir` **非空**才会发起检查，最稳的做法是手动落地一份（JSON5，允许注释/尾逗号）：

```json
{
  "paths": {
    "game_dir": "D:/test/vcmi",        // 指向任意存在的目录即可（不一定要真装了 VCMI）
    "user_data_dir": "D:/test/vcmi_ud" // 同上
  },
  "update": { "manifest_url": "http://127.0.0.1:8765/update/windows-x86_64.local.json" }
}
```

> 想用设置页填地址也行，但要先给启动器进程设 `VCMI_FV_UPDATER_ALLOW_HTTP=1`（UI 默认拒 http，见 §9 顶部）。

**③ 触发检查并应用**

1. 启动 `D:\test\launcher\vcmi-fv-launcher.exe`（0.1.0，release 构建）；
2. 设置页 → 检查更新（force，绕过 24h TTL）：应看到目标 **0.2.0**、notes 为「演示更新」、非强制；
3. 下载并应用：客户端从 `http://127.0.0.1:8765/0.2.0_20261001-02/portable_....zip` 拉包 → 校验 sha256 → 解压 → 替换 exe；
4. 重启后应为 **0.2.0**。

**④ 验收判据**

- [ ] 检查更新能发现 0.2.0（`has_update=true`，`target=0.2.0`）；
- [ ] 下载后 sha256 与索引一致 → 应用成功，重启后版本号变为 0.2.0；
- [ ] 同一包不反复提示（同一 sha 只提示一次，见 §7）；
- [ ] 应用后 `update.manifest_url` 被随包预置改回官方地址（**预期行为**，§9 坑④；要再测就重填一次）；
- [ ] 负面一项（可选）：把服务端 zip 改坏（sha 不符）→ 应用应**失败且报错**，而非静默放行。

**⑤ 不更新判定（健康性自检）**

拿 `releases\0.2.0_20261001-02\vcmi-fv-launcher.exe`（已是 0.2.0）按同样方式指向预演源 → 应报「已是最新」，确认不会降级 / 循环提示。

## 10. 端到端发行流程（照做清单）

一次正式发行，从改 ChangeLog 到对全体用户生效：

```bash
# ① 补变更说明：ChangeLog.md 加 "## v0.2.0" 节（语法见 §2）
#    缺这一节 ⇒ 写 notes 时报错（--allow-empty-notes 可放行，但更新弹窗会空无一物）

# ② 本地发包：正式版**不要**带 --no-update-chain（否则不进更新链，用户收不到）
npm run release:pack -- --version 0.2.0     # 版本写入 Cargo.toml + tauri.conf.json，需随发行提交
npm run release:pack                        # 已定版后可零参数（版本取自文件）

# ③ 先看计划（不联网、不写盘）
npm run release:publish -- --version 0.2.0 --dry-run

# ④ 发到 GitHub：建 tag/release → 传资产 → 写 releases/index.json（浏览）+ releases/update/<platform>.json（更新源）
npm run release:publish -- --version 0.2.0 --platform windows-x86_64

# ⑤ 推送索引与该平台更新清单 —— 这一步才对该平台用户生效
git add releases/index.json releases/update/windows-x86_64.json && git commit -m "release: v0.2.0" && git push
```

`--version` 传的是**发行标识**（Release title / tag 的来源），缺省 = 契约版本（`Cargo.toml`）；正式发行两者默认一致，见 §6。

发布前后各跑一次自检：

```bash
npm run release:verify                 # 契约自检（--online 逐平台探测下载地址 + 比对远端清单/索引）
npm run release:verify -- --fix        # 重算 ChangeLog 派生的 notes 后写回（per-target 清单无派生物）
npm run release:status                 # 索引 vs 本地已打包目录
```

### 验收清单（发布前）

- [ ] `ChangeLog.md` 有 `## v0.2.0` 节，且 `release:verify` 报「索引 notes 与 ChangeLog 一致」
- [ ] 片段里 `updateVersion` **非空**（`--no-update-chain` / `--test` 会让它为空 ⇒ 不进更新链）
- [ ] 更新资产是 `portable_*_<platform>.zip`：`verify` 校验「进更新链的发行有且仅有 1 个 `role=update` 资产」
- [ ] 便携包**不含** `.vcmi-fv-launcher/` 下的用户数据（§3 产物表）
- [ ] 随包预置补丁与索引 `preset` 一致（`verify` 已校验）
- [ ] 已用 §9.1 本地验收流程跑通「旧版 → 发现更新 → 下载 → 应用」整条链

### 验收清单（发布后）

- [ ] GitHub Release 页：tag / title / 正文 / 资产齐全
- [ ] `releases/index.json` 已推送；raw 地址取到新索引（CDN 有分钟级缓存）
- [ ] 旧版启动器点「检查更新」能收到该版本（启动器侧另有 24h TTL，手动检查立即拉取）
- [ ] 应用后版本正确、无反复提示（同一 sha 只提示一次，见 §7）
