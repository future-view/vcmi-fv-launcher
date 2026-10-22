//! 启动器自动更新的**纯逻辑辅助**（禁 import react；禁 invoke——IPC 统一在 `./tauri`）。
//!
//! IPC 命令契约与类型（`UpdateInfo` / `UpdateProgress` 等）见 `./tauri` 的
//! 「启动器自动更新」章节；本文件只放展示层要用的取词 / 格式化等无副作用逻辑。

import type { UpdateInfo } from "./tauri";

/** 按界面语言取更新说明（english 兜底，再兜任意非空语言），与领域多语言取词同款策略 */
export function pickNotes(info: UpdateInfo | null, uiLang: string): string[] {
  const map = info?.notesI18n;
  if (!map) return [];
  const lang = uiLang.trim().toLowerCase();
  if (lang && map[lang]?.length) return map[lang];
  if (map["english"]?.length) return map["english"];
  return Object.values(map).find((v) => v.length > 0) ?? [];
}

/**
 * 按界面语言取**完整 Markdown** 更新说明（english 兜底，再兜任意非空语言）。
 *
 * 新清单带 `notesMdI18n`（ChangeLog 语言块原文，含标题 / 表格 / 引用 / 代码块）→ 直接返回；
 * 旧清单只有 `notesI18n` 行数组 → 重铸成 `- ` 列表（与旧弹窗观感一致），保证 0.1.x 清单仍可读。
 * 两者都没有 → 空串（调用方据此显示「本次更新没有说明」）。
 */
export function pickNotesMarkdown(info: UpdateInfo | null, uiLang: string): string {
  const md = pickLangEntry(info?.notesMdI18n, uiLang);
  if (md?.trim()) return md;
  const lines = pickNotes(info, uiLang);
  return lines.length ? lines.map((x) => `- ${x}`).join("\n") : "";
}

/** 多语言取词：uiLang → english → 任意非空值（与 `pickNotes` 同款回退链） */
function pickLangEntry<T>(map: Record<string, T> | undefined, uiLang: string): T | undefined {
  if (!map) return undefined;
  const lang = uiLang.trim().toLowerCase();
  const hit = lang ? map[lang] : undefined;
  if (hit) return hit;
  if (map["english"]) return map["english"];
  return Object.values(map).find(Boolean);
}

/** 体积格式化（MB 保留 1 位；<1MB 显示 KB） */
export function formatBytes(n: number | undefined): string {
  if (!n || n <= 0) return "0";
  if (n < 1024 * 1024) return `${Math.max(1, Math.round(n / 1024))} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

/** 进度百分比（0-100，未知总量返回 null） */
export function progressPercent(received: number, total: number): number | null {
  if (total <= 0) return null;
  return Math.min(100, Math.round((received / total) * 100));
}
