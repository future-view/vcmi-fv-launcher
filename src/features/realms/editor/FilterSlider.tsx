import { Field } from "./Field";

/** 主界面外观里的单个滤镜滑条（标签 + range + 数值） */
export function FilterSlider({
  label,
  hint,
  min,
  max,
  step = 1,
  value,
  unit = "",
  onChange,
}: {
  label: string;
  hint?: string;
  min: number;
  max: number;
  step?: number;
  value: number;
  unit?: string;
  onChange: (v: number) => void;
}) {
  return (
    <Field label={label}>
      <div className="flex items-center gap-3">
        <input
          type="range"
          min={min}
          max={max}
          step={step}
          value={value}
          onChange={(e) => onChange(Number(e.target.value))}
          className="h-1.5 flex-1 cursor-pointer appearance-none rounded-full bg-slate-700 accent-blue-500"
        />
        <span className="w-12 text-right text-xs tabular-nums text-muted">{value}{unit}</span>
      </div>
      {hint && <p className="mt-1 text-xs text-muted">{hint}</p>}
    </Field>
  );
}
