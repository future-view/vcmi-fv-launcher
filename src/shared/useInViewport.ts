import { useEffect, useRef, useState, type RefObject } from "react";

/**
 * 元素是否**曾经进入视口**（一次性：一旦可见就保持 `true`，不再随滚动来回翻转）。
 *
 * ## 用途
 *
 * 长列表里"昂贵的逐行副作用"（如每行读一次模组缓存图片）只对**可见行**执行。
 * 否则数百行会在挂载瞬间同时发起数百次 IPC —— 首次启动时缓存尚未构建，
 * 这些请求全部失败并进入重试，是页面卡顿的主要来源之一
 *（见 `docs/features/startup-responsiveness.zh-CN.md` R3）。
 *
 * ## 行为
 *
 * - `rootMargin` 预取（默认 `300px`，滚动到附近即开始加载，用户察觉不到延迟）；
 * - 不支持 `IntersectionObserver` 的环境（老 WebView / 测试）**直接视为可见**
 *   —— 宁可多跑，不可不跑（容错铁律：不能因为优化让功能失效）。
 *
 * 用法：把返回的 `ref` 绑到行 / 卡片的根节点，用 `inView` 控制副作用开关。
 */
export function useInViewport<T extends Element>(
  rootMargin = "300px",
): { ref: RefObject<T>; inView: boolean } {
  const ref = useRef<T>(null);
  const [inView, setInView] = useState(false);
  useEffect(() => {
    if (inView) return;
    const el = ref.current;
    if (!el) return;
    if (typeof IntersectionObserver === "undefined") {
      setInView(true);
      return;
    }
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          setInView(true);
          io.disconnect();
        }
      },
      { rootMargin },
    );
    io.observe(el);
    return () => io.disconnect();
  }, [inView, rootMargin]);
  return { ref, inView };
}
