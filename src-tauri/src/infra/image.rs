//! 图像编解码：H3 专属 PCX、BMP 渲染、WebP 编码（有损 / 无损）。
//!
//! - `decode_h3_pcx`：H3 专有 "PCX"（`CBitmapHandler.cpp:28-105`，非标准 PCX）；
//! - `decode_pcx`：标准 PCX；
//! - `encode_webp*`：图标类小图走 libwebp 有损（约为无损 1/3 耗时），大图走 `image` 无损。

/// H3 专有 "PCX"（client/render/CBitmapHandler.cpp:28-105，`loadH3PCX`）：
/// 12 字节头 = u32 fSize / u32 width / u32 height，其后为像素数据，8bpp 时末尾另含 256×3 调色板。
/// 区分 8bpp/24bpp **必须看 fSize**（与 VCMI `isPCX` 一致）：`fSize==w*h` 为 8bpp、`fSize==w*h*3` 为 24bpp。
/// H3bitmap.lod 里的城镇背景（TBCSBACK.PCX 等）即是 **24bpp BGR888**（非标准 PCX，无 0x0A 魔数，
/// 也不能用 `decode_pcx`），原版英雄头像（HPL000KN.PCX）则是 8bpp。
///
/// 8bpp 的透明口径见 [`IndexedPalette`]（VCMI `BitmapHandler::loadBitmapFromDir`）；
/// **24bpp 一律不透明**（VCMI 会额外把纯青设为色键，但那类图（城镇幕布等）在百科里是
/// “最终画布”，抠洞只会露出卡片底色，故此处有意不抠 —— 已取证的偏离，见 `encode_webp` 注释）。
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
        let rule = IndexedPalette::new(&buf[pal..]);
        let mut rgba = Vec::with_capacity(px * 4);
        for i in 0..px {
            let idx = buf[12 + i];
            let o = pal + (idx as usize) * 3;
            let (r, g, b) = (buf[o], buf[o + 1], buf[o + 2]);
            match rule.alpha(idx) {
                // 透明像素保留原 RGB（后续 `hd` 的透明区色膨胀靠它，勿清零）
                Some(0) => rgba.extend_from_slice(&[r, g, b, 0]),
                Some(a) => rgba.extend_from_slice(&[0, 0, 0, a]),
                None => rgba.extend_from_slice(&[r, g, b, 255]),
            }
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
///
/// 透明口径与 [`decode_h3_pcx`] 一致，见 [`IndexedPalette`]（旧实现无条件抠索引 0，
/// 与 VCMI 不符：VCMI 只在“调色板 0 号是洋红”或“调色板里含纯青”时才设色键）。
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
    let rule = IndexedPalette::new(&buf[pal_start..]);
    for y in 0..h as usize {
        for x in 0..w as usize {
            let idx = idx.get(y * bpl + x).copied().unwrap_or(0);
            let o = pal_start + (idx as usize) * 3;
            if o + 2 < buf.len() {
                let (r, g, b) = (buf[o], buf[o + 1], buf[o + 2]);
                match rule.alpha(idx) {
                    Some(0) => rgba.extend_from_slice(&[r, g, b, 0]),
                    Some(a) => rgba.extend_from_slice(&[0, 0, 0, a]),
                    None => rgba.extend_from_slice(&[r, g, b, 255]),
                }
            } else {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
            }
        }
    }
    Some((w, h, rgba))
}

/// 8bpp 索引图的**色键与阴影**判定，逐字对齐 VCMI
/// `BitmapHandler::loadBitmapFromDir`（clientsdl2/render/CBitmapHandler.cpp:163-184）。
///
/// VCMI 对带调色板的位图只走两条分支：
/// 1. **调色板 0 号 == 洋红 (255,0,255)** —— H3「带阴影」约定：色键 = 0 号索引，
///    且 1 号色变黑 12.5%、4 号色变黑 50%（`setColorKey` + 调色板改写）；
/// 2. 其余 —— `setDefaultColorKeyPresize`：只有调色板里**恰好存在纯青 (0,255,255)** 时，
///    才把该索引设为色键；找不到就整图不透明（例如原版战斗背景，它本来就没有透明色）。
///
/// ⚠️ 容差恒为 0：SDL 色键是精确像素值，VCMI 里不存在“相近色也算背景”的判定。
pub(crate) struct IndexedPalette {
    /// 命中即透明的索引；`None` = 整图不透明。
    key: Option<u8>,
    /// 调色板 0 号是洋红 ⇒ 索引 1/4 是阴影档位。
    shadow: bool,
}

impl IndexedPalette {
    fn new(palette: &[u8]) -> Self {
        let at = |i: usize| -> [u8; 3] {
            [
                palette.get(i * 3).copied().unwrap_or(0),
                palette.get(i * 3 + 1).copied().unwrap_or(0),
                palette.get(i * 3 + 2).copied().unwrap_or(0),
            ]
        };
        if at(0) == [255, 0, 255] {
            return Self {
                key: Some(0),
                shadow: true,
            };
        }
        Self {
            key: (0..=255u8).find(|i| at(*i as usize) == H3_COLOR_KEY),
            shadow: false,
        }
    }

    /// `None` = 普通像素（调色板原色 + 不透明）；`Some(a)` = 特殊像素：
    /// `0` 为透明（RGB 由调用方保留），其余为阴影黑半透明。
    fn alpha(&self, idx: u8) -> Option<u8> {
        if self.key == Some(idx) {
            return Some(0);
        }
        if self.shadow {
            match idx {
                1 => return Some(32),
                4 => return Some(128),
                _ => {}
            }
        }
        None
    }
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
    enc.encode(rgba, w, h, image::ExtendedColorType::Rgba8)
        .ok()?;
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
/// 即 VCMI 的 `Colors::DEFAULT_KEY_COLOR`（client/render/Colors.cpp:26）——
/// 真彩无 alpha 的位图由 `CSDL_Ext::setDefaultColorKey` 无条件设成这个值作色键，
/// 8bpp 位图则由 `setDefaultColorKeyPresize` 在调色板里找它。
/// **容差恒为 0**：SDL 色键是精确像素值，VCMI 不存在“相近色也算背景”的判定。
const H3_COLOR_KEY: [u8; 3] = [0x00, 0xFF, 0xFF];

/// 位图解码为 RGBA，并对**无 alpha 通道**的位图套用 H3 色键（纯青 → 透明）。
///
/// 这是 VCMI `BitmapHandler::loadBitmapFromDir` 最后两个分支的等价物：
/// 带 alpha → 用 alpha（`SDL_SetSurfaceBlendMode(BLEND)`）；无 alpha → 色键 = 纯青。
///
/// ## 触发条件（两条都要满足）
///
/// - **没有 alpha 通道**（`DynamicImage::has_alpha()` 为假）：带 alpha 的图"透明与否"已由
///   alpha 明确表达，再抠色键只会误伤真实的青色像素；
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
///
/// ⚠️ **不得扩张成“相近色也算”**（2026-10-24 回归）：曾经的“四角取色 + 容差 48”
/// 把 58×64 的头像/生物大图抠成麻点。VCMI 只认精确纯青，这里也一样。
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

/// H3 索引图的**索引语义**：0 与 5–7 全透明，1–4 是阴影档位（半透明黑）。
///
/// 对齐 VCMI `ScalableImageParameters::setShadowTransparency`
/// （`clientsdl2/render/ScalableImage.cpp:177-203`）：战斗动画按
/// `EImageBlitMode::WITH_SHADOW*` 加载，**无条件**把调色板 0/1/4 换成
/// 全透明 / 黑 25%（alpha 64）/ 黑 50%（alpha 128）；2、3 是雾战阴影，同档位处理。
/// 5–7 是选择框与旗帜色 —— 百科不画选择框，全透明。
///
/// ⚠️ 这些索引的**调色板颜色是随意的**（实测 HotA 生物帧：0=青、1=浅洋红、4=洋红），
/// 所以**只能按索引判定**，不能按颜色：`image` crate 展开成 RGBA 后索引就没了，
/// 这正是"png 动画带天蓝幕布 + 脚下洋红星"的根因。
pub(crate) const H3_SPECIAL_ALPHA: [u8; 8] = [0, 64, 64, 128, 128, 0, 0, 0];

/// 索引 → 特殊像素（透明 / 阴影）；`None` = 该索引是普通颜色。
pub(crate) fn h3_index_rgba(idx: u8) -> Option<[u8; 4]> {
    match idx {
        0 | 5..=7 => Some([0, 0, 0, 0]),
        1..=4 => Some([0, 0, 0, H3_SPECIAL_ALPHA[idx as usize]]),
        _ => None,
    }
}

/// **调色板 PNG**（color type 3）按索引解码为 RGBA，套用 [`h3_index_rgba`] 的索引语义。
///
/// 只处理 8bpp 索引（H3 素材的实际情况，实测 HotA 生物战斗帧 450×400 / 8bpp）；
/// 其它位深或带 `tRNS` 的图返回 `None`，由调用方回退到 `image` 的常规解码：
/// - 带 `tRNS` 时透明由 alpha 表达（VCMI 的 SDL 端同样会把它展开成带 alpha 的图），
///   此时不做索引替换，避免把作者显式设成半透明的像素再按阴影重算。
pub(crate) fn decode_paletted_png(buf: &[u8]) -> Option<image::RgbaImage> {
    // 先按 PNG 魔数短路：非 PNG（bmp/webp/jpg）不必进解码器
    if !buf.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        return None;
    }
    let mut decoder = png::Decoder::new(std::io::Cursor::new(buf));
    decoder.set_transformations(png::Transformations::IDENTITY);
    let mut reader = decoder.read_info().ok()?;
    // 借用 `reader.info()` 期间不能取帧，故一次性把需要的字段拷出来
    let palette = {
        let info = reader.info();
        if info.color_type != png::ColorType::Indexed || info.bit_depth != png::BitDepth::Eight {
            return None;
        }
        if info.trns.is_some() {
            return None;
        }
        info.palette.as_ref()?.to_vec()
    };
    let mut indices = vec![0u8; reader.output_buffer_size()];
    let frame = reader.next_frame(&mut indices).ok()?;
    let px = (frame.width as usize) * (frame.height as usize);
    if indices.len() < px {
        return None;
    }
    let mut rgba = Vec::with_capacity(px * 4);
    for &idx in &indices[..px] {
        if let Some(special) = h3_index_rgba(idx) {
            rgba.extend_from_slice(&special);
            continue;
        }
        let o = (idx as usize) * 3;
        // 越界索引按黑处理（坏文件不应 panic）
        let r = palette.get(o).copied().unwrap_or(0);
        let g = palette.get(o + 1).copied().unwrap_or(0);
        let b = palette.get(o + 2).copied().unwrap_or(0);
        rgba.extend_from_slice(&[r, g, b, 255]);
    }
    image::RgbaImage::from_raw(frame.width, frame.height, rgba)
}

/// **动画帧**的统一解码入口：索引图走索引语义（阴影/透明），其余走位图色键。
///
/// 供 sprites-json（png 动画）等"要把帧叠到别的画面上"的链路使用；
/// DEF 帧不走这里（它们由 `infra::def` 按同一套索引语义直接渲染 BGRA）。
pub(crate) fn decode_anim_frame(buf: &[u8]) -> Option<image::RgbaImage> {
    if let Some(img) = decode_paletted_png(buf) {
        return Some(img);
    }
    let img = image::load_from_memory(buf).ok()?;
    Some(keyed_rgba(img))
}

/// 同上，但入口是**字节**：命中色键才返回 `Some`（已抠好的 RGBA），未命中返回 `None`。
///
/// 供"先判断要不要处理、再决定要不要重编码"的调用点使用（如 [`crate::modcache::assets`]
/// 的图标落盘），避免为每张图都白做一次编码。
///
/// 这是**真彩无 alpha 位图唯一合法的透明来源**（VCMI `setDefaultColorKey` = 精确纯青）。
/// 调用方不得再叠任何“按内容猜背景色”的二次抠图：
/// 实测 58×64 的头像 / 生物大图标源图全不透明、四角是画面内容，容差抠色会把它们抠成麻点。
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

#[cfg(test)]
mod tests {
    use super::{decode_paletted_png, h3_index_rgba, strip_color_key_rgba, IndexedPalette};

    /// 造 768 字节调色板，只填给定索引。
    fn palette_with(entries: &[(usize, [u8; 3])]) -> Vec<u8> {
        let mut p = vec![0u8; 768];
        for &(i, c) in entries {
            p[i * 3..i * 3 + 3].copy_from_slice(&c);
        }
        p
    }

    /// 8bpp 色键判定必须与 VCMI `BitmapHandler::loadBitmapFromDir` 同口径（容差 0）。
    #[test]
    fn 索引图色键只看洋红分支与纯青项() {
        // ① 调色板 0 号是洋红 → 键索引 0，且 1/4 是阴影档位（H3 带阴影约定）
        let magenta = IndexedPalette::new(&palette_with(&[
            (0, [255, 0, 255]),
            (1, [255, 150, 255]),
            (4, [255, 0, 255]),
        ]));
        assert_eq!(magenta.alpha(0), Some(0), "洋红调色板：索引 0 透明");
        assert_eq!(magenta.alpha(1), Some(32), "索引 1 = 黑 12.5%");
        assert_eq!(magenta.alpha(4), Some(128), "索引 4 = 黑 50%");
        assert_eq!(magenta.alpha(2), None, "2/3 号色不强行当阴影（VCMI 要颜色相符才算）");

        // ② 非洋红调色板：只有「恰好等于纯青」的索引才设色键
        let cyan = IndexedPalette::new(&palette_with(&[(0, [0, 0, 0]), (7, [0, 255, 255])]));
        assert_eq!(cyan.alpha(7), Some(0), "调色板含纯青 → 键该索引");
        assert_eq!(cyan.alpha(0), None, "0 号色不是色键就不透明（旧实现无条件抠 0）");
        // 相近色不键（容差被压到 0）：这是 58×64 画像被抠成麻点的根源
        let near = IndexedPalette::new(&palette_with(&[(0, [0, 0, 0]), (7, [2, 240, 245])]));
        assert_eq!(near.alpha(7), None, "近似青不属于色键");

        // ③ 无洋红也无纯青 → 整图不透明（如原版战斗背景，本就无透明色）
        let none = IndexedPalette::new(&palette_with(&[(0, [12, 34, 56])]));
        assert_eq!(none.alpha(0), None);
        assert_eq!(none.alpha(200), None);
    }

    /// 真彩无 alpha 且不含纯青的图（头像 / 生物大图标的典型形态）→ **不得抠任何像素**。
    #[test]
    fn 无alpha且无纯青的真彩图不抠色() {
        // 4×1 RGB：四角暖色（画面内容），不含纯青
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, 4, 1);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().expect("写 PNG 头");
            w.write_image_data(&[166, 27, 26, 195, 175, 130, 153, 90, 64, 89, 47, 42])
                .expect("写像素");
            w.finish().expect("收尾");
        }
        assert!(
            strip_color_key_rgba(&out).is_none(),
            "全不透明且无纯青的位图不得被抠色（曾因四角容差被抠成麻点）"
        );
    }

    /// 真彩无 alpha 且含**精确纯青** → 只抠这些像素，RGB 保留、相近色不动。
    #[test]
    fn 真彩图只抠精确纯青() {
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, 3, 1);
            enc.set_color(png::ColorType::Rgb);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().expect("写 PNG 头");
            // 纯青 / 近似青（2,240,245）/ 普通色
            w.write_image_data(&[0, 255, 255, 2, 240, 245, 10, 20, 30])
                .expect("写像素");
            w.finish().expect("收尾");
        }
        let img = strip_color_key_rgba(&out).expect("含纯青应命中");
        assert_eq!(img.get_pixel(0, 0).0, [0, 255, 255, 0], "精确纯青透明，RGB 保留");
        assert_eq!(img.get_pixel(1, 0).0[3], 255, "近似青不抠（容差 0）");
        assert_eq!(img.get_pixel(2, 0).0[3], 255, "普通色不抠");
    }

    /// 索引语义：0 与 5–7 全透明，1–4 是半透明黑阴影，其余是普通颜色。
    #[test]
    fn h3_索引语义() {
        assert_eq!(h3_index_rgba(0), Some([0, 0, 0, 0]));
        assert_eq!(h3_index_rgba(1), Some([0, 0, 0, 64]));
        assert_eq!(h3_index_rgba(2), Some([0, 0, 0, 64]));
        assert_eq!(h3_index_rgba(3), Some([0, 0, 0, 128]));
        assert_eq!(h3_index_rgba(4), Some([0, 0, 0, 128]));
        assert_eq!(h3_index_rgba(5), Some([0, 0, 0, 0]));
        assert_eq!(h3_index_rgba(6), Some([0, 0, 0, 0]));
        assert_eq!(h3_index_rgba(7), Some([0, 0, 0, 0]));
        assert_eq!(h3_index_rgba(8), None, "8 起是普通颜色");
    }

    /// 实测形态：HotA 生物帧的调色板 = [0]=青、(1)=浅洋红、(4)=洋红。
    /// 索引图必须按**索引**（0 透明、1/4 阴影）还原，而不是照抄 RGB。
    #[test]
    fn 调色板png按索引还原透明与阴影() {
        // 5×1 索引 PNG：0=青（色键）、1=浅洋红（阴影边框）、8=普通红、4=洋红（阴影本体）、5=选择色
        let mut palette = vec![0u8; 9 * 3];
        palette[0..3].copy_from_slice(&[0, 255, 255]);
        palette[3..6].copy_from_slice(&[255, 150, 255]);
        palette[12..15].copy_from_slice(&[255, 0, 255]);
        palette[24..27].copy_from_slice(&[255, 0, 0]);
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, 5, 1);
            enc.set_color(png::ColorType::Indexed);
            enc.set_depth(png::BitDepth::Eight);
            enc.set_palette(palette);
            let mut writer = enc.write_header().expect("写 PNG 头");
            writer.write_image_data(&[0u8, 1, 8, 4, 5]).expect("写像素");
            writer.finish().expect("收尾");
        }
        let img = decode_paletted_png(&out).expect("应识别为索引 PNG");
        assert_eq!(img.width(), 5);
        assert_eq!(
            img.get_pixel(0, 0).0,
            [0, 0, 0, 0],
            "索引 0（青）必须透明，而不是留下一块青"
        );
        assert_eq!(
            img.get_pixel(1, 0).0,
            [0, 0, 0, 64],
            "索引 1 是阴影边框（黑 25%）"
        );
        assert_eq!(
            img.get_pixel(2, 0).0,
            [255, 0, 0, 255],
            "索引起 8 的普通色原样保留"
        );
        assert_eq!(img.get_pixel(3, 0).0, [0, 0, 0, 128], "索引 4 是阴影本体");
        assert_eq!(img.get_pixel(4, 0).0, [0, 0, 0, 0], "索引 5 是选择色，不画");
    }

    /// 非 PNG（如 webp/bmp）与真彩 PNG 不走索引分支，交给调用方常规解码。
    #[test]
    fn 非索引图不进索引分支() {
        assert!(decode_paletted_png(b"not a png").is_none());
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, 1, 1);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut writer = enc.write_header().expect("写 PNG 头");
            writer.write_image_data(&[1u8, 2, 3, 4]).expect("写像素");
            writer.finish().expect("收尾");
        }
        assert!(
            decode_paletted_png(&out).is_none(),
            "真彩 PNG 不该走索引语义"
        );
    }
}
