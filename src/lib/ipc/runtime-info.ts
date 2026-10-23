/**
 * 分辨率与运行信息弹窗数据。
 *
 * 拆分自原 `@/lib/tauri.ts` 单体；对外仍由该 barrel 统一导出，调用方无需改动。
 */
import { invoke } from "@tauri-apps/api/core";
import { isTauri } from "./core";

/* ----------------------------- 运行信息弹窗 ----------------------------- */

export interface Resolution {
  width: number;
  height: number;
  primary?: boolean;
}

export interface RuntimeInfo {
  vcmiVersion: string;
  modsInstalled: number;
  modsEnabled: number;
  recommendedResolution: { width: number; height: number };
  gameDataLanguage: string;
  lastMap: string;
  lastSave: string;
  lastSettingsTab: string;
  setupCompleted: boolean;
  mainWindow: { geometry?: { x?: number; y?: number; width?: number; height?: number } } | null;
}

/** 常用分辨率候选列表（用于「分辨率」ButtonList） */
export async function listResolutions(): Promise<Resolution[]> {
  if (!isTauri()) {
    return [
      { width: 1280, height: 720 },
      { width: 1920, height: 1080 },
      { width: 2560, height: 1440 },
      { width: 3840, height: 2160 },
    ];
  }
  try {
    return await invoke<Resolution[]>("list_resolutions");
  } catch {
    return [];
  }
}

/**
 * 聚合运行态只读信息（版本 / 已安装 / 已启用 / 推荐分辨率）。
 * `configDir` 可选：传入领域配置目录时，**跟随作用域**的字段（最近地图 / 最近存档 /
 * 上次设置标签 / 游戏数据语言 / setupCompleted / 主窗口几何 / 已启用模组数）读该目录；
 * 不传 = 全局用户配置目录（见 settings-panel-design §六）。
 */
export async function getRuntimeInfo(configDir?: string): Promise<RuntimeInfo | null> {
  // 浏览器自测模式：模拟中文内容语言（与实际使用环境一致），其余运行态信息不可用
  if (!isTauri()) {
    return { gameDataLanguage: "chinese" } as RuntimeInfo;
  }
  try {
    return await invoke<RuntimeInfo>("get_runtime_info", { configDir: configDir ?? null });
  } catch {
    return null;
  }
}
