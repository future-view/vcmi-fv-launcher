import { zhCN } from "@/i18n/zh-CN";
import type { LangDict } from "@/i18n/types";

/** 把 `{name}` 占位符替换进模板（缺省值回退原占位符，避免显示 `{name}`） */
function fill(tpl: string, vars: Record<string, string>): string {
  return tpl.replace(/\{(\w+)\}/g, (whole, key: string) =>
    key in vars ? vars[key] : whole,
  );
}

/**
 * 任意抛出值 → 原始错误串：`Error` 取 `message`，其余 `String()`。
 *
 * Tauri 把后端 `Err(String)` 包装成 `Error`（`.message` 即原始串），
 * 因此**不要**再对错误值直接 `String(e)`（会带 `"Error: "` 前缀导致 JSON 解析失败）。
 */
export function errorRaw(raw: unknown): string {
  if (raw instanceof Error) return raw.message;
  if (raw == null) return "";
  return String(raw);
}

/** 解析结构化错误 `{"code","arg"}`；非结构化返回 `null`（兼容 `String(Error)` 的 `Error: ` 前缀） */
function parseStructured(raw: unknown): { code: string; arg: string } | null {
  const tryOne = (x: string): { code: string; arg: string } | null => {
    try {
      const v = JSON.parse(x) as { code?: unknown; arg?: unknown };
      if (v && typeof v.code === "string") {
        return { code: v.code, arg: typeof v.arg === "string" ? v.arg : "" };
      }
    } catch {
      // 旧式串或非 JSON
    }
    return null;
  };
  const s = errorRaw(raw);
  return tryOne(s) ?? (s.startsWith("Error: ") ? tryOne(s.slice(7)) : null);
}

/**
 * 取结构化错误码（非结构化返回 `null`）。
 *
 * 用于按码做分支判断（如 `id_conflict` 冲突流程）——按**码**判断不受当前语言译文影响，
 * 不要用本地化后的文案去 `includes("...")`。
 */
export function backendErrorCode(raw: unknown): string | null {
  return parseStructured(raw)?.code ?? null;
}

/**
 * 构造前端侧结构化错误（与后端 `infra::berror::be_err` 同构）。
 *
 * 便于前端自身产生的错误（如当前不在 Tauri 环境）走同一套本地化与展示链路。
 */
export function feErr(code: string, arg = ""): string {
  return JSON.stringify({ code, arg });
}

/**
 * 后端错误串 → 本地化文案。
 *
 * 入参可为任意抛出值：`Error`（Tauri 的 IPC 错误）取 `.message`，字符串原样使用。
 * 后端结构化错误为 `{"code","arg"}` 的 JSON（由 `infra::berror::be_err` 构造）；
 * 解析后按 `code` 查 `beErr`（传入的 `t` 优先，缺译时回退 `zhCN`，保证中文永不丢失）。
 * 非结构化（旧式中文串）或无法解析的 JSON 原样返回，保证向后兼容。
 *
 * 用法（统一替换组件里 `e instanceof Error ? e.message : ...` 与 `String(e)` 的展示点）：
 * ```ts
 * showToast("error", backendErrorText(e));        // 自动按当前语言本地化，缺译回退中文
 * showToast("error", backendErrorText(e, t));     // 也可显式传入 t
 * ```
 */
export function backendErrorText(raw: unknown, t?: LangDict): string {
  const s = errorRaw(raw);
  if (!s) return s;
  const parsed = parseStructured(s);
  if (parsed) {
    const tpl = t?.beErr?.[parsed.code] ?? zhCN.beErr?.[parsed.code];
    if (tpl) return fill(tpl, { arg: parsed.arg });
  }
  return s;
}
