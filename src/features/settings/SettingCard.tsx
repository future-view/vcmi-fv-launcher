import { cn } from "@/lib/utils";
import type { SettingItem } from "@/lib/settingsSchema";
import { SettingControl } from "./SettingControls";
import { useSettingText, type SettingOption } from "./text";

interface SettingCardProps {
  item: SettingItem;
  value: unknown;
  showDesc: boolean;
  highlight?: boolean;
  onEdit: (v: unknown) => void;
  /** 运行时覆盖的候选（如分辨率从显示器枚举获取） */
  candidatesOverride?: SettingOption[];
  /** 跨键控件：联动键当前值 */
  value2?: unknown;
  /** 跨键控件：一次写回多个键 */
  onEditMulti?: (entries: Record<string, unknown>) => void;
  /** 依赖禁用（如全屏时禁用分辨率、垂直同步时禁用帧率） */
  disabled?: boolean;
}

/** 单个设置项卡片：标签 + 描述 + 控件（文档「选项卡片网格平铺」） */
export function SettingCard({ item, value, showDesc, highlight, onEdit, candidatesOverride, value2, onEditMulti, disabled }: SettingCardProps) {
  const text = useSettingText();
  const label = text.itemLabel(item.id);
  const desc = text.itemDesc(item.id);

  return (
    <div
      className={cn(
        "flex flex-col gap-2 rounded-xl border border-slate-700/50 bg-slate-900/40 p-3.5",
        highlight && "ring-2 ring-yellow-400/60",
        disabled && "opacity-50"
      )}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="text-sm font-medium text-foreground">{label}</div>
          {showDesc && desc && (
            <p className="mt-0.5 text-xs leading-relaxed text-muted">{desc}</p>
          )}
        </div>
      </div>
      <div className="flex items-center">
        <SettingControl item={item} value={value} onChange={onEdit} candidatesOverride={candidatesOverride} value2={value2} onChangeMulti={onEditMulti} disabled={disabled} />
      </div>
    </div>
  );
}
