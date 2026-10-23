/**
 * 本地 zip 安装模组（探查 + 安装）。
 *
 * 拆分自原 `@/lib/tauri.ts` 单体；对外仍由该 barrel 统一导出，调用方无需改动。
 */
import { invoke } from "@tauri-apps/api/core";
import { feErr } from "@/lib/backendError";
import { isTauri } from "./core";

/* ----------------------------- 本地 zip 安装模组 ----------------------------- */

/** zip 包探查结果（安装前确认用；后端 `preview_mod_zip`） */
export interface ModZipPreview {
  /** 归一后的模组 id（= 目标目录名；已去掉尾随版本号） */
  modId: string;
  name: string;
  version: string;
  modType: string;
  /** 将安装的文件数（不含目录条目） */
  entryCount: number;
  /** 安装目标目录（绝对路径，位于用户 Mods 目录下） */
  target: string;
  /** 目标目录是否已存在（true → 需确认覆盖） */
  exists: boolean;
}

/** 探查 zip 包（**不写盘**）：识别 mod 根、读取元数据、推导 id 与目标目录。 */
export async function previewModZip(zipPath: string): Promise<ModZipPreview> {
  if (!isTauri()) throw new Error(feErr("frontend_not_tauri"));
  return await invoke<ModZipPreview>("preview_mod_zip", { zipPath });
}

export interface LocalModInstallResult {
  modId: string;
  dest: string;
  installedFiles: number;
  /** 是否替换了已存在的同名模组 */
  replaced: boolean;
}

/**
 * 安装本地 zip 到用户 Mods 目录（`<userModsDir>/<modId>`）。
 *
 * `overwrite=false` 且同名已存在 → 抛错（中文提示），由调用方确认后改传 `true`。
 * 后端保证：失败不留半截目录、不破坏既有安装（先解压到临时目录再就位）。
 */
export async function installLocalMod(
  zipPath: string,
  overwrite: boolean,
): Promise<LocalModInstallResult> {
  if (!isTauri()) throw new Error(feErr("frontend_not_tauri"));
  return await invoke<LocalModInstallResult>("install_local_mod", { zipPath, overwrite });
}
