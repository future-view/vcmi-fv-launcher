/**
 * `.def` 动画原始字节与 HD 帧解码。
 *
 * 拆分自原 `@/lib/tauri.ts` 单体；对外仍由该 barrel 统一导出，调用方无需改动。
 */
import { invoke } from "@tauri-apps/api/core";
import { isTauri } from "./core";

/** 跨 mod 定位并返回 `.def` 动画文件的原始字节（Uint8Array），供前端直接解码绘制。 */
export async function readDefBytes(
  modId: string,
  resource: string,
): Promise<Uint8Array | null> {
  if (!isTauri()) return null;
  try {
    return await invoke<Uint8Array>("read_def_bytes", { modId, resource });
  } catch {
    return null;
  }
}

/** 高清动画的**单帧**：位置是画布像素坐标，字节是该帧的 webp。 */
export interface HdDefFrame {
  x: number;
  y: number;
  w: number;
  h: number;
  /** webp 字节（后端侧已把 HD 的独立阴影切片合进这一张） */
  data: number[] | Uint8Array;
}

/** 高清动画分组（后端 `read_def_group_hd`；`null` = 该组不可高清化，需回退 1x）。 */
export interface HdDefGroup {
  /** 画布尺寸 = 显示窗口（可能已自适应外扩）× 档位 */
  w: number;
  h: number;
  /**
   * 显示窗口左上角在 **DEF 逻辑画布**（原版 450×400）中的坐标。
   * 背景图（100×130）固定贴在基准窗口 `(150|170, 155)`，前端靠它把两层对齐。
   */
  wx?: number;
  wy?: number;
  /** 幕布窗口（1x 画布坐标）：两种布局之一，100×130 或 200×130 */
  winX?: number;
  winY?: number;
  winW?: number;
  winH?: number;
  /** 适配参考尺寸（`win ∪ 待机/移动/悬停/防御` 的包围盒） */
  fitW?: number;
  fitH?: number;
  /** 档位（2 / 3） */
  scale: number;
  frames: HdDefFrame[];
}

/**
 * 取 `.def` 某分组的动画帧（**高清优先、1x 兜底**，VCMI 显示窗口已由后端合成）。
 *
 * - `doubleWide`：双格生物（窗口左上角 x=170，否则 150）；
 * - `need`：该区域实际要占的**物理像素**（显示框 CSS × 屏幕物理倍率），决定取哪一档。
 *
 * 返回 `null` = 该组没有可播放的帧（前端回退 TS 解码 / 静态图）。
 */
export async function readDefGroupHd(
  modId: string,
  resource: string,
  group: number,
  doubleWide: boolean,
  need: number,
  kind?: string | null,
): Promise<HdDefGroup | null> {
  if (!isTauri()) return null;
  try {
    // 后端以**二进制**回传（`[4B LE 元数据长度][元数据 JSON][帧体...]`）：
    // 帧字节不再走 JSON 数字数组，省掉 3~4 倍载荷膨胀与逐元素转 `Uint8Array` 的开销。
    const buf = await invoke<ArrayBuffer>("read_def_group_hd", {
      modId,
      resource,
      group,
      doubleWide,
      need,
      kind: kind ?? null,
    });
    return unpackHdGroup(buf);
  } catch {
    return null;
  }
}

/**
 * 解开后端二进制分组载荷：`[4B LE 元数据长度][元数据 JSON][帧体...]`。
 *
 * 元数据 `frames[i]` 只含 `{x,y,w,h,len}`，帧字节紧跟在元数据之后 —— 这里只做
 * `Uint8Array` 视图切片（零拷贝），不再逐字节拼数组。`frames[i]` 的字节交给
 * `Blob`/`createImageBitmap` 时会被固化，视图生命周期没有问题。
 */
function unpackHdGroup(buf: ArrayBuffer | Uint8Array): HdDefGroup | null {
  // `invoke` 对二进制响应通常给 `ArrayBuffer`；某些运行时给 `Uint8Array`，两者都接受。
  const ab: ArrayBuffer =
    buf instanceof Uint8Array
      ? (buf.buffer.slice(buf.byteOffset, buf.byteOffset + buf.byteLength) as ArrayBuffer)
      : buf;
  if (!ab || ab.byteLength < 4) return null;
  const dv = new DataView(ab);
  const metaLen = dv.getUint32(0, true);
  if (metaLen === 0 || 4 + metaLen > ab.byteLength) return null;
  let meta: HdDefGroup & { frames?: { x: number; y: number; w: number; h: number; len: number }[] };
  try {
    meta = JSON.parse(new TextDecoder().decode(new Uint8Array(ab, 4, metaLen)));
  } catch {
    return null;
  }
  const list = meta.frames ?? [];
  if (list.length === 0) return null;
  let off = 4 + metaLen;
  const frames: HdDefFrame[] = [];
  for (const f of list) {
    const len = f.len | 0;
    if (len <= 0 || off + len > ab.byteLength) return null;
    frames.push({ x: f.x, y: f.y, w: f.w, h: f.h, data: new Uint8Array(ab, off, len) });
    off += len;
  }
  return { ...meta, frames };
}
