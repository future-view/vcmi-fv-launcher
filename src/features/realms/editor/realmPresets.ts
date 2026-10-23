/**
 * 领域「模组组合（预设）」的**纯变换**：增删改 / 引用归属 / 清理 / 依赖补入。
 *
 * 从 `RealmEditor.tsx` 抽出：这一组逻辑只做「`Record<string, string[]>` → 新 Record」，
 * 不碰 React 状态也不读组件闭包，抽出来后既能给编辑器直接用，也能脱离 UI 单测
 * （见 `src/lib/__tests__/realmPresets.test.ts`）。
 *
 * ⚠️ 约定：**不改入参**（全部返回新对象）；「拒绝执行」用 `null` 表示，
 * 由调用方决定要不要提示 —— 这些条件（空名 / 重名 / 只剩一个组合 / 契约 id 改名）
 * 都是业务规则，不该散落在事件处理器里。
 */
import { REALM_PRESET_ID } from "./editorShared";

/** 组合名 → 模组 id 列表（子模组 id 可能带点号；写盘前由 `toVcmiPreset` 归一） */
export type PresetMap = Record<string, string[]>;

/** 新建空组合；名称为空或已存在 → `null`（不新建）。 */
export function addPreset(ps: PresetMap, name: string): PresetMap | null {
  const id = name.trim();
  if (!id || ps[id]) return null;
  return { ...ps, [id]: [] };
}

/**
 * 删除组合并给出「剩下该切到哪个」。
 *
 * 只剩一个组合时返回 `null`：引擎至少要有一份可激活的组合，
 * 空启用集会表现为"所有模组都不生效"，比留一个空组合更难排查。
 */
export function deletePreset(ps: PresetMap, id: string): { presets: PresetMap; active: string } | null {
  const rest = Object.keys(ps).filter((p) => p !== id);
  if (rest.length === 0) return null;
  const next = { ...ps };
  delete next[id];
  return { presets: next, active: rest[0] };
}

/**
 * 重命名组合：键整体迁移，其余顺序不变。
 *
 * `__realm__` 是领域契约 id（导出 / 激活都按它找），改名返回 `null`；
 * 新名为空、与原值相同、或与已有组合撞名同样拒绝。
 */
export function renamePreset(ps: PresetMap, from: string, to: string): PresetMap | null {
  const n = to.trim();
  if (!n || n === from || from === REALM_PRESET_ID || ps[n]) return null;
  const next: PresetMap = {};
  for (const [k, v] of Object.entries(ps)) next[k === from ? n : k] = v;
  return next;
}

/**
 * 从**全部组合**清理这些根模组及其子模组（realm-mods-management §7.7）：
 * 「取消引用 / 删除」的联动——物理层已移除的模组不应在任何启用集里残留。
 *
 * 子模组按 `rootOf` 归到根模组再判断（`P.S1` 的根是 `P`）。
 */
export function purgeRoots(
  ps: PresetMap,
  rootIds: string[],
  rootOf: (id: string) => string,
): PresetMap {
  if (rootIds.length === 0) return ps;
  const roots = new Set(rootIds.map((x) => x.toLowerCase()));
  const next: PresetMap = {};
  for (const [k, v] of Object.entries(ps)) {
    next[k] = v.filter((id) => !roots.has(rootOf(id).toLowerCase()));
  }
  return next;
}

/**
 * 「从模组预设引用」的归属（§7.5 核对点 3）：
 * - `create` 缺省：合并进当前领域的 `__realm__`（去重）；
 * - `create` 给定时：以该名新建组合（撞名自动加 `-2` / `-3` 后缀）。
 *
 * 返回**落定后的**组合名（供结果提示与激活切换）。
 */
export function adoptRefs(
  ps: PresetMap,
  ids: string[],
  create?: string,
): { presets: PresetMap; name: string } {
  if (create) {
    let name = create;
    let i = 2;
    while (ps[name]) name = `${create}-${i++}`;
    return { presets: { ...ps, [name]: [...new Set([...(ps[name] ?? []), ...ids])] }, name };
  }
  return {
    presets: { ...ps, [REALM_PRESET_ID]: [...new Set([...(ps[REALM_PRESET_ID] ?? []), ...ids])] },
    name: REALM_PRESET_ID,
  };
}

/**
 * 依赖处置（选项 A「实装 + 启用」）：把依赖补入**受影响子模组所在组合**的激活集。
 *
 * 引擎启动只认预设激活集（`ModManager::getActiveMods` = mods 数组 + settings 为 true 的
 * 子模组；`addNewModsToPreset` 对 root 直接跳过、绝不自动启用）——仅实装不启用，
 * 受影响子模组在游戏里仍被 BROKEN 剔除。
 *
 * 已在该组合里（大小写不敏感）的依赖不重复补入；一次都没补入时**返回原引用**，
 * 避免无改动也触发重渲染。
 */
export function enableDep(ps: PresetMap, presetNames: string[], canonical: string): PresetMap {
  if (presetNames.length === 0) return ps;
  const lc = canonical.toLowerCase();
  let changed = false;
  const next = { ...ps };
  for (const p of presetNames) {
    const cur = next[p];
    if (!cur || cur.some((v) => v.toLowerCase() === lc)) continue;
    next[p] = [...cur, canonical];
    changed = true;
  }
  return changed ? next : ps;
}
