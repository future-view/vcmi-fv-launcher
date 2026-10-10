import { useEffect, useState } from "react";
import { X } from "lucide-react";
import { useI18n } from "@/i18n";
import { Input } from "@/shared/ui/input";
import { cn } from "@/lib/utils";

interface PresetSaveDialogProps {
  open: boolean;
  inheritFrom: string;
  inheritLabel: string;
  presets: string[];
  onConfirm: (name: string) => void;
  onCancel: () => void;
}

export function PresetSaveDialog({
  open,
  inheritFrom,
  inheritLabel,
  presets,
  onConfirm,
  onCancel,
}: PresetSaveDialogProps) {
  const { t } = useI18n();
  const [name, setName] = useState("");

  useEffect(() => {
    if (open) setName("");
  }, [open]);

  if (!open) return null;

  const exists = presets.some((n) => n.toLowerCase() === name.trim().toLowerCase());
  const empty = !name.trim();
  const disabled = empty || exists;

  return (
    <div className="fixed inset-0 z-[100] flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm">
      <div className="w-full max-w-sm rounded-xl border border-slate-700/60 bg-slate-900 p-5 shadow-2xl">
        <div className="mb-3 flex items-start justify-between">
          <h3 className="text-base font-semibold text-slate-100">{t.preset.new}</h3>
          <button
            onClick={onCancel}
            className="rounded p-1 text-slate-500 hover:bg-slate-800 hover:text-slate-200"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="mb-4 space-y-3">
          <div>
            <label className="mb-1 block text-xs text-slate-500">{t.preset.newName}</label>
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={t.preset.newName}
              autoFocus
              onKeyDown={(e) => {
                if (e.key === "Enter" && !disabled) onConfirm(name.trim());
              }}
            />
            {exists && (
              <p className="mt-1 text-xs text-red-400">{t.preset.nameExists}</p>
            )}
          </div>
          <div>
            <label className="mb-1 block text-xs text-slate-500">{t.preset.inheritFrom}</label>
            <div className="rounded-lg border border-slate-700/50 bg-slate-800/50 px-3 py-2 text-sm text-slate-300">
              {inheritLabel}
            </div>
            <p className="mt-1 text-[11px] text-slate-500">{t.preset.inheritHint}</p>
          </div>
        </div>

        <div className="flex justify-end gap-2">
          <button
            onClick={onCancel}
            className="rounded-lg border border-slate-700/60 bg-slate-800/60 px-4 py-2 text-sm font-medium text-slate-300 hover:bg-slate-800"
          >
            {t.common.cancel}
          </button>
          <button
            onClick={() => onConfirm(name.trim())}
            disabled={disabled}
            className={cn(
              "rounded-lg px-4 py-2 text-sm font-medium",
              disabled
                ? "cursor-not-allowed bg-slate-700 text-slate-500"
                : "bg-indigo-500/90 text-white hover:bg-indigo-500",
            )}
          >
            {t.common.ok}
          </button>
        </div>
      </div>
    </div>
  );
}
