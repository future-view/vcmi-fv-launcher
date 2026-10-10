import type { LangDict } from "@/i18n/types";
import { fullImgUrl, pickName } from "@/lib/modcache";
import { DetailHeader, StatRow, formatCost, type ResourceInfo } from "./detailShared";

export function SpellDetail({ d, lang, t, modId, cacheDir, resources, compact }: { d: Record<string, unknown>; lang: string; t: LangDict; modId: string; cacheDir?: string | null; resources?: ResourceInfo[]; compact?: boolean }) {
  const desc = (d.description as Record<string, string>) ?? {};
  // 分级描述（VCMI 法术页按 初级/中级/高级 展开标题+描述，CSpell.cpp:141-145）
  const levelDesc = (d.levelDescriptions as Record<string, Record<string, string>>) ?? {};
  const name = pickName((d.name as Record<string, string>) ?? {}, lang);
  // 派系名（多语言）：`schoolNames`（后端按 `spellSchool.*.name` 生成；原版由 h3-core 提供
  // "气系/火系/土系/水系"）→ 回退 `schools` 英文 id → `school` 单值。此前直接显示 "air, fire"。
  const schoolText =
    ((d.schoolNames as Record<string, string>[]) ?? [])
      .map((n) => pickName(n ?? {}, lang))
      .filter(Boolean)
      .join(", ") ||
    (d.schools as string[])?.join(", ") ||
    (d.school as string) ||
    "";
  // 等级术语随启动器界面语言（与技能同源），不按内容语言硬编码，避免 ru/pl/de 界面回退英文
  const levelNames = ["", t.wiki.skillLevels.basic, t.wiki.skillLevels.advanced, t.wiki.skillLevels.expert];
  const levelLabel = (lvl: number) => levelNames[lvl] ?? "";
  const tiers = [1, 2, 3]
    .map((lv) => {
      const key = ["", "basic", "advanced", "expert"][lv];
      const v = levelDesc[key];
      // 原版法术的分级描述是纯字符串（SPTRAITS.TXT），mod 法术是多语言对象
      const raw = typeof v === "string" ? v : pickName(v ?? {}, lang);
      // 剥掉 H3 富文本的 {标题} 标记（标题行已单独显示）
      return { lv, txt: raw.replace(/^\{[^}]*\}\s*/, "").trim() };
    })
    .filter((x) => x.txt);
  return (
    <div className={compact ? "space-y-3" : "mt-3 space-y-3"}>
      <DetailHeader
        icon={fullImgUrl(cacheDir, d.image as string)}
        name={name}
        subtitle={`${schoolText || t.wiki.spells}${d.level ? ` · Lv ${d.level}` : ""}`}
        badge={t.wiki.spells}
      />
      <div className="grid grid-cols-2 gap-x-6 rounded-xl border border-slate-800/60 bg-slate-900/40 p-3">
        <StatRow k={t.wiki.spell.type} v={d.type} />
        <StatRow k={t.wiki.spell.school} v={schoolText || "—"} />
        <StatRow k={t.wiki.spell.level} v={d.level} />
        <StatRow k={t.wiki.spell.mana} v={d.mana} />
        <StatRow k={t.wiki.spell.cost} v={formatCost(d.cost, cacheDir, modId, resources, lang)} />
      </div>
      {tiers.length > 0 ? (
        <div className="space-y-2">
          {tiers.map(({ lv, txt }) => (
            <div key={lv}>
              <p className="text-[13px] font-semibold text-amber-200">{`${levelLabel(lv)}${name}`}</p>
              <p className="whitespace-pre-wrap text-sm leading-relaxed text-slate-300">{txt}</p>
            </div>
          ))}
        </div>
      ) : pickName(desc, lang) ? (
        <p className="whitespace-pre-wrap text-sm leading-relaxed text-slate-400">{pickName(desc, lang)}</p>
      ) : null}
    </div>
  );
}
