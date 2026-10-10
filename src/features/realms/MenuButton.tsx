import { cn } from "@/lib/utils";

/** 菜单项：整行铺满的矩形按钮，图标在上、文字在下居中（H3 主菜单风格，§9.2.2）。
 * hover 时边框发亮（蓝边 + 蓝光辉），背景透明度由 `bgOpacity` 控制，与「文字遮罩强度」一致。 */
export function MenuButton({
  icon,
  label,
  onClick,
  disabled,
  bgOpacity = 80,
}: {
  icon: React.ReactNode;
  label: string;
  onClick: () => void;
  disabled?: boolean;
  bgOpacity?: number;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={cn(
        "group relative flex w-full flex-col items-center gap-1.5 overflow-hidden rounded-xl border border-slate-700/70 px-4 py-4 text-slate-100 transition",
        "hover:border-blue-400/70 hover:shadow-[0_0_16px_rgba(96,165,250,0.25)]",
        disabled && "cursor-not-allowed hover:border-slate-700/70",
      )}
    >
      <span
        className="absolute inset-0 bg-slate-900 transition group-hover:bg-slate-800"
        style={{ opacity: bgOpacity / 100 }}
      />
      <span
        className={cn(
          "relative flex h-6 w-6 items-center justify-center text-slate-300 group-hover:text-blue-300",
          disabled && "opacity-50",
        )}
      >
        {icon}
      </span>
      <span className={cn("relative whitespace-nowrap text-sm font-medium", disabled && "opacity-50")}>
        {label}
      </span>
    </button>
  );
}
