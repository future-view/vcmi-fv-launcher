//! 百科缓存命令：读取单 mod 缓存、按需合成城镇预览、批量构建与进度/取消、索引与缓存目录。
//!
//! 实现全部位于 `wiki::*`；本文件只负责缓存根推导、`AppHandle` 事件回调与 `Result` 转换。

use crate::infra::berror::be_err;
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::State;

use crate::state::AppPaths;
use crate::vcmi::paths::VcmiPaths;
use crate::vcmi::resource::{set_wiki_mod_paths, set_wiki_user_data_dir};
use crate::vcmi::scan::cached_mod_paths;
use crate::modcache::batch::{
    BatchWikiMod, BuildSlot, REALM_BUILD_CANCEL, REALM_BUILD_DONE, REALM_BUILD_TOTAL,
    WIKI_BUILD_CANCEL, WIKI_BUILD_DONE, WIKI_BUILD_TOTAL,
};
use crate::modcache::build::{probe_mod_cache, ModCacheState};
use crate::modcache::index::{build_all_wiki_inner, read_mod_wiki_inner};
use crate::modcache::preview::generate_town_preview_inner;

/// 读取单个城镇 mod 的 Wiki 缓存 JSON（命中缓存时不再解析原始 mod）。
///
/// ⚠️ `async` + `spawn_blocking`：读整份 `cache.json` 是重 IO，且列表页会并发调用数百次，
/// 同步命令会冻结 WebView 消息循环（见 `docs/features/startup-responsiveness.zh-CN.md`）。
#[tauri::command]
pub async fn read_mod_cache(_state: State<'_, AppPaths>, mod_id: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Value, String> {
        let cache_root =
            VcmiPaths::mods_cache_dir().unwrap_or_else(|| PathBuf::from(".vcmi-fv-launcher").join("mods"));
        Ok(read_mod_wiki_inner(&cache_root, &mod_id))
    })
    .await
    .map_err(|e| be_err("cmd_blocking_task_failed", &format!("{}", e)))?
}

/// 批量读取**多个模组的元数据**（`cache.json` 的 `modMeta`：多语言名/描述 + 作者/版本/类型/依赖）。
///
/// 存在的理由：`list_installed_mods` 会对**每个**扫到的模组各读一次整份 `cache.json`
/// （实测 652 个 mod ≈ 126 MB 全量 JSON 解析，是 mods / realms 页面首屏卡顿的头号元凶）。
/// 领域信息页只关心启用集合里那百来个模组，传 id 列表即可把开销降一个数量级。
///
/// 参数名 `ids` 为前端契约（改名会导致 `invoke` 静默 404，见 `check:invoke-parity`）。
#[tauri::command]
pub async fn read_mod_metas(_state: State<'_, AppPaths>, ids: Vec<String>) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Value, String> {
        let cache_root =
            VcmiPaths::mods_cache_dir().unwrap_or_else(|| PathBuf::from(".vcmi-fv-launcher").join("mods"));
        Ok(crate::modcache::cached_mod_metas(&cache_root, &ids))
    })
    .await
    .map_err(|e| be_err("cmd_blocking_task_failed", &format!("{}", e)))?
}

/// 批量读取**城镇卡片所需的最小字段**（`{modId, factionId, name, description, preview}`）。
///
/// 领域信息页的城镇网格只需要这 5 个字段；若沿用 `read_mod_cache` 读整份 `cache.json`，
/// 本例 86 个城镇合计 31 MB。一次 IPC 取回数十 KB 后，前端无需再做视口懒加载。
#[tauri::command]
pub async fn read_town_cards(_state: State<'_, AppPaths>, ids: Vec<String>) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Value, String> {
        let cache_root =
            VcmiPaths::mods_cache_dir().unwrap_or_else(|| PathBuf::from(".vcmi-fv-launcher").join("mods"));
        Ok(crate::modcache::cached_town_cards(&cache_root, &ids))
    })
    .await
    .map_err(|e| be_err("cmd_blocking_task_failed", &format!("{}", e)))?
}

/// 阵营巡礼条目（IPC 入参契约，前端 camelCase：`{ modId, factionId, modPath? }`）。
///
/// ⚠️ 必须显式 `rename_all = "camelCase"`：Tauri 只对**顶层命令参数名**做 camelCase 映射，
/// 嵌套结构体的字段不会自动转换 —— 缺了它 serde 会以「missing field `mod_id`」反序列化失败，
/// 命令整体 Err，前端 `catch` 后拿到空 slides，表现为「当前组合没有可展示的阵营」。
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ShowcaseEntry {
    mod_id: String,
    faction_id: String,
    #[serde(default)]
    mod_path: Option<String>,
}

/// 读取阵营巡礼 slide 载荷（见设计 §8.1）。`modPath` 由前端从已扫描 mod 列表带入，
/// 用于解析阵营音乐磁盘路径 / zip 条目；缺失时仅返回磁盘已落地音乐。
#[tauri::command]
pub fn read_showcase_slides(entries: Vec<ShowcaseEntry>) -> Result<Value, String> {
    let cache_root = VcmiPaths::mods_cache_dir().unwrap_or_else(|| PathBuf::from(".vcmi-fv-launcher").join("mods"));
    let game_dir = VcmiPaths::detect().game_dir;
    // 转成内部 Value 形态（避免 `Vec<Value>` 命令入参触发 Rust 2024 never-type 兼容性硬错误）
    let vals: Vec<Value> = entries
        .iter()
        .map(|e| {
            json!({
                "modId": e.mod_id,
                "factionId": e.faction_id,
                "modPath": e.mod_path,
            })
        })
        .collect();
    Ok(crate::modcache::showcase::read_showcase_slides_inner(
        &cache_root, &game_dir, &vals,
    ))
}

/// zip 内阵营音乐单条目抽取落盘，返回绝对路径（见设计 §8.1 / §4.5.3）。
/// 前端在进入某 slide 且 `musicZip` 非空时调用，落盘后再用返回路径播放。
#[tauri::command]
pub fn ensure_showcase_audio(mod_path: String, mod_id: String, rel: String) -> Result<Value, String> {
    let cache_root = VcmiPaths::mods_cache_dir().unwrap_or_else(|| PathBuf::from(".vcmi-fv-launcher").join("mods"));
    let mp = PathBuf::from(&mod_path);
    let path = crate::modcache::showcase::ensure_showcase_audio_inner(&cache_root, &mod_id, &mp, &rel);
    Ok(json!({ "ok": true, "path": path }))
}

/// 城镇预览合成（后台按需申请，结果缓存）：前端在 faction 页发现 preview 缺失时调用。
#[tauri::command]
pub fn generate_town_preview(
    _state: State<AppPaths>,
    mod_id: String,
    faction_id: String,
) -> Result<Value, String> {
    let cache_root = VcmiPaths::mods_cache_dir().unwrap_or_else(|| PathBuf::from(".vcmi-fv-launcher").join("mods"));
    Ok(generate_town_preview_inner(&cache_root, &mod_id, &faction_id))
}

/// 读取批量构建进度（前端 busy 时 400ms 轮询）
#[tauri::command]
pub fn cache_build_progress() -> Value {
    use std::sync::atomic::Ordering::Relaxed;
    json!({
        "done": WIKI_BUILD_DONE.load(Relaxed),
        "total": WIKI_BUILD_TOTAL.load(Relaxed),
    })
}

/// 请求取消正在进行的批量构建（已分发到线程的 mod 仍会跑完，之后不再构建新 mod）
#[tauri::command]
pub fn cancel_cache_build() -> Result<(), String> {
    WIKI_BUILD_CANCEL.store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

/* --------------------------- 领域加急队列（S1） --------------------------- */

/// 领域加急构建进度（进度页 400ms 轮询兜底；实时通道为事件 `realm-cache-build-progress`）。
///
/// 刻意与 `cache_build_progress` 分离：后者是启动后的**全局**后台构建，
/// 两者可能并存，共用一组计数器会让进度条互相覆盖。
#[tauri::command]
pub fn realm_cache_build_progress() -> Value {
    use std::sync::atomic::Ordering::Relaxed;
    json!({
        "done": REALM_BUILD_DONE.load(Relaxed),
        "total": REALM_BUILD_TOTAL.load(Relaxed),
    })
}

/// 取消领域加急构建（进度页「返回」按钮）：已分发到线程的 mod 仍会跑完。
#[tauri::command]
pub fn cancel_realm_cache_build() -> Result<(), String> {
    REALM_BUILD_CANCEL.store(true, std::sync::atomic::Ordering::Relaxed);
    Ok(())
}

/// **领域加急构建**：只为「当前领域所需素材」构建，进度走独立通道。
///
/// 与全局构建的三点刻意差异：
/// 1. **独立进度 / 取消**（`BuildSlot::realm`）—— 进度页只反映本领域；
/// 2. **turbo 档**（`throttle = Some(2)`）—— 用户正停在进度页等着，不再"空闲才提档"；
/// 3. **两阶段一次跑完**（`phase = Some(2)`）—— 同一批内先数据后素材，省一次 IPC 往返。
#[tauri::command]
pub async fn build_realm_cache_urgent(
    app: tauri::AppHandle,
    game_dir: String,
    mods: Vec<BatchWikiMod>,
    hd_rate: Option<u32>,
    // 基准物理倍率（= 屏幕物理宽 / 1280）：按用途分级算倍率必需，见 `hd::HdOptions::phys`
    hd_phys: Option<f64>,
    hd_upscaler: Option<String>,
    // **AI 高清增强**（`launcher.json` 的 `aiHdEnhance`，默认 false）：
    // 开启后插画档优先走 Real-ESRGAN 超分（仅 4x），失败静默回退 Lanczos3。
    hd_ai: Option<bool>,
) -> Result<Value, String> {
    let cache_root = VcmiPaths::mods_cache_dir()
        .unwrap_or_else(|| PathBuf::from(".vcmi-fv-launcher").join("mods"));
    let hd = crate::modcache::hd::HdOptions::new(hd_rate, hd_phys, hd_upscaler, hd_ai);
    // 每次新构建都从"未取消"开始
    REALM_BUILD_CANCEL.store(false, std::sync::atomic::Ordering::Relaxed);
    tauri::async_runtime::spawn_blocking(move || {
        // HD 素材来源的注入在 batch 入口统一做（见 `build_mods_cache_batch_inner_hd`）
        let on_progress = |done: usize, total: usize| {
            use tauri::Emitter;
            let _ = app.emit("realm-cache-build-progress", json!({ "done": done, "total": total }));
        };
        crate::modcache::batch::build_mods_cache_batch_inner_hd(
            Some(&on_progress),
            &game_dir,
            &mods,
            &cache_root,
            Some(2),
            Some(2),
            hd,
            BuildSlot::realm(),
        )
    })
    .await
    .map_err(|e| be_err("cmd_realm_expedite_build_failed", &format!("{}", e)))?
}

/// 检查**领域所需模组的缓存与素材**是否就绪（领域信息 / 阵营巡礼入口的守门人）。
///
/// 判据只关心"能不能显示"，不做全量校验（否则要逐个 stat 上万张图）：
/// 1. 该 mod 的 `cache.json` 存在；
/// 2. **抽样**图片字段（城镇 `preview`/`background`、前 2 个 creatures / heroes / buildings 的 `image`）
///    指向的文件在缓存目录里确实存在。
///
/// 注意：**抽样列表为空（纯数据 / 音乐 / UI 类 mod，或被 `ref:true` 标记、本就无独立图片实体的子模组）
/// 不算缺失**——这类 mod 抽不出任何图片，但巡礼 / 领域信息也不需要它的图；若误判为缺失，调用方每次打开
/// 都会跑一遍加急队列（进度页闪现）。只有"抽到了图片路径、但该文件在缓存目录里不存在"才计入 `missing`。
#[tauri::command]
pub fn check_realm_assets(ids: Vec<String>) -> Value {
    let cache_root = VcmiPaths::mods_cache_dir()
        .unwrap_or_else(|| PathBuf::from(".vcmi-fv-launcher").join("mods"));
    let mut missing: Vec<String> = Vec::new();
    for id in &ids {
        let dir = cache_root.join(crate::modcache::sanitize_id(id));
        let Ok(text) = fs::read_to_string(dir.join("cache.json")) else {
            missing.push(id.clone());
            continue;
        };
        let v = crate::infra::json::parse_vcmi_json(&text);
        let mut samples: Vec<String> = Vec::new();
        // 城镇图（领域信息卡片 / 巡礼幕布的必备素材）
        if let Some(arr) = v.get("factions").and_then(|x| x.as_array()) {
            for f in arr.iter().take(3) {
                for k in ["preview", "background"] {
                    if let Some(s) = f.get(k).and_then(|x| x.as_str()) {
                        if !s.is_empty() {
                            samples.push(s.to_string());
                        }
                    }
                }
            }
        }
        // 兵种 / 英雄 / 建筑各取前 2 个
        for key in ["creatures", "heroes", "buildings"] {
            if let Some(arr) = v.get(key).and_then(|x| x.as_array()) {
                for it in arr.iter().take(2) {
                    if let Some(s) = it.get("image").and_then(|x| x.as_str()) {
                        if !s.is_empty() {
                            samples.push(s.to_string());
                        }
                    }
                }
            }
        }
        // 抽样列表为空（纯数据/音乐/UI 类 mod，或被 `ref:true` 标记、本就无独立图片实体的子模组）
        // 视为无图可显示、直接判就绪——否则这类 mod 会永远 missing，导致"每次打开都先跑进度页"。
        // 只有"抽到了图片路径、但该文件在缓存目录里不存在"才计入缺失（真实需要构建的情况）。
        if samples.iter().any(|s| !cache_root.join(s).is_file()) {
            missing.push(id.clone());
        }
    }
    json!({ "ready": missing.is_empty(), "missing": missing, "checked": ids.len() })
}

/// 启动预检：本次是否**真的需要重建缓存**（供前端决定是否弹进度蒙层 / 是否整批跳过）。
///
/// 与真实构建**共用同一判定**（[`probe_mod_cache`]），故口径一致 —— 不会出现"预检说不用建、
/// 构建却重建"或反之的错位。成本 = 每个 mod 一次 `compute_source_hash`（`mod.json` 全文 +
/// `content.zip` 首尾 64 KiB 采样），`652 个 mod` 量级为秒级，远低于真实构建。
///
/// 返回 `needData` / `needAssets` 两类 modId 列表 + `indexReady`（索引是否已可用）。
/// 三者皆"空 / 就绪"时，前端**不置 busy、不弹蒙层**，直接用现有 `index.json` 渲染
/// （见 `src/lib/modcache/status.ts::ensureWikiCacheBuilt`）—— 这是"每次启动都弹重建进度条"的修复点。
#[tauri::command]
pub async fn wiki_cache_state(
    game_dir: String,
    mods: Vec<BatchWikiMod>,
) -> Result<Value, String> {
    let cache_root = VcmiPaths::mods_cache_dir()
        .unwrap_or_else(|| PathBuf::from(".vcmi-fv-launcher").join("mods"));
    tauri::async_runtime::spawn_blocking(move || -> Result<Value, String> {
        // 与构建入口（`batch::build_mods_cache_batch_inner_hd`）同源的进程级注入：
        // 判定依赖 user_data_dir（原版数据指纹）、跨模组资源路径、HD 来源指纹，缺一都会误判。
        let paths = VcmiPaths::detect();
        set_wiki_user_data_dir(&paths.user_dir);
        set_wiki_mod_paths(&cached_mod_paths(&paths));
        crate::vcmi::hd_edition::setup_from_config(
            VcmiPaths::read_launcher_config().as_ref(),
            Path::new(&game_dir),
        );
        // 索引是否已可用（存在且非空）：决定"零重建"时能否直接跳过构建。
        let index_ready = {
            let p = cache_root.join("index.json");
            fs::read_to_string(&p)
                .ok()
                .and_then(|t| serde_json::from_str::<Value>(&t).ok())
                .and_then(|v| v.get("mods").and_then(|m| m.as_array()).map(|a| !a.is_empty()))
                .unwrap_or(false)
        };
        let gd = Path::new(&game_dir);
        let mut need_data: Vec<Value> = Vec::new();
        let mut need_assets: Vec<Value> = Vec::new();
        for m in &mods {
            match probe_mod_cache(&m.mod_path, &m.mod_id, &cache_root, gd).state {
                // 数据失效 → 阶段一重建，阶段二必然跟着补素材
                ModCacheState::NeedData => {
                    need_data.push(json!(m.mod_id));
                    need_assets.push(json!(m.mod_id));
                }
                ModCacheState::NeedAssets => need_assets.push(json!(m.mod_id)),
                ModCacheState::Cached => {}
            }
        }
        Ok(json!({
            "ok": true,
            "total": mods.len(),
            "indexReady": index_ready,
            "needData": need_data,
            "needAssets": need_assets,
        }))
    })
    .await
    .map_err(|e| be_err("cmd_blocking_task_failed", &format!("{}", e)))?
}

/// 多线程批量构建 Wiki 缓存（rayon 并行，增量：hash 命中即跳过）。
/// 完成后重建 `mods/index.json`（否则百科无数据）。
/// 进度事件节流（每 8 个/最后一个推送），避免事件风暴拖垮前端。
///
/// `throttle` 控制并发度：rayon 全局池默认吃满所有逻辑核，对"启动后在后台默默构建"
/// 过于激进，故改用**局部线程池**（`install`）限制线程数，不影响其它 rayon 使用点。
#[tauri::command]
pub async fn build_mods_cache_batch(
    app: tauri::AppHandle,
    game_dir: String,
    mods: Vec<BatchWikiMod>,
    // Option 参数在字段缺失时由 serde 自动取 None（无需 #[serde(default)]），
    // 旧调用方不传即按 eco 档处理
    throttle: Option<u8>,
    // 构建阶段：0=仅数据（阶段一）；1=仅素材（阶段二）；None/2=两阶段。
    // 前端按「全部 mod 阶段一 → 全部 mod 阶段二」分批调用，使文本数据整体先就绪。
    phase: Option<u8>,
    // 高清素材**倍率上限**（由前端按「显示器物理宽 + 启动器缩放 + 显示放大比」推导，
    // 见 `src/lib/hd.ts::computeHdRate`）：缺省 / 1 = 不生成（低分屏无需高清）。
    // ⚠️ 倍率必须由**命令层**传入：显示器是 tauri 概念，modcache 层不得出现 tauri 类型。
    hd_rate: Option<u32>,
    // 基准物理倍率（= 最大显示器物理宽 / 1280）：逐图**按用途分级**算倍率必需，
    // 缺省时由 `hd_rate` 反推上界（见 `hd::HdOptions::new`）。
    hd_phys: Option<f64>,
    // 放大算法 id（缺省 = `infra::upscale` 的默认算法，即 xBRZ）
    hd_upscaler: Option<String>,
    // **AI 高清增强**（`launcher.json` 的 `aiHdEnhance`，默认 false）：
    // 开启后插画档优先走 Real-ESRGAN 超分（仅 4x），失败静默回退 Lanczos3。
    hd_ai: Option<bool>,
) -> Result<Value, String> {
    let cache_root = VcmiPaths::mods_cache_dir().unwrap_or_else(|| PathBuf::from(".vcmi-fv-launcher").join("mods"));
    // 高清选项（`hd_rate < 2` → None：完全不进高清分支）
    let hd = crate::modcache::hd::HdOptions::new(hd_rate, hd_phys, hd_upscaler, hd_ai);
    // 重活放后台阻塞线程池：rayon 占满 CPU 时若在主线程执行会卡死前端 IPC/渲染
    // 重置取消标志（每次新构建都从"未取消"开始）
    WIKI_BUILD_CANCEL.store(false, std::sync::atomic::Ordering::Relaxed);
    tauri::async_runtime::spawn_blocking(move || {
        // HD 素材来源的注入在 `batch::build_mods_cache_batch_inner_hd` 入口统一做（覆盖所有调用者）
        // 进度事件：wiki 层只接回调（不依赖 tauri），事件推送由命令层负责
        let on_progress = |done: usize, total: usize| {
            use tauri::Emitter;
            let _ = app.emit("wiki-build-progress", json!({ "done": done, "total": total }));
        };
        crate::modcache::batch::build_mods_cache_batch_inner_hd(
            Some(&on_progress),
            &game_dir,
            &mods,
            &cache_root,
            throttle,
            phase,
            hd,
            BuildSlot::global(),
        )
    })
    .await
    .map_err(|e| be_err("cmd_wiki_batch_build_failed", &format!("{}", e)))?
}

/// 批量构建多个城镇 mod 的 Wiki 缓存，并汇总生成 `mods/index.json` 索引。
/// 入参 mods 为 scan_mods 返回的列表（需含 modPath/id/type/factions/name/diskName/parentId）。
/// 重活（解析 mod + 解码图片 + webp 编码）放到后台阻塞线程，立即释放 Tauri 命令池，
/// 避免阻塞其他同步命令（如 read_mod_cache / 动画切换）导致前端卡死。
#[tauri::command]
pub async fn build_all_mod_cache(mods: Value) -> Result<Value, String> {
    let cache_root = VcmiPaths::mods_cache_dir()
        .unwrap_or_else(|| PathBuf::from(".vcmi-fv-launcher").join("mods"));
    let _ = fs::create_dir_all(&cache_root);
    tauri::async_runtime::spawn_blocking(move || {
        let res = build_all_wiki_inner(&mods, &cache_root);
        Ok(res) as Result<Value, String>
    })
    .await
    .map_err(|e| be_err("cmd_wiki_bg_build_failed", &format!("{}", e)))?
}

/// 读取 mods/index.json 索引。
#[tauri::command]
pub async fn read_cache_index(_state: State<'_, AppPaths>) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Value, String> {
        let cache_root =
            VcmiPaths::mods_cache_dir().unwrap_or_else(|| PathBuf::from(".vcmi-fv-launcher").join("mods"));
        let p = cache_root.join("index.json");
        if !p.is_file() {
            return Ok(json!({ "ok": false, "error": "notcached" }));
        }
        let text = fs::read_to_string(&p).map_err(|e| be_err("cmd_read_failed", &format!("{}", e)))?;
        let v = serde_json::from_str::<Value>(&text).map_err(|e| be_err("cmd_parse_failed", &format!("{}", e)))?;
        Ok(json!({ "ok": true, "data": v }))
    })
    .await
    .map_err(|e| be_err("cmd_blocking_task_failed", &format!("{}", e)))?
}

/// 返回 mods 缓存根目录的绝对路径（前端用于拼接 convertFileSrc 的完整图片路径）。
#[tauri::command]
pub async fn get_mods_cache_dir(_state: State<'_, AppPaths>) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<Value, String> {
        let cache_root =
            VcmiPaths::mods_cache_dir().unwrap_or_else(|| PathBuf::from(".vcmi-fv-launcher").join("mods"));
        let _ = fs::create_dir_all(&cache_root);
        Ok(json!({ "ok": true, "cacheDir": cache_root.to_string_lossy() }))
    })
    .await
    .map_err(|e| be_err("cmd_blocking_task_failed", &format!("{}", e)))?
}
