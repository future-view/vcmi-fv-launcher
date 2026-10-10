/**
 * 帮助文档（内置 Markdown）：装载 / 受控子集解析 / 目录 / 搜索。
 *
 * 正文源：`src/assets/help/help.<tag>.md`（zh-CN / en / ru / pl / de），
 * 设计见 `docs/features/shell-and-navigation.zh-CN.md`。
 *
 * 本模块**纯 TS（无 React）**：解析产出结构化 `Block[]`，渲染由 `shell/HelpDrawer.tsx` 负责。
 * 刻意只支持**受控子集**（标题 / 段落 / 列表 / 粗体 / 斜体 / 行内代码 / 链接 / 引用 /
 * 分隔线 / 表格），不追求 CommonMark —— 避免自研渲染器失控。
 */

import { vcmiLangTag } from "./vcmiLangs";

/* ----------------------------- 类型 ----------------------------- */

/** 内联片段（行内样式） */
export type Inline =
  | { kind: "text"; text: string }
  | { kind: "bold"; text: string }
  | { kind: "italic"; text: string }
  | { kind: "code"; text: string }
  /** `anchor` = 文档内锚点（`#help-N`）；否则为外部链接（渲染为纯文本，见 D6） */
  | { kind: "link"; text: string; href: string; anchor: boolean };

/** 块级元素 */
export type Block =
  | { kind: "h2"; id: string; inlines: Inline[] }
  | { kind: "h3"; inlines: Inline[] }
  | { kind: "p"; inlines: Inline[] }
  | { kind: "ul"; items: Inline[][] }
  | { kind: "ol"; items: Inline[][] }
  | { kind: "quote"; inlines: Inline[] }
  | { kind: "hr" }
  | { kind: "table"; head: string[]; rows: string[][] };

/** 目录项（由 `##` 生成；`id` 用序号而非标题 slug —— 多语言/标点不会导致锚点失配） */
export interface TocItem {
  id: string;
  text: string;
}

export interface HelpDoc {
  /** 文档标题（`# ` 行）；抽屉头部已显示标题，正文不重复渲染 */
  title: string;
  blocks: Block[];
  toc: TocItem[];
}

/* ----------------------------- 语言与装载 ----------------------------- */

/**
 * VCMI 语言标识 → 帮助文档文件名 tag（BCP-47 风格，与 `src/i18n/*.ts` 的文件命名同源，
 * 亦与 i18n 的 `<html lang>` 映射一致；新增语言时两处同步）。
 */
const EN_TAG = vcmiLangTag("english")!; // "en"
const ZH_TAG = vcmiLangTag("chinese")!; // "zh-CN"

/** 回退顺序（tag 级）：当前 → en → zh-CN，去重。未知标识按原样尝试（新增语言零改动）。tag 取自 `vcmiLangTag`（VCMI 语言 id → BCP-47 的唯一事实来源，见 `vcmiLangs.ts`）。 */
function langOrder(lang: string): string[] {
  const tag = vcmiLangTag(lang) ?? lang;
  return [...new Set([tag, EN_TAG, ZH_TAG])];
}

type RawFiles = Record<string, string>;

/** Vite `?raw` 内联的 md（键形如 `../assets/help/help.zh-CN.md`）；惰性调用便于解析函数独立复用 */
function rawFiles(): RawFiles {
  return import.meta.glob("../assets/help/*.md", {
    query: "?raw",
    import: "default",
    eager: true,
  }) as RawFiles;
}

/** 取某语言的 Markdown 原文（按回退链；一份都没有则空串） */
export function loadHelpMarkdown(lang: string): string {
  const files = rawFiles();
  for (const tag of langOrder(lang)) {
    const key = Object.keys(files).find((k) => k.endsWith(`/help.${tag}.md`));
    if (key) return files[key];
  }
  return Object.values(files)[0] ?? "";
}

/** 装载并解析当前界面语言的帮助文档 */
export function loadHelpDoc(lang: string): HelpDoc {
  return parseHelpMarkdown(loadHelpMarkdown(lang));
}

/* ----------------------------- 解析（纯函数） ----------------------------- */

/** 行内解析：**粗体** / *斜体* / `code` / [文字](href) */
export function parseInline(text: string): Inline[] {
  const out: Inline[] = [];
  const re = /(\*\*[^*]+\*\*|\*[^*]+\*|`[^`]+`|\[[^\]]+\]\([^)]+\))/g;
  let last = 0;
  for (const m of text.matchAll(re)) {
    const i = m.index ?? 0;
    if (i > last) out.push({ kind: "text", text: text.slice(last, i) });
    const tok = m[0];
    if (tok.startsWith("**")) {
      out.push({ kind: "bold", text: tok.slice(2, -2) });
    } else if (tok.startsWith("`")) {
      out.push({ kind: "code", text: tok.slice(1, -1) });
    } else if (tok.startsWith("[")) {
      const mm = /^\[([^\]]+)\]\(([^)]+)\)$/.exec(tok);
      const href = (mm?.[2] ?? "").trim();
      out.push({ kind: "link", text: mm?.[1] ?? tok, href, anchor: href.startsWith("#") });
    } else {
      out.push({ kind: "italic", text: tok.slice(1, -1) });
    }
    last = i + tok.length;
  }
  if (last < text.length) out.push({ kind: "text", text: text.slice(last) });
  return out;
}

/** 块的纯文本（搜索与目录用） */
export function blockText(b: Block): string {
  switch (b.kind) {
    case "h2":
    case "h3":
    case "p":
    case "quote":
      return inlineText(b.inlines);
    case "ul":
    case "ol":
      return b.items.map(inlineText).join(" ");
    case "table":
      return [...b.head, ...b.rows.flat()].join(" ");
    case "hr":
      return "";
  }
}

function inlineText(list: Inline[]): string {
  return list.map((x) => x.text).join("");
}

const UL_RE = /^[-*]\s+/;
const OL_RE = /^\d+[.)]\s+/;

/** 解析受控子集 Markdown → `HelpDoc`（块序列 + 目录） */
export function parseHelpMarkdown(md: string): HelpDoc {
  const lines = md.replace(/\r\n?/g, "\n").split("\n");
  const blocks: Block[] = [];
  const toc: TocItem[] = [];
  let title = "";
  let h2Seq = 0;
  let i = 0;

  const isBlockStart = (t: string): boolean =>
    t === "" ||
    t === "---" ||
    t.startsWith("#") ||
    t.startsWith("> ") ||
    t.startsWith("|") ||
    UL_RE.test(t) ||
    OL_RE.test(t);

  while (i < lines.length) {
    const t = lines[i].trim();

    if (t === "") {
      i++;
      continue;
    }

    // `# ` 仅作文档标题（正文不渲染）
    if (t.startsWith("# ")) {
      title = t.slice(2).trim();
      i++;
      continue;
    }

    if (t.startsWith("## ")) {
      h2Seq += 1;
      const text = t.slice(3).trim();
      const id = `help-${h2Seq}`;
      blocks.push({ kind: "h2", id, inlines: parseInline(text) });
      toc.push({ id, text });
      i++;
      continue;
    }

    if (t.startsWith("### ")) {
      blocks.push({ kind: "h3", inlines: parseInline(t.slice(4).trim()) });
      i++;
      continue;
    }

    if (t === "---") {
      blocks.push({ kind: "hr" });
      i++;
      continue;
    }

    // 引用：连续的 `> ` 行合并为一段
    if (t.startsWith("> ")) {
      const buf: string[] = [];
      while (i < lines.length && lines[i].trim().startsWith("> ")) {
        buf.push(lines[i].trim().slice(2).trim());
        i++;
      }
      blocks.push({ kind: "quote", inlines: parseInline(buf.join(" ").trim()) });
      continue;
    }

    // 表格：连续 `|` 行；第二行若为分隔行则丢弃
    if (t.startsWith("|")) {
      const rows: string[][] = [];
      while (i < lines.length && lines[i].trim().startsWith("|")) {
        rows.push(splitTableRow(lines[i].trim()));
        i++;
      }
      const isSep = (r: string[]): boolean =>
        r.length > 0 && r.every((c) => /^:?-{2,}:?$/.test(c.trim()) || c.trim() === "");
      const body = rows.filter((r, idx) => !(idx === 1 && isSep(r)));
      if (body.length > 0) {
        blocks.push({ kind: "table", head: body[0], rows: body.slice(1) });
      }
      continue;
    }

    // 无序列表（允许缩进；不支持嵌套层级 —— 需要层级用 `###` 分节）
    if (UL_RE.test(t)) {
      const items: Inline[][] = [];
      while (i < lines.length && UL_RE.test(lines[i].trim())) {
        items.push(parseInline(lines[i].trim().replace(UL_RE, "").trim()));
        i++;
      }
      blocks.push({ kind: "ul", items });
      continue;
    }

    // 有序列表
    if (OL_RE.test(t)) {
      const items: Inline[][] = [];
      while (i < lines.length && OL_RE.test(lines[i].trim())) {
        items.push(parseInline(lines[i].trim().replace(OL_RE, "").trim()));
        i++;
      }
      blocks.push({ kind: "ol", items });
      continue;
    }

    // 段落：收集到空行 / 下一个块起点
    const buf: string[] = [];
    while (i < lines.length && !isBlockStart(lines[i].trim())) {
      buf.push(lines[i].trim());
      i++;
    }
    if (buf.length > 0) {
      blocks.push({ kind: "p", inlines: parseInline(buf.join(" ")) });
    } else {
      // 兜底：单行无法归类的文本也当段落（避免死循环）
      blocks.push({ kind: "p", inlines: parseInline(t) });
      i++;
    }
  }

  return { title, blocks, toc };
}

function splitTableRow(line: string): string[] {
  return line
    .replace(/^\|/, "")
    .replace(/\|$/, "")
    .split("|")
    .map((c) => c.trim());
}

/* ----------------------------- 搜索 ----------------------------- */

/**
 * 块级搜索：命中块 + 其所属章节标题（`##` 同时带出，保证上下文可见）。
 * 空查询返回全部块。
 */
export function filterBlocks(doc: HelpDoc, query: string): Block[] {
  const q = query.trim().toLowerCase();
  if (!q) return doc.blocks;

  const hit = (b: Block): boolean => blockText(b).toLowerCase().includes(q);
  const out: Block[] = [];
  let header: Block | null = null;
  let headerPushed = false;

  for (const b of doc.blocks) {
    if (b.kind === "h2") {
      header = b;
      headerPushed = false;
      if (hit(b)) {
        out.push(b);
        headerPushed = true;
      }
      continue;
    }
    if (b.kind === "hr") continue;
    if (hit(b)) {
      if (header && !headerPushed) {
        out.push(header);
        headerPushed = true;
      }
      out.push(b);
    }
  }
  return out;
}
