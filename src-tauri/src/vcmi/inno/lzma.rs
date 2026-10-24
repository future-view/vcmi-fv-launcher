//! 纯 Rust 的 raw LZMA1 解码器（只解码，不编码）。
//!
//! Inno Setup 5.x 用 LZMA1 压缩设置头和数据块，块内布局是
//! `1 字节 props + 4 字节字典大小(LE) + LZMA1 原始流`（见 innoextract
//! `src/stream/lzma.cpp`：读 5 字节头，`properties = header[0]`、`dict_size = LE(header+1)`）。
//!
//! 本实现只覆盖 LZMA1（不含 LZMA2），零外部依赖：range decoder + 概率模型 + 长度/距离解码。
//! 解压长度上限由调用方给定（Inno 的 data entry 里有解压后大小），也支持遇到 EOS 标记结束。

use std::fmt;

/// LZMA1 属性（来自流内 5 字节头）
#[derive(Debug, Clone, Copy)]
pub struct Props {
    pub lc: u32,
    pub lp: u32,
    pub pb: u32,
    pub dict_size: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LzmaError {
    /// 数据不完整 / 流格式错误
    Truncated(&'static str),
    /// 解压结果超出预期上限
    TooLarge,
    /// 距离引用越界（损坏数据）
    BadDistance,
}

impl fmt::Display for LzmaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LzmaError::Truncated(what) => write!(f, "LZMA 数据截断：{what}"),
            LzmaError::TooLarge => write!(f, "LZMA 解压结果超出预期大小"),
            LzmaError::BadDistance => write!(f, "LZMA 距离引用越界（数据损坏）"),
        }
    }
}

impl std::error::Error for LzmaError {}

const PROB_INIT: u16 = 1 << 10;
const K_TOP: u32 = 1 << 24;
/// 长度解码器低位表：`low[pos_state]` / `mid[pos_state]` 各 8 个概率
const LEN_LOW_SYMBOLS: usize = 8;
const LEN_MID_SYMBOLS: usize = 8;
const LEN_HIGH_SYMBOLS: usize = 256;

/// 状态机表（LZMA 规范 12 状态）
const STATE_UPDATE_CHAR: [usize; 12] = [0, 0, 0, 0, 1, 2, 3, 4, 5, 6, 4, 5];

/// 解析 5 字节流头。
pub fn parse_header(payload: &[u8]) -> Result<(Props, &[u8]), LzmaError> {
    if payload.len() < 5 {
        return Err(LzmaError::Truncated("LZMA 头不足 5 字节"));
    }
    let props_byte = payload[0] as u32;
    if props_byte > 9 * 5 * 5 {
        return Err(LzmaError::Truncated("LZMA 属性字节非法"));
    }
    let lc = props_byte % 9;
    let lp = (props_byte / 9) % 5;
    let pb = props_byte / 45;
    if lc + lp > 4 {
        return Err(LzmaError::Truncated("LZMA lc+lp 超过 4"));
    }
    let dict_size = u32::from_le_bytes([payload[1], payload[2], payload[3], payload[4]]);
    Ok((
        Props {
            lc,
            lp,
            pb,
            dict_size,
        },
        &payload[5..],
    ))
}

struct RangeDecoder<'a> {
    data: &'a [u8],
    pos: usize,
    range: u32,
    code: u32,
}

impl<'a> RangeDecoder<'a> {
    fn new(data: &'a [u8]) -> Result<Self, LzmaError> {
        if data.len() < 5 {
            return Err(LzmaError::Truncated("range coder 初始化数据不足"));
        }
        // 原始 LZMA 流的首字节固定为 0，其后 4 字节为 code（大端）
        if data[0] != 0 {
            return Err(LzmaError::Truncated("range coder 首字节非 0"));
        }
        let code = u32::from_be_bytes([data[1], data[2], data[3], data[4]]);
        Ok(Self {
            data,
            pos: 5,
            range: 0xFFFF_FFFF,
            code,
        })
    }

    fn normalize(&mut self) -> Result<(), LzmaError> {
        while self.range < K_TOP {
            let b = *self
                .data
                .get(self.pos)
                .ok_or(LzmaError::Truncated("range coder 数据耗尽"))?;
            self.pos += 1;
            self.range <<= 8;
            self.code = (self.code << 8) | b as u32;
        }
        Ok(())
    }

    fn bit(&mut self, prob: &mut u16) -> Result<u32, LzmaError> {
        let bound = (self.range >> 11) * (*prob as u32);
        let bit = if self.code < bound {
            self.range = bound;
            *prob += (2048 - *prob) >> 5;
            0
        } else {
            self.range -= bound;
            self.code -= bound;
            *prob -= *prob >> 5;
            1
        };
        self.normalize()?;
        Ok(bit)
    }

    fn direct_bits(&mut self, n: u32) -> Result<u32, LzmaError> {
        let mut res = 0u32;
        for _ in 0..n {
            self.range >>= 1;
            let bit = if self.code >= self.range {
                self.code -= self.range;
                1
            } else {
                0
            };
            res = (res << 1) | bit;
            self.normalize()?;
        }
        Ok(res)
    }
}

/// 正向位树解码（`probs` 从下标 1 开始用）
fn bit_tree(rc: &mut RangeDecoder<'_>, probs: &mut [u16], nbits: u32) -> Result<u32, LzmaError> {
    let mut m = 1usize;
    for _ in 0..nbits {
        let b = rc.bit(&mut probs[m])? as usize;
        m = (m << 1) | b;
    }
    Ok((m - (1usize << nbits)) as u32)
}

/// 反向位树解码（距离低位）
fn bit_tree_rev(
    rc: &mut RangeDecoder<'_>,
    probs: &mut [u16],
    base: usize,
    nbits: u32,
) -> Result<u32, LzmaError> {
    let mut m = 1usize;
    let mut res = 0u32;
    for i in 0..nbits {
        let idx = base + m;
        if idx >= probs.len() {
            return Err(LzmaError::Truncated("距离概率表越界"));
        }
        let b = rc.bit(&mut probs[idx])?;
        m = (m << 1) | b as usize;
        res |= b << i;
    }
    Ok(res)
}

struct LenDecoder {
    choice: u16,
    choice2: u16,
    low: Vec<[u16; LEN_LOW_SYMBOLS]>,
    mid: Vec<[u16; LEN_MID_SYMBOLS]>,
    high: [u16; LEN_HIGH_SYMBOLS],
}

impl LenDecoder {
    fn new(pos_states: usize) -> Self {
        Self {
            choice: PROB_INIT,
            choice2: PROB_INIT,
            low: vec![[PROB_INIT; LEN_LOW_SYMBOLS]; pos_states],
            mid: vec![[PROB_INIT; LEN_MID_SYMBOLS]; pos_states],
            high: [PROB_INIT; LEN_HIGH_SYMBOLS],
        }
    }

    fn decode(&mut self, rc: &mut RangeDecoder<'_>, pos_state: usize) -> Result<usize, LzmaError> {
        if rc.bit(&mut self.choice)? == 0 {
            Ok(bit_tree(rc, &mut self.low[pos_state], 3)? as usize)
        } else if rc.bit(&mut self.choice2)? == 0 {
            Ok(LEN_LOW_SYMBOLS + bit_tree(rc, &mut self.mid[pos_state], 3)? as usize)
        } else {
            Ok(LEN_LOW_SYMBOLS + LEN_MID_SYMBOLS + bit_tree(rc, &mut self.high, 8)? as usize)
        }
    }
}

/// 距离解码（`len` 为未 +2 的原始长度）
fn decode_distance(
    rc: &mut RangeDecoder<'_>,
    len: usize,
    pos_slot_probs: &mut [u16],
    pos_decoders: &mut [u16; 115],
    align: &mut [u16; 16],
) -> Result<u32, LzmaError> {
    let len_state = if len > 3 { 3 } else { len };
    let base = len_state * 64;
    let pos_slot = bit_tree(rc, &mut pos_slot_probs[base..base + 64], 6)?;
    if pos_slot < 4 {
        return Ok(pos_slot);
    }
    let num_direct = (pos_slot >> 1) - 1;
    let mut dist = (2 | (pos_slot & 1)) << num_direct;
    if pos_slot < 14 {
        dist += bit_tree_rev(rc, pos_decoders, (dist - pos_slot) as usize, num_direct)?;
    } else {
        dist += rc.direct_bits(num_direct - 4)? << 4;
        dist += bit_tree_rev(rc, align, 0, 4)?;
    }
    Ok(dist)
}

/// 解压一段 `props + dict + LZMA1 流`。
///
/// - `expected`：解压后字节数；给定时解到该长度即停（Inno data entry 有该值）。
///   为 `None` 时解到 EOS 标记（设置头块就是这种）。
pub fn decompress(payload: &[u8], expected: Option<usize>) -> Result<Vec<u8>, LzmaError> {
    let (props, stream) = parse_header(payload)?;
    let mut rc = RangeDecoder::new(stream)?;

    let pos_states = 1usize << props.pb;
    let mut is_match = vec![PROB_INIT; 12 * pos_states];
    let mut is_rep = [PROB_INIT; 12];
    let mut is_rep_g0 = [PROB_INIT; 12];
    let mut is_rep_g1 = [PROB_INIT; 12];
    let mut is_rep_g2 = [PROB_INIT; 12];
    let mut is_rep0_long = vec![PROB_INIT; 12 * pos_states];
    let mut pos_slot_probs = vec![PROB_INIT; 4 * 64];
    let mut pos_decoders = [PROB_INIT; 115];
    let mut align = [PROB_INIT; 16];
    let mut len_dec = LenDecoder::new(pos_states);
    let mut rep_len_dec = LenDecoder::new(pos_states);
    let mut literals = vec![PROB_INIT; 0x300usize << (props.lc + props.lp)];

    // 输出上限：显式长度 + 余量；无长度时给一个保守上限防止坏数据撑爆内存
    let hard_limit = match expected {
        Some(n) => n.saturating_add(1 << 20),
        None => 512 << 20,
    };

    // 用字典大小给输出缓冲一个上界提示（解码本身靠距离校验保证正确性）
    let dict_hint = (props.dict_size as usize).min(expected.unwrap_or(1 << 16));
    let mut out: Vec<u8> = Vec::with_capacity(dict_hint.min(64 << 20));
    let mut state = 0usize;
    let mut reps = [0u32; 4];

    loop {
        if let Some(n) = expected {
            if out.len() >= n {
                out.truncate(n);
                break;
            }
        }

        let pos_state = out.len() & (pos_states - 1);
        if rc.bit(&mut is_match[state * pos_states + pos_state])? == 0 {
            // 字面量
            let prev = *out.last().unwrap_or(&0) as usize;
            let lit_state =
                ((out.len() & ((1usize << props.lp) - 1)) << props.lc) + (prev >> (8 - props.lc));
            let probs = &mut literals[lit_state * 0x300..lit_state * 0x300 + 0x300];
            let mut symbol = 1usize;
            if state >= 7 {
                let dist = reps[0] as usize + 1;
                let mut match_byte = *out
                    .get(out.len().wrapping_sub(dist))
                    .filter(|_| dist <= out.len())
                    .ok_or(LzmaError::BadDistance)? as usize;
                while symbol < 0x100 {
                    let match_bit = (match_byte >> 7) & 1;
                    match_byte <<= 1;
                    let bit = rc.bit(&mut probs[((1 + match_bit) << 8) + symbol])? as usize;
                    symbol = (symbol << 1) | bit;
                    if match_bit != bit {
                        break;
                    }
                }
            }
            while symbol < 0x100 {
                let bit = rc.bit(&mut probs[symbol])? as usize;
                symbol = (symbol << 1) | bit;
            }
            out.push((symbol - 0x100) as u8);
            state = STATE_UPDATE_CHAR[state];
            continue;
        }

        // 匹配（新距离或 rep）
        let len;
        if rc.bit(&mut is_rep[state])? == 1 {
            if rc.bit(&mut is_rep_g0[state])? == 0 {
                // rep0，长度可能是 1（短 rep）
                if rc.bit(&mut is_rep0_long[state * pos_states + pos_state])? == 0 {
                    state = if state < 7 { 9 } else { 11 };
                    let dist = reps[0] as usize + 1;
                    if dist > out.len() {
                        return Err(LzmaError::BadDistance);
                    }
                    let b = out[out.len() - dist];
                    out.push(b);
                    if out.len() > hard_limit {
                        return Err(LzmaError::TooLarge);
                    }
                    continue;
                }
            } else {
                let idx = if rc.bit(&mut is_rep_g1[state])? == 0 {
                    1
                } else if rc.bit(&mut is_rep_g2[state])? == 0 {
                    2
                } else {
                    3
                };
                let dist = reps[idx];
                for i in (0..idx).rev() {
                    reps[i + 1] = reps[i];
                }
                reps[0] = dist;
            }
            len = rep_len_dec.decode(&mut rc, pos_state)?;
            state = if state < 7 { 8 } else { 11 };
        } else {
            reps[3] = reps[2];
            reps[2] = reps[1];
            reps[1] = reps[0];
            len = len_dec.decode(&mut rc, pos_state)?;
            state = if state < 7 { 7 } else { 10 };
            let dist = decode_distance(
                &mut rc,
                len,
                &mut pos_slot_probs,
                &mut pos_decoders,
                &mut align,
            )?;
            if dist == 0xFFFF_FFFF {
                break; // EOS 标记
            }
            reps[0] = dist;
        }

        let dist = reps[0] as usize + 1;
        if dist > out.len() {
            return Err(LzmaError::BadDistance);
        }
        let copy_len = len + 2;
        if out.len() + copy_len > hard_limit {
            return Err(LzmaError::TooLarge);
        }
        // 批量复制（距离小于长度时按 dist 分段，天然处理自引用重叠）
        let mut remaining = copy_len;
        while remaining > 0 {
            let n = remaining.min(dist);
            let start = out.len() - dist;
            out.extend_from_within(start..start + n);
            remaining -= n;
        }
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 已知向量：`b"Hello, Inno Setup! " × 3`，lc=3/lp=0/pb=2/dict=4096。
    /// 由 Python `lzma.compress(format=FORMAT_RAW, filters=[FILTER_LZMA1])` 生成。
    const VEC_TEXT: &str =
        "5d0010000000241949986f160287596984671c682dfec14e5b7b56ae7eabadfffff95d2000";
    /// 已知向量：`bytes(range(256)) × 8`（2048 字节，覆盖字面量 + 长距离匹配）。
    const VEC_RANGE: &str = "5d0010000000000052500a84f99bb28021a969d627e03e065a5f048d53d404ba39570509c15524de9db871593160a19ff96f4973f2c8ea8cba1a8b29692180fe338366af466dec9e898a0b83f03c0e898e3fed5fe79e90d91cff32f4b2e03951b2d21415b4c571badb06e3799a9fbb38c1b000ac930baa0619031208155b9bc848f0322efe2da087c8f0a4e0d251eb8d675692b24d84c5f18631df6a625bc2792dd9f73c73ba747407d83ca9562224a166f85a845f3067d2f64b492e7f20ebdbf8100e947877c73f6befb4cd95e26ff6446e06cf0b821acbdb7af0578d98ff90c03ee6c1124175ee032896eb13fba728ccaf32bba40e25f2563cf81a87fffeed4fa0";

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap())
            .collect()
    }

    #[test]
    fn decodes_known_vector_to_eos() {
        let out = decompress(&hex(VEC_TEXT), None).expect("decode");
        assert_eq!(out, b"Hello, Inno Setup! ".repeat(3));
    }

    #[test]
    fn decodes_known_vector_with_known_size() {
        let out = decompress(&hex(VEC_RANGE), Some(2048)).expect("decode");
        let mut want = Vec::new();
        for _ in 0..8 {
            want.extend(0u8..=255);
        }
        assert_eq!(out, want);
    }

    #[test]
    fn rejects_bad_props() {
        let mut bad = hex(VEC_TEXT);
        bad[0] = 0xFF;
        assert!(decompress(&bad, None).is_err());
    }
}
