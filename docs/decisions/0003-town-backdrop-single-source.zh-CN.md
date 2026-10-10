# ADR 0003：城镇幕布图以 `build_towns` 产物为唯一来源

- 状态：已采纳（2026-09-23）
- 相关：[modcache](../features/modcache.zh-CN.md) §6

## 背景

`factions[].background`（`images/town/<fid>.webp`，由 `build_towns` 生成）与 `factions[].preview`（`images/townscreen/<fid>.webp`，由 `compose_town_preview_for` 生成）此前各合成一份、各落一份盘。实测 84 个城镇：townscreen 42.5MB / town 47.4MB，仅 14 张（7MB）字节相同，**70 张不同**。

根因是两条路径取数口径分叉：`build_towns` 的 structures/buildings 是跨文件并集、底图优先 `town.townBackground`；`preview` 只取第一个命中文件、底图优先 `clientInfo.townScreen.background`。

## 决策

以 `build_towns` 产物为唯一幕布图来源：`preview` 直接复用 `background`；只有 `background` 为空时才兜底调 `compose_town_preview_for` 写 `townscreen/`。

## 理由

视觉抽查（ark-town、crossroads-of-worlds.Fen）确认 `build_towns` 口径**更完整**——`preview` 缺整组城塔 / 冰洞建筑。

## 后果

- `ASSETS_VERSION` 递增触发重建（会清空 `images/` 并重跑阶段二）；
- HD 的 `hd_target_box` 保留 townscreen 条目不动。
