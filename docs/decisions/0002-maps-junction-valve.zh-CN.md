# ADR 0002：地图屏蔽用 junction 阀门，懒执行

- 状态：已采纳（2026-09-04）
- 相关：[realms](../features/realms.zh-CN.md) §5、[vcmi-dirs](../reference/vcmi-dirs.zh-CN.md) §3

## 背景

VCMI 会同时挂载游戏目录的 `Maps/` 与用户目录的 `Maps/`，且**无法用 `dirs.json` 屏蔽游戏目录那一份**。领域需要"屏蔽游戏自带地图"的能力。

## 决策

一次性把游戏目录的 `Maps` 改名为 `Maps.vcmi-fv-native`，并建立 junction `Maps -> Maps_native`（默认态对用户完全透明）；开启"屏蔽游戏自带地图"时移除 junction。**懒执行**：首次开启时才改名，并保留还原入口（`repair_game_maps` 的 `revert`）。

## 理由

- VCMI 的目录遍历遵循 junction（boost `follow_directory_symlink`），因此 junction 对游戏完全透明；
- `mklink /J` 不需要管理员权限（符号链接需要特权，不可用）；
- 懒执行避免对所有用户都改动游戏目录。

## 后果

- ⚠️ **删除 junction 只能用 `remove_dir`**：`remove_dir_all` 会顺着联接删掉目标真实文件；
- 需要 `native_maps_status` / `repair_game_maps` 处理悬空联接、新自带地图合并等脏状态。
