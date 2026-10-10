/**
 * VCMI 富文本归一：HTML → Markdown / 纯文本。
 *
 * ## 为什么要"重铸"而不是"渲染"或"消毒"
 *
 * VCMI 官方启动器的处理流水线（`references/vcmi`）是：
 *   1. `launcher/modManager/modstate.cpp`
 *      `if (Qt::mightBeRichText(d)) { doc.setHtml(d); d = doc.toMarkdown(); }` —— HTML 先转成 Markdown；
 *   2. `launcher/modManager/cmodlistview_moc.cpp`
 *      `modInfoBrowser->document()->setMarkdown(text, MarkdownFeature::MarkdownNoHTML)`
 *      —— 再以**禁用内联 HTML** 的模式渲染 Markdown。
 *
 * 即官方口径是「HTML → Markdown 重铸 → 无 HTML 渲染」：作者写的 `<h1 style=...>` 在官方
 * 启动器里显示为正常标题，而不是标签串。mod.json 的描述由模组作者自由填写（实测含
 * `<h1 style='color:#002366'>` 之类），因此**绝不能**走 `dangerouslySetInnerHTML`。
 *
 * 本模块采用与官方同思路但更严格的做法：**白名单重铸**——只保留语义（标题/强调/列表/链接/段落），
 * 标签属性（style / on* / 协议可疑的 href）一律丢弃，输出的是纯 Markdown 文本，
 * 再由 `shared/Markdown.tsx` 渲染为 React 元素（全程不碰 innerHTML）。
 */

/** 需要"另起一段"的块级标签 */
const BLOCK_TAGS = new Set([
  "p", "div", "section", "article", "header", "footer", "blockquote", "pre",
  "table", "thead", "tbody", "tr", "ul", "ol", "h1", "h2", "h3", "h4", "h5", "h6",
]);

/** 标题级别：h1 → 1 */
const HEADING_LEVEL: Record<string, number> = {
  h1: 1, h2: 2, h3: 3, h4: 4, h5: 5, h6: 6,
};

/** 命名的 HTML 实体（只列 mod 描述里常见的一批，未知的原样保留） */
const ENTITIES: Record<string, string> = {
  amp: "&", lt: "<", gt: ">", quot: '"', apos: "'", nbsp: " ",
  ldquo: "“", rdquo: "”", lsquo: "‘", rsquo: "’",
  hellip: "…", mdash: "—", ndash: "–", middot: "·",
  trade: "™", reg: "®", copy: "©",
};

/** 解码 HTML 实体（命名 + 十进制/十六进制数字引用） */
function decodeEntities(s: string): string {
  return s.replace(/&(#x?[0-9a-f]+|[a-z]+);/gi, (m, g: string) => {
    if (g[0] === "#") {
      const isHex = g[1] === "x" || g[1] === "X";
      const code = parseInt(isHex ? g.slice(2) : g.slice(1), isHex ? 16 : 10);
      return Number.isFinite(code) && code > 0 ? String.fromCodePoint(code) : m;
    }
    return ENTITIES[g.toLowerCase()] ?? m;
  });
}

/** 允许的链接协议（阻断 javascript: / data: / vbscript: 等） */
const SAFE_PROTOCOL = /^(https?:\/\/|mailto:|\/|\.\/|\.\.\/)/i;

/** 从 `<a ...>` 标签里取 href；协议不合法或取不到则返回 null */
function safeHref(tag: string): string | null {
  const m = tag.match(/href\s*=\s*("([^"]*)"|'([^']*)'|([^\s>]+))/i);
  const raw = (m?.[2] ?? m?.[3] ?? m?.[4] ?? "").trim();
  if (!raw) return null;
  return SAFE_PROTOCOL.test(raw) ? raw : null;
}

/** 标签名（去掉 `<`、`/`、属性） */
function tagName(tag: string): string {
  const m = tag.match(/^<\s*\/?\s*([a-zA-Z][a-zA-Z0-9-]*)/);
  return m ? m[1].toLowerCase() : "";
}

/**
 * 从 `<` 起找到**标签真正的结束 `>`**（跳过属性值里的 `>`）。
 *
 * 简单 `indexOf(">")` 会在 `href="data:text/html,<b>x</b>"` 这类属性上提前截断，
 * 把属性值当成正文吐出来——那正是 XSS payload 常见的藏身之处。
 */
function findTagEnd(src: string, lt: number): number {
  let quote: string | null = null;
  for (let k = lt + 1; k < src.length; k++) {
    const c = src[k];
    if (quote) {
      if (c === quote) quote = null;
      continue;
    }
    if (c === '"' || c === "'") {
      quote = c;
      continue;
    }
    if (c === ">") return k;
  }
  return -1;
}

/**
 * 判断文本是否"像富文本（HTML）"。
 *
 * 对应 Qt 的 `mightBeRichText()`：只有含典型 HTML 标签时才走 HTML → Markdown 重铸，
 * 否则（作者直接用 Markdown 写的描述）原样保留，避免把 `# 标题` 之类误伤。
 */
export function looksLikeHtml(src: string): boolean {
  return /<\s*(?:html|body|p|div|br|span|h[1-6]|strong|b|em|i|a|img|ul|ol|li|table|tr|td|font|center|blockquote|pre)\b[^>]*>/i.test(
    src,
  );
}

/**
 * HTML → Markdown（白名单重铸）。
 *
 * 保留：标题 / 粗体 / 斜体 / 无序与有序列表 / 链接 / 段落。
 * 丢弃：所有标签属性（style / class / on* / 字体色等）、图片、表格结构、脚本与样式块。
 */
export function htmlToMarkdown(html: string): string {
  if (!html) return "";
  let src = html;
  // 整块丢弃（含内容）：脚本 / 样式 / 注释
  src = src
    .replace(/<!--[\s\S]*?-->/g, "")
    .replace(/<\s*script\b[\s\S]*?<\s*\/\s*script\s*>/gi, "")
    .replace(/<\s*style\b[\s\S]*?<\s*\/\s*style\s*>/gi, "");

  const out: string[] = [];
  /** 当前是否处于行首（决定要不要补换行） */
  let atLineStart = true;
  /** 开放中的链接（`<a href>` → 文本 → `</a>`）：非 null 时输出全部累进 linkText */
  let linkHref: string | null = null;
  let linkText = "";
  /** 链接是否"有效开放"（协议合法才有 href，否则退化成普通文本流） */
  let inLink = false;

  const push = (s: string) => {
    if (!s) return;
    if (inLink) {
      linkText += s;
      return;
    }
    out.push(s);
    atLineStart = false;
  };
  const newLine = () => {
    if (inLink) {
      linkText += " ";
      return;
    }
    if (!atLineStart) {
      out.push("\n");
      atLineStart = true;
    }
  };
  /** 段落分隔：保证输出以**空行**结尾（相邻段落之间恰好一个空行） */
  const newParagraph = () => {
    if (inLink) {
      linkText += " ";
      return;
    }
    if (out.length === 0) {
      atLineStart = true;
      return;
    }
    let nl = 0;
    for (let k = out.length - 1; k >= 0 && out[k] === "\n"; k--) nl++;
    for (let k = nl; k < 2; k++) out.push("\n");
    atLineStart = true;
  };

  let i = 0;
  while (i < src.length) {
    const lt = src.indexOf("<", i);
    if (lt === -1) {
      push(decodeEntities(src.slice(i)).replace(/\s+/g, " "));
      break;
    }
    if (lt > i) {
      const text = decodeEntities(src.slice(i, lt));
      // 折叠空白（HTML 里换行/缩进无语义）
      push(text.replace(/\s+/g, " "));
    }
    // 属性值里可能含 `>`（如 `href="data:text/html,<b>x</b>"`），必须跳过引号段找真正的标签尾
    const gt = findTagEnd(src, lt);
    if (gt === -1) break; // 残缺标签，丢弃尾巴
    const rawTag = src.slice(lt, gt + 1);
    const name = tagName(rawTag);
    const isClose = /^<\s*\//.test(rawTag);
    i = gt + 1;

    if (!name) continue;

    // —— 链接 ——
    if (name === "a") {
      if (isClose) {
        const text = linkText.trim();
        const href = linkHref;
        inLink = false;
        linkHref = null;
        linkText = "";
        push(href && text ? `[${text}](${href})` : text);
      } else {
        // 嵌套 <a>：先把外层文本落盘，避免丢内容
        if (inLink) {
          const text = linkText.trim();
          const href = linkHref;
          inLink = false;
          linkHref = null;
          linkText = "";
          push(href && text ? `[${text}](${href})` : text);
        }
        const href = safeHref(rawTag);
        linkText = "";
        linkHref = href;
        inLink = href !== null;
      }
      continue;
    }

    // —— 图片 / 嵌入内容：整体丢弃（不外链加载，也不保留 alt 噪音）——
    if (name === "img" || name === "picture" || name === "source") continue;
    if (name === "figure" || name === "iframe" || name === "object" || name === "embed") continue;

    // —— 换行 ——
    if (name === "br") {
      newLine();
      continue;
    }

    // —— 标题 ——
    if (HEADING_LEVEL[name]) {
      if (!isClose) {
        newParagraph();
        push(`${"#".repeat(HEADING_LEVEL[name])} `);
      } else {
        newParagraph();
      }
      continue;
    }

    // —— 列表项 ——
    if (name === "li") {
      if (!isClose) {
        newLine();
        push("- ");
      }
      continue;
    }
    if (name === "ul" || name === "ol") {
      newParagraph();
      continue;
    }

    // —— 强调 ——
    if (name === "strong" || name === "b") {
      push("**");
      continue;
    }
    if (name === "em" || name === "i") {
      push("*");
      continue;
    }
    if (name === "code" || name === "kbd") {
      push("`");
      continue;
    }

    // —— 其它块级：只作分段 ——
    if (BLOCK_TAGS.has(name)) {
      if (isClose || name === "p" || name === "blockquote" || name === "pre") newParagraph();
      continue;
    }

    // —— 未知/内联标签：忽略（属性一并丢弃）——
  }

  // 未闭合的 `<a>`：把已累积的文本补回去，避免整段丢失
  if (linkText) push(linkHref ? `[${linkText.trim()}](${linkHref})` : linkText.trim());

  return out
    .join("")
    .replace(/[ \t]+\n/g, "\n")
    .replace(/\n{3,}/g, "\n\n")
    .replace(/^\n+|\n+$/g, "")
    .trim();
}

/**
 * 统一入口：不管作者写的是 HTML 还是 Markdown，都产出**可交给 `Markdown` 渲染的 Markdown**。
 *
 * ⚠️ 必须先判定再重铸：`htmlToMarkdown` 会把无标签文本的换行折叠成空格，
 * 直接喂给 Markdown 描述会丢掉段落结构。
 */
export function toMarkdown(src: string): string {
  if (!src) return "";
  return looksLikeHtml(src) ? htmlToMarkdown(src) : src;
}

/**
 * 富文本（HTML 或 Markdown）→ **纯文本**（列表行 / 摘要 / 搜索匹配用）。
 *
 * 先按 VCMI 口径把 HTML 重铸为 Markdown，再剥掉 Markdown 标记。
 */
export function richToPlainText(src: string): string {
  if (!src) return "";
  const md = looksLikeHtml(src) ? htmlToMarkdown(src) : src;
  return md
    .replace(/!\[[^\]]*\]\([^)]*\)/g, "") // 图片
    .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1") // 链接 → 文本
    .replace(/^\s{0,3}#{1,6}\s+/gm, "") // 标题
    .replace(/^\s{0,3}[-*+]\s+/gm, "") // 无序列表符号
    .replace(/^\s{0,3}\d+[.)]\s+/gm, "") // 有序列表符号
    .replace(/^\s{0,3}>\s?/gm, "") // 引用
    .replace(/(\*\*|__)(.*?)\1/g, "$2") // 粗体
    .replace(/(\*|_)(.*?)\1/g, "$2") // 斜体
    .replace(/`/g, "") // 行内代码
    .replace(/\s+/g, " ")
    .trim();
}
