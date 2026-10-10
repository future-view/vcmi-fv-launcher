import * as React from "react";
import { cn } from "@/lib/utils";

export interface SwitchProps {
  checked: boolean;
  onCheckedChange: (v: boolean) => void;
  className?: string;
  disabled?: boolean;
}

export function Switch({ checked, onCheckedChange, className, disabled }: SwitchProps) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      disabled={disabled}
      onClick={() => onCheckedChange(!checked)}
      className={cn(
        "relative inline-flex h-[23px] w-[42px] flex-none items-center rounded-full transition-colors",
        checked
          ? "bg-gradient-to-br from-blue-500 to-indigo-500"
          : "bg-slate-700",
        disabled && "opacity-45 cursor-not-allowed",
        className,
      )}
    >
      <span
        className={cn(
          "inline-block h-[19px] w-[19px] transform rounded-full bg-white shadow transition-transform",
          checked ? "translate-x-[21px]" : "translate-x-[2px]",
        )}
      />
    </button>
  );
}
