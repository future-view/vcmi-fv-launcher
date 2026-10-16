//! 高清素材（@2x / @3x）生成算法的**选型验证**（确认点 5：效果 + 机器压力）。
//!
//! ```text
//! cargo test -p vcmi-fv-launcher hd_upscale_bench -- --ignored --nocapture
//! ```
//!
//! 做什么：
//! 1. 从真实缓存里按尺寸分桶抽样（原版 `core.castle` 的兵种/英雄/建筑 + 第三方 `HuaXia` 的高清 png）；
//! 2. 对「算法 × 倍率」全组合跑放大 + webp 编码，记录耗时与产物体积；
//! 3. 放大结果落盘到 `target/hd-bench/<算法>-<倍率>x/<桶>/`，**供肉眼 A/B**；
//! 4. 按实测均值外推「全量 25,335 张素材」的单线程总耗时，用于评判对机器的压力。
//!
//! ⚠️ 只测量，**不改任何缓存产物、不改 `cache.json` 字段**。
//!
//! 样本必须覆盖「原版小素材」与「第三方高清素材」两类：前者是本次要解决的对象
//! （58×64 / 150×70 / 800×374），后者用来验证"已足够大的图不应被无意义放大"。

use super::*;
use std::collections::HashMap;
use std::time::Instant;

use crate::infra::image::encode_rgba_auto;
use crate::infra::upscale::{all_upscalers, upscale_with};

/// 每个尺寸桶最多取几张样本（控制 bench 时长）
const SAMPLE_PER_BUCKET: usize = 3;

/// 参与对比的整数倍率（对应 1080p/2K → 2x，4K → 3x）
/// 算法列表来自 `infra::upscale::all_upscalers()`（新增算法登记后自动纳入对比）
const FACTORS: [u32; 2] = [2, 3];

/// 尺寸桶（按最大边长）：对应兵种/英雄 ≤64、建筑 ≤256、幕布与高清图 >256
const BUCKETS: [(u32, &str); 3] = [(64, "tiny-64"), (256, "mid-256"), (u32::MAX, "big")];

/// 全量素材基数：**由 `survey_cache()` 实测**（避免外推时按桶重复计数）。
/// 桶与体积档的对应关系（实测增强版缓存）：
/// `tiny-64` ≈ <10KB 的小图标、`mid-256` ≈ 10–100KB、`big` ≈ >100KB（城镇背景/合成预览）。
struct Survey {
    tiny: u64,
    mid: u64,
    big: u64,
}

impl Survey {
    fn total(&self) -> u64 {
        self.tiny + self.mid + self.big
    }
    fn of(&self, bucket: &str) -> u64 {
        match bucket {
            "tiny-64" => self.tiny,
            "mid-256" => self.mid,
            // `alpha` 是**抽样演示桶**（用于核对透明边缘），不是缓存里的独立分类
            // —— 其素材已计入 tiny，故不重复计数。
            "alpha" => 0,
            _ => self.big,
        }
    }
}

/// 遍历缓存根，按体积档统计 webp 数量（**不解码**，故只判体积不判边长）
fn survey_cache() -> Survey {
    let mut s = Survey {
        tiny: 0,
        mid: 0,
        big: 0,
    };
    let root = sample_root();
    let Ok(mods) = fs::read_dir(&root) else {
        return s;
    };
    for m in mods.flatten() {
        let img_dir = m.path().join("images");
        if !img_dir.is_dir() {
            continue;
        }
        let mut files = Vec::new();
        collect_webp(&img_dir, &mut files);
        for f in files {
            let Ok(meta) = fs::metadata(&f) else { continue };
            let len = meta.len();
            if len < 10 * 1024 {
                s.tiny += 1;
            } else if len < 100 * 1024 {
                s.mid += 1;
            } else {
                s.big += 1;
            }
        }
    }
    s
}

/// 默认并行度（与 `batch.rs` 的空闲档 eco 一致：cpus/8，最低 1）
fn eco_threads() -> u64 {
    std::thread::available_parallelism()
        .map(|n| (n.get() as u64 / 8).max(1))
        .unwrap_or(1)
}

/* ------------------------------ 客观质量指标 ------------------------------ */
// 用途：小图（32×32）与"透明边缘是否发暗"肉眼难判，且 1600×748 的大图产物
// 超出查看工具的上限 —— 故用可复现的数值指标替代肉眼：
//   1. 锐度比  = 放大图的平均相邻亮度差 / 原图（越接近 1 越"不掉锐"，越小越软）；
//   2. 往返 PSNR = 放大图降采样回原尺寸后与原图的 PSNR（dB）—— 高 = 忠实但可能偏软；
//   3. alpha 统计 = 半透明像素数及其平均亮度 vs 不透明像素（**暗边检测**：
//      若半透明像素明显偏暗，说明放大未做 alpha 预乘，会在透明边缘产生暗晕）。

fn luma(p: &[u8]) -> f32 {
    0.299 * p[0] as f32 + 0.587 * p[1] as f32 + 0.114 * p[2] as f32
}

/// 平均水平梯度（锐度代理）——**委托到生产实现**（`hd::grad_energy`），
/// 保证验收看到的数值与 `pick_upscaler` 的判定标准完全同源。
fn grad_energy(rgba: &[u8], w: u32, h: u32) -> f64 {
    crate::modcache::hd::grad_energy(rgba, w, h)
}

/// 放大图 → 降采样回原尺寸 → 与原图比 PSNR（只算 RGB 通道）
fn roundtrip_psnr(up: &[u8], uw: u32, uh: u32, src: &[u8], w: u32, h: u32) -> f64 {
    let Some(img) = image::RgbaImage::from_raw(uw, uh, up.to_vec()) else {
        return f64::NAN;
    };
    let down = image::imageops::resize(&img, w, h, image::imageops::FilterType::Triangle);
    let d = down.as_raw();
    let n = (w as usize) * (h as usize);
    let mut mse = 0.0f64;
    for i in 0..n {
        for c in 0..3 {
            let a = d[i * 4 + c] as f64;
            let b = src[i * 4 + c] as f64;
            mse += (a - b) * (a - b);
        }
    }
    mse /= (n * 3) as f64;
    if mse <= 1e-9 {
        99.0
    } else {
        10.0 * (255.0f64 * 255.0 / mse).log10()
    }
}

/// `(半透明像素数, 半透明平均亮度, 不透明平均亮度)`
fn alpha_stats(rgba: &[u8]) -> (usize, f64, f64) {
    let mut semi = 0usize;
    let mut semi_l = 0.0f64;
    let mut opq_l = 0.0f64;
    let mut opq = 0usize;
    for px in rgba.chunks_exact(4) {
        let a = px[3];
        if a == 0 {
            continue;
        }
        let l = luma(px) as f64;
        if a < 255 {
            semi += 1;
            semi_l += l;
        } else {
            opq += 1;
            opq_l += l;
        }
    }
    (
        semi,
        if semi > 0 {
            semi_l / semi as f64
        } else {
            f64::NAN
        },
        if opq > 0 {
            opq_l / opq as f64
        } else {
            f64::NAN
        },
    )
}

/// 高清生成的**幂等性与产物正确性**（真实素材 → 临时目录，**不污染用户缓存**）。
///
/// 覆盖三件事：
/// 1. 首次调用按样本数生成 `@Nx` 产物，尺寸严格 ×N；
/// 2. 二次调用全部跳过（幂等 —— 这是"增量重跑/中断续跑"正确性的前提）；
/// 3. `rate < 2` 时完全不动作（低分屏不该付任何代价）。
#[test]
#[ignore]
fn hd_generate_idempotent() {
    use crate::modcache::hd::{generate_for_mod, HdOptions};

    let src_dir = sample_root()
        .join("core.castle")
        .join("images")
        .join("creature");
    let mut files = Vec::new();
    collect_webp(&src_dir, &mut files);
    files.sort();
    assert!(files.len() >= 2, "样本不足：{}", src_dir.display());
    let picks: Vec<_> = files.into_iter().take(2).collect();

    let tmp = std::env::temp_dir().join("vcmi_fv_hd_test");
    let _ = fs::remove_dir_all(&tmp);
    let img_dir = tmp.join("hdtest").join("images").join("creature");
    fs::create_dir_all(&img_dir).expect("建临时目录");
    for f in &picks {
        let name = f.file_name().unwrap();
        fs::copy(f, img_dir.join(name)).expect("复制样本");
    }

    let opt = HdOptions {
        rate: 2,
        phys: 1.5, // 1080p：与 `computeHdPhys` 同口径
        upscaler: "xbrz".to_string(),
        ai: false,
        ai_model: None,
        usm: 0.0, // 本用例只验生成/跳过/清理的编排，关锐化保持产物可比
        ai_mix: 1.0,
    };

    let s1 = generate_for_mod(&tmp, "hdtest", &opt);
    assert_eq!(s1.generated, picks.len(), "首次应全部生成：{s1:?}");
    assert_eq!(s1.failed, 0, "不应有失败：{s1:?}");
    assert_eq!(s1.skipped, 0);

    for f in &picks {
        let stem = f.file_stem().unwrap().to_str().unwrap();
        let out = img_dir.join(format!("{stem}@2x.webp"));
        assert!(out.is_file(), "产物应存在：{}", out.display());
        let got = image::load_from_memory(&fs::read(&out).unwrap()).expect("产物应可解码");
        let orig = image::load_from_memory(&fs::read(f).unwrap()).unwrap();
        assert_eq!(got.width(), orig.width() * 2, "宽度应 ×2：{stem}");
        assert_eq!(got.height(), orig.height() * 2, "高度应 ×2：{stem}");
    }

    let s2 = generate_for_mod(&tmp, "hdtest", &opt);
    assert_eq!(s2.generated, 0, "二次不应重复生成：{s2:?}");
    // 注意口径：变体文件自身（`@2x.webp`）也会被计入 skipped，故这里是「≥ 样本数」
    assert!(s2.skipped >= picks.len(), "二次应跳过全部源文件：{s2:?}");
    assert_eq!(s2.pruned, 0, "倍率未变时不应清理：{s2:?}");

    // 换倍率（2x → 4x）：应生成 @4x，并**清理掉旧倍率的 @2x 残留**
    // （用户换显示器 / 倍率公式调整后，旧倍率不再被任何地方引用，留着只占磁盘）
    let opt4 = HdOptions {
        rate: 4,
        phys: 1.5,
        upscaler: "xbrz".to_string(),
        ai: false,
        ai_model: None,
        usm: 0.0,
        ai_mix: 1.0,
    };
    let s4 = generate_for_mod(&tmp, "hdtest", &opt4);
    assert_eq!(s4.generated, picks.len(), "换倍率应生成 @4x：{s4:?}");
    assert_eq!(s4.pruned, picks.len(), "旧 @2x 应被清理：{s4:?}");
    for f in &picks {
        let stem = f.file_stem().unwrap().to_str().unwrap();
        assert!(
            !img_dir.join(format!("{stem}@2x.webp")).exists(),
            "旧倍率残留应已删除：{stem}@2x.webp"
        );
        assert!(
            img_dir.join(format!("{stem}@4x.webp")).is_file(),
            "@4x 应已生成：{stem}@4x.webp"
        );
    }

    let s3 = generate_for_mod(
        &tmp,
        "hdtest",
        &HdOptions {
            rate: 1,
            phys: 1.5,
            upscaler: "xbrz".into(),
            usm: 0.0,
            ai_mix: 1.0,
            ai: false,
            ai_model: None,
        },
    );
    assert!(s3.is_empty(), "rate<2 不应动作：{s3:?}");

    let _ = fs::remove_dir_all(&tmp);
}

/// **AI 超分验证**（Real-ESRGAN ONNX，4x）：对样本跑一次推理，输出与 Lanczos3/xBRZ 的对比。
///
/// 模型 / 运行库在**外挂组件的源码位置** `resources/runtime/ai/`（打包后落
/// `<exe>/.vcmi-fv-launcher/runtime/ai/`，见 `runtime::COMPONENTS`）。
/// 用法：环境变量 `HD_AI_PREVIEW`（逗号分隔，路径相对缓存 mods 根）指定样本，或直接跑（分层抽样）；
/// `HD_BENCH_MODS` 可指定缓存 mods 根（缺省见 [`sample_root`]）。产物落
/// `target/hd-preview-ai/<样本>/`（`ai_4x.png` + `compare.png` + 各算法对照）。
///
/// ⚠️ 仅用于**评估效果与耗时**：本文件自带原型推理实现（送 AI 前自行做透明区膨胀、alpha 用
/// Lanczos3 放大）；**生产口径**在 `infra::ai::upscale`（同一套膨胀 + session 池 + 失败回退）。
///
/// ⚠️ **验证不了 `pin_vc_runtime` 的修复**：测试进程的"应用程序目录"是 `target/debug/deps/`，
/// 那里没有游戏目录里 VCMI 自带的旧 VC++ 运行库（14.29），而复现该故障要求 exe 装在游戏目录。
/// 也就是说：本测试**通过 ≠ 修复生效**（反之失败则确实有问题）。
/// 预期结论：AI 是唯一能**补出细节**的路线（传统插值只能"不失真"，无法创造纹理）。
#[test]
#[ignore]
fn hd_ai_preview() {
    use crate::infra::upscale::{upscale_with, upscaler_by_id};
    use ort::session::Session;
    use ort::value::Value;
    use std::time::Instant;

    let rt_dir = PathBuf::from("resources/runtime/ai");
    let model_path = rt_dir.join("realesr-animevideo-v3-x4.onnx");
    if !model_path.is_file() {
        println!("⚠️ 模型不存在：{}（跳过）", model_path.display());
        return;
    }
    // ⚠️ `load-dynamic` 下必须先 `init_from` 再用任何 ort API（否则 ort 内部 `.expect` → panic）。
    //    生产侧同一入口在 `infra::ai::load_runtime`，那里还会先钉住 VC++ 运行库版本。
    let dll = rt_dir.join("onnxruntime.dll");
    if !dll.is_file() {
        println!("⚠️ 运行库不存在：{}（跳过）", dll.display());
        return;
    }
    match ort::init_from(&dll) {
        Ok(b) => {
            let _ = b.commit();
        }
        Err(e) => {
            println!("⚠️ 运行库加载失败：{e}");
            return;
        }
    }
    let out_root = PathBuf::from("../target/hd-preview-ai");
    let _ = fs::create_dir_all(&out_root);

    // 样本：
    //   ① 环境变量 `HD_AI_PREVIEW`（逗号分隔，路径相对 mods 根）；
    //   ② 缺省 = **分层抽样**：`来源（原版 core 系 / 第三方）× sub × 尺寸档`，
    //      每个组合取最多 [`PER_KEY`] 种源尺寸 —— 保证**原版游戏素材与第三方素材都被覆盖**。
    //
    // "原版"判定：mod 目录名以 `core` 开头（`core` + 9 个 `core.<faction>`，由后端伪模组约定生成）。
    const PER_KEY: usize = 2;
    const MAX_SAMPLES: usize = 96;
    let root = sample_root();
    let env = std::env::var("HD_AI_PREVIEW").unwrap_or_default();
    let mut picks: Vec<(String, PathBuf)> = Vec::new();
    if !env.trim().is_empty() {
        for rel in env.split(',') {
            let p = root.join(rel.trim().replace('/', std::path::MAIN_SEPARATOR_STR));
            if p.is_file() {
                let name = p
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                picks.push((name, p));
            }
        }
    } else {
        use std::collections::BTreeMap;
        // (是否原版, sub) → (源尺寸 → 路径)
        let mut by_key: BTreeMap<(bool, String), BTreeMap<String, PathBuf>> = BTreeMap::new();
        // 两轮遍历：**原版优先**（core 系素材是"模糊"问题最典型的代表）
        'outer: for want_core in [true, false] {
            let Ok(mods) = fs::read_dir(&root) else { break };
            for m in mods.flatten() {
                let mod_name = m.file_name().to_string_lossy().into_owned();
                if mod_name.starts_with("core") != want_core {
                    continue;
                }
                let img = m.path().join("images");
                if !img.is_dir() {
                    continue;
                }
                let mut files = Vec::new();
                collect_webp(&img, &mut files);
                files.sort();
                for f in files {
                    let total: usize = by_key.values().map(|v| v.len()).sum();
                    if total >= MAX_SAMPLES {
                        break 'outer;
                    }
                    let Some(sub) = f
                        .parent()
                        .and_then(|p| p.file_name())
                        .and_then(|s| s.to_str())
                    else {
                        continue;
                    };
                    let e = by_key.entry((want_core, sub.to_string())).or_default();
                    if e.len() >= PER_KEY {
                        continue;
                    }
                    let Ok((w, h)) = image::image_dimensions(&f) else {
                        continue;
                    };
                    e.entry(format!("{w}x{h}")).or_insert(f);
                }
            }
        }
        for ((is_core, sub), sizes) in &by_key {
            for (dims, path) in sizes {
                let tag = if *is_core { "core" } else { "3rd" };
                picks.push((format!("{tag}-{sub}-{dims}"), path.clone()));
            }
        }
    }
    if picks.is_empty() {
        println!("没有可用样本");
        return;
    }

    let t0 = Instant::now();
    let mut session = match Session::builder().and_then(|mut b| b.commit_from_file(&model_path)) {
        Ok(s) => s,
        Err(e) => {
            println!("⚠️ 模型加载失败：{e}");
            return;
        }
    };
    let in_name = session.inputs()[0].name().to_string();
    let out_name = session.outputs()[0].name().to_string();
    println!(
        "\n=== AI 超分验证（Real-ESRGAN AnimeVideo v3 · 4x）===\n模型 {}  加载 {:.0}ms\n输入 `{in_name}` → 输出 `{out_name}`",
        model_path.display(),
        t0.elapsed().as_millis()
    );
    println!(
        "{:<28} {:>10} {:>9} {:>12}  {}",
        "样本（来源-sub-尺寸）", "源尺寸", "推理ms", "AI输出", "对比图"
    );

    // 总览格子用的缩略图（每样本一张，避免把大图整张留在内存里）
    let mut thumbs: Vec<(Vec<u8>, u32, u32)> = Vec::new();
    for (label, path) in &picks {
        let Ok(bytes) = fs::read(path) else { continue };
        let Ok(img) = image::load_from_memory(&bytes) else {
            continue;
        };
        let mut rgba = img.to_rgba8();
        let (w, h) = (rgba.width(), rgba.height());
        let n = (w as usize) * (h as usize);
        // ⚠️ 送 AI 之前先做**透明区颜色膨胀**（否则半透明边缘会露出抠图残留色，见 `dilate_colors`）
        dilate_colors(rgba.as_mut(), w, h, 3);

        // 输入：RGBA → RGB float32 NCHW（0-1 归一化，Real-ESRGAN 口径）。
        // RGB 已在上面做过透明区膨胀填色，这里直接用即可。
        let mut input = vec![0f32; 3 * n];
        for (i, px) in rgba.as_raw().chunks_exact(4).enumerate() {
            input[i] = px[0] as f32 / 255.0;
            input[n + i] = px[1] as f32 / 255.0;
            input[2 * n + i] = px[2] as f32 / 255.0;
        }
        let tensor = match Value::from_array(([1usize, 3, h as usize, w as usize], input)) {
            Ok(t) => t,
            Err(e) => {
                println!("⚠️ 构造输入失败：{e}");
                continue;
            }
        };
        let t1 = Instant::now();
        let outputs = match session.run(ort::inputs![in_name.as_str() => tensor]) {
            Ok(o) => o,
            Err(e) => {
                println!("⚠️ 推理失败（{w}x{h}）：{e}");
                continue;
            }
        };
        let ms = t1.elapsed().as_millis();
        let Ok((shape, data)) = outputs[out_name.as_str()].try_extract_tensor::<f32>() else {
            println!("⚠️ 取输出失败");
            continue;
        };
        let (oh, ow) = (shape[2] as u32, shape[3] as u32);

        // 输出 NCHW f32 → RGBA u8；alpha 用 Lanczos3 单独放大（AI 不处理 alpha）
        let on = (ow as usize) * (oh as usize);
        let mut out_rgba = vec![255u8; on * 4];
        for i in 0..on {
            for c in 0..3 {
                let v = data[c * on + i].clamp(0.0, 1.0) * 255.0;
                out_rgba[i * 4 + c] = v.round() as u8;
            }
        }
        if let Some((a_up, _, _)) =
            upscaler_by_id("lanczos3").and_then(|u| upscale_with(u, rgba.as_raw(), w, h, 4))
        {
            for i in 0..on.min(a_up.len() / 4) {
                out_rgba[i * 4 + 3] = a_up[i * 4 + 3];
            }
        }

        let dir = out_root.join(label);
        let _ = fs::create_dir_all(&dir);
        if let Some(im) = image::RgbaImage::from_raw(ow, oh, out_rgba.clone()) {
            let _ = im.save(dir.join("ai_4x.png"));
        }
        // 总览用缩略图（**必须在 `items` 移走 `out_rgba` 之前**生成）
        if let Some(t) = to_thumb(&out_rgba, ow, oh, 128) {
            thumbs.push(t);
        }
        // 对照：Lanczos3 4x（同尺寸拼接，便于肉眼 A/B）
        let lac = upscaler_by_id("lanczos3").and_then(|u| upscale_with(u, rgba.as_raw(), w, h, 4));
        let xbz = upscaler_by_id("xbrz").and_then(|u| upscale_with(u, rgba.as_raw(), w, h, 4));
        let mut items: Vec<(Vec<u8>, u32, u32)> = vec![(out_rgba, ow, oh)];
        if let Some(x) = xbz {
            items.push(x);
        }
        if let Some(l) = lac {
            items.push(l);
        }
        let (joined, jw, jh) = hstack(&items);
        if let Some(im) = image::RgbaImage::from_raw(jw, jh, joined) {
            let _ = im.save(dir.join("compare_ai_xbrz_lanczos.png"));
        }
        println!(
            "{:<28} {:>10} {:>9} {:>12}  {}",
            label,
            format!("{w}x{h}"),
            ms,
            format!("{ow}x{oh}"),
            "compare_ai_xbrz_lanczos.png"
        );
    }
    // **总览图**：一张图快速扫描所有类型（每格 = 一个样本的 AI 4x 缩略，8 列）
    let (ov, vw, vh) = grid_overview(&thumbs, 128, 8);
    if let Some(im) = image::RgbaImage::from_raw(vw, vh, ov) {
        let _ = im.save(out_root.join("overview.png"));
    }
    println!(
        "\n总览（{} 个样本，每格 128px、8 列）→ {}/overview.png",
        thumbs.len(),
        out_root.display()
    );
    println!(
        "单样本对比图（左=AI，中=xBRZ，右=Lanczos3）→ {}/<样本>/compare_ai_xbrz_lanczos.png",
        out_root.display()
    );
}

/// 把透明区（`alpha < 16`）的 RGB 用**邻接实心像素**逐圈填充（颜色膨胀，`rounds` 圈）。
///
/// ## 为什么必须做
///
/// H3 素材 `alpha=0` 处的 RGB 是**抠图残留**（DEF 帧常带青蓝底框色，抠图只置了 alpha）。
/// 直接把 RGBA 的 RGB 送进 AI，等于让模型把"垃圾色"当内容推理，放大后会在
/// **半透明边缘露出彩色描边**（实测：直接送 = 青边；用全局均值兜底 = 黄边，因为素材多为暖色）。
/// 只有让透明区颜色**等于紧邻的实心色**，AI 输出的边缘才与主体连续、混合自然。
///
/// `rounds` 取 3：伪影只出现在边缘 1~3 像素的过渡带；更深处的透明区用全局均值兜底即可
/// （远离边缘，不影响观感），这样成本仍是 O(n)。
fn dilate_colors(rgba: &mut [u8], w: u32, h: u32, rounds: usize) {
    let (wi, hi) = (w as usize, h as usize);
    if wi == 0 || hi == 0 {
        return;
    }
    let mut filled: Vec<bool> = rgba.chunks_exact(4).map(|p| p[3] >= 16).collect();
    // 全局均值（深层透明区兜底）
    let (mut r0, mut g0, mut b0, mut c0) = (0u64, 0u64, 0u64, 0u64);
    for px in rgba.chunks_exact(4) {
        if px[3] >= 16 {
            r0 += px[0] as u64;
            g0 += px[1] as u64;
            b0 += px[2] as u64;
            c0 += 1;
        }
    }
    let mean = if c0 > 0 {
        [(r0 / c0) as u8, (g0 / c0) as u8, (b0 / c0) as u8]
    } else {
        [128, 128, 128]
    };

    for _ in 0..rounds {
        let snapshot = rgba.to_vec();
        let mut newly: Vec<usize> = Vec::new();
        for y in 0..hi {
            for x in 0..wi {
                let i = y * wi + x;
                if filled[i] {
                    continue;
                }
                let (mut r, mut g, mut b, mut c) = (0u32, 0u32, 0u32, 0u32);
                for (dy, dx) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                    let (ny, nx) = (y as i32 + dy, x as i32 + dx);
                    if ny < 0 || nx < 0 || ny >= hi as i32 || nx >= wi as i32 {
                        continue;
                    }
                    let j = ny as usize * wi + nx as usize;
                    if filled[j] {
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
                    newly.push(i);
                }
            }
        }
        if newly.is_empty() {
            break;
        }
        for i in newly {
            filled[i] = true;
        }
    }
    for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
        if !filled[i] {
            px[0] = mean[0];
            px[1] = mean[1];
            px[2] = mean[2];
        }
    }
}

/// 等比缩放到 `max` 见方（总览网格用的缩略图）
fn to_thumb(rgba: &[u8], w: u32, h: u32, max: u32) -> Option<(Vec<u8>, u32, u32)> {
    let src = image::RgbaImage::from_raw(w, h, rgba.to_vec())?;
    let scale = (max as f32 / w as f32).min(max as f32 / h as f32);
    let nw = ((w as f32 * scale).round() as u32).max(1);
    let nh = ((h as f32 * scale).round() as u32).max(1);
    let t = image::imageops::resize(&src, nw, nh, image::imageops::FilterType::Triangle);
    Some((t.into_raw(), nw, nh))
}

/// 网格总览：缩略图按 `cols` 列、`cell` 见方居中铺进一张深色底的图。
fn grid_overview(cells: &[(Vec<u8>, u32, u32)], cell: u32, cols: u32) -> (Vec<u8>, u32, u32) {
    let rows = ((cells.len() as u32) + cols - 1) / cols;
    let w = cell * cols;
    let h = cell * rows.max(1);
    // 深色底（与启动器深蓝灰一致），否则透明区域在查看器里发白/发黑难辨
    let mut out = Vec::with_capacity((w as usize) * (h as usize) * 4);
    for _ in 0..(w as usize) * (h as usize) {
        out.extend_from_slice(&[18, 22, 32, 255]);
    }
    for (i, (rgba, iw, ih)) in cells.iter().enumerate() {
        let (r, c) = (i as u32 / cols, i as u32 % cols);
        let ox = c * cell + (cell.saturating_sub(*iw)) / 2;
        let oy = r * cell + (cell.saturating_sub(*ih)) / 2;
        for y in 0..*ih {
            for x in 0..*iw {
                let si = ((y * *iw + x) * 4) as usize;
                if si + 4 > rgba.len() {
                    continue;
                }
                let di = (((y + oy) * w + (x + ox)) * 4) as usize;
                if di + 4 > out.len() {
                    continue;
                }
                out[di..di + 4].copy_from_slice(&rgba[si..si + 4]);
            }
        }
    }
    (out, w, h)
}

/// 分级倍率的「施工前勘测」：按**真实缓存**统计每个 sub 的应得倍率分布。
///
/// **只读文件头**（`image_dimensions`，不解码全图）、不生成任何文件。用途：
/// - 验证「大图降档、小图升档」是否如预期（`town` 幕布应落 2x、`creature`/`hero` 应拿 4x）；
/// - 发现**未登记目标框**的 sub（表里返回"全局"的那些）—— 它们是体积优化的漏点来源。
#[test]
#[ignore]
fn hd_rate_survey() {
    use crate::modcache::hd::{hd_target_box, target_rate};
    use std::collections::BTreeMap;

    // 1080p：`computeHdPhys` = 1920/1280 = 1.5；`computeHdRate` = ceil(1.5×2.3125) = 4
    const PHYS: f64 = 1.5;
    const CAP: u32 = 4;

    let root = sample_root();
    let Ok(mods) = fs::read_dir(&root) else {
        println!("样本目录不存在：{}", root.display());
        return;
    };
    // sub → (源图数, rate 计数 [0..=6], 源尺寸直方图)
    let mut stat: BTreeMap<String, (u64, [u64; 7], BTreeMap<String, u64>)> = BTreeMap::new();
    for m in mods.flatten() {
        let img = m.path().join("images");
        if !img.is_dir() {
            continue;
        }
        let mut files = Vec::new();
        collect_webp(&img, &mut files);
        for f in files {
            let Some(sub) = f
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
            else {
                continue;
            };
            let Ok((w, h)) = image::image_dimensions(&f) else {
                continue;
            };
            let r = target_rate(w, h, sub, PHYS, CAP);
            let e = stat
                .entry(sub.to_string())
                .or_insert((0, [0; 7], BTreeMap::new()));
            e.0 += 1;
            e.1[r.min(6) as usize] += 1;
            *e.2.entry(format!("{w}x{h}")).or_insert(0) += 1;
        }
    }

    println!("\n=== 分级倍率勘测（phys={PHYS} → 1080p，上限 {CAP}x；只读文件头）===");
    println!(
        "{:<14} {:>7} {:<11} {:>6} {:>6} {:>5} {:>5} {:>5}  {}",
        "sub", "源图数", "目标框", "跳过", "2x", "3x", "4x", "5x+", "最常见源尺寸"
    );
    let mut total = 0u64;
    for (sub, (n, rates, sizes)) in &stat {
        total += n;
        let tgt = hd_target_box(sub)
            .map(|(w, h)| format!("{w}x{h}"))
            .unwrap_or_else(|| "—(全局)".into());
        let mut top: Vec<_> = sizes.iter().collect();
        top.sort_by(|a, b| b.1.cmp(a.1));
        let top3: Vec<String> = top
            .iter()
            .take(3)
            .map(|(k, v)| format!("{k}×{v}"))
            .collect();
        println!(
            "{:<14} {:>7} {:<11} {:>6} {:>6} {:>5} {:>5} {:>5}  {}",
            sub,
            n,
            tgt,
            rates[1],
            rates[2],
            rates[3],
            rates[4],
            rates[5] + rates[6],
            top3.join("  ")
        );
    }
    println!(
        "{:<14} {:>7}  （跳过 = 源图已够大，不生成变体）",
        "合计", total
    );
}

/// **小图放大验收**（肉眼逐张对比）：对指定素材按「每种倍率 × 每种编码策略」生成到
/// `target/hd-preview/`，每个样本额外输出一张**三联对比图**
/// `compare.webp` = 原图最近邻放大 │ xBRZ + 无损 │ xBRZ + 有损(q85)。
///
/// 用法（样本用环境变量给，相对 `<gameDir>/.vcmi-fv-launcher/mods`；缺省取代表集）：
///
/// ```powershell
/// $env:HD_PREVIEW="core.castle/images/creature/halberdier-large.webp"
/// cd src-tauri
/// cargo test -p vcmi-fv-launcher hd_preview -- --ignored --nocapture
/// ```
///
/// ⚠️ 三联图里「最近邻」是**清晰度上界参照**（无插值、零损失）：放大算法若明显不如它锐，
/// 就说明该算法/编码有问题；「无损 vs 有损」用来判断**编码**是否在毁掉像素画。
#[test]
#[ignore]
fn hd_preview() {
    use crate::infra::image::encode_rgba_auto;
    use crate::infra::upscale::{all_upscalers, upscale_with};

    const RATES: [u32; 2] = [2, 4];
    let out_root = PathBuf::from("../target/hd-preview");
    let _ = fs::remove_dir_all(&out_root);
    let _ = fs::create_dir_all(&out_root);

    // 样本来源（优先级）：
    //   ① 环境变量 `HD_PREVIEW`（逗号分隔）；
    //   ② 清单文件 `target/hd-preview-samples.txt`（每行一个相对路径，`#` 注释）—— 便于反复验收；
    //   ③ 内置代表集（按 sub 各取 1 张）。
    let root = sample_root();
    let list_file = PathBuf::from("../target/hd-preview-samples.txt");
    let env = if std::env::var("HD_PREVIEW")
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false)
    {
        std::env::var("HD_PREVIEW").unwrap_or_default()
    } else if list_file.is_file() {
        fs::read_to_string(&list_file)
            .unwrap_or_default()
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
            .collect::<Vec<_>>()
            .join(",")
    } else {
        String::new()
    };
    let mut samples: Vec<(String, PathBuf)> = Vec::new();
    if !env.trim().is_empty() {
        for rel in env.split(',') {
            let p = root.join(rel.trim().replace('/', std::path::MAIN_SEPARATOR_STR));
            if p.is_file() {
                let name = p.file_stem().unwrap().to_string_lossy().into_owned();
                samples.push((name, p));
            } else {
                println!("⚠️ 样本不存在：{}", p.display());
            }
        }
    } else {
        // 缺省：**自动抽样覆盖全部 sub × 尺寸档** —— 每个 sub 最多取 `PER_SUB` 种不同源尺寸
        // （同一尺寸在多张里取第一张即可，同档内容差异由清单文件按需补充）。
        // 这是"验收所有类型"的默认口径：sub 覆盖全（22 种），尺寸档覆盖全（含 1x1 之类脏数据）。
        use std::collections::BTreeMap;
        const PER_SUB: usize = 3;
        const MAX_SAMPLES: usize = 72;
        let mut by_sub: BTreeMap<String, BTreeMap<String, PathBuf>> = BTreeMap::new();
        'mods: for m in fs::read_dir(&root).into_iter().flatten().flatten() {
            let img = m.path().join("images");
            if !img.is_dir() {
                continue;
            }
            let mut files = Vec::new();
            collect_webp(&img, &mut files);
            files.sort();
            for f in files {
                let Some(sub) = f
                    .parent()
                    .and_then(|p| p.file_name())
                    .and_then(|s| s.to_str())
                else {
                    continue;
                };
                let e = by_sub.entry(sub.to_string()).or_default();
                if e.len() >= PER_SUB {
                    continue;
                }
                let Ok((w, h)) = image::image_dimensions(&f) else {
                    continue;
                };
                e.entry(format!("{w}x{h}")).or_insert(f);
                let total: usize = by_sub.values().map(|v| v.len()).sum();
                if total >= MAX_SAMPLES {
                    break 'mods;
                }
            }
        }
        for (sub, sizes) in &by_sub {
            for (dims, path) in sizes {
                samples.push((format!("{sub}-{dims}"), path.clone()));
            }
        }
        samples.sort_by(|a, b| a.0.cmp(&b.0));
    }
    // 判定汇总：一眼看清**每个类型**会被分派到哪个算法（与真实生成完全同口径）
    println!("\n=== 分派汇总（颜色占比 ≤24% → 像素画 / 否则插画）===");
    for (name, path) in &samples {
        let Ok(bytes) = fs::read(path) else { continue };
        let Ok(img) = image::load_from_memory(&bytes) else {
            continue;
        };
        let rgba = img.to_rgba8();
        let (w, h) = (rgba.width(), rgba.height());
        let pct = crate::modcache::hd::color_ratio_pct(rgba.as_raw(), w, h);
        let target = crate::modcache::hd::hd_target_box(
            path.parent()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
                .unwrap_or(""),
        )
        .map(|(tw, th)| format!("{tw}x{th}"))
        .unwrap_or_else(|| "—(全局)".into());
        // 打印**真实判定结果**（`pick_upscaler` 试算），而不是统计量推断
        let opt = crate::modcache::hd::HdOptions {
            rate: 6,
            phys: 1.0,
            upscaler: crate::infra::upscale::DEFAULT_UPSCALER_ID.to_string(),
            ai: false, // 验收用：显式关闭 AI，便于对照传统算法的分派结果
            ai_model: None,
            usm: 0.0, // 同上：关锐化，保持与历史产物可比
            ai_mix: 1.0,
        };
        let (algo, pixel_like) =
            crate::modcache::hd::pick_upscaler(rgba.as_raw(), w, h, &opt, 4, "");
        println!(
            "{:<26} {:>9} {:>4}%  {:<14} 目标框 {:<9}{}",
            name,
            format!("{w}x{h}"),
            pct,
            algo.id(),
            target,
            if pixel_like {
                " [像素画·无损]"
            } else {
                ""
            }
        );
    }
    if samples.is_empty() {
        println!("没有可用样本（检查 HD_PREVIEW 或缓存目录）");
        return;
    }

    let algos = all_upscalers();
    println!(
        "\n=== 小图放大验收 · 多算法并排（产物 → {}）===",
        out_root.display()
    );
    println!("锐度 = 平均相邻亮度差（高=更锐，但马赛克/噪声也会拉高）→ **仅作参考，以肉眼为准**");

    for (name, path) in &samples {
        let Ok(bytes) = fs::read(path) else { continue };
        let Ok(img) = image::load_from_memory(&bytes) else {
            continue;
        };
        let rgba = img.to_rgba8();
        let (w, h) = (rgba.width(), rgba.height());
        let dir = out_root.join(name);
        let _ = fs::create_dir_all(&dir);
        // 顺带打印分派判定依据：颜色占比 ≤20% → 判定为像素画（走 xBRZ + 无损），否则插画（Lanczos3）
        let pct = crate::modcache::hd::color_ratio_pct(rgba.as_raw(), w, h);
        println!(
            "\n── {name}   源 {w}x{h}   颜色占比 {pct}%  → {}",
            if crate::modcache::hd::is_pixel_art(rgba.as_raw(), w, h) {
                "像素画（xBRZ + 无损）"
            } else {
                "插画/写实（Lanczos3 + 有损/默认）"
            }
        );
        for rate in RATES {
            let mut rows: Vec<(String, f64, usize, Vec<u8>, u32, u32)> = Vec::new();
            let mut best = 0.0f64;
            for a in algos {
                let Some((out, ow, oh)) = upscale_with(*a, rgba.as_raw(), w, h, rate) else {
                    continue;
                };
                let eng = grad_energy(&out, ow, oh);
                // 体积用**无损**编码测：隔离"算法差异"，不掺入有损质量参数
                let sz = encode_rgba_auto(&out, ow, oh).map(|v| v.len()).unwrap_or(0);
                best = best.max(eng);
                rows.push((a.id().to_string(), eng, sz, out, ow, oh));
            }
            println!("   [{rate}x → {}x{}]", w * rate, h * rate);
            println!(
                "   {:<12} {:>9} {:>10} {:>9}",
                "算法", "锐度", "无损体积", "相对最锐"
            );
            let mut items: Vec<(Vec<u8>, u32, u32)> = Vec::new();
            for (id, eng, sz, out, ow, oh) in &rows {
                let pct = if best > 0.0 { eng / best * 100.0 } else { 0.0 };
                println!(
                    "   {:<12} {:>9.2} {:>9}KB {:>8.0}%",
                    id,
                    eng,
                    sz / 1024,
                    pct
                );
                // 每算法单独落盘（png 无损，便于逐张细看）
                if let Some(im) = image::RgbaImage::from_raw(*ow, *oh, out.clone()) {
                    let _ = im.save(dir.join(format!("{rate}x_{id}.png")));
                }
                items.push((out.clone(), *ow, *oh));
            }
            // 并排对比图：**一张图看全所有算法**（顺序 = 注册表，nearest 在最左作上界参照）
            let (joined, jw, jh) = hstack(&items);
            if let Some(im) = image::RgbaImage::from_raw(jw, jh, joined) {
                let _ = im.save(dir.join(format!("{rate}x_algo-compare.png")));
            }
            // **生产路径产物**：与缓存里 `@Nx` 生成走完全相同的分派 + 编码
            // （`upscale_file`）—— 验收时优先看这个文件，它就是最终效果。
            // ⚠️ 这里的 `ai: true` 让**生产路径产物**反映真实分派（插画档在 4x 时走 AI），
            // 便于端到端验收；上面的多算法对比图始终是传统算法，两者互补。
            let opt = crate::modcache::hd::HdOptions {
                rate: 6,
                phys: 1.0,
                upscaler: crate::infra::upscale::DEFAULT_UPSCALER_ID.to_string(),
                ai: true,
                // 生产路径产物要能真走 AI：这里显式给推荐模型（组件缺失 → None → 传统档）
                ai_model: crate::runtime::ai_model_path(crate::runtime::RECOMMENDED_AI_MODEL),
                usm: 0.0,
                ai_mix: 1.0,
            };
            let dest = dir.join(format!("{rate}x_production.webp"));
            // bench 走纯 1x 路径（`None`）：HD 源变体由真实构建覆盖，bench 只需可比基线
            if crate::modcache::hd::upscale_file(path, &dest, &opt, rate, None, "") {
                let sz = fs::metadata(&dest).map(|m| m.len() / 1024).unwrap_or(0);
                println!("   → 生产路径产物 {rate}x_production.webp（{sz}KB）");
            }
        }
    }
    println!("\n产物：<样本>/<倍率>x_<算法>.png 与 <倍率>x_algo-compare.png（后者一张看全）");
}

/// 横向拼接两张 RGBA（等高，居中，短图上下填透明）。返回 `(rgba, w, h)`。
fn hstack(items: &[(Vec<u8>, u32, u32)]) -> (Vec<u8>, u32, u32) {
    let h = items.iter().map(|(_, _, h)| *h).max().unwrap_or(0);
    let w: u32 = items.iter().map(|(_, w, _)| *w).sum();
    let mut out = vec![0u8; (w as usize) * (h as usize) * 4];
    let mut x0 = 0u32;
    for (rgba, iw, ih) in items {
        if *iw == 0 || *ih == 0 {
            x0 += *iw;
            continue;
        }
        let y0 = (h - *ih) / 2;
        for y in 0..*ih {
            for x in 0..*iw {
                let si = ((y * *iw + x) * 4) as usize;
                if si + 4 > rgba.len() {
                    continue;
                }
                let di = (((y + y0) * w + (x + x0)) * 4) as usize;
                out[di..di + 4].copy_from_slice(&rgba[si..si + 4]);
            }
        }
        x0 += *iw;
    }
    (out, w, h)
}

/// 高清变体的「质量 / 体积」权衡 bench —— 决定 `hd::HD_QUALITY`。
///
/// 变体一律走有损（放大后色带本就更不可见），质量越低体积越小；本 bench 对真实素材
/// 放大后按多档质量编码，输出体积对照，并把**首个样本的各档产物**落盘供肉眼 A/B。
///
/// 判定依据：从 q95 往下，体积明显下降而肉眼无差的最佳点（放大图的细节冗余高于原图，
/// 通常 q85 就能与 q95 观感等价）。
///
/// ⚠️ 与 [`hd_quality_bench`] 的区别：那个比的是**不同放大算法**在固定质量下的保真度
/// （锐度/PSNR），这个比的是**同一算法下不同编码质量**的体积代价。
#[test]
#[ignore]
fn hd_variant_quality_bench() {
    use crate::infra::image::encode_webp_lossy_q;
    use crate::infra::upscale::{default_upscaler, upscale_with};

    const QUALITIES: [f32; 5] = [95.0, 90.0, 85.0, 80.0, 75.0];
    const PER_SUB: usize = 2;
    let out_root = PathBuf::from("../target/hd-bench/quality");
    let _ = fs::create_dir_all(&out_root);

    // 每个 sub 取前 2 张（覆盖小图/中图/大图），跳过变体（`collect_webp` 已排除）
    let root = sample_root();
    let mut picked: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut samples: Vec<(String, PathBuf)> = Vec::new();
    let Ok(mods) = fs::read_dir(&root) else {
        println!("样本目录不存在：{}", root.display());
        return;
    };
    for m in mods.flatten() {
        let img = m.path().join("images");
        if !img.is_dir() {
            continue;
        }
        let mut files = Vec::new();
        collect_webp(&img, &mut files);
        files.sort();
        for f in files {
            let Some(sub) = f
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|s| s.to_str())
            else {
                continue;
            };
            let n = picked.entry(sub.to_string()).or_insert(0);
            if *n >= PER_SUB {
                continue;
            }
            *n += 1;
            let stem = f.file_stem().unwrap().to_string_lossy().into_owned();
            samples.push((format!("{sub}/{stem}"), f));
        }
    }
    samples.sort();

    let algo = default_upscaler();
    println!(
        "\n=== 高清变体 质量/体积权衡（2x，{}，{} 个样本）===",
        algo.id(),
        samples.len()
    );
    print!("{:<30} {:>7} {:>6}", "样本", "源KB", "尺寸");
    for q in QUALITIES {
        print!(" {:>12}", format!("q{}", q as u32));
    }
    println!("   （产物KB 相对q95）");

    let mut totals = [0usize; QUALITIES.len()];
    for (name, path) in &samples {
        let Ok(bytes) = fs::read(path) else { continue };
        let Ok(img) = image::load_from_memory(&bytes) else {
            continue;
        };
        let rgba = img.to_rgba8();
        let (w, h) = (rgba.width(), rgba.height());
        let Some((up, uw, uh)) = upscale_with(algo, rgba.as_raw(), w, h, 2) else {
            continue;
        };
        print!(
            "{:<30} {:>7} {:>6}",
            name,
            bytes.len() / 1024,
            format!("{w}x{h}")
        );
        let mut base = 0usize;
        for (i, q) in QUALITIES.iter().enumerate() {
            let Some(enc) = encode_webp_lossy_q(&up, uw, uh, *q) else {
                print!(" {:>12}", "-");
                continue;
            };
            totals[i] += enc.len();
            if i == 0 {
                base = enc.len();
            }
            let pct = if base > 0 { enc.len() * 100 / base } else { 0 };
            print!(" {:>12}", format!("{} {pct:>3}%", enc.len() / 1024));
            // 只对首个样本落全部档位，避免刷爆磁盘（其余样本体积已足够判断）
            if samples.first().map(|(n, _)| n.as_str()) == Some(name.as_str()) {
                let dst = out_root.join(format!("{}_q{}.webp", name.replace('/', "_"), *q as u32));
                let _ = fs::write(dst, &enc);
            }
        }
        println!();
    }

    print!("\n{:<30} {:>7} {:>6}", "合计", "", "");
    let base_total = totals[0];
    for t in totals.iter() {
        let pct = if base_total > 0 {
            t * 100 / base_total
        } else {
            0
        };
        print!(" {:>12}", format!("{} {pct:>3}%", t / 1024 / 1024));
    }
    println!("  （MB）");
    println!("肉眼 A/B（首个样本全档位）→ {}", out_root.display());
}

/// 客观质量对比：锐度保持 / 往返 PSNR / alpha 暗边
#[test]
#[ignore]
fn hd_quality_bench() {
    let samples = collect_all_samples();
    assert!(!samples.is_empty(), "未收集到样本");

    println!("\n=== 质量指标逐样本（锐度比 = 放大后梯度/原图梯度，越接近 1 越不掉锐）===");
    println!(
        "{:<34} {:<9} {:<11} {:>5} {:>9} {:>9} {:>10}",
        "样本", "桶", "算法", "倍率", "锐度比", "PSNR", "半透明px"
    );
    // 算法 → (锐度比和, 计数, PSNR 和)
    let mut agg: HashMap<&'static str, (f64, usize, f64)> = HashMap::new();
    let mut semi_total = 0usize;

    for (n, b, w, h, px) in &samples {
        let src_g = grad_energy(px, *w, *h);
        let (semi, _, _) = alpha_stats(px);
        semi_total += semi;
        for u in all_upscalers() {
            for factor in FACTORS {
                let Some((up, uw, uh)) = upscale_with(*u, px, *w, *h, factor) else {
                    continue;
                };
                let ratio = if src_g > 0.0 {
                    grad_energy(&up, uw, uh) / src_g
                } else {
                    f64::NAN
                };
                let psnr = roundtrip_psnr(&up, uw, uh, px, *w, *h);
                println!(
                    "{:<34} {:<9} {:<11} {:>5} {:>9.3} {:>9.2} {:>10}",
                    n,
                    b,
                    u.id(),
                    format!("{factor}x"),
                    ratio,
                    psnr,
                    semi
                );
                let e = agg.entry(u.id()).or_insert((0.0, 0, 0.0));
                e.0 += ratio;
                e.1 += 1;
                e.2 += psnr;
            }
        }
    }

    println!("\n=== 质量指标汇总（全部样本 × 2x/3x 平均）===");
    println!("{:<12} {:>10} {:>12}", "算法", "平均锐度比", "平均PSNR");
    let mut rows: Vec<_> = agg.iter().collect();
    rows.sort_by(|a, b| {
        b.1 .0
            .partial_cmp(&a.1 .0)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for (id, (rg, cnt, rp)) in rows {
        if *cnt == 0 {
            continue;
        }
        println!(
            "{:<12} {:>10.3} {:>12.2}",
            id,
            rg / *cnt as f64,
            rp / *cnt as f64
        );
    }

    // alpha 暗边核对：判据 = 放大图的「半透明像素亮度 / 不透明像素亮度」是否**低于原图**。
    // 未做 alpha 预乘的放大算法会让半透明像素与透明区混合，边缘出现暗环 → 该比值下降。
    println!("\n=== alpha 暗边核对（Δ = 放大图亮度比 − 原图亮度比；明显为负 = 有暗边）===");
    println!("样本中存在半透明像素的总数：{}", semi_total);
    if semi_total == 0 {
        println!("⚠️ 无带 alpha 的样本，本项未验证（H3 原版 DEF 素材 alpha 恒为 255）");
    }
    for (n, _b, w, h, px) in &samples {
        let (semi, sl, ol) = alpha_stats(px);
        if semi == 0 {
            continue;
        }
        let src_ratio = sl / ol;
        println!("  原图 {n} {w}x{h}：半透明 {semi} px，亮度比 {src_ratio:.3}");
        for u in all_upscalers() {
            let Some((up, uw, uh)) = upscale_with(*u, px, *w, *h, 2) else {
                continue;
            };
            let (us, usl, uol) = alpha_stats(&up);
            if !usl.is_finite() || !uol.is_finite() {
                continue;
            }
            let r = usl / uol;
            println!(
                "      {:<11} 2x → {uw}x{uh}：半透明 {us} px，亮度比 {r:.3}（Δ={:+.3}）",
                u.id(),
                r - src_ratio
            );
        }
    }
}

/// 样本根（mods 缓存目录）统一走 `mod.rs` 的 `sample_root()`：
/// ① 环境变量 `HD_BENCH_MODS` → 任意 `.vcmi-fv-launcher/mods` 目录（便携部署 / 实机缓存）；
/// ② 缺省 = 仓库内 `references/Heroes3-VCMI/.vcmi-fv-launcher/mods` 验证缓存。
///
/// ⚠️ 缺省路径**经常不存在**（`references/` 是按需挂的 junction，且缓存要跑过启动器才有）——
/// 那种情况下测试会打印"没有可用样本"退出，此时用 `HD_BENCH_MODS` 指向实际缓存即可。
/// （实现见 `mod.rs::sample_root`）

fn out_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../target/hd-bench")
}

fn bucket_of(edge: u32) -> &'static str {
    for (max, name) in BUCKETS {
        if edge <= max {
            return name;
        }
    }
    "big"
}

fn collect_webp(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_webp(&p, out);
        } else if p.extension().and_then(|s| s.to_str()) == Some("webp") {
            // ⚠️ 排除高清变体（`@Nx.webp`）：真机跑过一轮高清生成后缓存里就有它们，
            // 混进样本会让"首次生成数"断言失败（变体被判为幂等跳过），
            // 质量对比也会把变体当源图**再放大一次** → 结论失真。
            let is_variant = p
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(crate::modcache::hd::variant_rate)
                .is_some();
            if !is_variant {
                out.push(p);
            }
        }
    }
}

/// 抽样：按桶各取前 N 张，返回 `(样本名, 桶, w, h, rgba)`
fn collect_samples(mods: &[&str]) -> Vec<(String, &'static str, u32, u32, Vec<u8>)> {
    let mut per_bucket: HashMap<&'static str, Vec<(String, u32, u32, Vec<u8>)>> = HashMap::new();
    for m in mods {
        let dir = sample_root().join(m).join("images");
        if !dir.is_dir() {
            continue;
        }
        let mut files = Vec::new();
        collect_webp(&dir, &mut files);
        files.sort();
        for f in files {
            let Ok(bytes) = fs::read(&f) else { continue };
            let Ok(img) = image::load_from_memory(&bytes) else {
                continue;
            };
            let rgba = img.to_rgba8();
            let (w, h) = (rgba.width(), rgba.height());
            let b = bucket_of(w.max(h));
            let slot = per_bucket.entry(b).or_default();
            if slot.len() >= SAMPLE_PER_BUCKET {
                continue;
            }
            let stem = f.file_stem().and_then(|s| s.to_str()).unwrap_or("x");
            slot.push((format!("{m}-{stem}"), w, h, rgba.into_raw()));
        }
    }
    let mut out = Vec::new();
    for (_b, v) in per_bucket {
        for (n, w, h, px) in v {
            out.push((n, bucket_of(w.max(h)), w, h, px));
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// 补一个「带 alpha」样本桶。
///
/// **为什么必须显式抽样**：H3 原版 DEF 素材（兵种/英雄/建筑/幕布）解码后 alpha 恒为 255，
/// 首轮 bench 的 9 个样本半透明像素数为 **0** ⇒ "透明边缘是否发暗"这个风险点等于没测。
/// 真正带半透明边缘的是**第三方 mod 的小图标**（≤64px 会被缓存做四角抠图，见 `assets.rs`）。
fn collect_alpha_samples(
    mods: &[&str],
    limit: usize,
) -> Vec<(String, &'static str, u32, u32, Vec<u8>)> {
    let mut out = Vec::new();
    for m in mods {
        let dir = sample_root().join(m).join("images");
        if !dir.is_dir() {
            continue;
        }
        let mut files = Vec::new();
        collect_webp(&dir, &mut files);
        files.sort();
        for f in files {
            let Ok(bytes) = fs::read(&f) else { continue };
            let Ok(img) = image::load_from_memory(&bytes) else {
                continue;
            };
            let rgba = img.to_rgba8();
            let (w, h) = (rgba.width(), rgba.height());
            if w.max(h) > 128 {
                continue; // 只关心小图标（大图不会走抠图）
            }
            let raw = rgba.into_raw();
            let (semi, _, _) = alpha_stats(&raw);
            if semi == 0 {
                continue;
            }
            let stem = f.file_stem().and_then(|s| s.to_str()).unwrap_or("x");
            out.push((format!("{m}-{stem}"), "alpha", w, h, raw));
            if out.len() >= limit {
                return out;
            }
        }
    }
    out
}

/// 主样本集 = 按尺寸分桶抽样 + 显式补齐的带 alpha 样本
fn collect_all_samples() -> Vec<(String, &'static str, u32, u32, Vec<u8>)> {
    let mut v = collect_samples(&["core.castle", "HuaXia"]);
    v.extend(collect_alpha_samples(&["new-chinese-mods", "HuaXia"], 3));
    v
}

#[test]
#[ignore]
fn hd_upscale_bench() {
    let samples = collect_all_samples();
    assert!(
        !samples.is_empty(),
        "未收集到样本，检查缓存目录：{}",
        sample_root().display()
    );

    println!("\n=== HD 素材生成 bench：样本 {} 张 ===", samples.len());
    for (n, b, w, h, _) in &samples {
        println!("  [{}] {} {}x{}", b, n, w, h);
    }

    let out = out_root();
    let _ = fs::remove_dir_all(&out);

    // (算法, 倍率) → 桶 → (总耗时ms, 总字节, 张数)
    let mut table: Vec<(
        &'static str,
        u32,
        HashMap<&'static str, (u128, usize, usize)>,
    )> = Vec::new();

    for u in all_upscalers() {
        for factor in FACTORS {
            let mut per_bucket: HashMap<&'static str, (u128, usize, usize)> = HashMap::new();
            for (n, b, w, h, px) in &samples {
                let t = Instant::now();
                let Some((up, uw, uh)) = upscale_with(*u, px, *w, *h, factor) else {
                    continue;
                };
                let ms = t.elapsed().as_millis();
                let Some(enc) = encode_rgba_auto(&up, uw, uh) else {
                    continue;
                };
                let slot = per_bucket.entry(*b).or_insert((0, 0, 0));
                slot.0 += ms;
                slot.1 += enc.len();
                slot.2 += 1;

                // 落盘供肉眼对比：2x 全量 + 3x 的 Lanczos3/xBRZ（避免产物爆炸）
                if factor == 2 || u.id() == "lanczos3" || u.id() == "xbrz" {
                    let dir = out.join(format!("{}-{}x", u.id(), factor)).join(b);
                    let _ = fs::create_dir_all(&dir);
                    let _ = fs::write(dir.join(format!("{n}.webp")), &enc);
                }
            }
            table.push((u.id(), factor, per_bucket));
        }
    }

    println!("\n=== 耗时 / 体积（按桶）===");
    println!(
        "{:<12} {:<5} {:<10} {:>9} {:>9} {:>10}",
        "算法", "倍率", "桶", "总ms", "均ms/张", "产物KB"
    );
    for (algo, factor, per_bucket) in &table {
        let mut keys: Vec<_> = per_bucket.keys().copied().collect();
        keys.sort();
        for b in keys {
            let (ms, bytes, cnt) = per_bucket[b];
            let avg = if cnt > 0 { ms as f64 / cnt as f64 } else { 0.0 };
            println!(
                "{:<12} {:<5} {:<10} {:>9} {:>9.1} {:>10}",
                algo,
                format!("{factor}x"),
                b,
                ms,
                avg,
                bytes / 1024
            );
        }
    }

    let survey = survey_cache();
    println!(
        "\n=== 外推：全量素材（实测 {} 张 = tiny {} + mid {} + big {}）===",
        survey.total(),
        survey.tiny,
        survey.mid,
        survey.big
    );
    println!(
        "{:<12} {:<5} {:>14} {:>14}",
        "算法", "倍率", "单线程总s", "eco并行估s"
    );
    let par = eco_threads();
    for (algo, factor, per_bucket) in &table {
        let mut est_ms = 0.0f64;
        for (b, (ms, _bytes, cnt)) in per_bucket {
            if *cnt == 0 {
                continue;
            }
            let avg = *ms as f64 / *cnt as f64;
            est_ms += avg * survey.of(b) as f64;
        }
        println!(
            "{:<12} {:<5} {:>14.0} {:>14.0}",
            algo,
            format!("{factor}x"),
            est_ms / 1000.0,
            est_ms / 1000.0 / par as f64
        );
    }
    println!(
        "\n（eco 并行按 cpus/8 = {} 估算；产物已写到 {}）",
        par,
        out.display()
    );
}
