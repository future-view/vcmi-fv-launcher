//! VCMI/HoMM3 `.def` 动画文件解码器（移植自 references/vcmi-def-helper/main.py）。
//!
//! 仅实现"提取首帧为位图"的最小子集：
//! - 文件头：def_type / total_blocks / 256 色调色板(768 字节)
//! - 分组表：从偏移 784 起，自动检测表头 16/20 字节
//! - 帧头：32 字节 `<IIIIIIii` = (frame_size, fmt, fw, fh, w, h, lm, tm)
//! - 像素解码：format 0/1/2/3（RLE/段编码）
//! - 输出：32bpp BGRA BMP（带透明通道，palette[0] 视为透明）
//!
//! 不引入任何外部 crate，纯标准库实现，避免工具链风险。

const SSPRITE_DEF_SIZE: usize = 32;

/// 色键规则（对齐 VCMI CSDL_Ext::setColorKey(palette->colors[0])）：
/// **调色板第 0 项的颜色** 即色键——帧内 RGB 与该色相同的像素全部透明。
/// 旧实现误把索引 0–7 全抠（H3 调色板 1–7 是常用色，会破坏主体）。
fn palette_color_key(palette: &[u8]) -> (u8, u8, u8) {
    (
        *palette.first().unwrap_or(&0),
        *palette.get(1).unwrap_or(&0),
        *palette.get(2).unwrap_or(&0),
    )
}

#[derive(Debug, Default)]
#[allow(dead_code)]
struct DefHeader {
    def_type: u32,
    total_blocks: u32,
    palette: Vec<u8>, // 768 字节
    groups: Vec<DefGroup>, // 每个分组：帧偏移 + 帧名（同序、同过滤口径）
}

/// 一个 DEF 分组：帧偏移与帧名一一对应（`names[i]` 即 `offsets[i]` 处的 13 字节名字段）。
///
/// 帧名是 HD 高清素材（`.pak`）的索引键：VCMI 用 `RenderHandler::getAnimationFrameName`
/// 把 `ThCsMag1.21f` 归一成 `THCSMAG1` 再去 pak 查图，见 [`DefReader::frame_name`]。
#[derive(Debug, Default)]
struct DefGroup {
    offsets: Vec<u32>,
    names: Vec<String>,
}

pub struct DefReader<'a> {
    data: &'a [u8],
}

impl<'a> DefReader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    fn read_u32(&self, off: usize) -> u32 {
        if off + 4 > self.data.len() {
            return 0;
        }
        u32::from_le_bytes([
            self.data[off],
            self.data[off + 1],
            self.data[off + 2],
            self.data[off + 3],
        ])
    }

    fn parse_header(&self) -> Option<DefHeader> {
        if self.data.len() < 784 + 768 {
            return None;
        }
        let def_type = self.read_u32(0);
        let total_blocks = self.read_u32(12);
        let palette = self.data[16..16 + 768].to_vec();
        let file_size = self.data.len();
        let mut pos = 784usize;
        let mut groups: Vec<DefGroup> = Vec::new();
        let max_blocks = total_blocks.min(256) as usize;
        let mut block_header_size: Option<usize> = None;

        for _ in 0..max_blocks {
            if pos + 8 > file_size {
                break;
            }
            let block_id = self.read_u32(pos);
            let total_entries = self.read_u32(pos + 4);
            if total_entries == 0 || total_entries > 10000 {
                // 无法自动检测表头尺寸时，尝试默认 16
                let hdr = block_header_size.unwrap_or(16);
                pos += hdr;
                if block_header_size.is_some() {
                    if pos + (total_entries as usize) * 17 > file_size {
                        break;
                    }
                    pos += (total_entries as usize) * 17;
                }
                continue;
            }

            // 自动检测表头大小（16 或 20 字节）
            if block_header_size.is_none() {
                let mut best_hdr = 16usize;
                let mut best_score = -1.0f64;
                for test_hdr in [16usize, 20usize] {
                    let ns = pos + test_hdr;
                    if ns + 12 > file_size {
                        continue;
                    }
                    let os_off = ns + (total_entries as usize) * 13;
                    if os_off + (total_entries as usize) * 4 > file_size {
                        continue;
                    }
                    let mut valid_cnt = 0i32;
                    for j in 0..total_entries as usize {
                        let o = self.read_u32(os_off + j * 4);
                        if (32..file_size as u32).contains(&o) {
                            valid_cnt += 1;
                        }
                    }
                    let name_raw = &self.data[ns..(ns + 13).min(file_size)];
                    let ascii_score = name_raw.iter().filter(|&&b| (32..127).contains(&b)).count();
                    let score = valid_cnt as f64 + ascii_score as f64 / 100.0;
                    if score > best_score {
                        best_score = score;
                        best_hdr = test_hdr;
                    }
                }
                block_header_size = Some(best_hdr);
            }
            let hdr = block_header_size.unwrap();
            pos += hdr;

            let names_size = (total_entries as usize) * 13;
            let offsets_size = (total_entries as usize) * 4;
            if pos + names_size + offsets_size > file_size {
                break;
            }
            let names_start = pos;
            pos += names_size;
            let mut offsets = Vec::with_capacity(total_entries as usize);
            for _ in 0..total_entries {
                offsets.push(self.read_u32(pos));
                pos += 4;
            }
            // 帧名与偏移**同序、同过滤**：`decode_frame_at` 的序号是"过滤后序号"，
            // 帧名必须同口径才能与之一一对应（HD 素材按名查图依赖这一点）。
            let mut group = DefGroup::default();
            for (i, off) in offsets.into_iter().enumerate() {
                if off >= 32 && off + 32 <= file_size as u32 {
                    group.offsets.push(off);
                    group.names.push(self.read_frame_name(names_start + i * 13));
                }
            }
            if !group.offsets.is_empty() {
                // block_id 作为分组标识，这里简单用索引维护顺序
                groups.push(group);
                let _ = block_id;
            }
        }

        Some(DefHeader {
            def_type,
            total_blocks,
            palette,
            groups,
        })
    }

    /// 读取指定偏移处的帧头与像素数据（末尾两个 i32 = left/top margin）。
    fn read_frame(&self, offset: u32) -> Option<(u32, u32, u32, u32, u8, Vec<u8>, i32, i32)> {
        let off = offset as usize;
        if off + SSPRITE_DEF_SIZE > self.data.len() {
            return None;
        }
        let frame_size = self.read_u32(off);
        let fmt = self.read_u32(off + 4);
        let fw = self.read_u32(off + 8);
        let fh = self.read_u32(off + 12);
        let w = self.read_u32(off + 16);
        let h = self.read_u32(off + 20);
        // 帧头末尾的 i32 left/top margin（VCMI 绘制位置 = pos + margins）
        let lm = self.read_u32(off + 24) as i32;
        let tm = self.read_u32(off + 28) as i32;
        if fmt > 3 || w == 0 || h == 0 || w > 4096 || h > 4096 {
            return None;
        }
        // 像素数据起始：SSPRITE_DEF_SIZE=32，fmt==1 时按 python 逻辑减去 16
        let pixel_off = if fmt == 1 && w > fw && h > fh {
            off + SSPRITE_DEF_SIZE - 16
        } else {
            off + SSPRITE_DEF_SIZE
        };
        let pixel_end = if frame_size == 0 {
            pixel_off
        } else {
            (off + frame_size as usize).min(self.data.len())
        };
        let pixel_data = self.data[pixel_off..pixel_end].to_vec();
        Some((fmt, w, h, frame_size, 0, pixel_data, lm, tm))
    }

    /// 解码像素为索引二维数组 [h][w]。
    fn decode_pixels(fmt: u8, w: u32, h: u32, data: &[u8]) -> Vec<Vec<u8>> {
        match fmt {
            0 => Self::decode_format0(data, w as usize, h as usize),
            1 => Self::decode_format1(data, w as usize, h as usize),
            2 => Self::decode_format2(data, w as usize, h as usize),
            3 => Self::decode_format3(data, w as usize, h as usize),
            _ => Self::decode_format0(data, w as usize, h as usize),
        }
    }

    fn decode_format0(data: &[u8], w: usize, h: usize) -> Vec<Vec<u8>> {
        let mut rows = Vec::with_capacity(h);
        let mut i = 0usize;
        for _ in 0..h {
            let mut row = Vec::with_capacity(w);
            for _ in 0..w {
                row.push(*data.get(i).unwrap_or(&0));
                i += 1;
            }
            rows.push(row);
        }
        rows
    }

    fn decode_format1(data: &[u8], w: usize, h: usize) -> Vec<Vec<u8>> {
        if data.len() < h * 4 {
            return Self::decode_format0(data, w, h);
        }
        let mut rows = Vec::with_capacity(h);
        let data_len = data.len();
        for row_idx in 0..h {
            let off = row_idx * 4;
            let pos0 = u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]) as usize;
            let mut row = Vec::with_capacity(w);
            let mut pos = pos0;
            while row.len() < w && pos < data_len {
                if pos + 2 > data_len {
                    break;
                }
                let seg_type = data[pos];
                let length = (data[pos + 1] as usize + 1).min(w - row.len());
                pos += 2;
                if seg_type == 0xFF {
                    let avail = (data_len - pos).min(length);
                    row.extend_from_slice(&data[pos..pos + avail]);
                    row.extend(std::iter::repeat(0).take(length - avail));
                    pos += avail;
                } else {
                    row.extend(std::iter::repeat(seg_type).take(length));
                }
            }
            row.truncate(w);
            rows.push(row);
        }
        rows
    }

    fn decode_format2(data: &[u8], w: usize, h: usize) -> Vec<Vec<u8>> {
        if data.len() < 2 {
            return Vec::new();
        }
        let base = u16::from_le_bytes([data[0], data[1]]) as usize;
        let data_len = data.len();
        let mut pos = base;
        let mut rows = Vec::with_capacity(h);
        for _ in 0..h {
            let mut row = Vec::with_capacity(w);
            while row.len() < w {
                if pos >= data_len {
                    break;
                }
                let seg = data[pos];
                pos += 1;
                let code = seg / 32;
                let length = ((seg & 31) as usize + 1).min(w - row.len());
                if code == 7 {
                    let avail = (data_len - pos).min(length);
                    row.extend_from_slice(&data[pos..pos + avail]);
                    row.extend(std::iter::repeat(0).take(length - avail));
                    pos += avail;
                } else {
                    row.extend(std::iter::repeat(code).take(length));
                }
            }
            row.truncate(w);
            rows.push(row);
        }
        rows
    }

    fn decode_format3(data: &[u8], w: usize, h: usize) -> Vec<Vec<u8>> {
        let data_len = data.len();
        let row_offsets_count = h * 2 * (w / 32);
        if (w / 32) == 0 || row_offsets_count > data_len {
            return Self::decode_format0(data, w, h);
        }
        let mut rows = Vec::with_capacity(h);
        for row_idx in 0..h {
            let table_pos = row_idx * 2 * (w / 32);
            if table_pos + 2 > data_len {
                break;
            }
            let row_offset =
                u16::from_le_bytes([data[table_pos], data[table_pos + 1]]) as usize;
            let mut row = Vec::with_capacity(w);
            if row_offset < data_len {
                let mut pos = row_offset;
                while row.len() < w {
                    if pos >= data_len {
                        break;
                    }
                    let seg = data[pos];
                    pos += 1;
                    let code = seg / 32;
                    let length = ((seg & 31) as usize + 1).min(w - row.len());
                    if code == 7 {
                        let avail = (data_len - pos).min(length);
                        row.extend_from_slice(&data[pos..pos + avail]);
                        row.extend(std::iter::repeat(0).take(length - avail));
                        pos += avail;
                    } else {
                        row.extend(std::iter::repeat(code).take(length));
                    }
                }
            }
            row.truncate(w);
            rows.push(row);
        }
        rows
    }

    /// 解码 group 0 的第一个有效帧，输出 32bpp BGRA BMP 字节。
    pub fn first_frame_bmp(&self) -> Option<Vec<u8>> {
        let header = self.parse_header()?;
        if header.groups.is_empty() {
            return None;
        }
        let group = &header.groups[0];
        let mut frame_data: Option<(u32, u32, u32, Vec<Vec<u8>>)> = None;
        for &off in &group.offsets {
            if let Some((fmt, w, h, _fs, _pad, pixel, _lm, _tm)) = self.read_frame(off) {
                let rows = Self::decode_pixels(fmt as u8, w, h, &pixel);
                if rows.len() == h as usize && h > 0 {
                    frame_data = Some((w, h, w, rows));
                    break;
                }
            }
        }
        let (_w, _h, _fw, rows) = frame_data?;
        Some(render_bmp(&rows, &header.palette))
    }

    /// 解码所有方向组的**全部有效帧**为 32bpp BGRA 像素（动画模式用）。
    /// 返回 `Vec<(group_index, width, height, frame_bgra)>`，每组含多帧（顶向下 BGRA，色键已置透明）。
    #[allow(dead_code)] // 供动画模式（read_def_animation）使用
    /// **只解码指定 (group, index) 帧**。
    ///
    /// ⚠️ 此前取单帧也走 [`Self::decode_animation`]——把整个动画（DEF 动辄上百帧，
    /// 如 `CPRSMALL` 150+ 帧）全部解码后再挑一帧，实测 `def.decode` 占全量构建数百秒 CPU。
    /// 序号口径与 `decode_animation_full_inner` 一致：解不出 / 尺寸非法的帧不占序号。
    pub fn decode_frame_at(
        &self,
        group: usize,
        index: usize,
    ) -> Option<(u32, u32, Vec<u8>, i32, i32)> {
        let header = self.parse_header()?;
        let g = &header.groups.get(group)?.offsets;
        let mut k = 0usize;
        for &off in g {
            if let Some((fmt, w, h, _fs, _pad, pixel, lm, tm)) = self.read_frame(off) {
                if w == 0 || h == 0 || w > 4096 || h > 4096 {
                    continue;
                }
                let rows = Self::decode_pixels(fmt as u8, w, h, &pixel);
                if rows.len() != h as usize || h == 0 {
                    continue;
                }
                if k == index {
                    let bgra = render_bgra(&rows, &header.palette);
                    return Some((w, h, bgra, lm, tm));
                }
                k += 1;
            }
        }
        None
    }

    /// 读 13 字节帧名字段（`\0` 截断 + 去首尾空白；非 ASCII 走宽松解码，绝不 panic）。
    fn read_frame_name(&self, off: usize) -> String {
        let end = (off + 13).min(self.data.len());
        if off >= end {
            return String::new();
        }
        let raw = &self.data[off..end];
        let n = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        String::from_utf8_lossy(&raw[..n]).trim().to_string()
    }

    /// 该偏移上的帧是否"可用"——判定与 [`Self::read_frame`] 同源，但**不复制像素数据**。
    ///
    /// 用途：`frame_name` 要对整组帧逐个判定"是否占序号"，若走 `read_frame` 会把每帧像素
    /// 都拷一遍（DEF 上百帧时纯浪费）。
    fn frame_usable(&self, off: u32) -> bool {
        let off = off as usize;
        if off + SSPRITE_DEF_SIZE > self.data.len() {
            return false;
        }
        let fmt = self.read_u32(off + 4);
        let w = self.read_u32(off + 16);
        let h = self.read_u32(off + 20);
        fmt <= 3 && w > 0 && h > 0 && w <= 4096 && h <= 4096
    }

    /// 取指定帧的**资源名**（HD 高清素材 `.pak` 的索引键）。
    ///
    /// 命名规则对齐 VCMI `RenderHandler::getAnimationFrameName`：取第一个 `.` 之前的部分并大写
    /// （`ThCsMag1.21f` → `THCSMAG1`、`CPrS097S.p1t` → `CPRS097S`）。
    ///
    /// 序号口径与 [`Self::decode_frame_at`] 一致（不可用的帧不占序号），保证"取第 N 帧"与
    /// "取第 N 帧的名字"指向同一帧。
    pub fn frame_name(&self, group: usize, index: usize) -> Option<String> {
        let header = self.parse_header()?;
        let g = header.groups.get(group)?;
        let mut k = 0usize;
        for (i, &off) in g.offsets.iter().enumerate() {
            if !self.frame_usable(off) {
                continue;
            }
            if k == index {
                let raw = g.names.get(i)?;
                let base = raw.split('.').next().unwrap_or(raw).trim().to_uppercase();
                return if base.is_empty() { None } else { Some(base) };
            }
            k += 1;
        }
        None
    }

    /// 取指定帧的 **left/top margin**（1x 逻辑值，VCMI 绘制位置 = pos + margins）。
    ///
    /// 序号口径与 [`Self::frame_name`] **完全一致**（不可用的帧不占序号），因此
    /// "取第 N 帧的名字（去查 HD 图）"与"取第 N 帧的绘制偏移"必然指向同一帧。
    ///
    /// 存在的理由：HD 取图路径**只拿得到图**（`infra::def` 之外的调用方拿不到帧头），
    /// 而 pak 帧是按名字索引的独立图集 —— 若不给它配上 DEF 的 lm/tm，带非零 margin
    /// 的帧就会画错位置（实测城镇船坞 `TBCSDKNN` 的 tm=53，画到战神像身上）。
    pub fn frame_margins(&self, group: usize, index: usize) -> Option<(i32, i32)> {
        let header = self.parse_header()?;
        let g = header.groups.get(group)?;
        let mut k = 0usize;
        for &off in &g.offsets {
            if !self.frame_usable(off) {
                continue;
            }
            if k == index {
                let off = off as usize;
                return Some((self.read_u32(off + 24) as i32, self.read_u32(off + 28) as i32));
            }
            k += 1;
        }
        None
    }

    pub fn decode_animation(&self) -> Option<Vec<(usize, u32, u32, Vec<u8>)>> {
        self.decode_animation_full()
            .map(|v| v.into_iter().map(|(g, w, h, b, _lm, _tm)| (g, w, h, b)).collect())
    }

    /// 同上，但额外返回帧头的 left/top margin（VCMI 绘制位置 = pos + margins）。
    pub fn decode_animation_full(&self) -> Option<Vec<(usize, u32, u32, Vec<u8>, i32, i32)>> {
        crate::infra::metrics::scope("def.decode", || self.decode_animation_full_inner())
    }

    /// 同上，但额外返回帧头的 left/top margin（VCMI 绘制位置 = pos + margins）。
    fn decode_animation_full_inner(&self) -> Option<Vec<(usize, u32, u32, Vec<u8>, i32, i32)>> {
        let header = self.parse_header()?;
        if header.groups.is_empty() {
            return None;
        }
        let mut out = Vec::new();
        for (gi, group) in header.groups.iter().enumerate() {
            for &off in &group.offsets {
                if let Some((fmt, w, h, _fs, _pad, pixel, lm, tm)) = self.read_frame(off) {
                    if w == 0 || h == 0 || w > 4096 || h > 4096 {
                        continue;
                    }
                    let rows = Self::decode_pixels(fmt as u8, w, h, &pixel);
                    if rows.len() == h as usize && h > 0 {
                        let bgra = render_bgra(&rows, &header.palette);
                        out.push((gi, w, h, bgra, lm, tm));
                    }
                }
            }
        }
        Some(out)
    }
}

/// 将索引像素 + 调色板渲染为 32bpp BGRA BMP。
fn render_bmp(rows: &[Vec<u8>], palette: &[u8]) -> Vec<u8> {
    let h = rows.len();
    let w = if h > 0 { rows[0].len() } else { 0 };
    // BMP 像素区：每行 4 字节对齐
    let row_bytes = (w * 4 + 3) & !3;
    let pixel_area = row_bytes * h;
    let file_size = 54 + pixel_area;

    let mut out = Vec::with_capacity(file_size);
    // BITMAPFILEHEADER (14)
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(file_size as u32).to_le_bytes());
    out.extend_from_slice(&[0, 0, 0, 0]); // reserved
    out.extend_from_slice(&(54u32).to_le_bytes()); // pixel data offset
    // BITMAPINFOHEADER (40)
    out.extend_from_slice(&(40u32).to_le_bytes());
    out.extend_from_slice(&(w as i32).to_le_bytes());
    out.extend_from_slice(&(h as i32).to_le_bytes());
    out.extend_from_slice(&[1, 0]); // planes
    out.extend_from_slice(&[32, 0]); // bpp
    out.extend_from_slice(&[0, 0, 0, 0]); // compression
    out.extend_from_slice(&(pixel_area as u32).to_le_bytes());
    out.extend_from_slice(&[0x13, 0x0B, 0, 0]); // x ppm
    out.extend_from_slice(&[0x13, 0x0B, 0, 0]); // y ppm
    out.extend_from_slice(&[0, 0, 0, 0]); // colors used
    out.extend_from_slice(&[0, 0, 0, 0]); // important colors

    // 像素：BMP 从底向上，BGRA
    let pal: Vec<(u8, u8, u8)> = (0..256)
        .map(|i| {
            let p = i * 3;
            (
                *palette.get(p).unwrap_or(&0),
                *palette.get(p + 1).unwrap_or(&0),
                *palette.get(p + 2).unwrap_or(&0),
            )
        })
        .collect();
    let key = palette_color_key(palette);

    for y in (0..h).rev() {
        let row = &rows[y];
        for x in 0..w {
            let idx = *row.get(x).unwrap_or(&0) as usize;
            let (r, g, b) = pal[idx.min(255)];
            let alpha = if (r, g, b) == key { 0 } else { 255 };
            out.push(b);
            out.push(g);
            out.push(r);
            out.push(alpha);
        }
        // 补齐到 4 字节
        let pad = row_bytes - w * 4;
        out.extend(std::iter::repeat(0u8).take(pad));
    }
    out
}

/// 将索引像素 + 调色板渲染为**顶向下** 32bpp BGRA（供 webp 编码，无对齐/无头）。
fn render_bgra(rows: &[Vec<u8>], palette: &[u8]) -> Vec<u8> {
    let h = rows.len();
    let w = if h > 0 { rows[0].len() } else { 0 };
    let pal: Vec<(u8, u8, u8)> = (0..256)
        .map(|i| {
            let p = i * 3;
            (
                *palette.get(p).unwrap_or(&0),
                *palette.get(p + 1).unwrap_or(&0),
                *palette.get(p + 2).unwrap_or(&0),
            )
        })
        .collect();
    let key = palette_color_key(palette);
    let mut out = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let row = &rows[y];
        for x in 0..w {
            let idx = *row.get(x).unwrap_or(&0) as usize;
            let (r, g, b) = pal[idx.min(255)];
            let alpha = if (r, g, b) == key { 0 } else { 255 };
            out.push(b);
            out.push(g);
            out.push(r);
            out.push(alpha);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::infra::lod::LodArchive;
    use std::path::PathBuf;

    /// 真机冒烟：DEF 的 13 字节帧名 → HD 素材（`.pak`）索引键（VCMI `getAnimationFrameName` 口径）。
    ///
    /// 期望值来自 2026-09-23 对 `H3sprite.lod` 的独立解析（Python）与 HD pak 核对：
    /// `HALLCSTL.DEF` 首帧 = `ThCsMag1.21f` → `THCSMAG1`（pak 中 `HALLCSTL` 条目的第 0 帧名）。
    #[test]
    #[ignore]
    fn real_def_frame_names() {
        let game = std::env::var("VCMI_GAME_DIR")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../references/Heroes3-VCMI"));
        let lod = LodArchive::open(&PathBuf::from(game).join("Data").join("H3sprite.lod"))
            .expect("H3sprite.lod 应可打开");
        let cstl = lod.read("HALLCSTL.DEF").expect("应能读出 HALLCSTL.DEF");
        let def = DefReader::new(&cstl);
        assert_eq!(def.frame_name(0, 0).as_deref(), Some("THCSMAG1"));
        let names: Vec<String> = (0..12).filter_map(|i| def.frame_name(0, i)).collect();
        assert!(names.iter().any(|n| n == "THCSCAS1"), "应读到 THCSCAS1: {names:?}");
        // 序号口径与取帧一致：第 0 帧可解码且尺寸合法
        let (w, h, ..) = def.decode_frame_at(0, 0).expect("第 0 帧应可解码");
        assert!(w > 0 && h > 0);

        let cprs = lod.read("CPRSMALL.DEF").expect("应能读出 CPRSMALL.DEF");
        let def = DefReader::new(&cprs);
        let names: Vec<String> = (0..6).filter_map(|i| def.frame_name(0, i)).collect();
        assert!(
            names.iter().any(|n| n.starts_with("CPRS")),
            "应读到 CPRS* 帧名: {names:?}"
        );

        // 帧头 margin：`frame_margins`（HD 取图路径按它定位）与 `decode_frame_at`
        // （1x 取帧路径按它定位）必须给出同一个值 —— 两条路径口径不一致就会错位。
        let dock = lod.read("TBCSDOCK.DEF").expect("应能读出 TBCSDOCK.DEF");
        let def = DefReader::new(&dock);
        assert_eq!(def.frame_name(0, 0).as_deref(), Some("TBCSDKNN"));
        let (_, _, _, lm, tm) = def.decode_frame_at(0, 0).expect("第 0 帧应可解码");
        assert_eq!(def.frame_margins(0, 0), Some((lm, tm)));
        // 期望值来自 2026-09-24 对 H3sprite.lod 的独立解析（Python）：船坞帧 tm=53
        // —— 结构 y(134) + 53 才是它该在的岸边位置；丢了这 53px 就会盖住战神像。
        assert_eq!(def.frame_margins(0, 0), Some((0, 53)));
    }
}
