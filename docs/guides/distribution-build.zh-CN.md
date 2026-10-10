# 发行版定制构建（内置加密 zip 密码）

> 状态：已实现 ｜ 最后核对：2026-10-10 ｜ 代码：`src-tauri/src/vcmi/resource.rs`（`BUILTIN_ZIP_PASSWORDS`、`builtin_zip_passwords`、`zip_password_for`）、`src-tauri/src/vcmi/config.rs`（`ArchivesCfg`）、`docs/features/encrypted-zip-handling.zh-CN.md`

## 1. 背景

增强版等发行版给 mod 的 `content.zip` 加了密码（VCMI 引擎二进制里硬编码了密码，内存解密，磁盘不落明文）。FV 启动器建缓存需要解开这些包采集素材，但**不逆向、不绑定任何特定发行版**。因此由**发行版作者自己把密码交给自己构建的 FV**，两条路线任选：

- **路线 A（免构建，推荐）**：在随包 `launcher.json` 的 `archives.zip_passwords` 预置密码，用 stock FV 即可。
- **路线 B（需构建）**：把密码写进 FV 源码内置表，自行 `cargo build` 出你们的发行版 FV。

两路取密码优先级：**配置按目录名 → 内置按目录名 → 配置 `_default` → 内置 `_default`**。

## 2. 路线 B：内置密码（本指南重点）

### 2.1 在哪里填

编辑 `src-tauri/src/vcmi/resource.rs` 的常量数组 `BUILTIN_ZIP_PASSWORDS`，取消注释并改成你们的真实值：

```rust
// src-tauri/src/vcmi/resource.rs
pub(crate) const BUILTIN_ZIP_PASSWORDS: &[(&str, &str)] = &[
    ("adeptorium", "你们的真实密码"),                    // 某 mod 的专用密码
    ("_default", "所有未单独列出的 mod 共用的密码"),       // 全局兜底
];
```

- 每个元素是一对 `(mod 目录名, 密码)`；目录名取 mod 的**最末级目录**（如 `adeptorium`），嵌套子 mod 用最末级。
- `("_default", 密码)` 是**全局兜底**：任何没有单独列出的 mod 都用它。若你们所有加密包共用同一主密码，**只填这一行即可**，后续新增 mod 也不会漏。

> ⚠️ `BUILTIN_ZIP_PASSWORDS` 是**整张内置密码表**（可同时含具体 mod 条目与 `_default`），不是单一的 `_default` 密码。`_default` 只是这张表里的一个特殊键。它和路线 A 的 `launcher.json` 里的 `zip_passwords` 是同一套结构（目录名→密码 + `_default` 兜底）。

### 2.2 public 源码为何为空

`BUILTIN_ZIP_PASSWORDS` 在 public 仓库里是**空数组**。因此：

- 用 public 源码构建出的通用 FV 二进制**不携带任何发行版密码**，解不开任一发行版的加密包；
- 只有发行版**自行修改此处并构建**出的二进制才携带对应密码——密码留在你们自己的构建产物里，不进公开仓库。

### 2.3 构建

```bash
# 依赖安装（首次）
npm install

# 打包 Windows exe（产物：src-tauri/target/release/vcmi-fv-launcher.exe）
npx tauri build
```

构建出的 exe 即你们的发行版 FV，运行时会用内置密码解开对应 `content.zip`。可按需配合 [便携部署](./portable-deployment.zh-CN.md) 随包分发。

## 3. 路线 A：launcher.json 配置（免构建）

随包的 `launcher.json`（FV 数据目录下的 `launcher.json`，结构见 `config.rs` 的 `ArchivesCfg`）预置：

```json5
{
  "archives": {
    "zip_passwords": {
      "adeptorium": "发行版自设密码",
      "_default": "所有未单独列出的 mod 共用的密码"
    }
  }
}
```

用 stock FV 即可，无需 Rust/Tauri 工具链，也不产生版本分化。适合不想自行构建的发行版。

## 4. 安全边界（务必知悉）

密码最终都会落在发行版自己的**二进制**（路线 B）或**配置文件**（路线 A）里，技术层面仍可被逆向还原（与把密码硬编码进 `VCMI_client.exe` 同级）。因此这套机制**仅能挡小白，不提供强保密**。若需更强隔离，应把密码埋在发行版自有可信组件、让 FV 只调用不持有——但那同样客户端可逆，且破坏 FV 通用性，不在本指南范围。

## 5. 验证

- 不需要密码即可验证加密检测：扫描阶段会标记 `contentZipEncrypted`（见 `encrypted-zip-handling.zh-CN.md`）。
- 真机解密验证需要发行版提供真实密码：填好路线 A 或 B 后，对该加密 `content.zip` 跑一次缓存构建，确认素材被采集、无"加密跳过"提示。
