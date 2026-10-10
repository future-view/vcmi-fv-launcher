import {
  Cog,
  MonitorPlay,
  Map,
  Swords,
  Wrench,
  Bot,
  Mouse,
  Rocket,
  Package,
  Network,
  Users,
  ScrollText,
  SlidersHorizontal,
  type LucideIcon,
} from "lucide-react";
import { cn } from "@/lib/utils";
import type { SettingTag } from "@/lib/settingsSchema";
import { useSettingText } from "./text";

const ICONS: Record<string, LucideIcon> = {
  Cog,
  MonitorPlay,
  Map,
  Swords,
  Wrench,
  Bot,
  Mouse,
  Rocket,
  Package,
  Network,
  Users,
  ScrollText,
  SlidersHorizontal,
};

interface SettingsSidebarProps {
  tags: SettingTag[];
  activeTag: SettingTag["id"];
  onSelect: (id: SettingTag["id"]) => void;
  /** 搜索模式下，每个标签的命中数量（用于徽标） */
  matchCounts?: Record<string, number>;
  searching: boolean;
}

/** 左侧 12 标签导航栏（设计文档指定图标） */
export function SettingsSidebar({
  tags,
  activeTag,
  onSelect,
  matchCounts,
  searching,
}: SettingsSidebarProps) {
  const text = useSettingText();
  return (
    <aside className="flex w-44 shrink-0 flex-col gap-1 overflow-y-auto border-r border-slate-800 p-2.5">
      {tags.map((tag) => {
        const Icon = ICONS[tag.icon] ?? Cog;
        const label = text.tagLabel(tag.id);
        const count = matchCounts?.[tag.id] ?? 0;
        const active = activeTag === tag.id;
        const hasMatch = searching && count > 0;
        return (
          <button
            key={tag.id}
            type="button"
            onClick={() => onSelect(tag.id)}
            className={cn(
              "flex items-center gap-2 rounded-lg px-2.5 py-2 text-sm transition-colors",
              active
                ? "bg-slate-800 text-foreground"
                : "text-muted hover:bg-slate-800/50"
            )}
          >
            <Icon className="h-4 w-4 shrink-0" />
            <span className="flex-1 truncate text-left">{label}</span>
            {hasMatch && (
              <span className="shrink-0 rounded bg-blue-500/20 px-1.5 text-[10px] text-blue-300">
                {count}
              </span>
            )}
          </button>
        );
      })}
    </aside>
  );
}
