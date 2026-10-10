# ADR 0001：领域隔离靠改写 `dirs.json`

- 状态：已采纳（2026-09-04）
- 相关：[realms](../features/realms.zh-CN.md)、[vcmi-dirs](../reference/vcmi-dirs.zh-CN.md)

## 背景

需要多套互不干扰的 VCMI 环境（不同模组组合 + 各自存档与设置），但不能修改 VCMI 源码。

## 决策

隔离完全靠改写 `<游戏目录>/config/dirs.json` 的 5 个真实 key（`userDataPath` / `userConfigPath` / `userCachePath` / `userLogsPath` / `userSavePath`），指向 `{realm-data}/realms/<id>/vcmi/`。数据根目录默认 `%LOCALAPPDATA%\VCMI-FV-Launcher\`，**禁止放游戏目录**。

## 理由

- VCMI 官方只支持这 5 个 key（写其它字段会被静默忽略），这是唯一无需改源码的切目录手段；
- 放在游戏目录会污染游戏分发，且会被领域切换波及。

## 后果

- 还原时必须写回**原文备份**，不能写"官方默认路径"（增强版默认是 `My Games\vcmi`）；
- 备份只在首次创建时生成，切换不覆盖；
- 未显式指定的 4 个 key 由 `userDataPath` 派生，要共享某项必须显式写全局绝对路径。
