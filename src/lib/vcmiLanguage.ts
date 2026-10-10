/**
 * VCMI 游戏语言（`settings.json` 的 `general.language`）的**进程内单次加载 + 订阅**。
 *
 * 用途：VCMI 会把「`modType: "Translation"` 且 `mod.json.language` ≠ 当前游戏语言」的翻译模组
 * 判为 BROKEN，并从激活预设中**移除**它（`references/vcmi/lib/modding/ModManager.cpp` 的
 * `ModDependenciesResolver::tryAddMods`；官方 launcher 的 `canEnableMod` 也据此拒绝启用）。
 * 前端提前提示，避免"启用成功却在游戏里消失"。
 *
 * 与 `engineVersion.ts` 同款：模块级缓存 + `useSyncExternalStore`，避免每张卡片各查一次 IPC。
 */

import { isTauri, readVcmiSettings } from "./tauri";
import { SETTING_SCHEMA_DEFAULTS } from "@/generated/settingDefaults";
import { VCMI_DEFAULT_LANG } from "@/lib/vcmiLangs";

/** `undefined` = 尚未加载；`null` = 已加载但不可用（非 Tauri） */
let value: string | null | undefined;
let inflight: Promise<void> | null = null;
const listeners = new Set<() => void>();

function emit(): void {
  listeners.forEach((l) => l());
}

/** 同步快照：`undefined` 表示"还不知道"，调用方此时**不要**判定语言是否相符。 */
export function getVcmiLanguageSnapshot(): string | null | undefined {
  return value;
}

export function subscribeVcmiLanguage(cb: () => void): () => void {
  listeners.add(cb);
  return () => {
    listeners.delete(cb);
  };
}

/**
 * 幂等加载（模块级去重）。读不到 `general.language` 时回退官方 schema 默认值（`english`）。
 * 非 Tauri 环境保持 `undefined`（浏览器自测不做判定）。
 */
export async function loadVcmiLanguage(configDir?: string): Promise<void> {
  if (value !== undefined) return;
  if (inflight) return inflight;
  if (!isTauri()) return;
  inflight = (async () => {
    try {
      const settings = await readVcmiSettings(configDir);
      const raw = (settings?.general as Record<string, unknown> | undefined)?.language;
      value = typeof raw === "string" && raw.trim() ? raw.trim() : defaultLanguage();
    } catch {
      value = defaultLanguage();
    } finally {
      inflight = null;
      emit();
    }
  })();
  return inflight;
}

function defaultLanguage(): string {
  const d = SETTING_SCHEMA_DEFAULTS["general.language"];
  return typeof d === "string" && d.trim() ? d : VCMI_DEFAULT_LANG;
}

/**
 * 模组的基准语言（VCMI `ModDescription::getBaseLanguage`）：`mod.json` 未声明时视为 `english`。
 * ⚠️ 这是「翻译模组会服务哪种语言」的判定口径，必须与引擎一致。
 */
export function modBaseLanguage(modLanguage?: string | null): string {
  const v = (modLanguage ?? "").trim();
  return v || VCMI_DEFAULT_LANG;
}
