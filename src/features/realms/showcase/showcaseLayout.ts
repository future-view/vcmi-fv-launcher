import { useCallback, useRef } from "react";
import { imgCandidates } from "@/lib/hd";
import { fullImgUrl } from "@/lib/modcache";
import type { ShowcaseSlide } from "@/lib/tauri";
import { backendErrorText } from "@/lib/backendError";
import { DESIGN_CANVAS_H, DESIGN_CANVAS_W } from "@/lib/tauri";

/**
 * 「阵营巡礼」的**定尺布局规格与播放期小工具**。
 *
 * 从 `ShowcaseOverlay.tsx` 抽出：那一块是纯常量 / 纯函数（无 JSX、无组件状态），
 * 与演出组件本身的逻辑无关，独立成模块后组件只关心「怎么演」。
 *
 * 本模块所有像素值都是**设计稿像素**（1280×720 画布内部单位），
 * 整个演出界面按 `scale` 等比缩放，故不需要为 2K/4K 单独写值。
 */

/** 设计稿尺寸（§5.9.1）：整体等比缩放，长宽比变化留背景色不拉伸。
 * 与全局 UI 缩放共用同一坐标系（`DESIGN_CANVAS_W/H`）——窗口 16:9 时逻辑视口恒为
 * 1280×720，此处 `scale` 恰为 1.000（零黑边）；非 16:9 窗口才由它 letterbox 兜底。 */
export const DESIGN_W = DESIGN_CANVAS_W;
export const DESIGN_H = DESIGN_CANVAS_H;
/**
 * 幕布 frame：**高度撑满 sc-top 行的剩余空间**（宽度按 800/374 比例推出）。
 *
 * 为什么不再写死像素：底栏（标签栏 / 列表 / 字幕 / 切换条）高度一变，固定高的幕布就会
 * 与它们之间**空出一大块**（"sc-mid 与 sc-foot 之间空隙太大"就是这个）。
 * 改成从行高**实测**之后再算，空隙自动被幕布吃掉 —— 窗口 / 各处间距变化都不用再调数字。
 *
 * `FRAME_H_FALLBACK` 只用于首帧（尚未量到行高时），随后由 ResizeObserver 接管。
 */
export const FRAME_H_FALLBACK = 400;
/** 幕布原生宽高比（= 856/400 也等于它，故放大不改变比例） */
export const FRAME_RATIO = 800 / 374;
/** 右上巡礼区宽度 + 它与幕布之间的 gap（sc-top 行的固定占用，用于推算幕布宽度上限） */
export const ZONE_W = 300;
export const TOP_GAP = 16;
/** 幕布原生像素（border 坐标基准） */
export const BACKDROP_W = 800;
export const BACKDROP_H = 374;

/**
 * **底栏字幕的排版定量**（2026-09-22 调大，对标原版战役过场字幕）。
 *
 * ⚠️ 这里全部是**设计稿像素**（1280×720 画布内部单位）：整个演出界面是这块画布整体等比缩放的
 * （见 `DESIGN_W`），所以 1080P → 2K → 4K 往上走，字号是**跟着一起放大**的 —— 不需要给 2K/4K
 * 单独写值，改这里的基准值就等于改所有分辨率。
 *
 * 唯一会被拉小的情况是"窗口比设计稿还小"（`scale < 1`），由 `typeBoost` 反向补偿，上限 1.15
 * —— 再大会把幕布压到 `FRAME_H_FALLBACK` 的舒适区以下。
 *
 * 取值依据：原版过场字幕字高约占屏高 3.3~3.7%；本字幕属**次要信息**（焦点在幕布），
 * 故描述取 15/698 ≈ 2.1%，名称 20px bold 拉开一级层级。
 */
export const CAP_NAME_SIZE = 20;
export const CAP_NAME_LEAD = 26;
export const CAP_DESC_SIZE = 15;
export const CAP_DESC_LEAD = 21;
/** 描述最多几行（超出才尾部渐隐，完整内容在右上方巡礼区滚动显示） */
export const CAP_DESC_LINES = 3;
/** 描述行宽：**每行 ≈42 个汉字**（中英混排时略宽），这是 CJK 舒适阅读区间的上沿 */
export const CAP_DESC_W = 640;
/** 每行字数 × 行数 ≈ 渐隐阈值；`line-clamp` 之后还有内容才值得画那道遮罩 */
export const CAP_DESC_FADE_AT = 110;
/**
 * 巡礼序列的项类型（**统一模型**，2026-09-21 拍板）：
 * 兵种 → 建筑 → 英雄 三类拼接成一条序列；两种模式共用同一条序列与同一套 UI，
 * 差别只在"谁移动选中项"（定时器 / 用户点击）。
 * ⚠️ 列表**不再有 8 格上限**（旧 `MAX_SLOTS` + 「+N」已删除）：全部项横向滚动展示。
 */
export const FOCUS_KINDS = ["creatures", "buildings", "heroes"] as const;
export type FocusKind = (typeof FOCUS_KINDS)[number];
/**
 * 焦点项用 **id** 而非下标：
 * 列表展示"全部项"，巡礼序列只含"参与项" —— 两者下标不同。
 * 用 id 才能让"点未参与项看一眼"与"点参与项"走同一条路径（`peek`）。
 */
export type FocusItem = { kind: FocusKind; id: string };
/** 素材预取/就绪判定的张数上限：比实际渲染的略多，宁可多取几张也不让画面开天窗 */
export const PREFETCH_CREATURES = 12;
export const PREFETCH_BUILDINGS = 16;
export const PREFETCH_HEROES = 4;
/**
 * 进度点的数量上限：超了就**不显示圆点**。
 * 自动编排最多 48 个阵营，48 个 8px 点 + 间距 ≈ 670px，会把控制条挤爆；
 * 而这个信息"我在第几个"由 `5 / 38 阵营` 与下方阵营切换条承载（功能本就重复）。
 */
export const DOTS_MAX = 12;

/**
 * 详情卡的图框与三类图的约束方式。
 *
 * ⚠️ 三类图**原始比例完全不同**（兵种立绘竖长、建筑图标横宽 ≈2.2:1、英雄头像近方），
 * 所以统一规则是：**`object-contain` + 只约束一个维度，另一维度自适应** ——
 * 两个维度都定死会把图拉伸变形（这是"图片等比"的既有约定）。
 * 图框尺寸恒定（262×120）→ 不同比例的图都居中出现，卡片不会被撑变形。
 */
export const ZONE_IMG_BOX_H = 150;
export const DETAIL_IMG_H = 148; // 兵种/英雄：定**高**
export const DETAIL_IMG_W = 250; // 建筑：定**宽**

/**
 * 描述取值 + **语言回退链**：当前语言 → english → 任意一种。
 *
 * 描述在多语言下经常只有部分语言有（模组作者只写了一种，或 h3-core 只有中英），
 * 直接用 `d[lang]` 会"明明是中文界面却什么都没有"。
 */
export function pickDesc(d: Record<string, string> | undefined, lang: string): string {
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
export function slideImageUrls(s: ShowcaseSlide, cd: string, rate = 1): string[] {
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
export const SLOT_W = 86;
export const SLOT_IMG_W = 56;
/** 三个分类的**图位高度统一**（建筑不再矮一截）—— 列表等高，幕布就不跳 */
export const SLOT_IMG_H = 62;
/**
 * 建筑卡：建筑原图 **150×70**（横图），故卡更宽；但**图位高度与兵种/英雄完全一致**，
 * 图片在框内 `object-contain` **居中** → 比例不破、三项列表高度相同（拍板）。
 */
export const BLD_SLOT_W = 108;
export const BLD_IMG_W = 108;
export const BLD_IMG_H = SLOT_IMG_H;
/** 列表卡片总高（pt 5 + 图位 62 + 间距 2 + 名称 14 + pb 4）—— 三个分类共用同一高度 */
export const CELL_H = 5 + SLOT_IMG_H + 2 + 14 + 4;
/**
 * 列表容器固定高度：`pt-[4px]` + 卡片 + `pb-[3px]` + 横向滚动条 6 —— 定尺后换分类/换模式都不跳。
 *
 * ⚠️ 那 4px 上边距**不是装饰**：当前项会 `-translate-y-[2px]` 上浮，而容器是 `overflow-x-auto`
 * ——CSS 规定 `overflow-x` 非 `visible` 时 `overflow-y` 会被算成 `auto`，于是**纵向也裁**：
 * 少这 4px，当前项的金框上边与阴影就会被切掉（实测"部分金色边框无法显示"）。
 */
export const LIST_H = CELL_H + 15;
/**
 * 阵营切换条（2026-09-21 放大）：卡片 **140×72**、缩略图 54 高、名称 12px **居中**。
 * 旧值 116×58 / 40 高 / 名称 11px **左对齐** —— 缩略图是 `object-contain` 居中的，
 * 名称左对齐就会"名称与图片不对齐"（实测反馈），故名称改居中并整体放大一档。
 */
export const SW_W = 140;
export const SW_IMG_H = 54;

/** 局部动画（§9 转场 / 入场阶梯 / 幕布高亮脉冲），避免污染全局样式 */
export const KEYFRAMES = `
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
export const TRANSITION_ANIM = {
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
export function setVol(a: HTMLAudioElement | null, v: number): void {
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
export function useWheelScrollX() {
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
export function prog(now: number, t0: number, dur: number): number {
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
export function safePlay(a: HTMLAudioElement, retry = 2) {
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
