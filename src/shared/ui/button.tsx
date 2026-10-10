import * as React from "react";
import { cn } from "@/lib/utils";

type ButtonVariant =
  | "default"
  | "primary"
  | "success"
  | "warning"
  | "ghost"
  | "outline"
  | "danger";
type ButtonSize = "sm" | "md" | "lg" | "icon";

export interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
}

const variantClass: Record<ButtonVariant, string> = {
  default: "bg-slate-800/60 hover:bg-slate-700/70 border border-slate-700/40 text-foreground",
  primary:
    "bg-gradient-to-br from-blue-500 to-indigo-500 text-white shadow-lg shadow-blue-600/30 hover:brightness-110 border border-transparent",
  success:
    "bg-gradient-to-br from-green-500 to-green-600 text-white shadow-lg shadow-green-600/30 hover:brightness-110 border border-transparent",
  warning:
    "bg-gradient-to-br from-amber-500 to-orange-600 text-white shadow-lg shadow-amber-600/30 hover:brightness-110 border border-transparent",
  ghost: "bg-transparent hover:bg-slate-800/60 text-muted hover:text-foreground",
  outline: "bg-transparent border border-slate-700/40 text-foreground hover:bg-slate-800/50",
  danger: "bg-red-500/90 text-white hover:bg-red-600 border border-transparent",
};

const sizeClass: Record<ButtonSize, string> = {
  sm: "h-8 px-3 text-xs",
  md: "h-9 px-4 text-sm",
  lg: "h-11 px-6 text-base",
  icon: "h-9 w-9",
};

export const Button = React.forwardRef<HTMLButtonElement, ButtonProps>(
  ({ className, variant = "default", size = "md", disabled, ...props }, ref) => (
    <button
      ref={ref}
      className={cn(
        "inline-flex items-center justify-center gap-2 rounded-lg font-medium transition-all focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500/50 disabled:opacity-45 disabled:cursor-not-allowed disabled:pointer-events-none select-none",
        variantClass[variant],
        sizeClass[size],
        className,
      )}
      disabled={disabled}
      {...props}
    />
  ),
);
Button.displayName = "Button";
