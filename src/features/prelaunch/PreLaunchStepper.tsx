import { Fragment } from "react";
import { type PreLaunchStep, type StepRuntime } from "./usePreLaunchChecks";

/**
 * 「启动前检测」步骤条——节点 = 检测项，让用户一眼理解"本次检测了哪些项、各自结果"。
 *
 * - 节点主状态 = `scan` 的 severity；`scanning`/`resolving` 显示天蓝旋转环（不改主状态色）。
 * - **单项降级**：`steps.length >= 2` 才画连接线；单项保留节点 + 标签、无连线（避免孤圈 + 长线）。
 * - **预留滚动兼容**：点击节点 → `scrollIntoView` 聚焦对应卡片（卡片容器 id=`prelaunch-step-<id>`）。
 *   即便当前项少未必需交互，props/行为现在就就绪，项多时无需改结构。
 */

type NodeStatus = "pending" | "scanning" | "ok" | "warn" | "error";

function statusOf(rt?: StepRuntime): NodeStatus {
  if (!rt) return "pending";
  if (rt.scanning) return "scanning";
  if (!rt.scan) return "pending";
  return rt.scan.severity;
}

const ringClass: Record<NodeStatus, string> = {
  pending: "border-slate-600 text-slate-500",
  scanning: "border-sky-400 text-sky-300",
  ok: "border-green-500 text-green-400",
  warn: "border-amber-500 text-amber-400",
  error: "border-red-500 text-red-400",
};

const lineClass: Record<NodeStatus, string> = {
  pending: "bg-slate-700",
  scanning: "bg-slate-700",
  ok: "bg-green-500/50",
  warn: "bg-amber-500/50",
  error: "bg-red-500/50",
};

function NodeIcon({ status, index }: { status: NodeStatus; index: number }) {
  if (status === "scanning") {
    return (
      <svg className="h-3.5 w-3.5 animate-spin" viewBox="0 0 24 24" fill="none" aria-hidden="true">
        <circle cx="12" cy="12" r="9" stroke="currentColor" strokeWidth="3" opacity="0.3" />
        <path
          d="M21 12a9 9 0 0 0-9-9"
          stroke="currentColor"
          strokeWidth="3"
          strokeLinecap="round"
        />
      </svg>
    );
  }
  if (status === "ok") return <span className="text-sm font-bold">✓</span>;
  if (status === "warn") return <span className="text-sm font-bold">⚠</span>;
  if (status === "error") return <span className="text-sm font-bold">✕</span>;
  return <span className="text-xs font-semibold">{index + 1}</span>;
}

interface Props {
  steps: PreLaunchStep[];
  states: Record<string, StepRuntime>;
}

export function PreLaunchStepper({ steps, states }: Props) {
  const multi = steps.length >= 2;
  return (
    <div className="flex items-start">
      {steps.map((s, i) => {
        const rt = states[s.id];
        const st = statusOf(rt);
        return (
          <Fragment key={s.id}>
            <div className="flex flex-col items-center gap-1.5">
              <button
                type="button"
                // 预留滚动兼容：点击节点滚动到对应卡片
                onClick={() =>
                  document
                    .getElementById(`prelaunch-step-${s.id}`)
                    ?.scrollIntoView({ behavior: "smooth", block: "center" })
                }
                aria-label={s.label}
                className={`flex h-7 w-7 flex-none items-center justify-center rounded-full border-2 bg-slate-900 transition-colors ${ringClass[st]} ${
                  st === "scanning" || rt?.resolving ? "animate-pulse" : ""
                }`}
              >
                <NodeIcon status={st} index={i} />
              </button>
              <span
                className={`whitespace-nowrap text-[11px] ${
                  st === "scanning" ? "font-semibold text-sky-300" : "text-slate-400"
                }`}
              >
                {s.label}
              </span>
            </div>
            {multi && i < steps.length - 1 && (
              <div
                className={`mx-1 mt-3 h-0.5 flex-1 rounded ${lineClass[st]}`}
                aria-hidden="true"
              />
            )}
          </Fragment>
        );
      })}
    </div>
  );
}
