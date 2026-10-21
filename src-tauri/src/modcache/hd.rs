//! 高清素材（`@Nx`）生成 —— 素材阶段之后的**幂等后处理**。
//!
//! ## 为什么放在"构建之后"而不是 `save_wiki_image` 内部
//!
//! `save_wiki_image` 收到的是**已编码的 webp 字节**（不是 RGBA），在那里放大需要
//! "解码 → 放大 → 再编码"，且会牵动全部素材落盘调用点（几十处签名）。改为构建完成后
//! 统一遍历 `images/**` 的后处理：
//! - 与既有提取管线**完全解耦**，不改任何落盘路径与字段；
//! - **幂等可增量**（已存在即跳过），中断后重跑只补缺的；
//! - 未来「加急队列 / 按需补生成」可复用同一入口。
//!
//! ## 约定
//!
//! - 原素材 `<name>.webp` **始终保留**：前端读不到 `@Nx` 时回退原图（见 `src/lib/hd.ts`）；
//! - 高清变体命名 `<name>@<rate>x.webp`（同目录），`stem` 含 `@` 的文件一律跳过（避免二次放大）；
//! - 放大算法走 `infra::upscale` 的**可插拔注册表**（默认 xBRZ，未知 id 静默回退默认）；
//! - 倍率来自**用户显示器**（`commands` 层传入，见 `list_monitors` + `lib/hd.ts`）。
//!
//! 依赖方向：可依赖 `infra`；**不得**出现 `tauri` 类型
//! （见 `docs/architecture/backend.zh-CN.md` §4.1）。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::infra::image::{encode_rgba_auto, encode_webp_lossless, encode_webp_lossy_q};
use crate::infra::upscale::{default_upscaler, upscale_with, Upscaler};
use crate::modcache::sanitize_id;

/// 全局倍率上限，与 `infra::upscale::XbrzUpscaler::max_factor` 及前端 `MAX_HD_RATE` 一致。
pub const MAX_HD_RATE: u32 = 6;

/// 最苛刻的显示放大比（CSS）：巡礼放大区 148px 高 ÷ core 兵种源图 64px 高。
/// 与前端 `lib/hd.ts::WORST_DISPLAY_ZOOM` 同源，仅用于"旧前端未传 `phys` 时"的反推兜底。
const WORST_DISPLAY_ZOOM: f64 = 148.0 / 64.0;

/// 高清方案版本：**编码策略 / 分级规则 / 命名约定**变更时递增。
///
/// 递增后首次运行会**清理全部 `@Nx` 变体**并重新生成 —— 变体的幂等判定只看
/// "目标文件是否存在"，所以编码质量或分级规则变了，旧变体不会自己更新。
/// （仅"倍率随显示器变化"不在此列：那由逐图 `target_rate` 对比自动清理。）
pub const HD_SCHEME_VERSION: u32 = 2;

/// 进程内"方案已核对"标志：`generate_for_mod` 会被 600+ 个 mod 调用，只应核对一次。
static SCHEME_CHECKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// 本轮进程是否**清理过**旧变体（方案版本变化）。供批次对"缓存仍新鲜"的 mod 补跑一次
/// 变体生成，避免它们在方案切换后长期缺 `@Nx`。
static SCHEME_CHANGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 本轮进程是否因方案版本变化清理过旧变体（见 [`ensure_hd_scheme`]）。
///
/// **会先触发一次方案核对**（批次入口在动手前问，正是为了这一下）：核对发生在进程内
/// 只做一次，后续每个 mod 的 `generate_for_mod` 看到 `SCHEME_CHECKED` 即跳过。
pub fn scheme_changed(cache_root: &Path) -> bool {
    ensure_hd_scheme(cache_root);
    SCHEME_CHANGED.load(std::sync::atomic::Ordering::SeqCst)
}

/// 方案版本不匹配时清理全部 `@Nx` 变体（下次生成按新方案重建）。
///
/// 标记文件 `<cache_root>/.hd-scheme` 记录上次使用的方案版本；不匹配（含首次）即清理。
/// 进程内只核对一次；写标记失败也不会反复清理（跨进程最多多清一次，代价可接受）。
fn ensure_hd_scheme(cache_root: &Path) {
    if SCHEME_CHECKED.swap(true, std::sync::atomic::Ordering::SeqCst) {
        return;
    }
    let mark = cache_root.join(".hd-scheme");
    let cur = fs::read_to_string(&mark)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok());
    if cur == Some(HD_SCHEME_VERSION) {
        return;
    }
    SCHEME_CHANGED.store(true, std::sync::atomic::Ordering::SeqCst);
    let mut removed = 0usize;
    let mut maps = 0usize;
    if let Ok(mods) = fs::read_dir(cache_root) {
        for m in mods.flatten() {
            let img = m.path().join("images");
            if img.is_dir() {
                let mut files = Vec::new();
                collect_webp(&img, &mut files);
                for f in files {
                    let is_variant = f
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .and_then(variant_rate)
                        .is_some();
                    if is_variant && fs::remove_file(&f).is_ok() {
                        removed += 1;
                    }
                }
            }
            // 第三方 mod 的 HD 来源登记一并作废（v3：旧登记含"帧名误配原版素材"的脏项）。
            // core / core.<faction> 的登记保留 —— 那是原版资源的合法 HD 源，清掉会让
            // 原版生物/英雄/图标的高清集体退化成插值，直到各自重建。
            let id = m
                .file_name()
                .to_string_lossy()
                .to_string();
            if !crate::vcmi::h3core::parse_core_mod(&id).0
                && m.path().join("hd-map.json").is_file()
                && fs::remove_file(m.path().join("hd-map.json")).is_ok()
            {
                maps += 1;
            }
        }
    }
    tracing::info!(
        target: "wiki",
        "高清方案版本 {} → {}：清理旧变体 {} 个、作废第三方 HD 登记 {} 份（将按新方案重新生成）",
        cur.map(|v| v.to_string()).unwrap_or_else(|| "无".into()),
        HD_SCHEME_VERSION,
        removed,
        maps
    );
    let _ = fs::write(&mark, HD_SCHEME_VERSION.to_string());
}

/// 高清变体的有损质量（0-100）。
///
/// 与 `infra::image::LOSSY_QUALITY`（90，用于原素材小图标）分开：变体一律走有损，
/// 且放大后的图**细节冗余更高**，稍低的质量即可等价观感，换来显著体积收益。
///
/// bench 实测（`hd_variant_quality_bench`，1600×748 城镇幕布；**无损基线 1855 KB**）：
///
/// | 质量 | q95 | q90 | **q85** | q80 | q75 |
/// |---|---|---|---|---|---|
/// | 体积 | 447 KB | 320 KB | **234 KB** | 188 KB | 154 KB |
/// | 相对 q95 | 100% | 71% | **52%** | 42% | 34% |
///
/// 取 **85**：相对 q90 再省 27% 而观感无差；再往下收益递减（q80 仅再省 20%）。
pub const HD_QUALITY: f32 = 85.0;

/// 高清生成选项（`rate < 2` = 不生成）
#[derive(Clone, Debug)]
pub struct HdOptions {
    /// 全局倍率**上限**（2..=6）：由"屏幕 + 最苛刻显示场景"推出，同时是各用途倍率的钳位上限。
    pub rate: u32,
    /// **基准物理倍率** = 屏幕物理宽 / 1280（整页缩放使 1 CSS px = 该值物理像素）。
    ///
    /// 分级倍率必须用它：`需求物理 = 目标CSS × phys`。
    /// 旧版前端未传时按 `rate / WORST_DISPLAY_ZOOM` 反推上界；`rate` 已被钳到上限时
    /// 保守取 [`MAX_HD_RATE`]（宁可多生成也不糊——反推在钳位情形会低估）。
    pub phys: f64,
    /// 算法 id（未知 id 由 `infra::upscale` 静默回退默认算法）
    pub upscaler: String,
    /// **AI 高清增强**（调用方传入的开关意图；实际生效还要求 [`Self::ai_model`] 有值）。
    ///
    /// 开启后：**插画档**（试算判定为非像素画）优先走 `infra::ai` 的 Real-ESRGAN 超分；
    /// 失败则回退 Lanczos3。像素画仍走 xBRZ —— 实测 AI 会把像素风平滑成噪点。
    pub ai: bool,
    /// **AI 模型文件**（绝对路径，构造时已确认存在）。
    ///
    /// 来源 = `launcher.json` 的 `hd.ai.model` → 组件目录 `runtime/ai/`。
    /// `None` = 未指定 / 配置被拒 / 文件不存在 ⇒ **AI 不生效**（**无默认模型**，不猜）。
    /// 构造时已注入推理层（见 [`HdOptions::new`] 与 `infra::ai::configure_model`）。
    pub ai_model: Option<std::path::PathBuf>,
    /// **USM 锐化强度**（0 = 关闭；`launcher.json` 的 `hd.usm`，缺省 0.8）。
    ///
    /// 传统插值放大后偏软，叠 USM 拿回锐度且无生成伪影；像素画**不叠**（硬边怕光晕）。
    /// 由 [`HdOptions::new`] 从配置读取注入 —— 单测 / 对比集自行构造时给默认值即可。
    pub usm: f32,
    /// **AI 混合权重**（`launcher.json` 的 `hd.ai.mix`，缺省 0.75）：`α×AI + (1−α)×传统`。
    /// 只在 AI 真正生效时有意义；`1.0` = 纯 AI 输出。
    pub ai_mix: f32,
}

impl HdOptions {
    /// 由命令层入参构造。`rate` 缺省或 `< 2` → `None`（不做高清，只用原素材）。
    pub fn new(
        rate: Option<u32>,
        phys: Option<f64>,
        upscaler: Option<String>,
        ai: Option<bool>,
    ) -> Option<Self> {
        let rate = rate.unwrap_or(1);
        if rate < 2 {
            return None;
        }
        let rate = rate.min(MAX_HD_RATE);
        let phys = phys.filter(|p| *p > 0.0).unwrap_or_else(|| {
            if rate >= MAX_HD_RATE {
                MAX_HD_RATE as f64
            } else {
                rate as f64 / WORST_DISPLAY_ZOOM
            }
        });
        // USM 强度、AI 混合权重与 **AI 模型**只从 `launcher.json` 读（不暴露给前端）：
        // 构建期一次性取值，避免逐图读配置；配置缺失时各自回落默认档。
        let ai_on = ai.unwrap_or(false);
        // 模型解析：`hd.ai.model` → 组件目录内的绝对路径。
        // 缺省 / 非法 / 文件不存在 ⇒ `None`（AI 不生效，**无默认模型**）。
        // 四种原因在此各记一次日志（每批构建只走一次，不会逐图刷屏）。
        let ai_model = if ai_on {
            match crate::vcmi::paths::VcmiPaths::hd_ai_model_name() {
                None => {
                    tracing::warn!(
                        target: "launcher",
                        "AI 增强已开启但未指定模型（launcher.json 的 hd.ai.model）—— AI 不生效，插画档走 Lanczos3 + USM"
                    );
                    None
                }
                Some(name) => match crate::runtime::ai_model_path(&name) {
                    Some(p) => Some(p),
                    None => {
                        tracing::warn!(
                            target: "launcher",
                            "hd.ai.model 指向的模型不存在：{name}（组件目录内无此文件）—— AI 不生效"
                        );
                        None
                    }
                },
            }
        } else {
            None
        };
        // 注入推理层：全局单选（同一进程同一时刻只用一个模型），会话池按模型分桶
        crate::infra::ai::configure_model(ai_model.clone());
        Some(Self {
            rate,
            phys,
            upscaler: upscaler.unwrap_or_else(|| default_upscaler().id().to_string()),
            ai: ai_on,
            ai_model,
            usm: crate::vcmi::paths::VcmiPaths::hd_usm_amount(),
            ai_mix: crate::vcmi::paths::VcmiPaths::hd_ai_mix_amount(),
        })
    }

    /// 解析出算法对象（未知 id 回退默认）
    pub fn upscaler(&self) -> &'static dyn Upscaler {
        crate::infra::upscale::upscaler_by_id(&self.upscaler).unwrap_or_else(default_upscaler)
    }
}

/// **目标显示框表**：每个用途（`images/<sub>/` 目录名）声明"这张图最大会被显示到多大"，
/// 单位是**设计稿 CSS px**（启动器整页缩放，逻辑视口恒为 1280×720）。
///
/// ## 为什么需要它
///
/// 素材需求 = `目标CSS × 物理倍率 ÷ 源像素`。各用途差异极大：
/// - `creature` 58×64 要在巡礼放大区撑到 **148 CSS 高** → 1080p 需 4x；
/// - `town` 800×374 只在幕布里放大到 **1.075×**（860 CSS 宽）→ 1080p 只需 2x。
///
/// 若不分用途一律按最苛刻的倍率生成，`town`/`townscreen` 会被放大到 6x（4800×2244），
/// 体积从 ~250 KB 涨到 ~7 MB —— 多出的像素永远不会被看到。
///
/// ## 数值来源（改 UI 尺寸必须同步这里）
///
/// | sub | 目标框 | 依据（设计稿常量） |
/// |---|---|---|
/// | creature / hero | 134×148 | `ShowcaseOverlay.tsx` 放大区 `DETAIL_IMG_H=148`；源 AR 58/64 → 宽 134 |
/// | building | 250×117 | 放大区 `DETAIL_IMG_W=250`；源 AR 150/70 → 高 117 |
/// | town / townscreen / townborder | 911×426 | 幕布 `frameH` 上限 = `floor((1228−300−16)÷(800/374))` = 426（1228 = 1280−2×26 padding）；宽 426×2.139 = 911 |
/// | terrain | 96×96 | `TerrainDetail` `h-24 w-24` |
/// | herospec | 44×44 | `HeroDetail` 特长 `h-11 w-11` |
/// | luck / morale | 42×42 | `CreatureDetail` `h-[42px] w-[42px]` |
/// | ability | 52×52 | `CreatureDetail` 能力 `h-[52px] w-[52px]` |
/// | warmachine | 32×32 | `HeroDetail` 投石车 `h-8 w-8` |
/// | artifact | 32×32 | `ArtifactDetail` `h-8 w-8` |
/// | spell / skill / spellbon / spellbook / secsk32 | 56×56 | `detailShared` `DetailHeader` `h-14 w-14` |
/// | pskil42 | 32×32 | `HeroDetail` 主属性 `h-8 w-8`（原写 28×28 与实际不符，导致该目录永不生成变体） |
/// | stats | 20×20 | `FactionDetail` 表头 `h-5 w-5` |
/// | stackwin / resources | 16×16 | `CreatureDetail` 属性行 / `formatCost` `h-4 w-4` |
///
/// 返回 `None` = **未知用途** → 退回全局上限倍率（保守：宁可浪费也不糊）。
pub(crate) fn hd_target_box(sub: &str) -> Option<(u32, u32)> {
    Some(match sub {
        // 巡礼「放大展示区」——全站最苛刻的两处
        "creature" | "hero" => (134, 148),
        "building" => (250, 117),
        // 幕布（含按幕布坐标叠加的结构描边图）
        "town" | "townscreen" | "townborder" => (911, 426),
        // 详情页
        "terrain" => (96, 96),
        "herospec" => (44, 44),
        "luck" | "morale" => (42, 42),
        "ability" => (52, 52),
        "warmachine" | "artifact" => (32, 32),
        "spell" | "skill" | "spellbon" | "spellbook" | "secsk32" => (56, 56),
        // ⚠️ 必须是 32 而非 28：`HeroDetail` 主属性图标是 `h-8 w-8`（32 CSS px）。
        // 取 28 时 1080p 需求 = 28×1.5 = 42px，恰等于 PSKIL42 源帧 42×42 →
        // `target_rate` 恒为 1 ⇒ 该目录**一个 `@Nx` 都不生成**，HD 版（hd-map 已登记
        // `PSKIL42A/D/P/S`）也就永远用不上（实测 2026-10-20 用户报「pskil42 没走高清」）。
        "pskil42" => (32, 32),
        "stats" => (20, 20),
        "stackwin" | "resources" => (16, 16),
        _ => return None,
    })
}

/// 判定「像素画」的颜色数上限（百分比）：量化到 5 位/通道后，不同颜色数 ÷ 像素数。
///
/// 实测分布（`hd_bench::hd_preview`，core 原版素材）**分离得很干净**：
///
/// | 类型 | 样本 | 占比 |
/// |---|---|---|
/// | 像素画（走 xBRZ） | 建筑图标 150×70 / 城镇生物背景 100×130 / 战斗精灵 64×64 / 英雄头像 58×64 / 地形块 32×32 | 4% / 4% / 17% / 20% / **21%** |
/// | 插画（走 Lanczos3） | griffin / angel / archer / pikeman / halberdier `-large` 58×64 | **27%** / 27% / 28% / 29% / **32%** |
///
/// 阈值取 **24**（两类之间，两侧各留 ~3 个百分点余量）。
/// ⚠️ 若第三方素材出现误判（如高清像素画的占比偏高），优先调这里而不是改判定方式。
#[allow(dead_code)] // 供 `hd_preview` 汇总与诊断使用（生产判定走 `pick_upscaler` 的试算）
const PIXEL_ART_MAX_COLOR_PCT: u32 = 24;

/// [`pick_upscaler`] 的"大片纯色 → 直接判像素画"快速通道阈值（百分比）。
///
/// 单独成常量是因为 **HD 变体直出路径**（`k == 1`，官方高清图无需插值）也要用同一阈值
/// 决定编码分派（像素画 → 无损；否则按尺寸走有损）。
const PIXEL_ART_FAST_PCT: u32 = 10;

/// 量化后不同颜色数占像素数的**百分比**（`is_pixel_art` 的判定依据，也供 bench 标定）
#[allow(dead_code)] // 同上：诊断 + `pick_upscaler` 的"大片纯色"快速通道
pub(crate) fn color_ratio_pct(rgba: &[u8], w: u32, h: u32) -> u32 {
    let n = (w as usize) * (h as usize);
    if n == 0 {
        return 0;
    }
    let mut set = std::collections::HashSet::new();
    for px in rgba.chunks_exact(4).take(n) {
        let q = ((px[0] >> 3) as u32) << 10 | ((px[1] >> 3) as u32) << 5 | (px[2] >> 3) as u32;
        set.insert(q);
    }
    (set.len() * 100 / n) as u32
}

/// 是否**像素画**（色块少、硬边）。
///
/// ⚠️ **不要用它决定放大算法**（只作诊断/统计）。实测反例：
/// `creature-32x32`（像素画，要 xBRZ，占比 35%）与 `halberdier-large`（抖动插画，要 Lanczos3，
/// 占比 32%）统计量几乎相同，而最优算法相反 —— 颜色占比**不足以区分**二者。
/// 算法选择改用 [`pick_upscaler`]（试算看结果，不猜）。
#[allow(dead_code)] // 同上：仅诊断
pub(crate) fn is_pixel_art(rgba: &[u8], w: u32, h: u32) -> bool {
    color_ratio_pct(rgba, w, h) <= PIXEL_ART_MAX_COLOR_PCT
}

/// 平均相邻亮度差（锐度代理）：越高越锐（马赛克/噪声也会拉高，故仅作**相对比较**）。
pub(crate) fn grad_energy(rgba: &[u8], w: u32, h: u32) -> f64 {
    let mut sum = 0.0f64;
    let mut n = 0usize;
    for y in 0..h as usize {
        for x in 0..(w as usize).saturating_sub(1) {
            let i = (y * w as usize + x) * 4;
            if i + 8 > rgba.len() {
                continue;
            }
            let a =
                0.299 * rgba[i] as f32 + 0.587 * rgba[i + 1] as f32 + 0.114 * rgba[i + 2] as f32;
            let b = 0.299 * rgba[i + 4] as f32
                + 0.587 * rgba[i + 5] as f32
                + 0.114 * rgba[i + 6] as f32;
            sum += (a - b).abs() as f64;
            n += 1;
        }
    }
    if n == 0 {
        0.0
    } else {
        sum / n as f64
    }
}

/// 试算判定的**最大源边长**：超过此值不再试算，直接用平滑算法。
///
/// 试算要跑两次放大，代价随目标尺寸增长 —— 800×374 的幕布试算一次约 1.3s（debug），
/// 不值得；而大图以写实/背景为主，Lanczos3 本就是合适选择。
const AUTO_PICK_MAX_EDGE: u32 = 200;

/// 试算判定的**锐度倍数阈值**：xBRZ 的锐度至少是 Lanczos3 的 1.10 倍才选它。
///
/// 实测：真正的像素画差距极大（`hero` 3.3×、`creature-32x32` 1.5×），
/// 而插画上 xBRZ 反而不占优（`halberdier` 0.98×）→ 1.10 能干净分开两类。
const AUTO_PICK_RATIO: f64 = 1.10;

/// **强像素画**的锐度比下限：试算 `xBRZ/Lanczos3` 锐度比 ≥ 此值 ⇒ 真像素画（方块 / 索引色），
/// 坚持 xBRZ；落在 [`AUTO_PICK_RATIO`] 与此值之间的（草地 / 石墙这类**连续纹理**，
/// 会被试算判成像素画）在 AI 模式下交给 AI —— 实测 AI 在这类图上纹理更细腻、结构不变形。
const PIXEL_STRONG_RATIO: f64 = 2.0;

/// AI 的**源图长边下限**：更小的图（`herospec` 44px、`artifact` 24px 这类小图标）先 4x
/// 生成再降采样，重绘的细节被平均成噪点且形态漂移（实测火焰图标变形）⇒ 收益为负，不走 AI。
/// 取 64：`creature`/`hero`（64px）、`building`、`spellbon`、幕布都保留 AI；
/// `herospec`/`artifact`/`terrain`/`luck`/`warmachine`/`secsk32`/`pskil42`/小图标全部挡住。
const AI_MIN_SOURCE_EDGE: u32 = 64;

/// 小尺寸抖动插画的**源图长边上限**：试算判为"插画"且长边 ≤ 此值时改判像素画（xBRZ）。
///
/// 实测依据（`hd_compare` 42 样本 + 肉眼核对）：creature/hero 58×64 抖动头像的抖动点阵
/// 会把 xBRZ 的锐度优势稀释到 [`AUTO_PICK_RATIO`] 之下 → 被误判"插画"走 Lanczos3，
/// 把抖动磨成糊斑（用户报"2x/3x 模糊"的根因）；xBRZ 的可辨识度显著更高
///（代价：抖动点被当边缘锐化的"雕版感"镶边，可接受）。此尺寸下 Triangle/CatmullRom/
/// Lanczos3 锐度无差（6–8），换插值核无效 —— 只有 xBRZ 把锐度拉到 11.6。
///
/// ⚠️ 仅在 **AI 未接管**时生效（AI 实测抹抖动、重建纹理，优于 xBRZ，见 `smooth_or_ai`）。
const SMALL_DITHER_EDGE: u32 = 64;

/// 去重颜色占比（%）= `去重颜色数 × 100 / 像素数` —— **仅诊断，不参与判定**。
///
/// 曾试图用它做「AI 模式的纹理豁免」（占比高 ⇒ 连续纹理 ⇒ 交给 AI），实测判据不成立：
/// 真像素画 `terrain/dirt` 高达 91%（地形纹理本身就是彩色噪点），而想放行的
/// `town/castle-creature-bg` 只有 48% —— 方向相反。保留它是为了在 `hd_compare`
/// 输出里观察各类素材的色彩分布，供将来设计更可靠的判据（局部自相关 / 边缘方向一致性）。
///
/// 只在**试算路径**（源图 ≤ [`AUTO_PICK_MAX_EDGE`]）被调用，像素数上限 4 万，开销可忽略。
#[allow(dead_code)] // 诊断用（`hd_compare` 打印各类素材的色彩分布），非 test 构建无调用者
pub(crate) fn rich_texture_pct(rgba: &[u8], w: u32, h: u32) -> u32 {
    let n = (w as usize) * (h as usize);
    if n == 0 || rgba.len() < n * 4 {
        return 0;
    }
    let mut set: std::collections::HashSet<[u8; 4]> =
        std::collections::HashSet::with_capacity(1024);
    for i in 0..n {
        set.insert([
            rgba[i * 4],
            rgba[i * 4 + 1],
            rgba[i * 4 + 2],
            rgba[i * 4 + 3],
        ]);
    }
    ((set.len() * 100) / n) as u32
}

/// **自动选择放大算法**：试算两种候选，取"更锐"的结果。
///
/// 为什么不用图像统计量（颜色数、熵之类）：实测 `creature-32x32` 与 `halberdier-large`
/// 的统计量几乎相同，但一个要 xBRZ、一个要 Lanczos3 —— 只有**比较两种算法的实际产出**
/// 才是可靠判据（且不依赖任何阈值猜测）。
///
/// 规则：
/// - 源图任一边 > [`AUTO_PICK_MAX_EDGE`] → 直接用 [`smooth_upscaler`]（省一次大图试算）；
/// - 否则试算 2x（足以体现算法特性）：`xBRZ 锐度 ≥ Lanczos3 锐度 × 1.10` → 用配置的算法
///   （像素画特征），否则用 Lanczos3（插画特征）；
/// - **小尺寸例外**：试算判"插画"但源图长边 ≤ [`SMALL_DITHER_EDGE`] 且 AI 未接管 →
///   改判像素画（抖动点阵会把 xBRZ 锐度比稀释到阈值之下造成误判，见常量文档）。
/// 返回值第二项 = **是否为像素画**（选了边缘保持算法）——编码分派也用它：
/// 像素画走无损 webp（硬边对有损极敏感），其余按尺寸走默认/有损。
/// 插画档的算法：AI 开关打开 + **已指定且存在的模型** + 该用途未排除 AI + 运行库可用
/// + 源图不过小 → AI 超分；任一条件不满足 → Lanczos3（由 [`upscale_file`] 叠加 USM 补偿锐度）。
///
/// ⚠️ 模型只支持 4x，但 `AiUpscaler` 会把 4x 结果重采样到目标倍率，故倍率不再是门槛
/// （`_rate` 仅为历史签名保留）。
fn smooth_or_ai(
    rgba: &[u8],
    opt: &HdOptions,
    _rate: u32,
    sub: &str,
    w: u32,
    h: u32,
) -> &'static dyn Upscaler {
    // 细线描边类：低通插值会把透明区的抠图残留色混进边缘（实测一圈青色彩边，
    // 深色背景上尤其扎眼）⇒ 直接用**边缘保持算法**（xBRZ），而不是 Lanczos3。
    if ai_excluded(sub) {
        return opt.upscaler();
    }
    // 非 AI 档 + 源图带透明区：同样改用边缘保持算法（彩边根因，见 `has_transparency`）
    if !opt.ai && has_transparency(rgba, w, h) {
        return opt.upscaler();
    }
    // **AI 优先**：开关打开 + **已指定存在的模型** + 运行库可用 + 用途未排除 + 源图不过小。
    // 不再要求"倍率恰为 4" —— `AiUpscaler` 已支持任意倍率（4x 后重采样到目标尺寸）。
    // ⚠️ `ai_model` 缺省 = AI 不生效（**无默认模型**，见 `vcmi::config::AiCfg` 的四重合取）。
    if opt.ai
        && opt.ai_model.is_some()
        && !ai_excluded(sub)
        && crate::infra::ai::is_available()
        && w.max(h) >= AI_MIN_SOURCE_EDGE
    {
        if let Some(a) = crate::infra::upscale::upscaler_by_id("ai") {
            return a;
        }
    }
    smooth_upscaler()
}

/// 该**素材用途**是否排除 AI 超分。
///
/// 依据 `hd_compare` 的逐像素对照（`target/hd-compare/*/_accept*.png`）：
/// - `townborder`（城镇轮廓描边）：1px 细线 + 大面积透明，AI 会把线条**变淡偏灰**、
///   抹掉小的黄褐色装饰段、位置略偏 ⇒ 传统算法（xBRZ / Lanczos3+USM）明显更好，排除；
/// - 小图标类（`herospec` / `artifact` / `spell` … 目标框 ≤56px）**天然到不了这里**：
///   它们算出的目标倍率是 2x，已被 `rate == 4` 挡住；
/// - 像素画也不会走本函数（`pick_upscaler` 先按像素画判定返回 xBRZ）。
///
/// 新增排除项前请先补一份该用途的对照实验（`HD_COMPARE=<样本>` 定向跑法很快）。
fn ai_excluded(sub: &str) -> bool {
    matches!(sub, "townborder")
}

/// 试算判为"插画 / 连续纹理"后的**最终分派**：
///
/// - AI 真正接管（[`smooth_or_ai`] 返回 ai）→ AI；
/// - AI 未接管（传统档 Lanczos3，或透明区回退 xBRZ）且源图长边 ≤ [`SMALL_DITHER_EDGE`]
///   → **改判像素画**（xBRZ + 无损编码 + 不叠 USM）—— 抖动点阵在传统插值下必然被磨成
///   糊斑（`hd_compare` 42 样本 + 肉眼实测），xBRZ 可辨识度显著更高；USM 0.8 叠在 xBRZ
///   上对人脸过重（黑边变粗），故按像素画对待。
///
/// 两处调用：试算"插画"分支与 AI"连续纹理"接管分支 —— **后者正是抖动头像被误判的
/// 主路径**：试算 ratio ≥ 1.10 本已判对（xBRZ 更锐），却在 AI 模式被交给 AI，
/// AI 缺席（未配模型 / 运行库不可用）时回落 Lanczos3 ⇒ 糊。
fn dispatch_illustration(
    rgba: &[u8],
    opt: &HdOptions,
    rate: u32,
    sub: &str,
    w: u32,
    h: u32,
) -> (&'static dyn Upscaler, bool) {
    let u = smooth_or_ai(rgba, opt, rate, sub, w, h);
    if u.id() != "ai" && w.max(h) <= SMALL_DITHER_EDGE {
        return (opt.upscaler(), true);
    }
    (u, false)
}

pub(crate) fn pick_upscaler(
    rgba: &[u8],
    w: u32,
    h: u32,
    opt: &HdOptions,
    rate: u32,
    sub: &str,
) -> (&'static dyn Upscaler, bool) {
    let pixel_algo = opt.upscaler();
    if w.max(h) > AUTO_PICK_MAX_EDGE {
        return (smooth_or_ai(rgba, opt, rate, sub, w, h), false);
    }
    // 大片纯色（占比 ≤10%）→ 明显的像素画/纯色图，直接用边缘保持算法：
    // 这类图（建筑图标 6%、描边 0%、法术图标 10%）试算的锐度优势可能只有零点几个百分点，
    // 但 xBRZ 不会像 Lanczos3 那样在硬边周围产生光晕 —— 观感差异大于数值差异。
    // 插画占比在 27%+，不会被这条规则误伤。
    if color_ratio_pct(rgba, w, h) <= PIXEL_ART_FAST_PCT {
        return (pixel_algo, true);
    }
    // 试算用 2x：算法特性在 2x 就已体现，且比按目标倍率试算便宜得多
    let a = upscale_with(pixel_algo, rgba, w, h, 2);
    let b = upscale_with(smooth_upscaler(), rgba, w, h, 2);
    match (a, b) {
        (Some((pa, pw, ph)), Some((pb, bw, bh))) => {
            let ra = grad_energy(&pa, pw, ph);
            let rb = grad_energy(&pb, bw, bh);
            let ratio = if rb > 0.0 { ra / rb } else { f64::MAX };
            if rb > 0.0 && ra >= rb * AUTO_PICK_RATIO {
                // 锐度比落在 [1.10, 2.0) ⇒ **连续纹理**（草地 / 石墙）而非方块像素画：
                // AI 模式下交给 AI（实测纹理更细腻、结构不变形）；强像素画才坚持 xBRZ。
                // （曾用"去重颜色占比"做同样的事，实测方向相反，见 `rich_texture_pct`。）
                if opt.ai
                    && !ai_excluded(sub)
                    && w.max(h) >= AI_MIN_SOURCE_EDGE
                    && ratio < PIXEL_STRONG_RATIO
                    && crate::infra::ai::is_available()
                {
                    crate::infra::metrics::count("hd.ai.texture_override", 1);
                    dispatch_illustration(rgba, opt, rate, sub, w, h)
                } else {
                    (pixel_algo, true)
                }
            } else {
                dispatch_illustration(rgba, opt, rate, sub, w, h)
            }
        }
        (Some(_), None) => (pixel_algo, true),
        (None, Some(_)) => (smooth_or_ai(rgba, opt, rate, sub, w, h), false),
        (None, None) => (smooth_or_ai(rgba, opt, rate, sub, w, h), false),
    }
}

/// 「非像素画」使用的平滑插值算法：Lanczos3（保锐、无 xBRZ 的涂抹）。
///
/// 不暴露为配置项：这是**技术判断**而非用户偏好。未来接入 AI 超分时，
/// 应替换这里（把 `upscaler_ai` 作为插画类首选，本函数作回退）。
fn smooth_upscaler() -> &'static dyn Upscaler {
    crate::infra::upscale::upscaler_by_id("lanczos3").unwrap_or_else(default_upscaler)
}

/// 逐图算"该生成几倍"。
///
/// `rate = clamp(max(⌈目标W×phys/源W⌉, ⌈目标H×phys/源H⌉), 1, cap)`
///
/// - 两个维度取大：目标框与源图比例一致时两者相等，不一致时以保证**较缺的那一维**为准；
/// - 结果为 1 → 源图已经够大（**不生成**，这是体积优化的大头）；
/// - 未知用途 → 直接用 `cap`（全局上限）。
pub(crate) fn target_rate(src_w: u32, src_h: u32, sub: &str, phys: f64, cap: u32) -> u32 {
    let Some((tw, th)) = hd_target_box(sub) else {
        return cap;
    };
    if src_w == 0 || src_h == 0 {
        return cap;
    }
    let need_w = tw as f64 * phys;
    let need_h = th as f64 * phys;
    let rw = (need_w / src_w as f64).ceil();
    let rh = (need_h / src_h as f64).ceil();
    let r = rw.max(rh).max(1.0);
    if !r.is_finite() {
        return cap;
    }
    (r as u32).clamp(1, cap)
}

/// 单 mod / 单批次的生成统计
#[derive(Default, Clone, Copy, Debug)]
pub struct HdStats {
    /// 新生成
    pub generated: usize,
    /// 已存在（幂等跳过）
    pub skipped: usize,
    /// 解码 / 放大 / 编码失败
    pub failed: usize,
    /// 清理掉的**其它倍率**残留（换显示器后旧倍率不再被引用，删除回收磁盘）
    pub pruned: usize,
}

impl HdStats {
    pub fn is_empty(&self) -> bool {
        self.generated == 0 && self.skipped == 0 && self.failed == 0 && self.pruned == 0
    }
}

/// 从文件 stem 解析高清变体倍率：`<name>@<n>x` → `Some(n)`，否则 `None`。
///
/// 严格要求"`@` + 纯数字 + `x`"结尾 —— 避免把**名字里本来带 `@`** 的源素材误判成变体
/// （误判的后果是把它当旧倍率残留删掉）。
pub(crate) fn variant_rate(stem: &str) -> Option<u32> {
    let at = stem.rfind('@')?;
    let num = stem[at + 1..].strip_suffix('x')?;
    if num.is_empty() || !num.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    num.parse().ok()
}

/// 为单个 mod 的 `images/**` 生成高清变体（**幂等**：该图应得倍率的 `@Nx` 已存在则跳过）。
///
/// **按用途分级**：倍率不再全局统一，而是逐图由 [`target_rate`] 按 [`hd_target_box`] 算出：
/// 小图（兵种/英雄）拿高倍率，大图（城镇幕布）只拿够用的倍率，源图已够大的直接不生成。
///
/// 分两遍：第一遍只读**文件头**拿源图尺寸（`image_dimensions`，不解码全图），
/// 于是"跳过"路径依然廉价；第二遍才做真正的解码 + 放大 + 编码。
///
/// 失败（坏图 / 加密未解出的占位）只计数不中断——高清是增强项，绝不因此让批次失败。
pub fn generate_for_mod(cache_root: &Path, mod_id: &str, opt: &HdOptions) -> HdStats {
    let mut stats = HdStats::default();
    if opt.rate < 2 {
        return stats;
    }
    let dir = cache_root.join(sanitize_id(mod_id)).join("images");
    if !dir.is_dir() {
        return stats;
    }
    // 方案版本不匹配 → 先清掉全部旧变体（进程内只核对一次）
    ensure_hd_scheme(cache_root);

    let mut files = Vec::new();
    collect_webp(&dir, &mut files);

    // ── 第一遍：源图算"应得倍率"；变体登记待判 ──
    let mut want: HashMap<PathBuf, u32> = HashMap::new();
    // (变体路径, 其源图路径, 变体自身倍率)
    let mut variants: Vec<(PathBuf, PathBuf, u32)> = Vec::new();
    for f in &files {
        let Some(stem) = f.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if let Some(vr) = variant_rate(stem) {
            // 变体名由我们按 `<源stem>@<n>x.webp` 生成，故去掉后缀即源文件名（大小写一致）
            if let Some(at) = stem.rfind('@') {
                let src = f.with_file_name(format!("{}.webp", &stem[..at]));
                variants.push((f.clone(), src, vr));
            }
            continue;
        }
        let sub = f
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let Ok((w, h)) = image::image_dimensions(f) else {
            stats.failed += 1;
            crate::infra::metrics::count("hd.failed", 1);
            continue;
        };
        want.insert(f.clone(), target_rate(w, h, sub, opt.phys, opt.rate));
    }

    // HD 登记表：素材阶段记录了"哪些图有官方真高清源"（`<mod>/hd-map.json`，见 `modcache::hd_map`），
    // 变体优先由 HD 图生成。未装 hd-edition 时表为空 → 全链路行为与从前一致。
    let hd_map = crate::modcache::hd_map::load(cache_root, mod_id);

    // ── 第二遍：源图 → 生成（应得倍率 ≥2 才做）──
    for f in &files {
        let Some(&rate) = want.get(f) else { continue };
        if rate < 2 {
            continue; // 源图已经够大：不生成（历史变体在下一段被清理）
        }
        let Some(stem) = f.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let dest = f.with_file_name(format!("{stem}@{rate}x.webp"));
        if dest.is_file() {
            stats.skipped += 1;
            continue;
        }
        // HD 源查询键与素材登记口径一致：`<sub>/<stem>`（见 `assets::wiki_image_rel`）
        let sub = f
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let hd_name = hd_map.get(&format!("{sub}/{stem}")).map(String::as_str);
        if upscale_file(f, &dest, opt, rate, hd_name, sub) {
            stats.generated += 1;
            crate::infra::metrics::count("hd.generated", 1);
        } else {
            stats.failed += 1;
            crate::infra::metrics::count("hd.failed", 1);
        }
    }

    // ── 变体 → 与源图应得倍率一致则保留，否则清理 ──
    // 覆盖三种残留：换显示器后的旧倍率、源图已够大（不再需要变体）、孤儿（源图已删）。
    for (path, src, vr) in &variants {
        let need = want.get(src).copied().unwrap_or(1);
        if need >= 2 && need == *vr {
            stats.skipped += 1;
        } else if fs::remove_file(path).is_ok() {
            stats.pruned += 1;
            crate::infra::metrics::count("hd.pruned", 1);
        }
    }
    stats
}

/// 递归收集 `images/**/*.webp`
fn collect_webp(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_webp(&p, out);
        } else if p.extension().and_then(|s| s.to_str()) == Some("webp") {
            out.push(p);
        }
    }
}

/// 单文件变体生成：**定源**（HD 优先）→ 整数倍放大 → 编码 → 落盘。
///
/// ## 源（2026-09-23 拍板）
///
/// - `hd_name` 非空且 HD 素材可用 → 用官方真高清图作源：`hd_edition::lookup_for` 按需求档位
///   选源，三种落法：
///   ① 源档 == 需求 → 原图直出（零插值）；
///   ② 需求是源档的整数倍 → 整数放大（xBRZ / AI / Lanczos3+USM）；
///   ③ 否则（如 x3 源 → 4x 需求）→ **单步重采样到精确 `need × 1x`**（Lanczos3，预乘 alpha），
///      避免"先用低档放大到整数倍"细节不足（用户实测 `@4x` 糊）；
/// - 否则读 1x 基准图（阶段二产物）放大 `rate` 倍 —— 与从前行为一致。
///
/// 算法由 [`pick_upscaler`] 试算决定（像素画 → xBRZ；插画/写实 → Lanczos3 / AI）。
///
/// 编码分派：
/// - **像素画 / 小图** → 无损（`encode_rgba_auto`）—— 有损会在硬边产生振铃与色块，
///   而这类图放大后体积本就不大（58×64 → 4x 无损仅 12 KB）；
/// - **大图**（>256px）→ 有损（[`HD_QUALITY`]）—— 写实渐变图有损无碍，体积可降 5~8 倍
///   （1600×748 幕布：无损 1855 KB → 有损 234 KB）。
/// 放大前预处理：**低通插值专用** —— 透明区先填成邻近实心色。
///
/// DEF 素材 `alpha=0` 处的 RGB 是抠图残留（青蓝底框）。低通插值（Lanczos3 族）会把残留色
/// 与线条色混合，在半透明边缘留下一圈**青色彩边**（深色背景——幕布、图标底——上尤其扎眼）。
/// 先把透明区填成邻近实心色即可根治；AI 路径内部已 `dilate_colors`，像素画档走 xBRZ
/// （边缘保持，只取边缘一侧颜色）都不需要。
///
/// `hd_compare` 的实验列也调用本函数 —— 否则那一列会比生产产物多一圈彩边，失去对照意义。
pub(crate) fn preprocess_for_interpolation(rgba: &[u8], w: u32, h: u32, apply: bool) -> Vec<u8> {
    if !apply {
        return rgba.to_vec();
    }
    let mut pre = rgba.to_vec();
    crate::infra::ai::dilate_colors(&mut pre, w, h, crate::infra::ai::DILATE_ROUNDS);
    pre
}

/// 源图是否含**非不透明**像素（alpha < 255）。
///
/// 含透明区的素材走低通插值（Lanczos3 族）会把抠图残留色混进半透明边缘 ⇒ 青色彩边，
/// 实测 `20_lanczos3` 有 48 个青色像素、`31_lanczos3_usm080` 有 51 个，而 xBRZ 与 AI 都是 0。
/// 故非 AI 档遇到这类素材直接改用**边缘保持算法**（见 `pick_upscaler`）。
fn has_transparency(rgba: &[u8], w: u32, h: u32) -> bool {
    let n = (w as usize) * (h as usize);
    if n == 0 || rgba.len() < n * 4 {
        return false;
    }
    (0..n).any(|i| rgba[i * 4 + 3] < 255)
}

/// 半透明带去污染（defringe）：`16 ≤ alpha < 255` 的像素，RGB 替换为邻接**全实心**
/// （`alpha = 255`）像素的颜色，逐圈扩散至多 `rounds` 圈。
///
/// DEF 抗锯齿边缘的 RGB 是"线条色与青蓝底框的原始混合"——预乘插值只能阻止**全透明区**
/// 的残留色渗入，管不到半透明带自身的污染。必须把它的 RGB 净化成线条色，合成时才得到
/// 正确的"线条色 + alpha 渐变"，否则就是一圈青彩边（实测 `townborder`）。alpha=0 由
/// [`preprocess_for_interpolation`]（dilate）负责，这里跳过。
fn defringe(rgba: &mut [u8], w: u32, h: u32, rounds: usize) {
    let (wi, hi) = (w as usize, h as usize);
    if wi == 0 || hi == 0 || rgba.len() < wi * hi * 4 {
        return;
    }
    // `>= 128` 而非 `== 255`：DEF 抗锯齿带里未必存在纯实心邻居，用"近似实心"作源
    // 才能覆盖到每一层半透明像素（实测纯实心定义会漏掉 townborder 的青色边缘）。
    let mut solid: Vec<bool> = rgba.chunks_exact(4).map(|p| p[3] >= 128).collect();
    for _ in 0..rounds {
        let snapshot = rgba.to_vec();
        let mut changed = false;
        for y in 0..hi {
            for x in 0..wi {
                let i = y * wi + x;
                if solid[i] || rgba[i * 4 + 3] == 0 {
                    continue;
                }
                let (mut r, mut g, mut b, mut c) = (0u32, 0u32, 0u32, 0u32);
                for (dy, dx) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let (ny, nx) = (y as i32 + dy, x as i32 + dx);
                    if ny < 0 || nx < 0 || ny >= hi as i32 || nx >= wi as i32 {
                        continue;
                    }
                    let j = ny as usize * wi + nx as usize;
                    if solid[j] {
                        r += snapshot[j * 4] as u32;
                        g += snapshot[j * 4 + 1] as u32;
                        b += snapshot[j * 4 + 2] as u32;
                        c += 1;
                    }
                }
                if c > 0 {
                    rgba[i * 4] = (r / c) as u8;
                    rgba[i * 4 + 1] = (g / c) as u8;
                    rgba[i * 4 + 2] = (b / c) as u8;
                    solid[i] = true; // 净化后可作下一圈的源
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
}

/// 预乘 alpha（`RGB ×= A/255`）：低通插值前必须做 —— 非预乘 RGBA 直接插值会把
/// 透明区残留色混进半透明边缘（实测青色彩边的根因）。
fn premultiply_rgba(rgba: &mut [u8]) {
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3] as u32;
        for c in 0..3 {
            px[c] = ((px[c] as u32 * a + 127) / 255) as u8;
        }
    }
}

/// 反预乘（`RGB ÷= A/255`；alpha=0 与 255 原样保留）。8.8 定点除避免精度损失。
fn unpremultiply_rgba(rgba: &mut [u8]) {
    for px in rgba.chunks_exact_mut(4) {
        let a = px[3];
        if a > 0 && a < 255 {
            let inv = (255u32 << 8) / a as u32;
            for c in 0..3 {
                px[c] = (((px[c] as u32 * inv) >> 8).min(255)) as u8;
            }
        }
    }
}

/// 低通插值的**正确姿势**：预乘 → 插值 → 反预乘 → 全透明区 RGB 回填源色。
///
/// 非预乘 RGBA 直接插值会把 `alpha=0` 处的残留色混进半透明边缘（青色彩边）；
/// 预乘后透明区对 RGB 的贡献归零，边缘色只来自实心像素，与合成语义一致。
/// 反预乘后 alpha≈0 的像素 RGB 归零，为避免"黑底"，回填 dilate 后的源色
/// （入参应是已过 [`preprocess_for_interpolation`] 的图）。**仅用于 Lanczos3 族**：
/// xBRZ（边缘保持）与 AI（内部已 dilate）都不需要。
pub(crate) fn upscale_lowpass(
    u: &'static dyn Upscaler,
    rgba: &[u8],
    w: u32,
    h: u32,
    factor: u32,
) -> Option<(Vec<u8>, u32, u32)> {
    let n = (w as usize) * (h as usize);
    if w == 0 || h == 0 || rgba.len() < n * 4 {
        return None;
    }
    let mut src = rgba[..n * 4].to_vec();
    defringe(&mut src, w, h, 3);
    premultiply_rgba(&mut src);
    let (mut out, ow, oh) = crate::infra::upscale::upscale_with(u, &src, w, h, factor)?;
    unpremultiply_rgba(&mut out);
    // 低 alpha 带回填源色：阈值取 **96** 而非"全透明"—— 反预乘会把 alpha≈30 处的插值
    // 噪声放大近 9 倍，微量残留色在那里变成可见的暗青彩边（实测 `townborder` 48 个青色
    // 像素全在这一带）。DEF 素材的有效半透明边缘只有 1px 源像素宽，盖不到 96 这么深。
    // 回填源色（已 dilate + defringe）同时规避"黑底"。
    let f = factor.max(1);
    let total = (ow as usize) * (oh as usize);
    for i in 0..total {
        if out[i * 4 + 3] < 96 {
            let x = (i % ow as usize) as u32;
            let y = (i / ow as usize) as u32;
            let sx = ((x / f) as usize).min(w as usize - 1);
            let sy = ((y / f) as usize).min(h as usize - 1);
            let j = (sy * w as usize + sx) * 4;
            out[i * 4] = src[j];
            out[i * 4 + 1] = src[j + 1];
            out[i * 4 + 2] = src[j + 2];
        }
    }
    Some((out, ow, oh))
}

/// 单步重采样到任意目标尺寸（Lanczos3 + 预乘 alpha）。
///
/// 用途：HD 源档与需求**不是整数倍**时（x3 源 → 4x 需求 = 1.33×）一次到位，
/// 而不是"先用低档放大到整数倍"再让浏览器下采样 —— 后者细节更少、观感更糊。
/// alpha 处理与 [`upscale_lowpass`] 同口径：defringe → 预乘插值 → 反预乘 →
/// 低 alpha 带回填源色（源色取自**未预乘**的入参，避免回填出偏暗的边）。
fn resample_lowpass(
    rgba: &[u8],
    w: u32,
    h: u32,
    tw: u32,
    th: u32,
) -> Option<(Vec<u8>, u32, u32)> {
    let n = (w as usize) * (h as usize);
    if w == 0 || h == 0 || tw == 0 || th == 0 || rgba.len() < n * 4 {
        return None;
    }
    let mut pre = rgba[..n * 4].to_vec();
    defringe(&mut pre, w, h, 3);
    premultiply_rgba(&mut pre);
    let sheet = image::RgbaImage::from_raw(w, h, pre)?;
    let mut out = image::imageops::resize(
        &sheet,
        tw,
        th,
        image::imageops::FilterType::Lanczos3,
    )
    .into_raw();
    unpremultiply_rgba(&mut out);
    for y in 0..th as usize {
        for x in 0..tw as usize {
            let i = y * tw as usize + x;
            if out[i * 4 + 3] < 96 {
                let sx = (x * w as usize / tw as usize).min(w as usize - 1);
                let sy = (y * h as usize / th as usize).min(h as usize - 1);
                let j = (sy * w as usize + sx) * 4;
                out[i * 4] = rgba[j];
                out[i * 4 + 1] = rgba[j + 1];
                out[i * 4 + 2] = rgba[j + 2];
            }
        }
    }
    Some((out, tw, th))
}

pub(crate) fn upscale_file(
    src: &Path,
    dest: &Path,
    opt: &HdOptions,
    rate: u32,
    hd_name: Option<&str>,
    sub: &str,
) -> bool {
    // 1) 定源与放大倍数：HD 真高清优先（命中即计埋点，便于与"1x 放大"区分统计）
    let need = rate.max(1);
    let hd_src = hd_name.and_then(|n| crate::vcmi::hd_edition::lookup_for(n, need));
    let hd_hit = hd_src.is_some();
    let (img, src_scale) = match hd_src {
        Some((img, h, _k)) => {
            crate::infra::metrics::count("hd_src.variant", 1);
            (img, h.max(1))
        }
        None => {
            let Ok(bytes) = fs::read(src) else {
                return false;
            };
            let Ok(decoded) = image::load_from_memory(&bytes) else {
                return false;
            };
            (decoded.to_rgba8(), 1u32)
        }
    };
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return false;
    }
    // HD 源档与需求不是整数倍（如 x3 源 → 4x 需求）：单步重采样到精确需求。
    // 旧规则此时选 x2 档再放大 2×（`h*k` 恰等于需求），但 x2 原图细节不足，
    // 2× 插值后明显比"x3 重采样到 4x"糊（用户实测），且重采样不增加任何体积。
    let exact = hd_hit && src_scale > 1 && need % src_scale != 0;
    let k = if hd_hit {
        need.div_ceil(src_scale).max(1)
    } else {
        need
    };

    // 2) 放大与编码分派；`k == 1`（HD 原图直出）跳过插值，只判编码
    let (rgba, out_w, out_h, pixel_like) = if exact {
        // HD 源 → 精确需求尺寸（单步 Lanczos3，预乘 alpha；不叠 AI/xBRZ）
        let tw = ((w as u64 * need as u64 + src_scale as u64 / 2) / src_scale as u64).max(1) as u32;
        let th = ((h as u64 * need as u64 + src_scale as u64 / 2) / src_scale as u64).max(1) as u32;
        let pixel_like = color_ratio_pct(img.as_raw(), w, h) <= PIXEL_ART_FAST_PCT;
        let buf = preprocess_for_interpolation(img.as_raw(), w, h, !pixel_like);
        let Some((mut out, ow, oh)) = resample_lowpass(&buf, w, h, tw, th) else {
            return false;
        };
        crate::infra::metrics::count("hd_src.resample", 1);
        if !pixel_like && opt.usm > 0.0 && !ai_excluded(sub) {
            crate::infra::metrics::count("hd.usm", 1);
            out = crate::infra::sharpen::usm(
                &out,
                ow,
                oh,
                crate::infra::sharpen::DEFAULT_SIGMA,
                opt.usm,
            );
        }
        (out, ow, oh, pixel_like)
    } else if k <= 1 {
        let pixel_like = color_ratio_pct(img.as_raw(), w, h) <= PIXEL_ART_FAST_PCT;
        (img.into_raw(), w, h, pixel_like)
    } else {
        let (upscaler, pixel_like) = pick_upscaler(img.as_raw(), w, h, opt, k, sub);
        // 放大前预处理（**仅非像素画档**）：DEF 素材 alpha=0 处的 RGB 是抠图残留（青蓝底框），
        // 低通插值（Lanczos3 族）会把残留色混进半透明边缘 ⇒ 一圈青色彩边，深色背景上极扎眼。
        // 先把透明区填成邻近实心色即可消除（与 AI 路径的 `dilate_colors` 同款）；
        // 像素画档不需要 —— xBRZ 是边缘保持，只取边缘一侧颜色，本就不会混色。
        let src_rgba = preprocess_for_interpolation(img.as_raw(), w, h, !pixel_like);
        // 低通插值（Lanczos3 族）必须走预乘插值（见 `upscale_lowpass`）；xBRZ / AI 直调
        let run_upscaler = |u: &'static dyn Upscaler, rgba: &[u8]| -> Option<(Vec<u8>, u32, u32)> {
            if u.id() == "lanczos3" {
                upscale_lowpass(u, rgba, w, h, k)
            } else {
                upscale_with(u, rgba, w, h, k)
            }
        };
        // AI 是增强项：失败（模型不可用 / 推理异常）时**静默回退** Lanczos3，绝不因此丢图
        let mut used_ai = upscaler.id() == "ai";
        let Some((up, uw, uh)) = run_upscaler(upscaler, &src_rgba).or_else(|| {
            if used_ai {
                crate::infra::metrics::count("hd.ai.fallback", 1);
                used_ai = false;
                run_upscaler(smooth_upscaler(), &src_rgba)
            } else {
                None
            }
        }) else {
            return false;
        };
        // 3) 锐度补偿（**分派的一部分**）：传统插值放大后偏软，USM 拿回锐度且不引入生成伪影。
        //    条件 = 非像素画（硬边会产生光晕）+ 强度非 0 + **最终不是 AI 产出** —— 后一条同时
        //    覆盖"AI 不可用/倍率不符时选了 Lanczos3"与"选了 AI 但失败回退"两种情形，
        //    即 AI 缺席的插画档统一由 `Lanczos3 + USM` 顶替（见 `hd_compare` 的对照结论）。
        //    强度来自 `launcher.json` 的 `hd.usm`（0 = 关闭）。
        // 描边类（`ai_excluded`）也不叠：USM 会把插值彩边一并锐化，反而放大缺陷
        let wants_usm = !pixel_like && !used_ai && opt.usm > 0.0 && !ai_excluded(sub);
        let (up, uw, uh) = if wants_usm {
            crate::infra::metrics::count("hd.usm", 1);
            (
                crate::infra::sharpen::usm(
                    &up,
                    uw,
                    uh,
                    crate::infra::sharpen::DEFAULT_SIGMA,
                    opt.usm,
                ),
                uw,
                uh,
            )
        } else {
            (up, uw, uh)
        };
        // 4) AI 混合（`launcher.json` 的 `hd.ai.mix`，缺省 0.75）：`α×AI + (1−α)×传统`。
        //    混合要一份传统结果作基，故此时传统档会**再跑一次**（只发生在 AI 命中的 4x 插画档，
        //    代价可接受）；`α ≥ 1` 时整段跳过，纯 AI 不白算。
        let (up, uw, uh) = if used_ai && opt.ai_mix < 1.0 {
            let base = run_upscaler(smooth_upscaler(), &src_rgba).map(|(b, _, _)| {
                if !pixel_like && opt.usm > 0.0 {
                    crate::infra::sharpen::usm(
                        &b,
                        uw,
                        uh,
                        crate::infra::sharpen::DEFAULT_SIGMA,
                        opt.usm,
                    )
                } else {
                    b
                }
            });
            match base {
                Some(b) => {
                    crate::infra::metrics::count("hd.ai.mix", 1);
                    // AI 结果先**色彩对齐**到传统基（消除生成式偏色 / 对比度漂移，6B 尤甚），
                    // 再按权重混合 —— 还原度提升且不损清晰度（见 `infra::sharpen::color_match`）。
                    crate::infra::metrics::count("hd.ai.colormatch", 1);
                    let aligned = crate::infra::sharpen::color_match(&up, &b);
                    (
                        crate::infra::sharpen::mix_rgb(&aligned, &b, opt.ai_mix),
                        uw,
                        uh,
                    )
                }
                None => (up, uw, uh),
            }
        } else {
            (up, uw, uh)
        };
        (up, uw, uh, pixel_like)
    };

    // - **像素画一律无损** —— 硬边 + 纯色块对有损极敏感（振铃/色块直接毁观感），
    //   而放大后的像素画体积本就不大（58×64 → 4x 约 12 KB）；
    // - 小图（≤256）交给 `encode_rgba_auto` 的默认策略（有损 q90）；
    // - 大图走有损 q85（写实渐变有损无碍，1600×748 幕布从 1855 KB 降到 234 KB）。
    let enc = if pixel_like {
        encode_webp_lossless(&rgba, out_w, out_h)
    } else if out_w.max(out_h) <= crate::infra::image::LOSSY_MAX_EDGE {
        encode_rgba_auto(&rgba, out_w, out_h)
    } else {
        encode_webp_lossy_q(&rgba, out_w, out_h, HD_QUALITY)
    };
    let Some(enc) = enc else {
        return false;
    };
    if let Some(parent) = dest.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(dest, &enc).is_ok()
}

#[cfg(test)]
mod tests {
    use super::{hd_target_box, target_rate};

    /// 回归（2026-10-20）：`pskil42` 的 1x 源帧是 42×42，而 UI 显示 32 CSS px。
    ///
    /// 1080p（phys=1.5）需求 48px > 42px → 必须出 2x 变体（有 HD 源时即 HD 直出）。
    /// 此前目标框错写 28×28：28×1.5 = 42 恰等于源尺寸 → `target_rate` 恒为 1 →
    /// 该目录**一个 `@Nx` 都不生成**，hd-map 里已登记的 `PSKIL42A/D/P/S` 永远用不上。
    #[test]
    fn pskil42_gets_variant_at_1080p() {
        assert_eq!(hd_target_box("pskil42"), Some((32, 32)));
        assert_eq!(target_rate(42, 42, "pskil42", 1.5, 4), 2);
        // 低 DPI（phys=1）下 32px 需求仍小于 42px 源图 → 不生成，避免白占磁盘
        assert_eq!(target_rate(42, 42, "pskil42", 1.0, 4), 1);
    }
}
