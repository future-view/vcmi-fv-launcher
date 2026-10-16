//! 启动器自身配置（`launcher.json`）的**磁盘数据模型**。
//!
//! ## 为什么分组
//!
//! 早期所有配置项平铺在根对象上（27 个同级键），带来两个问题：
//! - **看不出从属关系**：`hd_edition_use` 属于高清素材还是 runtime？`ui_lang` 和 `log_level` 谁管什么？只能靠注释猜；
//! - **发行版难以下手**：想只预置「高清素材」一类配置时，要在 27 个平铺键里逐个挑。
//!
//! 现在按**用途**划成 7 个子对象，层级即语义，键名仍统一 snake_case：
//!
//! ```json5
//! {
//!   "paths":   { "game_dir": "…", "user_data_dir": "…", … },   // 目录与便携接管
//!   "ui":      { "lang": "chinese", "window_size": { … }, … },  // 界面（语言/窗口/设置面板）
//!   "hd":      { "enhance": false, "usm": 0.8, "ai": { … } },   // 高清素材生成（含 AI 子链路）
//!   "mods":    { "required": [], "builtin_presets": [] },       // 模组策略（发行版潜规则）
//!   "realms":  { "active": null, "repository_urls": [] },       // 领域
//!   "update":  { "manifest_url": null, "auto_check": true, … }, // 启动器自动更新
//!   "runtime": { "log_level": "info", "components": { … } },    // 进程级选项 + 外挂组件
//!   "network": { "proxy": null, "mirror_rules": [] },           // 网络出口（L1 代理 / L2 镜像规则表，默认直连）
//!   "archives": { "zip_passwords": {} }                          // 压缩包解密（仅加密发行版）
//! }
//! ```
//!
//! ## 三层格式仍然各自冻结（见 `docs/architecture/backend.zh-CN.md` §7.1）
//!
//! 磁盘结构**变了形状没变风格**（仍是 snake_case，只是多了分组层）；
//! IPC 出参 / 入参的 camelCase 契约（`commands/settings.rs`）**一字未改**，前端无感。
//!
//! ## 不写兼容层
//!
//! 旧版「全平铺」的 launcher.json **不再被读取**（项目尚无对外发布版本，无历史用户配置包袱）：
//! 没有迁移代码，也不做"读到旧键再搬一遍"的兜底 —— 保持一条格式、一处真相。
//! 旧文件会被判为无效配置，用户重跑一次初始化向导即可。

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use crate::vcmi::paths::normalize_abs;

/// 路径持久化形式：**统一使用 `/` 分隔**。
///
/// Windows 上 `PathBuf` 默认序列化成 `a\b`，与手写的相对写法（`./userdata`）拼接后
/// 会出现 `./userdata\config` 这种分隔符混用的不规范形式，故 launcher.json 一律写 `/`
/// （Windows 完全接受 `/`，且便于跨平台阅读 / 手写编辑）。
#[cfg(windows)]
fn path_to_config_str(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

#[cfg(not(windows))]
fn path_to_config_str(p: &Path) -> String {
    p.to_string_lossy().to_string()
}

/// `PathBuf` 字段的 serde 适配：以 `/` 形式读写 launcher.json。
mod path_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::path::PathBuf;

    pub fn serialize<S: Serializer>(p: &PathBuf, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&super::path_to_config_str(p))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<PathBuf, D::Error> {
        Ok(PathBuf::from(String::deserialize(d)?))
    }
}

/// `Option<PathBuf>` 版本：缺失写 `null`，有值同样以 `/` 形式持久化。
mod opt_path_serde {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::path::PathBuf;

    pub fn serialize<S: Serializer>(p: &Option<PathBuf>, s: S) -> Result<S::Ok, S::Error> {
        match p {
            Some(v) => s.serialize_str(&super::path_to_config_str(v)),
            None => s.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<PathBuf>, D::Error> {
        Ok(Option::<String>::deserialize(d)?.map(PathBuf::from))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct WindowSize {
    pub width: u32,
    pub height: u32,
}

/// launcher.json 根结构：**只由 7 个分组构成**，不再有平铺的散项。
///
/// 各分组均能整体缺省（便携包只需给 `"paths"` 两块即可），缺省值见各自的 `Default`。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LauncherConfig {
    /// VCMI 目录与便携接管 —— 配置里唯一「必须有值」的分组（见 [`Self::is_valid`]）。
    #[serde(default)]
    pub paths: PathsCfg,
    /// 启动器界面偏好（语言 / 窗口 / 设置面板视图）。
    #[serde(default)]
    pub ui: UiCfg,
    /// 高清素材（`@Nx` 变体）的生成策略与来源。
    #[serde(default)]
    pub hd: HdCfg,
    /// 模组策略：发行版潜规则（强启模组）+ 追加的内置预设。
    #[serde(default)]
    pub mods: ModsCfg,
    /// 领域系统（只留「当前激活哪个」与「远程索引源」，领域元数据在各领域目录的 `realm.json`）。
    #[serde(default)]
    pub realms: RealmsCfg,
    /// 启动器自身自动更新（VCMI 之外的另一条更新链）。
    #[serde(default)]
    pub update: UpdateCfg,
    /// 进程级选项（日志等级）+ 外挂运行时组件（`components.<id>`）。
    #[serde(default)]
    pub runtime: RuntimeCfg,
    /// 网络出口（GitHub 直连↔代理自动回退；默认直连）。
    #[serde(default)]
    pub network: NetworkCfg,
    /// 压缩包解密（仅加密发行版需要）：mod content.zip 等密码表。
    #[serde(default)]
    pub archives: ArchivesCfg,
}

/// `paths`：目录相关（`-P` 便携模式下多为相对 exe 目录的写法）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PathsCfg {
    /// 游戏目录（必须含 `VCMI_client.exe`）
    ///
    /// 带上 `default` 而非"必填"：便携包 / 补丁常常只预置零散几项而没有路径，
    /// 缺了路径也必须能把**其余字段**取回来（见 `VcmiPaths::read_launcher_config_lenient`）。
    /// "是否可用"由 [`LauncherConfig::is_valid`] 单独判定，不靠反序列化失败来表达。
    #[serde(default, with = "path_serde")]
    pub game_dir: PathBuf,
    /// VCMI 用户数据根目录（同上：可缺省，合法性另行判定）
    #[serde(default, with = "path_serde")]
    pub user_data_dir: PathBuf,
    /// 缺省时由 `read_launcher_config` 基于 `user_data_dir` 推导（<user_data_dir>/config）
    #[serde(default, with = "path_serde")]
    pub user_config_dir: PathBuf,
    /// 缺省时由 `read_launcher_config` 基于 `user_data_dir` 推导（<user_data_dir>/Mods）
    #[serde(default, with = "path_serde")]
    pub user_mods_dir: PathBuf,
    /// 备用：游戏目录自身也含 Mods 时，作为兜底来源（仅当用户目录缺失时用）
    #[serde(default, with = "opt_path_serde")]
    pub game_mods_dir: Option<PathBuf>,
    /// 便携接管：是否将 `game_dir/config/dirs.json` 的 userDataPath 重定向到本分组的 `user_data_dir`，
    /// 以独立目录接管用户数据，避免与官方预设的 `Documents/My Games/vcmi` 冲突。
    /// 仅全局模式（未激活领域）生效；激活领域时由领域接管 dirs.json，本项不干预。
    #[serde(default)]
    pub redirect_user_data: bool,
}

/// `ui`：启动器界面偏好（**不含 VCMI 游戏内设置**，后者在 settings.json）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UiCfg {
    /// 启动器界面语言（VCMI 语言标识，如 `chinese` / `english`）。
    ///
    /// 纳入配置而非前端 `localStorage`：`localStorage` 绑定系统用户数据目录、
    /// 不随便携包走，导致便携包语言无法统一。缺失时由前端按系统语言探测回退。
    #[serde(default)]
    pub lang: Option<String>,
    /// 启动器窗口尺寸（首次启动按显示器自适应后保存）
    #[serde(default)]
    pub window_size: Option<WindowSize>,
    /// 设置面板可见级别：common（常用）/ advanced（进阶）/ all（全部），默认 common
    #[serde(default = "default_settings_level")]
    pub settings_level: String,
    /// 设置面板是否显示详细描述，默认关闭
    #[serde(default)]
    pub settings_show_desc: bool,
    /// 设置面板是否显示全部参数（含高级项），默认关闭（保留以兼容旧字段）
    #[serde(default)]
    pub show_all_settings: bool,
    /// 模组列表 hover 时是否弹出城镇背景大图预览，**默认关闭**。
    /// 关闭时前端既不触发浮层、也不读取大图缓存（省掉每行的 IPC 与重试轮询）。
    #[serde(default)]
    pub town_preview: bool,
}

impl Default for UiCfg {
    fn default() -> Self {
        Self {
            lang: None,
            window_size: None,
            settings_level: default_settings_level(),
            settings_show_desc: false,
            show_all_settings: false,
            town_preview: false,
        }
    }
}

/// `hd`：高清素材（`@Nx` 变体）的生成策略。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HdCfg {
    /// **高清素材生成总开关**（`FV启动器` 标签），默认 **false**。
    ///
    /// 关：完全**不生成** `@Nx` 变体（前端不传 hd 参数，后端直接跳过高清分支）——
    /// 这是"标清已可用"的最快路径；开：按用途分级生成（插画档再受 [`AiCfg::enhance`] 影响）。
    #[serde(default)]
    pub enhance: bool,
    /// **AI 超分子链路**：Real-ESRGAN 超分插画类素材（见 [`AiCfg`]）。
    ///
    /// 发行版可在随包的 `launcher.json` 里预置
    /// `"hd": { "ai": { "enhance": true, "model": "realesrgan_anime6b.onnx" } }` 来默认开启 ——
    /// ⚠️ `enhance` 与 `model` 缺一不可：只开开关而不指定模型 = **AI 不生效**（无默认模型）。
    #[serde(default)]
    pub ai: AiCfg,
    /// **高清素材来源**（官方 HD 版素材 mod `hd-edition` 的 `.pak`，见 `vcmi::hd_edition`）：
    ///
    /// - 缺省 / `"auto"`（推荐）：检测到即用 —— 有真高清就优先取真高清生成 `@Nx` 变体；
    /// - `"off"`：完全不用（全链路退化为"原版 1x 放大"，与历史行为一致）。
    ///
    /// ⚠️ 语义是**素材来源**而非运行时依赖：只在构建期读 pak，产物仍落 `.vcmi-fv-launcher/mods/`。
    #[serde(default = "default_auto")]
    pub edition_use: String,
    /// 素材档位：缺省 / `"auto"` = 取可用最高档（x3 优先）；`"x2"` = 只用 x2（省磁盘）。
    #[serde(default = "default_auto")]
    pub edition_scale: String,
    /// **USM 锐化强度**（缺省 `0.8`；`0` = 关闭）。
    ///
    /// 传统插值放大后偏软，叠 USM 拿回锐度且不引入生成伪影 ——
    /// 同时是插画档在 AI 不可用时的锐度补偿（见 `modcache::hd::upscale_file`）。
    /// 像素画不叠（硬边会产生光晕），不受本项影响。
    #[serde(default)]
    pub usm: Option<f32>,
}

impl Default for HdCfg {
    fn default() -> Self {
        Self {
            enhance: false,
            ai: AiCfg::default(),
            edition_use: default_auto(),
            edition_scale: default_auto(),
            usm: None,
        }
    }
}

/// `hd.ai`：**AI 超分子链路**（仅插画档参与；像素画永远走 xBRZ）。
///
/// ## 生效条件（四重合取）
///
/// 1. [`HdCfg::enhance`]（总开关）；
/// 2. [`AiCfg::enhance`]（本开关）；
/// 3. [`AiCfg::model`] **非空** —— 无默认模型，不写 = AI 不生效；
/// 4. 该模型文件确实存在于组件目录（`runtime.components.ai.dir`，缺省
///    `<exe>/.vcmi-fv-launcher/runtime/ai/`）。
///
/// 任一不满足 → 插画档回落 `Lanczos3 + USM`（见 `modcache::hd::smooth_or_ai`，
/// 四种原因会在日志里分别标注，便于排查"为什么没走 AI"）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AiCfg {
    /// **AI 高清增强开关**（`FV启动器` 标签），默认 **false**。
    ///
    /// 默认关：普通用户不关心，且开启会增加素材构建时的 CPU 开销。
    #[serde(default)]
    pub enhance: bool,
    /// **AI 模型文件名**（组件目录内的 `*.onnx`），默认 **None**。
    ///
    /// - 可写全名（`"realesrgan_anime6b.onnx"`）或省略扩展名（`"realesrgan_anime6b"`），
    ///   读取侧统一规范化；**必须是不含路径分隔符的纯文件名**；
    /// - 缺省 / 空串 / 非法值 → **AI 不生效**（无默认模型，不猜）；
    /// - 目录内实际可用的模型由 `runtime::scan_ai_models` 扫描得出（前端下拉即取此列表）。
    #[serde(default)]
    pub model: Option<String>,
    /// **AI 混合权重**（0~1，缺省 `0.75`；仅在 AI 真正生效时有意义）。
    ///
    /// AI 超分结果先色彩对齐、再与 `Lanczos3 + USM` 结果按 `α×AI + (1−α)×传统` 混合：
    /// - `1.0` = 纯 AI；`0.75`（默认）保留细节的同时稀释生成式幻觉与色偏；
    /// - `0` = 完全不用 AI 结果（等价于传统档，但仍会白跑一次 AI 推理，通常应直接关 `enhance`）。
    ///
    /// 依据 `target/hd-compare` 的逐像素对照（`50/51_ai_mix*` 列）。
    #[serde(default)]
    pub mix: Option<f32>,
}

/// `mods`：模组策略（发行版潜规则 + 追加的内置预设）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ModsCfg {
    /// 全局强制模组（潜规则）：每次写 modSettings 时始终启用，用户不可关闭。
    /// 典型如 H3E 发行版强制 `h3e-main-menu`，否则无法启动。属发行版侧（launcher.json），
    /// 领域（realm.json）不携带此特性——领域是多个发行版公用的共享容器。
    #[serde(default)]
    pub required: Vec<String>,
    /// 内置预设（可选）：发行版可在不改源码情况下追加语言版/新预设，与源码内置 `__official__` 合并追加。
    /// 缺省为空，回落到源码内置预设。
    #[serde(default)]
    pub builtin_presets: Vec<BuiltinPresetCfg>,
}

/// `realms`：领域系统接入点。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RealmsCfg {
    /// 当前激活的领域 id（None = 未激活任何领域，使用全局 VCMI 配置）。
    /// 领域元信息的唯一权威是**各领域目录内的 `realm.json`**（§3.1「目录即领域」），
    /// launcher.json 中**只保留**领域相关的这一项。
    #[serde(default)]
    pub active: Option<String>,
    /// 远程领域仓库索引地址列表（多源）。每个元素是一个能返回 index.json 的 https 直链，
    /// 可指向 Gitee raw / GitHub raw / 自建静态托管。发行版可通过随包的 `launcher.json`
    /// 预置（不再硬编码内置默认）；列表为空表示未配置任何仓库，远程领域列表将提示用户添加。
    #[serde(default)]
    pub repository_urls: Vec<String>,
    /// 「自动检测更新」（launcher.json 本地开关，默认开）：仅领域「模组管理」页生效——
    /// 打开管理页时静默检查一次模组仓库索引（fetch_mod_index，不弹 toast；与 mods 页
    /// 共用同一索引与缓存）。发行版不随包分发该字段，由各人本地开关决定；
    /// `None` / `Some(true)` = 开。
    #[serde(default)]
    pub mod_auto_check: Option<bool>,
    /// 「导出领域 → 发布到领域仓库」记住的上次目录（含 `index.json` 的那级）；
    /// `None` / 空 = 未配置。仅本机发布用，不随发行版分发。
    #[serde(default)]
    pub publish_repo_dir: Option<String>,
    /// 直写仓库时 `download` / `cover` 的 URL 前缀**人工覆盖值**（如 `https://github.com/<owner>/<repo>/raw/main`）；
    /// 留空则由 `derive_repo_url_base` 按「仓库声明 > git remote+分支 > 既有条目」推导。
    /// 只影响本机，**不会写回仓库** `index.json`。
    #[serde(default)]
    pub publish_url_base: Option<String>,
    /// 导出弹窗上次用的分支：`"share"`（导出分享文件）/ `"publish"`（发布到领域仓库）。
    /// 存配置而非 localStorage：便携包要求偏好随目录走（见 `src/i18n/index.ts` 的同款说明）。
    #[serde(default)]
    pub publish_mode: Option<String>,
}

/// `network`：网络出口。代理仅支持 **http(s)**（reqwest 0.12/0.13 已移除内置 SOCKS）；
/// 镜像规则表（L2）按声明序尝试，类型复用 `infra::http::MirrorRule`（纯网络概念，见方案 §3.2）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct NetworkCfg {
    /// HTTP(S) 代理地址（如 `http://127.0.0.1:7890`）；`None` / `"off"` / 空 = 直连（默认）。
    /// 配置后，对 GitHub 直链的请求会「先直连、连接失败自动改走代理」。
    #[serde(default)]
    pub proxy: Option<String>,
    /// GitHub 镜像规则表（L2）；**空 = 不用（默认不内置，镜像域名会腐化由用户自填）**。
    /// 直连失败后按声明序逐条改写重试，全部失败再落到代理（若配）。
    #[serde(default)]
    pub mirror_rules: Vec<crate::infra::http::MirrorRule>,
}

/// `archives`：压缩包（mod `content.zip` 等）解密配置（路线 A：运行时配置）。
///
/// 由该发行版作者在自己分发 / 构建的 FV 启动器 `launcher.json` 中预置，不进 FV 公开源码；
/// FV 核心保持通用（见 `vcmi::resource::zip_password_for`）。未配置时回退到 rs 源码内置密码表
/// （路线 B：`vcmi::resource::builtin_zip_passwords`，供发行版自行构建时填入）。
/// 两路都无密码 = 不解密，加密条目按原行为跳过，并在扫描中标记 `contentZipEncrypted`。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ArchivesCfg {
    /// 加密 zip 的密码表：键为 mod 目录名（如 `adeptorium`）或 `_default`（全局兜底），值为解密密码。
    ///
    /// 匹配规则（见 `resource::zip_password_for`）：先按 mod 目录名精确匹配，否则取 `_default`。
    /// 嵌套子 mod 以最末级目录名（磁盘目录名）为准。
    #[serde(default)]
    pub zip_passwords: HashMap<String, String>,
}

/// `update`：启动器自身自动更新（与 VCMI / 模组仓库无关的另一条链）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateCfg {
    /// 更新清单地址（https 直链，指向版本链 JSON）。
    /// 缺省 / 空 = 完全不检查更新（零网络行为，绝不阻塞启动器使用）。
    #[serde(default)]
    pub manifest_url: Option<String>,
    /// 启动时是否自动静默检查更新（默认 true）。手动「检查更新」不受此开关限制。
    #[serde(default = "default_update_auto_check")]
    pub auto_check: bool,
    /// 用户选择「跳过此版本」的版本号（仅对普通更新生效，强制更新忽略）。
    #[serde(default)]
    pub skip_version: Option<String>,
    /// 上次**自动弹出更新提示**的版本号（7 天节流用，见 `updater::manifest::check_update`）。
    /// 语义：同一版本在 `prompt_at` 起 7 天内不再自动弹窗（红点等被动感知不受限）。
    #[serde(default)]
    pub prompt_version: Option<String>,
    /// 上次自动弹出更新提示的时间（Unix 秒）；与 `prompt_version` 配套，由
    /// `mark_update_prompted` 写入（计时从弹窗**展示**起算）。
    #[serde(default)]
    pub prompt_at: Option<u64>,
    /// 记录 `prompt_version` 时的**启动器 exe 版本**（节流只在"exe 版本未变"时生效）。
    ///
    /// 为什么需要它：`prompt_version` 只记**目标版本号**，不记"当时是哪个 exe 版本"，
    /// 于是「已升到 0.2.0 → 手工换回 0.1.0」会被 7 天节流压住 —— 用户明明落后一个版本却不再被提醒。
    /// 有了本字段，exe 版本一变（升级 / 降级 / 手工换包）→ 旧节流记录自动失效、重新评估。
    /// ⚠️ **不依赖"观察到版本切换"**（那种做法会被不含该逻辑的旧构建吃掉过渡），
    ///    且对存量配置天然自愈：本字段缺失即视为节流失效 → 立即提醒。
    #[serde(default)]
    pub prompt_exe_version: Option<String>,
}

impl Default for UpdateCfg {
    fn default() -> Self {
        Self {
            manifest_url: None,
            auto_check: default_update_auto_check(),
            skip_version: None,
            prompt_version: None,
            prompt_at: None,
            prompt_exe_version: None,
        }
    }
}

/// `runtime`：进程级选项 + **外挂运行时组件**（按组件 id 分组，如 `ai`）。
///
/// 组件设计要点：
/// - **通用**：任何"可选的、不随每次更新变化的外部依赖"都注册成一个组件，
///   不再为某个功能平铺 `xxx_runtime_path` / `xxx_package_url` 一组字段。
/// - **文件清单不在这里**：组件需要哪些文件、各自 sha256 是多少，由**代码常量**定义
///   （见 `runtime::component`）—— 那是代码强依赖，让用户填没意义，填错反而有害。
///   这里只解决两件事：**放哪**（`dir`）和 **从哪来**（`package`）。
/// - **缺省即可用**：不配也能工作，走约定目录 `<exe>/.vcmi-fv-launcher/runtime/<id>`。
///
/// 三种分发形态共用同一份逻辑：
/// 便携内置（发行方放好文件）/ 安装包内置（`-c tauri.ai-runtime.conf.json`）/ 远程下载（配 `package`）。
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RuntimeCfg {
    /// 日志等级：`error|warn|info|debug|trace`。缺失或非法回退 `info`。
    /// 发行版 / 开发者可预置（如便携包置 `debug` 以便现场排查）。
    #[serde(default)]
    pub log_level: Option<String>,
    /// 外挂组件表：`components.<id>` → 该组件的落点与远程包。
    #[serde(default)]
    pub components: BTreeMap<String, RuntimeComponentCfg>,
}

/// 单个外挂运行时组件的配置。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuntimeComponentCfg {
    /// 组件目录。相对路径按 **exe 同目录** 解析（沿用 launcher.json 的便携约定）；
    /// 缺省 = `<exe>/.vcmi-fv-launcher/runtime/<id>`。
    #[serde(default, with = "opt_path_serde")]
    pub dir: Option<PathBuf>,
    /// 远程安装包。**配了才允许按需下载**；不配则只能由发行方内置。
    #[serde(default)]
    pub package: Option<RuntimePackage>,
}

/// 远程组件包（zip）。`url` / `sha256` 必填 —— 远程来源必须可校验。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimePackage {
    /// https 直链
    pub url: String,
    /// 压缩包 sha256（小写 hex）
    pub sha256: String,
    /// 声明体积（字节），用于进度与上限校验
    #[serde(default)]
    pub size: Option<u64>,
    /// 组件版本（用于 UI 展示与"是否需要升级"判断）
    #[serde(default)]
    pub version: Option<String>,
}

/// 与前端 `BuiltinPreset` 对齐的内置预设配置结构（见 src/builtinPresets.ts）。
#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct BuiltinPresetCfg {
    pub id: String,
    /// 展示名，键为 VCMI 语言标识（chinese/english/...）
    #[serde(default)]
    pub label: HashMap<String, String>,
    /// 默认开启的模组（通用名单）
    #[serde(default)]
    pub mods: Vec<String>,
    /// 按 VCMI 语言标识启用的不同模组名单
    #[serde(default, rename = "modsByLang")]
    pub mods_by_lang: HashMap<String, Vec<String>>,
    /// 是否锁定（仅展示用；强制启用始终由 `mods.required` 决定）
    #[serde(default)]
    pub locked: bool,
}

fn default_settings_level() -> String {
    "common".to_string()
}

fn default_update_auto_check() -> bool {
    true
}

fn default_auto() -> String {
    "auto".to_string()
}

impl LauncherConfig {
    /// 校验：游戏目录必须含 VCMI_client.exe；用户数据目录非空。
    /// 用户数据目录允许尚不存在（首次进入后由启动器/VCMI 创建）。
    pub fn is_valid(&self) -> bool {
        self.paths.game_dir.join("VCMI_client.exe").is_file()
            && !self.paths.user_data_dir.as_os_str().is_empty()
    }

    /// 便携版支持：将相对的 `game_dir` / `user_data_dir` / `user_config_dir` / `user_mods_dir`
    /// 解析到基准目录（启动器 exe 所在目录，即游戏目录）。
    /// 配置写 `"./"` 即表示"启动器所在的游戏目录"，不依赖解压路径，也与 CWD 无关。
    /// 注意：仅解析，不改写磁盘文件，保留 `launcher.json` 里的相对写法以维持可移植性。
    pub fn resolve_relative_to(mut self, base: &Path) -> Self {
        // 一律走 `normalize_abs`：既解析相对路径，也消除 "." / ".." 成分。
        // 不能用 `base.join`——它会保留 "./"，落盘到 dirs.json 会变成
        // `D:\game\./userdata` 这种 Windows 不接受的形式。
        // 空值保持为空（空表示「后续按 user_data_dir 推导」，不能误填成 base）。
        let p = &mut self.paths;
        if !p.game_dir.as_os_str().is_empty() {
            p.game_dir = normalize_abs(base, &p.game_dir);
        }
        if !p.user_data_dir.as_os_str().is_empty() {
            p.user_data_dir = normalize_abs(base, &p.user_data_dir);
        }
        if !p.user_config_dir.as_os_str().is_empty() {
            p.user_config_dir = normalize_abs(base, &p.user_config_dir);
        }
        if !p.user_mods_dir.as_os_str().is_empty() {
            p.user_mods_dir = normalize_abs(base, &p.user_mods_dir);
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grouped_config_deserializes() {
        let cfg: LauncherConfig = serde_json::from_value(serde_json::json!({
            "paths": { "game_dir": "D:/g", "user_data_dir": "D:/u", "redirect_user_data": true },
            "ui": { "lang": "chinese", "settings_level": "all", "town_preview": true },
            "hd": {
                "enhance": true,
                "usm": 0.5,
                "ai": { "enhance": true, "model": "realesrgan_anime6b.onnx", "mix": 0.6 }
            },
            "mods": { "required": ["h3e-main-menu"] },
            "realms": { "active": "realm-a", "repository_urls": ["https://x/index.json"] },
            "update": { "skip_version": "1.2.3" },
            "runtime": { "log_level": "debug", "components": { "ai": { "dir": "runtime/ai" } } }
        }))
        .expect("分组结构应能反序列化");
        assert_eq!(cfg.paths.game_dir, PathBuf::from("D:/g"));
        assert_eq!(cfg.paths.user_data_dir, PathBuf::from("D:/u"));
        assert!(cfg.paths.redirect_user_data);
        assert_eq!(cfg.ui.lang.as_deref(), Some("chinese"));
        assert_eq!(cfg.ui.settings_level, "all");
        assert!(cfg.ui.town_preview);
        assert!(cfg.hd.enhance && cfg.hd.ai.enhance);
        assert_eq!(cfg.hd.ai.model.as_deref(), Some("realesrgan_anime6b.onnx"));
        assert_eq!(cfg.hd.ai.mix, Some(0.6));
        assert_eq!(cfg.hd.usm, Some(0.5));
        assert_eq!(cfg.mods.required, vec!["h3e-main-menu".to_string()]);
        assert_eq!(cfg.realms.active.as_deref(), Some("realm-a"));
        assert_eq!(cfg.update.skip_version.as_deref(), Some("1.2.3"));
        assert_eq!(cfg.runtime.log_level.as_deref(), Some("debug"));
        assert_eq!(
            cfg.runtime.components["ai"].dir.as_deref(),
            Some(Path::new("runtime/ai"))
        );
    }

    /// 旧版扁平键**不再被识别**：会被解成一份空配置（进而判为无效、走初始化向导），
    /// 而不是"半猜半就"地搬一部分字段 —— 项目不做这层兼容。
    #[test]
    fn legacy_flat_keys_are_ignored() {
        let cfg: LauncherConfig = serde_json::from_value(serde_json::json!({
            "game_dir": "D:/g", "user_data_dir": "D:/u", "hd_enhance": true
        }))
        .expect("未知顶层键不应导致反序列化失败");
        assert!(cfg.paths.game_dir.as_os_str().is_empty());
        assert!(!cfg.hd.enhance);
        assert!(!cfg.is_valid());
    }

    #[test]
    fn group_defaults_are_stable() {
        let cfg: LauncherConfig =
            serde_json::from_value(serde_json::json!({})).expect("空对象应可反序列化");
        assert_eq!(cfg.ui.settings_level, "common");
        assert!(cfg.update.auto_check);
        assert_eq!(cfg.hd.edition_use, "auto");
        assert_eq!(cfg.hd.edition_scale, "auto");
        assert!(!cfg.hd.enhance);
        // 无默认模型：不写 `hd.ai` → 开关关、模型空 ⇒ AI 不生效（见 `AiCfg` 的四重合取）
        assert!(!cfg.hd.ai.enhance);
        assert!(cfg.hd.ai.model.is_none());
        assert!(cfg.hd.ai.mix.is_none());
        assert!(cfg.runtime.log_level.is_none());
        assert!(cfg.runtime.components.is_empty());
    }
}
