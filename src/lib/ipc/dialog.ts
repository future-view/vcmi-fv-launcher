/**
 * 原生文件 / 目录选择对话框（极薄的 `plugin-dialog` 包装）。
 *
 * 拆分自原 `@/lib/tauri.ts` 单体；对外仍由该 barrel 统一导出，调用方无需改动。
 */
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { isTauri } from "./core";

/** 打开文件夹选择对话框（`title` 必传：由调用方从 i18n 词典取词，避免硬编码回退） */
export async function pickDirectory(title: string): Promise<string | null> {
  if (!isTauri()) return null;
  const path = await openDialog({ directory: true, title });
  return path ? String(path) : null;
}

/** 选择单个文件（可传扩展名过滤，如 `["exe"]`）；非 Tauri 环境返回 null */
export async function pickFile(title: string, extensions?: string[]): Promise<string | null> {
  if (!isTauri()) return null;
  const path = await openDialog({
    directory: false,
    title,
    filters: extensions && extensions.length > 0 ? [{ name: title, extensions }] : undefined,
  });
  return path ? String(path) : null;
}
