/**
 * 外挂运行时组件（runtime，如 AI 超分 ONNX Runtime）。
 *
 * 拆分自原 `@/lib/tauri.ts` 单体；对外仍由该 barrel 统一导出，调用方无需改动。
 */
import { invoke } from "@tauri-apps/api/core";
import { isTauri } from "./core";

/** 组件内单个文件的状态 */
export interface RuntimeFileStatus {
  name: string;
  present: boolean;
  sha_ok: boolean;
}

/** 组件状态（`runtime_status` 返回） */
export interface RuntimeStatus {
  id: string;
  /** 组件目录（绝对路径，`/` 分隔）；`null` = 无法确定 */
  dir: string | null;
  /** 文件齐备且 sha256 全部匹配 */
  installed: boolean;
  /** 已安装组件的版本；未安装时为 null */
  version: string | null;
  files: RuntimeFileStatus[];
  /** 是否配置了远程包 —— 决定能否显示「安装」 */
  has_package: boolean;
}

/** 安装进度（事件 `runtime-install-progress`） */
export interface RuntimeProgress {
  id: string;
  received: number;
  total: number;
  done?: boolean;
}

/** AI 组件 id（与后端 `runtime::AI` 对齐） */
export const AI_RUNTIME_ID = "ai";

/** 查询组件状态；失败返回 null（组件未配置 / 未知 id 都属正常） */
export async function fetchRuntimeStatus(id: string): Promise<RuntimeStatus | null> {
  if (!isTauri()) return null;
  try {
    return (await invoke("runtime_status", { id })) as RuntimeStatus;
  } catch (e) {
    console.error("查询运行时组件失败", e);
    return null;
  }
}

/** 安装组件（**阻塞**至完成），返回安装后的最新状态 */
export async function installRuntime(id: string): Promise<RuntimeStatus> {
  const r = (await invoke("runtime_install", { id })) as { ok: boolean; status: RuntimeStatus };
  return r.status;
}

/** 取消安装（置哨兵，下载循环下一块退出） */
export async function cancelRuntime(id: string): Promise<void> {
  if (!isTauri()) return;
  try {
    await invoke("runtime_cancel", { id });
  } catch (e) {
    console.error("取消安装失败", e);
  }
}

/** 卸载组件（只删程序文件，不动用户数据），返回最新状态 */
export async function uninstallRuntime(id: string): Promise<RuntimeStatus> {
  const r = (await invoke("runtime_uninstall", { id })) as { ok: boolean; status: RuntimeStatus };
  return r.status;
}

/** 订阅组件安装进度，返回取消订阅函数 */
export async function onRuntimeProgress(cb: (p: RuntimeProgress) => void): Promise<() => void> {
  if (!isTauri()) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  return await listen<RuntimeProgress>("runtime-install-progress", (e) => cb(e.payload));
}
