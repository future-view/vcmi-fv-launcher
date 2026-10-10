# VCMI 目录机制（硬事实）

> 状态：实测结论 ｜ 最后核对：2026-09-30 ｜ 来源：VCMI 源码（1.7.5）与真实游戏目录实测
> 这些是**外部系统的行为**，不是本项目的设计选择；改动相关代码前先看本文。

## 1. `dirs.json`

- 位置：`<游戏目录>/config/dirs.json`。
- VCMI **只识别 5 个 key**：`userDataPath` / `userCachePath` / `userConfigPath` / `userLogsPath` / `userSavePath`。
- 写 `mods` / `config` / `maps` 等字段会被**静默忽略**并回退默认值——这是领域系统只能靠 5 个 key 做隔离的原因。

## 2. 路径解析

- 经 `ExpandEnvironmentStringsW` 展开，**只认 `%VAR%` 形式**。
- 相对路径按**进程工作目录**解析（`dataPaths()` 硬编码返回 `"."`），**不是**相对游戏目录 → 启动器写入时**必须用绝对路径**。
- 未显式指定的 `userCache` / `userConfig` / `userSave` / `userLogs` 全部由 `userDataPath` 派生：只切 `userDataPath` 等价于全切；要共享某项必须**显式写全局绝对路径**，省略字段无效。

## 3. 双重挂载（关键）

游戏目录的 `Mods/`、`Maps/` 与 `userDataPath` 下的 `Mods/`、`Maps/` 会**同时**挂载到虚拟 `MODS/` 与 `MAPS/` 命名空间，**无法通过 `dirs.json` 屏蔽游戏目录那一份**。

- 官方版：游戏目录无 `Maps`；
- 增强版：游戏目录有 164 张 h3m + 35 张 h3c。

→ 这是"地图阀门"存在的直接原因（见 [realms](../features/realms.zh-CN.md) §5）。

## 4. 目录创建

VCMI 只自动 `create_directories(userDataPath / cache / config / logs / Saves)`，**不会创建 `Mods` 和 `Maps` 子目录**，需要启动器自建。

## 5. 官方启动器的行为

- 下载的 mod 装到 `userDataPath/Mods`，地图装到 `userDataPath/Maps`。
- 不同发行版默认预设 `__official__` 依赖的中文包名字**无规范**：增强版（Heroes3-VCMI）装的是 `simplified-chinese-translation`，官方 VCMI 装的是 `chinese-translation-vcmi`，Expansion 1.83 则是 `ChineseTrueTypeFonts`——领域不"借用"它就会丢中文。**本启动器不再写死任何名字**，改由 `lib/langMods.ts` 按「已装模组的 `modType==Translation && language==当前语言`」分类发现（见 `docs/features/lang-mods.zh-CN.md`），领域场景按此口径决定是否借用。

## 6. 链接与联接

- 目录遍历遵循符号链接 / 目录联接（boost `follow_directory_symlink`），因此可以用 `cmd /C mklink /J` 建 junction（**不需要管理员**）；符号链接需要特权，不要用 `std::os::windows::fs::symlink_dir`。
- ⚠️ **删除 junction 只能用 `remove_dir`**：`remove_dir_all` 会顺着联接删掉目标真实文件。

## 7. 预设迁移只发生一次

`presets` 为空时才发生一次 1.5 → 预设格式的迁移（`ModManager.cpp` 的 `ModsPresetState`）。**不能反复写 1.5 的 `activeMods` 格式去触发迁移**。

## 8. 增强版的差异

增强版 `dirs.json` 使用 `%USERDOCUMENTS%`，而 VCMI 源码里**没有这个变量**——增强版是改过的构建。启动器 `vcmi/paths.rs` 的 `resolve_env_var` 已做兼容映射（**不要删除**）。

## 9. 引擎版本怎么读

exe **没有** PE 版本资源，`mod.json` 里的 `1.5` **不是**引擎版本。权威来源是 `VCMI_client.exe -v`。
