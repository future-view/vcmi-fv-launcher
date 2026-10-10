//! 应用外壳与运行信息：窗口控制、资源管理器、分辨率、调试开关、编译/运行信息、前端日志。
//!
//! 引擎版本探测在 `vcmi::version`（`-v` → PE → 字节扫描三级），本层只做解包与 OS 版本读取。
//! `get_runtime_info` 会调用 `scan_mods` / `read_mod_settings` / `read_vcmi_settings` /
//! `get_compile_info` 汇总运行态字段（跨域只读调用）。

use crate::infra::berror::be_err;
use serde_json::{json, Value};
use std::process::Command;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
use tauri::{Manager, State};

use crate::state::AppPaths;
use crate::vcmi::paths::VcmiPaths;

use super::mod_settings::read_mod_settings;
use super::mods::scan_mods;
use super::vcmi_settings::read_vcmi_settings;

/// 前端日志上报：写入与后端同一日志管道（`tracing`），落盘到 `<exe>/.vcmi-fv-launcher/logs`。
///
/// - `level`：`error|warn|info|debug|trace`，非法值按 `info` 处理（与 `log_level` 同源解析）。
/// - `target`：前端模块名（如 `ModView`），为空则记 `frontend`。
/// - `message`：前端已做限流；此处再兜底截断，避免超长日志撑爆文件。
#[tauri::command]
pub fn log_frontend(level: String, target: String, message: String) {
    use crate::infra::logging::LogLevel;
    let msg: String = message.chars().take(4096).collect();
    let tgt = if target.trim().is_empty() {
        "frontend".to_string()
    } else {
        target
    };
    match LogLevel::parse(&level) {
        LogLevel::Error => tracing::error!(target: "frontend", "[{tgt}] {msg}"),
        LogLevel::Warn => tracing::warn!(target: "frontend", "[{tgt}] {msg}"),
        LogLevel::Debug => tracing::debug!(target: "frontend", "[{tgt}] {msg}"),
        LogLevel::Trace => tracing::trace!(target: "frontend", "[{tgt}] {msg}"),
        LogLevel::Info => tracing::info!(target: "frontend", "[{tgt}] {msg}"),
    }
}

/// 关闭当前主窗口
///
/// 「关闭时自动应用更新」挂在两处（幂等，谁先命中谁应用）：
/// 本命令（自定义 X 按钮路径）与 `lib.rs` 的 `on_window_event(CloseRequested)`
/// （覆盖 Alt+F4 / 系统级关闭）。判定与 spawn 逻辑见 `updater::apply::should_auto_apply_on_close`。
#[tauri::command]
pub fn close_window(app: tauri::AppHandle) -> Result<(), String> {
    if crate::updater::apply::should_auto_apply_on_close() {
        // 失败（如目录不可写）只记日志，仍正常关闭，pending 保留到下次启动再提示
        if let Err(e) = crate::updater::apply::apply_update(false) {
            tracing::warn!(target: "updater", "关闭时自动应用更新失败，回退普通关闭：{e}");
        }
    }
    app.get_webview_window("main")
        .ok_or_else(|| be_err("common_window_missing", ""))?
        .close()
        .map_err(|e| be_err("common_window_op_failed", &format!("{}", e)))
}

/// 最小化当前主窗口
#[tauri::command]
pub fn minimize_window(app: tauri::AppHandle) -> Result<(), String> {
    app.get_webview_window("main")
        .ok_or_else(|| be_err("common_window_missing", ""))?
        .minimize()
        .map_err(|e| be_err("common_window_op_failed", &format!("{}", e)))
}

/// 切换当前主窗口的最大化 / 还原状态。
///
/// 走系统能力而非手动 `set_size`：本窗口是无边框窗口（`decorations: false`），
/// tao 保留了 `WS_OVERLAPPEDWINDOW` 样式（靠 `WM_NCCALCSIZE` 去掉非客户区），
/// 因此系统最大化会按显示器**工作区**计算尺寸，不会盖住任务栏。
/// 前端不要自己维护最大化布尔值——还原几何由系统持有（见 `WindowControls` 的 `onResized`）。
#[tauri::command]
pub fn toggle_maximize_window(app: tauri::AppHandle) -> Result<(), String> {
    let win = app
        .get_webview_window("main")
        .ok_or_else(|| be_err("common_window_missing", ""))?;
    if win
        .is_maximized()
        .map_err(|e| be_err("common_window_op_failed", &format!("{}", e)))?
    {
        win.unmaximize()
            .map_err(|e| be_err("common_window_op_failed", &format!("{}", e)))
    } else {
        win.maximize()
            .map_err(|e| be_err("common_window_op_failed", &format!("{}", e)))
    }
}

/// 用系统资源管理器打开指定目录
#[tauri::command]
pub fn open_in_explorer(path: String) -> Result<(), String> {
    Command::new("explorer")
        .arg(&path)
        .spawn()
        .map_err(|e| be_err("cmd_open_failed", &format!("{}", e)))?;
    Ok(())
}

/// 主窗口 label：`tauri.conf.json` 的 `app.windows[0]`（未显式设 label → 默认 `main`）。
const MAIN_WINDOW: &str = "main";

/// 取主显示器的物理尺寸与缩放（取不到时返回 0/0，由 `compute_window_size` 兜底成设计稿尺寸）。
fn primary_monitor_of(win: &tauri::WebviewWindow) -> (u32, u32, f64) {
    match win.primary_monitor() {
        Ok(Some(m)) => (m.size().width, m.size().height, m.scale_factor()),
        _ => (0, 0, 1.0),
    }
}

/// **窗口显示前**定好初始几何：应用尺寸（持久化优先 / 否则按显示器自适应）→ 居中 → 显示。
///
/// 由 `lib.rs` 的 `setup` 调用，与 `tauri.conf.json` 的 `"visible": false` 配套使用：
/// 窗口创建时只有配置里的 1280×720，若先让它显示、再靠前端 `setSize` 放大，用户会看到一次
/// "先小后大"的跳变；且 Windows 的 `SetWindowPos` 以**左上角为锚**扩张，放大后的窗口会同时
/// 顶出右边界与下边界（1080p 上 1280×720 → 1632×918，右下各溢 32/18px）。
///
/// 除 `show()` 外的失败都只记日志（最坏情况退回配置默认尺寸）；**`show()` 必须执行** ——
/// `visible: false` 时没有别处会显示窗口。
pub fn apply_main_window_geometry(app: &tauri::AppHandle) {
    // ⚠️ `visible: false` 的失败模式是"应用彻底不可见"，所以找窗口要带兜底：
    // label 被改 / 配置多窗口时，宁可给非预期的窗口套几何，也不能让谁都不显示。
    let win = match app.get_webview_window(MAIN_WINDOW) {
        Some(w) => w,
        None => match app.webview_windows().into_values().next() {
            Some(w) => {
                tracing::warn!(target: "window", "未找到 {MAIN_WINDOW} 窗口，改用兜底窗口显示");
                w
            }
            None => {
                tracing::error!(target: "window", "没有任何窗口可显示（检查 tauri.conf.json）");
                return;
            }
        },
    };
    let saved = VcmiPaths::saved_window_size();
    let (mon_w, mon_h, scale) = primary_monitor_of(&win);
    let (width, height) =
        crate::infra::window::compute_window_size(saved, (mon_w, mon_h), scale);

    // 仅"按显示器自适应"这一次落盘：已有尺寸就不再写，免得每次启动都重写配置文件
    if saved.is_none() {
        if let Err(e) = VcmiPaths::store_window_size(width, height) {
            tracing::warn!(target: "window", "保存窗口尺寸失败（可忽略）：{e}");
        }
    }
    if let Err(e) = win.set_size(tauri::LogicalSize::new(width, height)) {
        tracing::warn!(target: "window", "设置窗口尺寸失败（沿用配置默认值）：{e}");
    }
    if let Err(e) = win.center() {
        tracing::warn!(target: "window", "窗口居中失败：{e}");
    }
    if let Err(e) = win.show() {
        tracing::error!(target: "window", "显示窗口失败：{e}");
    }
    tracing::info!(target: "window", "窗口初始几何 {width}×{height}（显示器 {mon_w}×{mon_h}@{scale}，持久化={saved:?}）");
}

/// 获取启动器主窗口所在显示器的工作区尺寸
#[tauri::command]
pub fn get_primary_monitor(window: tauri::Window) -> Result<Value, String> {
    let monitor = window
        .primary_monitor()
        .map_err(|e| be_err("cmd_monitor_get_failed", &format!("{}", e)))?
        .ok_or_else(|| be_err("common_monitor_missing", ""))?;
    let size = monitor.size();
    let scale = monitor.scale_factor();
    Ok(json!({
        "width": size.width,
        "height": size.height,
        "scale": scale,
    }))
}

/// 列出**全部**显示器（多显示器场景下用于推导高清素材倍率）。
///
/// 与 `get_primary_monitor` 的区别：后者只返回主显示器，而高清素材倍率必须取
/// **所有显示器中的最大物理宽度** —— 窗口可被拖到任意一块屏，便携包换机器后也会变化；
/// 只按主显示器算会让"拖到副屏"的高分屏用户拿到不够用的素材倍率。
///
/// ⚠️ 同 `get_primary_monitor`：`monitor.size()` 返回的是**全屏物理尺寸**（未扣任务栏）。
#[tauri::command]
pub fn list_monitors(window: tauri::Window) -> Result<Vec<Value>, String> {
    let monitors = window
        .available_monitors()
        .map_err(|e| be_err("cmd_monitor_list_failed", &format!("{}", e)))?;
    Ok(monitors
        .iter()
        .map(|m| {
            let size = m.size();
            json!({
                "width": size.width,
                "height": size.height,
                "scale": m.scale_factor(),
            })
        })
        .collect())
}

/// 获取编译/环境信息（游戏版本、内置核心 mod 版本、操作系统）
///
/// ⚠️ `async` + `spawn_blocking`：版本探测冷缓存时会 **spawn `VCMI_client.exe -v`**
/// （进程创建 + DLL 加载），并含注册表查询。模组页 / 设置弹窗首次打开恰是它的首个调用点，
/// 同步执行会表现为"页面刚可用就卡一下"（见 `docs/features/startup-responsiveness.zh-CN.md`）。
///
/// 另注：**不要把持锁做这件事** —— 其它命令都要同一把锁，先取出路径快照再下沉。
#[tauri::command]
pub async fn get_compile_info(state: State<'_, AppPaths>) -> Result<Value, String> {
    let (game_dir, mods_dirs) = {
        let paths = state.0.lock().expect("paths lock poisoned");
        (
            paths.game_dir.clone(),
            vec![paths.user_mods_dir(), paths.game_mods_dir()],
        )
    };
    tauri::async_runtime::spawn_blocking(move || {
        let info = crate::vcmi::version::engine_version(&game_dir, &mods_dirs);
        json!({
            "vcmiVersion": info.version,
            // 内置核心 mod 的资产版本（**不是**引擎版本，仅供诊断显示）
            "coreModVersion": info.core_mod_version,
            // 版本来源：cli / pe / scan / none（排查"版本是怎么读出来的"）
            "versionSource": info.source,
            "osVersion": read_os_version(),
        })
    })
    .await
    .map_err(|e| be_err("cmd_blocking_task_failed", &format!("{}", e)))
}

/// 读取 Windows 操作系统版本（**注册表优先**，locale 无关）。
///
/// 为什么废弃旧的 wmic / `cmd /C ver` 两条路径（2026-09-29 实测，Win11 25H2 显示 Unknown）：
/// - `wmic` 在 Windows 11 24H2+ **默认不再安装**，调用直接失败；
/// - `cmd /C ver` 在中文系统输出 `Microsoft Windows [版本 10.0.26100.1742]`，是 OEM(GBK)
///   编码，`from_utf8_lossy` 会把「版本」二字替换成 U+FFFD，本地化关键字匹配失败 → "Unknown"。
///
/// 关键事实：Windows 11 的注册表 `ProductName` **仍写 "Windows 10 …"**（微软已知行为），
/// 需按 `CurrentBuild >= 22000` 覆写为 "Windows 11"；`DisplayVersion` 即 "25H2" 这类版本名。
/// 输出形如：`Windows 11 Pro 25H2 (26100.1742)`。
fn read_os_version() -> String {
    #[cfg(windows)]
    {
        if let Some(s) = reg_query(r"HKLM\SOFTWARE\Microsoft\Windows NT\CurrentVersion") {
            let mut product = String::new();
            let mut display = String::new();
            let mut build = String::new();
            let mut ubr = String::new();
            for line in s.lines() {
                // 值行形如：`    ProductName    REG_SZ    Windows 10 Pro`（首行为键路径，列数不足被过滤）
                let cols: Vec<&str> = line.split_whitespace().collect();
                if cols.len() < 3 {
                    continue;
                }
                let data = cols[2..].join(" ");
                match cols[0] {
                    "ProductName" => product = data,
                    "DisplayVersion" => display = data,
                    "CurrentBuild" | "CurrentBuildNumber" => build = data,
                    "UBR" => ubr = data,
                    _ => {}
                }
            }
            let build_num = build.parse::<u32>().ok();
            // build ≥ 22000 = Windows 11（ProductName 不可信，需覆写）
            let is_win11 = build_num.map(|b| b >= 22000).unwrap_or(false);
            if !product.is_empty() || build_num.is_some() {
                let mut name = if product.is_empty() {
                    if is_win11 {
                        "Windows 11".to_string()
                    } else {
                        "Windows".to_string()
                    }
                } else if is_win11 {
                    product.replacen("Windows 10", "Windows 11", 1)
                } else {
                    product.clone()
                };
                if !display.is_empty() {
                    name.push(' ');
                    name.push_str(&display);
                }
                if let Some(b) = build_num {
                    name.push_str(&match parse_ubr(&ubr) {
                        Some(u) => format!(" ({b}.{u})"),
                        None => format!(" ({b})"),
                    });
                }
                if !name.trim().is_empty() {
                    return name;
                }
            }
        }
        // 回退：从 `cmd /C ver` 里**只提数字版本串**（不依赖任何本地化关键字）
        if let Some(v) = ver_output_version() {
            return v;
        }
        "Unknown".into()
    }
    #[cfg(not(windows))]
    {
        std::env::consts::OS.to_string()
    }
}

/// 查询注册表键的全部值（`reg query <key>`）；键不存在 / 命令不可用时返回 None。
#[cfg(windows)]
fn reg_query(key: &str) -> Option<String> {
    let out = Command::new("reg")
        .args(["query", key])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// `UBR` 是 REG_DWORD：reg.exe 可能显示为 `0x6ce`（十六进制），也可能是十进制字符串。
#[cfg(windows)]
fn parse_ubr(raw: &str) -> Option<u32> {
    let t = raw.trim();
    if t.is_empty() {
        return None;
    }
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        u32::from_str_radix(hex, 16).ok()
    } else {
        t.parse::<u32>().ok()
    }
}

/// 回退：从 `cmd /C ver` 输出提取 `主.次.构建[.修订]` 数字串（大小写 / 本地化无关，
/// 中文系统的 `[版本 10.0.26100.1742]` 同样能命中），并按构建号判定 Windows 11 / 10。
#[cfg(windows)]
fn ver_output_version() -> Option<String> {
    let out = Command::new("cmd")
        .args(["/C", "ver"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
    let s = String::from_utf8_lossy(&out.stdout);
    // 扫描最长的「数字 + 点」串
    let mut best = String::new();
    let mut cur = String::new();
    let flush = |cur: &mut String, best: &mut String| {
        if cur.split('.').count() >= 3 && cur.len() > best.len() {
            *best = cur.clone();
        }
        cur.clear();
    };
    for ch in s.chars() {
        if ch.is_ascii_digit() || (ch == '.' && !cur.is_empty() && !cur.ends_with('.')) {
            cur.push(ch);
        } else {
            flush(&mut cur, &mut best);
        }
    }
    flush(&mut cur, &mut best);
    if best.is_empty() {
        return None;
    }
    let build = best.split('.').nth(2).and_then(|b| b.parse::<u32>().ok());
    let head = match build {
        Some(b) if b >= 22000 => "Windows 11",
        _ => "Windows 10",
    };
    Some(format!("{head} ({best})"))
}

/// 返回当前是否为调试构建（tauri build --debug / tauri dev）。
/// 前端据此显示“可按 F12 打开开发者控制台”的提示。
#[tauri::command]
pub fn is_debug() -> bool {
    cfg!(debug_assertions)
}

/// 返回常用游戏分辨率候选列表（用于设置面板「分辨率」ButtonList）。
/// 包含主流 16:9 / 16:10 / 4:3 分辨率，并附带主显示器当前分辨率（若可获取）。
#[tauri::command]
pub fn list_resolutions(window: tauri::Window) -> Vec<Value> {
    let mut list: Vec<Value> = vec![
        json!({ "width": 1280, "height": 720 }),
        json!({ "width": 1366, "height": 768 }),
        json!({ "width": 1600, "height": 900 }),
        json!({ "width": 1920, "height": 1080 }),
        json!({ "width": 2560, "height": 1440 }),
        json!({ "width": 3840, "height": 2160 }),
        json!({ "width": 1280, "height": 800 }),
        json!({ "width": 1440, "height": 900 }),
        json!({ "width": 1680, "height": 1050 }),
        json!({ "width": 1024, "height": 768 }),
        json!({ "width": 1280, "height": 960 }),
        json!({ "width": 1600, "height": 1200 }),
    ];
    if let Ok(monitor) = window.primary_monitor() {
        if let Some(m) = monitor {
            let s = m.size();
            let w = s.width as i64;
            let h = s.height as i64;
            if w > 0 && h > 0 {
                // 始终标记「当前显示器原生分辨率」：命中内置项则就地打 `primary`，
                // 否则在头部插入。前端据此把该比例组排在首位、原生项挂「当前」徽标。
                let mut hit = false;
                for r in list.iter_mut() {
                    if r["width"] == w && r["height"] == h {
                        r["primary"] = json!(true);
                        hit = true;
                    }
                }
                if !hit {
                    list.insert(0, json!({ "width": w, "height": h, "primary": true }));
                }
            }
        }
    }
    list
}

/// 聚合运行态只读信息（设置面板「运行信息」弹窗使用）：
/// VCMI 版本、已安装模组数、已启用模组数、主显示器推荐分辨率。
///
/// `config_dir` 可选：传入领域配置目录时，**跟随作用域**的字段（最近地图 / 最近存档 /
/// 上次设置标签、游戏数据语言、setupCompleted、主窗口几何、已启用模组数）读该目录下的
/// `settings.json` / `modSettings.json`（与设置读写同一 configDir —— 领域隔离后领域目录
/// 有自己的运行态，读全局会显示过期值，见 `settings-panel-design` §六）。
/// 环境类字段（VCMI 版本 / 已安装模组数 / 推荐分辨率）与作用域无关，保持全局口径。
#[tauri::command]
pub async fn get_runtime_info(
    state: State<'_, AppPaths>,
    window: tauri::Window,
    config_dir: Option<String>,
) -> Result<Value, String> {
    // ⚠️ 本命令在**设置面板「运行信息」弹窗**打开时调用，内部含全量 `scan_mods` 与文件读取：
    // 改 `async` 前它在主线程执行，会把 WebView 消息循环整个冻结（弹窗"点了没反应"）。
    // 聚合语义保持"绝不失败"：子命令出错按 0 / 空值降级（原实现同样只取成功结果）。
    let installed = match scan_mods(state.clone()).await {
        Ok(v) => v.as_array().map(|a| a.len()).unwrap_or(0),
        Err(_) => 0,
    };

    let enabled = read_mod_settings(state.clone(), config_dir.clone())
        .await
        .ok()
        .and_then(|v| v.get("mods").and_then(|m| m.as_object()).cloned())
        .map(|mods| {
            mods.values()
                .filter(|m| m.get("enabled").and_then(|e| e.as_bool()).unwrap_or(false))
                .count()
        })
        .unwrap_or(0);

    let version = get_compile_info(state.clone())
        .await
        .ok()
        .and_then(|v| {
            v.get("vcmiVersion")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_else(|| "Unknown".to_string());
    let primary = window
        .primary_monitor()
        .ok()
        .flatten()
        .map(|m| {
            let s = m.size();
            json!({ "width": s.width, "height": s.height })
        })
        .unwrap_or(json!({ "width": 1920, "height": 1080 }));

    // 运行态只读字段（文档：不渲染，仅在运行信息弹窗展示）——按 config_dir 跟随作用域
    let settings = read_vcmi_settings(state.clone(), config_dir)
        .await
        .unwrap_or(json!({}));
    let general = settings.get("general").and_then(|g| g.as_object());
    let launcher = settings.get("launcher").and_then(|l| l.as_object());
    let last_map = general
        .and_then(|g| g.get("lastMap"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let last_save = general
        .and_then(|g| g.get("lastSave"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let last_tab = general
        .and_then(|g| g.get("lastSettingsTab"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let game_data_lang = general
        .and_then(|g| g.get("gameDataLanguage"))
        .and_then(|v| v.as_str())
        .unwrap_or("auto")
        .to_string();
    let setup_completed = launcher
        .and_then(|l| l.get("setupCompleted"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let main_window = launcher
        .and_then(|l| l.get("mainWindow"))
        .cloned()
        .unwrap_or(Value::Null);

    Ok(json!({
        "vcmiVersion": version,
        "modsInstalled": installed,
        "modsEnabled": enabled,
        "recommendedResolution": primary,
        "gameDataLanguage": game_data_lang,
        "lastMap": last_map,
        "lastSave": last_save,
        "lastSettingsTab": last_tab,
        "setupCompleted": setup_completed,
        "mainWindow": main_window,
    }))
}
