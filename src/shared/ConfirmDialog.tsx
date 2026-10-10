import { X } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";

interface ConfirmDialogProps {
  open: boolean;
  title: string;
  description?: string;
  confirmText?: string;
  cancelText?: string;
  danger?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
  /** 描述与按钮之间的附加内容（如取消引用的「级联清理依赖」勾选项） */
  children?: ReactNode;
}

export function ConfirmDialog({
  open,
  title,
  description,
  confirmText,
  cancelText,
  danger = false,
  onConfirm,
  onCancel,
  children,
}: ConfirmDialogProps) {
  // 默认按钮文案随启动器界面语言取词（此前硬编码中文「确认/取消」，
  // 非中文界面下漏传 props 的调用点会显示中文）
  const { t } = useI18n();
  const confirmLabel = confirmText ?? t.common.ok;
  const cancelLabel = cancelText ?? t.common.cancel;
  if (!open) return null;
  return (
    <div className="fixed inset-0 z-[100] flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm">
      <div className="w-full max-w-sm rounded-xl border border-slate-700/60 bg-slate-900 p-5 shadow-2xl">
        <div className="mb-3 flex items-start justify-between">
          <h3 className="text-base font-semibold text-slate-100">{title}</h3>
          <button
            onClick={onCancel}
            className="rounded p-1 text-slate-500 hover:bg-slate-800 hover:text-slate-200"
          >
            <X className="h-4 w-4" />
          </button>
        </div>
        {/* whitespace-pre-line：允许调用方用 \n 列出条目（如整合模组重名清单） */}
        {description && (
          <p className={cn("whitespace-pre-line text-sm text-slate-400", children ? "mb-3" : "mb-5")}>{description}</p>
        )}
        {children}
        <div className={cn("flex justify-end gap-2", children && "mt-2")}>
          <button
            onClick={onCancel}
            className="rounded-lg border border-slate-700/60 bg-slate-800/60 px-4 py-2 text-sm font-medium text-slate-300 hover:bg-slate-800"
          >
            {cancelLabel}
          </button>
          <button
            onClick={onConfirm}
            className={cn(
              "rounded-lg px-4 py-2 text-sm font-medium",
              danger
                ? "bg-red-500/90 text-white hover:bg-red-500"
                : "bg-indigo-500/90 text-white hover:bg-indigo-500",
            )}
          >
            {confirmLabel}
          </button>
        </div>
      </div>
    </div>
  );
}
