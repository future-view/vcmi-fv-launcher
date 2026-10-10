import type { LangDict } from "@/i18n/types";
import type { WikiEntry, WikiLinkTarget } from "@/lib/modcache";
import { fullImgCandidates, pickName } from "@/lib/modcache";
import { HdImg } from "@/shared/HdImg";
import { useHdRate } from "@/shared/useHdRate";
import { DetailProps, type ResourceInfo } from "./detailShared";
import { BuildingDetail } from "./BuildingDetail";
import { CreatureDetail } from "./CreatureDetail";
import { HeroDetail } from "./HeroDetail";
import { FactionDetail } from "./FactionDetail";
import { ArtifactDetail } from "./ArtifactDetail";
import { SpellDetail } from "./SpellDetail";
import { SkillDetail } from "./SkillDetail";
import { ModDetail } from "./ModDetail";
import { TerrainDetail } from "./TerrainDetail";

export function EntryDetail({ entry, contentLang, t, cacheDir, modCache, onNavigate, compact }: DetailProps) {
  const d = entry.data as Record<string, unknown>;

  const name = pickName(entry.name, contentLang);

  /** 高清倍率：通用大图头部（`max-h-[320px]`）会用高清变体 */
  const rate = useHdRate();
  const imageCandidates = entry.image
    ? fullImgCandidates(cacheDir ?? null, entry.image, rate)
    : [];

  // faction 页面由 FactionDetail 全权渲染（标题+合成图+各表），跳过通用大图头部避免双 header；
  // spells / skills / artifacts 的子组件自带 DetailHeader（VCMI 这些页无大图 header），同样跳过；
  // creatures / heroes / terrains / buildings 详情子组件自带标题/头像/图片，跳过通用 header 避免重复显示。
  const skipGenericHeader =
    entry.category === "factions" ||
    entry.category === "spells" ||
    entry.category === "skills" ||
    entry.category === "artifacts" ||
    entry.category === "creatures" ||
    entry.category === "heroes" ||
    entry.category === "terrains" ||
    entry.category === "buildings" ||
    entry.category === "mods";

  return (
    <div className={compact ? "" : "p-4"}>
      {/* 顶部：大图（仅在非 compact 详情页展示；VCMI 生物窗无静态图，动画由 CreatureDetail 自绘） */}
      {!compact && !skipGenericHeader && (
        <>
          <div className="relative overflow-hidden rounded-xl border border-slate-700/60">
            {imageCandidates.length > 0 ? (
              <div className="flex min-h-[200px] items-center justify-center bg-slate-800/60 p-4">
                <HdImg
                  src={imageCandidates}
                  alt={name}
                  className="max-h-[320px] w-auto object-contain"
                  draggable={false}
                />
              </div>
            ) : (
              <div className="flex min-h-[120px] items-center justify-center bg-slate-800/40 text-sm text-slate-500">
                {t.wiki.noImage}
              </div>
            )}
          </div>

          {/* 名称 */}
          <h2 className="mt-3 text-xl font-bold text-slate-100">{name}</h2>
          {pickName(entry.name, contentLang === "chinese" ? "english" : "chinese") && (
            <p className="text-sm text-slate-400">{pickName(entry.name, contentLang === "chinese" ? "english" : "chinese")}</p>
          )}
        </>
      )}

      {/* 按类型渲染详情 */}
      {renderDetail(entry, d, contentLang, t, onNavigate, cacheDir, modCache, compact)}
    </div>
  );
}

function renderDetail(
  entry: WikiEntry,
  d: Record<string, unknown>,
  lang: string,
  t: LangDict,
  onNavigate?: (target: WikiLinkTarget) => void,
  cacheDir?: string | null,
  modCache?: Record<string, unknown>,
  compact?: boolean,
) {
  const modId = entry.modId;
  switch (entry.category) {
    case "buildings":
      return <BuildingDetail d={d} lang={lang} modId={modId} cacheDir={cacheDir} resources={modCache?.resources as ResourceInfo[] | undefined} onNavigate={onNavigate} compact={compact} />;
    case "creatures":
      return <CreatureDetail d={d} lang={lang} modId={modId} cacheDir={cacheDir} modCache={modCache} t={t} onNavigate={onNavigate} compact={compact} />;
    case "heroes":
      return <HeroDetail d={d} lang={lang} t={t} modId={modId} cacheDir={cacheDir} modCache={modCache} onNavigate={onNavigate} compact={compact} />;
    case "factions":
      return <FactionDetail d={d} lang={lang} t={t} modId={modId} cacheDir={cacheDir} modCache={modCache} onNavigate={onNavigate} />;
    case "artifacts":
      return <ArtifactDetail d={d} lang={lang} t={t} modId={modId} cacheDir={cacheDir} modCache={modCache} onNavigate={onNavigate} compact={compact} />;
    case "spells":
      return <SpellDetail d={d} lang={lang} t={t} modId={modId} cacheDir={cacheDir} resources={modCache?.resources as ResourceInfo[] | undefined} compact={compact} />;
    case "skills":
      return <SkillDetail d={d} lang={lang} t={t} cacheDir={cacheDir} compact={compact} />;
    case "terrains":
      return <TerrainDetail d={d} lang={lang} t={t} cacheDir={cacheDir} compact={compact} />;
    case "mods":
      return <ModDetail d={d} lang={lang} t={t} modId={modId} modCache={modCache} onNavigate={onNavigate} compact={compact} />;
    default:
      return null;
  }
}
