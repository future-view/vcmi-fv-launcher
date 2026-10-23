/**
 * 窗口控制 + 「窗口尺寸驱动」的界面缩放（`infra/window.rs` 的前端对侧）。
 *
 * 拆分自原 `@/lib/tauri.ts` 单体；对外仍由该 barrel 统一导出，调用方无需改动。
 */
import { invoke } from "@tauri-apps/api/core";
import { isTauri } from "./core";

/** 关闭当前窗口（Tauri 桌面端有效） */
export async function closeWindow(): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("close_window");
  } catch (e) {
    console.error("关闭窗口失败", e);
  }
}

/** 最小化当前窗口（Tauri 桌面端有效） */
export async function minimizeWindow(): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("minimize_window");
  } catch (e) {
    console.error("最小化窗口失败", e);
  }
}

/**
 * 切换当前窗口最大化 / 还原（Tauri 桌面端有效）。
 *
 * 走自定义命令而非 JS 的 `window.maximize()`：`core:window:default` 只含只读状态查询
 * 与 `allow-internal-toggle-maximize`，显式 `maximize`/`unmaximize` 需额外 capability；
 * 自定义命令与既有的 `close_window`/`minimize_window` 保持一致。
 */
export async function toggleMaximizeWindow(): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("toggle_maximize_window");
  } catch (e) {
    console.error("切换窗口最大化失败", e);
  }
}

/** 当前窗口是否处于最大化（浏览器 mock 模式恒为 false）。`allow-is-maximized` 在默认权限内。 */
export async function isWindowMaximized(): Promise<boolean> {
  if (!isTauri()) return false;
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    return await getCurrentWindow().isMaximized();
  } catch {
    return false;
  }
}

/**
 * 订阅窗口尺寸变化，用于同步「最大化 / 还原」图标。
 *
 * 必须监听系统事件而不是只信自己维护的布尔值：最大化后用户拖动还原、
 * 或双击标题栏最大化，都不会经过本应用的按钮。返回取消订阅函数。
 */
export async function onWindowResized(cb: () => void): Promise<() => void> {
  if (!isTauri()) return () => {};
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    return await getCurrentWindow().onResized(() => cb());
  } catch {
    return () => {};
  }
}

/**
 * 订阅窗口**焦点**变化（Tauri v2 window API，`onFocusChanged`）。
 *
 * 用途：后台轮询类任务（如 wiki 构建进度的 400ms 兜底轮询）在窗口失焦时
 * 暂停 —— 用户把鼠标/注意力移出窗口后，主线程不该再被周期性 IPC 持续唤醒
 * （真机反馈：构建期鼠标移出窗口会不定期遇到光标转圈，即 WebView 对
 * `WM_SETCURSOR` 的响应被后台工作延迟）。恢复焦点时由调用方立即补一次。
 *
 * 非 Tauri / 失败返回 `null`（调用方退化为"始终视为聚焦"）。
 */
export async function onWindowFocusChanged(
  cb: (focused: boolean) => void,
): Promise<(() => void) | null> {
  if (!isTauri()) return null;
  try {
    const { getCurrentWindow } = await import("@tauri-apps/api/window");
    return await getCurrentWindow().onFocusChanged(({ payload: focused }) => cb(focused));
  } catch {
    return null;
  }
}

/**
 * 设计稿画布（**全站唯一坐标系**，单位 = CSS 像素，16:9）。
 *
 * 与「阵营巡礼」播放页的定尺画布（`ShowcaseOverlay`：`scale = min(vw/DW, vh/DH)`）
 * 是**同一个** 1280×720。窗口严格 16:9 时 `Z = H / DH`，于是
 * ```
 * 逻辑视口 = (W/Z) × (H/Z) = (16/9·DH) × DH = DW × DH
 * ```
 * **恒等于设计稿** —— 1080p / 2K / 4K 下排布完全一致，只有物理放大倍数不同。
 */
export const DESIGN_CANVAS_W = 1280;
export const DESIGN_CANVAS_H = 720;
// 默认窗口尺寸的常量（最小尺寸 / 0.85 填充 / 1920×1080 上限）已随计算逻辑一起搬到
// Rust 侧：`infra/window.rs`（窗口必须在显示前定尺寸，前端拿不到那个时机）。
/** 窗口尺寸变化后重算缩放的防抖（拖动 / 最大化动画结束后只算一次） */
const UI_SCALE_RESIZE_DEBOUNCE_MS = 120;

/**
 * 由**客户区逻辑尺寸**推导界面缩放（窗口驱动）。
 *
 * 设计稿 1280×720 = 1.0（与 `DESIGN_CANVAS_*` 同值）：窗口比设计稿大多少，UI 就等比放大多少，
 * 于是「最大化窗口」不会让元素显得更小——任何屏幕上最大化后的视觉密度一致。
 * 取横纵比例的**较小者**（不会从短边溢出），并**下限 1.0**：窗口小于设计稿时不缩小，
 * 保持设计稿的可读性，超出部分交给页面内部滚动。
 */
export function computeWindowScale(logicalWidth: number, logicalHeight: number): number {
  if (!(logicalWidth > 0) || !(logicalHeight > 0)) return 1;
  return Math.max(
    1,
    Math.min(logicalWidth / DESIGN_CANVAS_W, logicalHeight / DESIGN_CANVAS_H),
  );
}

/** 已应用的 WebView 缩放（值未变时跳过 setZoom，避免无谓重排） */
let appliedZoom = 1;

/**
 * 安装「窗口尺寸驱动」的界面缩放：启动算一次，窗口尺寸变化（最大化 / 还原 / 拖动）后防抖重算。
 *
 * **输入必须用 Tauri 的窗口逻辑尺寸（物理尺寸 ÷ scaleFactor），不能用 `window.innerWidth`**：
 * WebView 缩放会改变 `innerWidth`，用它推导会形成「缩放 → 视口变化 → 再缩放」的反馈震荡；
 * 窗口逻辑尺寸只与窗口本身有关、与缩放无关，天然稳定。
 *
 * 整页缩放会把 px / rem / vh 一并等比放大，故主界面、领域页、抽屉、弹层全部自动生效。
 */
export async function installUiScale(): Promise<void> {
  if (!isTauri()) return;
  let timer: number | undefined;
  const apply = async () => {
    try {
      const { getCurrentWindow } = await import("@tauri-apps/api/window");
      const win = getCurrentWindow();
      const [physical, dpr, minimized] = await Promise.all([
        win.innerSize(),
        win.scaleFactor(),
        win.isMinimized(),
      ]);
      // 最小化（或其瞬间的退化尺寸）不重算缩放：Windows 下最小化会触发
      // `Resized(0×0)`，若照常计算 `computeWindowScale` 兜底返回 1 → 整页缩放回
      // 100%，还原时再跳回原比例，造成可见的「缩放比例重新调整」。最小化期间
      // 窗口不可见，本就该保持原有缩放不变；还原后的真实尺寸与最小化前相同，
      // 下面 `scale` 与 `appliedZoom` 一致会被跳过，故全程无跳变。
      if (minimized) return;
      const logical = physical.toLogical(dpr);
      // 退化尺寸（宽或高为 0，极少数 Windows 构建在还原首帧仍会短暂报 0）同样忽略，
      // 避免错误地 `setZoom(1)`。
      if (!(logical.width > 0) || !(logical.height > 0)) return;
      const scale = computeWindowScale(logical.width, logical.height);
      if (Math.abs(scale - appliedZoom) < 0.005) return;
      const { getCurrentWebview } = await import("@tauri-apps/api/webview");
      await getCurrentWebview().setZoom(scale);
      appliedZoom = scale;
    } catch (e) {
      console.warn("应用界面缩放失败", e);
    }
  };
  await apply();
  await onWindowResized(() => {
    window.clearTimeout(timer);
    timer = window.setTimeout(() => void apply(), UI_SCALE_RESIZE_DEBOUNCE_MS);
  });
}

/**
 * ⚠️ 窗口初始尺寸**不在前端计算 / 应用**：由 Rust 在窗口显示前定好 ——
 * `infra/window.rs::compute_window_size`（唯一公式来源）+ `commands/app.rs::apply_main_window_geometry`，
 * 配合 `tauri.conf.json` 的 `"visible": false` 与 setup 里的 `show()`。
 *
 * 前端参与会带来两个问题：① 窗口先以配置默认尺寸露出、再跳大（可见跳变）；
 * ② Windows 的 `SetWindowPos` 改尺寸**以左上角为锚**向右下扩张，放大后右下角顶出屏幕。
 * 前端只**消费**结果：`installUiScale` 读到的已是最终逻辑尺寸，首帧缩放即正确。
 * 持久化字段仍是 `launcher.json` 的 `ui.window_size`（snake_case），改默认策略时清掉它即可回到自适应。
 */
