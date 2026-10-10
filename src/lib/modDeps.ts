/**
 * 领域模组依赖求解器（「依赖处置」体系的计算核心）。
 *
 * 输入三源（领域已实装 ∪ 全局已装 ∪ 仓库索引）+ 预设表，输出：
 * - `byRoot`：按**根模组**聚合的依赖缺陷（「已实装」行按钮 / 处理弹窗消费）；
 * - `defectModIds`：依赖未满足的模组 id 集（预设页树内角标消费）；
 * - `defectRootCount`：有缺陷的根模组数（预设页操作栏徽章 / 告知弹层消费）。
 *
 * 判定口径与 `ModBrowser` 守卫一致：领域范围实装集（metaOk 过滤）、
 * 「随父模组压缩在 zip 内的子模组」按根容错；id 匹配大小写不敏感
 * （对齐 VCMI `to_lower_copy` 的磁盘扫描语义）。
 *
 * 危害分级（引擎视角，见 `ModManager.cpp` `tryAddMods` 的 BROKEN 判定）：
 * - `active`（🔴）：受影响模组已在某预设启用 —— 游戏启动时该模组正被引擎剔除；
 * - `latent`（🟠）：受影响模组未在任何预设启用 —— 启用时将受限。
 *
 * 修复四档（弹窗操作依据）：
 * - `installed`  ：依赖根在全局已装 → 「引入」（`realmModRef` 引用台账，随保存落盘）；
 * - `repo`       ：依赖根在仓库索引 → 「安装」（`installModFromRepo` 领域副本，即时执行）；
 * - `unavailable`：两处都没有 → 保持缺失（游戏运行时引擎自行 BROKEN 剔除）。
 *
 * ⚠️ 修复必须「实装 + 补入受影响预设激活集」两段式：引擎启动只认预设激活集
 * （`getActiveMods` = mods 数组 + settings 为 true 的子模组），已装但未启用的依赖
 * 仍判 BROKEN；补入的 id 用**依赖声明的精确粒度**（子模组 id 由 `toVcmiPreset`
 * 自动带出父根模组，见 `vcmiPreset.ts`）。
 *
 * 闭包解析（`solveDepClosure`）：逐条修复不收敛——引入一个根 = 整树实装，新树自带
 * 新依赖（洋葱效应）。该函数从当前实装集做 BFS 递归，一次性算清「引入哪些根 /
 * 装哪些包 / 哪些无解」，替代多轮手工修复。语言识别（`detectTranslationLang`）
 * 依据 VCMI 引擎语义：翻译模组 baseLanguage ≠ 当前语言时启动即被 BROKEN 剔除
 * （`ModDependenciesResolver::isResolved`），故非领域语言的翻译包默认不装。
 *
 * 「忽略」（`realm.json mods.depIgnores`）为纯 UI 静音 + 落盘记录：VCMI 引擎本身
 * 容忍依赖缺失（BROKEN 剔除、不阻断启动，`ModManager::updatePreset` 自愈），
 * 无需任何激活拦截。
 */
import type { InstalledMod, ModRepoEntry, RealmInstalledMod } from "./tauri";
import { RESERVED_MOD_IDS } from "./modScope";

/** 缺失依赖的可修复来源（四档模型的 UI 三态） */
export type DepSource = "installed" | "repo" | "unavailable";

/** 受某缺失依赖影响的已实装模组 */
export interface DepAffected {
  id: string;
  name: string;
  /** 启用了该模组的预设名（空数组 = 未在任何预设启用 → 潜在危害） */
  enabledIn: string[];
}

/** 单条缺失依赖 */
export interface DepDefect {
  /** 依赖 id（声明原样；补入激活集按此粒度——子模组 id 会自动带出父根） */
  dep: string;
  /** 依赖根 id（小写；「引入 / 安装」动作的目标粒度） */
  depRoot: string;
  source: DepSource;
  /** source === "repo" 时的仓库条目（展示名称 / 下载体积） */
  entry?: ModRepoEntry;
  affected: DepAffected[];
  severity: "active" | "latent";
}

/** 按根模组聚合的缺陷（「已实装」行按钮 / 处理弹窗的数据单元） */
export interface RootDepDefect {
  rootId: string;
  rootName: string;
  defects: DepDefect[];
  severity: "active" | "latent";
  /** 可快速处理（引入 / 安装）的依赖数 */
  fixable: number;
  /** 缺陷总数 */
  total: number;
}

export interface DepSolveResult {
  /** key = 根模组 id 小写（行按钮 / 弹窗按根 id 查找） */
  byRoot: Map<string, RootDepDefect>;
  /** 依赖未满足的模组 id（小写）：预设页树内角标 */
  defectModIds: Set<string>;
  /** 有依赖缺陷的根模组数（预设页操作栏徽章 / 弹层清单） */
  defectRootCount: number;
}

/** 第一段（点号前）：VCMI 模组 id 的根粒度。 */
const rootSeg = (id: string) => (id.includes(".") ? id.split(".")[0] : id);

/** 依赖满足判定（与 ModBrowser 守卫同口径）：保留 id → 精确命中 → 根容错（zip 内子模组）。 */
const satisfiedWith = (has: (lowerId: string) => boolean, dep: string): boolean => {
  const d = dep.toLowerCase();
  if (RESERVED_MOD_IDS.has(d)) return true;
  if (has(d)) return true;
  const r = rootSeg(d);
  return r !== d && has(r);
};

/** 缺失依赖分类：可引入（全局已装）/ 可安装（仓库可得）/ 不可得。 */
const classifyDep = (
  dep: string,
  hasGlobal: (lowerId: string) => boolean,
  repoLower: Map<string, ModRepoEntry>,
): { source: DepSource; entry?: ModRepoEntry } => {
  const d = dep.toLowerCase();
  const r = rootSeg(d);
  if (hasGlobal(r) || hasGlobal(d)) return { source: "installed" };
  const e = repoLower.get(r) ?? repoLower.get(d);
  if (e) return { source: "repo", entry: e };
  return { source: "unavailable" };
};

export function solveRealmDeps(input: {
  /** 领域已实装（`list_realm_mods` ∪ 内存引用的展示层，平铺含子模组） */
  installed: RealmInstalledMod[];
  /** 全局已安装模组（含子模组） */
  globalMods: InstalledMod[];
  /** 仓库索引（null = 本轮尚未检查成功） */
  repoIndex: ModRepoEntry[] | null;
  /** 预设表（每个预设的完整启用 id 集合，含子模组） */
  presets: Record<string, string[]>;
  /** 已忽略的依赖 id（小写，`realm.json mods.depIgnores`）：不再计入缺陷清单 */
  ignoredDeps?: Set<string>;
}): DepSolveResult {
  const { installed, globalMods, repoIndex, presets, ignoredDeps } = input;
  // 有效实装：metaOk=false（台账损坏）由「缺失 N」徽章单独表达，不重复报依赖缺陷
  const items = installed.filter((x) => x.metaOk);
  const byLower = new Map<string, RealmInstalledMod>();
  for (const x of items) byLower.set(x.id.toLowerCase(), x);
  const installedLower = new Set(byLower.keys());

  const satisfied = (dep: string): boolean => satisfiedWith((k) => installedLower.has(k), dep);

  const globalLower = new Set(globalMods.map((x) => x.id.toLowerCase()));
  const repoLower = new Map<string, ModRepoEntry>();
  for (const e of repoIndex ?? []) {
    const key = (e.id ?? "").toLowerCase();
    if (key) repoLower.set(key, e);
  }
  const classify = (dep: string): { source: DepSource; entry?: ModRepoEntry } =>
    classifyDep(dep, (k) => globalLower.has(k), repoLower);

  // 遍历实装条目（根 + 子模组），按「根 → 缺失依赖」两级分组
  interface Group {
    rootId: string;
    deps: Map<string, { dep: string; affected: RealmInstalledMod[] }>;
  }
  const groups = new Map<string, Group>();
  const affectedIds = new Set<string>();

  for (const x of items) {
    const deps = x.depends ?? [];
    if (deps.length === 0) continue;
    for (const dep of deps) {
      if (satisfied(dep)) continue;
      if (ignoredDeps?.has(dep.toLowerCase())) continue; // 已忽略：静音
      affectedIds.add(x.id.toLowerCase());
      const rootId = rootSeg(x.id);
      const gk = rootId.toLowerCase();
      let g = groups.get(gk);
      if (!g) {
        g = { rootId, deps: new Map() };
        groups.set(gk, g);
      }
      const dk = dep.toLowerCase();
      let it = g.deps.get(dk);
      if (!it) {
        it = { dep, affected: [] };
        g.deps.set(dk, it);
      }
      if (!it.affected.includes(x)) it.affected.push(x);
    }
  }

  /** 某模组启用于哪些预设（预设表为完整 id 集合，含子模组）。 */
  const enabledOf = (id: string): string[] => {
    const lc = id.toLowerCase();
    const out: string[] = [];
    for (const [preset, ids] of Object.entries(presets)) {
      if (ids.some((v) => v.toLowerCase() === lc)) out.push(preset);
    }
    return out;
  };

  const byRoot = new Map<string, RootDepDefect>();
  for (const [gk, g] of groups) {
    const rootItem = byLower.get(gk);
    const defects: DepDefect[] = [];
    for (const { dep, affected } of g.deps.values()) {
      const cls = classify(dep);
      const rows: DepAffected[] = affected.map((x) => ({
        id: x.id,
        name: x.name || x.id,
        enabledIn: enabledOf(x.id),
      }));
      defects.push({
        dep,
        depRoot: rootSeg(dep.toLowerCase()),
        source: cls.source,
        entry: cls.entry,
        affected: rows,
        severity: rows.some((r) => r.enabledIn.length > 0) ? "active" : "latent",
      });
    }
    // 实际危害排前：弹窗 / 徽章清单优先暴露正在失效的依赖
    defects.sort((a, b) => (a.severity === b.severity ? 0 : a.severity === "active" ? -1 : 1));
    byRoot.set(gk, {
      rootId: rootItem?.id ?? g.rootId,
      rootName: rootItem?.name || rootItem?.id || g.rootId,
      defects,
      severity: defects.some((d) => d.severity === "active") ? "active" : "latent",
      fixable: defects.filter((d) => d.source !== "unavailable").length,
      total: defects.length,
    });
  }

  return { byRoot, defectModIds: affectedIds, defectRootCount: byRoot.size };
}

/* ============================ 闭包解析（一键处置） ============================ */

/** 翻译包标记词元：命中其一（或官方 `vcmi-mod-ce-*` 族前缀）才做语言判定，压误报。 */
const TRANSLATION_MARKERS = new Set([
  "translation",
  "translated",
  "voice",
  "voiceover",
  "localization",
  "language",
  "lang",
]);
/** 语言词表（**词元级**匹配，避免 `haven` 命中 `en` 之类的子串误报）。 */
const LANG_TOKENS: Record<string, string[]> = {
  chinese: ["chinese", "chs", "cht", "zh", "cn", "mandarin"],
  english: ["english", "eng", "en"],
  german: ["german", "deutsch", "ger", "deu", "de"],
  french: ["french", "francais", "fra", "fr"],
  polish: ["polish", "polski", "pol"],
  russian: ["russian", "rus", "ru"],
  spanish: ["spanish", "espanol", "spa", "es"],
  czech: ["czech", "cestina", "ces", "cs"],
  ukrainian: ["ukrainian", "ukr", "uk", "ua"],
  italian: ["italian", "ita", "it"],
  portuguese: ["portuguese", "portugues", "por", "pt"],
  hungarian: ["hungarian", "magyar", "hun"],
  japanese: ["japanese", "jpn", "jp", "ja"],
  korean: ["korean", "kor", "ko"],
};

/**
 * 从 id / 展示名推断翻译包语言：返回语言（小写）| `"unknown"`（是翻译包但语言未识别）| `null`（非翻译包）。
 *
 * 依据 VCMI 引擎语义（`ModDependenciesResolver::isResolved`）：翻译模组
 * baseLanguage ≠ 当前首选语言时启动即被 BROKEN 剔除——非当前语言的翻译包
 * 默认不装是引擎固有行为。
 */
export function detectTranslationLang(id: string, name?: string | null): string | null {
  const idLower = id.toLowerCase();
  const tokens = new Set(
    `${idLower} ${(name ?? "").toLowerCase()}`.split(/[^a-z0-9]+/).filter(Boolean),
  );
  const isCePack = idLower.startsWith("vcmi-mod-ce-");
  const marked = isCePack || [...tokens].some((tk) => TRANSLATION_MARKERS.has(tk));
  if (!marked) return null;
  for (const [lang, words] of Object.entries(LANG_TOKENS)) {
    if (words.some((w) => tokens.has(w))) return lang;
  }
  return "unknown";
}

/** 闭包处置单：一项「将引用」的根模组。 */
export interface ClosureRefItem {
  /** 依赖根 id（小写匹配键） */
  rootKey: string;
  /** 磁盘写法根 id（引用动作目标） */
  rootId: string;
  rootName: string;
  /** 整树新增条目数（根 + 子模组，仅计尚未实装的部分） */
  itemCount: number;
  /** 直接解决的缺失依赖项数 */
  resolves: number;
  /** 受影响模组所在预设（需补入激活集；去重，空 = 仅潜在） */
  presetNames: string[];
  /** 被解决的依赖声明 id 原样（补激活集按精确粒度） */
  deps: string[];
  /** 迭代轮次（1 起） */
  round: number;
}

/** 闭包处置单：一项「需安装」（仓库可得）。 */
export interface ClosureInstallItem {
  dep: string;
  depRoot: string;
  name: string;
  entry?: ModRepoEntry;
  /** 翻译包语言（null = 非翻译包；"unknown" = 翻译包但语言未识别） */
  lang: string | null;
  presetNames: string[];
  /** 受影响模组显示名（tooltip） */
  affectedNames: string[];
}

/** 闭包处置单：一项无法解决（全局与仓库均无）。 */
export interface ClosureDeadItem {
  dep: string;
  depRoot: string;
  name: string;
  presetNames: string[];
  affectedNames: string[];
}

export interface DepClosureResult {
  /** 将引用的根（按发现轮次排序，去重） */
  refs: ClosureRefItem[];
  /** 需安装的项 */
  installs: ClosureInstallItem[];
  /** 无法解决的项 */
  deads: ClosureDeadItem[];
  /** 迭代轮数（收敛后 = 最后一轮号） */
  rounds: number;
  /** 引入将解决的缺失依赖项总数（去重） */
  resolvedCount: number;
  /** 执行「引用 + 安装」后实装集条目总数（含整树展开） */
  itemCountAfter: number;
}

/**
 * 依赖闭包解析（一键处置的计算核心）：从当前实装集出发 BFS 递归求解——
 * 每轮扫描缺失依赖，把「全局可引入」的根**整树**并入工作集后进入下一轮，
 * 直到无新可引入（收敛或只剩需安装 / 无解项）。一次算清多轮手工修复的完整代价。
 *
 * 与 `solveRealmDeps` 同口径（satisfied / classify / 危害分级），额外容忍：
 * ① 已忽略依赖（`ignoredDeps`）不参与；② 仓库条目仅在最终轮分类为「需安装」。
 */
export function solveDepClosure(input: {
  /** 领域已实装（`list_realm_mods` ∪ 内存引用的展示层，平铺含子模组） */
  installed: RealmInstalledMod[];
  /** 全局已安装模组（含子模组）：引入候选来源 */
  globalMods: InstalledMod[];
  /** 仓库索引（null = 本轮尚未检查成功） */
  repoIndex: ModRepoEntry[] | null;
  /** 预设表（每个预设的完整启用 id 集合，含子模组） */
  presets: Record<string, string[]>;
  /** 已忽略的依赖 id（小写）：不参与闭包，也不进终态清单 */
  ignoredDeps?: Set<string>;
}): DepClosureResult {
  const { installed, globalMods, repoIndex, presets, ignoredDeps } = input;

  // 全局根树索引：root 小写 → 该根条目列表（引入 = 整树并入工作集）
  const globalTrees = new Map<string, InstalledMod[]>();
  for (const g of globalMods) {
    const r = rootSeg(g.id.toLowerCase());
    let arr = globalTrees.get(r);
    if (!arr) {
      arr = [];
      globalTrees.set(r, arr);
    }
    arr.push(g);
  }
  /** 根 id 的磁盘写法：优先取全局树里「无点号」的根条目，缺失时回退声明 id 的根段。 */
  const canonRoot = (rootKey: string, depDecl: string): string =>
    globalTrees.get(rootKey)?.find((x) => !x.id.includes("."))?.id ?? rootSeg(depDecl);

  /** 工作集：逻辑实装集（id 小写 → 条目）。初始 = 领域已实装（metaOk）。 */
  interface WNode {
    id: string;
    name: string;
    depends: string[];
  }
  const working = new Map<string, WNode>();
  for (const x of installed) {
    if (!x.metaOk) continue;
    working.set(x.id.toLowerCase(), { id: x.id, name: x.name || x.id, depends: x.depends ?? [] });
  }

  const ignored = ignoredDeps ?? new Set<string>();
  const repoLower = new Map<string, ModRepoEntry>();
  for (const e of repoIndex ?? []) {
    const k = (e.id ?? "").toLowerCase();
    if (k) repoLower.set(k, e);
  }

  const satisfied = (dep: string) => satisfiedWith((k) => working.has(k), dep);
  /** 某模组启用于哪些预设（预设表为完整 id 集合，含子模组）。 */
  const enabledOf = (id: string): string[] => {
    const lc = id.toLowerCase();
    const out: string[] = [];
    for (const [preset, ids] of Object.entries(presets)) {
      if (ids.some((v) => v.toLowerCase() === lc)) out.push(preset);
    }
    return out;
  };

  const refs: ClosureRefItem[] = [];
  const refByKey = new Map<string, ClosureRefItem>();
  const refResolves = new Map<string, Set<string>>();
  const refDeps = new Map<string, Set<string>>();
  const refPresets = new Map<string, Set<string>>();
  let lastMissing = new Map<string, { dep: string; affected: Set<string> }>();

  const MAX_ROUNDS = 20; // 防御性上限：闭包单调递增必然终止，正常 5~6 轮收敛
  let rounds = 0;
  for (let round = 1; round <= MAX_ROUNDS; round++) {
    rounds = round;
    // ① 扫描工作集全部缺失依赖（按 dep 去重，记录受影响条目）
    const missing = new Map<string, { dep: string; affected: Set<string> }>();
    for (const n of working.values()) {
      for (const dep of n.depends) {
        if (satisfied(dep)) continue;
        const dk = dep.toLowerCase();
        if (ignored.has(dk)) continue;
        let it = missing.get(dk);
        if (!it) {
          it = { dep, affected: new Set() };
          missing.set(dk, it);
        }
        it.affected.add(n.id);
      }
    }
    lastMissing = missing;
    if (missing.size === 0) break; // 收敛（无缺失）
    // ② 执行「引入」：可引入根的整树并入工作集（下一轮重扫后继续收敛）
    let introduced = 0;
    for (const [dk, it] of missing) {
      const r = rootSeg(dk);
      const rootKey = globalTrees.has(r) ? r : globalTrees.has(dk) ? dk : null;
      if (!rootKey) continue; // 不可引入（repo / 无解留在终态扫描）
      const tree = globalTrees.get(rootKey)!;
      let item = refByKey.get(rootKey);
      if (!item) {
        item = {
          rootKey,
          rootId: canonRoot(rootKey, it.dep),
          rootName:
            tree.find((x) => !x.id.includes("."))?.name || canonRoot(rootKey, it.dep),
          itemCount: 0,
          resolves: 0,
          presetNames: [],
          deps: [],
          round,
        };
        refByKey.set(rootKey, item);
        refs.push(item);
        refResolves.set(rootKey, new Set());
        refDeps.set(rootKey, new Set());
        refPresets.set(rootKey, new Set());
      }
      refResolves.get(rootKey)!.add(dk);
      refDeps.get(rootKey)!.add(it.dep);
      for (const aid of it.affected) for (const p of enabledOf(aid)) refPresets.get(rootKey)!.add(p);
      for (const g of tree) {
        const gl = g.id.toLowerCase();
        if (working.has(gl)) continue;
        working.set(gl, { id: g.id, name: g.name || g.id, depends: g.depends ?? [] });
        item.itemCount++;
      }
      introduced++;
    }
    if (introduced === 0) break; // 无可引入 → 剩余即终态
  }

  // ③ 落定 refs 统计（resolves / deps / presetNames 在引入过程中累计）
  for (const item of refs) {
    item.resolves = refResolves.get(item.rootKey)?.size ?? 0;
    item.deps = [...(refDeps.get(item.rootKey) ?? [])];
    item.presetNames = [...(refPresets.get(item.rootKey) ?? [])];
  }

  // ④ 终态剩余：需安装（仓库可得）/ 无法解决（无源）
  const installs: ClosureInstallItem[] = [];
  const deads: ClosureDeadItem[] = [];
  for (const [dk, it] of lastMissing) {
    const r = rootSeg(dk);
    const affectedNames = [...it.affected].map((id) => working.get(id.toLowerCase())?.name ?? id);
    const presetNames = [...new Set([...it.affected].flatMap((id) => enabledOf(id)))];
    const entry = repoLower.get(r) ?? repoLower.get(dk);
    if (entry) {
      installs.push({
        dep: it.dep,
        depRoot: r,
        name: entry.name || it.dep,
        entry,
        lang: detectTranslationLang(entry.id ?? it.dep, entry.name),
        presetNames,
        affectedNames,
      });
    } else {
      deads.push({ dep: it.dep, depRoot: r, name: it.dep, presetNames, affectedNames });
    }
  }
  // 实际危害排前（需补激活集的先暴露），组内按名称稳定排序
  const bySeverity = (a: { presetNames: string[] }, b: { presetNames: string[] }) =>
    Number(b.presetNames.length > 0) - Number(a.presetNames.length > 0);
  installs.sort(bySeverity);
  deads.sort(bySeverity);

  return {
    refs,
    installs,
    deads,
    rounds,
    resolvedCount: refs.reduce((acc, r) => acc + r.resolves, 0),
    itemCountAfter: working.size,
  };
}
