import { cn } from "@/lib/utils";

/**
 * 锚点侧栏（scroll-spy 导航）。
 *
 * 样式与交互对齐 `features/settings/SettingsSidebar`：
 * 点击 **只做滚动定位**（不切换视图），当前分组由滚动位置反向高亮。
 *
 * 与 SettingsSidebar 的区别：后者绑定 `SettingTag`（settings 域语义 + 图标），
 * 这里是通用件（只有 id / 文案 / 计数），供领域信息页等长页面复用。
 */
export interface AnchorItem {
  id: string;
  label: string;
  /** 分组条目数（0 也显示，便于玩家一眼看到某类没有内容） */
  count?: number;
}

interface AnchorSidebarProps {
  items: AnchorItem[];
  activeId: string;
  onSelect: (id: string) => void;
}

export function AnchorSidebar({ items, activeId, onSelect }: AnchorSidebarProps) {
  return (
    <aside className="flex w-40 shrink-0 flex-col gap-1 overflow-y-auto border-r border-slate-800/70 p-2.5">
      {items.map((it) => {
        const active = activeId === it.id;
        return (
          <button
            key={it.id}
            type="button"
            onClick={() => onSelect(it.id)}
            className={cn(
              "flex items-center gap-2 rounded-lg px-2.5 py-2 text-sm transition-colors",
              active ? "bg-slate-800 text-foreground" : "text-muted hover:bg-slate-800/50",
            )}
          >
            <span className="min-w-0 flex-1 truncate text-left">{it.label}</span>
            {typeof it.count === "number" && (
              <span
                className={cn(
                  "shrink-0 rounded px-1.5 text-[10px]",
                  it.count > 0
                    ? "bg-slate-700/60 text-slate-300"
                    : "bg-slate-800/60 text-slate-500",
                )}
              >
                {it.count}
              </span>
            )}
          </button>
        );
      })}
    </aside>
  );
}
