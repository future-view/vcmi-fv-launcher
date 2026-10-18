import { AppBrand } from "@/shared/AppBrand";
import { WindowControls } from "@/shared/WindowControls";

/**
 * 首启标题栏：与主界面 `TopBar` **同一套视觉模板与拖拽结构**（60px 高、同底色/分隔线、
 * 左品牌 + 右窗口控件），但没有页签 / 设置 / 多语言 —— 初始化完成前它们不可用
 * （设置依赖已保存的路径，语言在向导第 1 步选择）。
 *
 * ⚠️ 拖拽区结构必须与 `TopBar` 完全一致：`data-tauri-drag-region` + `cursor-move` 挂在
 * **内层 flex-1 div** 上，窗口控件是它的**兄弟**而非后代。此前把拖拽区挂在整个 `<header>` 上，
 * 控件成了拖拽区的后代——控件自身的 `cursor-pointer` 被 header 的 `cursor-move` 语境吞掉，
 * 右上角三个按钮 hover 没有手型（用户连报两轮）。兄弟结构下按钮在拖拽区之外，光标不受影响。
 */
export function InitTopBar() {
  return (
    <header className="flex h-[60px] flex-none items-center justify-between border-b border-slate-800/70 bg-slate-950/85 px-5 backdrop-blur-xl select-none">
      <div
        data-tauri-drag-region
        style={{ WebkitAppRegion: "drag" } as React.CSSProperties}
        className="flex h-full min-w-0 flex-1 cursor-move items-center gap-3"
      >
        <AppBrand />
      </div>
      <div
        style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
        className="flex flex-none"
      >
        <WindowControls />
      </div>
    </header>
  );
}
