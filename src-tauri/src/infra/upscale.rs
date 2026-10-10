//! 高清素材放大（**可插拔算法层**）。
//!
//! 设计目标：算法可替换（开发者要求预留接入其它算法的可能性）。当前内置：
//! - `xbrz`　　：xBRZ 1.8 的 Rust 移植（**默认**）—— 像素画专用的边缘保持放大，
//!   与 VCMI 引擎 `SDLImageShared::scale` 同款；
//! - `nearest` ：整数倍最近邻 —— 唯一"真无损"放大（无插值），观感为马赛克，
//!   在此作为**清晰度上界基线**：任何算法若不如它锐利就没有接入价值；
//! - `ai`　　　：Real-ESRGAN 超分（模型内嵌，**仅 4 倍**）—— 插画类首选，见 `infra::ai`；
//! - `lanczos3` / `catmullrom` / `triangle`：`image` crate 重采样（低通插值，
//!   放大后偏软；用于对照与"写实类大图"场景）。
//!
//! 新增算法（如未来的 ONNX 超分）只需实现 [`Upscaler`] 并在 [`ALL`] 登记，
//! **调用方无需改动** —— 全部调用都走 [`upscale_with`] / [`upscale_rgba`]。
//!
//! 依赖方向：`infra` 层 —— 无 tauri 类型、无 VCMI 语义
//! （见 `docs/architecture/backend.zh-CN.md` §4.1）。
//!
//! ⚠️ 所有实现必须 `Send + Sync`：素材阶段在 rayon 池里并行调用。
//!
//! 选型依据见 `modcache/smoke_tests/hd_bench.rs`（真实素材实测）。关键结论：
//! xBRZ 的开销由**源像素数**决定（逐源像素邻域分析 + 块填充）⇒ 3x 几乎不比 2x 贵；
//! Lanczos3 由**目标像素数**决定（逐目标像素卷积）⇒ 3x 约为 2x 的 2.25 倍。

use image::imageops::FilterType;
use image::RgbaImage;

/// 放大算法。**实现体必须是 `'static`**（注册表以静态引用持有）。
pub trait Upscaler: Send + Sync {
    /// 稳定标识（配置 / 日志 / 未来的缓存键）
    fn id(&self) -> &'static str;

    /// 支持的倍率上限；超出由 [`upscale_with`] 钳位
    fn max_factor(&self) -> u32;

    /// RGBA → RGBA **整数倍**放大，返回 `(rgba, nw, nh)`；`None` 表示失败（调用方回退原图）。
    ///
    /// 入参保证：`rgba.len() == w * h * 4` 且 `factor ∈ 1..=max_factor()`
    /// （由 [`upscale_with`] 统一校验，实现内不必重复）。
    fn upscale(&self, rgba: &[u8], w: u32, h: u32, factor: u32) -> Option<(Vec<u8>, u32, u32)>;
}

/// 默认算法：xBRZ（像素画专用，与 VCMI 引擎观感一致）
pub const DEFAULT_UPSCALER_ID: &str = "xbrz";

/// 最近邻（整数倍块复制）
pub struct NearestUpscaler;

/// `image` crate 的通用重采样（Lanczos3 / CatmullRom / Triangle）
pub struct FilterUpscaler {
    id: &'static str,
    filter: FilterType,
}

/// xBRZ（`xbrz-rs`，**GPL-3.0-only**）
pub struct XbrzUpscaler;

impl Upscaler for NearestUpscaler {
    fn id(&self) -> &'static str {
        "nearest"
    }
    fn max_factor(&self) -> u32 {
        8
    }
    fn upscale(&self, rgba: &[u8], w: u32, h: u32, factor: u32) -> Option<(Vec<u8>, u32, u32)> {
        resize_with(rgba, w, h, factor, FilterType::Nearest)
    }
}

impl Upscaler for FilterUpscaler {
    fn id(&self) -> &'static str {
        self.id
    }
    fn max_factor(&self) -> u32 {
        8
    }
    fn upscale(&self, rgba: &[u8], w: u32, h: u32, factor: u32) -> Option<(Vec<u8>, u32, u32)> {
        resize_with(rgba, w, h, factor, self.filter)
    }
}

impl Upscaler for XbrzUpscaler {
    fn id(&self) -> &'static str {
        "xbrz"
    }
    /// xBRZ 原版只支持 1..=6
    fn max_factor(&self) -> u32 {
        6
    }
    fn upscale(&self, rgba: &[u8], w: u32, h: u32, factor: u32) -> Option<(Vec<u8>, u32, u32)> {
        let f = factor as usize;
        // crate 在入参长度不符时会 panic；长度已由 `upscale_with` 切片保证。
        let out = xbrz::scale_rgba(rgba, w as usize, h as usize, f);
        let expect = (w as usize * f) * (h as usize * f) * 4;
        if out.len() != expect {
            return None;
        }
        Some((out, w * factor, h * factor))
    }
}

/// AI 超分（Real-ESRGAN x4，模型内嵌）—— 实现与全部设计说明见 [`crate::infra::ai`]。
///
/// ⚠️ **只支持 4 倍**：`upscale(.., factor != 4)` 一律返回 `None`（不会静默产出错误尺寸）。
/// 因此调用方（`modcache::hd::pick_upscaler`）只在目标倍率恰为 4 时才选它，
/// 其余倍率走传统算法。
struct AiUpscaler;

impl Upscaler for AiUpscaler {
    fn id(&self) -> &'static str {
        "ai"
    }
    /// 模型本身只支持 4x，但对外**支持任意倍率**：先跑 4x、再重采样到目标尺寸。
    ///
    /// 目标是 2x/3x 时即"4x → 降采样"：降采样会平均掉一部分生成式幻觉，
    /// 对照实验（`hd_compare` 的 `41_ai_down_to_rate` 列）显示它在纹理类上并不比插值差。
    /// 这样"AI 优先"策略才不局限于 4x 素材。
    fn max_factor(&self) -> u32 {
        8
    }
    fn upscale(&self, rgba: &[u8], w: u32, h: u32, factor: u32) -> Option<(Vec<u8>, u32, u32)> {
        let target = (w * factor, h * factor);
        let (ai4, aw, ah) = crate::infra::ai::upscale(rgba, w, h, crate::infra::ai::AI_FACTOR)?;
        if (aw, ah) == target {
            return Some((ai4, aw, ah));
        }
        resize_to(&ai4, aw, ah, target.0, target.1, FilterType::Lanczos3)
    }
}

/// 任意**目标尺寸**重采样（含**缩小**）：`image::imageops::resize` 薄封装。
///
/// 供 `AiUpscaler` 把 4x 输出落到其它倍率；整数倍放大仍走 [`upscale_with`]。
pub fn resize_to(
    rgba: &[u8],
    w: u32,
    h: u32,
    tw: u32,
    th: u32,
    ft: FilterType,
) -> Option<(Vec<u8>, u32, u32)> {
    if w == 0 || h == 0 || tw == 0 || th == 0 {
        return None;
    }
    let need = (w as usize) * (h as usize) * 4;
    if rgba.len() < need {
        return None;
    }
    let src = RgbaImage::from_raw(w, h, rgba[..need].to_vec())?;
    let out = image::imageops::resize(&src, tw, th, ft);
    Some((out.into_raw(), tw, th))
}

/// `image` crate 重采样的公共实现
fn resize_with(
    rgba: &[u8],
    w: u32,
    h: u32,
    factor: u32,
    ft: FilterType,
) -> Option<(Vec<u8>, u32, u32)> {
    let src = RgbaImage::from_raw(w, h, rgba.to_vec())?;
    let out = image::imageops::resize(&src, w * factor, h * factor, ft);
    Some((out.into_raw(), w * factor, h * factor))
}

static NEAREST: NearestUpscaler = NearestUpscaler;
static TRIANGLE: FilterUpscaler = FilterUpscaler {
    id: "triangle",
    filter: FilterType::Triangle,
};
static CATMULLROM: FilterUpscaler = FilterUpscaler {
    id: "catmullrom",
    filter: FilterType::CatmullRom,
};
static LANCZOS3: FilterUpscaler = FilterUpscaler {
    id: "lanczos3",
    filter: FilterType::Lanczos3,
};
static XBRZ: XbrzUpscaler = XbrzUpscaler;
static AI: AiUpscaler = AiUpscaler;

/// 已注册的全部算法（bench 遍历 / 配置校验用）
static ALL: [&dyn Upscaler; 6] = [&NEAREST, &TRIANGLE, &CATMULLROM, &LANCZOS3, &XBRZ, &AI];

/// 全部算法（新增实现后登记到 [`ALL`] 即可被自动纳入）
pub fn all_upscalers() -> &'static [&'static dyn Upscaler] {
    &ALL
}

/// 按 id 取算法；未知 id 返回 `None`（调用方应回退默认算法）
pub fn upscaler_by_id(id: &str) -> Option<&'static dyn Upscaler> {
    all_upscalers().iter().copied().find(|u| u.id() == id)
}

/// 默认算法（取不到时兜底最近邻，绝不返回 None）
pub fn default_upscaler() -> &'static dyn Upscaler {
    upscaler_by_id(DEFAULT_UPSCALER_ID).unwrap_or(&NEAREST)
}

/// 统一入口（按算法名）：校验尺寸 → 钳位倍率 → 调实现。
///
/// 未知算法名**静默回退默认算法**而非失败：素材放大是增强项，不应因配置拼写错误丢图。
pub fn upscale_rgba(
    id: &str,
    rgba: &[u8],
    w: u32,
    h: u32,
    factor: u32,
) -> Option<(Vec<u8>, u32, u32)> {
    let u = upscaler_by_id(id).unwrap_or_else(default_upscaler);
    upscale_with(u, rgba, w, h, factor)
}

/// 统一入口（按算法对象）：校验尺寸 → 钳位倍率 → 调实现。
pub fn upscale_with(
    u: &dyn Upscaler,
    rgba: &[u8],
    w: u32,
    h: u32,
    factor: u32,
) -> Option<(Vec<u8>, u32, u32)> {
    if w == 0 || h == 0 {
        return None;
    }
    let f = factor.clamp(1, u.max_factor());
    let need = (w as usize) * (h as usize) * 4;
    if rgba.len() < need {
        return None;
    }
    u.upscale(&rgba[..need], w, h, f)
}
