import { useEffect, useRef, useState } from "react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";

/**
 * 轻提示 / 提示条（Toast）：顶部或底部居中、自动消失、可选手动关闭与操作按钮。
 *
 * - 默认承载「操作已完成 / 需要引起注意」的瞬时反馈，也可用于需要用户阅读的守卫信息。
 * - 支持点击内容区一键复制到剪贴板，方便用户核对问题后贴到讨论区/日志。
 * - 提供 `onClose` 时渲染关闭按钮；无论是否手动关闭，到点后都会先淡出再自动卸载。
 * - `resetKey` 变更即重启计时（连续提示复用同一实例不会提前消失）。
 */
export function Toast({
  text,
  kind = "success",
  resetKey,
  duration,
  onDone,
  onClose,
  action,
  copyable = true,
  className,
  placement = "top",
}: {
  text: string;
  kind?: "success" | "error" | "info" | "warning";
  /** 变更即重启计时（建议传自增 id / 时间戳 / 文案本身） */
  resetKey?: string | number;
  /** 自动消失时长（ms）；缺省 success 2400 / error 4000 / info 4000 / warning 5000 */
  duration?: number;
  onDone: () => void;
  /** 手动关闭回调；提供时会在右侧渲染关闭按钮 */
  onClose?: () => void;
  /** 附加操作按钮（如「立即重启」） */
  action?: { label: string; run: () => void };
  /** 是否允许点击内容区复制提示文本；默认 true */
  copyable?: boolean;
  /** 外层定位类名，可覆盖 z-index 等 */
  className?: string;
  /** 显示位置；默认顶部（顶栏下方），bottom 用于领域/设置/底栏等 */
  placement?: "top" | "bottom";
}) {
  const { t } = useI18n();
  const [leaving, setLeaving] = useState(false);
  const [copied, setCopied] = useState(false);
  // 用 ref 持 onDone：父级内联函数每次渲染都是新引用，直接放进依赖会让计时器反复重启、永不消失
  const doneRef = useRef(onDone);
  doneRef.current = onDone;

  const ms =
    duration ??
    (kind === "error" ? 4000 : kind === "info" ? 4000 : kind === "warning" ? 5000 : 2400);

  useEffect(() => {
    setLeaving(false);
    setCopied(false);
    const hide = window.setTimeout(() => setLeaving(true), ms);
    const done = window.setTimeout(() => doneRef.current(), ms + 240);
    return () => {
      window.clearTimeout(hide);
      window.clearTimeout(done);
    };
  }, [resetKey, ms, text]);

  const handleCopy = async () => {
    if (!copyable) return;
    try {
      await navigator.clipboard.writeText(text);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1200);
    } catch {
      // 剪贴板不可用时不阻断提示条显示
    }
  };

  const kindClasses = {
    success:
      "border-emerald-500/40 bg-emerald-950/95 text-emerald-200 shadow-emerald-900/40",
    error: "border-red-500/40 bg-red-950/95 text-red-200 shadow-red-900/40",
    info: "border-blue-500/40 bg-blue-950/95 text-blue-200 shadow-blue-900/40",
    warning:
      "border-amber-500/40 bg-amber-950/95 text-amber-200 shadow-amber-900/40",
  };

  const closeBtnColor =
    kind === "error"
      ? "text-red-300 hover:bg-red-500/20"
      : kind === "info"
        ? "text-blue-300 hover:bg-blue-500/20"
        : kind === "warning"
          ? "text-amber-300 hover:bg-amber-500/20"
          : "text-emerald-300 hover:bg-emerald-500/20";

  const actionBtnColor =
    kind === "warning"
      ? "bg-amber-500/20 hover:bg-amber-500/30 text-amber-100"
      : "bg-white/15 hover:bg-white/25 text-white";

  return (
    // 外层只管居中（用 flex，避免与 animate-in/out 的 transform 关键帧互相覆盖）
    <div
      className={cn(
        "pointer-events-none fixed inset-x-0 z-[160] flex justify-center px-4",
        placement === "bottom" ? "bottom-6" : "top-[68px]",
        className,
      )}
    >
      <div
        role="status"
        aria-live="polite"
        onClick={handleCopy}
        title={copyable ? t.common.copyTip : undefined}
        className={cn(
          "pointer-events-auto flex max-w-[min(92vw,720px)] items-center justify-between gap-3 rounded-lg border px-4 py-2.5 text-xs shadow-xl backdrop-blur-sm",
          kindClasses[kind],
          copyable && "cursor-pointer",
          leaving
            ? "animate-out fade-out slide-out-to-top-1 fill-mode-forwards duration-200"
            : "animate-in fade-in slide-in-from-top-2 duration-200",
        )}
      >
        <span className="min-w-0 flex-1 break-words leading-relaxed">{text}</span>
        {copied && (
          <span className="shrink-0 text-[10px] leading-none opacity-80">{t.common.tipCopied}</span>
        )}
        {action && (
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              action.run();
            }}
            className={cn(
              "shrink-0 rounded-lg px-2 py-1 text-[11px] font-medium transition-colors",
              actionBtnColor,
            )}
          >
            {action.label}
          </button>
        )}
        {onClose && (
          <button
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              onClose();
            }}
            className={cn(
              "flex-none rounded px-1.5 py-0.5 transition-colors",
              closeBtnColor,
            )}
            aria-label={t.common.close}
            title={t.common.close}
          >
            {copied ? "✓" : "✕"}
          </button>
        )}
      </div>
    </div>
  );
}
