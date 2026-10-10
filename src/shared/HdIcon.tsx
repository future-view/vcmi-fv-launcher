/**
 * 「单个 URL 的高清图标」——`HdImg` 的便捷封装：内部订阅倍率并生成候选链。
 *
 * ## 为什么需要它
 *
 * `HdImg` 刻意只接收**候选数组**（`imgCandidates(...)` 的结果），调用方需自己拿 `useHdRate()`。
 * 这在组件里没问题，但**纯函数**（如 `detailShared::formatCost` 返回 `ReactNode`）里不能用 hook。
 * 而这类地方恰恰很多：花费图标、资源图标、属性图标、列表行图标……
 *
 * `HdIcon` 把"订阅倍率 + 生成候选 + 像素画判定"收进组件内部，于是纯函数可以照常返回
 * `<HdIcon src={url} .../>` —— React 渲染该元素时才会执行它的 hook，规则完全合法。
 *
 * ## 与 `SafeImg` 的分工
 *
 * - `SafeImg(cacheDir, rel)`：接收**缓存相对路径**，自己拼 URL（`fullImgCandidates`）。详情页用。
 * - `HdIcon(url)`：接收**已拼好的 URL**（如 `fullImgUrl` 的产物、`resourceVisual` 的 icon）。
 *   凡是"URL 已经在手"的展示点都用它，避免重复传 cacheDir。
 *
 * ⚠️ 高清变体只对 `.webp` 生效（`hdVariantUrl`），作者自备图（png/jpg）或领域包素材传进来
 * 会原样使用 —— 这是刻意的：那些素材不受缓存管，也没有 `@Nx` 变体。
 *
 * 为什么放 `shared/`：被 `features/wiki/**` 与 `features/realms/**` 共用，本身无业务语义。
 */
import { HdImg } from "./HdImg";
import { useHdRate } from "./useHdRate";
import { imgCandidates } from "@/lib/hd";

interface Props {
  /** 已拼好的图片 URL（空值 → 渲染占位） */
  src?: string | null;
  alt?: string;
  className?: string;
  /** ⚠️ 不要写 `imageRendering`（由像素画动态判定接管） */
  style?: React.CSSProperties;
  /** 占位类（缺省 = 与 `className` 同尺寸的空 div，避免布局塌陷） */
  placeholderClassName?: string;
  draggable?: boolean;
  /** 是否像素画素材（图标类默认 `true`；写实大图传 `false`） */
  pixelArt?: boolean;
  /** 原生 loading（长列表的图标用 `"lazy"` 省解码） */
  loading?: "lazy" | "eager";
}

export function HdIcon({
  src,
  alt,
  className,
  style,
  placeholderClassName,
  draggable,
  pixelArt = true,
  loading,
}: Props) {
  const rate = useHdRate();
  return (
    <HdImg
      src={imgCandidates(src ?? "", rate)}
      alt={alt}
      className={className}
      style={style}
      placeholderClassName={placeholderClassName}
      draggable={draggable}
      pixelArt={pixelArt}
      loading={loading}
    />
  );
}
