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
import { getHdPhys } from "@/lib/hd";
import { readDefBytes, readDefGroupHd, type HdDefGroup } from "@/lib/tauri";
import { decodeGroup, type DefFrame } from "./decoder";

/** 缓存的 DEF 字节条目上限（单条可能数 MB） */
const MAX_BYTES_ENTRIES = 6;
/** 缓存的已解码分组条目上限 */
const MAX_GROUP_ENTRIES = 96;

const bytesCache = new Map<string, Uint8Array>();
const groupCache = new Map<string, DefFrame[]>();
/** 高清分组缓存（**已解码的 ImageBitmap**，必须 `close()` 才能回收显存） */
const hdCache = new Map<string, HdGroup>();

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

/** LRU 写入：超容量时淘汰最久未用者（`dispose` 用于释放非 GC 资源，如 ImageBitmap） */
function lruSet<T>(
  m: Map<string, T>,
  key: string,
  value: T,
  max: number,
  dispose?: (v: T) => void,
): void {
  if (m.has(key)) m.delete(key);
  m.set(key, value);
  while (m.size > max) {
    const oldest = m.keys().next();
    if (oldest.done) break;
    const evicted = m.get(oldest.value);
    m.delete(oldest.value);
    if (evicted !== undefined) dispose?.(evicted);
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

/**
 * **高清**动画分组：帧已在后端合成好（含 HD 独立阴影），前端只需按 `x/y` 贴到画布。
 *
 * 与 1x 的 [`DefFrame`] 是两套绘制路径（位图 vs BGRA 逐像素），但**共用同一播放器**：
 * 见 [`getAnimGroup`] 的统一包装。
 */
export interface HdGroup {
  w: number;
  h: number;
  scale: number;
  /** 窗口左上角（DEF 逻辑画布坐标） */
  wx: number;
  wy: number;
  /** 幕布窗口（1x 画布坐标） */
  winX: number;
  winY: number;
  winW: number;
  winH: number;
  /** 适配参考尺寸（决定等比缩放，不含死亡/攻击的越界） */
  fitW: number;
  fitH: number;
  frames: { x: number; y: number; w: number; h: number; img: ImageBitmap }[];
}

/**
 * 一个可播放的动作分组：**高清优先**，不可高清化时是 1x 的 BGRA 帧。
 *
 * 播放器按 `hd` 是否为 null 分派绘制（`drawImage` / `putImageData`）—— 两条路径始终并存，
 * 高清缺失 / 未装 hd-edition / 第三方生物都走 1x，行为与加高清前完全一致。
 */
export interface AnimGroup {
  /** 画布宽（像素）= 显示窗口（可能已自适应外扩）× 档位 */
  w: number;
  h: number;
  /** 显示窗口左上角在 DEF 逻辑画布中的坐标（背景对齐用，缺省为基准窗口） */
  wx: number;
  wy: number;
  /** 档位（2 / 3；1x 与 TS 兜底恒 1）。`w / scale` = 窗口的 1x 逻辑宽度。 */
  scale: number;
  /** 幕布窗口（1x 画布坐标）：两种布局之一，100×130 或 200×130 */
  winX: number;
  winY: number;
  winW: number;
  winH: number;
  /** 适配参考尺寸（`win ∪ 待机/移动/悬停/防御`）：决定等比缩放 */
  fitW: number;
  fitH: number;
  /** 高清帧（含位置）；`null` = 走 1x */
  hd: HdGroup["frames"] | null;
  /** 1x 帧（BGRA，画在 0,0）；`hd` 非空时为 null */
  raw: DefFrame[] | null;
  count: number;
}

/**
 * 取某动作分组（**高清优先，1x 兜底**），带分组级缓存。
 *
 * 顺序不可颠倒：先问 `read_def_group_hd`，它返回 null 才解码 1x ——
 * 后端已保证"整组命中或整组回退"，前端不做逐帧混档判断。
 *
 * `boxCss`：动画框的 CSS 边长（130/180）。它与屏幕物理倍率一起决定后端取哪一档高清
 * （1080p 大图 x3、弹窗 x2、4K 封顶 x3）—— **构图恒定 130 源像素窗口，只有分辨率随屏幕变**。
 */
export async function getAnimGroup(
  modId: string,
  resource: string,
  groupIndex: number,
  doubleWide: boolean,
  boxCss: number,
): Promise<AnimGroup | null> {
  const need = Math.round(boxCss * (await getHdPhys()));
  // 档位不同 → 产物不同，缓存键必须带上 `need`
  const gKey = `${groupKey(modId, resource, groupIndex)}\u0000${need}`;
  // 1x / TS 兜底都按**基准窗口**画（后端没参与，拿不到自适应窗口）
  const baseWin = (w: number, h: number) => ({
    w,
    h,
    wx: doubleWide ? 170 : 150,
    wy: 155,
    scale: 1,
    winX: doubleWide ? 170 : 150,
    winY: 155,
    winW: 100,
    winH: 130,
    fitW: 100,
    fitH: 130,
  });
  const hdHit = lruGet(hdCache, gKey);
  if (hdHit) {
    return {
      w: hdHit.w,
      h: hdHit.h,
      wx: hdHit.wx,
      wy: hdHit.wy,
      scale: hdHit.scale,
      winX: hdHit.winX,
      winY: hdHit.winY,
      winW: hdHit.winW,
      winH: hdHit.winH,
      fitW: hdHit.fitW,
      fitH: hdHit.fitH,
      hd: hdHit.frames,
      raw: null,
      count: hdHit.frames.length,
    };
  }
  const rawHit = lruGet(groupCache, groupKey(modId, resource, groupIndex));
  if (rawHit && rawHit.length > 0) {
    return {
      ...baseWin(rawHit[0].w, rawHit[0].h),
      hd: null,
      raw: rawHit,
      count: rawHit.length,
    };
  }
  // 1) 后端合成（HD 优先，未收录时后端自己走 1x 同口径窗口）
  const hd = await fetchHdGroup(modId, resource, groupIndex, doubleWide, need);
  if (hd) {
    lruSet(hdCache, gKey, hd, MAX_HD_GROUP_ENTRIES, closeHdGroup);
    return {
      w: hd.w,
      h: hd.h,
      wx: hd.wx,
      wy: hd.wy,
      scale: hd.scale,
      winX: hd.winX,
      winY: hd.winY,
      winW: hd.winW,
      winH: hd.winH,
      fitW: hd.fitW,
      fitH: hd.fitH,
      hd: hd.frames,
      raw: null,
      count: hd.frames.length,
    };
  }
  // 2) 最后兜底：前端 TS 解码（命令整体失败时才会走到，构图与后端不完全一致）
  const frames = await getDefGroup(modId, resource, groupIndex);
  if (frames.length === 0) return null;
  return { ...baseWin(frames[0].w, frames[0].h), hd: null, raw: frames, count: frames.length };
}

/**
 * 高清分组条目上限（单条含十余张 300×390 位图，约 4~6MB）。
 *
 * 4 个状态合计会触碰约 12 个组（待机 3 / 移动 5 / 攻击 4 / 受击 3），
 * 留 4 个余量避免在状态间切换时反复 close/reopen。
 */
const MAX_HD_GROUP_ENTRIES = 16;

/** 释放一组高清帧的显存（ImageBitmap 不受 GC 管辖，必须显式 close） */
function closeHdGroup(g: HdGroup): void {
  g.frames.forEach((f) => f.img.close());
}

/** 取并解码高清分组（webp 字节 → ImageBitmap）；任一环节失败 → null。 */
async function fetchHdGroup(
  modId: string,
  resource: string,
  groupIndex: number,
  doubleWide: boolean,
  need: number,
): Promise<HdGroup | null> {
  if (typeof createImageBitmap !== "function") return null;
  let g: HdDefGroup | null = null;
  try {
    g = await readDefGroupHd(modId, resource, groupIndex, doubleWide, need);
  } catch {
    return null;
  }
  if (!g || !g.frames || g.frames.length === 0 || g.w <= 0 || g.h <= 0) return null;
  const frames: HdGroup["frames"] = [];
  try {
    for (const f of g.frames) {
      const bytes = f.data instanceof Uint8Array ? f.data : new Uint8Array(f.data);
      // 拷进独立 ArrayBuffer：`BlobPart` 不接受 `Uint8Array<ArrayBufferLike>`（TS 5.7+ 泛型），
      // 且 IPC 传来的视图可能指向共享内存，交给解码器前先固化成普通 ArrayBuffer。
      const ab = new ArrayBuffer(bytes.length);
      new Uint8Array(ab).set(bytes);
      const img = await createImageBitmap(new Blob([ab], { type: "image/webp" }));
      frames.push({ x: f.x, y: f.y, w: f.w, h: f.h, img });
    }
  } catch {
    // 解码到一半失败：已解出的必须关闭，且**不得**缓存半成品
    frames.forEach((f) => f.img.close());
    return null;
  }
  return {
    w: g.w,
    h: g.h,
    scale: g.scale ?? 1,
    wx: g.wx ?? (doubleWide ? 170 : 150),
    wy: g.wy ?? 155,
    winX: g.winX ?? (doubleWide ? 170 : 150),
    winY: g.winY ?? 155,
    winW: g.winW ?? 100,
    winH: g.winH ?? 130,
    fitW: g.fitW ?? 100,
    fitH: g.fitH ?? 130,
    frames,
  };
}

/** 清空缓存（切换领域/游戏目录后源数据可能变化时使用）。 */
export function clearDefAnimationCache(): void {
  bytesCache.clear();
  groupCache.clear();
  for (const g of hdCache.values()) g.frames.forEach((f) => f.img.close());
  hdCache.clear();
}
