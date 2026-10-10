import { useEffect, useRef, useState } from "react";
import type { LangDict } from "@/i18n/types";
import type { WikiLinkTarget } from "@/lib/modcache";
import { fullImgCandidates, fullImgUrl, pickName } from "@/lib/modcache";
import { getDefGroup } from "@/lib/def/animationCache";
import type { DefFrame } from "@/lib/def/decoder";
import { HdIcon } from "@/shared/HdIcon";
import { HdImg } from "@/shared/HdImg";
import { useHdRate } from "@/shared/useHdRate";
import { useWikiPopup, usePopupTriggers } from "./WikiPopupContext";
import { ANIM, CREA_PREVIEW, MACH_PREVIEW } from "./detailShared";

export function CreatureDetail({
  d,
  lang,
  modId,
  cacheDir,
  modCache,
  t,
  compact,
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

  const sharedIcons = (modCache?.sharedIcons ?? {}) as Record<string, string>;
  const stackIcon = (key: string) => fullImgUrl(cacheDir, sharedIcons[`stackwin/${key}`]);
  const dmgMin = Number(d.damageMin ?? 0);
  const dmgMax = Number(d.damageMax ?? 0);
  const damageText = dmgMin || dmgMax ? `${dmgMin}${dmgMax !== dmgMin ? `-${dmgMax}` : ""}` : undefined;

  // VCMI CStackWindow::MainSection（CCreatureWindow.cpp:657-698）：
  // 属性行按 EStat 枚举顺序固定（攻/防/弹药/伤害/生命/剩余生命/速度/法力），
  // 百科弹窗（非战斗）不显示"剩余生命"。伤害为区间，其余为单值。
  const statRows = [
    { icon: stackIcon("attack"), label: t.wiki.creature.attack, value: d.attack },
    { icon: stackIcon("defense"), label: t.wiki.creature.defense, value: d.defense },
    Number(d.shots) > 0
      ? { icon: stackIcon("shots"), label: t.wiki.creature.shots, value: d.shots }
      : null,
    { icon: stackIcon("damage"), label: t.wiki.creature.damage, value: damageText },
    { icon: stackIcon("health"), label: t.wiki.creature.health, value: d.hitPoints },
    { icon: stackIcon("speed"), label: t.wiki.creature.speed, value: d.speed },
    Number(d.casts) > 0 ? { icon: stackIcon("mana"), label: t.wiki.creature.mana, value: d.casts } : null,
  ].filter(Boolean) as { icon?: string; label: string; value: unknown }[];

  // 士气/运气：MiscWidgets.cpp:662 帧号 = clamp(value + 3, 0, 6)
  const moraleVal = Number(d.morale ?? 0);
  const luckVal = Number(d.luck ?? 0);
  const mlFrame = (v: number) => Math.max(0, Math.min(6, v + 3));
  const moraleIcon = fullImgUrl(cacheDir, sharedIcons[`morale/${mlFrame(moraleVal)}`]);
  const luckIcon = fullImgUrl(cacheDir, sharedIcons[`luck/${mlFrame(luckVal)}`]);
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

  // 沿用启动器暗色主题（不复刻 VCMI 皮革背景），仅复刻布局与内容组织
  return (
    <div className={compact ? "space-y-3" : "mt-3 space-y-3"}>
      <h2 className="text-center text-lg font-bold text-amber-200">{name}</h2>

      {/* MainSection：左生物图（CCreaturePic 130px）| 右属性 + 右上士气/运气 */}
      <div className="flex gap-3">
        <div className="relative h-[130px] w-[130px] flex-none overflow-hidden rounded-xl border border-slate-600/60 bg-[#0a0e1a]">
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

        <div className="relative min-w-0 flex-1">
          {/* MoraleLuckBox：右上角，42×42，图标 + 数值 */}
          <div className="absolute right-0 top-0 flex gap-1.5">
            {[
              { url: moraleIcon, v: moraleVal, title: t.wiki.creature.morale },
              { url: luckIcon, v: luckVal, title: t.wiki.creature.luck },
            ].map((x, i) => (
              <div
                key={i}
                className="relative h-[42px] w-[42px] flex-none"
                title={`${x.title} ${x.v}`}
              >
                {x.url ? (
                  <HdIcon src={x.url} alt="" className="h-full w-full object-contain" />
                ) : (
                  <div className="h-full w-full rounded border border-slate-700/50 bg-slate-800/40" />
                )}
                <span className="absolute bottom-0 right-0 text-[10px] font-semibold text-slate-200">
                  {x.v}
                </span>
              </div>
            ))}
          </div>

          <div className="space-y-[3px] pt-11">
            {statRows.map((s) => (
              <div key={s.label} className="flex items-center gap-1.5">
                {s.icon ? (
                  <HdIcon src={s.icon} alt="" className="h-4 w-4 flex-none object-contain" />
                ) : (
                  <span className="h-4 w-4 flex-none" />
                )}
                <span className="min-w-0 flex-1 truncate text-[11px] text-slate-400">
                  {s.label}
                </span>
                <span className="text-[11px] font-medium text-slate-100">{String(s.value ?? "")}</span>
              </div>
            ))}
          </div>
        </div>
      </div>

      {/* BonusesSection：每行 2 项，每项 = 52×52 图标框（底部来源标签）+ 右侧描述 */}
      {abilities.length > 0 && (
        <div className="mt-3 grid grid-cols-2 gap-x-3 gap-y-1.5">
          {abilities.map((ab, i) => {
            const abName = pickName((ab.name as Record<string, string>) ?? {}, lang);
            const abDesc = pickName((ab.description as Record<string, string>) ?? {}, lang) || abName;
            if (!abDesc) return null;
            // 能力图标（VCMI JsonBonus.cpp:892 取 ability["icon"]）
            const abIcon = ab.icon ? fullImgUrl(cacheDir, ab.icon as string) : undefined;
            return (
              <div
                key={i}
                className="flex cursor-help gap-1.5"
                // VCMI：能力格右键 = 描述文本气泡
                {...triggers((p) => popup.showText(abDesc, p))}
              >
                <div className="relative h-[52px] w-[52px] flex-none rounded border border-slate-700/50 bg-slate-800/40">
                  {abIcon ? (
                    <HdIcon src={abIcon} alt="" className="h-full w-full object-contain" />
                  ) : null}
                  <span className="absolute inset-x-0 bottom-0 truncate bg-slate-900/80 px-0.5 text-center text-[8px] leading-3 text-slate-400">
                    {sourceLabel(ab.source as string)}
                  </span>
                </div>
                <p className="min-w-0 flex-1 text-[10px] leading-tight text-slate-300">{abDesc}</p>
              </div>
            );
          })}
        </div>
      )}

      {desc && !compact && (
        <div className="mt-3 border-t border-slate-700/50 pt-2">
          <p className="text-[11px] leading-relaxed text-slate-400">{desc}</p>
        </div>
      )}
    </div>
  );
}
