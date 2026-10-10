//! **放大算法取证**：对缓存里全部 `@Nx` 变体，用 `hd::pick_upscaler` 的**生产判定**复算
//! 「哪张图由谁放大」——输出「档位 × 算法」矩阵，以及各素材类在 `@2x` 档的分派。
//!
//! ## 为什么需要
//! 缓存文件**不带算法标记**：`@4x.webp` 既可能是 Real-ESRGAN 产物，也可能是 xBRZ 产物。
//! 三条判据合起来才能定性：
//! 1. **档位**：AI 只支持 4x（`AiUpscaler::max_factor`），`smooth_or_ai` 仅在目标倍率恰为 4 时选它
//!    ⇒ `@2x` / `@3x` 不可能是 AI；
//! 2. **生成时间**：必须晚于日志里 `AI 组件已加载，Real-ESRGAN 超分可用` 那一刻，
//!    否则当时 AI 不可用、已回退 Lanczos3（`infra::ai::upscale` 失败即返回 None）；
//! 3. **类别**：像素画永远走 xBRZ（实测 AI 会把像素风平滑成噪点），插画走 AI（rate==4）或 Lanczos3。
//! 本测试负责第 1、3 条；第 2 条要自己对日志（见上）。
//!
//! ## 用法
//! ```bash
//! HD_BENCH_MODS="$PWD/../references/Heroes3-VCMI/.vcmi-fv-launcher/mods" \
//!   cargo test --lib ai_audit_upscaler_split -- --ignored --nocapture
//! ```

use super::sample_root;
use crate::modcache::hd::{pick_upscaler, HdOptions};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// `xxx@4x.webp` → `(4, "xxx")`
fn parse_variant(name: &str) -> Option<(u32, String)> {
    let stem = name.strip_suffix(".webp")?;
    let idx = stem.rfind('@')?;
    let (base, suffix) = stem.split_at(idx);
    let n = suffix.get(1..)?.strip_suffix('x')?.parse::<u32>().ok()?;
    if n < 2 {
        return None;
    }
    Some((n, base.to_string()))
}

/// 递归收集 `@Nx.webp` 变体
fn collect_variants(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(ents) = std::fs::read_dir(dir) else { return };
    for e in ents.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_variants(&p, out);
        } else if p
            .file_name()
            .and_then(|s| s.to_str())
            .is_some_and(|n| n.contains('@') && n.ends_with(".webp"))
        {
            out.push(p);
        }
    }
}

#[test]
#[ignore]
fn ai_audit_upscaler_split() {
    let root = sample_root();
    if !root.is_dir() {
        println!("⚠️ 缓存目录不存在：{}（用 HD_BENCH_MODS 指定实际缓存）", root.display());
        return;
    }
    // 与生产同口径：ai=true、像素画算法 xbrz（复算只看 opt.upscaler 与 opt.ai，rate 由文件名给出）
    let Some(opt) = HdOptions::new(Some(4), Some(1.5), Some("xbrz".to_string()), Some(true)) else {
        println!("⚠️ HdOptions 构造失败");
        return;
    };

    let mut files = Vec::new();
    collect_variants(&root, &mut files);
    files.sort();
    println!("扫描到 @Nx 变体 {} 个（根：{}）\n", files.len(), root.display());

    let mut by_rate_algo: BTreeMap<(u32, &'static str), usize> = BTreeMap::new();
    // `@2x` 档按素材类细分（城镇幕布 / 兵种 / 地形…各自用了什么算法）
    let mut x2_by_sub: BTreeMap<(String, &'static str), usize> = BTreeMap::new();
    let mut by_rate_total: BTreeMap<u32, usize> = BTreeMap::new();
    let mut missing_src = 0usize;
    let mut decode_fail = 0usize;

    for f in &files {
        let Some(name) = f.file_name().and_then(|s| s.to_str()) else { continue };
        let Some((rate, base)) = parse_variant(name) else { continue };
        let src = f.with_file_name(format!("{base}.webp"));
        if !src.is_file() {
            missing_src += 1;
            continue;
        }
        let Ok(bytes) = std::fs::read(&src) else {
            decode_fail += 1;
            continue;
        };
        let Ok(img) = image::load_from_memory(&bytes) else {
            decode_fail += 1;
            continue;
        };
        let rgba = img.to_rgba8();
        let (w, h) = (rgba.width(), rgba.height());
        // 用途（sub）参与分派：`townborder` 等被排除 AI 的用途必须按真实值审计，否则统计失真
        let sub = f
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let (u, _pixel) = pick_upscaler(rgba.as_raw(), w, h, &opt, rate, sub);
        *by_rate_algo.entry((rate, u.id())).or_insert(0) += 1;
        *by_rate_total.entry(rate).or_insert(0) += 1;

        if rate == 2 {
            let sub = f
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .unwrap_or("?")
                .to_string();
            *x2_by_sub.entry((sub, u.id())).or_insert(0) += 1;
        }
    }

    println!("=== 档位 × 算法（生产 pick_upscaler 复算）===");
    for (rate, total) in &by_rate_total {
        println!("  @{rate}x  合计 {total}");
        for ((r, algo), n) in &by_rate_algo {
            if r == rate {
                let pct = (*n as f64) * 100.0 / (*total as f64);
                println!("        {algo:<10} {n:>6}  ({pct:.1}%)");
            }
        }
    }
    println!("  源图缺失 {missing_src}，解码失败 {decode_fail}\n");

    println!("=== @2x 各素材类的算法分派 ===");
    for ((sub, algo), n) in &x2_by_sub {
        println!("  {sub:<14} {algo:<10} {n}");
    }
}
