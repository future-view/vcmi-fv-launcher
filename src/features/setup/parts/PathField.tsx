import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";

/**
 * 只读路径字段 + 选择按钮（向导内通用）。
 *
 * 只负责展示与回调，取词由调用方传入（本组件不碰 i18n，便于复用）。
 */
export function PathField({
  label,
  hint,
  choose,
  value,
  onChoose,
  disabled,
  error,
}: {
  label: string;
  hint: string;
  choose: string;
  value: string;
  onChoose: () => void | Promise<unknown>;
  disabled?: boolean;
  error?: string;
}) {
  return (
    <div className="mb-3">
      <div className="mb-1 flex items-baseline justify-between gap-2">
        <label className="text-sm font-medium text-slate-300">{label}</label>
        <span className="min-w-0 break-words text-xs text-slate-500">{hint}</span>
      </div>
      <div className="flex gap-2">
        <Input
          value={value}
          readOnly
          className="min-w-0 flex-1 bg-slate-950/60 text-slate-200 placeholder:text-slate-600"
        />
        <Button
          type="button"
          variant="outline"
          onClick={onChoose}
          disabled={disabled}
          className="shrink-0 border-slate-700 text-slate-300 hover:bg-slate-800"
        >
          {choose}
        </Button>
      </div>
      {error && <p className="mt-1 text-xs text-red-400">{error}</p>}
    </div>
  );
}
