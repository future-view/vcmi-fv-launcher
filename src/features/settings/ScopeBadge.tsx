import { useEffect, useRef, useState } from "react";
import { Check, Copy, Lock, Share2 } from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";

interface ScopeBadgeProps {
  /** `global` = 与所有使用同一配置的领域共享；`realm` = 仅本领域独立 */
  variant: "global" | "realm";
  /** 面板内的作用域说明（domain-agnostic：启动器设置页与领域设置页共用文案） */
  hint: string;
  /** 该作用域对应的配置目录：面板内展示 + 「复制路径」 */
  path?: string;
}

/**
 * 作用域状态（顶部区右段）：全局共享 / 领域独立。
 *
 * v2 重构（settings-panel-design §2.1）：**只读信息不做按钮外观** —— 常态为无边框小字
 * （琥珀 / 靛蓝），hover 提亮 + 下划线；面板内容：作用域说明 + 完整配置目录 +
 * 「复制路径」（复制后就地反馈「已复制 ✓」1.5 秒）。
 *
 * 打开方式：**hover 即展开**（鼠标在「触发器 + 桥接区 + 面板」整体上时保持打开），
 * 点击 = 钉住（触屏 / 键盘可达），外点或 Esc 关闭。
 *
 * 图标避坑：不要用 `Globe`（与右上角「多语言」按钮同形，易误导）；两状态按
 * 「共享 / 私有」语义分别取 `Share2` / `Lock`。
 */
export function ScopeBadge({ variant, hint, path }: ScopeBadgeProps) {
  const { t } = useI18n();
  const isGlobal = variant === "global";
  const [hovered, setHovered] = useState(false);
  const [pinned, setPinned] = useState(false);
  const [copied, setCopied] = useState(false);
  const ref = useRef<HTMLSpanElement>(null);
  const closeTimer = useRef<number | null>(null);
  const open = hovered || pinned;

  // 打开/关闭：hover 进出 + 120ms 延迟关闭（快速划过不闪烁）。
  //
  // ⚠️ 触发器与面板之间的空隙必须用**内边距（pt-2）桥接**，不能用外边距（mt-2）：
  // 外边距留出的 8px 不属于容器布局盒，鼠标经过时会先触发容器的 mouseleave，
  // 面板在指针到达之前就被卸载（2026-09-29 实测缺陷：hover 打开后移不进面板）。
  const clearCloseTimer = () => {
    if (closeTimer.current != null) {
      window.clearTimeout(closeTimer.current);
      closeTimer.current = null;
    }
  };
  const openNow = () => {
    clearCloseTimer();
    setHovered(true);
  };
  const closeSoon = () => {
    clearCloseTimer();
    closeTimer.current = window.setTimeout(() => {
      closeTimer.current = null;
      setHovered(false);
    }, 120);
  };

  // 卸载时清理关闭定时器
  useEffect(() => clearCloseTimer, []);

  // 外点关闭（清钉住态）；Esc 关闭
  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setPinned(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        setPinned(false);
        setHovered(false);
      }
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  // 面板关闭后复位「已复制」反馈
  useEffect(() => {
    if (open || !copied) return;
    setCopied(false);
  }, [open, copied]);

  // 「已复制 ✓」就地反馈，1.5 秒后复原（不弹 toast，不打断）
  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 1500);
    return () => window.clearTimeout(timer);
  }, [copied]);

  const copyPath = async () => {
    if (!path) return;
    try {
      await navigator.clipboard.writeText(path);
      setCopied(true);
    } catch {
      /* 剪贴板不可用：保持原状（路径在面板中仍可手动选择复制） */
    }
  };

  return (
    <span
      className="relative inline-flex"
      ref={ref}
      onMouseEnter={openNow}
      onMouseLeave={closeSoon}
    >
      <button
        type="button"
        onClick={() => setPinned((p) => !p)}
        aria-expanded={open}
        className={cn(
          "inline-flex items-center gap-1 rounded text-xs font-medium underline-offset-4 transition-colors hover:underline",
          isGlobal
            ? "text-amber-200/90 hover:text-amber-100"
            : "text-indigo-200/90 hover:text-indigo-100",
        )}
      >
        {isGlobal ? <Share2 className="h-3 w-3" /> : <Lock className="h-3 w-3" />}
        {isGlobal ? t.settings.scopeGlobal : t.settings.scopeRealm}
      </button>

      {open && (
        // 外层仅作「空隙桥接」：pt-2 让鼠标从触发器移向面板时始终停留在容器子树内
        <div className="absolute right-0 top-full z-[120] pt-2">
          <div
            role="dialog"
            className="w-72 rounded-xl border border-slate-700 bg-slate-900 p-3 text-left shadow-2xl"
          >
            <div
              className={cn(
                "flex items-center gap-1.5 text-xs font-semibold",
                isGlobal ? "text-amber-200" : "text-indigo-200",
              )}
            >
              {isGlobal ? <Share2 className="h-3.5 w-3.5" /> : <Lock className="h-3.5 w-3.5" />}
              {isGlobal ? t.settings.scopeGlobal : t.settings.scopeRealm}
            </div>
            <p className="mt-2 text-xs leading-relaxed text-slate-300">{hint}</p>
            {path && (
              <p className="mt-2 break-all font-mono text-[11px] leading-relaxed text-slate-400">
                {path}
              </p>
            )}
            {path && (
              <button
                type="button"
                onClick={() => void copyPath()}
                className={cn(
                  "mt-3 flex w-full items-center justify-center gap-1.5 rounded-lg border px-2.5 py-1.5 text-xs transition-colors",
                  copied
                    ? "border-emerald-500/40 text-emerald-300"
                    : "border-slate-700 text-slate-300 hover:bg-slate-800",
                )}
              >
                {copied ? <Check className="h-3.5 w-3.5" /> : <Copy className="h-3.5 w-3.5" />}
                {copied ? t.settings.scopeCopied : t.settings.scopeCopyPath}
              </button>
            )}
          </div>
        </div>
      )}
    </span>
  );
}
