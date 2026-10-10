import { createContext, useCallback, useContext, useEffect, useRef } from "react";
import type { WikiCategory } from "@/lib/modcache";

/**
 * 对齐 VCMI 的右键弹窗交互（client/gui/EventDispatcher.cpp）：
 *   右键按下 → dispatchShowPopup()  → showPopupWindow()
 *   右键松开 → dispatchClosePopup() → popWindows(1)
 * 触屏同理：长按 longTouchTimeMilliseconds 显示，抬手关闭。
 * 因此弹窗是"按住显示、松开关闭"的瞬时窗：无遮罩、无关闭按钮、不阻塞底层交互。
 *
 * 单独成文件是为了避免 EntryDetail ↔ WikiPopup 的循环依赖。
 */

export type Point = { x: number; y: number };

export type WikiPopupApi = {
  /** 文本气泡（CRClickPopup）：居中于光标，InfoWindows.cpp: center(cursor) + fitToScreen(10) */
  showText: (text: string, p: Point) => void;
  /** 完整窗（CStackWindow / CHeroOverview）：屏幕居中 */
  showEntry: (modId: string, category: WikiCategory, obj: Record<string, unknown>) => void;
  close: () => void;
};

export const WikiPopupCtx = createContext<WikiPopupApi | null>(null);

export function useWikiPopup(): WikiPopupApi {
  const api = useContext(WikiPopupCtx);
  return (
    api ?? {
      showText: () => {},
      showEntry: () => {},
      close: () => {},
    }
  );
}

/**
 * 行级触发器工厂：表格行数不固定，无法每行各挂一个 hook，
 * 故返回可复用的 props 工厂（共享一个长按计时器，同时只可能有一个长按）。
 */
export function usePopupTriggers(longTouchMs = 500) {
  const timer = useRef<number | null>(null);
  const start = useRef<Point>({ x: 0, y: 0 });

  const clear = useCallback(() => {
    if (timer.current !== null) {
      window.clearTimeout(timer.current);
      timer.current = null;
    }
  }, []);

  useEffect(() => clear, [clear]);

  return useCallback(
    (fire: (p: Point) => void) => ({
      onMouseDown: (e: React.MouseEvent) => {
        if (e.button !== 2) return;
        e.preventDefault();
        fire({ x: e.clientX, y: e.clientY });
      },
      onContextMenu: (e: React.MouseEvent) => {
        e.preventDefault();
      },
      onTouchStart: (e: React.TouchEvent) => {
        const t = e.touches[0];
        if (!t) return;
        start.current = { x: t.clientX, y: t.clientY };
        clear();
        timer.current = window.setTimeout(() => fire(start.current), longTouchMs);
      },
      onTouchMove: (e: React.TouchEvent) => {
        const t = e.touches[0];
        if (!t) return;
        if (Math.hypot(t.clientX - start.current.x, t.clientY - start.current.y) > 12) clear();
      },
      onTouchEnd: clear,
      onTouchCancel: clear,
    }),
    [clear, longTouchMs],
  );
}
