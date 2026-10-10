import { useEffect } from "react";
import { createPortal } from "react-dom";
import { useI18n } from "@/i18n";
import { PreLaunchStepper } from "./PreLaunchStepper";
import { type PreLaunchStep, type StepRuntime } from "./usePreLaunchChecks";

/**
 * 「启动前检测」弹窗的**通用编排外壳**——只负责外壳，不认识任何具体检查项。
 *
 * 设计取舍（见 `docs/features/lang-mods` §10）：
 * - 遮罩 + 标题 + 显式关闭按钮（✕ / 遮罩 / Esc）+ 步骤条 + 卡片槽，与具体检查项解耦。
 * - 步骤条（总览）：节点 = 检测项，让用户一眼理解"本次检测了哪些项、各自结果"；`steps.length < 2` 时去连接线（单项降级）。
 * - 每个检查项各自实现为一个 `PreLaunchStep`（见 `LangPackStepCard` / `makeLangPackStep`），
 *   通过 `steps` 传入并列展示；新增检查项时**不改本组件**。
 * - 检查项卡片容器带 `id="prelaunch-step-<id>"`，供步骤条节点点击滚动（预留兼容）。
 * - 动作回调（onProceed / onGoToMods / onBusyChange）由 `makeLangPackStep` 闭包注入，本外壳不感知。
 *
 * 关闭路径有三条且都显式可发现：右上角 ✕、点击遮罩、Esc。
 * （反馈：部分用户不知道"点遮罩能关"，故必须有可见的关闭按钮。）
 */
interface Props {
  open: boolean;
  /** 弹窗标题（如「启动前检测」） */
  title: string;
  /** 检查项定义列表 */
  steps: PreLaunchStep[];
  /** 各 step 的运行态（scan / resolve / resolving），由 `usePreLaunchChecks` 提供 */
  states: Record<string, StepRuntime>;
  /** 关闭（点 ✕ / 点遮罩 / Esc） */
  onClose: () => void;
  /** 有检查项正在忙碌（下载/启用中）时禁用关闭，避免中断进行中的操作 */
  closeDisabled?: boolean;
}

export function PreLaunchCheckDialog({
  open,
  title,
  steps,
  states,
  onClose,
  closeDisabled = false,
}: Props) {
  const { t } = useI18n();

  // Esc 关闭（忙碌时忽略）
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !closeDisabled) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, closeDisabled, onClose]);

  if (!open) return null;

  // 用 portal 挂到 body：BottomBar 的 <footer> 带 backdrop-blur-xl，会给 position:fixed
  // 后代建立包含块，导致 fixed inset-0 相对 footer（64px 高）而非视口定位 → 弹窗不居中。
  return createPortal(
    <div
      className="fixed inset-0 z-[150] flex items-center justify-center bg-black/60"
      onClick={closeDisabled ? undefined : onClose}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={title}
        className="max-h-[85vh] w-[460px] max-w-[92vw] overflow-auto rounded-2xl border border-slate-700 bg-slate-900 p-5 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        {/* 标题栏：统一「标题 + 显式关闭按钮」 */}
        <div className="mb-3 flex items-start justify-between gap-3">
          <div className="text-base font-semibold text-slate-100">{title}</div>
          <button
            type="button"
            aria-label={t.common.close}
            title={t.common.close}
            disabled={closeDisabled}
            onClick={onClose}
            className="-mr-1 -mt-1 flex h-7 w-7 flex-none items-center justify-center rounded-lg text-slate-400 transition-colors hover:bg-slate-800 hover:text-slate-100 disabled:cursor-not-allowed disabled:opacity-40"
          >
            <svg width="14" height="14" viewBox="0 0 14 14" fill="none" aria-hidden="true">
              <path
                d="M1 1l12 12M13 1L1 13"
                stroke="currentColor"
                strokeWidth="1.6"
                strokeLinecap="round"
              />
            </svg>
          </button>
        </div>

        {/* 步骤条（总览）：本次检测了哪些项、各自结果 */}
        <div className="mb-4 rounded-xl bg-slate-800/40 px-4 py-3">
          <PreLaunchStepper steps={steps} states={states} />
        </div>

        {/* 检查项卡片槽：后续新增检查项直接追加 step 即可 */}
        <div className="space-y-4">
          {steps.map((s) => {
            const rt = states[s.id];
            if (!rt?.scan) return null;
            const StepComp = s.Component;
            return (
              <div key={s.id} id={`prelaunch-step-${s.id}`}>
                <StepComp scan={rt.scan} resolve={rt.resolve} resolving={rt.resolving} />
              </div>
            );
          })}
        </div>
      </div>
    </div>,
    document.body,
  );
}
