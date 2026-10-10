/**
 * 引擎版本（VCMI 版本号）的**进程内单次加载 + 订阅**。
 *
 * 为什么要有这一层：兼容性判定（{@link isModCompatible}）几乎每个模组卡片都要用，
 * 而版本号来自一次 IPC（`get_compile_info`：先读 `VCMI_client.exe` 的 PE 文件版本，
 * 失败再回退内置 mod 的 `version`）。若每个组件各查一次，几百张卡片就是几百次 IPC。
 *
 * 于是做成**模块级缓存 + `useSyncExternalStore` 订阅**：
 * - 首次 `loadEngineVersion()` 只有一个 in-flight 请求（多处调用共享同一 promise）；
 * - 加载完成后通知所有订阅者重渲染，卡片角标"补齐"而不用外部传 prop。
 */

import { getCompileInfo, isTauri } from "./tauri";

/** `undefined` = 尚未加载；`null` = 已加载但拿不到版本（非 Tauri / 读失败） */
let value: string | null | undefined;
let inflight: Promise<void> | null = null;
const listeners = new Set<() => void>();

function emit(): void {
  listeners.forEach((l) => l());
}

/**
 * 同步快照：`undefined` 表示"还不知道"，调用方此时**不要**做兼容性判定
 * （宁可放过也不要误报成"不兼容"）。
 */
export function getEngineVersionSnapshot(): string | null | undefined {
  return value;
}

/** 订阅加载完成事件（配合 `useSyncExternalStore`），返回取消订阅函数。 */
export function subscribeEngineVersion(cb: () => void): () => void {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}

/** 幂等加载（模块级去重）。非 Tauri 环境直接保持 `undefined`（浏览器自测不做判定）。 */
export async function loadEngineVersion(): Promise<void> {
  if (value !== undefined) return;
  if (inflight) return inflight;
  if (!isTauri()) return;
  inflight = (async () => {
    try {
      const info = await getCompileInfo();
      value = info?.vcmiVersion?.trim() || null;
    } catch {
      value = null;
    } finally {
      inflight = null;
      emit();
    }
  })();
  return inflight;
}
