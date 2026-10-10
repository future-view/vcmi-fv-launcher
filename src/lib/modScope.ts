/**
 * VCMI 内置/保留 scope。
 * 这些模组由 VCMI 引擎自带（官方启动器不在列表中显示），但保存预设时仍需写入 modSettings.json。
 */
export const RESERVED_MOD_IDS = new Set(["vcmi", "core"]);

export function isReservedMod(id: string): boolean {
  return RESERVED_MOD_IDS.has(id);
}
