//! 批量构建的公共状态与并发档位。
//!
//! - [`WIKI_BUILD_DONE`] / [`WIKI_BUILD_TOTAL`]：进度（供 `cache_build_progress` 命令与事件推送读取）；
//! - [`WIKI_BUILD_CANCEL`]：取消标志（每个 mod 开始前检查）。
//!
//! ⚠️ 事件推送（`AppHandle::emit`）与线程池装配由命令层负责 —— 本模块**不出现 `tauri` 类型**。
//!
//! 依赖方向：可依赖 `infra` 与 `vcmi`（见 `docs/architecture/backend.zh-CN.md` §4.1）；
//! 进度事件通过 `on_progress` 回调外抛，本层不依赖 `tauri`。

use std::fs;
use std::path::Path;

use serde_json::{json, Value};

use crate::vcmi::paths::VcmiPaths;
use crate::vcmi::resource::{set_wiki_mod_paths, set_wiki_user_data_dir};
use crate::vcmi::scan::cached_mod_paths;
use crate::modcache::build::build_mod_wiki_phase;
use crate::modcache::sanitize_id;

/// 批量构建全局进度（供前端轮询兜底——事件在部分环境可能丢失）
pub static WIKI_BUILD_DONE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub static WIKI_BUILD_TOTAL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// 取消标志：由 `cancel_cache_build` 置位，job 内每个 mod 开始前检查，置位即跳过剩余
pub static WIKI_BUILD_CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// **领域加急队列**的独立进度。
///
/// ⚠️ 必须与全局进度分离：App 启动时的全局构建（`ensureWikiCacheBuilt`）与领域进度页
/// 可能并存，共用一组计数器会让两个进度条互相覆盖（用户看到"进度倒退/乱跳"）。
pub static REALM_BUILD_DONE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub static REALM_BUILD_TOTAL: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// 领域加急队列的取消标志（进度页「返回」→ 停止构建剩余 mod）
pub static REALM_BUILD_CANCEL: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 一次批次把**进度与取消**写到哪一组计数器。
///
/// - 默认（[`BuildSlot::global`]）：启动后的后台全局构建；
/// - [`BuildSlot::realm`]：领域加急队列 —— 进度页只反映"本领域所需素材"。
#[derive(Clone, Copy)]
pub struct BuildSlot {
    pub done: &'static std::sync::atomic::AtomicUsize,
    pub total: &'static std::sync::atomic::AtomicUsize,
    pub cancel: &'static std::sync::atomic::AtomicBool,
}

impl BuildSlot {
    /// 全局后台构建（默认）
    pub fn global() -> Self {
        Self {
            done: &WIKI_BUILD_DONE,
            total: &WIKI_BUILD_TOTAL,
            cancel: &WIKI_BUILD_CANCEL,
        }
    }

    /// 领域加急队列（独立进度通道）
    pub fn realm() -> Self {
        Self {
            done: &REALM_BUILD_DONE,
            total: &REALM_BUILD_TOTAL,
            cancel: &REALM_BUILD_CANCEL,
        }
    }
}

/// 系统空闲 ≥ 此秒数视为"用户离开"，提档一级
pub(crate) const IDLE_NORMAL_SECS: u64 = 30;
/// 系统空闲 ≥ 此秒数视为"长时间无人使用"，提到最高后台档
pub(crate) const IDLE_TURBO_SECS: u64 = 300;

/// 构建档位：**并发度即限速**（后台缓存构建不应影响用户正常使用电脑）。
/// - `0` / 缺省 = eco    ：`max(1, cpus/8)` 线程 —— 默认档，启动后后台静默构建
/// - `1`        = normal ：`max(1, cpus/4)`
/// - `2`        = turbo  ：`max(2, cpus/2)` —— 用户主动触发（手动重建 / 长时间空闲）
/// - `3`        = max    ：用满全部逻辑核 —— **仅验收/CI 场景**，会明显拖慢同时使用电脑
///
/// 环境变量 `VCMI_WIKI_THREADS` 可强制覆盖并发度（`0` = 用满全部逻辑核），
/// 供 CI / 验收在不改代码的前提下跑满机器；应用默认路径不受影响。
///
/// **空闲感知（默认档）**：ECO 的本意是不打扰正在用电脑的用户。用户长时间无键鼠输入时
/// 机器是空闲的，此时提档可把首次构建压到最短：
/// - 空闲 ≥ [`IDLE_TURBO_SECS`]（5 分钟）→ `cpus/2`
/// - 空闲 ≥ [`IDLE_NORMAL_SECS`]（30 秒）→ `cpus/4`
/// - 否则 → `cpus/8`（ECO）
///
/// 档位在**每块**构建开始时重估（建池时，块见 `build_mods_cache_batch_inner_hd` 的
/// `THROTTLE_BLOCK_MODS`），故用户中途回来 / 离开都会在下一块自动回落 / 提档
/// —— 长批（如高清阶段处理 600+ mod）若只在批开始时判一次，中途状态变化会失效。
pub(crate) fn wiki_threads(throttle: Option<u8>) -> usize {
    let n = std::thread::available_parallelism()
        .map(|v| v.get())
        .unwrap_or(4);
    if let Ok(v) = std::env::var("VCMI_WIKI_THREADS") {
        if let Ok(parsed) = v.trim().parse::<usize>() {
            return if parsed == 0 { n } else { parsed.clamp(1, n.max(1)) };
        }
    }
    match throttle.unwrap_or(0) {
        1 => (n / 4).max(1),
        2 => (n / 2).max(2),
        3 => n,
        _ => {
            let idle = idle_seconds_for_scheduling().unwrap_or(0);
            if idle >= IDLE_TURBO_SECS {
                (n / 2).max(2)
            } else if idle >= IDLE_NORMAL_SECS {
                (n / 4).max(1)
            } else {
                (n / 8).max(1)
            }
        }
    }
}

/// 每批诊断日志（并发档位 / 耗时构成 Top15）是否输出。
///
/// 默认输出——性能排查依赖它（见 `docs/features/modcache.zh-CN.md`）；
/// 用户若觉得日志啰嗦可设环境变量 `VCMI_WIKI_QUIET=1` 关闭。
/// 批次汇总行（`批量 wiki 构建完成：mods/失败/耗时`）**不受**此开关影响。
fn batch_log_verbose() -> bool {
    !matches!(std::env::var("VCMI_WIKI_QUIET").as_deref(), Ok("1") | Ok("true"))
}

/// 当前系统空闲秒数。
///
/// - `Some(n)`：n 秒无键鼠输入（来自 Win32 空闲检测，或环境变量覆盖）；
/// - `None`：**无法判定**（非 Windows / API 调用失败）——调用方按"用户在用"处理，保持 ECO。
///
/// `VCMI_WIKI_IDLE_SECS` 可覆盖（供验收 / 自测模拟空闲机器，与 `VCMI_WIKI_THREADS`
/// 同款的"环境变量旁路"约定）。
pub(crate) fn idle_seconds_for_scheduling() -> Option<u64> {
    if let Ok(v) = std::env::var("VCMI_WIKI_IDLE_SECS") {
        if let Ok(parsed) = v.trim().parse::<u64>() {
            return Some(parsed);
        }
    }
    crate::infra::idle::idle_seconds()
}

/// 批量构建输入项（modId + 原始 mod 目录 + scan_mods 原始节点剩余字段，供索引重建）
#[derive(serde::Deserialize)]
pub struct BatchWikiMod {
    #[serde(rename = "modId")]
    pub mod_id: String,
    #[serde(rename = "modPath")]
    pub mod_path: String,
    #[serde(flatten)]
    pub extra: Value,
}

/// 兼容入口（不做高清生成）：供 smoke_tests / 未来"仅重建素材"的调用点复用。
#[allow(dead_code)]
pub(crate) fn build_mods_cache_batch_inner(
    on_progress: Option<&(dyn Fn(usize, usize) + Sync)>,
    game_dir: &str,
    mods: &[BatchWikiMod],
    cache_root: &Path,
    throttle: Option<u8>,
    phase: Option<u8>,
) -> Result<Value, String> {
    build_mods_cache_batch_inner_hd(
        on_progress,
        game_dir,
        mods,
        cache_root,
        throttle,
        phase,
        None,
        BuildSlot::global(),
    )
}

/// 完整入口：`hd` 非空且 `phase` 涉及素材（1/2）时，为每个 mod **追加生成高清变体**（幂等）。
///
/// 高清与素材**同批并行**（同一 rayon job 内），因此"缓存构建完成"即意味着高清也已就绪
/// —— 领域信息页 / 阵营巡礼无需再等第二次构建。
pub(crate) fn build_mods_cache_batch_inner_hd(
    on_progress: Option<&(dyn Fn(usize, usize) + Sync)>,
    game_dir: &str,
    mods: &[BatchWikiMod],
    cache_root: &Path,
    throttle: Option<u8>,
    phase: Option<u8>,
    hd: Option<crate::modcache::hd::HdOptions>,
    // 进度与取消写到哪一组计数器（全局 / 领域加急，见 `BuildSlot`）
    slot: BuildSlot,
) -> Result<Value, String> {
    let phase = phase.unwrap_or(2);
    // 提取协议自检（仅 debug 构建 + 每批一次，开销可忽略）：
    // 内置 VCMI schema 的文本字段是否被协议表完全登记？任何"新增字段没进协议"都会在此处
    // 与 `cargo test`（smoke_tests::protocol）双重暴露，避免重演"提取缺漏靠用户发现"。
    #[cfg(debug_assertions)]
    if let Err(violations) = crate::modcache::schema::check_protocol_coverage() {
        eprintln!(
            "[protocol] 协议未登记 {} 个文本字段（补进 mod-cache-protocol.json）:\n{}",
            violations.len(),
            violations.join("\n")
        );
    }
    // 让资源回退解析也能搜到 `user_data_dir/Data`（便携运行原版数据常在此处）
    let paths = VcmiPaths::detect();
    set_wiki_user_data_dir(&paths.user_dir);
    // 让资源回退解析也能搜到其它已装模组内的素材（跨模组引用，如子模组 Fen 的精灵被父模组引用）
    set_wiki_mod_paths(&cached_mod_paths(&paths));
    // HD 素材来源（`hd-edition` 的 `.pak`）：按 `launcher.json` 决定（默认 auto = 检测到即用），
    // 每批注入一次（进程级）。放在**唯一构建入口**而非各命令层 —— 覆盖启动器构建、领域加急、
    // 重建脚本（`rebuild_all_with_core`）与未来调用者，避免"某入口忘了注入 → 产物缺 HD 素材"。
    crate::vcmi::hd_edition::setup_from_config(
        VcmiPaths::read_launcher_config().as_ref(),
        std::path::Path::new(game_dir),
    );
    let total = mods.len();
    // 每批独立统计：批开始清空全局累计，批结束打印的即为**本批**耗时构成
    crate::infra::metrics::reset_global();
    // **分块重估档位**的块大小（mod 数）。
    //
    // 档位若只在**批开始时**决定一次，长批（高清阶段的 rest 批 600+ mod、core 全量
    // 2.5 万+ 张图）中途"用户回来"不会回落 —— 空闲感知形同虚设。分块后每块建池前重估：
    // 用户回来最多再等一块（数十秒~几分钟）即降档，用户离开（≥30s → cpus/4、
    // ≥5min → cpus/2）同样自动提档。
    // 块不可太小：每块重建一次 rayon 池（毫秒级），太小会放大建池开销、也削弱负载均衡。
    const THROTTLE_BLOCK_MODS: usize = 8;
    let done = std::sync::atomic::AtomicUsize::new(0);
    slot.total.store(total, std::sync::atomic::Ordering::Relaxed);
    slot.done.store(0, std::sync::atomic::Ordering::Relaxed);
    let failed_previews: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());
    // 建筑图标失败计数（声明了 buildingsIcons 但多通道全失败）：批次结束 WARN 汇总，
    // 明细在各 mod cache.json 的 `iconFailures`（此前完全静默，用户只能肉眼发现缺图）
    let icon_failures_total = std::sync::atomic::AtomicUsize::new(0);
    // 高清素材生成统计（仅在 `hd` 非空时有意义）
    let hd_generated = std::sync::atomic::AtomicUsize::new(0);
    let hd_skipped = std::sync::atomic::AtomicUsize::new(0);
    let hd_failed = std::sync::atomic::AtomicUsize::new(0);
    let hd_pruned = std::sync::atomic::AtomicUsize::new(0);
    use rayon::prelude::*;
    // 单 mod 的完整处理（**并行与串行兜底共用**；逻辑与原逐 mod 闭包一致，不因分块改变语义）
    let run_mod = |m: &BatchWikiMod| -> Option<String> {
        // 取消请求：尽快停止（已分发到线程的 mod 仍会跑完，这里跳过后续）
        if slot.cancel.load(std::sync::atomic::Ordering::Relaxed) {
            let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            slot.done.store(n, std::sync::atomic::Ordering::Relaxed);
            return None;
        }
        // mod 级耗时埋点（debug）：便于开启 debug 级别分析缓存构建性能
        let t0 = std::time::Instant::now();
        let r = build_mod_wiki_phase(
            &m.mod_path,
            &m.mod_id,
            &cache_root,
            paths.game_dir.as_path(),
            phase,
        );
        // 高清变体生成（幂等）：仅素材阶段（1/2）且 rate≥2。
        // 计在该 mod 的总耗时内，日志里能看出高清带来的真实增量。
        if let Some(opt) = hd.as_ref() {
            if phase == 1 || phase == 2 {
                let st = crate::modcache::hd::generate_for_mod(&cache_root, &m.mod_id, opt);
                if !st.is_empty() {
                    tracing::debug!(
                        target: "wiki",
                        "mod={} 高清生成 generated={} skipped={} failed={}",
                        m.mod_id, st.generated, st.skipped, st.failed
                    );
                }
                hd_generated.fetch_add(st.generated, std::sync::atomic::Ordering::Relaxed);
                hd_skipped.fetch_add(st.skipped, std::sync::atomic::Ordering::Relaxed);
                hd_failed.fetch_add(st.failed, std::sync::atomic::Ordering::Relaxed);
                hd_pruned.fetch_add(st.pruned, std::sync::atomic::Ordering::Relaxed);
            }
        }
        let elapsed_ms = t0.elapsed().as_millis();
        tracing::debug!(
            target: "wiki",
            "mod={} ok={} cached={} 耗时={}ms",
            m.mod_id,
            r.get("ok").and_then(|x| x.as_bool()) == Some(true),
            r.get("cached").and_then(|x| x.as_bool()).unwrap_or(false),
            elapsed_ms
        );
        let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        slot.done.store(n, std::sync::atomic::Ordering::Relaxed);
        if n % 8 == 0 || n == total {
            if let Some(cb) = on_progress {
                cb(n, total);
            }
        }
        // 预览合成失败的 faction 汇总（modId:fid），便于诊断
        if let Some(fps) = r.get("failedPreviews").and_then(|x| x.as_array()) {
            for fid in fps {
                if let Some(s) = fid.as_str() {
                    failed_previews.lock().unwrap().push(format!("{}:{}", m.mod_id, s));
                }
            }
        }
        // 建筑图标失败计数（明细已在 cache.json 的 iconFailures 中）
        if let Some(n) = r.get("failedIcons").and_then(|x| x.as_u64()) {
            if n > 0 {
                icon_failures_total.fetch_add(n as usize, std::sync::atomic::Ordering::Relaxed);
                tracing::debug!(target: "wiki", "mod={} 建筑图标失败 {} 个", m.mod_id, n);
            }
        }
        if r.get("ok").and_then(|x| x.as_bool()) != Some(true) {
            Some(m.mod_id.clone())
        } else {
            None
        }
    };
    // 分块构建：**每块建池前重估档位**（空闲感知，见 `THROTTLE_BLOCK_MODS` / `wiki_threads`）
    // —— 用户中途回来 / 离开都在下一块生效，不必等整批结束。
    let t_batch = std::time::Instant::now();
    let mut failed: Vec<String> = Vec::new();
    let mut last_threads = 0usize;
    let block_total = mods.len().div_ceil(THROTTLE_BLOCK_MODS).max(1);
    for (bi, chunk) in mods.chunks(THROTTLE_BLOCK_MODS).enumerate() {
        // 取消请求：整块跳过（`run_mod` 内每个 mod 开始前的检查仍在，双保险）
        if slot.cancel.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        let threads = wiki_threads(throttle);
        if batch_log_verbose() && threads != last_threads {
            tracing::info!(
                target: "wiki",
                "并发档位：线程={}（块 {}/{}，throttle={:?} 空闲={:?}）",
                threads,
                bi + 1,
                block_total,
                throttle,
                idle_seconds_for_scheduling()
            );
        }
        last_threads = threads;
        // 局部线程池：并发度即限速（见 wiki_threads）。
        // ⚠️ 建池失败（极少见）→ **串行兜底**，绝不回退 rayon 全局池：全局池会用满全部逻辑核，
        //    与"后台构建不影响用户正常使用电脑"的前提直接冲突（宁可慢，不可吃满）。
        let partial: Vec<String> = match rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
        {
            Ok(pool) => pool.install(|| chunk.par_iter().filter_map(|m| run_mod(m)).collect()),
            Err(_) => chunk.iter().filter_map(|m| run_mod(m)).collect(),
        };
        failed.extend(partial);
    }
    // 批次级耗时埋点（info）：直接给出整批缓存构建的总时长与失败数
    tracing::info!(
        target: "wiki",
        "批量 wiki 构建完成：mods={} 失败={} 耗时={}ms",
        total,
        failed.len(),
        t_batch.elapsed().as_millis()
    );
    // 建筑图标失败汇总（WARN）：>0 时提示，避免"凭空少图"无法排查
    let icon_failed = icon_failures_total.load(std::sync::atomic::Ordering::Relaxed);
    if icon_failed > 0 {
        tracing::warn!(
            target: "wiki",
            "建筑图标提取失败 {} 个（建筑声明了 buildingsIcons 但多通道全失败；明细见各 mod cache.json 的 iconFailures）",
            icon_failed
        );
    }
    // 本批耗时构成（Top 15）：定位热点用，例如 `lod.open` 的读取总量、
    // `res.dir_scan` 的调用次数、`town.preview` 的合成耗时。
    if batch_log_verbose() {
        tracing::info!(
            target: "wiki",
            "本批耗时构成 Top15（phase={} 线程={}）：{}",
            phase,
            last_threads.max(1),
            crate::infra::metrics::format_global_top(15)
        );
    }

    // 重建 index.json：合并已有索引，避免分多批次调用（如 ensureWikiCacheBuilt
    // 先 Town 后 rest）时互相覆盖——否则首批构建的 mod 会从最终索引中丢失。
    // 以 modId 为 key 合并：本批条目覆盖同名，其他批次条目保留。
    //
    // ⚠️ **刻意不做差集清理（勿"顺手修好"）**：索引与磁盘产物都只增不删，
    //    卸载模组 / 切换领域后的条目会永久留在 index.json 与 `<mod>/` 目录里。原因：
    //    缓存根 `<exe 目录>/.vcmi-fv-launcher/mods` **不含游戏目录 hash**，被所有游戏安装
    //    与所有领域共用；领域激活会改写 `dirs.json` 的 userDataPath，使某些模组（borrowed
    //    目录联接 / bundled 随包）只在对应领域可见。按"本次批次"做差集会把**另一个领域
    //    仍在用**的缓存删掉，下次进入又要重建（还会在 I/O 与 CPU 上反复付费）。
    //    ⇒ 结论：index.json 是「已缓存清单」，**不是**「安装清单」。
    //      "哪些还有效"由**读侧**判定：前端把索引与实时扫描（`scan_mods`）求交后再渲染
    //      （见 `src/lib/modcache/buildTargets.ts::visibleModKeySet` 与 WikiView）。
    //      需要回收磁盘时，只应走用户显式确认的手动入口，不要放进这条构建路径。
    let idx_path = cache_root.join("index.json");
    let mut merged: std::collections::HashMap<String, Value> = std::collections::HashMap::new();
    if let Ok(text) = fs::read_to_string(&idx_path) {
        if let Ok(v) = serde_json::from_str::<Value>(&text) {
            if let Some(arr) = v.get("mods").and_then(|x| x.as_array()) {
                for m in arr {
                    if let Some(id) = m.get("modId").and_then(|x| x.as_str()) {
                        merged.insert(id.to_string(), m.clone());
                    }
                }
            }
        }
    }
    for m in mods.iter() {
        let data_path = cache_root.join(format!("{}/cache.json", sanitize_id(&m.mod_id)));
        let extra_name = m.extra.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
        // 原版伪模组（core / core.<faction>）没有 mod.json，名称从数据派生（见 `index::core_mod_display_name`）
        // —— 扫描期 extra.name 对它们不再是名称来源。城镇名要读缓存里的 `factions[0].name`
        // （游戏配置 + 翻译 mod 的全语言名），故**仅对 core.<faction>** 先读一次缓存；
        // 其余 mod 的缓存读取仍在下方统一进行，不产生额外 I/O。
        let town_names = if crate::vcmi::h3core::parse_core_mod(&m.mod_id).1.is_some() {
            fs::read_to_string(&data_path)
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .and_then(|v| v.pointer("/factions/0/name").and_then(|x| x.as_object()).cloned())
        } else {
            None
        };
        let core_name = crate::modcache::index::core_mod_display_name(&m.mod_id, town_names.as_ref());
        // 多语言模组名：常规模组优先取 cache.json 的 modMeta.name（与 mods/realms 页同源）；
        // 缺失时回退 extra.name 单串（中英文同值），使 wiki 模组列表随界面语言切换。
        let mut mod_name = match &core_name {
            Some(n) => n.clone(),
            None => json!({ "chinese": extra_name.clone(), "english": extra_name }),
        };
        let mut counts = serde_json::Map::new();
        let mut h3_missing = false;
        if let Ok(text) = fs::read_to_string(&data_path) {
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                // 原版 H3 数据文件缺失标记：前端据此提示用户补放 Data/*.lod
                h3_missing = v.get("h3DataMissing").and_then(|x| x.as_bool()).unwrap_or(false);
                // 多语言模组名：从已构建缓存的 modMeta 提取（见 build/mod.rs::build_mod_meta）。
                // 原版伪模组的名称来自数据派生，不被 modMeta（其自身无 mod.json）覆盖。
                if core_name.is_none() {
                    if let Some(meta) = v.get("modMeta").and_then(|x| x.as_object()) {
                        if let Some(n) = meta.get("name").and_then(|x| x.as_object()) {
                            if !n.is_empty() {
                                mod_name = Value::Object(n.clone());
                            }
                        }
                    }
                }
                // spells/skills 计数拆分：builtin（原版注入）与自定义分开报，
                // 前端聚合时 builtin 只计一次，避免 81×N 虚高
                let builtin_of = |key: &str| -> usize {
                    v.get(key)
                        .and_then(|x| x.as_array())
                        .map(|a| {
                            a.iter()
                                .filter(|s| s.get("builtin").and_then(|b| b.as_bool()).unwrap_or(false))
                                .count()
                        })
                        .unwrap_or(0)
                };
                for key in ["factions", "creatures", "heroes", "spells", "artifacts", "skills", "buildings"] {
                    // 跨 mod 引用条目（构建期标记 `ref:true`，见 build/mod.rs::mark_ref_entries）
                    // 与 builtin 一样不参与计数，保证侧栏计数与列表口径一致。
                    let n = v
                        .get(key)
                        .and_then(|x| x.as_array())
                        .map(|a| {
                            a.iter()
                                .filter(|s| !s.get("ref").and_then(|b| b.as_bool()).unwrap_or(false))
                                .count()
                        })
                        .unwrap_or(0);
                    let n = if key == "spells" {
                        n - builtin_of("spells")
                    } else if key == "skills" {
                        n - builtin_of("skills")
                    } else {
                        n
                    };
                    counts.insert(key.to_string(), json!(n));
                }
                counts.insert("spellBuiltin".to_string(), json!(builtin_of("spells")));
                counts.insert("skillBuiltin".to_string(), json!(builtin_of("skills")));
                // 跨 mod 引用条目数（`<modId>:<entityId>` 补丁语法，构建期已标记 `ref:true`）：
                // 诊断口径，前端 collectCounts 已跳过该键，不参与任何类目展示。
                let ref_n: usize = [
                    "factions", "creatures", "heroes", "spells", "artifacts", "skills", "buildings",
                ]
                .iter()
                .map(|k| {
                    v.get(*k)
                        .and_then(|x| x.as_array())
                        .map(|a| {
                            a.iter()
                                .filter(|s| s.get("ref").and_then(|b| b.as_bool()).unwrap_or(false))
                                .count()
                        })
                        .unwrap_or(0)
                })
                .sum();
                counts.insert("ref".to_string(), json!(ref_n));
            }
        }
        let entry = json!({
            "modId": m.mod_id,
            "diskName": m.extra.get("diskName").and_then(|x| x.as_str()).unwrap_or(""),
            "parentId": m.extra.get("parentId").and_then(|x| x.as_str()).unwrap_or(""),
            "type": m.extra.get("type").and_then(|x| x.as_str()).unwrap_or(""),
            "name": mod_name,
            "encrypted": m.extra.get("contentZipEncrypted").and_then(|x| x.as_bool()).unwrap_or(false),
            "h3DataMissing": h3_missing,
            "cacheDir": format!("mods/{}", sanitize_id(&m.mod_id)),
            "contentCount": Value::Object(counts),
        });
        // 本批条目覆盖已有同名条目（modId 唯一），其余批次条目保留
        if let Some(id) = entry.get("modId").and_then(|x| x.as_str()) {
            merged.insert(id.to_string(), entry);
        }
    }
    let index_mods: Vec<Value> = merged.into_values().collect();
    let index = json!({
        "generatedAt": crate::infra::time::now_millis(),
        "version": 2,
        "mods": index_mods,
    });
    let idx_ok = fs::write(&idx_path, serde_json::to_string_pretty(&index).unwrap()).is_ok();

    // 高清生成汇总（info）：仅在本次确实带了 hd 选项时打印，便于核对"高清是否真的跑了"
    let (hd_gen, hd_skip, hd_fail, hd_prune) = (
        hd_generated.load(std::sync::atomic::Ordering::Relaxed),
        hd_skipped.load(std::sync::atomic::Ordering::Relaxed),
        hd_failed.load(std::sync::atomic::Ordering::Relaxed),
        hd_pruned.load(std::sync::atomic::Ordering::Relaxed),
    );
    if hd.is_some() && (hd_gen > 0 || hd_fail > 0 || hd_prune > 0) {
        tracing::info!(
            target: "wiki",
            "高清素材生成完成：新增={} 跳过={} 失败={} 清理旧倍率={}（倍率由显示器推导）",
            hd_gen,
            hd_skip,
            hd_fail,
            hd_prune
        );
    }

    // 构建收尾**主动回收内存**：素材 / 高清阶段会涨出两类完成后不该继续占用的大对象 ——
    // ① AI session 池（模型权重 + ORT arena 高水位，进程级缓存）；② HD 来源的 sheet 解码
    // 缓存（≤128 MB 已解码图集）。不主动释放,构建后任务管理器占用会停在高水位不回落。
    // 仅在涉及素材的阶段（phase 1/2）执行;AI 未启用时 clear_sessions 是 no-op。
    if phase == 1 || phase == 2 {
        crate::infra::ai::clear_sessions();
        crate::vcmi::hd_edition::trim_source_cache();
    }

    Ok(json!({
        "ok": true,
        "total": total,
        "failed": failed,
        "indexed": idx_ok,
        "failedPreviews": failed_previews.into_inner().unwrap_or_default(),
        "hd": { "generated": hd_gen, "skipped": hd_skip, "failed": hd_fail, "pruned": hd_prune },
    }))
}
