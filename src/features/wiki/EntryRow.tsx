import { ChevronRight } from "lucide-react";
import { useI18n } from "@/i18n";
import { cn } from "@/lib/utils";
import { fullImgUrl, type WikiEntry } from "@/lib/modcache";
import { HdIcon } from "@/shared/HdIcon";
import { useWikiPopup, usePopupTriggers } from "@/features/wiki/detail/WikiPopupContext";

/**
 * 中栏条目行：VCMI WikiListItem::showPopupWindow —— 右键弹文本气泡。
 *
 * 模组类目带层级（`depth` / `hasChildren` / `expanded`）：chevron 与主按钮是**兄弟**
 * 而非嵌套（按钮里套按钮是非法 HTML），chevron 只切展开、不改变选中。
 */
export function EntryRow({
  entry,
  cacheDir,
  name,
  sub,
  tip,
  alignment,
  active,
  depth = 0,
  hasChildren = false,
  expanded = false,
  onToggleExpand,
  onClick,
}: {
  entry: WikiEntry;
  cacheDir: string | null;
  name: string;
  sub: string;
  /** 右键气泡文本：描述/传记类字段，回退条目名（见 WikiView 里的取值口径） */
  tip: string;
  alignment: string;
  active: boolean;
  /** 树形缩进层级（0 = 顶层） */
  depth?: number;
  hasChildren?: boolean;
  expanded?: boolean;
  onToggleExpand?: () => void;
  onClick: () => void;
}) {
  const { t } = useI18n();
  const popup = useWikiPopup();
  const triggers = usePopupTriggers();
  const indented = hasChildren || depth > 0;
  return (
    <div
      // 返回后中栏要滚到恢复的行：列表容器据此查询高亮行（见"返回后把中栏滚到恢复的高亮行"）
      data-wiki-active={active ? "true" : undefined}
      className={cn(
        "flex w-full items-stretch border-l-2 transition-colors",
        active ? "border-blue-500 bg-blue-500/15" : "border-transparent hover:bg-slate-800/50",
      )}
    >
      {hasChildren ? (
        <button
          type="button"
          onClick={onToggleExpand}
          aria-expanded={expanded}
          title={expanded ? t.mods.collapse : t.mods.expand}
          aria-label={expanded ? t.mods.collapse : t.mods.expand}
          style={{ marginLeft: depth * 14 + 2 }}
          className="flex w-4 flex-none items-center justify-center text-slate-500 transition-colors hover:text-slate-200"
        >
          <ChevronRight
            className={cn("h-3.5 w-3.5 transition-transform", expanded && "rotate-90")}
          />
        </button>
      ) : depth > 0 ? (
        // 无子模组的子行：占位宽度与 chevron 一致，保证同级图标左沿对齐
        <span className="flex-none" style={{ width: depth * 14 + 18 }} />
      ) : null}
      <button
        type="button"
        onClick={onClick}
        // VCMI WikiListItem::showPopupWindow：条目行右键 = 文本气泡（居中于光标）。
        // 显示 `tip`（描述/传记）而不是副标题 —— 副标题是分类信息（T6 / 狂化人），
        // 作为气泡没有信息量；`tip` 一定非空（回退了条目名），故恒挂触发器。
        {...triggers((p) => popup.showText(tip, p))}
        className={cn(
          "flex min-w-0 flex-1 items-center gap-2 py-2 pr-3 text-left",
          // 只有"顶层且无子节点"的行保持原来的左内边距；其余由 chevron / 占位撑开
          indented ? "pl-0" : "pl-3",
        )}
      >
        <EntryIcon entry={entry} cacheDir={cacheDir} />
        <span className="min-w-0 flex-1">
          <span className="flex items-center justify-between gap-2">
            <span className="min-w-0 flex-1 truncate text-sm text-slate-100">{name}</span>
            {alignment ? (
              <span
                className={cn(
                  "flex-none rounded px-1.5 py-0.5 text-[10px] font-medium",
                  alignment.toLowerCase() === "good"
                    ? "bg-emerald-500/20 text-emerald-300"
                    : alignment.toLowerCase() === "evil"
                      ? "bg-red-500/20 text-red-300"
                      : "bg-slate-600/30 text-slate-400",
                )}
              >
                {t.wiki.alignment[alignment.toLowerCase() as keyof typeof t.wiki.alignment] ?? alignment}
              </span>
            ) : null}
          </span>
          {sub && sub !== name ? (
            <span className="mt-0.5 block truncate text-[11px] text-slate-500">{sub}</span>
          ) : null}
        </span>
      </button>
    </div>
  );
}

export function EntryIcon({ entry, cacheDir }: { entry: WikiEntry; cacheDir: string | null }) {
  if (!entry.image) {
    // 无图条目（如中立建筑）用名称前两字代替 id 前缀，避免整屏相同的 "NE"/"CO"
    const label = (entry.name.chinese || entry.name.english || entry.id).slice(0, 2);
    return (
      <div className="flex h-8 w-8 flex-none items-center justify-center rounded bg-slate-800 text-[10px] text-slate-500">
        {label.toUpperCase()}
      </div>
    );
  }
  return (
    <HdIcon
      src={fullImgUrl(cacheDir, entry.image)}
      alt={entry.id}
      className="h-10 w-10 flex-none rounded object-contain"
      draggable={false}
      loading="lazy"
    />
  );
}
