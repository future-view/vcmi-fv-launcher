import { Fragment, useEffect, useRef, useState } from "react";
import { cn } from "@/lib/utils";
import type { LangDict } from "@/i18n/types";
import type { WikiCategory, WikiEntry, WikiLinkTarget } from "@/lib/modcache";
import { fullImgCandidates, fullImgUrl, pickName } from "@/lib/modcache";
import { getAnimCacheGen, getAnimGroup, type AnimGroup } from "@/lib/def/animationCache";
import { HdIcon } from "@/shared/HdIcon";
import { HdImg } from "@/shared/HdImg";
import { useHdRate } from "@/shared/useHdRate";
import { useWikiPopup, usePopupTriggers } from "./WikiPopupContext";
import {
  RelatedRow,
  StatTableRow,
  costText,
  formatResourceAmount,
  makeRefResolver,
  type ResourceInfo,
} from "./detailShared";
import {
  MOTION_HOME,
  buildIdleSimple,
  buildSteps,
  planPatrol,
  type AnimState,
  type AnimStep,
  type Motion,
} from "./animScenario";

/**
 * 当前动画**状态**（会话内粘性：切换其它生物也沿用，重启回默认"待机"）。
 *
 * 只存内存、不落盘：落盘会让用户下次打开时"永远在播受击"，看起来像坏了。
 * 2026-10-21 拍板：去掉"自动轮播"这个含糊选项，**默认即待机状态**
 * （待机状态内部本来就随机切换 站立/防御/悬停，观感与原来的预览轮播一致）。
 */
let sessionState: AnimState = "idle";

/** H3 固定 100ms/帧（`Images.cpp`，与战斗速度无关） */
const ANIM_FRAME_MS = 100;

/**
 * 缩放下限：适配参考再大，生物也不缩到 70% 以下（超出幕布的部分正常裁掉）。
 *
 * 依据（2026-10-22 全量 3331 个第三方动画实测）：适配包围盒改用 p10–p90 分位后，
 * `fitK < 0.70` 只剩 3.9%，且全是超规格素材（青璃龙女 `fitW=419`、黄金黑暗魔龙 379…）。
 * 不设下限时这些会缩到 0.24~0.53，观感就是"迷你"；裁边与 VCMI 游戏内 `clipRect`
 * 同源，属于素材本身超出窗口时的正常表现。
 */
const FIT_MIN_SCALE = 0.7;

/** 把某帧画到 canvas（高清=位图按 x/y 贴；1x=BGRA 逐像素） */
function drawAnimFrame(ctx: CanvasRenderingContext2D, g: AnimGroup, idx: number) {
  if (g.hd) {
    const f = g.hd[Math.min(idx, g.hd.length - 1)];
    ctx.drawImage(f.img, f.x, f.y);
    return;
  }
  if (g.raw) {
    const f = g.raw[Math.min(idx, g.raw.length - 1)];
    const rgba = new Uint8ClampedArray(f.bgra.length);
    for (let i = 0; i < f.bgra.length; i += 4) {
      rgba[i] = f.bgra[i + 2];
      rgba[i + 1] = f.bgra[i + 1];
      rgba[i + 2] = f.bgra[i];
      rgba[i + 3] = f.bgra[i + 3];
    }
    ctx.putImageData(new ImageData(rgba, f.w, f.h), 0, 0);
  }
}

/**
 * 「该生物没有这个动作分组」的会话记忆：`modId\u0000resource` → 组号集合。
 *
 * 存在理由：射击/特技在近战生物、部分分组在部分生物上并不存在，而"组是否存在"
 * **只能加载后才知道**（不预扫描，避免打开详情就连发十几次 IPC）。记住后，
 * 粘性选中这些组时不必每次切生物都重跑一次注定失败的加载。
 */
const missingGroups = new Map<string, Set<number>>();

export function CreatureDetail({
  d,
  lang,
  modId,
  cacheDir,
  modCache,
  t,
  compact,
  onNavigate,
  getEntry,
}: {
  d: Record<string, unknown>;
  lang: string;
  modId: string;
  cacheDir?: string | null;
  modCache?: Record<string, unknown>;
  t: LangDict;
  onNavigate?: (target: WikiLinkTarget) => void;
  compact?: boolean;
  getEntry?: (category: WikiCategory, id: string) => WikiEntry | undefined;
}) {
  // VCMI CCreaturePic（client/widgets/MiscWidgets.cpp:675-706）：
  //   背景 = 阵营 creatureBackground 130px；动画 = creature.animDefName（战斗动画），
  //   startPreview() 循环播放预览动作，帧间隔 100ms（Images.cpp:482）。
  // VCMI 生物窗没有静态图模式，也没有"启用动画"开关 —— 始终播放动画。
  // 构建期登记的**动画指针**（见 `modcache/build/data/creature.rs::probe_animation`）：
  // - `owner`：承载该 `animation` 文件的模组，直接当读取命令的 `modId`（跨模组引用也能命中）；
  // - `kind`：`def` / `json` / `unknown` / `none`。`def` 让后端跳过 sprites-json 探测，
  //   `none` 表示本模组与游戏根都没有该资源 → 不必发注定失败的请求。
  const animPtr = (d.anim ?? {}) as { owner?: string; kind?: string };
  const animOwner = typeof animPtr.owner === "string" && animPtr.owner ? animPtr.owner : modId;
  const animKind = typeof animPtr.kind === "string" ? animPtr.kind : undefined;
  const battleRes = animKind === "none" ? "" : ((d.animation as string) ?? "");
  /** 高清素材倍率：兵种大图在本页会被**放大**显示，高清收益最明显的一处 */
  const rate = useHdRate();
  // 生物窗背景（VCMI `CCreaturePic` 的 creatureBackground）。
  //
  // 很多扩展 mod **只加生物、不定义阵营**（如 forgotten-field / tides-of-war 的城堡扩展、
  // wake-of-gods 的 T8），它们的 `factions` 是空的 → 构建期查不到背景、只能黑底。
  // 这里按 `faction` 从**全局索引**取该阵营的 `creatureBackground` 路径 ——
  // 只引用既有文件、**不复制**（与城镇幕布去重的口径一致：同一张图只有一份）。
  const factionId = String(d.faction ?? "");
  const factionBg = factionId
    ? ((getEntry?.("factions", factionId)?.data as Record<string, unknown> | undefined)
        ?.creatureBackground as string | undefined)
    : undefined;
  const bgRel = (d.creatureBackground as string | undefined) || factionBg;
  const backgroundCandidates = bgRel ? fullImgCandidates(cacheDir ?? null, bgRel, rate) : [];
  // 只持有**当前播放分组**的帧：大型战斗 DEF 含十几个分组数百帧，
  // 一次性全解会在主线程同步执行、打开页面时掉帧（设计 §13.1）。
  // 按需只解当前动作，并带字节级/分组级 LRU 缓存（§13.2）。
  // 帧序列**同时兼容两条路径**：
  // - 高清（`hd`）：后端已用官方 HD 素材合成好每帧（含独立阴影切片），按 x/y 贴到位图；
  // - 1x（`raw`）：原来的 DEF 字节 + 前端逐像素 BGRA。
  // 二者由 `getAnimGroup` 统一产出，高清缺失时自动回退 1x，播放器行为完全一致。
  const [frames, setFrames] = useState<AnimGroup | null>(null);
  // 当前状态（粘性见模块级 `sessionState`）
  const [animState, setAnimState] = useState<AnimState>(sessionState);
  /**
   * 实际播放的状态。
   *
   * **弹窗（compact）恒为待机**：右键弹窗是"按住偷看"（`mouseup` 即关），可见时长
   * 通常不到 1 秒；若沿用详情页的选择（移动/攻击/受击），会出现"刚开就关、还带位移
   * 跑到画框外"的观感，且首开要多载好几个组。详情页保持原有的状态机与粘性。
   */
  const runState: AnimState = compact ? "idle" : animState;
  // 画布姿态（位移/镜像/缩放，移动状态用；其它状态回 `MOTION_HOME`）
  const [motion, setMotion] = useState<Motion>(MOTION_HOME);
  // 姿态过渡时长（ms）：行走时跟随本步播放时长，转身/收步为短过渡
  const [motionMs, setMotionMs] = useState(0);
  const [menuOpen, setMenuOpen] = useState(false);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  /** 舞台外层容器：用于读取"这一栏实际可用宽度"（窄屏/弹窗/多列都要自适应） */
  const stageWrapRef = useRef<HTMLDivElement | null>(null);
  const [availW, setAvailW] = useState(0);
  const popup = useWikiPopup();
  const triggers = usePopupTriggers();
  // 弹窗态能力列（第三列）的滚动容器：用于判断"还有没有下方内容"，决定底部渐隐提示
  const abilityScrollRef = useRef<HTMLDivElement | null>(null);
  const [abilityCanScroll, setAbilityCanScroll] = useState(false);

  /**
   * 播放器：按**状态**连续播放（代替原先"单组循环 + 随机跳组"）。
   *
   * 每轮由 [`buildSteps`] 生成步骤；步骤里的组**按需加载**（`getAnimGroup` 带缓存），
   * 缺失的组（近战没有 SHOOT、原版没有 SPECIAL/TURN）跳过并记入 `missingGroups`，
   * 之后同一生物不再重试 —— 避免每次进入状态都付一次注定失败的加载。
   */
  useEffect(() => {
    if (!battleRes) {
      setFrames(null);
      return;
    }
    let cancelled = false;
    let raf = 0;
    // 切状态即归位（离开移动状态不残留位移/缩放）。
    // ⚠️ 用 260ms 过渡而不是瞬移：位置归零与缩放回 1 同步插值，过程中不会出框，
    // 观感是"从走位处滑回中央"，直接 `setMotionMs(0)` 会看到生硬的空间跳跃。
    setMotion(MOTION_HOME);
    setMotionMs(260);
    const doubleWide = d.doubleWide === true;
    // 传**宽度**（窗口 100 源像素宽）；后端据此算档位
    const boxCss = compact ? 100 : 180;
    const missKey = `${modId}\u0000${battleRes}`;

    /** 等 canvas 挂上：首帧 `setFrames` 之后才会渲染 canvas */
    const waitCanvas = async () => {
      for (let i = 0; i < 20; i++) {
        const c = canvasRef.current;
        if (c) return c;
        await new Promise((r) => setTimeout(r, 16));
      }
      return null;
    };

    /** 播一个步骤（循环 N 圈 / 一次性停末帧 + 停留），完成时 resolve */
    const play = (g: AnimGroup, step: AnimStep) =>
      new Promise<void>((resolve) => {
        const canvas = canvasRef.current;
        const ctx = canvas?.getContext("2d");
        if (!canvas || !ctx || g.count === 0) {
          resolve();
          return;
        }
        if (canvas.width !== g.w || canvas.height !== g.h) {
          canvas.width = g.w;
          canvas.height = g.h;
        }
        let fi = 0;
        let done = 0;
        // 减掉一帧：首个 tick 就画出第 0 帧，使"起步"与位移过渡同时开始
        let last = performance.now() - ANIM_FRAME_MS;
        const finish = () => {
          if (step.holdMs > 0) window.setTimeout(resolve, step.holdMs);
          else resolve();
        };
        const tick = (now: number) => {
          if (cancelled) {
            resolve();
            return;
          }
          // 缓存被整体释放（切领域 / 空闲回收）→ 手里的位图已被 close()，本步作废，
          // 交给 `run` 的下一轮重新取（`getAnimGroup` 此时会重新请求/解码）。
          if (g.gen !== getAnimCacheGen()) {
            resolve();
            return;
          }
          if (now - last >= ANIM_FRAME_MS) {
            last = now;
            ctx.clearRect(0, 0, canvas.width, canvas.height);
            drawAnimFrame(ctx, g, Math.min(fi, g.count - 1));
            if (step.once) {
              // 一次性（死亡）：停在末帧，不循环、不跳组
              if (fi >= g.count - 1) {
                finish();
                return;
              }
              fi++;
            } else {
              fi++;
              if (fi >= g.count) {
                fi = 0;
                done++;
                if (done >= Math.max(1, step.loops)) {
                  finish();
                  return;
                }
              }
            }
          }
          raf = requestAnimationFrame(tick);
        };
        raf = requestAnimationFrame(tick);
      });

    const boxH = compact ? 130 : 234;

    /**
     * 应用一步的姿态。
     *
     * `syncMotion`（行走）的过渡时长跟随**本步实际播放时长** = 圈数 × 帧数 × 100ms，
     * 否则会出现"脚在迈、身已到"的滑行感。
     */
    const applyMotion = (step: AnimStep, g: AnimGroup) => {
      if (!step.motion) return;
      setMotion(step.motion);
      setMotionMs(step.syncMotion ? step.loops * g.count * ANIM_FRAME_MS : 260);
    };

    const run = async () => {
      let emptyRounds = 0;
      // 巡逻光标：跨轮保存当前位置/朝向（"走到一边再走回来"）
      let cursor: Motion = MOTION_HOME;
      while (!cancelled) {
        let played = 0;
        let steps: AnimStep[];
        if (runState === "move") {
          const plan = planPatrol(cursor, boxCss, boxH);
          steps = plan.steps;
          cursor = plan.next;
        } else {
          // 弹窗只播 HOLDING（见 `buildIdleSimple` 的说明），其余走完整状态机
          steps = compact ? buildIdleSimple() : buildSteps(runState as Exclude<AnimState, "move">);
        }
        for (const step of steps) {
          if (cancelled) return;
          if (missingGroups.get(missKey)?.has(step.group)) continue;
          const g = await getAnimGroup(
            animOwner,
            battleRes,
            step.group,
            doubleWide,
            boxCss,
            animKind,
          );
          if (cancelled) return;
          if (!g || g.count === 0) {
            const set = missingGroups.get(missKey) ?? new Set<number>();
            set.add(step.group);
            missingGroups.set(missKey, set);
            continue;
          }
          // `wantMs` → 圈数：各生物 MOVING 帧数差异很大（2~12），按实际帧数换算才不像滑冰
          const loops = step.wantMs
            ? Math.max(1, Math.min(8, Math.round(step.wantMs / Math.max(1, g.count * ANIM_FRAME_MS))))
            : step.loops;
          const playStep = loops === step.loops ? step : { ...step, loops };
          setFrames(g);
          applyMotion(playStep, g);
          if (!(await waitCanvas())) return;
          played++;
          await play(g, playStep);
        }
        // 整个状态一个组都播不了（该生物没有这些组）→ 交给静态图兜底，别空转
        if (played === 0) {
          emptyRounds++;
          if (emptyRounds >= 2) {
            setFrames(null);
            setMotion(MOTION_HOME);
            return;
          }
        }
      }
    };
    // 预热当前状态的**全部组**：播放循环是"按步骤串行 `await getAnimGroup`"的，
    // 首轮要依次等 2~5 次后端往返。这里先并发展开（`getAnimGroup` 自带 inflight 去重，
    // 下面 `run` 再请求同一组时共用同一次往返）。上限 8 组，避免一次打满线程池。
    {
      const warmSteps =
        runState === "move"
          ? planPatrol(MOTION_HOME, boxCss, boxH).steps
          : compact
            ? buildIdleSimple()
            : buildSteps(runState as Exclude<AnimState, "move">);
      const groups = [...new Set(warmSteps.map((s: AnimStep) => s.group))].slice(0, 8);
      for (const g of groups)
        void getAnimGroup(animOwner, battleRes, g, doubleWide, boxCss, animKind);
    }
    void run();
    return () => {
      cancelled = true;
      cancelAnimationFrame(raf);
    };
  }, [battleRes, animOwner, animKind, runState, d.doubleWide, compact]);

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

  // 士气/运气**不在百科页展示**（2026-10-21 移除，原本是一处设计错误）：
  // - VCMI 百科页 `WikiCreatureContent.cpp` 不显示这两项，只有战斗堆叠窗 `CStackWindow`
  //   的 `MoraleLuckBox` 才显示（那是"该堆叠此刻的士气/运气"，会叠加英雄/宝物/地形）；
  // - 生物配置里的 `MORALE`/`LUCK` 能力是 `propagator: ARMY`（给**全军**的加成），
  //   如大天使「提高友军士气」，它已经在能力区以「Increases allies morale」出现，
  //   在这里再画一个「士气 +1」既是重复、也是语义误读；
  // - 全库 312 个生物只有 3 个非 0（archangel/archDevil/ghostDragon），条件渲染还造成布局跳动。

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

  // 升级链 / 招募建筑：名称、图标、归属 mod 都走共享解析器（本 mod 缓存优先 →
  // 全类目索引兜底）。跨 mod 是常态（mod 生物升级 core 生物），没有兜底时相关行既无名也无图。
  const creatureRef = makeRefResolver({ category: "creatures", modId, lang, modCache, getEntry });
  const creatureLabel = (id: string) => creatureRef(id).name;
  /**
   * 图标 URL **基准路径**（不含高清变体）。
   *
   * ⚠️ 不要在这里取候选链的第 0 项（`fullImgCandidates(...)[0]`）：那是 `@Nx` 变体，
   * 而 `HdIcon` 会对传入的 src 再算一次候选链 → 变成 `a@4x@4x.webp`，兜底项也是这个
   * 不存在的文件 → 整行必然空白（见 `lib/hd.ts::hdVariantUrl` 的护栏注释）。
   * 交给 `HdIcon` 自己从基准路径展开候选链。
   */
  const creatureIconUrl = (id: string) => {
    const img = creatureRef(id).image;
    return img ? fullImgUrl(cacheDir ?? null, img) : null;
  };
  /** 升级链指向**别的模组**时，跳转要带对方的 modId，否则会跳到本 mod 的同名空位。 */
  const creatureModId = (id: string) => creatureRef(id).modId;
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
      const key = `${creatureLabel(id)}|${creatureRef(id).image ?? ""}`;
      if (seenKey.has(key)) continue;
      seenKey.add(key);
      out.push(id);
    }
    return out;
  };
  const upgradeTo = dedupeRel(idList(d.upgrades));
  const upgradeFrom = dedupeRel(idList(d.upgradedFrom));
  const navTo = (id: string) =>
    onNavigate?.({ modId: creatureModId(id), category: "creatures", id });

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

  /**
   * 能力无图标时的占位标识：取 type 的缩写。
   *
   * 规则：蛇形命名按 `_` 分段取各段首字母（`ON_COMBAT_EVENT` → `OCE`），
   * 只有一段时取前三个字母（`FLYING` → `FLY`），上限 4 字符（52px 框内可读）。
   *
   * ⚠️ 刻意不用通用图标：占位必须能反推出"缺的是哪个能力"，
   * 否则上游（Bonus Icons 的 bonuses.json）补了真图标也无法从界面上察觉。
   */
  const abilityTypeAbbr = (type: string): string => {
    const parts = type.split("_").filter(Boolean);
    if (parts.length === 0) return "?";
    if (parts.length === 1) return parts[0].slice(0, 3).toUpperCase();
    return parts
      .map((p) => p[0])
      .join("")
      .toUpperCase()
      .slice(0, 4);
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

  /**
   * 状态列表（2026-10-21 拍板：术语由"动作组"改为"状态"，一行一个状态）。
   *
   * 不再暴露单个 DEF 组（"待机/受击/攻击·正…"）—— 那正是"动画零散"的来源；
   * 组合状态内部的编排见 `animScenario.ts`。
   */
  const stateRows: { state: AnimState; label: string }[] = [
    { state: "idle", label: t.wiki.creature.anim.state.idle },
    { state: "move", label: t.wiki.creature.anim.state.move },
    { state: "attack", label: t.wiki.creature.anim.state.attack },
    { state: "hurt", label: t.wiki.creature.anim.state.hurt },
  ];

  /** 切换状态：写入模块级粘性值（跨生物沿用） */
  const pickState = (st: AnimState) => {
    setMenuOpen(false);
    sessionState = st;
    setAnimState(st);
  };

  // 换生物 / 换资源即收起浮层（状态保留，见 `sessionState`）
  useEffect(() => {
    setMenuOpen(false);
  }, [modId, battleRes]);

  // 浮层的关闭：点外部 / Esc。用 `data-anim-menu` 圈定"内部"范围，
  // 避免把面板自己的点击也判成"点外部"。
  useEffect(() => {
    if (!menuOpen) return;
    const onDown = (e: MouseEvent) => {
      const el = e.target;
      if (!(el instanceof Element) || !el.closest("[data-anim-menu]")) setMenuOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setMenuOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [menuOpen]);

  /** 当前状态的可读文本 */
  const animLabel =
    stateRows.find((r) => r.state === animState)?.label ?? t.wiki.creature.anim.state.idle;

  // ── 三层矩形（都是 1x 画布坐标，后端 `anim_window` 给出）──
  // - `win`：**幕布窗口**（100×130 或 200×130）→ 决定画框比例与黑幕范围；
  // - `tex`：**裁剪矩形**（win ∪ 全部组包围盒）→ 帧位图的尺寸；
  // - `fit`：**适配参考**（win ∪ 待机/移动/悬停/防御）→ 决定等比缩放倍数。
  //
  // ⚠️ 必须拆开：早期把"裁剪"和"显示"用同一个矩形，导致头部在**进前端之前**就被切掉，
  // 前端再怎么缩放也补不回来（实测凤凰/骨龙头顶仍被裁）。
  const texScale = frames?.scale ?? 1;
  const texW = frames ? Math.max(1, Math.round(frames.w / texScale)) : 100;
  const texH = frames ? Math.max(1, Math.round(frames.h / texScale)) : 130;
  const texX = frames?.wx ?? (d.doubleWide ? 170 : 150);
  const texY = frames?.wy ?? 155;
  const curtainW = frames?.winW ?? 100;
  const curtainH = frames?.winH ?? 130;
  const curtainX = frames?.winX ?? (d.doubleWide ? 170 : 150);
  const curtainY = frames?.winY ?? 155;
  const fitW = frames?.fitW ?? curtainW;
  const fitH = frames?.fitH ?? curtainH;
  const baseX = d.doubleWide ? 170 : 150;
  const baseY = 155;

  // 量一次可用宽度，并在容器变化（窗口缩放/换布局）时更新
  useEffect(() => {
    const el = stageWrapRef.current;
    if (!el) return;
    const apply = () => {
      const w = el.getBoundingClientRect().width;
      if (w > 0) setAvailW(Math.round(w));
    };
    apply();
    if (typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(apply);
    ro.observe(el);
    return () => ro.disconnect();
  }, [compact]);

  // 舞台 = 幕布窗口的比例，尺寸按**容器可用宽度**收敛（绝不撑破右侧内容区）。
  //
  // - 详情页：按游戏构图 1.8× 起算（宽度够就更大），上限 430×340；
  // - 弹窗（compact）：VCMI `CStackWindow` 本身就是"大图窗"，这里不再用 100×130 缩略图，
  //   而是尽量占满弹窗宽度（上限 300×300）—— 之前写死 100×130，在 880px 宽的弹窗里
  //   会缩成一个小方块（用户实机反馈："弹窗里只有一个很小的生物"）。
  //
  // ⚠️ 弹窗（compact）用**固定画框** `200×130`（= 宽幕布档），不跟生物的布局走：
  // 布局 A(100:130) 竖长、布局 B(200:130) 横长，若画框跟着幕布比例变，弹窗里就会出现
  // "有的生物预览特别高"（实测枯木战士 280px vs 独角神兽 140px）。固定画框后所有生物
  // 高度一致，布局 A 的生物居中显示、两侧留黑幕（与详情页的幕布语言一致）。
  const frameW = compact ? 200 : curtainW;
  const frameH = compact ? 130 : curtainH;
  const maxStageW = compact ? 320 : 430;
  const maxStageH = compact ? 260 : 340;
  const avail = Math.max(120, availW || maxStageW);
  let sw = Math.min(compact ? avail : frameW * (180 / 100), avail, maxStageW);
  let sh = (sw * frameH) / frameW;
  if (sh > maxStageH) {
    sh = maxStageH;
    sw = (sh * frameW) / frameH;
  }
  const stageW = Math.max(40, Math.round(sw));
  const stageH = Math.max(40, Math.round(sh));

  // 统一缩放：让"适配参考矩形"完整落进幕布（≥1 时不放大），并受 [`FIT_MIN_SCALE`] 兜底。
  // 适配参考本身已是 p10–p90 分位包围盒（后端 `plan_from_bounds`），离群帧不参与缩放。
  const fitK = Math.max(
    FIT_MIN_SCALE,
    Math.min(1, curtainW / Math.max(1, fitW), curtainH / Math.max(1, fitH)),
  );
  const pxPerUnit = stageW / frameW; // 每个 1x 画布单位 = 多少舞台像素
  const centerX = curtainX + curtainW / 2; // 幕布中心（画布坐标）
  const groundY = curtainY + curtainH; // 地平线 = 幕布底边

  /**
   * 把画布坐标里的矩形换算成舞台内的绝对定位（底部对齐地平线 + 水平居中）。
   *
   * `scale` 分开传：**生物按 fitK 缩、背景不缩** —— 背景一缩就会在幕布里露出
   * 上下黑边（用户已明确不接受"顶部一条黑"）；背景保持 1:1 时，垂直方向正好铺满，
   * 只保留幕布两侧的黑，视觉最干净。
   */
  const placeRect = (x0: number, y0: number, w: number, h: number, scale = 1) => {
    const u = scale * pxPerUnit;
    return {
      left: `${stageW / 2 + (x0 - centerX) * u}px`,
      top: `${stageH - (groundY - y0) * u}px`,
      width: `${w * u}px`,
      height: `${h * u}px`,
    };
  };
  // 背景：VCMI 把 creatureBackground(100×130) 钉在基准窗口，跟着同一套变换走
  const bgBox = placeRect(baseX, baseY, 100, 130, 1);
  // 生物层：裁剪矩形按 fitK 摆放（帧位图与它同宽高比 → 拉伸即精确对齐）
  const texBox = placeRect(texX, texY, texW, texH, fitK);

  const picBlock = (
    <div ref={stageWrapRef} className="space-y-2">
      {/* 只有两种布局：A = 与 VCMI 一致的 100×130（无黑幕，超出即裁）；
          B = 200×130（左右各 50px 黑幕，高度不变）。舞台宽度按容器可用宽度收敛，
          宽度变了高度跟着按比例走，所以背景始终铺满高度、只在左右出现黑幕。 */}
      <div
        className="relative mx-auto overflow-hidden rounded-xl border border-slate-600/60 bg-[#0a0e1a]"
        style={{ width: stageW, height: stageH }}
      >
        {/* VCMI CCreaturePic：阵营 creatureBackground 130px 作底，战斗动画叠在其上。
            ⚠️ 用底层 `<img>` 而非 CSS `background-image`：后者**无法在加载失败时回退**，
            而高清变体是按需生成的（可能暂时没有）—— 回退是常态，缺图会整块空白。 */}
        {backgroundCandidates.length > 0 && (
          <HdImg
            src={backgroundCandidates}
            alt=""
            className="pointer-events-none absolute"
            style={bgBox}
          />
        )}
        <div className="relative flex h-full w-full items-center justify-center">
          {frames && frames.count > 0 ? (
            // 生物层 = **裁剪矩形**整块，按与背景同一套变换摆放
            // （等比缩放 fitK + 底部对齐 + 水平居中）；超出幕布的部分由舞台裁掉
            <div className="absolute" style={texBox}>
              <canvas
                ref={canvasRef}
                style={{
                  // 位移 + 水平镜像（向左走 = 向右动画镜像，VCMI `reverse` 同款）+ 纵深缩放。
                  // 顺序：先平移到目标点，再镜像，最后缩放（镜像以元素中心为轴，不影响位置）。
                  transform: `translate(${motion.x}px, ${motion.y}px) scaleX(${
                    motion.flip ? -1 : 1
                  }) scale(${motion.scale})`,
                  // 行走用 linear（与地面平移一致，匀速）；转身/收步用短 ease-out
                  transition: `transform ${motionMs}ms ${motionMs > 400 ? "linear" : "ease-out"}`,
                }}
                // 位图宽高比 == 裁剪矩形宽高比（后端按同一矩形裁帧）→ 拉伸即精确对齐
                className="h-full w-full"
              />
            </div>
          ) : (
            <HdImg
              src={fullImgCandidates(
                cacheDir ?? null,
                (d.imageLarge as string) ?? (d.image as string),
                rate,
              )}
              alt=""
              // ⚠️ 必须 `object-contain`（等比 + 留黑边）：弹窗画框是 200:130 横幅，
              // 兜底图是 100:130 竖图 —— 用 `cover` 会放大裁切填满，观感就是"被拉伸"
              // （用户实机反馈）。动画层不受影响（它按裁剪矩形等比摆放）。
              className="h-full w-full object-contain"
              pixelArt
            />
          )}
        </div>
      </div>
      {/* 动作栏：**画布外**（解决"按钮压幕布"），宽度与画布对齐（同宽、居中），
          文本 `flex-1 truncate` + 按钮 `flex-none` 贴右缘 —— 多语言再长也不会推动按钮。
          compact（130px 弹窗）不渲染：高度紧张，且无空间。 */}
      {!compact && battleRes && (
        <div
          className="relative mx-auto flex w-[180px] items-center gap-2"
          data-anim-menu
        >
          {/* 只显示当前状态本身：按钮就在右侧，"动作："前缀属冗余（多语言还会顶掉按钮空间）。
              2026-10-21：去掉"自动轮播"，默认即"待机状态"，不再有歧义选项。 */}
          <span
            className="min-w-0 flex-1 truncate text-[11px] leading-none text-slate-300"
            title={`${t.wiki.creature.anim.label}：${animLabel}`}
          >
            {animLabel}
          </span>
          <button
            type="button"
            aria-label={t.wiki.creature.anim.entry}
            title={t.wiki.creature.anim.entry}
            onClick={() => setMenuOpen((v) => !v)}
            className={cn(
              "flex h-[20px] w-[20px] flex-none items-center justify-center rounded border text-[11px] leading-none transition-colors",
              animState !== "idle"
                ? "border-amber-400/80 bg-slate-800/80 text-amber-200"
                : "border-slate-600/60 bg-slate-900/60 text-slate-300 hover:text-slate-100",
            )}
          >
            ≡
          </button>
          {/* 状态面板：**一行一个状态**（2026-10-21 拍板），只有 4 行 → 无滚动、无截断。
              挂在状态栏上（栏不在 overflow-hidden 容器内），向下弹出，不遮动画区。 */}
          {menuOpen && (
            <div className="absolute right-0 top-[26px] z-30 w-[168px] rounded-lg border border-slate-700/70 bg-slate-900/95 p-1 shadow-lg">
              {stateRows.map((row) => {
                const active = row.state === animState;
                return (
                  <button
                    key={row.state}
                    type="button"
                    onClick={() => pickState(row.state)}
                    title={row.label}
                    className={cn(
                      "flex w-full items-center gap-1 rounded px-1 py-1 text-left text-[11.5px]",
                      "text-slate-200 hover:bg-slate-800/70",
                      active && "bg-slate-800/60 text-amber-200",
                    )}
                  >
                    <span className="w-2 flex-none text-center">{active ? "●" : ""}</span>
                    <span className="min-w-0 flex-1 truncate">{row.label}</span>
                  </button>
                );
              })}
            </div>
          )}
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
      icon={creatureIconUrl(id)}
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

  /**
   * 能力卡片（宽态网格与弹窗第三列共用，但两种版面**对"没有图标"的处理不同**）：
   *
   * - `wikiStyle=true`（百科页右栏，对齐 VCMI `WikiCreatureContent.cpp:330-332`）：
   *   `hasIcon = !icon.empty()`，**没有图标就不画图标列**，文字整宽左对齐、行更高更矮。
   *   这是本安装绝大多数能力的实际情况（能力图标来自 bonuses 的 graphics 表，本发行版
   *   `config/bonuses.json` 里一个都没有）—— 旧实现恒画一个 52px 空框，
   *   于是整页出现一排"空框 + 技能"。
   * - `wikiStyle=false`（右键弹窗，对齐 `CCreatureWindow.cpp` 的 BonusLineSection）：
   *   框与来源小字**恒画**（那里框本身承载来源配色信息），只是框内没有图标。
   */
  const abilityCards = (wikiStyle: boolean) =>
    abilities.map((ab, i) => {
      const abName = pickName((ab.name as Record<string, string>) ?? {}, lang);
      const abDesc = pickName((ab.description as Record<string, string>) ?? {}, lang) || abName;
      if (!abDesc && !abName) return null;
      // 能力图标（VCMI JsonBonus.cpp:892 取 ability["icon"]）
      const abIcon = ab.icon ? fullImgUrl(cacheDir, ab.icon as string) : undefined;
      const showIconCol = !wikiStyle || !!abIcon;
      return (
        <div key={i} className="flex gap-2 rounded-lg border border-slate-800/60 bg-slate-900/40 p-2">
          {showIconCol && (
            <div className="flex-none">
              <div
                className={cn(
                  "relative h-[52px] w-[52px] rounded border bg-slate-800/40",
                  abilityBorder(String(ab.source ?? "")),
                )}
                // VCMI：能力格右键 = 描述文本气泡
                {...triggers((p) => popup.showText(abDesc, p))}
              >
                {abIcon ? (
                  <HdIcon src={abIcon} alt="" className="h-full w-full object-contain" />
                ) : (
                  // 无图标占位：能力 type 的缩写。刻意不用通用图标 ——
                  // 占位必须能反推出"缺的是哪个能力"，否则以后想补真图标时无从发现。
                  <span
                    className="absolute inset-0 flex items-center justify-center text-[13px] font-semibold tracking-tight text-slate-500"
                    title={`${String(ab.type ?? "")}（无图标）`}
                  >
                    {abilityTypeAbbr(String(ab.type ?? ""))}
                  </span>
                )}
                <span className="absolute inset-x-0 bottom-0 truncate bg-slate-900/80 px-0.5 text-center text-[8px] leading-3 text-slate-400">
                  {sourceLabel(ab.source as string)}
                </span>
              </div>
            </div>
          )}
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
      <div className="grid grid-cols-[repeat(auto-fit,minmax(260px,1fr))] gap-2">
        {abilityCards(true)}
      </div>
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
          {abilityCards(false)}
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
