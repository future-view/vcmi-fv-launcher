import { readLauncherConfig, saveTownPreview } from "./tauri";

/**
 * 「预览城镇背景」开关的**进程内缓存**（模组列表 hover 是否弹出城镇大图）。
 *
 * 权威值在 `launcher.json` 的 `ui.town_preview`（默认 false）。本模块负责：
 * - 启动时读一次磁盘（`loadTownPreview`），避免每个 `ModCard` 各自读一轮配置；
 * - 切换时**先更新内存再落盘**（`setTownPreviewEnabled`），保证 UI 立即响应；
 * - 供组件用 `useSyncExternalStore` 订阅（布尔快照，天然不可变，不涉及引用比较陷阱）。
 *
 * 放 `lib/` 而非组件内部：`ModBrowser`（开关 UI）与 `ModCard`（hover 行为）都要读，
 * 且 Mods 页与领域「领域模组」页共用同一组件，需要同一份状态。
 */
let enabled = false;
const listeners = new Set<() => void>();

export function getTownPreviewEnabled(): boolean {
  return enabled;
}

export function subscribeTownPreview(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

function emit(): void {
  listeners.forEach((l) => l());
}

/** 从 launcher.json 载入开关（重复调用安全；读取失败保持默认关闭） */
export async function loadTownPreview(): Promise<void> {
  try {
    const cfg = await readLauncherConfig();
    const next = cfg?.townPreview === true;
    if (next !== enabled) {
      enabled = next;
      emit();
    }
  } catch {
    // 读取失败：保持默认关闭，不阻塞界面
  }
}

/** 切换开关：先更新内存（UI 立即生效），再异步落盘 */
export async function setTownPreviewEnabled(next: boolean): Promise<void> {
  if (next !== enabled) {
    enabled = next;
    emit();
  }
  await saveTownPreview(next);
}
