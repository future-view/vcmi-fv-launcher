/**
 * 原版 9 城镇（`core.<faction>` 伪模组）。
 *
 * 它们是 VCMI 引擎自带的，不会出现在领域的启用组合 `presets[activePreset]` 里
 * （`collectEnabledIds` 会把保留 scope 过滤掉），所以任何「按启用集合推导」的列表
 * —— 领域信息页（`InfoBody`）、阵营巡礼牌组（`buildShowcaseDeck`）——
 * 都必须**显式**把这 9 个并进去，它们才会出现。
 */
export const CORE_TOWN_IDS = [
  "core.castle",
  "core.conflux",
  "core.dungeon",
  "core.fortress",
  "core.inferno",
  "core.necropolis",
  "core.rampart",
  "core.stronghold",
  "core.tower",
];

/** 是否为原版 9 城镇之一。 */
export function isCoreTownId(id: string): boolean {
  return CORE_TOWN_IDS.includes(id);
}
