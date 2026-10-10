//! 外挂运行时组件（runtime）—— **不随每次版本更新变化的外部依赖**的通用管理。
//!
//! ## 为什么需要
//!
//! 像 ONNX Runtime 这类原生依赖，**静态链进 exe 会让体积翻倍**（21.7MB → 42.8MB），
//! 而版本更新是「整包替换 / 解压覆盖」→ 每次更新都得重下。外置成组件后：
//! - 默认包不含它（体积小、更新快）；
//! - 组件只装一次，常驻磁盘；
//! - 真要升级时随某次更新下发即可（zip 更新天然支持多文件）。
//!
//! ## 三种分发形态（共用同一份逻辑）
//!
//! | 形态 | 谁放文件 | launcher.json |
//! |---|---|---|
//! | 便携内置 | 发行方把目录放进便携包 | 可不配（走约定目录） |
//! | 安装包内置 | `tauri build -c tauri.ai-runtime.conf.json` | 可不配 |
//! | 远程下载 | 用户点「安装」 | 配 `runtime.components.<id>.package` |
//!
//! ## 配置契约（launcher.json）
//!
//! ```json5
//! "runtime": {
//!   "components": {
//!     "ai": { … }
//!   }
//! }
//! ```
//!
//! 内层展开：
//!
//! ```json5
//! "runtime": { "components": {
//!   "ai": {
//!     "dir": ".vcmi-fv-launcher/runtime/ai",   // 可选；缺省即这个值
//!     "package": { "url": "https://…", "sha256": "…", "size": 10485760, "version": "1.28.0" }
//!   }
//! } }
//! ```
//!
//! ⚠️ **文件清单不进配置**：组件需要哪些文件、各自 sha256 是多少由本模块的
//! [`COMPONENTS`] 常量定义 —— 那是代码强依赖，让用户填没意义，填错反而有害。
//! 配置只解决「放哪」与「从哪来」。
//!
//! ## 分层
//!
//! 依赖方向 `commands → runtime → {vcmi, infra}`；本层**不含 tauri 类型**（事件 emit
//! 由 `commands` 层负责）。⚠️ 本层**不读 launcher.json**，配置由 `lib.rs` 经 [`configure`] 注入。

pub mod install;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::Serialize;

use crate::infra::hash::sha256_file;
use crate::vcmi::config::{RuntimeComponentCfg, RuntimePackage};
use crate::vcmi::paths::VcmiPaths;

use crate::infra::berror::be_err;

/// 组件 id：AI 超分（ONNX Runtime + Real-ESRGAN 模型）
pub const AI: &str = "ai";

/// 组件内的一个文件（**代码常量**，不暴露给配置）
pub struct ComponentFile {
    pub name: &'static str,
    /// 期望 sha256（小写 hex）。版本错配时拒绝使用，避免原生库 ABI 不匹配导致崩溃。
    pub sha256: &'static str,
}

/// 组件定义。新增组件只需往 [`COMPONENTS`] 加一项。
pub struct ComponentDef {
    pub id: &'static str,
    /// **必需文件**：齐备且 sha256 全中，组件才算"已安装/可用"（见 [`is_ready`]）。
    pub files: &'static [ComponentFile],
    /// **随包分发的可选模型**：安装时照常解压并校验（见 `install.rs`），
    /// 但**缺失不影响组件可用性** —— 用户可只保留要用的模型，或自备模型。
    /// 这里的 sha256 用于给扫描结果标注"已知/已验证"（见 [`scan_ai_models`]）。
    pub models: &'static [ComponentFile],
}

/// 已注册的组件清单
///
/// ⚠️ **清单只列"代码强依赖"的文件**：ORT 运行库 / 模型缺失或版本错配会让加载崩溃或静默失效，
/// 必须逐文件 sha256 校验。而 AI 版包在组件目录里**额外**分发的 5 个 VC++ 运行库
/// （`vcruntime140*` / `msvcp140*`，源 `src-tauri/resources/runtime/ai/`）**不在此列**：
/// 它们只是"依赖解析顺序"的补丁（见 `infra::ai::pin_vc_runtime`），缺失时自动回落 System32，
/// 属可选项 —— 纳入校验反而会把已有用户的 3 文件组件判成"未安装"。
pub const COMPONENTS: &[ComponentDef] = &[ComponentDef {
    id: AI,
    // 来源：官方 ORT 1.28.0 win-x64（onnxruntime-win-x64-1.28.0.zip）
    files: &[
        ComponentFile {
            name: "onnxruntime.dll",
            sha256: "18370c375f07357fa5874344a9d9ac17e6b6fe1eb18b1dd209d79483b4470257",
        },
        ComponentFile {
            name: "onnxruntime_providers_shared.dll",
            sha256: "599629fa643707defe9156140ae5edd73531f221aa97b7585b1c9bb0a93586f8",
        },
    ],
    // 随包模型 ×2（`resources/runtime/ai/`，均为 Real-ESRGAN 4x）—— **可选**：
    // 组件可用性只看 `files`；这里用于（a）远程安装照常解压校验，（b）扫描时标注"已验证"。
    //   - `realesrgan_anime6b.onnx`（**推荐模型**，RRDB 6-block，17.5MB）——
    //     识图对照（`hd_compare` 的 `_model.png`）显示其在插画/幕布/图标上细节与锐度全面
    //     优于 animevideov3，且结构不变形；色偏更明显由 mix+USM 链路压制；
    //   - `realesr-animevideo-v3-x4.onnx`（轻量备选，2.4MB）。
    // ⚠️ 模型**不是推理默认值**：`hd.ai.model` 缺省 = AI 不生效（无默认，见 `vcmi::config::AiCfg`）。
    models: &[
        ComponentFile {
            name: "realesrgan_anime6b.onnx",
            sha256: "45bd54934aeabe8df744c8fdacb9e8846c9b55cb4e60c499db77405d1625a667",
        },
        ComponentFile {
            name: "realesr-animevideo-v3-x4.onnx",
            sha256: "00ece3ac21c43ee31459216b5174b2cea0c5325044c5142aeb840f4890e175ff",
        },
    ],
}];

/// 推荐的 AI 模型（UI 的"恢复推荐"用）—— **不是推理默认值**：
/// 不写 `hd.ai.model` 时 AI 一律不生效（见 `vcmi::config::AiCfg`）。
pub const RECOMMENDED_AI_MODEL: &str = "realesrgan_anime6b.onnx";

/// 取组件定义（未知 id → None）
pub fn def(id: &str) -> Option<&'static ComponentDef> {
    COMPONENTS.iter().find(|c| c.id == id)
}

// ── 配置注入 ──────────────────────────────────────────────────────────────
// ⚠️ 本层不读 launcher.json：由 lib.rs 启动时注入（保持 runtime → vcmi 的单向依赖）

static CFGS: OnceLock<Mutex<BTreeMap<String, RuntimeComponentCfg>>> = OnceLock::new();

fn cfgs() -> &'static Mutex<BTreeMap<String, RuntimeComponentCfg>> {
    CFGS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

/// 注入 `launcher.json` 的 `runtime.components`（启动时调用一次）。
pub fn configure(components: BTreeMap<String, RuntimeComponentCfg>) {
    if let Ok(mut g) = cfgs().lock() {
        *g = components;
    }
}

fn cfg_of(id: &str) -> Option<RuntimeComponentCfg> {
    cfgs().lock().ok().and_then(|g| g.get(id).cloned())
}

/// 启进程 exe 所在目录（便携包可从任意 cwd 启动，不能用进程工作目录）
fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
}

/// 组件目录（**唯一位置**）：
/// 1. `launcher.json` 的 `runtime.components.<id>.dir`（相对 → 按 **exe 同目录** 解析）
/// 2. 缺省 = `<exe>/.vcmi-fv-launcher/runtime/<id>`
///
/// ⚠️ **三种分发形态落点完全一致**：打包预置（`bundle.resources` 里的
/// `.vcmi-fv-launcher/runtime/<id>`）、远程下载安装、便携内置都写这一处。
/// 于是便携部署只需"拷走 exe + `.vcmi-fv-launcher/`"，不会漏掉组件；
/// 也让"预制内容统一放 `.vcmi-fv-launcher/`"这一约定对本项目其余部分同样成立
/// （配置补丁 / 删除清单本就在该目录下）。
///
/// ⚠️ 代价：**不再兼容**早期把组件平铺在 `<exe>/runtime/<id>` 的开发版布局
/// （该布局从未发布过，按项目约定不写迁移代码）。
pub fn dir_of(id: &str) -> Option<PathBuf> {
    let preferred = VcmiPaths::launcher_data_dir()
        .map(|d| d.join("runtime").join(id))
        .or_else(|| {
            exe_dir().map(|d| d.join(".vcmi-fv-launcher").join("runtime").join(id))
        });
    match cfg_of(id).and_then(|c| c.dir.clone()) {
        Some(p) if !p.as_os_str().is_empty() => {
            if p.is_relative() {
                Some(exe_dir()?.join(p))
            } else {
                Some(p)
            }
        }
        _ => preferred,
    }
}

/// 组件的远程包配置（`package`），没有则 None
pub fn package_of(id: &str) -> Option<RuntimePackage> {
    cfg_of(id).and_then(|c| c.package.clone())
}

// ── 状态查询 ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
pub struct FileStatus {
    pub name: String,
    /// 文件是否存在
    pub present: bool,
    /// sha256 是否与代码常量一致（`present=false` 时恒为 false）
    pub sha_ok: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ComponentStatus {
    pub id: String,
    /// 组件目录（绝对路径；`null` = 无法确定 exe 位置）
    pub dir: Option<String>,
    /// 所有文件齐备且 sha256 全部匹配
    pub installed: bool,
    /// 已安装组件的版本（来自 `package.version`，仅作展示）
    pub version: Option<String>,
    pub files: Vec<FileStatus>,
    /// 是否配置了远程包（决定 UI 能否显示「安装」）
    pub has_package: bool,
}

/// 组件是否可用：文件齐备 + sha256 全中
pub fn is_ready(id: &str) -> bool {
    let Some(def) = def(id) else { return false };
    let Some(dir) = dir_of(id) else { return false };
    def.files
        .iter()
        .all(|f| sha256_matches(&dir.join(f.name), f.sha256))
}

fn sha256_matches(path: &Path, want: &str) -> bool {
    sha256_file(path)
        .map(|got| got.eq_ignore_ascii_case(want.trim()))
        .unwrap_or(false)
}

/// 查询组件状态（供命令层序列化给前端）
pub fn status(id: &str) -> Result<ComponentStatus, String> {
    let d = def(id).ok_or_else(|| be_err("runtime_unknown_component", id))?;
    let dir = dir_of(id);
    let mut files = Vec::new();
    if let Some(dir) = &dir {
        for f in d.files {
            let p = dir.join(f.name);
            let present = p.is_file();
            let sha_ok = present && sha256_matches(&p, f.sha256);
            files.push(FileStatus {
                name: f.name.to_string(),
                present,
                sha_ok,
            });
        }
    }
    let installed = !files.is_empty() && files.iter().all(|f| f.sha_ok);
    let pkg = package_of(id);
    Ok(ComponentStatus {
        id: id.to_string(),
        dir: dir.map(|p| p.to_string_lossy().replace('\\', "/")),
        installed,
        // 只有真正装好才报版本，避免 UI 显示"已装 1.28.0"却用不了
        version: if installed { pkg.as_ref().and_then(|p| p.version.clone()) } else { None },
        files,
        has_package: pkg.is_some(),
    })
}

// ── AI 模型（扫描 / 校验 / 解析）────────────────────────────────────────────

/// 扫描到的 AI 模型（供前端下拉；**轻量，不读文件内容**）
#[derive(Debug, Clone, Serialize)]
pub struct AiModelInfo {
    /// 文件名（含 `.onnx`）—— 即 `hd.ai.model` 可写的值
    pub file: String,
    /// 文件大小（字节，仅供 UI 展示）
    pub size: u64,
    /// 是否在组件清单的已知模型里（已知 → UI 可标"已验证"）
    pub known: bool,
    /// 是否为推荐模型（UI 的"恢复推荐"）
    pub recommended: bool,
}

/// 扫描 AI 组件目录内的 `*.onnx`（按文件名升序）。
///
/// 这是 UI 下拉的**唯一数据源**：目录里有什么就能选什么，包括用户自备的模型
/// （`known=false` → 标"未验证"，仍可用）。**不读文件内容**（17.5MB 级模型逐个哈希会让
/// 面板卡住）；需要校验时由 [`verify_ai_model`] 按需触发。
///
/// 目录不存在 / 无权限 → 空列表（不报错：组件未安装是正常状态）。
pub fn scan_ai_models() -> Vec<AiModelInfo> {
    let Some(dir) = dir_of(AI) else {
        return Vec::new();
    };
    let Ok(ents) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let known: &[ComponentFile] = def(AI).map(|d| d.models).unwrap_or(&[]);
    let mut out: Vec<AiModelInfo> = Vec::new();
    for e in ents.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !name.to_ascii_lowercase().ends_with(".onnx") {
            continue;
        }
        out.push(AiModelInfo {
            size: e.metadata().map(|m| m.len()).unwrap_or(0),
            known: known.iter().any(|f| f.name == name),
            recommended: name == RECOMMENDED_AI_MODEL,
            file: name,
        });
    }
    out.sort_by(|a, b| a.file.cmp(&b.file));
    out
}

/// 校验某个模型的 sha256 是否与组件清单一致（**按需调用**：会读整个文件）。
///
/// 返回 `None` = 该文件不在已知清单里（**未验证**，不是失败）；`Some(false)` = 已知但损坏。
pub fn verify_ai_model(file: &str) -> Option<bool> {
    let dir = dir_of(AI)?;
    let f = def(AI)?.models.iter().find(|f| f.name == file)?;
    Some(sha256_matches(&dir.join(file), f.sha256))
}

/// 已存在的 AI 模型文件路径：`<组件目录>/<file>`（不存在 → `None`）。
///
/// `file` 应是 [`scan_ai_models`] 产出的名字，或经 `VcmiPaths::hd_ai_model_name`
/// 规范化过的纯文件名（那里已拒绝路径分隔符，这里再挡一次以防直调）。
pub fn ai_model_path(file: &str) -> Option<PathBuf> {
    if file.is_empty() || file.contains(['/', '\\', ':']) || file.contains("..") {
        return None;
    }
    let p = dir_of(AI)?.join(file);
    p.is_file().then_some(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ai_model_path_rejects_unsafe_names() {
        // 路径穿越 / 目录分隔符 / 盘符 / 空串：一律拒绝
        // （非法检查发生在 `dir_of` 之前，故不依赖组件目录是否存在）
        for bad in ["", "a/b.onnx", "a\\b.onnx", "C:/x.onnx", "..", "../x.onnx"] {
            assert!(ai_model_path(bad).is_none(), "非法模型名应被拒绝：{bad:?}");
        }
    }

    #[test]
    fn scan_and_verify_are_safe_without_component() {
        // 组件目录不存在时：扫描返回空列表、校验返回 None（都不 panic、不报错）
        let _ = scan_ai_models();
        assert!(verify_ai_model("nonexistent.onnx").is_none());
    }

    #[test]
    fn required_files_exclude_models() {
        // 组件可用性只看必需文件；模型是"可选"（缺任一模型都不应让组件判未安装）
        let d = def(AI).expect("AI 组件应已注册");
        let required: Vec<&str> = d.files.iter().map(|f| f.name).collect();
        assert!(
            required.iter().all(|n| !n.ends_with(".onnx")),
            "必需文件不应含模型：{required:?}"
        );
        assert!(!d.models.is_empty(), "应登记随包模型（安装校验 + 扫描标注）");
        assert!(
            d.models.iter().any(|f| f.name == RECOMMENDED_AI_MODEL),
            "推荐模型应在清单内"
        );
    }
}
