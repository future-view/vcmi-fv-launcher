/**
 * 领域（Realm）生命周期：列表 / 激活 / 增删改 / 自检修复 / VCMI 进程。
 *
 * 拆分自原 `@/lib/tauri.ts` 单体；对外仍由该 barrel 统一导出，调用方无需改动。
 */
import { invoke } from "@tauri-apps/api/core";
import { feErr } from "@/lib/backendError";
import { isTauri, wrapLaunch, type LaunchResult } from "./core";
import { invalidateModsScan } from "./wiki";

/* ----------------------------- 领域（Realm）系统 ----------------------------- */

/**
 * 领域元信息（设计 §6；唯一权威是 `{realm-dir}/realm.json`）。
 * 类型定义统一放在 `src/realm/types.ts`，此处仅转发，避免出现第二份结构定义。
 */
export type { Realm, RealmList } from "@/lib/realm/types";
import type { Realm, RealmList } from "@/lib/realm/types";

/** 列出全部领域 + 当前激活 id */
export async function listRealms(): Promise<RealmList> {
  if (!isTauri()) return { realms: [], activeRealm: null };
  try {
    return await invoke<RealmList>("list_realms");
  } catch {
    return { realms: [], activeRealm: null };
  }
}

/** 当前激活领域 id */
export async function getActiveRealm(): Promise<string | null> {
  if (!isTauri()) return null;
  try {
    const r = await invoke<{ activeRealm: string | null }>("get_active_realm");
    return r.activeRealm ?? null;
  } catch {
    return null;
  }
}

/** 取某领域隔离目录的 config 绝对路径（作为后续读写领域设置的 configDir） */
export async function realmConfigDir(realmId: string): Promise<string | null> {
  if (!isTauri()) return null;
  try {
    return await invoke<string>("realm_config_dir", { realmId });
  } catch {
    return null;
  }
}

/** 写入领域的**全部模组预设**（多预设支持，§3.3）：整体接管 `presets` 与 `activePreset`。
 * 这是「启用组合」的权威来源；与 realm.json 的 borrowed / bundled（携带清单）解耦。 */
export async function writeRealmPresets(
  realmId: string,
  presets: Record<string, { mods: string[]; settings?: Record<string, unknown> }>,
  activePreset: string,
): Promise<void> {
  return invoke("write_realm_presets", { realmId, presets, activePreset });
}

/**
 * 把启动器语言写入 VCMI 生效的 `settings.json` 的 `general.language`（语言设置层）。
 *
 * 必须与翻译模组注入**成对**执行：VCMI 会把 `language` 与 `general.language` 不一致的
 * `modType: "Translation"` 模组判为 BROKEN 并从激活预设中移除
 * （`references/vcmi/lib/modding/ModManager.cpp:829-832`），只写预设不写该设置会被立刻删掉。
 *
 * `configDir` 传入时作用于该领域配置目录，否则作用于全局用户配置目录。幂等（值一致不写盘）。
 */
export async function setVcmiLanguage(lang: string, configDir?: string): Promise<void> {
  if (!isTauri()) return;
  return invoke("set_vcmi_language", { lang, configDir });
}

/**
 * 按启动器语言同步某预设的「语言相关模组」（语言层）：从预设移除 `remove`、加入 `add`。
 * `configDir` 传入时作用于该领域隔离目录的 modSettings.json，否则作用于全局。
 * 返回 `add` 中磁盘上找不到的模组 id（仅用于提示，不阻断）。
 */
export async function patchPresetLangMods(
  configDir: string | undefined,
  presetId: string,
  add: string[],
  remove: string[],
): Promise<string[]> {
  if (!isTauri()) return [];
  const r = await invoke<{ missing?: string[] }>("patch_preset_lang_mods", {
    configDir,
    presetId,
    add,
    remove,
  });
  return r?.missing ?? [];
}

/** 新建领域（**id 必填**）：立即建目录并落盘 realm.json，返回创建后的领域 */
export async function createRealm(data: unknown): Promise<Realm | null> {
  if (!isTauri()) throw new Error(feErr("frontend_not_tauri"));
  return invoke<Realm>("create_realm", { data });
}

/** 领域 ID 可用性校验（新建弹窗实时校验）：`available=false` 时 `reason` ∈ empty/invalid/occupied */
export async function checkRealmId(
  id: string,
): Promise<{ available: boolean; reason: string }> {
  if (!isTauri()) return { available: true, reason: "" };
  return invoke<{ available: boolean; reason: string }>("check_realm_id", { id });
}

/** 更新已存在领域（按 id 合并字段） */
export async function updateRealm(realmId: string, data: unknown): Promise<void> {
  if (!isTauri()) throw new Error(feErr("frontend_not_tauri"));
  return invoke("update_realm", { realmId, data });
}

/** 删除领域（若激活中则先停用还原 dirs.json） */
export async function deleteRealm(realmId: string): Promise<void> {
  if (!isTauri()) throw new Error(feErr("frontend_not_tauri"));
  return invoke("delete_realm", { realmId });
}

/** 激活领域：改写 dirs.json 指向领域隔离目录 + 从全局提取用户设置（§9.7.2） */
export async function activateRealm(realmId: string): Promise<LaunchResult> {
  if (!isTauri()) return { ok: false, error: feErr("frontend_not_tauri") };
  const res = await wrapLaunch(() => invoke("activate_realm", { realmId }));
  // 领域激活会改变模组的可见集合（bundled 复制 / borrowed 借用）→ 扫描记忆必须失效，
  // 否则回到模组页看到的是陈旧列表（见 scanRealMods 的进程内记忆注释）
  if (res.ok) invalidateModsScan();
  return res;
}

/** 停用领域：用原文备份还原 dirs.json，设置回到全局 */
export async function deactivateRealm(): Promise<LaunchResult> {
  if (!isTauri()) return { ok: false, error: feErr("frontend_not_tauri") };
  const res = await wrapLaunch(() => invoke("deactivate_realm"));
  if (res.ok) invalidateModsScan();
  return res;
}

/** 切换领域「屏蔽游戏自带地图」阀门（§8.3），并持久化偏好 */
export async function setRealmMapsBlocked(realmId: string, blocked: boolean): Promise<LaunchResult> {
  if (!isTauri()) return { ok: false, error: feErr("frontend_not_tauri") };
  return wrapLaunch(() => invoke("set_realm_maps_blocked", { realmId, blocked }));
}

/** 进程守卫（§4.4）：强制结束**游戏本体**（「结束游戏」按钮；不动官方启动器） */
export async function killVcmi(): Promise<void> {
  if (!isTauri()) return;
  return invoke("kill_vcmi");
}

/** 崩溃自检（§4.5）报告 */
export interface RealmSelfCheck {
  ok: boolean;
  status: string;
  activeRealm: string | null;
  dirsRealm: string | null;
  /** 后端诊断原文（中文，仅供日志/兜底；UI 文案按 status 走 i18n） */
  message: string;
  /** 是否可由 `realm_self_heal` 安全自动修复 */
  fixable: boolean;
  /** 可选修复动作（§4.5）：heal=自动自愈；unlink/merge/revert=地图阀门的人工修复 */
  actions?: string[];
}

/** 崩溃自检（§4.5）：启动后调用，返回异常报告 */
export async function realmSelfCheck(): Promise<RealmSelfCheck | null> {
  if (!isTauri()) return null;
  try {
    return await invoke<RealmSelfCheck>("realm_self_check");
  } catch {
    return null;
  }
}

/** 崩溃自愈（§4.5）：执行安全自动修复，返回修复后的报告 */
export async function realmSelfHeal(): Promise<RealmSelfCheck | null> {
  if (!isTauri()) return null;
  try {
    return await invoke<RealmSelfCheck>("realm_self_heal");
  } catch {
    return null;
  }
}

/**
 * 修复崩坏的地图阀门形态（§8.3.3 / §4.5）。
 * action：`unlink` 删悬空联接 / `merge` 合并新出现的自带地图 / `revert` 彻底还原游戏目录。
 */
export async function repairGameMaps(action: string): Promise<LaunchResult> {
  if (!isTauri()) return { ok: false, error: feErr("frontend_not_tauri") };
  return wrapLaunch(() => invoke("repair_game_maps", { action }));
}
