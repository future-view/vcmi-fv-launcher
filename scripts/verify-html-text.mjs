#!/usr/bin/env node
/**
 * 富文本归一校验（`src/lib/html.ts`）：HTML → Markdown / 纯文本。
 *
 * 为什么要单独立一个校验脚本：
 *   mod.json 的描述由模组作者自由填写（实测含 `<h1 style='color:#002366'>` 之类 HTML），
 *   官方启动器的处理是「HTML → Markdown 重铸 → 无 HTML 渲染」
 *   （`references/vcmi/launcher/modManager/{modstate,cmodlistview_moc}.cpp`）。
 *   我们的解析器是手写的白名单重铸，**安全相关的行为必须有回归**：
 *   一旦有人改成放行属性或走 innerHTML，本脚本立刻失败。
 *
 * 校验重点：① 标签/属性（含 style、on*）全部消失；② 脚本块与 javascript:/data: 链接被丢弃；
 *   ③ Markdown 输入不被误伤；④ 语义（标题/强调/列表/链接/实体）保留。
 *
 * 用法：node scripts/verify-html-text.mjs      （失败退出码 1）
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { transform } from "esbuild";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(__dirname, "..");

/** 用 esbuild 把 TS 转成 ESM 字符串（vite 的传递依赖，无需新增 devDependency） */
const src = fs.readFileSync(path.join(REPO, "src", "lib", "html.ts"), "utf8");
const { code } = await transform(src, { loader: "ts", format: "esm", target: "es2020" });
const mod = await import(`data:text/javascript;base64,${Buffer.from(code).toString("base64")}`);
const { htmlToMarkdown, toMarkdown, richToPlainText, looksLikeHtml } = mod;

let fail = 0;
const report = (ok, name, extra) => {
  if (!ok) fail++;
  console.log(`${ok ? "PASS" : "FAIL"}  ${name}${ok || !extra ? "" : `\n      ${extra}`}`);
};
const eq = (name, got, want) =>
  report(got === want, name, `got : ${JSON.stringify(got)}\n      want: ${JSON.stringify(want)}`);
const has = (name, got, sub) =>
  report(got.includes(sub), name, `missing: ${JSON.stringify(sub)}\n      in: ${JSON.stringify(got.slice(0, 200))}`);
const hasnt = (name, got, sub) =>
  report(!got.includes(sub), name, `unexpected: ${JSON.stringify(sub)}`);

// —— 真实样本：`new-monsters-pack` 的英文描述（内联 style + 列表 + 链接）——
const real =
  `<h1 style='color:#002366; font-size:28px; font-weight:bold;'>New Monsters Pack</h1>` +
  `<p style='font-size:16px; color:#333333; font-style:italic;'> Explore the expanded horizons of the <i>New Monsters Pack</i>. </p>` +
  `<p style='font-size:18px; color:#002366; font-weight:bold;'>An Array of Powerful Units</p>` +
  `<ul><li>the corrosive <span style='color:#002366; font-weight:bold;'>Acidic Roper</span></li>` +
  `<li>the stealthy <b>Dragonfish</b></li></ul>` +
  `<p>See <a href="https://vcmi.eu">VCMI</a> for more &amp; details.</p>`;

const md = htmlToMarkdown(real);
console.log("---- 真实样本输出 ----\n" + md + "\n----------------------\n");
has("真实样本→一级标题", md, "# New Monsters Pack");
has("真实样本→斜体保留", md, "*New Monsters Pack*");
has("真实样本→粗体保留", md, "**Dragonfish**");
has("真实样本→列表项", md, "- the corrosive Acidic Roper");
has("真实样本→链接", md, "[VCMI](https://vcmi.eu)");
has("真实样本→实体解码", md, "& details");
hasnt("真实样本→无残留标签", md, "<");
hasnt("真实样本→无 style 属性", md, "style=");
hasnt("真实样本→无 span", md, "span");

// —— 安全（本脚本存在的核心理由）——
eq("脚本块整体丢弃", htmlToMarkdown("<script>alert(1)</script>hi"), "hi");
eq("样式块整体丢弃", htmlToMarkdown("<style>a{}</style>hi"), "hi");
eq("javascript: 链接降级为纯文本", htmlToMarkdown(`<a href="javascript:alert(1)">click</a>`), "click");
eq("data: 链接降级为纯文本", htmlToMarkdown(`<a href="data:text/html,<b>x</b>">click</a>`), "click");
eq("href 缺失时不产生链接", htmlToMarkdown(`<a>click</a>`), "click");
eq("onclick 属性丢弃", htmlToMarkdown(`<p onclick="alert(1)">t</p>`), "t");

// —— 基础语法 ——
eq("换行", htmlToMarkdown("a<br>b"), "a\nb");
eq("段落分段", htmlToMarkdown("<p>a</p><p>b</p>"), "a\n\nb");
eq("h2→##", htmlToMarkdown("<h2>T</h2>"), "## T");
eq("h6→######", htmlToMarkdown("<h6>T</h6>"), "###### T");
eq("粗体", htmlToMarkdown("<b>x</b>"), "**x**");
eq("斜体", htmlToMarkdown("<em>x</em>"), "*x*");
eq("代码", htmlToMarkdown("<code>x</code>"), "`x`");
eq("图片丢弃", htmlToMarkdown(`<img src="x.png" alt="A">t`), "t");
eq("实体 &amp;", htmlToMarkdown("a &amp; b"), "a & b");
eq("实体 &#39;", htmlToMarkdown("it&#39;s"), "it's");

// —— Markdown 输入不被误伤（`toMarkdown` 必须先判定再重铸）——
const mdIn = "## Title\n\nSome **bold** text.";
eq("Markdown 输入原样保留", toMarkdown(mdIn), mdIn);
eq("looksLikeHtml: HTML 为真", String(looksLikeHtml("<p>x</p>")), "true");
eq("looksLikeHtml: Markdown 为假", String(looksLikeHtml("## Title\n\nx")), "false");

// —— 纯文本（列表行 / 摘要 / 搜索匹配）——
const plain = richToPlainText(real);
hasnt("纯文本无标签", plain, "<");
hasnt("纯文本无 # 标记", plain, "#");
has("纯文本含标题文字", plain, "New Monsters Pack");
has("纯文本含链接文字（去 URL）", plain, "VCMI for more");
eq("纯文本折叠空白", richToPlainText("<p>a</p>   <p>b</p>"), "a b");

console.log(fail === 0 ? "\n✅ 富文本归一校验通过" : `\n❌ ${fail} 项失败`);
process.exit(fail === 0 ? 0 : 1);
