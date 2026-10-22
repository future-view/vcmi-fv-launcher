use crate::infra::berror::be_err;
use crate::infra::fs::{CopyOutcome, CopyProgress};
use std::path::{Component, Path, PathBuf};

use crate::vcmi::config::LauncherConfig;

const DEFAULT_USER_DIR_NAME: &str = "vcmi";
/// 首启向导「启动器专属用户数据目录」的目录名（位于 exe 同目录；见 `portable_user_data_dir`）。
const PORTABLE_USER_DATA_DIR_NAME: &str = "userdata";
/// 迁移暂存目录后缀：`<目标目录名>.fv-copy-tmp`（与目标同级，保证 rename 同盘瞬时）。
const COPY_STAGING_SUFFIX: &str = ".fv-copy-tmp";
const COMMON_INSTALL_PATHS: &[&str] = &[
    "C:\\Program Files\\VCMI",
    "C:\\Program Files (x86)\\VCMI",
    "C:\\VCMI",
];

/// 规范化为**干净的绝对路径**：相对路径按 `base` 解析，并消除 `.` / `..` 成分。
///
/// `PathBuf::join("./userdata")` 会保留 `./`，直接落盘会得到 `D:\game\./userdata`
/// —— 既不是规范相对路径，Windows 也不接受这种绝对路径写法。
/// 因此**任何要写入 `dirs.json` 的路径**落盘前必须先经本函数规范化
/// （分隔符保持当前平台原生形式，跨平台正确）。
/// Windows 下把「无盘符的盘根路径」（如 `/userdata`）补全为带盘符的绝对路径
/// （如 `C:\userdata`）。否则它既会被 `cmd`/mklink 当成命令开关（报 `无效开关 - "userdata"`），
/// 也会被 VCMI 当成相对当前工作目录的路径（dataPaths() 硬编码 "."），两种场景都会失效。
/// 补全盘符取自 `base`（启动器 exe 所在盘）——便携模型的基准，比进程当前工作目录更可靠
/// （GUI 进程的 CWD 可能不可达，导致补盘失败、又把原样 `/userdata` 传给 mklink 而报错）。
/// 非 Windows 下 `/userdata` 已是合法绝对路径，原样返回。
#[cfg(windows)]
fn drive_qualified(p: &Path, base: &Path) -> Option<PathBuf> {
    if matches!(p.components().next(), Some(Component::RootDir)) {
        base_drive_root(base).map(|r| r.join(p.strip_prefix("/").unwrap_or(p)))
    } else {
        None
    }
}
#[cfg(not(windows))]
fn drive_qualified(_p: &Path, _base: &Path) -> Option<PathBuf> {
    None
}

/// 取 `base` 所在盘的根（如 `D:\`）：沿祖先链找到 parent() 为 None 的最顶级。
/// 用启动器 exe 目录而非进程 CWD——GUI 进程 CWD 可能不可达。
fn base_drive_root(base: &Path) -> Option<PathBuf> {
    base.ancestors()
        .find(|a| a.parent().is_none())
        .map(|a| a.to_path_buf())
}

pub fn normalize_abs(base: &Path, p: &Path) -> PathBuf {
    let joined = if p.is_absolute() {
        // Windows 上 `/userdata` 这种「无盘符盘根路径」也 is_absolute()==true，
        // 必须先补全盘符（取 base 所在盘），否则既会被 mklink 当成开关，也会被 VCMI 当成相对 CWD 的路径。
        drive_qualified(p, base).unwrap_or_else(|| p.to_path_buf())
    } else {
        base.join(p)
    };
    let mut out = PathBuf::new();
    for c in joined.components() {
        match c {
            Component::CurDir => {} // 跳过 "."
            Component::ParentDir => {
                out.pop(); // 回退一级（已在根则无操作）
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// 迁移暂存目录：与目标**同级**，保证 rename 落在同一卷（跨卷 rename 会失败）。
fn staging_dir_for(dst: &Path) -> PathBuf {
    let name = dst
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| PORTABLE_USER_DATA_DIR_NAME.to_string());
    let staged = format!("{name}{COPY_STAGING_SUFFIX}");
    match dst.parent() {
        Some(p) => p.join(staged),
        None => PathBuf::from(staged),
    }
}

/// 用户数据迁移结果（`migrate_user_data_dir`）。
#[derive(Debug, Clone, Copy)]
pub struct UserDataMigration {
    /// 是否真的复制了（`false` = 目标已存在 / 源不存在 / 同一路径，静默跳过）。
    pub copied: bool,
    /// 是否因用户「跳过复制」中断。
    pub cancelled: bool,
    pub files: u64,
    pub bytes: u64,
}

#[derive(Debug, Clone)]
pub struct VcmiPaths {
    /// 启动器 exe 所在目录
    pub exe_dir: PathBuf,
    /// VCMI 游戏目录（含 VCMI_client.exe）
    pub game_dir: PathBuf,
    /// 用户数据目录（默认 Documents/My Games/vcmi，可用 dirs.json 覆盖）
    pub user_dir: PathBuf,
    /// 配置目录（user_dir/config）
    pub user_config_dir: PathBuf,
    /// 日志目录（user_dir/logs）
    pub logs_dir: PathBuf,
    /// 存档目录（user_dir/Saves）
    pub saves_dir: PathBuf,
    /// 是否来自已持久化的 launcher.json（避免自动探测误报 ready）
    pub from_config: bool,
    /// 自动探测时是否找到有效的游戏目录
    pub detected: bool,
    /// 是否启用「接管 dirs.json」：将 VCMI 用户数据重定向到本启动器的 `user_dir`
    pub redirect_user_data: bool,
}

impl VcmiPaths {
    pub fn empty(exe_dir: PathBuf) -> Self {
        Self {
            user_config_dir: PathBuf::new(),
            game_dir: PathBuf::new(),
            logs_dir: PathBuf::new(),
            saves_dir: PathBuf::new(),
            user_dir: PathBuf::new(),
            exe_dir,
            from_config: false,
            detected: false,
            redirect_user_data: false,
        }
    }

    /// 是否真正就绪：必须来自已保存的 launcher.json（from_config），且游戏目录含 VCMI_client.exe。
    /// 自动探测（尚未保存）只用于回填初始化表单，不算就绪，避免未保存就执行扫描/写入等副作用。
    pub fn is_ready(&self) -> bool {
        self.detected && self.from_config && self.game_dir.join("VCMI_client.exe").is_file()
    }

    /// 启动器 exe 所在目录：**便携版 launcher.json 里相对路径的解析基准**。
    ///
    /// 约定：只有 `launcher.json` 允许写相对路径（维持便携可移植性），
    /// 任何要**落盘到 `dirs.json`** 或用于磁盘操作的路径都必须是绝对路径——
    /// VCMI 按**进程工作目录**解析 dirs.json 中的相对路径（`dataPaths()` 硬编码 "."），
    /// 而非相对游戏目录，写相对值会随启动方式变化而失效。
    pub fn exe_dir() -> PathBuf {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// 持久化配置路径：数据根目录 / launcher.json（数据根按平台分派，见 `launcher_data_dir`）
    pub fn launcher_config_path() -> Option<PathBuf> {
        Self::launcher_data_dir().map(|d| d.join("launcher.json"))
    }

    /// 启动器数据根目录（ADR 0008 D10：**按平台分派**），用于缓存 / 配置 / 领域 / 下载暂存等。
    ///
    /// - Windows：exe 同级 `.vcmi-fv-launcher`（**便携单文件夹是既定特性**，不变）；
    /// - macOS：`~/Library/Application Support/com.vcmi.fv-launcher/`（`.app` 装进
    ///   `/Applications` 后包内写数据会破坏代码签名，且整包替换即丢数据）；
    /// - Linux：`$XDG_DATA_HOME/vcmi-fv-launcher`（缺省 `~/.local/share/...`；AppImage
    ///   挂载的是**只读** squashfs，exe 同级根本不可写）。
    ///
    /// ⚠️ `mods_cache_dir` / `realm_vcmi_dir` / `downloads_dir` 等全部由本函数派生——
    /// 只改这里即可全链路切换（当初分层收敛的好处）。
    pub fn launcher_data_dir() -> Option<PathBuf> {
        #[cfg(windows)]
        {
            // Windows 维持"exe 同级"：便携语义（解压覆盖换目录即丢数据）是产品特性
            std::env::current_exe()
                .ok()
                .and_then(|exe| exe.parent().map(|d| d.to_path_buf()))
                .map(|d| d.join(".vcmi-fv-launcher"))
        }
        #[cfg(not(windows))]
        {
            Self::platform_data_dir()
        }
    }

    /// 非 Windows 的数据根（平台标准位置；ADR 0008 D10）
    #[cfg(not(windows))]
    fn platform_data_dir() -> Option<PathBuf> {
        #[cfg(target_os = "macos")]
        {
            // identifier = com.vcmi.fv-launcher（tauri.conf.json）
            dirs::data_dir().map(|d| d.join("com.vcmi.fv-launcher"))
        }
        #[cfg(target_os = "linux")]
        {
            // dirs::data_dir 在 Linux 上即 XDG_DATA_HOME（缺省 ~/.local/share）
            dirs::data_dir().map(|d| d.join("vcmi-fv-launcher"))
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            None
        }
    }

    /// Wiki 数据源缓存根目录：exe 目录 / .vcmi-fv-launcher/mods
    pub fn mods_cache_dir() -> Option<PathBuf> {
        Self::launcher_data_dir().map(|d| d.join("mods"))
    }

    /// 「启动器专属用户数据目录」：**exe 同目录 / userdata**（首启向导的自定义目录默认值）。
    ///
    /// 动机：多个 VCMI 发行版共用 `Documents\My Games\vcmi` 会互相踩踏（预设 / Mods / 存档混杂），
    /// 让启动器把用户数据收在自己旁边，便携包即可"整目录拷走"。
    /// ⚠️ 该目录在**游戏目录内**也安全：VCMI 的初始加载器只把 `dataPaths()` 与
    /// `userDataPath` 各自根下的 `MODS` / `CONFIG` / `DATA` 挂进虚拟命名空间
    /// （`CResourceHandler::createInitial`，`references/vcmi/lib/filesystem/Filesystem.cpp:147-156`），
    /// 不会递归扫到兄弟目录，因此不会把同一份模组挂两次。
    pub fn portable_user_data_dir() -> PathBuf {
        Self::exe_dir().join(PORTABLE_USER_DATA_DIR_NAME)
    }

    /// 迁移既有用户数据目录（首启一次性，**可选**）：`src` → `dst`。
    ///
    /// 三条不复制规则（与产品口径一致，全部静默跳过、不报错）：
    /// 1. `dst` 已存在 —— 用户已有数据，绝不覆盖；
    /// 2. `src` 不存在（全新安装，官方用户目录还没建）；
    /// 3. `src` 与 `dst` 同一路径。
    ///
    /// ⚠️ **staging + rename 是刚需**：复制中途关掉启动器（进程退出 → 复制线程被强杀）
    /// 时，目标目录绝不能留下半截内容 —— 否则 `dirs.json` 一旦指向它，VCMI 就会用残缺数据启动
    /// （`modSettings.json` 缺失 = 预设全丢）。写盘顺序由调用方保证：**先迁移、后写配置**。
    pub fn migrate_user_data_dir(
        src: &Path,
        dst: &Path,
        on_progress: &mut dyn FnMut(CopyProgress),
    ) -> Result<UserDataMigration, String> {
        let skip = || {
            Ok(UserDataMigration {
                copied: false,
                cancelled: false,
                files: 0,
                bytes: 0,
            })
        };
        if dst.exists() || !src.is_dir() {
            return skip();
        }
        let src_c = src.canonicalize().unwrap_or_else(|_| src.to_path_buf());
        let dst_c = dst
            .parent()
            .and_then(|p| p.canonicalize().ok())
            .unwrap_or_else(|| dst.to_path_buf());
        if src_c == dst_c || dst_c.starts_with(&src_c) {
            // 目标在源内部：复制会自我递归，直接拒绝
            return Err(be_err("setup_userdata_invalid", &dst.to_string_lossy()));
        }
        let Some(parent) = dst.parent() else {
            return Err(be_err("setup_userdata_invalid", &dst.to_string_lossy()));
        };
        std::fs::create_dir_all(parent)
            .map_err(|e| be_err("cmd_mkdir_failed", &format!("{}", e)))?;

        let staging = staging_dir_for(dst);
        if staging.exists() {
            // 上一次中断的残留（我们自己建的暂存目录，非联接，可安全递归删）
            std::fs::remove_dir_all(&staging)
                .map_err(|e| be_err("infra_stale_dir_clear_failed", &format!("{}", e)))?;
        }
        crate::infra::fs::reset_dir_copy();
        let mut done = CopyProgress::default();
        let outcome = crate::infra::fs::copy_dir_progress(src, &staging, &mut |p| {
            done = p;
            on_progress(p);
        })?;
        if outcome == CopyOutcome::Cancelled {
            let _ = std::fs::remove_dir_all(&staging);
            return Ok(UserDataMigration {
                copied: false,
                cancelled: true,
                files: done.done_files,
                bytes: done.done_bytes,
            });
        }
        std::fs::rename(&staging, dst).map_err(|e| {
            let _ = std::fs::remove_dir_all(&staging);
            be_err("setup_userdata_move_failed", &format!("{}", e))
        })?;
        tracing::info!(
            target: "paths",
            "用户数据已迁移：{} → {}（{} 个文件 / {} 字节）",
            src.display(),
            dst.display(),
            done.done_files,
            done.done_bytes
        );
        Ok(UserDataMigration {
            copied: true,
            cancelled: false,
            files: done.done_files,
            bytes: done.done_bytes,
        })
    }

/// 领域隔离目录（设计 §3.1）：exe 目录 / .vcmi-fv-launcher/realms/<id>/vcmi
    /// 5 个真实 dirs.json key 全部指向这里的子目录，从而把用户目录与游戏目录分离。
    pub fn realm_vcmi_dir(id: &str) -> Option<PathBuf> {
        Self::launcher_data_dir().map(|d| {
            d.join("realms")
                .join(Self::sanitize_realm_id(id))
                .join("vcmi")
        })
    }

    /// 领域隔离目录下的 5 个真实 key 对应子目录（§3.2）。
    pub fn realm_vcmi_subdirs(base: &Path) -> Vec<PathBuf> {
        vec![
            base.to_path_buf(),
            base.join("config"),
            base.join("cache"),
            base.join("logs"),
            base.join("Saves"),
            base.join("Mods"),
        ]
    }

    /// 清洗领域 id，仅保留字母/数字/下划线/连字符，禁止路径穿越。
    pub fn sanitize_realm_id(id: &str) -> String {
        let cleaned: String = id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect();
        if cleaned.is_empty() {
            "realm".to_string()
        } else {
            cleaned
        }
    }

    pub fn read_launcher_config() -> Option<LauncherConfig> {
        Self::read_launcher_config_raw()
            .and_then(|raw| serde_json::from_value::<LauncherConfig>(raw).ok())
            .and_then(|cfg| Self::apply_config_defaults(cfg))
    }

    /// 读取 launcher.json 原始 JSON（**不做合法性门槛**，不要求含 paths.game_dir/user_data_dir）。
    ///
    /// 用于"保留既有字段"场景：即便既有文件不完整（缺路径被 `read_launcher_config` 判为 `None`），
    /// 也能取回其中的 `realms.active` / `ui.lang` 等字段，
    /// 避免初始化向导保存时把已 seeded 的配置清空（与"合法性判定"解耦）。
    pub fn read_launcher_config_raw() -> Option<serde_json::Value> {
        let p = Self::launcher_config_path()?;
        if !p.is_file() {
            return None;
        }
        let text = std::fs::read_to_string(&p).ok()?;
        // 宽容解析（JSON5 子集）：手工编辑的 launcher.json 常带 `//` 注释与尾逗号
        crate::infra::json::parse_json_lenient(&text).ok()
    }

    /// 尽力还原配置：**不要求合法**（缺失路径也返回 `Some`）。
    ///
    /// 给「进程早期」的场景用（lib.rs 的日志等级 / 外挂组件注入）与「首次向导保存时继承既有值」
    /// 的场景 —— 这些都不该因为 launcher.json 尚不完整而拿不到发行版预置的其它字段。
    pub fn read_launcher_config_lenient() -> Option<LauncherConfig> {
        Self::read_launcher_config_raw()
            .and_then(|raw| serde_json::from_value::<LauncherConfig>(raw).ok())
    }

    /// 补齐派生默认值：`user_config_dir` / `user_mods_dir` 未显式配置时基于 `user_data_dir` 推导
    /// （VCMI 约定：<user_data_dir>/config、<user_data_dir>/Mods）。
    /// 这样便携版 launcher.json 只需填 paths 里的 game_dir + user_data_dir 两项即可。
    /// 路径为空的配置（尚未初始化）直接判为无效，与 `is_valid` 同门槛。
    fn apply_config_defaults(cfg: LauncherConfig) -> Option<LauncherConfig> {
        let mut cfg = cfg;
        if cfg.paths.game_dir.as_os_str().is_empty()
            || cfg.paths.user_data_dir.as_os_str().is_empty()
        {
            return None;
        }
        if cfg.paths.user_config_dir.as_os_str().is_empty() {
            cfg.paths.user_config_dir = cfg.paths.user_data_dir.join("config");
        }
        if cfg.paths.user_mods_dir.as_os_str().is_empty() {
            cfg.paths.user_mods_dir = cfg.paths.user_data_dir.join("Mods");
        }
        Some(cfg)
    }

    /// 读取 USM 锐化强度（`launcher.json` 的 `hd.usm`）。
    ///
    /// 缺省 `0.8`；`0` / 负数 = 关闭锐化；非法值（NaN）回退缺省，其余钳到 `0..=2`。
    /// 配置缺失或未初始化时**返回缺省而不报错** —— 锐化是增强项，配置坏了不该阻断构建。
    pub fn hd_usm_amount() -> f32 {
        let raw = Self::read_launcher_config()
            .and_then(|c| c.hd.usm)
            .unwrap_or(crate::infra::sharpen::ILLUSTRATION_AMOUNT);
        if raw.is_finite() {
            raw.clamp(0.0, 2.0)
        } else {
            crate::infra::sharpen::ILLUSTRATION_AMOUNT
        }
    }

    /// 读取 AI 混合权重（`launcher.json` 的 `hd.ai.mix`）：`α×AI + (1−α)×传统`。
    ///
    /// 缺省 `0.75`（对照实验里"更保真且仍清晰"的档位）；非法值回退缺省，其余钳到 `0..=1`。
    pub fn hd_ai_mix_amount() -> f32 {
        const DEFAULT_MIX: f32 = 0.75;
        let raw = Self::read_launcher_config()
            .and_then(|c| c.hd.ai.mix)
            .unwrap_or(DEFAULT_MIX);
        if raw.is_finite() {
            raw.clamp(0.0, 1.0)
        } else {
            DEFAULT_MIX
        }
    }

    /// 读取 **AI 模型文件名**（`launcher.json` 的 `hd.ai.model`），已规范化。
    ///
    /// 规范化规则（**只做纯字符串处理，不碰文件系统** —— 目录解析与存在性判断见
    /// `runtime::ai_model_path`）：
    /// - 去首尾空白；空串 → `None`（= AI 不生效，见 `config::AiCfg` 的四重合取）；
    /// - `xxx` 与 `xxx.onnx` 两种写法都接受，统一补成 `xxx.onnx`；
    /// - **拒绝含路径分隔符 / 盘符 / `..` 的值** —— 防止配置把推理指到组件目录之外。
    pub fn hd_ai_model_name() -> Option<String> {
        let raw = Self::read_launcher_config()?.hd.ai.model?;
        let name = raw.trim();
        if name.is_empty() {
            return None;
        }
        if name.contains(['/', '\\', ':']) || name.contains("..") {
            tracing::warn!(
                target: "launcher",
                "hd.ai.model 非法（必须是纯文件名，不含路径分隔符）：{name} —— AI 不生效"
            );
            return None;
        }
        if name.to_ascii_lowercase().ends_with(".onnx") {
            Some(name.to_string())
        } else {
            Some(format!("{name}.onnx"))
        }
    }

    /// 读取已持久化的窗口尺寸（`launcher.json` 的 `ui.window_size`）。
    ///
    /// 只在**配置有效**时返回（沿用 `read_launcher_config` 的门槛）：向导未完成时本就没有人
    /// 写过该字段，用无效配置里的残留值反而会让"首次启动按显示器自适应"失效。
    pub fn saved_window_size() -> Option<(u32, u32)> {
        Self::read_launcher_config()?
            .ui
            .window_size
            .filter(|s| s.width > 0 && s.height > 0)
            .map(|s| (s.width, s.height))
    }

    /// 把窗口尺寸写回 `launcher.json`。
    ///
    /// **配置尚不存在 / 无效时不落盘**（同 `save_window_size` 的既有约定）：首次运行向导完成前
    /// 写配置会生成一个只有 `ui.window_size` 的空 launcher.json，反而让目录探测以为"有配置"。
    pub fn store_window_size(width: u32, height: u32) -> Result<(), String> {
        use crate::vcmi::config::WindowSize;
        let Some(mut cfg) = Self::read_launcher_config() else {
            return Ok(());
        };
        cfg.ui.window_size = Some(WindowSize { width, height });
        Self::write_launcher_config(&cfg)
    }

    pub fn write_launcher_config(cfg: &LauncherConfig) -> Result<(), String> {
        let p = Self::launcher_config_path()
            .ok_or_else(|| be_err("common_app_config_dir_unknown", ""))?;
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| be_err("realm_config_dir_create_failed", &format!("{}", e)))?;
        }
        let text = serde_json::to_string_pretty(cfg)
            .map_err(|e| be_err("vcmi_config_serialize_failed", &format!("{}", e)))?;
        std::fs::write(&p, text).map_err(|e| be_err("vcmi_config_write_failed", &format!("{}", e)))
    }

    /// 自动探测可用的 VCMI 目录组合。
    /// 1. 优先读取 launcher.json；
    /// 2. 否则按 VCMI 源码逻辑：exe 同目录做游戏目录 + dirs.json/默认用户目录；
    /// 3. 常见安装路径；
    /// 4. 探测不到返回空，等待初始化页。
    pub fn detect() -> Self {
        let exe_dir = std::env::current_exe()
            .map(|p| {
                p.parent()
                    .map(|d| d.to_path_buf())
                    .unwrap_or_else(|| PathBuf::from("."))
            })
            .unwrap_or_else(|_| PathBuf::from("."));

        // 1. 环境变量最高优先（CI/测试/多安装切换场景）：VCMI_GAME_DIR 显式指定时
        //    覆盖 launcher.json —— 旧配置可能指向已废弃的游戏目录
        if let Ok(gd) = std::env::var("VCMI_GAME_DIR") {
            let game_dir = PathBuf::from(gd);
            if let Some(mut paths) = Self::try_game_dir(&game_dir, &exe_dir) {
                paths.from_config = Self::read_launcher_config()
                    .map(|c| c.paths.game_dir == game_dir)
                    .unwrap_or(false);
                return paths;
            }
        }

        // 2. launcher.json（相对路径解析到启动器所在目录，支持便携版任意解压路径）
        if let Some(cfg) = Self::read_launcher_config() {
            let cfg = cfg.resolve_relative_to(&exe_dir);
            if cfg.is_valid() {
                let mut paths = Self::from_config(exe_dir.clone(), &cfg);
                paths.detected = true;
                paths.from_config = true;
                // 配置优先：以 launcher.json 的 game_dir / user_data_dir 为准，先确定路径再做目录初始化
                tracing::info!(
                    target: "paths",
                    "采用 launcher.json 配置：realms.active={:?} redirect_user_data={}",
                    cfg.realms.active,
                    cfg.paths.redirect_user_data
                );
                // 便携初始化：创建用户数据目录 +（可选）接管 dirs.json 避免与官方路径冲突
                if let Err(e) = Self::ensure_user_data_dir(&paths, cfg.realms.active.as_deref()) {
                    tracing::warn!(target: "paths", "用户数据初始化失败（可忽略）：{e}");
                }
                return paths;
            }
            tracing::warn!(target: "paths", "launcher.json 配置无效（缺 game_dir/user_data_dir），回退自动探测");
        }

        // 2. exe 同目录作为游戏目录
        if let Some(paths) = Self::try_game_dir(&exe_dir, &exe_dir) {
            return paths;
        }

        // 3. 常见安装路径
        for raw in COMMON_INSTALL_PATHS {
            let game_dir = PathBuf::from(raw);
            if let Some(paths) = Self::try_game_dir(&game_dir, &exe_dir) {
                return paths;
            }
        }

        // 未检测到，返回空结构体，由前端引导初始化
        Self::empty(exe_dir)
    }

    fn from_config(exe_dir: PathBuf, cfg: &LauncherConfig) -> Self {
        Self {
            exe_dir,
            game_dir: cfg.paths.game_dir.clone(),
            user_dir: cfg.paths.user_data_dir.clone(),
            user_config_dir: cfg.paths.user_config_dir.clone(),
            logs_dir: cfg.paths.user_data_dir.join("logs"),
            saves_dir: cfg.paths.user_data_dir.join("Saves"),
            from_config: true,
            detected: true,
            redirect_user_data: cfg.paths.redirect_user_data,
        }
    }

    /// 以 game_dir 作为游戏目录尝试构建路径。
    /// 用户目录按 VCMI 规则：优先读 game_dir/config/dirs.json 的 userDataPath，
    /// 否则用 Documents/My Games/vcmi。
    pub fn try_game_dir(game_dir: &Path, exe_dir: &Path) -> Option<Self> {
        if !game_dir.join("VCMI_client.exe").is_file() {
            return None;
        }
        let user_dir = Self::resolve_user_dir(game_dir);
        Some(Self {
            exe_dir: exe_dir.to_path_buf(),
            game_dir: game_dir.to_path_buf(),
            user_config_dir: user_dir.join("config"),
            logs_dir: user_dir.join("logs"),
            saves_dir: user_dir.join("Saves"),
            user_dir,
            redirect_user_data: false,
            from_config: false,
            detected: true,
        })
    }

    /// 按 VCMI 源码 VCMIDirsWIN32 解析用户目录：
    /// 1. game_dir/config/dirs.json 的 userDataPath（支持环境变量展开）；
    /// 2. 否则 Documents/My Games/vcmi。
    pub fn resolve_user_dir(game_dir: &Path) -> PathBuf {
        let dirs_json = game_dir.join("config").join("dirs.json");
        if dirs_json.is_file() {
            if let Ok(text) = std::fs::read_to_string(&dirs_json) {
                if let Ok(v) = crate::infra::json::parse_json_lenient(&text) {
                    if let Some(u) = v.get("userDataPath").and_then(|x| x.as_str()) {
                        let expanded = expand_env(u);
                        let parsed = PathBuf::from(&expanded);
                        if parsed.is_absolute() {
                            return parsed;
                        }
                        // 展开后仍残留未定义变量（如 %USERDOCUMENTS% 无法展开）：
                        // 不能当作相对路径拼到游戏目录，回退默认用户目录
                        if has_unresolved_var(&expanded) {
                            return default_user_dir();
                        }
                        // 相对路径视为相对游戏目录
                        let joined = game_dir.join(parsed);
                        return joined;
                    }
                }
            }
        }
        default_user_dir()
    }

    pub fn user_mods_dir(&self) -> PathBuf {
        self.user_dir.join("Mods")
    }

    /// 用户地图目录（官方 `CLauncherDirs::mapsPath()` 同口径：`<userDataPath>/Maps`）。
    ///
    /// 地图 / 战役导入（含 zip 地图包解出）的统一目标；VCMI 引擎按此目录扫描用户地图。
    pub fn user_maps_dir(&self) -> PathBuf {
        self.user_dir.join("Maps")
    }

    /// 游戏目录自身的 Mods（单目录安装/portable 兜底来源）
    pub fn game_mods_dir(&self) -> PathBuf {
        self.game_dir.join("Mods")
    }

    pub fn mod_settings_path(&self) -> PathBuf {
        self.user_config_dir.join("modSettings.json")
    }

    pub fn vcmi_settings_path(&self) -> PathBuf {
        self.user_config_dir.join("settings.json")
    }

    /// VCMI **实际会读**的配置目录：`<game_dir>/config/dirs.json → userConfigPath`
    /// （支持 `%VAR%` / `$VAR` 展开），缺失 / 无法解析时回退 `<userDataPath>/config`
    /// （与本结构体其余派生字段同口径）。
    ///
    /// 为什么必须按 `dirs.json` 解析，而不是直接用启动器全局快照：
    /// **领域激活**会把 5 个 key 改写为领域隔离目录 —— 此时 VCMI 读的是
    /// `<realm>/vcmi/config`，而 `AppPaths.user_config_dir` 仍指向全局用户目录；
    /// 用它落盘会**改错文件**（详见 `realms/activate.rs` §配置共享）。
    ///
    /// ⚠️ 凡「写 VCMI 用户配置」的动作（清洗 persistentStorage.json、
    /// 播种 setupCompleted 等）都应按本函数解析目录，不要传全局快照路径。
    pub fn effective_config_dir(game_dir: &Path) -> Option<PathBuf> {
        if game_dir.as_os_str().is_empty() {
            return None;
        }

        let dirs_path = game_dir.join("config").join("dirs.json");
        if dirs_path.is_file() {
            let configured = std::fs::read_to_string(&dirs_path)
                .ok()
                .and_then(|text| crate::infra::json::parse_json_lenient(&text).ok())
                .and_then(|v| v.get("userConfigPath").and_then(|x| x.as_str()).map(str::to_string))
                .map(|raw| expand_env(&raw));

            if let Some(expanded) = configured {
                let parsed = PathBuf::from(expanded.trim());
                if parsed.is_absolute() && !has_unresolved_var(&expanded) {
                    return Some(parsed);
                }
            }
        }

        let user_dir = Self::resolve_user_dir(game_dir);
        if user_dir.as_os_str().is_empty() {
            return None;
        }
        Some(user_dir.join("config"))
    }

    pub fn update_from_config(&mut self, cfg: &LauncherConfig) {
        self.game_dir = cfg.paths.game_dir.clone();
        self.user_dir = cfg.paths.user_data_dir.clone();
        self.user_config_dir = cfg.paths.user_config_dir.clone();
        self.logs_dir = cfg.paths.user_data_dir.join("logs");
        self.saves_dir = cfg.paths.user_data_dir.join("Saves");
        self.redirect_user_data = cfg.paths.redirect_user_data;
        self.detected = true;
        self.from_config = true;
    }

    /// 便携初始化：复刻 VCMI `IVCMIDirs::init()` 的幂等目录创建，并（可选）接管 `dirs.json`。
    /// - 创建 `user_dir` 及 `cache/config/logs/Saves/Mods/Maps` 子目录（全部 `create_dir_all`，可重复调用）。
    /// - 当 `redirect_user_data` 为真且当前无激活领域时，把 `game_dir/config/dirs.json` 的
    ///   `userDataPath` 等 5 个 key 指向 `user_dir`，使 VCMI 使用独立用户目录，
    ///   避免与官方 `Documents/My Games/vcmi` 冲突。
    ///
    /// ⚠️ `Maps` 必须由启动器建：VCMI 只自建 `userDataPath/cache/config/logs/Saves`，
    /// **不建 `Mods` / `Maps`**（见 `docs/reference/vcmi-dirs.zh-CN.md` 硬事实 7）。
    pub fn ensure_user_data_dir(
        paths: &VcmiPaths,
        active_realm: Option<&str>,
    ) -> Result<(), String> {
        for d in [
            paths.user_dir.clone(),
            paths.user_dir.join("cache"),
            paths.user_config_dir.clone(),
            paths.user_dir.join("logs"),
            paths.user_dir.join("Saves"),
            paths.user_mods_dir(),
            paths.user_maps_dir(),
        ] {
            std::fs::create_dir_all(&d)
                .map_err(|e| be_err("vcmi_userdata_dir_create_failed", &format!("{}", e)))?;
        }
        tracing::debug!(
            target: "paths",
            "用户数据目录就绪：redirect_user_data={} active_realm={:?}",
            paths.redirect_user_data,
            active_realm
        );
        if paths.redirect_user_data && active_realm.is_none() {
            Self::apply_user_data_redirect(paths)?;
            // 接管后必须补一步：播种「已完成初始化」标记。
            // 否则官方启动器把新用户目录当全新安装，每次都弹首次运行向导。
            Self::ensure_vcmi_setup_completed(paths)?;
        }
        Ok(())
    }

    /// 播种 VCMI 的 `settings.json` 里的 `launcher.setupCompleted`：让官方启动器
    /// （`VCMI_launcher.exe` / vcmiqt）**跳过首次运行向导**（语言选择 + 数据导入页）。
    ///
    /// 为什么必须做：官方启动器只认
    /// `settings["launcher"]["setupCompleted"]`（`launcher/mainwindow_moc.cpp:83,129-134`；
    /// schema 默认 false），而我们重定向/便携出的用户目录里没有这个标记 → 被当成全新安装。
    /// 实现见 `vcmi::setup::seed_setup_completed`（语义与安全约束写在那边）。
    pub fn ensure_vcmi_setup_completed(paths: &VcmiPaths) -> Result<(), String> {
        crate::vcmi::setup::seed_setup_completed(&paths.user_config_dir)
    }

    /// 接管 `game_dir/config/dirs.json`：仅改写 VCMI 的 5 个用户目录 key，保留其它字段；
    /// 首次改写前备份原文（独立后缀，避免与领域激活的 `vcmi-fv-backup` 互相覆盖）。
    fn apply_user_data_redirect(paths: &VcmiPaths) -> Result<(), String> {
        let config_dir = paths.game_dir.join("config");
        let dirs_path = config_dir.join("dirs.json");
        let backup = config_dir.join("dirs.json.fv-userdata-bak");
        let target = paths.user_dir.to_string_lossy().to_string();

        // 已指向相同 userDataPath 则跳过（幂等）
        if dirs_path.is_file() {
            if let Ok(text) = std::fs::read_to_string(&dirs_path) {
                if let Ok(v) = crate::infra::json::parse_json_lenient(&text) {
                    if v.get("userDataPath").and_then(|x| x.as_str()) == Some(target.as_str()) {
                        return Ok(());
                    }
                }
            }
            // 仅首次备份原文（独立后缀，不与领域备份冲突）
            if !backup.is_file() {
                std::fs::copy(&dirs_path, &backup)
                    .map_err(|e| be_err("realm_dirs_backup_failed", &format!("{}", e)))?;
            }
        }

        // 读取既有内容并合并 5 个 key，避免覆盖其它自定义字段
        let mut map = if dirs_path.is_file() {
            std::fs::read_to_string(&dirs_path)
                .ok()
                .and_then(|t| crate::infra::json::parse_json_lenient(&t).ok())
                .and_then(|v| v.as_object().cloned())
                .unwrap_or_default()
        } else {
            serde_json::Map::new()
        };
        map.insert("userDataPath".into(), serde_json::Value::String(target));
        map.insert(
            "userCachePath".into(),
            serde_json::Value::String(paths.user_dir.join("cache").to_string_lossy().to_string()),
        );
        map.insert(
            "userConfigPath".into(),
            serde_json::Value::String(paths.user_config_dir.to_string_lossy().to_string()),
        );
        map.insert(
            "userLogsPath".into(),
            serde_json::Value::String(paths.user_dir.join("logs").to_string_lossy().to_string()),
        );
        map.insert(
            "userSavePath".into(),
            serde_json::Value::String(paths.user_dir.join("Saves").to_string_lossy().to_string()),
        );
        let current = serde_json::Value::Object(map);

        std::fs::create_dir_all(&config_dir)
            .map_err(|e| be_err("realm_config_dir_create_failed", &format!("{}", e)))?;
        let text = serde_json::to_string_pretty(&current)
            .map_err(|e| be_err("realm_dirs_serialize_failed", &format!("{}", e)))?;
        std::fs::write(&dirs_path, text)
            .map_err(|e| be_err("realm_dirs_write_failed", &format!("{}", e)))?;
        tracing::info!(target: "paths", "已接管 dirs.json：用户数据重定向到 user_data_dir");
        Ok(())
    }
}

/// VCMI 用户数据目录的标准成员：**任一存在**即认为该目录"像"用户数据目录。
///
/// 集合取自 [`VcmiPaths::ensure_user_data_dir`] 的创建清单（VCMI 自建
/// `cache`/`config`/`logs`/`Saves`，启动器补 `Mods`/`Maps`，见 vcmi-dirs 硬事实 7）。
/// `config` 由 VCMI 首次运行即创建 ⇒ 合法用户数据目录必然命中，不会误杀。
/// `cache`/`logs`/`Maps` 单独存在时**不**作充分条件：单独复制它们无价值，
/// 且更可能是误选（如下载目录）。
const USER_DATA_MEMBERS: &[&str] = &["config", "Mods", "Saves"];

/// 游戏目录标记：含此可执行文件的一律视为游戏目录，而非用户数据目录。
const GAME_EXE_MARKER: &str = "VCMI_client.exe";

/// 「从任意目录迁移 VCMI 用户数据」前的**结构闸门**（判据的单一来源，前端无副本）。
///
/// 与 [`VcmiPaths::migrate_user_data_dir`] 的三条静默跳过规则互补：那些规则回答
/// "要不要复制"（目标已存在 / 源不存在 / 同路径），本函数回答"这个源是不是
/// VCMI 用户数据目录"——选错目录必须**在任何字节复制之前**拒绝。
///
/// 判据：
/// - 拒绝：存在 `VCMI_client.exe`（**游戏目录同样含 `config`/`Mods`**——
///   `config/dirs.json` 与发行版自带 mod，而首启向导第 ① 步刚选过游戏目录，
///   误选概率最高；不放行才能避免把整个游戏目录（含 `Data` 数 GB）搬进 userdata）；
/// - 通过：`config` / `Mods` / `Saves` 任一子目录存在；
/// - 拒绝：以上均无（mod 目录 / 压缩包解压的中间层 / 空目录）。
pub fn looks_like_vcmi_user_data(dir: &Path) -> bool {
    if dir.join(GAME_EXE_MARKER).is_file() {
        return false;
    }
    USER_DATA_MEMBERS
        .iter()
        .any(|m| dir.join(m).is_dir())
}

/// VCMI 官方默认用户数据目录：`文档\My Games\vcmi`。
///
/// 两个用途：dirs.json 未接管时的解析回退；首启向导「从既有目录复制」的**源目录**。
pub fn default_user_dir() -> PathBuf {
    dirs::document_dir()
        .map(|d| d.join("My Games").join(DEFAULT_USER_DIR_NAME))
        .unwrap_or_else(|| {
            PathBuf::from(".")
                .join("My Games")
                .join(DEFAULT_USER_DIR_NAME)
        })
}

/// 解析 %VAR% / $VAR 变量：优先系统环境变量；对 VCMI 自定义变量做映射。
fn resolve_env_var(var: &str) -> Option<String> {
    if let Some(v) = std::env::var_os(var) {
        return Some(v.to_string_lossy().to_string());
    }
    match var {
        // VCMI 源码自定义变量：USERDOCUMENTS 映射到用户"我的文档"目录
        "USERDOCUMENTS" => dirs::document_dir().map(|p| p.to_string_lossy().to_string()),
        _ => None,
    }
}

/// 判断字符串中是否残留未展开的环境变量（%VAR% 或 $VAR 形式）。
/// 用于防御：展开失败时应回退默认，而非把字面量当相对路径拼到游戏目录。
pub(crate) fn has_unresolved_var(s: &str) -> bool {
    let bytes = s.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'%' || b == b'$' {
            if let Some(&next) = bytes.get(i + 1) {
                if next.is_ascii_alphabetic() || next == b'_' {
                    return true;
                }
            }
        }
    }
    false
}

/// 展开 Windows 环境变量（%VAR% 与 $VAR 两种形式）
pub(crate) fn expand_env(raw: &str) -> String {
    let mut result = raw.to_string();
    // %VAR% 形式
    let chars: Vec<char> = raw.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '%' {
            if let Some(close) = chars[i + 1..].iter().position(|&c| c == '%') {
                let end = i + 1 + close;
                let var: String = chars[i + 1..end].iter().collect();
                if let Some(val) = resolve_env_var(&var) {
                    result = result.replace(&format!("%{var}%"), &val);
                    i = end + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    // $VAR 形式
    let mut out = String::new();
    let mut it = result.chars().peekable();
    while let Some(c) = it.next() {
        if c == '$' {
            let mut name = String::new();
            while let Some(&n) = it.peek() {
                if n.is_ascii_alphanumeric() || n == '_' {
                    name.push(n);
                    it.next();
                } else {
                    break;
                }
            }
            if !name.is_empty() {
                if let Some(val) = resolve_env_var(&name) {
                    out.push_str(&val);
                } else {
                    out.push('$');
                    out.push_str(&name);
                }
            } else {
                out.push('$');
            }
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vcmi-fv-paths-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("临时目录应可创建");
        d
    }

    #[test]
    fn user_data_gate_cases() {
        let d = tmp("user-data-gate");

        // 合法用户数据目录：config 由 VCMI 自建 ⇒ 必然命中
        let user = d.join("vcmi");
        std::fs::create_dir_all(user.join("config")).unwrap();
        assert!(looks_like_vcmi_user_data(&user), "标准用户数据目录应放行");

        // 只有 Mods / Saves 的目录同样放行（备份型目录常见）
        let mods_only = d.join("mods-only");
        std::fs::create_dir_all(mods_only.join("Mods")).unwrap();
        assert!(looks_like_vcmi_user_data(&mods_only));
        let saves_only = d.join("saves-only");
        std::fs::create_dir_all(saves_only.join("Saves")).unwrap();
        assert!(looks_like_vcmi_user_data(&saves_only));

        // 游戏目录：同样含 config/Mods，但有 VCMI_client.exe ⇒ 必须拒绝
        // （否则会把整个游戏目录含 Data 数 GB 搬进 userdata）
        let game = d.join("game");
        std::fs::create_dir_all(game.join("config")).unwrap();
        std::fs::create_dir_all(game.join("Mods")).unwrap();
        std::fs::write(game.join("VCMI_client.exe"), b"MZ").unwrap();
        assert!(
            !looks_like_vcmi_user_data(&game),
            "游戏目录必须拒绝（config/Mods 与用户数据目录同构，只能靠 exe 标记区分）"
        );

        // mod 目录（有 mod.json/content，无标准成员）⇒ 拒绝
        let mod_dir = d.join("hota");
        std::fs::create_dir_all(mod_dir.join("content")).unwrap();
        std::fs::write(mod_dir.join("mod.json"), b"{}").unwrap();
        assert!(!looks_like_vcmi_user_data(&mod_dir));

        // 压缩包解压的中间层 / 空目录 ⇒ 拒绝
        let empty = d.join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(!looks_like_vcmi_user_data(&empty));

        // 只有 cache 的目录 ⇒ 拒绝（缓存单独复制无价值，且易是误选）
        let cache_only = d.join("cache-only");
        std::fs::create_dir_all(cache_only.join("cache")).unwrap();
        assert!(!looks_like_vcmi_user_data(&cache_only));

        let _ = std::fs::remove_dir_all(&d);
    }
}
