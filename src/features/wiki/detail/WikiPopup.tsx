import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import type { LangDict } from "@/i18n/types";
import { buildEntry, loadModCache, type WikiCategory, type WikiEntry, type WikiLinkTarget } from "@/lib/modcache";
import { EntryDetail } from "./EntryDetail";
import { WikiPopupCtx, type Point, type WikiPopupApi } from "./WikiPopupContext";

/**
 * VCMI 式弹窗渲染层：
 * - 文本气泡（CRClickPopup）：居中于光标，fitToScreen(10)
 * - 完整窗（CStackWindow / CHeroOverview）：屏幕居中
 * - 两者均为"按住显示、松开关闭"，无遮罩、无关闭按钮
 */

// 完整窗宽度。creatures 刻意比 VCMI 的 CStackWindow（430px）宽：弹窗内容是**三列**
// （图+经济 | 统计+招募建筑 | 能力），能力数是最大的高度变数（原版 0–15 项），
// 立为第三列并限高 5 行后总高被压成常量（≤477px）—— 这个窗口是"按住右键才存在"的，
// 用户没法在按住右键的同时拖滚动条，所以用宽度换高度，滚动只发生在能力列内部
// （与 VCMI BonusesSection 的"固定高 + CListBox 内滚"同语义）。
const ENTRY_WIDTH: Partial<Record<WikiCategory, number>> = {
  creatures: 880,
  heroes: 620, // CHeroOverview（600×485）
};

export function WikiPopupProvider({
  contentLang,
  t,
  cacheDir,
  getModCache,
  getEntry,
  onNavigate,
  children,
}: {
  contentLang: string;
  t: LangDict;
  cacheDir: string | null;
  getModCache: (modId: string) => Record<string, unknown> | undefined;
  /** 跨模组按 (类目, id) 查条目（升级链等跨 mod 引用；缺省则只查条目所属模组） */
  getEntry?: (category: WikiCategory, id: string) => WikiEntry | undefined;
  onNavigate?: (target: WikiLinkTarget) => void;
  children: ReactNode;
}) {
  const [text, setText] = useState<{ text: string; x: number; y: number } | null>(null);
  const [entry, setEntry] = useState<WikiEntry | null>(null);
  const [entryModCache, setEntryModCache] = useState<Record<string, unknown> | undefined>();
  const bubbleRef = useRef<HTMLDivElement>(null);
  const [bubbleSize, setBubbleSize] = useState({ w: 0, h: 0 });

  const close = useCallback(() => {
    setText(null);
    setEntry(null);
    setEntryModCache(undefined);
  }, []);

  const showText = useCallback((txt: string, p: Point) => {
    if (!txt) return;
    setText({ text: txt, x: p.x, y: p.y });
  }, []);

  const showEntry = useCallback(
    (modId: string, category: WikiCategory, obj: Record<string, unknown>) => {
      // LRU 命中就直接用；未命中先渲染（无 modCache），由下方 effect 按需补齐。
      const cached = getModCache(modId);
      setEntryModCache(cached);
      setEntry(buildEntry(cached ?? { modId }, category, obj));
    },
    [getModCache],
  );

  /**
   * 弹窗条目的 modCache **按需加载**（根因：`cacheRef` 是 24 条 LRU，而一次装载要遍历
   * 572 个 mod，装载完只剩最后 24 个在里面 → 弹窗拿到 `undefined` → `sharedIcons` /
   * `creatures` / `resources` 全空，表现为"弹窗里所有需要查表的图标都不显示"：
   * 英雄四维属性 / 初始部队 / 战争机器 / 魔法书、生物升级链的名称与图、招募花费资源图标）。
   *
   * 与宽态详情页同一路径（`WikiView.tsx` 的 `loadModCache(selected.modId)`），
   * 本地读一份 cache.json（MB 级），按住右键的时长足够覆盖。
   */
  useEffect(() => {
    if (!entry || entryModCache) return;
    let cancelled = false;
    void loadModCache(entry.modId).then((c) => {
      if (!cancelled && c) setEntryModCache(c);
    });
    return () => {
      cancelled = true;
    };
  }, [entry, entryModCache]);

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
              modCache={entryModCache}
              getEntry={getEntry}
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
