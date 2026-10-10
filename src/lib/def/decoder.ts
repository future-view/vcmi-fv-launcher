// VCMI / HoMM3 `.def` 动画解码器（移植自 src-tauri/src/def.rs，纯 TS 实现）。
// 算法固定：文件头 784 + 768 调色板 → 分组表 → 帧头 32 字节 → format 0/1/2/3 解码 → 调色板映射 BGRA。

const SSPRITE_DEF_SIZE = 32;
// 色键规则（对齐 VCMI CSDL_Ext::setColorKey(palette->colors[0])）：
// 调色板第 0 项的 RGB 颜色即为色键；帧内与该 RGB 相同的像素全部透明。
// 不可把索引 0-7 全抠，H3 调色板 1-7 是常用主体色。

export interface DefFrame {
  w: number;
  h: number;
  bgra: Uint8Array; // 顶向下 BGRA，透明色 alpha=0
}

export interface DefGroup {
  group: number;
  frames: DefFrame[];
}

function readU32(data: Uint8Array, off: number): number {
  if (off + 4 > data.length) return 0;
  return (
    (data[off] |
      (data[off + 1] << 8) |
      (data[off + 2] << 16) |
      (data[off + 3] << 24)) >>>
    0
  );
}

interface DefHeader {
  totalBlocks: number;
  palette: Uint8Array; // 768 字节
  groups: number[][]; // 每个分组的帧偏移列表
}

function parseHeader(data: Uint8Array): DefHeader | null {
  if (data.length < 784 + 768) return null;
  const totalBlocks = readU32(data, 12);
  const palette = data.slice(16, 16 + 768);
  const fileSize = data.length;
  let pos = 784;
  const groups: number[][] = [];
  const maxBlocks = Math.min(totalBlocks, 256);
  let blockHeaderSize: number | null = null;

  for (let _ = 0; _ < maxBlocks; _++) {
    if (pos + 8 > fileSize) break;
    const blockId = readU32(data, pos);
    const totalEntries = readU32(data, pos + 4);
    if (totalEntries === 0 || totalEntries > 10000) {
      const hdr = blockHeaderSize ?? 16;
      pos += hdr;
      if (blockHeaderSize !== null) {
        if (pos + totalEntries * 17 > fileSize) break;
        pos += totalEntries * 17;
      }
      continue;
    }

    if (blockHeaderSize === null) {
      let bestHdr = 16;
      let bestScore = -1;
      for (const testHdr of [16, 20]) {
        const ns = pos + testHdr;
        if (ns + 12 > fileSize) continue;
        const osOff = ns + totalEntries * 13;
        if (osOff + totalEntries * 4 > fileSize) continue;
        let validCnt = 0;
        for (let j = 0; j < totalEntries; j++) {
          const o = readU32(data, osOff + j * 4);
          if (o >= 32 && o < fileSize) validCnt++;
        }
        const nameRaw = data.slice(ns, Math.min(ns + 13, fileSize));
        let asciiScore = 0;
        for (const b of nameRaw) if (b >= 32 && b < 127) asciiScore++;
        const score = validCnt + asciiScore / 100;
        if (score > bestScore) {
          bestScore = score;
          bestHdr = testHdr;
        }
      }
      blockHeaderSize = bestHdr;
    }
    const hdr = blockHeaderSize;
    pos += hdr;
    const namesSize = totalEntries * 13;
    const offsetsSize = totalEntries * 4;
    if (pos + namesSize + offsetsSize > fileSize) break;
    pos += namesSize; // 跳过 13 字节/条目的名字
    const offsets: number[] = [];
    for (let _ = 0; _ < totalEntries; _++) {
      offsets.push(readU32(data, pos));
      pos += 4;
    }
    const valid = offsets.filter((o) => o >= 32 && o + 32 <= fileSize);
    if (valid.length > 0) {
      groups.push(valid);
      void blockId;
    }
  }

  return { totalBlocks, palette, groups };
}

interface RawFrame {
  fmt: number;
  w: number;
  h: number;
  pixel: Uint8Array;
}

function readFrame(data: Uint8Array, offset: number): RawFrame | null {
  const off = offset;
  if (off + SSPRITE_DEF_SIZE > data.length) return null;
  const frameSize = readU32(data, off);
  const fmt = readU32(data, off + 4);
  const fw = readU32(data, off + 8);
  const fh = readU32(data, off + 12);
  const w = readU32(data, off + 16);
  const h = readU32(data, off + 20);
  if (fmt > 3 || w === 0 || h === 0 || w > 4096 || h > 4096) return null;
  const pixelOff =
    fmt === 1 && w > fw && h > fh ? off + SSPRITE_DEF_SIZE - 16 : off + SSPRITE_DEF_SIZE;
  const pixelEnd = frameSize === 0 ? pixelOff : Math.min(off + frameSize, data.length);
  const pixel = data.slice(pixelOff, pixelEnd);
  return { fmt, w, h, pixel };
}

function decodePixels(fmt: number, w: number, h: number, data: Uint8Array): Uint8Array {
  switch (fmt) {
    case 0:
      return decodeFormat0(data, w, h);
    case 1:
      return decodeFormat1(data, w, h);
    case 2:
      return decodeFormat2(data, w, h);
    case 3:
      return decodeFormat3(data, w, h);
    default:
      return decodeFormat0(data, w, h);
  }
}

function decodeFormat0(data: Uint8Array, w: number, h: number): Uint8Array {
  const out = new Uint8Array(w * h);
  let i = 0;
  for (let r = 0; r < h; r++) {
    for (let c = 0; c < w; c++) {
      out[r * w + c] = data[i] ?? 0;
      i++;
    }
  }
  return out;
}

function decodeFormat1(data: Uint8Array, w: number, h: number): Uint8Array {
  if (data.length < h * 4) return decodeFormat0(data, w, h);
  const out = new Uint8Array(w * h);
  const dataLen = data.length;
  for (let rowIdx = 0; rowIdx < h; rowIdx++) {
    const base = rowIdx * 4;
    let pos = (data[base] | (data[base + 1] << 8) | (data[base + 2] << 16) | (data[base + 3] << 24)) >>> 0;
    let c = 0;
    while (c < w && pos < dataLen) {
      if (pos + 2 > dataLen) break;
      const segType = data[pos];
      const length = Math.min(data[pos + 1] + 1, w - c);
      pos += 2;
      if (segType === 0xff) {
        const avail = Math.min(dataLen - pos, length);
        for (let k = 0; k < avail; k++) out[rowIdx * w + c + k] = data[pos + k] ?? 0;
        for (let k = avail; k < length; k++) out[rowIdx * w + c + k] = 0;
        pos += avail;
      } else {
        for (let k = 0; k < length; k++) out[rowIdx * w + c + k] = segType;
      }
      c += length;
    }
  }
  return out;
}

function decodeFormat2(data: Uint8Array, w: number, h: number): Uint8Array {
  if (data.length < 2) return new Uint8Array(w * h);
  const base = (data[0] | (data[1] << 8)) >>> 0;
  const dataLen = data.length;
  const out = new Uint8Array(w * h);
  let pos = base;
  for (let r = 0; r < h; r++) {
    let c = 0;
    while (c < w) {
      if (pos >= dataLen) break;
      const seg = data[pos];
      pos++;
      const code = Math.floor(seg / 32);
      const length = Math.min((seg & 31) + 1, w - c);
      if (code === 7) {
        const avail = Math.min(dataLen - pos, length);
        for (let k = 0; k < avail; k++) out[r * w + c + k] = data[pos + k] ?? 0;
        for (let k = avail; k < length; k++) out[r * w + c + k] = 0;
        pos += avail;
      } else {
        for (let k = 0; k < length; k++) out[r * w + c + k] = code;
      }
      c += length;
    }
  }
  return out;
}

function decodeFormat3(data: Uint8Array, w: number, h: number): Uint8Array {
  const rowOffsetsCount = h * 2 * (w / 32);
  if (w / 32 === 0 || rowOffsetsCount > data.length) return decodeFormat0(data, w, h);
  const dataLen = data.length;
  const out = new Uint8Array(w * h);
  for (let rowIdx = 0; rowIdx < h; rowIdx++) {
    const tablePos = rowIdx * 2 * (w / 32);
    if (tablePos + 2 > dataLen) break;
    const rowOffset = (data[tablePos] | (data[tablePos + 1] << 8)) >>> 0;
    if (rowOffset >= dataLen) continue;
    let pos = rowOffset;
    let c = 0;
    while (c < w) {
      if (pos >= dataLen) break;
      const seg = data[pos];
      pos++;
      const code = Math.floor(seg / 32);
      const length = Math.min((seg & 31) + 1, w - c);
      if (code === 7) {
        const avail = Math.min(dataLen - pos, length);
        for (let k = 0; k < avail; k++) out[rowIdx * w + c + k] = data[pos + k] ?? 0;
        for (let k = avail; k < length; k++) out[rowIdx * w + c + k] = 0;
        pos += avail;
      } else {
        for (let k = 0; k < length; k++) out[rowIdx * w + c + k] = code;
      }
      c += length;
    }
  }
  return out;
}

// H3 DEF 特殊调色板索引：0=透明，1-4=阴影，5-7=选择高亮。
// VCMI 渲染时会将其作为蒙版/覆盖层处理（阴影压黑、选择按玩家色替换）。
// 在 wiki 静态展示中：阴影渲染为半透明黑色，选择层直接隐藏，避免显示粉色/金色描边。
const SPECIAL_ALPHA = [0, 64, 64, 128, 128, 0, 128, 64];

function renderBgra(rows: Uint8Array, w: number, h: number, palette: Uint8Array): Uint8Array {
  const out = new Uint8Array(w * h * 4);
  for (let i = 0; i < w * h; i++) {
    const idx = rows[i] ?? 0;
    if (idx === 0 || (idx >= 5 && idx <= 7)) {
      // 透明 / 选择层不显示
      out[i * 4] = 0;
      out[i * 4 + 1] = 0;
      out[i * 4 + 2] = 0;
      out[i * 4 + 3] = 0;
      continue;
    }
    const p = idx * 3;
    const r = palette[p] ?? 0;
    const g = palette[p + 1] ?? 0;
    const b = palette[p + 2] ?? 0;
    // 阴影层（1-4）按半透明黑色渲染，与游戏内阴影效果一致
    if (idx >= 1 && idx <= 4) {
      const alpha = SPECIAL_ALPHA[idx] ?? 128;
      out[i * 4] = 0;
      out[i * 4 + 1] = 0;
      out[i * 4 + 2] = 0;
      out[i * 4 + 3] = alpha;
      continue;
    }
    out[i * 4] = b;
    out[i * 4 + 1] = g;
    out[i * 4 + 2] = r;
    out[i * 4 + 3] = 255;
  }
  return out;
}

/** 解码整个 def 动画：返回每个方向组（group）的全部帧 BGRA。 */
export function decodeAnimation(data: Uint8Array): DefGroup[] {
  const header = parseHeader(data);
  if (!header || header.groups.length === 0) return [];
  const result: DefGroup[] = [];
  header.groups.forEach((group, gi) => {
    const frames: DefFrame[] = [];
    for (const off of group) {
      const rf = readFrame(data, off);
      if (!rf) continue;
      if (rf.w === 0 || rf.h === 0 || rf.w > 4096 || rf.h > 4096) continue;
      const rows = decodePixels(rf.fmt, rf.w, rf.h, rf.pixel);
      if (rows.length !== rf.w * rf.h) continue;
      const bgra = renderBgra(rows, rf.w, rf.h, header.palette);
      frames.push({ w: rf.w, h: rf.h, bgra });
    }
    if (frames.length > 0) result.push({ group: gi, frames });
  });
  return result;
}

/**
 * 只解码**单个分组**的帧（惰性解码）。
 *
 * 大型生物战斗 DEF 可达数 MB、含十几个分组数百帧；`decodeAnimation` 会把全部分组
 * 一次性解出并在主线程同步执行，打开详情页时造成明显掉帧/假死。
 * 动画播放实际只会用到少数几个动作分组（见 CREA_PREVIEW），故按需只解当前分组。
 *
 * @returns 该分组的帧数组；分组不存在或无有效帧时返回空数组。
 */
export function decodeGroup(data: Uint8Array, groupIndex: number): DefFrame[] {
  const header = parseHeader(data);
  if (!header || groupIndex < 0 || groupIndex >= header.groups.length) return [];
  const frames: DefFrame[] = [];
  for (const off of header.groups[groupIndex]) {
    const rf = readFrame(data, off);
    if (!rf) continue;
    if (rf.w === 0 || rf.h === 0 || rf.w > 4096 || rf.h > 4096) continue;
    const rows = decodePixels(rf.fmt, rf.w, rf.h, rf.pixel);
    if (rows.length !== rf.w * rf.h) continue;
    const bgra = renderBgra(rows, rf.w, rf.h, header.palette);
    frames.push({ w: rf.w, h: rf.h, bgra });
  }
  return frames;
}

/** 读取 DEF 的分组数量（不解码像素，开销极小），用于判断某动作是否存在。 */
export function defGroupCount(data: Uint8Array): number {
  return parseHeader(data)?.groups.length ?? 0;
}
