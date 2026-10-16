import type { LangDict } from "@/i18n/types";
import type { WikiCategory, WikiEntry, WikiLinkTarget } from "@/lib/modcache";
import { fullImgUrl, pickName } from "@/lib/modcache";
import { useWikiPopup, usePopupTriggers } from "./WikiPopupContext";
import { HdIcon } from "@/shared/HdIcon";
import {
  DetailHeader,
  StatRow,
  WikiLink,
  formatResourceAmount,
  makeRefResolver,
  type ResourceInfo,
} from "./detailShared";

export function ArtifactDetail({
  d,
  lang,
  t,
  modId,
  cacheDir,
  modCache,
  onNavigate,
  compact,
  getEntry,
}: {
  d: Record<string, unknown>;
  lang: string;
  t: LangDict;
  modId: string;
  cacheDir?: string | null;
  modCache?: Record<string, unknown>;
  onNavigate?: (target: WikiLinkTarget) => void;
  compact?: boolean;
  getEntry?: (category: WikiCategory, id: string) => WikiEntry | undefined;
}) {
  // 组合组件/所属组合的名字、图标与归属 mod：本 mod 神器表优先，缺失回退全局索引。
  // 实测该发行版 47% 的组件引用只存在于 core（mod 的组合宝物引用原版组件）——
  // 没有这条兜底时三格只剩裸 id 且无图标。
  const artifactRef = makeRefResolver({
    category: "artifacts",
    modId,
    lang,
    modCache,
    getEntry,
  });
  const components = (d.components as string[]) ?? [];
  const slots = (d.slots as string[]) ?? [];
  const partOf = (d.partOf as string[]) ?? [];
  // 宝物等级/槽位本地化（对齐 VCMI WikiArtifactContent class/slot 表）
  const clsRaw = String(d.class ?? "");
  const clsNum = Number(clsRaw);
  const classKey = Number.isFinite(clsNum) && clsRaw !== ""
    ? (["classTreasure", "classMinor", "classMajor", "classRelic", "classScroll"][clsNum] ?? "classOther")
    : (`class${clsRaw.charAt(0).toUpperCase()}${clsRaw.slice(1).toLowerCase()}` as keyof typeof t.wiki.artifact);
  const classLabel = (t.wiki.artifact as Record<string, string>)[classKey as string] ?? clsRaw;
  // 槽位来自 ARTRAITS.TXT 枚举名（LEFT_HAND / MACH1 / MISC5 / SPELLBOOK ...），
  // 需转成 camelCase i18n 键；多个 Misc/战争机器槽位合并显示为一项。
  const slotLabel = (s: string) => {
    const art = t.wiki.artifact as Record<string, string>;
    if (s.startsWith("MACH")) return art.slotWarMachine ?? s;
    if (s.startsWith("MISC")) return art.slotMisc ?? s;
    if (s.toUpperCase() === "SPELLBOOK") return art.slotSpellbook ?? s;
    const key =
      "slot" +
      s
        .toLowerCase()
        .split("_")
        .map((p) => p.charAt(0).toUpperCase() + p.slice(1))
        .join("");
    return art[key] ?? s;
  };
  const slotText =
    slots.length > 0 ? [...new Set(slots.map(slotLabel))].join(", ") : "—";
  // VCMI WikiArtifactContent：组成部件/所属组合为竖排列表，每行 = 图标 + 名称
  const artifactRows = (ids: string[]) => (
    <div className="space-y-1">
      {ids.map((id) => {
        const ref = artifactRef(id);
        return (
          <div
            key={id}
            className="flex items-center gap-2 rounded-lg border border-slate-800/60 bg-slate-900/40 px-2 py-1.5"
          >
            {ref.image ? (
              <HdIcon
                src={fullImgUrl(cacheDir, ref.image)}
                alt=""
                className="h-8 w-8 flex-none rounded border border-slate-700/50 object-contain"
              />
            ) : (
              <span className="h-8 w-8 flex-none rounded border border-slate-700/40 bg-slate-800/40" />
            )}
            <WikiLink
              modId={ref.modId}
              category="artifacts"
              id={id}
              label={ref.name}
              onNavigate={onNavigate}
            />
          </div>
        );
      })}
    </div>
  );
  // VCMI WikiArtifactContent：宝物行右键 = 描述文本气泡
  const popup = useWikiPopup();
  const triggers = usePopupTriggers();
  return (
    <div className={compact ? "space-y-3" : "mt-3 space-y-3"}>
      <DetailHeader
        icon={fullImgUrl(cacheDir, d.image as string)}
        name={pickName((d.name as Record<string, string>) ?? {}, lang)}
        subtitle={clsRaw ? classLabel : t.wiki.artifacts}
        badge={d.isCombined === true ? t.wiki.artifact.combined : t.wiki.artifacts}
      />
      <div className="grid grid-cols-2 gap-x-6 rounded-xl border border-slate-800/60 bg-slate-900/40 p-3">
        <StatRow k={t.wiki.artifact.headerClass} v={classLabel} />
        <StatRow
          k={t.wiki.artifact.headerPrice}
          v={formatResourceAmount(d.price, "gold", cacheDir, modCache?.resources as ResourceInfo[] | undefined, lang)}
        />
        <StatRow k={t.wiki.artifact.headerSlots} v={slotText} />
        {d.isCombined === true ? <StatRow k={t.wiki.artifact.combined} v={t.wiki.stats.yes} /> : null}
      </div>
      {/* VCMI 宝物页：描述在属性表下方 */}
      {pickName((d.description as Record<string, string>) ?? {}, lang) ? (
        <p className="text-sm leading-relaxed text-slate-300">
          {pickName((d.description as Record<string, string>) ?? {}, lang)}
        </p>
      ) : null}
      {components.length > 0 && (
        <div>
          <h3 className="mb-1 text-sm font-semibold text-slate-300">{t.wiki.artifact.components}</h3>
          {artifactRows(components)}
        </div>
      )}
      {partOf.length > 0 && (
        <div>
          <h3 className="mb-1 text-sm font-semibold text-slate-300">{t.wiki.artifact.partOf}</h3>
          {artifactRows(partOf)}
        </div>
      )}
      {(() => {
        const bonuses = (d.bonuses as Record<string, unknown>[]) ?? [];
        return bonuses.length > 0 ? (
          <div>
            <h3 className="mb-1 text-sm font-semibold text-slate-300">{t.wiki.artifact.effects}</h3>
            <ul className="space-y-1.5">
              {bonuses.map((b, i) => {
                const bname = pickName((b.name as Record<string, string>) ?? {}, lang);
                const bdesc = pickName((b.description as Record<string, string>) ?? {}, lang);
                const val = Number(b.val ?? 0);
                const title = bname || String(b.type ?? t.wiki.artifact.effect);
                return (
                  <li
                    key={i}
                    className="cursor-help rounded-lg border border-slate-800/60 bg-slate-900/40 p-2"
                    // VCMI WikiArtifactContent：宝物效果行右键 = 描述文本气泡
                    {...triggers((p) => popup.showText(bdesc || title, p))}
                  >
                    <div className="text-[13px] font-medium text-slate-200">
                      {title}
                      {val !== 0 ? <span className="ml-1 text-blue-300">+{val}</span> : null}
                    </div>
                    {bdesc && bdesc !== title ? (
                      <p className="mt-0.5 text-xs leading-relaxed text-slate-400">{bdesc}</p>
                    ) : null}
                  </li>
                );
              })}
            </ul>
          </div>
        ) : null;
      })()}
    </div>
  );
}
