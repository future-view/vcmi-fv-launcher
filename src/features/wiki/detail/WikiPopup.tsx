import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import type { LangDict } from "@/i18n/types";
import { buildEntry, type WikiCategory, type WikiEntry, type WikiLinkTarget } from "@/lib/modcache";
import { EntryDetail } from "./EntryDetail";
import { WikiPopupCtx, type Point, type WikiPopupApi } from "./WikiPopupContext";

/**
 * VCMI 式弹窗渲染层：
 * - 文本气泡（CRClickPopup）：居中于光标，fitToScreen(10)
 * - 完整窗（CStackWindow / CHeroOverview）：屏幕居中
 * - 两者均为"按住显示、松开关闭"，无遮罩、无关闭按钮
 */

// 完整窗宽度（对齐 VCMI 原窗口尺寸）
const ENTRY_WIDTH: Partial<Record<WikiCategory, number>> = {
  creatures: 460, // CStackWindow
  heroes: 620, // CHeroOverview（600×485）
};

export function WikiPopupProvider({
  contentLang,
  t,
  cacheDir,
  getModCache,
  onNavigate,
  children,
}: {
  contentLang: string;
  t: LangDict;
  cacheDir: string | null;
  getModCache: (modId: string) => Record<string, unknown> | undefined;
  onNavigate?: (target: WikiLinkTarget) => void;
  children: ReactNode;
}) {
  const [text, setText] = useState<{ text: string; x: number; y: number } | null>(null);
  const [entry, setEntry] = useState<WikiEntry | null>(null);
  const bubbleRef = useRef<HTMLDivElement>(null);
  const [bubbleSize, setBubbleSize] = useState({ w: 0, h: 0 });

  const close = useCallback(() => {
    setText(null);
    setEntry(null);
  }, []);

  const showText = useCallback((txt: string, p: Point) => {
    if (!txt) return;
    setText({ text: txt, x: p.x, y: p.y });
  }, []);

  const showEntry = useCallback(
    (modId: string, category: WikiCategory, obj: Record<string, unknown>) => {
      const modCache = getModCache(modId) ?? { modId };
      setEntry(buildEntry(modCache, category, obj));
    },
    [getModCache],
  );

  // 右键松开 / 抬手 → 关闭（对齐 EventDispatcher::dispatchClosePopup）
  useEffect(() => {
    const onMouseUp = (e: MouseEvent) => {
      if (e.button === 2) close();
    };
    const onTouchEnd = () => close();
    window.addEventListener("mouseup", onMouseUp);
    window.addEventListener("touchend", onTouchEnd);
    window.addEventListener("touchcancel", onTouchEnd);
    return () => {
      window.removeEventListener("mouseup", onMouseUp);
      window.removeEventListener("touchend", onTouchEnd);
      window.removeEventListener("touchcancel", onTouchEnd);
    };
  }, [close]);

  // 气泡尺寸测量（用于 fitToScreen）
  useLayoutEffect(() => {
    if (!text || !bubbleRef.current) return;
    const r = bubbleRef.current.getBoundingClientRect();
    if (r.width > 0 && (r.width !== bubbleSize.w || r.height !== bubbleSize.h)) {
      setBubbleSize({ w: r.width, h: r.height });
    }
  }, [text, bubbleSize.w, bubbleSize.h]);

  const api: WikiPopupApi = { showText, showEntry, close };

  // fitToScreen(10)：保证气泡完整落在视口内
  let bubbleLeft = text?.x ?? 0;
  let bubbleTop = text?.y ?? 0;
  if (text && bubbleSize.w > 0) {
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    const halfW = bubbleSize.w / 2;
    const halfH = bubbleSize.h / 2;
    bubbleLeft = Math.min(Math.max(text.x, halfW + 10), Math.max(halfW + 10, vw - halfW - 10));
    bubbleTop = Math.min(Math.max(text.y, halfH + 10), Math.max(halfH + 10, vh - halfH - 10));
  }

  return (
    <WikiPopupCtx.Provider value={api}>
      {children}

      {/* 文本气泡（CRClickPopup）：居中于光标，无遮罩，松开即关 */}
      {text && (
        <div
          ref={bubbleRef}
          data-wiki-popup="bubble"
          // z-[200]：必须高于领域信息页的城镇详情抽屉（z-[150]），否则弹层会被抽屉盖住
          className="pointer-events-none fixed z-[200] max-w-[380px] whitespace-pre-wrap break-words rounded-xl border border-slate-600/70 bg-slate-950/95 px-3 py-2 text-[13px] leading-relaxed text-slate-200 shadow-2xl"
          style={{
            left: bubbleLeft,
            top: bubbleTop,
            transform: "translate(-50%, -50%)",
            visibility: bubbleSize.w > 0 ? "visible" : "hidden",
          }}
        >
          {text.text}
        </div>
      )}

      {/* 完整窗（CStackWindow / CHeroOverview）：屏幕居中，无遮罩，松开即关 */}
      {entry && (
        // z-[200]：同文本气泡，需高于领域信息页的城镇详情抽屉（z-[150]）
        <div className="pointer-events-none fixed inset-0 z-[200] flex items-center justify-center">
          <div
            data-wiki-popup="entry"
            data-wiki-category={entry.category}
            className="pointer-events-auto max-h-[86vh] overflow-y-auto rounded-2xl border border-slate-700/50 bg-slate-950/95 p-4 shadow-2xl"
            style={{ width: ENTRY_WIDTH[entry.category] ?? 560, maxWidth: "92vw" }}
          >
            <EntryDetail
              key={`popup-${entry.modId}:${entry.id}`}
              entry={entry}
              contentLang={contentLang}
              t={t}
              cacheDir={cacheDir}
              modCache={getModCache(entry.modId)}
              onNavigate={(target) => {
                close();
                onNavigate?.(target);
              }}
              compact
            />
          </div>
        </div>
      )}
    </WikiPopupCtx.Provider>
  );
}
