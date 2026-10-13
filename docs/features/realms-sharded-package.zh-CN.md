# 领域包分片分发（`.vcmirealm` Sharding）

> 状态：**已实施**（2026-10-13 落地 P1–P4；**真机端到端待验**，见 §12）｜ 最后核对：2026-10-13 ｜ **体裁：设计 + 实施记录**（§1–§10 决策与契约，§11 分期，§12 验证结果）
> 代码：`src-tauri/src/realms/transfer.rs`（切分 / `parts` 条目 / 派生字段管理）、`src-tauri/src/repository/download.rs`（逐片下载 + 拼回）、`scripts/build-realms-index.mjs`、`src/features/realms/editor/ExportRealmDialog.tsx`
> 关联：[realms-repository](../features/realms-repository.zh-CN.md)（消费端行为）、[repository-index](../guides/repository-index.zh-CN.md)（索引编写规范）、[realms-editor](../features/realms-editor.zh-CN.md)（导出表单）

## 1. 目标与范围

**做**：让单个 `.vcmirealm` 超过 git 托管站单文件容忍线时，**自动切分为固定大小的分片**落库，经索引 `parts` 契约分发；启动器下载端逐片校验、拼回整包、复用现有安装流程。

**不做**：
- 不改 `.vcmirealm` 包格式（ZIP 结构、`realm.json`、条目 checksum 语义全部冻结）；
- 不做断点续传 / 并发分片下载（v2，见 §10）；
- 不做分享分支分片（一对一文件传递没有拼接约定，见 §9.4）；
- 不改导入端 `import_realm_impl`（拼回后的文件就是标准包）。

触发背景：领域仓库托管在 GitHub（`future-view/vcmi-fv-launcher-realms`），GitHub 对 **>50 MB 的文件告警、>100 MB 拒绝 push**；带过场视频 / 阵营巡礼素材缓存的领域包很容易越过 50 MB。社区上传门户方案（未实施）同样只收 ≤50 MB 的 git 轨文件，分片是其硬边界的自然解法。

## 2. 现状事实

### 2.1 包格式与分发链路

- `.vcmirealm` = ZIP：ZIP 注释魔数 `vcmirealm` + 包根 `realm.json`（含全条目 checksum）+ `vcmi/{Mods,Maps,config}`、`assets/`、可选 `showcase-cache/`（`src-tauri/src/realms/transfer.rs:363-367`）。
- 完整性双轨：包内 `realm.json.checksum` = 按路径排序的条目内容 SHA-256（导入校验，`transfer.rs:1577-1613`）；索引条目 `sha256` = **整包文件哈希**（下载强制校验，`repository/download.rs:110-114`）。
- 发布链路：编辑器「发布」分支 → `export_realm`（`repo_dir` 模式）→ 包落 `<repo>/realms/<id>/<id>.vcmirealm` → `publish_realm_to_repo` 写封面 + 合并 `index.json`（`transfer.rs:1235-1390`）。
- 消费链路：`fetch_realm_index`（ETag + 30 min TTL + 多源合并）→ `install_remote_realm` → 流式下载 + 强制 sha256 → `import_realm_impl`（`repository/download.rs:53-138`）。

### 2.2 已预留的分片契约（本项目代码现状）

| 位置 | 现状 |
|---|---|
| `transfer.rs:1094-1108` | `DERIVED_INDEX_FIELDS` 已含 `"parts"`，注释「一期不实现分片…读取端只拦非空数组」 |
| `transfer.rs:1195-1196` | 合并时刻意**不主动写** `parts`，既有值不触碰 |
| `repository/download.rs:89-97` | 遇非空 `parts` → 拒绝并返回 `repo_realm_sharded` |
| `src/i18n/*.ts`（5 语言） | `repo_realm_sharded`「该领域使用分片分发，请升级启动器」文案已就位 |
| `scripts/build-realms-index.mjs:19,373` | 脚本侧同款预留注释 |

即：**索引契约、错误码、文案都已存在，缺的只是发布端切分与下载端拼回**——这是本方案可行性最高的根本原因。

### 2.3 单文件上限事实

| 位置 | 值 | 语义 |
|---|---|---|
| GitHub | 50 MB 告警 / 100 MB 拒绝 push | 分片目标线 |
| `PACKAGE_MAX_BYTES`（`repository/mod.rs:27-28`） | 100 MB | 下载端硬放弃（原为 Gitee 附件上限） |
| `PUBLISH_GIT_MAX`（`transfer.rs:705`） | 50 MB | 发布黄色告警 |
| `PUBLISH_DOWNLOAD_MAX`（`transfer.rs:703`） | 100 MB | 发布红色告警 |

仓库现状：三个包 1.4 / 3.5 / 9.1 MB，均无需分片——**无存量迁移成本**，分片只对未来的大包生效。

## 3. 方案选型

| 方案 | 思路 | 结论 |
|---|---|---|
| **A. 成品字节切分** | 导出流程原样产出整包 ZIP，再按固定字节区间切成 `part001…partNNN`；下载端逐片下载校验、拼回整包、走现有导入 | **采纳**。包格式 / 校验语义 / 导入代码零改；整包 sha256 契约不变；任意静态托管可用；正好落在已预留的 `parts` 契约上 |
| B. 多 ZIP 清单式包（格式 v2） | manifest + N 个各含条目子集的 ZIP | 否决：导出 / 导入 / 校验 / 脚本 / 回归全重写，且单文件包格式要永久兼容，churn 远超收益 |
| C. GitHub Releases 附件（单文件 ≤2 GB） | 启动器零改（跟随重定向即可） | 否决为主方案：与「git 轨」门户方案冲突（要管 tag/release，Actions 合入模型作废）。保留为特大领域的兜底路径 |
| D. Git LFS / 外置 CDN | — | 否决：LFS 免费带宽 1 GB/月且 raw 对 LFS 对象行为别扭；外置 CDN 破坏「两个 URL」契约 |

## 4. 分片规则

- **片大小常量**：`REALM_PART_MAX_BYTES = 45 MiB`（47,185,920 字节）。
  - 算术依据：GitHub 的 50 MB 告警线无论按 MiB（52,428,800）还是按十进制 MB（50,000,000）理解，45 MiB 都留有余量；同时兼容 Gitee 免费版 git 50 MB 线。
- **命名**：`<id>.part001`、`<id>.part002`…（3 位补零，平铺在 `realms/<id>/` 下）。999 片上限 ≈ 43.9 GiB，远超实际需要。
- **阈值语义**：`pkg_size > REALM_PART_MAX_BYTES` 才切；等于或小于则保持单文件（既有行为逐字不变）。
- **分片是成品文件的字节区间切片**，与 ZIP 内部结构无关：第 i 片 = 原文件 `[i*45MiB, min((i+1)*45MiB, size))`。拼回 = 按序拼接，与原文逐字节一致。

## 5. 索引契约（`parts` 规格定稿）

```jsonc
{
  "urlBase": "https://raw.githubusercontent.com/future-view/vcmi-fv-launcher-realms/main/",
  "realms": {
    "<id>": {
      "version": "1.0.0",
      "sha256": "…",              // 整包（拼回后）文件哈希——权威校验，语义不变
      "size": 123456789,          // 整包字节数——语义不变
      // —— 以下二选一（互斥）——
      "download": "https://…/realms/<id>/<id>.vcmirealm",   // 单包条目
      "parts": [                                                // 分片条目
        { "url": "https://…/realms/<id>/<id>.part001", "size": 47185920, "sha256": "…" }
        // …按数组序拼接
      ]
    }
  }
}
```

规则：

1. `parts` 非空 ⇒ 分片分发，条目**不写** `download`；`parts` 缺省/空数组 ⇒ 单包分发，条目**不写** `parts`。
2. 每片 `size ≤ 45 MiB` 由发布端保证；下载端只把「单片 > 100 MB」当索引损坏拒绝（sanity 护栏）。
3. 条目级 `sha256` / `size` 始终描述**整包**——五态更新矩阵（`repository/index.rs:70-92`）、`origin.sha256`、ETag 缓存全部不受影响。
4. `schemaVersion` 保持 **1**（纯增量可选字段；旧启动器遇 `parts` 已有 `repo_realm_sharded` 拦截，见 §10.1）。
5. 每片 `url` 为绝对地址（与既有 `download` 风格一致，不用相对 `urlBase` 拼接——显式 URL 对任何托管形态都成立）。

## 6. 仓库布局与发布流程

```
<repo>/realms/<id>/<id>.vcmirealm      # ≤45 MiB：单包（现状不变）
<repo>/realms/<id>/<id>.part001        # >45 MiB：分片（平铺，命名对齐 cover.webp 风格）
<repo>/realms/<id>/<id>.part002
<repo>/realms/<id>/cover.<ext>         # 不变
<repo>/index.json                      # 条目按 §5 二选一
```

**陈旧清理（发布端职责，双向）**：
- 单包 → 分片：删除仓库内既有 `<id>.vcmirealm`；
- 分片 → 单包：删除仓库内既有 `<id>.part*`；
- 分片 → 分片（片数变化）：覆盖同名片、删除多出来的尾部片（如 5 片改 3 片，删 `part004/005`）。

清理在 `publish_realm_to_repo` 合并索引**之前**完成，保证「文件系统与索引」不出现中间态。

## 7. 导出端设计（`src-tauri/src/realms/transfer.rs`）

### 7.1 流程（实施后口径）

```
export_realm_impl：zip.finish() → pkg_sha256 / pkg_size（打包、取消、进度、checksum 零改）
  ↓
publish_realm_to_repo（发布分支）：
  pkg_size > 45 MiB？
  ├─ 否 → 既有单包路径 + prune_stale_parts(dir, id, 0)（清掉上次残留的分片）
  └─ 是 → url_base 可推导？
           ├─ 否 → 保留单包 + 告警 shard-needs-url-base（超限告警照常触发）
           └─ 是 → split_realm_package(pkg_path)
                    → 删除单包 → prune_stale_parts(dir, id, N)（清掉多出来的旧片）
                    → entry.parts = [{url,size,sha256}]、移除 entry.download
                    → 告警 sharded
  ↓
merge_repo_index：parts / download 互斥派生管理（§7.2）→ 写 index.json
```

> **与初版设计的一处偏离**：切分点从「`export_realm_impl` 打包后立刻切」改为「`publish_realm_to_repo` 内、拿到 `plan.url_base` 之后」。理由：分片 URL 需要 `url_base`，而它要到 `plan_publish` 才定下来——在拿不到地址的场景下提前切分，会让仓库残留「单包已删、索引无 `parts`」的死状态。现在拿不到地址就**不分片**（保留单包 + 告警），宁可不分也绝不残缺。分享分支恒不分片。

### 7.2 `merge_repo_index` 派生字段双向管理（最需谨慎的契约变更）

`parts` / `download` 从「不主动写、既有值不触碰」升级为**完全派生的互斥字段**：

| 字段 | 新条目有值 | 新条目无值 / 空 |
|---|---|---|
| `download` | 覆盖 | **删除既有值** |
| `parts` | 覆盖（非空数组） | **删除既有值** |
| 其余可推导字段 | 覆盖（空值保护不变） | 保留既有值（不变） |

- 删除计入 `changed`（`entryState` 三态判定依赖 `changed`，`transfer.rs:1348-1354`）。
- 依据：这两个字段由发布产物完全决定，不存在「手工维护值」——仓库里手写 `download` 指向别处的维护方式没有存在场景（`urlBase` 才是手工出口）。
- **`scripts/build-realms-index.mjs` 必须同步实现同口径**（现有「Rust 与脚本两侧逐字一致」纪律）。
- 既有测试 `merge_keeps_manual_fields_and_reports_changed_keys` 的「不主动写 parts」断言（`transfer.rs:1774-1775`）反转为新语义；新增「单包 → 分片 → 单包」迁移用例。

### 7.3 预览估算

`preview_realm_publish` 新增返回：

- `estimatedPartCount`：对 `realm_data_root(id)` 做与 `collect_realm_files` 同口径的 **metadata 遍历**求 `bytesTotal`（不读文件内容，仍是快操作），`ceil(bytesTotal / 45 MiB)`；
- `partMaxBytes`：片大小常量（前端提示语用，不硬编码数字）。

前端仅在 `estimatedPartCount > 1` 时显示预估行（见 §9.3）——常态小包零噪声。

### 7.4 导出结果回传

`export_realm` 返回体新增：

- `partCount: usize`（0 = 未分片，两分支都有）；
- `repo.parts: [{path, size, sha256}]`（仅发布分支；结果页罗列——前端不自行拼分片文件名，延续「不抄仓库布局约定」原则，`ExportRealmDialog.tsx:106-107`）。

### 7.5 告警语义变化

| 告警码 | 现状 | 变更（实施后） |
|---|---|---|
| `over-git-max`（>50 MB 黄） | 两分支都可能触发 | **发布分支仅在不分片时触发**（自动分片后不存在超限单文件落盘）；分享分支保留 |
| `over-download-max`（>100 MB 红） | 两分支都触发且红级 | 发布分支仅在不分片时触发；分片时改发黄级 `over-total-hint`（提示仓库体积增长） |
| `sharded`（信息） | — | 新增：分片成功，`arg` = 「N × ≤45.0 MB」 |
| `shard-needs-url-base`（黄） | — | 新增：需分片但推不出 `url_base`，已按单包发布（推送可能被拒） |
| `shard-failed`（黄） | — | 新增：切分失败，已按单包发布 |
| `bundled` / `bad-id` / `version-unchanged` / `pkg-unchanged` / `no-url-base` | — | 不变 |

## 8. 下载安装端设计（`src-tauri/src/repository/download.rs`）

```
定位索引条目（不变）
  ↓
读 parts（判定前移到 download / sha256 读取之前——分片条目没有 download）
  ├─ parts 非空 →
  │   1. 建 downloads/<id>-<version>.parts/
  │   2. for (i, part) in parts.iter().enumerate():
  │        download_to_file(part.url → <dir>/<id>.partNNN)     // 既有函数零改
  │        - 单片 > PACKAGE_MAX_BYTES(100MB) → repo_realm_part_too_large（索引损坏）
  │        - sha256_file(part) ≠ part.sha256 → 清理 + repo_realm_part_sha
  │        - 进度事件聚合 + 附加 partIndex / partCount
  │        - is_cancelled 哨兵检查（既有机制，跨片生效）→ 清理 + REALM_DOWNLOAD_CANCELLED
  │   3. 按 parts 数组序拼接为 downloads/<id>-<version>.vcmirealm
  │   4. 整包 sha256 校验（既有 repo_realm_install_sha，不符拒装并清理）
  │   5. import_realm_impl（零改）→ 写 origin（sha256 用整包值，现有逻辑不变）
  │   6. 清理分片目录
  └─ parts 空 → 既有单文件流程（逐字不变）
```

配套调整：

- **护栏**：分片条目增设总量 sanity 上限 `SHARDED_TOTAL_MAX_BYTES = 1 GiB`（防索引误填；正常领域远低于此），复用既有错误码 `repo_realm_pack_too_large`；单片护栏沿用 `PACKAGE_MAX_BYTES = 100 MB`。
- **进度事件** `realm-download-progress`：`received` / `total` 改为跨片求和（UI 无感），附加可选字段 `partIndex` / `partCount`（前端可显示「分片 3/12」，不显示也能用）；`done` 只在**末片**收尾帧置真。
- **错误码**：新增 `repo_realm_part_sha`（第 N 片校验失败）、`repo_realm_parts_invalid`（`parts` 条目缺 `url` / `sha256`），已补 5 语言 i18n；`repo_realm_sharded` 的拦截逻辑退役（新启动器已支持分片），i18n 文案保留备用。
- **分片本地命名**：`part001`…（**按数组序命名**，不取远端文件名——避免路径穿越与命名冲突）。
- **无新增 IPC 命令、无参数变更**——不触碰契约冻结清单；`check:invoke-parity` 仍照跑。

## 9. 「导出领域」表单：自动分片 vs 分片选项（决策）

### 9.1 决策

**自动分片，不加任何勾选 / 输入项。** 发布分支在包超过 45 MiB 时静默切分；表单控件零增加、IPC 零增加。

### 9.2 理由

1. **分片是传输层适配，不是内容决策。** 弹窗现有选项全部回答「这个领域包含什么、发到哪」——素材缓存（`includeShowcaseCache`）、索引片段（`includeFragment`）、仓库目录（`repoDir`）、URL 前缀（`urlBase`）。分片回答的是「托管站单文件上限怎么绕」——发布管线的实现细节，与领域内容无关。
2. **勾选会产生不可自解释的坏组合。** 关掉分片 + 包 >50 MB ⇒ GitHub push 被拒 / 告警，且报错发生在弹窗关闭很久之后，要懂 git 才能定位。把正确行为设为默认、错误行为留给选项，与该弹窗既有哲学相悖——下载地址推导正是「把填空换成确认」的同类设计（`ExportRealmDialog.tsx:415-416`）。
3. **勾选唯一的正当场景（自建托管无 50 MB 限制）属于高级出口**，与 `urlBase` 手工覆盖同类；真要开放也应进设置面板高级项，不占主表单——且一期无此需求。
4. **自动分片不是黑盒。** 发布前有预估（§7.3），发布后有实际清单（§7.4）；分片文件名规则固定，用户随时可去仓库目录核对。知情无需决策。
5. **分享分支不分的理由**：分片的接收方需要拼接约定，一对一文件传递没有这个约定；保持单文件 + 超限告警，与「分片只在仓库分发链路生效」的定位一致。

### 9.3 表单变化清单（`ExportRealmDialog.tsx`）

| 区域 | 变化 |
|---|---|
| idle 控件区 | **零变化**（无新勾选、无新状态位、`canExport` 逻辑不变） |
| idle 预览区 | `estimatedPartCount > 1` 时，在「将写入」清单下方加一行信息条：「预计分为 N 片（每片 ≤ 45 MiB），发布后见结果页」。小包不显示 |
| result 区 | `partCount > 0` 时：ResultRow「分片」=「N 片」，并以 mono 小字列出各片路径（数据来自 `repo.parts`，前端不拼名） |
| 新 i18n 键 | `exportShardEstimate`、`exportFieldParts`、`exportWarnOverTotalHint`（实施时补 5 语言） |

### 9.4 分享分支

零变化：仍出单文件 + 可选索引片段；>50 MB / >100 MB 告警原样保留（`RED_WARNING_CODES` 不动，`ExportRealmDialog.tsx:37`）。

## 10. 兼容性、风险与开放问题

### 10.1 兼容性

- **旧启动器**（未实现分片）：遇 `parts` 非空 → `repo_realm_sharded`「请升级启动器后再安装」（已实现）。升级后正常安装，无数据迁移。
- **旧索引条目**（无 `parts`）：消费行为逐字不变。
- **回滚**：发布端出 bug 时，把仓库条目 `parts` 删掉、补回 `download` 即回到单包分发——前提是单包文件还在。删除单包发生在分片产物全部落位之后（同一函数内），不存在「单包已删、分片残缺」的中间态。

### 10.2 风险

| 风险 | 评估 / 缓解 |
|---|---|
| 拼回临时盘占用 = 整包大小 | 可接受（下载目录即既有临时区）；v2 可实现跨片 `Read+Seek` 适配器免拼回 |
| 仓库历史增长（git 保留每版每片） | 大领域反复发布会撑大 clone。缓解：周期性 history squash；特大领域走 Releases 兜底（方案 C）。GitHub 建议仓库整体 <1 GB |
| `merge_repo_index` 语义变更波及既有约定 | 集中在两个互斥字段，其余「空值不动既有值」不变；Rust 与 mjs 同步改 + 迁移用例全覆盖 |
| 预览估算偏差 | 仅用于提示行，标注「预计」；实际片数以结果页为准 |

### 10.3 开放问题（v2 候选）

1. **断点续传**：GitHub raw 支持 Range，按片续传是自然延伸（重试只补失败片）。
2. **并发下载**：一期串行（简单、可取消）；GitHub raw 单 IP 并发限制未实测，需谨慎评估。
3. **45 MiB 是常量不是配置**：先跑；自建 CDN 场景真实出现再进设置面板高级项。
4. **社区门户交互**：门户（未实施）建议收整包、由 realms 仓库内 Actions 切分，保持上传侧简单——分片不增加门户设计负担。

## 11. 实施分期与改动清单

| 期 | 内容 | 文件 | 状态 |
|---|---|---|---|
| P0 | 契约定稿 + 文档 | 本文、`docs/guides/repository-index.zh-CN.md`、`docs/features/realms-repository.zh-CN.md` | ✅ 完成 |
| P1 | 导出 / 发布：`split_realm_package` / `prune_stale_parts`、`entry.parts`、preview 估算（`estimatedPartCount` / `partMaxBytes`）、merge 互斥派生管理、告警语义 | `src-tauri/src/realms/transfer.rs` | ✅ 完成 |
| P2 | 下载 / 安装：`parse_parts` + 逐片下载校验 + 拼回 + 聚合进度 + 护栏 + 错误码 | `src-tauri/src/repository/{download,mod}.rs`、`src/i18n/*.ts`（5 语言） | ✅ 完成 |
| P3 | 索引脚本分片（`--part-size`）+ `parts` / `download` 互斥派生（与 Rust 同口径） | `scripts/build-realms-index.mjs` | ✅ 完成 |
| P4 | 前端：类型、预览提示行、结果态分片清单、告警文案映射（含 i18n 5 语言 × 8 键） | `src/lib/tauri.ts`、`ExportRealmDialog.tsx`、`src/i18n/{types,zh-CN,en,ru,pl,de}.ts` | ✅ 完成 |
| P5 | 真机端到端 | — | ⏳ 待做（见 §12.2） |

## 12. 验证结果

### 12.1 已完成的验证（2026-10-13）

| 项 | 命令 / 方式 | 结果 |
|---|---|---|
| Rust 单测 | `cargo test --lib` | 151 passed / 0 failed（新增 8 例：切分边界、split→拼回逐字节一致、陈旧分片清理、单↔分片迁移矩阵、`parts` 解析从严、聚合进度、拼回字节一致） |
| IPC 契约 | `npm run check:invoke-parity` | 后端 136 / 注册表 136 / 前端 invoke 136 一致（本次无新增命令） |
| 前端类型与构建 | `npm run build`（`tsc && vite build`） | 通过 |
| 索引脚本端到端 | 真实包 `vcmi-realm.vcmirealm`（1.4 MB）+ `--part-size 500000 --dry` | 切出 3 片（500000 / 500000 / 417393 = 1,417,393 = 整包）；条目写入 `parts`、无 `download`、`sha256`/`size` 仍为整包；`cat part*` 的 sha256 与整包**逐字节一致** |

### 12.2 待做：真机端到端（P5）

构造 >45 MiB 的测试领域（带过场视频或勾选巡礼素材缓存即可）后：

1. 编辑器「发布」到**临时 git 工作树** → 审查 `git diff`：`realms/<id>/<id>.part00N` 新增、单包删除、`index.json` 条目出现 `parts` 且无 `download`；
2. 启动器加源（索引必须 https，可用 GitHub 草稿仓库或临时静态站点）→ 安装 → 校验 `origin.sha256` 等于整包哈希、领域可激活；
3. 覆盖安装（分片 → 分片）、再改小领域发布一次（分片 → 单包），确认陈旧分片与 `parts` 字段都被清理。

## 13. 关联文档

- 消费端行为：[realms-repository](../features/realms-repository.zh-CN.md)
- 索引编写规范：[repository-index](../guides/repository-index.zh-CN.md)
- 导出表单所在编辑器：[realms-editor](../features/realms-editor.zh-CN.md)
- 领域系统总览：[realms](../features/realms.zh-CN.md)
