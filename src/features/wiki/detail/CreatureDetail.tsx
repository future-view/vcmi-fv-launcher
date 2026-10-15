import { Fragment, useEffect, useRef, useState } from "react";
import { cn } from "@/lib/utils";
import type { LangDict } from "@/i18n/types";
import type { WikiLinkTarget } from "@/lib/modcache";
import { fullImgCandidates, fullImgUrl, pickName } from "@/lib/modcache";
import { getDefGroup } from "@/lib/def/animationCache";
import type { DefFrame } from "@/lib/def/decoder";
import { HdIcon } from "@/shared/HdIcon";
import { HdImg } from "@/shared/HdImg";
import { useHdRate } from "@/shared/useHdRate";
import { useWikiPopup, usePopupTriggers } from "./WikiPopupContext";
import {
  ANIM,
  CREA_PREVIEW,
  MACH_PREVIEW,
  RelatedRow,
  StatTableRow,
  costText,
  formatResourceAmount,
  type ResourceInfo,
} from "./detailShared";

export function CreatureDetail({
  d,
  lang,
  modId,
  cacheDir,
  modCache,
  t,
  compact,
  onNavigate,
}: {
  d: Record<string, unknown>;
  lang: string;
  modId: string;
  cacheDir?: string | null;
  modCache?: Record<string, unknown>;
  t: LangDict;
  onNavigate?: (target: WikiLinkTarget) => void;
  compact?: boolean;
}) {
  // VCMI CCreaturePic（client/widgets/MiscWidgets.cpp:675-706）：
  //   背景 = 阵营 creatureBackground 130px；动画 = creature.animDefName（战斗动画），
  //   startPreview() 循环播放预览动作，帧间隔 100ms（Images.cpp:482）。
  // VCMI 生物窗没有静态图模式，也没有"启用动画"开关 —— 始终播放动画。
  const battleRes = (d.animation as string) ?? "";
  /** 高清素材倍率：兵种大图在本页会被**放大**显示，高清收益最明显的一处 */
  const rate = useHdRate();
  const backgroundCandidates = d.creatureBackground
    ? fullImgCandidates(cacheDir ?? null, d.creatureBackground as string, rate)
    : [];
  // 只持有**当前播放分组**的帧：大型战斗 DEF 含十几个分组数百帧，
  // 一次性全解会在主线程同步执行、打开页面时掉帧（设计 §13.1）。
  // 按需只解当前动作，并带字节级/分组级 LRU 缓存（§13.2）。
  const [frames, setFrames] = useState<DefFrame[]>([]);
  const [groupIdx, setGroupIdx] = useState<number>(ANIM.HOLDING);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const popup = useWikiPopup();
  const triggers = usePopupTriggers();
  // 弹窗态能力列（第三列）的滚动容器：用于判断"还有没有下方内容"，决定底部渐隐提示
  const abilityScrollRef = useRef<HTMLDivElement | null>(null);
  const [abilityCanScroll, setAbilityCanScroll] = useState(false);

  // 加载当前动作分组的帧（惰性 + 缓存）
  useEffect(() => {
    if (!battleRes) {
      setFrames([]);
      return;
    }
    let cancelled = false;
    getDefGroup(modId, battleRes, groupIdx).then((f) => {
      if (cancelled) return;
      if (f.length > 0) {
        setFrames(f);
        return;
      }
      // 该动作分组不存在（部分生物缺少某些动作）→ 回落到 HOLDING
      if (groupIdx !== ANIM.HOLDING) {
        getDefGroup(modId, battleRes, ANIM.HOLDING).then((h) => {
          if (!cancelled && h.length > 0) setFrames(h);
        });
        return;
      }
      // HOLDING 也缺失：按 VCMI 预览表依次尝试，避免直接退化到静态图
      const previewList = d.siegeWeapon === true ? MACH_PREVIEW : CREA_PREVIEW;
      (async () => {
        for (const g of previewList) {
          if (cancelled) return;
          const h = await getDefGroup(modId, battleRes, g);
          if (h.length > 0) {
            setGroupIdx(g);
            setFrames(h);
            return;
          }
        }
        setFrames([]);
      })();
    });
    return () => {
      cancelled = true;
    };
  }, [battleRes, modId, groupIdx]);

  // 播放循环：100ms/帧；一组播完后随机切换预览动作（VCMI loopPreview, Images.cpp:488-529）
  useEffect(() => {
    if (frames.length === 0) return;
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;
    const f0 = frames[0];
    if (canvas.width !== f0.w || canvas.height !== f0.h) {
      canvas.width = f0.w;
      canvas.height = f0.h;
    }
    const previewList = d.siegeWeapon === true ? MACH_PREVIEW : CREA_PREVIEW;
    let raf = 0;
    let fi = 0;
    let last = performance.now();
    const interval = 100; // H3 固定 100ms/帧，与战斗速度无关
    const draw = (now: number) => {
      if (now - last >= interval) {
        last = now;
        const frame = frames[fi % frames.length];
        ctx.clearRect(0, 0, canvas.width, canvas.height);
        const rgba = new Uint8ClampedArray(frame.bgra.length);
        for (let i = 0; i < frame.bgra.length; i += 4) {
          rgba[i] = frame.bgra[i + 2];
          rgba[i + 1] = frame.bgra[i + 1];
          rgba[i + 2] = frame.bgra[i];
          rgba[i + 3] = frame.bgra[i + 3];
        }
        ctx.putImageData(new ImageData(rgba, frame.w, frame.h), 0, 0);
        fi++;
        if (fi >= frames.length) {
          // 一组播完 → 随机挑下一个预览动作（VCMI loopPreview 行为）
          setGroupIdx(previewList[Math.floor(Math.random() * previewList.length)]);
        }
      }
      raf = requestAnimationFrame(draw);
    };
    raf = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(raf);
  }, [frames, groupIdx, d.siegeWeapon]);

  const name = pickName((d.name as Record<string, string>) ?? {}, lang);
  const desc = pickName((d.description as Record<string, string>) ?? {}, lang);
  const abilities = (d.abilities as Record<string, unknown>[]) ?? [];

  // 能力列（弹窗第三列）是否"还有下方内容"：滚到底就撤掉底部渐隐。
  // 依赖 abilities / compact / lang —— 切换条目、切换语言、宽窄态切换都要重算。
  useEffect(() => {
    const el = abilityScrollRef.current;
    if (!el) {
      setAbilityCanScroll(false);
      return;
    }
    const update = () =>
      setAbilityCanScroll(el.scrollTop + el.clientHeight < el.scrollHeight - 1);
    update();
    el.addEventListener("scroll", update, { passive: true });
    return () => el.removeEventListener("scroll", update);
  }, [abilities, compact, lang]);

  const sharedIcons = (modCache?.sharedIcons ?? {}) as Record<string, string>;

  // 士气/运气：VCMI `WikiCreatureContent.cpp` 的百科页**不显示**这两项（只有战斗弹窗
  // CStackWindow 显示）；且原版 146 只生物里只有 9 只非 0（天使/大天使/牛头怪/牛头王
  // +1 士气，骨龙/鬼龙 −1，恶魔/大恶魔 运气 −1，矮妖精 +1），恒显只会得到两个 0。
  // → 非 0 才渲染，且值带符号。
  const moraleVal = Number(d.morale ?? 0);
  const luckVal = Number(d.luck ?? 0);
  const hasMoraleLuck = moraleVal !== 0 || luckVal !== 0;
  const mlFrame = (v: number) => Math.max(0, Math.min(6, v + 3));
  const moraleIcon = fullImgUrl(cacheDir, sharedIcons[`morale/${mlFrame(moraleVal)}`]);
  const luckIcon = fullImgUrl(cacheDir, sharedIcons[`luck/${mlFrame(luckVal)}`]);
  const signed = (v: number) => (v > 0 ? `+${v}` : String(v));

  // 统计表：**行序与行名对齐 VCMI `WikiCreatureContent.cpp:130-172`**（等级 → 攻击力 →
  // 防御力 → 杀伤力 → 速度 → 生命值 → 生长率 → 弹药量 → 法力 → 囤兵增长 → 双格单位 →
  // 地图部队 → AI价值 → 战斗价值）。前 7 行常驻，其余按实测覆盖率条件渲染：
  // 弹药 23% / 囤兵 15% / 双格 37% / 地图部队 96.6%（h3-core 150 条实测）。
  const dmgMin = Number(d.damageMin ?? 0);
  const dmgMax = Number(d.damageMax ?? 0);
  const damageText =
    dmgMin || dmgMax ? (dmgMax !== dmgMin ? `${dmgMin} - ${dmgMax}` : `${dmgMin}`) : "";
  const mapMin = Number(d.advMapAmountMin ?? 0);
  const mapMax = Number(d.advMapAmountMax ?? 0);
  const mapText = mapMin || mapMax ? (mapMin === mapMax ? `${mapMin}` : `${mapMin} – ${mapMax}`) : "";
  const statRows: { k: string; v: string }[] = [
    { k: t.wiki.stats.level, v: String(Number(d.level ?? 0)) },
    { k: t.wiki.stats.attack, v: String(Number(d.attack ?? 0)) },
    { k: t.wiki.stats.defense, v: String(Number(d.defense ?? 0)) },
    { k: t.wiki.stats.damage, v: damageText },
    { k: t.wiki.stats.speed, v: String(Number(d.speed ?? 0)) },
    { k: t.wiki.stats.health, v: String(Number(d.hitPoints ?? 0)) },
    { k: t.wiki.stats.growth, v: String(Number(d.growth ?? 0)) },
  ];
  const shots = Number(d.shots ?? 0);
  if (shots > 0) statRows.push({ k: t.wiki.stats.shots, v: String(shots) });
  const casts = Number(d.casts ?? 0);
  if (casts > 0) statRows.push({ k: t.wiki.stats.mana, v: String(casts) });
  const horde = Number(d.horde ?? 0);
  if (horde > 0) statRows.push({ k: t.wiki.stats.horde, v: String(horde) });
  if (d.doubleWide === true) statRows.push({ k: t.wiki.stats.doubleWide, v: t.wiki.stats.yes });
  if (mapText) statRows.push({ k: t.wiki.stats.mapAmount, v: mapText });
  statRows.push({ k: t.wiki.stats.ai, v: String(Number(d.aiValue ?? 0)) });
  statRows.push({ k: t.wiki.stats.fightValue, v: String(Number(d.combatValue ?? 0)) });

  // 招募花费：主显 **fullRecruitCost**（含升级链总投入，VCMI `getFullRecruitCost()`，
  // 见 CCreatureHandler.cpp:182 / UpgradeInfo.cpp:20 用它算升级差价），
  // tooltip 补"本阶花费"差额。原版实测 cost 组合最多 2 种资源（128 只纯金币 +
  // 18 只 金币+一种稀有资源），故图标是必需的：只写数字无法区分资源种类。
  const resources = modCache?.resources as ResourceInfo[] | undefined;
  const fullCost = (d.fullRecruitCost ?? {}) as Record<string, unknown>;
  const ownCost = (d.cost ?? {}) as Record<string, unknown>;
  const hasFullCost = Object.values(fullCost).some((v) => Number(v) > 0);
  const hasOwnCost = Object.values(ownCost).some((v) => Number(v) > 0);
  const fullCostText = costText(fullCost, cacheDir ?? null, resources, lang);
  const ownCostText = costText(ownCost, cacheDir ?? null, resources, lang);
  const costTitle =
    hasFullCost && hasOwnCost && fullCostText !== ownCostText
      ? `${t.wiki.sections.recruitCost}（含升级链）: ${fullCostText}\n${t.wiki.sections.cost}: ${ownCostText}`
      : undefined;

  // 升级链 / 招募建筑：名称与图标都从**当前 mod 的缓存**里按 id 取（同 mod 是常态，
  // 原版生物全在 core.<faction> 内闭环）；跨 mod 的升级（mod 生物升级 core 生物）
  // 取不到名字时回退 id，跳转由 WikiView 的按 id 兜底处理。
  const modCreatures = (modCache?.creatures as Record<string, unknown>[]) ?? [];
  const creatureById = (id: string) => modCreatures.find((c) => String(c.id ?? "") === id);
  const creatureLabel = (id: string) => {
    const hit = creatureById(id);
    return (hit && pickName(hit.name as Record<string, string>, lang)) || id;
  };
  const creatureIcon = (id: string) => {
    const hit = creatureById(id);
    return hit ? fullImgCandidates(cacheDir ?? null, String(hit.image ?? ""), rate) : [];
  };
  const idList = (v: unknown) =>
    Array.isArray(v) ? v.filter((x): x is string => typeof x === "string" && !!x) : [];
  /**
   * 升级链去重。实测「阿尔法龙的升级自」出现两行一模一样的幼龙：同一个基础生物被
   * 多处注册（mod 覆盖 core 时，缓存里 core 的原版条目与 mod 自己的条目都指向它），
   * 二者 id 不同但**显示名与图标完全相同**。
   * 去重口径 = 先按 id，再按「显示名 + 图标」：同名同图即同一生物的重复注册；
   * 仅同名而图不同仍视为不同生物（那是真的有两个同名兵种）。
   */
  const dedupeRel = (ids: string[]) => {
    const seenId = new Set<string>();
    const seenKey = new Set<string>();
    const out: string[] = [];
    for (const id of ids) {
      if (seenId.has(id)) continue;
      seenId.add(id);
      const key = `${creatureLabel(id)}|${creatureIcon(id)[0] ?? ""}`;
      if (seenKey.has(key)) continue;
      seenKey.add(key);
      out.push(id);
    }
    return out;
  };
  const upgradeTo = dedupeRel(idList(d.upgrades));
  const upgradeFrom = dedupeRel(idList(d.upgradedFrom));
  const navTo = (id: string) => onNavigate?.({ modId, category: "creatures", id });

  // 招募建筑：后端 `creature_dwellings` 是「语言 → 建筑名数组」（town.rs:165-178），
  // 没有 id/图标，故渲染为不可点的名称条（不是伪链接）。
  const dwMap = (d.dwellings ?? {}) as Record<string, string[]>;
  const dwNames =
    (Array.isArray(dwMap[lang]) && dwMap[lang]) ||
    (Array.isArray(dwMap.english) && dwMap.english) ||
    (Array.isArray(dwMap.chinese) && dwMap.chinese) ||
    Object.values(dwMap).find((a) => Array.isArray(a) && a.length > 0) ||
    [];

  // 能力来源标签（VCMI `CCreatureWindow.cpp:286-297` 的 bonusNames：图标框底部一行小字）
  const sourceLabel = (src?: string) => {
    switch (src) {
      case "SPELL":
        return t.wiki.bonusSource.spell;
      case "SECONDARY_SKILL":
      case "HERO_SPECIAL":
        return t.wiki.bonusSource.hero;
      case "STACK_EXPERIENCE":
        return t.wiki.bonusSource.experience;
      case "COMMANDER":
        return t.wiki.bonusSource.commander;
      case "CREATURE_ABILITY":
        return t.wiki.bonusSource.creature;
      default:
        return t.wiki.bonusSource.other;
    }
  };

  // 能力图标框的描边色：对齐 VCMI `CCreatureWindow.cpp:277-290` 的 bonusColors
  // （ARTIFACT 绿 / CREATURE_ABILITY 黄 / SPELL_EFFECT 橙 / 技能系 紫 / 经验系 青）。
  const abilityBorder = (src: string) => {
    switch (src) {
      case "ARTIFACT":
      case "ARTIFACT_INSTANCE":
        return "border-emerald-500/60";
      case "SPELL_EFFECT":
        return "border-orange-500/60";
      case "SECONDARY_SKILL":
      case "HERO_SPECIAL":
        return "border-purple-500/60";
      case "STACK_EXPERIENCE":
      case "COMMANDER":
        return "border-cyan-500/60";
      default:
        return "border-amber-500/60";
    }
  };

  /* ------------------------------ 各内容块 ------------------------------ */

  const picBlock = (
    <div className="space-y-2">
      <div
        className={cn(
          "relative mx-auto overflow-hidden rounded-xl border border-slate-600/60 bg-[#0a0e1a]",
          compact ? "h-[130px] w-[130px]" : "h-[180px] w-[180px]",
        )}
      >
        {/* VCMI CCreaturePic：阵营 creatureBackground 130px 作底，战斗动画叠在其上。
            ⚠️ 用底层 `<img>` 而非 CSS `background-image`：后者**无法在加载失败时回退**，
            而高清变体是按需生成的（可能暂时没有）—— 回退是常态，缺图会整块空白。 */}
        {backgroundCandidates.length > 0 && (
          <HdImg
            src={backgroundCandidates}
            alt=""
            className="pointer-events-none absolute inset-0 h-full w-full object-contain"
          />
        )}
        <div className="relative flex h-full w-full items-center justify-center">
          {frames.length > 0 ? (
            <canvas ref={canvasRef} className="max-h-full max-w-full pixelated" />
          ) : (
            <HdImg
              src={fullImgCandidates(
                cacheDir ?? null,
                (d.imageLarge as string) ?? (d.image as string),
                rate,
              )}
              alt=""
              className="max-h-full max-w-full object-contain"
              pixelArt
            />
          )}
        </div>
      </div>
      {hasMoraleLuck && (
        <div className="flex justify-center gap-3">
          {[
            { url: moraleIcon, v: moraleVal, title: t.wiki.creature.morale },
            { url: luckIcon, v: luckVal, title: t.wiki.creature.luck },
          ]
            .filter((x) => x.v !== 0)
            .map((x) => (
              <div key={x.title} className="flex items-center gap-1" title={x.title}>
                {x.url ? (
                  <HdIcon src={x.url} alt="" className="h-5 w-5 object-contain" />
                ) : (
                  <div className="h-5 w-5 rounded border border-slate-700/50 bg-slate-800/40" />
                )}
                <span className="text-[11px] font-medium text-slate-200">{signed(x.v)}</span>
              </div>
            ))}
        </div>
      )}
    </div>
  );

  const sectionTitle = (text: string) => (
    <h3 className="text-xs font-semibold text-amber-200/90">{text}</h3>
  );

  const statsBlock = (
    <div className="space-y-1">
      {sectionTitle(t.wiki.sections.stats)}
      <div className="rounded-lg border border-slate-800/60 bg-slate-900/40 px-1 py-0.5">
        {statRows.map((r) => (
          <StatTableRow key={r.k} k={r.k} v={r.v} />
        ))}
      </div>
      {/* AI价值 / 战斗价值 是原版数据里就有、但游戏内界面从不展示的两个估值
          （VCMI 首次把它们放进百科），这里说明出处，避免被误当成启动器自造指标。
          compact（右键弹窗）不渲染：弹窗高度紧张，脚注是第一个可以让步的内容。 */}
      {!compact && (
        <p className="px-1 text-[10px] leading-relaxed text-slate-500">
          {t.wiki.stats.ai} / {t.wiki.stats.fightValue}：{t.wiki.creature.statNote}
        </p>
      )}
    </div>
  );

  const costBlock = hasFullCost ? (
    <div className="space-y-1">
      {sectionTitle(t.wiki.sections.recruitCost)}
      {/* ⚠️ 不用 `formatCost`：它是"右对齐 + 靠父级 flex 收缩"的一行式排版，
          资源种类多时（实测 mod 兵种可到 9 种：60000 金 + 8 种稀有资源）不会换行、
          直接溢出容器。这里改用**每个资源一枚**的 chip 行，`flex-wrap` 保证任意数量
          都能在 280px 的左栏里折成多行。 */}
      <div
        className="flex flex-wrap items-center gap-x-3 gap-y-1 rounded-lg border border-slate-800/60 bg-slate-900/40 px-2 py-1.5"
        title={costTitle}
      >
        {Object.entries(fullCost)
          .filter(([, v]) => Number(v) > 0)
          .map(([k, v]) => (
            <Fragment key={k}>
              <span className="text-amber-300">
                {formatResourceAmount(v, k, cacheDir ?? null, resources, lang)}
              </span>
            </Fragment>
          ))}
      </div>
    </div>
  ) : null;

  const relRow = (id: string) => (
    <RelatedRow
      key={id}
      icon={creatureIcon(id)[0]}
      label={creatureLabel(id)}
      onClick={() => navTo(id)}
      onContextMenu={(e) => {
        e.preventDefault();
        popup.showText(creatureLabel(id), { x: e.clientX, y: e.clientY });
      }}
    />
  );

  const upgradesBlock = (
    <>
      {upgradeFrom.length > 0 && (
        <div className="space-y-1">
          {sectionTitle(t.wiki.stats.upgradeOf)}
          <div className="space-y-1">{upgradeFrom.map(relRow)}</div>
        </div>
      )}
      {upgradeTo.length > 0 && (
        <div className="space-y-1">
          {sectionTitle(t.wiki.stats.upgradeTo)}
          <div className="space-y-1">{upgradeTo.map(relRow)}</div>
        </div>
      )}
    </>
  );

  const dwellingsBlock = dwNames.length > 0 && (
    <div className="space-y-1">
      {sectionTitle(t.wiki.sections.dwellings)}
      <div className="flex flex-wrap gap-1">
        {dwNames.map((n, i) => (
          <span
            key={`${n}-${i}`}
            className="rounded-full border border-slate-700/60 bg-slate-800/40 px-2 py-0.5 text-[11px] text-slate-300"
          >
            {n}
          </span>
        ))}
      </div>
    </div>
  );

  /** 能力卡片列表（宽态网格与弹窗第三列共用同一份卡片） */
  const abilityCards = abilities.map((ab, i) => {
    const abName = pickName((ab.name as Record<string, string>) ?? {}, lang);
    const abDesc = pickName((ab.description as Record<string, string>) ?? {}, lang) || abName;
    if (!abDesc && !abName) return null;
    // 能力图标（VCMI JsonBonus.cpp:892 取 ability["icon"]）
    const abIcon = ab.icon ? fullImgUrl(cacheDir, ab.icon as string) : undefined;
    return (
      <div key={i} className="flex gap-2 rounded-lg border border-slate-800/60 bg-slate-900/40 p-2">
        <div className="flex-none">
          <div
            className={cn(
              "relative h-[52px] w-[52px] rounded border bg-slate-800/40",
              abilityBorder(String(ab.source ?? "")),
            )}
            // VCMI：能力格右键 = 描述文本气泡
            {...triggers((p) => popup.showText(abDesc, p))}
          >
            {abIcon ? <HdIcon src={abIcon} alt="" className="h-full w-full object-contain" /> : null}
            <span className="absolute inset-x-0 bottom-0 truncate bg-slate-900/80 px-0.5 text-center text-[8px] leading-3 text-slate-400">
              {sourceLabel(ab.source as string)}
            </span>
          </div>
        </div>
        <div className="min-w-0 flex-1">
          <p className="truncate text-[11px] font-medium text-slate-200" title={abName}>
            {abName || abDesc}
          </p>
          <p className="mt-0.5 line-clamp-3 text-[10px] leading-tight text-slate-400" title={abDesc}>
            {abDesc}
          </p>
        </div>
      </div>
    );
  });

  /** 宽态（wiki 右栏）：能力区是两列主干之下的全宽网格，列数随可用宽度 1→3 */
  const abilitiesBlock = abilities.length > 0 && (
    <div className="space-y-2">
      {sectionTitle(t.wiki.creature.abilities)}
      <div className="grid grid-cols-[repeat(auto-fit,minmax(260px,1fr))] gap-2">{abilityCards}</div>
    </div>
  );

  /**
   * 弹窗态：能力区是**第三列**，且限高 5 行、超出在列内滚动。
   *
   * 为什么弹窗要单独一列而不是跟宽态一样铺在下面：能力数是弹窗最大的高度变数
   * （原版 0–15 项），铺在下面时高度随项数线性涨到 1200px+；立为第三列并限高后，
   * 弹窗总高被压成常量（见下方布局注释），且 83.5% 的兵种（≤5 项）根本不会出现滚动条。
   * 滚动语义与官方一致：VCMI `CCreatureWindow.cpp:336-355` 的 BonusesSection 就是
   * "固定高度 + CListBox 内滚"（只露 3 行）。
   */
  const abilityColumn = abilities.length > 0 && (
    <div className="space-y-1">
      {sectionTitle(t.wiki.creature.abilities)}
      <div className="relative">
        <div
          ref={abilityScrollRef}
          className="max-h-[385px] space-y-2 overflow-y-auto pr-0.5"
        >
          {abilityCards}
        </div>
        {/* 底部渐隐：仅当"还有下方内容"时出现，作为可滚动的可见提示 */}
        {abilityCanScroll && (
          <div className="pointer-events-none absolute inset-x-0 bottom-0 h-8 bg-gradient-to-t from-slate-950 to-transparent" />
        )}
      </div>
    </div>
  );

  const descBlock = desc && !compact && (
    <div className="border-t border-slate-700/50 pt-2">
      <p className="max-w-[72ch] text-[11px] leading-relaxed text-slate-400">{desc}</p>
    </div>
  );

  // 沿用启动器暗色主题（不复刻 VCMI 皮革背景），仅复刻布局与内容组织。
  //
  // ── 弹窗态（右键弹窗，880px）：三列 ─────────────────────────────────
  // 能力数是弹窗最大的高度变数（原版 0–15 项）：铺在下面会随项数涨到 1200px+，
  // 立为第三列并限高 5 行后总高被压成常量 ——
  //   40(标题) + max(左264, 中238–326, 能力18+385=403) + 34 = 477px
  //   N≤3 → 338px；N=4 → 400px；N≥5 → 477px（列内滚动）
  // 列宽下限 240px：880−34(padding/border)−24(2×gap) = 822 → 3×274，三列均分，
  // 统计表（144 标签列 + 值列）与升级链名都放得下；窗口更窄时自动退化成 2 列。
  // 滚动语义对齐 VCMI `CCreatureWindow.cpp:336-355`（BonusesSection 固定高 + CListBox 内滚）。
  if (compact) {
    return (
      <div className="space-y-3">
        <h2 className="text-center text-lg font-bold text-amber-200">{name}</h2>

        <div className="grid items-start gap-3 grid-cols-[repeat(auto-fit,minmax(240px,1fr))]">
          <div className="space-y-3">
            {picBlock}
            {costBlock}
            {upgradesBlock}
          </div>
          <div className="space-y-3">
            {statsBlock}
            {dwellingsBlock}
          </div>
          {/* 第三列：能力。N=0 时整列留空（不渲染），保持三列宽度一致、视觉统一 */}
          <div>{abilityColumn}</div>
        </div>
      </div>
    );
  }

  // ── 宽态（wiki 右栏）：两列 ──────────────────────────────────────────
  // **经济/成长跟图走**（招募花费、升级链），**表格式的归右列**（统计、招募建筑），
  // 能力区是两列之下的全宽网格（列数随宽度 1→3）。列宽下限 280px。
  return (
    <div className="mt-3 max-w-[960px] space-y-3">
      <h2 className="text-center text-lg font-bold text-amber-200">{name}</h2>

      <div className="grid items-start gap-3 grid-cols-[repeat(auto-fit,minmax(280px,1fr))]">
        <div className="space-y-3">
          {picBlock}
          {costBlock}
          {upgradesBlock}
        </div>
        <div className="space-y-3">
          {statsBlock}
          {dwellingsBlock}
        </div>
      </div>

      {abilitiesBlock}
      {descBlock}
    </div>
  );
}
