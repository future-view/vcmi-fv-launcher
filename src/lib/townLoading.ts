// 跟踪城镇图片（背景/兵种图标）异步提取的进行中数量。
// 当 Mod 图片仍在加载时，点"启动游戏"应先弹窗提示用户等待或继续。

let count = 0;
const listeners = new Set<(n: number) => void>();

export function townLoadingInc() {
  count += 1;
  emit();
}

export function townLoadingDec() {
  count = Math.max(0, count - 1);
  emit();
}

export function getTownLoading(): number {
  return count;
}

export function subscribeTownLoading(cb: (n: number) => void): () => void {
  listeners.add(cb);
  return () => listeners.delete(cb);
}

function emit() {
  for (const cb of listeners) cb(count);
}
