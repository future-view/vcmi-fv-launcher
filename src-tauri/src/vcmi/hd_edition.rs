//! HD 高清素材来源：官方「Heroes III HD Edition」素材 mod（`hd-edition`）的探测、注入与按名取图。
//!
//! ## 背景
//!
//! VCMI 官方的该图形 mod 把育碧 HD 版的 `*.pak` **原样打包**（由官方 Launcher 的 `hdextractor`
//! 从用户自备的 HD 版安装拷贝，不转换），由引擎按**原版资源名**索引 —— 容器级解析见 `infra::pak`，
//! 格式与命名实测见项目记忆 `reference_hd_edition_pak.md`。本模块负责三件事：
//!
//! 1. **探测**：在用户模组目录 / 游戏模组目录下找 `hd-edition`。判据是 **pak 文件存在**，
//!    不硬编码 mod id 或目录名大小写（`content` 与 `Content` 两种写法都有实测先例）；
//! 2. **注入**：构建批次开始时 `set_source(...)` 一次（进程级），供提取链路按需取图；
//! 3. **取图**：`lookup`（位图/PCX 类，同名直查）与 `lookup_def_frame`（DEF 类，用 13 字节
//!    帧名换算，见 `infra::def::DefReader::frame_name`）。
//!
//! ## 与 VCMI 的口径对齐（易错点，改前先读）
//!
//! - pak 内是 **x2 / x3** 两档**独立图集**，帧矩形是**该档像素**；
//!   ⚠️ 帧尺寸**不严格**等于 1x × 档位（按 DXT 4×4 块对齐，实测 `CPRS000C` = 62×64 而 1x 为 30×32，
//!   严格 2× 应为 60×64）—— 因此 HD 帧与 1x 图**不能按尺寸互相推导**，也不要用尺寸做断言；
//! - `sprite_offset` 是 VCMI `ImageEntry::spriteOffsetX/Y`，**数值随档位写入**（实测船坞帧
//!   x2 = `(0,1)` / x3 = `(0,2)`）→ 只能与**同一档**的图配对，别跨档比较、别当 1x 常量；
//! - **按坐标叠加**时必须用 [`HdEditionSource::lookup_with_offset`]，绘制偏移 =
//!   `DEF 帧的 lm/tm − sprite_offset`（VCMI `HdImageLoader::getImage` 的
//!   `setMargins((margins - spriteOffset) * scalingFactor)`）—— 帧名命中 HD 不代表能沿用结构 x/y；
//! - 帧表里的 `rotation=1` 表示该帧需**顺时针旋转 90°**（复刻 VCMI `CSDL_Ext::Rotate90`：
//!   `dst[h-1-y][x] = src[x][y]`，实测资源/士气图标大量用到，不旋转会歪）；
//! - 默认取**可用最高档**（x3 → x2）：小图在 4K 下需求可达 6x，真高清起点越高越好；
//! - 未命中（HD 版未收录元素城、`SPELLBON`、地形块等）返回 `None`，调用方回落原版素材 ——
//!   本模块**永不**让缺失变成错误，只计埋点。
//!
//! 依赖方向：`vcmi` 层可依赖 `infra`；**不得**出现 `tauri` 类型。

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, RwLock};

use crate::infra::def::DefReader;
use crate::infra::pak::{PakArchive, PakFrame};

/// sheet 解码缓存上限。
///
/// 构建期同一 entry 的帧是**连续查询**（如 `CPRSMALL` 的 124 帧共用同一张图集），
/// 因此按"整张 sheet 的解码结果"缓存命中率极高。128 MB ≈ 8~16 张 1024²/2048² 图集，
/// 足够覆盖单次提取的工作集（同档位下几十个 entry，逐个进逐个出）。
const SHEET_CACHE_MAX_BYTES: usize = 128 * 1024 * 1024;

/// HD 版收录的最高档位（VCMI 只为 x2/x3 提供 pak；x4 及以上由引擎实时放大）。
const MAX_HD_SCALE: u32 = 3;

/// 一档（x2 / x3）的 pak 集合。
struct ScaleSource {
    scale: u32,
    /// 该档的全部 pak（`sprite` / `bitmap`），查询按顺序命中即返回
    paks: Vec<PakArchive>,
}

/// sheet 解码缓存（FIFO 淘汰；构建期局部性强，无需真 LRU）。
#[derive(Default)]
struct SheetCache {
    map: HashMap<(u32, String, u32), Arc<image::RgbaImage>>,
    order: VecDeque<(u32, String, u32)>,
    bytes: usize,
}

/// HD 素材来源（只读；跨线程共享）。
pub struct HdEditionSource {
    dir: PathBuf,
    /// 可用档位，**按 scale 降序**（x3 优先）
    scales: Vec<ScaleSource>,
    sheets: Mutex<SheetCache>,
}

impl HdEditionSource {
    /// 来源根目录（`hd-edition` 本体）
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// 可用档位（降序，如 `[3, 2]`）
    pub fn scales(&self) -> Vec<u32> {
        self.scales.iter().map(|s| s.scale).collect()
    }

    /// 释放 sheet 解码缓存（≤128 MB 的已解码图集；构建结束时调用）。
    ///
    /// 与 `infra::ai::clear_sessions` 对称：sheet 缓存是高清 / 素材阶段涨上去、完成后
    /// 不该继续占用的部分（否则内存停在高水位）。**只丢解码结果**，pak 索引 / 帧表
    /// 仍保留，下批按需重新解码，不影响正确性（缓存本就是纯性能优化）。
    pub fn trim_sheets(&self) {
        if let Ok(mut guard) = self.sheets.lock() {
            guard.map.clear();
            guard.order.clear();
            guard.bytes = 0;
        }
    }

    /// 来源指纹：目录 + 各档 pak 的 `(文件名, 大小, mtime)`。
    ///
    /// 用途：用户装 / 卸 / 升级 `hd-edition` 或切换档位后，**素材阶段必须重跑一次**
    /// 才能补上登记表（`hd-map.json` 是提取时写的，图已存在会走短路而不登记）。
    fn fingerprint(&self) -> String {
        let mut s = format!("{}|", self.dir.display());
        for sc in &self.scales {
            s.push_str(&format!("x{}:", sc.scale));
            for pak in &sc.paks {
                let md = std::fs::metadata(pak.path());
                let (len, mtime) = match md {
                    Ok(m) => (
                        m.len(),
                        m.modified()
                            .ok()
                            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                            .map(|d| d.as_secs())
                            .unwrap_or(0),
                    ),
                    Err(_) => (0, 0),
                };
                let name = pak
                    .path()
                    .file_name()
                    .map(|f| f.to_string_lossy().to_string())
                    .unwrap_or_default();
                s.push_str(&format!("{name}:{len},{mtime};"));
            }
        }
        s
    }

    /// 全量帧名（**只读诊断**）：`(档位, 条目名, 帧名)`，供覆盖度审计比对
    /// "HD 收录了什么" vs "百科实际登记/使用了什么"。
    #[cfg(test)]
    pub fn dump_frames(&self) -> Vec<(u32, String, String)> {
        let mut out = Vec::new();
        for sc in &self.scales {
            for pak in &sc.paks {
                for (entry, frames) in pak.entries_with_frames() {
                    for f in frames {
                        out.push((sc.scale, entry.clone(), f));
                    }
                }
            }
        }
        out
    }

    /// 按资源名取图（直查：位图/PCX 类用同名，DEF 类用帧名换算后的名字）。
    ///
    /// 名字经 [`candidate_keys`] 归一（**去路径 / 去扩展名**），因此调用方可直接传配置里的
    /// 原始引用（`TBCSBACK.bmp`、`Sprites/HPL000KN.pcx`）。
    ///
    /// 返回 `(图, 该图的 HD 档位)`；未命中返回 `None`（调用方回落原版素材）。
    ///
    /// ⚠️ 需要**按位置**绘制时不要用本函数，用 [`Self::lookup_with_offset`] ——
    /// 位置不能只看 DEF 的 lm/tm，还要减去 pak 帧声明的 `spriteOffset`（见该函数）。
    pub fn lookup(&self, name: &str) -> Option<(image::RgbaImage, u32)> {
        self.lookup_with_offset(name)
            .map(|(img, scale, _)| (img, scale))
    }

    /// 同 [`Self::lookup`]，但额外给出该帧声明的 **1x 逻辑绘制偏移**（pak 帧表的
    /// `spriteOffsetX/Y`）。
    ///
    /// 用途：**按坐标叠加**的调用方（城镇幕布合成）必须复刻 VCMI
    /// `HdImageLoader::getImage` 的定位公式
    /// `setMargins((defMargins - spriteOffset) * scalingFactor)` ——
    /// 即绘制偏移 = **DEF 帧的 lm/tm 减去本函数的返回值**。
    /// 帧名命中 HD **不代表**位置可以沿用结构自身的 `x/y`：DEF 帧自带 margin
    /// （实测城镇船坞 `TBCSDKNN` 的 tm=53，忽略就会把船坞画到战神像身上）。
    /// 单图用途（图标 / 头像 / 变体放大）与位置无关，用 [`Self::lookup`] 即可。
    pub fn lookup_with_offset(&self, name: &str) -> Option<(image::RgbaImage, u32, (i32, i32))> {
        let keys = candidate_keys(name);
        if keys.is_empty() {
            return None;
        }
        for s in &self.scales {
            for pak in &s.paks {
                for key in &keys {
                    if let Some(fr) = pak.find(key) {
                        match self.frame_image(s.scale, &fr) {
                            Some(img) => {
                                crate::infra::metrics::count("hd_src.hit", 1);
                                return Some((img, s.scale, fr.sprite_offset));
                            }
                            // 命中但解码失败：继续试其它 pak/档位（宁可要低档，不要回落原版）
                            None => crate::infra::metrics::count("hd_src.fail", 1),
                        }
                    }
                }
            }
        }
        crate::infra::metrics::count("hd_src.miss", 1);
        None
    }

    /// 按**需求倍率** `need`（相对 1x 基准图）选最优源档取图。
    ///
    /// 选源规则分两段（2026-10-20 修订，起因：用户实测 `@4x` 明显糊）：
    ///
    /// - `need <= 最高档`（常见 2x / 3x）→ 沿用 2026-09-23 拍板：对每个可用档 `h` 取
    ///   `k = ceil(need / h)`，选 **`h*k` 最小**者（体积最小），同分取 **`k` 最小**者。
    ///   `need=2` → (h=2,k=1) 官方原图直出，不跨档去解更大的图集。
    /// - `need > 最高档`（4x / 5x…）→ **一律取最高档 `h`**，由调用方重采样到精确需求。
    ///   旧规则此时选 x2 再放大 2×（`h*k` 恰好 = 4），但 x2 原图细节本就不足，
    ///   2× 插值后比"x3 重采样到 4x"明显更糊（同一 x3 素材多 1.5× 线性细节）。
    ///
    /// 返回 `(图, 源档 h, 建议整数放大倍数 k = ceil(need/h))`。调用方决定：
    /// `need % h == 0` 用整数放大 `k`；否则 `h > need` 走下采样、`h < need` 走
    /// `hd::resample_hd_to_need` 的单步重采样（见 `modcache::hd::upscale_file`）。
    pub fn lookup_for(&self, name: &str, need: u32) -> Option<(image::RgbaImage, u32, u32)> {
        let keys = candidate_keys(name);
        if keys.is_empty() {
            return None;
        }
        let need = need.max(1);
        let max_scale = self.scales.iter().map(|s| s.scale).max().unwrap_or(1);
        let mut best: Option<(u32, u32, u32)> = None; // (h, k, h*k)
        for s in &self.scales {
            let k = need.div_ceil(s.scale).max(1);
            let cover = s.scale * k;
            let better = match best {
                None => true,
                // 大倍率需求：取最高档（细节最多），尺寸交给调用方重采样
                Some((bh, bk, bc)) => {
                    if need > max_scale {
                        s.scale > bh
                    } else {
                        cover < bc || (cover == bc && k < bk)
                    }
                }
            };
            if better {
                best = Some((s.scale, k, cover));
            }
        }
        let (h, k, _) = best?;
        let s = self.scales.iter().find(|s| s.scale == h)?;
        for pak in &s.paks {
            for key in &keys {
                if let Some(fr) = pak.find(key) {
                    if let Some(img) = self.frame_image(h, &fr) {
                        crate::infra::metrics::count("hd_src.hit", 1);
                        return Some((img, h, k));
                    }
                }
            }
        }
        crate::infra::metrics::count("hd_src.miss", 1);
        None
    }

    /// 取该帧在**指定档位**的**阴影图**（VCMI `HdImageLoader::getImage(shadow=true)`）。
    ///
    /// HD 版把生物战斗帧的阴影存成**独立切片**（`PakFrame::shadow`），本体图里不再含阴影
    /// —— 不画就会"悬空"。绘制位置与本体**同一套公式**（`(lm/tm − spriteOffset) × 档位`，
    /// VCMI `HdImageLoader.cpp:176` 对两者一视同仁），故本函数只返回图，不返回偏移：
    /// 调用方先画阴影、再画本体即可。
    ///
    /// ⚠️ 必须传**本体实际用的档位**：阴影切片与本体是同一动画的两半，跨档尺寸口径不一致。
    pub fn lookup_shadow(&self, name: &str, scale: u32) -> Option<image::RgbaImage> {
        let keys = candidate_keys(name);
        if keys.is_empty() {
            return None;
        }
        let s = self.scales.iter().find(|x| x.scale == scale)?;
        for pak in &s.paks {
            for key in &keys {
                if let Some(fr) = pak.find(key) {
                    if let Some((sheet, rect)) = fr.shadow {
                        return self.frame_rect(scale, &fr.entry, sheet, rect, fr.shadow_rot);
                    }
                    // 该帧没有阴影切片 → 不必再看同档其它 pak
                    break;
                }
            }
        }
        None
    }

    /// 按**目标倍数 `need`** 取图（含绘制偏移）：选**最小满足** `档位 ≥ need` 的档位，
    /// 都不满足则取最高档。
    ///
    /// 与 [`Self::lookup_with_offset`]（恒取最高档）的区别就是这一句：
    /// 动画帧体积大（整组十余帧），在 1080p 下恒取 x3 属于浪费，
    /// 而 `need` 由"显示框 CSS × 屏幕物理倍率 ÷ 窗口源像素"算出，够用即止。
    pub fn lookup_with_offset_for(
        &self,
        name: &str,
        need: u32,
    ) -> Option<(image::RgbaImage, u32, (i32, i32))> {
        let keys = candidate_keys(name);
        if keys.is_empty() {
            return None;
        }
        let mut asc: Vec<u32> = self.scales.iter().map(|s| s.scale).collect();
        asc.sort_unstable();
        let want = need.max(1);
        let chosen = asc
            .iter()
            .copied()
            .find(|s| *s >= want)
            .unwrap_or_else(|| *asc.last().unwrap_or(&1));
        let s = self.scales.iter().find(|x| x.scale == chosen)?;
        for pak in &s.paks {
            for key in &keys {
                if let Some(fr) = pak.find(key) {
                    if let Some(img) = self.frame_image(chosen, &fr) {
                        return Some((img, chosen, fr.sprite_offset));
                    }
                }
            }
        }
        None
    }

    /// 把某帧从图集裁出（含 `rotation=1` 的顺时针 90°，复刻 VCMI `CSDL_Ext::Rotate90`）。
    fn frame_image(&self, scale: u32, fr: &PakFrame) -> Option<image::RgbaImage> {
        self.frame_rect(scale, &fr.entry, fr.sheet, fr.rect, fr.rotation)
    }

    /// 按 `(entry, sheet, rect, rotation)` 裁帧：本体与**阴影**共用（两者各有独立的 rotation）。
    fn frame_rect(
        &self,
        scale: u32,
        entry: &str,
        sheet_idx: u32,
        rect: (u32, u32, u32, u32),
        rotation: u32,
    ) -> Option<image::RgbaImage> {
        let sheet = self.load_sheet(scale, entry, sheet_idx)?;
        let (x, y, w, h) = rect;
        if w == 0
            || h == 0
            || x.checked_add(w)? > sheet.width()
            || y.checked_add(h)? > sheet.height()
        {
            return None;
        }
        let cropped = image::imageops::crop_imm(sheet.as_ref(), x, y, w, h).to_image();
        Some(if rotation == 1 {
            rotate90_cw(&cropped)
        } else {
            cropped
        })
    }

    /// 取（并缓存）某档某 entry 某 sheet 的**整图集**（已解码 RGBA）。
    fn load_sheet(&self, scale: u32, entry: &str, sheet: u32) -> Option<Arc<image::RgbaImage>> {
        let key = (scale, entry.to_string(), sheet);
        if let Ok(guard) = self.sheets.lock() {
            if let Some(v) = guard.map.get(&key) {
                return Some(v.clone());
            }
        }
        let s = self.scales.iter().find(|s| s.scale == scale)?;
        for pak in &s.paks {
            let Some(bytes) = pak.read_sheet(entry, sheet) else {
                continue;
            };
            let Some((w, h, rgba)) = crate::infra::image::decode_dds(&bytes) else {
                continue;
            };
            let Some(img) = image::RgbaImage::from_raw(w, h, rgba) else {
                continue;
            };
            let img = Arc::new(img);
            let bytes_used = (w as usize) * (h as usize) * 4;
            if let Ok(mut guard) = self.sheets.lock() {
                // FIFO 淘汰：超限就从头丢，直到装得下（单张上限即整个缓存上限时直接不缓存）
                while guard.bytes + bytes_used > SHEET_CACHE_MAX_BYTES {
                    let Some(old) = guard.order.pop_front() else {
                        break;
                    };
                    if let Some(v) = guard.map.remove(&old) {
                        guard.bytes = guard
                            .bytes
                            .saturating_sub((v.width() as usize) * (v.height() as usize) * 4);
                    }
                }
                if bytes_used <= SHEET_CACHE_MAX_BYTES {
                    guard.bytes += bytes_used;
                    guard.order.push_back(key.clone());
                    guard.map.insert(key, img.clone());
                }
            }
            return Some(img);
        }
        None
    }
}

/// 顺时针旋转 90°（复刻 VCMI `CSDL_Ext::Rotate90`：`dst[h-1-y][x] = src[x][y]`）。
fn rotate90_cw(src: &image::RgbaImage) -> image::RgbaImage {
    let (w, h) = (src.width(), src.height());
    let mut out = image::RgbaImage::new(h, w);
    for y in 0..h {
        for x in 0..w {
            out.put_pixel(h - 1 - y, x, *src.get_pixel(x, y));
        }
    }
    out
}

// ── 探测 ────────────────────────────────────────────────────────────────────

/// 在候选根目录（用户模组目录、游戏模组目录…）下探测 HD 素材来源。
///
/// `max_scale`：`None` = 取可用最高档（x3 优先）；`Some(2)` = 只用 x2（省磁盘）。
///
/// 探测顺序：`<root>/hd-edition` → `<root>` 下任意一层子目录（兜底，兼容改名/手工安装）。
/// 判据始终是 **pak 文件是否存在**，不依赖 mod id。
pub fn detect(roots: &[PathBuf], max_scale: Option<u32>) -> Option<HdEditionSource> {
    for root in roots {
        let mut dirs: Vec<PathBuf> = Vec::new();
        if let Some(d) = find_child_ci(root, "hd-edition") {
            dirs.push(d);
        } else if let Ok(rd) = std::fs::read_dir(root) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    dirs.push(p);
                }
            }
        }
        for d in dirs {
            if let Some(src) = open_source(&d, max_scale) {
                return Some(src);
            }
        }
    }
    None
}

/// 打开某目录为 HD 来源（该目录内需能定位到 `*_DXT_com_x{2,3}.pak`）。
fn open_source(dir: &Path, max_scale: Option<u32>) -> Option<HdEditionSource> {
    let limit = max_scale.unwrap_or(MAX_HD_SCALE).clamp(2, MAX_HD_SCALE);
    let mut scales = Vec::new();
    for scale in (2..=limit).rev() {
        let mut paks = Vec::new();
        for kind in ["sprite", "bitmap"] {
            if let Some(a) = pak_in(dir, scale, kind) {
                paks.push(a);
            }
        }
        if !paks.is_empty() {
            scales.push(ScaleSource { scale, paks });
        }
    }
    if scales.is_empty() {
        return None;
    }
    Some(HdEditionSource {
        dir: dir.to_path_buf(),
        scales,
        sheets: Mutex::new(SheetCache::default()),
    })
}

/// 在候选位置找 `<kind>_DXT_com_x<scale>.pak`（覆盖 `mods/x2/...`、`x2/...`、根 `content/data` 三种形态）。
fn pak_in(dir: &Path, scale: u32, kind: &str) -> Option<PakArchive> {
    let file = format!("{kind}_DXT_com_x{scale}.pak");
    let sub = format!("x{scale}");
    let bases = [
        resolve_ci(dir, &["mods", &sub, "content", "data"]),
        resolve_ci(dir, &[&sub, "content", "data"]),
        resolve_ci(dir, &["content", "data"]),
    ];
    for base in bases.into_iter().flatten() {
        if let Some(f) = find_child_file_ci(&base, &file) {
            if let Some(a) = PakArchive::open(&f) {
                return Some(a);
            }
        }
    }
    None
}

/// 逐级大小写不敏感地解析路径（如 `["mods","x3","content","data"]`）。
fn resolve_ci(base: &Path, parts: &[&str]) -> Option<PathBuf> {
    let mut cur = base.to_path_buf();
    for p in parts {
        cur = find_child_ci(&cur, p)?;
    }
    Some(cur)
}

/// 大小写不敏感地找子目录
fn find_child_ci(parent: &Path, name: &str) -> Option<PathBuf> {
    let rd = std::fs::read_dir(parent).ok()?;
    for e in rd.flatten() {
        if e.file_name().to_string_lossy().eq_ignore_ascii_case(name) {
            let p = e.path();
            if p.is_dir() {
                return Some(p);
            }
        }
    }
    None
}

/// 大小写不敏感地找文件
fn find_child_file_ci(parent: &Path, name: &str) -> Option<PathBuf> {
    let rd = std::fs::read_dir(parent).ok()?;
    for e in rd.flatten() {
        if e.file_name().to_string_lossy().eq_ignore_ascii_case(name) {
            let p = e.path();
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

// ── 进程级注入 ──────────────────────────────────────────────────────────────

static SOURCE: OnceLock<RwLock<Option<Arc<HdEditionSource>>>> = OnceLock::new();

fn cell() -> &'static RwLock<Option<Arc<HdEditionSource>>> {
    SOURCE.get_or_init(|| RwLock::new(None))
}

/// 测试用全局互斥：涉及**进程级注入**（[`set_source`]）的测试必须串行 ——
/// `cargo test` 默认多线程，一个测试的 `set_source(None)` 会把另一个测试的来源清掉
/// （实测表现为端到端用例偶发"无登记"）。
#[cfg(test)]
pub(crate) fn test_guard() -> std::sync::MutexGuard<'static, ()> {
    static L: std::sync::Mutex<()> = std::sync::Mutex::new(());
    L.lock().unwrap_or_else(|e| e.into_inner())
}

/// 注入/清除 HD 来源（构建批次开始时调用一次；`None` = 本次构建不使用 HD 素材）。
///
/// 仿 `vcmi::resource::set_wiki_mod_paths` 的进程级注入：提取链路遍布几十处调用点，
/// 逐个传参会污染所有签名，且 HD 来源对单次构建是**全局唯一**的上下文。
pub fn set_source(src: Option<HdEditionSource>) {
    if let Ok(mut g) = cell().write() {
        *g = src.map(Arc::new);
    }
}

/// 当前注入的来源（未注入 → `None`，全链路退化为原版行为）
pub fn source() -> Option<Arc<HdEditionSource>> {
    cell().read().ok().and_then(|g| g.clone())
}

/// 回收 HD 来源的 sheet 解码缓存（构建结束时调用，见 [`HdEditionSource::trim_sheets`]）。
///
/// 未注入来源时是 no-op。与 `infra::ai::clear_sessions` 一起在 `modcache::batch` 收尾调用，
/// 把素材 / 高清阶段解码出的图集（最多 128 MB）从内存里释放回去。
pub fn trim_source_cache() {
    if let Some(src) = source() {
        src.trim_sheets();
    }
}

/// 当前来源指纹（未注入 → `"off"`）。见 [`HdEditionSource::fingerprint`]。
///
/// 缓存侧（`modcache::hd_map::source_mark_matches`）据此判断"是否需要重跑素材阶段补登记"。
pub fn source_fingerprint() -> String {
    match source() {
        Some(s) => s.fingerprint(),
        None => "off".to_string(),
    }
}

/// 按 `launcher.json` 配置探测并注入（**构建批次入口调用一次**）。
///
/// - `hd.edition_use == "off"` → 清除注入（本次构建不用 HD 素材）；
/// - 其余（缺省 / `"auto"`）→ 在用户模组目录 / 游戏模组目录探测，检测到即用；
/// - `hd.edition_scale == "x2"` → 只打开 x2 档（省磁盘）；其余取可用最高档（x3 优先）。
///
/// `fallback_game_dir` 用于配置缺失时的兜底（游戏根 `Mods/`）。
pub fn setup_from_config(
    cfg: Option<&crate::vcmi::config::LauncherConfig>,
    fallback_game_dir: &Path,
) {
    let use_kind = cfg.map(|c| c.hd.edition_use.as_str()).unwrap_or("auto");
    if use_kind.eq_ignore_ascii_case("off") {
        set_source(None);
        return;
    }
    let scale = cfg.and_then(|c| match c.hd.edition_scale.to_ascii_lowercase().as_str() {
        "x2" => Some(2),
        _ => None,
    });
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(c) = cfg {
        roots.push(c.paths.user_mods_dir.clone());
        if let Some(gm) = &c.paths.game_mods_dir {
            roots.push(gm.clone());
        }
        roots.push(c.paths.game_dir.join("Mods"));
    }
    if roots.is_empty() {
        roots.push(fallback_game_dir.join("Mods"));
    }
    // 保序去重（`game_mods_dir` 常与 `game_dir/Mods` 相同；顺序即优先级，不能排序）
    let mut seen = std::collections::HashSet::new();
    roots.retain(|p| seen.insert(p.clone()));
    let src = detect(&roots, scale);
    if let Some(s) = &src {
        tracing::info!(
            target: "wiki",
            "HD 素材来源已启用: {}（档位 {:?}）",
            s.dir().display(),
            s.scales()
        );
    }
    set_source(src);
}

/// 便捷入口：按资源名查 HD 图（未注入来源时恒为 `None`）。
pub fn lookup(name: &str) -> Option<(image::RgbaImage, u32)> {
    source()?.lookup(name)
}

/// 便捷入口：按资源名查 HD 图 + 该帧的 1x 逻辑绘制偏移（见 [`HdEditionSource::lookup_with_offset`]）。
pub fn lookup_with_offset(name: &str) -> Option<(image::RgbaImage, u32, (i32, i32))> {
    source()?.lookup_with_offset(name)
}

/// 探测当前状态（设置页展示用；**只读、不注入**）。
///
/// 返回：`{ found, dir, scales, enabled }` —— `enabled` 反映配置开关（`hd.edition_use != "off"`），
/// `found` 反映磁盘上是否真的能在候选目录里定位到 pak。
pub fn status_json() -> serde_json::Value {
    status_json_impl(None, None)
}

/// 按**显式目录**探测（首启向导：`launcher.json` 尚未落盘，`read_launcher_config()` 为空，
/// 但用户已经在第 ③ 步把素材装进了所选目标）。传空 = 退回已保存配置。
pub fn status_json_at(game_dir: Option<&Path>, user_data_dir: Option<&Path>) -> serde_json::Value {
    status_json_impl(game_dir, user_data_dir)
}

fn status_json_impl(
    game_dir: Option<&Path>,
    user_data_dir: Option<&Path>,
) -> serde_json::Value {
    use serde_json::json;
    let cfg = crate::vcmi::paths::VcmiPaths::read_launcher_config();
    let enabled = cfg
        .as_ref()
        .map(|c| !c.hd.edition_use.eq_ignore_ascii_case("off"))
        .unwrap_or(true);
    let scale = cfg
        .as_ref()
        .and_then(|c| match c.hd.edition_scale.to_ascii_lowercase().as_str() {
            "x2" => Some(2),
            _ => None,
        });
    let mut roots: Vec<PathBuf> = Vec::new();
    match (game_dir, user_data_dir) {
        (Some(g), Some(u)) => {
            // 与已保存配置同口径：用户模组目录 + 游戏模组目录（两处都可能存在 hd-edition）
            roots.push(u.join("Mods"));
            roots.push(g.join("Mods"));
        }
        _ => {
            if let Some(c) = &cfg {
                roots.push(c.paths.user_mods_dir.clone());
                if let Some(gm) = &c.paths.game_mods_dir {
                    roots.push(gm.clone());
                }
                roots.push(c.paths.game_dir.join("Mods"));
            }
        }
    }
    match detect(&roots, scale) {
        Some(s) => json!({
            "found": true,
            "dir": s.dir().to_string_lossy(),
            "scales": s.scales(),
            "enabled": enabled,
        }),
        None => json!({ "found": false, "dir": null, "scales": [], "enabled": enabled }),
    }
}

/// 便捷入口：按**需求倍率**取 HD 图（未注入来源 / 未命中 → `None`）。
///
/// 供 `modcache::hd::upscale_file` 在生成 `@Nx` 变体时调用：返回 `(图, 源档, 放大倍数)`
/// —— `k == 1` 表示原图直出、零插值。
pub fn lookup_for(name: &str, need: u32) -> Option<(image::RgbaImage, u32, u32)> {
    source()?.lookup_for(name, need)
}

/// 便捷入口：取某帧在指定档位的**阴影图**（见 [`HdEditionSource::lookup_shadow`]）。
pub fn lookup_shadow(name: &str, scale: u32) -> Option<image::RgbaImage> {
    source()?.lookup_shadow(name, scale)
}

/// 便捷入口：按**目标倍数**取图 + 绘制偏移（见 [`HdEditionSource::lookup_with_offset_for`]）。
pub fn lookup_with_offset_for(name: &str, need: u32) -> Option<(image::RgbaImage, u32, (i32, i32))> {
    source()?.lookup_with_offset_for(name, need)
}

/// 便捷入口：按 DEF 的 `(group, index)` 查 HD 图（帧名换算见 [`DefReader::frame_name`]）。
///
/// 传入的是**已在手的 DEF 字节**（提取链路本来就要读它），因此不需要再次解析资源。
pub fn lookup_def_frame(
    def_bytes: &[u8],
    group: usize,
    index: usize,
) -> Option<(image::RgbaImage, u32)> {
    let src = source()?;
    let def = DefReader::new(def_bytes);
    let name = def.frame_name(group, index)?;
    src.lookup(&name)
}

/// 便捷入口：按 DEF 帧**探测** HD 是否收录（**不解码图像**，只查索引）。
///
/// 用途：素材阶段把"哪些图有真高清"登记进 `<mod>/hd-map.json`（见 `modcache::hd_map`），
/// 供 HD 变体阶段（`modcache::hd`）取源 —— 登记只付一次哈希查表的代价。
pub fn probe_def_frame(def_bytes: &[u8], group: usize, index: usize) -> Option<String> {
    let src = source()?;
    let def = DefReader::new(def_bytes);
    let name = def.frame_name(group, index)?;
    if let Some(hit) = probe_in(&src, &name) {
        return Some(hit);
    }
    // 动画化 DEF（地形 tiles、河流等）的帧名是**动画根**（`WATRTL01`），HD 素材按
    // **子帧** 存（`WATRTL01_0`…`WATRTL01_9`）：只查主名必 miss —— 实测水域 tiles
    // 因此永远登记不上（`terrain/watrtl_def~g0f0` 缺失，只能 1x 放大）。
    // 回退首子帧：语义上正是"该动画的第 0 个子帧"，与本函数"取静态首帧"的用途一致。
    probe_in(&src, &format!("{name}_0"))
}

/// 便捷入口：按资源名**探测** HD 是否收录（只查索引，不解码）。
///
/// 名字取资源基名即可（如 `HPL000KN.PCX` → `HPL000KN`）；pak 内部统一大写。
pub fn probe(name: &str) -> Option<String> {
    let src = source()?;
    probe_in(&src, name)
}

/// 查名字是否存在于来源的任一档（返回**命中的归一化键**，供登记表使用）
fn probe_in(src: &HdEditionSource, name: &str) -> Option<String> {
    let keys = candidate_keys(name);
    if keys.is_empty() {
        return None;
    }
    for s in &src.scales {
        for pak in &s.paks {
            for key in &keys {
                if pak.find(key).is_some() {
                    return Some(key.to_uppercase());
                }
            }
        }
    }
    None
}

/// 资源名 → 候选键（**去路径 / 去扩展名**，按"具体 → 宽松"排序）。
///
/// pak 的键是 H3 原版**基名**（`TBCSBACK`、`HPL000KN`），而调用方常传带路径/扩展名的引用
/// （`TBCSBACK.bmp`、`Sprites/HPL000KN.pcx`）—— 直接查会全 miss，故统一在此归一。
///
/// 只对长度 ≤4 的尾部扩展名做剥离（避免把 `core.castle` 这类含点号的名字切错）。
fn candidate_keys(name: &str) -> Vec<String> {
    let raw = name.trim().replace('\\', "/");
    let file = raw.rsplit('/').next().unwrap_or(&raw).trim().to_string();
    if file.is_empty() {
        return Vec::new();
    }
    let mut out = vec![file.clone()];
    if let Some((stem, ext)) = file.rsplit_once('.') {
        if !stem.is_empty() && !ext.is_empty() && ext.len() <= 4 {
            out.push(stem.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 首启向导第 ④ 步口径：**显式目录**探测（此时 `launcher.json` 还没写），
    /// 应命中游戏目录 `Mods/hd-edition`；不存在的目录不得误报。
    /// `cargo test --lib status_json_at -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn status_json_at_uses_explicit_dirs() {
        let root =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../references/Heroes3-VCMI");
        if !root.is_dir() {
            eprintln!("跳过：缺少 references/Heroes3-VCMI");
            return;
        }
        let user = root.join("userdata");
        let v = status_json_at(Some(root.as_path()), Some(user.as_path()));
        println!("explicit dirs → {v}");
        assert_eq!(v["found"], true, "游戏目录 Mods 下的 hd-edition 应被命中");
        assert_eq!(
            v["dir"].as_str().unwrap_or("").to_lowercase().contains("mods"),
            true,
            "命中路径应带 Mods 段"
        );

        let none = status_json_at(Some(Path::new("Z:/vcmi-fv-none")), Some(Path::new("Z:/vcmi-fv-none")));
        assert_eq!(none["found"], false, "空目录不应误报");
    }

    /// 真机冒烟：探测本机 hd-edition，并按名取到幕布 / 建筑图标 / 头像。
    ///
    /// 期望值来自 2026-09-23 的独立实测（x2 pak：`TBCSBACK` 1600×748、`THCSMAG1` 300×140、
    /// `HPL000KN` 116×128、`CPRS097S` 64×64 且 rotation=1 → 旋转后仍 64×64）。
    #[test]
    #[ignore]
    fn real_hd_source_lookup() {
        let _g = crate::vcmi::hd_edition::test_guard();
        let user_mods = std::env::var("VCMI_USER_MODS_DIR")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../references/Heroes3-VCMI/userdata/Mods")
            });
        let src = detect(&[PathBuf::from(user_mods)], Some(2)).expect("应探测到 hd-edition（x2）");
        assert_eq!(src.scales(), vec![2], "限定 x2 时只应打开 x2 档");

        let (bg, scale) = src.lookup("TBCSBACK").expect("应取到城镇幕布");
        assert_eq!(scale, 2);
        assert_eq!(bg.dimensions(), (1600, 748));

        let (icon, _) = src.lookup("THCSMAG1").expect("应取到建筑图标");
        assert_eq!(icon.dimensions(), (300, 140));

        let (portrait, _) = src.lookup("HPL000KN").expect("应取到英雄头像");
        assert_eq!(portrait.dimensions(), (116, 128));

        // rotation=1 的帧：截取后仍应是 64×64（旋转不改正方形尺寸，但验证不 panic 且非空）
        let (cprs, _) = src.lookup("CPRS097S").expect("应取到生物小图");
        assert_eq!(cprs.dimensions(), (64, 64));

        // 未收录项：元素城（HD 版不含 SoD 内容）应未命中而不是报错
        assert!(src.lookup("TBELBACK").is_none(), "元素城幕布不应命中");
    }

    /// DEF 帧名 → HD 取图的完整链路（需本机游戏目录 + hd-edition）
    #[test]
    #[ignore]
    fn real_hd_lookup_def_frame() {
        let _g = crate::vcmi::hd_edition::test_guard();
        let game = std::env::var("VCMI_GAME_DIR")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../references/Heroes3-VCMI")
            });
        let user_mods = std::env::var("VCMI_USER_MODS_DIR")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../references/Heroes3-VCMI/userdata/Mods")
            });
        let src = detect(&[PathBuf::from(user_mods)], Some(2)).expect("应探测到 hd-edition");
        set_source(Some(src));
        let lod = crate::infra::lod::LodArchive::open(
            &PathBuf::from(game).join("Data").join("H3sprite.lod"),
        )
        .expect("H3sprite.lod 应可打开");
        let def_bytes = lod.read("HALLCSTL.DEF").expect("应能读出 HALLCSTL.DEF");
        let (img, _scale) = lookup_def_frame(&def_bytes, 0, 0).expect("第 0 帧应取到 HD 图");
        assert_eq!(img.dimensions(), (300, 140), "THCSMAG1 = 300×140");
        set_source(None);
    }

    /// 档位选择规则（x2 与 x3 都可用时）：
    /// `need <= 最高档` → 最小覆盖、其次最小放大倍数；`need > 最高档` → **取最高档**。
    #[test]
    #[ignore]
    fn real_hd_lookup_for_scale_choice() {
        let _g = crate::vcmi::hd_edition::test_guard();
        let user_mods = std::env::var("VCMI_USER_MODS_DIR")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../references/Heroes3-VCMI/userdata/Mods")
            });
        let src = detect(&[PathBuf::from(user_mods)], None).expect("应探测到 hd-edition");
        assert_eq!(src.scales(), vec![3, 2], "默认应含 x3 与 x2（降序）");

        // need=4（> 最高档 3）→ 取 x3（细节最多），由 `upscale_file` 重采样到 4× 精确尺寸。
        // 旧规则选 x2 再放大 2×，实测明显更糊（x2 源本身细节不足）。
        let (img, h, k) = src.lookup_for("TBCSBACK", 4).expect("应取到幕布");
        assert_eq!((h, k), (3, 2));
        assert_eq!(img.dimensions(), (2400, 1122));

        // need=6：x3 档 k=2 与 x2 档 k=3 覆盖同为 6x → 取 k 小的 x3
        let (img, h, k) = src.lookup_for("TBCSBACK", 6).expect("应取到幕布");
        assert_eq!((h, k), (3, 2));
        assert_eq!(img.dimensions(), (2400, 1122));

        // need=2：x2 档原图直出（k=1，零插值）
        let (_img, h, k) = src.lookup_for("HPL000KN", 2).expect("应取到头像");
        assert_eq!((h, k), (2, 1));
    }

    /// 资源名归一：去路径、去短扩展名；含点号但不是扩展名的名字（`core.castle`）不误切。
    ///
    /// 这是幕布（`TBCSBACK.bmp`）与头像（`Sprites/HPL000KN.pcx`）能命中 pak 键的关键。
    #[test]
    fn candidate_keys_normalizes() {
        assert_eq!(
            candidate_keys("TBCSBACK.bmp"),
            vec!["TBCSBACK.bmp", "TBCSBACK"]
        );
        assert_eq!(
            candidate_keys("Sprites/HPL000KN.pcx"),
            vec!["HPL000KN.pcx", "HPL000KN"]
        );
        assert_eq!(candidate_keys("core.castle"), vec!["core.castle"]);
        assert!(candidate_keys("   ").is_empty());
    }

    /// 配置语义：`off` 必须清除注入；`auto` 但磁盘上没有 pak 时不得误启用。
    #[test]
    fn setup_from_config_off_and_missing() {
        let _g = test_guard();
        let mut cfg: crate::vcmi::config::LauncherConfig =
            serde_json::from_value(serde_json::json!({
                "paths": {
                    "game_dir": "Z:/definitely/not/here",
                    "user_data_dir": "Z:/definitely/not/here",
                    "user_mods_dir": "Z:/definitely/not/here/Mods"
                }
            }))
            .expect("LauncherConfig 应可由最小 JSON 构造");

        cfg.hd.edition_use = "off".into();
        setup_from_config(Some(&cfg), Path::new("Z:/definitely/not/here"));
        assert!(source().is_none(), "off 应清除注入");

        cfg.hd.edition_use = "auto".into();
        setup_from_config(Some(&cfg), Path::new("Z:/definitely/not/here"));
        assert!(source().is_none(), "探测不到 pak 时不应启用");
    }
}
