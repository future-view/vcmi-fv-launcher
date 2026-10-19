import type { LangDict } from "@/i18n/types";
import type { WikiLinkTarget } from "@/lib/modcache";
import { fullImgUrl, pickName } from "@/lib/modcache";
import { HdIcon } from "@/shared/HdIcon";
import { SafeImg } from "./detailShared";
import { useWikiPopup, usePopupTriggers } from "./WikiPopupContext";

/**
 * 模组页的内容节顺序（对齐 VCMI `WikiModContent.cpp` 的 buildModContent）。
 *
 * ⚠️ **不含 `terrains`**：地形是游戏级数据，`read_terrain_type` 会把游戏根
 * `config/terrains.json` 的 10 个原版地形并进**每一个** mod 的 cache.json
 * （实测 123/124 个 mod 都带同一份），而 VCMI 判"是否属于本 mod"靠实体 scope
 * （`isEntityOfMod`，:65-68）——地形的 scope 是 `core`，所以它对任何第三方 / 核心
 * 子模组都不是"自己的内容"。列在这里只会让每个模组页都挂着一节一字不差的地形。
 * 地形本身有独立的 `terrains` 类目可查。
 */
const SECTIONS: WikiLinkTarget["category"][] = [
  "factions",
  "heroes",
  "creatures",
  "artifacts",
  "spells",
  "skills",
];

export function ModDetail({
  d,
  lang,
  t,
  modId,
  cacheDir,
  modCache,
  onNavigate,
  sourceLabels,
  compact,
}: {
  d: Record<string, unknown>;
  lang: string;
  t: LangDict;
  modId: string;
  cacheDir?: string | null;
  modCache?: Record<string, unknown>;
  onNavigate?: (target: WikiLinkTarget) => void;
  sourceLabels?: Record<string, { modId: string; label: string }>;
  compact?: boolean;
}) {
  const popup = useWikiPopup();
  const triggers = usePopupTriggers();
  // 模组元数据（多语言 name/description/version）：来自完整缓存 cache.json 顶层 modMeta
  const meta = (modCache?.modMeta as Record<string, unknown>) ?? {};
  const metaName = (meta.name as Record<string, string>) ?? null;
  const metaDesc = (meta.description as Record<string, string>) ?? null;
  const metaVersion = meta.version != null ? String(meta.version) : "";
  // 该 mod 自己定义的类目（后端 `defines`）：决定这一节要不要列。
  // 旧缓存没有该字段时退化为"数组非空即列"（地形会重复，但好过整页空白）。
  const defines = (modCache?.defines as Record<string, boolean> | undefined) ?? undefined;
  // 法术图标回退：mod 法术常只有 iconIndex（不在 H3 法术表内的为 -1），
  // 与 HeroDetail 同口径拼 `core/images/spellbon/<idx>.webp`
  const spellbonDir = ((modCache?.sharedIcons as Record<string, string>) ?? {}).spellbonDir;

  const labelOf = (key: WikiLinkTarget["category"]) =>
    key === "factions"
      ? t.wiki.factions
      : key === "heroes"
        ? t.wiki.heroes
        : key === "creatures"
          ? t.wiki.creatures
          : key === "artifacts"
            ? t.wiki.artifacts
            : key === "spells"
              ? t.wiki.spells
              : key === "skills"
                ? t.wiki.skills
                : t.wiki.terrains;

  /** 条目图标的**缓存相对路径**（由 SafeImg / HdIcon 各自拼 URL） */
  const iconRel = (cat: WikiLinkTarget["category"], it: Record<string, unknown>): string | null =>
    cat === "factions"
      ? ((it.preview as string) ?? (it.background as string) ?? null)
      : cat === "spells"
        ? ((it.image as string) ??
          (Number(it.iconIndex) >= 0 && spellbonDir
            ? `${spellbonDir}/${Number(it.iconIndex)}.webp`
            : null))
        : ((it.image as string) ?? null);

  return (
    <div className={compact ? "space-y-3" : "mt-3 space-y-3"}>
      {/* VCMI 模组页：标题 = mod 名（居中），副标题 = mod 类型，无属性表 */}
      <div className="text-center">
        <h2 className="text-lg font-bold text-amber-200">
          {pickName(metaName ?? (d.name as Record<string, string>) ?? {}, lang) || modId}
        </h2>
        <p className="text-sm text-slate-400">{String(d.type ?? "")}</p>
      </div>
      {/* 模组描述：来自完整缓存 cache.json 顶层 modMeta 的多语言 description */}
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

      {SECTIONS.map((key) => {
        // 跨 mod 补丁引用（VCMI `<modId>:<entityId>` 语法，构建期标 `ref:true`）不是本 mod
        // 的实体：列表与计数均已排除，详情页同样不能列。否则 `core.*` 伪模组页会多出
        // 21 条"法术"（全是城堡/墓园护城河的 moat / trigger 引用），且同一条会被列两次
        // （如 `core:dungeonMoat` 与 `core:dungeonMoatTrigger` 都叫 Boiling Oil）。
        const items = (((modCache?.[key] as unknown[]) ?? []) as Record<string, unknown>[]).filter(
          (it) => !it.ref,
        );
        if (items.length === 0) return null;
        // `defines` 存在时以它为准：这一节只在"本 mod 自己定义了该类内容"时出现
        //（地形是游戏级共享数据，否则每个模组页都会挂一节相同的地形）
        if (defines && defines[key] === false) return null;
        const label = labelOf(key);
        const sorted = [...items].sort((a, b) =>
          (
            pickName((a.name as Record<string, string>) ?? {}, lang) || String(a.id ?? "")
          ).localeCompare(
            pickName((b.name as Record<string, string>) ?? {}, lang) || String(b.id ?? ""),
          ),
        );
        return (
          <div key={key}>
            <h3 className="mb-1 text-sm font-semibold text-slate-300">
              {label}
              <span className="ml-1.5 text-xs font-normal text-slate-500">{items.length}</span>
            </h3>
            {/* 一条一行（对齐 VCMI `addIconRow`，WikiModContent.cpp:170-200）：
                整行可点，32px 图标 + 名称；聚合态在行尾标注来源子模组。 */}
            <div className="divide-y divide-slate-800/50 overflow-hidden rounded-lg border border-slate-800/60 bg-slate-900/40">
              {sorted.map((it) => {
                const id = String(it.id ?? it.factionId ?? "");
                const name = pickName((it.name as Record<string, string>) ?? {}, lang) || id;
                const rel = iconRel(key, it);
                const src = sourceLabels?.[`${key}:${id}`];
                const desc =
                  pickName((it.description as Record<string, string>) ?? {}, lang) ||
                  pickName((it.biography as Record<string, string>) ?? {}, lang);
                const jump = () =>
                  onNavigate?.({ modId: src?.modId ?? modId, category: key, id });
                return (
                  <button
                    key={id}
                    type="button"
                    onClick={jump}
                    // 右键：生物/英雄弹完整窗（对齐 VCMI 的 CStackWindow / CHeroOverview），
                    // 其余弹描述文本气泡；位置由 triggers 给的光标坐标决定
                    {...triggers((p) => {
                      if (key === "creatures" || key === "heroes") popup.showEntry(src?.modId ?? modId, key, it);
                      else popup.showText(desc || name, p);
                    })}
                    title={desc || name}
                    className="flex w-full items-center gap-2 px-2 py-1 text-left transition-colors hover:bg-slate-800/50"
                  >
                    <span className="flex h-8 w-8 flex-none items-center justify-center overflow-hidden rounded border border-slate-700/50 bg-slate-800/60">
                      {rel ? (
                        <SafeImg
                          src={fullImgUrl(cacheDir, rel)}
                          alt=""
                          className="h-full w-full object-contain"
                          placeholderClassName="h-full w-full bg-slate-800/60"
                        />
                      ) : null}
                    </span>
                    <span className="min-w-0 flex-1 truncate text-[11px] text-slate-200">{name}</span>
                    {src ? (
                      <span className="flex-none truncate rounded bg-slate-800/70 px-1.5 py-0.5 text-[10px] text-slate-400">
                        {src.label}
                      </span>
                    ) : null}
                  </button>
                );
              })}
            </div>
          </div>
        );
      })}
    </div>
  );
}
