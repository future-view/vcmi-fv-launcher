/**
 * 订阅「高清素材倍率」的 React 钩子。
 *
 * 倍率由**显示器物理宽**推导（见 `lib/hd.ts::computeHdRate`），启动后异步探测一次。
 * 组件必须用本钩子（而不是在渲染里同步读 `peekHdRate()`）：
 * 后者在首帧拿到 1x 后**不会**在探测完成时更新，高清素材就永远切不进来。
 *
 * 放 `shared/` 的理由同 `HdImg`：被 `features/wiki/**` 与 `features/realms/**` 共用，无业务语义。
 */
import { useEffect, useSyncExternalStore } from "react";
import { getHdRate, peekHdRate, subscribeHdRate } from "@/lib/hd";

export function useHdRate(): number {
  const rate = useSyncExternalStore(
    subscribeHdRate,
    // getSnapshot 必须返回稳定值（number 满足）；未探测完成时按 1 处理
    () => peekHdRate() ?? 1,
    // 无 SSR 场景，仅为 API 完整性
    () => 1,
  );
  // 首次挂载触发探测（幂等：已探测过直接返回缓存值）
  useEffect(() => {
    void getHdRate();
  }, []);
  return rate;
}
