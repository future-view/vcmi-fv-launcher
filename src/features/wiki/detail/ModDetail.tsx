import type { LangDict } from "@/i18n/types";
import type { WikiLinkTarget } from "@/lib/modcache";
import { pickName } from "@/lib/modcache";
import { WikiLink } from "./detailShared";

export function ModDetail({
  d,
  lang,
  t,
  modId,
  modCache,
  onNavigate,
  compact,
}: {
  d: Record<string, unknown>;
  lang: string;
  t: LangDict;
  modId: string;
  modCache?: Record<string, unknown>;
  onNavigate?: (target: WikiLinkTarget) => void;
  compact?: boolean;
}) {
  const cc = (d.contentCount as Record<string, number>) ?? {};
  // 模组元数据（多语言 name/description/version）：来自完整缓存 cache.json 顶层 modMeta
  // （与 mods/realms 页同源，见后端 build/mod.rs::build_mod_meta）。索引条目仅含单串 name，
  // 故详情标题/描述一律优先取 modMeta，缺失时回退索引 d.name。
  const meta = (modCache?.modMeta as Record<string, unknown>) ?? {};
  const metaName = (meta.name as Record<string, string>) ?? null;
  const metaDesc = (meta.description as Record<string, string>) ?? null;
  const metaVersion = meta.version != null ? String(meta.version) : "";
  // 内容清单（对齐 VCMI WikiModContent：mod 页分节列出城镇/生物/英雄/宝物等，可跳转）
  const sections: { key: WikiLinkTarget["category"]; label: string }[] = [
    { key: "factions", label: t.wiki.factions },
    { key: "creatures", label: t.wiki.creatures },
    { key: "heroes", label: t.wiki.heroes },
    { key: "artifacts", label: t.wiki.artifacts },
    { key: "spells", label: t.wiki.spells },
    { key: "skills", label: t.wiki.skills },
  ];
  const listFor = (cat: WikiLinkTarget["category"]): Record<string, unknown>[] =>
    ((modCache?.[cat] as unknown[]) ?? []) as Record<string, unknown>[];
  return (
    <div className={compact ? "space-y-3" : "mt-3 space-y-3"}>
      {/* VCMI 模组页（WikiModContent）：标题 = mod 名，副标题 = mod 类型，无属性表 */}
      <div className="text-center">
        <h2 className="text-lg font-bold text-amber-200">
          {pickName(metaName ?? (d.name as Record<string, string>) ?? {}, lang) || modId}
        </h2>
        <p className="text-sm text-slate-400">{String(d.type ?? "")}</p>
      </div>
      {/* 模组描述：来自完整缓存 cache.json 顶层 modMeta 的多语言 description（索引条目不含该字段） */}
      {(() => {
        const desc = metaDesc ? pickName(metaDesc, lang) : "";
        const version = metaVersion;
        if (!desc && !version) return null;
        return (
          <div className="mx-auto mt-3 max-w-2xl space-y-1 px-4 text-center">
            {desc ? (
              <p className="whitespace-pre-line text-sm leading-relaxed text-slate-300">{desc}</p>
            ) : null}
            {version ? <p className="text-xs text-slate-500">v{version}</p> : null}
          </div>
        );
      })()}
      {sections.map(({ key, label }) => {
        const items = listFor(key);
        if (items.length === 0) return null;
        return (
          <div key={key}>
            <h3 className="mb-1 text-sm font-semibold text-slate-300">
              {label}
              <span className="ml-1.5 text-xs font-normal text-slate-500">{items.length}</span>
            </h3>
            <div className="flex flex-wrap gap-1.5">
              {items.map((it) => {
                const id = String(it.id ?? it.factionId ?? "");
                const name = pickName((it.name as Record<string, string>) ?? {}, lang) || id;
                return (
                  <WikiLink key={id} modId={modId} category={key} id={id} label={name} onNavigate={onNavigate} />
                );
              })}
            </div>
          </div>
        );
      })}
    </div>
  );
}
