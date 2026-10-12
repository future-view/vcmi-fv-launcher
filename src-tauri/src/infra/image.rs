//! 图像编解码：H3 专属 PCX、BMP 渲染、WebP 编码（有损 / 无损）。
//!
//! - `decode_h3_pcx`：H3 专有 "PCX"（`CBitmapHandler.cpp:28-105`，非标准 PCX）；
//! - `decode_pcx`：标准 PCX；
//! - `encode_webp*`：图标类小图走 libwebp 有损（约为无损 1/3 耗时），大图走 `image` 无损。


/// H3 专有 "PCX"（client/render/CBitmapHandler.cpp:28-105，`loadH3PCX`）：
/// 12 字节头 = u32 fSize / u32 width / u32 height，其后为像素数据，8bpp 时末尾另含 256×3 调色板。
/// 区分 8bpp/24bpp **必须看 fSize**（与 VCMI `isPCX` 一致）：`fSize==w*h` 为 8bpp、`fSize==w*h*3` 为 24bpp。
/// H3bitmap.lod 里的城镇背景（TBCSBACK.PCX 等）即是 **24bpp BGR888**（非标准 PCX，无 0x0A 魔数，
/// 也不能用 `decode_pcx`），原版英雄头像（HPL000KN.PCX）则是 8bpp。透明色为**洋红 (0,255,255)**，
/// 不是索引 0（否则会误把背景填色变透明）。
pub(crate) fn decode_h3_pcx(buf: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    if buf.len() < 12 {
        return None;
    }
    let w = u32::from_le_bytes([buf[4], buf[5], buf[6], buf[7]]);
    let h = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
    if w == 0 || h == 0 || w > 4096 || h > 4096 {
        return None;
    }
    let px = (w as usize) * (h as usize);
    let fsize = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    // 8bpp：12 头 + w*h 索引 + 768 调色板（fSize==w*h 与 VCMI 同款判定，避免 24bpp 文件误入此分支）
    if fsize == px && buf.len() >= 12 + px + 768 {
        let pal = buf.len() - 768;
        let mut rgba = Vec::with_capacity(px * 4);
        for i in 0..px {
            let v = buf[12 + i] as usize;
            let o = pal + v * 3;
            let r = buf[o];
            let g = buf[o + 1];
            let b = buf[o + 2];
            // VCMI：调色板为洋红 (0,255,255) 的像素透明
            let a = if r == 0 && g == 255 && b == 255 { 0u8 } else { 255u8 };
            rgba.extend_from_slice(&[r, g, b, a]);
        }
        return Some((w, h, rgba));
    }
    // 24bpp：12 头 + w*h*3（BGR888，VCMI `Format_BGR888` —— 字节序为 B,G,R，需交换 R/B）
    if fsize == px * 3 && buf.len() >= 12 + px * 3 {
        let mut rgba = Vec::with_capacity(px * 4);
        for i in 0..px {
            let o = 12 + i * 3;
            let b = buf[o];
            let g = buf[o + 1];
            let r = buf[o + 2];
            rgba.extend_from_slice(&[r, g, b, 255]);
        }
        return Some((w, h, rgba));
    }
    None
}

/// 标准 PCX 解码：8bpp 单平面 + RLE + 尾部 768B 调色板（image crate 0.25 未内置 PCX）。
pub(crate) fn decode_pcx(buf: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    if buf.len() < 128 + 768 || buf[0] != 0x0A || buf[3] != 8 || buf[65] != 1 {
        return None;
    }
    let xmin = u16::from_le_bytes([buf[4], buf[5]]) as u32;
    let ymin = u16::from_le_bytes([buf[6], buf[7]]) as u32;
    let xmax = u16::from_le_bytes([buf[8], buf[9]]) as u32;
    let ymax = u16::from_le_bytes([buf[10], buf[11]]) as u32;
    let bpl = u16::from_le_bytes([buf[66], buf[67]]) as usize;
    let w = xmax.saturating_add(1).saturating_sub(xmin);
    let h = ymax.saturating_add(1).saturating_sub(ymin);
    if w == 0 || h == 0 || bpl == 0 || bpl < w as usize {
        return None;
    }
    // 调色板固定在文件末尾 768 字节（前导 0x0C 标记）
    let pal_start = buf.len() - 768;
    let mut idx: Vec<u8> = Vec::with_capacity(h as usize * bpl);
    let mut p = 128usize;
    while p < pal_start && idx.len() < h as usize * bpl {
        let b = buf[p];
        p += 1;
        if b & 0xC0 == 0xC0 {
            let run = (b & 0x3F) as usize;
            if p >= pal_start {
                break;
            }
            let v = buf[p];
            p += 1;
            for _ in 0..run {
                idx.push(v);
            }
        } else {
            idx.push(b);
        }
    }
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let v = idx.get(y * bpl + x).copied().unwrap_or(0) as usize;
            let o = pal_start + v * 3;
            if o + 2 < buf.len() {
                // H3 头像以调色板 0 号色为透明（与 VCMI 一致）
                let a = if v == 0 { 0u8 } else { 255u8 };
                rgba.push(buf[o]);
                rgba.push(buf[o + 1]);
                rgba.push(buf[o + 2]);
                rgba.push(a);
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    Some((w, h, rgba))
}

/// 图标类小图的最大边长：不超过则走 libwebp 有损。
/// Wiki 图标（生物/英雄/法术/宝物/技能/建筑）基本都在 128px 内，
/// 而城镇背景与合成预览图（800×600 级）远超此值，保持无损以免渐变天空出现色带。
pub(crate) const LOSSY_MAX_EDGE: u32 = 256;

/// 有损质量（0-100）：90 对像素图标几乎无视觉差异，编码耗时约为无损（VP8L）的 1/3。
pub(crate) const LOSSY_QUALITY: f32 = 90.0;

/// 解码 DDS 图集（H3 高清版素材：DXT1/DXT5，见 `infra::pak`）。
///
/// 只解**基础层**（HD pak 的 sheet 无 mipmap 链）；返回整张图集的 RGBA + 尺寸。
/// 帧裁剪由调用方（`vcmi::hd_edition`）按 pak 帧矩形完成 —— 同一 sheet 常被多帧共享，
/// 先整张解一次比"每帧解一遍"划算得多。
pub(crate) fn decode_dds(buf: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let img = image::load_from_memory_with_format(buf, image::ImageFormat::Dds).ok()?;
    let rgba = img.to_rgba8();
    Some((rgba.width(), rgba.height(), rgba.into_raw()))
}

/// 按尺寸选择编码策略：小图走 libwebp 有损（快），大图走 image 无损（视觉敏感且数量少）。
/// 有损失败时回退无损——容错底线：绝不因编码失败丢图。
pub(crate) fn encode_rgba_auto(rgba: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    crate::infra::metrics::scope("img.rgba", || {
        let out = encode_rgba_auto_inner(rgba, w, h)?;
        crate::infra::metrics::bytes("img.bytes.out", out.len() as u64);
        Some(out)
    })
}

fn encode_rgba_auto_inner(rgba: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    if w.max(h) <= LOSSY_MAX_EDGE {
        if let Some(v) = encode_webp_lossy(rgba, w, h) {
            return Some(v);
        }
    }
    encode_webp_lossless(rgba, w, h)
}

/// 无损 webp 编码（VP8L）。
///
/// 供**像素画**使用：硬边 + 纯色块对有损压缩极敏感（振铃/色块会直接把观感毁掉），
/// 而放大后的像素画体积本就不大（58×64 → 4x 无损约 12 KB）。
pub(crate) fn encode_webp_lossless(rgba: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    if rgba.len() < (w as usize) * (h as usize) * 4 {
        return None;
    }
    let mut out = Vec::new();
    let enc = image::codecs::webp::WebPEncoder::new_lossless(&mut out);
    enc.encode(rgba, w, h, image::ExtendedColorType::Rgba8).ok()?;
    Some(out)
}

/// libwebp 有损编码（RGBA，含 alpha 通道——alpha 分区由 libwebp 单独处理，不受质量影响）。
pub(crate) fn encode_webp_lossy(rgba: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    if rgba.len() < (w as usize) * (h as usize) * 4 {
        return None;
    }
    let mem = webp::Encoder::from_rgba(rgba, w, h).encode(LOSSY_QUALITY);
    let out = mem.to_vec();
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// 高清变体（`@Nx`）专用编码：**一律有损**，质量可调。
///
/// 为什么不用 `encode_rgba_auto`：它按"最大边长 ≤256 才有损"分流，而高清变体放大后
/// 必然超过该阈值（幕布 2x 就是 1600×748）→ 走无损 → **1600×748 约 1.5 MB/张**，
/// 全量会让缓存从 134 MB 膨胀到 717 MB。
///
/// 放大后的图有损压缩更划算：色带随分辨率提升本就更不可见，而体积可降 5~8 倍。
/// 质量由 `hd::HD_QUALITY` 统一给出（经 bench 在"观感 / 体积"间取平衡）。
pub(crate) fn encode_webp_lossy_q(rgba: &[u8], w: u32, h: u32, q: f32) -> Option<Vec<u8>> {
    if rgba.len() < (w as usize) * (h as usize) * 4 {
        return None;
    }
    let mem = webp::Encoder::from_rgba(rgba, w, h).encode(q);
    let out = mem.to_vec();
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// H3 系素材的**位图色键**色：纯青 `(0,255,255)`。
///
/// 与既有两条解码路径同款：`decode_h3_pcx`（调色板洋青像素透明）、
/// `infra::def` 的 `palette[0]` 色键。BMP/PNG 这类**没有 alpha 通道**的位图
/// 没有调色板可依，只能沿用这个约定色表达透明。
const H3_COLOR_KEY: [u8; 3] = [0x00, 0xFF, 0xFF];

/// 位图解码为 RGBA，并对**无 alpha 通道**的位图套用 H3 色键（纯青 → 透明）。
///
/// ## 触发条件（两条都要满足）
///
/// - **没有 alpha 通道**（`DynamicImage::has_alpha()` 为假）：带 alpha 的图"透明与否"已由
///   alpha 明确表达，再抠色键只会误伤真实的青色像素（不同 mod 的色键色并不统一）；
/// - 且图中**确实存在**该色：否则原样返回 `None`，调用方零额外开销（不重编码）。
///
/// ## 为什么要有它
///
/// 实测两类素材都靠纯青色表达透明，缺了这一步就是"糊一块青色"：
/// - `New Pavilion` 的 grail 动画帧（`Sprites/towndefsPavilion/grail/NPavGrail ani*.png`，
///   409×164 RGB）用纯青铺"画面外"区域 → 城镇幕布左上角一块纯青矩形；
/// - `hota.Factory` 的法术图标（`content/Sprites/spells/iconCancel.bmp`，78×65）用纯青当底
///   → 百科法术卡片带青底。
///
/// ⚠️ **不适用于幕布底图**：底图是最终画布（VCMI 亦原样渲染），抠洞只会露出卡片底色，
/// 因此 [`encode_webp`] 保持不变 —— 只对"要贴到别的东西上面的图"（结构帧 / 图标）生效。
pub(crate) fn keyed_rgba(img: image::DynamicImage) -> image::RgbaImage {
    let has_alpha = img.has_alpha();
    let mut rgba = img.to_rgba8();
    if has_alpha {
        return rgba;
    }
    for px in rgba.chunks_exact_mut(4) {
        if px[0] == H3_COLOR_KEY[0] && px[1] == H3_COLOR_KEY[1] && px[2] == H3_COLOR_KEY[2] {
            px[3] = 0;
        }
    }
    rgba
}

/// 同上，但入口是**字节**：命中色键才返回 `Some`（已抠好的 RGBA），未命中返回 `None`。
///
/// 供"先判断要不要处理、再决定要不要重编码"的调用点使用（如 [`crate::modcache::assets`]
/// 的图标落盘），避免为每张图都白做一次编码。
pub(crate) fn strip_color_key_rgba(buf: &[u8]) -> Option<image::RgbaImage> {
    let img = image::load_from_memory(buf).ok()?;
    let has_alpha = img.has_alpha();
    let mut rgba = img.to_rgba8();
    if has_alpha {
        return None;
    }
    let mut hit = false;
    for px in rgba.chunks_exact_mut(4) {
        if px[0] == H3_COLOR_KEY[0] && px[1] == H3_COLOR_KEY[1] && px[2] == H3_COLOR_KEY[2] {
            px[3] = 0;
            hit = true;
        }
    }
    if hit {
        Some(rgba)
    } else {
        None
    }
}

/// 将位图字节（bmp/png/jpg/pcx）编码为 webp。失败返回 None。
/// 编码策略见 `encode_rgba_auto`：小图有损、大图无损。
pub(crate) fn encode_webp(buf: &[u8], ext: &str) -> Option<Vec<u8>> {
    crate::infra::metrics::count("img.encode", 1);
    if ext.eq_ignore_ascii_case("pcx") {
        let (w, h, rgba) = decode_h3_pcx(buf).or_else(|| decode_pcx(buf))?;
        return encode_rgba_auto(&rgba, w, h);
    }
    if ext.eq_ignore_ascii_case("def") {
        // DEF 先解首帧为 BMP
        let bmp = crate::infra::def::DefReader::new(buf).first_frame_bmp()?;
        return encode_webp(&bmp, "bmp");
    }
    if buf.is_empty() {
        return None;
    }
    let img = image::load_from_memory(buf).ok()?;
    let rgba = img.to_rgba8();
    encode_rgba_auto(rgba.as_raw(), rgba.width(), rgba.height())
}

/// 将已解码的 32bpp BGRA 像素编码为 webp（无损）。供 DEF 动画帧直接编码。
/// 注意：DefReader 输出为 BGRA 顶向下，此处需交换 R/B 后再按 RGBA 编码。
pub(crate) fn encode_webp_from_bgra(bgra: &[u8], w: u32, h: u32) -> Option<Vec<u8>> {
    crate::infra::metrics::count("img.encode", 1);
    if bgra.len() < (w as usize * h as usize * 4) {
        return None;
    }
    let mut rgba = bgra[..(w as usize * h as usize * 4)].to_vec();
    for px in rgba.chunks_exact_mut(4) {
        px.swap(0, 2); // B,G,R,A -> R,G,B,A
    }
    encode_rgba_auto(&rgba, w, h)
}
