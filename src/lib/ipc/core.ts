/**
 * IPC 基础设施：运行环境判定（`isTauri`）、mock 缓存前缀，以及启动类命令的统一结果封装
 * （`LaunchResult` / `wrapLaunch`）。
 * 几乎所有域都依赖它，故单独成文件，避免任何循环依赖。
 *
 * 拆分自原 `@/lib/tauri.ts` 单体；对外仍由该 barrel 统一导出，调用方无需改动。
 */
import { backendErrorText } from "@/lib/backendError";

export function isTauri(): boolean {
  return typeof window !== "undefined" && (window as any).__TAURI_INTERNALS__ != null;
}

/** 浏览器自测模式（非 Tauri）下，mock 缓存数据/图片的 URL 前缀（由 vite mock-cache 中间件服务） */
export function mockCacheBase(): string {
  return "/mock-cache/mods";
}

/** 启动类命令（启动游戏 / 编辑器 / 打开目录 / 激活领域…）的统一返回形状 */
export interface LaunchResult {
  ok: boolean;
  error?: string;
}

/** 把「抛异常」的命令收敛成 `LaunchResult`（调用方无需 try/catch） */
export async function wrapLaunch(fn: () => Promise<void>): Promise<LaunchResult> {
  try {
    await fn();
    return { ok: true };
  } catch (e: any) {
    return { ok: false, error: backendErrorText(e) };
  }
}
