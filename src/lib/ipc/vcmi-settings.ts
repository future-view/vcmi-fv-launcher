/**
 * 设置面板：VCMI `settings.json` 的默认值对比、单项写入与重置。
 *
 * 拆分自原 `@/lib/tauri.ts` 单体；对外仍由该 barrel 统一导出，调用方无需改动。
 */
import { collapseToArrayKey, flatten, hasArrayIndexSegment, readVcmiSettings, writeVcmiSettings } from "./settings-json";

/**
 * 深度相等比较（用于判断设置项是否等于官方默认值，决定是否写入）。
 */
function deepEqual(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (typeof a !== typeof b) return false;
  if (a === null || b === null || typeof a !== "object") return a === b;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  const ka = Object.keys(a as object);
  const kb = Object.keys(b as object);
  if (ka.length !== kb.length) return false;
  return ka.every((k) => deepEqual((a as any)[k], (b as any)[k]));
}

/**
 * 读取 VCMI settings.json 实时内容，并与官方默认值合并补全缺失键。
 * 纯前端（非 Tauri）环境下返回官方默认值表，便于离线预览。
 */
export async function loadVcmiSettings(
  officialDefaults: Record<string, unknown>,
  configDir?: string
): Promise<Record<string, unknown>> {
  const base: Record<string, unknown> = { ...officialDefaults };
  const real = await readVcmiSettings(configDir);
  if (real) {
    // VCMI 文件是嵌套结构，展开为平面键后再合并，避免与 schema 平面键混用
    const flat = flatten(real);
    for (const [k, v] of Object.entries(flat)) base[k] = v;
  }
  return base;
}

/**
 * 计算"仅差异"部分：从 current 中剔除与默认值深度相等的键。
 * 设计文档第 13 行要求：只对与默认值不同的配置进行写入。
 */
function diffFromDefaults(
  current: Record<string, unknown>,
  defaults: Record<string, unknown>
): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(current)) {
    const def = defaults[k];
    // 官方未定义的键（如旧版遗留）一律保留
    if (def === undefined) {
      out[k] = v;
      continue;
    }
    if (!deepEqual(v, def)) out[k] = v;
  }
  return out;
}

/**
 * 单键写回 VCMI settings.json。
 * - 设为「非默认」值：直接更新该键（合并写）。
 * - 显式改回「默认值」：删除该键，让 VCMI 回退到内置默认（而非"不写"）。
 * 注意：不能走 diffFromDefaults 的"等于默认就剔除"逻辑——否则改回默认时
 * 差异为空、后端合并写不会动文件里的旧非默认值，造成"改不回去"的缺陷。
 */
export async function writeVcmiSetting(
  key: string,
  value: unknown,
  defaults: Record<string, unknown>,
  configDir?: string
): Promise<void> {
  const isKnown = key in defaults;
  const equalToDefault = isKnown && deepEqual(value, defaults[key]);
  if (equalToDefault) {
    // 改回默认：从文件中删除该键，让 VCMI 回退到内置默认。
    // 防御：仅对「非数组索引路径」执行删除——含索引的数组键若走 remove_path
    // 会逐元素缩位、破坏数组结构；此类键改为整键更新（保留其当前值）。
    if (!hasArrayIndexSegment(key)) {
      await writeVcmiSettings({ __vcmi_fv_delete__: [collapseToArrayKey(key)] }, configDir);
      return;
    }
  }
  // 非默认 / 或含数组索引的键：更新该键（平面键会被 writeVcmiSettings 还原为嵌套）
  await writeVcmiSettings({ [key]: value }, configDir);
}

/**
 * 重置为默认：读出当前 settings.json，收集其中所有「本启动器已知且可管理」的键
 * （即用户可能自定义过的官方参数），交由后端将该键从文件中删除 ⇒
 * 这些参数回到 VCMI 内置默认值；不写入任何硬编码默认值，也不动其它遗留键。
 * 这与设计目标一致：减少硬编码逻辑，跟随 VCMI 内置默认。
 */
export async function resetVcmiSettings(
  defaults: Record<string, unknown>,
  configDir?: string
): Promise<void> {
  const real = await readVcmiSettings(configDir);
  const flat = real ? flatten(real) : {};
  const keysToClear = new Set<string>();
  for (const k of Object.keys(flat)) {
    // 仅收集官方 schema 定义的已知键，遗留键不动
    if (k in defaults) {
      // 数组元素路径（如 logging.loggers.0）归并到整键（logging.loggers）删除，
      // 避免后端对数组做逐元素缩位删除而破坏结构
      keysToClear.add(collapseToArrayKey(k));
    }
  }
  // 通过 write_vcmi_settings 的删除指令（__vcmi_fv_delete__）真正移除这些键，
  // 使它们回退到 VCMI 内置默认值；不写入任何硬编码默认值。
  await writeVcmiSettings({ __vcmi_fv_delete__: [...keysToClear] }, configDir);
}
