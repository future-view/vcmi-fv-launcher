//! 缓存素材落盘：图标 / DEF 帧 / 特长图标的提取与保存（受线程局部素材开关控制）。
//!
//! 产物写入 `mods/<sanitize_id(owner)>/images/<sub>/<资源标识[~帧]>.webp`，返回**相对缓存根**的路径。
//!
//! `owner` 是**素材来源方**（游戏级素材 → `core`、跨模组引用 → 被引用方、自有素材 → 自己），
//! 文件名由**资源标识 + 帧**决定。二者合力保证：同一份素材无论被多少个模组、多少个条目提取，
//! 磁盘上只存一份（实测原版地形曾被 124 个模组各存一份、`@4x` 变体各放大一遍）。
//!
//! 依赖方向：可依赖 `infra` 与 `vcmi`（见 `docs/architecture/backend.zh-CN.md` §4.1）。

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::Value;

use crate::infra::hash::fnv1a_of;
use crate::infra::image::{encode_rgba_auto, encode_webp, encode_webp_from_bgra};
use crate::infra::json::parse_vcmi_json;
use crate::modcache::{assets_enabled, sanitize_asset_name, sanitize_id};
use crate::vcmi::resource::{
    resolve_resource, resolve_resource_fallback, resolve_resource_origin, Origin,
};

/// webp 编码结果缓存（内容寻址）：`(源字节指纹, 源长度, 扩展名) → webp 字节`。
///
/// ⚠️ 必要性：跨 mod 共享的同一份源素材（原版建筑图标 `HALLCSTL.DEF` 的同一帧、
/// 原版英雄头像 `TPLICHxx`、特长/技能图标等）此前**每个 mod 都要重新解码 + 编码一遍**——
/// 实测 4.9 万次 `img.icon` ≈ 3,440s CPU。缓存后同一份源字节只编码一次。
///
/// ⚠️ 产物仍按原相对路径逐个落盘（`img.cached` / `img.saved` 语义与前端路径契约均不变），
/// 这里省掉的只是重复的解码 + 编码算力。
/// 编码缓存的内部结构（`clippy::type_complexity` 要求拆出别名）。
///
/// 容量按**字节上界**（`mem::cache_budget().image_bytes`，随机器内存分档）而非条目数：
/// webp 产物从几 KB 到几百 KB 不等，按条目数设限无法约束真实占用。
type EncodeCache = Mutex<crate::infra::lru::SizedCache<(u64, usize, String), Arc<Vec<u8>>>>;
static ENCODE_CACHE: OnceLock<EncodeCache> = OnceLock::new();
/// 只缓存小图：城镇幕布图等大图基本不重复，缓存它们只会白占内存。
const ENCODE_CACHE_MAX_SRC: usize = 256 * 1024;

/// 已成功创建的输出目录（`images/<sub>/`）。
///
/// ⚠️ 原实现**每张图都 `create_dir_all` 一次**（实测 4 万+ 张图 → 4 万次目录系统调用，
/// 每次都要 stat 各级父目录）。产物目录总数只有数十个，记下来即可省掉绝大部分调用。
static DIRS_READY: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();

/// 确保目录存在（已确认过的目录不再系统调用）。
fn ensure_dir(dir: &Path) -> bool {
    let set = DIRS_READY.get_or_init(|| Mutex::new(HashSet::new()));
    if let Ok(g) = set.lock() {
        if g.contains(dir) {
            return true;
        }
    }
    if fs::create_dir_all(dir).is_err() {
        return false;
    }
    if let Ok(mut g) = set.lock() {
        g.insert(dir.to_path_buf());
    }
    true
}

/// 释放素材侧缓存（构建全部结束后调用；见 [`crate::infra::cache_gc`]）。
///
/// 编码/帧缓存按机器内存分档各占一份图片预算（16~32 GB 机器合计近 200 MB），
/// `DIRS_READY` 是纯加速用的目录集合。三者都可重建，构建完没有保留价值。
pub(crate) fn release_caches() {
    crate::infra::lru::clear_sized(ENCODE_CACHE.get());
    crate::infra::lru::clear_sized(FRAME_CACHE.get());
    if let Some(c) = DIRS_READY.get() {
        if let Ok(mut g) = c.lock() {
            g.clear();
            g.shrink_to_fit();
        }
    }
}

/// DEF 单帧解码缓存（内容寻址）：`(源字节指纹, 长度, group, frame) → BGRA 帧`。
///
/// ⚠️ 必要性：与编码缓存同源——同一份 DEF 帧被多个 mod / 多个条目重复解码
/// （实测 `img.icon` 4.9 万次，DEF 解码占其中可观一块）。命中后省掉解码 + 逐像素还原。
///
/// 只缓存 ≤2 MB 的帧（超大帧缓存收益低、占预算快）；与编码缓存共用图片预算，按 FIFO 淘汰。
type FrameCache = Mutex<crate::infra::lru::SizedCache<(u64, usize, u16, u16), Arc<DefFrame>>>;
static FRAME_CACHE: OnceLock<FrameCache> = OnceLock::new();
/// 单帧上限（BGRA 字节；800×600 全屏帧约 1.9 MB）
const FRAME_CACHE_MAX_BYTES: usize = 2 * 1024 * 1024;

/// `DefReader::decode_frame_at` 的结果（去掉左/上边距两个 i32 后的可缓存形态）。
#[derive(Clone)]
struct DefFrame {
    w: u32,
    h: u32,
    bgra: Vec<u8>,
    #[allow(dead_code)]
    left: i32,
    #[allow(dead_code)]
    top: i32,
}

/// 带缓存的 DEF 单帧解码。
fn decode_def_frame_cached(buf: &[u8], group: u16, frame: u16) -> Option<DefFrame> {
    let cap = crate::infra::mem::cache_budget().image_bytes;
    let cache = FRAME_CACHE.get_or_init(|| Mutex::new(crate::infra::lru::SizedCache::new(cap)));
    let key = (fnv1a_of(buf), buf.len(), group, frame);
    if let Ok(g) = cache.lock() {
        if let Some(v) = g.get(&key) {
            crate::infra::metrics::count("img.decode.reuse", 1);
            return Some(v.as_ref().clone());
        }
    }
    let def = crate::infra::def::DefReader::new(buf);
    let (w, h, bgra, lm, tm) = def.decode_frame_at(group as usize, frame as usize)?;
    let f = DefFrame {
        w,
        h,
        bgra,
        left: lm,
        top: tm,
    };
    if f.bgra.len() <= FRAME_CACHE_MAX_BYTES {
        if let Ok(mut g) = cache.lock() {
            g.insert(key, f.bgra.len(), Arc::new(f.clone()));
        }
    }
    Some(f)
}

/// 素材来源 → **产物归属者**（产物落在谁的缓存目录下）。
///
/// 这是缓存去重的关键：产物归属**素材真正的来源方**，而非提取调用方。
/// - 游戏级素材（`Origin::Game`）→ `core`：原版地形/英雄特长/生物图只落一份，124 个 mod 共用；
/// - 跨模组引用（`Origin::Mod(id)`）→ 被引用方：A 借用 B 的精灵时产物落在 B 目录，A 只引用路径；
/// - 本模组素材（`Origin::SelfMod`）→ 自己：第三方自有素材仍在各自目录，保持自包含。
///
/// 与既有 `global.rs`「游戏级图标归位 core」的方向一致，只是判定改为由解析结果给出，
/// 不再靠"调用方猜"。
///
/// ⚠️ **`core.<faction>` 伪模组的 `SelfMod` 必须归 `core`**：它们没有自己的磁盘目录，
/// 构建时的 `mod_path` 就是 VCMI 基础模组（`Mods/vcmi`），所以"本模组命中"实际等于
/// "VCMI 核心自带素材"。若按 `mod_id` 归位，9 个城镇 + 父 core 会各写一份完全相同
/// 的文件（实测 2026-10-20：`battle/actionGenie.png` 在 `core/images/spell/` 与
/// 9 个 `core.<faction>/images/spell/` 各一份，共 10 份）。
fn owner_of(origin: &Origin, mod_id: &str) -> String {
    match origin {
        Origin::Game => "core".to_string(),
        Origin::SelfMod if is_core_scope(mod_id) => "core".to_string(),
        Origin::SelfMod => mod_id.to_string(),
        Origin::Mod(id) => id.clone(),
    }
}

/// 是否原版伪模组（`core` / `core.<faction>`）——与 `vcmi::h3core::parse_core_mod` 同口径。
fn is_core_scope(mod_id: &str) -> bool {
    crate::vcmi::h3core::parse_core_mod(mod_id).0
}

/// 产物名：**资源标识 + 帧**，而不是条目名。
///
/// 这是消除「同一份素材被多个条目各存一份」的核心：
/// - 实测 `core.rampart` 的 7 个建筑共用同一个 `buildingsIcons` 帧、
///   68 个生物共用「飞行」能力图标 —— 按条目名落盘就各存一份（同内容不同名）；
/// - 按资源标识命名后，它们天然指向同一个文件（`...~g0f0.webp`）。
///
/// 帧标识约定：DEF → `g<group>f<frame>`；sprites-json / 序列位图 → `f<index>`；整图 → 省略。
/// `hd.rs` 的变体命名是 `<stem>@Nx`，该格式对它无影响（`@` 仍在末尾）。
fn asset_name(res: &str, frame: Option<&str>) -> String {
    let key = crate::vcmi::resource::resource_asset_key(res);
    match frame {
        Some(f) => format!("{key}~{f}"),
        None => key,
    }
}

/// 「产物路径 ← 资源引用」声明表（进程级）。用途见 [`note_asset_declared`] 与
/// [`flush_asset_manifest`]：撞名探测 + 诊断落盘。
static ASSET_DECLARED: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

/// 记录「产物路径 ← 资源引用」的首次声明，探测文件名冲突。
///
/// `resource_asset_key` 的 `_` 归一存在单射性瑕疵（`a/b` 与 `a_b` 会撞）。
/// 撞名意味着**两个不同素材共用一个文件**，属静默错图级别 —— 必须可观测。
fn note_asset_declared(owner: &str, sub: &str, fname: &str, res: &str) {
    let map = ASSET_DECLARED.get_or_init(|| Mutex::new(HashMap::new()));
    let key = format!("{owner}/{sub}/{fname}");
    let mut g = match map.lock() {
        Ok(g) => g,
        Err(e) => e.into_inner(),
    };
    match g.get(&key) {
        Some(prev) if prev != res => {
            crate::infra::metrics::count("img.name_collision", 1);
            eprintln!(
                "[assets] 产物名冲突：{key} 同时由 `{prev}` 与 `{res}` 声明 → 其中一方会显示错误图片"
            );
        }
        Some(_) => {}
        None => {
            g.insert(key, res.to_string());
        }
    }
}

/// 把「产物路径 ← 资源引用」声明表落盘为 `<cacheRoot>/asset-manifest.json`。
///
/// **纯诊断产物，不参与命名，也不影响缓存正确性**：文件名现在是「资源标识 + 帧」，
/// 一眼能看出取自哪个资源，但要拼回原始引用（含虚拟前缀与大小写）仍需查表。
/// 每批次末调用一次（清空后重填，避免跨批次残留失效条目）。
pub(crate) fn flush_asset_manifest(cache_root: &Path) {
    let map = ASSET_DECLARED.get_or_init(|| Mutex::new(HashMap::new()));
    let taken = {
        let mut g = match map.lock() {
            Ok(g) => g,
            Err(e) => e.into_inner(),
        };
        std::mem::take(&mut *g)
    };
    if taken.is_empty() {
        return;
    }
    if let Ok(text) = serde_json::to_string_pretty(&taken) {
        let _ = fs::write(cache_root.join("asset-manifest.json"), text);
        crate::infra::metrics::count("asset.manifest", taken.len() as u64);
    }
}

/// 缓存图片的相对路径（相对缓存根）。落盘与"是否已存在"判定必须**同一口径**，否则短路会误判。
///
/// ⚠️ 单点生成是硬要求：存在性判定 / 落盘 / `cache.json` 写入 / `hd_map` 登记四处必须共用它。
/// 任一处另算路径 → 出现「cache.json 存 A 径、实际写在 B 径」，比路径超长难查得多。
/// （文件名安全化在 `modcache::sanitize_asset_name`——与 `sanitize_id` 是两套口径，见其注释。）
///
/// `owner` 是**素材来源方**（见 [`owner_of`]），不是提取调用方。
pub(crate) fn wiki_image_rel(owner: &str, sub: &str, name: &str) -> String {
    format!(
        "{}/images/{}/{}.webp",
        sanitize_id(owner),
        sub,
        sanitize_asset_name(name)
    )
}

/// 完整路径预算（字符数），对应 Windows 默认 `MAX_PATH = 260` 留出安全余量。
///
/// 注意：**换成内容短哈希并不能根治长路径** —— 路径前缀（`<游戏目录>/.vcmi-fv-launcher/mods`
/// + 嵌套 modId，实测最长 71 字符）由用户安装位置决定，文件名方案改不了它。
/// 因此这里只做"逐路径预算 + 超限降级"：把文件名段压到最短，把余量全让给前缀。
const PATH_BUDGET: usize = 240;

/// 带路径预算的产物路径（相对缓存根）：超预算时文件名降级为定长短哈希名。
///
/// 降级仍需保证**唯一**（否则不同素材互相覆盖 → 静默错图），故用资源标识的
/// 64 位指纹而非截断原始名（截断会让前缀相同的两个长名撞在一起）。
fn resolve_rel(cache_root: &Path, owner: &str, sub: &str, name: &str) -> String {
    let rel = wiki_image_rel(owner, sub, name);
    if cache_root.join(&rel).to_string_lossy().len() <= PATH_BUDGET {
        return rel;
    }
    let short = format!("{:016x}", fnv1a_of(name.as_bytes()));
    crate::infra::metrics::count("img.path_fallback", 1);
    wiki_image_rel(owner, sub, &short)
}

/// 产物是否已存在（命中返回相对路径）。
///
/// ⚠️ **必须在资源解析 / DEF 解码 / webp 编码之前调用**。
/// 短路原先只在 `save_wiki_image` 的落盘前判断，于是：
/// - 阶段一（只出数据、不落盘）仍把每个图标的资源解析、DEF 解码、抠图、编码**全做一遍**再丢弃；
/// - 跨 mod 共享图标（statIcons / sharedIcons / 技能图标）每个 mod 都重新编码一遍再丢弃。
/// 实测这两项合计占全量构建 CPU 的三成以上（见运行日志 `本批耗时构成`）。
pub(crate) fn wiki_image_exists(
    cache_root: &Path,
    owner: &str,
    sub: &str,
    name: &str,
) -> Option<String> {
    if !assets_enabled() {
        return None;
    }
    let rel = resolve_rel(cache_root, owner, sub, name);
    if cache_root.join(&rel).is_file() {
        crate::infra::metrics::count("img.reuse", 1);
        Some(rel)
    } else {
        None
    }
}

/// 落盘（归属 `owner` 目录，见 [`owner_of`]）。
///
/// ⚠️ `owner` 是素材来源方：同一份素材被 N 个 mod 提取时，产物路径相同 →
/// 首个 mod 落盘后其余全部命中短路，**既不重复写也不重复解码/编码**。
pub(crate) fn save_wiki_image(
    cache_root: &Path,
    owner: &str,
    sub: &str,
    name: &str,
    buf: &[u8],
    ext: &str,
) -> Option<String> {
    // 阶段一（只出数据）不产图：调用方拿到 None，实体 image 字段留空，由阶段二补齐
    if !assets_enabled() {
        return None;
    }
    let rel = resolve_rel(cache_root, owner, sub, name);
    let out = cache_root.join(&rel);
    // 产物幂等：已存在则跳过解码+编码。增量重跑、中断续跑时省下大量 CPU。
    // 缓存失效重建前会清空该 mod 的 images/ 目录（见 build_mod_wiki_inner），故不会残留旧版本产物。
    if out.is_file() {
        // 幂等命中：增量重跑/中断续跑时省下解码+编码（埋点用于统计复用率）
        crate::infra::metrics::count("img.cached", 1);
        return Some(rel);
    }
    let webp = encode_webp_cached(buf, ext)?;
    // 目录创建结果记忆化（见 `ensure_dir`）：原实现每张图都 create_dir_all 一次
    ensure_dir(out.parent()?).then_some(())?;
    fs::write(&out, &webp).ok()?;
    crate::infra::metrics::count("img.saved", 1);
    crate::infra::metrics::bytes("img.bytes.disk", webp.len() as u64);
    Some(rel)
}

/// 带缓存的 webp 编码（见 [`ENCODE_CACHE`]）：大图直接编码不入缓存。
fn encode_webp_cached(buf: &[u8], ext: &str) -> Option<Vec<u8>> {
    if buf.len() > ENCODE_CACHE_MAX_SRC {
        return encode_webp(buf, ext);
    }
    let cap = crate::infra::mem::cache_budget().image_bytes;
    let cache = ENCODE_CACHE.get_or_init(|| Mutex::new(crate::infra::lru::SizedCache::new(cap)));
    let key = (fnv1a_of(buf), buf.len(), ext.to_string());
    if let Ok(g) = cache.lock() {
        if let Some(v) = g.get(&key) {
            crate::infra::metrics::count("img.encode.reuse", 1);
            return Some(v.as_ref().clone());
        }
    }
    let v = Arc::new(encode_webp(buf, ext)?);
    if let Ok(mut g) = cache.lock() {
        g.insert(key, v.len(), v.clone());
    }
    Some(v.as_ref().clone())
}

/// 描边图接 HD 的**形状校验**：HD 帧必须与 1x 同宽高比（±2%）且为整数倍。
///
/// 原版 Castle 实测 `TOCSMAG1` 的 x3 帧恰好是 1x 的 3 倍（见 `extract_border_and_save`
/// 的注释），整数倍 + 同比例说明两者是同一套美术；mod 自带描边若沿用原名但比例不同，
/// 直接拒接 —— 否则整张图会被换成原版素材。
fn hd_shape_matches(res: &str, w: u32, h: u32) -> bool {
    let Some((img, _scale)) = crate::vcmi::hd_edition::lookup(res) else {
        return false;
    };
    let (hw, hh) = (img.width(), img.height());
    if hw == 0 || hh == 0 || w == 0 || h == 0 {
        return false;
    }
    let ar1 = w as f64 / h as f64;
    let ar2 = hw as f64 / hh as f64;
    (ar1 - ar2).abs() / ar1 < 0.02 && hw % w == 0 && hh % h == 0
}

/// 位图类资源登记：命中官方 HD（`hd-edition`）时把 `<sub>/<name>` → HD 键 写入登记表。
///
/// 与 DEF 帧类的区别：这里用**资源名**（去路径 / 去扩展名由 `hd_edition::candidate_keys` 负责），
/// 无需读 DEF 头，成本仅一次哈希查表。
/// 登记键必须与产物路径同属一个 owner：HD 阶段按 `<sub>/<name>` 在 **owner** 的表里取源，
/// 若登记归到调用方、产物却落在来源方，HD 阶段查表必然落空。
fn note_hd_bitmap(owner: &str, sub: &str, name: &str, res: &str, origin: Option<&Origin>) {
    // 默认只对**原版资源**（`Origin::Game`）或 core 伪模组产物登记。
    // HD 键属于原版命名空间；mod 自带资源常**沿用原名**装自己的美术 —— 实测
    // 1-knight-town 的 `CPrS005C.bmp` 里是长枪兵，而 pak 的 `CPRS005C` 是另一种生物，
    // 不设防就会把 mod 的美术整张换成原版高清图（用户实机反馈：@3x/@4x 素材错误）。
    // `origin = None` 表示调用方自带等价守卫（见 `hd_shape_matches`）。
    let allow = match origin {
        Some(o) => matches!(o, Origin::Game) || is_core_scope(owner),
        None => true,
    };
    if !allow {
        return;
    }
    if let Some(hd) = crate::vcmi::hd_edition::probe(res) {
        crate::modcache::hd_map::record(owner, sub, name, &hd);
    }
}

/// 位图类资源落盘：**HD 命中则用官方 HD 图**（尺寸大，走有损 q85），否则原样落盘。
///
/// 与 DEF 帧类不同，位图类不需要帧名换算 —— `res` 的基名（`TPCASCAS`、`TBCSBACK.bmp`）
/// 就是 pak 的键（去路径/去扩展名由 `hd_edition::candidate_keys` 负责）。
///
/// 用途：兵种背景（`creatureBackground`）等"整图直取"的合成类素材。
pub(crate) fn save_hd_or_raw_image(
    cache_root: &Path,
    mod_id: &str,
    sub: &str,
    name: &str,
    res: &str,
    buf: &[u8],
    ext: &str,
) -> Option<String> {
    if let Some((img, scale)) = crate::vcmi::hd_edition::lookup(res) {
        if scale >= 2 && img.width() > 0 && img.height() > 0 {
            if let Some(webp) = crate::infra::image::encode_webp_lossy_q(
                img.as_raw(),
                img.width(),
                img.height(),
                crate::modcache::hd::HD_QUALITY,
            ) {
                return save_wiki_image(cache_root, mod_id, sub, name, &webp, "webp");
            }
        }
    }
    save_wiki_image(cache_root, mod_id, sub, name, buf, ext)
}

/// 解析单个资源字段并转 webp 保存，返回相对 `images/` 路径。
/// 小图标（≤64px）自动执行**四角取色抠图**：mod 自带的 png/webp 图标本体常带蓝底/青底，
/// VCMI 端无此处理但视觉上期望透明底（绿幕式抠图）。容差 48 覆盖抗锯齿边缘。
pub(crate) fn extract_icon_and_save(
    p: &Path,
    game_dir: &Path,
    cache_root: &Path,
    mod_id: &str,
    sub: &str,
    name: &str,
    res: Option<&str>,
) -> Option<String> {
    crate::infra::metrics::scope("img.icon", || {
        extract_icon_and_save_inner(p, game_dir, cache_root, mod_id, sub, name, res)
    })
}

fn extract_icon_and_save_inner(
    p: &Path,
    game_dir: &Path,
    cache_root: &Path,
    mod_id: &str,
    sub: &str,
    _name: &str,
    res: Option<&str>,
) -> Option<String> {
    // 阶段一（只出数据）：**在解析资源之前**直接返回，省下 resolve/解码/抠图/编码
    if !assets_enabled() {
        return None;
    }
    let res = res?;
    let res_trim = res.trim();
    if res_trim.is_empty() {
        return None;
    }
    //
    // ⚠️ 顺序与旧实现相反：**先解析、后短路**。
    // 产物归属由素材来源决定（游戏级 → `core`、跨模组引用 → 被引用方），不解析就不知道
    // 该去哪个 owner 目录查。代价是短路不再能省掉 resolve；收益是跨 mod 共享素材
    // （实测原版地形 124 份、跨模组引用的精灵）只落一份盘、只解码编码一次 —— 远大于代价。
    // 且 resolve 侧已有「目录索引 / zip 索引 / LOD 条目表」三级缓存，单次成本本就很低。
    let (buf, ext, origin) = resolve_resource_origin(p, game_dir, res_trim)?;
    let owner = owner_of(&origin, mod_id);
    //
    // 产物名由**资源标识 + 帧**决定，而非条目名（见 `asset_name` 的说明）。
    // 帧在这里即可确定（DEF 取首帧、sprites-json 取首帧、整图无帧），
    // 因此短路仍然发生在**解码之前**。
    let fname = if ext.eq_ignore_ascii_case("def") {
        asset_name(res_trim, Some("g0f0"))
    } else if ext.eq_ignore_ascii_case("json") || res_trim.to_lowercase().ends_with(".json") {
        asset_name(res_trim, Some("f0"))
    } else {
        asset_name(res_trim, None)
    };
    note_asset_declared(&owner, sub, &fname, res_trim);
    // 产物已存在：直接复用路径（省掉解码 + 编码 + 落盘）
    if let Some(rel) = wiki_image_exists(cache_root, &owner, sub, &fname) {
        return Some(rel);
    }
    // sprites-json 资源（VCMI 1.6+）：**按内容识别**（resolve 的尾匹配可能命中同名 .def，
    // 故 ext==json 或引用尾缀 .json 时都尝试解析）；支持 sequences 形态、basepath 缺省、
    // file 自带扩展名（bmp/png）与 zip 内定义（见 read_sprite_json / sprite_json_frame）。
    let (buf, ext) =
        if ext.eq_ignore_ascii_case("json") || res_trim.to_lowercase().ends_with(".json") {
            match read_sprite_json(p, res_trim).and_then(|v| sprite_json_frame(&v, 0, 0)) {
                Some((base, file)) => {
                    let mut got: Option<(Vec<u8>, String)> = None;
                    for cand in sprite_ref_candidates(base.as_deref(), &file) {
                        if let Some(r) = resolve_resource_fallback(p, game_dir, &cand) {
                            got = Some(r);
                            break;
                        }
                    }
                    got.unwrap_or((Vec::new(), String::new()))
                }
                None => (buf, ext),
            }
        } else {
            (buf, ext)
        };
    if buf.is_empty() {
        return None;
    }
    // DEF 资源（如 HOTA 的 spells/xxx.def、mod 图标 xxx.def）：取 group0 首帧。
    // 首帧常带法术卷轴/宝物底框色（青蓝），小帧做四角抠图去底（对齐小图标 matte）。
    if ext.eq_ignore_ascii_case("def") {
        // 走解码缓存（见 `FRAME_CACHE`）：同一份 DEF 首帧被同一 mod 的多个条目重复解码
        let Some(f) = decode_def_frame_cached(&buf, 0, 0) else {
            return None;
        };
        let (w, h, bgra) = (f.w, f.h, f.bgra);
        let rel = match save_def_frame_matte(cache_root, &owner, sub, &fname, w, h, &bgra) {
            Some(rel) => Some(rel),
            None => {
                let webp = encode_webp_from_bgra(&bgra, w, h)?;
                save_wiki_image(cache_root, &owner, sub, &fname, &webp, "webp")
            }
        };
        // 登记该帧的官方 HD 源（此前本分支漏了这步 → 走这条路径的素材永远从 1x 插值）。
        // 实测 2026-10-20：地形 tiles（`DIRTTL.DEF` 首帧 `TDTS000`、`ROCKTL.DEF` 首帧
        // `ROCKTL01`）在 hd-edition 里**确实有**高清源，却因缺少登记而只生成普通放大变体。
        // 与 `extract_def_frame_and_save` 同口径：帧名→HD 只对**原版** DEF 生效，
        // mod 自带 DEF 的帧名可能撞上原版资源名（会把无关的原版高清图塞进来）。
        if rel.is_some() && matches!(origin, Origin::Game) {
            if let Some(hd) = crate::vcmi::hd_edition::probe_def_frame(&buf, 0, 0) {
                crate::modcache::hd_map::record(&owner, sub, &fname, &hd);
            }
        }
        return rel;
    }
    // 无 alpha 的位图（bmp/png）常拿纯青 `(0,255,255)` 当透明底（H3 色键，见
    // [`crate::infra::image::keyed_rgba`]）：实测 `hota.Factory` 的法术图标
    // `iconCancel.bmp`（78×65）整张带青底。命中就先抠干净再落盘，否则青底会原样进缓存。
    // ⚠️ 只处理**位图**：DEF 帧的透明已由 `palette[0]` 色键处理，且 78×65 这类超过
    // matte 阈值（≤64px）的帧正是此前漏网的那一档。
    let (buf, ext) = match crate::infra::image::strip_color_key_rgba(&buf) {
        Some(rgba) => {
            let mut webp = Vec::new();
            match image::codecs::webp::WebPEncoder::new_lossless(&mut webp).encode(
                rgba.as_raw(),
                rgba.width(),
                rgba.height(),
                image::ExtendedColorType::Rgba8,
            ) {
                Ok(()) => (webp, String::from("webp")),
                Err(_) => (buf, ext),
            }
        }
        None => (buf, ext),
    };
    // 四角抠图：仅对 ≤64px 的小图标且源为位图（png/bmp/webp）时执行
    let small_icon = matches!(ext.to_lowercase().as_str(), "png" | "bmp" | "webp");
    if small_icon {
        let rel = crate::infra::metrics::scope("img.matte", || {
            matte_and_save(cache_root, &owner, sub, &fname, &buf)
        });
        if let Some(rel) = rel {
            note_hd_bitmap(&owner, sub, &fname, res_trim, Some(&origin));
            return Some(rel);
        }
    }
    let rel = save_wiki_image(cache_root, &owner, sub, &fname, &buf, &ext)?;
    // 位图类资源也可能有官方 HD 版（如第三方 mod 引用原版 `HPL000KN.pcx`）→ 登记
    note_hd_bitmap(&owner, sub, &fname, res_trim, Some(&origin));
    Some(rel)
}

/// DEF 单帧（group0/frame0）四角抠图去底框色后落盘：小帧（≤64px，如 SPELLBK/ARTIFACT 32×32、
/// HOTA 法术/宝物 def 首帧）常带青蓝底框，期望透明底（对齐小图标 matte 逻辑，见 [`matte_rgba`]）。
fn save_def_frame_matte(
    cache_root: &Path,
    owner: &str,
    sub: &str,
    name: &str,
    w: u32,
    h: u32,
    bgra: &[u8],
) -> Option<String> {
    if w > 64 || h > 64 {
        return None;
    }
    let mut rgba = bgra_to_rgba(w, h, bgra);
    matte_rgba(&mut rgba);
    let webp = encode_rgba_auto(rgba.as_raw(), w, h)?;
    save_wiki_image(cache_root, owner, sub, name, &webp, "webp")
}

/// BGRA（DefReader 输出，顶向下）转 RGBA 像素缓冲（交换 R/B）。
fn bgra_to_rgba(w: u32, h: u32, bgra: &[u8]) -> image::RgbaImage {
    let n = (w as usize * h as usize * 4).min(bgra.len());
    let mut v = bgra[..n].to_vec();
    for px in v.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    image::RgbaImage::from_raw(w, h, v).unwrap_or_else(|| image::RgbaImage::new(w, h))
}

/// 四角取色抠图（绿幕式）：小图标/小 DEF 帧常带蓝底/青底框色，期望透明底。
/// 仅把与任一不透明角点颜色相近（容差 48，覆盖抗锯齿边缘）的像素置透明；
/// 四角全透明则原地返回（不破坏本就透明的帧，如多数建筑图标）。
fn matte_rgba(img: &mut image::RgbaImage) {
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return;
    }
    let corners = [
        *img.get_pixel(0, 0),
        *img.get_pixel(w - 1, 0),
        *img.get_pixel(0, h - 1),
        *img.get_pixel(w - 1, h - 1),
    ];
    let opaque: Vec<[u8; 4]> = corners.iter().map(|p| p.0).filter(|c| c[3] > 0).collect();
    if opaque.is_empty() {
        return;
    }
    let dist = |a: [u8; 4], b: [u8; 4]| -> i32 {
        (a[0] as i32 - b[0] as i32).abs()
            + (a[1] as i32 - b[1] as i32).abs()
            + (a[2] as i32 - b[2] as i32).abs()
    };
    for px in img.pixels_mut() {
        if px.0[3] == 0 {
            continue;
        }
        if opaque.iter().any(|c| dist(*c, px.0) < 48) {
            px.0[3] = 0;
        }
    }
}

/// 小图标（≤64px 位图）四角取色抠图后落盘：mod 自带图标常带蓝底/青底，
/// VCMI 端无此处理但视觉上期望透明底（绿幕式抠图），容差 48 覆盖抗锯齿边缘。
/// 不适用（尺寸超限 / 无透明角 / 编码失败）时返回 None，由调用方走常规编码路径。
fn matte_and_save(
    cache_root: &Path,
    owner: &str,
    sub: &str,
    name: &str,
    buf: &[u8],
) -> Option<String> {
    let img = image::load_from_memory(buf).ok()?;
    if img.width() > 64 || img.height() > 64 {
        return None;
    }
    let mut rgba = img.to_rgba8();
    matte_rgba(&mut rgba);
    let (w, h) = (rgba.width(), rgba.height());
    let mut webp = Vec::new();
    if image::codecs::webp::WebPEncoder::new_lossless(&mut webp)
        .encode(rgba.as_raw(), w, h, image::ExtendedColorType::Rgba8)
        .is_ok()
    {
        return save_wiki_image(cache_root, owner, sub, name, &webp, "webp");
    }
    None
}

/// 解析 DEF 资源（或裸 .def 路径），按 (group, index) 取出指定帧并转 webp 保存。
/// 用于建筑图标（城镇 town-screen 的 DEF 帧）。
pub(crate) fn extract_def_frame_and_save(
    p: &Path,
    game_dir: &Path,
    cache_root: &Path,
    mod_id: &str,
    sub: &str,
    _name: &str,
    def_res: &str,
    group: usize,
    index: usize,
) -> Option<String> {
    let def_res = def_res.trim();
    if def_res.is_empty() {
        return None;
    }
    // 阶段一不解析
    if !assets_enabled() {
        return None;
    }
    // 先解析取归属、后短路（理由见 `extract_icon_and_save_inner` 的同款注释）
    let (buf, _ext, origin) = resolve_resource_origin(p, game_dir, def_res)?;
    let owner = owner_of(&origin, mod_id);
    // 产物名 = 资源标识 + 帧：同一帧被多个条目引用时只落一份
    let frame = format!("g{group}f{index}");
    let fname = asset_name(def_res, Some(&frame));
    note_asset_declared(&owner, sub, &fname, def_res);
    if let Some(rel) = wiki_image_exists(cache_root, &owner, sub, &fname) {
        return Some(rel);
    }
    // 只解目标帧：此前解码整个动画再挑一帧（DEF 动辄上百帧）
    let def = crate::infra::def::DefReader::new(&buf);
    let (w, h, bgra, _lm, _tm) = def.decode_frame_at(group, index)?;
    let rel = save_wiki_image(
        cache_root,
        &owner,
        sub,
        &fname,
        &encode_webp_from_bgra(&bgra, w, h)?,
        "webp",
    )?;
    // 该帧若有官方 HD 真高清源则登记（HD 变体阶段据此取源放大，替代从 1x 插值）。
    //
    // ⚠️ **只对原版资源开放**（`Origin::Game`）：帧名→HD 键是我们为原版 DEF 加的启发式
    // （原版 DEF 的 13 字节帧名就是 HD pak 的键），并非 VCMI 语义 —— VCMI 只按资源名匹配，
    // 绝不会把原版图塞进 mod 自带的 DEF。mod 自带 DEF 的帧名常**沿用原版命名**：
    // 实测 1-knight-town 的 `Knight/HALLCSTL.def` 帧名叫 `THCSMAG1`（原版 H3 魔法行会），
    // 不设防就会把原版建筑图替换掉该 mod 的 H2 风美术（用户实机反馈：@3x 用了错误素材）。
    if matches!(origin, Origin::Game) {
        if let Some(hd) = crate::vcmi::hd_edition::probe_def_frame(&buf, group, index) {
            crate::modcache::hd_map::record(&owner, sub, &fname, &hd);
        }
    }
    Some(rel)
}

/// 从游戏目录（磁盘散文件或 Data/*.lod 包）解析 DEF 并提取指定 (group, frame) 帧转 webp 保存。
/// 用于 H3 原版资源（SMALRES、UN32/UN44、stackWindow/* 等，均在 H3sprite.lod 内）。
pub(crate) fn extract_game_def_frame_and_save(
    game_dir: &Path,
    cache_root: &Path,
    _mod_id: &str,
    def_name: &str,
    group: usize,
    frame: usize,
    sub: &str,
    _name: &str,
) -> Option<String> {
    // 阶段一（只出数据）不解 DEF 取帧，由阶段二补齐
    if !assets_enabled() {
        return None;
    }
    let target = if def_name.to_uppercase().ends_with(".DEF") {
        def_name.trim().to_string()
    } else {
        format!("{}.DEF", def_name.trim())
    };
    // 产物名 = 资源标识 + 帧：原版同一帧（如 68 个生物共用的「飞行」图标）只落一份
    let frame_tag = format!("g{group}f{frame}");
    let fname = asset_name(&target, Some(&frame_tag));
    note_asset_declared("core", sub, &fname, &target);
    // 原版游戏级图标统一归位到父 `core` 模组的 images/ 下：产物已存在则直接复用，
    // 不再读 lod、不再解码 DEF（此前每个 mod 都完整重做一遍）
    if let Some(rel) = wiki_image_exists(cache_root, "core", sub, &fname) {
        return Some(rel);
    }
    let bytes = crate::vcmi::resource::read_game_resource_fallback(game_dir, &target)?;
    // 只解目标帧：此前解码整个动画（CPRSMALL 150+ 帧只取 1 帧）
    let def = crate::infra::def::DefReader::new(&bytes);
    let (w, h, bgra, _lm, _tm) = def.decode_frame_at(group, frame)?;
    // 小帧（SPELLBK/ARTIFACT 32×32 等）含底框色（青蓝），四角抠图去底（对齐小图标 matte）；
    // 大帧（如生物大图）保持原样。
    let webp = if w <= 64 && h <= 64 {
        let mut rgba = bgra_to_rgba(w, h, &bgra);
        matte_rgba(&mut rgba);
        encode_rgba_auto(rgba.as_raw(), w, h)
    } else {
        encode_webp_from_bgra(&bgra, w, h)
    }?;
    // 落盘走单一入口（与上面的存在性判定同口径；`_shared` 已收编进 core，
    // 迁移时仅需拷贝 `core*` 目录）。各 mod 的 statIcons/sharedIcons 均引用此路径。
    let rel = save_wiki_image(cache_root, "core", sub, &fname, &webp, "webp")?;
    // 共享 UI 图标的产物**归属父 `core`**（可能由任意 mod 的构建执行），登记也必须归到 core
    if let Some(hd) = crate::vcmi::hd_edition::probe_def_frame(&bytes, group, frame) {
        crate::modcache::hd_map::record("core", sub, &fname, &hd);
    }
    Some(rel)
}

/// 与 [`extract_game_def_frame_and_save`] 同逻辑（从游戏目录 lod 解析 DEF 取指定帧转 webp），
/// 但允许调用方指定 `sub`，且产物同样**归位 `core`**。
///
/// 用途：原版生物/英雄资源（CPRSMALL/TWCRPORT/UN32/UN44 等游戏级 DEF）。
/// 历史：早期按 `core.<faction>` 分目录存放（使原版城镇自包含）；改为「产物归属素材来源方」后，
/// 来源恒为游戏根 → 统一归 `core`，避免同一帧在 9 个城镇模组里各存一份。
pub(crate) fn extract_game_def_frame_and_save_mod(
    game_dir: &Path,
    cache_root: &Path,
    _mod_id: &str,
    def_name: &str,
    group: usize,
    frame: usize,
    sub: &str,
    _name: &str,
) -> Option<String> {
    if !assets_enabled() {
        return None;
    }
    //
    // 本函数只读**游戏根**资源（见 `read_game_resource_fallback`），来源恒为游戏级
    // → 产物归属恒为 `core`，因此短路可以**早于**读字节（连 LOD 读取一并省掉）。
    let owner = "core";
    let target = if def_name.to_uppercase().ends_with(".DEF") {
        def_name.trim().to_string()
    } else {
        format!("{}.DEF", def_name.trim())
    };
    // 产物名 = 资源标识 + 帧：原版同一帧被多个条目（不同英雄/生物）引用时只落一份
    let frame_tag = format!("g{group}f{frame}");
    let fname = asset_name(&target, Some(&frame_tag));
    note_asset_declared(owner, sub, &fname, &target);
    if let Some(rel) = wiki_image_exists(cache_root, owner, sub, &fname) {
        return Some(rel);
    }
    let bytes = crate::vcmi::resource::read_game_resource_fallback(game_dir, &target)?;
    // 只解目标帧
    let def = crate::infra::def::DefReader::new(&bytes);
    let (w, h, bgra, _lm, _tm) = def.decode_frame_at(group, frame)?;
    let webp = encode_webp_from_bgra(&bgra, w, h)?;
    let rel = save_wiki_image(cache_root, owner, sub, &fname, &webp, "webp")?;
    // 生物小/大图、英雄特长图标（CPRSMALL / TWCRPORT / UN32 / UN44）：登记 HD 源
    if let Some(hd) = crate::vcmi::hd_edition::probe_def_frame(&bytes, group, frame) {
        crate::modcache::hd_map::record(owner, sub, &fname, &hd);
    }
    Some(rel)
}

/// 原版英雄头像：按 imageIndex 抽取单帧，归位到各模组目录
/// （`core.<faction>/images/hero/<hero_id>.webp`；中性归入 `core/images/hero/`）。
///
/// 兼容两种资源形态：老版 `PortraitsLarge.DEF`（group0 顺序帧），
/// 与新版 animation json（`Mods/vcmi/Content/Sprites/PortraitsLarge.json` + `H3bitmap.lod` 中的位图）。
/// 逻辑移植自 `global.rs` 的 PortraitsLarge 提取，但产物改为按模组落盘。
pub(crate) fn extract_portrait_frame_mod(
    game_dir: &Path,
    cache_root: &Path,
    _mod_id: &str,
    _hero_id: &str,
    image_index: i64,
) -> Option<String> {
    if !assets_enabled() || image_index < 0 {
        return None;
    }
    // 两个分支都只读**游戏根**资源 → 归属恒为 `core`，短路可**早于**读字节
    let owner = "core";
    let idx = image_index as usize;
    // 产物名 = 资源标识 + 帧（头像索引），与"老 DEF / 新版 json+bitmap"两种格式无关：
    // 同一 `imageIndex` 的英雄头像只产出一份，跨 mod 也共用（此前按英雄 id 命名 → 各存一份）
    let frame_tag = format!("g0f{idx}");
    let fname = asset_name("PortraitsLarge.DEF", Some(&frame_tag));
    note_asset_declared(owner, "hero", &fname, "PortraitsLarge.DEF");
    if let Some(rel) = wiki_image_exists(cache_root, owner, "hero", &fname) {
        return Some(rel);
    }
    // a) 老版本 PortraitsLarge.DEF（group0 顺序帧）
    if let Some(bytes) =
        crate::vcmi::resource::read_game_resource_fallback(game_dir, "PortraitsLarge.DEF")
    {
        // 只解目标帧：此前每个英雄都解码**整个** PortraitsLarge（150+ 帧）再挑 1 帧
        let def = crate::infra::def::DefReader::new(&bytes);
        if let Some((w, h, bgra, _lm, _tm)) = def.decode_frame_at(0, idx) {
            if let Some(webp) = encode_webp_from_bgra(&bgra, w, h) {
                let rel = save_wiki_image(cache_root, owner, "hero", &fname, &webp, "webp")?;
                // 头像有官方 HD 版（`HPL*`），按 DEF 帧名登记
                if let Some(name) = def.frame_name(0, idx) {
                    if let Some(hd) = crate::vcmi::hd_edition::probe(&name) {
                        crate::modcache::hd_map::record(owner, "hero", &fname, &hd);
                    }
                }
                return Some(rel);
            }
        }
    }
    // b) VCMI 现版本：animation json + H3bitmap.lod 中的位图帧
    let json_path = game_dir
        .join("Mods")
        .join("vcmi")
        .join("Content")
        .join("Sprites")
        .join("PortraitsLarge.json");
    if let Ok(txt) = fs::read_to_string(&json_path) {
        if let Ok(v) = serde_json::from_str::<Value>(&txt) {
            if let Some(images) = v.get("images").and_then(|x| x.as_array()) {
                let file = images
                    .iter()
                    .find(|img| img.get("frame").and_then(|x| x.as_u64()) == Some(idx as u64))
                    .and_then(|img| img.get("file").and_then(|x| x.as_str()))
                    .unwrap_or("");
                if !file.is_empty() {
                    // lod 内条目名扩展名常与 json 声明不一致（json 写 .bmp，lod 实为 .pcx），
                    // 故依次尝试：原文名 / 无扩展名 / 无扩展名+常见扩展名
                    let stem = file.rsplit_once('.').map(|(a, _)| a).unwrap_or(file);
                    let tries: Vec<(String, &str)> = vec![
                        (file.to_string(), file.rsplit('.').next().unwrap_or("bmp")),
                        (stem.to_string(), "bmp"),
                        (format!("{stem}.pcx"), "pcx"),
                        (format!("{stem}.bmp"), "bmp"),
                        (format!("{stem}.png"), "png"),
                    ];
                    for (cand, ext) in tries {
                        if let Some(b) =
                            crate::vcmi::resource::read_game_resource_fallback(game_dir, &cand)
                        {
                            if let Some(webp) = encode_webp(&b, ext) {
                                let rel = save_wiki_image(
                                    cache_root, owner, "hero", &fname, &webp, "webp",
                                )?;
                                // json 的 `file`（如 `HPL000KN.bmp`）就是 HD pak 的键（`HPL*`）→ 登记
                                if let Some(hd) = crate::vcmi::hd_edition::probe(file) {
                                    crate::modcache::hd_map::record(owner, "hero", &fname, &hd);
                                }
                                return Some(rel);
                            }
                        }
                    }
                }
            }
        }
    }
    None
}

/// 与 [`resolve_specialty_icon`] 同语义，但产物按 `mod_id` 落到各模组目录
/// （`core.<faction>/images/herospec/...`；中性归入 `core/images/herospec/`），
/// 不再写入全局 `_shared/herospec/`。
pub(crate) fn resolve_specialty_icon_mod(
    p: &Path,
    game_dir: &Path,
    cache_root: &Path,
    mod_id: &str,
    hero_id: &str,
    raw: &str,
    image_index: i64,
    small: bool,
) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let sub = "herospec";
    let name = format!("{}-{}", hero_id, if small { "s" } else { "l" });
    if raw.eq_ignore_ascii_case("default") {
        if image_index < 0 {
            return None;
        }
        let def_name = if small { "UN32" } else { "UN44" };
        return extract_game_def_frame_and_save_mod(
            game_dir,
            cache_root,
            mod_id,
            def_name,
            0,
            image_index as usize,
            sub,
            &name,
        );
    }
    if let Some((buf, ext)) = resolve_resource(p, raw) {
        // mod 自有特长图（来源 = 自己）：按资源标识命名，
        // 同一张图被多个英雄引用时只存一份
        let is_def = ext.eq_ignore_ascii_case("def");
        let fname = asset_name(raw, is_def.then_some("g0f0"));
        note_asset_declared(mod_id, sub, &fname, raw);
        if is_def {
            // 只要 group0 首帧：不再解码整个动画
            let def = crate::infra::def::DefReader::new(&buf);
            if let Some((w, h, bgra, _lm, _tm)) = def.decode_frame_at(0, 0) {
                if let Some(webp) = encode_webp_from_bgra(&bgra, w, h) {
                    return save_wiki_image(cache_root, mod_id, sub, &fname, &webp, "webp");
                }
            }
            return None;
        }
        return save_wiki_image(cache_root, mod_id, sub, &fname, &buf, &ext);
    }
    extract_game_def_frame_and_save_mod(game_dir, cache_root, mod_id, raw, 0, 0, sub, &name)
}

/// 精确读取 .json 资源文件（避免 resolve_resource 尾匹配到同名 .def/.png）。
pub(crate) fn read_exact_sprite_json(p: &Path, res: &str) -> Option<Value> {
    let norm = res.trim_start_matches('/').replace('\\', "/");
    let lower = norm.to_lowercase();
    if !lower.ends_with(".json") {
        return None;
    }
    for root in ["Content/Sprites", "Content/Data", "Content"] {
        let cand = p.join(root).join(&norm);
        if cand.is_file() {
            if let Ok(text) = fs::read_to_string(&cand) {
                // HuaXia 等 mod 的 json 含注释/trailing comma，用容错解析器
                let v = parse_vcmi_json(&text);
                if !v.is_null() {
                    return Some(v);
                }
            }
        }
    }
    None
}

/// 读取 sprites-json 定义（VCMI 1.6+）：支持**无扩展名**引用与 **content.zip 内**定义。
///
/// 与 [`read_exact_sprite_json`] 的差别：后者只读磁盘散文件且要求 `.json` 后缀。
/// 实测多个 mod 把 `clientInfo.buildingsIcons` 写成无扩展名（`xanadu/town-hall`、
/// `hota/cove/town/townhall`）或把定义放在 zip 内（cove / bulwark / christmas）——
/// 只按"配置字符串后缀"判定会误入 DEF 解析（把 json 字节按 DEF 拆 → 建筑图标整批为空）。
/// 本函数统一走 [`resolve_resource`]（key 已做去根前缀/去扩展名/小写归一，覆盖磁盘与 zip）。
pub(crate) fn read_sprite_json(p: &Path, res: &str) -> Option<Value> {
    let res = res.trim().trim_start_matches('/').replace('\\', "/");
    if res.is_empty() {
        return None;
    }
    // 候选引用：原样 + 无扩展名时补 `.json`
    let mut exact = vec![res.clone()];
    if !res.to_lowercase().ends_with(".json") {
        exact.push(format!("{res}.json"));
    }
    // 1) 精确磁盘读取（首选：避免 resolve 的尾匹配命中同名 .def）
    for t in &exact {
        if let Some(v) = read_exact_sprite_json(p, t) {
            return Some(v);
        }
    }
    // 2) 磁盘/zip 统一解析（zip 内定义，或引用与实际文件名大小写/扩展名不一致）
    //
    // ⚠️ 必须用**容错**解析器 `parse_vcmi_json`（与 step1 一致）：VCMI 社区 / 第三方 mod 的
    // sprites-json 普遍带 `//` 注释与尾逗号（实测 new-pavilion 的 `26-grail ani.json`、
    // `21-special-2.json` 等）。step1 的磁盘精确读取走容错解析，但**仅覆盖 `Content/Sprites` 等
    // 目录**；zip 内定义（pavilion 是 content.zip，精灵在根 `Sprites/`）只能经此步 `resolve_resource`
    // 取出——若此处用严格 `serde_json::from_slice`，注释会让解析直接失败 → `read_sprite_json`
    // 返回 None → 这些建筑被误判为 DEF、再因后缀不符解码失败 → 幕布图丢失整组建筑。
    for t in &exact {
        if let Some((buf, ext)) = resolve_resource(p, t) {
            if !ext.eq_ignore_ascii_case("json") {
                continue;
            }
            let text = String::from_utf8_lossy(&buf);
            let v = parse_vcmi_json(&text);
            if !v.is_null() {
                return Some(v);
            }
        }
    }
    None
}

/// sprites-json 取帧（两种形态，均以 VCMI 实际写法为准）：
/// - `images: [{group?, frame, file}]`（xanadu / cove / bulwark；`group` 缺省视为匹配）；
/// - `sequences: [{group, frames: [file...]}]`（shuHan；按 group 取 `frames[frame]`）。
///
/// 返回 `(basepath, file)`；`basepath` 缺省为 `None`（由调用方按"定义文件所在目录"等回退）。
pub(crate) fn sprite_json_frame(
    v: &Value,
    frame: usize,
    group: usize,
) -> Option<(Option<String>, String)> {
    let base = v
        .get("basepath")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());
    if let Some(images) = v.get("images").and_then(|x| x.as_array()) {
        let g_of = |x: &Value| -> Option<usize> {
            x.get("group")
                .and_then(|g| g.as_i64())
                .map(|g| g.max(0) as usize)
        };
        let f = images
            .iter()
            .find(|x| {
                x.get("frame").and_then(|f| f.as_i64()) == Some(frame as i64)
                    && g_of(x).map(|g| g == group).unwrap_or(true)
            })
            .or_else(|| {
                images
                    .iter()
                    .find(|x| g_of(x).map(|g| g == group).unwrap_or(true))
            })
            .or_else(|| images.first())?;
        let file = f.get("file").and_then(|x| x.as_str())?.to_string();
        return Some((base, file));
    }
    if let Some(seqs) = v.get("sequences").and_then(|x| x.as_array()) {
        let seq = seqs
            .iter()
            .find(|s| {
                s.get("group")
                    .and_then(|g| g.as_i64())
                    .map(|g| g.max(0) as usize == group)
                    .unwrap_or(true)
            })
            .or_else(|| seqs.first())?;
        let file = seq
            .get("frames")
            .and_then(|x| x.as_array())?
            .get(frame)
            .and_then(|x| x.as_str())?
            .to_string();
        return Some((base, file));
    }
    None
}

/// 组合 sprites-json 的 `basepath + file` 为候选资源引用（按序尝试）：
/// - `file` 无扩展名 → 补 `.png`（历史行为）；
/// - `file` 自带相对路径与扩展名（christmas `christmas/Hall/Mage1.bmp`）→ 原样再试；
/// - 统一附 `Sprites/` 前缀变体（`resolve_resource` 会去常见根前缀，两侧都试更稳）。
pub(crate) fn sprite_ref_candidates(base: Option<&str>, file: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let file_has_ext = Path::new(file).extension().is_some();
    if let Some(b) = base.filter(|b| !b.is_empty()) {
        let b = if b.ends_with('/') {
            b.to_string()
        } else {
            format!("{b}/")
        };
        let joined = format!("{b}{file}");
        out.push(if file_has_ext {
            joined
        } else {
            format!("{joined}.png")
        });
    }
    if !file.is_empty() {
        out.push(file.to_string());
    }
    let mut full: Vec<String> = Vec::new();
    for c in out {
        if !c.to_lowercase().starts_with("sprites/") {
            full.push(format!("Sprites/{c}"));
        }
        full.push(c);
    }
    full.dedup();
    full
}

/// 按**资源真实类型**提取精灵帧并落盘（建筑图标 / 结构帧），替代"看配置字符串后缀"的旧口径：
/// 1) sprites-json 定义（含无扩展名引用、zip 内定义）→ 取帧 → 位图（png/bmp/pcx…）直接保存；
/// 2) 非 json → DEF 单帧解码（保持 [`extract_def_frame_and_save`] 语义）；裸位图（无 json 定义）直接保存；
/// 3) json 定义存在但帧资源缺失 → 返回 `None`（**不**再把 json 字节按 DEF 解析）。
pub(crate) fn extract_sprite_frame_and_save(
    p: &Path,
    game_dir: &Path,
    cache_root: &Path,
    mod_id: &str,
    sub: &str,
    name: &str,
    res_ref: &str,
    frame: usize,
) -> Option<String> {
    let res_ref = res_ref.trim();
    if res_ref.is_empty() || !assets_enabled() {
        return None;
    }
    if let Some(rel) = wiki_image_exists(cache_root, mod_id, sub, name) {
        return Some(rel);
    }
    // 1) sprites-json：定义 → 帧引用 → 位图
    if let Some(v) = read_sprite_json(p, res_ref) {
        if let Some((base, file)) = sprite_json_frame(&v, frame, 0) {
            for cand in sprite_ref_candidates(base.as_deref(), &file) {
                if let Some((buf, ext, origin)) = resolve_resource_origin(p, game_dir, &cand) {
                    let e = ext.to_lowercase();
                    if e == "json" || e == "def" {
                        continue; // 帧引用指向定义文件属异常写法，跳过
                    }
                    let rel = save_wiki_image(cache_root, mod_id, sub, name, &buf, &ext)?;
                    note_hd_bitmap(mod_id, sub, name, &cand, Some(&origin));
                    return Some(rel);
                }
            }
        }
        // 定义存在但取不到位图：直接失败（旧口径在此会误按 DEF 解析 → 静默无图）
        return None;
    }
    // 2) 非 json：裸位图直接保存；其余（DEF 等）委托单帧解码
    if let Some((buf, ext, origin)) = resolve_resource_origin(p, game_dir, res_ref) {
        let e = ext.to_lowercase();
        if e == "json" {
            return None;
        }
        if matches!(e.as_str(), "png" | "bmp" | "pcx" | "webp") {
            let rel = save_wiki_image(cache_root, mod_id, sub, name, &buf, &ext)?;
            note_hd_bitmap(mod_id, sub, name, res_ref, Some(&origin));
            return Some(rel);
        }
    }
    extract_def_frame_and_save(
        p, game_dir, cache_root, mod_id, sub, name, res_ref, 0, frame,
    )
}

/// 城镇结构「金色描边」图（`town.structures[id].border`）提取并落盘，返回 `(相对缓存路径, 宽, 高)`。
///
/// 用途：城镇界面 hover 时**罩住整栋建筑**的金边——这正是原版游戏的 hover 效果。
/// 对齐 VCMI `client/windows/CCastleInterface.cpp:325-348`（`CBuildingRect::show`）：
/// 描边图绘制在结构的**原始 `x/y`**（不像建筑精灵那样再加 DEF margin），
/// `EImageBlitMode::COLORKEY` 透明，静止态按**原色**绘制（原图描边本身即金褐色，
/// 无需再染色）。因此前端只需把本图按 `(x, y)` + 原始 `(w, h)` 叠加到同尺寸幕布
/// （原版 800×374）即可像素级贴合，既不依赖任何猜测的包围盒，也不怕不规则轮廓。
///
/// 兼容三类来源：
/// - 原版：H3 专有 PCX（在 `H3bitmap.lod` 内，如 `TOCSPIK1.PCX`；洋红 `(0,255,255)` 透明）；
/// - 第三方散装 8bpp BMP（`.../town-screen/buildings-border/*.bmp`；VCMI 色键 = 青色）；
/// - 新式模组：sprites-json 定义（无扩展名引用 / zip 内定义）。
///
/// 宽高必须一并产出（前端按百分比定位靠它），故先在内存里解码取尺寸再编码。
pub(crate) fn extract_border_and_save(
    p: &Path,
    game_dir: &Path,
    cache_root: &Path,
    mod_id: &str,
    faction_id: &str,
    structure_id: &str,
    res: &str,
) -> Option<(String, u32, u32)> {
    // 阶段一（只出数据）不产图
    if !assets_enabled() {
        return None;
    }
    let name = format!("{}-{}", faction_id, structure_id);
    let rel = wiki_image_rel(mod_id, "townborder", &name);
    let out = cache_root.join(&rel);
    let (buf, ext) = resolve_border_bytes(p, game_dir, res)?;
    let (w, h, rgba) = decode_border_rgba(&buf, &ext)?;
    if w == 0 || h == 0 {
        return None;
    }
    if !out.is_file() {
        let webp = encode_rgba_auto(&rgba, w, h)?;
        fs::create_dir_all(out.parent()?).ok()?;
        fs::write(&out, &webp).ok()?;
        crate::infra::metrics::count("img.saved", 1);
        crate::infra::metrics::bytes("img.bytes.disk", webp.len() as u64);
    } else {
        crate::infra::metrics::count("img.cached", 1);
    }
    // 登记官方 HD 源：原版城镇描边在配置里写作 `TOCSMAG1.bmp`，而 hd-edition 里
    // 正好有同名帧（`probe` 会去扩展名后直查）。实测 Castle 37 个结构**全部命中**，
    // 且 HD 帧 = 恰好 3× 的 1x 产物（`TOCSMAG1` x3 = 279×309 vs 89→93×103），
    // 宽高比一致 ⇒ 按百分比定位的高亮框不会错位，可安全接 HD。
    if hd_shape_matches(res, w, h) {
        note_hd_bitmap(mod_id, "townborder", &name, res, None);
    }
    Some((rel, w, h))
}

/// 解析描边图字节：sprites-json 定义 → 取帧引用位图；否则走 mod → 游戏根回退解析。
fn resolve_border_bytes(p: &Path, game_dir: &Path, res: &str) -> Option<(Vec<u8>, String)> {
    let res = res.trim();
    if res.is_empty() {
        return None;
    }
    // sprites-json 定义（按内容识别，兼容无扩展名 / zip 内定义）
    if let Some(v) = read_sprite_json(p, res) {
        if let Some((base, file)) = sprite_json_frame(&v, 0, 0) {
            for cand in sprite_ref_candidates(base.as_deref(), &file) {
                if let Some((buf, ext)) = resolve_resource_fallback(p, game_dir, &cand) {
                    if !ext.eq_ignore_ascii_case("json") {
                        return Some((buf, ext));
                    }
                }
            }
        }
        // 定义存在但取不到位图：不再把 json 字节按位图解析
        return None;
    }
    resolve_resource_fallback(p, game_dir, res)
}

/// 描边图解码为 RGBA，并按 VCMI 规则处理透明：
/// - H3 专有 PCX（8bpp）：洋红 `(0,255,255)` 透明（[`crate::infra::image::decode_h3_pcx`]）；
/// - 标准 PCX：调色板 0 号色透明；
/// - 8bpp BMP：VCMI `DEFAULT_KEY_COLOR=(0,255,255)` 与 H3 洋红 `(255,0,255)` 均透明；
/// - png / webp 等：保留自带 alpha（真实 alpha 优先）。
///
/// 无论哪种来源，最后统一再过一遍**色键兜底**：对任何*不透明*像素，命中 `(0,255,255)` /
/// `(255,0,255)` 即透明。这一步专门兜住"解码器本该抠却漏掉"的情况——如 24bpp H3 PCX、
/// 散装 BMP、png 等把色键色留在不透明区域的素材（HoTA 工厂阵营建筑高亮图的青色底即此），
/// 无需逐格式打补丁；已带真实 alpha 的像素不受影响。
fn decode_border_rgba(buf: &[u8], ext: &str) -> Option<(u32, u32, Vec<u8>)> {
    use crate::infra::image::{decode_h3_pcx, decode_pcx};
    let e = ext.to_lowercase();
    let (w, h, mut rgba) = if e == "pcx" {
        // H3 专有 PCX（含洋红 (0,255,255) 透明）优先；不行再退标准 PCX（索引 0 透明）
        if let Some(r) = decode_h3_pcx(buf).or_else(|| decode_pcx(buf)) {
            r
        } else {
            return None;
        }
    } else {
        let img = image::load_from_memory(buf).ok()?.to_rgba8();
        (img.width(), img.height(), img.into_raw())
    };
    // 统一色键兜底：只对不透明像素生效，命中 VCMI 约定的两种透明色即透明
    // （与 `CSDL_Ext::setDefaultColorKeyPresize` / VCMI `setColorKey` 行为一致）。
    for px in rgba.chunks_exact_mut(4) {
        if px[3] == 255
            && ((px[0] == 0 && px[1] == 255 && px[2] == 255)
                || (px[0] == 255 && px[1] == 0 && px[2] == 255))
        {
            px[3] = 0;
        }
    }
    Some((w, h, rgba))
}

#[cfg(test)]
mod tests {
    use super::{is_core_scope, owner_of, Origin};

    /// 回归（2026-10-20）：`core.<faction>` 伪模组的 `SelfMod` 素材必须归 `core`。
    ///
    /// 它们的 `mod_path` 就是 VCMI 基础模组（`Mods/vcmi`），"本模组命中"实际是核心自带素材；
    /// 按 `mod_id` 归位时 9 个城镇 + 父 core 会各写一份（实测同一张 `battle/actionGenie.png`
    /// 在 10 个目录里各一份）。
    #[test]
    fn core_pseudo_mod_assets_belong_to_core() {
        assert_eq!(owner_of(&Origin::SelfMod, "core.castle"), "core");
        assert_eq!(owner_of(&Origin::SelfMod, "core"), "core");
        // 第三方模组不受影响：自有素材仍落在自己目录，保持自包含
        assert_eq!(owner_of(&Origin::SelfMod, "hota"), "hota");
        assert_eq!(owner_of(&Origin::SelfMod, "farraway.Town"), "farraway.Town");
        // 游戏级 / 跨模组引用口径不变
        assert_eq!(owner_of(&Origin::Game, "core.dungeon"), "core");
        assert_eq!(
            owner_of(&Origin::Mod("hota".into()), "core.castle"),
            "hota"
        );
        assert!(is_core_scope("core.rampart"));
        assert!(!is_core_scope("coverage"));
    }
}
