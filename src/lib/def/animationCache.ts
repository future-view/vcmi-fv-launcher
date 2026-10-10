/**
 * DEF 动画的按需加载 + LRU 缓存（纯 TS，无 React 依赖）。
 *
 * 背景（设计 §13.1 / §13.2）：
 * - 详情页打开时才取 DEF 原始字节（后端 `read_def_bytes` 跨 mod + 游戏目录 lod 解析），
 *   单个数 MB，每次打开都重传重解会造成明显卡顿；
 * - `decodeAnimation` 一次解出**全部分组**，主线程同步执行会掉帧。
 *
 * 故此处：
 * 1. 字节级 LRU 缓存 —— 同一 DEF 重复打开不再走 IPC；
 * 2. 分组级 LRU 缓存 —— 只解当前播放用到的动作分组（`decodeGroup`），切动作时按需补解。
 *
 * 容量刻意取小：单条字节可达数 MB，宁可淘汰重取也不要常驻占内存。
 */
import { readDefBytes } from "@/lib/tauri";
import { decodeGroup, type DefFrame } from "./decoder";

/** 缓存的 DEF 字节条目上限（单条可能数 MB） */
const MAX_BYTES_ENTRIES = 6;
/** 缓存的已解码分组条目上限 */
const MAX_GROUP_ENTRIES = 96;

const bytesCache = new Map<string, Uint8Array>();
const groupCache = new Map<string, DefFrame[]>();

const bytesKey = (modId: string, resource: string) => `${modId}\u0000${resource}`;
const groupKey = (modId: string, resource: string, groupIndex: number) =>
  `${modId}\u0000${resource}\u0000${groupIndex}`;

/** LRU 读取：命中后移到末尾（最近使用） */
function lruGet<T>(m: Map<string, T>, key: string): T | undefined {
  if (!m.has(key)) return undefined;
  const v = m.get(key) as T;
  m.delete(key);
  m.set(key, v);
  return v;
}

/** LRU 写入：超容量时淘汰最久未用者 */
function lruSet<T>(m: Map<string, T>, key: string, value: T, max: number): void {
  if (m.has(key)) m.delete(key);
  m.set(key, value);
  while (m.size > max) {
    const oldest = m.keys().next();
    if (oldest.done) break;
    m.delete(oldest.value);
  }
}

/** 取 DEF 原始字节（带 LRU 缓存）。失败返回 null。 */
export async function getDefBytes(
  modId: string,
  resource: string,
): Promise<Uint8Array | null> {
  if (!resource) return null;
  const key = bytesKey(modId, resource);
  const hit = lruGet(bytesCache, key);
  if (hit) return hit;
  try {
    const bytes = await readDefBytes(modId, resource);
    if (!bytes || bytes.length === 0) return null;
    const data = bytes instanceof Uint8Array ? bytes : new Uint8Array(bytes);
    lruSet(bytesCache, key, data, MAX_BYTES_ENTRIES);
    return data;
  } catch {
    return null;
  }
}

/** 取某 DEF 指定分组的帧（带分组级缓存，首次才解码）。 */
export async function getDefGroup(
  modId: string,
  resource: string,
  groupIndex: number,
): Promise<DefFrame[]> {
  const gKey = groupKey(modId, resource, groupIndex);
  const cached = lruGet(groupCache, gKey);
  if (cached) return cached;
  const data = await getDefBytes(modId, resource);
  if (!data) return [];
  let frames: DefFrame[];
  try {
    frames = decodeGroup(data, groupIndex);
  } catch {
    frames = [];
  }
  if (frames.length > 0) lruSet(groupCache, gKey, frames, MAX_GROUP_ENTRIES);
  return frames;
}

/** 清空缓存（切换领域/游戏目录后源数据可能变化时使用）。 */
export function clearDefAnimationCache(): void {
  bytesCache.clear();
  groupCache.clear();
}
