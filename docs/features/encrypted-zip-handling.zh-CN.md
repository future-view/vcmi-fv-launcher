# 加密 zip 模组资源采集方案

> 状态：已定稿，进入实现。决策树闭合：X（默认安全行为）+ A（主路线：运行时配置）+ B（备选：构建期注入）；监控磁盘与逆向两条路径已证伪。

## 1. 背景与问题

某些 VCMI 发行版（如 `Heroes3-VCMI`）把 mod 的 `content.zip` 做了密码加密。FV 启动器生成 wiki 缓存时需从 `content.zip` 提取素材，但当前无法处理加密包：

- FV 用的 `zip` crate（2.4.2）在 `Cargo.toml` 里只开了 `deflate`，**未开 `aes-crypto`**，无法解密 AES 条目（传统 ZipCrypto 仅需密码即可，不需该特性）；
- 代码里加密条目被 `if let Ok` 静默跳过（`resource.rs` 的 `zip_index_of`），且 `scan.rs` 的 `contentZipEncrypted` 写死 `false`，前端无从知晓"该 mod 资源被加密、未采集"。

## 2. 已排除的路径（基于上游源码核实）

- **运行时监听磁盘**：上游 VCMI 仅在挂载 `LOD/VID/SND` 原版容器时把条目解包到 `userCachePath/extracted`（`CArchiveLoader::extractToFolder`，`references/vcmi/lib/filesystem/CArchiveLoader.cpp:216`）；`content.zip` 等 zip 类由 `CZipLoader` 纯内存读取，**加密 zip 解密后也不落盘**。监控磁盘只能捡到原版 LOD 数据（FV 已能直接读 LOD），捡不到加密 mod 素材。→ 死路。
- **逆向 `VCMI_client.exe` 提密码**：技术上可行（密码硬编码在增强版二进制），但属规避第三方技术保护措施，且把 FV 焊死在特定构建上、破坏通用性。→ 不采用。

## 3. 决策：FV 支持密码解包 + 发行版两种喂密码方式

加密是发行版作者自己加的，最干净的做法是**让他们把密码交给自己构建的 FV**。已实现两条并列路线，取密码优先级为 **配置（A）> 内置（B）**：

- **路线 A（运行时配置，主推）**：发行版在随包的 `launcher.json` 里预置 `archives.zip_passwords`，用 stock FV 即可，无需重建。一份通用 FV 适配所有发行版（每个游戏目录各带自己的 `launcher.json`）。
- **路线 B（构建期注入源码，备选）**：发行版修改 rs 源码里的 `vcmi::resource::builtin_zip_passwords`（一处带注释的入口）填入密码，自行 `cargo build` 出他们的发行版 FV。密码编译进二进制、不进公开仓库。

两条路线都满足"加密方自己把密码交给自己构建的 FV"，合法且保住 FV 核心通用性。**关键安全结论**：

- **stock FV（public 源码、无内置密码、无 `launcher.json`）= 不携带任何发行版密码**，通用二进制无法解开任一发行版的加密包；只有发行版**自行修改源码内置密码**或**自带 `launcher.json`** 的构建/部署，才能解密。
- 密码最终都会落在发行版自己的二进制或配置文件里，技术层面仍可被逆向还原（与把密码硬编码进 `VCMI_client.exe` 同级），仅能挡小白，不提供强保密。

**默认安全行为 X**：无论是否提供密码，FV 都**真正接线加密检测**——扫描阶段标记 `contentZipEncrypted`，取代当前"静默丢资源"（前端明示按需接，本期未做）。

## 4. 前置工程（本次实现）

1. `Cargo.toml`：`zip` 特性 `["deflate", "aes-crypto"]`（AES 解密；传统 ZipCrypto 仅需密码无需该特性）。
2. `config.rs`：新增 `archives` 分组与 `zip_passwords: HashMap<String,String>`（路线 A，键为 mod 目录名或 `_default` 兜底）；保存配置时保留既有密码，避免清掉发行版预置。
3. `resource.rs`：
   - `ZipIndex` 增加 `has_encrypted`；
   - `zip_index_of` 用 `by_index_raw(i)` 取条目名与 `encrypted()`，纳入加密条目并标记；
   - `read_zip_entry_cached` 按 mod 目录查 `zip_password_for`，有密码走 `by_index_decrypt`，否则 `by_index`；
   - `zip_password_for`：**先查 `launcher.json` 配置（A），查不到回退 `builtin_zip_passwords`（B）**，具体 mod 目录名优先、`_default` 兜底；
   - **`BUILTIN_ZIP_PASSWORDS`：rs 源码内置密码常量数组（路线 B 入口），`builtin_zip_passwords()` 由它构建 `HashMap`；public 源码数组为空，发行版开发者在此填密码后自行构建**；
   - 新增 `content_zip_is_encrypted(mod_path)` 供扫描轻量判定。
4. `scan.rs`：`contentZipEncrypted` 改为调用 `content_zip_is_encrypted(dir)`。

## 5. 发行版喂密码的两种接入方式

### 路线 A：`launcher.json` 运行时配置（免构建，推荐）

随包的 `launcher.json` 预置，用 stock FV 即可：

```json5
{
  "archives": {
    // 键：mod 目录名（如 "adeptorium"）或 "_default"（全局兜底）；值：解密密码
    "zip_passwords": {
      "adeptorium": "发行版自设密码",
      "_default": "通用兜底密码"
    }
  }
}
```

### 路线 B：rs 源码内置密码（需发行版自行构建）

**在 `src-tauri/src/vcmi/resource.rs` 的常量数组 `BUILTIN_ZIP_PASSWORDS` 里填入密码**（取消注释并改成你们的真实值），再自行 `cargo build`：

```rust
// src-tauri/src/vcmi/resource.rs
pub(crate) const BUILTIN_ZIP_PASSWORDS: &[(&str, &str)] = &[
    ("adeptorium", "你们的真实密码"),          // ← 某 mod 的专用密码
    ("_default", "所有未单独列出的 mod 共用的密码"), // ← 所有加密包共用同一密码时只填这一行
];
```

- 每行 `(mod 目录名, 密码)`；目录名为 mod 最末级目录（如 `adeptorium`），嵌套子 mod 用最末级。
- `("_default", 密码)` 是全局兜底：**任何没有单独列出的 mod 都用它**。若你们所有加密包都用同一个主密码，只填这一行即可，新增 mod 也不会漏。
- public 源码该数组为空 → 构建出的通用 FV 二进制**不携带任何发行版密码**，解不开任一发行版加密包；只有发行版自己填了并构建的二进制才携带。

**取密码优先级（见 `zip_password_for`）**：`launcher.json` 按目录名匹配 → 内置按目录名匹配 → `launcher.json` 的 `_default` → 内置的 `_default`。

## 6. 安全分析

- 密码硬编码在发行版二进制里，任何拿到二进制的人都能还原——发行版那层加密本就只挡小白。让发行版把密码写进（或编进）自己的 FV，**真实暴露度不变**。
- 客户端能解密 ⇒ 强保密不可能；本方案目标是"让 FV 完成素材采集"，而非"对终端用户保密"。保密诉求若强，应走 B（密码埋二进制）而非 A（明文配置）。

## 7. 不在本次范围

- 不实现运行时磁盘监控、不实现逆向提取。
- 不自动探测 / 猜测密码。
