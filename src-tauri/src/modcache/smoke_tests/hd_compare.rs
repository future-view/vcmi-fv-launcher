//! **放大算法受控对比实验**：对真实素材样本，把各候选方案并排落盘供肉眼 A/B。
//!
//! 回答三个实际观察问题：① AI 产物"变样"；② xBRZ 产物"外观一致但模糊"；
//! ③ 城镇背景这类大图（只需 `@2x`）该用哪种算法。
//!
//! ## 定位：**实验台** —— 每个样本都跑全部算法
//!
//! 所有列在**所有样本**上都会生成，与"生产实际用哪个"无关：生产分派（`hd.rs::pick_upscaler`
//! / `smooth_or_ai`）只影响 `05_production` 这一参考列的存在与否。尤其 AI：倍率 ≠ 4 的样本
//! 照样跑，以「4x → 降采样」呈现（`41_ai_down_to_rate.png`）—— 故**不存在"某列因倍率不匹配而缺失"**。
//!
//! ## 列（`target/hd-compare/<素材类>-<样本名>/`）
//!
//! | 列 | 含义 |
//! |---|---|
//! | `00_ref_nearest.png` | 原图 nearest 扩到目标尺寸 —— 零插值「原貌」基准 |
//! | `05_production.png` | **缓存里真实的生产产物**（参考列：含 webp 编码与 HD 真素材的影响） |
//! | `10/11/12_xbrz*` | xBRZ + USM 锐化（0.4 / 0.8） |
//! | `20/30/31_lanczos3*` | Lanczos3 + USM 锐化（0.4 / 0.8） |
//! | `40_ai.png` | Real-ESRGAN 4x 原始输出（倍率 = 4 的样本走这张） |
//! | `41_ai_down_to_rate.png` | AI 4x → 降采样到目标倍率（倍率 ≠ 4 的样本） |
//! | `50/51_ai_mix*` | AI 与传统结果混合（AI 权重 0.5 / 0.75） |
//! | `60_ai_colorfix.png` | AI + 逐通道色彩/对比度对齐 |
//! | `compare.png` | **各族代表 6 列**的拼接（原貌 / 现状 / xBRZ / Lanczos3 / AI / 混合） |
//! | `compare_all.png` | 全部列的拼接（超宽时跳过） |
//!
//! ⚠️ USM / 混合 / 色彩对齐都是**实验实现**，尚未进生产代码（生产只跑 xBRZ / Lanczos3 / AI）。
//!
//! ## 用法
//! ```bash
//! HD_BENCH_MODS="$PWD/../references/Heroes3-VCMI/.vcmi-fv-launcher/mods" \
//!   cargo test --lib hd_compare_controlled -- --ignored --nocapture
//! ```
//! - `HD_COMPARE="modId/images/sub/name.webp,..."`：指定样本（相对缓存根），否则自动挑；
//! - `HD_COMPARE_STRIP_MAX=19200`：拼接图总宽上限（默认 9600；全列拼接需调大）。

use super::sample_root;
use crate::modcache::hd::{pick_upscaler, target_rate, HdOptions};
use image::imageops::FilterType;
use image::RgbaImage;
use std::path::{Path, PathBuf};

/// 与前端一致的基准物理倍率（1080p：1920 / 1280）
const PHYS: f64 = 1.5;
/// 与前端一致的全局倍率上限
const CAP: u32 = 4;
/// USM 的高斯 sigma
const USM_SIGMA: f32 = 0.6;
/// 跑 AI 的源图面积上限 —— **只为防内存爆炸，与"倍率限制"无关**。
///
/// 512k 像素 ⇒ 4x 输出 8.2M 像素（RGBA 约 33 MB），在安全范围内；VCMI 最大的幕布
/// （800×374 = 299k）远低于此限。若真有样本超限，输出里会显式标注「AI 列缺失：超安全阈值」。
const AI_MAX_PIXELS: u32 = 512_000;

/// 横向拼接的总宽上限（`HD_COMPARE_STRIP_MAX` 可覆盖，默认 9600 px）。
///
/// 拼接宽 = 目标宽 × 列数：`compare.png`（各族代表 6 列）在幕布上 = 6 × 1600 = 9600，
/// 默认正好放得下；`compare_all.png`（12 列 = 19200）需手动调大上限才会生成。
fn strip_max_w() -> u32 {
    std::env::var("HD_COMPARE_STRIP_MAX")
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(9600)
}

// ── 图像辅助（实验用） ─────────────────────────────────────────────────────

/// USM（unsharp mask）：`out = src + amount × (src − blur(src))`，只作用于 RGB。
fn usm(rgba: &[u8], w: u32, h: u32, sigma: f32, amount: f32) -> Vec<u8> {
    let Some(img) = RgbaImage::from_raw(w, h, rgba.to_vec()) else {
        return rgba.to_vec();
    };
    let blurred = image::imageops::blur(&img, sigma);
    let mut out = img.clone();
    for (dst, (s, b)) in out.pixels_mut().zip(img.pixels().zip(blurred.pixels())) {
        for c in 0..3 {
            let v = s[c] as f32 + amount * (s[c] as f32 - b[c] as f32);
            dst[c] = v.clamp(0.0, 255.0) as u8;
        }
        dst[3] = s[3];
    }
    out.into_raw()
}

/// 去抖动预滤波：**只模糊 RGB**（alpha 用原图覆盖，避免透明边被晕成半透明光晕）。
///
/// 针对有序抖动（Bayer 棋盘）插画：直接插值会把抖动点平均成"泥浆"；先用小半径高斯模糊
/// 把 2×2 / 4×4 抖动单元合并成平滑渐变，再交给 Lanczos3 + USM 放大，可同时消除
/// "泥浆感"并保住大边缘。这是**非 AI** 手段里对抗抖动糊化的唯一有效杠杆。
fn dedither_prepass(rgba: &[u8], w: u32, h: u32, sigma: f32) -> Vec<u8> {
    let Some(img) = RgbaImage::from_raw(w, h, rgba.to_vec()) else {
        return rgba.to_vec();
    };
    let mut blurred = image::imageops::blur(&img, sigma);
    let Some(orig) = RgbaImage::from_raw(w, h, rgba.to_vec()) else {
        return blurred.into_raw();
    };
    for (b, o) in blurred.pixels_mut().zip(orig.pixels()) {
        b[3] = o[3];
    }
    blurred.into_raw()
}

/// 逐通道线性混合：`out = a × alpha + b × (1 − alpha)`
fn blend(a: &[u8], b: &[u8], alpha: f32) -> Vec<u8> {
    a.iter()
        .zip(b.iter())
        .map(|(x, y)| (*x as f32 * alpha + *y as f32 * (1.0 - alpha)).clamp(0.0, 255.0) as u8)
        .collect()
}

/// 把 `src` 的逐通道均值/标准差对齐到 `reference`（只统计 RGB，alpha 不动）。
///
/// 目的是消除生成式模型常见的整体偏色/对比度漂移，让观感"回到原图"。
fn color_match(src: &[u8], reference: &[u8]) -> Vec<u8> {
    if src.len() != reference.len() {
        return src.to_vec();
    }
    let mut out = src.to_vec();
    let n = src.len() / 4;
    if n == 0 {
        return out;
    }
    for c in 0..3 {
        // ⚠️ 统计必须按 alpha 加权：描边 / 图标类大面积透明，未加权时对齐目标会被透明区
        //（抠图残留色 / AI 在透明区编造的颜色）污染，反而把可见区拉偏（实测色偏 14+）
        let (mut ms, mut mr, mut vs, mut vr, mut sw) = (0.0f64, 0.0f64, 0.0f64, 0.0f64, 0.0f64);
        for i in 0..n {
            let w = (src[i * 4 + 3].min(reference[i * 4 + 3]) as f64) / 255.0;
            if w <= 0.01 {
                continue;
            }
            let s = src[i * 4 + c] as f64;
            let r = reference[i * 4 + c] as f64;
            ms += s * w;
            mr += r * w;
            vs += s * s * w;
            vr += r * r * w;
            sw += w;
        }
        if sw <= 0.0 {
            continue;
        }
        ms /= sw;
        mr /= sw;
        let ss = ((vs / sw) - ms * ms).max(1e-6).sqrt();
        let sr = ((vr / sw) - mr * mr).max(1e-6).sqrt();
        let k = (sr / ss).clamp(0.5, 2.0);
        for i in 0..n {
            let s = src[i * 4 + c] as f64;
            out[i * 4 + c] = ((s - ms) * k + mr).clamp(0.0, 255.0).round() as u8;
        }
    }
    out
}

/// 整图重采样（nearest 参考 / AI 降采样都用它，只是 filter 不同）
fn resample(rgba: &[u8], w: u32, h: u32, tw: u32, th: u32, f: FilterType) -> Option<Vec<u8>> {
    let img = RgbaImage::from_raw(w, h, rgba.to_vec())?;
    Some(image::imageops::resize(&img, tw, th, f).into_raw())
}

/// 列标签 —— **只说明"这列是哪个算法（或哪类参考）"，不表达"生产用不用它"**。
///
/// 本用例是实验台：每一列都在**所有样本**上生成（AI 在倍率 ≠ 4 时以「4x → 降采样」呈现），
/// 不存在"某列因倍率不匹配而缺失"；生产实际只用其中一族，仅供参考（见 README 的"生产判定"行）。
fn column_scope(name: &str, algo_id: &str) -> &'static str {
    match name {
        "00_ref_nearest.png" => "[参考] 原貌基准",
        "05_production.png" => "[现状] 缓存产物",
        n if n.starts_with("10_") || n.starts_with("11_") || n.starts_with("12_") => {
            if algo_id == "xbrz" {
                "[xBRZ] · 本样本生产档"
            } else {
                "[xBRZ]"
            }
        }
        n if n.starts_with("20_") || n.starts_with("30_") || n.starts_with("31_") => {
            if algo_id == "lanczos3" {
                "[Lanczos3] · 本样本生产档"
            } else {
                "[Lanczos3]"
            }
        }
        n if n.starts_with("22_") || n.starts_with("32_") => {
            if algo_id == "lanczos3" {
                "[Lanczos3+去抖] · 本样本生产档"
            } else {
                "[Lanczos3+去抖]"
            }
        }
        n if n.starts_with("15_") || n.starts_with("16_") => "[重采样] CatmullRom/Triangle 对照",
        n if n.starts_with("40_")
            || n.starts_with("41_")
            || n.starts_with("50_")
            || n.starts_with("51_")
            || n.starts_with("60_") =>
        {
            if algo_id == "ai" {
                "[AI] · 本样本生产档"
            } else {
                "[AI]"
            }
        }
        _ => "[?]",
    }
}

/// 横向拼接（高对齐到最大值，顶部对齐）
fn hstack(items: &[(Vec<u8>, u32, u32)]) -> Option<(Vec<u8>, u32, u32)> {
    if items.is_empty() {
        return None;
    }
    let h = items.iter().map(|(_, _, h)| *h).max()?;
    let w: u32 = items.iter().map(|(_, w, _)| *w).sum();
    let mut out = vec![0u8; (w as usize) * (h as usize) * 4];
    let mut x0 = 0u32;
    for (data, iw, ih) in items {
        for y in 0..*ih {
            for x in 0..*iw {
                let si = ((y * *iw + x) * 4) as usize;
                let di = ((y * w + x0 + x) * 4) as usize;
                out[di..di + 4].copy_from_slice(&data[si..si + 4]);
            }
        }
        x0 += *iw;
    }
    Some((out, w, h))
}

// ── 客观评估（数值参考，不能替代肉眼） ────────────────────────────────────

#[derive(Debug, Clone, Copy, Default)]
struct Metrics {
    /// 忠实度：结果降采样回原尺寸后与原图比 PSNR（dB，越高越"不引入偏差"）
    psnr: f64,
    /// 锐度：平均相邻亮度差（越高越锐；噪声也会拉高，仅作相对比较）
    sharp: f64,
    /// 色偏：与 `00_ref_nearest` 的逐通道均值绝对偏移（0-255，越小越"没跑色"）
    color: f64,
    /// 结构保持：与 `00_ref_nearest` 的梯度图 Pearson 相关（1.0 = 结构完全一致）
    corr: f64,
    /// 平坦度：3×3 邻域亮度极差 < 6/255 的可见像素占比 —— 量化"糊成色块"
    flat: f64,
}

fn luma(px: &[u8]) -> f64 {
    0.299 * px[0] as f64 + 0.587 * px[1] as f64 + 0.114 * px[2] as f64
}

/// 放大结果 → 降采样回原尺寸 → 与原图比 PSNR（只算 RGB，**按 alpha 加权**）
///
/// ⚠️ 必须加权：描边 / 图标类素材大面积透明，透明处的 RGB 是抠图残留（AI 还会在其上"编造"颜色），
/// 不加权会得出"色偏 105、PSNR 6.6"这种污染全局平均的假数据。
fn psnr_roundtrip(up: &[u8], uw: u32, uh: u32, src: &[u8], w: u32, h: u32) -> f64 {
    let Some(back) = resample(up, uw, uh, w, h, FilterType::Lanczos3) else {
        return f64::NAN;
    };
    let n = (w as usize) * (h as usize);
    let mut se = 0.0f64;
    let mut wt = 0.0f64;
    for i in 0..n {
        let alpha_w = (back[i * 4 + 3].min(src[i * 4 + 3]) as f64) / 255.0;
        if alpha_w <= 0.01 {
            continue;
        }
        for c in 0..3 {
            let d = back[i * 4 + c] as f64 - src[i * 4 + c] as f64;
            se += d * d * alpha_w;
            wt += alpha_w;
        }
    }
    let mse = se / wt.max(1.0);
    if mse <= 1e-9 {
        99.0
    } else {
        10.0 * (255.0f64 * 255.0 / mse).log10()
    }
}

/// 与参考图的逐通道均值绝对偏移（**按 alpha 加权**，理由同上）
fn color_shift(a: &[u8], b: &[u8]) -> f64 {
    let n = (a.len() / 4).max(1);
    let mut acc = 0.0;
    for c in 0..3 {
        let (mut sa, mut sb, mut sw) = (0.0f64, 0.0f64, 0.0f64);
        for i in 0..n {
            let w = (a[i * 4 + 3].min(b[i * 4 + 3]) as f64) / 255.0;
            if w <= 0.01 {
                continue;
            }
            sa += a[i * 4 + c] as f64 * w;
            sb += b[i * 4 + c] as f64 * w;
            sw += w;
        }
        acc += (sa - sb).abs() / sw.max(1.0);
    }
    acc / 3.0
}

/// 梯度图（4 邻域亮度差均值）
fn grad_map(rgba: &[u8], w: u32, h: u32) -> Vec<f64> {
    let (wu, hu) = (w as usize, h as usize);
    let mut out = vec![0.0f64; wu * hu];
    for y in 0..hu {
        for x in 0..wu {
            let i = (y * wu + x) * 4;
            if i + 3 >= rgba.len() {
                continue;
            }
            let l0 = luma(&rgba[i..i + 4]);
            let mut g = 0.0;
            if x + 1 < wu && i + 7 < rgba.len() {
                g += (luma(&rgba[i + 4..i + 8]) - l0).abs();
            }
            if y + 1 < hu {
                let j = i + wu * 4;
                if j + 3 < rgba.len() {
                    g += (luma(&rgba[j..j + 4]) - l0).abs();
                }
            }
            out[y * wu + x] = g * 0.5;
        }
    }
    out
}

fn pearson(a: &[f64], b: &[f64]) -> f64 {
    let n = a.len().min(b.len());
    if n == 0 {
        return 0.0;
    }
    let (mut ma, mut mb) = (0.0f64, 0.0f64);
    for i in 0..n {
        ma += a[i];
        mb += b[i];
    }
    ma /= n as f64;
    mb /= n as f64;
    let (mut cov, mut va, mut vb) = (0.0f64, 0.0f64, 0.0f64);
    for i in 0..n {
        let da = a[i] - ma;
        let db = b[i] - mb;
        cov += da * db;
        va += da * da;
        vb += db * db;
    }
    if va <= 1e-12 || vb <= 1e-12 {
        return 0.0;
    }
    cov / (va.sqrt() * vb.sqrt())
}

/// 局部平坦度（量化"糊成色块"）：3×3 邻域内亮度极差 < 6/255 的**可见**像素占比。
///
/// 越高 = 越多区域被糊成同一块颜色（用户说的"糊成色块"）。只在 alpha 可见像素上统计，
/// 避免透明区干扰。对平滑类算法（Triangle/CatmullRom/Lanczos3）越低越好；
/// 但 nearest 也会偏高（块内平坦），需与"锐度"合看 —— 锐度高且平坦度低才是真保留细节。
fn flat_blob_ratio(rgba: &[u8], w: u32, h: u32) -> f64 {
    if w < 3 || h < 3 {
        return 0.0;
    }
    let lum = |x: u32, y: u32| -> f64 {
        let i = ((y * w + x) * 4) as usize;
        luma(&rgba[i..i + 4])
    };
    let mut flat = 0.0f64;
    let mut vis = 0.0f64;
    for y in 1..h - 1 {
        for x in 1..w - 1 {
            let c = ((y * w + x) * 4) as usize;
            if rgba[c + 3] < 8 {
                continue; // 近乎透明不算
            }
            let mut mn = f64::MAX;
            let mut mx = f64::MIN;
            for dy in 0..3u32 {
                for dx in 0..3u32 {
                    let v = lum(x - 1 + dx, y - 1 + dy);
                    mn = mn.min(v);
                    mx = mx.max(v);
                }
            }
            vis += 1.0;
            if (mx - mn) < 6.0 {
                flat += 1.0;
            }
        }
    }
    if vis <= 0.0 {
        0.0
    } else {
        flat / vis
    }
}

fn save_png(dir: &Path, name: &str, rgba: &[u8], w: u32, h: u32) {
    if let Some(img) = RgbaImage::from_raw(w, h, rgba.to_vec()) {
        let _ = img.save(dir.join(name));
    }
}

fn load(p: &Path) -> Option<RgbaImage> {
    let bytes = std::fs::read(p).ok()?;
    image::load_from_memory(&bytes).ok().map(|i| i.to_rgba8())
}

/// 生产变体产物路径：`<同目录>/<stem>@<rate>x.webp`（命名见 `hd.rs::upscale_file`）。
///
/// 这是**唯一的"生产现状"来源**：它可能来自 HD 真素材（`hd_edition::lookup_for`）、
/// 经 webp 编码、且由当时那次构建的参数决定 —— 与本用例现场重算的结果未必一致。
fn production_variant(src: &Path, rate: u32) -> Option<PathBuf> {
    if rate < 2 {
        return None;
    }
    let stem = src.file_stem().and_then(|s| s.to_str())?;
    let p = src.with_file_name(format!("{stem}@{rate}x.webp"));
    p.is_file().then_some(p)
}

/// 同目录下该源图**实际存在**的变体倍率（升序）—— 诊断"缓存是否用别的参数构建"。
fn existing_variant_rates(src: &Path) -> Vec<u32> {
    let (Some(dir), Some(stem)) = (src.parent(), src.file_stem().and_then(|s| s.to_str())) else {
        return Vec::new();
    };
    let Ok(ents) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<u32> = Vec::new();
    for e in ents.flatten() {
        let n = e.file_name();
        let Some(n) = n.to_str() else { continue };
        // 只认 `<stem>@<纯数字>x.webp`（`@` 后必须紧接数字，避免误吞 `stem_other.webp`）
        if let Some(rest) = n.strip_prefix(stem).and_then(|r| r.strip_prefix('@')) {
            if let Some(num) = rest.strip_suffix("x.webp") {
                if let Ok(v) = num.parse::<u32>() {
                    out.push(v);
                }
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

/// 候选 1x 图（不含 `@Nx`）
fn collect_1x(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(ents) = std::fs::read_dir(dir) else {
        return;
    };
    for e in ents.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_1x(&p, out);
        } else if p
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|n| n.ends_with(".webp") && !n.contains('@'))
        {
            out.push(p);
        }
    }
}

struct Pick {
    label: String,
    sub: String,
    path: PathBuf,
}

/// 挑选样本：**覆盖全部素材类**，每类按"生产判定"各取一张代表。
///
/// 同一类可能同时出现多种判定（`town`：大幕布判插画 → Lanczos3；小兵种背景判像素画 → xBRZ），
/// 故每类最多取 3 张（对应 ai / xbrz / lanczos3 三种结果）。
fn all_picks(root: &Path, opt: &HdOptions) -> Vec<Pick> {
    // 只扫原版 core 系（素材齐全、最能代表问题），避免遍历 674 个 mod
    let mods = [
        "core",
        "core.castle",
        "core.rampart",
        "core.tower",
        "core.inferno",
        "core.necropolis",
        "core.dungeon",
        "core.stronghold",
        "core.fortress",
        "core.conflux",
    ];
    let mut by_sub: std::collections::BTreeMap<String, Vec<PathBuf>> = Default::default();
    for m in mods {
        let img = root.join(m).join("images");
        if !img.is_dir() {
            continue;
        }
        let mut files = Vec::new();
        collect_1x(&img, &mut files);
        files.sort();
        for f in files {
            let sub = f
                .parent()
                .and_then(|p| p.file_name())
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            if sub.is_empty() {
                continue;
            }
            let e = by_sub.entry(sub).or_default();
            if e.len() < 80 {
                e.push(f);
            }
        }
    }

    // 每类：判定结果 → 代表样本（各一张）
    let mut chosen: std::collections::BTreeMap<
        String,
        std::collections::BTreeMap<&'static str, Pick>,
    > = Default::default();
    let mut scanned = 0usize;
    for (sub, cands) in &by_sub {
        for f in cands {
            let e = chosen.entry(sub.clone()).or_default();
            if e.len() >= 3 || scanned >= 4000 {
                break;
            }
            let Some(img) = load(f) else { continue };
            scanned += 1;
            let (w, h) = (img.width(), img.height());
            let rate = target_rate(w, h, sub, PHYS, CAP);
            let (algo, _) = pick_upscaler(img.as_raw(), w, h, opt, rate, sub);
            e.entry(algo.id()).or_insert_with(|| Pick {
                label: f
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "sample".into()),
                sub: sub.clone(),
                path: f.clone(),
            });
        }
    }

    // 展平并排序（先类名、再判定名）→ 输出稳定、便于对照
    let mut picks: Vec<Pick> = Vec::new();
    for (_, m) in chosen {
        let mut v: Vec<(&str, Pick)> = m.into_iter().collect();
        v.sort_by_key(|(k, _)| *k);
        picks.extend(v.into_iter().map(|(_, p)| p));
    }
    picks
}

#[test]
#[ignore]
fn hd_compare_controlled() {
    let root = sample_root();
    if !root.is_dir() {
        println!(
            "⚠️ 缓存目录不存在：{}（用 HD_BENCH_MODS 指定）",
            root.display()
        );
        return;
    }
    let out_root = PathBuf::from("../target/hd-compare");
    let _ = std::fs::create_dir_all(&out_root);

    // AI 组件目录（源码位置；与打包映射一致）
    let rt = PathBuf::from("resources/runtime/ai");
    let ai_ready =
        rt.join("onnxruntime.dll").is_file() && rt.join("realesr-animevideo-v3-x4.onnx").is_file();
    if ai_ready {
        crate::infra::ai::configure_runtime(Some(rt));
    } else {
        println!("⚠️ 组件目录不完整，AI 方案将全部缺席：{}", rt.display());
    }

    let Some(opt) = HdOptions::new(Some(CAP), Some(PHYS), Some("xbrz".to_string()), Some(true))
    else {
        println!("⚠️ HdOptions 构造失败");
        return;
    };

    let env = std::env::var("HD_COMPARE").unwrap_or_default();
    let picks = if env.trim().is_empty() {
        all_picks(&root, &opt)
    } else {
        env.split(',')
            .filter(|s| !s.trim().is_empty())
            .map(|rel| {
                let path = root.join(rel.trim().replace('/', std::path::MAIN_SEPARATOR_STR));
                let sub = path
                    .parent()
                    .and_then(|p| p.file_name())
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "?".into());
                let label = path
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "sample".into());
                Pick { label, sub, path }
            })
            .collect()
    };
    if picks.is_empty() {
        println!("没有可用样本（检查缓存或 HD_COMPARE）");
        return;
    }

    let strip_max = strip_max_w();
    println!("=== 受控对比（目标倍率按 PHYS={PHYS}、CAP={CAP} 逐图计算）===");
    println!(
        "实验台口径：**所有算法在所有样本上都生成**（AI 在倍率 ≠ 4 时以「4x → 降采样」呈现）。\n\
         05_production = 缓存里的现状产物（参考列）；compare.png = 各族代表 6 列；\
         compare_all.png = 全部列；拼接总宽上限 {strip_max}px（HD_COMPARE_STRIP_MAX 可调）。\n"
    );
    let mut index_lines: Vec<String> = Vec::new();
    let mut agg: std::collections::BTreeMap<String, Vec<Metrics>> = Default::default();
    for p in &picks {
        let Some(src) = load(&p.path) else {
            println!("⚠️ 解码失败：{}", p.path.display());
            continue;
        };
        let (w, h) = (src.width(), src.height());
        let rate = target_rate(w, h, &p.sub, PHYS, CAP);
        let (algo, is_pixel) = pick_upscaler(src.as_raw(), w, h, &opt, rate, &p.sub);
        let (tw, th) = (w * rate, h * rate);
        let folder = out_root.join(format!("{}-{}", p.sub, p.label));
        let _ = std::fs::create_dir_all(&folder);
        // AI 在生产里是否可达：`hd.rs::smooth_or_ai` 只在**插画档**被调用 ——
        // 故需同时满足：开关打开 + 倍率恰为 4 + 本图判定为插画（像素画即使 4x 也走 xBRZ）
        // AI 现已支持任意倍率（4x → 重采样到目标），故只看"生产是否真的选了 AI"
        let ai_reachable = opt.ai && algo.id() == "ai";
        let variant_rates = existing_variant_rates(&p.path);
        println!(
            "▶ {}/{label} ｜ {w}×{h} → {tw}×{th} @{rate}x ｜ 生产判定：{}（{}）｜ 去重色占比 {}%",
            p.sub,
            algo.id(),
            if is_pixel { "像素画" } else { "插画" },
            crate::modcache::hd::rich_texture_pct(src.as_raw(), w, h),
            label = p.label
        );
        println!(
            "    生产路径：@{rate}x → {}（生产是否用 AI：{}）　｜ {}",
            algo.id(),
            if ai_reachable {
                "用"
            } else if !opt.ai {
                "开关关闭"
            } else if rate != 4 {
                "不用（倍率 ≠ 4）"
            } else if algo.id() == "xbrz" {
                "不用（像素画走 xBRZ）"
            } else if crate::infra::ai::is_available() {
                "不用（本用途排除 AI，见 hd::ai_excluded）"
            } else {
                "不用（AI 运行库不可用）"
            },
            if variant_rates.is_empty() {
                "缓存：无该源图的变体".to_string()
            } else {
                format!("缓存变体倍率 {variant_rates:?}")
            }
        );

        // 收集"与目标尺寸同尺寸"的方案，供拼接
        let mut items: Vec<(String, Vec<u8>)> = Vec::new();

        // 参考：nearest（零插值）
        if let Some(v) = resample(src.as_raw(), w, h, tw, th, FilterType::Nearest) {
            save_png(&folder, "00_ref_nearest.png", &v, tw, th);
            items.push(("00_ref_nearest.png".into(), v));
        }

        // ★ 生产现状基准：缓存里真实落盘的 `<stem>@<rate>x.webp`（含 webp 编码与 HD 真素材的影响）
        let prod_path = production_variant(&p.path, rate);
        let mut prod_in_items = false;
        let prod_note = match prod_path.as_ref().and_then(|pp| load(pp).map(|i| (pp, i))) {
            Some((_, img)) => {
                let (pw, ph) = (img.width(), img.height());
                save_png(&folder, "05_production.png", img.as_raw(), pw, ph);
                if (pw, ph) == (tw, th) {
                    items.push(("05_production.png".into(), img.into_raw()));
                    prod_in_items = true;
                    format!("✓ {pw}×{ph}（与目标一致，含 webp 编码）")
                } else {
                    // 尺寸与本次算出的目标不一致 → 不参与拼接（会错位），仅落盘供单独查看
                    format!("⚠️ {pw}×{ph} ≠ 目标 {tw}×{th}（缓存由其它参数构建？不参与拼接）")
                }
            }
            None => format!("✗ 缓存无 @{rate}x 产物（未构建，或该倍率未被生产生成）"),
        };
        println!("    生产产物：{prod_note}");

        // 实验台：所有算法族**总是全跑** —— 每类素材都要横向对比全部算法的效果，
        // 不受"本样本生产用哪个"影响（生产分派只体现在 05_production 这一参考列上）。
        let (want_xbrz, want_lanczos, want_ai) = (true, true, true);

        // 传统算法 A：xBRZ（像素画档）+ 其 USM 两档
        let xbrz_out = if want_xbrz {
            crate::infra::upscale::upscaler_by_id("xbrz")
                .and_then(|u| crate::infra::upscale::upscale_with(u, src.as_raw(), w, h, rate))
        } else {
            None
        };
        if let Some((base, bw, bh)) = &xbrz_out {
            save_png(&folder, "10_xbrz.png", base, *bw, *bh);
            if (*bw, *bh) == (tw, th) {
                items.push(("10_xbrz.png".into(), base.clone()));
            }
            for (tag, amount) in [("11_xbrz_usm040.png", 0.4f32), ("12_xbrz_usm080.png", 0.8)] {
                let v = usm(base, tw, th, USM_SIGMA, amount);
                save_png(&folder, tag, &v, tw, th);
                items.push((tag.into(), v));
            }
        }

        // 传统算法 B：Lanczos3（插画档回退）+ 其 USM 两档。
        // 插画档带**生产同款的放大前预处理**（透明区填色）以消除插值彩边 ——
        // 否则这一列会比生产产物多一圈青边，失去对照意义（见 `hd::preprocess_for_interpolation`）。
        let lanc_src =
            crate::modcache::hd::preprocess_for_interpolation(src.as_raw(), w, h, !is_pixel);
        let lanczos = if want_lanczos {
            crate::infra::upscale::upscaler_by_id("lanczos3")
                .and_then(|u| crate::modcache::hd::upscale_lowpass(u, &lanc_src, w, h, rate))
        } else {
            None
        };
        if let Some((base, bw, bh)) = &lanczos {
            save_png(&folder, "20_lanczos3.png", base, *bw, *bh);
            if (*bw, *bh) == (tw, th) {
                items.push(("20_lanczos3.png".into(), base.clone()));
            }
            for (tag, amount) in [
                ("30_lanczos3_usm040.png", 0.4f32),
                ("31_lanczos3_usm080.png", 0.8),
            ] {
                let v = usm(base, tw, th, USM_SIGMA, amount);
                save_png(&folder, tag, &v, tw, th);
                items.push((tag.into(), v));
            }
        }

        // 传统算法 B'：Lanczos3 + **去抖动预滤波**（针对有序抖动插画）。
        // 抖动插画直接 Lanczos3 会把 2×2/4×4 抖动点平均成"泥浆"；先用小半径高斯模糊
        // 把抖动单元合并成平滑渐变（仅模糊 RGB、保留 alpha 防透明边晕开），再 Lanczos3 + USM，
        // 既消泥浆感又保留大边缘。这是"传统、非 AI"范围内对抗抖动糊化的唯一有效杠杆。
        let dd_sigma = 0.8f32;
        let lanc_src_dd = crate::modcache::hd::preprocess_for_interpolation(
            &dedither_prepass(src.as_raw(), w, h, dd_sigma),
            w,
            h,
            !is_pixel,
        );
        let lanczos_dd = if want_lanczos {
            crate::infra::upscale::upscaler_by_id("lanczos3")
                .and_then(|u| crate::modcache::hd::upscale_lowpass(u, &lanc_src_dd, w, h, rate))
        } else {
            None
        };
        if let Some((base, bw, bh)) = &lanczos_dd {
            save_png(&folder, "22_lanczos3_dedither.png", base, *bw, *bh);
            if (*bw, *bh) == (tw, th) {
                items.push(("22_lanczos3_dedither.png".into(), base.clone()));
            }
            let v = usm(base, tw, th, USM_SIGMA, 0.8);
            save_png(&folder, "32_lanczos3_dedither_usm080.png", &v, tw, th);
            items.push(("32_lanczos3_dedither_usm080.png".into(), v));
        }

        // 传统算法 C/D：Triangle / CatmullRom（image crate 重采样，作为"比 Lanczos3 锐一点"的对照）。
        // 与 Lanczos3 共用生产同款预处理源（lanc_src），保证差异只来自滤波核本身。
        for (tag, id) in [
            ("16_triangle.png", "triangle"),
            ("15_catmullrom.png", "catmullrom"),
        ] {
            if let Some(u) = crate::infra::upscale::upscaler_by_id(id) {
                if let Some((v, vw, vh)) =
                    crate::modcache::hd::upscale_lowpass(u, &lanc_src, w, h, rate)
                {
                    save_png(&folder, tag, &v, vw, vh);
                    if (vw, vh) == (tw, th) {
                        items.push((tag.into(), v));
                    }
                }
            }
        }

        // —— 传统算法族总览：原貌 | Triangle | CatmullRom | Lanczos3 | xBRZ ——
        // 回答"哪个传统算法不糊成色块"：一图横比全部传统放大器，供开发者肉眼定案。
        let mut trad: Vec<(Vec<u8>, u32, u32)> = Vec::new();
        for key in [
            "00_ref_nearest.png",
            "16_triangle.png",
            "15_catmullrom.png",
            "20_lanczos3.png",
            "10_xbrz.png",
        ] {
            if let Some((_, v)) = items.iter().find(|(n, _)| n == key) {
                trad.push((v.clone(), tw, th));
            }
        }
        if trad.len() == 5 {
            if let Some((joined, jw, jh)) = hstack(&trad) {
                save_png(&folder, "trad_compare.png", &joined, jw, jh);
            }
        }

        // AI（仅 4x；超面积跳过）
        let mut ai: Option<(Vec<u8>, u32, u32)> = None;
        if ai_ready && want_ai && w * h <= AI_MAX_PIXELS {
            let t0 = std::time::Instant::now();
            ai = crate::infra::ai::upscale(src.as_raw(), w, h, 4);
            match &ai {
                Some((_, aw, ah)) => {
                    println!("    AI 4x → {aw}×{ah}  用时 {}ms", t0.elapsed().as_millis())
                }
                None => println!("    AI 返回 None（运行库/推理失败，详见日志）"),
            }
        } else if ai_ready && want_ai {
            println!(
                "    跳过 AI：源图 {} px 超过安全阈值（整图推理无 tiling）",
                w * h
            );
        }

        if let Some((ai_raw, aw, ah)) = ai {
            // —— "预放大 → AI 4x → 降采样回目标"两阶段（用户提案）——
            // 与 `41`（原图直入 AI）的差异不在降采样比（同为 4/rate），而在**输入分布**：
            // 预放大图的像素已被插值平滑，模型面对的是训练分布外的输入。用对照定案。
            for (tag, up_id) in [
                ("70_preup_lanczos_ai.png", "lanczos3"),
                ("71_preup_xbrz_ai.png", "xbrz"),
            ] {
                let Some(u) = crate::infra::upscale::upscaler_by_id(up_id) else {
                    continue;
                };
                let Some((pre, pw, ph)) =
                    crate::infra::upscale::upscale_with(u, src.as_raw(), w, h, rate)
                else {
                    continue;
                };
                if (pw as u64) * (ph as u64) > AI_MAX_PIXELS as u64 {
                    println!("    （{tag} 跳过：预放大后 {pw}×{ph} 超 AI 安全阈值）");
                    continue;
                }
                let Some((a4, aw4, ah4)) = crate::infra::ai::upscale(&pre, pw, ph, 4) else {
                    continue;
                };
                let Some(v) = resample(&a4, aw4, ah4, tw, th, FilterType::Lanczos3) else {
                    continue;
                };
                save_png(&folder, tag, &v, tw, th);
                items.push((tag.into(), v));
            }

            // —— 多模型对比：组件目录里每个 `*.onnx` 各跑一列（`80_ai_<模型名>.png`）——
            // 开发者把新模型（如 `realesrgan-x4plus-anime6b.onnx`）丢进
            // `resources/runtime/ai/`，重跑本用例即可横向核对各模型的细节 / 变形 / 色偏。
            // 走 `upscale_with_model`：**不经 sha256 白名单**（实验台专用入口）。
            let rt_dir = PathBuf::from("resources/runtime/ai");
            if let Ok(ents) = std::fs::read_dir(&rt_dir) {
                let mut models: Vec<PathBuf> = ents
                    .flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        p.extension()
                            .map(|x| x.eq_ignore_ascii_case("onnx"))
                            .unwrap_or(false)
                    })
                    .collect();
                models.sort();
                for m in models {
                    if (w as u64) * (h as u64) > AI_MAX_PIXELS as u64 {
                        continue;
                    }
                    let stem = m.file_stem().and_then(|s| s.to_str()).unwrap_or("model");
                    let tag = format!("80_ai_{stem}.png");
                    let Some((v, vw, vh)) =
                        crate::infra::ai::upscale_with_model(src.as_raw(), w, h, 4, &m)
                    else {
                        continue;
                    };
                    let v = if (vw, vh) == (tw, th) {
                        v
                    } else {
                        match resample(&v, vw, vh, tw, th, FilterType::Lanczos3) {
                            Some(x) => x,
                            None => continue,
                        }
                    };
                    save_png(&folder, &tag, &v, tw, th);
                    // `81_` = 同模型的**色彩对齐变体**（对齐到 Lanczos3 结果，与生产链路同款）：
                    // 用于验收"偏色是否被消除、细节是否保住"，是 6B 调参的主要观测列。
                    let cf_v = lanczos.as_ref().and_then(|(b, _, _)| {
                        (b.len() == v.len()).then(|| crate::infra::sharpen::color_match(&v, b))
                    });
                    items.push((tag, v));
                    if let Some(cf) = cf_v {
                        let cf_tag = format!("81_ai_{stem}_colormatch.png");
                        save_png(&folder, &cf_tag, &cf, tw, th);
                        items.push((cf_tag, cf));
                    }
                }
            }
            let ai_at_target = if rate == 4 {
                save_png(&folder, "40_ai.png", &ai_raw, aw, ah);
                Some(ai_raw.clone())
            } else {
                // 4x 原始输出也落盘（不入拼接：尺寸与目标不同），便于对照"降采样损失了什么"
                save_png(&folder, "40_ai_4x_raw.png", &ai_raw, aw, ah);
                resample(&ai_raw, aw, ah, tw, th, FilterType::Lanczos3)
            };
            if let Some(ai_t) = ai_at_target {
                if rate != 4 {
                    save_png(&folder, "41_ai_down_to_rate.png", &ai_t, tw, th);
                }
                items.push((
                    if rate == 4 {
                        "40_ai.png"
                    } else {
                        "41_ai_down_to_rate.png"
                    }
                    .into(),
                    ai_t.clone(),
                ));
                if let Some((base, _, _)) = &lanczos {
                    if base.len() == ai_t.len() {
                        for (tag, alpha) in
                            [("50_ai_mix050.png", 0.5f32), ("51_ai_mix075.png", 0.75)]
                        {
                            let v = blend(&ai_t, base, alpha);
                            save_png(&folder, tag, &v, tw, th);
                            items.push((tag.into(), v));
                        }
                        let v = color_match(&ai_t, base);
                        save_png(&folder, "60_ai_colorfix.png", &v, tw, th);
                        items.push(("60_ai_colorfix.png".into(), v));
                    }
                }
            }
        }

        // ── 客观评估（数值参考，不能替代肉眼） ────────────────────────────
        if let Some((_, refv)) = items.first() {
            let ref_grad = grad_map(refv, tw, th);
            println!("    ── 数值评估（基准 = 00_ref_nearest）──");
            for (name, data) in &items {
                if data.len() != refv.len() {
                    continue;
                }
                let m = Metrics {
                    psnr: psnr_roundtrip(data, tw, th, src.as_raw(), w, h),
                    sharp: crate::modcache::hd::grad_energy(data, tw, th),
                    color: color_shift(data, refv),
                    corr: pearson(&grad_map(data, tw, th), &ref_grad),
                    flat: flat_blob_ratio(data, tw, th),
                };
                println!(
                    "      {:<26} PSNR {:>6.2}  锐度 {:>7.3}  色偏 {:>6.3}  平坦 {:>6.3}  结构相关 {:>7.4}",
                    name.trim_end_matches(".png"),
                    m.psnr,
                    m.sharp,
                    m.color,
                    m.flat,
                    m.corr
                );
                agg.entry(name.trim_end_matches(".png").to_string())
                    .or_default()
                    .push(m);
            }
        }

        // 目录内说明文件（打开目录即可看懂每列是什么、哪张代表现状，不必回头查日志）
        let production = match algo.id() {
            "ai" if rate == 4 => "40_ai.png（Real-ESRGAN 4x）",
            "ai" => "41_ai_down_to_rate.png（AI 4x → 降采样）",
            "xbrz" => "10_xbrz.png（xBRZ，像素画档）",
            "lanczos3" => "20_lanczos3.png（Lanczos3，插画档回退）",
            _ => "(未知)",
        };
        let prod_line = if prod_in_items {
            "05_production.png = 缓存真实产物（与本次目标一致，含 webp 编码）".to_string()
        } else if prod_path.is_some() {
            format!(
                "⚠️ 缓存产物与目标尺寸不符：{prod_note}；现状请单独打开源目录里的 @{rate}x.webp"
            )
        } else {
            format!("⚠️ 缓存无 @{rate}x 产物：{prod_note}；本次「现状」只能参考 {production}")
        };
        // AI 列在实验台里**总是生成**：倍率 = 4 时是模型原始输出，其余倍率是「4x → 降采样」。
        // 这里只说明生成方式（及生产是否同款），不做"可用 / 不可用"的判断。
        let ai_line = if !opt.ai {
            "40/41/50/51/60 —— 实验台照常生成（本样本生产未启用 AI）".to_string()
        } else if ai_reachable {
            "40/41/50/51/60 —— 模型 4x 原始输出（本样本生产也用 AI，故此组即现状）".to_string()
        } else if rate != 4 {
            format!("40/41/50/51/60 —— AI 4x → 降采样到 @{rate}x（生产未启用 AI，此处为实验对照）")
        } else {
            "40/41/50/51/60 —— AI 4x 输出（本用途已排除 AI / 运行库不可用，生产不会走 AI）"
                .to_string()
        };
        let mut readme = format!(
            "样本：{}/{}    源尺寸 {}×{} → 目标 {}×{} @{}x\r\n\
             生产判定：{}（{}）—— 仅说明生产走哪条路，**不影响本页任何算法列的生成**\r\n\
             生产现状（参考列）：{prod_line}\r\n\
             AI 列：{ai_line}\r\n\
             \r\n── 列清单（compare.png = 各族代表 6 列；compare_all.png = 全部列）──\r\n",
            p.sub,
            p.label,
            w,
            h,
            tw,
            th,
            rate,
            algo.id(),
            if is_pixel { "像素画" } else { "插画" },
        );
        for (i, (n, _)) in items.iter().enumerate() {
            readme.push_str(&format!(
                "  {:>2}. {:<28} {}\r\n",
                i + 1,
                n,
                column_scope(n, algo.id())
            ));
        }
        readme.push_str(
            "\r\n各方案含义：\r\n  \
             00_ref_nearest      原图最近邻放大 —— 零插值参考（原貌基准）\r\n  \
             05_production       缓存里真实的生产产物（含 webp 编码 / HD 真素材影响），参考列\r\n  \
             10_xbrz + 11/12     xBRZ + USM 锐化（0.4 / 0.8）\r\n  \
             20_lanczos3 / 30/31 Lanczos3 + USM 锐化（0.4 / 0.8）\r\n  \
             40_ai / 41_ai_*     默认模型（**anime6b**）4x 原始输出 / 降到目标倍率\r\n  \
             50/51_ai_mix*       AI 与传统结果混合（AI 权重 0.5 / 0.75）\r\n  \
             60_ai_colorfix      AI + 逐通道色彩/对比度对齐（实验列写法）\r\n  \
             22_lanczos3_dedither       Lanczos3 + 去抖动预滤波（仅模糊 RGB；针对抖动插画）\r\n  \
             32_lanczos3_dedither_usm080  上者 + USM 锐化（0.8）\r\n  \
             15_catmullrom / 16_triangle  CatmullRom / Triangle 重采样（传统族锐度对照）\r\n  \
             xbrz_unify.png / trad_compare.png  提案对照：原貌|xBRZ|Lanczos3 / 全传统族五连\r\n  \
             70/71_preup_*_ai    【实验】先 Lanczos3/xBRZ 放大到目标、再 AI 4x、再降回目标\r\n  \
             80_ai_<模型名>       【实验】组件目录里每个 *.onnx 各一列（放入新模型重跑即多一列）\r\n  \
             81_ai_<模型名>_colormatch  【生产同款】该模型 + 色彩对齐（对齐到 Lanczos3 结果）\r\n\
             \r\n怎么比：00（原貌）→ 05（现状）→ 各族代表横向比（compare.png）；\r\n\
             AI 模型选型看 80 与 81 的对照（81 = 生产实际链路），USM / 混合 / 对齐的组合看 compare_all.png。\r\n",
        );
        // 统一走 xBRZ 提案说明（每目录自带，开发者开图即知本样本是否会变）
        let unify_note = if is_pixel {
            "统一走 xBRZ 提案：本样本当前已走 xBRZ（像素画判定），强制无变化。"
        } else {
            "统一走 xBRZ 提案：本样本当前走 Lanczos3（插画判定，较糊）；强制 xBRZ 后会改用 xBRZ（见 xbrz_unify.png 中栏）。"
        };
        readme.push_str("\r\n");
        readme.push_str(unify_note);
        readme.push_str("\r\n");

        // UTF-8 BOM：保证记事本 / VS Code 打开都不乱码
        let _ = std::fs::write(folder.join("README.txt"), format!("\u{feff}{readme}"));

        // 横向拼接：
        // - `compare.png` = **各族代表 6 列**（原貌 / 现状 / xBRZ / Lanczos3 / AI / AI+混合）——
        //   平铺"同一张图上各算法的差异"，列少，幕布上 6×1600 = 9600 也默认放得下；
        // - `compare_all.png` = 全部列（含 USM 各强度、colorfix），超宽则跳过。
        // 跳过时**删掉上一轮的残留**，避免误读旧图。
        let keep_rep = |n: &str| {
            matches!(
                n,
                "00_ref_nearest.png"
                    | "05_production.png"
                    | "10_xbrz.png"
                    | "20_lanczos3.png"
                    | "40_ai.png"              // 倍率 = 4 的样本落这张
                    | "41_ai_down_to_rate.png" // 其余倍率：AI 4x → 降采样（同样代表 AI 族）
                    | "22_lanczos3_dedither.png" // 去抖动变体（传统对抗抖动糊化的对照列）
            )
        };
        let mut strip_rep: Vec<(Vec<u8>, u32, u32)> = Vec::new();
        let mut strip_all: Vec<(Vec<u8>, u32, u32)> = Vec::new();
        for (n, v) in &items {
            strip_all.push((v.clone(), tw, th));
            if keep_rep(n) {
                strip_rep.push((v.clone(), tw, th));
            }
        }
        // —— 统一走 xBRZ 提案对照：原貌 | xBRZ（提案）| Lanczos3（现状插画档）——
        // 供开发者核对"小尺寸抖动头像统一走 xBRZ"是否可接受：中栏即提案结果，右栏即当前现状。
        let mut unify: Vec<(Vec<u8>, u32, u32)> = Vec::new();
        for key in ["00_ref_nearest.png", "10_xbrz.png", "20_lanczos3.png"] {
            if let Some((_, v)) = items.iter().find(|(n, _)| n == key) {
                unify.push((v.clone(), tw, th));
            }
        }
        if unify.len() == 3 {
            if let Some((joined, jw, jh)) = hstack(&unify) {
                save_png(&folder, "xbrz_unify.png", &joined, jw, jh);
            }
        }

        for (tag, strip) in [("compare.png", &strip_rep), ("compare_all.png", &strip_all)] {
            let total_w = tw.saturating_mul(strip.len() as u32);
            if strip.is_empty() || total_w > strip_max {
                let _ = std::fs::remove_file(folder.join(tag));
                if !strip.is_empty() {
                    println!(
                        "    （{tag} 总宽 {total_w}px > 上限 {strip_max}px，跳过；HD_COMPARE_STRIP_MAX 可调大）"
                    );
                }
                continue;
            }
            if let Some((joined, jw, jh)) = hstack(strip) {
                save_png(&folder, tag, &joined, jw, jh);
            }
        }
        println!(
            "    列顺序：{}\n    → {}\n",
            items
                .iter()
                .map(|(n, _)| n.trim_end_matches(".png"))
                .collect::<Vec<_>>()
                .join(" | "),
            folder.display()
        );
        index_lines.push(format!(
            "{:<32} {:>4}x  {:<6}  生产算法={:<9} 现状产物={}{}",
            format!("{}-{}", p.sub, p.label),
            rate,
            if is_pixel { "像素画" } else { "插画" },
            algo.id(),
            if prod_in_items {
                "有"
            } else if prod_path.is_some() {
                // 文件在、但尺寸 ≠ 本次目标 → 缓存由别的参数（phys / 倍率）构建过
                "尺寸不符"
            } else {
                "无"
            },
            if rate < 2 {
                "   ← 生产不生成变体（源图已够大），此页仅作算法对照"
            } else {
                ""
            }
        ));
    }

    // 根目录索引：一眼看全「每类素材生产走哪条路 / 缓存里有没有现状产物」
    let mut index = String::from(
        "hd-compare 索引（实验台：每个样本都跑全部算法与全部 AI 模型，与生产分派无关）\r\n\
         列：样本目录 / 目标倍率 / 判定 / 生产算法 / 缓存现状产物\r\n\
         \r\n• 默认 AI 模型 = anime6b（见 infra::ai::RT_MODEL / runtime::COMPONENTS）；\r\n\
         • 每样本含 `80_ai_<模型名>.png`（组件目录里每个 *.onnx 一列，放入新模型重跑即多一列）\r\n\
           与 `81_ai_<模型名>_colormatch.png`（+ 色彩对齐 = **生产实际链路**）；\r\n\
         • 各目录内 README.txt 写明每一列的含义；compare.png = 各族代表 6 列，\r\n\
           compare_all.png = 全部列（超宽样本需 HD_COMPARE_STRIP_MAX 调大）。\r\n\r\n",
    );
    for l in &index_lines {
        index.push_str(l);
        index.push_str("\r\n");
    }
    let _ = std::fs::write(out_root.join("INDEX.txt"), format!("\u{feff}{index}"));
    // ── 数值汇总（跨样本平均）──
    println!(
        "\n=== 数值评估汇总（跨 {} 个样本平均；基准 = 各样本的 00_ref_nearest）===",
        picks.len()
    );
    println!(
        "{:<26} {:>8} {:>9} {:>8} {:>8} {:>10}",
        "方案", "PSNR↑", "锐度↑", "色偏↓", "平坦↓", "结构相关↑"
    );
    for (k, ms) in &agg {
        let n = ms.len().max(1) as f64;
        println!(
            "{:<26} {:>8.2} {:>9.3} {:>8.3} {:>8.3} {:>10.4}   n={}",
            k,
            ms.iter().map(|m| m.psnr).sum::<f64>() / n,
            ms.iter().map(|m| m.sharp).sum::<f64>() / n,
            ms.iter().map(|m| m.color).sum::<f64>() / n,
            ms.iter().map(|m| m.flat).sum::<f64>() / n,
            ms.iter().map(|m| m.corr).sum::<f64>() / n,
            ms.len()
        );
    }
    println!(
        "完成：共 {} 个样本（根目录另有 INDEX.txt 汇总）。",
        picks.len()
    );
}
