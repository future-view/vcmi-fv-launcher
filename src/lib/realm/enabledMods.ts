import { isReservedMod } from "@/lib/modScope";
import type { Realm } from "@/lib/realm/types";

export interface EnabledSet {
  ids: string[];
  /** 实际采用的预设 id（用于页头标注） */
  presetId: string | null;
}

/**
 * 从领域（或全局）`modSettings.json` 推导**启用模组集合**。
 *
 * 对齐 `references/vcmi/lib/modding/ModManager.cpp`：
 * - `getActiveRootMods()` = `presets[id].mods` + 内置 `core`；
 * - `getActiveMods()`     = 根模组 ⊕ `settings[root][child] === true` 展开成 `root.child`
 *   （`setModActive` 的拆点号语义）。
 *
 * 只读 `mods` 会漏掉**全部子模组**——实测本例 86 个启用城镇里只有 32 个是顶层城镇。
 *
 * 未出现在 `settings` 里的子模组**不补**：VCMI 的 `addNewModsToPreset()` 虽然会按
 * `!keepDisabled` 自动启用它们，但那属于 VCMI 运行期推导，落盘文件里 `false` 的条目
 * 往往是它自己判定的 broken 项，一并捞回来会失真。原版（`core.*`）改由页面开关控制。
 *
 * 复用于阵营巡礼 auto-deck 过滤（设计 §7.1：auto = 本领域启用阵营）。
 */
export function collectEnabledIds(
  settings: Record<string, unknown> | null,
  r: Realm,
  isGlobal: boolean,
): EnabledSet {
  const presets = settings?.presets as
    | Record<string, { mods?: unknown; settings?: Record<string, Record<string, unknown>> }>
    | undefined;
  const activePreset = typeof settings?.activePreset === "string" ? settings.activePreset : null;

  // 领域私有配置 → 领域预设（§3.3 固定 `__realm__`）；共享全局配置 → VCMI 当前激活的全局预设
  const wanted = !isGlobal && r.presetId ? r.presetId : activePreset;
  const preset =
    (wanted ? presets?.[wanted] : undefined) ?? (activePreset ? presets?.[activePreset] : undefined);
  if (!preset) return { ids: [], presetId: wanted ?? activePreset };

  const rootIds = (Array.isArray(preset.mods) ? preset.mods : []).filter(
    (x): x is string => typeof x === "string" && !!x && !isReservedMod(x),
  );
  const ids: string[] = [...rootIds];
  const sub = preset.settings ?? {};
  for (const root of rootIds) {
    const map = sub[root];
    if (!map) continue;
    for (const [child, on] of Object.entries(map)) {
      if (on === true) ids.push(`${root}.${child}`);
    }
  }
  return { ids, presetId: wanted ?? activePreset };
}
