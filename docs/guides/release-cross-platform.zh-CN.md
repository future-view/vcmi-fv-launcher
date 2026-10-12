# 跨平台发行与更新：实施计划与新用法

> ADR [0008](../decisions/0008-cross-platform-release-and-update.zh-CN.md) 的配套实施文档。
> 状态：**已实施**（2026-10-12；macOS `bundle` 模式与签名 CI 待证书基建，就绪前该平台 `manual` 交付）。
> 实施已完成，本文保留作平台矩阵 / 改动点速查；命令细节以 [release.zh-CN.md](./release.zh-CN.md) 为准。

## 0. 三个真相源与不变量

平台知识分布在三处，**改平台必须三处同步**（互相注释引用）：

| 真相源 | 内容 |
|---|---|
| `scripts/release.mjs` 的 `TARGETS` | 构建命令、产物名、bundles、便携包组装方式 |
| `src-tauri/src/updater/platform.rs`（新增） | `{target}` 映射、二进制名、updateMode |
| 清单 schema `vcmi-fv-launcher-update-manifest/2` | 每平台更新条目的形状 |

**不变量（任何一步都不得破坏）**：每 target 恰 1 个 `role=update` 资产；确定性三件套（条目排序 + 固定时间戳 + Unix 权限位）；更新包绝不碰用户数据；404 = 无更新（静默）/ 非 JSON = 检查失败（缓存兜底）。

## 1. Step 1–2：`scripts/release.mjs`

### 1.1 平台描述符与 CLI

```js
/** 平台描述符 = pack 侧唯一真相源（与 updater/platform.rs 镜像，改一处必须改另一处） */
const TARGETS = {
  "windows-x86_64": { bin: "vcmi-fv-launcher.exe", kind: "exe", build: ["tauri", "build"], bundles: ["nsis", "msi"], updateMode: "inplace" },
  "darwin-aarch64": { bin: "VCMI-FV-Launcher.app", kind: "app", build: ["tauri", "build", "--target", "aarch64-apple-darwin"], bundles: ["dmg", "app"], updateMode: "bundle" },
  "darwin-x86_64": { bin: "VCMI-FV-Launcher.app", kind: "app", build: ["tauri", "build", "--target", "x86_64-apple-darwin"], bundles: ["dmg", "app"], updateMode: "bundle" },
  "linux-x86_64": { bin: "vcmi-fv-launcher", kind: "elf", build: ["tauri", "build"], bundles: ["deb", "appimage"], updateMode: "inplace" },
};
```

| 新参数 | 取值 | 语义 |
|---|---|---|
| `--platform <triple>` | `TARGETS` 的键 | 缺省 = **宿主推断**（映射表见 §7.1，与 Rust `platform.rs` 一致）；不在表中 → `die` 并列出可用值 |
| `--bundles <csv>` | 描述符 `bundles` 的子集 | **缺省 = 不出安装包**（`--no-bundle`）；传入则原样透传 `tauri build --bundles` |

⚠️ 不能用 `--target`：已被 publish 的 `target_commitish`（sha|branch）占用（`release.mjs:2368`）。`--no-bundle` 保留为显式语义（现等同默认）。

### 1.2 构建（改 `release.mjs:1305-1308`）

```js
const desc = TARGETS[platform];
const buildArgs = ["run", "tauri", "--", "build", ...desc.build.slice(2)];
if (opts.flavor === "ai") buildArgs.push("--config", "src-tauri/tauri.ai-runtime.conf.json");
const bundles = (opts.bundles ?? "").split(",").map((s) => s.trim()).filter(Boolean);
if (bundles.length) {
  const bad = bundles.filter((b) => !desc.bundles.includes(b));
  if (bad.length) die(`平台 ${platform} 不支持 bundles：${bad.join(", ")}（可用：${desc.bundles.join(", ")}）`);
  buildArgs.push("--bundles", bundles.join(","));
} else {
  buildArgs.push("--no-bundle"); // 默认：只出二进制 + portable
}
```

配套校验调整：
- WiX 纯数字版本限制 → 仅 `bundles.includes("msi") && /[+-]/.test(targetVersion)` 时触发（原 `:1220-1229` 的 `!noBundle` 条件作废）。
- `--flavor ai` 与 `msi` 互斥保留；**`--flavor ai` 仅允许 `windows-x86_64`**（onnxruntime 等原生组件当前只有 Windows 库，其他平台 fail-closed 拒绝并说明）。
- 日志标注平台：`构建（release / linux-x86_64 / bundles=deb,appimage）`。

### 1.3 产物提取（重写 `extractArtifacts`，`release.mjs:710-841`）

- 签名加 `target`：`extractArtifacts({ profile, flavor, zip, platform })`。
- **`dirName` 必须加 `_${platform}` 后缀**（`:721`）：`<version>_<stamp>[_flavor]_<platform>`。否则同一小时打两个平台，第二次的 `fs.rmSync(outDir)`（`:723`）会**删掉第一次的产物**。
- 二进制定位 `profileDir/<desc.bin>`；`kind:"app"` 是目录。
- 便携 zip 按 `kind` 分派：
  - `exe` / `elf`：主程序(0755) + AI 组件 + patch sidecar（现逻辑，仅补 mode）；
  - `app`：整个 `.app` 目录打包（`walk` 已保证排序）；`.vcmi-fv-launcher/launcher.patch.json` 由 `bundle.resources` 在构建期进入 `Contents/Resources/`——**实施时验证**；若 `config_patch` 找不到（见 §2.5），改为注入到 `Contents/MacOS/.vcmi-fv-launcher/`；
  - `appimage`（将来）：AppImage 自身即 update 资产，跳过 zip。
- zip 名：`portable_<version>_<ts>_<platform>[_flavor].zip`。
- **`buildZip` 支持 mode（D3）**：条目加 `mode` 字段（缺省 `0o644`，主程序 `0o755`）；中央目录写 `versionMadeBy = (3 << 8) | 20`（Unix 宿主）、`externalAttrs = mode << 16`；`assertZipStructurallyValid` 增加 mode 断言。
- 安装包提取：`takeBundleArtifact` 改为按 `bundles` 实际请求的类型去 `bundle/<type>/` 找（现有 kind 子目录机制通用，nsis/msi/dmg/deb/appimage 同名）。
- `release.json` 增加 `platform` / `updateMode` 字段；`bundleBuiltAt` 按实际构建类型记录。

### 1.4 签名：本期不做（ADR D9 已修订）

- **不引入 minisign**。复议结论：默认通道是 GitHub 单信任域；CI 持钥挡不住 `contents:write` 的 workflow 注入；本机持钥破坏零参数发布流；同一威胁用 GitHub 侧硬化（2FA / fine-grained PAT / 分支保护）更便宜。
- 清单条目 schema **保留可选 `signature` 字段**（前向兼容，零迁移成本）；`ManifestEntry` 保留 `Option<String>`，本期不校验。
- `release.json` 不加签名字段；pack 流程无签名步骤，`--test` 与正式包行为一致。
- 信任模型写进 `release.zh-CN.md`：**"更新链路信任锚 = GitHub 仓库写权限；HTTPS + sha256 防传输与镜像投毒，不防仓库被控"**；重新评估的触发条件（用户量 / 第二位维护者 / 分发渠道变化 / 真实泄露事件）一并记录。

### 1.5 索引与 publish

- **新文件** `releases/update/<platform>.json`（schema `vcmi-fv-launcher-update-manifest/2`）：
  ```jsonc
  { "schema": "vcmi-fv-launcher-update-manifest/2", "platform": "windows-x86_64",
    "latest": "0.2.0", "minSupported": "0.1.0",
    "versions": [{ "version": "0.2.0", "download": ".../portable_0.2.0_<ts>_windows-x86_64.zip",
      "sha256": "…", "signature": "…", "size": 1, "format": "zip", "kind": "full",
      "updateMode": "inplace", "critical": false, "publishedAt": "2026-10-12", "notes": {} }] }
  ```
- publish 加 `--platform`（缺省宿主推断）：上传资产后读/建该平台清单 → append 条目 → 更新 `latest` / `minSupported` → 原子写。**不同平台各写各的文件，天然无写争用、可并行。**
- `releases/index.json`：**删除 `versions[]` 投影与 `minSupported`**（不再承担更新源）；保留 `releases[]`（浏览）+ repo/download/preset；`preset.base.update.manifest_url` 改为模板：
  `https://raw.githubusercontent.com/future-view/vcmi-fv-launcher/main/releases/update/{platform}.json`
- `projectVersions` / `refreshProjection` 随之退役；`verify --fix` 改为校验 per-platform 清单。
- preview 的 `index.local.json`：本地预演只服务本机平台，派生逻辑按 `--platform` 生成。

### 1.6 verify 改写

| 校验 | 新规则 |
|---|---|
| per-platform 清单 | schema 正确；`platform` 与文件名一致；`versions` 按 semver 单调；`latest` = 首条；`minSupported` 只升不降；进链条目**必须有 signature** |
| release 目录 | 每平台恰 1 个 `role=update`；zip 名含 platform 段；`release.json.platform` 与目录名一致 |
| `--online` | 对每个 per-platform 清单的 `latest.download` 做 HEAD |
| 退役 | 针对 `index.json versions[]` 的旧断言 |

## 2. Step 3：Rust `src-tauri/src/updater`

### 2.1 `platform.rs`（新增）

```rust
pub fn target_triple() -> Option<&'static str>   // (OS,ARCH) 显式映射；macos→darwin；未知 → None
pub fn binary_name(triple: &str) -> &str          // 与 TARGETS.bin 镜像
pub fn update_mode(triple: &str) -> UpdateMode     // inplace / appimage / bundle / manual
```
单测：映射表全覆盖（含 `darwin-x86_64` Rosetta 语义说明）、未知组合返回 None、与 JS 描述符的一致性靠注释约定。

### 2.2 `manifest.rs`

- 新增 `resolve_manifest_url(template)`：替换 `{platform}`；无占位符原样返回（设置页手填兼容）。
- 拉取分流：**404 → `UpdateInfo` 空（无更新，静默）**；非 200 / 非 JSON → 走既有 TTL+ETag+任意年龄缓存兜底，无缓存静默（不新增错误出口）。
- `ManifestEntry` 加 `signature: Option<String>`、`update_mode: Option<String>`；`check_update` 用解析后的 URL，其余比较逻辑（semver / sha256 去重 / 7 天节流 / 回退识别）不变。

### 2.3 `download.rs`

- `STAGED_EXE` 常量 → `staged_binary_name(triple)`。
- 校验顺序维持：**比 sha256 → 解压**（ADR D9 修订：本期不加验签）；`ManifestEntry.signature` 保留为可选字段但不校验。
- `allowed_rel` 白名单仅 `inplace` 模式使用；`bundle` / `appimage` 不走 zip 解压，分流各自策略。

### 2.4 `apply.rs`（按 `update_mode` 分派）

| 模式 | 实现 |
|---|---|
| `inplace` + Windows | **保留 `APPLY_PS1` 全链路不动**（`apply.rs:110-370`） |
| `inplace` + Linux | 新增 `APPLY_SH`：等 PID 退出 → `mv 旧 → .old` → `mv 新 → 原位` → `chmod +x` → 按需 relaunch；detached spawn（新进程组，不随父进程退出被回收）；脚本体保持"路径只经参数"纪律；纯 ASCII；单测 `apply_sh_must_be_ascii` 对齐 PS 既有测试 |
| `appimage` | 下载的 `.AppImage` → `.AppImage.new` → 替换 → `chmod +x` → relaunch（无解压） |
| `bundle`（macOS） | 下载 `.app.tar.gz` → 解压覆盖 `.app` → `codesign -f -s "$IDENTITY" -o runtime <app>`（identity 缺失回退 ad-hoc `-s -`）→ relaunch |
| `manual` | `apply` 返回 `upd_manual_only`；前端展示"请手动下载"并打开条目 `download` 链接 |

保留约束：绝不删目录（junction 风险，rules 04）、主程序最后替换、失败回滚并保留暂存、写日志。`should_auto_apply_on_close` 按模式分派（Linux `inplace` 支持关闭时自动应用）。

### 2.5 `paths.rs` 数据目录（ADR D10）

- `launcher_data_dir()` 按平台分派：Windows 维持 exe 同级；macOS → `dirs::data_dir()/com.vcmi.fv-launcher`；Linux → `${XDG_DATA_HOME:-~/.local/share}/vcmi-fv-launcher`。只改这一个函数，`launcher_config_path` / `mods_cache_dir` / `realm_vcmi_dir` / `downloads_dir` 全链路跟随。
- ⚠️ macOS 需连带确认 `config_patch::apply()` 的补丁查找位置：`.app` 形态下 exe 在 `Contents/MacOS/`，而 `bundle.resources` 把 patch 放进 `Contents/Resources/`——查找顺序改为"数据根 → exe 同级 → Contents/Resources"三处，避免补丁静默不生效。

## 3. Step 4：前端

- `updateFlow.ts` 状态机增 `manual` 出口：下载完成即提示去下载页，不进 `applying`；`restarted===false` 既有语义保留。
- `UpdateDialog` / `ForcedUpdateOverlay`：`upd_manual_only` / `upd_signature_invalid` 文案与动作（后者给"重试/手动下载"）。
- 设置页 `updateManifestUrl` 帮助文案说明可用 `{platform}` 占位符；`set_update_manifest_url` 宽容化（含则解析、不含原样）。

## 4. Step 5：macOS 标准流程 CI

- `tauri.conf.json` 补 `bundle.macOS`：`signingIdentity`（CI 注入）、`entitlements`、`hardenedRuntime: true`、`providerShortName`；Windows 侧按需补 `signCommand`。
- CI（macos runner）：`tauri build --bundles dmg,app` → `codesign`（Developer ID Application）→ `notarytool submit --wait` → `stapler staple` → 上传。
- 证书/公证基建就绪前，macOS 平台以 `updateMode: "manual"` 交付（描述符一个字段切换，无架构补丁）。

## 5. Step 6：文档同步

| 文档 | 改动 |
|---|---|
| `guides/release.zh-CN.md` | 产物清单 / 参数表 / 索引结构三节重写（本文 §7 并入） |
| `features/launcher-update.zh-CN.md` | §3 清单（per-platform + signature）、§5 应用（分派表）、§11 开放问题（差分更新仍未做；minisign 已做） |
| `ide/rules/05-build-run-commands.mdc` | 命令表 + `release:pack` 说明 + 产物树（**两份副本**） |

## 6. 验收清单

- [ ] `zipinfo` / `unzip -l` 验证便携包主程序为 `-rwxr-xr-x`（Unix 权限位真落盘）
- [ ] `release:verify` 通过（含 per-platform 单调性与 signature 存在性）
- [ ] publish 后 `update/<platform>.json` 可拉、`{platform}` 模板三端各自解析正确
- [ ] 真机升级链：Windows（PS1）/ Linux（shell）旧版 → 新版；macOS 过渡期 manual
- [ ] 投毒演练：篡改 zip → sha256 不符被拒且不应用（本期无验签，信任模型见 ADR D9）
- [ ] 404 演练：删除某平台清单 → 该平台静默无更新、其他平台不受影响
- [ ] 同小时连打两个平台 → 两个目录并存（`dirName` 后缀生效）
- [ ] macOS 数据目录落在 `Application Support`，`.app` 内无用户数据写入

## 7. 新用法演示（`npm run release:pack`）

### 7.1 默认平台 = 当前主机（**不是**写死 Windows）

`--platform` 缺省时按**运行命令的这台机器**推断（与 Rust 端 `updater/platform.rs` 同一张映射表）：

| 宿主（OS / CPU） | 推断出的 platform |
|---|---|
| Windows / x64 | `windows-x86_64` |
| Windows / arm64 | `windows-aarch64` |
| macOS / Apple Silicon | `darwin-aarch64` |
| macOS / Intel | `darwin-x86_64` |
| Linux / x64 | `linux-x86_64` |
| Linux / arm64 | `linux-aarch64` |
| 其他组合 | `die` 并列出可用值（**绝不静默回退 Windows**） |

```bash
# 在 Windows 上零参数 → windows-x86_64
npm run release:pack -- --version 0.2.0
#   → releases/0.2.0_20261012-14_windows-x86_64/
#     ├─ vcmi-fv-launcher.exe
#     ├─ portable_0.2.0_20261012-14_windows-x86_64.zip   ← 更新资产
#     └─ release.json

# 同一行命令，在 mac（Apple Silicon）上 → darwin-aarch64
npm run release:pack -- --version 0.2.0
#   → releases/0.2.0_20261012-14_darwin-aarch64/
#     ├─ VCMI-FV-Launcher.app
#     ├─ portable_0.2.0_20261012-14_darwin-aarch64.zip   ← 更新资产
#     └─ release.json

# 同一行命令，在 Linux（x64）上 → linux-x86_64
npm run release:pack -- --version 0.2.0
#   → releases/0.2.0_20261012-14_linux-x86_64/
#     ├─ vcmi-fv-launcher
#     ├─ portable_0.2.0_20261012-14_linux-x86_64.zip
#     └─ release.json
```

三端**同一行命令**，产物各自带平台后缀——这就是"默认只出二进制 + portable"的日常态。

### 7.2 安装包与显式平台

```bash
# ── 需要安装包：--bundles 透传（可组合；不传 = 不出安装包）──
npm run release:pack -- --version 0.2.0 --bundles nsis,msi

# ── 显式指定平台 ──
npm run release:pack -- --version 0.2.0 --platform darwin-aarch64 --bundles dmg,app
npm run release:pack -- --version 0.2.0 --platform linux-x86_64 --bundles deb,appimage
```

⚠️ **跨 OS 打包的工具链现实**（脚本应在 bundler 失败前给出明确指引，而不是把 WiX/codesign 的底层错误直接抛给用户）：

| 想打的平台 | 必须在哪构建 | 原因 |
|---|---|---|
| `windows-*` | Windows（或装好 WiX/NSIS 的容器） | WiX / NSIS 工具链 |
| `darwin-*` | **只能 macOS** | codesign / notarization 需要 macOS + Xcode |
| `linux-*` | Linux（或 WSL2 / 容器） | deb / AppImage 需要 Linux 工具链 |

即 `--platform` 的典型用途是"**显式确认或多架构选择**"（mac 上同时打 aarch64 与 x86_64、Windows 上打 x86_64 与 aarch64）；跨 OS 请在该 OS 的机器或 CI runner 上执行。

# ── AI 版（仅 windows-x86_64：原生组件当前只有 Windows 库）──
npm run release:pack -- --version 0.2.0 --flavor ai

# ── 测试包：不改源码、不进更新链、不签名 ──
npm run release:pack -- --test

# ── 已构建过，只重新提取 + 签名 ──
npm run release:pack -- --version 0.2.0 --no-build

# ── 发布：每平台各跑一次，各写各的 update/<platform>.json ──
npm run release:publish -- --version 0.2.0 --platform windows-x86_64
npm run release:publish -- --version 0.2.0 --platform darwin-aarch64
npm run release:publish -- --version 0.2.0 --platform linux-x86_64

# ── 自检 ──
npm run release:verify              # 结构 / 签名 / per-platform 单调性
npm run release:verify -- --online  # 追加下载地址可达性探测
```

**产物总览**（三平台各一份）：

```
releases/
├─ 0.2.0_20261012-14_windows-x86_64/     ├─ 0.2.0_20261012-14_darwin-aarch64/   ├─ 0.2.0_20261012-14_linux-x86_64/
│  ├─ vcmi-fv-launcher.exe               │  ├─ VCMI-FV-Launcher.app            │  ├─ vcmi-fv-launcher
│  ├─ portable_…_windows-x86_64.zip      │  ├─ portable_…_darwin-aarch64.zip   │  ├─ portable_…_linux-x86_64.zip
│  └─ release.json                       │  ├─ VCMI-FV-Launcher_0.2.0_aarch64.dmg │  ├─ vcmi-fv-launcher_0.2.0_amd64.deb
│                                        │  └─ release.json                     │  └─ release.json
├─ update/                               ← 启动器唯一更新源（publish 写，提交进 git）
│  ├─ windows-x86_64.json                ← Windows 用户拉这份
│  ├─ darwin-aarch64.json                ← Apple Silicon 用户拉这份
│  └─ linux-x86_64.json                  ← Linux 用户拉这份
├─ index.json                            ← 浏览数据（人/工具看）；preset 里是 {platform} 模板
```

**常见错误速查**：

| 报错 | 原因 | 出路 |
|---|---|---|
| `平台 xxx 不支持 bundles：dmg` | 该平台描述符没声明 | 查 §1.1 表，换平台或去掉 |
| `版本 0.2.0-rc1 带后缀且请求 msi` | WiX 只收纯数字版本 | 用纯数字版本，或不请求 msi |
| `--flavor ai 仅支持 windows-x86_64` | AI 原生组件只有 Windows 库 | 换平台或去掉 flavor |

## 8. 重置到初始状态（冷启动）

适用场景：推倒重来、清空所有发行记录、或新仓库第一次接入。初始状态 = **两个文件**，仓库已带 `.init` 模板（`releases/index.json.init` 与 `releases/update/windows-x86_64.json.init`，提交进 git 专供重置；任何代码路径都不会读它们——`loadIndex`/`loadTargetManifest` 只认精确文件名，`publishedVersions` 与 verify 只收 `.json` 结尾）：

```jsonc
// releases/index.json（首次 pack 会自举；也可手工写成这形态）
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

步骤：

1. `releases/index.json`：`cp releases/index.json.init releases/index.json`（或直接删掉——首次 pack 会自举出同形态内容）；
2. `releases/update/*.json`：按平台 `cp releases/update/<platform>.json.init releases/update/<platform>.json`，或全部删除（两者等价：空清单与 404 都按"无更新"静默处理；其他平台的 `.init` 照 windows 的模板改 `platform` 字段即可）；
3. 清理本地产物（可选，均 gitignored）：`releases/<版本>_*` 打包目录；
4. **重新生成随包预置**（关键，生成物但提交进 git）：跑一次
   `npm run release:pack -- --version <当前版本> --no-build`——它会把 preset 规范成 `{platform}` 模板
   并重写 `src-tauri/resources/release-preset/*.json`（旧口径的 `manifest_url` 会被自动升级，见 `resolvePreset`）；
5. 提交：`git add releases/index.json releases/update/ src-tauri/resources/release-preset/`；
6. 验证：`npm run release:verify`（预期：`update/<platform>：0 条` + `releases[] 为空` 告警，**无错误**）；
   启动器侧：拉到的空清单 / 404 → 静默无更新，手动"检查更新"也不报错。

注意事项：

- **单调性守卫随之重置**：清单为空后任何版本号都可再发（`publishedVersions()` 为空）——若要防止旧版本号被重发，
  记住之前的最高版本，或保留一条最小台账条目；
- **存量已安装用户**：本地的"已应用 sha / 待应用版本"记录不受影响；回退后重发**同一版本同一包**（sha 不变）
  对老用户不会提示（按 sha256 去重）——要么 bump 版本，要么接受无提示；
- **GitHub 侧不受回退影响**：已发布的 Release / 资产还在网上，回退只改 git 里的索引与清单；
  要让用户彻底拿不到，需另行删除 GitHub Release 或把清单指向别处；
- **冷启动自举的仓库解析**：无索引时 owner/repo 从 `git remote get-url origin` 推断（**任意主机**——自定义 SSH 别名 / GHE 域名都能解析；旧正则只认字面 `github.com` 会静默失败），branch 按 `origin/HEAD` → 当前 HEAD → `main` 回退（**绝不猜 `master`**，否则 raw 更新地址指错分支 = 用户侧 404）；都推断不出会 die 并给出 `--owner/--repo/--branch` 指引；
- `manifest_url` 的 `{platform}` 模板从冷启动第一天就是默认值（`resolvePreset`），不会再出现指向
  `index.json` 的旧口径补丁。
