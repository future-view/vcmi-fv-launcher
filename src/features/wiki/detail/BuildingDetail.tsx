import type { WikiLinkTarget } from "@/lib/modcache";
import { fullImgUrl, pickName } from "@/lib/modcache";
import { HdIcon } from "@/shared/HdIcon";
import { formatCost, type ResourceInfo } from "./detailShared";

// 建筑详情（右键弹窗内容）：标题居中 + 大图 + 描述 + 花费（对齐 VCMI CInfoWindow + BUILDING 组件）
export function BuildingDetail({
  d,
  lang,
  modId,
  cacheDir,
  resources,
  compact,
}: {
  d: Record<string, unknown>;
  lang: string;
  modId: string;
  cacheDir?: string | null;
  resources?: ResourceInfo[];
  onNavigate?: (target: WikiLinkTarget) => void;
  compact?: boolean;
}) {
  const imgUrl = d.image ? fullImgUrl(cacheDir, d.image as string) : null;
  const name = pickName((d.name as Record<string, string>) ?? {}, lang) || "—";
  const desc = pickName((d.description as Record<string, string>) ?? {}, lang);
  return (
    <div className={compact ? "p-1" : "p-4"}>
      <h2 className="text-center text-lg font-bold text-amber-100">{name}</h2>
      <div className="mt-3 flex justify-center">
        {imgUrl ? (
          <HdIcon src={imgUrl} alt="" className="h-32 w-40 flex-none rounded-lg border border-slate-600/50 bg-slate-800/40 object-contain p-2" />
        ) : (
          <div className="h-32 w-40 flex-none rounded-lg border border-slate-600/50 bg-slate-800/60" />
        )}
      </div>
      {desc && (
        <p className="mt-3 text-center text-sm leading-relaxed text-slate-300">{desc}</p>
      )}
      <div className="mt-4 flex flex-wrap items-center justify-center gap-2 rounded-xl border border-slate-800/60 bg-slate-900/40 px-3 py-2">
        {formatCost(d.cost, cacheDir, modId, resources, lang)}
      </div>
    </div>
  );
}
