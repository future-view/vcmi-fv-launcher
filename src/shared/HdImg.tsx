/**
 * 高清优先、自动回退的图片，并按需选择重采样方式。
 *
 * 候选列表由 `lib/hd.ts::imgCandidates` 产出：**高清在前、原图在后**。
 * 加载失败即换下一个（并记住该高清变体缺失，避免反复 404）；全部失败才显示占位块。
 *
 * ## `pixelArt`：为什么必须"动态"决定重采样方式
 *
 * 像素画素材（兵种/英雄/建筑）一旦被**放大**显示，浏览器默认的双线性插值就会把
 * 硬边抹成渐变 → 观感"糊"。反过来，当素材**充足**（1:1 或缩小）时，最近邻缩小会
 * 周期丢行/列产生摩尔纹，此时平滑插值才是正确选择。
 *
 * 因此本组件在图片加载后读 `naturalWidth`，与**物理显示宽度**（元素 CSS 宽 × DPR）比较：
 *
 * - 素材像素 **< 物理需求** → 正在被放大 → `image-rendering: pixelated`（保锐，绝不糊）；
 * - 素材像素 **≥ 物理需求** → `auto`（平滑，缩小质量最好）。
 *
 * 这比"按是否命中高清候选"猜测可靠得多：素材倍率由显示器推导，但**窗口可随时改变**，
 * 同一张图在不同窗口尺寸下可能一个放大、一个缩小。
 *
 * ## 默认值取"最近邻"
 *
 * 首帧尚未测量，先按**锐利**渲染（若该素材确实充足，加载后会自动切回平滑）。
 * 宁可"先锐后平滑"，也不要"先糊后锐"——后者正是用户投诉的观感。
 *
 * 为什么放 `shared/`：被 `features/wiki/**`（百科详情）与 `features/realms/**`（阵营巡礼）
 * 共用，且本身**无业务语义**（纯 props 驱动）。
 *
 * ⚠️ 「原素材始终保留 + 读不到高清就回退」是既定约定（见 `src/lib/hd.ts` 头注），
 * 本组件是该约定的唯一执行点，不要改成"只认高清"。
 */
import { useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import { markHdMissing, peekHdEpoch, subscribeHdEpoch } from "@/lib/hd";

interface Props {
  /** 候选 URL：**高清在前、原图在后**。空数组 = 直接渲染占位 */
  src: string[];
  alt?: string;
  className?: string;
  /** ⚠️ 不要在此写 `imageRendering`（由 `pixelArt` 统一管理） */
  style?: React.CSSProperties;
  /** 占位类（缺省 = 与 `className` 同尺寸的灰块） */
  placeholderClassName?: string;
  draggable?: boolean;
  /**
   * 是否**像素画素材**（兵种/英雄/建筑/图标 → `true`；写实幕布、作者自备图 → 留空）。
   *
   * 置 `true` 即启用上面的动态判定；置空则始终使用平滑插值。
   */
  pixelArt?: boolean;
  /** 原生 loading（长列表的图标用 `"lazy"` 省解码） */
  loading?: "lazy" | "eager";
}

export function HdImg({
  src,
  alt,
  className,
  style,
  placeholderClassName,
  draggable,
  pixelArt,
  loading,
}: Props) {
  // 用拼接串做依赖：数组字面量每次渲染都是新引用，直接依赖会无限重置
  const key = src.join("|");
  const [idx, setIdx] = useState(0);
  // 默认按锐利渲染（见头注：宁可先锐后平滑）
  const [needsNearest, setNeedsNearest] = useState(pixelArt === true);
  const imgRef = useRef<HTMLImageElement | null>(null);

  const measure = useCallback(() => {
    const el = imgRef.current;
    if (!el || !el.naturalWidth) return;
    const dpr = window.devicePixelRatio || 1;
    // 元素未完成布局时 clientWidth 为 0 → 退回自然宽（视为 1:1，不需要最近邻）
    const physW = (el.clientWidth || el.naturalWidth) * dpr;
    setNeedsNearest(el.naturalWidth < physW * 0.98);
  }, []);

  // 高清可用性版本号：阶段三生成完高清后会自增（`clearHdMissing`），
  // 借此让**已渲染**的图片回到候选链首位重试 —— 否则它们的 src 仍停在原图。
  const hdEpoch = useSyncExternalStore(subscribeHdEpoch, peekHdEpoch, () => 0);

  // 候选变化（换阵营 / 换条目 / 倍率变化 / 高清刚就绪）时回到第一候选
  useEffect(() => {
    setIdx(0);
  }, [key, hdEpoch]);

  // 窗口尺寸 / 系统缩放变化会改变全部图的物理尺寸 → 重新判定
  useEffect(() => {
    if (!pixelArt) return;
    window.addEventListener("resize", measure);
    return () => window.removeEventListener("resize", measure);
  }, [pixelArt, measure]);

  const url = src[idx];
  if (!url) {
    return <div className={placeholderClassName ?? className} />;
  }

  const finalStyle =
    pixelArt && needsNearest ? { ...style, imageRendering: "pixelated" as const } : style;

  return (
    <img
      ref={imgRef}
      src={url}
      alt={alt ?? ""}
      className={className}
      style={finalStyle}
      draggable={draggable}
      loading={loading}
      onLoad={pixelArt ? measure : undefined}
      onError={() => {
        // 高清缺失 → 记住**这一档**（同一路径不再重试），前进到下一个候选；
        // 原图也失败 → idx 越界 → 占位块（不再有 broken 图标）
        //
        // ⚠️ 条件是 `idx < src.length - 1` 而非 `idx === 0`：后者只记首档失败，
        // 于是 `@3x` / `@2x`（第二、三次失败）永远不入册 —— 组件每次重新挂载
        // 都会把它们再请求一遍（实测同一 URL 反复 404，单次会话最多 4 次），
        // 而每次 404 都会在应用日志里留一行 ERROR。
        // 末项（原图）不入册：`imgCandidates` 原本就无条件保留原图兜底。
        if (idx < src.length - 1) markHdMissing(url);
        setIdx((v) => v + 1);
      }}
    />
  );
}
