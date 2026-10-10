//! AI 超分（Real-ESRGAN ONNX）—— **插画类**素材的增强放大。
//!
//! ## 定位
//!
//! 传统插值（Lanczos3）只能"不失真"，无法补出细节；AI 能生成合理纹理。
//! 实测（`hd_bench::hd_ai_preview` 的三联对比）：插画上 AI **碾压** xBRZ/Lanczos3，
//! 而**像素画上 AI 反而更差**（把像素风平滑成噪点）—— 故只在"非像素画"档启用，
//! 像素画仍走 xBRZ（判定见 `modcache::hd::pick_upscaler`）。
//!
//! ## 模型分发（跟随外挂组件）
//!
//! 模型**不再内嵌**进 exe，而是与 `onnxruntime.dll` 一起属于外挂组件 `runtime::AI`
//! （约定目录 `<exe>/.vcmi-fv-launcher/runtime/ai/`），由 `runtime` 模块统一扫描/校验与安装。
//! 理由：AI 是可选项，默认包不该为它付几十 MB；且 exe 与组件可独立演进。
//!
//! ⚠️ **本模块不选模型、也没有默认模型**：用哪个 `.onnx` 由 `launcher.json` 的
//! `hd.ai.model` 决定，上层解析成绝对路径后经 [`configure_model`] 注入；未注入 →
//! 一律返回 `None`（AI 不生效，回退传统算法）。session 池按模型分桶（见 [`with_session`]）。
//!
//! ## 运行库分发（ONNX Runtime = **外部 DLL**，不静态链接）
//!
//! `ort` 以 `load-dynamic` 方式**运行时加载** `onnxruntime.dll`，不编进 exe：
//! - 静态链接会把 **~30 MB** 的 ONNX Runtime 塞进 exe（10.7MB → 42.8MB）；
//! - 而版本更新是「整包替换 exe」→ 每次更新都要重下这 30MB。外置后 **DLL 不随更新变化**。
//!
//! 路径 = 外挂组件 `runtime::AI` 的约定目录（`<exe>/.vcmi-fv-launcher/runtime/ai/`），
//! 齐备性与 sha256 由上层 `runtime::is_ready` 校验后再注入（本模块不读配置）。
//! **懒加载**：只有首次真正用到 AI 时才 `LoadLibrary`，未开启 AI 的用户零开销。
//!
//! ⚠️ **加载前必须先钉住 VC++ 运行库**（[`pin_vc_runtime`]）：ORT 1.28 需要 MSVC 14.40+，
//! 而"应用程序目录优先"会让它绑上游戏目录里 VCMI 自带的旧版（实测 14.29）→ DllMain 失败。
//!
//! ⚠️ **为什么必须自己先校验再加载**：ort 内部 `setup_api()` 在 dylib 缺失时是
//! `.expect("Failed to load ONNX Runtime dylib")` —— 会 **panic**，与本项目
//! 「AI 失败静默回退」的语义冲突。故本模块先自查存在性 + sha256，再走
//! `ort::init_from`（返回 `Result`），且**保证是第一个被调用的 ort API**。
//! 任何环节失败 → 整进程内永久禁用 AI，`upscale()` 一律返回 `None` → 回退传统算法。
//!
//! ## 并行安全
//!
//! `ort::Session::run` 需要 `&mut self`，而素材阶段在 rayon 池里并行调用 —— 故用
//! **Session 池**（每个 session 用 `Mutex` 包裹，池上限 = [`POOL_MAX`]）。
//! 池为空时新建（约 100 ms）并归还，避免"每次推理都重建"。
//!
//! ## 失败语义
//!
//! 任何环节失败（模型不可用 / 推理异常 / 尺寸不支持）一律返回 `None`，
//! 由调用方回退传统算法 —— **AI 是增强项，绝不因它丢图或让批次失败**。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use ort::session::Session;
use ort::value::Value;

use crate::infra::berror::be_err;

/// 组件内的运行库文件名（清单与 sha256 见上层的 `runtime::COMPONENTS`）
const RT_DLL: &str = "onnxruntime.dll";
/// 本模型只支持 4 倍放大
pub const AI_FACTOR: u32 = 4;

/// Session 池上限（与 `batch.rs` 的最高档 turbo 并行度同量级即可）
const POOL_MAX: usize = 8;

/// 透明区颜色膨胀圈数（详见 [`dilate_colors`]）。
///
/// `hd.rs::upscale_file` 的**非像素画档**也用同一取值做放大前预处理（消除插值彩边）。
pub(crate) const DILATE_ROUNDS: usize = 3;

/// 「视为全透明」的 alpha 阈值：输出时低于它的像素，RGB 还原为原图（Lanczos3 放大）的颜色。
///
/// 输入端为避免模型把抠图残留色当内容，做过 `dilate_colors` 填色；那些颜色会留在输出里
/// （实测描边图整片透明区变暗，PNG 打开呈黑底）。alpha≈0 在屏幕上不可见，但查看器、
/// 缩放与合成路径会暴露它 —— 还原成与其它算法列一致的原图色最稳妥。
const TRANSPARENT_ALPHA: u8 = 8;

/// 全局 session 池（懒初始化）—— **按模型文件分桶**：
/// 不同模型的 session 不能互相复用（权重/结构不同，复用会"串味"）。
static POOL: OnceLock<Mutex<HashMap<PathBuf, Vec<Session>>>> = OnceLock::new();

fn pool() -> &'static Mutex<HashMap<PathBuf, Vec<Session>>> {
    POOL.get_or_init(|| Mutex::new(HashMap::new()))
}

// ── 外挂组件（runtime::AI）懒加载 ─────────────────────────────────────────
//
// ⚠️ **分层约束**：`infra` 禁止反向依赖 `runtime` / `vcmi`（会成环：`runtime → vcmi → infra`）。
// 故本模块**不知道组件目录在哪**，只接受上层注入的、已通过校验的目录。

/// 运行时状态：组件目录 + 当前模型 + 加载结果（可重置 —— 组件安装/卸载后要能重新加载）
struct RtState {
    dir: Option<PathBuf>,
    /// 当前 AI 模型（**绝对路径**）—— 由上层解析 `hd.ai.model` 后注入。
    /// `None` = 未指定模型 ⇒ `upscale` 一律返回 `None`（**无默认模型**）。
    model: Option<PathBuf>,
    loaded: Option<Result<(), String>>,
}

static RT: OnceLock<Mutex<RtState>> = OnceLock::new();

fn rt() -> &'static Mutex<RtState> {
    RT.get_or_init(|| {
        Mutex::new(RtState {
            dir: None,
            model: None,
            loaded: None,
        })
    })
}

/// 注入组件目录。由 `lib.rs` 启动时、以及组件安装 / 卸载成功后调用。
///
/// ⚠️ **只接受"已确认可用"的目录**：齐备性与 sha256 校验在上层 `runtime::is_ready` 完成
/// （dll 版本错配会让 ORT 的 `GetApi` 返回 null → 崩溃，必须拦在这一层之前）。
/// 传 `None` = 组件未安装 → AI 保持禁用。
pub fn configure_runtime(dir: Option<PathBuf>) {
    if let Ok(mut g) = rt().lock() {
        g.dir = dir.clone();
        // 目录变了就丢弃旧的加载结果，下次用到时重新尝试
        g.loaded = None;
        if let Some(d) = dir {
            tracing::info!(target: "launcher", "AI 组件目录已就绪：{}", d.display());
        }
    }
}

/// 注入**当前 AI 模型**（绝对路径，须已由上层确认存在）。
///
/// 由 `modcache::hd::HdOptions::new` 在每次构建开始时调用一次：
/// - `None` = 用户没配 `hd.ai.model` / 配置被拒 / 文件不存在 ⇒ **AI 不生效**（无默认模型）；
/// - 与上次相同则不动（避免重复日志）；不同则替换 —— 池按模型分桶，旧桶下次用到时自然淘汰。
pub fn configure_model(model: Option<PathBuf>) {
    if let Ok(mut g) = rt().lock() {
        if g.model != model {
            if let Some(m) = &model {
                tracing::info!(target: "launcher", "AI 模型已就绪：{}", m.display());
            }
            g.model = model;
        }
    }
}

/// **清空 session 池**：丢弃所有缓存的 ORT session（释放模型权重 + 其 arena 占用）。
///
/// 高清 / AI 构建结束时由 `modcache::batch` 调用。session 池是进程级 `OnceLock`，
/// 否则一次构建会把每个 session 的**模型权重（数十 MB）+ 推理 arena 高水位**永久留在内存里
/// —— 任务管理器的占用停在高水位不回落，就是这么来的。下次构建首张 AI 图会重建 session
/// （约 100 ms/个），代价可忽略。正在推理中被取出的 session 由该线程持有，不受影响。
pub fn clear_sessions() {
    if let Ok(mut g) = pool().lock() {
        if !g.is_empty() {
            g.clear();
            tracing::debug!(target: "launcher", "已清空 AI session 池（回收模型 / arena 内存）");
        }
    }
}

/// 当前模型路径（未配置 → `None`）
fn current_model() -> Option<PathBuf> {
    rt().lock().ok().and_then(|g| g.model.clone())
}

/// VC++ 运行库（MSVC 14.x）名单：ORT 与其同源 dll 导入表里的 CRT 依赖。
///
/// ⚠️ 与随 AI 版包分发的 `src-tauri/resources/runtime/ai/` 里那 5 个文件**一一对应**，
///    改名 / 增删需两处同步；它们**不进** `runtime::COMPONENTS` 校验清单（见那里的说明）。
#[cfg(windows)]
const CRT_DLLS: [&str; 5] = [
    "vcruntime140.dll",
    "vcruntime140_1.dll",
    "msvcp140.dll",
    "msvcp140_1.dll",
    "msvcp140_2.dll",
];

/// 把 VC++ 运行库**钉进进程**：组件目录自带副本优先，其次 System32。
///
/// **为什么必须做**：Windows 解析依赖是**应用程序目录优先**。启动器 exe 装在游戏目录时，
/// VCMI 官方包自带的 `msvcp140.dll` / `vcruntime140.dll`（实测 14.29.30157）会先于 System32 的
/// 14.51 被绑上，而 ORT 1.28 需要 ≥ 14.40 → `onnxruntime.dll` 的 DllMain 初始化失败：
/// `LoadLibraryExW` 返回 NULL、`GetLastError()=1114 ERROR_INIT_ROUTINE_FAILED`
/// —— 症状是"AI 静默回退传统算法"，与文件损坏 / 缺依赖极难区分（实测复现见项目记忆）。
///
/// 先按**完整路径**把正确版本加载进来，之后同名模块一律复用（Windows 按 base name 去重），
/// ORT 的 CRT 依赖便全部落到这一份上。
///
/// 失败一律忽略（拿不到就维持原行为），由上层把 ORT 的加载失败连同 Win32 错误码记进日志。
#[cfg(windows)]
fn pin_vc_runtime(dir: &Path) {
    let sys32 = std::env::var_os("SystemRoot").map(|r| PathBuf::from(r).join("System32"));
    for name in CRT_DLLS {
        let mut candidates = vec![dir.join(name)];
        if let Some(s) = &sys32 {
            candidates.push(s.join(name));
        }
        for p in candidates {
            if !p.is_file() {
                continue;
            }
            // SAFETY: 加载的是系统目录 / 组件自带的可信运行库，只执行其 DllMain。
            match unsafe { libloading::Library::new(&p) } {
                Ok(lib) => {
                    // ⚠️ 故意泄漏句柄：drop 会 FreeLibrary 把刚钉住的版本卸掉，
                    //    而这份绑定必须覆盖整个进程生命周期。
                    std::mem::forget(lib);
                    tracing::debug!(target: "launcher", "已固定 VC 运行库：{}", p.display());
                    break;
                }
                Err(e) => {
                    tracing::debug!(target: "launcher", "固定 VC 运行库失败（{}）：{e}", p.display());
                }
            }
        }
    }
}

#[cfg(not(windows))]
fn pin_vc_runtime(_dir: &Path) {}

/// 真正加载：`LoadLibrary` + 注册 env。只在首次用到 AI 时执行一次。
fn load_runtime(dir: Option<PathBuf>) -> Result<(), String> {
    let dir = dir
        .ok_or_else(|| be_err("ai_component_not_installed", ""))?;
    let dll = dir.join(RT_DLL);
    if !dll.is_file() {
        return Err(be_err("ai_runtime_dll_missing", &format!("{}", dll.display())));
    }
    // ⚠️ 顺序不能反：先把正确版本的 CRT 钉进进程，再加载 ORT（原因见 `pin_vc_runtime`）
    pin_vc_runtime(&dir);
    // ⚠️ `init_from` 必须是**第一个**被调用的 ort API：ort 内部 `setup_api()` 在 dylib
    // 缺失时是 `.expect(...)` → panic；这里返回 `Result`，不 panic。
    let builder = ort::init_from(&dll).map_err(|e| {
        // 带上 Win32 错误码：1114 = 依赖的 DllMain 初始化失败（运行库版本错配的典型症状）
        format!(
            "加载 AI 运行库失败：{e}（Win32 错误 {}）",
            std::io::Error::last_os_error()
        )
    })?;
    // `commit()` 把 env 选项注册进全局（返回 bool；已注册过时返回 false，无害）
    let _ = builder.commit();
    Ok(())
}

/// 懒加载 —— **仅首次真正用到 AI 时**才执行。
///
/// 未开启「AI 高清增强」的用户不会付出任何加载开销。结果缓存在 [`RtState::loaded`]。
fn ensure_runtime() -> bool {
    let Ok(mut g) = rt().lock() else { return false; };
    let res = match g.loaded.clone() {
        Some(r) => r,
        None => {
            let r = load_runtime(g.dir.clone());
            // 只在**首次**判定时打日志：`upscale` 对每张图都会走这里，不能每次都写
            match &r {
                Ok(()) => tracing::info!(target: "launcher", "AI 组件已加载，Real-ESRGAN 超分可用"),
                Err(e) => {
                    tracing::warn!(target: "launcher", "AI 高清增强不可用，将回退传统算法：{e}")
                }
            }
            g.loaded = Some(r.clone());
            r
        }
    };
    res.is_ok()
}

/// AI 运行库是否可用（**首次调用会触发一次加载尝试**，结果在进程内缓存）。
///
/// 供算法分派层提前判断：不可用时直接选传统算法，省掉"先选 AI、再由失败兜底"的绕路，
/// 也保证同一次构建里的分派是稳定的（见 `modcache::hd::smooth_or_ai`）。
/// 未开启 AI 增强的用户不该调用它（会白付一次加载开销）。
pub fn is_available() -> bool {
    ensure_runtime()
}

/// 用**指定模型文件**推理（实验台 / 多模型对比专用）。
///
/// 与 [`upscale`] 的唯一区别是模型来源：指哪个 onnx 就用哪个（**不看 `hd.ai.model`**，
/// 也不校验 sha256）—— 开发者把新模型丢进组件目录，重跑 `hd_compare` 就能立刻横向对比。
/// `ensure_runtime()` 仍会先确认运行库可用（ORT 缺失时绝不能触碰 ort API）。
///
/// ⚠️ session 池按模型分桶（见 [`with_session`]），多模型轮换不会串味；
/// 但生产路径仍只走 [`upscale`]。
#[allow(dead_code)] // 实验台 / 离线对比专用（生产路径一律走 `upscale`，见 `hd_compare`）
pub fn upscale_with_model(
    rgba: &[u8],
    w: u32,
    h: u32,
    factor: u32,
    model: &std::path::Path,
) -> Option<(Vec<u8>, u32, u32)> {
    if !ensure_runtime() {
        return None;
    }
    if factor != AI_FACTOR || w == 0 || h == 0 {
        return None;
    }
    let n = (w as usize) * (h as usize);
    if rgba.len() < n * 4 {
        return None;
    }
    let mut src = rgba[..n * 4].to_vec();
    dilate_colors(&mut src, w, h, DILATE_ROUNDS);

    let mut input = vec![0f32; 3 * n];
    for (i, px) in src.chunks_exact(4).enumerate() {
        input[i] = px[0] as f32 / 255.0;
        input[n + i] = px[1] as f32 / 255.0;
        input[2 * n + i] = px[2] as f32 / 255.0;
    }
    let tensor = Value::from_array(([1usize, 3, h as usize, w as usize], input)).ok()?;

    let (out_shape, data) = with_session(model, |s| {
        let in_name = s.inputs()[0].name().to_string();
        let out_name = s.outputs()[0].name().to_string();
        let outputs = s.run(ort::inputs![in_name.as_str() => tensor]).ok()?;
        let (shape, data) = outputs[out_name.as_str()].try_extract_tensor::<f32>().ok()?;
        Some((vec![shape[2] as u32, shape[3] as u32], data.to_vec()))
    })
    .flatten()?;
    let (ow, oh) = (out_shape[1], out_shape[0]);
    let on = (ow as usize) * (oh as usize);
    if data.len() < 3 * on {
        return None;
    }

    let mut out = vec![255u8; on * 4];
    for i in 0..on {
        for c in 0..3 {
            out[i * 4 + c] = (data[c * on + i].clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }
    // alpha 与透明区处理同 [`upscale`] 第 5 步
    if let Some(up) = crate::infra::upscale::upscale_rgba("xbrz", &rgba[..n * 4], w, h, AI_FACTOR) {
        let (a, _, _) = up;
        for i in 0..on.min(a.len() / 4) {
            let al = a[i * 4 + 3];
            out[i * 4 + 3] = al;
            if al < TRANSPARENT_ALPHA {
                out[i * 4] = a[i * 4];
                out[i * 4 + 1] = a[i * 4 + 1];
                out[i * 4 + 2] = a[i * 4 + 2];
            }
        }
    }
    Some((out, ow, oh))
}

/// 从**该模型**的池桶取一个 session 执行 `f`，用完归还（桶未满时）。
///
/// 桶空则新建（约 100 ms）。多个线程同时遇到空桶时会各自新建，可能短暂超过
/// [`POOL_MAX`] —— 无害（只是多占一点内存），换来实现上的简单与无死锁。
///
/// ## 线程模型（低影响的关键，勿去掉）
///
/// session 固定 **`intra_op = inter_op = 1` 线程**：ONNX Runtime 不显式设置时，每个
/// session 会用满**物理核**（线程池默认还允许 spin 空转）。而调用方
/// （`modcache::hd`）已在 rayon 池里按 CPU 档位并行多张图 —— 不限制时实际并发可达
/// `POOL_MAX × 物理核` 个计算线程，现象就是"生成高清缓存时整台电脑 CPU 被吃满"，
/// 外层的档位限速完全失效。单线程 session 把并行度**统一交还给外层线程池**
/// （并发度即限速，见 `modcache::batch::wiki_threads`）。
fn with_session<T>(model: &Path, f: impl FnOnce(&mut Session) -> T) -> Option<T> {
    let mutex = pool();
    // 先尝试从池里取（取出后立即释放锁，推理期间不阻塞其它线程）
    let taken = mutex
        .lock()
        .ok()
        .and_then(|mut g| g.get_mut(model).and_then(|v| v.pop()));
    let mut session = match taken {
        Some(s) => s,
        None => Session::builder()
            .ok()?
            .with_intra_threads(1)
            .ok()?
            .with_inter_threads(1)
            .ok()?
            // 不让 inter/intra 工作线程自旋空等：池里的 session 空闲时会持续占 CPU 与线程，
            // 关掉自旋让其休眠（配合批末 [`clear_sessions`] 回收，见函数文档）。
            .with_intra_op_spinning(false)
            .ok()?
            .with_inter_op_spinning(false)
            .ok()?
            // CPU arena 每次 `run` 后收缩回操作系统：ORT 默认只记高水位、绝不还内存，
            // 这正是"生成高清缓存后内存不回落"的主因（`cpu:0` = 收缩到底）。
            .with_config_entry("memory.enable_memory_arena_shrinkage", "cpu:0")
            .ok()?
            .commit_from_file(model)
            .ok()?,
    };
    let out = f(&mut session);
    // 归还（桶满则丢弃，交由 GC/析构释放）
    if let Ok(mut g) = mutex.lock() {
        let bucket = g.entry(model.to_path_buf()).or_default();
        if bucket.len() < POOL_MAX {
            bucket.push(session);
        }
    }
    Some(out)
}

/// AI 超分：**仅支持 `factor == 4`**，其余情形返回 `None`（调用方回退传统算法）。
///
/// `factor != 4` 不做"AI 4x 再缩放"：那样等于在 AI 结果上再插值一次，
/// 既慢又损质量，不如直接用传统算法放大。
///
/// 模型取 [`configure_model`] 注入的路径；**未注入 → 返回 `None`**（无默认模型）。
pub fn upscale(rgba: &[u8], w: u32, h: u32, factor: u32) -> Option<(Vec<u8>, u32, u32)> {
    // 运行库未就绪 → **立即**返回 None：绝不能触碰任何 ort API（会 panic）。
    // 调用方（`hd.rs::smooth_or_ai`）据此回退 Lanczos3。
    if !ensure_runtime() {
        return None;
    }
    if factor != AI_FACTOR || w == 0 || h == 0 {
        return None;
    }
    // 当前模型：**没有默认模型** —— 未注入（用户没配 `hd.ai.model`）时 AI 不生效。
    // 上层已保证注入的路径存在，这里再判一次 `is_file`（防配置文件在批次中途被改动）。
    let model = current_model().filter(|p| p.is_file())?;
    let n = (w as usize) * (h as usize);
    if rgba.len() < n * 4 {
        return None;
    }

    // 1) 透明区颜色膨胀：素材 alpha=0 处的 RGB 是抠图残留（H3 的 DEF 常带青蓝底框），
    //    直接送进模型会被当内容推理，放大后在半透明边缘露出彩色描边。
    let mut src = rgba[..n * 4].to_vec();
    dilate_colors(&mut src, w, h, DILATE_ROUNDS);

    // 2) RGB → NCHW float32（0-1 归一化，Real-ESRGAN 口径）
    let mut input = vec![0f32; 3 * n];
    for (i, px) in src.chunks_exact(4).enumerate() {
        input[i] = px[0] as f32 / 255.0;
        input[n + i] = px[1] as f32 / 255.0;
        input[2 * n + i] = px[2] as f32 / 255.0;
    }
    let tensor = Value::from_array(([1usize, 3, h as usize, w as usize], input)).ok()?;

    // 3) 推理 + 取输出
    let (out_shape, data) = with_session(&model, |s| {
        let in_name = s.inputs()[0].name().to_string();
        let out_name = s.outputs()[0].name().to_string();
        let outputs = s.run(ort::inputs![in_name.as_str() => tensor]).ok()?;
        let (shape, data) = outputs[out_name.as_str()]
            .try_extract_tensor::<f32>()
            .ok()?;
        Some((vec![shape[2] as u32, shape[3] as u32], data.to_vec()))
    })
    .flatten()?;
    let (ow, oh) = (out_shape[1], out_shape[0]);
    let on = (ow as usize) * (oh as usize);
    if data.len() < 3 * on {
        return None;
    }

    // 4) NCHW f32 → RGBA u8
    let mut out = vec![255u8; on * 4];
    for i in 0..on {
        for c in 0..3 {
            out[i * 4 + c] = (data[c * on + i].clamp(0.0, 1.0) * 255.0).round() as u8;
        }
    }

    // 5) alpha **不由 AI 生成**（模型只吃 RGB）：单独放大原 alpha。
    //    - 用 **xBRZ（边缘保持）**：细线 / 描边的 alpha 一旦被低通滤波（Lanczos3）平滑，
    //      线条会在半透明边缘发虚 —— 实测 `townborder` 类是 AI 输出发虚的主因之一；
    //    - 不用 AI 自己生成 alpha：避免"AI 猜出来的孔洞"。
    //    同时把**全透明区**的 RGB 还原为原图色（见 [`TRANSPARENT_ALPHA`]）：否则输入端的
    //    `dilate_colors` 填色会被模型当内容输出，表现为黑底 / 半透明边缘的彩边。
    if let Some(up) = crate::infra::upscale::upscale_rgba(
        "xbrz",
        &rgba[..n * 4],
        w,
        h,
        AI_FACTOR,
    ) {
        let (a, _, _) = up;
        for i in 0..on.min(a.len() / 4) {
            let al = a[i * 4 + 3];
            out[i * 4 + 3] = al;
            if al < TRANSPARENT_ALPHA {
                out[i * 4] = a[i * 4];
                out[i * 4 + 1] = a[i * 4 + 1];
                out[i * 4 + 2] = a[i * 4 + 2];
            }
        }
    }

    Some((out, ow, oh))
}

/// 把透明区（`alpha < 16`）的 RGB 用**邻接实心像素**逐圈填充（颜色膨胀，`rounds` 圈）。
///
/// 为什么必须做：H3 素材 `alpha=0` 处的 RGB 是抠图残留（DEF 帧的青蓝底框，抠图只置了
/// alpha）。直接把 RGB 送进 AI，等于让模型把垃圾色当内容推理，放大后会在**半透明边缘
/// 露出彩色描边**（实测：直接送 = 青边；用全局均值兜底 = 黄边）。只有让透明区颜色
/// 等于紧邻的实心色，AI 输出的边缘才与主体连续。
///
/// `rounds` 取 3：伪影只出现在边缘 1~3 像素的过渡带；更深处的透明区用全局均值兜底
/// （远离边缘，不影响观感），因此整体成本仍是 O(n)。
pub(crate) fn dilate_colors(rgba: &mut [u8], w: u32, h: u32, rounds: usize) {
    let (wi, hi) = (w as usize, h as usize);
    if wi == 0 || hi == 0 || rgba.len() < wi * hi * 4 {
        return;
    }
    let mut filled: Vec<bool> = rgba.chunks_exact(4).map(|p| p[3] >= 16).collect();
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
