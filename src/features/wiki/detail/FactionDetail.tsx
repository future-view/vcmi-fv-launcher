import { useState, Fragment } from "react";
import type { LangDict } from "@/i18n/types";
import type { WikiLinkTarget } from "@/lib/modcache";
import { fullImgCandidates, fullImgUrl, pickName, type WikiCategory } from "@/lib/modcache";
import { cn } from "@/lib/utils";
import { HdIcon } from "@/shared/HdIcon";
import { HdImg } from "@/shared/HdImg";
import { useHdRate } from "@/shared/useHdRate";
import { useWikiPopup, usePopupTriggers } from "./WikiPopupContext";
import { SafeImg, WikiLink, formatCost, type ResourceInfo } from "./detailShared";

export function FactionDetail({
  d,
  lang,
  t,
  modId,
  cacheDir,
  modCache,
  onNavigate,
}: {
  d: Record<string, unknown>;
  lang: string;
  t: LangDict;
  modId: string;
  cacheDir?: string | null;
  modCache?: Record<string, unknown>;
  onNavigate?: (target: WikiLinkTarget) => void;
}) {
  /** 高清素材倍率：本页顶部城镇背景是**全宽大图**，是"糊"最明显的一处 */
  const rate = useHdRate();
  // 资源多语言名（cache.json.resources，v79 起）：花费显示用
  const resList = modCache?.resources as ResourceInfo[] | undefined;
  const creatureList = (d.creatureList as string[]) ?? [];
  const creaturesAll = ((modCache?.creatures as unknown[]) ?? []) as Record<string, unknown>[];
  const creatureRows = creatureList
    .map((id) => creaturesAll.find((c) => String(c.id) === id))
    .filter((c): c is Record<string, unknown> => !!c);
  const buildings = (d.buildings as Record<string, unknown>[]) ?? [];
  const heroesAll = ((modCache?.heroes as unknown[]) ?? []) as Record<string, unknown>[];
  // VCMI 式弹窗：右键按住显示 / 松开关闭（建筑行=文本气泡，生物行/英雄行=完整窗）
  const popup = useWikiPopup();
  const triggers = usePopupTriggers();
  const nameC = (n: Record<string, string> | undefined) => pickName(n ?? {}, lang);
  // 生物表属性表头图标（来自缓存 statIcons：游戏 stackWindow def 提取）
  const statIcons = (modCache?.statIcons ?? {}) as Record<string, string>;
  // 城镇预览合成图（后台按需生成并缓存；生成前回退原始 background）
  const [preview] = useState<string | null>((d.preview as string) ?? null);
  /** 完整窗：取缓存对象后弹窗（生物=CStackWindow，英雄=CHeroOverview） */
  const openEntry = (category: WikiCategory, id: string) => {
    let obj: Record<string, unknown> | undefined;
    if (category === "buildings") {
      for (const f of ((modCache?.factions as unknown[]) ?? [])) {
        const bs = ((f as Record<string, unknown>).buildings as unknown[]) ?? [];
        const hit = bs.find((b) => String((b as Record<string, unknown>).id) === id);
        if (hit) {
          obj = hit as Record<string, unknown>;
          break;
        }
      }
    } else {
      const hit = ((modCache?.[category] as unknown[]) ?? []).find(
        (b) => String((b as Record<string, unknown>).id) === id,
      );
      if (hit) obj = hit as Record<string, unknown>;
    }
    if (obj) popup.showEntry(modId, category, obj);
  };
  // 优先显示原始城镇背景图；仅旧缓存缺失 background 时才回退到后台合成的 preview
  return (
    <div className="mt-3 space-y-4">
      {/* 顶部：城镇背景大图 + 标题 + 阵营。
          **无图模式**（素材未就绪 / 该条目本就没有背景图）：容器不再塌陷 ——
          `min-h` + 渐变底保住版式，标题与阵营仍在左下角，与有图时视觉连续。
          图加载失败时 `placeholderClassName` 也撑住同样高度，避免"图挂了连标题一起被裁"。 */}
      <div className="relative min-h-[132px] overflow-hidden rounded-xl border border-slate-800/60 bg-gradient-to-br from-slate-800/50 via-slate-900/60 to-slate-950">
        {(d.background || preview) ? (
          <HdImg
            src={fullImgCandidates(cacheDir ?? null, (d.background ?? preview) as string, rate)}
            alt=""
            className="w-full object-contain"
            placeholderClassName="h-[132px] w-full bg-gradient-to-br from-slate-800/50 via-slate-900/60 to-slate-950"
            draggable={false}
          />
        ) : (
          <div className="relative flex h-[132px] w-full items-center justify-center overflow-hidden">
            {/* 纯 CSS 径向光晕：不引入任何图标 / 图片资源，暗色主题下也不显脏 */}
            <div className="absolute inset-0 bg-[radial-gradient(ellipse_at_center,rgba(99,102,241,0.14),transparent_70%)]" />
            <span className="relative text-[11px] tracking-wide text-slate-500">
              {t.wiki.preparingBg}
            </span>
          </div>
        )}
        <div className="absolute left-3 bottom-3 flex items-end gap-2">
          <h2 className="rounded bg-slate-950/60 px-2 py-1 text-2xl font-bold text-white drop-shadow backdrop-blur-sm">
            {nameC(d.name as Record<string, string>)}
          </h2>
          {String(d.alignment ?? "") ? (
            <span
              className={cn(
                "mb-1 inline-flex rounded px-2 py-0.5 text-[11px] font-medium backdrop-blur-sm",
                String(d.alignment).toLowerCase() === "good"
                  ? "bg-emerald-500/25 text-emerald-200"
                  : String(d.alignment).toLowerCase() === "evil"
                    ? "bg-red-500/25 text-red-200"
                    : "bg-slate-600/40 text-slate-200",
              )}
            >
              {t.wiki.alignment[String(d.alignment).toLowerCase() as keyof typeof t.wiki.alignment] ?? String(d.alignment)}
            </span>
          ) : null}
        </div>
      </div>
      {pickName((d.description as Record<string, string>) ?? {}, lang) && (
        <p className="text-sm leading-relaxed text-slate-400">{pickName((d.description as Record<string, string>) ?? {}, lang)}</p>
      )}
      {buildings.length > 0 && (
        <div>
          <h3 className="mb-1 text-sm font-semibold text-slate-300">{t.wiki.sections.buildings}</h3>
          <div className="overflow-x-auto rounded-xl border border-slate-800/60">
            <table className="w-full text-left text-xs">
              <thead className="bg-slate-800/60 text-slate-400">
                <tr>
                  <th className="px-2 py-1"></th>
                  {/* 名称列加长：建筑名较长，窄容器里容易被压成竖排 */}
                  <th className="min-w-[150px] px-2 py-1">{t.wiki.sections.name}</th>
                  <th className="px-2 py-1">{t.wiki.sections.cost}</th>
                </tr>
              </thead>
              <tbody>
                {[...buildings]
                  .filter((b) => {
                    const mode = String(b.mode ?? "normal");
                    const name = pickName((b.name as Record<string, string>) ?? {}, lang);
                    return mode === "normal" && name.trim().length > 0;
                  })
                  .sort((x, y) => Number(x.bid ?? 0) - Number(y.bid ?? 0))
                  .map((b) => {
                  const bid = String(b.id);
                  const bName = pickName((b.name as Record<string, string>) ?? {}, lang);
                  const bDesc = pickName((b.description as Record<string, string>) ?? {}, lang);
                  return (
                    <tr
                      key={bid}
                      className="cursor-pointer border-t border-slate-800/40 hover:bg-slate-800/30"
                      onClick={() => onNavigate?.({ modId, category: "buildings", id: bid })}
                      // VCMI WikiTownContent.cpp:277-285：建筑行右键 = 文本气泡 "{名称}\n\n描述"
                      // 建筑常无描述：缺省时只显示名称，避免气泡里出现一段空白
                      {...triggers((p) =>
                        popup.showText(bDesc ? `{${bName}}\n\n${bDesc}` : `{${bName}}`, p),
                      )}
                    >
                      <td className="px-2 py-1">
                        <SafeImg
                          src={fullImgUrl(cacheDir, b.image as string)}
                          alt=""
                          className="h-14 w-20 pixelated rounded border border-slate-700/50 object-contain"
                          placeholderClassName="h-14 w-20 rounded border border-slate-700/50 bg-slate-800/60"
                        />
                      </td>
                      <td className="min-w-[150px] px-2 py-1">
                        <WikiLink
                          modId={modId}
                          category="buildings"
                          id={bid}
                          label={bName}
                          onNavigate={onNavigate}
                        />
                      </td>
                      <td className="px-2 py-1 text-amber-300">{formatCost(b.cost, cacheDir, modId, resList, lang)}</td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        </div>
      )}
      {creatureRows.length > 0 && (
        <div>
          <h3 className="mb-1 text-sm font-semibold text-slate-300">{t.wiki.sections.creatures}</h3>
          <div className="overflow-x-auto rounded-xl border border-slate-800/60">
            {/* 本表共 11 列（图片 / 名称 / 花费 + 8 项属性）：名称列给足最小宽，
                避免窄容器里中文名被压成竖排；图片另有 min-w-10 兜底。 */}
            <table className="w-full text-left text-xs">
              <thead className="bg-slate-800/60 text-slate-400">
                <tr>
                  <th className="px-2 py-1"></th>
                  {/* 名称列加长：避免中文名被压成竖排 */}
                  <th className="min-w-[110px] px-2 py-1">{t.wiki.sections.name}</th>
                  <th className="px-2 py-1">{t.wiki.sections.cost}</th>
                  {(["level", "attack", "defense", "damage", "speed", "health", "growth", "ai"] as const).map((k) => {
                    const icon = statIcons[k];
                    return (
                      <th
                        key={k}
                        className="px-1.5 py-1 text-center"
                        title={t.wiki.stats[k]}
                        // VCMI WikiTownContent.cpp:362-368：表头图标右键 = 属性说明文本气泡
                        {...triggers((p) => popup.showText(t.wiki.stats[k], p))}
                      >
                        {icon ? (
                          <HdIcon
                            src={fullImgUrl(cacheDir, icon)}
                            alt={t.wiki.stats[k]}
                            className="mx-auto h-5 w-5 object-contain"
                            draggable={false}
                          />
                        ) : (
                          t.wiki.stats[k]
                        )}
                      </th>
                    );
                  })}
                </tr>
              </thead>
              <tbody>
                {(() => {
                  // 升级链去重：跨条目追踪已渲染 id，避免同一生物既作 + 行又作 T 行重复显示
                  const seen = new Set<string>();
                  return creatureRows.map((c) => {
                    const cid = String(c.id);
                    if (seen.has(cid)) return null;
                    const chain: Record<string, unknown>[] = [c];
                    seen.add(cid);
                    let cur = c;
                    for (let k = 0; k < 8; k++) {
                      const nx = ((cur.upgrades as string[]) ?? [])
                        .map((u) => creaturesAll.find((x) => String(x.id) === u))
                        .find((x): x is Record<string, unknown> => !!x);
                      const nid = nx ? String(nx.id) : "";
                      if (!nx || seen.has(nid)) break;
                      seen.add(nid);
                      chain.push(nx);
                      cur = nx;
                    }
                    return (
                    <Fragment key={cid}>
                      {chain.map((r, idx) => {
                        const rid = String(r.id);
                        const dm = r as Record<string, unknown>;
                        const dmgMin = Number(dm.damageMin ?? 0);
                        const dmgMax = Number(dm.damageMax ?? 0);
                        return (
                          <tr
                            key={rid}
                            className="cursor-pointer border-t border-slate-800/40 hover:bg-slate-800/30"
                            onClick={() => onNavigate?.({ modId, category: "creatures", id: rid })}
                            // VCMI WikiTownContent.cpp:434：生物行右键 = CStackWindow 完整生物窗
                            {...triggers(() => openEntry("creatures", rid))}
                          >
                            <td className="px-2 py-1">
                              <SafeImg
                                src={fullImgUrl(cacheDir, (r.image as string) || (r.advMapImage as string))}
                                alt=""
                                // min-w-10：Tailwind 基础样式有 `img { max-width:100% }`，
                                // 窄容器里图片会跟着被压缩的单元格缩小，这里兜底不让其低于 40px
                                className="h-8 w-10 min-w-10 pixelated rounded border border-slate-700/50 object-contain"
                                placeholderClassName="h-8 w-10 min-w-10 rounded border border-slate-700/50 bg-slate-800/60"
                              />
                            </td>
                            <td className="min-w-[110px] px-2 py-1">
                              <span className="text-slate-200">
                                {idx === 0
                                  ? Number(r.level) > 0
                                    ? `T${r.level}: `
                                    : ""
                                  : "+ "}
                                {pickName((r.name as Record<string, string>) ?? {}, lang)}
                              </span>
                            </td>
                            <td className="px-2 py-1 text-amber-300">{formatCost(r.fullRecruitCost ?? r.cost, cacheDir, modId, resList, lang)}</td>
                            <td className="px-2 py-1 text-slate-300">{String(r.level ?? "")}</td>
                            <td className="px-2 py-1 text-slate-300">{String(r.attack ?? "")}</td>
                            <td className="px-2 py-1 text-slate-300">{String(r.defense ?? "")}</td>
                            <td className="px-2 py-1 text-slate-300">
                              {dmgMin || dmgMax ? `${dmgMin}-${dmgMax}` : ""}
                            </td>
                            <td className="px-2 py-1 text-slate-300">{String(r.speed ?? "")}</td>
                            <td className="px-2 py-1 text-slate-300">{String(r.hitPoints ?? r.health ?? "")}</td>
                            <td className="px-2 py-1 text-slate-300">{String(r.growth ?? "")}</td>
                            <td className="px-2 py-1 text-slate-300">{String(r.aiValue ?? "")}</td>
                          </tr>
                        );
                      })}
                    </Fragment>
                    );
                  });
                })()}
              </tbody>
            </table>
          </div>
        </div>
      )}
      {heroesAll.length > 0 && (() => {
        // 对齐 VCMI addHeroesTable：按 heroClass affinity 分 might/magic 两个表；
        // 组标题 = 组内出现过的职业名（" / " 连接）；special 英雄不显示
        const visible = heroesAll.filter((h) => h.special !== true);
        const byAff = new Map<string, Record<string, unknown>[]>();
        visible.forEach((h) => {
          const aff = String(h.affinity ?? "");
          if (!byAff.has(aff)) byAff.set(aff, []);
          byAff.get(aff)!.push(h);
        });
        const classNamesOf = (rows: Record<string, unknown>[]) =>
          Array.from(
            new Set(rows.map((h) => pickName((h.className as Record<string, string>) ?? {}, lang) || String(h.class ?? ""))),
          ).join(" / ");
        const renderGroup = (heroes: Record<string, unknown>[], key: string) =>
          heroes.length === 0 ? null : (
            <div key={key}>
              <h3 className="mb-1 text-sm font-semibold text-slate-300">{t.wiki.sections.heroes}</h3>
              <h4 className="mb-1 text-xs font-semibold uppercase tracking-wide text-slate-400">{classNamesOf(heroes)}</h4>
              <div className="overflow-x-auto rounded-xl border border-slate-800/60">
                <table className="w-full text-left text-xs">
                  <thead className="bg-slate-800/60 text-slate-400">
                    <tr>
                      <th className="px-2 py-1"></th>
                      <th className="px-2 py-1">{t.wiki.sections.name}</th>
                      <th className="px-2 py-1">{t.wiki.sections.gender}</th>
                      <th className="px-2 py-1"></th>
                      <th className="px-2 py-1">{t.wiki.sections.specialty}</th>
                    </tr>
                  </thead>
                  <tbody>
                    {heroes.map((h) => {
                      const hid = String(h.id);
                      const spec = h.specialty as Record<string, string> | undefined;
                      const specName = spec ? pickName(spec, lang) : "";
                      const specIcon = h.specialtyIcon as string | undefined;
                      return (
                        <tr
                          key={hid}
                          className="border-t border-slate-800/40 hover:bg-slate-800/30"
                          onClick={() => onNavigate?.({ modId, category: "heroes", id: hid })}
                          // VCMI WikiTownContent.cpp:562：英雄行右键 = CHeroOverview 完整英雄窗
                          {...triggers(() => openEntry("heroes", hid))}
                        >
                          <td className="px-2 py-1">
                            <SafeImg
                              src={fullImgUrl(cacheDir, h.image as string)}
                              alt=""
                              className="h-9 w-9 pixelated rounded border border-slate-700/50 object-contain"
                              placeholderClassName="h-9 w-9 rounded border border-slate-700/50 bg-slate-800/60"
                            />
                          </td>
                          <td className="px-2 py-1 text-slate-200">{pickName((h.name as Record<string, string>) ?? {}, lang)}</td>
                          <td className="px-2 py-1 text-slate-300">{h.female === true ? "♀" : "♂"}</td>
                          {/* 特长图标（VCMI: UN44 帧 / mod 自定义 sprite） */}
                          <td className="px-2 py-1">
                            {specIcon ? (
                              <SafeImg
                                src={fullImgUrl(cacheDir, specIcon)}
                                alt=""
                                className="h-7 w-7 pixelated rounded border border-slate-700/50 object-contain"
                                placeholderClassName="h-7 w-7 rounded border border-slate-700/50 bg-slate-800/60"
                              />
                            ) : (
                              <span className="text-amber-400/70">✦</span>
                            )}
                          </td>
                          <td className="px-2 py-1">
                            {specName ? (
                              <span className="block max-w-[140px] truncate text-[11px] text-amber-300/80" title={specName}>
                                {specName}
                              </span>
                            ) : null}
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>
            </div>
          );
        return (
          <div className="space-y-3">
            {renderGroup(byAff.get("might") ?? [], "might")}
            {renderGroup(byAff.get("magic") ?? [], "magic")}
            {!byAff.has("might") && !byAff.has("magic") ? renderGroup(visible, "all") : null}
          </div>
        );
      })()}
    </div>
  );
}
