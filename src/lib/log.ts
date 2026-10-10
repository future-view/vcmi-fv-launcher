import { logFrontend } from "./tauri";

/**
 * 前端日志桥接：把前端错误 / 关键事件汇入**与后端同一份**日志文件
 * （`<exe>/.vcmi-fv-launcher/logs`），便于统一排查。
 *
 * 设计约束：
 * - **绝不影响业务**：上报失败静默吞掉，不抛出、不阻塞。
 * - **防日志风暴**：同一 `level|target|message` 在窗口期内只上报一条。
 * - **长度截断**：后端也会再兜底截断一次。
 */

export type FrontendLogLevel = "error" | "warn" | "info" | "debug";

/** 相同日志的去重窗口（毫秒） */
const THROTTLE_MS = 2000;
/** 单条消息最大长度（防止超长堆栈撑爆日志文件） */
const MAX_MSG_LEN = 2000;
/** 去重表容量上限，超出即清空，避免无界增长 */
const MAX_KEYS = 500;

const lastAt = new Map<string, number>();

/** 统一前端日志入口。 */
export function log(level: FrontendLogLevel, target: string, message: string): void {
  const key = `${level}|${target}|${message.slice(0, 120)}`;
  const now = Date.now();
  if (now - (lastAt.get(key) ?? 0) < THROTTLE_MS) return;
  if (lastAt.size > MAX_KEYS) lastAt.clear();
  lastAt.set(key, now);
  logFrontend(level, target, message.slice(0, MAX_MSG_LEN));
}

export const logError = (target: string, message: string) => log("error", target, message);
export const logWarn = (target: string, message: string) => log("warn", target, message);
export const logInfo = (target: string, message: string) => log("info", target, message);
export const logDebug = (target: string, message: string) => log("debug", target, message);

/** 把任意 `unknown` 错误格式化为可读字符串（含 name/message）。 */
export function fmtError(e: unknown): string {
  if (e instanceof Error) return `${e.name}: ${e.message}`;
  return String(e);
}

let installed = false;

/**
 * 安装全局错误钩子：`window.onerror` / `unhandledrejection` → 日志文件。
 * 幂等，只安装一次。
 */
export function installGlobalLogging(): void {
  if (installed || typeof window === "undefined") return;
  installed = true;
  window.addEventListener("error", (e) => {
    logError("window", `${e.message} @ ${e.filename}:${e.lineno}:${e.colno}`);
  });
  window.addEventListener("unhandledrejection", (e) => {
    logError("promise", fmtError(e.reason));
  });
}
