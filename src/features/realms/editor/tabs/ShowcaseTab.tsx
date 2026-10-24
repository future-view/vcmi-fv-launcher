import { useEffect, useMemo, useRef, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import {
  ensureShowcaseAudio,
  importRealmAsset,
  pickOpenPath,
  readWikiIndex,
  readTownCards,
  readShowcaseSlides,
  scanRealMods,
  type ShowcaseSlide,
} from "@/lib/tauri";
import { pickName } from "@/lib/modcache";
import { flattenTree } from "@/lib/modcache/buildTargets";
import {
  deckToEntries,
  modPathOf,
  resolveTownIds,
  fallbackSeed,
  randomSeed,
  shuffleBySeed,
  estimateStayMs,
  resolveDwellingIds,
  defaultHeroIds,
  DEFAULT_MAX_SLIDES,
  FOCUS_STEP_MS,
  MUSIC_FADE_MS,
  MUSIC_PRE_FADE_MS,
  DEFAULT_HERO_COUNT,
} from "@/lib/realm/showcase";
import type { RealmShowcase, ShowcaseDeckEntry } from "@/lib/realm/types";
import { backendErrorText } from "@/lib/backendError";
import { Picker, ScopeBadge, Slider, fmtSec } from "./showcaseControls";

type Card = Awaited<ReturnType<typeof readTownCards>>[number];
const dkey = (m: string, f: string) => `${m}/${f}`;

interface Props {
  showcase: RealmShowcase | undefined;
  onChange: (s: RealmShowcase | undefined) => void;
  /** 领域 id：自动编排的缺省随机种子由它派生（与运行时同一口径，保证"预览 = 实播"） */
  realmId: string;
  /** 当前领域启用的模组集合（来自编辑器预设，未落盘也能正确过滤） */
  enabledIds?: Set<string> | null;
  /** 启用组合是否已从领域 modSettings 读出（未就绪前不要展示"全量城镇"的假列表） */
  presetsReady?: boolean;
  /** 当前正在编辑的语言（VCMI 标识）——字幕沿用领域多语言的「切语言单独编辑」机制 */
  curLang?: string;
  /** 上报错误（复用领域编辑器的 toast 通道，避免在 Tab 里用 window.alert） */
  onError?: (msg: string) => void;
}

export function ShowcaseTab({
  showcase,
  onChange,
  realmId,
  enabledIds,
  presetsReady,
  curLang,
  onError,
}: Props) {
  const { t } = useI18n();
  const lang = t.uiLang;
  /** 字幕编辑语言：跟随领域编辑器的「当前编辑语言」 */
  const editLang = curLang ?? lang;
  /**
   * 配置**只有一种写法**（`mode` / `auto.*` / `manual.deck`），没有老字段兼容层。
   * 好处：不存在"扁平字段与容器字段谁优先"这种口径问题；代价是升级后需重配一次。
   */
  const cfg = showcase ?? {};
  const update = (patch: Partial<RealmShowcase>) => onChange({ ...cfg, ...patch });
  /** 只改**自动编排**的选项（写进 `cfg.auto`） */
  const updateAuto = (patch: Partial<NonNullable<RealmShowcase["auto"]>>) =>
    update({ auto: { ...cfg.auto, ...patch } });
  /** 只改**手工编排**的选项（写进 `cfg.manual`） */
  const updateManual = (patch: Partial<NonNullable<RealmShowcase["manual"]>>) =>
    update({ manual: { ...cfg.manual, ...patch } });

  const deck = cfg.manual?.deck ?? [];
  /**
   * 模式由**显式字段**决定（不再由"有没有牌组"推断）。
   * 这样"切回自动"就不必清空牌组 —— 手工编排的内容留着，切回去立刻恢复（平滑切换）。
   */
  const mode: "auto" | "manual" = cfg.mode === "manual" ? "manual" : "auto";

  /** 「包含原版 9 城镇」是**公共选项**：关闭时手工牌组里的原版城镇同样不参与（与自动模式同一口径） */
  const includeCore = cfg.includeCore === true;
  const coreBlockedCount = includeCore
    ? 0
    : deck.filter((d) => d.modId.startsWith("core.")).length;

  const [allCards, setAllCards] = useState<Card[]>([]);
  const [slideMap, setSlideMap] = useState<Map<string, ShowcaseSlide>>(new Map());
  const [loading, setLoading] = useState(false);
  const [dragIdx, setDragIdx] = useState<number | null>(null);
  /** 拖拽过程中指针下方的目标行（用于高亮落点） */
  const [dropIdx, setDropIdx] = useState<number | null>(null);
  const [editIdx, setEditIdx] = useState<number | null>(null);
  const [addOpen, setAddOpen] = useState(false);
  const [addQuery, setAddQuery] = useState("");
  const [auditionOn, setAuditionOn] = useState(false);
  const auditionRef = useRef<HTMLAudioElement | null>(null);
  /** modId → modPath（zip 内音乐试听需先抽取） */
  const modPathRef = useRef<Map<string, string>>(new Map());

  const deckKey = deck.map((d) => dkey(d.modId, d.factionId)).join(",");

  // 候选阵营（与运行时共用 resolveTownIds）+ 手写牌组的素材清单（供多选）
  useEffect(() => {
    let alive = true;
    (async () => {
      setLoading(true);
      const [idx, realMods] = await Promise.all([readWikiIndex(), scanRealMods()]);
      const modPathMap = new Map<string, string>();
      // ⚠️ 与运行时同口径：`flattenTree` 展开子模组，否则子模组 modPath 缺失
      // （阵营音乐解析与试听都会落空 —— 见 ShowcaseOverlay 的同款注释）。
      for (const m of flattenTree(realMods as Record<string, unknown>[])) {
        const id = (m.id ?? m.modId) as string | undefined;
        const mp = m.modPath as string | undefined;
        if (id && mp) modPathMap.set(id, mp);
      }
      modPathRef.current = modPathMap;
      const townIds = resolveTownIds(idx, cfg.includeCore === true, enabledIds);
      const cards = await readTownCards(townIds);
      // 取 slide（兵种/英雄/建筑清单）：
      // 手工牌组按 deck，自动牌组按"运行时将会生成的那一份"（同样的洗牌种子 + 截断）。
      // 有了它才能在两种模式下都显示各阵营的计数、并估算"焦点一轮耗时"。
      const maxSlides = cfg.auto?.maxSlides ?? DEFAULT_MAX_SLIDES;
      const autoList: ShowcaseDeckEntry[] = shuffleBySeed(
        cards.map((c) => ({ modId: c.modId, factionId: c.factionId })),
        cfg.auto?.autoSeed ?? fallbackSeed(realmId),
      );
      const entries: ShowcaseDeckEntry[] =
        mode === "manual" && deck.length > 0
          ? deck
          : maxSlides > 0
            ? autoList.slice(0, maxSlides)
            : autoList;
      const slides = new Map<string, ShowcaseSlide>();
      if (entries.length > 0) {
        const sl = await readShowcaseSlides(deckToEntries(entries, modPathMap));
        for (const s of sl) slides.set(dkey(s.modId, s.factionId), s);
      }
      if (!alive) return;
      setAllCards(cards);
      setSlideMap(slides);
      setLoading(false);
    })();
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    deckKey,
    cfg.includeCore,
    cfg.mode,
    enabledIds,
    cfg.auto?.maxSlides,
    cfg.auto?.autoSeed,
    realmId,
  ]);

  /** 自动牌组（= 运行时会生成的那份，**随机顺序**） */
  const autoEntries = useMemo<ShowcaseDeckEntry[]>(() => {
    const max = cfg.auto?.maxSlides ?? DEFAULT_MAX_SLIDES;
    const base: ShowcaseDeckEntry[] = allCards.map((c) => ({
      modId: c.modId,
      factionId: c.factionId,
    }));
    // 与运行时同一口径（`buildShowcaseDeck`）：有 autoSeed 用种子，否则按领域 id 派生；先洗牌再截断
    const list = shuffleBySeed(base, cfg.auto?.autoSeed ?? fallbackSeed(realmId));
    return max > 0 ? list.slice(0, max) : list;
  }, [allCards, cfg.auto?.maxSlides, cfg.auto?.autoSeed, realmId]);

  /**
   * 每张阵营的**巡礼体检**（列表行与抽屉共用，**与 entries 同长同序**）。
   *
   * 为什么集中算：这些数字原先散在三处（列表行只报总数、抽屉不报、播放器自己又算一遍），
   * 于是"编辑器显示"与"实际播放"经常对不上。现在：
   * - 停留时长走**共用** `estimateStayMs`（播放器调度器用的同一个函数）；
   * - 参与格数按**播放器同一口径**（含 `creatureMode` 过滤、兵种槽位上限 8、自动带出巢穴）。
   *
   * ⚠️ 返回数组与 entries **同长同序**（缺 slide 的记为 `missing`），
   * 这样 `entryStats[i]` 才能直接配到列表第 i 行 / 抽屉的 `editIdx`
   * —— 早先 `stayEstimates` 是"跳过缺失项"的短数组，下标对不上，根本无法按行显示。
   */
  type EntryStat = {
    missing: boolean;
    stayMs: number;
    cycleMs: number;
    /** 参与焦点的格数 = 兵种槽位 + 入序列的建筑 */
    slots: number;
    /** [参与巡礼数, 该阵营总数] */
    creatures: [number, number];
    heroes: [number, number];
    buildings: [number, number];
    /** 本张是否有专属音乐（无则回退领域 BGM） */
    hasMusic: boolean;
    /** 作者在本张覆盖过任意参数或清单 */
    customized: boolean;
    /** **自动带出的招募建筑 id**（按兵种顺序）—— 建筑多选里置顶 + 锁定，播放器清单也由它起头 */
    autoDwellingIds: string[];
  };
  const entryStats = useMemo<EntryStat[]>(() => {
    const entries = mode === "manual" ? deck : autoEntries;
    return entries.map((e) => {
      const missing: EntryStat = {
        missing: true,
        stayMs: 0,
        cycleMs: 0,
        slots: 0,
        creatures: [0, 0],
        heroes: [0, 0],
        buildings: [0, 0],
        hasMusic: false,
        customized: false,
        autoDwellingIds: [],
      };
      const sl = slideMap.get(dkey(e.modId, e.factionId));
      if (!sl) return missing;

      const step = e.focusStepMs ?? cfg.focusStepMs ?? FOCUS_STEP_MS;
      // ⚠️ 三类的"参与清单"必须与**播放器同口径**（Overlay 的 `tourCreatures/Buildings/Heroes`），
      //    否则编辑器里的"预计播放时间"与实播时长会对不上：
      //    ① 兵种：清单非空用清单，否则按「兵种展示」过滤后的全部；
      //    ② 建筑：**不勾就不播放**（默认 0）；
      //    ③ 英雄：清单非空用清单，否则**每个职业各 1 个**（`defaultHeroIds`）。
      const tourC = e.highlightCreatures ?? [];
      const modeCreatures = sl.creatures.filter((c) => {
        const up = !!(c.upgradedFrom?.length);
        // `highlight` 是废弃值（老配置残留）→ 与播放器一致按 `tiers` 处理
        if ((cfg.creatureMode ?? "tiers") === "tiers" || cfg.creatureMode === "highlight")
          return !up;
        if (cfg.creatureMode === "upgraded") return up;
        return true;
      });
      const participants =
        tourC.length > 0
          ? tourC.flatMap((id) => sl.creatures.filter((c) => c.id === id))
          : modeCreatures;
      const tourB = e.highlightBuildings ?? [];
      const tourH = e.highlightHeroes ?? [];
      const heroCount =
        tourH.length > 0 ? tourH.length : defaultHeroIds(sl.heroes).length;
      /**
       * **建筑巡礼清单 = 自动巢穴（按兵种顺序）+ 作者额外勾选**（与播放器同一口径）。
       * ⚠️ 从前这里只数 `e.highlightBuildings` → 自动编排下作者从不勾建筑，
       * 于是编辑器显示 `建筑 0/41`、播放器也真的一个建筑都不巡（实测反馈）。
       */
      const autoDwellings = resolveDwellingIds(sl.buildings, participants, lang);
      /**
       * 建筑巡礼清单 = 作者勾选里**排除招募建筑**（拍板）：招募建筑已在**兵种焦点**那一步用
       * 「招募建筑」次块展示过，再单独当一站播是重复的。
       */
      const tourBuildingsIds = tourB.filter((id) => !autoDwellings.includes(id));
      // 焦点格数 = 三类之和（**不再有 8 格上限**；招募建筑不占建筑格，它跟随兵种那一步）
      const slots = participants.length + tourBuildingsIds.length + heroCount;
      const { stayMs, cycleMs } = estimateStayMs({
        focusSlots: slots,
        focusStepMs: step,
        untilFocus: cfg.untilFocusExhausted,
        durationMs: e.durationMs,
      });
      return {
        missing: false,
        stayMs,
        cycleMs,
        slots,
        creatures: [participants.length, sl.creatures.length],
        heroes: [heroCount, sl.heroes.length],
        buildings: [tourBuildingsIds.length, sl.buildings.length],
        autoDwellingIds: autoDwellings,
        hasMusic: !!(sl.music || sl.musicZip),
        customized: !!(
          e.caption ||
          e.overrideBackground ||
          e.focusStepMs != null ||
          e.durationMs != null ||
          e.transition ||
          e.musicOffsetMs != null ||
          e.musicVolume != null ||
          e.musicUseRealmBgm ||
          tourC.length > 0 ||
          tourH.length > 0 ||
          tourB.length > 0 ||
          (e.keyCreatures ?? []).length > 0 ||
          (e.keyBuildings ?? []).length > 0 ||
          (e.keyHeroes ?? []).length > 0
        ),
      };
    });
  }, [
    mode,
    deck,
    autoEntries,
    slideMap,
    cfg.focusStepMs,
    cfg.untilFocusExhausted,
    cfg.creatureMode,
    cfg.auto?.autoSeed,
    cfg.auto?.maxSlides,
  ]);

  /** 有数据的那些张（用于均值/总计） */
  const scored = useMemo(() => entryStats.filter((s) => !s.missing), [entryStats]);
  /** 单张的平均停留（用于展示"每个城镇停留 ≈ X"） */
  const cycleEstimateMs =
    scored.length > 0 ? Math.round(scored.reduce((a, b) => a + b.stayMs, 0) / scored.length) : null;
  /** 全部阵营跑完一趟的总时长（自动计算，不再手配） */
  const totalEstimateMs = scored.length > 0 ? scored.reduce((a, b) => a + b.stayMs, 0) : null;


  const nameOf = (m: string, f: string, caption?: Record<string, string>) => {
    if (caption?.[editLang]) return caption[editLang];
    const s = slideMap.get(dkey(m, f));
    if (s) return pickName(s.name, lang) || f;
    const c = allCards.find((x) => x.modId === m && x.factionId === f);
    return (c && pickName(c.name, lang)) || f;
  };

  /**
   * 转为手工编排：把**当前自动会生成的那份**原样落成可编辑牌组（"所见即所得"的起点）。
   * 已有手工清单时只切模式，不动内容 —— 所以来回切换不会丢东西。
   */
  const toManual = () =>
    update({
      mode: "manual",
      manual: { ...cfg.manual, deck: deck.length > 0 ? deck : autoEntries.map((e) => ({ ...e })) },
    });
  /**
   * 切回自动：**只改模式、不清手工清单**（清单保留在 `cfg.manual.deck`，切回即恢复），
   * 并按需求口径重新随机一次自动顺序。
   *
   * ⚠️ 这里刻意**不再** `deck: undefined`：从前清空才能让"自动"生效，
   * 于是切一次就丢一份手工编排，作者不敢点 —— 模式与数据分离就是为了解决这个。
   */
  const toAuto = () => {
    update({ mode: "auto", auto: { ...cfg.auto, autoSeed: randomSeed() } });
    setEditIdx(null);
  };
  /** 换一批：只重排顺序，不动牌组（自动态可用） */
  const rerollAuto = () => updateAuto({ autoSeed: randomSeed() });
  /** 彻底丢弃手工清单（**唯一**会清空牌组的入口，显式点击才会发生） */
  const clearManual = () => {
    update({ mode: "auto", manual: { ...cfg.manual, deck: undefined } });
    setEditIdx(null);
  };

  /**
   * 上传本阵营的自定义底图。
   *
   * 文件复制到 `{realm-dir}/assets/showcase/`，**文件名按阵营唯一**（`<modId>-<factionId>`）：
   * 若沿用扁平 `assets/<kind>.<ext>`，多个阵营的底图会互相覆盖（所以 `import_realm_asset`
   * 加了 `sub`/`stem` 覆盖参数）。同一阵营重复上传会替换旧文件并清掉旧扩展名。
   * 底图落在领域目录内 → 能随 `.vcmirealm` 一起导出（§7.1）。
   */
  const pickOverrideBg = async (i: number) => {
    const d = deck[i];
    if (!d) return;
    const p = await pickOpenPath([
      { name: t.realm.showcaseOverrideBg, extensions: ["png", "jpg", "jpeg", "webp"] },
    ]);
    if (!p) return;
    try {
      const rel = await importRealmAsset(
        realmId,
        p,
        "showcaseBg",
        "showcase",
        `${d.modId}-${d.factionId}`,
      );
      patchDeck(i, { overrideBackground: rel });
    } catch (e) {
      onError?.(backendErrorText(e));
    }
  };

  const reorder = (from: number, to: number) => {
    if (from === to || from < 0 || to < 0) return;
    const next = [...deck];
    const [m] = next.splice(from, 1);
    next.splice(to, 0, m);
    updateManual({ deck: next });
    setEditIdx(null);
  };

  /**
   * 手工编排的手柄拖拽排序（落在 ⠿ 手柄上）。
   *
   * ⚠️ **不用 HTML5 拖放**（`draggable` + `dragstart/dragover/drop`）：Tauri 的 WebView 默认
   * 启用原生 drop 处理（`dragDropEnabled` 未关闭），原生处理会吞掉页面内的 drag 事件 ——
   * 外部表现就是"列表怎么拖都不动"。Pointer Events 是纯指针事件，与 webview 原生拖放无关。
   *
   * 指针捕获（`setPointerCapture`）后 `pointermove` 全部回到手柄，因此用
   * `document.elementFromPoint` 反查指针下的目标行（行上带 `data-deck-idx`）。
   */
  const startDrag = (from: number) => (e: React.PointerEvent<HTMLSpanElement>) => {
    e.preventDefault();
    const handle = e.currentTarget;
    try {
      handle.setPointerCapture(e.pointerId);
    } catch {
      /* 某些环境不支持捕获：退化为不带捕获的拖拽（事件仍能到达，只是移出元素后可能丢） */
    }
    setDragIdx(from);
    setDropIdx(from);
    const hit = (x: number, y: number): number | null => {
      const el = document.elementFromPoint(x, y);
      const li = el?.closest?.("[data-deck-idx]") as HTMLElement | null;
      if (!li) return null;
      const n = Number(li.dataset.deckIdx);
      return Number.isNaN(n) ? null : n;
    };
    const onMove = (ev: PointerEvent) => {
      const to = hit(ev.clientX, ev.clientY);
      if (to !== null) setDropIdx(to);
    };
    const finish = (ev: PointerEvent) => {
      handle.removeEventListener("pointermove", onMove);
      handle.removeEventListener("pointerup", finish);
      handle.removeEventListener("pointercancel", finish);
      try {
        handle.releasePointerCapture(ev.pointerId);
      } catch {
        /* 未捕获时释放会抛，忽略 */
      }
      const to = hit(ev.clientX, ev.clientY);
      setDragIdx(null);
      setDropIdx(null);
      if (to !== null) reorder(from, to);
    };
    handle.addEventListener("pointermove", onMove);
    handle.addEventListener("pointerup", finish);
    handle.addEventListener("pointercancel", finish);
  };

  const patchDeck = (i: number, patch: Partial<ShowcaseDeckEntry>) => {
    updateManual({ deck: deck.map((d, j) => (j === i ? { ...d, ...patch } : d)) });
  };

  const removeFaction = (i: number) => {
    updateManual({ deck: deck.filter((_, j) => j !== i) });
    setEditIdx(null);
  };

  // 可添加的阵营：已加入的排除；「包含原版 9 城镇」关闭时原版城镇也排除（加了也不会参与巡礼）
  const availableToAdd = allCards.filter(
    (c) =>
      !deck.some((d) => d.modId === c.modId && d.factionId === c.factionId) &&
      (includeCore || !c.modId.startsWith("core.")),
  );
  const addList = useMemo(() => {
    const kw = addQuery.trim().toLowerCase();
    if (!kw) return availableToAdd;
    return availableToAdd.filter(
      (c) =>
        (pickName(c.name, lang) || "").toLowerCase().includes(kw) ||
        c.modId.toLowerCase().includes(kw) ||
        c.factionId.toLowerCase().includes(kw),
    );
  }, [availableToAdd, addQuery, lang]);

  /** 试听**某个阵营自己**的音乐（起始位置沿用该张的覆盖值）。自动编排没有具体阵营，故只在 ✎ 里提供。 */
  const audition = async (i: number) => {
    const a = auditionRef.current;
    if (!a) return;
    // 已在播 → 再点一次停止
    if (!a.paused) {
      a.pause();
      setAuditionOn(false);
      return;
    }
    const d = deck[i];
    const sl = slideMap.get(dkey(d.modId, d.factionId));
    if (!sl) return;
    let src = "";
    if (sl.music) src = convertFileSrc(sl.music);
    else if (sl.musicZip) {
      const mp = modPathOf(modPathRef.current, sl.musicZip.modId);
      const p = await ensureShowcaseAudio(mp, sl.musicZip.modId, sl.musicZip.rel);
      if (p) src = convertFileSrc(p);
    }
    if (!src) return;
    a.src = src;
    try {
      a.currentTime = (d.musicOffsetMs ?? 0) / 1000;
    } catch {
      /* 解码失败静默降级 */
    }
    a.play()
      .then(() => setAuditionOn(true))
      .catch(() => setAuditionOn(false));
  };

  return (
    // `relative` 给「✎ 配置抽屉」提供定位上下文（抽屉用 absolute inset-0 覆盖整个 Tab）
    <div className="relative space-y-5">
      {/* 顶栏的「▶ 预览巡礼」已移除：它与页面顶部「预览 ▾」下拉里的
          「预览阵营巡礼」是同一件事（直开播放页），两处入口反而让人以为是不同功能。
          统一走顶部的预览按钮组。 */}
      <div className="flex items-start justify-between gap-3">
        <div>
          <h3 className="text-sm font-semibold text-foreground">{t.realm.showcaseEditor}</h3>
          <p className="mt-1 text-xs text-slate-400">{t.realm.showcaseAutoHint}</p>
        </div>
      </div>

      {/* 启用开关 */}
      <div className="rounded-xl border border-slate-800 bg-slate-900/40 p-4">
        <label className="flex cursor-pointer items-center justify-between gap-3">
          <span className="min-w-0">
            <span className="block text-sm text-slate-100">{t.realm.showcaseEnable}</span>
            <span className="mt-0.5 block text-xs text-slate-500">{t.realm.showcaseEnableHint}</span>
          </span>
          <input
            type="checkbox"
            checked={cfg.enabled === true}
            onChange={(e) => {
              const on = e.target.checked;
              // 开启阵营巡礼时重排一次自动顺序（与"从手工切回自动"同一口径）
              if (on) update({ enabled: true, auto: { ...cfg.auto, autoSeed: randomSeed() } });
              else update({ enabled: false });
            }}
            className="h-4 w-4 shrink-0 accent-blue-500"
          />
        </label>

        {/* 进入领域后的**入口行为**（默认关闭 = 停在主界面等玩家点按钮） */}
        <label className="mt-3 flex cursor-pointer items-start gap-3 border-t border-slate-800 pt-3">
          <input
            type="checkbox"
            checked={cfg.afterIntro === true}
            disabled={cfg.enabled !== true}
            onChange={(e) => update({ afterIntro: e.target.checked })}
            className="mt-0.5 h-4 w-4 shrink-0 accent-blue-500 disabled:opacity-40"
          />
          <span className="min-w-0">
            <span className="flex items-center gap-1.5">
              <span className="text-sm text-slate-200">{t.realm.showcaseAfterIntro}</span>
              <ScopeBadge scope="shared" />
            </span>
            <span className="mt-0.5 block text-xs text-slate-500">
              {t.realm.showcaseAfterIntroHint}
            </span>
          </span>
        </label>
      </div>

      {/* 播放设置 */}
      <div className="rounded-xl border border-slate-800 bg-slate-900/40 p-4">
        <div className="mb-3 flex items-center gap-2 text-xs font-medium uppercase tracking-wide text-slate-500">
          {t.realm.showcasePlayback}
          <ScopeBadge scope="shared" />
        </div>

        {/* 「进阶设置」**整体删除**（拍板）：它把"音乐细项"藏在一个折叠区里，
            自动编排下点开只是给音乐加更多选项（层级多余），手工编排下又**看不到抽屉里的**
            逐张覆盖（更容易误解成"这里改了全局都变"）。现在细项直接放在它生效的地方：
            音乐 → 城镇抽屉（逐张）。 */}
        <div className="space-y-4">
          <label className="flex cursor-pointer items-center justify-between gap-3">
            <span className="min-w-0">
              <span className="block text-sm text-slate-200">{t.realm.showcaseAuto}</span>
              <span className="mt-0.5 block text-xs text-slate-500">{t.realm.showcaseManualMode}</span>
            </span>
            <input
              type="checkbox"
              checked={cfg.autoPlay !== false}
              onChange={(e) => update({ autoPlay: e.target.checked })}
              className="h-4 w-4 shrink-0 accent-blue-500"
            />
          </label>

          {/* 「每个城镇停留」已改为**自动计算**（= 参与焦点的格数 × 焦点切换间隔），不再手配 */}
          <div className="rounded-lg bg-slate-800/40 px-2.5 py-1.5">
            <div className="flex items-center justify-between gap-2">
              <span className="min-w-0 text-sm text-slate-200">
                {t.realm.showcaseFactionDuration}
              </span>
              <span className="shrink-0 tabular-nums text-xs text-slate-300">
                {cycleEstimateMs !== null
                  ? t.realm.showcaseStayAuto.replace("{v}", fmtSec(cycleEstimateMs, t.realm.showcaseSeconds, t.realm.showcaseMinutes))
                  : "—"}
              </span>
            </div>
            <p className="mt-0.5 text-xs text-slate-500">
              {cycleEstimateMs !== null
                ? t.realm.showcaseStayAutoHint
                : t.realm.showcaseCycleEstimateUnknown}
            </p>
          </div>
          {totalEstimateMs !== null && (
            <p className="text-xs text-slate-400">
              {t.realm.showcaseTotalDuration.replace("{v}", fmtSec(totalEstimateMs, t.realm.showcaseSeconds, t.realm.showcaseMinutes))}
            </p>
          )}

          {/* 焦点节奏。
              ⚠️ 2026-09-21 删除「兵种轮播」总开关（`focusRotation`）：它曾是"关掉就没有焦点巡礼"，
              但播放器**从未读取**它（焦点轮播一直生效）→ 关掉它只会把下面这些设置藏起来，
              是纯粹的误导。 */}
          <div className="space-y-3">
            <Slider
              label={t.realm.showcaseFocusStep}
              hint={t.realm.showcaseFocusStepHint}
              min={400}
              max={10000}
              step={200}
              value={cfg.focusStepMs ?? FOCUS_STEP_MS}
              onChange={(v) => update({ focusStepMs: v })}
            />
            <label className="flex cursor-pointer items-start gap-2 text-sm text-slate-300">
              <input
                type="checkbox"
                checked={cfg.untilFocusExhausted !== false}
                onChange={(e) => update({ untilFocusExhausted: e.target.checked })}
                className="mt-0.5 h-4 w-4 shrink-0 accent-blue-500"
              />
              <span className="min-w-0">
                <span className="block">{t.realm.showcaseUntilFocus}</span>
                <span className="mt-0.5 block text-xs text-slate-500">
                  {t.realm.showcaseUntilFocusHint}
                </span>
              </span>
            </label>
            {/* 「走完一轮是否重来」**只在关闭「轮完一轮才切」时**才有意义：
                开着时停留时长恰好等于一轮，永远走不到第二轮（给了就是误导）。 */}
            {cfg.untilFocusExhausted === false && (
              <label className="flex cursor-pointer items-start gap-2 pl-6 text-sm text-slate-300">
                <input
                  type="checkbox"
                  checked={cfg.focusLoop !== false}
                  onChange={(e) => update({ focusLoop: e.target.checked })}
                  className="mt-0.5 h-4 w-4 shrink-0 accent-blue-500"
                />
                <span className="min-w-0">
                  <span className="block">{t.realm.showcaseFocusLoop}</span>
                  <span className="mt-0.5 block text-xs text-slate-500">
                    {t.realm.showcaseFocusLoopHint}
                  </span>
                </span>
              </label>
            )}
          </div>

          {/* 「最大停留上限」已**整体删除**（拍板）：停留时长 = 焦点格数 × 间隔，
              本来就是从巡礼清单推算出来的（编辑器里每行都写着"预计播放时间"）。
              它当年是"防止单张过长"的自我防御阈值 —— 但要短就减格子或调间隔，
              那两个都是作者看得见的数字，多一个隐藏的帽子只会制造"我设了 45s 却播了 30s"的疑问。 */}

          <label className="flex items-center justify-between gap-3">
            <span className="min-w-0">
              <span className="block text-sm text-slate-200">{t.realm.showcaseTransition}</span>
              <span className="mt-0.5 block text-xs text-slate-500">
                {t.realm.showcaseTransitionHint}
              </span>
            </span>
            <select
              value={cfg.transition ?? "fade"}
              onChange={(e) => update({ transition: e.target.value as RealmShowcase["transition"] })}
              className="shrink-0 rounded-lg border border-slate-700 bg-slate-950 px-2 py-1 text-sm text-slate-100"
            >
              <option value="fade">{t.realm.showcaseTransitionFade}</option>
              <option value="slide">{t.realm.showcaseTransitionSlide}</option>
              <option value="dissolve">{t.realm.showcaseTransitionDissolve}</option>
            </select>
          </label>

          <label className="flex items-center justify-between gap-3">
            <span className="min-w-0">
              <span className="block text-sm text-slate-200">{t.realm.showcaseCreatureMode}</span>
              <span className="mt-0.5 block text-xs text-slate-500">
                {t.realm.showcaseCreatureModeHint}
              </span>
            </span>
            <select
              // 「仅重点」已抽成下面的独立开关，下拉里不再重复（同值时显示"未升级"）
              value={cfg.creatureMode === "highlight" ? "tiers" : (cfg.creatureMode ?? "tiers")}
              onChange={(e) => update({ creatureMode: e.target.value as RealmShowcase["creatureMode"] })}
              className="shrink-0 rounded-lg border border-slate-700 bg-slate-950 px-2 py-1 text-sm text-slate-100"
            >
              <option value="tiers">{t.realm.showcaseCreatureTiers}</option>
              <option value="upgraded">{t.realm.showcaseCreatureUpgraded}</option>
              <option value="all">{t.realm.showcaseCreatureAll}</option>
            </select>
          </label>

          {/* 英雄的三项细调（职业均衡 / 随机洗牌 / 轮换间隔）**已全部删除**：
              英雄进了主序列（兵种 → 建筑 → 英雄），由「焦点切换间隔」统一驱动，
              "随机轮换池"的概念不存在了；默认清单 = 每个职业各 1 个（`defaultHeroIds`）。 */}

          {/* 走完一趟是否从头再来（默认关闭 = 每个阵营各展示一次后自动结束） */}
          <label className="flex cursor-pointer items-center justify-between gap-3">
            <span className="min-w-0">
              <span className="block text-sm text-slate-200">{t.realm.showcaseLoop}</span>
              <span className="mt-0.5 block text-xs text-slate-500">{t.realm.showcaseLoopHint}</span>
            </span>
            <input
              type="checkbox"
              checked={cfg.loop === true}
              onChange={(e) => update({ loop: e.target.checked })}
              className="h-4 w-4 shrink-0 accent-blue-500"
            />
          </label>
        </div>
      </div>

      {/* 音乐 */}
      <div className="rounded-xl border border-slate-800 bg-slate-900/40 p-4">
        <div className="mb-3 text-xs font-medium uppercase tracking-wide text-slate-500">
          {t.realm.showcaseMusic}
        </div>
        <div className="space-y-4">
          <label className="flex cursor-pointer items-center justify-between gap-3">
            <span className="min-w-0">
              <span className="block text-sm text-slate-200">{t.realm.showcaseMusicFaction}</span>
              <span className="mt-0.5 block text-xs text-slate-500">
                {t.realm.showcaseMusicFactionHint}
              </span>
            </span>
            <input
              type="checkbox"
              checked={cfg.musicFaction !== false}
              onChange={(e) => update({ musicFaction: e.target.checked })}
              className="h-4 w-4 shrink-0 accent-blue-500"
            />
          </label>


          {/* 音乐细项（淡入时长 / 起始位置 / 提前淡出）**已下沉到城镇抽屉**（拍板）：
              它们只在"某一站想特别处理"时才需要改，而**领域级放一份**既没有入口（幽灵字段）
              又让人以为"改了全局都变"。现在：默认值走代码常量，要改就在对应的城镇抽屉里改。 */}          <p className="rounded-lg bg-slate-800/40 px-2.5 py-1.5 text-xs text-slate-400">
            ⓘ {t.realm.showcaseNoMusic}
          </p>
        </div>
      </div>

      {/* 巡礼顺序：显式 自动 / 手工（模式存 `cfg.mode`，切换**不丢数据**） */}
      <div className="rounded-xl border border-slate-800 bg-slate-900/40 p-4">
        <div className="mb-3 flex items-center gap-3">
          <span className="text-sm text-slate-200">{t.realm.showcaseOrder}</span>
          <div className="flex gap-1.5 rounded-lg border border-slate-700 bg-slate-950 p-1">
            <button
              type="button"
              onClick={toAuto}
              className={cn(
                "rounded-md px-3 py-1 text-xs",
                mode === "auto"
                  ? "bg-blue-600 text-white"
                  : "text-slate-300 hover:bg-slate-800/60",
              )}
            >
              {t.realm.showcaseDeckModeAuto}
            </button>
            <button
              type="button"
              onClick={toManual}
              className={cn(
                "rounded-md px-3 py-1 text-xs",
                mode === "manual"
                  ? "bg-blue-600 text-white"
                  : "text-slate-300 hover:bg-slate-800/60",
              )}
            >
              {t.realm.showcaseDeckModeManual}
            </button>
          </div>
        </div>

        {/* 巡礼序列是**固定**的（拍板）：兵种 → 建筑 → 英雄。写在最显眼处，避免作者找"顺序"配置 */}
        <p className="mb-2 rounded-lg bg-slate-950/60 px-2.5 py-1.5 text-xs text-slate-400">
          {t.realm.showcaseTourOrder}
        </p>

        {/* 两种模式的**差异**必须写在切换处：不说清的话「自动/手工」只是两个词，
            作者不知道手工会失去什么、自动会随机什么，也就不敢切。 */}
        <p className="mb-3 rounded-lg bg-slate-800/40 px-2.5 py-1.5 text-xs leading-relaxed text-slate-400">
          {mode === "auto"
            ? t.realm.showcaseModeExplainAuto.replace("{n}", String(autoEntries.length))
            : t.realm.showcaseModeExplainManual}
        </p>
        {/* 自动态下手工清单**仍然保留**：明确告诉作者"没丢、可回"，否则他会以为切走即清空 */}
        {mode === "auto" && deck.length > 0 && (
          <div className="mb-3 flex flex-wrap items-center gap-2 rounded-lg border border-slate-700/70 bg-slate-950/60 px-2.5 py-1.5">
            <span className="min-w-0 text-xs text-slate-300">
              {t.realm.showcaseManualKept.replace("{n}", String(deck.length))}
            </span>
            <button
              type="button"
              onClick={clearManual}
              className="ml-auto shrink-0 rounded border border-slate-700 px-2 py-0.5 text-[11px] text-slate-400 hover:bg-slate-800/60 hover:text-slate-200"
            >
              {t.realm.showcaseClearManual}
            </button>
          </div>
        )}

        {/* **公共选项**：手工 / 自动两种模式都生效，所以放在分段开关之外，
            不能塞进自动分支里（否则手工编排时会被误认为"已失效"）。 */}
        <label className="mb-3 flex cursor-pointer items-center gap-2 rounded-lg bg-slate-950/50 px-3 py-2 text-sm text-slate-300">
          <input
            type="checkbox"
            checked={cfg.includeCore === true}
            onChange={(e) => update({ includeCore: e.target.checked })}
            className="h-4 w-4 shrink-0 accent-blue-500"
          />
          <span className="min-w-0">
            {t.realm.showcaseIncludeCore}
            <span className="ml-1 text-xs text-slate-500">{t.realm.showcaseIncludeCoreHint}</span>
            <span className="ml-1 text-[11px] text-slate-600">{t.realm.showcaseIncludeCoreManual}</span>
          </span>
          {/* 共用项：两种模式都生效（手工模式下是"过滤不删除"） */}
          <ScopeBadge scope="shared" />
        </label>

        {mode === "auto" ? (
          <>
            <p className="mb-2 text-xs text-slate-500">{t.realm.showcaseAutoHint}</p>
            {presetsReady === false ? (
              <div className="rounded-lg border border-slate-800 bg-slate-950/60 px-3 py-4 text-xs text-slate-500">
                {t.realm.showcaseLoadingPreset}
              </div>
            ) : (
              <div className="max-h-52 space-y-1 overflow-y-auto text-xs text-slate-400">
                {autoEntries.map((e) => (
                  <div key={dkey(e.modId, e.factionId)} className="truncate">
                    {nameOf(e.modId, e.factionId)} · {e.modId}
                  </div>
                ))}
                {!loading && autoEntries.length === 0 && (
                  <div className="text-slate-500">{t.realm.showcaseEmpty}</div>
                )}
              </div>
            )}
            <div className="mt-3 flex flex-wrap items-center gap-2">
              <button
                type="button"
                onClick={toManual}
                disabled={presetsReady === false || autoEntries.length === 0}
                title={t.realm.showcaseToManualHint.replace("{n}", String(autoEntries.length))}
                className="rounded-lg border border-amber-500/50 bg-amber-500/15 px-3 py-1.5 text-xs text-amber-200 hover:bg-amber-500/25 disabled:cursor-not-allowed disabled:opacity-50"
              >
                ＋ {t.realm.showcaseToManual}
              </button>
              {/* 开启「每次进入都换一批」后，🎲 就不再有意义（顺序每次都现算，
                  保存里的 `autoSeed` 只用于编辑器展示）→ 直接隐藏，避免"点了没反应"的困惑。 */}
              {cfg.auto?.rerollOnEnter !== true && (
                <button
                  type="button"
                  onClick={rerollAuto}
                  disabled={presetsReady === false || autoEntries.length === 0}
                  className="rounded-lg border border-slate-700 px-3 py-1.5 text-xs text-slate-300 hover:bg-slate-800/60 disabled:cursor-not-allowed disabled:opacity-50"
                >
                  🎲 {t.realm.showcaseReroll}
                </button>
              )}
            </div>
            <p className="mt-2 text-xs text-slate-500">{t.realm.showcaseAutoRandomHint}</p>
            {/* 「每次进入巡礼都换一批」：默认顺序是**写死的**（见 `autoSeed`），
                开启后改为每次打开巡礼现算一个种子（只在本次会话生效、不落盘）。 */}
            <label className="mt-2 flex cursor-pointer items-start gap-2">
              <input
                type="checkbox"
                checked={cfg.auto?.rerollOnEnter === true}
                onChange={(e) => updateAuto({ rerollOnEnter: e.target.checked })}
                className="mt-0.5 h-4 w-4 shrink-0 accent-blue-500"
              />
              <span className="min-w-0">
                <span className="flex items-center gap-1.5">
                  <span className="text-sm text-slate-200">{t.realm.showcaseRerollEachEntry}</span>
                  <ScopeBadge scope="auto" />
                </span>
                <span className="mt-0.5 block text-xs text-slate-500">
                  {t.realm.showcaseRerollEachEntryHint}
                </span>
              </span>
            </label>
            <div className="mt-4 space-y-3 border-t border-slate-800 pt-3">
              <label className="flex items-center justify-between gap-3">
                <span className="min-w-0">
                  <span className="flex items-center gap-1.5">
                    <span className="text-sm text-slate-200">{t.realm.showcaseMaxSlides}</span>
                    {/* 仅自动：手工编排时顺序与数量由清单决定，本项不参与 */}
                    <ScopeBadge scope="auto" />
                  </span>
                  <span className="mt-0.5 block text-xs text-slate-500">
                    {t.realm.showcaseMaxSlidesHint}
                  </span>
                </span>
                <input
                  type="number"
                  min={0}
                  max={48}
                  value={cfg.auto?.maxSlides ?? DEFAULT_MAX_SLIDES}
                  onChange={(e) =>
                    updateAuto({
                      maxSlides: Math.max(0, Math.min(48, Number(e.target.value) || 0)),
                    })
                  }
                  className="w-20 rounded-lg border border-slate-700 bg-slate-950 px-2 py-1 text-right text-sm text-slate-100"
                />
              </label>
            </div>
          </>
        ) : (
          <>
            <div className="mb-2 flex items-center gap-2">
              <p className="min-w-0 text-xs text-slate-500">{t.realm.showcaseDeckHint}</p>
              {/* 仅手工：清单、顺序、逐张微调都只在这一模式下生效 */}
              <ScopeBadge scope="manual" />
            </div>
            <ul className="space-y-1.5">
              {deck.map((d, i) => {
                const sl = slideMap.get(dkey(d.modId, d.factionId));
                /** 公共选项关闭时，本行是原版城镇 → 不参与巡礼（数据保留，只是被挡掉） */
                const coreBlocked = !includeCore && d.modId.startsWith("core.");

                // 巡礼兵种 → 自动带出的巢穴建筑（编辑器里置顶并锁定；播放器用同一个
                // `dwellingsOfCreatures` 自动合并，两边同源 → 所见即所得）。
                const tourC = d.highlightCreatures ?? [];
                const modeCreatures = (sl?.creatures ?? []).filter((c) => {
                  const up = !!(c.upgradedFrom?.length);
                  if ((cfg.creatureMode ?? "tiers") === "tiers") return !up;
                  if (cfg.creatureMode === "upgraded") return up;
                  return true;
                });
                /** 本张的巡检数据（停留时长/参与数/音乐来源），见 `entryStats` */
                const st = entryStats[i];
                const stepSec =
                  Math.round(
                    ((d.focusStepMs ?? cfg.focusStepMs ?? FOCUS_STEP_MS) / 1000) * 10,
                  ) / 10;
                return (
                  <li
                    key={`${dkey(d.modId, d.factionId)}-${i}`}
                    data-deck-idx={i}
                    className={cn(
                      "rounded-lg border border-slate-700/70 bg-slate-950/60 px-2.5 py-2 transition-colors",
                      coreBlocked && "opacity-60",
                      dragIdx === i && "opacity-50",
                      // 落点提示：拖拽中指针所在的那一行
                      dragIdx !== null &&
                        dropIdx === i &&
                        dragIdx !== i &&
                        "border-blue-400/80 bg-blue-500/10",
                    )}
                  >
                    {/* 行一：标识 + **预计耗时**（最关键的节奏信息）+ 操作。
                        耗时放行一（而不是挤在计数里）：它是"这一张要播多久"的答案，
                        被硬上限截断时标黄 —— 否则"最后几个兵种没轮到"根本看不出原因。 */}
                    <div className="flex items-center gap-2">
                      <span
                        onPointerDown={startDrag(i)}
                        className="cursor-grab touch-none select-none px-1 text-slate-500 hover:text-slate-300 active:cursor-grabbing"
                        title={t.realm.showcaseDragHint}
                      >
                        ⠿
                      </span>
                      <span className="min-w-0 flex-1 truncate text-sm text-slate-100">
                        {nameOf(d.modId, d.factionId, d.caption)}
                      </span>
                      {st?.customized && (
                        <span
                          className="shrink-0 rounded bg-slate-700/50 px-1.5 py-0.5 text-[10px] text-slate-300"
                          title={t.realm.showcaseCustomizedHint}
                        >
                          {t.realm.showcaseCustomized}
                        </span>
                      )}
                      {coreBlocked && (
                        <span className="shrink-0 rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] text-amber-300">
                          {t.realm.showcaseCoreExcluded}
                        </span>
                      )}
                      {st && !st.missing && (
                        <span
                          className="shrink-0 whitespace-nowrap tabular-nums text-[11px] text-slate-300"
                          title={t.realm.showcaseStayEstimateHint
                            .replace("{n}", String(st.slots))
                            .replace("{s}", String(stepSec))}
                        >
                          ⏱ {fmtSec(st.stayMs, t.realm.showcaseSeconds, t.realm.showcaseMinutes)}
                        </span>
                      )}
                      <button
                        type="button"
                        onClick={() => setEditIdx(editIdx === i ? null : i)}
                        className="shrink-0 rounded border border-slate-700 px-2 py-0.5 text-xs text-slate-300 hover:bg-slate-800/60"
                      >
                        {t.realm.showcaseEdit}
                      </button>
                      <button
                        type="button"
                        onClick={() => removeFaction(i)}
                        className="shrink-0 rounded border border-rose-800/60 px-2 py-0.5 text-xs text-rose-300 hover:bg-rose-900/30"
                      >
                        ✕
                      </button>
                    </div>

                    {/* 行二：id + **参与数/总数** + 音乐来源。
                        列"分子/分母"而不是只列总数：分母 =「该阵营有多少」，
                        分子 =「这次巡礼实际用多少」—— 只显示 14 会让人以为 14 个都上，
                        而勾了 3 个兵种时其实只巡礼 3 个。 */}
                    <div className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-0.5 pl-6 text-[10px] text-slate-500">
                      <span className="max-w-[180px] truncate font-mono">{d.modId}</span>
                      {st && !st.missing ? (
                        <>
                          <span className="whitespace-nowrap">
                            {t.realm.showcaseCreatures} {st.creatures[0]}/{st.creatures[1]}
                          </span>
                          <span className="whitespace-nowrap">
                            {t.realm.showcaseHeroes} {st.heroes[0]}/{st.heroes[1]}
                          </span>
                          <span className="whitespace-nowrap">
                            {t.realm.showcaseBuildings} {st.buildings[0]}/{st.buildings[1]}
                          </span>
                          <span
                            className={cn(
                              "ml-auto whitespace-nowrap",
                              st.hasMusic ? "text-slate-400" : "text-slate-600",
                            )}
                            title={
                              st.hasMusic
                                ? t.realm.showcaseMusicOwnHint
                                : t.realm.showcaseMusicFallbackHint
                            }
                          >
                            ♪ {st.hasMusic ? t.realm.showcaseMusicOwn : t.realm.showcaseMusicFallback}
                          </span>
                        </>
                      ) : (
                        <span className="text-slate-600">
                          {loading ? "…" : t.realm.showcaseCycleEstimateUnknown}
                        </span>
                      )}
                    </div>

                    {/* ✎ 配置面板走**抽屉**：内联展开会被启动器可视高度截断（配置项太多）。
                        ⚠️ 必须 `fixed`，不能用 `absolute`：
                        ① `absolute` 以 Tab 根节点为定位上下文，而 Tab 在滚动容器内且被 `max-w-xl` 居中
                           → 抽屉右边界落在 Tab 右边界，**不是窗口最右侧**（错位根源）；
                        ② 抽屉的 `h-full` 取的是「Tab 内容高度」→ 反过来把整页撑高。
                        `fixed` 相对视口定位且不参与布局流，两个问题一起解决。
                        ⚠️ z-index 必须 **> 135**：领域覆盖层顶部有一条 `z-[135]` 的窗口拖拽条
                           （见 `RealmShell`），低于它时抽屉头部（含 ✕）会被拖拽条盖住点不动。 */}
                    {editIdx === i && (
                      <div className="fixed inset-0 z-[150] flex justify-end">
                        <div className="absolute inset-0 bg-black/40" onClick={() => setEditIdx(null)} />
                        <div className="relative flex h-full w-[460px] max-w-[92vw] flex-col border-l border-slate-800 bg-slate-900 shadow-2xl">
                          <div className="flex items-start justify-between gap-2 border-b border-slate-800 px-3 py-2.5">
                            <div className="min-w-0">
                              <div className="truncate text-sm font-semibold text-slate-100">
                                {nameOf(d.modId, d.factionId, d.caption)}
                              </div>
                              <div className="mt-0.5 text-[11px] text-slate-500">{d.modId}</div>
                            </div>
                            <button
                              type="button"
                              onClick={() => setEditIdx(null)}
                              className="shrink-0 rounded border border-slate-700 px-2 py-0.5 text-xs text-slate-300 hover:bg-slate-800/60"
                            >
                              ✕
                            </button>
                          </div>
                          <div className="min-h-0 flex-1 space-y-3 overflow-y-auto px-3 py-3">
                        {/* 字幕：沿用领域多语言「切语言单独编辑」机制 */}
                        <div className="space-y-1">
                          <div className="flex items-center justify-between">
                            <span className="text-xs text-slate-400">{t.realm.showcaseCaption}</span>
                            <span className="rounded bg-blue-500/15 px-1.5 py-0.5 text-[10px] text-blue-300">
                              {t.realm.showcaseEditLang}: {editLang}
                            </span>
                          </div>
                          <input
                            value={d.caption?.[editLang] ?? ""}
                            onChange={(e) =>
                              patchDeck(i, {
                                caption: { ...(d.caption ?? {}), [editLang]: e.target.value },
                              })
                            }
                            placeholder={nameOf(d.modId, d.factionId)}
                            className="w-full rounded-lg border border-slate-700 bg-slate-950 px-2 py-1 text-sm text-slate-100"
                          />
                          <p className="text-[11px] text-slate-500">{t.realm.showcaseEditLangHint}</p>
                        </div>

                        {/* 自定义底图：可以直接「上传」（复制进领域目录并回填路径），也可以手填相对路径 */}
                        <div className="space-y-1">
                          <span className="text-xs text-slate-400">{t.realm.showcaseOverrideBg}</span>
                          <div className="flex items-center gap-2">
                            <button
                              type="button"
                              onClick={() => void pickOverrideBg(i)}
                              className="shrink-0 rounded-lg border border-slate-700 px-2.5 py-1 text-xs text-slate-300 hover:bg-slate-800/60"
                            >
                              {t.realm.showcaseOverrideBgPick}
                            </button>
                            <input
                              value={d.overrideBackground ?? ""}
                              onChange={(e) =>
                                patchDeck(i, { overrideBackground: e.target.value || undefined })
                              }
                              placeholder="assets/showcase/xxx.png"
                              className="min-w-0 flex-1 rounded-lg border border-slate-700 bg-slate-950 px-2 py-1 text-sm text-slate-100"
                            />
                          </div>
                          <p className="text-[11px] leading-relaxed text-slate-500">
                            {t.realm.showcaseOverrideBgHint}
                          </p>
                        </div>

                        {/* 高亮：固定选项多选，不用手打 id */}
                        {[
                          {
                            key: "highlightCreatures" as const,
                            label: t.realm.showcaseHighlightCreatures,
                            hint: t.realm.showcaseHighlightCreaturesHint,
                            markKey: "keyCreatures" as const,
                            placeholder: t.realm.showcaseSearchCreature,
                            items: (sl?.creatures ?? []).map((c) => ({
                              id: c.id,
                              label: pickName(c.name, lang) || c.id,
                            })),
                          },
                          {
                            key: "highlightHeroes" as const,
                            label: t.realm.showcaseHighlightHeroes,
                            hint: t.realm.showcaseHighlightHeroesHint,
                            // 三类**统一**都有「★ 重点」列（拍板第 5 点）：焦点项是★时巡礼区加金色描边
                            markKey: "keyHeroes" as const,
                            placeholder: t.realm.showcaseSearchHero,
                            items: (sl?.heroes ?? []).map((h) => ({
                              id: h.id,
                              label: pickName(h.name, lang) || h.id,
                              // 副文本＝职业名：英雄列表只给名字/ID 时很难认人
                              sub:
                                typeof h.className === "string"
                                  ? h.className
                                  : pickName(h.className, lang),
                            })),
                          },
                          {
                            key: "highlightBuildings" as const,
                            label: t.realm.showcaseHighlightBuildings,
                            hint: t.realm.showcaseHighlightBuildingsHint,
                            markKey: "keyBuildings" as const,
                            placeholder: t.realm.showcaseSearchBuilding,
                            // **巡礼兵种的招募建筑**：自动置顶 + 固定选中 + **不可取消**（拍板），
                            // 顺序与兵种勾选顺序一致（`st.autoDwellingIds` 就是按兵种顺序解析的）。
                            locked: new Set(st?.autoDwellingIds ?? []),
                            items: (() => {
                              const auto = st?.autoDwellingIds ?? [];
                              const all = sl?.buildings ?? [];
                              const byId = new Map(all.map((b) => [b.id, b]));
                              const head = auto.flatMap((id) => (byId.has(id) ? [byId.get(id)!] : []));
                              return [...head, ...all.filter((b) => !auto.includes(b.id))].map((b) => ({
                                id: b.id,
                                label: pickName(b.name, lang) || b.id,
                              }));
                            })(),
                          },
                        ].map((g) => {
                          // 选中状态 = 作者清单 + **自动项**（自动项不写回配置：播放器会自行合并，
                          // 这样"换兵种"时自动项跟着变，不会在配置里留下过期 id）
                          const autoIds = "locked" in g ? g.locked : undefined;
                          const selected = new Set([
                            ...(d[g.key] ?? []),
                            ...(autoIds ? [...autoIds] : []),
                          ]);
                          return (
                            <div key={g.key} className="space-y-1">
                              <span className="block text-xs text-slate-400">{g.label}</span>
                              <p className="text-[11px] leading-relaxed text-slate-500">{g.hint}</p>

                              {g.items.length > 0 ? (
                                <Picker
                                  items={g.items}
                                  selected={selected}
                                  // 自动项（巡礼兵种带出的招募建筑）：**勾选固定、不可取消**
                                  lockedIds={autoIds}
                                  // 「★ 重点」列：**三类都有**（拍板：统一；焦点项是★时巡礼区加金色描边）
                                  markIds={new Set(d[g.markKey] ?? [])}
                                  onMark={(id) => {
                                    const mk = g.markKey;
                                    const cur = d[mk] ?? [];
                                    patchDeck(i, {
                                      [mk]: cur.includes(id)
                                        ? cur.filter((x) => x !== id)
                                        : [...cur, id],
                                    } as Partial<ShowcaseDeckEntry>);
                                  }}
                                  placeholder={g.placeholder}
                                  emptyText={t.realm.showcaseNoFactionToAdd}
                                  // 批量操作（全选 / 随机 / 清空）**不写自动项**：
                                  // 自动项由兵种清单推导，写进配置会在换兵种时留下过期 id。
                                  onBulk={(ids) =>
                                    patchDeck(i, {
                                      [g.key]: ids.filter((x) => !autoIds?.has(x)),
                                    } as Partial<ShowcaseDeckEntry>)
                                  }
                                  onToggle={(id) => {
                                    if (autoIds?.has(id)) return; // 自动项不可取消（按钮已 disabled，双保险）
                                    const cur = d[g.key] ?? [];
                                    patchDeck(i, {
                                      [g.key]: cur.includes(id)
                                        ? cur.filter((x) => x !== id)
                                        : [...cur, id],
                                    } as Partial<ShowcaseDeckEntry>);
                                  }}
                                />
                              ) : (
                                <div className="text-[11px] text-slate-500">—</div>
                              )}
                            </div>
                          );
                        })}

                        {/* 每阵营可覆盖的播放参数（三级回退：本张 → 领域 → 默认） */}
                        <div className="space-y-3 rounded-lg border border-slate-800 bg-slate-950/40 p-2">
                          <Slider
                            label={t.realm.showcaseFocusStep}
                            hint={t.realm.showcaseDeckFocusStepHint}
                            min={400}
                            max={10000}
                            step={200}
                            value={d.focusStepMs ?? cfg.focusStepMs ?? FOCUS_STEP_MS}
                            onChange={(v) => patchDeck(i, { focusStepMs: v })}
                          />
                          <label className="flex items-center justify-between gap-3">
                            <span className="min-w-0">
                              <span className="block text-xs text-slate-400">
                                {t.realm.showcaseTransition}
                              </span>
                              <span className="mt-0.5 block text-[11px] text-slate-500">
                                {t.realm.showcaseDeckTransitionHint}
                              </span>
                            </span>
                            <select
                              value={d.transition ?? ""}
                              onChange={(e) =>
                                patchDeck(i, {
                                  transition:
                                    (e.target.value || undefined) as ShowcaseDeckEntry["transition"],
                                })
                              }
                              className="shrink-0 rounded-lg border border-slate-700 bg-slate-950 px-2 py-1 text-xs text-slate-100"
                            >
                              <option value="">{t.realm.showcaseFollowRealm}</option>
                              <option value="fade">{t.realm.showcaseTransitionFade}</option>
                              <option value="slide">{t.realm.showcaseTransitionSlide}</option>
                              <option value="dissolve">{t.realm.showcaseTransitionDissolve}</option>
                            </select>
                          </label>
                        </div>

                        {/* 本张**预计播放时间**：放在「音乐」之前 —— 它决定音乐能播多久，
                            也是"这一张配了 10 个兵种会不会太长"的答案。
                            数值来自共用 `estimateStayMs` = 播放器真正切走的时刻，所见即所得。 */}
                        {st && !st.missing ? (
                          <div className="space-y-1 rounded-lg border border-slate-800 bg-slate-950/40 p-2">
                            <div className="flex items-center justify-between gap-2">
                              <span className="text-xs text-slate-400">
                                {t.realm.showcaseStayEstimate}
                              </span>
                              <span className="shrink-0 tabular-nums text-sm text-slate-100">
                                {fmtSec(st.stayMs, t.realm.showcaseSeconds, t.realm.showcaseMinutes)}
                              </span>
                            </div>
                            <p className="text-[11px] leading-relaxed text-slate-500">
                              {t.realm.showcaseStayEstimateHint
                                .replace("{n}", String(st.slots))
                                .replace("{s}", String(stepSec))}
                            </p>
                          </div>
                        ) : (
                          <div className="rounded-lg border border-slate-800 bg-slate-950/40 p-2 text-[11px] text-slate-500">
                            {t.realm.showcaseCycleEstimateUnknown}
                          </div>
                        )}

                        {/* 本张音乐（不改解析源，只调怎么播） */}
                        <div className="space-y-2 rounded-lg border border-slate-800 bg-slate-950/40 p-2">
                          <div className="flex items-center justify-between">
                            <span className="text-xs text-slate-400">{t.realm.showcaseMusic}</span>
                            <button
                              type="button"
                              onClick={() => void audition(i)}
                              className={cn(
                                "rounded border px-2 py-0.5 text-[11px]",
                                auditionOn
                                  ? "border-emerald-500/60 bg-emerald-500/15 text-emerald-300"
                                  : "border-slate-700 text-slate-300 hover:bg-slate-800/60",
                              )}
                            >
                              {auditionOn ? `■ ${t.realm.showcaseAuditionStop}` : `▶ ${t.realm.showcaseAudition}`}
                            </button>
                          </div>
                          <label className="flex items-center gap-2 text-xs text-slate-400">
                            {t.realm.showcaseMusicOffset}
                            <input
                              type="number"
                              min={0}
                              max={240}
                              value={Math.round((d.musicOffsetMs ?? 0) / 1000)}
                              onChange={(e) =>
                                patchDeck(i, {
                                  musicOffsetMs: Math.max(0, Number(e.target.value) || 0) * 1000,
                                })
                              }
                              className="w-16 rounded border border-slate-700 bg-slate-950 px-2 py-1 text-slate-100"
                            />
                            <span>{t.realm.showcaseSeconds}</span>
                          </label>
                          <label className="flex items-center gap-2 text-xs text-slate-400">
                            {t.realm.showcaseMusicVolume}
                            <input
                              type="number"
                              min={0}
                              max={100}
                              value={d.musicVolume ?? 70}
                              onChange={(e) =>
                                patchDeck(i, {
                                  musicVolume: Math.max(0, Math.min(100, Number(e.target.value) || 0)),
                                })
                              }
                              className="w-16 rounded border border-slate-700 bg-slate-950 px-2 py-1 text-slate-100"
                            />
                          </label>
                          {/* 音乐细项**只在这里**（拍板：删掉领域级「进阶设置」折叠区）——
                              它们在"某一站想特别处理"时才需要改，逐张放在抽屉里才找得到。 */}
                          <label className="flex items-center gap-2 text-xs text-slate-400">
                            {t.realm.showcaseMusicFade}
                            <input
                              type="number"
                              min={0}
                              max={3000}
                              step={100}
                              value={d.musicFadeMs ?? MUSIC_FADE_MS}
                              onChange={(e) =>
                                patchDeck(i, {
                                  musicFadeMs: Math.max(0, Number(e.target.value) || 0),
                                })
                              }
                              className="w-16 rounded border border-slate-700 bg-slate-950 px-2 py-1 text-slate-100"
                            />
                            <span>ms</span>
                          </label>
                          <label className="flex items-center gap-2 text-xs text-slate-400">
                            {t.realm.showcaseMusicPreFade}
                            <input
                              type="number"
                              min={0}
                              max={10}
                              value={Math.round((d.musicPreFadeMs ?? MUSIC_PRE_FADE_MS) / 1000)}
                              onChange={(e) =>
                                patchDeck(i, {
                                  musicPreFadeMs:
                                    Math.max(0, Math.min(10, Number(e.target.value) || 0)) * 1000,
                                })
                              }
                              className="w-16 rounded border border-slate-700 bg-slate-950 px-2 py-1 text-slate-100"
                            />
                            <span>{t.realm.showcaseSeconds}</span>
                          </label>
                          <label className="flex cursor-pointer items-start gap-2 text-xs text-slate-300">
                            <input
                              type="checkbox"
                              checked={d.musicUseRealmBgm === true}
                              onChange={(e) => patchDeck(i, { musicUseRealmBgm: e.target.checked })}
                              className="mt-0.5 h-3.5 w-3.5 shrink-0 accent-blue-500"
                            />
                            <span className="min-w-0">
                              <span className="block">{t.realm.showcaseMusicUseBgm}</span>
                              <span className="mt-0.5 block text-[11px] text-slate-500">
                                {t.realm.showcaseMusicUseBgmHint}
                              </span>
                            </span>
                          </label>
                        </div>
                          </div>
                          <div className="border-t border-slate-800 px-3 py-2 text-right">
                            <button
                              type="button"
                              onClick={() => setEditIdx(null)}
                              className="rounded-lg bg-blue-600 px-3 py-1 text-sm text-white hover:bg-blue-500"
                            >
                              {t.common.close}
                            </button>
                          </div>
                        </div>
                      </div>
                    )}
                  </li>
                );
              })}
            </ul>

            {/* 添加阵营（带搜索） */}
            <div className="mt-3">
              {addOpen ? (
                <div className="space-y-2 rounded-lg border border-slate-700 bg-slate-950/60 p-2">
                  <input
                    autoFocus
                    value={addQuery}
                    onChange={(e) => setAddQuery(e.target.value)}
                    placeholder={t.realm.showcaseSearchFaction}
                    className="w-full rounded-lg border border-slate-700 bg-slate-950 px-2 py-1 text-sm text-slate-100"
                  />
                  <div className="max-h-44 space-y-0.5 overflow-y-auto">
                    {addList.map((c) => (
                      <button
                        key={dkey(c.modId, c.factionId)}
                        type="button"
                        onClick={() => {
                          updateManual({
                            deck: [...deck, { modId: c.modId, factionId: c.factionId }],
                          });
                          setAddOpen(false);
                          setAddQuery("");
                        }}
                        className="block w-full truncate rounded px-2 py-1 text-left text-xs text-slate-200 hover:bg-slate-800/60"
                      >
                        {pickName(c.name, lang) || c.factionId} · {c.modId}
                      </button>
                    ))}
                    {addList.length === 0 && (
                      <div className="px-2 py-1 text-xs text-slate-500">
                        {t.realm.showcaseNoFactionToAdd}
                      </div>
                    )}
                  </div>
                  <button
                    type="button"
                    onClick={() => {
                      setAddOpen(false);
                      setAddQuery("");
                    }}
                    className="w-full rounded border border-slate-700 py-1 text-xs text-slate-300 hover:bg-slate-800/60"
                  >
                    ✕
                  </button>
                </div>
              ) : (
                <button
                  type="button"
                  onClick={() => setAddOpen(true)}
                  className="w-full rounded-lg border border-dashed border-slate-700 py-1.5 text-sm text-slate-300 hover:bg-slate-800/40"
                >
                  ＋ {t.realm.showcaseAddFaction}
                </button>
              )}
            </div>

            {coreBlockedCount > 0 && (
              <p className="mt-3 rounded-lg bg-amber-500/10 px-2.5 py-1.5 text-xs text-amber-300">
                {t.realm.showcaseCoreExcludedNote.replace("{n}", String(coreBlockedCount))}
              </p>
            )}
            <div className="mt-3 flex items-center justify-between gap-2 border-t border-slate-800 pt-3">
              <span className="text-xs text-slate-500">{t.realm.showcaseManualNote}</span>
              <button
                type="button"
                onClick={toAuto}
                className="shrink-0 rounded-lg border border-slate-700 px-2.5 py-1 text-xs text-slate-300 hover:bg-slate-800/60"
              >
                {t.realm.showcaseResetDeck}
              </button>
            </div>
          </>
        )}
      </div>

      {/** 每阵营试听（音乐来源由后端解析；zip 内音乐会先抽取） */}
      <audio
        ref={auditionRef}
        className="hidden"
        onEnded={() => setAuditionOn(false)}
        onPause={() => setAuditionOn(false)}
      />
    </div>
  );
}
