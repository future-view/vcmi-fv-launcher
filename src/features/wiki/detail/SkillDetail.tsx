import type { LangDict } from "@/i18n/types";
import { fullImgUrl, pickName } from "@/lib/modcache";
import { DetailHeader } from "./detailShared";

export function SkillDetail({ d, lang, t, cacheDir, compact }: { d: Record<string, unknown>; lang: string; t: LangDict; cacheDir?: string | null; compact?: boolean }) {
  const levels = (d.levels as Record<string, Record<string, string>>) ?? {};
  // VCMI 等级术语（NSecondarySkill：basic/advanced/expert）随启动器界面语言取词（`t`）；
  // 不能按 `lang` 硬编码 —— `lang` 是内容取词语言（仅 zh/en），ru/pl/de 界面会回退英文。
  const order: [string, string][] = [
    ["basic", t.wiki.skillLevels.basic],
    ["advanced", t.wiki.skillLevels.advanced],
    ["expert", t.wiki.skillLevels.expert],
  ];
  return (
    <div className={compact ? "space-y-3" : "mt-3 space-y-3"}>
      <DetailHeader
        icon={fullImgUrl(cacheDir, d.image as string)}
        name={pickName((d.name as Record<string, string>) ?? {}, lang)}
        subtitle={d.type ? String(d.type) : t.wiki.skills}
        badge={t.wiki.skills}
      />
      {order.map(([key, label]) => {
        const txt = pickName(levels[key] ?? {}, lang);
        if (!txt) return null;
        return (
          <div key={key} className="rounded-xl border border-slate-800/60 bg-slate-900/40 p-3">
            <h3 className="mb-1 text-sm font-semibold text-slate-300">{label}</h3>
            <p className="text-sm leading-relaxed text-slate-400">{txt}</p>
          </div>
        );
      })}
    </div>
  );
}
