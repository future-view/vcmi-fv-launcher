import type { ModInfo } from "./modScanner";

/**
 * VCMI 原生预设格式（对应 `references/vcmi/lib/modding/ModManager.cpp` 的 `CModsPresetState`）：
 * - `mods`: 仅**根模组** id 数组（`getRootMods` 读取，VCMI 只从根模组加载）。
 * - `settings`: `根模组 id -> { 子模组名: bool }`（`getModSettings` 读取）。
 *   子模组名是「去掉根模组前缀的点号之后的部分」，例如 `P.S1` 对应 `settings["P"]["S1"]`
 *   （`setModActive("P.S1")` → `setSettingActive("P", "S1")`）。
 * - 启动期 `addNewModsToPreset` 会把**未写入 settings 的子模组**默认成 `!keepDisabled`
 *   （即 keepDisabled=false → 启用，true → 保持关）。
 */
export interface VcmiPreset {
  mods: string[];
  settings?: Record<string, Record<string, boolean>>;
}

function buildChildrenMap(flat: ModInfo[]): Map<string, ModInfo[]> {
  const map = new Map<string, ModInfo[]>();
  for (const m of flat) {
    if (m.isSubmod && m.parentId) {
      const arr = map.get(m.parentId);
      if (arr) arr.push(m);
      else map.set(m.parentId, [m]);
    }
  }
  return map;
}

/**
 * `settings[父][子]` 的键：子模组 id 去掉父前缀，并**小写**。
 *
 * VCMI 扫描已装模组时统一 `to_lower_copy`（`ModManager.cpp`），而后端 `norm_mod_id`
 * 保留磁盘目录大小写（`Mods/hota/mods/mainMenu` → `hota.mainMenu`）。若照原样落
 * `mainMenu`，VCMI 按 `mainmenu` 查找**匹配不上** → 视作未设置 → 被
 * `addNewModsToPreset` 补成默认启用。读写两侧必须同口径，否则界面与游戏不一致。
 */
function submodKey(parentId: string, childId: string): string {
  return childId.slice(parentId.length + 1).toLowerCase();
}

/**
 * 判定某模组是否与「已启用集合」互斥（对齐 `ModDependenciesResolver` 的双向判定）。
 *
 * VCMI 每次进程启动都会跑 `updatePreset()`：两个已启用模组互斥时，**较晚解算的一方**
 * 被判 BROKEN 并 `setModActive(false)`，随后 `CModHandler` 落盘 —— 于是我们写进预设的
 * 模组「运行一次游戏后自动消失」（实测 `h3e-main-menu` × `hota.mainmenu`）。
 *
 * 因此写盘侧必须把冲突子模组显式钉 `false`：VCMI `addNewModsToPreset` 会把**未写入
 * settings 的子模组**补成 `!keepDisabled`（默认 true），省略 ≠ 禁用。id 一律小写比较。
 */
function conflictsWithEnabled(
  modId: string,
  enabledLower: Set<string>,
  lowerFlat: Map<string, ModInfo>,
): boolean {
  const lc = modId.toLowerCase();
  // 正向：本模组把某个已启用模组列为 conflicts
  const self = lowerFlat.get(lc);
  if (self) {
    for (const c of self.conflicts) {
      if (enabledLower.has(c.toLowerCase())) return true;
    }
  }
  // 反向：某个已启用模组把本模组列为 conflicts（精确 id，不按根模组模糊匹配）
  for (const id of enabledLower) {
    const m = lowerFlat.get(id);
    if (!m) continue;
    for (const c of m.conflicts) {
      if (c.toLowerCase() === lc) return true;
    }
  }
  return false;
}

/**
 * 把启动器内部的「扁平选中集合」（根模组 + 子模组 id 并列）转成 VCMI 原生格式。
 *
 * - `mods`: 所有根模组（含基线/强制），子模组 id 不进此处。
 * - `settings`: 对 `managedRoots` 的**全部**已装子模组**无条件**写真值（含 false）。
 *   基线/强制模组（vcmi / core / 必需 mod）的子模组交给 VCMI 默认，**不钉死**。
 *
 * 无条件写真值（而非「只写与 VCMI 默认不同者」）是必须的：
 * ①「省略」与「有意沿用默认」在 VCMI 格式里不可区分，省略项会被自动补成启用；
 * ②即使界面认为某子模组已选中，只要它与已启用模组互斥，仍落 `false`——否则 VCMI
 *   会在下次启动时把互斥另一方剔出预设（见 `conflictsWithEnabled`）。
 * 此外还需覆盖「压缩在 zip 内、不在 flat 中」的冲突目标子模组：它们同样会被
 * `addNewModsToPreset` 自动启用，故按预设里声明的 conflicts 反向钉 false。
 */
export function toVcmiPreset(
  flat: ModInfo[],
  ids: Iterable<string>,
  managedRoots?: Set<string>,
): VcmiPreset {
  const flatMap = new Map(flat.map((m) => [m.id, m]));
  const lowerFlat = new Map<string, ModInfo>();
  for (const m of flat) lowerFlat.set(m.id.toLowerCase(), m);
  const childrenMap = buildChildrenMap(flat);
  const enabled = new Set(ids);
  const enabledLower = new Set<string>();
  for (const id of enabled) enabledLower.add(id.toLowerCase());
  const mods = new Set<string>();
  const settings: Record<string, Record<string, boolean>> = {};

  for (const id of ids) {
    const m = flatMap.get(id);
    if (m && m.isSubmod && m.parentId) {
      // 子模组依赖父根模组加载：确保父根模组被启用（与 VCMI setModActive 一致）
      mods.add(m.parentId);
      continue;
    }
    mods.add(id);
  }

  const rootsToScan = managedRoots ?? mods;
  // 可管辖根模组的「小写 → 原 id」索引，供冲突目标的归属判定（id 大小写不敏感）
  const managedRootByLower = new Map<string, string>();
  for (const root of rootsToScan) {
    const rm = flatMap.get(root);
    if (rm && !rm.isSubmod) managedRootByLower.set(root.toLowerCase(), root);
  }

  const setFalse = (rootId: string, rel: string) => {
    (settings[rootId] ??= {})[rel.toLowerCase()] = false;
  };

  for (const root of rootsToScan) {
    const rm = flatMap.get(root);
    if (!rm || rm.isSubmod) continue;
    const children = childrenMap.get(root);
    if (!children) continue;
    for (const child of children) {
      const rel = submodKey(root, child.id);
      const wanted = enabled.has(child.id);
      const active =
        wanted && !conflictsWithEnabled(child.id, enabledLower, lowerFlat);
      (settings[root] ??= {})[rel] = active;
    }
  }

  // zip 内子模组不在 flat 中（看不到 clones 关系），只能按「已启用模组的 conflicts 声明」
  // 反向补钉：目标形如 `parent.child` 且 parent 属可由用户管理的根模组时才介入。
  for (const id of enabled) {
    const m = lowerFlat.get(id.toLowerCase());
    if (!m) continue;
    for (const c of m.conflicts) {
      const dot = c.indexOf(".");
      if (dot <= 0) continue; // 根模组级互斥由 toggle 守卫拦截，不在此擅自移除
      const owner = managedRootByLower.get(c.slice(0, dot).toLowerCase());
      if (!owner) continue; // 不改基线/强制模组管辖范围
      setFalse(owner, c.slice(dot + 1));
    }
  }

  return { mods: [...mods], settings: Object.keys(settings).length ? settings : undefined };
}

/**
 * 反向：VCMI 原生 `{mods, settings}` → 启动器扁平选中集合（根模组 + 实际启用的子模组 id）。
 * - 子模组启用 = `settings[root][rel] === true`，或「未显式设置且 `!keepDisabled`（默认启用）」。
 * - **id 匹配大小写不敏感**：VCMI 写盘时统一 `to_lower_copy`（`ModManager.cpp`），而磁盘目录
 *   可能保留大小写（预设 `marketoftime` / `fairytale` vs 磁盘 `marketOfTime` / `Fairytale`）——
 *   原样比较会把这类模组静默丢掉（`mods` 数组里的条目解析不到就永远不会出现在选中集合）。
 *   输出一律采用 **`flat` 中的磁盘写法**（权威 id）；`flat` 里解析不到的 id 原样保留
 *   （缺失模组防御，由调用方据实报缺失）。
 */
export function fromVcmiPreset(flat: ModInfo[], preset: VcmiPreset): string[] {
  const lowerFlat = new Map<string, ModInfo>();
  for (const m of flat) lowerFlat.set(m.id.toLowerCase(), m);
  const childrenMap = buildChildrenMap(flat);
  // 小写键索引：子模组表的 parentId（磁盘写法）与 settings 的根键都可能与查询写法不一致
  const lowerChildren = new Map<string, ModInfo[]>();
  for (const [pid, list] of childrenMap) lowerChildren.set(pid.toLowerCase(), list);
  const lowerSettings = new Map<string, Record<string, boolean>>();
  for (const [k, v] of Object.entries(preset.settings ?? {})) lowerSettings.set(k.toLowerCase(), v);

  const out = new Set<string>();
  const mods = preset.mods ?? [];

  for (const root of mods) {
    const m = lowerFlat.get(root.toLowerCase());
    if (m && m.isSubmod) continue; // 防御：mods 只应含根
    const rid = m ? m.id : root; // 磁盘写法优先；解析不到则原样保留
    out.add(rid);
    const children = lowerChildren.get(rid.toLowerCase());
    if (!children) continue;
    for (const child of children) {
      // rel 用子模组**自带的磁盘 parentId** 取后缀（同目录扫描口径），不依赖查询写法
      const rel = submodKey(child.parentId, child.id);
      const override = lowerSettings.get(rid.toLowerCase())?.[rel];
      const effective = override !== undefined ? override : !child.keepDisabled;
      if (effective) out.add(child.id);
    }
  }
  return [...out];
}

/**
 * 从选中集合中剔除「基线模组及其子模组」，得到用户可管理的 id（基线永远启用、不可取消）。
 * `flat` 用于解析子模组的父归属。
 */
export function excludeBaseSubmods(
  flat: ModInfo[],
  ids: Iterable<string>,
  baseMods: Set<string> | string[],
): string[] {
  const base = baseMods instanceof Set ? baseMods : new Set(baseMods);
  const flatMap = new Map(flat.map((m) => [m.id, m]));
  const out: string[] = [];
  for (const id of ids) {
    const m = flatMap.get(id);
    if (m?.isSubmod) {
      if (m.parentId && base.has(m.parentId)) continue; // 基线模组的子模组不参与用户选择
    } else if (base.has(id)) {
      continue;
    }
    out.push(id);
  }
  return out;
}
