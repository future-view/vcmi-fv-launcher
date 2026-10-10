import { type ReactNode } from "react";
import { cn } from "@/lib/utils";

/**
 * 极简 Markdown 渲染器（**输出到 React 元素，全程不碰 innerHTML**）。
 *
 * ## 为什么自写而不引 marked / markdown-it
 *
 * 1. 输入已经过 `lib/html.ts::htmlToMarkdown()` 的**白名单重铸**，只会产生 6 种语法
 *    （标题 / 粗体 / 斜体 / 列表 / 链接 / 段落），没有表格、脚注、内嵌 HTML 等复杂语法；
 * 2. 输出为 React 元素 → 天然无法注入脚本，等价于 VCMI 的 `MarkdownNoHTML` 渲染模式；
 * 3. 项目当前无 Markdown 依赖，为一个摘要/详情展示场景引入运行时 + 消毒器不划算。
 *
 * 支持的语法：`# ~ ######` 标题、`- ` 无序列表、`1. ` 有序列表、`> ` 引用、
 * `---` 分隔线、`| a | b |` 表格、**粗体**、*斜体*、`` `代码` ``、`[文本](链接)`，
 * 其余按段落处理。
 *
 * 表格来源：模组预设报告（`lib/presetReport.ts`）与作者手写的 Markdown 描述。
 * mod 描述的 HTML 路径（`htmlToMarkdown`）会剥掉表格结构，因此表格解析不影响既有内容。
 */

/** 允许的链接协议（与 `lib/html.ts::safeHref` 同口径，阻断 javascript: / data:） */
const SAFE_PROTOCOL = /^(https?:\/\/|mailto:|\/|\.\/|\.\.\/)/i;

interface MarkdownProps {
  /** Markdown 源文本（建议先经 `htmlToMarkdown` 重铸） */
  source: string;
  className?: string;
}

export function Markdown({ source, className }: MarkdownProps) {
  const blocks = parseBlocks(source ?? "");
  if (blocks.length === 0) return null;
  return (
    <div className={cn("space-y-2 text-[13px] leading-relaxed text-slate-300", className)}>
      {blocks.map((b, i) => (
        <BlockNode key={i} block={b} />
      ))}
    </div>
  );
}

/* ----------------------------- 块级解析 ----------------------------- */

type Block =
  | { kind: "heading"; level: number; text: string }
  | { kind: "list"; ordered: boolean; items: string[] }
  | { kind: "quote"; text: string }
  | { kind: "hr" }
  | { kind: "table"; headers: string[]; rows: string[][]; width: number }
  | { kind: "para"; text: string };

/** 表格分隔行：只由 `| - : 空白` 组成，且至少含一段 `--`（如 `| --- | :--: |`） */
function isTableSep(line: string): boolean {
  const s = line.trim();
  if (!s.includes("|")) return false;
  if (s.replace(/[|\-:\s]/g, "") !== "") return false;
  return /-{2,}/.test(s);
}

/** 拆分表格行：去首尾 `|`，按 `|` 切分并 trim（报告侧已把内容里的 `|` 换成全角） */
function splitRow(line: string): string[] {
  let s = line.trim();
  if (s.startsWith("|")) s = s.slice(1);
  if (s.endsWith("|")) s = s.slice(0, -1);
  return s.split("|").map((c) => c.trim());
}

function parseBlocks(src: string): Block[] {
  const lines = src.replace(/\r\n?/g, "\n").split("\n");
  const out: Block[] = [];
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];

    // 空行
    if (!line.trim()) {
      i++;
      continue;
    }

    // 分隔线（`---` / `***` / `- - -`）：注意字符类里不能写 `\1` 反向引用
    if (/^\s{0,3}([-*_])(?:[ \t]*\1){2,}[ \t]*$/.test(line)) {
      out.push({ kind: "hr" });
      i++;
      continue;
    }

    // 表格：表头行 + 分隔行（`| --- | --- |`），数据行直到空行或非表格行
    if (line.includes("|") && i + 1 < lines.length && isTableSep(lines[i + 1])) {
      const headers = splitRow(line);
      i += 2;
      const rows: string[][] = [];
      while (i < lines.length && lines[i].trim() && lines[i].includes("|")) {
        rows.push(splitRow(lines[i]));
        i++;
      }
      const width = Math.max(headers.length, ...rows.map((r) => r.length));
      out.push({ kind: "table", headers, rows, width });
      continue;
    }

    // 标题
    const h = line.match(/^\s{0,3}(#{1,6})\s+(.*)$/);
    if (h) {
      out.push({ kind: "heading", level: h[1].length, text: h[2].trim() });
      i++;
      continue;
    }

    // 引用（合并连续行）
    if (/^\s{0,3}>\s?/.test(line)) {
      const buf: string[] = [];
      while (i < lines.length && /^\s{0,3}>\s?/.test(lines[i])) {
        buf.push(lines[i].replace(/^\s{0,3}>\s?/, ""));
        i++;
      }
      out.push({ kind: "quote", text: buf.join(" ").trim() });
      continue;
    }

    // 列表（合并连续项；空行或下一个非列表项结束）
    const ul = line.match(/^\s{0,3}[-*+]\s+(.*)$/);
    const ol = line.match(/^\s{0,3}\d+[.)]\s+(.*)$/);
    if (ul || ol) {
      const ordered = !!ol && !ul;
      const items: string[] = [];
      while (i < lines.length) {
        const m = ordered
          ? lines[i].match(/^\s{0,3}\d+[.)]\s+(.*)$/)
          : lines[i].match(/^\s{0,3}[-*+]\s+(.*)$/);
        if (!m) break;
        items.push(m[1].trim());
        i++;
      }
      out.push({ kind: "list", ordered, items });
      continue;
    }

    // 段落（合并到空行）
    const buf: string[] = [];
    while (
      i < lines.length &&
      lines[i].trim() &&
      !/^\s{0,3}(#{1,6})\s+/.test(lines[i]) &&
      !/^\s{0,3}[-*+]\s+/.test(lines[i]) &&
      !/^\s{0,3}\d+[.)]\s+/.test(lines[i]) &&
      !/^\s{0,3}>\s?/.test(lines[i])
    ) {
      buf.push(lines[i].trim());
      i++;
    }
    if (buf.length) out.push({ kind: "para", text: buf.join(" ") });
  }

  return out;
}

function BlockNode({ block }: { block: Block }) {
  switch (block.kind) {
    case "heading": {
      const size = block.level <= 1
        ? "text-[15px] font-semibold text-slate-100"
        : block.level === 2
          ? "text-[14px] font-semibold text-slate-100"
          : "text-[13px] font-semibold text-slate-200";
      return <div className={size}>{renderInline(block.text)}</div>;
    }
    case "hr":
      return <hr className="border-slate-700/60" />;
    case "table":
      return (
        <div className="overflow-x-auto">
          {/* 不设 w-full：表格按内容自适应（两列统计表不会被拉得很空），超宽由外层横向滚动兜底 */}
          <table className="border-collapse text-[12.5px]">
            <thead>
              <tr>
                {Array.from({ length: block.width }, (_, ci) => (
                  <th
                    key={ci}
                    className="whitespace-nowrap border-b border-slate-700/60 px-3 py-1.5 text-left font-medium text-slate-400"
                  >
                    {renderInline(block.headers[ci] ?? "")}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {block.rows.map((r, ri) => (
                <tr key={ri} className="border-b border-slate-800/60 last:border-0">
                  {Array.from({ length: block.width }, (_, ci) => (
                    <td key={ci} className="px-3 py-1.5 align-top">
                      {renderInline(r[ci] ?? "")}
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      );
    case "quote":
      return (
        <blockquote className="border-l-2 border-slate-600 pl-2.5 text-slate-400">
          {renderInline(block.text)}
        </blockquote>
      );
    case "list":
      return block.ordered ? (
        <ol className="list-decimal space-y-0.5 pl-5">
          {block.items.map((it, i) => (
            <li key={i}>{renderInline(it)}</li>
          ))}
        </ol>
      ) : (
        <ul className="list-disc space-y-0.5 pl-5">
          {block.items.map((it, i) => (
            <li key={i}>{renderInline(it)}</li>
          ))}
        </ul>
      );
    case "para":
    default:
      return <p>{renderInline(block.text)}</p>;
  }
}

/* ----------------------------- 行内解析 ----------------------------- */

// 链接 → 双星粗体 → 下划线粗体 → 单星斜体 → 下划线斜体 → 反引号代码
const INLINE_RE =
  /(\[[^\]\n]*\]\([^)\s]*\))|(\*\*[^*\n]+\*\*)|(__[^_\n]+__)|(\*[^*\n]+\*)|(_[^_\n]+_)|(`[^`\n]+`)/g;

/** 行内 Markdown → React 元素（供列表项等场景复用，与 `Markdown` 组件口径一致） */
export function renderInline(text: string): ReactNode[] {
  const out: ReactNode[] = [];
  let last = 0;
  let key = 0;
  INLINE_RE.lastIndex = 0;

  for (let m = INLINE_RE.exec(text); m; m = INLINE_RE.exec(text)) {
    if (m.index > last) out.push(text.slice(last, m.index));
    const tok = m[0];

    if (tok.startsWith("[")) {
      const lm = tok.match(/^\[([^\]]*)\]\(([^)\s]*)\)$/);
      const label = lm?.[1] ?? "";
      const href = lm?.[2] ?? "";
      if (href && SAFE_PROTOCOL.test(href)) {
        out.push(
          <a
            key={key++}
            href={href}
            target="_blank"
            rel="noreferrer noopener"
            className="text-blue-400 underline decoration-blue-400/50 underline-offset-2 hover:text-blue-300"
          >
            {label || href}
          </a>,
        );
      } else {
        out.push(label || href);
      }
    } else if (tok.startsWith("**") || tok.startsWith("__")) {
      out.push(<strong key={key++}>{tok.slice(2, -2)}</strong>);
    } else if (tok.startsWith("`")) {
      out.push(
        <code key={key++} className="rounded bg-slate-800 px-1 py-0.5 text-[12px] text-slate-200">
          {tok.slice(1, -1)}
        </code>,
      );
    } else {
      out.push(<em key={key++}>{tok.slice(1, -1)}</em>);
    }

    last = m.index + tok.length;
  }

  if (last < text.length) out.push(text.slice(last));
  return out;
}
