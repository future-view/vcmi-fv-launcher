import type { LangDict } from "@/i18n/types";
import { pickNameAny } from "@/lib/modcache";
import { Markdown } from "@/shared/Markdown";

/**
 * 词汇表详情（VCMI 游戏内 Wiki 的 Glossary 分类）。
 *
 * 正文是 Markdown，但有两处 VCMI 私有语法与标准 Markdown 不同：
 * - `![alt](wikiPicture/x)`：资源引用不带扩展名，且图片不在启动器的素材缓存里；
 * - `{术语}`：VCMI 的 wiki 内链语法，不是 Markdown。
 *
 * 首版按"纯文本可读"处理：剥离图片、内链降级为普通文本。
 * 后续接入配图/跳转时，只需改 `normalize`（图片走资源转存 + `HdImg`，内链走 `onNavigate`）。
 */
function normalize(src: string): string {
  const stripped = src
    .replace(/!\[[^\]]*\]\([^)\s]*\)/g, "")
    .replace(/\{([^{}\n]+)\}/g, "$1");

  // 删掉"只剩标题、下面没有正文"的空章节（典型：`## LOGO` 下只有一张被剥离的图）
  const lines = stripped.replace(/\r\n?/g, "\n").split("\n");
  const out: string[] = [];
  for (let i = 0; i < lines.length; i++) {
    if (/^#{1,6}\s+/.test(lines[i])) {
      let j = i + 1;
      while (j < lines.length && !lines[j].trim()) j++;
      if (j >= lines.length || /^#{1,6}\s+/.test(lines[j])) continue;
    }
    out.push(lines[i]);
  }
  return out.join("\n").replace(/\n{3,}/g, "\n\n").trim();
}

export function GlossaryDetail({
  d,
  lang,
  t,
  compact,
}: {
  d: Record<string, unknown>;
  lang: string;
  t: LangDict;
  compact?: boolean;
}) {
  // 词汇表常只有中文翻译（实测 Expansion 只带 chinese.json）：按
  // 「当前语言 → 英文 → auto → 中文 → 任意」回退，避免英文界面显示 key。
  const name = pickNameAny((d.name as Record<string, string>) ?? {}, lang);
  const text = normalize(pickNameAny((d.description as Record<string, string>) ?? {}, lang));

  return (
    <div className={compact ? "space-y-3" : "mt-3 space-y-3"}>
      <h2 className="text-center text-lg font-bold text-amber-200">{name || t.wiki.glossary}</h2>
      {text ? <Markdown source={text} /> : null}
    </div>
  );
}
