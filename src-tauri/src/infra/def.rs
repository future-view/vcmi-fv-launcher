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
    /// 画布宽（帧坐标空间，原版生物 450）
    width: u32,
    /// 画布高（原版生物 400）
    height: u32,
    total_blocks: u32,
    palette: Vec<u8>,      // 768 字节
    groups: Vec<DefGroup>, // 每个分组：帧偏移 + 帧名（同序、同过滤口径）
}

/// 按 **DEF 组号** 取分组（组号可能不连续，不能用 Vec 下标）。
fn group_by_id(header: &DefHeader, group: usize) -> Option<&DefGroup> {
    header
        .groups
        .iter()
        .find(|g| g.block_id as usize == group)
}

/// 一个 DEF 分组：帧偏移与帧名一一对应（`names[i]` 即 `offsets[i]` 处的 13 字节名字段）。
///
/// 帧名是 HD 高清素材（`.pak`）的索引键：VCMI 用 `RenderHandler::getAnimationFrameName`
/// 把 `ThCsMag1.21f` 归一成 `THCSMAG1` 再去 pak 查图，见 [`DefReader::frame_name`]。
#[derive(Debug, Default)]
struct DefGroup {
    /// DEF 组号（分组表里的 block id，如 HOLDING=2）。**可能不连续**（原版骑士就没有
    /// 6/14-19），因此按组号取组必须靠它，不能用 Vec 下标（下标 == 组号只在连续时成立）。
    block_id: u32,
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
        // 文件头 = def_type(4) + width(4) + height(4) + total_blocks(4) + 调色板(768) = **784** 字节，
        // 分组表紧随其后（下方 `pos = 784`）。
        //
        // ⚠️ 此处原写 `784 + 768`（把调色板长度重复加了一次 → 门槛变成 1552）：
        // 任何小于 1552 字节的 DEF 都会被直接判为不可解析，小尺寸素材**整批静默丢失**
        // （实测 horde-town `silo.def` 1398B、cetatea-town `TBRMBLAK.def` 1387B、
        // hota.factory `TBFadw_1`/`TBFaup_1`、atlantis-town `tbrhall.def` 849B 的城镇结构帧）。
        // 调色板已被包含在 784 内，故门槛就是 784（同时保证下方 `data[16..16+768]` 切片安全）。
        if self.data.len() < 784 {
            return None;
        }
        let def_type = self.read_u32(0);
        let width = self.read_u32(4);
        let height = self.read_u32(8);
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
            //
            // ⚠️ 判定口径（2026-10-14 修正）：**必须用"偏移是否指向可信帧头"当证据**，
            // 不能用"帧名里 ASCII 字符多不多"当 tie-break —— 单帧 DEF（`total_entries=1`）
            // 的两个候选各只有 1 个偏移，只要落在 `[32, file_size)` 就都算"有效"打平，
            // 此时旧实现的 ASCII 打分会看走眼：20 字节候选的 13 字节窗口右移 4 字节，
            // 正好覆盖"帧名尾部 + 后面的偏移表字节"（如 `.bip\0\0\0\0` + `1\x03`），
            // ASCII 数反而更多 → 误选 20 → 偏移表整体错位 4 字节 → 帧头读出天文数字
            // （实测 kremlin-town 24/40、atlantis-town 23/37 的结构帧因此整批丢失）。
            // 帧名多是非 ASCII（西里尔字母），ASCII 打分对它们本就无判别力。
            if block_header_size.is_none() {
                let mut best_hdr = 16usize;
                let mut best_score = 0i32;
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
                        if self.frame_header_plausible(o) {
                            valid_cnt += 1;
                        }
                    }
                    // 严格 `>`：打平时保留先测的 16（VCMI 标准分块表头宽度）
                    if valid_cnt > best_score {
                        best_score = valid_cnt;
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
            let mut group = DefGroup {
                block_id,
                ..DefGroup::default()
            };
            for (i, off) in offsets.into_iter().enumerate() {
                if off >= 32 && off + 32 <= file_size as u32 {
                    group.offsets.push(off);
                    group.names.push(self.read_frame_name(names_start + i * 13));
                }
            }
            if !group.offsets.is_empty() {
                groups.push(group);
            }
        }

        Some(DefHeader {
            def_type,
            width,
            height,
            total_blocks,
            palette,
            groups,
        })
    }

    /// 指定偏移处的帧头是否"可信"——供分块表头尺寸（16/20）自动检测当证据。
    ///
    /// 判定口径与 [`Self::read_frame`] 的拒绝条件**严格对齐**：`read_frame` 会丢弃
    /// `fmt>3 || w==0 || h==0 || w>4096 || h>4096` 的帧，故这类偏移不算"这个表头尺寸可信"的证据
    /// （否则会像旧实现那样被落在文件范围内的垃圾偏移蒙对，见 `parse_header` 的说明）。
    fn frame_header_plausible(&self, offset: u32) -> bool {
        let off = offset as usize;
        if off + SSPRITE_DEF_SIZE > self.data.len() {
            return false;
        }
        let fmt = self.read_u32(off + 4);
        let w = self.read_u32(off + 16);
        let h = self.read_u32(off + 20);
        !(fmt > 3 || w == 0 || h == 0 || w > 4096 || h > 4096)
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
            let pos0 = u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
                as usize;
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
            let row_offset = u16::from_le_bytes([data[table_pos], data[table_pos + 1]]) as usize;
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
    /// DEF 画布尺寸（帧坐标空间，原版生物 450×400）。
    pub fn canvas_size(&self) -> Option<(u32, u32)> {
        let header = self.parse_header()?;
        Some((header.width, header.height))
    }

    /// **逐帧**包围盒（画布坐标）：`(组号, (x0, y0, x1, y1))`。
    ///
    /// 只读帧头（不解码任何像素），用于「显示窗口/缩放自适应」：VCMI 的窗口是固定矩形
    /// （`clipRect(150|170, 155, 100, 130)`），而部分生物的素材高出/宽出窗口
    /// （凤凰 HOLDING 顶部 y=117 < 155，头会被裁掉 25%）。
    ///
    /// 返回**帧级**而不按组合并：调用方要按组取 p10–p90 分位剔除离群帧。第三方素材常见
    /// 单帧把整幅画布当帧 / 混入特效帧，并集会被撑大 5~20 倍，整组跟着缩到 0.2~0.4
    /// （实测 `巨角鼠` 21 倍、`Invisible Man` 23 倍）。需要组级并集时调用方自行合并。
    ///
    /// 坏帧过滤：越画布 / 负 margin 一律丢弃（部分 mod DEF 尾部有垃圾分组，lm/tm 随机）。
    pub fn frame_rects_by_group(&self) -> Vec<(usize, (i64, i64, i64, i64))> {
        let Some(header) = self.parse_header() else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for g in header.groups.iter() {
            let gid = g.block_id as usize;
            for &off in &g.offsets {
                let Some((_fmt, w, h, _fs, _pad, _pixel, lm, tm)) = self.read_frame(off) else {
                    continue;
                };
                if w == 0 || h == 0 {
                    continue;
                }
                // 画布外的帧是坏帧（部分 mod DEF 尾部有垃圾分组，lm/tm 是随机值）——
                // 不滤掉会把包围盒撑到整张画布，窗口自适应随之失控。
                if lm < 0
                    || tm < 0
                    || lm as i64 + w as i64 > header.width as i64
                    || tm as i64 + h as i64 > header.height as i64
                {
                    continue;
                }
                out.push((
                    gid,
                    (lm as i64, tm as i64, lm as i64 + w as i64, tm as i64 + h as i64),
                ));
            }
        }
        out
    }

    pub fn decode_frame_at(
        &self,
        group: usize,
        index: usize,
    ) -> Option<(u32, u32, Vec<u8>, i32, i32)> {
        self.decode_frame_at_impl(group, index, false)
    }

    /// 动画专用：与 [`Self::decode_frame_at`] 同一解码，但按**特殊调色板索引**渲染 ——
    /// H3 DEF 约定索引 0=透明、1-4=阴影（半透明黑）、5-7=选择高亮（不显示）。
    ///
    /// 战斗生物 DEF 普遍用索引 1-4 画阴影；静态图标提取不走本函数
    /// （沿用 [`render_bgra`] 的历史口径，避免影响既有产物）。
    /// 与前端 `lib/def/decoder.ts::renderBgra` 的 `SPECIAL_ALPHA` 同口径。
    pub fn decode_frame_at_anim(
        &self,
        group: usize,
        index: usize,
    ) -> Option<(u32, u32, Vec<u8>, i32, i32)> {
        self.decode_frame_at_impl(group, index, true)
    }

    fn decode_frame_at_impl(
        &self,
        group: usize,
        index: usize,
        special_palette: bool,
    ) -> Option<(u32, u32, Vec<u8>, i32, i32)> {
        let header = self.parse_header()?;
        let g = &group_by_id(&header, group)?.offsets;
        // ⚠️ 帧号越界剪枝（2026-10-16）：原实现在 index 越界时会**把整组帧逐个解码一遍**
        // 才发现找不到目标帧（循环里 `decode_pixels` 先于 `k == index` 判定执行）。
        // 建筑图标常引用 DEF 里根本不存在的帧号（实测 299 个，如 `dwellingLvl8` 要
        // bid=150，而 `town-hall.def` 只有几十帧）→ 每次失败都白解一整个动画。
        // `offsets.len()` 是帧数**上界**（个别帧可能解不出），故越界必为 None，剪枝安全。
        if index >= g.len() {
            return None;
        }
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
                    let bgra = if special_palette {
                        render_bgra_anim(&rows, &header.palette)
                    } else {
                        render_bgra(&rows, &header.palette)
                    };
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
        let g = group_by_id(&header, group)?;
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
        let g = group_by_id(&header, group)?;
        let mut k = 0usize;
        for &off in &g.offsets {
            if !self.frame_usable(off) {
                continue;
            }
            if k == index {
                let off = off as usize;
                return Some((
                    self.read_u32(off + 24) as i32,
                    self.read_u32(off + 28) as i32,
                ));
            }
            k += 1;
        }
        None
    }

    pub fn decode_animation(&self) -> Option<Vec<(usize, u32, u32, Vec<u8>)>> {
        self.decode_animation_full().map(|v| {
            v.into_iter()
                .map(|(g, w, h, b, _lm, _tm)| (g, w, h, b))
                .collect()
        })
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

/// 动画专用渲染：在 [`render_bgra`] 的色键之外，额外处理 **H3 特殊调色板索引**
/// （0=透明、1-4=阴影 → 半透明黑、5-7=选择高亮 → 不显示）。
///
/// 口径与前端 `lib/def/decoder.ts::renderBgra` 的 `SPECIAL_ALPHA` 完全一致 ——
/// 缺了它，生物战斗帧的阴影（索引 1-4）会被画成调色板里的洋红/金色实体色。
fn render_bgra_anim(rows: &[Vec<u8>], palette: &[u8]) -> Vec<u8> {
    // 索引语义（0/5-7 透明、1-4 阴影黑）由 `infra::image::h3_index_rgba` 单点定义：
    // 与 VCMI `ScalableImageParameters::setShadowTransparency` 同口径，png 动画帧也走它。
    // **RGB 必须置 0**：这些索引的调色板色是洋红/金色渐变，只改 alpha 会透出洋红（实测农民帧）。
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
            let idx = (*row.get(x).unwrap_or(&0)).min(255) as usize;
            if let Some(special) = crate::infra::image::h3_index_rgba(idx as u8) {
                out.extend_from_slice(&special);
                continue;
            }
            let (r, g, b) = pal[idx];
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
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../references/Heroes3-VCMI")
            });
        let lod = LodArchive::open(&PathBuf::from(game).join("Data").join("H3sprite.lod"))
            .expect("H3sprite.lod 应可打开");
        let cstl = lod.read("HALLCSTL.DEF").expect("应能读出 HALLCSTL.DEF");
        let def = DefReader::new(&cstl);
        assert_eq!(def.frame_name(0, 0).as_deref(), Some("THCSMAG1"));
        let names: Vec<String> = (0..12).filter_map(|i| def.frame_name(0, i)).collect();
        assert!(
            names.iter().any(|n| n == "THCSCAS1"),
            "应读到 THCSCAS1: {names:?}"
        );
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

    /// DEF 分块表头尺寸（16/20）自动检测 + 最小长度门槛的回归（2026-10-14）。
    ///
    /// 构造一个**单帧** DEF（`total_entries=1`），帧名是非 ASCII（西里尔字母）——这正是
    /// kremlin-town 24/40 结构帧丢失的形态：
    /// ① 旧逻辑在「两个候选各只有 1 个有效偏移」时用「帧名 ASCII 字符数」做 tie-break，
    ///    会误选 20 字节表头 → 偏移表整体错位 4 字节 → 帧头读出垃圾、整批丢帧；
    /// ② 旧 `parse_header` 门槛写成 `784 + 768 = 1552`，本 DEF 仅 865 字节会被直接判无效。
    /// 新逻辑改用「偏移是否指向可信帧头」做证据、门槛改回 784，两者都应判出正好 1 帧。
    #[test]
    fn single_frame_def_nonascii_name_decodes() {
        let mut b = vec![0u8; 865];
        // 文件头：type=0x42, w=4, h=4, total_blocks=1（16 字节）
        b[0..4].copy_from_slice(&0x42u32.to_le_bytes());
        b[4..8].copy_from_slice(&4u32.to_le_bytes());
        b[8..12].copy_from_slice(&4u32.to_le_bytes());
        b[12..16].copy_from_slice(&1u32.to_le_bytes());
        // 调色板（16..784）保持 0
        // 分组表：block0 id=0 (784..788), total_entries=1 (788..792)
        b[784..788].copy_from_slice(&0u32.to_le_bytes());
        b[788..792].copy_from_slice(&1u32.to_le_bytes());
        // 16 字节表头口径：帧名 13 字节（非 ASCII + ".bip\0"）@800..813
        let name = b"\xd0\x91\xd0\x92\xd0\x93.bip\x00\x00\x00"; // 13 bytes
        b[800..813].copy_from_slice(name);
        // 偏移表：offsets[0] = 817 @813..817
        b[813..817].copy_from_slice(&817u32.to_le_bytes());
        // 帧头 @817：frame_size=48, fmt=0, fw=4, fh=4, w=4, h=4, lm=0, tm=0（32 字节）
        let fh = 817usize;
        b[fh..fh + 4].copy_from_slice(&48u32.to_le_bytes());
        b[fh + 4..fh + 8].copy_from_slice(&0u32.to_le_bytes()); // fmt=0
        b[fh + 8..fh + 12].copy_from_slice(&4u32.to_le_bytes()); // fw
        b[fh + 12..fh + 16].copy_from_slice(&4u32.to_le_bytes()); // fh
        b[fh + 16..fh + 20].copy_from_slice(&4u32.to_le_bytes()); // w
        b[fh + 20..fh + 24].copy_from_slice(&4u32.to_le_bytes()); // h
                                                                  // 像素数据 @849..865（fmt=0：fw*fh = 16 字节）
        for i in 849..865 {
            b[i] = (i % 256) as u8;
        }

        let def = DefReader::new(&b);
        let frames = def
            .decode_animation_full()
            .expect("单帧 DEF 应至少解出 1 帧");
        assert_eq!(
            frames.len(),
            1,
            "单帧 DEF 必须解出正好 1 帧（旧逻辑会误判 0 帧：16/20 误检 + 784+768 门槛）"
        );
    }
}
