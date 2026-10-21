//! sprites-json 动画（VCMI 1.6+ 的 **png 动画模式**）读取。
//!
//! 两种写法（VCMI `RenderHandler::initFromJson` 都支持，必须先铺 `sequences` 再让
//! `images` 逐帧覆盖）：
//!
//! ```json
//! {
//!   "basepath": "yokai/creatures/battle/lv7up2/",
//!   "sequences": [ { "group": 0, "frames": ["00100.PNG", ...] }, ... ]
//! }
//! ```
//!
//! ```json
//! { "basepath": "hota/factory/creatures/battle/CDREAD/",
//!   "images": [ { "group": 0, "frame": 0, "file": "CDREAD.dA000..png" }, ... ] }
//! ```
//!
//! 四个关键事实（都实测过）：
//! 1. 帧是**整幅 450×400 画布**的 png —— 与原版 DEF 同一坐标空间，所以能直接套用
//!    VCMI `CCreaturePic` 的显示窗口裁剪（见 [`crate::vcmi::resource`] 的 `WIN_*`）；
//! 2. `basepath` 相对**模组的 Sprites 根**（`<mod>/Content/Sprites/`），且大小写不敏感
//!    （配置写 `00100.PNG`、磁盘是 `00100.png`）—— 由 `resolve_resource_fallback`
//!    的归一化索引天然处理；
//! 3. **只认 `sequences` 是不够的**（2026-10-21 修正）：HotA Factory 的沙虫 / 死亡沙虫 /
//!    重型机甲 / 无畏机甲 4 个生物**没有 `.def`**，动画完全由 `images` 写法提供
//!    （`graphics.animation` 里写的 `.def` 只是个不存在的基名）。只认 `sequences` 时
//!    这 4 个生物所有动作组都解析失败 → 百科退回静态大图（用户报"动画无法播放"）；
//! 4. 但 `images` 是"逐帧覆盖"语义，跨模组命中会顶掉别人完整的 DEF —— 故只在
//!    **请求方自己的模组**里才采用（见 [`locate`] 的跨模组限制）。
//!
//! `generateShadow` / `generateOverlay` 暂不实现：实测这些模组都写 0（不生成）。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::Value;

/// 载入结果：帧 + 该生物的**自适应显示窗口**。
pub(crate) struct JsonAnim {
    pub frames: Vec<image::RgbaImage>,
    /// `(x, y, w, h)`（1x 画布坐标）。跨组只增不减，避免切动作时窗口跳动。
    pub window: (i64, i64, i64, i64),
}

/// 每生物累计见过的最大的自适应窗口：`(mod_id, resource)` → 窗口。
///
/// png 动画没有 DEF 那种"读帧头就知道边界"的便宜渠道，只能解码后算 alpha 包围盒；
/// 而按组算会让窗口随动作变化（待机小、攻击大）→ 动画跳动。这里**只增不减**地累计，
/// 用户看过几个动作后窗口即稳定，且永远不会比已见过的任何一帧小。
static WINDOW_CACHE: Mutex<Option<HashMap<(String, String), (i64, i64, i64, i64)>>> =
    Mutex::new(None);

/// 载入某分组的全部帧（**整画布 RGBA**）及其自适应窗口。组不存在 / 任一帧读不到 → `None`。
pub(crate) fn load_group(
    paths: &crate::vcmi::paths::VcmiPaths,
    mod_id: &str,
    resource: &str,
    group: usize,
    double_wide: bool,
) -> Option<JsonAnim> {
    let (text, owner) = locate(paths, mod_id, resource)?;
    let v: Value = serde_json::from_str(&text).ok()?;
    // `basepath` 在 `locate` 里已确认存在；缺失时按空串处理（等价于相对 Sprites 根）
    let base = v.get("basepath").and_then(|x| x.as_str()).unwrap_or("");
    let names = group_frame_names(&v, group)?;
    let mut out = Vec::with_capacity(names.len());
    let mut bbox: Option<(i64, i64, i64, i64)> = None;
    for name in &names {
        let img = load_frame(&owner, &paths.game_dir, base, name)?;
        if let Some(r) = alpha_bbox(&img) {
            bbox = Some(match bbox {
                Some((x0, y0, x1, y1)) => (x0.min(r.0), y0.min(r.1), x1.max(r.2), y1.max(r.3)),
                None => r,
            });
        }
        out.push(img);
    }
    if out.is_empty() {
        return None;
    }
    // 与 DEF 同款：基准窗口 ∪ 包围盒（封顶外扩 + 最小留白），再按生物累计取最大
    let key = (mod_id.to_string(), resource.to_string());
    let canvas = Some((out[0].width(), out[0].height()));
    let win = crate::vcmi::resource::two_layout_window(double_wide, canvas, bbox);
    let merged = {
        let mut guard = match WINDOW_CACHE.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let map = guard.get_or_insert_with(HashMap::new);
        let prev = map.get(&key).copied();
        let m = match prev {
            Some(p) => (p.0.min(win.0), p.1.min(win.1), p.2.max(win.2), p.3.max(win.3)),
            None => win,
        };
        map.insert(key.clone(), m);
        m
    };
    Some(JsonAnim {
        frames: out,
        window: merged,
    })
}

/// 某分组的帧文件名，**两种写法都认**（与 VCMI `RenderHandler::initFromJson` 同口径）：
///
/// - `sequences`：`[{ "group": 0, "frames": ["a.png", ...] }]` —— 整组列出；
/// - `images`：`[{ "group": 0, "frame": 3, "file": "a.png" }]` —— 逐帧**覆盖/补齐**。
///
/// VCMI 的实际顺序是"先用 `sequences` 铺满组，再让 `images` 按 `frame` 覆盖"
/// （`source[group][frame] = ImageLocator(...)`），这里同序合并：先铺槽位再按帧号写入。
///
/// **空洞直接跳过**（`frame` 跳号且无 `sequences` 打底）：这种写法只出现在"对既有 DEF
/// 逐帧覆盖"的场景（实测 `fix_h3creatures` 的 `CAELEM/CHDRGN/Cstorm/CWRAIT.json`），
/// 基础帧本应由 DEF 提供 —— 本模块不做 DEF 合并（属独立待办），跳空槽比凭空造一帧诚实。
fn group_frame_names(v: &Value, group: usize) -> Option<Vec<String>> {
    // 槽位：`None` = 该帧号没有 png（由 DEF 提供或本就没有）
    let mut slots: Vec<Option<String>> = Vec::new();
    if let Some(seqs) = v.get("sequences").and_then(|x| x.as_array()) {
        if let Some(seq) = seqs.iter().find(|s| group_of(s) == Some(group)) {
            if let Some(frames) = seq.get("frames").and_then(|x| x.as_array()) {
                slots = frames.iter().map(frame_name).collect();
            }
        }
    }
    if let Some(imgs) = v.get("images").and_then(|x| x.as_array()) {
        for it in imgs.iter().filter(|it| group_of(it) == Some(group)) {
            let Some(frame) = it.get("frame").and_then(|f| f.as_u64()).map(|f| f as usize) else {
                continue;
            };
            let Some(name) = frame_name(it) else { continue };
            if slots.len() <= frame {
                slots.resize(frame + 1, None);
            }
            slots[frame] = Some(name);
        }
    }
    let names: Vec<String> = slots.into_iter().flatten().collect();
    if names.is_empty() {
        None
    } else {
        Some(names)
    }
}

/// 节点上的 `group`（缺省/非数字 → `None`）
fn group_of(node: &Value) -> Option<usize> {
    node.get("group").and_then(|g| g.as_u64()).map(|g| g as usize)
}

/// 帧名：字符串（`sequences.frames[]` 常规写法）或对象内的 `file`（`images[]` 写法）。
fn frame_name(node: &Value) -> Option<String> {
    let s = match node {
        Value::String(s) => s.as_str(),
        Value::Object(_) => node.get("file").and_then(|f| f.as_str())?,
        _ => return None,
    };
    if s.trim().is_empty() {
        None
    } else {
        Some(s.to_string())
    }
}

/// 一帧的**不透明像素包围盒**（画布坐标）。
fn alpha_bbox(img: &image::RgbaImage) -> Option<(i64, i64, i64, i64)> {
    let (w, h) = (img.width(), img.height());
    let mut x0 = w as i64;
    let mut y0 = h as i64;
    let mut x1 = -1i64;
    let mut y1 = -1i64;
    for (px, py, p) in img.enumerate_pixels() {
        if p.0[3] == 0 {
            continue;
        }
        let (x, y) = (px as i64, py as i64);
        x0 = x0.min(x);
        y0 = y0.min(y);
        x1 = x1.max(x);
        y1 = y1.max(y);
    }
    if x1 < x0 || y1 < y0 {
        return None;
    }
    Some((x0, y0, x1 + 1, y1 + 1))
}

/// 定位动画 json：**请求方 mod 优先 → 其它已装 mod → 游戏根**（与 DEF 同款顺序，
/// 避免从几百个 mod 里逐个开 zip）。返回 `(json 文本, 帧资源归属目录)`。
///
/// ## 跨模组命中的限制（仅 `images` 方言）
///
/// `images` 写法几乎都是"对既有 DEF 的逐帧覆盖"（实测 `fix_h3creatures` 给 core 的
/// 气元素/金龙/雷元素/幽灵各写一份），自身**不含**完整动画。若允许跨模组命中，
/// 请求 core 生物时会拿别的模组"修图用的覆盖帧"顶替它自己完整的 DEF 动画
/// （帧数变少、缺组）—— 故这类定义**只认请求方自己的模组**。
/// `sequences` 是整组定义（可独立播放），维持原有的跨模组检索。
fn locate(
    paths: &crate::vcmi::paths::VcmiPaths,
    mod_id: &str,
    resource: &str,
) -> Option<(String, PathBuf)> {
    crate::vcmi::resource::set_wiki_user_data_dir(&paths.user_dir);
    let all = crate::vcmi::scan::cached_mod_paths(paths);
    crate::vcmi::resource::set_wiki_mod_paths(&all);
    let own = crate::vcmi::resource::ordered_mod_paths(paths, mod_id);
    for mp in own.iter().chain(all.iter().map(|(p, _)| p)) {
        let is_own = own.iter().any(|p| p == mp);
        if let Some((buf, _ext, origin)) =
            crate::vcmi::resource::resolve_resource_origin(mp, &paths.game_dir, resource)
        {
            // 只认"长得像动画定义"的 json：`basepath` 必须有，组描述是
            // `sequences`（整组）或 `images`（逐帧覆盖）**任一**即可。
            //
            // ⚠️ 非 UTF-8 / 非 JSON 的命中要 `continue` 而不是 `?`：同一资源名在不同
            // mod 里可能是**真正的 `.def`**（二进制），一个二进制命中就把整个函数判死，
            // 会让排在后面的 json 定义永远读不到（跨 mod 覆盖场景必踩）。
            let Ok(text) = String::from_utf8(buf) else { continue };
            let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
            let has_sequences = v.get("sequences").and_then(|x| x.as_array()).is_some();
            let has_images = v.get("images").and_then(|x| x.as_array()).is_some();
            if v.get("basepath").and_then(|x| x.as_str()).is_none()
                || !(has_sequences || has_images)
            {
                continue;
            }
            // 只有 `images`、又是别的模组的定义 → 不采用（理由见函数文档）
            if has_images && !has_sequences && !is_own {
                continue;
            }
            let owner = match origin {
                crate::vcmi::resource::Origin::Game => paths.game_dir.clone(),
                _ => mp.clone(),
            };
            return Some((text, owner));
        }
    }
    None
}

/// 取一帧：`basepath + 帧名`，附 `Sprites/` 前缀变体（VCMI 会给自己补 `SPRITES/`）。
///
/// ⚠️ 解码必须走 [`crate::infra::image::decode_anim_frame`]，不能直接用
/// `image::load_from_memory(...).to_rgba8()`：H3 素材是**8bpp 索引色**，透明/阴影表达在
/// **调色板索引**上（0=透明、1/4=阴影），`image` 展开成 RGBA 后这些语义就丢了 ——
/// 实测 HotA 生物帧（`CSANDWX/*.png`）因此整块天蓝幕布 + 脚下洋红星（用户报图）。
fn load_frame(owner: &Path, game_dir: &Path, base: &str, name: &str) -> Option<image::RgbaImage> {
    let base = base.replace('\\', "/");
    let rel = format!("{base}{name}");
    for cand in [rel.clone(), format!("Sprites/{rel}"), format!("sprites/{rel}")] {
        if let Some((buf, ext)) =
            crate::vcmi::resource::resolve_resource_fallback(owner, game_dir, &cand)
        {
            let e = ext.to_lowercase();
            if !matches!(e.as_str(), "png" | "bmp" | "pcx" | "webp" | "jpg" | "jpeg") {
                continue;
            }
            if let Some(img) = crate::infra::image::decode_anim_frame(&buf) {
                return Some(img);
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{frame_name, group_frame_names};
    use serde_json::json;

    fn names(v: &serde_json::Value, group: usize) -> Option<Vec<String>> {
        group_frame_names(v, group)
    }

    /// 老写法：`sequences[].frames[]` 字符串数组，按组取。
    #[test]
    fn sequences_写法按组取帧() {
        let v = json!({
            "basepath": "yokai/lv7up2/",
            "sequences": [
                { "group": 0, "frames": ["00100.PNG", "00101.PNG"] },
                { "group": 1, "frames": ["00200.PNG"] }
            ]
        });
        assert_eq!(
            names(&v, 0),
            Some(vec!["00100.PNG".to_string(), "00101.PNG".to_string()])
        );
        assert_eq!(names(&v, 1), Some(vec!["00200.PNG".to_string()]));
        assert_eq!(names(&v, 2), None, "不存在的组必须是 None");
    }

    /// 实测形态：HotA Factory 的 `CDREAD.json` / `CSANDWX.json`（无 `.def` 兜底）
    /// —— 只有 `images`，按 `(group, frame)` 归位，且文件顺序与帧号**不一致**。
    #[test]
    fn images_写法按帧号归位() {
        let v = json!({
            "basepath": "hota/factory/creatures/battle/CDREAD/",
            "images": [
                { "group": 0, "frame": 1, "file": "CDREAD.dA001..png" },
                { "group": 0, "frame": 0, "file": "CDREAD.dA000..png" },
                { "group": 5, "frame": 0, "file": "CDREAD.dF000..png" }
            ]
        });
        assert_eq!(
            names(&v, 0),
            Some(vec![
                "CDREAD.dA000..png".to_string(),
                "CDREAD.dA001..png".to_string()
            ])
        );
        assert_eq!(names(&v, 5), Some(vec!["CDREAD.dF000..png".to_string()]));
        assert_eq!(names(&v, 3), None);
    }

    /// 两种写法同存时：`sequences` 打底、`images` 按帧号覆盖（VCMI 同序）。
    #[test]
    fn images_覆盖sequences_同帧号() {
        let v = json!({
            "basepath": "x/",
            "sequences": [{ "group": 0, "frames": ["a.png", "b.png", "c.png"] }],
            "images": [{ "group": 0, "frame": 1, "file": "b_fix.png" }]
        });
        assert_eq!(
            names(&v, 0),
            Some(vec!["a.png".to_string(), "b_fix.png".to_string(), "c.png".to_string()])
        );
    }

    /// 帧号跳号（对 DEF 做逐帧覆盖的写法）：空洞跳过，不凭空造帧。
    #[test]
    fn images_空洞跳过() {
        let v = json!({
            "basepath": "x/",
            "images": [
                { "group": 0, "frame": 0, "file": "a.png" },
                { "group": 0, "frame": 3, "file": "d.png" }
            ]
        });
        assert_eq!(names(&v, 0), Some(vec!["a.png".to_string(), "d.png".to_string()]));
    }

    /// 帧项兼容对象写法的 `file`（`sequences` 里也可能出现对象）。
    #[test]
    fn 帧项可为对象() {
        assert_eq!(
            frame_name(&json!({ "file": "a.png" })),
            Some("a.png".to_string())
        );
        assert_eq!(frame_name(&json!("b.png")), Some("b.png".to_string()));
        assert_eq!(frame_name(&json!({ "file": "  " })), None, "空文件名不得当成帧");
        assert_eq!(frame_name(&json!(1)), None);
    }
}
