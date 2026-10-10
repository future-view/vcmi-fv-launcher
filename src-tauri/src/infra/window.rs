//! 启动器主窗口几何：按显示器推导初始窗口尺寸（**默认自适应的唯一公式来源**）。
//!
//! 计算发生在 Rust 侧、**窗口显示之前**（`lib.rs` 的 `setup`）——前端不再参与尺寸决策。
//! 否则窗口会先以 `tauri.conf.json` 的 1280×720 露一下、再被前端 `setSize` 跳到目标尺寸：
//! 一是可见跳变，二是 Windows 的 `SetWindowPos` 以**左上角为锚**扩张，放大后右下角会顶出屏幕。
//! 前端只消费结果：`installUiScale` 读到的已经是最终逻辑尺寸，首帧缩放即正确。
//!
//! 本层不出现 `tauri` 类型：显示器物理尺寸与缩放由调用方（`commands/app.rs`）取好传进来。

/// 设计稿画布（全站唯一坐标系，16:9）。与前端 `lib/tauri.ts` 的 `DESIGN_CANVAS_*` 同值。
pub const DESIGN_W: u32 = 1280;
pub const DESIGN_H: u32 = 720;
/// 最小窗口尺寸：与 `tauri.conf.json` 的 `minWidth` / `minHeight` 对齐。
pub const MIN_W: u32 = 960;
pub const MIN_H: u32 = 640;
/// 目标填充率：同时充当任务栏与桌面边距余量
/// （`Monitor::size()` 返回**全屏物理尺寸**、未扣任务栏，1080p 下余 162px，远大于任务栏 ~48px）。
const WINDOW_FILL: f64 = 0.85;
/// 上限：再大只是等比放大，且会把 800×374 幕布素材拉得更糊。
const MAX_W: u32 = 1920;
const MAX_H: u32 = 1080;

/// 解析主窗口初始尺寸（**逻辑像素**）：
///
/// 1. 有持久化尺寸 → 原样使用（用户机器上的既有约定优先）；
/// 2. 否则按显示器自适应：先按高度取（通常高度先受限）、宽度不够时从宽度回推，
///    **始终保持严格 16:9**，宽度向下取 16 的倍数使 `h = w·9/16` 必为整数（比例无 1px 误差）；
/// 3. 下限 = `max(MIN_W, DESIGN_W)`：保证设计稿 1280×720 能完整落下（1366×768 小屏落在此处）。
pub fn compute_window_size(
    saved: Option<(u32, u32)>,
    monitor_physical: (u32, u32),
    scale: f64,
) -> (u32, u32) {
    if let Some((w, h)) = saved {
        if w > 0 && h > 0 {
            return (w, h);
        }
    }
    let (mw, mh) = monitor_physical;
    if mw == 0 || mh == 0 {
        return (DESIGN_W, DESIGN_H); // 取不到显示器：退回配置默认值，至少不会更差
    }
    let dpr = if scale > 0.0 { scale } else { 1.0 };
    let logical_w = mw as f64 / dpr;
    let logical_h = mh as f64 / dpr;

    // ① 按高度取，并夹到上限
    let h = (logical_h * WINDOW_FILL).min(MAX_H as f64);
    let mut w = h * 16.0 / 9.0;
    // ② 宽度不够时从宽度回推（高度在 ④ 由最终宽度统一反推，此处不必算）
    if w > logical_w * WINDOW_FILL {
        w = logical_w * WINDOW_FILL;
    }
    // ③ 下限夹取
    let min_w = MIN_W.max(DESIGN_W) as f64;
    w = min_w.max(w.min(MAX_W as f64));
    // ④ 宽度取 16 的倍数（`min_w` 本身是 16 的倍数，故向下取整不会低于下限）
    let wq = min_w.max((w / 16.0).floor() * 16.0);
    (wq as u32, (wq * 9.0 / 16.0).max(MIN_H as f64) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_size_wins() {
        assert_eq!(
            compute_window_size(Some((1000, 600)), (3840, 2160), 2.0),
            (1000, 600)
        );
        // 非法（0）持久化值 → 忽略并回到自适应
        assert_eq!(
            compute_window_size(Some((0, 0)), (1920, 1080), 1.0),
            (1632, 918)
        );
    }

    #[test]
    fn adapts_to_1080p() {
        assert_eq!(compute_window_size(None, (1920, 1080), 1.0), (1632, 918));
    }

    #[test]
    fn hdpi_uses_logical_size() {
        // 4K + 200%：逻辑 1920×1080 → 与 1080p 结果一致（不按物理像素放大）
        assert_eq!(compute_window_size(None, (3840, 2160), 2.0), (1632, 918));
    }

    #[test]
    fn small_screen_falls_back_to_design_canvas() {
        // 1366×768：宽度回推低于下限 → 落到设计稿尺寸
        assert_eq!(compute_window_size(None, (1366, 768), 1.0), (1280, 720));
    }

    #[test]
    fn clamps_to_upper_bound() {
        // 高度先触上限：1920×1080
        assert_eq!(compute_window_size(None, (5120, 2880), 2.0), (1920, 1080));
    }

    #[test]
    fn keeps_strict_16_9_on_fractional_scale() {
        // 2560×1440 @125%：逻辑 2048×1152 → 宽度取 16 的倍数，高度必为整数
        let (w, h) = compute_window_size(None, (2560, 1440), 1.25);
        assert_eq!((w, h), (1728, 972));
        assert_eq!(w % 16, 0);
    }

    #[test]
    fn no_monitor_uses_design_canvas() {
        assert_eq!(compute_window_size(None, (0, 0), 1.0), (1280, 720));
    }
}
