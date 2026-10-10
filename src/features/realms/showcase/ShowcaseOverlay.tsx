import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
  type RefObject,
} from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import {
  ChevronLeft,
  ChevronRight,
  RotateCcw,
  Pause,
  Play,
  Volume2,
  VolumeX,
  X,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import {
  ensureShowcaseAudio,
  getModsCacheDir,
  readModSettingsJson,
  readWikiIndex,
  readShowcaseSlides,
  realmAssetUrl,
  realmConfigDir,
  realmDir,
  scanRealMods,
  DESIGN_CANVAS_H,
  DESIGN_CANVAS_W,
  type ShowcaseEntry,
  type ShowcaseSlide,
} from "@/lib/tauri";
import { imgCandidates, markHdMissing } from "@/lib/hd";
import { fullImgCandidates, fullImgUrl, pickName } from "@/lib/modcache";
import { HdImg } from "@/shared/HdImg";
import { useWikiStagesDone, WikiBuildOverlay } from "@/shared/WikiBuildOverlay";
import { useHdRate } from "@/shared/useHdRate";
import { realmAsset } from "@/lib/realm/intro";
import {
  buildShowcaseDeck,
  deckToEntries,
  defaultHeroIds,
  estimateStayMs,
  resolveDwellingId,
  resolveDwellingIds,
  randomSeed,
  FOCUS_STEP_MS,
  heroClassKey,
  MUSIC_FADE_MS,
  MUSIC_PRE_FADE_MS,
} from "@/lib/realm/showcase";
import { collectEnabledIds } from "@/lib/realm/enabledMods";
import type { Realm, ShowcaseDeckEntry } from "@/lib/realm/types";
import { backendErrorText } from "@/lib/backendError";

interface Props {
  realm: Realm;
  lang: string;
  /** 领域素材根目录（自定义底图 / 回退 BGM 用） */
  root?: string;
  /** 玩家已静音时巡礼也不出声（§7.8） */
  bgmMuted?: boolean;
  /** 切换静音（父级给回调时会把状态持久化到领域级 BGM 开关；不给则只在巡礼内生效） */
  onToggleMute?: () => void;
  /** 外部注入「本领域启用模组集合」（编辑器预览用：编辑期改动尚未落盘，读不到 modSettings） */
  enabledIds?: Set<string> | null;
  /**
   * **领域主界面的 BGM 元素**（`RealmShell` 常驻的那个）。
   *
   * 给了它，巡礼就**接管同一个元素**而不是自建一个 →
   * 从主界面进巡礼时主题曲**接着播、不从头开始**，退出后也停在同一时间线上。
   * 没给（例如独立预览）时退回组件内部自己的 `<audio>`，行为与从前一致。
   */
  realmBgmRef?: RefObject<HTMLAudioElement | null>;
  onExit: () => void;
}

type ScanMod = { id?: string; modId?: string; modPath?: string; type?: string };

/** 设计稿尺寸（§5.9.1）：整体等比缩放，长宽比变化留背景色不拉伸。
 * 与全局 UI 缩放共用同一坐标系（`DESIGN_CANVAS_W/H`）——窗口 16:9 时逻辑视口恒为
 * 1280×720，此处 `scale` 恰为 1.000（零黑边）；非 16:9 窗口才由它 letterbox 兜底。 */
const DESIGN_W = DESIGN_CANVAS_W;
const DESIGN_H = DESIGN_CANVAS_H;
/**
 * 幕布 frame：**高度撑满 sc-top 行的剩余空间**（宽度按 800/374 比例推出）。
 *
 * 为什么不再写死像素：底栏（标签栏 / 列表 / 字幕 / 切换条）高度一变，固定高的幕布就会
 * 与它们之间**空出一大块**（"sc-mid 与 sc-foot 之间空隙太大"就是这个）。
 * 改成从行高**实测**之后再算，空隙自动被幕布吃掉 —— 窗口 / 各处间距变化都不用再调数字。
 *
 * `FRAME_H_FALLBACK` 只用于首帧（尚未量到行高时），随后由 ResizeObserver 接管。
 */
const FRAME_H_FALLBACK = 400;
/** 幕布原生宽高比（= 856/400 也等于它，故放大不改变比例） */
const FRAME_RATIO = 800 / 374;
/** 右上巡礼区宽度 + 它与幕布之间的 gap（sc-top 行的固定占用，用于推算幕布宽度上限） */
const ZONE_W = 300;
const TOP_GAP = 16;
/** 幕布原生像素（border 坐标基准） */
const BACKDROP_W = 800;
const BACKDROP_H = 374;

/**
 * **底栏字幕的排版定量**（2026-09-22 调大，对标原版战役过场字幕）。
 *
 * ⚠️ 这里全部是**设计稿像素**（1280×720 画布内部单位）：整个演出界面是这块画布整体等比缩放的
 * （见下方 `scale`），所以 1080P → 2K → 4K 往上走，字号是**跟着一起放大**的 —— 不需要给 2K/4K
 * 单独写值，改这里的基准值就等于改所有分辨率。
 *
 * 唯一会被拉小的情况是"窗口比设计稿还小"（`scale < 1`），由 `typeBoost` 反向补偿，上限 1.15
 * —— 再大会把幕布压到 `FRAME_H_FALLBACK` 的舒适区以下。
 *
 * 取值依据：原版过场字幕字高约占屏高 3.3~3.7%；本字幕属**次要信息**（焦点在幕布），
 * 故描述取 15/698 ≈ 2.1%，名称 20px bold 拉开一级层级。
 */
const CAP_NAME_SIZE = 20;
const CAP_NAME_LEAD = 26;
const CAP_DESC_SIZE = 15;
const CAP_DESC_LEAD = 21;
/** 描述最多几行（超出才尾部渐隐，完整内容在右上方巡礼区滚动显示） */
const CAP_DESC_LINES = 3;
/** 描述行宽：**每行 ≈42 个汉字**（中英混排时略宽），这是 CJK 舒适阅读区间的上沿 */
const CAP_DESC_W = 640;
/** 每行字数 × 行数 ≈ 渐隐阈值；`line-clamp` 之后还有内容才值得画那道遮罩 */
const CAP_DESC_FADE_AT = 110;
/**
 * 巡礼序列的项类型（**统一模型**，2026-09-21 拍板）：
 * 兵种 → 建筑 → 英雄 三类拼接成一条序列；两种模式共用同一条序列与同一套 UI，
 * 差别只在"谁移动选中项"（定时器 / 用户点击）。
 * ⚠️ 列表**不再有 8 格上限**（旧 `MAX_SLOTS` + 「+N」已删除）：全部项横向滚动展示。
 */
const FOCUS_KINDS = ["creatures", "buildings", "heroes"] as const;
type FocusKind = (typeof FOCUS_KINDS)[number];
/**
 * 焦点项用 **id** 而非下标：
 * 列表展示"全部项"，巡礼序列只含"参与项" —— 两者下标不同。
 * 用 id 才能让"点未参与项看一眼"与"点参与项"走同一条路径（`peek`）。
 */
type FocusItem = { kind: FocusKind; id: string };
/** 素材预取/就绪判定的张数上限：比实际渲染的略多，宁可多取几张也不让画面开天窗 */
const PREFETCH_CREATURES = 12;
const PREFETCH_BUILDINGS = 16;
const PREFETCH_HEROES = 4;
/**
 * 进度点的数量上限：超了就**不显示圆点**。
 * 自动编排最多 48 个阵营，48 个 8px 点 + 间距 ≈ 670px，会把控制条挤爆；
 * 而这个信息"我在第几个"由 `5 / 38 阵营` 与下方阵营切换条承载（功能本就重复）。
 */
const DOTS_MAX = 12;

/**
 * 详情卡的图框与三类图的约束方式。
 *
 * ⚠️ 三类图**原始比例完全不同**（兵种立绘竖长、建筑图标横宽 ≈2.2:1、英雄头像近方），
 * 所以统一规则是：**`object-contain` + 只约束一个维度，另一维度自适应** ——
 * 两个维度都定死会把图拉伸变形（这是"图片等比"的既有约定）。
 * 图框尺寸恒定（262×120）→ 不同比例的图都居中出现，卡片不会被撑变形。
 */
const ZONE_IMG_BOX_H = 150;
const DETAIL_IMG_H = 148; // 兵种/英雄：定**高**
const DETAIL_IMG_W = 250; // 建筑：定**宽**

/**
 * 描述取值 + **语言回退链**：当前语言 → english → 任意一种。
 *
 * 描述在多语言下经常只有部分语言有（模组作者只写了一种，或 h3-core 只有中英），
 * 直接用 `d[lang]` 会"明明是中文界面却什么都没有"。
 */
function pickDesc(d: Record<string, string> | undefined, lang: string): string {
  if (!d) return "";
  const hit = (v?: string) => (v && v.trim() ? v : "");
  return (
    hit(d[lang]) ||
    hit(d.english) ||
    hit(Object.values(d).find((v) => hit(v))) ||
    ""
  );
}

/**
 * 一张 slide **实际会渲染**的图片 URL 集合。
 *
 * ⚠️ 「预取」与「就绪判定」必须用**同一个集合**，否则会出现"预取了但没判到位"，
 * 或者反过来把永远用不到的图算进就绪条件里（拖慢开播）。
 * 之前预取只取 `imageLarge` 且**完全没取建筑图** —— 而兵种卡在只有 `image` 时会空白、
 * 建筑条（44 格）几乎全空，正是"素材没加载"的观感来源。
 */
function slideImageUrls(s: ShowcaseSlide, cd: string, rate = 1): string[] {
  const urls: string[] = [];
  /**
   * 原图与高清变体**都预取**：渲染用高清时不会"等到切张才加载"。
   * 高清变体不存在也无妨 —— 预取失败会立即 `onerror`（就绪判定里失败同样放行），
   * 而渲染侧 `HdImg` 会自动回退原图。
   */
  const push = (rel?: string) => {
    if (!rel) return;
    const base = fullImgUrl(cd, rel);
    if (!base) return;
    // 预取**整条候选链**（高清逐档 + 原图）：渲染侧按同一顺序尝试，
    // 只预取最高档会让"后端按用途只生成 2x 的图"在切张时才临时加载（可见的闪烁）。
    // 不存在的档位会立即 `onerror`（本地文件），代价可忽略。
    for (const u of imgCandidates(base, rate)) urls.push(u);
  };
  push(s.backdrop);
  for (const c of s.creatures.slice(0, PREFETCH_CREATURES)) {
    push(c.imageLarge ?? c.image);
  }
  for (const b of s.buildings.slice(0, PREFETCH_BUILDINGS)) {
    push(b.image);
  }
  for (const h of s.heroes.slice(0, PREFETCH_HEROES)) {
    push(h.image);
  }
  return urls;
}

/**
 * 纵向空间预算（设计稿 720 − padding 22 = **698 可用**）。
 *
 * ⚠️ 这里是定尺设计稿：**没有自适应**，各段高度必须能被预算装下，否则
 * `sc-mid`（`flex-1 min-h-0`）的盒子会被压缩、而**内容不会缩** → 兵种行直接压到建筑条上（重叠）。
 *
 * 逐段实算（`text-[11px]` 只给 font-size → 行高取 preflight `html{line-height:1.5}` = 16.5）：
 *
 * | 区段 | 构成 | 高度 |
 * |---|---|---|
 * | sc-top | **弹性行**：幕布（实测行高撑满，宽 = 高 × 800/374）+ 右上巡礼区 300 宽 | 余量 |
 * | 列表区 | mt 12 + 标签栏 ~24 + 卡片 5+62+2+14+4=87 +（pt 4 + pb 3 + 滚动条 6 + 余 2） | 130 |
 * | sc-foot | mt 8 + max(左列 26+2+3×21+2+2+pb 4=99, 右列 30+4+16+4+10=64) | 107 |
 * | 阵营切换条 | mt 12 + 卡片 ~74 + pb 4 + 滚动条 6 | 96 |
 * | **合计** | | **698 − 296 = 402 给 sc-top** |
 *
 * ⚠️ 2026-09-21 大改：**删掉了"建筑条"这一整行**，中间只留「标签栏 + 一个列表」
 * （兵种/建筑/英雄三选一）。**两种模式共用这一套布局**。
 *
 * ⚠️ **幕布高度不再写死**：sc-top 是 `flex-1`，幕布由 `frameH`（实测行高）撑满 ——
 * 从前写死 400px 时，底栏高度一变就会在"列表与字幕之间空一大块"。
 *
 * sc-foot 的左列 = 名称 + 描述 + **进度条 2 + 间距 2 + 托底 pb 4**（见 `CAP_*` 常量。
 *
 * ⚠️ 2026-09-22 字号重排（对标原版战役过场字幕）：**名称 18/24 → 20/26、描述 12/17 → 15/21、
 * 行数 2 → 3、行宽 720 → 640**。旧值实测偏小的根因是"描述只占逻辑屏高 1.5%"，而原版过场
 * 字幕占屏高 **3.3~3.7%** —— 15px/698 ≈ **2.1%**，既追上一级，又不至于把幕布压扁（左列 64→99）。
 * 同时补了**渐变托底**：从前只有弥散文字阴影（`0 2px 8px`），CJK 小字笔画会被冲淡；有底之后
 * 阴影收紧到 `0 1px 3px`，对比度反而更稳（与 §分析一致）。
 * 行宽收窄到 640 是为了**每行字数**：中英混排下 ≈42 字/行（旧 720/12 ≈58，超出中文舒适区 28~38）。
 * 描述取 3 行是因为它是**过场正文**而不是标签：幕布阶段观众只在余光里扫字幕，截断成本比详情页高。
 *
 * 兵种区是唯一的 `flex-1`：实得 698 − (691 − 125.5) = **132.5**，需要 125.5 → 余 7 即它内部底部留白。
 * **手工查看模式**反而宽松：只显示一行列表（+ 分段条），另一行隐藏 → `sc-mid` 多出 ~90px。
 *
 * 📌 增删元素**必须重算上表**。两处易错：
 * ① 只给 `font-size` 的写法（如 `text-[11px]`）行高是 **16.5**，别按 11 算；
 * ② `items-end` 的 sc-foot 是**取大**不是相加。
 * 📌 与原型 `faction-showcase-prototype.html` 的差异是**有意**的，不可照抄：原型自称"纵向共 690 可用"(:74)，
 * 但其流式区实测 ≈716（**自身溢出 18px**，靠 `overflow:hidden` 裁掉，还把控制条做成绝对层压在切换条上）；
 * 其右侧放大区三卡 ≈376，**又比自己的 337 容器高 ≈39**。故原型的间距（14/12/12/8）、建筑卡 104×48、
 * 切换条 132×46 **都不能照搬** —— 照搬会吃光余量。
 */
/** 兵种卡位与图：对齐原型 `.cslot`（width 86 / img 56×62） */
const SLOT_W = 86;
const SLOT_IMG_W = 56;
/** 三个分类的**图位高度统一**（建筑不再矮一截）—— 列表等高，幕布就不跳 */
const SLOT_IMG_H = 62;
/**
 * 建筑卡：建筑原图 **150×70**（横图），故卡更宽；但**图位高度与兵种/英雄完全一致**，
 * 图片在框内 `object-contain` **居中** → 比例不破、三项列表高度相同（拍板）。
 */
const BLD_SLOT_W = 108;
const BLD_IMG_W = 108;
const BLD_IMG_H = SLOT_IMG_H;
/** 列表卡片总高（pt 5 + 图位 62 + 间距 2 + 名称 14 + pb 4）—— 三个分类共用同一高度 */
const CELL_H = 5 + SLOT_IMG_H + 2 + 14 + 4;
/**
 * 列表容器固定高度：`pt-[4px]` + 卡片 + `pb-[3px]` + 横向滚动条 6 —— 定尺后换分类/换模式都不跳。
 *
 * ⚠️ 那 4px 上边距**不是装饰**：当前项会 `-translate-y-[2px]` 上浮，而容器是 `overflow-x-auto`
 * ——CSS 规定 `overflow-x` 非 `visible` 时 `overflow-y` 会被算成 `auto`，于是**纵向也裁**：
 * 少这 4px，当前项的金框上边与阴影就会被切掉（实测"部分金色边框无法显示"）。
 */
const LIST_H = CELL_H + 15;
/**
 * 阵营切换条（2026-09-21 放大）：卡片 **140×72**、缩略图 54 高、名称 12px **居中**。
 * 旧值 116×58 / 40 高 / 名称 11px **左对齐** —— 缩略图是 `object-contain` 居中的，
 * 名称左对齐就会"名称与图片不对齐"（实测反馈），故名称改居中并整体放大一档。
 */
const SW_W = 140;
const SW_IMG_H = 54;

/** 局部动画（§9 转场 / 入场阶梯 / 幕布高亮脉冲），避免污染全局样式 */
const KEYFRAMES = `
@keyframes scFadeIn{from{opacity:0}to{opacity:1}}
@keyframes scSlideIn{from{opacity:0;transform:translateX(6%)}to{opacity:1;transform:none}}
@keyframes scDissolveIn{from{opacity:0;filter:blur(8px);transform:scale(.96)}to{opacity:1;filter:blur(0);transform:scale(1)}}
@keyframes scRise{from{opacity:0;transform:translateY(12px) scale(.94)}to{opacity:1;transform:none}}
@keyframes scHeroIn{from{opacity:0;transform:translateX(24px)}to{opacity:1;transform:none}}
@keyframes scPulse{0%{opacity:.42}55%{opacity:1}100%{opacity:.42}}
/* 横向滚动条：设计稿是定尺 1280×720，内容溢出时必须能看见且能拖 */
.sc-scroll{scrollbar-width:thin;scrollbar-color:rgba(148,163,184,.5) rgba(15,23,42,.45);}
.sc-scroll::-webkit-scrollbar{height:6px}
.sc-scroll::-webkit-scrollbar-track{background:rgba(15,23,42,.45);border-radius:3px}
.sc-scroll::-webkit-scrollbar-thumb{background:rgba(148,163,184,.5);border-radius:3px}
.sc-scroll::-webkit-scrollbar-thumb:hover{background:rgba(148,163,184,.8)}
`;

/** 转场：fade 400ms / slide 420ms / dissolve 520ms（§9） */
const TRANSITION_ANIM = {
  fade: "scFadeIn .4s cubic-bezier(.4,0,.2,1) both",
  slide: "scSlideIn .42s cubic-bezier(.4,0,.2,1) both",
  dissolve: "scDissolveIn .52s both",
} as const;

/**
 * **唯一**允许写 `HTMLMediaElement.volume` 的入口：先夹到 [0,1] 再写。
 *
 * ⚠️ 越界写入会**抛 `IndexSizeError`**，而它的调用点全在 `requestAnimationFrame` 回调里 ——
 * 异常会直接打断那条 rAF 链：淡入淡出停在一半、`then` 永不执行
 * （例如"淡出后 pause()"没跑到，或淡入没跑完导致音量卡在 0）。
 * 外部表现就是"**概率性没有声音**"，且在控制台只留一条与音频无关的 IndexSizeError。
 *
 * 越界怎么来的：`from * (1 - k)` 里的 `k` 由 rAF 时间戳算出，而 rAF 时间戳可能
 * 回退或与大粒度时钟混用（`performance.now()` 与帧时间戳不完全同源），
 * 于是 `k` 会短暂地 < 0 或 > 1（实测报错值 -0.0027125 = 0.7 × 0.003875，即 3.1ms/800ms 的越界）。
 * 因此：**夹值 + 非有限值兜底**，双重保险。
 */
function setVol(a: HTMLAudioElement | null, v: number): void {
  if (!a) return;
  a.volume = Math.max(0, Math.min(1, Number.isFinite(v) ? v : 0));
}

/**
 * 把一个**横向滚动容器**变成"滚轮也能横向滚"（返回**回调 ref**，直接挂到容器上）。
 *
 * ⚠️ 两个坑：
 * ① React 的 `onWheel` 是**被动**监听（passive），在它里面 `preventDefault()` 无效 →
 *    必须原生 `addEventListener(..., { passive: false })`；
 * ② 内容层带 `key={idx}`，每次换张**整棵重挂载** → 普通 `useRef` + `useEffect([])` 会绑到
 *    已经卸载的节点，所以这里用回调 ref，每次挂载重新绑定。
 *
 * 纵向 `deltaY` 转横向；本来就是横向的（触控板双指横滑 / Shift+滚轮）交给浏览器原生处理。
 */
function useWheelScrollX() {
  const off = useRef<(() => void) | null>(null);
  return useCallback((el: HTMLElement | null) => {
    off.current?.();
    off.current = null;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      if (Math.abs(e.deltaY) <= Math.abs(e.deltaX)) return;
      if (el.scrollWidth - el.clientWidth <= 1) return; // 无需横向滚动 → 不拦截（别影响页面纵向滚）
      e.preventDefault();
      el.scrollLeft += e.deltaY;
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    off.current = () => el.removeEventListener("wheel", onWheel);
  }, []);
}

/** rAF 时间差 → 0..1 的进度。负值（时间戳回退）归 0，避免淡入淡出反向或越界 */
function prog(now: number, t0: number, dur: number): number {
  if (!(dur > 0)) return 1;
  return Math.max(0, Math.min(1, (now - t0) / dur));
}

/**
 * `play()` 的容错包装。
 *
 * 换源瞬间调用 `play()` 极易被拒绝，且都是**瞬时**错误：
 * - `AbortError`：上一次 `play()` 的加载被新的 `src` 中断（**换张时最常见**）；
 * - `NotSupportedError`：元数据尚未就绪。
 * 稍后重试即可成功；不重试就会静默失败 → 外部表现"切到下一个阵营没有音乐"。
 * 其余错误（真正的路径/格式问题）交给调用方的回退逻辑与 `onError`。
 */
function safePlay(a: HTMLAudioElement, retry = 2) {
  a.play().catch((e: unknown) => {
    const name = (e as { name?: string })?.name ?? "";
    if (retry > 0 && (name === "AbortError" || name === "NotSupportedError")) {
      window.setTimeout(() => safePlay(a, retry - 1), 140);
      return;
    }
    if (name !== "AbortError") {
      console.warn("[showcase] 音频播放被拒", {
        src: a.getAttribute("src"),
        name,
        err: backendErrorText(e),
      });
    }
  });
}

export function ShowcaseOverlay({
  realm,
  lang,
  root,
  bgmMuted,
  onToggleMute,
  enabledIds,
  realmBgmRef,
  onExit,
}: Props) {
  const { t } = useI18n();
  const cfg = realm.showcase ?? {};
  /**
   * 高清素材倍率（按显示器物理宽推导：1080p/2K → 2、4K → 3）。
   * 素材阶段已按同一倍率生成 `@Nx.webp`；本组件只负责"取高清、读不到就回退原素材"。
   */
  const hdRate = useHdRate();
  const realmFocusStep = cfg.focusStepMs ?? FOCUS_STEP_MS;
  /**
   * 「每次进入巡礼都换一批」的**会话种子**：开启时**进入时现算一次**，整场复用。
   *
   * ⚠️ 必须是"一次"（`useState` 惰性初始化）：放在渲染里每次算会让顺序**每帧乱跳**；
   * ⚠️ **不写回配置**（只是本次播放的顺序），保存里的 `autoSeed` 仍然是编辑器显示的那一版。
   */
  const [sessionSeed] = useState<number | null>(() =>
    cfg.auto?.rerollOnEnter === true ? randomSeed() : null,
  );

  /**
   * 领域 BGM 元素：**优先接管主界面那一个**（`realmBgmRef`），没有就用组件内部的。
   * 这样"从主界面进巡礼"时主题曲是**接续**的，不会从头开始（实测反馈）。
   */
  const bgmEl = useCallback(() => realmBgmRef?.current ?? bgmAudioRef.current, [realmBgmRef]);

  /**
   * 借用主界面的 BGM 元素时，退出巡礼要把**音量还原** ——
   * 巡礼用的是「音乐音量」设置（默认 70），不该把主界面的音量永久改掉。
   */
  useEffect(() => {
    const a = realmBgmRef?.current;
    if (!a) return;
    const prev = a.volume;
    return () => setVol(a, prev);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [realmBgmRef]);

  const [slides, setSlides] = useState<ShowcaseSlide[]>([]);
  /**
   * 当前张的图是否**全部就绪**。声明在此处（而不是用它的 effect 旁边）是因为
   * `auto` 要用它 —— 素材没就绪就开始计时，前几张的时间会被空白画面吃掉。
   */
  const [assetsReady, setAssetsReady] = useState(false);
  /**
   * 当前张的**进度**（已播 ms / 总时长 ms），供进度指示用。
   *
   * 为什么必须有：停留时长是**自动算出来的**（= 焦点一轮），用户完全不知道还剩多久，
   * 于是"想细看却怕马上被切走"只能靠暂停来保险 —— 给一条进度线比任何按钮都更早解决问题。
   * 节流到 ~200ms 更新一次（每帧 setState 会让 9 张牌组的长巡礼持续重渲染）。
   */
  const [progress, setProgress] = useState<{ elapsed: number; total: number }>({
    elapsed: 0,
    total: 0,
  });
  const [deckList, setDeckList] = useState<ShowcaseDeckEntry[]>([]);
  const [idx, setIdx] = useState(0);
  const [playing, setPlaying] = useState(cfg.autoPlay !== false);
  /** 巡礼序列的游标（自动播放按焦点间隔推进它） */
  const [focusIdx, setFocusIdx] = useState(0);
  /** 左下列表的分类（两种模式共用；自动播放会随焦点自动跟随） */
  const [listTab, setListTab] = useState<FocusKind>("creatures");
  const [cacheDir, setCacheDir] = useState<string | null>(null);
  const [audioPath, setAudioPath] = useState<string | null>(null);
  const [status, setStatus] = useState<"loading" | "empty" | "ready">("loading");
  const [reduced, setReduced] = useState(false);
  /**
   * 巡礼内的静音开关（§7.8）。本地状态保证点一下**立刻**生效，并跟随父级 `bgmMuted` 同步；
   * 给了 `onToggleMute` 时顺带持久化到领域级 BGM 开关。
   */
  const [muted, setMuted] = useState(bgmMuted === true);
  useEffect(() => setMuted(bgmMuted === true), [bgmMuted]);
  const toggleMute = () => {
    setMuted((prev) => !prev);
    onToggleMute?.();
  };

  const stageRef = useRef<HTMLDivElement | null>(null);
  const switchRef = useRef<HTMLDivElement | null>(null);
  /** modId → modPath（zip 内音乐抽取要用真实磁盘路径，不是 mod id） */
  const modPathRef = useRef<Map<string, string>>(new Map());
  /** 「提前淡出」是否已在本轮触发（每轮换张重置，避免每帧重复淡出） */
  const preFadeDoneRef = useRef(false);
  /**
   * 提前淡出的 rAF 句柄。**必须在换张时取消** —— 它是一条独立于调度器的 rAF 链，
   * 若淡出时长大于「距切换剩余时间」，它会在换张**之后**继续跑并与新的淡入抢 `volume`，
   * 把下一个阵营的音乐压回 0（外部表现就是"切到下一个阵营后没有音乐"）。
   */
  const preFadeRafRef = useRef(0);
  /** 提前淡出的**代际**：调度器 effect 每重跑一次 +1，旧链每帧自检后自杀（与 `fadeGenRef` 同一思路） */
  const preFadeGenRef = useRef(0);
  /** 已写入 `<audio src>` 的阵营音乐 URL。用于区分「换源」与「同源续播」 */
  const faSrcRef = useRef("");
  /** 换源后待应用的起始位置（ms）。src 变更瞬间设置 `currentTime` 会被忽略，
   *  故挂到 `onLoadedMetadata` 里消费（每阵营的音乐起始位置这才真正生效）。 */
  const pendingOffsetMsRef = useRef(0);
  /** 音乐淡入淡出的**代际**：每次音乐 effect 重跑就 +1，让上一条链自杀（见 fade() 注释） */
  const fadeGenRef = useRef(0);
  /**
   * 「重开当前张」信号（自增即触发）。给"单张牌组 / 绕回自己"时的
   * 上一张 / 下一张按钮一个**可见反馈**，见 `go()`。
   */
  const [restartTick, setRestartTick] = useState(0);
  /** 已消费的 restartTick，用于区分"重开"与"换张" */
  const lastRestartRef = useRef(0);
  /**
   * **已累计的播放时长（ms）**。
   *
   * 暂停要"冻结剩余时间"：恢复后从剩余处继续，而不是把整张重播一遍
   * ——否则暂停 20 秒回来发现这段白看了，且节奏参数形同虚设。
   * 换张 / 重开时清零。
   */
  const accRef = useRef(0);
  /** 上一次调度器服务的 idx：用于区分"换张"（要清零计时与焦点）与"暂停/恢复"（要接着来） */
  const lastIdxRef = useRef(-1);
  /** 列表容器（兵种/建筑/英雄**共用同一个**，随标签切换内容 → 只需一个 ref） */
  const listRef = useRef<HTMLDivElement | null>(null);
  /**
   * 滚轮横滚（拍板：底部城镇列表用滚轮可左右滑）。两个横向滚动容器都接上：
   * 分类列表 + 阵营切换条。用**回调 ref** 合并，保留原 ref 对象给滚动跟随逻辑用。
   */
  const wheelList = useWheelScrollX();
  const wheelSwitch = useWheelScrollX();
  const listRefCb = useCallback(
    (el: HTMLDivElement | null) => {
      listRef.current = el;
      wheelList(el);
    },
    [wheelList],
  );
  const switchRefCb = useCallback(
    (el: HTMLDivElement | null) => {
      switchRef.current = el;
      wheelSwitch(el);
    },
    [wheelSwitch],
  );
  /**
   * 幕布高度：**实测 sc-top 行高**得到（见文件头 `FRAME_H_FALLBACK` 的说明）。
   *
   * ⚠️ 必须用**回调 ref**（而不是 `useRef` + `useEffect(…, [])`）：内容层带 `key={idx}`，
   * 每次换张都会**整棵重挂载** —— 普通 ref 的 effect 只跑一次，观察者会指向已卸载的节点。
   */
  const [frameH, setFrameH] = useState(FRAME_H_FALLBACK);
  const topRoRef = useRef<ResizeObserver | null>(null);
  const topRef = useCallback((el: HTMLDivElement | null) => {
    topRoRef.current?.disconnect();
    topRoRef.current = null;
    if (!el) return;
    const apply = () => {
      const rowH = el.clientHeight;
      // 宽度上限：左列（去掉巡礼区与 gap）能容纳的最大高度，避免幕布压到巡礼区
      const colW = el.clientWidth - ZONE_W - TOP_GAP;
      const maxH = Math.floor(colW / FRAME_RATIO);
      const next = Math.max(220, Math.min(rowH, maxH));
      setFrameH((prev) => (Math.abs(prev - next) < 1 ? prev : next));
    };
    apply();
    if (typeof ResizeObserver !== "undefined") {
      const ro = new ResizeObserver(apply);
      ro.observe(el);
      topRoRef.current = ro;
    }
  }, []);
  const factionAudioRef = useRef<HTMLAudioElement | null>(null);
  const bgmAudioRef = useRef<HTMLAudioElement | null>(null);
  const focusRef = useRef(0);

  const slide = slides[idx];
  const deckEntry = deckList[idx];
  const cd = cacheDir ?? "";
  /**
   * ⚠️ 必须包含 `assetsReady`：图片是异步解码的，如果计时器在素材就绪前起跑，
   * 前几张的停留时长会被"空白画面"吃掉（且 `t0` 不会因此顺延）。
   * 等一下就绪再起跑，等价于"每张都完整看到"。
   */
  const auto = playing && status === "ready" && slides.length > 0 && assetsReady;

  // 每阵营可覆盖的播放参数（三级回退：`deckEntry ?? 领域级 ?? 默认`）
  const focusStep = deckEntry?.focusStepMs ?? realmFocusStep;
  const transition = deckEntry?.transition ?? cfg.transition ?? "fade";

  // ── 设计稿整体等比缩放（§5.9.1） ──────────────────────────────
  const [scale, setScale] = useState(1);
  useEffect(() => {
    // ⚠️ 依赖 `status`：loading / empty 分支的 return 里**没有** stageRef 元素，
    // 若只跑一次（deps=[]），首次提交时 `stageRef.current === null` → 直接 return，
    // 之后永远不再观察 → scale 恒为 1 → 1280 宽的设计稿在窄窗口里左右溢出被裁。
    const el = stageRef.current;
    if (!el) return;
    const fit = (w: number, h: number) => setScale(Math.min(w / DESIGN_W, h / DESIGN_H));
    const ro = new ResizeObserver((entries) => {
      const r = entries[0]?.contentRect;
      if (r) fit(r.width, r.height);
    });
    ro.observe(el);
    fit(el.clientWidth, el.clientHeight);
    return () => ro.disconnect();
  }, [status]);

  /**
   * **字幕排版的响应式补偿**（作用于 `CAP_*` 的基准值）。
   *
   * 两级缩放的关系要分清，否则会做反：
   * ① 全局 `installUiScale` 按窗口逻辑尺寸 zoom 整个 WebView；
   * ② 本 overlay 再把 1280×720 画布 `scale` 到窗口里。
   * 1024 → 1080P → 2K → 4K 一路往上时，`scale`≈1 而 zoom 变大 → 字幕**已经在放大**，此处恒为 1；
   * 只有"窗口比设计稿小"（`scale < 1`，例如还原态 ~1000×600）才会把设计稿整体压小，
   * 这时才反向补回去（上限 1.15，留够幕布空间）。
   *
   * ⚠️ 不能反过来写成 `1 / scale` 无条件生效：那样在 4K 上 `scale > 1` 会把字**缩小**，
   * 恰好抹掉了大屏应有的放大收益。
   */
  const typeBoost = scale >= 1 ? 1 : Math.min(1.15, 1 / Math.max(scale, 0.01));

  useEffect(() => {
    const mq = window.matchMedia("(prefers-reduced-motion: reduce)");
    const apply = () => setReduced(mq.matches);
    apply();
    mq.addEventListener("change", apply);
    return () => mq.removeEventListener("change", apply);
  }, []);



  // 就绪门（放在数据 effect 之前：effect 需要它做守卫与依赖）。
  const ready = useWikiStagesDone(["data", "assets"]);

  // ── 加载 deck + slide 数据 ───────────────────────────────────
  useEffect(() => {
    // ⚠️ 未就绪不加载：`readWikiIndex` / `readShowcaseSlides` 读的是 **data 产物**，
    // 提前跑会拿到空结果，而依赖（`realm`）不变就**不会重跑** → 就绪后巡礼页空白。
    if (!ready) return;
    let alive = true;
    (async () => {
      const [cd, idxData, realMods, configDir] = await Promise.all([
        getModsCacheDir(),
        readWikiIndex(),
        scanRealMods(),
        realmConfigDir(realm.id),
      ]);
      const modPathMap = new Map<string, string>();
      for (const m of realMods as ScanMod[]) {
        const id = m.id ?? m.modId;
        if (id && m.modPath) modPathMap.set(id, m.modPath);
      }
      modPathRef.current = modPathMap;
      // 本领域启用 mod 集合（auto-deck 过滤源，设计 §7.1）。
      // 编辑器预览会直接注入 `enabledIds`：编辑期改动尚未落盘，读 modSettings 读不到。
      // ⚠️ 三态：null = 读不到（退化为全量兜底）；Set（可能为空）= 权威值，空集就该是空。
      let enabled: Set<string> | null = enabledIds ?? null;
      if (!enabled) {
        const isGlobal = realm.dirs?.configPath === "global";
        const settings = configDir ? await readModSettingsJson(configDir) : null;
        enabled = settings
          ? new Set(collectEnabledIds(settings, realm, isGlobal).ids)
          : null;
      }
      const deck = await buildShowcaseDeck(realm, idxData, modPathMap, enabled, sessionSeed);
      if (!alive) return;
      if (deck.length === 0) {
        setStatus("empty");
        return;
      }
      const entries: ShowcaseEntry[] = deckToEntries(deck, modPathMap);
      const sl = await readShowcaseSlides(entries);
      if (!alive) return;
      setCacheDir(cd);
      // ⚠️ `slides` 与 `deckList` 必须**同序同长**：后面全部用 `slides[idx]` / `deckList[idx]`
      // 取"当前这一张"，直接 `filter(missing)` 会让两者错位 —— 于是每张的音乐参数
      // （`musicOffsetMs`/`musicVolume`/`musicUseRealmBgm`）与巡礼清单会套到**另一个阵营**上。
      // 自动编排更容易命中（自动选出的阵营里缺素材的比例更高）。
      // 这里按 `modId::factionId` 配对后再一起过滤，不依赖后端返回顺序。
      const key = (m: string, f: string) => `${m}::${f}`;
      const byKey = new Map(deck.map((d) => [key(d.modId, d.factionId), d]));
      const keepSlides: ShowcaseSlide[] = [];
      const keepDeck: ShowcaseDeckEntry[] = [];
      for (const s of sl) {
        const d = byKey.get(key(s.modId, s.factionId));
        if (s.missing || !d) continue;
        keepSlides.push(s);
        keepDeck.push(d);
      }
      setDeckList(keepDeck);
      setSlides(keepSlides);
      setStatus(keepSlides.length > 0 ? "ready" : "empty");
    })().catch((e) => {
      // ⚠️ 容错底线：加载链路任何一环抛错都不允许把用户困死在 loading 页
      // （此前 buildShowcaseDeck 在老领域缺 showcase 字段时抛 TypeError → 永远"正在准备阵营巡礼…"）。
      // 落到 empty 页，那里有"退出巡礼"按钮可用。
      if (!alive) return;
      console.warn("[showcase] 牌组加载失败，回退空场次", backendErrorText(e));
      setStatus("empty");
    });
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [realm, ready]);

  // ── 派生：三类**清单**（参与巡礼）与**全部项**（列表展示） ──────────
  //
  // 统一模型（2026-09-21 拍板）：**两种模式共用同一套 UI 与同一条巡礼序列**
  //   · 序列顺序固定 = **兵种 → 建筑 → 英雄**；
  //   · 列表展示**全部项**（未参与项暗显，仍可点开"看一眼"）；
  //   · 区别只在"谁移动选中项"：定时器（自动播放） / 用户点击（手工查看）。
  /** 作者勾选的三类清单（有序，按勾选顺序）；**列表本身就是参与巡礼的范围** */
  const tourCreatureIds = deckEntry?.highlightCreatures ?? [];
  const tourHeroIds = deckEntry?.highlightHeroes ?? [];
  /** 作者**显式勾选**的建筑（自动巢穴另算，见 `tourBuildingIds`） */
  const authoredBuildingIds = deckEntry?.highlightBuildings ?? [];
  /** 「★ 重点」：与参与清单相互独立；焦点项是★时，巡礼区卡片加金色描边特色 */
  const keyCreatures = new Set(deckEntry?.keyCreatures ?? []);
  const keyBuildingSet = new Set(deckEntry?.keyBuildings ?? []);
  const keyHeroSet = new Set(deckEntry?.keyHeroes ?? []);
  /** 「兵种展示」口径：只在**没勾巡礼兵种**时生效（勾了就以清单为准） */
  const cm = cfg.creatureMode ?? "tiers";

  // 兵种形态：`creatureList` 含基础与进阶，用 `upgradedFrom` 区分（§5.3 / 拍板「仅进阶」）
  const isUpgraded = (c: { upgradedFrom?: string[] }) => !!(c.upgradedFrom?.length);
  const allCreatures = slide?.creatures ?? [];
  const modeCreatures = allCreatures.filter((c) => {
    // `highlight` 是废弃值（老配置残留）：按 `tiers` 处理，与编辑器下拉显示一致
    if (cm === "tiers" || cm === "highlight") return !isUpgraded(c);
    if (cm === "upgraded") return isUpgraded(c);
    return true; // all
  });
  /**
   * 三类清单：
   * - **兵种**：勾了 → 只巡这些；没勾 → 按「兵种展示」过滤后的全部（7 项 ≈22s，开箱即用）
   * - **建筑**：**不勾就不播放**（拍板）—— 建筑多且价值参差，交给作者挑
   * - **英雄**：勾了 → 只巡这些；没勾 → **每个职业各 1 个**（确定性的亮相，不再随机轮换）
   */
  const tourCreatures =
    tourCreatureIds.length > 0
      ? tourCreatureIds.flatMap((id) => allCreatures.filter((c) => c.id === id))
      : modeCreatures;
  const allBuildings = slide?.buildings ?? [];
  /**
   * 巡礼兵种的**招募建筑**（自动解析，按兵种顺序）—— 走共用的 `resolveDwellingIds`（名称优先），
   * 与编辑器的自动项**同源**。
   */
  const autoBuildingIds = resolveDwellingIds(allBuildings, tourCreatures, lang);
  /**
   * **建筑巡礼清单 = 作者勾选里排除"招募建筑"的部分**（拍板）。
   *
   * ⚠️ 招募建筑**不再进建筑序列**：兵种焦点那一步已经用「招募建筑」次块展示过（大图 + 名称），
   * 再逐个当焦点播一遍纯属重复。所以建筑段只巡"**非巢穴**且作者勾选"的建筑。
   * （从前播放器只读作者勾选、且没有自动项 → 自动编排下建筑段恒为空，那是另一个 bug。）
   */
  const tourBuildingIds = authoredBuildingIds.filter((id) => !autoBuildingIds.includes(id));
  const tourBuildings = tourBuildingIds.flatMap((id) => allBuildings.filter((b) => b.id === id));
  /**
   * 自动播放的建筑**列表**：**招募建筑在前**（它们跟着兵种顺序，是"这个阵营的兵从哪来"的对照），
   * 后面才是额外参与巡礼的建筑。
   *
   * ⚠️ 顺序与**播放序列相反**是有意的：播放时招募建筑已在兵种那一站展示过（不再单独成站），
   * 但列表把它放前面才与兵种顺序对得上（拍板）。
   * 它们在列表里**暗显**（未参与建筑巡礼），点开仍可看。
   */
  const buildListAuto = [
    ...autoBuildingIds,
    ...tourBuildingIds.filter((id) => !autoBuildingIds.includes(id)),
  ].flatMap((id) => allBuildings.filter((b) => b.id === id));
  const allHeroes = slide?.heroes ?? [];
  const tourHeroes =
    tourHeroIds.length > 0
      ? tourHeroIds.flatMap((id) => allHeroes.filter((h) => h.id === id))
      : defaultHeroIds(allHeroes);

  /** 三类清单的 id 集合（列表用它们判断"是否参与"→ 未参与暗显） */
  const tourCreatureSet = new Set(tourCreatures.map((c) => c.id));
  const tourBuildingSet = new Set(tourBuildings.map((b) => b.id));
  /** 招募建筑集合：自动播放的建筑列表里它们**暗显**（已在兵种那一步展示过），仅作对照 */
  const autoBuildingSet = new Set(autoBuildingIds);
  const tourHeroSet = new Set(tourHeroes.map((h) => h.id));

  /**
   * **手工查看模式**（时间冻结，用户自由点击列表查看）。
   * 与播放模式共用同一套 UI，差别只在"谁移动选中项"。
   */
  const inspect = !playing;

  /**
   * **列表内容随模式变化**（拍板）：
   * - **自动播放** = 只列"这一趟会巡礼的"（所见即所播；否则列表里混着一堆永远轮不到的项）；
   * - **手工查看** = 列**全部**（自由翻看，未参与的暗显但仍可点开）。
   * 两模式共用同一套卡片外观，只是数据集不同 —— 切模式时不会"换一套 UI"。
   */
  // ⚠️ 手工查看 = **全部项**（含"兵种展示"过滤掉的那些，如升级形态）——
  //    `modeCreatures` 会按 `creatureMode` 过滤，实测下就是"手工查看里兵种没显示全"。
  const creatureList = inspect ? allCreatures : tourCreatures;
  const buildingList = inspect ? allBuildings : buildListAuto;
  const heroList = inspect ? allHeroes : tourHeroes;



  /**
   * **巡礼序列**：兵种 → 建筑 → 英雄（三类拼接，顺序固定）。
   * `focusIdx` 是它上面的游标，自动播放按「焦点切换间隔」推进。
   */
  const tourSeq: FocusItem[] = [
    ...tourCreatures.map((c) => ({ kind: "creatures" as const, id: c.id })),
    ...tourBuildings.map((b) => ({ kind: "buildings" as const, id: b.id })),
    ...tourHeroes.map((h) => ({ kind: "heroes" as const, id: h.id })),
  ];
  const focusOn = tourSeq.length > 0;

  /**
   * 「看一眼」目标（列表点击的结果）。
   * - 手工查看：一直有效，直到点别的或按「继续播放」；
   * - 自动播放：**下一格时自动清空**，回到巡礼序列的当前位置（自愈，不会打乱顺序）。
   */
  const [peek, setPeek] = useState<FocusItem | null>(null);
  /** 当前展示项：优先 peek；否则巡礼序列的当前游标 */
  const cur: FocusItem | undefined =
    peek ?? tourSeq[Math.min(focusIdx, Math.max(0, tourSeq.length - 1))];

  /**
   * **换阵营（换张）→ 一律复位**：清掉「看一眼」、焦点游标回到第一项。
   *
   * ⚠️ 不能只靠调度器里的复位：**手工查看（暂停）时调度器直接 `return`** ——
   * 于是"上一张 / 下一张"换了张，但巡礼区还停在你上次点的那一项、焦点也没动，
   * 观感就是**按钮没反应**（实测反馈）。这里独立于播放状态，两种模式都生效。
   */
  useEffect(() => {
    setPeek(null);
    focusRef.current = 0;
    setFocusIdx(0);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [idx]);

  /**
   * **恢复播放**（不论从哪个入口）→ 清掉「看一眼」。
   *
   * 按钮里已经清了一次；这里覆盖**其它入口**（最典型是**空格键**，它只 flip `playing`）
   * —— 否则"暂停 → 点别处 → 按空格继续"时巡礼区仍停在你点过的那一项，
   * 要等下一个焦点步才跳回来，观感就是"没回到原位置"。
   *
   * ⚠️ 不要在这里动 `focusIdx`：那是**自动播放的真实位置**，必须保留
   * （动它就会与进度条错位，见调度器里那条注释）。
   */
  useEffect(() => {
    if (playing) setPeek(null);
  }, [playing]);

  /** 是否最后一张：决定「下一个阵营」能否点，以及「继续播放」的提示语 */
  const isLast = idx >= slides.length - 1;
  /** 剩余秒数（进度指示）。停留时长是自动算出来的，不显示就只剩"怕被切走"的焦虑 */
  const remainSec = Math.max(0, Math.ceil((progress.total - progress.elapsed) / 1000));

  // 当前展示项（三类之一）—— 巡礼区与列表高亮都看它。
  // ⚠️ 一律从**全部项**（`allCreatures`）里按 id 取，**不能**用 `modeCreatures`：
  //    后者被「兵种展示」过滤过（如"仅进阶"只剩进阶兵种），而手工查看的列表是**全部兵种** ——
  //    点一个未升级兵种就查不到 → 巡礼区显示成"— / 暂无描述"（实测反馈）。
  const focusCreature =
    cur?.kind === "creatures" ? allCreatures.find((c) => c.id === cur.id) : undefined;
  const focusBuilding =
    cur?.kind === "buildings" ? allBuildings.find((b) => b.id === cur.id) : undefined;
  const focusHero = cur?.kind === "heroes" ? allHeroes.find((h) => h.id === cur.id) : undefined;
  /** 该展示项是否为★重点（决定巡礼区卡片的金色描边特色） */
  const focusKey =
    (focusCreature && keyCreatures.has(focusCreature.id)) ||
    (focusBuilding && keyBuildingSet.has(focusBuilding.id)) ||
    (focusHero && keyHeroSet.has(focusHero.id)) ||
    false;

  // 兵种 → 巢穴建筑（§5.6）。**进阶兵种优先取进阶巢穴**（`dwellingUpLvl<N>`），
  // 否则进阶兵种会联动到基础兵种的建筑（"进阶兵种没对齐进阶建筑"的根因）。
  /**
   * 当前焦点的**招募建筑**（联动）：走共用的 `resolveDwellingId`（**名称优先**，再按 id 候选
   * 且允许模组前缀）—— 与「建筑巡礼清单的自动项」「编辑器的自动置顶」**同源**。
   * 只有兵种焦点才有；建筑/英雄焦点没有"招募建筑"这一说。
   */
  const dwellingId = slide && focusCreature ? resolveDwellingId(allBuildings, focusCreature, lang) : null;
  const dwelling = dwellingId ? (allBuildings.find((b) => b.id === dwellingId) ?? null) : null;
  /**
   * 联动失败时留证据：打印兵种、缓存带来的巢穴名与建筑清单样本，
   * 下一次"没有招募建筑"就能立刻定位是 id 约定还是数据缺失（而不是反复猜）。
   */
  useEffect(() => {
    if (!focusCreature || dwellingId) return;
    console.warn("[showcase] 未匹配到招募建筑", {
      creature: focusCreature.id,
      level: focusCreature.level,
      upgraded: isUpgraded(focusCreature),
      /** 缓存带来的巢穴名（多语言）—— 为空说明是老缓存，重建缓存即可 */
      dwellingNames: focusCreature.dwellingNames ?? null,
      buildings: allBuildings.slice(0, 8).map((b) => `${b.id}|${pickName(b.name, lang) || ""}`),
    });
  }, [focusCreature?.id, dwellingId, idx]); // eslint-disable-line react-hooks/exhaustive-deps
  // 幕布高亮：优先用构建期落盘的 border 描边图（像素级贴合整栋建筑，§5.9）。
  // 兵种焦点 → 它的招募建筑；建筑焦点 → 建筑自身（自动 / 手工同一套 id，都能在 structures 里命中）。
  // 装饰性建筑（如旗帜 / 飞鸟，无城屏实体）在 structures 里查不到 → 走下方圆形光晕兜底。
  const pin =
    slide && dwelling
      ? (slide.structures ?? []).find((s) => s.id === dwelling.id) ?? null
      : null;
  const buildingPin =
    slide && focusBuilding
      ? (slide.structures ?? []).find((s) => s.id === focusBuilding.id) ?? null
      : null;
  // 当前幕布高亮：兵种与建筑不会同时是焦点，二选一（兵种优先）
  const activePin = pin ?? buildingPin;
  const linkedBuilding = dwelling ?? focusBuilding ?? null;

  /** 把容器内第 i 个子元素横向滚进可视区。
   * ⚠️ 不用 `scrollIntoView`：它会连带滚动**祖先/视口**，而这里是定尺设计稿，
   * 祖先一被滚就把整页顶走（表现为"内容跑到可视区外"）。只操作自己的 scrollLeft 才安全。 */
  const scrollChildIntoView = (c: HTMLDivElement | null, i: number) => {
    const el = c?.children[i] as HTMLElement | undefined;
    if (!c || !el) return;
    const target = el.offsetLeft - (c.clientWidth - el.clientWidth) / 2;
    c.scrollTo({ left: Math.max(0, target), behavior: "smooth" });
  };

  // 阵营切换条：当前项滚进可视区（列表可能长于设计稿宽度）
  useEffect(() => {
    scrollChildIntoView(switchRef.current, idx);
  }, [idx]);

  /**
   * 自动播放时，**列表分类跟随当前焦点分类**（兵种 → 建筑 → 英雄）。
   * 不做的话会出现"巡礼区在讲建筑、下面的列表还停在兵种"这种自相矛盾的画面。
   * 手工查看模式**不跟随**（用户自己点哪个分类就看哪个）。
   */
  useEffect(() => {
    if (!inspect && cur) setListTab(cur.kind);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [cur?.kind, inspect]);

  /**
   * 焦点列表：**把当前展示项滚进可视区**（自动播放时随计时推进，手工查看时随点击）。
   * 索引在该分类的"全部项"里找（列表展示的是全部项，不是只有参与项）。
   */
  useEffect(() => {
    if (!listRef.current) return;
    // 与列表渲染用**同一份数据集**（自动=巡礼项 / 手工=全部），否则下标会错位
    const list =
      listTab === "creatures" ? creatureList : listTab === "buildings" ? buildingList : [];
    const curId =
      listTab === "creatures"
        ? focusCreature?.id
        : listTab === "buildings"
          ? focusBuilding?.id
          : focusHero?.id;
    if (listTab === "heroes") {
      // 英雄列表带职业分组标签（会插进 DOM），下标不再直接可用 → 按 data-hid 查找
      const el = listRef.current.querySelector<HTMLElement>(`[data-hid="${curId}"]`);
      if (el) {
        const target = el.offsetLeft - (listRef.current.clientWidth - el.clientWidth) / 2;
        listRef.current.scrollTo({ left: Math.max(0, target), behavior: "smooth" });
      }
      return;
    }
    const i = list.findIndex((x) => x.id === curId);
    if (i >= 0) scrollChildIntoView(listRef.current, i);
  }, [idx, focusIdx, focusCreature?.id, focusBuilding?.id, focusHero?.id, listTab]);

  /**
   * 跳转到某一张（上一张 / 下一张 / 进度点 / 键盘 ←→）。
   *
   * 语义（拍板）：
   * - **目标不是当前张** → 换张；新张**从头开始计满**它的停留时长
   *   （手动跳转的意图是"我要看这一张"，按剩余时间又立刻切走等于没看到）；
   * - **目标就是当前张**（单张牌组 / 绕回自己）→ **重开这一张**。
   *   从前这里直接 `setIdx(同一个值)` = 无任何反应，于是单张牌组下
   *   「上一张 / 下一张」看起来是坏的按钮。
   */
  const go = (next: number) => {
    const n = slides.length;
    if (n === 0) return;
    const target = ((next % n) + n) % n;
    if (target === idx) {
      setRestartTick((t) => t + 1);
      return;
    }
    setIdx(target);
  };

  /**
   * 点击列表项 → 设定「看一眼」目标（`peek`）。
   *
   * 两种模式**同一套语义**：
   * - 手工查看：一直有效，直到点别的或按「继续播放」（从这儿接着走）；
   * - 自动播放：**下一格自动失效**（见调度器里 `setPeek(null)`），回到巡礼序列的当前位置 ——
   *   这样"随手点一下看细节"不会把自动顺序打乱，也不需要额外状态。
   * 未参与巡礼的项**同样可以点**（列表里暗显但可查看），这正是"全貌可见"的价值。
   */
  const focusAt = (kind: FocusKind, id: string) => {
    setPeek({ kind, id });
    setListTab(kind);
  };

  /**
   * 切换分类：把「看一眼」目标设成该分类的第一项（避免巡礼区还显示上一个分类的东西），
   * 并让列表滚到开头。
   */
  const switchTab = (k: FocusKind) => {
    setListTab(k);
    setPeek({ kind: k, id: firstIdOf(k) });
    listRef.current?.scrollTo({ left: 0, behavior: "smooth" });
  };
  /** 该分类的第一项 id（空分类时给空串，`focus*` 查不到就不显示，安全） */
  const firstIdOf = (k: FocusKind) =>
    k === "creatures"
      ? (creatureList[0]?.id ?? "")
      : k === "buildings"
        ? (buildingList[0]?.id ?? "")
        : (heroList[0]?.id ?? "");
  const tabLabel = (k: FocusKind) =>
    k === "creatures"
      ? t.realm.showcaseCreatures
      : k === "buildings"
        ? t.realm.showcaseBuildings
        : t.realm.showcaseHeroes;
  /** 标签上的计数：`参与/全部` —— 一眼看出该分类有多少项会进巡礼 */
  const tabTours = (k: FocusKind) =>
    k === "creatures"
      ? tourCreatures.length
      : k === "buildings"
        ? tourBuildings.length
        : tourHeroes.length;
  /** 「全部」= 该分类的**完整项数**（分母）。⚠️ 兵种不是 `modeCreatures`：那是"兵种展示"过滤后的
   *  子集（默认只看未升级），拿它当分母会让 `7/7` 掩盖"其实有 14 个兵种"（实测反馈）。 */
  const tabTotals = (k: FocusKind) =>
    k === "creatures"
      ? allCreatures.length
      : k === "buildings"
        ? allBuildings.length
        : allHeroes.length;
  /** 该分类的"参与"判定（列表暗显 vs 亮显） */
  const inTour = (kind: FocusKind, id: string) =>
    kind === "creatures"
      ? tourCreatureSet.has(id)
      : kind === "buildings"
        ? tourBuildingSet.has(id)
        : tourHeroSet.has(id);

  // ── 双层轮播调度器（§7.6）：外层城镇 + 中层兵种焦点 + 旁支英雄 ──
  useEffect(() => {
    // 本条 effect 每重跑一次（换张 / 暂停恢复）就让**上一条提前淡出链自杀**：
    // 只靠 `cancelAnimationFrame` 不够稳（句柄可能已被覆盖），代际号是最终判据。
    preFadeGenRef.current += 1;
    if (!auto) return;
    /**
     * ⚠️ 这里**不能**无条件 `focusRef = 0 / setFocusIdx(0) / setPeek(null)`：
     *    本 effect 在"**暂停 → 继续播放**"时也会重跑，那样会**把焦点归零**，
     *    而计时是接着 `accRef` 走的 → 画面回到第一项、进度条却停在原处
     *    —— 正是"继续播放后不回到原位置 / 进度条与实际位置对不上"（实测反馈）。
     *    归零只该发生在**换张 / 重开**，那由下面的 `lastIdxRef / lastRestartRef` 分支负责。
     */
    // ⚠️ 不再有「最大停留上限」（拍板删除）：停留时长 = 焦点格数 × 间隔，本来就是
    //    从巡礼清单推算出来的，见 `estimateStayMs`。
    const untilFocus = cfg.untilFocusExhausted ?? true;
    /**
     * 「每个城镇停留」**自动计算**（不再让作者手配，见编辑器说明），口径抽到
     * `estimateStayMs` —— **编辑器显示的"预计播放时间"用的就是同一函数**，
     * 否则"编辑器里看到的数"与"播放器真正切走的时刻"必然漂移。
     */
    const { stayMs: effFactionMs } = estimateStayMs({
      focusSlots: tourSeq.length,
      focusStepMs: focusStep,
      untilFocus,
      durationMs: deckEntry?.durationMs,
    });
    // 停留与切场点现在是同一个时刻（自动计算），提前淡出按它往前推
    const switchAtMs = effFactionMs;
    const preFadeMs = Math.max(0, effMusicPreFadeMs);
    /** 循环播放（默认 **false** = 走完一趟就结束，退出巡礼回到领域主界面） */
    const loopAll = cfg.loop === true;
    preFadeDoneRef.current = false;
    let raf = 0;
    let cancelled = false;
    /**
     * 换张 / 重开 → 计时清零、焦点回到第一格；
     * **暂停恢复 → 什么都不重置**，从已累计的进度接着播（见 `accRef`）。
     */
    if (lastIdxRef.current !== idx || lastRestartRef.current !== restartTick) {
      lastIdxRef.current = idx;
      lastRestartRef.current = restartTick;
      accRef.current = 0;
      focusRef.current = 0;
      setFocusIdx(0);
      setPeek(null);
    }
    let base = accRef.current; // 本次运行的起点偏移（暂停恢复时非 0）
    let t0 = performance.now();
    let lastStep = t0;
    let lastUi = 0;

    /** 原地重开一轮（单张 + 循环时用；**必须自己把下一帧排上**） */
    const restart = () => {
      preFadeDoneRef.current = false;
      focusRef.current = 0;
      setFocusIdx(0);
      setPeek(null);
      base = 0;
      accRef.current = 0;
      t0 = performance.now();
      lastStep = t0;
      raf = requestAnimationFrame(loop);
    };

    /**
     * 切换到下一张 / 重开一轮 / **结束**。
     *
     * ⚠️ 这里**绝不能"返回但不排下一帧"**。旧写法在 `deck` 只有 1 张时：
     * `go(idx + 1)` → `setIdx(0)` 值没变 → React 不重渲染 → 本条 effect 不重跑 →
     * 而 `return` 又跳过了 `requestAnimationFrame(loop)` → **rAF 链彻底断掉**：
     * 兵种不再轮换、外层不再切张、也永远不会结束 —— 表现为"只有一个阵营时预览不对 /
     * 超过停留时间也不自动结束 / 配了高亮但兵种不再自动播放"，全是这一个根因。
     */
    const advance = () => {
      if (idx < slides.length - 1) {
        go(idx + 1); // idx 变化 → 本条 effect 重跑，新一轮接管
        return;
      }
      if (loopAll) {
        if (slides.length > 1) go(0);
        else restart();
        return;
      }
      onExit(); // 走完一趟 → 结束
    };

    const loop = (now: number) => {
      if (cancelled) return;
      // ⚠️ 必须带 `base`：暂停恢复后 `t0` 是新起点，只有 `now - t0` 会把已播时长丢掉（整张重播）
      const elapsed = base + (now - t0);
      // 进度指示（节流 ~200ms，避免每帧 setState）
      if (now - lastUi >= 200) {
        lastUi = now;
        setProgress({ elapsed, total: effFactionMs });
      }
      // 提前淡出：自动切到下一阵营前先把当前阵营音乐降下来（仅阵营音乐，BGM 不参与）
      if (
        !preFadeDoneRef.current &&
        preFadeMs > 0 &&
        switchAtMs > preFadeMs &&
        elapsed >= switchAtMs - preFadeMs
      ) {
        preFadeDoneRef.current = true;
        const fa = factionAudioRef.current;
        if (fa && cfg.musicFaction !== false && !fa.paused) {
          const from = fa.volume;
          // ⚠️ 淡出时长夹到「距切换还剩的时间」：否则淡出会跨过切换点继续跑，
          //    与换张后的淡入互相覆盖 → 下一个阵营的音乐恒为 0（"切张后没音乐"的成因之一）。
          const dur = Math.min(
            effMusicFadeMs,
            Math.max(120, switchAtMs - elapsed),
          );
          const s0 = performance.now();
          const pgen = preFadeGenRef.current; // 本次运行代号（换张时 +1 → 旧链自杀）
          const step = (t: number) => {
            if (preFadeGenRef.current !== pgen) return; // 已换张 → 立刻停手，别再和新淡入抢音量
            const k = prog(t, s0, dur);
            setVol(fa, from * (1 - k));
            if (k < 1) preFadeRafRef.current = requestAnimationFrame(step);
          };
          preFadeRafRef.current = requestAnimationFrame(step);
        }
      }
      // 外层硬上限
      if (elapsed >= effFactionMs) {
        advance();
        return;
      }
      // 中层焦点推进（**兵种 → 建筑 → 英雄** 同一条序列，不再有"旁支英雄轮换"）
      if (focusOn && now - lastStep >= focusStep) {
        lastStep = now;
        // 推进的同时**清掉「看一眼」目标**：自动播放不该被随手一点打乱顺序（下一格自愈）
        setPeek(null);
        const next = focusRef.current + 1;
        if (next >= tourSeq.length) {
          if (untilFocus) {
            advance();
            return;
          }
          focusRef.current = cfg.focusLoop === false ? focusRef.current : 0;
        } else {
          focusRef.current = next;
        }
        setFocusIdx(focusRef.current);
      }
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return () => {
      cancelled = true;
      cancelAnimationFrame(raf);
      // 提前淡出是同一条 effect 之外的 rAF 链，必须一起撤销（见 preFadeRafRef 注释）
      cancelAnimationFrame(preFadeRafRef.current);
      // **累计已播时长**（暂停/恢复、参数微调导致的 effect 重跑都会走到这里）。
      // 换张时下一轮会把 `accRef` 清零，所以这里"多存"不会串到别的阵营上。
      accRef.current = base + (performance.now() - t0);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [auto, idx, restartTick, slides.length, focusOn, tourSeq.length, focusStep]);

  // ── 音乐解析：磁盘绝对路径直接；zip 内异步抽取 ────────────────
  useEffect(() => {
    let cancelled = false;
    if (!slide) {
      setAudioPath(null);
      return;
    }
    const resolve = async () => {
      if (slide.music) {
        // 诊断日志：音乐解析链路（F12 控制台可见）。定尺设计稿下"没声音"最难定位，
        // 这里把后端解析出的路径显式打出来，便于区分"后端没解析到"与"前端播放被拒"。
        console.info(
          `[showcase] 阵营音乐 · 磁盘路径 · ${slide.modId}/${slide.factionId} → ${slide.music}`,
        );
        if (!cancelled) setAudioPath(slide.music);
        return;
      }
      if (slide.musicZip) {
        const mp = modPathRef.current.get(slide.musicZip.modId) ?? "";
        console.info(
          `[showcase] 阵营音乐 · zip 抽取 · ${slide.musicZip.modId} rel=${slide.musicZip.rel} modPath=${mp || "(空)"}`,
        );
        const p = await ensureShowcaseAudio(mp, slide.musicZip.modId, slide.musicZip.rel);
        console.info(`[showcase] zip 抽取结果 → ${p ?? "null（失败/超限）"}`);
        if (!cancelled) setAudioPath(p);
        return;
      }
      console.info(
        `[showcase] 该阵营无专属音乐，回退领域 BGM · ${slide.modId}/${slide.factionId} · musicTheme=${slide.musicTheme ?? "null（配置里没有）"}`,
      );
      if (!cancelled) setAudioPath(null);
    };
    void resolve();
    return () => {
      cancelled = true;
    };
  }, [slide]);

  /**
   * 领域素材根目录：调用方没给 `root` 时**自己解析**。
   *
   * 兜底理由：编辑器「巡礼」Tab 的预览曾经传空 root（`assetRoot` 只在「领域登场」Tab 解析），
   * 于是领域 BGM 与自定义底图都解析不出来 —— 而阵营音乐走后端绝对路径，照常能播，
   * 表现为"勾了本张改放领域 BGM 却没声"这种极难定位的现象。
   */
  const [selfRoot, setSelfRoot] = useState("");
  useEffect(() => {
    if (root) {
      setSelfRoot("");
      return;
    }
    let alive = true;
    realmDir(realm.id)
      .then((d) => alive && setSelfRoot(d))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [root, realm.id]);
  const assetRoot = root || selfRoot;

  // ── 交叉淡入（§7.8）：阵营音乐 ↔ 领域 BGM 两路 volume rAF 渐变 ──
  const bgmSrc = assetRoot ? realmAssetUrl(assetRoot, realmAsset(realm, "music", lang)) : "";
  // 每张可覆盖的音乐参数（音乐来源仍由后端按 musicTheme 解析，这里只调怎么播）
  // 音乐细项**只在每个城镇抽屉里逐张可调**（领域级默认值已删：没有界面入口 = 幽灵字段）
  const effMusicOffsetMs = deckEntry?.musicOffsetMs ?? 0;
  const effMusicVolume = deckEntry?.musicVolume ?? 70;
  /** 本张音乐淡入淡出时长 / 提前淡出：抽屉可覆盖，缺省即常量 */
  const effMusicFadeMs = deckEntry?.musicFadeMs ?? MUSIC_FADE_MS;
  const effMusicPreFadeMs = deckEntry?.musicPreFadeMs ?? MUSIC_PRE_FADE_MS;
  /**
   * 本张是否**播放领域 BGM**（来源判定，拍板后的单一口径）：
   *
   * ```
   * 用领域BGM = 未启用阵营音乐  ||  本张勾了「改放领域背景音乐」
   * ```
   *
   * ⚠️ 关键在第一行：作者关掉「启用阵营音乐」的自然理解是"**别用城镇自己的曲子**"，
   * 所以此时改用领域 BGM —— **不是静音**。
   * 另一个方向保持拍板不变：解析不到专属音乐时该站**安静**（不自动串主界面的曲子）。
   */
  const effUseRealmBgm = cfg.musicFaction === false || deckEntry?.musicUseRealmBgm === true;
  /**
   * 领域级「是否播放阵营音乐」：false = 整场只放领域 BGM（领域也没有就不出声）。
   * ⚠️ 与上面的 `effUseRealmBgm` 语义不同：那个是"**这张**改放 BGM"，
   * 这个是"**所有张**都不放阵营音乐"。
   */
  const factionOn = cfg.musicFaction !== false;
  useEffect(() => {
    /**
     * 本次运行的**代号**。每条淡入淡出链在每一帧都检查它是否还是"当前代号"，
     * 不是就直接自杀（连 `then` 都不执行）。
     *
     * ⚠️ 这是"切到下一个阵营没声音"的**真正根因**：`fade()` 的链**没有清理**，
     * 于是"上一张无专属音乐 → `fade(fa, 0, () => fa.pause())`"那条链，
     * 会在**下一张已经 `play()` 之后**才走到终点，把刚播起来的音轨 `pause()` 掉。
     * 时序上它只差 800ms，所以表现为"**大概率**没声音"而不是必现 —— 这也是前几轮
     * 只改换源/重试都没能修掉它的原因（那些是真问题，但不是这一条）。
     */
    const gen = ++fadeGenRef.current;
    const fade = (a: HTMLAudioElement | null, to: number, then?: () => void) => {
      if (!a) return then?.();
      const from = a.volume;
      const dur = effMusicFadeMs;
      // 时长为 0 = 立即到位：不走 rAF（`(now-t0)/0` 会是 NaN/Infinity，进 setVol 就会被兜成 0，
      // 反而把音量莫名压到 0 —— 宁可在这里显式短路）
      if (!(dur > 0)) {
        setVol(a, to);
        return then?.();
      }
      const t0 = performance.now();
      const step = (now: number) => {
        if (fadeGenRef.current !== gen) return; // 已被后续运行取代 → 停止且**不执行 then**
        const k = prog(now, t0, dur);
        setVol(a, from + (to - from) * k);
        if (k < 1) requestAnimationFrame(step);
        else then?.();
      };
      requestAnimationFrame(step);
    };
    const fa = factionAudioRef.current;
    /** 领域 BGM：**优先用领域主界面那一个元素**（`realmBgmRef`）→ 接续播放、不从头开始 */
    const ba = bgmEl();
    const vol = Math.max(0, Math.min(1, effMusicVolume / 100));
    // 玩家已静音（或巡礼内点了 ♪）→ 巡礼也不出声（§7.8）
    if (muted) {
      fade(fa, 0, () => fa?.pause());
      fade(ba, 0, () => ba?.pause());
      return;
    }
    // 优先级：领域级禁用 → 本张改放 BGM → 后端是否解析到 → 回退领域 BGM → 静音
    const target =
      audioPath && factionOn && !effUseRealmBgm ? convertFileSrc(audioPath) : "";
    if (target) {
      // 有阵营音乐：BGM 淡出、阵营音乐淡入
      fade(ba, 0, () => ba?.pause());
      if (fa) {
        fa.loop = true;
        if (faSrcRef.current !== target) {
          // 换源：`src` 刚变更时设 `currentTime` 会被忽略，且此刻 `play()` 常被浏览器以
          // AbortError 拒绝（上一次加载被中断）—— 这两点合起来就是"切到下一个阵营后没有音乐"。
          // 故：起始位置挂到 `onLoadedMetadata` 消费，播放走可重试的 `safePlay`。
          console.info(`[showcase] 阵营音乐换源 → ${slide?.factionId ?? "?"} · ${target}`);
          faSrcRef.current = target;
          pendingOffsetMsRef.current = effMusicOffsetMs;
          fa.src = target;
          fa.load();
        }
        safePlay(fa);
      }
      fade(fa, vol);
      return;
    }
    // 无阵营音乐（未解析到 / 本张改放 BGM）→ 只有**明确要求用领域 BGM** 才放；
    // 否则**本张安静**（拍板：避免巡礼里突然响起领域主页面的主题曲，像是串台）。
    fade(fa, 0, () => fa?.pause());
    if (!effUseRealmBgm || !bgmSrc) {
      // 留日志便于区分三种"没声音"：未解析到阵营音乐 / 未开启回退 / 领域没配 BGM。
      console.info(
        `[showcase] 本张无音乐 · ${slide?.factionId ?? "?"}：` +
          (audioPath
            ? `已解析到阵营音乐但${effUseRealmBgm ? "本张改放领域 BGM 且" : "未开启回退，故不出声"}`
            : "未解析到阵营音乐") +
          `${effUseRealmBgm ? "（领域未配置背景音乐 asset）" : "（已按默认策略静音）"}`,
      );
      fade(ba, 0, () => ba?.pause());
      return;
    }
    if (ba) {
      // ⚠️ 用共享元素时它**已有 src** → 不重设（重设 `src` 会把 `currentTime` 归零，
      //    那就是"从主界面进巡礼，主题曲从头开始"的老毛病）。仅当音源不同才换（罕见）。
      if (ba.getAttribute("src") !== bgmSrc) ba.src = bgmSrc;
      ba.loop = realm.main?.loopMusic !== false;
      // ⚠️ 必须走 `safePlay`：上一张有阵营音乐时，BGM 元素被 `pause()` 过；切到"无专属音乐"的
      //    阵营时要把它重新拉起，此刻 `play()` 常被以 AbortError/NotSupportedError 瞬时拒绝
      //    （尤其在**自动编排**下——自动选出的阵营里没音乐的比例高，于是这一步被频繁触发）。
      //    裸 `play().catch()` = 一次失败就永久静音，外部表现正是
      //    "自动编排下切到下一个阵营不会播放背景音乐"。
      safePlay(ba);
    }
    fade(ba, vol);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    audioPath,
    bgmSrc,
    muted,
    factionOn,
    effMusicOffsetMs,
    effMusicVolume,
    effMusicFadeMs,
    effUseRealmBgm,
    // 暂停/继续也要重跑：提前淡出已把音量降到 0，恢复时需要淡回目标音量
    playing,
  ]);

  /**
   * 音乐看门狗（§7.8 兜底）：自动播放期间，本该出声的那一路若处于暂停，补一次播放。
   *
   * 覆盖换源被 AbortError 拒绝、asset 协议首帧未就绪等**瞬时**情况 —— "切张后没音乐"的最后一道保险。
   * ⚠️ 必须**两路都看**：只盯阵营音乐时，切到"无专属音乐 → 应放领域 BGM"的阵营就没人兜底了，
   * 而这恰恰是自动编排下最常发生的分支。
   */
  useEffect(() => {
    // ⚠️ 用**周期**检查而不是一次性 setTimeout：真正的失败有延迟（例如被过期的淡出链
    //    `pause()` 掉、asset 协议首帧未就绪、play() 被瞬时拒绝），一次性的兜底经常刚好错开。
    //    `muted` 必须排除：静音是用户意图，不能被他兜起来。
    if (!auto || muted) return;
    const tick = () => {
      const fa = factionAudioRef.current;
      const ba = bgmEl();
      // 本张应有阵营音乐 → 兜它；否则兜领域 BGM（有配才兜）
      if (audioPath && !effUseRealmBgm && factionOn) {
        if (fa?.paused) {
          // 留日志：能区分"我们的兜底救回了播放"与"根本没触发"。
          // 若这行频繁出现，说明还有别的东西在暂停音轨（本次已修掉过期淡出链，
          // 这条日志就是它没修干净的探测器）。
          console.warn("[showcase] 阵营音乐处于暂停，兜底重播", {
            idx,
            src: fa.getAttribute("src"),
          });
          safePlay(fa);
        }
      } else if (effUseRealmBgm && bgmSrc && ba?.getAttribute("src") && ba.paused) {
        // ⚠️ 必须带上 `effUseRealmBgm`：否则"本张就该安静"（解析不到阵营音乐 + 不要求 BGM）
        //    的站会被这里**重新拉起 BGM** —— 那正是"没有音乐时突然响起主界面曲子"的串台现象。
        safePlay(ba);
      }
    };
    const id = window.setInterval(tick, 1000);
    return () => window.clearInterval(id);
  }, [auto, muted, audioPath, bgmSrc, effUseRealmBgm, factionOn, idx]);

  // ── 素材就绪门槛：**当前张的图没加载完就不开始计时** ─────────────
  //
  // 不这样就必然出现「巡礼已经开始跑、第一张（以及任何预取没赶上的张）是空白画面」——
  // 计时器从进入那一刻起跑，而 asset 协议的图片是异步解码的，前 1~2 秒画面全空，
  // 停留时长照扣，于是"前面的图片看不到"。
  useEffect(() => {
    if (status !== "ready" || !slide) {
      setAssetsReady(false);
      return;
    }
    const urls = slideImageUrls(slide, cd, hdRate);
    if (urls.length === 0) {
      setAssetsReady(true);
      return;
    }
    setAssetsReady(false);
    // 换张时把进度**清零**：否则在加载期间会短暂显示**上一张**的剩余秒数（计时还没起跑）
    setProgress({ elapsed: 0, total: 0 });
    let alive = true;
    let left = urls.length;
    const done = () => {
      if (alive && --left <= 0) setAssetsReady(true);
    };
    for (const u of urls) {
      const img = new Image();
      // ⚠️ 失败也放行：一张坏图不该把整场巡礼卡死在第一张
      img.onload = done;
      img.onerror = () => {
        // 高清变体缺失就记住（`HdImg` 渲染时直接回退原图，不再重复请求）
        if (u.includes("@")) markHdMissing(u);
        done();
      };
      img.src = u;
    }
    // ⚠️ 超时兜底：请求既不 onload 也不 onerror（协议层挂住）时，宁可带空图开播，
    //    也不能让巡礼停在第一张不动 —— 那是比"图慢一点"严重得多的故障。
    const kill = window.setTimeout(() => {
      if (alive) setAssetsReady(true);
    }, 3000);
    return () => {
      alive = false;
      window.clearTimeout(kill);
    };
  }, [slide, cd, status, hdRate]);

  // ── 图片预取（§8.3）：进入 Slide i 时预取 i+1 的素材（原图 + 高清变体） ───────────
  useEffect(() => {
    if (status !== "ready" || slides.length < 2) return;
    const next = slides[(idx + 1) % slides.length];
    if (!next) return;
    for (const u of slideImageUrls(next, cd, hdRate)) {
      const img = new Image();
      img.onerror = () => {
        if (u.includes("@")) markHdMissing(u);
      };
      img.src = u;
    }
  }, [idx, slides, cd, status, hdRate]);

  // ── 键盘控制（§9.1） ─────────────────────────────────────────
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onExit();
      else if (e.key === "ArrowRight") go(idx + 1);
      else if (e.key === "ArrowLeft") go(idx - 1);
      else if (e.key === " ") {
        e.preventDefault();
        setPlaying((p) => !p);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [idx, slides.length, onExit]);

  if (status === "loading") {
    return (
      <div className="fixed inset-0 z-[150] flex items-center justify-center bg-black/95 text-sm text-slate-300">
        {t.realm.showcaseLoading}
      </div>
    );
  }
  if (status === "empty" || !slide) {
    return (
      <div className="fixed inset-0 z-[150] flex flex-col items-center justify-center gap-4 bg-black/95 text-slate-300">
        <div className="text-sm">{t.realm.showcaseEmpty}</div>
        <button
          type="button"
          onClick={onExit}
          className="rounded-lg border border-slate-700 px-4 py-2 text-sm text-slate-200 hover:bg-slate-800"
        >
          {t.common.close}
        </button>
      </div>
    );
  }

  /**
   * 幕布（城镇全景图）：**永远用缓存素材**。
   * 建筑聚焦高亮（structures）是按它的 800×374 坐标标定的，换图就会对不上位 ——
   * 所以「阵营背景图」不再替换幕布，只换下面那层氛围底图。
   */
  const stageCandidates = fullImgCandidates(cd, slide.backdrop, hdRate);
  const stageImg = stageCandidates[0] ?? "";
  /** 全窗口氛围底图（`inset:-6%` + blur(30px) 的铺底层）：与坐标无关，可由「阵营背景图」覆盖 */
  const ambientImg =
    deckEntry?.overrideBackground && assetRoot
      ? realmAssetUrl(assetRoot, deckEntry.overrideBackground)
      : stageImg;
  const captionText = pickName(deckEntry?.caption, lang) || pickName(slide.name, lang) || slide.factionId;
  const desc = pickName(slide.description, lang) ?? "";
  const anim = (a: string) => (reduced ? undefined : a);

  /**
   * **列表项的统一外观**（两种模式同一套视觉语言）：
   * - 亮/暗 = 是否参与巡礼（列表展示**全部项**，未参与暗显但**仍可点开查看**）；
   * - 金色边框 + 上移 2px + 阴影 = **当前展示项**（建筑、英雄与兵种完全一致）；
   * - 左上小 ★ = 作者标记的重点（主要表达在巡礼区卡片的描边上，这里只作识别）。
   * 位移/阴影/透明度都是**零纵向成本**。
   */
  const cellCls = (isCur: boolean, isInTour: boolean) =>
    cn(
      "relative flex-none cursor-pointer overflow-hidden rounded-[10px] border bg-slate-900/60 px-1 pb-[4px] pt-[5px] text-center backdrop-blur-sm transition-all duration-300",
      isCur
        // 阴影从 26px 收到 18px：滚动容器会裁掉下缘，太大的模糊半径被切得更明显
        ? "-translate-y-[2px] border-[var(--gold,#F5C56B)] shadow-[0_5px_18px_rgba(0,0,0,.5)]"
        : isInTour
          ? "border-slate-700/60 hover:border-slate-500"
          : "border-slate-800/60 hover:border-slate-600",
      // 亮度：**手工查看下只有"当前项"亮**（拍板：其他一律变暗，不再按"是否参与"分两档 ——
      // 手工查看是"逐个看"，一堆中亮的项会让人分不清自己在看哪个）。
      // 自动播放仍保留"参与=90 / 未参与=45"：那时列表就是巡礼进度条，需要看出走到哪一段。
      isCur ? "opacity-100" : inspect ? "opacity-45" : isInTour ? "opacity-90" : "opacity-45",
    );
  /** ★ 小角标 */
  const keyDot = (on: boolean) =>
    on ? (
      <span className="absolute left-1 top-1 text-[10px] leading-none text-[var(--gold,#F5C56B)] [text-shadow:0_0_6px_rgba(0,0,0,.9)]">
        ★
      </span>
    ) : null;

  /** 兵种格（**不限 8 格**：全部项横向滚动看全 —— 旧版折成「+N」就点不到了） */
  const creatureCells = creatureList.map((c) => (
    <div
      key={c.id}
      onClick={() => focusAt("creatures", c.id)}
      title={pickName(c.name, lang) || c.id}
      className={cellCls(focusCreature?.id === c.id, tourCreatureSet.has(c.id))}
      style={{ width: SLOT_W }}
    >
      {c.imageLarge || c.image ? (
        <HdImg
          src={fullImgCandidates(cd, c.imageLarge ?? c.image, hdRate)}
          alt=""
          className="mx-auto block object-contain"
          style={{ width: SLOT_IMG_W, height: SLOT_IMG_H }}
          pixelArt
          draggable={false}
        />
      ) : (
        <div style={{ height: SLOT_IMG_H }} />
      )}
      <div
        className={cn(
          "mt-[2px] block h-[14px] truncate text-[11px] leading-[14px]",
          focusCreature?.id === c.id ? "font-bold text-white" : "text-slate-300",
        )}
      >
        {pickName(c.name, lang) || c.id}
      </div>
      {keyDot(keyCreatures.has(c.id))}
      {c.level != null && (
        <span className="absolute right-1 top-1 rounded-md bg-[var(--gold,#F5C56B)] px-1 text-[9px] font-bold text-[#0F172A]">
          {c.level}
        </span>
      )}
    </div>
  ));

  /** 建筑格（默认**不参与巡礼** —— 作者勾选才播放，故默认整排暗显） */
  const buildingCells = buildingList.map((b) => (
    <div
      key={b.id}
      onClick={() => focusAt("buildings", b.id)}
      title={
        (pickName(b.name, lang) || b.id) +
        // 招募建筑在建筑列表里暗显 → 说清原因，否则像"没被选中"
        (autoBuildingSet.has(b.id) && !tourBuildingSet.has(b.id)
          ? ` · ${t.realm.showcaseDwellingCovered}`
          : "")
      }
      className={cellCls(focusBuilding?.id === b.id, tourBuildingSet.has(b.id))}
      style={{ width: BLD_SLOT_W }}
    >
      {b.image ? (
        // 图位 108×62（与兵种同高）：150×70 的横图按 contain 缩放为 ~108×50 并**居中**，
        // 比例不变、上下留白均等 —— 卡高与兵种/英雄一致。
        <HdImg
          src={fullImgCandidates(cd, b.image, hdRate)}
          alt=""
          className="mx-auto block object-contain"
          style={{ width: BLD_IMG_W, height: BLD_IMG_H }}
          pixelArt
          draggable={false}
        />
      ) : (
        <div style={{ height: BLD_IMG_H }} />
      )}
      {/* 名称行与兵种/英雄**同字号同行高**（卡高一致的前提） */}
      <div
        className={cn(
          "mt-[2px] block h-[14px] truncate text-[11px] leading-[14px]",
          focusBuilding?.id === b.id ? "font-bold text-white" : "text-slate-300",
        )}
      >
        {pickName(b.name, lang) || b.id}
      </div>
      {keyDot(keyBuildingSet.has(b.id))}
    </div>
  ));

  /**
   * 英雄格：**按职业分组**（组前一条竖标 + 竖排职业名）。
   * 分组而不是筛选：零额外点击，一眼看出"这个阵营有哪几类英雄"。
   * 职业名要取**显示名**（`className` 可能是字符串或多语言对象），排序用归一后的键。
   */
  const heroCells: ReactNode[] = (() => {
    const out: ReactNode[] = [];
    /**
     * 排序规则（拍板）：
     * - **自动播放 = 按巡礼顺序**（也就是清单/数据里的顺序，**不做任何重排**）——
     *   列表里金框会**顺着往下走**；从前按职业名重排，观感就是"英雄巡礼在乱跳"。
     * - **手工查看 = 按职业分组**（浏览用，一眼看出这个阵营有哪几类英雄）。
     */
    const sorted = inspect
      ? [...heroList].sort((a, b) => {
          const ka = heroClassKey(a.className);
          const kb = heroClassKey(b.className);
          if (ka !== kb) return ka < kb ? -1 : 1;
          return (pickName(a.name, lang) || a.id).localeCompare(pickName(b.name, lang) || b.id);
        })
      : [...tourHeroes];
    let last = "";
    for (const h of sorted) {
      const k = heroClassKey(h.className);
      const clsName =
        typeof h.className === "string" ? h.className : pickName(h.className, lang);
      if (k !== last) {
        last = k;
        out.push(
          <div key={`grp-${k}`} className="flex flex-none items-center gap-[5px]">
            <span className="h-[48px] w-px bg-slate-700/70" />
            <span className="whitespace-nowrap text-[10px] text-slate-500 [writing-mode:vertical-rl]">
              {clsName || "—"}
            </span>
          </div>,
        );
      }
      out.push(
        <div
          key={h.id}
          data-hid={h.id}
          onClick={() => focusAt("heroes", h.id)}
          title={pickName(h.name, lang) || h.id}
          className={cellCls(focusHero?.id === h.id, tourHeroSet.has(h.id))}
          style={{ width: SLOT_W }}
        >
          {h.image ? (
            <HdImg
              src={fullImgCandidates(cd, h.image, hdRate)}
              alt=""
              className="mx-auto block object-contain"
              style={{ height: SLOT_IMG_H }}
              pixelArt
              draggable={false}
            />
          ) : (
            <div style={{ height: SLOT_IMG_H }} />
          )}
          <div
            className={cn(
              "mt-[2px] block h-[14px] truncate text-[11px] leading-[14px]",
              focusHero?.id === h.id ? "font-bold text-white" : "text-slate-300",
            )}
          >
            {pickName(h.name, lang) || h.id}
          </div>
          {keyDot(keyHeroSet.has(h.id))}
        </div>,
      );
    }
    return out;
  })();

  /**
   * **巡礼区**数据（两种模式共用）：由当前展示项决定 —— 兵种 / 建筑 / 英雄 三套内容。
   * 图片只给一个维度（见 `DETAIL_IMG_*` 注释），`object-contain` 兜住比例，绝不拉伸。
   */
  const zone = (() => {
    if (focusCreature) {
      const lvl = focusCreature.level;
      return {
        img: focusCreature.imageLarge ?? focusCreature.image,
        imgStyle: { height: DETAIL_IMG_H },
        title: pickName(focusCreature.name, lang) || focusCreature.id,
        meta: [
          lvl != null ? (lvl === 0 ? t.realm.showcaseLevelZero : `Lv.${lvl}`) : "",
          isUpgraded(focusCreature) ? t.realm.showcaseUpgradedFrom : "",
          keyCreatures.has(focusCreature.id) ? t.realm.showcaseKeyMark : "",
        ].filter(Boolean),
        desc: pickDesc(focusCreature.description, lang),
        /** 次块：招募建筑（联动）—— 只在兵种焦点出现 */
        linked: linkedBuilding
          ? {
              image: linkedBuilding.image,
              name: pickName(linkedBuilding.name, lang) || linkedBuilding.id,
            }
          : null,
      };
    }
    if (focusBuilding) {
      return {
        img: focusBuilding.image,
        imgStyle: { width: DETAIL_IMG_W },
        title: pickName(focusBuilding.name, lang) || focusBuilding.id,
        meta: [
          tourBuildingSet.has(focusBuilding.id)
            ? t.realm.showcaseInTour
            : t.realm.showcaseNotInTour,
          keyBuildingSet.has(focusBuilding.id) ? t.realm.showcaseKeyMark : "",
        ].filter(Boolean),
        desc: pickDesc(focusBuilding.description, lang),
        linked: null,
      };
    }
    if (focusHero) {
      return {
        img: focusHero.image,
        imgStyle: { height: DETAIL_IMG_H },
        title: pickName(focusHero.name, lang) || focusHero.id,
        meta: [
          typeof focusHero.className === "string"
            ? focusHero.className
            : pickName(focusHero.className, lang),
          tourHeroSet.has(focusHero.id) ? t.realm.showcaseInTour : t.realm.showcaseNotInTour,
          keyHeroSet.has(focusHero.id) ? t.realm.showcaseKeyMark : "",
        ].filter(Boolean),
        desc: pickDesc(focusHero.description, lang),
        linked: null,
      };
    }
    return null;
  })();

  // ⚠️ **未就绪时只渲染蒙层，不渲染舞台**（早返回）：巡礼是整页动画 + 大量图，
  // 蒙层期间把它挂载起来毫无意义（用户看不到），还会白烧 CPU 与解码。
  if (!ready) {
    return (
      <div className="fixed inset-0 z-[150] grid place-items-center bg-[#070b14]">
        <WikiBuildOverlay deps={["data", "assets"]} onExit={onExit} />
      </div>
    );
  }

  return (
    <div
      ref={stageRef}
      className="fixed inset-0 z-[150] grid place-items-center overflow-hidden bg-[#070b14]"
    >
      <style>{KEYFRAMES}</style>
      <div
        className="relative overflow-hidden bg-[#080d18]"
        style={{
          width: DESIGN_W,
          height: DESIGN_H,
          transform: `scale(${scale})`,
          transformOrigin: "center center",
        }}
      >
        {/* ── 幕布三层（§5.2）：模糊铺底 + 居中幕布 + 渐变 scrim ── */}
        {ambientImg && (
          /* 氛围铺底：与幕布同源（缓存 webp → 高清候选）；作者自备的 `overrideBackground`
             是 png/jpg，`imgCandidates` 会原样返回，故这里可以统一走 HdImg。 */
          <HdImg
            src={imgCandidates(ambientImg, hdRate)}
            alt=""
            className="absolute object-cover"
            style={{
              inset: "-6%",
              width: "112%",
              height: "112%",
              filter: "blur(30px) brightness(.40) saturate(1.15)",
            }}
            draggable={false}
          />
        )}
        <div
          className="absolute inset-0"
          style={{
            background:
              "linear-gradient(180deg,rgba(8,13,24,.80) 0%,rgba(8,13,24,.10) 34%,rgba(8,13,24,.55) 72%,rgba(8,13,24,.92) 100%)",
          }}
        />

        {/* ── 内容层（转场作用在整层，随 idx 重挂载，§9） ── */}
        <div
          key={idx}
          className="relative z-[3] flex h-full flex-col px-[26px] pb-[10px] pt-[12px]"
          style={{ animation: anim(TRANSITION_ANIM[transition]) }}
        >
          {/* sc-top：幕布 + 右上巡礼区。**这一行是 `flex-1`** ——
              所有固定高度的区块（标签栏/列表/字幕/切换条）排完之后的剩余空间全给它，
              幕布再按实测行高撑满（见 `frameH`）。这样任何一端改高度都不会在中间留空洞。 */}
          <div
            ref={topRef}
            className="relative flex min-h-0 flex-1 items-start gap-4 overflow-hidden"
          >
            <div className="relative flex min-w-0 flex-1 items-start justify-center">
              <div
                className="relative overflow-hidden rounded-xl border border-[rgba(245,197,107,.42)]"
                style={{ width: Math.round(frameH * FRAME_RATIO), height: frameH }}
              >
                {/* 幕布按**原始比例**缩放铺满（`cover` 不拉伸、只裁边）：
                    缓存幕布是 800×374，frame 是 720×336.6 —— 比例都是 2.1390，**完全一致**，
                    所以 `cover` 与原 `fill` 在此**逐像素等价**；但 `overrideBackground` 是作者自备图
                    （任意比例），用 `fill` 会被**拉伸变形**（本行就是为了修这个）。
                    ⚠️ 不要改回 `fill`；也不要改 `contain` —— 幕布是全出血底图，留边会露出后面的模糊层。 */}
                {/* 幕布：高清优先 + 原图兜底。写实大图**不用** pixelated（见 `HdImg` 头注） */}
                <HdImg
                  src={stageCandidates}
                  alt=""
                  className="block h-full w-full object-cover"
                  draggable={false}
                />
                {/* 幕布高亮：跟随焦点的建筑描边图（§5.9）；兵种 / 建筑焦点共用 */}
                {activePin && (
                  <HdImg
                    src={fullImgCandidates(cd, activePin.image, hdRate)}
                    alt=""
                    className="pointer-events-none absolute"
                    style={{
                      left: `${(activePin.x / BACKDROP_W) * 100}%`,
                      top: `${(activePin.y / BACKDROP_H) * 100}%`,
                      width: `${(activePin.w / BACKDROP_W) * 100}%`,
                      height: `${(activePin.h / BACKDROP_H) * 100}%`,
                      filter:
                        "drop-shadow(0 0 5px rgba(255,214,120,.9)) drop-shadow(0 0 16px rgba(245,197,107,.55))",
                      animation: anim("scPulse 1.7s ease-out infinite"),
                    }}
                    draggable={false}
                  />
                )}
                {/* 兜底：无 structures（装饰性建筑 / 无城屏实体）时用柔和圆形光晕，绝不用矩形硬框 */}
                {!activePin && linkedBuilding && (
                  <div
                    className="pointer-events-none absolute h-24 w-24 -translate-x-1/2 -translate-y-1/2 rounded-full"
                    style={{
                      left: "50%",
                      top: "55%",
                      background:
                        "radial-gradient(circle, rgba(245,197,107,.42), rgba(245,197,107,0) 70%)",
                      animation: anim("scPulse 1.7s ease-out infinite"),
                    }}
                  />
                )}
              </div>
              {/* ⚠️ 幕布上**不放任何 UI 元素**：
                  ① 建筑高亮描边按 800×374 坐标铺在幕布上，**位置不固定**（任何一条边都可能被压），
                     而描边是金色 —— 金色进度条贴底边会与它同色竞争（实测反馈）；
                  ② 幕布是 `rounded-xl overflow-hidden`，贴边的线两头会被圆角裁掉；
                  ③ 从前的「城镇名名牌」也压在这里，且与底部字幕重复、长名还会被 overflow-hidden 裁断。
                  进度条已移到**底部字幕列下方**（见 sc-foot），幕布因此回归纯画面。 */}
            </div>

            {/* 右上「巡礼区」300×400：**两种模式完全同一套**（这是"统一"的核心）。
                内容随当前焦点类型变化：
                  · 兵种 → 焦点块（头像 / 名称 / 级别 / 进阶 / 描述）+ 次块（招募建筑：图 + 名）
                  · 建筑 → 单块（图 + 名称 + 描述）
                  · 英雄 → 单块（头像 + 名称 + 职业 + 描述）
                ★ = 重点：焦点项是★时**整卡加金色描边 + 外发光 + ★ 角标**（★ 的新语义）。
                ⚠️ 图仍只约束一个维度 + `object-contain`（三类比例不同），绝不拉伸。
                ⚠️ 不用浮层盖幕布：幕布上的建筑描边位置不固定，浮层必然压到它。 */}
            <div className="flex flex-none flex-col" style={{ width: ZONE_W }}>
              {/* 与幕布**等高**（`frameH`）：两者底边对齐，中间不会出现参差 */}
              <div
                className={cn(
                  "flex flex-col rounded-xl border bg-slate-900/60 px-3 py-2 backdrop-blur-sm transition-shadow",
                  focusKey
                    ? "border-[var(--gold,#F5C56B)] shadow-[0_0_22px_rgba(245,197,107,.35)]"
                    : "border-slate-700/60",
                )}
                style={{ animation: anim("scHeroIn .34s both"), height: frameH }}
              >
                {/* 阵营行：补上"去掉幕布名牌"后**长城镇名无处可看**的问题 */}
                <div className="flex items-baseline gap-1 text-[10px] leading-[14px] text-slate-500">
                  <span className="min-w-0 flex-1 truncate">{captionText}</span>
                  {focusKey && <span className="shrink-0 text-[var(--gold,#F5C56B)]">★</span>}
                  <span className="shrink-0 tabular-nums">
                    {idx + 1}/{slides.length}
                  </span>
                </div>
                {/* 固定图框：三类图比例不同，靠"只约束一个维度 + object-contain"保证不变形。
                    ⚠️ 这里保持**通栏大图**（不做左右分栏）：巡礼区是"看这一项"的地方，
                    图要占满宽度才有辨识度；密度问题由**底部栏**（左右分栏 + 多行）解决。 */}
                <div
                  className="mt-[6px] grid w-full place-items-center overflow-hidden rounded-lg bg-slate-950/50"
                  style={{ height: ZONE_IMG_BOX_H }}
                >
                  {zone?.img ? (
                    <HdImg
                      src={fullImgCandidates(cd, zone.img, hdRate)}
                      alt=""
                      className="block object-contain"
                      style={{ ...zone.imgStyle }}
                      pixelArt
                      draggable={false}
                    />
                  ) : (
                    <span className="text-[11px] text-slate-600">—</span>
                  )}
                </div>
                {/* 标题 + 属性行 */}
                <div className="mt-[6px] truncate text-[16px] font-bold leading-[21px] text-slate-100">
                  {zone?.title ?? "—"}
                </div>
                <div className="mt-[1px] flex flex-wrap items-center gap-x-2 text-[11px] leading-[16px] text-slate-400">
                  {(zone?.meta ?? []).map((m, k) => (
                    <span key={k}>{m}</span>
                  ))}
                </div>
                {/* 描述：吃掉剩余高度并**滚动**（不截断）；CJK 无空格 → 必须按字换行 */}
                <div className="mt-[6px] min-h-0 flex-1 overflow-y-auto pr-1 text-[12px] leading-[18px] text-slate-300 [overflow-wrap:anywhere]">
                  {zone?.desc ? (
                    zone.desc
                  ) : (
                    <span className="text-slate-500">{t.realm.showcaseNoDesc}</span>
                  )}
                </div>
                {/* 次块：招募建筑（联动）—— 仅兵种焦点出现 */}
                {zone?.linked && (
                  // 招募建筑：**宽幅大图**（建筑原图 150×70，此处约束高度 62 → 渲染约 133×62，
                  // 比从前的 56×36 大三倍多；`object-contain` 保比例、绝不拉伸）。
                  // 该兵种**没有描述**时本块 `flex-1` 吃掉空白（截图里正是大片空白 + 小图看不清）。
                  <div
                    className={cn(
                      "mt-[6px] flex items-center gap-[10px] border-t border-slate-700/60 pt-[8px]",
                      zone.desc ? "flex-none" : "min-h-0 flex-1",
                    )}
                  >
                    <div className="grid h-[64px] w-fit flex-none place-items-center overflow-hidden rounded-md bg-slate-950/50 px-[3px]">
                      {zone.linked.image && (
                        <HdImg
                          src={fullImgCandidates(cd, zone.linked.image, hdRate)}
                          alt=""
                          className="block object-contain"
                          style={{ maxHeight: 62, maxWidth: 150 }}
                          pixelArt
                          draggable={false}
                        />
                      )}
                    </div>
                    <div className="min-w-0">
                      <div className="text-[10px] tracking-[1px] text-slate-500">
                        {t.realm.showcaseLinkedDwelling}
                      </div>
                      <div className="truncate text-[14px] text-slate-200">{zone.linked.name}</div>
                    </div>
                  </div>
                )}
              </div>
            </div>
          </div>

          {/* sc-mid：**标签栏 + 当前分类列表**（兵种 / 建筑 / 英雄）。
              两种模式**完全同一套 UI**，差别只在"谁移动选中项"：
                · 自动播放：计时器按「焦点切换间隔」推进（顺序 = 兵种 → 建筑 → 英雄）
                · 手工查看：点击列表项自由查看
              列表**放开格数限制**（旧的 8 格 + 「+N」已删除）→ 全部项横向滚动看全。
              亮/暗 = 是否参与巡礼（未参与仍可点开查看）；金框 = 当前展示项。
              `overflow-hidden` 是兜底：尺寸再偏 1~2px 也不会压到下面的字幕行。 */}
          <div className="mt-[12px] flex-none">
            <div className="mb-[5px] flex items-center gap-[6px]">
              {FOCUS_KINDS.map((k) => (
                <button
                  key={k}
                  type="button"
                  onClick={() => switchTab(k)}
                  className={cn(
                    "rounded-md border px-[10px] py-[2px] text-[11px] transition-colors",
                    listTab === k
                      ? "border-[var(--gold,#F5C56B)] bg-[rgba(245,197,107,.14)] font-bold text-[var(--gold,#F5C56B)]"
                      : "border-slate-700/60 text-slate-300 hover:bg-slate-800/60",
                  )}
                >
                  {tabLabel(k)}{" "}
                  <span className="tabular-nums opacity-70">
                    {tabTours(k)}/{tabTotals(k)}
                  </span>
                </button>
              ))}
              {/* 计数含义 + **两种模式下列表内容不同**的说明：
                  自动播放只列"这一趟会巡礼的"，手工查看列全部（未参与暗显） */}
              <span className="ml-1 text-[10px] text-slate-500">
                {inspect ? t.realm.showcaseTabCountHint : t.realm.showcaseTabAutoHint}
              </span>
            </div>
            {/* 列表**定尺**（`LIST_H`）：三个分类的卡片同高 → 换标签 / 换模式时列表高度恒定，
                幕布因此不再跟着变（拍板："三个列表高度必须一样"）。 */}
            <div
              ref={listRefCb}
              // `pt-[4px]` 给"当前项上浮 2px + 金框 + 阴影"留空间（否则上边被裁，见 `LIST_H`）；
              // `px-[2px]` 让第一张/最后一张的描边不贴着裁切边缘（滚动出视口的仍会被裁，这是滚动容器的固有行为）
              className="sc-scroll flex items-start gap-2 overflow-x-auto px-[2px] pb-[3px] pt-[4px]"
              style={{ height: LIST_H }}
            >
              {listTab === "creatures" ? creatureCells : listTab === "buildings" ? buildingCells : heroCells}
            </div>
          </div>

          {/* sc-foot：字幕 + 控制（进度点 / 上一 / 暂停 / 下一 / 跳过） */}
          <div
            className="mt-2 flex flex-none items-end justify-between gap-4"
            style={{ animation: anim("scFadeIn .3s .26s both") }}
          >
            {/* 字幕列：**自带渐变托底**（`-ml-3 pl-3` 让文字左边缘仍与幕布对齐，
                视觉上只是往左边多铺了一层暗色，不会改变任何 x 坐标）。
                ① 对齐原版战役过场 —— 字幕坐在一片暗底上，与画面分离，视线不用"抠"字；
                ② 有托底之后文字阴影可以收紧（`0 2px 8px` → `0 1px 3px`）：
                   弥散阴影让小字号 CJK 笔画发虚，收紧后反而更清晰。 */}
            <div className="relative -ml-3 min-w-0 flex-1 pb-1 pl-3 pr-8">
              {/* 托底层：**绝对定位**且不吃布局（贴着字幕列铺满），所以下面的量算照旧 */}
              {/* ⚠️ `inset-0` 已包含父级的 `pr-8`（绝对定位相对 padding box），故不必再外扩，
                  否则会越过 `gap-4` 压到右列的 `border-l` 上 */}
              <div className="pointer-events-none absolute inset-0 rounded-r-xl bg-gradient-to-r from-black/65 via-black/40 to-transparent" />
              {/* 名称（**唯一主标签**，去掉幕布名牌后更值得单独一档）：20/26 bold */}
              <div
                className="relative truncate font-bold text-white"
                style={{
                  fontSize: CAP_NAME_SIZE * typeBoost,
                  lineHeight: `${CAP_NAME_LEAD * typeBoost}px`,
                  textShadow: "0 1px 3px rgba(0,0,0,.9)",
                }}
              >
                {captionText}
              </div>
              {/* 描述：**多行设计**（最多 3 行，超出在行尾渐隐）。
                  为什么是 3 行而不是继续往上：它是**过场正文**——幕布阶段观众只用余光扫，
                  截断成本远高于详情页；再多行就要跟幕布抢高度了，完整内容仍在巡礼区滚动显示。
                  `line-clamp-3` + 尾部渐隐 = 让读者知道"还有内容"。
                  ⚠️ 行宽随 `typeBoost` 同步放大，保证任何缩放档下**每行字数都是 ~42**。 */}
              <div
                className="relative mt-[2px]"
                style={{ maxWidth: CAP_DESC_W * typeBoost }}
              >
                <div
                  className="line-clamp-3 overflow-hidden text-slate-200"
                  style={{
                    fontSize: CAP_DESC_SIZE * typeBoost,
                    lineHeight: `${CAP_DESC_LEAD * typeBoost}px`,
                    textShadow: "0 1px 2px rgba(0,0,0,.85)",
                  }}
                >
                  {desc || <span className="opacity-55">{t.realm.showcaseNoDesc}</span>}
                </div>
                {desc.length > CAP_DESC_FADE_AT && (
                  <span
                    className="pointer-events-none absolute bottom-0 right-0 w-[54px] bg-gradient-to-l from-[#080d18] to-transparent"
                    style={{ height: CAP_DESC_LEAD * typeBoost }}
                  />
                )}
              </div>
              {/* 当前张的进度条：**放在字幕列底部**（与"这一张的进度"语义同源）。
                  ⚠️ 不放幕布：金色描边位置不固定、圆角会裁切、同色竞争（见幕布处注释）。
                  ⚠️ 字幕列是 `items-end` 底部对齐 → **有没有描述，进度条的 y 都相同**
                     （无描述时只是上方留白多一行），所以"某些城镇没描述"不会让位置漂移。
                  宽度 = `CAP_DESC_W`（与描述文本块对齐）：45 秒上限时也有 ~14px/秒，余光可见。
                  ⚠️ 必须带 `relative`：字面所在层是 absolute 的托底，静态元素会被它盖住。 */}
              <div
                className="relative mt-[2px] h-[2px] w-full overflow-hidden rounded-full bg-white/12"
                style={{ maxWidth: CAP_DESC_W * typeBoost }}
              >
                <div
                  className={cn(
                    "h-full rounded-full transition-[width] duration-200",
                    inspect ? "bg-slate-400/70" : "bg-[var(--gold,#F5C56B)]",
                  )}
                  style={{
                    width: `${
                      progress.total > 0
                        ? Math.min(100, (progress.elapsed / progress.total) * 100)
                        : 0
                    }%`,
                  }}
                />
              </div>
            </div>
            {/* 底栏**左右分栏**：左 = 城镇名 / 描述 / 进度条，右 = 播放控件（两行）。
                两个诉求都落在这里：
                ① 视觉上分成两块 —— 右列加一道 `border-l`，不再"文字顶到按钮上"；
                ② 右侧**多行排列** —— 行一 = 控制按钮，行二 = 进度点 + 计数/状态。
                两行总高 30 + 6 + 16 = 52 ≤ 左列 64 → **零纵向成本**。
                键盘提示不再常驻（按钮都有 title），腾出的横向空间给计数。 */}
            <div className="flex flex-none flex-col items-end gap-[6px] border-l border-slate-700/40 pl-4">
              <div className="flex items-center gap-[10px]">
                {inspect ? (
                  <>
                    {/* ── 手工查看模式 ── */}
                    <button
                      type="button"
                      // ⚠️ 必须**清掉「看一眼」**：手工查看时点过的项会一直占着巡礼区，
                      //    恢复播放后若不清理，看起来就是"没有回到原来的播放位置"（实测反馈）。
                      //    计时本身是接着走的（`accRef`），这里只是把画面交还给巡礼序列。
                      onClick={() => {
                        setPeek(null);
                        setPlaying(true);
                      }}
                      // 最后一张时把后果写在按钮上（"点了就播完结束"），避免以为会继续往后
                      title={isLast ? t.realm.showcaseResumeLast : t.realm.showcaseResume}
                      className="flex h-[30px] items-center gap-1 rounded-[9px] border border-emerald-500/50 bg-emerald-500/15 px-2.5 text-[12px] text-emerald-200 hover:bg-emerald-500/25"
                    >
                      <Play className="h-3.5 w-3.5" />
                      {t.realm.showcaseResume}
                    </button>
                    {/* ⚠️「重播」**只在自动播放出现**（拍板）：手工查看下时间本来是冻结的，
                        点它没有任何可观察结果 —— 那不是"按钮坏了"，而是这个动作在这里没有意义。 */}
                    {slides.length > 1 && (
                      <button
                        type="button"
                        onClick={() => go(idx - 1)}
                        title={t.realm.showcasePrev}
                        className="grid h-[30px] w-[34px] place-items-center rounded-[9px] border border-slate-700 bg-slate-900/70 text-slate-300 hover:bg-slate-800"
                      >
                        <ChevronLeft className="h-4 w-4" />
                      </button>
                    )}
                    {slides.length > 1 && (
                      <button
                        type="button"
                        onClick={() => go(idx + 1)}
                        title={t.realm.showcaseNext}
                        className="grid h-[30px] w-[34px] place-items-center rounded-[9px] border border-slate-700 bg-slate-900/70 text-slate-300 hover:bg-slate-800"
                      >
                        <ChevronRight className="h-4 w-4" />
                      </button>
                    )}
                  </>
                ) : (
                  <>
                    {/* ── 播放模式 ── */}
                    <button
                      type="button"
                      onClick={() => setPlaying(false)}
                      title={t.realm.showcaseInspectHint}
                      className="grid h-[30px] w-[34px] place-items-center rounded-[9px] border border-slate-700 bg-slate-900/70 text-slate-300 hover:bg-slate-800"
                    >
                      <Pause className="h-4 w-4" />
                    </button>
                    {/* 「重播」：**本张从头再播**（焦点归零 + 重新计时）—— 只有播放模式有意义 */}
                    <button
                      type="button"
                      onClick={() => setRestartTick((v) => v + 1)}
                      title={t.realm.showcaseReplayHint}
                      className="grid h-[30px] w-[34px] place-items-center rounded-[9px] border border-slate-700 bg-slate-900/70 text-slate-300 hover:bg-slate-800"
                    >
                      <RotateCcw className="h-4 w-4" />
                    </button>
                    {/* 「下一个阵营」：播放模式下 = 跳过本张；**最后一张时禁用**
                        （否则会绕回第 1 张 = 重新开始，那不是"跳过"的意思） */}
                    <button
                      type="button"
                      onClick={() => go(idx + 1)}
                      disabled={isLast}
                      title={t.realm.showcaseNext}
                      className="grid h-[30px] w-[34px] place-items-center rounded-[9px] border border-slate-700 bg-slate-900/70 text-slate-300 hover:bg-slate-800 disabled:cursor-not-allowed disabled:opacity-40"
                    >
                      <ChevronRight className="h-4 w-4" />
                    </button>
                  </>
                )}
                {/* 巡礼内静音开关（原来只是个纯图标，点了没反应） */}
                <button
                  type="button"
                  onClick={toggleMute}
                  title={muted ? t.realm.showcaseUnmute : t.realm.showcaseMute}
                  className={cn(
                    "grid h-[30px] w-[34px] place-items-center rounded-[9px] border transition-colors",
                    muted
                      ? "border-amber-500/50 bg-amber-500/15 text-amber-300 hover:bg-amber-500/25"
                      : "border-slate-700 bg-slate-900/70 text-slate-300 hover:bg-slate-800",
                  )}
                >
                  {muted ? <VolumeX className="h-4 w-4" /> : <Volume2 className="h-4 w-4" />}
                </button>
                {/* 语义（拍板）：这是**退出巡礼**（结束并返回领域主界面），与隔壁的
                    「下一个阵营」不是一回事 —— 从前写作「跳过」+ ⏭ 图标，两重歧义
                    （⏭ 在播放器语境里就是"跳到下一个"，且最后一站没有"剩余"可跳）。
                    现在：✕ + 「退出巡礼」+ tooltip 说明去向与 ESC 快捷方式。 */}
                <button
                  type="button"
                  onClick={onExit}
                  title={t.realm.showcaseSkipHint}
                  className="flex h-[30px] items-center gap-1 rounded-[9px] border border-slate-600 bg-slate-800/80 px-3 text-[12.5px] text-slate-100 hover:border-slate-400 hover:bg-slate-700 hover:text-white"
                >
                  <X className="h-3.5 w-3.5" />
                  {t.realm.showcaseSkip}
                </button>
              </div>

              {/* 行二：进度点 + 计数/状态（"我在第几个 / 还剩多久 / 当前模式"都在这一行） */}
              <div className="flex items-center gap-[10px]">
                {/* 进度点分档：1 个没有信息量 → 隐藏；**超过 12 个会挤爆**（自动编排最多 48 个）
                    → 也隐藏。此时"我在第几个"由计数与下方阵营切换条表达 */}
                {slides.length > 1 && slides.length <= DOTS_MAX && (
                  <div className="flex items-center gap-[6px]">
                    {slides.map((_, i) => (
                      <button
                        key={i}
                        type="button"
                        onClick={() => go(i)}
                        aria-label={`${i + 1}`}
                        className={cn(
                          "h-2 w-2 rounded-full transition-all",
                          i === idx
                            ? "scale-[1.25] bg-[var(--gold,#F5C56B)] shadow-[0_0_10px_rgba(245,197,107,.85)]"
                            : "bg-slate-300/30 hover:bg-slate-300/60",
                        )}
                      />
                    ))}
                  </div>
                )}
                <span className="whitespace-nowrap text-xs text-slate-400">
                  {slides.length > 1 &&
                    t.realm.showcaseFactionCount
                      .replace("{i}", String(idx + 1))
                      .replace("{n}", String(slides.length))}
                  {/* 剩余时间：**停留时长是自动算出来的**，不显示就只剩"怕被切走"的焦虑。
                      total 仍为 0（刚换张、尚未起跑）时留空，别显示"剩 0 秒"这种假数字 */}
                  {inspect
                    ? ` · ${t.realm.showcaseInspecting}`
                    : progress.total > 0
                      ? ` · ${t.realm.showcaseRemain.replace("{v}", String(remainSec))}`
                      : ""}
                </span>
              </div>
            </div>
          </div>

          {/* 阵营切换条（§5.11）：132×68，幕布缩略图做图标。
              ⚠️ 不能加 `justify-center`：flex + overflow-x-auto 时居中会把左侧内容顶出
              不可滚动区域（表现为"第一个选项显示不全、滚动条拉不到最左"）。
              用 `scrollChildIntoView` 自动把当前项滚进视野即可。 */}
          <div
            ref={switchRefCb}
            className="sc-scroll mt-[12px] flex flex-none gap-2 overflow-x-auto pb-1"
            style={{ animation: anim("scRise .3s .5s both") }}
          >
            {slides.map((s, i) => (
              <button
                key={`${s.modId}/${s.factionId}-${i}`}
                type="button"
                onClick={() => go(i)}
                className={cn(
                  "flex-none overflow-hidden rounded-[10px] border bg-slate-900/60 text-center transition-all",
                  i === idx
                    ? "border-[var(--gold,#F5C56B)] bg-[rgba(245,197,107,.14)]"
                    : "border-slate-700/60 hover:border-blue-500 hover:bg-blue-500/10",
                )}
                style={{ width: SW_W }}
              >
                {/* 缩略图按**原始比例**缩放（`contain`，不裁边）：幕布是 800×374（比例 2.14），
                    而卡片内框是 116×40（比例 2.9）—— 用 `cover` 会裁掉上下约 26% 的画面。
                    卡片底色 `bg-slate-900/60` 让留边读作"装裱"；若要满出血就得把框改成 116×54，
                    那会吃掉兵种区仅剩的 14px 纵向余量（见文件头预算表），故不采用。 */}
                <HdImg
                  src={fullImgCandidates(cd, s.backdrop, hdRate)}
                  alt=""
                  className={cn(
                    "block w-full object-contain",
                    i === idx ? "opacity-100" : "opacity-75",
                  )}
                  style={{ height: SW_IMG_H }}
                  draggable={false}
                />
                <div
                  className={cn(
                    "truncate px-[6px] py-[2px] text-[11px] leading-[14px]",
                    i === idx ? "font-bold text-[var(--gold,#F5C56B)]" : "text-slate-300",
                  )}
                >
                  {pickName(s.name, lang) || s.factionId}
                </div>
              </button>
            ))}
          </div>
        </div>

        {/* 左右热区（§5.6：宽 7%，hover 才显形）——**播放模式专属**。
            ⚠️ 三处偏离原型的 `top-0 / bottom:150`（prototype:182 `.hot`），都是为了"不抢点击"：
            ① 纵向只覆盖 **幕布那一行**（`top:12 / height:337`，正好是幕布的 y 区间）：
               原型的 `bottom:150` 会一路盖到建筑条，而我们的兵种/建筑卡是**可点击**的
               （原型里不可点击，所以原型没暴露这个冲突）—— 实测"手工查看下点最左侧卡片
               会切到上一个阵营"就是它；
            ② `z-[2]`：低于内容层（z-3），即使几何漂移也是卡片赢；
            ③ **手工查看模式下不渲染**：右栏右边缘到 x=1254，而右热区带从 x=1190.4 起
               → 会压住右栏 64px；那时右栏是可交互的详情卡，热区只会捣乱。
            几何核对：热区带 x∈[0,89.6]；幕布左边缘 = 26 + (950−720)/2 = **141** →
            左侧带落在幕布左侧的空白边距里，**不与任何卡片重叠**。 */}
        {!inspect && (
          <>
            <div
              onClick={() => go(idx - 1)}
              className="absolute left-0 z-[2] grid w-[7%] place-items-center text-[26px] text-transparent transition-colors hover:bg-gradient-to-r hover:from-blue-500/15 hover:text-slate-200"
              style={{ top: 12, height: frameH }}
            >
              ‹
            </div>
            <div
              onClick={() => go(idx + 1)}
              className="absolute right-0 z-[2] grid w-[7%] place-items-center text-[26px] text-transparent transition-colors hover:bg-gradient-to-l hover:from-blue-500/15 hover:text-slate-200"
              style={{ top: 12, height: frameH }}
            >
              ›
            </div>
          </>
        )}

        {/* ⚠️ 这里**不再有**浮动提示条：
            ① 旧的「首次提示条」（`bottom-186px`，原型 `.hint`）会压住列表行；
            ② 「手工查看中」徽标会压住字幕/描述（实测反馈）。
            键盘提示改放进控制条里的一行小字（永久可见、零遮挡）。 */}

        {/* 素材就绪前不计时 → 要让用户知道"在等什么"，否则观感就是"卡住了没反应" */}
        {status === "ready" && !assetsReady && (
          <div className="pointer-events-none absolute bottom-[112px] left-1/2 z-[7] -translate-x-1/2 rounded-[14px] border border-slate-700 bg-slate-900/75 px-[12px] py-[4px] text-[11px] text-slate-300 backdrop-blur-sm">
            {t.realm.showcaseLoadingAssets}
          </div>
        )}
      </div>

      {/* 阵营音乐 + 领域 BGM 回退（§7.8）。
          ⚠️ `onLoadedMetadata` 里消费「起始位置」：`src` 刚变更时设 `currentTime` 会被忽略
             （readyState = HAVE_NOTHING）→ 换源后起始位置就丢了，所以定位必须等元数据。
          ⚠️ `onError` 不能只记日志：坏音源要退回领域 BGM，否则整场静音。 */}
      <audio
        ref={factionAudioRef}
        className="hidden"
        preload="auto"
        onLoadedMetadata={(e) => {
          const a = e.currentTarget;
          const off = pendingOffsetMsRef.current;
          pendingOffsetMsRef.current = 0;
          if (off > 0) {
            try {
              a.currentTime = off / 1000;
            } catch {
              /* 定位失败静默降级：从 0 播 */
            }
          }
        }}
        onError={(e) => {
          const a = e.currentTarget;
          console.warn("[showcase] 阵营音乐加载失败（asset 协议 / 文件路径）", {
            src: a.getAttribute("src"),
            code: a.error?.code,
          });
          // ⚠️ **不再**回退领域 BGM（拍板）：那会让"某个城镇音源坏了"变成
          //    "突然响起领域主界面的主题曲"，与"解析不到专属音乐 → 该站安静"的口径矛盾。
          faSrcRef.current = "";
        }}
      />
      <audio
        ref={bgmAudioRef}
        className="hidden"
        onError={(e) => {
          const a = e.currentTarget;
          console.warn("[showcase] 领域 BGM 加载失败", {
            src: a.getAttribute("src"),
            code: a.error?.code,
          });
        }}
      />
    </div>
  );
}
