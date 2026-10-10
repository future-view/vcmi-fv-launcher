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
