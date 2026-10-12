# 磁盘持久化布局

> 状态：已实现 ｜ 最后核对：2026-10-12 ｜ 代码：`src-tauri/src/vcmi/paths.rs`、`vcmi/config.rs`、`updater/migrate.rs`、`realms/base.rs`

所有启动器自己的数据都在**单个数据目录**下，游戏侧只改动 VCMI 既有文件。本文是"磁盘上到底写了什么"的唯一权威清单。

## 1. 数据根目录

**按平台分派**（ADR 0008 D10，2026-10-12 起；`vcmi/paths.rs` 的 `launcher_data_dir()`）：

| 平台 | 数据根 | 理由 |
|---|---|---|
| Windows | `<exe 所在目录>/.vcmi-fv-launcher/` | 便携单文件夹是既定特性（"解压覆盖"语义依赖它）；按 exe 路径推导、**不用进程 cwd**，任意工作目录启动都落同一处 |
| macOS | `~/Library/Application Support/com.vcmi.fv-launcher/` | 系统标准位置；`.app` 装进 `/Applications` 后包内写数据会破坏签名 |
| Linux | `${XDG_DATA_HOME:-~/.local/share}/vcmi-fv-launcher/` | XDG 规范；AppImage 挂载的是只读 squashfs，exe 同级不可写 |

- 禁止把数据目录放进游戏目录（会污染游戏、且被领域切换波及）。
- `launcher_config_path` / `mods_cache_dir` / `realm_vcmi_dir` / `downloads_dir` 全部由这一个函数派生——只改它即全链路切换。

## 2. `launcher.json` —— 启动器配置

按用途分组的 **snake_case**（2026-09-24 由扁平结构改来，**没有迁移层**：旧的扁平文件视为无效配置）。三层格式各自冻结：磁盘 snake_case / `read_launcher_config` 出参手写 camelCase / `write_launcher_config` 入参 camelCase。

| 分组 | 字段 | 说明 |
|---|---|---|
| `paths` | `game_dir`、`user_data_dir`、`user_config_dir`、`user_mods_dir`、`game_mods_dir`、`redirect_user_data` | 目录识别结果，绝对路径 |
| `ui` | `lang`、`window_size{width,height}`、`settings_level`（common/advanced/all）、`settings_show_desc`、`show_all_settings`、`town_preview` | 界面偏好 |
| `hd` | `enhance`、`ai{enhance,model,mix}`、`edition_use`、`edition_scale`、`usm` | 高清三链路开关，见 [hd-assets](../features/hd-assets.zh-CN.md) |
| `mods` | `required`、`builtin_presets[{id,label,mods,modsByLang,locked}]` | 必需模组与内置预设（可按 id 字段级合并） |
| `realms` | `active`、`repository_urls`、`mod_auto_check`、`publish_repo_dir`、`publish_url_base`、`publish_mode` | 领域系统全局项；领域本身元数据不在这里 |
| `update` | `manifest_url`、`auto_check`、`skip_version`、`prompt_version`、`prompt_at` | 自更新偏好与 7 天节流 |
| `runtime` | `log_level`、`components{<id>:{dir,package{url,sha256,size,version}}}` | 外挂组件声明 |
| `network` | `proxy`、`mirror_rules[{enabled,note,hosts,mode,prefix,template}]` | 网络出口，上限 10 条镜像规则 |
| `archives` | `zip_passwords{<mod目录名\|_default>: 密码}` | 加密 `content.zip` 解密密码表（发行版经随包 `launcher.json` 预置；保存配置时**保留既有值**、UI 不暴露写入），见 [distribution-build](../guides/distribution-build.zh-CN.md) |

**写入纪律**：`write_launcher_config` 走「宽容继承」回退（`read_launcher_config_lenient`），防止首次向导清掉便携包预置的语言 / 高清开关 / 组件地址。

## 3. `state.json` —— 运行态

字段：`lastRunVersion`、`appliedUpdateSha`、`appliedUpdateVersion`、`updatedAt`。

按 **sha256 而非版本号**记录"已应用的更新"，因为纯配置包不改 exe 版本。

## 4. `realms/<id>/` —— 领域

```
realms/<id>/
├── realm.json          唯一权威元数据（id/版本/作者/多语言名描述/标签/dirs/intro/showcase/mods/maps）
├── lock                编辑权标记：仅本机创建的领域有；导入的领域没有 → 只读
├── .introPlayedAt      登场演出上次播放时间（unix 秒）
├── .bgmMuted           BGM 静音
├── .prevActivePreset   进入领域前的 activePreset，退出时恢复
├── assets/**           壁纸/封面/视频/主题曲（intro-music / music / intro）
└── vcmi/               隔离后的 VCMI 用户目录
    ├── config/  cache/  logs/  Saves/  Mods/
    ├── .vcmi-fv-shared.json   配置共享标记
    ├── Maps.vcmi-fv-realm/    领域地图真相源（maps/, campaigns/, index.json）
    └── Maps/                  可视地图目录（全是 junction，按语言过滤挂载）
```

`lock`、`.introPlayedAt`、`.bgmMuted`、`.prevActivePreset` 与所有目录联接**不进导出包**。

## 5. `mods/` —— 百科缓存（modcache）

```
mods/
├── index.json                 已缓存清单（只合并不删除）
├── .hd-scheme                 HD 方案版本指纹
└── <modId>/
    ├── cache.json             投影数据（modMeta / resources / objects / 各类目条目）
    ├── raw.json               全量原始配置聚合（供幕布合成等复用）
    ├── audit.json             素材缺口审计（无缺口时删除）
    ├── hd-map.json            缓存图 → HD 资源名映射
    ├── .hd-src                HD 来源指纹
    ├── images/<sub>/*.webp    素材（creature/hero/building/artifact/spell/skill/terrain/town/townscreen/...）
    ├── images/**/*@Nx.webp    高清变体（与原图同目录）
    └── audio/*                阵营音乐（巡礼用）
```

原版数据以**伪模组**形式入缓存：`core`（无归属实体）与 `core.<faction>`（9 个原版城镇）。详见 [modcache](../features/modcache.zh-CN.md)。

## 6. `cache/` —— 远端数据缓存

| 子目录 | 内容 |
|---|---|
| `realmIndex/<urlHash>.json` | 远程领域仓库索引（TTL 30 分钟 + ETag） |
| `realmCovers/<id>-<version>.<ext>` | 领域封面（后端下载，规避部分站点的 Referer 403） |
| `modIndex/<urlHash>.json` | 模组仓库索引 |
| `modVersions.json` | 模组版本台账 |
| `modPackages/<modId>__<urlKey16>.zip` | 模组包缓存（2GB LRU） |
| `updateManifest/manifest.json` | 启动器更新清单（TTL 24 小时 + ETag） |

## 7. 其余目录

| 路径 | 用途 |
|---|---|
| `downloads/` | 临时下载：领域包、模组包、更新包（`VCMI-FV-Launcher_<ver>.{zip,exe.update}`）、`staged_<ver>/` |
| `logs/vcmi-fv.YYYY-MM-DD[.N].log` | 按天轮转日志 |
| `runtime/<id>/` | 外挂组件（当前只有 `ai`：onnxruntime dll + 可选模型） |
| `.tmp-runtime/` | 组件安装暂存 |

**一次性文件**：`launcher.patch.json`（随包预置 / 更新携带的配置补丁，合并后删除）、`launcher.delete.json`（删除清单，处理后删除）、`update-apply.ps1`（自替换脚本）、`update-apply.log`（自替换现场日志）。

## 8. 游戏侧文件

| 文件 | 启动器如何对待 |
|---|---|
| `<游戏目录>/config/dirs.json` | 领域激活时改写 5 个 key（userDataPath/userConfigPath/userCachePath/userLogsPath/userSavePath），首次改写前备份为 `dirs.json.vcmi-fv-backup`；停用必须**还原备份原文**，不能写"官方默认路径" |
| `<userConfig>/settings.json` | VCMI 官方设置，读-合并-写，写前备份 `.json.backup` |
| `<userConfig>/modSettings.json` | 预设载体，写前备份 `.json.backup` |
| `Mods/`、`Maps/` | 游戏目录与用户目录两份**同时**挂载到 VCMI 命名空间，无法通过 dirs.json 屏蔽游戏目录那一份（这是地图阀门存在的原因） |

## 9. 写盘纪律（易踩坑）

1. `dirs.json` 一律写**绝对路径**（VCMI 按进程工作目录解析相对路径）。
2. 删除目录联接（junction）**只能用 `remove_dir`**；`remove_dir_all` 会顺着联接删掉目标真实文件。
3. 任何覆盖用户数据的写操作先备份；`write_mod_settings`、`write_vcmi_settings` 均已内置。
4. 更新包解压白名单拒绝覆盖 `launcher.json`、`realms/`、`mods/`（用户数据）。
