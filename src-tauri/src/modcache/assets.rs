//! 缓存素材落盘：图标 / DEF 帧 / 特长图标的提取与保存（受线程局部素材开关控制）。
//!
//! 产物写入 `mods/<sanitize_id(mod_id)>/images/<sub>/<name>.<ext>`，返回**相对缓存根**的路径。
//!
//! 依赖方向：可依赖 `infra` 与 `vcmi`（见 `docs/architecture/backend.zh-CN.md` §4.1）。

use std::fs;
use std::path::Path;

use serde_json::Value;

use crate::infra::image::{encode_rgba_auto, encode_webp, encode_webp_from_bgra};
use crate::infra::json::parse_vcmi_json;
use crate::vcmi::resource::{resolve_resource, resolve_resource_fallback};
use crate::modcache::{assets_enabled, sanitize_id};

/// 缓存图片的相对路径（相对缓存根）。落盘与"是否已存在"判定必须**同一口径**，否则短路会误判。
pub(crate) fn wiki_image_rel(mod_id: &str, sub: &str, name: &str) -> String {
    format!(
        "{}/images/{}/{}.webp",
        sanitize_id(mod_id),
        sub,
        sanitize_id(name)
    )
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
    mod_id: &str,
    sub: &str,
    name: &str,
) -> Option<String> {
    if !assets_enabled() {
        return None;
    }
    let rel = wiki_image_rel(mod_id, sub, name);
    if cache_root.join(&rel).is_file() {
        crate::infra::metrics::count("img.reuse", 1);
        Some(rel)
    } else {
        None
    }
}

pub(crate) fn save_wiki_image(cache_root: &Path, mod_id: &str, sub: &str, name: &str, buf: &[u8], ext: &str) -> Option<String> {
    // 阶段一（只出数据）不产图：调用方拿到 None，实体 image 字段留空，由阶段二补齐
    if !assets_enabled() {
        return None;
    }
    let rel = wiki_image_rel(mod_id, sub, name);
    let out = cache_root.join(&rel);
    // 产物幂等：已存在则跳过解码+编码。增量重跑、中断续跑时省下大量 CPU。
    // 缓存失效重建前会清空该 mod 的 images/ 目录（见 build_mod_wiki_inner），故不会残留旧版本产物。
    if out.is_file() {
        // 幂等命中：增量重跑/中断续跑时省下解码+编码（埋点用于统计复用率）
        crate::infra::metrics::count("img.cached", 1);
        return Some(rel);
    }
    let webp = encode_webp(buf, ext)?;
    fs::create_dir_all(out.parent()?).ok()?;
    fs::write(&out, &webp).ok()?;
    crate::infra::metrics::count("img.saved", 1);
    crate::infra::metrics::bytes("img.bytes.disk", webp.len() as u64);
    Some(rel)
}

/// 位图类资源登记：命中官方 HD（`hd-edition`）时把 `<sub>/<name>` → HD 键 写入登记表。
///
/// 与 DEF 帧类的区别：这里用**资源名**（去路径 / 去扩展名由 `hd_edition::candidate_keys` 负责），
/// 无需读 DEF 头，成本仅一次哈希查表。
fn note_hd_bitmap(mod_id: &str, sub: &str, name: &str, res: &str) {
    if let Some(hd) = crate::vcmi::hd_edition::probe(res) {
        crate::modcache::hd_map::record(mod_id, sub, name, &hd);
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
pub(crate) fn extract_icon_and_save(p: &Path, game_dir: &Path, cache_root: &Path, mod_id: &str, sub: &str, name: &str, res: Option<&str>) -> Option<String> {
    crate::infra::metrics::scope("img.icon", || {
        extract_icon_and_save_inner(p, game_dir, cache_root, mod_id, sub, name, res)
    })
}

fn extract_icon_and_save_inner(p: &Path, game_dir: &Path, cache_root: &Path, mod_id: &str, sub: &str, name: &str, res: Option<&str>) -> Option<String> {
    // 阶段一（只出数据）：**在解析资源之前**直接返回，省下 resolve/解码/抠图/编码
    if !assets_enabled() {
        return None;
    }
    // 产物已存在：直接复用路径，不再解析与编码（跨 mod 共享图标此前每 mod 重做一遍）
    if let Some(rel) = wiki_image_exists(cache_root, mod_id, sub, name) {
        return Some(rel);
    }
    let res = res?;
    let res_trim = res.trim();
    if res_trim.is_empty() {
        return None;
    }
    // mod 优先，失败回退游戏根资源（原版英雄头像 TPLICHxx.DEF 等位于游戏根 lods）
    let (buf, ext) = resolve_resource_fallback(p, game_dir, res_trim)?;
    // sprites-json 资源（VCMI 1.6+）：**按内容识别**（resolve 的尾匹配可能命中同名 .def，
    // 故 ext==json 或引用尾缀 .json 时都尝试解析）；支持 sequences 形态、basepath 缺省、
    // file 自带扩展名（bmp/png）与 zip 内定义（见 read_sprite_json / sprite_json_frame）。
    let (buf, ext) = if ext.eq_ignore_ascii_case("json") || res_trim.to_lowercase().ends_with(".json") {
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
        let def = crate::infra::def::DefReader::new(&buf);
        if let Some((w, h, bgra, _lm, _tm)) = def.decode_frame_at(0, 0) {
            if let Some(rel) = save_def_frame_matte(cache_root, mod_id, sub, name, w, h, &bgra) {
                return Some(rel);
            }
            let webp = encode_webp_from_bgra(&bgra, w, h)?;
            return save_wiki_image(cache_root, mod_id, sub, name, &webp, "webp");
        }
        return None;
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
            matte_and_save(cache_root, mod_id, sub, name, &buf)
        });
        if let Some(rel) = rel {
            note_hd_bitmap(mod_id, sub, name, res_trim);
            return Some(rel);
        }
    }
    let rel = save_wiki_image(cache_root, mod_id, sub, name, &buf, &ext)?;
    // 位图类资源也可能有官方 HD 版（如第三方 mod 引用原版 `HPL000KN.pcx`）→ 登记
    note_hd_bitmap(mod_id, sub, name, res_trim);
    Some(rel)
}

/// DEF 单帧（group0/frame0）四角抠图去底框色后落盘：小帧（≤64px，如 SPELLBK/ARTIFACT 32×32、
/// HOTA 法术/宝物 def 首帧）常带青蓝底框，期望透明底（对齐小图标 matte 逻辑，见 [`matte_rgba`]）。
fn save_def_frame_matte(
    cache_root: &Path,
    mod_id: &str,
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
    save_wiki_image(cache_root, mod_id, sub, name, &webp, "webp")
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
    let opaque: Vec<[u8; 4]> = corners
        .iter()
        .map(|p| p.0)
        .filter(|c| c[3] > 0)
        .collect();
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
    mod_id: &str,
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
        return save_wiki_image(cache_root, mod_id, sub, name, &webp, "webp");
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
    name: &str,
    def_res: &str,
    group: usize,
    index: usize,
) -> Option<String> {
    let def_res = def_res.trim();
    if def_res.is_empty() {
        return None;
    }
    // 同上：阶段一不解析；产物已存在则直接复用（短路必须早于资源解析与 DEF 解码）
    if !assets_enabled() {
        return None;
    }
    if let Some(rel) = wiki_image_exists(cache_root, mod_id, sub, name) {
        return Some(rel);
    }
    // mod 优先，失败回退游戏根资源（原版建筑图标 HALLCSTL.DEF 等位于游戏根 lods）
    let (buf, _ext) = resolve_resource_fallback(p, game_dir, def_res)?;
    // 只解目标帧：此前解码整个动画再挑一帧（DEF 动辄上百帧）
    let def = crate::infra::def::DefReader::new(&buf);
    let (w, h, bgra, _lm, _tm) = def.decode_frame_at(group, index)?;
    let rel = save_wiki_image(
        cache_root,
        mod_id,
        sub,
        name,
        &encode_webp_from_bgra(&bgra, w, h)?,
        "webp",
    )?;
    // 该帧若有官方 HD 真高清源则登记（HD 变体阶段据此取源放大，替代从 1x 插值）
    if let Some(hd) = crate::vcmi::hd_edition::probe_def_frame(&buf, group, index) {
        crate::modcache::hd_map::record(mod_id, sub, name, &hd);
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
    name: &str,
) -> Option<String> {
    // 阶段一（只出数据）不解 DEF 取帧，由阶段二补齐
    if !assets_enabled() {
        return None;
    }
    // 原版游戏级图标统一归位到父 `core` 模组的 images/ 下：产物已存在则直接复用，
    // 不再读 lod、不再解码 DEF（此前每个 mod 都完整重做一遍）
    if let Some(rel) = wiki_image_exists(cache_root, "core", sub, name) {
        return Some(rel);
    }
    let target = if def_name.to_uppercase().ends_with(".DEF") {
        def_name.trim().to_string()
    } else {
        format!("{}.DEF", def_name.trim())
    };
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
    // 原版游戏级图标统一归位到父 `core` 模组的 images/ 下（`_shared` 收编进 core，
    // 迁移时仅需拷贝 `core*` 目录）。各 mod 的 statIcons/sharedIcons 均引用此路径。
    let dir = cache_root.join("core").join("images").join(sub);
    fs::create_dir_all(&dir).ok()?;
    let rel = format!("core/images/{}/{}.webp", sub, sanitize_id(name));
    let out = cache_root.join(&rel);
    if out.is_file() {
        return Some(rel);
    }
    fs::write(&out, &webp).ok()?;
    // 共享 UI 图标的产物**归属父 `core`**（可能由任意 mod 的构建执行），登记也必须归到 core
    if let Some(hd) = crate::vcmi::hd_edition::probe_def_frame(&bytes, group, frame) {
        crate::modcache::hd_map::record("core", sub, name, &hd);
    }
    Some(rel)
}

/// 与 [`extract_game_def_frame_and_save`] 同逻辑（从游戏目录 lod 解析 DEF 取指定帧转 webp），
/// 但产物按 `mod_id` 落到 `cache_root/<mod_id>/images/<sub>/<name>.webp`，而非全局 `_shared/`。
///
/// 用途：原版生物/英雄资源（CPRSMALL/TWCRPORT/UN32/UN44 等游戏级 DEF）归位到各自的
/// `core.<faction>` 模组目录（中性归入 `core`），使每个原版城镇模组自包含、便于打包迁移，
/// 不再把所有阵营资源混进全局 `_shared/`。
pub(crate) fn extract_game_def_frame_and_save_mod(
    game_dir: &Path,
    cache_root: &Path,
    mod_id: &str,
    def_name: &str,
    group: usize,
    frame: usize,
    sub: &str,
    name: &str,
) -> Option<String> {
    if !assets_enabled() {
        return None;
    }
    let target = if def_name.to_uppercase().ends_with(".DEF") {
        def_name.trim().to_string()
    } else {
        format!("{}.DEF", def_name.trim())
    };
    let bytes = crate::vcmi::resource::read_game_resource_fallback(game_dir, &target)?;
    // 只解目标帧
    let def = crate::infra::def::DefReader::new(&bytes);
    let (w, h, bgra, _lm, _tm) = def.decode_frame_at(group, frame)?;
    let webp = encode_webp_from_bgra(&bgra, w, h)?;
    let rel = save_wiki_image(cache_root, mod_id, sub, name, &webp, "webp")?;
    // 生物小/大图、英雄特长图标（CPRSMALL / TWCRPORT / UN32 / UN44）：登记 HD 源
    if let Some(hd) = crate::vcmi::hd_edition::probe_def_frame(&bytes, group, frame) {
        crate::modcache::hd_map::record(mod_id, sub, name, &hd);
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
    mod_id: &str,
    hero_id: &str,
    image_index: i64,
) -> Option<String> {
    if !assets_enabled() || image_index < 0 {
        return None;
    }
    let idx = image_index as usize;
    // a) 老版本 PortraitsLarge.DEF（group0 顺序帧）
    if let Some(bytes) = crate::vcmi::resource::read_game_resource_fallback(game_dir, "PortraitsLarge.DEF") {
        // 只解目标帧：此前每个英雄都解码**整个** PortraitsLarge（150+ 帧）再挑 1 帧
        let def = crate::infra::def::DefReader::new(&bytes);
        if let Some((w, h, bgra, _lm, _tm)) = def.decode_frame_at(0, idx) {
            if let Some(webp) = encode_webp_from_bgra(&bgra, w, h) {
                let rel = save_wiki_image(cache_root, mod_id, "hero", hero_id, &webp, "webp")?;
                // 头像有官方 HD 版（`HPL*`），按 DEF 帧名登记
                if let Some(name) = def.frame_name(0, idx) {
                    if let Some(hd) = crate::vcmi::hd_edition::probe(&name) {
                        crate::modcache::hd_map::record(mod_id, "hero", hero_id, &hd);
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
                        if let Some(b) = crate::vcmi::resource::read_game_resource_fallback(game_dir, &cand) {
                            if let Some(webp) = encode_webp(&b, ext) {
                                let rel = save_wiki_image(cache_root, mod_id, "hero", hero_id, &webp, "webp")?;
                                // json 的 `file`（如 `HPL000KN.bmp`）就是 HD pak 的键（`HPL*`）→ 登记
                                if let Some(hd) = crate::vcmi::hd_edition::probe(file) {
                                    crate::modcache::hd_map::record(mod_id, "hero", hero_id, &hd);
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
        return extract_game_def_frame_and_save_mod(game_dir, cache_root, mod_id, def_name, 0, image_index as usize, sub, &name);
    }
    if let Some((buf, ext)) = resolve_resource(p, raw) {
        if ext.eq_ignore_ascii_case("def") {
            // 只要 group0 首帧：不再解码整个动画
            let def = crate::infra::def::DefReader::new(&buf);
            if let Some((w, h, bgra, _lm, _tm)) = def.decode_frame_at(0, 0) {
                if let Some(webp) = encode_webp_from_bgra(&bgra, w, h) {
                    return save_wiki_image(cache_root, mod_id, sub, &name, &webp, "webp");
                }
            }
            return None;
        }
        return save_wiki_image(cache_root, mod_id, sub, &name, &buf, &ext);
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
            .or_else(|| images.iter().find(|x| g_of(x).map(|g| g == group).unwrap_or(true)))
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
                if let Some((buf, ext)) = resolve_resource_fallback(p, game_dir, &cand) {
                    let e = ext.to_lowercase();
                    if e == "json" || e == "def" {
                        continue; // 帧引用指向定义文件属异常写法，跳过
                    }
                    let rel = save_wiki_image(cache_root, mod_id, sub, name, &buf, &ext)?;
                    note_hd_bitmap(mod_id, sub, name, &cand);
                    return Some(rel);
                }
            }
        }
        // 定义存在但取不到位图：直接失败（旧口径在此会误按 DEF 解析 → 静默无图）
        return None;
    }
    // 2) 非 json：裸位图直接保存；其余（DEF 等）委托单帧解码
    if let Some((buf, ext)) = resolve_resource_fallback(p, game_dir, res_ref) {
        let e = ext.to_lowercase();
        if e == "json" {
            return None;
        }
        if matches!(e.as_str(), "png" | "bmp" | "pcx" | "webp") {
            let rel = save_wiki_image(cache_root, mod_id, sub, name, &buf, &ext)?;
            note_hd_bitmap(mod_id, sub, name, res_ref);
            return Some(rel);
        }
    }
    extract_def_frame_and_save(p, game_dir, cache_root, mod_id, sub, name, res_ref, 0, frame)
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
