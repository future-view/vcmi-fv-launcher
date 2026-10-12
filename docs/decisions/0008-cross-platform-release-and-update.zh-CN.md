# ADR 0008：跨平台分发与更新模型 —— 平台描述符 + per-target 清单 + `updateMode` 分派

- 状态：**已确认并实施**（2026-10-12 与开发者核对通过并一次性落地；macOS `bundle` 模式与签名 CI 待证书基建，就绪前该平台 `manual` 交付）
- 相关：`scripts/release.mjs`、`src-tauri/src/updater/**`、`src-tauri/src/infra/http.rs`、`src-tauri/src/vcmi/config.rs`（`UpdateCfg`）、`guides/release.zh-CN.md`、`features/launcher-update.zh-CN.md`、ADR [0007](./0007-version-injection-at-pack.zh-CN.md)

## 评审结论（2026-10-12）

| 核对点 | 结论 |
|---|---|
| 1 移动端 | **排除**——`launch.rs:20-23` 靠 `Command::new(VCMI_client.exe).spawn()` 工作，Tauri 移动端沙箱不能 spawn 进程且 exe 是 Windows PE，产品上不成立 |
| 2 清单结构 | **按推荐**：per-target 清单 + `{target}` 占位符模板（一个 indexUrl） |
| 3 macOS | **按标准流程**：Developer ID 签名 → `notarytool` 公证 → `stapler`；更新用 `.app.tar.gz` 整包替换 + 重签 |
| 4 `--no-bundle` 默认 | **确定** |
| 5 minisign | **评估后本期不引入**（复议：GitHub 单信任域 + CI 持钥边界窄 + 更便宜的硬化替代；schema 保留可选 `signature`，见 D9） |
| 6 分期 | **一次性处理**——代码一次落地；macOS `bundle` 模式待证书基建，就绪前以 `manual` 交付 |

## 背景

发行与更新链路今天是 **Windows 单平台隐式假设**，`portable` 只是最表层症状。逐层实测证据：

| 层 | 证据 | 后果 |
|---|---|---|
| 打包脚本 | `release.mjs:42` `EXE_NAME` 硬编码 exe；`:799-816` 只认 `bundle/nsis`+`bundle/msi`；`:1145-1151` `classifyAsset` 按 `.exe/.msi`；`:1354-1357` 强制"每目录恰 1 个 update 资产"；`:196-214` zip **不写 Unix 权限位** | 出不了 mac/linux 的包；多平台各出一个便携包即 `die`；即便解出来也**不可执行** |
| 更新链路 | `download.rs:26` `STAGED_EXE` 硬编码；`apply.rs:110-370` `APPLY_PS1`+`spawn_replacer` Windows-only，非 Windows 直接 `Err("upd_windows_only")`（`:372-375`）；`:248-252` 关闭时自动应用非 Windows 恒 false；`download.rs:48-64` 白名单装不下 `.app` 目录 | 应用内自替换只在 Windows 成立 |
| 索引/清单 | `index.json:41-49` `versions[]` 单 `download`+单 `sha256`，**无 platform 维度**；`manifest.rs:305+` 只比 semver、**不按平台挑包** | mac/linux 用户被提示更新 → 下载 Windows zip → 应用失败：**提示了却装不上** |
| 签名/工具链 | `tauri.conf.json` 无 `windows.signCommand`、无 `macOS.{signingIdentity,entitlements,hardenedRuntime}`；全仓 `minisign`/签名零命中 | 产物全部未签名；macOS 无法过 Gatekeeper；更新完整性仅靠 sha256（见 D9） |

按项目"一功能一命令、不做老兼容"的既有惯例（ADR 0007），本 ADR **不保留新老兼容**：索引结构、资产命名、`--no-bundle` 语义、updater 分派全部重做。

## 决策

### D1 平台描述符 = pack 侧唯一真相源

```js
const TARGETS = {
  "windows-x86_64": { bin: "vcmi-fv-launcher.exe", kind: "exe", build: ["tauri", "build"], bundles: ["nsis", "msi"], updateMode: "inplace" },
  "darwin-aarch64": { bin: "VCMI-FV-Launcher.app", kind: "app", build: ["tauri", "build", "--target", "aarch64-apple-darwin"], bundles: ["dmg", "app"], updateMode: "bundle" },
  "darwin-x86_64": { bin: "VCMI-FV-Launcher.app", kind: "app", build: ["tauri", "build", "--target", "x86_64-apple-darwin"], bundles: ["dmg", "app"], updateMode: "bundle" },
  "linux-x86_64": { bin: "vcmi-fv-launcher", kind: "elf", build: ["tauri", "build"], bundles: ["deb", "appimage"], updateMode: "inplace" },
};
```

`bin`/`kind` 决定便携包怎么组装（exe/elf 直接 zip；`.app` 是目录需整体打包；AppImage 自身即便携包）；`bundles` 决定 `--bundles` 透传哪些合法、提取时去 `bundle/<type>/` 找；`updateMode` 决定该平台更新方式。描述符之外**没有**第二个平台分叉点。

### D2 `--bundles` 透传，默认只出 portable

- 默认 `--no-bundle`（只出二进制 + 便携 zip）；`--bundles nsis,msi` 原样透传给 `tauri build`。**复用 Tauri 机制，不另造 `--nsis`/`--msi` 旗标。**
- portable 永远由脚本组装，不交给 Tauri bundler：它是**更新产物**，需要确定性（条目排序 + 固定时间戳）、白名单、绝不含用户数据——这三条 bundler 都给不了。
- WiX 纯数字版本限制（ADR 0007 实测）改为**仅当请求的 bundles 含 `msi`** 时触发；默认无 bundles 后，`0.2.0-rc1` 不再被误伤。
- `--flavor ai` 与 bundles 含 `msi` **互斥**（WiX 不打包 `.vcmi-fv-launcher` 点开头目录，出了等于"标称 AI 版却不含组件"）。

### D3 zip 必须写 Unix 权限位

`buildZip` 的中央目录改为：`versionMadeBy` 置 Unix 宿主（高字节 `3`）、`externalAttrs` 写 `0o755`（主程序）/ `0o644`（组件与配置）。**这是 mac/linux 便携包可执行的前提**，与条目排序、固定时间戳并列为确定性三件套。

### D4 资产命名带 target

`portable_<ver>_<ts>_<target>.zip`（如 `portable_0.2.0_20261012-14_linux-x86_64.zip`）。安装包名由 Tauri 生成、已含平台信息，不动。GitHub Release 内资产名必须唯一——无 target 段会跨平台同名冲突。

### D5 分平台更新清单；`indexUrl` 是**一个模板**而非每平台一个配置

- `releases/update/<target-triple>.json` 是启动器的**唯一更新源**：
  ```jsonc
  { "schema": "vcmi-fv-launcher-update-manifest/2", "target": "linux-x86_64",
    "latest": "0.2.0", "minSupported": "0.1.0",
    "versions": [{ "version": "0.2.0", "download": ".../portable_0.2.0_<ts>_linux-x86_64.zip",
      "sha256": "…", "signature": "…", "size": 1, "format": "zip", "kind": "full",
      "updateMode": "inplace", "critical": false, "notes": {} }] }
  ```
- `manifest_url` 存**带 `{target}` 占位符的模板**，原样穿过 preset → patch → `launcher.json`（`config.rs:363-370`），在 `manifest.rs:347` 取到 url 之后、发起请求之前替换。这样随包预置补丁保持**与平台无关**，一份 patch 三端通用。
- 占位符映射**必须查显式表**（`std::env::consts::OS` 在 macOS 是 `macos`，而 triple 是 `darwin`，字符串拼接必错）；用**运行时** `std::env::consts` 而非编译期 `cfg!`——Rosetta 下 x86_64 构建报 `darwin-x86_64` 是正确的（更新必须换同架构二进制）。

  | `(OS, ARCH)` | triple | | `(OS, ARCH)` | triple |
  |---|---|---|---|---|---|
  | `(windows, x86_64)` | `windows-x86_64` | | `(macos, x86_64)` | `darwin-x86_64` |
  | `(windows, aarch64)` | `windows-aarch64` | | `(linux, x86_64)` | `linux-x86_64` |
  | `(macos, aarch64)` | `darwin-aarch64` | | `(linux, aarch64)` | `linux-aarch64` |
- **聚合 `releases/index.json` 降级为派生的浏览数据**（由各 `index.entry.json` 片段生成，供 `release:status`/文档站/人查看），**不再是更新源**。
- 二进制资产 URL 每平台天然不同，由**文件名**承载，经既有 `download.pathTemplate` 的 `{name}` 占位符（`index.json:13-17`）自动打通——索引一个模板、资产多个链接，两者不冲突。

**per-target 拆分对"分平台输出更新包"的增益**（评审补充点）。先区分两件事：*产出包*靠平台描述符与构建步骤，与清单拆分无关；*分发与更新节奏*才靠清单拆分：

| 增益 | 单清单 | per-target |
|---|---|---|
| 发布节奏解耦 | ❗一个文件原子更新 ⇒ 所有平台必须同版本同时发；任一平台缺工具链/构建失败，整个发行卡住 | 各平台独立发版、可停不同版本（`minSupported` 各自独立） |
| 灰度 / 回滚粒度 | 整包（所有平台一起） | 单平台（只给 linux 发 rc；只回滚 mac） |
| 故障隔离 | 一个坏条目波及所有平台的兜底逻辑 | 只影响该平台 |
| CI 并行 | 多 OS 同时 publish 同一文件 = git 写争用 | 各写各的文件，天然并行 |
| 客户端开销 | 每次检查拉全平台元数据 | 只拉本平台 |
| 全局视图 | 自带 | 唯一代价：版本漂移时需派生的浏览索引回答"整体最新是哪版" |

### D6 两条必须新立的容错规则

1. **目标平台无清单（404）= 无更新，静默。** 区别于 `manifest_url` 未配置 = 零网络行为（`manifest.rs:352-353`）。未构建的 target（如 `linux-aarch64` 暂不发）不能变成错误提示。
2. **非 200 或非 JSON = 本次检查失败**，并入既有 TTL + ETag + 任意年龄缓存兜底路径，无缓存则静默，**不新增错误出口**。自建 CDN / Pages 对不存在文件可能返回 200 + HTML，不能当"有更新"解析。

### D7 `updateMode` 分派（updater 侧）

| 模式 | 适用 | 替换策略 |
|---|---|---|
| `inplace` | Windows / Linux（elf） | Windows 保留 `APPLY_PS1`；Linux 新增 POSIX shell replacer（等进程退出 → `rename` 旧→`.old` → `move` 新→原位 → `chmod +x` → 按需 relaunch），与 PS1 同构 |
| `appimage` | Linux AppImage | 下载新 AppImage → `.new` → 替换 → `chmod +x` → relaunch（AppImage 自身即便携包，无 zip） |
| `bundle` | macOS | **标准流程**（核对点 3）：Developer ID Application 证书签名（hardened runtime + entitlements）→ `notarytool` 提交公证 → `stapler` 盖章；分发 dmg，**更新用 `.app.tar.gz`**（`tauri build --bundles app`）整包替换 + 本地重签后拉起。替换后必须仍是系统信任的签名，否则 Gatekeeper 拦截 |
| `manual` | macOS（无证书过渡期） | 不应用：提示"有新版，请手动下载"并打开下载页。这不是将就——未签名包替换内容即破签名，`manual` 是该状态下唯一诚实的行为；证书与公证 CI 就绪后翻 `bundle`（改一个字段） |
| `store` | — | 占位；移动端已排除（见背景） |

用户态字段（`skip_version`/`prompt_*`/`appliedUpdateSha`）语义不变，按平台各自独立。

### D8 保留自研 updater，不换 `tauri-plugin-updater`

整包替换模型做不到本项目四件事：白名单局部更新（`runtime`/`mods`/`realms` 分目录落地，`download.rs:48-64`）、`launcher.patch.json` 配置合并、纯配置包按 sha256 去重（`mod.rs:84-112`，不改 exe 版本的更新不能反复提示）、`delete_patch` 删除清单。可借的只有 minisign 机制本身——见 D9。

### D9 更新签名：本期不引入 minisign（2026-10-12 复议修订）

**复议动因**：更新清单与资产的默认通道都是 GitHub（`raw.githubusercontent.com` + `releases/download`），信任锚本就是"GitHub 仓库写权限"；为此引入一套签名体系的边际价值需要重新称重。

**原评估仍成立的部分**：sha256 + https 覆盖传输损坏与"清单可信、资产被换"（含镜像投毒）；不覆盖"清单与资产同源被控"。

**为什么本期不做**：

1. **CI 持钥的边界比看起来窄**：私钥若存 GitHub Actions secrets，持 `contents:write` 的 PAT 即可推一个 workflow 把 secret 打印出来——真正被挡住的只剩"能发 release、不能推代码"这一小类攻击者。
2. **本机持钥则破坏零参数发布流**：key 只存维护者本机才真正与 GitHub 全链路脱钩，但这与 ADR 0007 的"换机器 / CI 零参数发包"直接冲突，每版发行多一步不可自动化的手工。
3. **同一威胁有更便宜的解法**：GitHub 侧硬化（2FA、fine-grained PAT 限单仓 Contents、tag guard 已有、保护分支）直接压低"PAT 泄露"这个最现实的入口，成本远低于一套签名体系。
4. 信任边界仅在用户**自配镜像规则**（`network.mirror_rules`，hosts 任意，`infra/http.rs:12-18`）时才 widened；默认直连 GitHub 不存在该问题。

**决策**：

- 本期**不引入 minisign**；`download` 校验维持 **sha256 + https + 版本严格单调**（单调性同时天然抑制"回滚到旧版"攻击）。
- 清单 schema **保留可选 `signature` 字段**（零成本前向兼容）：将来若引入签名，存量清单无需迁移；`ManifestEntry` 同样保留 `Option<String>`，本期不校验。
- 在 release 指南显式写下信任模型：**"更新链路信任锚 = GitHub 仓库写权限；HTTPS + sha256 防传输与镜像投毒，不防仓库被控"**——让这个取舍是清醒的而非偶然的。
- **重新评估的触发条件**：用户量显著增长 / 增加第二位维护者或 CI runner / 分发渠道离开 GitHub Releases / 出现真实凭证泄露事件。

### D10 数据目录按平台分派（实施计划阶段补充确认）

`paths.rs:135-140` 的 `launcher_data_dir()` 写死 **exe 同级 `.vcmi-fv-launcher/`**——这对 Windows 便携版是核心特性（"解压覆盖"语义依赖它），但另两端不成立：macOS `.app` 装进 `/Applications` 后包内写数据既破坏签名也可能要管理员权限，且整包替换即丢数据；Linux AppImage 挂载的是**只读** squashfs，exe 同级根本不可写。

| 平台 | 数据根 | 理由 |
|---|---|---|
| Windows | `<exe>/.vcmi-fv-launcher/`（**不变**） | 便携单文件夹是既定特性 |
| macOS | `~/Library/Application Support/com.vcmi-fv-launcher/` | 系统标准位置；签名安全 |
| Linux | `${XDG_DATA_HOME:-~/.local/share}/vcmi-fv-launcher/` | XDG 规范；AppImage 与 apt 两种形态都成立 |

`launcher_config_path` / `mods_cache_dir` / `realm_vcmi_dir` / `downloads_dir` 全部由 `launcher_data_dir()` 派生（`paths.rs:134-151`），**只改这一个函数**即可全链路切换；`config_patch::PATCH_REL` 与 `delete_patch` 白名单随之落在平台数据根内，语义不变。updater 的"原位替换"前提也由此解除——非 Windows 的数据根本来就不在 exe 旁。

### D11 冷启动契约与 `.init` 初始模板（实施阶段补充确认）

**契约**：`releases/index.json` 与 `releases/update/<platform>.json` 都不是"版本构建"的硬前置——pack 缺失索引时自举配置块（repo/download/preset），publish 缺失清单时自动新建，启动器对 404/空清单按 D6 静默无更新。**唯一必需的是流程顺序**：publish/preview 要求索引存在（pack 先跑即可）。

**初始模板**：仓库提交 `releases/index.json.init` 与 `releases/update/<platform>.json.init` 两份空态模板，供"推倒重来 / 新仓库接入"时 `cp` 使用。`.init` 后缀保证**全链路惰性**——`loadIndex`/`loadTargetManifest` 只认精确文件名，`publishedVersions` 与 verify 只收 `.json` 结尾；`.gitignore` 需显式放行 `index.json.init`（`releases/*` 会吞掉它）。

**仓库信息解析回退链**（自举正确性依赖它，2026-10-12 实测修正）：owner/repo = CLI > 索引 > git remote（**任意主机**——自定义 SSH 别名 / GHE 域名都要能解析，旧正则只认字面 `github.com` 会静默失败）；branch = CLI > 索引 > `origin/HEAD` > 当前 HEAD > `main`。**绝不猜 `master`**——raw 更新地址指错分支等于用户侧 404。

## 不可绕过的约束（实施时必须校验）

- 版本契约继承 ADR 0007：合法 semver、严格单调递增、`Cargo.toml` ↔ `tauri.conf.json` 一致。
- **每 target 恰好 1 个 `role=update` 资产**（替代现有"每目录 1 个"）；`classifyAsset` 与 `verify` 的 zip 命名识别改为带 target 段解析。
- 更新包仍**绝不含用户数据**（`realms`/`cache`/`downloads`/`mods`/`launcher.json`）——跨平台同样成立。
- 确定性三件套保持：条目排序、固定时间戳、**新增的 Unix 权限位**。
- `{target}` 无法映射（未知 OS/ARCH）→ 按"无更新"静默处理，不报错。
- `set_update_manifest_url`（`commands/updater.rs:141-158`）取值宽容化：含 `{target}` 则解析，不含则原样使用；设置页 UI 说明可用该占位符。

## 实施顺序（一次性，核对点 6）

| # | 内容 | 依赖 |
|---|---|---|
| 1 | pack 平台化：描述符、`--bundles` 透传、zip 权限位、资产带 target | 无 |
| 2 | 分平台清单生成 + `{target}` 解析 + 两条容错规则 | 1 |
| 3 | updater `updateMode` 分派：Windows PS1 保留、Linux shell replacer、AppImage、macOS `.app.tar.gz` + 重签 | 1,2 |
| 4 | GitHub 侧硬化（2FA / fine-grained PAT / 分支保护）与信任模型入档 | 2 |
| 5 | macOS 标准流程 CI（签名/公证/staple）与 `manual` → `bundle` 翻转 | 3 + 证书基建 |
| 6 | 文档同步：`release.zh-CN.md`、`launcher-update.zh-CN.md`、`ide/rules` 两份副本 | 全部 |

代码一次落地；macOS `bundle` 的**实机验证**依赖证书与公证基建到位（组织侧前置），就绪前该平台以 `manual` 交付，架构不为过渡态打补丁。

## 后果

- **行为变更（不做老兼容）**：`manifest_url` 语义（模板 + `{target}`）、资产命名（带 target）、`--no-bundle` 成为默认、`releases/index.json` 不再是更新源、updater 按 `updateMode` 分派、清单 schema 保留可选 `signature` 字段（本期不签）。
- **冷启动契约**（D11）：两份发行文件均非 pack 前置（自举 / 按需创建）；初始态由 `.init` 模板定义；仓库解析回退链固定为"任意主机 remote + `origin/HEAD` → 当前 HEAD → `main`"，消灭 `master` 误判导致的用户侧 404。
- 需同步文档：`guides/release.zh-CN.md`（产物清单/参数表/索引结构三节）、`features/launcher-update.zh-CN.md`（§3 清单 / §5 应用 / §11 开放问题）、`ide/rules/05-build-run-commands.mdc`（两份副本）。
- **仍开放**：派生的 `releases/update/index.json` 平台目录（可选增强，把"静默无更新"升级为"明确告知此平台未发布"）；macOS 证书基建的归属与排期；增量/差分更新仍不在范围（`launcher-update` §1）。
