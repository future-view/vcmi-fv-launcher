/**
 * 纯数据辅助函数（不依赖 node:fs/path，可被打进浏览器 bundle）。
 * flattenMods 递归把根 mod 列表（含子 mod 树）展平为可勾选单元。
 */
import type { ModInfo } from "./modScanner";

export function flattenMods(rootMods: ModInfo[]): ModInfo[] {
  const out: ModInfo[] = [];
  const walk = (m: ModInfo) => {
    out.push(m);
    m.subs.forEach(walk);
  };
  rootMods.forEach(walk);
  return out;
}

/**
 * 将预设里的模组 ID 校正为实际已安装的 ID。
 * 1. 精确匹配 id
 * 2. 匹配磁盘目录名 diskName
 * 3. 归一化后匹配（忽略 -/_）
 * 都未命中则保留原 ID，用于后续缺失标记。
 */
export function resolvePresetIds(ids: string[], flat: ModInfo[]): string[] {
  const byId = new Map(flat.map((m) => [m.id, m]));
  const byDisk = new Map(flat.map((m) => [m.diskName.toLowerCase(), m]));
  const byNorm = new Map(
    flat.map((m) => [m.id.toLowerCase().replace(/[-_]/g, ""), m]),
  );

  return ids.map((id) => {
    if (byId.has(id)) return id;
    const disk = byDisk.get(id.toLowerCase());
    if (disk) return disk.id;
    const norm = byNorm.get(id.toLowerCase().replace(/[-_]/g, ""));
    if (norm) return norm.id;
    return id;
  });
}
