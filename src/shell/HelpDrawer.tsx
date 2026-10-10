import { useEffect, useMemo, useRef, useState } from "react";
import { Search } from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import { Button } from "@/shared/ui/button";
import { filterBlocks, loadHelpDoc, type Block, type Inline } from "@/lib/helpDoc";

interface HelpDrawerProps {
  onClose: () => void;
}

/**
 * 帮助抽屉（§7.2）：正文为**随包内置的 Markdown 离线文档**
 * （`src/assets/help/help.<tag>.md`，5 语言 + 回退链，见 `help-drawer-design.zh-CN.md`）。
 *
 * 三层结构：头部（标题 + 副标题）/ 搜索框 + 目录（`##` 生成）/ 正文（受控子集渲染）。
 * `Esc` 或点遮罩关闭；抽屉宽 `min(640px, 92vw)`。
 */
export function HelpDrawer({ onClose }: HelpDrawerProps) {
  const { t, lang } = useI18n();
  const doc = useMemo(() => loadHelpDoc(lang), [lang]);
  const [query, setQuery] = useState("");
  const [activeId, setActiveId] = useState("");
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const searching = query.trim().length > 0;
  const blocks = useMemo(() => filterBlocks(doc, query), [doc, query]);

  // Esc 关闭
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    document.addEventListener("keydown", onKey);
    return () => document.removeEventListener("keydown", onKey);
  }, [onClose]);

  // scroll-spy：滚动经过某个 `##` 时高亮目录项（与设置页左栏同款策略）
  useEffect(() => {
    const root = scrollRef.current;
    if (!root || searching || doc.toc.length === 0) return;
    const observer = new IntersectionObserver(
      (entries) => {
        let best: string | null = null;
        let bestTop = Infinity;
        for (const e of entries) {
          if (e.isIntersecting && e.boundingClientRect.top < bestTop) {
            bestTop = e.boundingClientRect.top;
            best = e.target.id;
          }
        }
        if (best) setActiveId(best);
      },
      { root, rootMargin: "0px 0px -70% 0px", threshold: 0 },
    );
    for (const item of doc.toc) {
      const el = document.getElementById(item.id);
      if (el) observer.observe(el);
    }
    return () => observer.disconnect();
  }, [doc, searching]);

  const scrollTo = (id: string) => {
    setActiveId(id);
    document.getElementById(id)?.scrollIntoView({ behavior: "smooth", block: "start" });
  };

  return (
    <>
      <div className="fixed inset-0 z-[70] bg-black/55 backdrop-blur-sm" onClick={onClose} />
      <aside className="fixed inset-y-0 right-0 z-[71] flex h-screen w-[min(640px,92vw)] flex-col border-l border-slate-700/40 bg-slate-950/95 backdrop-blur-xl shadow-2xl">
        <header className="flex items-center gap-3 border-b border-slate-800 p-5">
          <div className="grid h-11 w-11 flex-none place-items-center rounded-xl bg-slate-800 text-xl">❓</div>
          <div className="min-w-0 flex-1">
            <div className="text-lg font-semibold">{t.help.title}</div>
            <div className="text-xs text-slate-400">{t.help.docHint}</div>
          </div>
          <Button variant="ghost" size="sm" onClick={onClose}>
            {t.common.close} ✕
          </Button>
        </header>

        {/* 搜索 + 目录（搜索态隐藏目录，避免与命中过滤互相干扰） */}
        <div className="border-b border-slate-800 px-5 py-3">
          <div className="relative">
            <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-slate-500" />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={t.help.searchPlaceholder}
              className="w-full rounded-lg border border-slate-700 bg-slate-900 py-1.5 pl-9 pr-3 text-sm text-foreground placeholder:text-slate-500 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500/50"
            />
          </div>
          {!searching && doc.toc.length > 0 && (
            <nav className="mt-3">
              <div className="mb-1.5 text-[11px] font-medium uppercase tracking-wide text-slate-500">
                {t.help.toc}
              </div>
              <div className="flex flex-wrap gap-1.5">
                {doc.toc.map((it) => (
                  <button
                    key={it.id}
                    type="button"
                    onClick={() => scrollTo(it.id)}
                    className={cn(
                      "rounded-md px-2 py-1 text-xs transition-colors",
                      activeId === it.id
                        ? "bg-slate-800 text-foreground"
                        : "text-slate-400 hover:bg-slate-800/60 hover:text-slate-200",
                    )}
                  >
                    {it.text}
                  </button>
                ))}
              </div>
            </nav>
          )}
        </div>

        {/* 正文 */}
        <div ref={scrollRef} className="flex-1 overflow-y-auto px-5 py-4 text-sm text-slate-300">
          {searching && blocks.length === 0 ? (
            <div className="flex h-full items-center justify-center text-muted">
              {t.help.noResult}
            </div>
          ) : (
            blocks.map((b, i) => <BlockView key={`${b.kind}-${i}`} block={b} onAnchor={scrollTo} />)
          )}
        </div>
      </aside>
    </>
  );
}

/** 块级渲染（返回 React 节点，不使用 dangerouslySetInnerHTML） */
function BlockView({ block, onAnchor }: { block: Block; onAnchor: (id: string) => void }) {
  switch (block.kind) {
    case "h2":
      return (
        <h2
          id={block.id}
          className="mb-2 mt-6 scroll-mt-4 border-b border-slate-800 pb-1.5 text-base font-semibold text-slate-100 first:mt-0"
        >
          <InlineView list={block.inlines} onAnchor={onAnchor} />
        </h2>
      );
    case "h3":
      return (
        <h3 className="mb-1.5 mt-4 text-[13.5px] font-semibold text-slate-200">
          <InlineView list={block.inlines} onAnchor={onAnchor} />
        </h3>
      );
    case "p":
      return (
        <p className="mb-2.5 leading-relaxed">
          <InlineView list={block.inlines} onAnchor={onAnchor} />
        </p>
      );
    case "ul":
      return (
        <ul className="mb-3 space-y-1.5">
          {block.items.map((it, i) => (
            <li key={i} className="flex gap-2 leading-relaxed">
              <span className="mt-[8px] h-1 w-1 flex-none rounded-full bg-slate-600" />
              <span className="min-w-0">
                <InlineView list={it} onAnchor={onAnchor} />
              </span>
            </li>
          ))}
        </ul>
      );
    case "ol":
      return (
        <ol className="mb-3 space-y-1.5">
          {block.items.map((it, i) => (
            <li key={i} className="flex gap-2 leading-relaxed">
              <span className="w-4 flex-none text-right text-slate-500">{i + 1}.</span>
              <span className="min-w-0">
                <InlineView list={it} onAnchor={onAnchor} />
              </span>
            </li>
          ))}
        </ol>
      );
    case "quote":
      return (
        <blockquote className="mb-3 border-l-2 border-slate-600 pl-3 leading-relaxed text-slate-400">
          <InlineView list={block.inlines} onAnchor={onAnchor} />
        </blockquote>
      );
    case "hr":
      return <hr className="my-4 border-slate-800" />;
    case "table":
      return (
        <div className="mb-3 overflow-x-auto">
          <table className="w-full border-collapse text-[12.5px]">
            <thead>
              <tr>
                {block.head.map((c, i) => (
                  <th
                    key={i}
                    className="border-b border-slate-700 px-2 py-1.5 text-left font-medium text-slate-200"
                  >
                    {c}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {block.rows.map((r, i) => (
                <tr key={i}>
                  {r.map((c, j) => (
                    <td key={j} className="border-b border-slate-800/70 px-2 py-1.5 text-slate-400">
                      {c}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
  }
  return null;
}

/** 行内渲染：粗体 / 斜体 / 行内代码 / 锚点链接（可跳）/ 外部链接（纯文本，D6） */
function InlineView({ list, onAnchor }: { list: Inline[]; onAnchor: (id: string) => void }) {
  const { t } = useI18n();
  return (
    <>
      {list.map((x, i) => {
        switch (x.kind) {
          case "bold":
            return (
              <strong key={i} className="font-semibold text-slate-100">
                {x.text}
              </strong>
            );
          case "italic":
            return <em key={i}>{x.text}</em>;
          case "code":
            return (
              <code
                key={i}
                className="rounded bg-slate-800 px-1 py-0.5 font-mono text-[12px] text-sky-200"
              >
                {x.text}
              </code>
            );
          case "link":
            if (x.anchor) {
              return (
                <button
                  key={i}
                  type="button"
                  onClick={() => onAnchor(x.href.slice(1))}
                  className="text-sky-400 underline underline-offset-2 transition-colors hover:text-sky-300"
                >
                  {x.text}
                </button>
              );
            }
            return (
              <span key={i} title={t.help.linkExternalHint}>
                {x.text}
                <span className="ml-1 select-all text-[11px] text-slate-500">{x.href}</span>
              </span>
            );
          default:
            return <span key={i}>{x.text}</span>;
        }
      })}
    </>
  );
}
