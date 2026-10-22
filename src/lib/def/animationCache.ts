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
/**
 * 1x / TS 兜底分组缓存上限。
 *
 * 单组是**整幅 450×400 BGRA**（约 720KB/帧），原值 96 在极端情况下可占数百 MB；
 * 而这条路径只在后端命令整体失败时才走到，一个状态的组数不超过 5，故收到 8。
 */
const MAX_GROUP_ENTRIES = 8;

const bytesCache = new Map<string, Uint8Array>();
const groupCache = new Map<string, DefFrame[]>();
/** 高清分组缓存项：位图 + **估算字节数**（用于按预算淘汰，见 `MAX_HD_BYTES`） */
interface HdEntry {
  g: HdGroup;
  bytes: number;
}
/** 高清分组缓存（**已解码的 ImageBitmap**，必须 `close()` 才能回收显存） */
const hdCache = new Map<string, HdEntry>();
/** 当前已占用的位图字节（估算 = 每帧 `w*h*4`） */
let hdBytes = 0;
/** 缓存代数：每次整体释放 +1，播放器据此丢弃已 `close()` 的位图（见 `AnimGroup.gen`）。 */
let cacheGen = 0;

/** 当前缓存代数（见 `AnimGroup.gen`）。 */
export const getAnimCacheGen = (): number => cacheGen;

/** 后端档位 = `ceil(need / 100)`（`resource.rs::WIN_W`）；产物尺寸只由档位决定。 */
const scaleOf = (need: number) => Math.max(1, Math.ceil(need / 100));

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
  /**
   * 缓存代数。全局缓存被释放（切领域 / 空闲回收）后递增，
   * 播放器据此丢弃手里已失效的位图（`ImageBitmap` 已被 `close()`）。
   */
  gen: number;
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
  kind?: string | null,
): Promise<AnimGroup | null> {
  touchIdle();
  const need = Math.round(boxCss * (await getHdPhys()));
  // 产物尺寸只由**档位**决定（后端 `ceil(need/100)`），故缓存键按档位归一：
  // 窗口尺寸/倍率微调（need 变几十像素）不会再让整组缓存失效重取。
  const gKey = `${groupKey(modId, resource, groupIndex)}\u0000s${scaleOf(need)}`;
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
    const g = hdHit.g;
    return {
      w: g.w,
      h: g.h,
      wx: g.wx,
      wy: g.wy,
      scale: g.scale,
      winX: g.winX,
      winY: g.winY,
      winW: g.winW,
      winH: g.winH,
      fitW: g.fitW,
      fitH: g.fitH,
      hd: g.frames,
      raw: null,
      count: g.frames.length,
      gen: cacheGen,
    };
  }
  const rawHit = lruGet(groupCache, groupKey(modId, resource, groupIndex));
  if (rawHit && rawHit.length > 0) {
    // 1x 帧（整幅 BGRA）预转成 ImageBitmap：播放时只剩 drawImage，
    // 不再每 100ms 在主线程做一次逐像素 BGRA→RGBA + `putImageData`。
    const bm = await bitmapsFromRaw(rawHit);
    if (bm && bm.length > 0) {
      return { ...baseWin(bm[0].w, bm[0].h), hd: bm, raw: null, count: bm.length, gen: cacheGen };
    }
    return {
      ...baseWin(rawHit[0].w, rawHit[0].h),
      hd: null,
      raw: rawHit,
      count: rawHit.length,
      gen: cacheGen,
    };
  }
  // 1) 后端合成（HD 优先，未收录时后端自己走 1x 同口径窗口）
  //    同一 `gKey` 只发一次：预取与播放循环会并发请求同一组。
  let pending = inflightHd.get(gKey);
  if (!pending) {
    pending = fetchHdGroup(modId, resource, groupIndex, doubleWide, need, kind ?? null);
    inflightHd.set(gKey, pending);
    const done = () => inflightHd.delete(gKey);
    void pending.then(done, done);
  }
  const hd = await pending;
  if (hd) {
    putHdCache(gKey, hd);
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
      gen: cacheGen,
    };
  }
  // 2) 最后兜底：前端 TS 解码（命令整体失败时才会走到，构图与后端不完全一致）
  const frames = await getDefGroup(modId, resource, groupIndex);
  if (frames.length === 0) return null;
  const bm = await bitmapsFromRaw(frames);
  if (bm && bm.length > 0) {
    return { ...baseWin(bm[0].w, bm[0].h), hd: bm, raw: null, count: bm.length, gen: cacheGen };
  }
  return {
    ...baseWin(frames[0].w, frames[0].h),
    hd: null,
    raw: frames,
    count: frames.length,
    gen: cacheGen,
  };
}

/**
 * 高清分组条目上限（单条含十余张 300×390 位图，约 4~6MB）。
 *
 * 原值是 16，按"4 个状态约 12 个组"估算；但攻击/射击/施法各有 3 个方向组
 * （ATTACK 11/12/13、SHOOT 14/15/16、CAST 30/31/32），四态真实工作集约 20~23 组。
 * 容量小于工作集时，状态间来回切换会不断淘汰重取 —— 每次重取都是一次后端合成往返，
 * 正是"每个动画状态都要等"的直接来源。故取 32：覆盖全部四态并留余量。
 *
 * 真正的内存闸门是 [`MAX_HD_BYTES`]：条数只是兜底，避免"小图很多"时条数失控。
 */
const MAX_HD_GROUP_ENTRIES = 32;

/**
 * 位图缓存**字节预算**（估算 = 每帧 `w*h*4`，ImageBitmap 解码后按 RGBA 计）。
 *
 * 为什么需要预算限流：32 组 × 十余帧 × 300×390 位图 ≈ 150MB，且 `ImageBitmap` 不受 GC
 * 管辖（只能显式 `close()`）。不设预算时"看完几十个兵种"会把整个四态工作集留在显存里，
 * 拖慢后续渲染。96MB 足够容纳一个生物的完整四态（约 20 组 × 4.7MB ≈ 94MB），
 * 超出即淘汰最久未用的组。
 */
const MAX_HD_BYTES = 96 * 1024 * 1024;

/** 空闲多久后自动释放（用户停止浏览百科即回收，避免切走页面后仍占显存）。 */
const IDLE_RELEASE_MS = 60_000;
let idleTimer: number | null = null;

/**
 * 每次取组时重置空闲计时器：60 秒没有任何动画请求 → 释放全部位图。
 *
 * ⚠️ 页面隐藏时**不启动**计时器（并清掉已有的）：后台标签页的 `requestAnimationFrame`
 * 已暂停，恢复时播放器仍会 `drawImage` 这些 `ImageBitmap` —— 提前 `close()` 会让恢复后
 * 第一帧抛 `InvalidStateError`。后台的内存回收交给"离开百科时释放"。
 */
function touchIdle(): void {
  if (typeof window === "undefined") return;
  if (typeof document !== "undefined" && document.visibilityState === "hidden") {
    if (idleTimer !== null) {
      window.clearTimeout(idleTimer);
      idleTimer = null;
    }
    return;
  }
  if (idleTimer !== null) window.clearTimeout(idleTimer);
  idleTimer = window.setTimeout(() => {
    idleTimer = null;
    releaseDefAnimationCache();
  }, IDLE_RELEASE_MS);
}

/** 正在取/解码中的高清分组：预取与播放若同时命中同一组，共用一次后端往返。 */
const inflightHd = new Map<string, Promise<HdGroup | null>>();

/** 释放一组高清帧的显存（ImageBitmap 不受 GC 管辖，必须显式 close） */
function closeHdGroup(g: HdGroup): void {
  g.frames.forEach((f) => f.img.close());
}

/** 一组位图的估算字节数（按 RGBA 计）。 */
function hdEntryBytes(g: HdGroup): number {
  return g.frames.reduce((n, f) => n + f.w * f.h * 4, 0);
}

/** 写入高清缓存：按「条数 + 字节预算」双闸门淘汰，淘汰即 `close()`。 */
function putHdCache(key: string, g: HdGroup): void {
  const prev = hdCache.get(key);
  if (prev) {
    // ⚠️ 同一个对象被重复写入（预取与播放并发、共享一次 inflight 请求）时**必须幂等**：
    // 否则这里 `closeHdGroup(prev.g)` 会把刚交给播放器的那份 ImageBitmap 关掉，
    // 播放器下一帧 `drawImage` 直接抛 `InvalidStateError: The image source is detached`。
    if (prev.g === g) return;
    hdBytes -= prev.bytes;
    closeHdGroup(prev.g);
    hdCache.delete(key);
  }
  const bytes = hdEntryBytes(g);
  hdCache.set(key, { g, bytes });
  hdBytes += bytes;
  while ((hdCache.size > MAX_HD_GROUP_ENTRIES || hdBytes > MAX_HD_BYTES) && hdCache.size > 1) {
    const oldest = hdCache.keys().next();
    if (oldest.done) break;
    const ev = hdCache.get(oldest.value);
    hdCache.delete(oldest.value);
    if (!ev) continue;
    hdBytes -= ev.bytes;
    closeHdGroup(ev.g);
  }
}

/**
 * 1x 帧（整幅 BGRA）→ `ImageBitmap`。
 *
 * 1x 兜底路径原来是"每 100ms 在主线程做一次逐像素 BGRA→RGBA + `putImageData`"，
 * 播放期间持续占用主线程。这里在取回时**一次性**转换，播放器统一走 `drawImage`。
 */
async function bitmapsFromRaw(
  frames: DefFrame[],
): Promise<HdGroup["frames"] | null> {
  if (typeof createImageBitmap !== "function") return null;
  const out: HdGroup["frames"] = [];
  try {
    for (const f of frames) {
      const rgba = new Uint8ClampedArray(f.w * f.h * 4);
      const src = f.bgra;
      for (let i = 0; i < rgba.length; i += 4) {
        rgba[i] = src[i + 2];
        rgba[i + 1] = src[i + 1];
        rgba[i + 2] = src[i];
        rgba[i + 3] = src[i + 3];
      }
      const img = await createImageBitmap(new ImageData(rgba, f.w, f.h));
      out.push({ x: 0, y: 0, w: f.w, h: f.h, img });
    }
    return out;
  } catch {
    out.forEach((f) => f.img.close());
    return null;
  }
}

/** 取并解码高清分组（webp 字节 → ImageBitmap）；任一环节失败 → null。 */
async function fetchHdGroup(
  modId: string,
  resource: string,
  groupIndex: number,
  doubleWide: boolean,
  need: number,
  kind?: string | null,
): Promise<HdGroup | null> {
  if (typeof createImageBitmap !== "function") return null;
  let g: HdDefGroup | null = null;
  try {
    g = await readDefGroupHd(modId, resource, groupIndex, doubleWide, need, kind ?? null);
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

/**
 * 释放全部动画缓存（**保留**在途请求，它们完成时自会重新入缓存）。
 *
 * 调用时机：
 * - 空闲 60s（见 [`touchIdle`]）；
 * - 离开百科 / 切换分类 / 切换领域（见 `WikiView`）；
 * - 页面隐藏（`visibilitychange`）。
 *
 * `ImageBitmap` 不受 GC 管辖，必须逐个 `close()` 才能真正回收显存。
 */
export function releaseDefAnimationCache(): void {
  bytesCache.clear();
  groupCache.clear();
  for (const e of hdCache.values()) closeHdGroup(e.g);
  hdCache.clear();
  hdBytes = 0;
  // 手里还持有旧 `AnimGroup` 的播放器据此丢弃（位图已 close，再 draw 会抛错）
  cacheGen += 1;
}

/** 清空缓存（切换领域/游戏目录后源数据可能变化时使用；在途请求也一并作废）。 */
export function clearDefAnimationCache(): void {
  releaseDefAnimationCache();
  inflightHd.clear();
}
