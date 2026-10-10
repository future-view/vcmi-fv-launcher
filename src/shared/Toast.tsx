import { useEffect, useRef, useState } from "react";
import { cn } from "@/lib/utils";

/**
 * 轻提示（Toast）：顶部居中、**自动消失**、无手动关闭按钮。
 *
 * 与 `ModView` 的手动关闭 banner 分工：banner 承载需要用户确认阅读的守卫/维护信息，
 * Toast 只承载"操作已完成"类的瞬时反馈（复制成功、粘贴失败等）。
 *
 * 消失机制：到点先播 200ms 淡出，再回调 `onDone` 让父级卸载；
 * `resetKey` 变更即重启计时（连续提示复用同一实例不会提前消失）。
 */
export function Toast({
  text,
  kind = "success",
  resetKey,
  duration,
  onDone,
}: {
  text: string;
  kind?: "success" | "error";
  /** 变更即重启计时（建议传自增 id / 时间戳） */
  resetKey?: string | number;
  /** 自动消失时长（ms）；缺省 success 2400 / error 4000 */
  duration?: number;
  onDone: () => void;
}) {
  const [leaving, setLeaving] = useState(false);
  // 用 ref 持 onDone：父级内联函数每次渲染都是新引用，直接放进依赖会让计时器反复重启、永不消失
  const doneRef = useRef(onDone);
  doneRef.current = onDone;

  const ms = duration ?? (kind === "error" ? 4000 : 2400);

  useEffect(() => {
    setLeaving(false);
    const hide = window.setTimeout(() => setLeaving(true), ms);
    const done = window.setTimeout(() => doneRef.current(), ms + 240);
    return () => {
      window.clearTimeout(hide);
      window.clearTimeout(done);
    };
  }, [resetKey, ms, text]);

  return (
    // 外层只管居中（用 flex，避免与 animate-in/out 的 transform 关键帧互相覆盖）
    <div className="pointer-events-none fixed inset-x-0 top-[68px] z-[160] flex justify-center px-4">
      <div
        role="status"
        className={cn(
          "max-w-[min(92vw,720px)] rounded-lg border px-4 py-2.5 text-xs shadow-xl backdrop-blur-sm",
          kind === "error"
            ? "border-red-500/40 bg-red-950/95 text-red-200 shadow-red-900/40"
            : "border-emerald-500/40 bg-emerald-950/95 text-emerald-200 shadow-emerald-900/40",
          leaving
            ? "animate-out fade-out slide-out-to-top-1 fill-mode-forwards duration-200"
            : "animate-in fade-in slide-in-from-top-2 duration-200",
        )}
      >
        <span className="break-words leading-relaxed">{text}</span>
      </div>
    </div>
  );
}
