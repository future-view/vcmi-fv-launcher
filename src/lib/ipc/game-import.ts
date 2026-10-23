/**
 * 导入游戏文件（底栏入口）：地图 / 战役 / 存档 / 模组 / HD 资源。
 *
 * 拆分自原 `@/lib/tauri.ts` 单体；对外仍由该 barrel 统一导出，调用方无需改动。
 */
import { invoke } from "@tauri-apps/api/core";
import { feErr } from "@/lib/backendError";
import { isTauri } from "./core";

/* ----------------------------- 导入游戏文件（底栏入口） ----------------------------- */

/** 地图元数据（`infra::h3m::MapMeta`，camelCase；仅声明前端展示用到的字段） */
export interface GameImportMapMeta {
  kind: string;
  lang: string;
  name: string;
  description: string;
  version: string;
  players: number;
  /** S / M / L / XL / H / XH / G */
  sizeClass: string;
  /** 战役场景数（仅 .h3c） */
  scenarioCount: number;
}

/** zip 包判定为模组时的探查结果 */
export interface GameImportModPreview {
  modId: string;
  name: string;
  version: string;
  entryCount: number;
  target: string;
  exists: boolean;
}

/** zip 识别结论（`overridden` = 用户改判生效） */
export interface GameImportZipPreview {
  detected: "mod" | "maps" | "saves";
  overridden: boolean;
  /** false + detected=mod → 内容未识别，按模组尝试 */
  hasModJson: boolean;
  name?: string;
  version?: string;
  mapCount: number;
  saveCount: number;
}

/** HD 目录预检：校验 / 语言 / pak 命中数 */
export interface GameImportHdPreview {
  valid: boolean;
  /** VCMI 语言 identifier（english…） */
  language?: string;
  /** 短码（EN…） */
  languageCode?: string;
  paks: { found: number; total: number };
  missing: string[];
}

/** 普通目录递归预检：各类型计数 + 忽略数 + 冲突数（一项多文件，与 zip 同口径） */
export interface GameImportDirPreview {
  mapCount: number;
  campaignCount: number;
  saveCount: number;
  configCount: number;
  zipCount: number;
  /** 目录内不支持（无关文件 / `.exe` / 嵌套 HD 标记）的项数 */
  ignored: number;
  /** 平铺文件目标已存在的项数（zip 子项不计） */
  conflictCount: number;
}

/** 预检条目（后端 `vcmi::game_import::PreviewItem`） */
export interface GameImportPreviewItem {
  path: string;
  kind: "map" | "campaign" | "zip" | "save" | "config" | "chronicles" | "hd" | "dir" | "unsupported";
  ext: string;
  size: number;
  meta?: GameImportMapMeta;
  mod?: GameImportModPreview;
  zip?: GameImportZipPreview;
  hd?: GameImportHdPreview;
  /** 仅普通目录输入：递归收集结果统计 */
  dir?: GameImportDirPreview;
  /** 仅 .exe：Inno Setup 特征串识别结论 */
  inno?: boolean;
  target: string;
  conflict: boolean;
  ok: boolean;
  reason?: string;
}

export interface GameImportPreviewReport {
  items: GameImportPreviewItem[];
  summary: {
    total: number;
    byKind: Record<string, number>;
    conflicts: number;
    unsupported: number;
  };
}

/** 执行结果条目 */
export interface GameImportExecItem {
  path: string;
  kind: string;
  dest?: string;
  status: "added" | "replaced" | "skipped" | "failed" | "unsupported";
  reason?: string;
}

/** 本次装成的模组（前端据此发 `vcmi-fv:mods-installed` 事件） */
export interface GameImportModInstalled {
  modId: string;
  name: string;
  version: string;
}

export interface GameImportExecReport {
  total: number;
  added: number;
  skipped: number;
  failed: number;
  replaced: number;
  unsupported: number;
  items: GameImportExecItem[];
  modsInstalled: GameImportModInstalled[];
}

export type GameImportConflict = "skip" | "overwrite" | "rename";
export type GameImportOverride = "mod" | "maps" | "saves";

interface GameImportOpts {
  paths: string[];
  conflict: GameImportConflict;
  dryRun: boolean;
  /** zip 逐项改判（`path → mod|maps|saves`） */
  overrides?: Record<string, GameImportOverride>;
  /**
   * 可选覆盖目录（首启向导用：配置尚未保存，导入必须按用户选的目录落盘；
   * 历代记还要从游戏目录取内置核心模组的 `portraitsChronicles.json`）。
   * 不传 = 沿用已保存配置。
   */
  gameDir?: string;
  userDataDir?: string;
}

/** 预检（不写盘）：分类 / 识别 / 冲突检查。 */
export function importGameFiles(opts: GameImportOpts & { dryRun: true }): Promise<GameImportPreviewReport>;
/** 执行导入（按 `conflict` 落盘；`overrides` 对 zip 生效）。 */
export function importGameFiles(opts: GameImportOpts & { dryRun: false }): Promise<GameImportExecReport>;
export async function importGameFiles(
  opts: GameImportOpts,
): Promise<GameImportPreviewReport | GameImportExecReport> {
  if (!isTauri()) throw new Error(feErr("frontend_not_tauri"));
  return await invoke("import_game_files", {
    paths: opts.paths,
    conflict: opts.conflict,
    dryRun: opts.dryRun,
    overrides: opts.overrides ?? null,
    gameDir: opts.gameDir,
    userDataDir: opts.userDataDir,
  });
}
