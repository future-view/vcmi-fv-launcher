//! 城镇预览合成：把 `structures`（建筑）叠加到 `townScreen` 背景，产出 `preview` 图。
//!
//! 结果写入 `mods/<id>/images/preview...` 并回写 `mod.json` 的 `factions[].preview`；
//! 前端在详情页发现 `preview` 缺失时也会调用（按需合成）。
//!
//! 依赖方向：可依赖 `infra` 与 `vcmi`（见 `docs/architecture/backend.zh-CN.md` §4.1）。

use std::fs;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::infra::image::encode_webp;
use crate::vcmi::paths::VcmiPaths;
use crate::vcmi::resource::resolve_resource;
use crate::vcmi::town::value_to_resource;
use crate::modcache::assets::{read_sprite_json, sprite_json_frame, sprite_ref_candidates};
use crate::modcache::sanitize_id;

/// 解析结构动画引用为 RGBA 图（返回图 + left/top margin）：
/// - VCMI 1.6+ sprites-json（`basepath + file` → 整画布帧，x/y=0 为正常值）：
///   定义**按内容判定**——支持无扩展名引用（`xanadu/town-hall` 实际是 `.json`）、
///   zip 内定义、`images[{frame,file}]` 与 `sequences[{group,frames[]}]` 两种形态、
///   `basepath` 缺省与 `file` 自带扩展名（christmas 的 `.bmp`）；
/// - 传统 DEF（group0/frame0 解码，BGRA→RGBA，palette[0] 色键透明），绘制位置 = pos + margins。
///
/// 旧实现只看 `anim.ends_with(".json")`：实测 5 个城镇的引用后缀与实体类型不符 →
/// 结构帧/背景合成整批失败（与建筑图标同一根因）。
pub(crate) fn load_structure_frame(
    p: &Path,
    core: &Path,
    game_dir: &Path,
    anim: &str,
) -> Option<(image::RgbaImage, i64, i64)> {
    // 1) sprites-json：先按内容识别定义（磁盘精确 + zip 统一解析）
    if let Some(v) = read_sprite_json(p, anim).or_else(|| read_sprite_json(core, anim)) {
        if let Some((base, file)) = sprite_json_frame(&v, 0, 0) {
            for cand in sprite_ref_candidates(base.as_deref(), &file) {
                if let Some((pb, _pe)) =
                    resolve_resource(p, &cand).or_else(|| resolve_resource(core, &cand))
                {
                    if let Ok(img) = image::load_from_memory(&pb) {
                        return Some((img.to_rgba8(), 0, 0));
                    }
                }
            }
        }
        // 定义存在但取不到位图：不再误按 DEF 解析该 json 字节
        return None;
    }
    // 2) 传统 DEF
    let bytes = resolve_resource(p, anim)
        .or_else(|| resolve_resource(core, anim))
        .or_else(|| crate::vcmi::resource::read_game_resource_fallback(game_dir, anim).map(|b| (b, "def".to_string())))?
        .0;
    let def = crate::infra::def::DefReader::new(&bytes);
    let frames = def.decode_animation_full()?;
    let (_g, w, h, mut bgra, lm, tm) = frames.into_iter().find(|(g, _, _, _, _, _)| *g == 0)?;
    if w == 0 || h == 0 {
        return None;
    }
    for px in bgra.chunks_exact_mut(4) {
        px.swap(0, 2);
    }
    let img = image::RgbaImage::from_raw(w, h, bgra)?;
    Some((img, lm as i64, tm as i64))
}

/// 把建筑结构叠加到底图画布，返回合成后的 webp 字节。
///
/// 与 `compose_town_preview_for` 共用同一套合成逻辑（升级链去重 + z 升序 + alpha-over），
/// 供 `build_towns` 在**背景**阶段即生成"底图 + 各建筑"的合成图——
/// 原版城镇 `townBackground` 只是裸底图（如 TBCSBACK），需在此叠加建筑精灵，
/// 才能与其它（背景已预合成）的城镇视觉一致。
///
/// `base_ext` 为资源实际扩展名（lod 命中时为 pcx/bmp 等），统一走 `encode_webp` 管线解码，
/// 兼容 H3 PCX / DEF / Lod 裸图。
pub(crate) fn composite_structures_to_canvas(
    mod_path: &Path,
    game_dir: &Path,
    base_buf: &[u8],
    base_ext: &str,
    structures: &std::collections::HashMap<String, Value>,
    buildings: &std::collections::HashMap<String, Value>,
) -> Option<Vec<u8>> {
    use image::imageops;
    // 1) 底图解码为 RGBA 画布（统一走 encode_webp，兼容 H3 PCX/DEF/Lod 裸图）
    let webp = encode_webp(base_buf, base_ext)?;
    let mut canvas: image::RgbaImage = image::load_from_memory(&webp).ok()?.to_rgba8();

    // 2) 结构挑选（升级链去重 + z 升序）—— 与 HD 路径共用同一规则，避免两处口径分叉
    let to_render = collect_structures_to_render(structures, buildings);

    let core = game_dir.join("Mods").join("vcmi");
    for st in &to_render {
        match load_structure_frame(mod_path, &core, game_dir, &st.anim) {
            Some((img, lm, tm)) => {
                imageops::overlay(&mut canvas, &img, st.x + lm, st.y + tm);
            }
            None => continue,
        }
    }

    let mut out = Vec::new();
    let enc = image::codecs::webp::WebPEncoder::new_lossless(&mut out);
    enc.encode(
        canvas.as_raw(),
        canvas.width(),
        canvas.height(),
        image::ExtendedColorType::Rgba8,
    )
    .ok()?;
    Some(out)
}

/// 城镇幕布的 **HD 合成**（2026-09-23 拍板的"选项 A"）：底图与结构帧都优先用官方 HD 素材。
///
/// ## 为什么单独一个函数
///
/// 一是**不改既有调用点/测试**：未装 hd-edition（或该城底图未被收录）时逐字节退回
/// [`composite_structures_to_canvas`]；二是坐标系复杂 —— HD 底图尺寸 = 原版 × 档位（x2/x3），
/// 而 `structures` 的 `x/y` 与 DEF 帧的 `lm/tm` 都是 **1x 逻辑值**，必须整体乘档位才对得上
/// （否则建筑会挤在左上角）。
///
/// ## 结构帧的取源
///
/// HD pak 按原版 DEF 的帧名索引，命中则用 HD 帧；**未命中则把 1x 帧整数放大**到同一档位
/// —— 官方 HD 版并未收录全部城镇建筑外观（实测 `TBCSCSTL` 在 pak 里只有 1 帧），
/// 于是成品是"底图真高清 + 建筑略糊"，这是素材覆盖度决定的观感上限。
///
/// ## 编码
///
/// HD 幕布体积大（x3 = 2400×1122），走**有损 q85**（同 `hd::HD_QUALITY`）：
/// 无损会有数 MB/张（实测 1600×748 无损 1855 KB → 有损 234 KB）。
pub(crate) fn composite_structures_to_canvas_hd(
    mod_path: &Path,
    game_dir: &Path,
    base_res: &str,
    base_buf: &[u8],
    base_ext: &str,
    structures: &std::collections::HashMap<String, Value>,
    buildings: &std::collections::HashMap<String, Value>,
) -> Option<Vec<u8>> {
    // HD 底图（`TBCSBACK.bmp` → pak 的 `TBCSBACK`，名字归一在 `hd_edition` 内做）
    let fallback = |_: ()| {
        composite_structures_to_canvas(mod_path, game_dir, base_buf, base_ext, structures, buildings)
    };
    let Some((mut canvas, hd_scale)) = crate::vcmi::hd_edition::lookup(base_res) else {
        return fallback(());
    };
    if hd_scale < 2 {
        return fallback(());
    }

    let to_render = collect_structures_to_render(structures, buildings);
    let core = game_dir.join("Mods").join("vcmi");
    let s = hd_scale as i64;
    for st in &to_render {
        if let Some((img, lm, tm)) = load_structure_frame_hd(mod_path, &core, game_dir, &st.anim, hd_scale) {
            // 坐标统一乘档位：`structures.x/y` 与 DEF 的 `lm/tm` 均为 1x 逻辑值
            image::imageops::overlay(&mut canvas, &img, (st.x + lm) * s, (st.y + tm) * s);
        }
    }
    crate::infra::image::encode_webp_lossy_q(
        canvas.as_raw(),
        canvas.width(),
        canvas.height(),
        crate::modcache::hd::HD_QUALITY,
    )
}

/// 一条待绘制结构（幕布坐标 + 动画引用）
struct StructDraw {
    x: i64,
    y: i64,
    z: i64,
    anim: String,
}

/// 从 `structures` / `buildings` 挑选要绘制的结构：绑定组按升级链取最深、装饰直接绘制、
/// 结果按 `z` 升序。**1x 与 HD 两条合成路径共用**（规则只应有一份）。
fn collect_structures_to_render(
    structures: &std::collections::HashMap<String, Value>,
    buildings: &std::collections::HashMap<String, Value>,
) -> Vec<StructDraw> {
    // 升级链深度：depth=沿 upgrades 步数；root=链尾（base）
    let mut depth: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut root_of: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    for bid in buildings.keys() {
        let mut cur = bid.clone();
        let mut d = 0usize;
        loop {
            let nxt = buildings
                .get(&cur)
                .and_then(|b| b.get("upgrades"))
                .and_then(|u| u.as_str())
                .map(|s| s.to_string());
            match nxt {
                Some(n) if buildings.contains_key(&n) && &n != bid && d < 16 => {
                    cur = n;
                    d += 1;
                }
                _ => break,
            }
        }
        depth.insert(bid.clone(), d);
        root_of.insert(bid.clone(), cur);
    }

    let mut to_render: Vec<StructDraw> = Vec::new();
    if !structures.is_empty() {
        let mut groups: std::collections::HashMap<String, (usize, StructDraw)> =
            std::collections::HashMap::new();
        for (skey, sv) in structures {
            let anim = match sv.get("animation").and_then(value_to_resource) {
                Some(a) => a,
                None => continue,
            };
            let sx = sv.get("x").and_then(|v| v.as_i64()).unwrap_or(0);
            let sy = sv.get("y").and_then(|v| v.as_i64()).unwrap_or(0);
            let sz = sv.get("z").and_then(|v| v.as_i64()).unwrap_or(0);
            let st = StructDraw {
                x: sx,
                y: sy,
                z: sz,
                anim,
            };
            let bound = sv
                .get("builds")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| {
                    if buildings.contains_key(skey) {
                        Some(skey.clone())
                    } else {
                        None
                    }
                });
            match bound {
                Some(b) => {
                    let d = *depth.get(&b).unwrap_or(&0);
                    let r = root_of.get(&b).cloned().unwrap_or_else(|| b.clone());
                    let replace = match groups.get(&r) {
                        Some((bd, _)) => *bd < d,
                        None => true,
                    };
                    if replace {
                        groups.insert(r, (d, st));
                    }
                }
                None => to_render.push(st),
            }
        }
        for (_r, (_d, st)) in groups {
            to_render.push(st);
        }
    }
    to_render.sort_by_key(|s| s.z);
    to_render
}

/// 同 [`load_structure_frame`]，但**优先取官方 HD 帧**，并把结果统一到 `dst_scale` 坐标系。
///
/// - 先按 DEF 帧名查 HD pak（sprites-json 属第三方资源、pak 不收录 → 直接走回退分支）；
/// - 未命中 HD：取 1x 帧后**整数放大**到 `dst_scale`（像素画优先 xBRZ，失败回退 Lanczos3）。
///
/// `pub(crate)`：供 `modcache::smoke_tests` 直调断言"HD 帧带对了绘制偏移"。
pub(crate) fn load_structure_frame_hd(
    p: &Path,
    core: &Path,
    game_dir: &Path,
    anim: &str,
    dst_scale: u32,
) -> Option<(image::RgbaImage, i64, i64)> {
    // 1) HD 优先：只有 DEF 有意义（pak 按原版 DEF 的 13 字节帧名索引）
    let def_bytes = resolve_resource(p, anim)
        .or_else(|| resolve_resource(core, anim))
        .or_else(|| {
            crate::vcmi::resource::read_game_resource_fallback(game_dir, anim)
                .map(|b| (b, "def".to_string()))
        });
    if let Some((buf, ext)) = def_bytes {
        if !ext.eq_ignore_ascii_case("json") {
            let def = crate::infra::def::DefReader::new(&buf);
            if let Some(name) = def.frame_name(0, 0) {
                if let Some((hd_img, h, off)) = crate::vcmi::hd_edition::lookup_with_offset(&name) {
                    if let Some(img) = fit_scale(&hd_img, h, dst_scale) {
                        // 位置必须复刻 VCMI `HdImageLoader::getImage`：
                        // `setMargins((defMargins - spriteOffset) * scalingFactor)`。
                        // 帧名命中 HD **不等于**能沿用结构自身的 x/y —— DEF 帧自带 lm/tm
                        // （城镇船坞 `TBCSDKNN` 的 tm=53：忽略就把船坞上移 53px 盖住战神像）。
                        let (lm, tm) = def.frame_margins(0, 0).unwrap_or((0, 0));
                        return Some((
                            img,
                            lm as i64 - off.0 as i64,
                            tm as i64 - off.1 as i64,
                        ));
                    }
                }
            }
        }
    }
    // 2) 回退：原逻辑取 1x 帧，再放大到目标档位
    let (img, lm, tm) = load_structure_frame(p, core, game_dir, anim)?;
    Some((scale_up_frame(&img, dst_scale)?, lm, tm))
}

/// 把 1x 图**整数放大** `scale` 倍（城镇建筑外观是硬边像素画 → 优先 xBRZ；
/// 失败或超倍率上限时回退 Lanczos3）。
fn scale_up_frame(img: &image::RgbaImage, scale: u32) -> Option<image::RgbaImage> {
    if scale <= 1 {
        return Some(img.clone());
    }
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 {
        return None;
    }
    if let Some((px, uw, uh)) = crate::infra::upscale::upscale_rgba("xbrz", img.as_raw(), w, h, scale) {
        if let Some(out) = image::RgbaImage::from_raw(uw, uh, px) {
            return Some(out);
        }
    }
    Some(image::imageops::resize(
        img,
        w * scale,
        h * scale,
        image::imageops::FilterType::Lanczos3,
    ))
}

/// 把 HD 帧从它的原始档位 `src_scale` 归一到 `dst_scale`：相等直接复用；
/// 倍数关系走整数放大；其余（罕见）用 Lanczos3 重采样。
fn fit_scale(img: &image::RgbaImage, src_scale: u32, dst_scale: u32) -> Option<image::RgbaImage> {
    if src_scale == dst_scale {
        return Some(img.clone());
    }
    if dst_scale > src_scale && src_scale > 0 && dst_scale % src_scale == 0 {
        return scale_up_frame(img, dst_scale / src_scale);
    }
    let (w, h) = (img.width(), img.height());
    if w == 0 || h == 0 || src_scale == 0 {
        return None;
    }
    let tw = (w * dst_scale / src_scale).max(1);
    let th = (h * dst_scale / src_scale).max(1);
    Some(image::imageops::resize(
        img,
        tw,
        th,
        image::imageops::FilterType::Lanczos3,
    ))
}

/// 城镇预览合成（**兜底 + 按需**入口）。
///
/// 构建期主路径已不经过它：`build_towns` 产出的 `images/town/<fid>.webp` 自带建筑叠加，
/// 且其 structures/buildings 取自**跨文件并集**（比本函数的"单文件优先命中"口径更完整），
/// 故 `build/data/terrain.rs` 直接把它复用为 `preview`。
/// 本函数只在「该 faction 连底图都没有、`background` 为空」时兜底合成，
/// 以及前端 `generateTownPreview` 按需补图时调用。
///
/// ⚠️ 本体**不做 HD 合成**（走 1x）：主路径已由 [`composite_structures_to_canvas_hd`] HD 化；
/// 本路径只在底图缺失的罕见场景触发，产物尺寸与登记缺失都能被下次构建的增量补齐。
///
/// towns_data = rawConfigs（含 towns 归一数据）。幂等：目标图已存在则直接返回相对路径。
pub(crate) fn compose_town_preview_for(
    mod_path: &Path,
    cache_root: &Path,
    mod_id: &str,
    faction_id: &str,
    towns_data: &Value,
) -> Result<String, String> {
    // 埋点：城镇预览图合成（解码全部建筑 DEF + 大图拼合 + 编码）是素材阶段最重的单项
    crate::infra::metrics::scope("town.preview", || {
        compose_town_preview_for_inner(mod_path, cache_root, mod_id, faction_id, towns_data)
    })
}

fn compose_town_preview_for_inner(
    mod_path: &Path,
    cache_root: &Path,
    mod_id: &str,
    faction_id: &str,
    towns_data: &Value,
) -> Result<String, String> {
    let sid = sanitize_id(mod_id);
    let fid2 = sanitize_id(faction_id);
    let rel = format!("{}/images/townscreen/{}.webp", sid, fid2);
    if cache_root.join(&rel).is_file() {
        return Ok(rel);
    }
    let towns = towns_data.get("towns").and_then(|t| t.as_object());
    let town_obj: Value = towns
        .and_then(|m| m.get(faction_id))
        .cloned()
        .or_else(|| {
            towns.and_then(|m| {
                m.values()
                    .find_map(|v| v.get(faction_id).cloned())
            })
        })
        .or_else(|| {
            // 兜底：files 里内容嗅探
            towns_data.get("files").and_then(|f| f.as_object()).and_then(|m| {
                m.values().find(|v| {
                    let direct = !v.get(faction_id).cloned().unwrap_or(Value::Null).is_null();
                    let marks = |x: &Value| {
                        x.get("clientInfo").is_some() || x.get("townScreen").is_some() || x.get("buildings").is_some()
                    };
                    direct || marks(v) || v.as_object().map(|o| o.values().any(marks)).unwrap_or(false)
                })
            }).and_then(|v| v.get(faction_id).cloned().or_else(|| Some(v.clone())))
        })
        .unwrap_or(Value::Null);
    if town_obj.is_null() {
        return Err(String::from("notown"));
    }
    let ci = town_obj.get("clientInfo");
    let town_sub = town_obj.get("town");
    let bg_res = ci
        .and_then(|c| c.get("townScreen"))
        .and_then(|t| t.get("background"))
        .or_else(|| town_sub.and_then(|t| t.get("townBackground")))
        .or_else(|| town_obj.get("townBackground"))
        .and_then(value_to_resource)
        .ok_or_else(|| String::from("nobg"))?;
    let structures = ci
        .and_then(|c| c.get("structures"))
        .and_then(|s| s.as_object())
        .or_else(|| town_sub.and_then(|t| t.get("structures")).and_then(|s| s.as_object()));
    let buildings_raw = town_obj.get("town").and_then(|t| t.get("buildings")).and_then(|b| b.as_object());

    let game_dir = VcmiPaths::detect().game_dir;
    let core = game_dir.join("Mods").join("vcmi");
    let bg_bytes = resolve_resource(mod_path, &bg_res)
        .or_else(|| resolve_resource(&core, &bg_res))
        // 原版背景多在 H3bitmap.lod / H3sprite.lod 包内（read_game_resource 返回裸 Vec<u8>）
        .map(|(b, e)| (b, Some(e)))
        .or_else(|| crate::vcmi::resource::read_game_resource_fallback(&game_dir, &bg_res).map(|b| (b, None)))
        .ok_or_else(|| String::from("bgresolve"))?;
    let mut canvas: image::RgbaImage = match &bg_bytes.1 {
        Some(_) => image::load_from_memory(&bg_bytes.0).map_err(|_| String::from("bgdecode"))?.to_rgba8(),
        None => {
            // lod 读出的裸图（bmp）走 encode_webp 解码管线
            let webp = encode_webp(&bg_bytes.0, "bmp").ok_or_else(|| String::from("bgdecode"))?;
            image::load_from_memory(&webp).map_err(|_| String::from("bgdecode"))?.to_rgba8()
        }
    };

    let mut depth: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut root_of: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    if let Some(bs) = buildings_raw {
        for bid in bs.keys() {
            let mut cur = bid.clone();
            let mut d = 0usize;
            loop {
                let nxt = bs.get(&cur).and_then(|b| b.get("upgrades")).and_then(|u| u.as_str()).map(|s| s.to_string());
                match nxt {
                    Some(n) if bs.contains_key(&n) && &n != bid && d < 16 => {
                        cur = n;
                        d += 1;
                    }
                    _ => break,
                }
            }
            depth.insert(bid.clone(), d);
            root_of.insert(bid.clone(), cur);
        }
    }

    struct St { x: i64, y: i64, z: i64, anim: String }
    let mut to_render: Vec<St> = Vec::new();
    if let Some(ss) = structures {
        let mut groups: std::collections::HashMap<String, (usize, St)> = std::collections::HashMap::new();
        for (skey, sv) in ss {
            let anim = match sv.get("animation").and_then(value_to_resource) {
                Some(a) => a,
                None => continue,
            };
            let sx = sv.get("x").and_then(|v| v.as_i64()).unwrap_or(0);
            let sy = sv.get("y").and_then(|v| v.as_i64()).unwrap_or(0);
            let sz = sv.get("z").and_then(|v| v.as_i64()).unwrap_or(0);
            // 注意：sprites-json 新格式的结构 x/y=0、z=-1 是正常值（整画布帧），不得跳过
            let st = St {
                x: sx,
                y: sy,
                z: sz,
                anim,
            };
            let bound = sv
                .get("builds")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| {
                    if buildings_raw.map(|b| b.contains_key(skey)).unwrap_or(false) {
                        Some(skey.clone())
                    } else {
                        None
                    }
                });
            match bound {
                Some(b) => {
                    let d = *depth.get(&b).unwrap_or(&0);
                    let r = root_of.get(&b).cloned().unwrap_or_else(|| b.clone());
                    let replace = match groups.get(&r) {
                        Some((bd, _)) => *bd < d,
                        None => true,
                    };
                    if replace {
                        groups.insert(r, (d, st));
                    }
                }
                None => to_render.push(st),
            }
        }
        for (_r, (_d, st)) in groups {
            to_render.push(st);
        }
    }
    to_render.sort_by_key(|s| s.z);

    for st in &to_render {
        // 单个结构解析失败只跳过该结构（不得毁掉整张预览）
        match load_structure_frame(mod_path, &core, &game_dir, &st.anim) {
            Some((img, lm, tm)) => {
                image::imageops::overlay(&mut canvas, &img, st.x + lm, st.y + tm);
            }
            None => continue,
        }
    }

    let mut webp = Vec::new();
    let enc = image::codecs::webp::WebPEncoder::new_lossless(&mut webp);
    enc.encode(canvas.as_raw(), canvas.width(), canvas.height(), image::ExtendedColorType::Rgba8)
        .map_err(|_| String::from("encode"))?;
    let dir = cache_root.join(&sid).join("images").join("townscreen");
    fs::create_dir_all(&dir).map_err(|_| String::from("io"))?;
    fs::write(cache_root.join(&rel), &webp).map_err(|_| String::from("io"))?;
    Ok(rel)
}

/// 城镇预览合成核心逻辑：读 raw.json 的 towns.<faction>，
/// 按 VCMI WikiTownView（镜像 CCastleInterface::recreate）叠加建筑 def 帧：
/// - 结构绑定建筑 = "builds" 字段或结构 key；绑定组按升级链 root 分组，取链最深者渲染
/// - 装饰结构直接渲染；全部按 z 升序叠加到 townScreen background
/// 结果缓存 images/townscreen/<faction>.webp，并把 preview 写回 mod.json 的 faction。
/// 还原 cache.json 的 `modPath`（由 `encode_mod_path_for_cache` 写入的 `gameMods:`/`userMods:`/`realm:<id>:` 编码串，
/// 或极特殊来源的兜底绝对路径）为运行时真实目录。
///
/// base 全部来自 `launcher.json` 权威配置与 `.vcmi-fv-launcher` 根目录，不依赖 `VcmiPaths::detect()`
/// 重新探测游戏目录，消除「探测到别的游戏目录」的歧义。缓存随包迁移后，相对串依旧能定位 mod 源。
fn resolve_cache_mod_path(
    stored: &str,
    launcher_data_dir: &Path,
    game_dir: &Path,
    user_mods_dir: &Path,
) -> Option<PathBuf> {
    if let Some(r) = stored.strip_prefix("gameMods:") {
        return Some(game_dir.join("Mods").join(r));
    }
    if let Some(r) = stored.strip_prefix("userMods:") {
        return Some(user_mods_dir.join(r));
    }
    if let Some(r) = stored.strip_prefix("realm:") {
        // r = "<realmId>:<modRel>"，base = launcher_data_dir/realms/<id>/vcmi/Mods
        if let Some((id, rest)) = r.split_once(':') {
            return Some(
                launcher_data_dir
                    .join("realms")
                    .join(id)
                    .join("vcmi")
                    .join("Mods")
                    .join(rest),
            );
        }
        return None;
    }
    // 兜底：绝对路径（极特殊来源，如实 symlink 目录）；不存在则不还原
    let abs = PathBuf::from(stored);
    if abs.is_dir() {
        return Some(abs);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_mod_path_schemes() {
        let ldd = PathBuf::from("C:/exe/.vcmi-fv-launcher");
        let game_dir = PathBuf::from("C:/game");
        let user_mods = PathBuf::from("C:/user/Mods");
        // 游戏自带 Mods
        assert_eq!(
            resolve_cache_mod_path("gameMods:abyss-town", &ldd, &game_dir, &user_mods),
            Some(PathBuf::from("C:/game/Mods/abyss-town"))
        );
        // 用户目录 Mods
        assert_eq!(
            resolve_cache_mod_path("userMods:foo", &ldd, &game_dir, &user_mods),
            Some(PathBuf::from("C:/user/Mods/foo"))
        );
        // 领域整合模组
        assert_eq!(
            resolve_cache_mod_path("realm:rl:bundled", &ldd, &game_dir, &user_mods),
            Some(PathBuf::from("C:/exe/.vcmi-fv-launcher/realms/rl/vcmi/Mods/bundled"))
        );
        // 嵌套 realm mod
        assert_eq!(
            resolve_cache_mod_path("realm:rl:sub/mod", &ldd, &game_dir, &user_mods),
            Some(PathBuf::from("C:/exe/.vcmi-fv-launcher/realms/rl/vcmi/Mods/sub/mod"))
        );
        // 兜底：绝对路径需真实存在（此处不存在 → None）；编码侧的绝对兜底见 encode 测试
        assert_eq!(
            resolve_cache_mod_path("D:/other/mod", &ldd, &game_dir, &user_mods),
            None
        );
    }
}

pub(crate) fn generate_town_preview_inner(cache_root: &Path, mod_id: &str, faction_id: &str) -> Value {
    let sid = sanitize_id(mod_id);
    let mod_json_path = cache_root.join(format!("{}/cache.json", sid));
    let mut mod_json: Value = match fs::read_to_string(&mod_json_path).ok().and_then(|t| serde_json::from_str(&t).ok()) {
        Some(v) => v,
        None => return json!({ "ok": false, "error": "notcached" }),
    };
    // 0) 幂等：已合成且文件存在 → 直接返回
    if let Some(fa) = mod_json
        .get("factions")
        .and_then(|f| f.as_array())
        .and_then(|arr| arr.iter().find(|f| f.get("factionId").and_then(|x| x.as_str()) == Some(faction_id)))
    {
        if let Some(prev) = fa.get("preview").and_then(|x| x.as_str()) {
            if !prev.is_empty() && cache_root.join(prev).is_file() {
                return json!({ "ok": true, "cached": true, "preview": prev });
            }
        }
    }
    // 还原 modPath 的 base：优先 launcher.json 权威配置，缺失则退回 detect()
    let launcher_data_dir = VcmiPaths::launcher_data_dir()
        .unwrap_or_else(|| cache_root.to_path_buf());
    let cfg = VcmiPaths::read_launcher_config();
    let game_dir = cfg
        .as_ref()
        .map(|c| c.paths.game_dir.clone())
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| VcmiPaths::detect().game_dir);
    let user_mods_dir = cfg
        .as_ref()
        .map(|c| c.paths.user_mods_dir.clone())
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| VcmiPaths::detect().user_mods_dir());
    let mod_path_str = mod_json.get("modPath").and_then(|x| x.as_str()).unwrap_or("").to_string();
    // 运行时还原：cache.json 里的 modPath 是相对 launcher.json 配置 / .vcmi-fv-launcher 根的编码串，
    // 由权威配置合成 base，使缓存随包迁移后仍可定位 mod 源以重建预览。
    let p = match resolve_cache_mod_path(&mod_path_str, &launcher_data_dir, &game_dir, &user_mods_dir) {
        Some(p) => p,
        None => return json!({ "ok": false, "error": "nomodpath" }),
    };

    // 1) raw.json → towns.<faction>
    let raw_path = cache_root.join(format!("{}/raw.json", sid));
    let raw: Value = match fs::read_to_string(&raw_path).ok().and_then(|t| serde_json::from_str(&t).ok()) {
        Some(v) => v,
        None => return json!({ "ok": false, "error": "noraw" }),
    };
    let towns = raw.get("rawConfigs").and_then(|r| r.get("towns")).and_then(|t| t.as_object());
    let town_obj: Value = towns
        .and_then(|m| m.get(faction_id))
        .cloned()
        .or_else(|| {
            towns.and_then(|m| {
                m.values()
                    .find_map(|v| v.get(faction_id).cloned())
            })
        })
        .unwrap_or(Value::Null);
    // 兜底：旧缓存 rawConfigs 无 towns 时，从 files 里按内容嗅探（含 clientInfo/buildings/townScreen）
    let town_obj = if town_obj.is_null() {
        raw.get("rawConfigs")
            .and_then(|r| r.get("files"))
            .and_then(|f| f.as_object())
            .and_then(|m| {
                m.values().find(|v| {
                    let direct = !v.get(faction_id).cloned().unwrap_or(Value::Null).is_null();
                    let looks = v.get("clientInfo").is_some() || v.get("townScreen").is_some() || v.get("buildings").is_some();
                    direct || looks
                })
            })
            .and_then(|v| v.get(faction_id).cloned().map(|x| x).or_else(|| Some(v.clone())))
            .unwrap_or(Value::Null)
    } else {
        town_obj
    };
    if town_obj.is_null() {
        return json!({ "ok": false, "error": "notown" });
    }
    let ci = town_obj.get("clientInfo");
    let town_sub = town_obj.get("town");
    let bg_res = ci
        .and_then(|c| c.get("townScreen"))
        .and_then(|t| t.get("background"))
        .or_else(|| town_sub.and_then(|t| t.get("townBackground")))
        .or_else(|| town_obj.get("townBackground"))
        .and_then(value_to_resource);
    let bg_res = match bg_res {
        Some(r) => r,
        None => return json!({ "ok": false, "error": "nobg" }),
    };
    let structures = ci
        .and_then(|c| c.get("structures"))
        .and_then(|s| s.as_object())
        .or_else(|| town_sub.and_then(|t| t.get("structures")).and_then(|s| s.as_object()));
    let buildings_raw = town_obj.get("buildings").and_then(|b| b.as_object());

    // 2) 背景解码为 RGBA 画布
    let (bg_buf, _ext) = match resolve_resource(&p, &bg_res) {
        Some(x) => x,
        None => return json!({ "ok": false, "error": "bgresolve" }),
    };
    let mut canvas: image::RgbaImage = match image::load_from_memory(&bg_buf) {
        Ok(i) => i.to_rgba8(),
        Err(_) => return json!({ "ok": false, "error": "bgdecode" }),
    };

    // 3) 升级链深度：depth=沿 upgrades 步数；root=链尾（base）
    let mut depth: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut root_of: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    if let Some(bs) = buildings_raw {
        for bid in bs.keys() {
            let mut cur = bid.clone();
            let mut d = 0usize;
            loop {
                let nxt = bs
                    .get(&cur)
                    .and_then(|b| b.get("upgrades"))
                    .and_then(|u| u.as_str())
                    .map(|s| s.to_string());
                match nxt {
                    Some(n) if bs.contains_key(&n) && &n != bid && d < 16 => {
                        cur = n;
                        d += 1;
                    }
                    _ => break,
                }
            }
            depth.insert(bid.clone(), d);
            root_of.insert(bid.clone(), cur);
        }
    }

    // 4) 结构挑选：绑定组取最深升级；装饰直接渲染
    struct St { x: i64, y: i64, z: i64, anim: String }
    let mut to_render: Vec<St> = Vec::new();
    if let Some(ss) = structures {
        let mut groups: std::collections::HashMap<String, (usize, St)> = std::collections::HashMap::new();
        for (skey, sv) in ss {
            let anim = match sv.get("animation").and_then(value_to_resource) {
                Some(a) => a,
                None => continue,
            };
            let sx = sv.get("x").and_then(|v| v.as_i64()).unwrap_or(0);
            let sy = sv.get("y").and_then(|v| v.as_i64()).unwrap_or(0);
            let sz = sv.get("z").and_then(|v| v.as_i64()).unwrap_or(0);
            // 注意：sprites-json 新格式的结构 x/y=0、z=-1 是正常值（整画布帧），不得跳过
            let st = St {
                x: sx,
                y: sy,
                z: sz,
                anim,
            };
            let bound = sv
                .get("builds")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
                .or_else(|| {
                    if buildings_raw.map(|b| b.contains_key(skey)).unwrap_or(false) {
                        Some(skey.clone())
                    } else {
                        None
                    }
                });
            match bound {
                Some(b) => {
                    let d = *depth.get(&b).unwrap_or(&0);
                    let r = root_of.get(&b).cloned().unwrap_or_else(|| b.clone());
                    let replace = match groups.get(&r) {
                        Some((bd, _)) => *bd < d,
                        None => true,
                    };
                    if replace {
                        groups.insert(r, (d, st));
                    }
                }
                None => to_render.push(st),
            }
        }
        for (_r, (_d, st)) in groups {
            to_render.push(st);
        }
    }
    to_render.sort_by_key(|s| s.z);

    // 5) 叠加 def 帧（group0 首帧，BGRA→RGBA，alpha-over）
    for st in &to_render {
        let buf = match resolve_resource(&p, &st.anim) {
            Some((b, _)) => b,
            None => continue,
        };
        let def = crate::infra::def::DefReader::new(&buf);
        let frames = match def.decode_animation() {
            Some(f) => f,
            None => continue,
        };
        let frame = match frames.into_iter().find(|(g, _, _, _)| *g == 0) {
            Some(f) => f,
            None => continue,
        };
        let (_g, w, h, mut bgra) = frame;
        if w == 0 || h == 0 {
            continue;
        }
        for px in bgra.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        let overlay_img = match image::RgbaImage::from_raw(w, h, bgra) {
            Some(i) => i,
            None => continue,
        };
        image::imageops::overlay(&mut canvas, &overlay_img, st.x, st.y);
    }

    // 6) 编码保存 + 写回 preview
    let mut webp = Vec::new();
    {
        let enc = image::codecs::webp::WebPEncoder::new_lossless(&mut webp);
        if enc.encode(canvas.as_raw(), canvas.width(), canvas.height(), image::ExtendedColorType::Rgba8).is_err() {
            return json!({ "ok": false, "error": "encode" });
        }
    }
    let dir = cache_root.join(&sid).join("images").join("townscreen");
    if fs::create_dir_all(&dir).is_err() {
        return json!({ "ok": false, "error": "io" });
    }
    let fid2 = sanitize_id(faction_id);
    let rel = format!("{}/images/townscreen/{}.webp", sid, fid2);
    if fs::write(cache_root.join(&rel), &webp).is_err() {
        return json!({ "ok": false, "error": "io" });
    }
    if let Some(factions) = mod_json.get_mut("factions").and_then(|f| f.as_array_mut()) {
        for f in factions.iter_mut() {
            if f.get("factionId").and_then(|x| x.as_str()) == Some(faction_id) {
                if let Some(obj) = f.as_object_mut() {
                    obj.insert("preview".to_string(), json!(rel));
                }
            }
        }
    }
    let _ = fs::write(&mod_json_path, serde_json::to_string_pretty(&mod_json).unwrap());
    json!({ "ok": true, "preview": rel })
}
