//! 图像锐化算子（**纯函数层**，无 tauri / VCMI 语义）。
//!
//! 目前只有 USM（unsharp mask）一个实现，服务于「高清素材放大」：
//! 传统插值（Lanczos3）放大后偏软，叠加 USM 能在**不引入生成伪影**的前提下拿回锐度
//! —— 这是 AI 超分不可用（组件缺失 / 倍率不符 / 类型不适用）时的主力补偿手段。
//!
//! 选型依据见 `modcache/smoke_tests/hd_compare.rs` 的对照实验：
//! - 幕布类大图：`Lanczos3 + USM 0.8` 的锐度接近 AI 版，且无生成纹理与色调漂移；
//! - 像素画**不要**用（硬边会产生光晕）——调用方按 `pixel_like` 判定跳过；
//! - 强度 0.8 / sigma 0.6 是在插画与写实两类素材上都验证过的档位。

use image::RgbaImage;

/// USM 的高斯 sigma（模糊半径）
pub const DEFAULT_SIGMA: f32 = 0.6;
/// USM 在插画/写实档的默认强度
pub const ILLUSTRATION_AMOUNT: f32 = 0.8;

/// 非锐化掩模：`out = src + amount × (src − blur(src))`。
///
/// - **只作用于 RGB**：alpha 原样保留（描边/图标的透明边缘不该被锐化改变覆盖度）；
/// - `amount <= 0` 或尺寸不合法 → 原样返回（调用方可无条件调用）；
/// - 钳位到 `[0,255]`，不做 gamma 校正（素材是 sRGB 直出，实测这样观感最自然）。
pub fn usm(rgba: &[u8], w: u32, h: u32, sigma: f32, amount: f32) -> Vec<u8> {
    if w == 0 || h == 0 || amount <= 0.0 {
        return rgba.to_vec();
    }
    let n = (w as usize) * (h as usize);
    if rgba.len() < n * 4 {
        return rgba.to_vec();
    }
    let Some(img) = RgbaImage::from_raw(w, h, rgba[..n * 4].to_vec()) else {
        return rgba.to_vec();
    };
    let blurred = image::imageops::blur(&img, sigma.max(0.1));
    let src = img.into_raw();
    let mut out = src.clone();
    for i in 0..n {
        for c in 0..3 {
            let s = src[i * 4 + c] as f32;
            let b = blurred.as_raw()[i * 4 + c] as f32;
            out[i * 4 + c] = (s + amount * (s - b)).clamp(0.0, 255.0).round() as u8;
        }
    }
    out
}

/// 逐通道线性混合：`out = a × alpha + b × (1 − alpha)`，**alpha 通道原样取 `a`**。
///
/// 用于 AI 混合（`launcher.json` 的 `hd_ai_mix`）：`a` = AI 结果、`b` = 传统结果。
/// 混合能稀释生成式幻觉与色偏，同时保住大部分 AI 的细节（见 `hd_compare` 的 `50/51` 列）。
/// 两者长度必须一致；不符时原样返回 `a`（调用方失败不该导致丢图）。
pub fn mix_rgb(a: &[u8], b: &[u8], alpha: f32) -> Vec<u8> {
    let ratio = alpha.clamp(0.0, 1.0);
    if a.is_empty() || a.len() != b.len() || ratio >= 1.0 {
        return a.to_vec();
    }
    let mut out = a.to_vec();
    for i in 0..a.len() / 4 {
        for c in 0..3 {
            let va = a[i * 4 + c] as f32;
            let vb = b[i * 4 + c] as f32;
            out[i * 4 + c] = (va * ratio + vb * (1.0 - ratio)).round() as u8;
        }
        out[i * 4 + 3] = a[i * 4 + 3];
    }
    out
}

/// 把 `src` 的逐通道均值/标准差对齐到 `reference`（**按 alpha 加权**；alpha 不动）。
///
/// 生成式模型有整体偏色与对比度漂移（6B 更明显：更饱和、偏暖），对齐后观感"回到原图"，
/// 且不触碰细节与锐度 ⇒ 对"还原度 + 清晰度"是双赢后处理。
///
/// 权重取 `min(a_src, a_ref)/255`：透明区不参与统计（其 RGB 是抠图残留，见 `dilate_colors`），
/// 否则描边 / 图标类的大片透明会把统计拉偏。增益限幅 `0.5..=2.0`，避免极端值放大噪声。
pub fn color_match(src: &[u8], reference: &[u8]) -> Vec<u8> {
    if src.len() != reference.len() || src.is_empty() {
        return src.to_vec();
    }
    let n = src.len() / 4;
    let mut out = src.to_vec();
    for c in 0..3 {
        let (mut sa, mut sb, mut sw) = (0.0f64, 0.0f64, 0.0f64);
        for i in 0..n {
            let w = (src[i * 4 + 3].min(reference[i * 4 + 3]) as f64) / 255.0;
            if w <= 0.01 {
                continue;
            }
            sa += src[i * 4 + c] as f64 * w;
            sb += reference[i * 4 + c] as f64 * w;
            sw += w;
        }
        if sw < 1.0 {
            continue;
        }
        let (ma, mb) = (sa / sw, sb / sw);
        let (mut va, mut vb) = (0.0f64, 0.0f64);
        for i in 0..n {
            let w = (src[i * 4 + 3].min(reference[i * 4 + 3]) as f64) / 255.0;
            if w <= 0.01 {
                continue;
            }
            va += w * (src[i * 4 + c] as f64 - ma).powi(2);
            vb += w * (reference[i * 4 + c] as f64 - mb).powi(2);
        }
        let (sda, sdb) = ((va / sw).sqrt(), (vb / sw).sqrt());
        if sda < 1e-6 {
            continue;
        }
        let gain = (sdb / sda).clamp(0.5, 2.0);
        for i in 0..n {
            if src[i * 4 + 3] == 0 {
                continue;
            }
            let v = (src[i * 4 + c] as f64 - ma) * gain + mb;
            out[i * 4 + c] = v.clamp(0.0, 255.0).round() as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_match_aligns_mean_and_keeps_alpha() {
        // src 整体偏亮偏红，reference 是中性灰 → 对齐后均值应相近
        let mut src = vec![0u8; 4 * 4];
        let mut refv = vec![0u8; 4 * 4];
        for i in 0..4 {
            // 红通道要有变化：源标准差为 0 时对齐按设计跳过（无法缩放常量）
            src[i * 4] = (180 + i * 10) as u8;
            src[i * 4 + 1] = 60;
            src[i * 4 + 2] = 60;
            src[i * 4 + 3] = (i * 60) as u8;
            refv[i * 4] = 120;
            refv[i * 4 + 1] = 120;
            refv[i * 4 + 2] = 120;
            refv[i * 4 + 3] = (i * 60) as u8;
        }
        let out = color_match(&src, &refv);
        for i in 0..4 {
            assert_eq!(out[i * 4 + 3], src[i * 4 + 3], "alpha 必须不动");
        }
        // 红通道的加权均值应向 120 靠拢（原 200）
        let mean: f64 = (0..4)
            .map(|i| out[i * 4] as f64 * (src[i * 4 + 3] as f64 / 255.0))
            .sum::<f64>()
            / (0..4).map(|i| src[i * 4 + 3] as f64 / 255.0).sum::<f64>();
        assert!((mean - 200.0).abs() > 30.0, "应对齐参考色的均值，实际 {mean}");
        assert_eq!(color_match(&src, &[]), src, "长度不符时原样返回");
    }

    #[test]
    fn mix_interpolates_rgb_and_keeps_alpha() {
        let a = vec![200u8, 0, 0, 255, 0, 200, 0, 128];
        let b = vec![0u8, 0, 200, 255, 200, 0, 0, 128];
        let m = mix_rgb(&a, &b, 0.5);
        assert_eq!(&m[0..3], &[100, 0, 100]);
        assert_eq!(m[3], 255, "alpha 必须取 a");
        assert_eq!(&m[4..7], &[100, 100, 0]);
        assert_eq!(m[7], 128);
        assert_eq!(mix_rgb(&a, &b, 1.0), a, "α=1 应为纯 AI 结果");
        assert_eq!(mix_rgb(&a, &[], 0.5), a, "长度不符时原样返回");
    }

    /// 造一张「左黑右白」的竖直边缘图（灰度），便于观察锐化方向
    fn edge_image(w: u32, h: u32) -> Vec<u8> {
        let mut v = vec![0u8; (w as usize) * (h as usize) * 4];
        for y in 0..h {
            for x in 0..w {
                let i = ((y * w + x) * 4) as usize;
                let val = if x < w / 2 { 80 } else { 180 };
                v[i] = val;
                v[i + 1] = val;
                v[i + 2] = val;
                v[i + 3] = 255;
            }
        }
        v
    }

    #[test]
    fn usm_preserves_alpha() {
        let (w, h) = (8u32, 4u32);
        let mut src = edge_image(w, h);
        // 给几个像素设不同 alpha，确认不被改动
        for x in 0..w {
            let i = (x * 4) as usize;
            src[i + 3] = (x * 30) as u8;
        }
        let out = usm(&src, w, h, DEFAULT_SIGMA, ILLUSTRATION_AMOUNT);
        for i in 0..(w as usize * h as usize) {
            assert_eq!(out[i * 4 + 3], src[i * 4 + 3], "alpha 必须原样保留（第 {i} 个像素）");
        }
    }

    #[test]
    fn usm_sharpens_edge_and_keeps_flat_area() {
        let (w, h) = (16u32, 8u32);
        let src = edge_image(w, h);
        let out = usm(&src, w, h, DEFAULT_SIGMA, ILLUSTRATION_AMOUNT);
        let lum = |v: &[u8], x: u32, y: u32| v[((y * w + x) * 4) as usize] as i32;

        // 远离边缘处（平坦区）基本不变：允许 ±2
        assert!((lum(&out, 1, 4) - lum(&src, 1, 4)).abs() <= 2);
        assert!((lum(&out, 14, 4) - lum(&src, 14, 4)).abs() <= 2);

        // 边缘两侧对比度必须增大（USM 的核心效果）
        let (dl, dr) = (lum(&src, 7, 4), lum(&src, 8, 4));
        let (ol, or) = (lum(&out, 7, 4), lum(&out, 8, 4));
        assert!(or - ol > dr - dl, "边缘对比度应被增强：原 {} → 新 {}", dr - dl, or - ol);
    }

    #[test]
    fn usm_is_noop_when_disabled_or_invalid() {
        let (w, h) = (4u32, 4u32);
        let src = edge_image(w, h);
        assert_eq!(usm(&src, w, h, DEFAULT_SIGMA, 0.0), src, "amount=0 应原样返回");
        assert_eq!(usm(&src, w, h, DEFAULT_SIGMA, -1.0), src);
        assert_eq!(usm(&[], w, h, DEFAULT_SIGMA, 1.0), Vec::<u8>::new());
    }
}
