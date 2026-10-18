# 后端架构

> 状态：已实现 ｜ 最后核对：2026-09-30 ｜ 代码：`src-tauri/src/`

## 1. 分层与依赖方向

```
commands ──► updater / runtime / repository / realms / modcache / vcmi ──► infra
                     repository ──► realms
              realms / modcache / runtime ──► vcmi
```

| 目录 | 层级 | 职责 |
|---|---|---|
| `commands/` | IPC 壳 | 解包 state → 调下层 → `Result` 转换；按域分 `app` / `mods` / `mod_install` / `mod_settings` / `settings` / `vcmi_settings` / `paths` / `launch` / `cache` / `runtime` / `updater` / `gamedata` / `game_import` |
| `infra/` | 设施 | 无业务语义的底层能力 |
| `vcmi/` | 领域 | 游戏与模组侧知识：`paths`（目录识别）/ `config`（launcher.json）/ `scan` / `town` / `gamedata` / `h3core` / `h3data`（原版数据闸门）/ `i18n` / `resource`（三来源解析）/ `health` / `hd_edition` / `game_import` / `mod_install` / `modmeta`（生成模组的 mod.json 文案表，外部 JSON + 编译期嵌入）/ `setup` / `version` |
| `modcache/` | 领域 | 百科缓存构建（`build/` 按实体采集 + `assets` / `preview` / `hd` / `batch` / `index` 等） |
| `realms/` | 领域 | 领域生命周期、隔离、地图、模组 |
| `repository/` | 领域 | 远端索引抓取缓存与下载安装 |
| `runtime/` | 领域 | 外挂原生组件 |
| `updater/` | 领域 | 启动器自更新 |

**禁 Tauri 类型**：`infra` / `vcmi` / `modcache` / `runtime` / `updater` 中不得出现 `State` / `AppHandle` / `Window`。进度一律用**回调**向下传，事件 `emit` 只在 `commands/` 做。

> 例外（与注释不符的实际状态）：`realms/` 与 `repository/` 目前直接承载命令定义与 Tauri 类型（`realms/mod.rs`、`repository/mod.rs`）。因此"`commands/` 是唯一允许 tauri 类型的层"这条注释已过期，新增命令时按现状就近放置即可。

## 2. `infra/` 模块清单

| 模块 | 职责 |
|---|---|
| `json` | 宽容 JSON（JSON5 风格）解析 |
| `def` | DEF 动画解码（头 / 分组表 / 帧头，format 0-3，palette[0] 色键透明） |
| `lod` | LOD 归档读取（zlib 条目） |
| `pak` | HD Edition `.pak` 容器解析（只读） |
| `image` | H3 PCX / 标准 PCX / BMP / DDS 解码与 WebP 编码 |
| `h3m` | 地图 / 战役头与语言解析 |
| `http` | 统一网络出口（跟随 302、ETag、镜像与代理回退） |
| `hash` | SHA-256 / FNV |
| `cache` | 文件指纹 + 进程内 LRU |
| `links` | 联接 / 符号链接的安全创建与移除 |
| `fs` | 目录复制（显式栈递归、进度回调节流、可取消）；首启用户数据迁移的底座（staging/rename 在 `vcmi::paths`） |
| `logging` | tracing 初始化 + 按天轮转 |
| `time` | UTC 日期与时间戳 |
| `version` | 点分版本号比较 |
| `idle` | Win32 `GetLastInputInfo` 空闲检测 |
| `metrics` | 线程局部性能埋点 |
| `upscale` | 可插拔放大算法注册表 |
| `sharpen` | USM 锐化 |
| `ai` | ONNX Real-ESRGAN 超分（外挂组件） |
| `window` | 首屏窗口几何 |
| `berror` | 结构化错误 `be_err(code, arg)` |

## 3. 并发与节流

- 百科批量构建用 rayon 局部池，**按块（默认 8 个 mod）重建池并重估空闲档位**：空闲 ≥5min 取 `cpus/2`、≥30s 取 `cpus/4`、否则 `cpus/8`（`modcache/batch.rs`）。
- 进度事件每 8 个 / 最后一个推送一次，避免事件风暴；前端另有 400ms 轮询兜底。
- 长时间操作统一用**取消哨兵**（`CANCELLED` 全局列表 / 逐文件查哨兵），不用线程中断。

## 4. 禁改清单（易踩坑）

| 位置 | 约束 |
|---|---|
| `lib.rs` | 配置补丁与删除补丁必须最早执行；日志初始化必须早于路径探测 |
| `lib.rs` | 命令注册表**每行一个命令**，否则 parity 脚本漏读 |
| `commands/settings.rs` | 磁盘 snake_case / 出参 camelCase / 入参 camelCase 三层各自冻结 |
| `vcmi/paths.rs` | `dirs.json` 必须写绝对路径 |
| `vcmi/setup.rs` | 解析失败必须报错，**禁止**降级为空对象覆写 |
| `vcmi/version.rs` | 核心 mod 的 `1.5` 不是引擎版本，切勿当作引擎版本 |
| `infra/http.rs` | 必须跟随 302；**禁止**按 Content-Type 判断成功 |
| `realms/activate.rs` | 停用必须恢复 `dirs.json` 原文备份 |
| `realms/transfer.rs` | 包 sha256 与内容 checksum 不得混用 |
| `updater/migrate.rs` | 能靠 bump 缓存版本号解决的绝不写迁移；迁移必须幂等且不阻断启动 |

## 5. 开放问题

- `realms/` 与 `repository/` 直接承载命令，与"commands 是唯一 IPC 壳"的设计意图不一致，是否回迁待定。
