import type { LangDict } from "@/i18n/types";
import { fullImgUrl, pickName } from "@/lib/modcache";
import { SafeImg } from "./detailShared";

/**
 * 地形详情（VCMI WikiWindow.cpp:660-696）
 * 内容 = 地形名 + 地形图块 + 移动消耗（负数为不可通行）+ 以该地形为本族的城镇列表。
 */
export function TerrainDetail({
  d,
  lang,
  t,
  cacheDir,
  compact,
}: {
  d: Record<string, unknown>;
  lang: string;
  t: LangDict;
  cacheDir?: string | null;
  compact?: boolean;
}) {
  const name = pickName((d.name as Record<string, string>) ?? {}, lang);
  const cost = Number(d.moveCost ?? -1);
  const towns = (d.nativeTowns as string[]) ?? [];
  return (
    <div className={compact ? "space-y-3" : "mt-3 space-y-3"}>
      <h2 className="text-center text-lg font-bold text-amber-200">{name}</h2>
      <div className="flex justify-center">
        <SafeImg
          src={fullImgUrl(cacheDir, d.image as string)}
          alt=""
          className="h-24 w-24 rounded-lg border border-slate-600/50 bg-slate-800/40 pixelated object-contain"
          placeholderClassName="h-24 w-24 rounded-lg border border-slate-600/50 bg-slate-800/40"
        />
      </div>
      <div className="space-y-1.5 rounded-xl border border-slate-700/50 bg-slate-900/40 p-3">
        <div className="flex items-center justify-between gap-2">
          <span className="text-sm text-slate-400">{t.wiki.terrainDetail.moveCost}</span>
          <span className="text-sm font-medium text-slate-100">
            {cost < 0 ? t.wiki.terrainDetail.impassable : `${cost}%`}
          </span>
        </div>
        {towns.length > 0 ? (
          <div className="flex items-start justify-between gap-2">
            <span className="flex-none text-sm text-slate-400">
              {t.wiki.terrainDetail.nativeTowns}
            </span>
            <span className="text-right text-sm text-slate-100">{towns.join("、")}</span>
          </div>
        ) : null}
      </div>
    </div>
  );
}
