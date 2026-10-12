import { useDeferredValue, useEffect, useMemo, useState, useSyncExternalStore, type ReactNode } from "react";
import {
  ChevronDown,
  ChevronUp,
  FoldVertical,
  LayoutList,
  ListTree,
  UnfoldVertical,
} from "lucide-react";
import { useI18n } from "@/i18n";
import {
  getTownPreviewEnabled,
  loadTownPreview,
  setTownPreviewEnabled,
  subscribeTownPreview,
} from "@/lib/townPreview";
import { RESERVED_MOD_IDS } from "@/lib/modScope";
import type { ModInfo } from "@/lib/modScanner";
import { localizedName, localizedDescription } from "@/lib/modScanner";
import { richToPlainText } from "@/lib/html";
import { TYPE_ORDER, TYPE_COLORS } from "@/lib/minimapBase";
import { ModListItem } from "./ModCard";
import { DetailDrawer } from "./DetailDrawer";
import { Input } from "@/shared/ui/input";
import { cn } from "@/lib/utils";
import { compatRangeText, isModCompatible } from "@/lib/modCompat";
import {
  getEngineVersionSnapshot,
  loadEngineVersion,
  subscribeEngineVersion,
} from "@/lib/engineVersion";
import { vcmiLangName } from "@/lib/vcmiLangs";
import {
  getVcmiLanguageSnapshot,
  loadVcmiLanguage,
  modBaseLanguage,
  subscribeVcmiLanguage,
} from "@/lib/vcmiLanguage";

type FilterMode = "unjoined" | "partial" | "all";

/**
 * 当前语言下**不可用**的翻译模组：基准语言 ≠ 游戏语言。
 *
 * VCMI 自己会在加载时把这类模组判 BROKEN 并从预设中剔除（同 `ModCard` 的 `langTip` 口径，
 * 必须用 `rawType` 原文而非推断的 `type`），故界面**直接从列表过滤掉**——与运行时行为一致，
 * 也避免"切到英文后仍看到中文语言包被勾选"的困惑。
 *
 * ⚠️ **只影响显示**：`selected` / 预设数据一律不动，隐藏项原有的启用状态在保存预设时原样保留。
 */
function langUnusable(m: ModInfo, vcmiLang: string | null | undefined): boolean {
  return (
    m.rawType === "Translation" && !!vcmiLang && modBaseLanguage(m.language) !== vcmiLang
  );
}

/**
 * 子模组容器：整段缩进（ml-5 20px + 导轨 1px + pl-2 8px ≈ 29px）+ **一条**连续导轨。
 * 父子对齐（方案 A）的两个支点之一（另一个是 `ModCard` 里的定宽前置槽 `w-4`）：
 * - `ml-5` 让导轨正好落在父行 chevron（x 12~28）的中线 x=20 上，视觉上"挂"在展开按钮下；
 * - 缩进与导轨挂在**容器**上、只画一次，不再逐行 `ml-6 border-l-4`（竖线被行间距打断）；
 * - 左栏列表与右栏树共用同一常量，避免「同一概念两套视觉」。
 */
const CHILD_CONTAINER = "ml-5 flex flex-col gap-1.5 border-l border-slate-700/40 pl-2";

export interface ModBrowserProps {
  /** 扁平模组列表（含子模组；isSubmod/parentId 已就绪）。内部会过滤掉 VCMI 内置保留模组。 */
  mods: ModInfo[];
  /** 受控选中集合（不含基线模组）。 */
  selected: Set<string>;
  /** 基线模组（引擎核心，永远启用且不可取消）。 */
  baseMods: string[];
  /** 选中变化回调：内部已完成父子 / 依赖联动，回传最终集合。 */
  onChange: (next: Set<string>) => void;
  /** 工具栏右侧额外插槽（如领域「一键借用全部」）。 */
  toolbarExtra?: ReactNode;
  leftTitle?: string;
  leftSub?: string;
  emptyLeft?: string;
  className?: string;
  /** 全局强制模组（锁定、不可取消）。命中后该模组不显示开关且展示「强制」角标。 */
  lockedMods?: Set<string>;
  /** 初始化加载中：调用方的权威选中集合尚未解析完成。
   *  此时左栏仍照常渲染（可浏览、可看详情），但**不提供加入/移除按钮**，右栏显示加载提示 ——
   *  否则用户在半初始化态的改动会被随后的预设解析覆盖。默认 false（领域模组页等不受影响）。 */
  loading?: boolean;
  /** 必需依赖守卫触发回调：禁用某模组会破坏其它仍启用模组的 `depends` 时被调用（带本地化文案）。
   *  由调用方决定如何提示（如 toast / 内联横幅）；返回后 toggle 不生效。 */
  onDependencyBlocked?: (message: string) => void;
  /** 依赖缺陷模组 id（小写）集合：命中行显示「依赖缺失」角标（依赖处置体系，见 `lib/modDeps.ts`）。 */
  defectIds?: Set<string>;
  /** P6（T3 定案）：预设页零仓库信息——可更新 chip/卡片升级角标/抽屉安装更新入口已全部迁至模组管理页 */
}

/**
 * 通用模组浏览/选择组件（受控）。
 *
 * 把 `Mods` 页面的内容区（左树 + 搜索/类型/过滤工具栏 + 右栏分组列表 + 详情抽屉）
 * 与选中逻辑（父子联动、依赖自动选中、自动展开含子模组父项、冲突检测）整体抽出来，
 * 供 `ModView`（预设模式）与「领域模组」tab（固定 __realm__ 预设）共用，单一数据源。
 *
 * 本组件不感知任何预设管理（PresetBar / 读写 modSettings.json / 借用整合），
 * 仅通过 `selected` / `onChange` 与外部状态对接。
 */
export function ModBrowser({
  mods,
  selected,
  baseMods,
  onChange,
  toolbarExtra,
  leftTitle,
  leftSub,
  emptyLeft,
  className,
  lockedMods,
  loading = false,
  onDependencyBlocked,
  defectIds,
}: ModBrowserProps) {
  const { t, lang } = useI18n();
  const [search, setSearch] = useState("");
  /**
   * 过滤用**延迟值**：输入框自身始终即时响应，列表过滤（每行名称 / 描述 + 正则清洗）
   * 由 React 在后台按最新输入重算 —— 大列表下输入搜索词不掉帧（并发特性）。
   */
  const deferredSearch = useDeferredValue(search);
  const [typeFilter, setTypeFilter] = useState("all");
  const [filter, setFilter] = useState<FilterMode>("unjoined");
  const [detailMod, setDetailMod] = useState<ModInfo | null>(null);
  const [expandedParents, setExpandedParents] = useState<Set<string>>(new Set());
  // 城镇标签页中「含子模组」分组的折叠状态（默认全部展开，可手动收起）
  const [collapsedGroups, setCollapsedGroups] = useState<Set<string>>(new Set());
  // 「预览城镇背景」开关：进程内缓存 + launcher.json 持久化（默认关闭）。
  // 领域「领域模组」页复用本组件，与 Mods 页共享同一份开关状态。
  const townPreview = useSyncExternalStore(subscribeTownPreview, getTownPreviewEnabled);
  useEffect(() => {
    void loadTownPreview();
  }, []);
  // 引擎版本（模块级单次加载）：兼容性角标与启用守卫都依赖它；加载完成会通知订阅者重渲染。
  const engineVersion = useSyncExternalStore(subscribeEngineVersion, getEngineVersionSnapshot);
  useEffect(() => {
    void loadEngineVersion();
  }, []);
  // 游戏语言（`settings.json` 的 `general.language`）：翻译模组语言不符守卫用（同款单次加载）。
  const vcmiLanguage = useSyncExternalStore(subscribeVcmiLanguage, getVcmiLanguageSnapshot);
  useEffect(() => {
    void loadVcmiLanguage();
  }, []);

  const flat = useMemo(() => mods.filter((m) => !RESERVED_MOD_IDS.has(m.id)), [mods]);
  const flatMap = useMemo(() => new Map(flat.map((m) => [m.id, m])), [flat]);
  const installedIds = useMemo(
    () => new Set([...flat.map((m) => m.id), ...RESERVED_MOD_IDS]),
    [flat],
  );
  // VCMI 模组 id 大小写不敏感（如磁盘目录 `mainMenu` 与冲突声明 `hota.mainmenu`）。
  // 建小写索引，供依赖/冲突匹配容错（见 `isActiveMod` / `conflictsActive`）。
  const lowerFlat = useMemo(() => {
    const m = new Map<string, ModInfo>();
    flat.forEach((f) => m.set(f.id.toLowerCase(), f));
    return m;
  }, [flat]);
  const lowerInstalled = useMemo(
    () => new Set([...flat.map((m) => m.id.toLowerCase()), ...RESERVED_MOD_IDS]),
    [flat],
  );
  // 是否为内置保留 scope（`vcmi` / `core`）：大小写不敏感（mod.json 声明可能与保留 id 大小写不一致）。
  const isReserved = (id: string) =>
    [...RESERVED_MOD_IDS].some((r) => r.toLowerCase() === id.toLowerCase());

  const parentMap = useMemo(() => {
    const map = new Map<string, ModInfo>();
    flat.forEach((m) => {
      if (!m.isSubmod) map.set(m.id, m);
    });
    return map;
  }, [flat]);

  const childrenMap = useMemo(() => {
    const map = new Map<string, ModInfo[]>();
    flat.forEach((m) => {
      if (m.isSubmod) {
        const list = map.get(m.parentId) ?? [];
        list.push(m);
        map.set(m.parentId, list);
      }
    });
    map.forEach((list) =>
      list.sort((a, b) => localizedName(a, lang).localeCompare(localizedName(b, lang))),
    );
    return map;
  }, [flat, lang]);

  const isSelected = (id: string) => baseMods.includes(id) || selected.has(id);
  // 取模组 id 的「根模组」前缀（parent.child → parent）；顶层模组原样返回。
  // 用于识别「随父模组压缩在 content.zip 内的子模组」：这类子模组不在目录扫描结果中，
  // 但父模组已安装时 VCMI 会在加载父模组时一并解析（见下方依赖/冲突守卫的容错）。
  const rootId = (id: string) => (id.includes(".") ? id.slice(0, id.indexOf(".")) : id);

  /** 依赖缺陷角标文案（命中缺陷表；hover 说明）。 */
  const depTipOf = (m: ModInfo): string | undefined =>
    defectIds?.has(m.id.toLowerCase()) ? t.mods.depTagTip : undefined;

  const toggle = (id: string) => {
    if (baseMods.includes(id)) return; // 基线不可取消
    if (lockedMods?.has(id)) return; // 强制模组不可取消
    const mod = flatMap.get(id);
    const next = new Set(selected);
    if (next.has(id)) {
      // 取消父模组时，同时取消其下所有子模组（含深层）
      const removeSet = new Set<string>([id]);
      const stack = [id];
      while (stack.length) {
        const cur = stack.pop()!;
        childrenMap.get(cur)?.forEach((c) => {
          if (!removeSet.has(c.id)) {
            removeSet.add(c.id);
            stack.push(c.id);
          }
        });
      }
      // 必需依赖守卫（对齐 VCMI ModManager：依赖断裂会被拒绝加载）：
      // 若仍有其它启用模组把待移除的模组（或其子模组）列为 required `depends`，则阻止关闭。
      let blocker: { by: ModInfo; dep: string } | null = null;
      // 依赖 id 比较大小写不敏感（同启用路径）：声明常与扫描 id 大小写不一致
      // （实测 `New Pavillion` 声明 `new pavilion.dunes-terrain`，扫描 id 为 `New Pavilion.…`）
      const removeLower = new Set<string>([...removeSet].map((x) => x.toLowerCase()));
      for (const r of removeSet) {
        for (const y of next) {
          if (removeSet.has(y)) continue;
          const ym = flatMap.get(y);
          if (ym && ym.depends.some((d) => removeLower.has(d.toLowerCase()))) {
            blocker = { by: ym, dep: r };
            break;
          }
        }
        if (blocker) break;
      }
      if (blocker) {
        const depName = localizedName(flatMap.get(blocker.dep) ?? toMissingMod(blocker.dep, t), lang);
        const byName = localizedName(blocker.by, lang);
        onDependencyBlocked?.(
          t.mods.depBlocked.replace("{name}", depName).replace("{by}", byName),
        );
        return;
      }
      removeSet.forEach((r) => next.delete(r));
    } else {
      // 激活：计算「本次要带出的全部模组」闭包（对齐 VCMI ModManager::tryEnableMods）：
      // 递归拉齐 required `depends`（含依赖的依赖），并逐级带出父模组与默认子项。
      // 同时校验：① 必需依赖须存在（缺失则拒绝启用）；② 与已启用/同批模组无 `conflicts` 互斥。
      const toAdd = new Set<string>([id]);
      const stack: string[] = [id];
      const missingDeps = new Set<string>();
      /** 模组 id → 其缺失依赖（分层处置：区分「目标自身 / 根模组」与「可跳过的子模组」） */
      const missingOf = new Map<string, Set<string>>();
      // 判定「某模组是否处于激活态」：自身已启用/本批新增，或它是压缩在 zip 内的子模组
      // （其根模组处于激活态）——与依赖守卫的 content.zip 容错口径一致。id 比较大小写不敏感。
      const isActiveMod = (cid: string): boolean => {
        if (isSelected(cid) || toAdd.has(cid)) return true;
        const lm = lowerFlat.get(cid.toLowerCase());
        if (lm && (isSelected(lm.id) || toAdd.has(lm.id))) return true;
        const rid = rootId(cid);
        if (rid !== cid) {
          if (isSelected(rid) || toAdd.has(rid)) return true;
          const lrm = lowerFlat.get(rid.toLowerCase());
          if (lrm && (isSelected(lrm.id) || toAdd.has(lrm.id))) return true;
        }
        return false;
      };
      // 判定「模组 cid 是否与某个已激活/本批新增模组互斥」：
      // 正向 cid 把某激活模组列为 conflicts（含 zip 内子模组目标，经根模组判定）；
      // 反向某激活模组把 cid（精确 id，大小写不敏感）列为 conflicts——不要用根模组模糊匹配，
      // 否则父模组会因「同根子模组冲突」被误伤，违背 VCMI「仅排除冲突子模组、父模组仍可启用」的语义。
      const conflictsActive = (cid: string): boolean => {
        const m = flatMap.get(cid);
        if (m && m.conflicts.some((c) => isActiveMod(c))) return true;
        const lc = cid.toLowerCase();
        for (const a of [...selected, ...toAdd]) {
          const am = flatMap.get(a);
          if (am && am.conflicts.some((c) => c.toLowerCase() === lc)) return true;
        }
        return false;
      };
      while (stack.length) {
        const cur = stack.pop()!;
        const cm = flatMap.get(cur);
        if (!cm) continue;
        if (cm.isSubmod) {
          // 子模组依附父模组加载，必须带出父模组
          const pid = cm.parentId;
          if (!isSelected(pid) && !toAdd.has(pid)) {
            toAdd.add(pid);
            stack.push(pid);
          }
        } else {
          // 顶层：按默认态补齐子模组（含兄弟）
          childrenMap.get(cur)?.forEach((c) => {
            if (c.keepDisabled) return;
            // 排它冲突子模组：与已强制/启用模组互斥时仅该子模组不启用，
            // 不影响父模组与其它兄弟子模组（对齐 VCMI 行为）。
            if (conflictsActive(c.id)) return;
            if (!toAdd.has(c.id)) {
              toAdd.add(c.id);
              stack.push(c.id);
            }
          });
        }
        // 必需依赖（递归）：存在则一并启用，缺失则记下以便拒绝（与 VCMI 一致）。
        // ⚠️ id 匹配必须大小写不敏感（VCMI 语义，同 isActiveMod / conflictsActive）：
        // 实测 mod.json 里的依赖声明常与扫描 id 大小写不一致 ——
        // `New Pavillion` 声明 `new pavilion.dunes-terrain`（实际 `New Pavilion.dunes-terrain`）、
        // `tartarus` 声明 `tidesofwar`（实际 `tidesOfWar`）与 `newtown-terrains.icelandterrain`
        // （实际 `...icelandTerrain`）。精确匹配会把**已安装**依赖误判为缺失 → 硬拦截启用。
        for (const d of cm.depends) {
          const dm = flatMap.get(d) ?? lowerFlat.get(d.toLowerCase());
          if (dm) {
            // 真实可勾选项（顶层或已展开到磁盘的子模组）：一并启用（用扫描到的规范 id）；
            // 但若它是与已启用模组互斥的子模组，仅排除该子模组（不阻断其父/依赖链）。
            if (dm.isSubmod && conflictsActive(dm.id)) continue;
            if (!isSelected(dm.id) && !toAdd.has(dm.id)) {
              toAdd.add(dm.id);
              stack.push(dm.id);
            }
          } else if (isReserved(d)) {
            // 内置保留 scope（`vcmi` / `core`）：由引擎自带、不在目录扫描结果里，
            // 但 VCMI 预设的启用列表确实含它们（实测官方预设 `mods: [vcmi, core]`）→
            // 视为恒已满足。⚠️ 不能当成"未安装依赖"，否则会误报「缺少必需依赖 vcmi」。
            if (!isSelected(d) && !toAdd.has(d)) toAdd.add(d);
          } else {
            // 依赖项不在目录扫描结果中：若它是「随父模组压缩在 content.zip 内的子模组」
            // （父模组 id 已安装），VCMI 会在加载父模组时一并解析 → 视为已满足、
            // 不报缺失、也不入 selected（由父模组整体启用带动）。其余才是真正缺失的依赖。
            const rid = rootId(d);
            if (!(rid !== d && lowerInstalled.has(rid.toLowerCase()))) {
              missingDeps.add(d);
              // 记「哪个模组缺哪些依赖」：分层处置时按来源判定可否「跳过子模组」
              const s = missingOf.get(cur) ?? new Set<string>();
              s.add(d);
              missingOf.set(cur, s);
            }
          }
        }
      }
      // 依赖缺失守卫（分层处置，依赖处置体系）：
      // ① 缺失源自「本批带出的子模组」→ 剔除问题子模组（级联剔除依赖它 / 依附它的子模组），
      //    主干照常启用；仅 toast 聚合告知（可在「模组管理-已实装」批量处置依赖）。
      // ② 缺失源自目标自身或根模组 → 无法通过跳过解决，硬拦截（原行为保留）。
      if (missingDeps.size > 0) {
        const removable = new Set<string>();
        let hardBlocked = false;
        for (const mid of missingOf.keys()) {
          const mm = flatMap.get(mid);
          if (mid === id || !mm?.isSubmod) {
            hardBlocked = true; // 目标自身或根级缺依赖：整体拒绝
            break;
          }
          removable.add(mid);
        }
        // 级联：依赖「将被剔除者」或依附其父的子模组同样剔除；命中根模组则整体拒绝
        if (!hardBlocked) {
          let grew = true;
          while (grew && !hardBlocked) {
            grew = false;
            const remLower = new Set<string>();
            for (const r of removable) remLower.add(r.toLowerCase());
            for (const t2 of toAdd) {
              if (removable.has(t2)) continue;
              const tm = flatMap.get(t2);
              if (!tm) continue;
              const hit =
                tm.depends.some((d) => remLower.has(d.toLowerCase())) ||
                (tm.isSubmod && !!tm.parentId && remLower.has(tm.parentId.toLowerCase()));
              if (!hit) continue;
              if (t2 === id || !tm.isSubmod) {
                hardBlocked = true;
                break;
              }
              removable.add(t2);
              grew = true;
            }
          }
        }
        if (hardBlocked) {
          const name = localizedName(mod ?? toMissingMod(id, t), lang);
          const depNames = [...missingDeps]
            .map((d) => localizedName(flatMap.get(d) ?? toMissingMod(d, t), lang))
            .join("、");
          onDependencyBlocked?.(
            t.mods.depMissing.replace("{name}", name).replace("{dep}", depNames),
          );
          return;
        }
        // 跳过问题子模组：从本批剔除 + 聚合告知（继续后续守卫）
        const skipped: string[] = [];
        for (const r of removable) {
          toAdd.delete(r);
          next.delete(r);
          skipped.push(localizedName(flatMap.get(r) ?? toMissingMod(r, t), lang));
        }
        onDependencyBlocked?.(t.mods.depSkipSubs.replace("{list}", skipped.join("、")));
      }
      // 冲突守卫（对齐 VCMI：互斥模组同时启用会被拒绝加载）
      // 注意：冲突子模组已在上方展开时单独排除（父模组仍可启用）。本守卫仅拦截
      // 顶层/依赖级的「硬性互斥」——即被启用模组本身与已激活模组精确互斥。
      let conflictBy: { by: ModInfo; cand: string } | null = null;
      for (const cand of toAdd) {
        const cm = flatMap.get(cand);
        if (!cm) continue;
        // 正向：本批次新增模组把 cand 列为 conflicts（含同批新增的其它模组、zip 内子模组）
        for (const c of cm.conflicts) {
          if (c === cand) continue;
          if (isActiveMod(c)) {
            conflictBy = { by: flatMap.get(c) ?? flatMap.get(rootId(c)) ?? toMissingMod(c, t), cand };
            break;
          }
        }
        if (conflictBy) break;
        // 反向：已启用模组把 cand 列为 conflicts（精确 id、大小写不敏感匹配；
        // 不按根模组模糊，否则父模组会因「同根子模组冲突」被误伤，与上面的子模组排除逻辑矛盾）
        for (const y of next) {
          if (toAdd.has(y)) continue;
          const ym = flatMap.get(y);
          if (ym && ym.conflicts.some((c) => c.toLowerCase() === cand.toLowerCase())) {
            conflictBy = { by: ym, cand };
            break;
          }
        }
        if (conflictBy) break;
      }
      if (conflictBy) {
        const candName = localizedName(flatMap.get(conflictBy.cand) ?? toMissingMod(conflictBy.cand, t), lang);
        const byName = localizedName(conflictBy.by, lang);
        onDependencyBlocked?.(
          t.mods.conflictBlocked.replace("{name}", candName).replace("{by}", byName),
        );
        return;
      }
      // 引擎版本守卫（对齐 VCMI `ModDependenciesResolver::tryAddMods` 的 BROKEN 判定之一）：
      // `compatibility.min/max` 不满足的模组会被引擎判为 BROKEN 并从激活预设中剔除 ——
      // 与其让用户启用了却在游戏里"消失"，不如在此拦下并说明原因。
      // 版本未知（尚未加载 / 读不到）时**不拦**，避免误报。
      if (engineVersion) {
        for (const cand of toAdd) {
          const cm = flatMap.get(cand);
          if (!cm) continue;
          if (!isModCompatible(engineVersion, cm.compatibility)) {
            onDependencyBlocked?.(
              t.mods.incompatBlocked
                .replace("{name}", localizedName(cm, lang))
                .replace("{range}", compatRangeText(cm.compatibility) ?? "?")
                .replace("{ver}", engineVersion),
            );
            return;
          }
        }
      }
      // 翻译模组语言守卫（同一条 BROKEN 规则的另一个条件；官方 launcher 的 `canEnableMod` 也据此拒绝启用）：
      // `modType: "Translation"` 且基准语言 ≠ 当前游戏语言 → 引擎会忽略它并从激活预设中移除。
      // ⚠️ 用 `rawType`（mod.json 原文）而非推断出的 `type`，否则会被误判成翻译模组的模组全面误伤。
      if (vcmiLanguage) {
        for (const cand of toAdd) {
          const cm = flatMap.get(cand);
          if (!cm || cm.rawType !== "Translation") continue;
          const base = modBaseLanguage(cm.language);
          if (base !== vcmiLanguage) {
            onDependencyBlocked?.(
              t.mods.langBlocked
                .replace("{name}", localizedName(cm, lang))
                .replace("{lang}", vcmiLangName(base))
                .replace("{cur}", vcmiLangName(vcmiLanguage)),
            );
            return;
          }
        }
      }
      toAdd.forEach((x) => next.add(x));
    }
    onChange(next);
  };

  const openDetail = (id: string) => {
    setDetailMod(flatMap.get(id) ?? toMissingMod(id, t));
  };

  const conflictSet = useMemo(() => {
    const out = new Set<string>();
    selected.forEach((id) => {
      const m = flatMap.get(id);
      m?.conflicts.forEach((c) => {
        if (isSelected(c)) {
          out.add(id);
          out.add(c);
        }
      });
    });
    return out;
  }, [selected, flatMap, baseMods]);

  const q = deferredSearch.trim().toLowerCase();

  const matchesQuery = (m: ModInfo) => {
    if (!q) return true;
    return (
      localizedName(m, lang).toLowerCase().includes(q) ||
      (richToPlainText(localizedDescription(m, lang)) || "").toLowerCase().includes(q)
    );
  };

  const parentState = (parent: ModInfo, children: ModInfo[]) => {
    const pSel = isSelected(parent.id);
    const allChildrenSel = children.length > 0 && children.every((c) => isSelected(c.id));
    const anyChildSel = children.some((c) => isSelected(c.id));
    if (!pSel && !anyChildSel) return "none";
    if (pSel && (children.length === 0 || allChildrenSel)) return "all";
    return "partial";
  };

  const leftTree = useMemo(() => {
    const rows: { parent: ModInfo; children: ModInfo[] }[] = [];
    const matchesTypeFilter = (m: ModInfo) => {
      if (typeFilter === "all") return true;
      if (typeFilter !== "Town") return m.type === typeFilter;
      return m.type === "Town" && (m.factions?.length ?? 0) > 0;
    };
    parentMap.forEach((parent) => {
      const children = childrenMap.get(parent.id) ?? [];
      const pMatchesType = matchesTypeFilter(parent);
      const visibleChildren = children.filter(
        (c) => !langUnusable(c, vcmiLanguage) && matchesTypeFilter(c) && matchesQuery(c),
      );
      const pMatchesQuery = matchesQuery(parent);
      const keepParent = !langUnusable(parent, vcmiLanguage) && pMatchesType && pMatchesQuery;
      const keepByChild = visibleChildren.length > 0;
      if (!keepParent && !keepByChild) return;

      const state = parentState(parent, children);
      // unjoined：默认只列「全未选」的父模组；但若父模组还有未激活的子模组（partial），
      // 需保留父与剩余子模组，方便用户继续勾选（对齐 VCMI 父→子联动后仍有未选项的场景）。
      if (filter === "unjoined") {
        if (state === "all") return;
        if (state === "partial") {
          rows.push({ parent, children: visibleChildren.filter((c) => !isSelected(c.id)) });
          return;
        }
      } else if (filter === "partial" && state !== "partial") {
        return;
      }

      rows.push({ parent, children: visibleChildren });
    });
    rows.sort((a, b) =>
      localizedName(a.parent, lang).localeCompare(localizedName(b.parent, lang)),
    );
    return rows;
  }, [parentMap, childrenMap, typeFilter, q, filter, selected, lang, vcmiLanguage]);

  // 城镇标签页：拆分为「独立城镇」与「城镇模组组（含子模组，固定排在最后）」。
  // ⚠️ `standalone` 只接受**真城镇**（type=Town 且 factions 非空，口径同 L493 / ModCard 的 isTown）。
  // 非城镇父级（如扩展包 hota）可能因「子模组已全部选中」被 unjoined 过滤裁成空 children
  // （见 leftTree L512），若无条件落到 standalone，就会以紧凑行（非城镇不会渲染大图卡）
  // 混进城镇大图网格 —— 即截图中「深渊号角」被挤竖的「扩展」徽标那张怪卡。
  const townLayout = useMemo(() => {
    const standalone: ModInfo[] = [];
    const grouped: { parent: ModInfo; children: ModInfo[] }[] = [];
    leftTree.forEach(({ parent, children }) => {
      if (children.length > 0) {
        grouped.push({ parent, children });
      } else if (parent.type === "Town" && (parent.factions?.length ?? 0) > 0) {
        standalone.push(parent);
      }
    });
    return { standalone, grouped };
  }, [leftTree]);

  // 空状态判定：城镇页以 townLayout 为准 —— leftTree 可能残留被 townLayout 丢弃的非城镇父级
  // （见上方 standalone 过滤），若仍看 leftTree.length 会渲染出空白左栏而非「无结果」提示。
  const listEmpty =
    typeFilter === "Town"
      ? townLayout.standalone.length === 0 && townLayout.grouped.length === 0
      : leftTree.length === 0;

  /* ---------------- 仓库渲染支路已移除（P6：全部迁至模组管理页） ---------------- */

  // 标签栏顺序：按 TYPE_ORDER 固定排列（城镇、生物、英雄…），未列出的类型排到最后。
  // 不能直接用 `new Set` 的插入顺序——那取决于扫描结果，会随模组加载次序漂移。
  const types = useMemo(() => {
    // 已过滤当前语言不可用的翻译模组：避免某类型仅由语言包贡献时留下一个点进去空列表的死标签
    const uniq = [...new Set(flat.filter((m) => !langUnusable(m, vcmiLanguage)).map((m) => m.type))];
    uniq.sort((a, b) => {
      const ia = TYPE_ORDER.indexOf(a);
      const ib = TYPE_ORDER.indexOf(b);
      if (ia === -1 && ib === -1) return a.localeCompare(b);
      if (ia === -1) return 1;
      if (ib === -1) return -1;
      return ia - ib;
    });
    return ["all", ...uniq];
  }, [flat, vcmiLanguage]);

  // 自动展开含已选子模组的父模组
  useEffect(() => {
    setExpandedParents((prev) => {
      const next = new Set(prev);
      childrenMap.forEach((children, parentId) => {
        if (children.some((c) => selected.has(c.id))) next.add(parentId);
      });
      return next;
    });
  }, [childrenMap, selected]);

  return (
    <div className={cn("relative flex min-h-0 flex-1 flex-col", className)}>
      {/* ⚠️ 本页**刻意不加构建蒙层**（2026-09-22 拍板撤除）：
          列表所需的**全部语义字段**（名称 / 描述 / 类型 / **依赖 `depends`** / **冲突 `conflicts`** /
          子模组层级 `isSubmod`/`parentId` / 基线判定）都来自 `scan_mods`（`mod.json` + 文件系统），
          **与 wiki 缓存无关**；`cache.json` 只提供**城镇图片的相对路径**，缺图时 `ModCard` 已有占位
          （TypeIcon / 渐变底），并会在 **assets 完成后**自动补齐（见 `ModCard` 的
          `useWikiStageDone("assets")`）。
          预设读写同样与缓存解耦（`toVcmiPreset` 只用 scan 字段）→ **不存在"写空依赖"的代码路径**。
          ⚠️ 曾误以为"依赖 / 材质来自 `cache.json`"而加了阻塞蒙层，已撤除。
          依赖矩阵见 `docs/features/modcache.zh-CN.md` §2。 */}
      <div className="flex min-h-0 flex-1">
        {/* 左栏 */}
        <div className="flex w-[58%] flex-col overflow-hidden border-r border-slate-800/70">
          <div className="space-y-2 p-4">
            <div className="flex items-center gap-3">
              <div>
                <div className="text-[15px] font-semibold">
                  {leftTitle ?? t.mods.leftTitle}
                </div>
                <div className="text-xs text-slate-500">{leftSub ?? t.mods.leftSub}</div>
              </div>
              <Input
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                placeholder={t.mods.search}
                className="flex-1"
              />
            </div>
            <div className="flex flex-wrap items-center gap-1.5">
              {types.map((tpe) => (
                <button
                  key={tpe}
                  onClick={() => setTypeFilter(tpe)}
                  className={cn(
                    "rounded-full border px-3 py-1 text-xs transition-colors",
                    typeFilter === tpe
                      ? "border-indigo-500/60 bg-indigo-500/20 text-white"
                      : "border-slate-700/50 bg-slate-800/30 text-slate-400 hover:text-white",
                  )}
                >
                  {tpe === "all" ? t.mods.all : (t.mods.typeLabels[tpe] ?? tpe)}
                </button>
              ))}
              {toolbarExtra}
            </div>
            <div className="flex flex-wrap items-center gap-1.5 text-xs">
              {(
                [
                  ["unjoined", t.mods.unjoinedOnly],
                  ["partial", t.mods.partial],
                  ["all", t.mods.showAll],
                ] as [FilterMode, string][]
              ).map(([k, label]) => (
                <button
                  key={k}
                  onClick={() => setFilter(k)}
                  className={cn(
                    "rounded-lg border px-2.5 py-1 transition-colors",
                    filter === k
                      ? "border-blue-500/60 bg-blue-500/15 text-white"
                      : "border-slate-700/50 text-slate-400 hover:text-white",
                  )}
                >
                  {label}
                </button>
              ))}
              {/* 批量更新入口已迁至模组管理页操作栏（P5/T4）；本页仅保留「可更新」过滤 chip 与卡片徽章（展示信息） */}
              {/* 右对齐：城镇背景预览开关（默认关闭，写入 launcher.json）。
                  关闭时 ModCard 既不触发浮层，也不读取大图缓存（省掉每行的 IPC）。 */}
              <label className="ml-auto flex cursor-pointer select-none items-center gap-1.5 pl-2 text-slate-400 hover:text-white">
                <input
                  type="checkbox"
                  checked={townPreview}
                  onChange={(e) => void setTownPreviewEnabled(e.target.checked)}
                  className="h-3.5 w-3.5 accent-blue-500"
                />
                {t.mods.townPreview}
              </label>
            </div>
          </div>

          <div className="flex-1 overflow-y-auto p-4 pt-0">
            {listEmpty ? (
              <div className="py-10 text-center text-sm text-slate-500">
                {emptyLeft ?? t.mods.emptyLeft}
              </div>
            ) : typeFilter === "Town" ? (
              <div className="flex flex-col gap-5">
                {townLayout.standalone.length > 0 && (
                  <div className="grid grid-cols-2 gap-3">
                    {townLayout.standalone.map((mod) => (
                      <ModListItem
                        key={mod.id}
                        mod={mod}
                        lang={lang}
                        isSelected={isSelected(mod.id)}
                        hasConflict={conflictSet.has(mod.id)}
                        depTip={depTipOf(mod)}
                        townCard
                        locked={lockedMods?.has(mod.id)}
                        showTypeTag
                        onToggle={loading ? undefined : () => toggle(mod.id)}
                        onDetail={() => openDetail(mod.id)}
                        onClick={() => openDetail(mod.id)}
                      />
                    ))}
                  </div>
                )}
                {townLayout.grouped.length > 0 && (
                  <div className="flex flex-col gap-4">
                    {townLayout.grouped.map(({ parent, children }) => (
                      <TownGroupSection
                        key={parent.id}
                        parent={parent}
                        children={children}
                        lang={lang}
                        lockedMods={lockedMods}
                        collapsed={collapsedGroups.has(parent.id)}
                        onToggleCollapse={() =>
                          setCollapsedGroups((prev) => {
                            const next = new Set(prev);
                            if (next.has(parent.id)) next.delete(parent.id);
                            else next.add(parent.id);
                            return next;
                          })
                        }
                        isSelected={isSelected}
                        hasConflict={(id: string) => conflictSet.has(id)}
                        defectIds={defectIds}
                        onToggle={toggle}
                        disableToggle={loading}
                        onDetail={openDetail}
                      />
                    ))}
                  </div>
                )}
              </div>
            ) : (
              <div className="flex flex-col gap-2">
                {leftTree.map(({ parent, children }) => {
                  const hasChildren = (childrenMap.get(parent.id) ?? []).length > 0;
                  const parentMatchesFilter =
                    typeFilter === "all" || parent.type === typeFilter;
                  const parentMatchesQuery = matchesQuery(parent);
                  const keptByChild =
                    !parentMatchesFilter || !parentMatchesQuery;
                  const expanded = expandedParents.has(parent.id) || keptByChild;
                  return (
                    <div key={parent.id} className="flex flex-col gap-1.5">
                      <ModListItem
                        mod={parent}
                        lang={lang}
                        isSelected={isSelected(parent.id)}
                        hasConflict={conflictSet.has(parent.id)}
                        depTip={depTipOf(parent)}
                        expandable={hasChildren}
                        expanded={expanded}
                        showTypeTag
                        onToggle={loading ? undefined : () => toggle(parent.id)}
                        onDetail={() => openDetail(parent.id)}
                        onClick={() => openDetail(parent.id)}
                        onExpandToggle={() =>
                          setExpandedParents((prev) => {
                            const next = new Set(prev);
                            if (next.has(parent.id)) next.delete(parent.id);
                            else next.add(parent.id);
                            return next;
                          })
                        }
                      />
                      {/* 子模组整体包一个容器：整段缩进 + **一条**连续导轨。
                          不再逐行 `ml-6 border-l-4`（竖线断续、左右栏两套规范），
                          也不再逐行写「X 的子模组」——归属由容器表达。 */}
                      {expanded && children.length > 0 && (
                        <div className={CHILD_CONTAINER}>
                          {children.map((child) => (
                            <ModListItem
                              key={child.id}
                              mod={child}
                              lang={lang}
                              isChild
                              isSelected={isSelected(child.id)}
                              hasConflict={conflictSet.has(child.id)}
                              depTip={depTipOf(child)}
                              showTypeTag
                              onToggle={loading ? undefined : () => toggle(child.id)}
                              onDetail={() => openDetail(child.id)}
                              onClick={() => openDetail(child.id)}
                            />
                          ))}
                        </div>
                      )}
                    </div>
                  );
                })}
              </div>
            )}
          </div>
        </div>

        {/* 右栏 */}
        <RightPane
          baseMods={baseMods}
          selected={selected}
          flatMap={flatMap}
          installedIds={installedIds}
          conflictSet={conflictSet}
          lockedMods={lockedMods}
          defectIds={defectIds}
          loading={loading}
          onRemove={toggle}
          onDetail={openDetail}
        />
      </div>

      <DetailDrawer
        mod={detailMod}
        selected={new Set([...selected, ...baseMods])}
        onToggle={toggle}
        // 加载中不允许在抽屉里改选中（会被随后的预设解析覆盖）
        readOnly={loading}
        onClose={() => setDetailMod(null)}
      />
    </div>
  );
}

function TownGroupSection({
  parent,
  children,
  lang,
  collapsed,
  onToggleCollapse,
  isSelected,
  hasConflict,
  onToggle,
  onDetail,
  lockedMods,
  defectIds,
  disableToggle = false,
}: {
  parent: ModInfo;
  children: ModInfo[];
  lang: string;
  collapsed: boolean;
  onToggleCollapse: () => void;
  isSelected: (id: string) => boolean;
  hasConflict: (id: string) => boolean;
  onToggle: (id: string) => void;
  onDetail: (id: string) => void;
  lockedMods?: Set<string>;
  /** 依赖缺陷模组 id（小写）集合：命中卡片显示「依赖缺失」角标 */
  defectIds?: Set<string>;
  /** 数据未就绪：不传 onToggle（行内按钮整体不渲染），避免改动被后续解析覆盖 */
  disableToggle?: boolean;
}) {
  const { t } = useI18n();
  const parentName = localizedName(parent, lang);
  const parentIsTown = parent.type === "Town";
  // 父模组自身是否真提供城镇（口径同 ModCard 的 isTown / L553 的 standalone）：
  // 伞形 Town 包（实测 `New Pavilion`）自身 `factions` 为空、城镇内容全在子模组里，
  // 但它 `modType=Town` 会被算作城镇行 —— 照旧渲染就会在城镇网格里多出一张
  // 没有幕布图的卡片，与子模组的城镇图重复。自身无 factions 时不渲染父卡片
  // （父模组仍可通过分组标题进详情、勾选子模组即联动父模组）。
  const parentHasTown = parentIsTown && (parent.factions?.length ?? 0) > 0;
  const depTipOf = (m: ModInfo) =>
    defectIds?.has(m.id.toLowerCase()) ? t.mods.depTagTip : undefined;

  return (
    <section className="rounded-xl border border-slate-700/50 bg-slate-900/30 p-3">
      <header className="mb-3 flex items-center justify-between gap-2">
        <div className="flex min-w-0 items-center gap-2">
          <button
            onClick={() => onDetail(parent.id)}
            className="truncate text-sm font-semibold text-slate-100 hover:text-indigo-300 hover:underline"
            title={parentName}
          >
            {parentName}
          </button>
          {!parentIsTown && (
            <span
              className="flex-none rounded bg-slate-800 px-1.5 py-0.5 text-[10px] font-medium"
              style={{ color: TYPE_COLORS[parent.type] ?? "#94a3b8" }}
            >
              {t.mods.typeLabels[parent.type] ?? parent.type}
            </span>
          )}
          <span className="flex-none rounded-full bg-slate-800/80 px-2 py-0.5 text-[11px] text-slate-400">
            {children.length} {t.mods.submods}
          </span>
        </div>
        <button
          onClick={onToggleCollapse}
          className="flex-none rounded-lg border border-slate-700/50 px-2.5 py-1 text-[11px] text-slate-400 hover:text-white"
        >
          {collapsed ? `▸ ${t.mods.expand}` : `▾ ${t.mods.collapse}`}
        </button>
      </header>

      {!collapsed && (
        <div className="grid grid-cols-2 gap-3">
          {parentHasTown && (
            <ModListItem
              mod={parent}
              lang={lang}
              isSelected={isSelected(parent.id)}
              hasConflict={hasConflict(parent.id)}
              depTip={depTipOf(parent)}
              townCard
              isParent
              locked={lockedMods?.has(parent.id)}
              showTypeTag
              onToggle={disableToggle ? undefined : () => onToggle(parent.id)}
              onDetail={() => onDetail(parent.id)}
              onClick={() => onDetail(parent.id)}
            />
          )}
          {children.map((child) => (
            <ModListItem
              key={child.id}
              mod={child}
              lang={lang}
              isChild
              isSelected={isSelected(child.id)}
              hasConflict={hasConflict(child.id)}
              depTip={depTipOf(child)}
              townCard
              locked={lockedMods?.has(child.id)}
              showTypeTag
              onToggle={disableToggle ? undefined : () => onToggle(child.id)}
              onDetail={() => onDetail(child.id)}
              onClick={() => onDetail(child.id)}
            />
          ))}
        </div>
      )}
    </section>
  );
}

function RightPane({
  baseMods,
  selected,
  flatMap,
  installedIds,
  conflictSet,
  lockedMods,
  defectIds,
  loading = false,
  onRemove,
  onDetail,
}: {
  baseMods: string[];
  selected: Set<string>;
  flatMap: Map<string, ModInfo>;
  installedIds: Set<string>;
  conflictSet: Set<string>;
  lockedMods?: Set<string>;
  /** 依赖缺陷模组 id（小写）集合：命中行显示「依赖缺失」角标 */
  defectIds?: Set<string>;
  /** 调用方选中集合尚未就绪：显示加载提示，不提供移除按钮 */
  loading?: boolean;
  onRemove: (id: string) => void;
  onDetail: (id: string) => void;
}) {
  const { t, lang } = useI18n();
  const [openTypes, setOpenTypes] = useState<Set<string>>(new Set());
  // 右栏双视图：tree（默认，主体树，可移除）/ grouped（按类型分类，只读）
  const [viewMode, setViewMode] = useState<"tree" | "grouped">("tree");
  const [collapsedRoots, setCollapsedRoots] = useState<Set<string>>(new Set());

  const groups = useMemo(() => {
    const map = new Map<string, { id: string; isBase: boolean }[]>();
    const add = (id: string, isBase: boolean) => {
      if (RESERVED_MOD_IDS.has(id)) return; // 内置模组不在列表显示
      const m = flatMap.get(id);
      const type = m ? m.type : isBase ? "Core" : "Other";
      if (!map.has(type)) map.set(type, []);
      map.get(type)!.push({ id, isBase });
    };
    baseMods.forEach((b) => add(b, true));
    selected.forEach((id) => add(id, false));
    // 缺失模组单独分组（排除内置保留 scope）
    const missingIds = [...baseMods, ...selected].filter(
      (id) => !installedIds.has(id) && !RESERVED_MOD_IDS.has(id),
    );
    if (missingIds.length > 0) {
      if (!map.has("Missing")) map.set("Missing", []);
      missingIds.forEach((id) => {
        if (!map.get("Missing")!.some((x) => x.id === id)) {
          map.get("Missing")!.push({ id, isBase: false });
        }
      });
    }
    return map;
  }, [selected, flatMap, baseMods, installedIds]);

  // 默认展开包含选中项的类型，避免子模组被折叠隐藏
  useEffect(() => {
    setOpenTypes((prev) => {
      const next = new Set(prev);
      groups.forEach((items, type) => {
        if (items.length > 0) next.add(type);
      });
      return next;
    });
  }, [groups]);

  const sorted = [...groups.keys()].sort((a, b) => {
    if (a === "Missing") return 1;
    if (b === "Missing") return -1;
    return TYPE_ORDER.indexOf(a) - TYPE_ORDER.indexOf(b);
  });

  const toggleType = (type: string) => {
    setOpenTypes((prev) => {
      const next = new Set(prev);
      if (next.has(type)) next.delete(type);
      else next.add(type);
      return next;
    });
  };

  // 同类型分组内按 parentId 归并：父卡片在上、子模组缩进其下，呈现父子树。
  const renderGroupItems = (
    items: { id: string; isBase: boolean }[],
    groupType: string,
    readOnly = false,
  ): ReactNode[] => {
    const kidsOf = new Map<string, { id: string; isBase: boolean }[]>();
    const tops: { id: string; isBase: boolean }[] = [];
    for (const it of items) {
      const m = flatMap.get(it.id);
      const pid = m?.isSubmod ? m.parentId : undefined;
      if (pid && pid !== it.id && flatMap.has(pid)) {
        const arr = kidsOf.get(pid) ?? [];
        arr.push(it);
        kidsOf.set(pid, arr);
      } else {
        tops.push(it);
      }
    }
    const out: ReactNode[] = [];
    const renderItem = (
      it: { id: string; isBase: boolean },
      isChild: boolean,
    ): ReactNode => {
      const m = flatMap.get(it.id);
      if (!m) {
        return (
          <ModListItem
            key={it.id}
            mod={toMissingMod(it.id, t)}
            lang={lang}
            isMissing
            isSelected
            locked={lockedMods?.has(it.id)}
            showTypeTag
            onDetail={() => onDetail(it.id)}
            onClick={() => onDetail(it.id)}
          />
        );
      }
      // 归属文本只在「孤立子模组」时补齐：有容器时由缩进 + 导轨表达，不再逐行写
      const parentName =
        isChild || !m.isSubmod
          ? undefined
          : localizedName(flatMap.get(m.parentId) ?? toMissingMod(m.parentId, t), lang);
      return (
        <ModListItem
          key={it.id}
          mod={m}
          lang={lang}
          isSelected
          isBase={it.isBase}
          isMissing={groupType === "Missing"}
          locked={lockedMods?.has(it.id)}
          hasConflict={conflictSet.has(it.id)}
          depTip={defectIds?.has(m.id.toLowerCase()) ? t.mods.depTagTip : undefined}
          showTypeTag
          parentName={parentName}
          onToggle={
            readOnly || loading || it.isBase || lockedMods?.has(it.id)
              ? undefined
              : () => onRemove(it.id)
          }
          onDetail={() => onDetail(it.id)}
          onClick={() => onDetail(it.id)}
        />
      );
    };
    for (const top of tops) {
      out.push(renderItem(top, false));
      const kids = kidsOf.get(top.id);
      if (kids && kids.length > 0) {
        out.push(
          <div key={`${top.id}__kids`} className={CHILD_CONTAINER}>
            {kids.map((k) => renderItem(k, true))}
          </div>,
        );
      }
    }
    // 父不在本分组的孤儿子模组：平铺并保留父级标签（没有容器可表达归属）
    kidsOf.forEach((kids, pid) => {
      if (!tops.some((t2) => t2.id === pid)) {
        kids.forEach((k) => out.push(renderItem(k, false)));
      }
    });
    return out;
  };

  // 主体树视图：将全部已选项（base + selected）按「根父模组」归并为森林。
  // 父在选中集合内 → 子模组嵌套其下；父未选中 → 归入「独立子模组」；缺失根 → 「缺失模组」。
  const allItems = useMemo<{ id: string; isBase: boolean }[]>(() => {
    const arr: { id: string; isBase: boolean }[] = [];
    groups.forEach((items) => items.forEach((it) => arr.push(it)));
    return arr;
  }, [groups]);

  const idSet = useMemo(() => new Set(allItems.map((i) => i.id)), [allItems]);

  const pidOf = (id: string): string => {
    const m = flatMap.get(id);
    if (m?.isSubmod) return m.parentId || "";
    if (id.includes(".")) return id.split(".")[0]; // zip 内子模组不在 flatMap，按点号取父
    return "";
  };

  const forest = useMemo(() => {
    const childMap = new Map<string, { id: string; isBase: boolean }[]>();
    const roots: { id: string; isBase: boolean }[] = [];
    const orphans: { id: string; isBase: boolean }[] = [];
    for (const it of allItems) {
      const pid = pidOf(it.id);
      if (pid && pid !== it.id && idSet.has(pid)) {
        const arr = childMap.get(pid) ?? [];
        arr.push(it);
        childMap.set(pid, arr);
      } else if (it.id.includes(".")) {
        orphans.push(it);
      } else {
        roots.push(it);
      }
    }
    const typeIdx = (id: string) => {
      const m = flatMap.get(id);
      const ty = m ? m.type : "Other";
      if (ty === "Missing") return 999;
      const i = TYPE_ORDER.indexOf(ty);
      return i === -1 ? 998 : i;
    };
    const cmp = (a: { id: string }, b: { id: string }) =>
      typeIdx(a.id) - typeIdx(b.id) ||
      localizedName(flatMap.get(a.id) ?? toMissingMod(a.id, t), lang).localeCompare(
        localizedName(flatMap.get(b.id) ?? toMissingMod(b.id, t), lang),
        lang,
      );
    roots.sort(cmp);
    orphans.sort(cmp);
    return {
      childMap,
      normalRoots: roots.filter((r) => installedIds.has(r.id)),
      missingRoots: roots.filter((r) => !installedIds.has(r.id)),
      orphans,
    };
  }, [allItems, flatMap, idSet, installedIds, lang, t]);

  const renderTreeNode = (it: { id: string; isBase: boolean }, depth: number): ReactNode => {
    const m = flatMap.get(it.id);
    const missing = !installedIds.has(it.id);
    const kids = forest.childMap.get(it.id) ?? [];
    // 缩进与导轨交给子容器（CHILD_CONTAINER），这里不再按 depth 逐节点加 ml / border
    return (
      <div key={it.id}>
        <ModListItem
          mod={m ?? toMissingMod(it.id, t)}
          lang={lang}
          isSelected
          isBase={it.isBase}
          isMissing={missing}
          isChild={depth > 0}
          locked={lockedMods?.has(it.id)}
          hasConflict={conflictSet.has(it.id)}
          showTypeTag
          isParent={depth === 0 && kids.length > 0}
          expandable={depth === 0 && kids.length > 0}
          expanded={!collapsedRoots.has(it.id)}
          onToggle={
            loading || it.isBase || lockedMods?.has(it.id) ? undefined : () => onRemove(it.id)
          }
          onDetail={() => onDetail(it.id)}
          onClick={() => onDetail(it.id)}
          onExpandToggle={() =>
            setCollapsedRoots((prev) => {
              const n = new Set(prev);
              if (n.has(it.id)) n.delete(it.id);
              else n.add(it.id);
              return n;
            })
          }
        />
        {depth === 0 && kids.length > 0 && !collapsedRoots.has(it.id) && (
          <div className={CHILD_CONTAINER}>
            {kids.map((k) => renderTreeNode(k, depth + 1))}
          </div>
        )}
      </div>
    );
  };

  return (
    <div className="flex min-w-0 flex-1 flex-col overflow-hidden">
      <div className="flex min-w-0 items-center justify-between gap-2 p-4 pb-2">
        <div className="min-w-0">
          <div className="truncate text-[15px] font-semibold">{t.mods.rightTitle}</div>
          {/* 副标题是视图切换的图例：英文（`Tree (removable) / By type (read-only)`）比中文长近一倍，
              原来会把操作区挤到按钮内断词。改为单行截断 + hover 全文，空间不足时优雅降级。 */}
          <div className="truncate text-xs text-slate-500" title={t.mods.rightSub}>
            {t.mods.rightSub}
          </div>
        </div>
        {/* 操作区：文字标签在英文下（Main tree / By type / Expand / Collapse ≈ 258px）
            会把右栏挤爆，改为**图标按钮 + title**（≈130px）：中英文与窄窗口都稳定。
            文案仍取自既有 i18n 键，只作无障碍名与提示，不新增键。 */}
        <div className="flex flex-none items-center gap-1.5">
          <div className="flex overflow-hidden rounded-lg border border-slate-700/50">
            <button
              onClick={() => setViewMode("tree")}
              title={t.mods.viewTree}
              aria-label={t.mods.viewTree}
              className={`px-2.5 py-2 ${viewMode === "tree" ? "bg-indigo-500/30 text-white" : "text-slate-400 hover:text-white"}`}
            >
              <ListTree className="h-4 w-4" />
            </button>
            <button
              onClick={() => setViewMode("grouped")}
              title={t.mods.viewGrouped}
              aria-label={t.mods.viewGrouped}
              className={`border-l border-slate-700/50 px-2.5 py-2 ${viewMode === "grouped" ? "bg-indigo-500/30 text-white" : "text-slate-400 hover:text-white"}`}
            >
              <LayoutList className="h-4 w-4" />
            </button>
          </div>
          <button
            onClick={() =>
              viewMode === "tree"
                ? setCollapsedRoots(new Set())
                : setOpenTypes(new Set(sorted))
            }
            title={t.mods.expand}
            aria-label={t.mods.expand}
            className="rounded border border-slate-700/50 p-2 text-slate-400 hover:text-white"
          >
            <UnfoldVertical className="h-4 w-4" />
          </button>
          <button
            onClick={() =>
              viewMode === "tree"
                ? setCollapsedRoots(
                    new Set([...forest.normalRoots, ...forest.missingRoots].map((r) => r.id)),
                  )
                : setOpenTypes(new Set())
            }
            title={t.mods.collapse}
            aria-label={t.mods.collapse}
            className="rounded border border-slate-700/50 p-2 text-slate-400 hover:text-white"
          >
            <FoldVertical className="h-4 w-4" />
          </button>
        </div>
      </div>
      {viewMode === "grouped" && (
        <div className="px-4 pb-1 text-[11px] text-slate-500">{t.mods.readonlyHint}</div>
      )}
      <div className="flex-1 overflow-y-auto px-4 pb-4">
        {loading ? (
          <div className="py-10 text-center text-sm text-slate-500">{t.mods.loading}</div>
        ) : allItems.length === 0 ? (
          <div className="py-10 text-center text-sm text-slate-500">{t.mods.emptyRight}</div>
        ) : viewMode === "tree" ? (
          <div className="flex flex-col gap-2">
            {forest.normalRoots.map((r) => renderTreeNode(r, 0))}
            {forest.orphans.length > 0 && (
              <div className="mt-1">
                <div className="mb-1.5 px-1 text-[11px] font-medium uppercase tracking-wide text-slate-500">
                  {t.mods.orphanSubs}
                </div>
                <div className="flex flex-col gap-2">
                  {forest.orphans.map((o) => renderTreeNode(o, 0))}
                </div>
              </div>
            )}
            {forest.missingRoots.length > 0 && (
              <div className="mt-1">
                <div className="mb-1.5 flex items-center gap-2 px-1 text-[11px] font-medium uppercase tracking-wide text-red-400">
                  {t.mods.missing}
                </div>
                <div className="flex flex-col gap-2">
                  {forest.missingRoots.map((r) => renderTreeNode(r, 0))}
                </div>
              </div>
            )}
          </div>
        ) : (
          sorted.map((type) => {
            const items = groups.get(type)!;
            const open = openTypes.has(type);
            const hasConf = items.some((x) => conflictSet.has(x.id));
            const label = type === "Missing" ? t.mods.missing : (t.mods.typeLabels[type] ?? type);
            return (
              <div key={type} className="mb-3">
                <button
                  onClick={() => toggleType(type)}
                  className="mb-1.5 flex w-full items-center justify-between rounded-lg border border-slate-700/40 bg-slate-800/40 px-3 py-2 text-left hover:bg-slate-800/70"
                >
                  <div className="flex items-center gap-2">
                    <span className="text-sm font-semibold">{label}</span>
                    <span className="rounded-full bg-slate-800/60 px-2 py-0.5 text-[11px] text-slate-400">
                      {items.length}
                    </span>
                    {hasConf && (
                      <span className="rounded-full bg-red-500/15 px-2 py-0.5 text-[11px] text-red-400">
                        ⚠
                      </span>
                    )}
                  </div>
                  {open ? (
                    <ChevronUp className="h-4 w-4 text-slate-500" />
                  ) : (
                    <ChevronDown className="h-4 w-4 text-slate-500" />
                  )}
                </button>
                {open && (
                  <div className="flex flex-col gap-2 pl-1">
                    {renderGroupItems(items, type, true)}
                  </div>
                )}
              </div>
            );
          })
        )}
      </div>
    </div>
  );
}

function toMissingMod(
  id: string,
  t: { mods: { missing: string; missingTip: string; typeLabels: Record<string, string> } },
): ModInfo {
  return {
    id,
    diskName: id,
    parentId: id.includes(".") ? id.split(".")[0] : id,
    name: id,
    localizedName: id,
    desc: t.mods.missingTip,
    type: "Other",
    depends: [],
    softDepends: [],
    conflicts: [],
    modPath: "",
    hasContentZip: false,
    contentZipEncrypted: false,
    isSubmod: id.includes("."),
    subs: [],
  };
}
