import { startTransition, useEffect, useMemo, useRef, useState } from "react";
import { useI18n } from "@/i18n";
import type { LangDict } from "@/i18n/types";
import {
  buildPresetReport,
  type PresetReportInput,
  type ReportMode,
  type ReportTexts,
} from "@/lib/presetReport";
import { ReportPanel } from "./ReportPanel";
import { Toast } from "@/shared/Toast";
import { flattenMods, resolvePresetIds } from "@/lib/modHelpers";
import { RESERVED_MOD_IDS } from "@/lib/modScope";
import type { ModInfo } from "@/lib/modScanner";
import {
  BUILTIN_PRESETS,
  builtinPresetMods,
  builtinPresetLabel,
  isBuiltinPreset,
  mergeBuiltinPresets,
  setActiveBuiltinPresets,
  getActiveBuiltinPresets,
} from "@/lib/builtinPresets";
import { ModBrowser } from "./ModBrowser";
import { ModManagerPanel, type ModQueueItem } from "./ModManagerPanel";
import { modRepoIndex, modUpdateMap } from "@/lib/modRepo";
import {
  fetchModIndex,
  autoCheckReposEnabled,
  listModLedgers,
  installModFromRepo,
  cancelModDownload,
  onModIndexProgress,
  onModIndexUpdated,
  MOD_DOWNLOAD_CANCELLED,
  type ModIndexProgress,
  type ModRepoEntry,
  type ModLedgerInfo,
} from "@/lib/tauri";
import { fromVcmiPreset, toVcmiPreset, excludeBaseSubmods } from "@/lib/vcmiPreset";
import { PresetBar } from "./PresetBar";
import { PresetSaveDialog } from "./PresetSaveDialog";
import { ConfirmDialog } from "@/shared/ConfirmDialog";
import { cn } from "@/lib/utils";
import {
  isTauri,
  scanRealMods,
  rescanRealMods,
  readModSettingsJson,
  writeModSettingsJson,
  readLauncherConfig,
  clearModValidation,
} from "@/lib/tauri";
import { LoadingOverlay } from "@/shared/LoadingOverlay";
import { backendErrorText } from "@/lib/backendError";
import { loadEngineVersion, getEngineVersionSnapshot } from "@/lib/engineVersion";
import { loadVcmiLanguage } from "@/lib/vcmiLanguage";
import { loadTownPreview } from "@/lib/townPreview";

interface PresetData {
  mods: string[];
  settings: Record<string, Record<string, boolean>>;
}

/** 报告文案打包：i18n 词典 → `lib/presetReport` 的文本注入（保持报告模块无 i18n 运行时依赖） */
function reportTextsFrom(t: LangDict): ReportTexts {
  const p = t.preset;
  return {
    title: p.reportTitle,
    heading: p.reportHeading,
    generated: p.reportGenerated,
    engine: p.reportEngine,
    gameLang: p.reportGameLang,
    presetKind: p.reportPresetKind,
    kindBuiltin: p.reportKindBuiltin,
    kindUser: p.reportKindUser,
    total: p.reportTotal,
    updatesHint: p.reportUpdatesHint,
    missingHint: p.reportMissingHint,
    overview: p.reportOverview,
    colItem: p.reportColItem,
    colCount: p.reportColCount,
    statEnabled: p.reportStatEnabled,
    statBase: p.reportStatBase,
    statRequired: p.reportStatRequired,
    statChosen: p.reportStatChosen,
    statSettings: p.reportStatSettings,
    statUpdates: p.reportStatUpdates,
    statMissing: p.reportStatMissing,
    sectionMods: p.reportSectionMods,
    groupHeading: p.reportGroupHeading,
    colMod: p.reportColMod,
    colId: p.reportColId,
    colVersion: p.reportColVersion,
    colNote: p.reportColNote,
    noteBase: p.reportNoteBase,
    noteRequired: p.reportNoteRequired,
    noteUpdate: p.reportNoteUpdate,
    noteLangMismatch: p.reportNoteLangMismatch,
    noteIncompat: p.reportNoteIncompat,
    updateMark: p.reportUpdateMark,
    settingsTitle: p.reportSettingsTitle,
    colParent: p.reportColParent,
    colSub: p.reportColSub,
    colState: p.reportColState,
    stateOn: p.reportStateOn,
    stateOff: p.reportStateOff,
    missingTitle: p.reportMissingTitle,
    missingNote: p.reportMissingNote,
    briefMissing: p.reportBriefMissing,
    typeLabel: (type: string) => t.mods.typeLabels[type] ?? type,
  };
}

function buildInitialPresets(lang: string): Record<string, PresetData> {
  return Object.fromEntries(
    getActiveBuiltinPresets().map((p) => [
      p.id,
      { mods: builtinPresetMods(p.id, lang), settings: {} },
    ]),
  );
}
const initialActive = getActiveBuiltinPresets()[0]?.id ?? "__official__";

/**
 * 内置预设「只补不删」：以磁盘现有 mods 为基准，确保基线（builtinPresetMods 的最小必选集，
 * 如 vcmi/core）存在，但**不丢弃**磁盘上被运行期动态注入的额外 mod（如语言包）。
 * - 顺序：缺失的基线项置前，其余保持磁盘原顺序（语言包等追加项位置稳定、始终启用）。
 * - 这是修复「重新打开启动器后语言包被抹掉」的核心：内置预设定义只声明通用基线，
 *   语言包由 langMods 按当前语言注入到预设数组，ModView 必须尊重磁盘现状。
 */
function ensureBaseline(diskMods: string[], baseline: string[]): string[] {
  const diskSet = new Set(diskMods);
  const missing = baseline.filter((m) => !diskSet.has(m));
  return [...missing, ...diskMods];
}

/**
 * 「导入完成」但 ModView 未挂载（用户在其他视图导入）或初始化未就绪期间到达的模组队列。
 * 挂载 / 就绪后统一消费：重扫 + 自动勾选 + 成功横幅。
 */
const pendingInstalledMods: { modId: string; name: string }[] = [];

// 从原生 activeMods 嵌套树中收集所有「启用」的模组 id（用于 presets 缺失时恢复用户模式）
function collectEnabledMods(activeMods: unknown): string[] {
  const out: string[] = [];
  const walk = (node: Record<string, unknown>) => {
    for (const [id, v] of Object.entries(node)) {
      if (!v || typeof v !== "object") continue;
      const entry = v as { active?: boolean; mods?: Record<string, unknown> };
      if (entry.active !== false) out.push(id);
      if (entry.mods) walk(entry.mods as Record<string, unknown>);
    }
  };
  if (activeMods && typeof activeMods === "object") walk(activeMods as Record<string, unknown>);
  return out;
}

export function ModView({
  onModsLoadingChange,
  mode = "presets",
  onModeChange,
}: {
  onModsLoadingChange?: (loading: boolean) => void;
  /** 视图态：预设编排（presets）/ 模组管理（manage）。由 App 提升持有，便于启动前弹窗「前往模组管理」从外部切入管理页 */
  mode?: "presets" | "manage";
  /** 切换视图态（setMode）。受控于 App；不传则内部回退默认，避免非受控场景报错 */
  onModeChange?: (m: "presets" | "manage") => void;
} = {}) {
  const { t, lang, vcmiLang } = useI18n();
  const initialPresets = useMemo(() => buildInitialPresets(lang), [lang]);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [presets, setPresets] = useState<Record<string, PresetData>>(initialPresets);
  // 空串 = 真实 activePreset 尚未读出。解析完成前 PresetBar 只显示「加载中」，
  // 不再先把内置预设（原版）当成当前预设渲染出来、扫完再跳变。
  const [activePreset, setActivePreset] = useState<string>("");
  const [rootMods, setRootMods] = useState<ModInfo[]>([]);
  const [dirty, setDirty] = useState(false);
  const [loadingMods, setLoadingMods] = useState(true);
  // 初始化阶段机：resolving（读配置：真实预设已确定、不可交互）
  //   → scanning（扫模组：左栏渐进填充、禁止改动选中）
  //   → ready（按预设解析出选中集合、允许交互与写盘）。
  const [boot, setBoot] = useState<"resolving" | "scanning" | "ready">("resolving");
  // 全局强制模组（来自 launcher.json 的 requiredMods），用户不可关闭，始终包含在选中集合与保存结果中。
  const [requiredMods, setRequiredMods] = useState<string[]>([]);
  const lockedMods = useMemo(() => new Set(requiredMods), [requiredMods]);
  // 顶部浮层提示：`error` = 依赖/冲突/版本守卫拦截（红）；`success` = 维护操作结果（绿）。
  // 手动关闭；下一次成功的勾选（handleToggle）也会自动清除。
  const [banner, setBanner] = useState<{ text: string; kind: "error" | "success" } | null>(null);
  // 轻提示（自动消失）：复制/粘贴等瞬时反馈走 Toast，不占用需手动关闭的 banner；
  // `id` 用于连续提示时重启消失计时（见 shared/Toast.tsx）。
  const [toast, setToast] = useState<{ id: number; text: string; kind: "success" | "error" } | null>(null);
  // 报告（页内覆盖层）：状态在页级持有（报告覆盖预设栏与列表，不属于 PresetBar 自身）。
  const [reportOpen, setReportOpen] = useState(false);
  const [reportMode, setReportMode] = useState<ReportMode>("brief");
  /** 阅读宽度：默认居中（电子书感），可切全宽看宽表格 */
  const [reportFullWidth, setReportFullWidth] = useState(false);
  // 把强制模组并入任意「选中 id 集合」，作为统一兜底（加载/切换/保存均经此）。
  const withRequired = (ids: Iterable<string>): Set<string> => {
    const s = new Set(ids);
    for (const r of requiredMods) s.add(r);
    return s;
  };

  useEffect(() => {
    onModsLoadingChange?.(loadingMods);
  }, [loadingMods, onModsLoadingChange]);

  const allFlat = useMemo(() => flattenMods(rootMods), [rootMods]);
  const flat = useMemo(() => allFlat.filter((m) => !RESERVED_MOD_IDS.has(m.id)), [allFlat]);

  // 基线 mod 判定（通用）：modType 为 "Core" 的是引擎核心（vcmi / core），
  // 不可由用户取消；不依赖具体版本（增强版/官方版/第三方）的写死名单。
  // RESERVED_MOD_IDS 显式加入，确保即使未扫描到目录也会写入保存。
  const baseMods = useMemo(() => {
    const coreIds = allFlat.filter((m) => m.type === "Core").map((m) => m.id);
    return [...new Set([...coreIds, ...RESERVED_MOD_IDS])];
  }, [allFlat]);

  // 语言切换时：重新计算内置预设的 mods，若当前正在使用内置预设则自动重载
  const didInitLang = useRef(false);
  useEffect(() => {
    if (!didInitLang.current) {
      didInitLang.current = true;
      return;
    }
    setPresets((prev) => {
      const next = { ...prev };
      for (const name of Object.keys(next)) {
        if (isBuiltinPreset(name)) {
          // 只补基线、不删磁盘上被注入的语言包等额外 mod
          next[name] = {
            ...next[name],
            mods: ensureBaseline(next[name].mods, builtinPresetMods(name, lang)),
          };
        }
      }
      return next;
    });
    if (isBuiltinPreset(activePreset)) {
      const mods = ensureBaseline(
        presets[activePreset]?.mods ?? [],
        builtinPresetMods(activePreset, lang),
      );
      const resolved = resolvePresetIds(mods, flat);
      setSelected(withRequired(resolved.filter((id) => !baseMods.includes(id))));
      setDirty(false);
    }
    // 注意：modSettings.json 的写回由下方 sync effect 统一负责，
    // 保证语言切换 / 预设重载后界面态与磁盘态一致。
  }, [lang, activePreset, baseMods, flat]);

  // 界面态 ↔ 磁盘态同步：内存中的预设组合/当前预设变化后写回 modSettings.json。
  // 覆盖首次加载、语言切换、内置预设重载等所有场景，避免启动游戏时配置落后于界面。
  const firstSync = useRef(true);
  // 保存整份原始 modSettings.json（含原生 activeMods / core / validatedMods 等顶层字段），
  // 写回时只 patch activePreset / presets，避免清掉用户已有的原生配置（修复启动即丢失模式的问题）。
  const fullSettingsRef = useRef<Record<string, unknown> | null>(null);
  // 标记「本次加载是否成功读到了磁盘上的真实配置」。读取失败（如路径未就绪）时保持 false，
  // 任何写盘操作都将被跳过，避免用空壳覆盖用户已有的 modSettings.json（修复启动即丢失模式的问题）。
  const readSucceededRef = useRef(false);
  const buildWritePayload = (
    nextActive: string,
    nextPresets: Record<string, PresetData>,
  ): Record<string, unknown> => {
    const base = fullSettingsRef.current ?? {};
    return { ...base, activePreset: nextActive, presets: nextPresets };
  };
  // 实际写盘：仅当成功读取过真实配置后才允许写入，否则跳过（保护用户数据）。
  const writePresetsNow = (nextActive: string, nextPresets: Record<string, PresetData>) => {
    if (!isTauri() || !readSucceededRef.current) return;
    writeModSettingsJson(buildWritePayload(nextActive, nextPresets)).catch((e) =>
      console.error("[ModView] 写回 modSettings 失败:", e),
    );
  };
  // 统一的写盘出口：初始化未完成前一律不写，避免用半初始化的预设字典覆盖用户已有配置。
  const writePresetsSafe = (nextActive: string, nextPresets: Record<string, PresetData>) => {
    if (boot !== "ready") return;
    writePresetsNow(nextActive, nextPresets);
  };
  useEffect(() => {
    if (firstSync.current) {
      firstSync.current = false;
      return;
    }
    writePresetsSafe(activePreset, presets);
  }, [activePreset, presets]);

  // 加载真实数据：三阶段（配置 → 扫描 → 解析选中）。
  // 顺序刻意把「读配置」提到「扫模组」之前：扫模组是慢操作（真实环境数百模组、分块渲染），
  // 先读配置可让预设名/预设字典在第一阶段就位，消除「先显示原版、扫完再跳变」的错帧。
  useEffect(() => {
    let cancelled = false;
    setLoadingMods(true);
    (async () => {
      if (!isTauri()) {
        // 浏览器 mock：无磁盘配置，沿用内置预设常量并直接进入可交互态
        setActivePreset(initialActive);
        setLoadingMods(false);
        setBoot("ready");
        return;
      }
      try {
        // —— 阶段 1：配置（launcher.json + modSettings.json 并行，快）——
        // 同时**预热"行渲染依赖的模块级订阅"**（引擎版本 / 游戏语言 / 城镇预览开关）：
        // 它们原本在 ModBrowser / ModListItem 挂载时才发起 IPC，返回后会让数百行**整体重渲染**
        //（表现为"列表出现后又卡一下"）。预热后 ready 时已是终值；三者在模块内幂等。
        const [cfg, msRaw] = await Promise.all([
          readLauncherConfig(),
          readModSettingsJson(),
          loadEngineVersion(),
          loadVcmiLanguage(),
          loadTownPreview(),
        ]);
        if (cancelled) return;
        // 读取启动器配置：合并内置预设 + 提取全局强制模组
        cfgRef.current = cfg;
        const effective = mergeBuiltinPresets(BUILTIN_PRESETS, cfg?.builtinPresets as any);
        setActiveBuiltinPresets(effective);
        const req = cfg?.requiredMods ?? [];
        setRequiredMods(req);

        const ms = msRaw as {
          activePreset?: string;
          presets?: Record<string, PresetData>;
          activeMods?: unknown;
          core?: unknown;
          validatedMods?: unknown;
        } | null;
        // 保留整份原文（含原生 activeMods/core/validatedMods），写回时只 patch 部分字段
        fullSettingsRef.current = (ms as Record<string, unknown> | null) ?? null;
        // 仅当成功读到磁盘配置后才允许后续写盘；读失败则保持 false 以阻止覆盖真实文件。
        readSucceededRef.current = ms !== null;
        // 统一处理：始终基于文件现有 presets 合并（保留用户预设），并确保内置 __official__ 存在
        const filePresets =
          ms?.presets && typeof ms.presets === "object"
            ? (ms.presets as Record<string, PresetData>)
            : {};
        const merged: Record<string, PresetData> = { ...initialPresets, ...filePresets };
        // 内置预设只补基线（vcmi/core）、不删磁盘上被运行期注入的额外 mod（语言包等）：
        // 语言包由 langMods 按当前语言注入到预设数组，重新打开启动器时不能被覆盖抹掉。
        for (const name of Object.keys(merged)) {
          if (isBuiltinPreset(name)) {
            merged[name] = {
              ...merged[name],
              mods: ensureBaseline(merged[name].mods, builtinPresetMods(name, lang)),
            };
          }
        }
        // 若文件中没有任何「用户预设」（仅内置或 presets 为空），且原生 activeMods 存在，
        // 则从 activeMods 树恢复一个 default 预设，避免用户已有模式被清空
        const hasUserPreset = Object.keys(filePresets).some((k) => !isBuiltinPreset(k));
        if (!hasUserPreset && ms?.activeMods && typeof ms.activeMods === "object") {
          const recovered = collectEnabledMods(ms.activeMods);
          if (recovered.length) {
            merged["default"] = {
              mods: recovered,
              settings:
                (filePresets["default"]?.settings as Record<string, Record<string, boolean>>) ?? {},
            };
          }
        }
        setPresets(merged);
        const ap =
          ms?.activePreset && merged[ms.activePreset]
            ? ms.activePreset
            : merged["default"]
              ? "default"
              : Object.keys(merged)[0];
        setActivePreset(ap);
        if (cancelled) return;
        // 预设已确定：PresetBar 立刻显示真实预设名，列表区转为「扫描中」
        setBoot("scanning");

        // —— 阶段 2：扫描模组（慢）——
        const realMods = (await scanRealMods()) as ModInfo[];
        if (cancelled) return;

        // —— 阶段 3：按真实 mod 树解析当前预设的选中集合 ——
        // 这里用本次扫描结果本地计算基线（而不是闭包里的 baseMods，后者此时仍是空列表派生值）。
        const allLocal = flattenMods(realMods);
        const flatLocal = allLocal.filter((m) => !RESERVED_MOD_IDS.has(m.id));
        const baseLocal = [
          ...new Set([
            ...allLocal.filter((m) => m.type === "Core").map((m) => m.id),
            ...RESERVED_MOD_IDS,
          ]),
        ];
        const p = merged[ap];
        if (p) {
          const resolvedMods = resolvePresetIds(p.mods, flatLocal);
          const flatSel = fromVcmiPreset(flatLocal, { mods: resolvedMods, settings: p.settings });
          setSelected(new Set([...excludeBaseSubmods(flatLocal, flatSel, baseLocal), ...req]));
        }
        // 与磁盘同步一次（界面态为权威）：buildWritePayload 保留原生的
        // activeMods/core/validatedMods，不会清空用户模式。
        writePresetsNow(ap, merged);
        if (cancelled) return;
        // ⚠️ 列表数据与「就绪」标记**必须同一批次**提交（此处共用同一个 transition）：
        // 若只把列表放进 transition、而 `setBoot("ready")` 走紧急更新，React 会**先**提交就绪
        //（蒙层撤除、列表仍为空），几百行的渲染再单独跑 —— 用户看到的就是
        //「蒙层消失 → 卡一下 → 列表出现」（2026-10-08 实测反馈）。
        // 曾按 15 个一组分块 setState（"渐进渲染"）：每次 setState 都重算排序等全部 useMemo
        // 并重渲染**已挂载的全部行**，渲染量近似 O(N²)，是首屏卡顿的主因（文档 §2 R2）。
        startTransition(() => {
          setRootMods(realMods);
          setLoadingMods(false);
          setBoot("ready");
        });
      } catch (e) {
        console.error("[ModView] 加载真实数据失败:", e);
        setLoadingMods(false);
        // 读取失败仍进入可用态（保持内置预设），且因 readSucceededRef 为 false 不会写盘
        setActivePreset((prev) => prev || initialActive);
        setBoot("ready");
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const persistPresets = (next: Record<string, PresetData>, nextActive: string) => {
    setPresets(next);
    setActivePreset(nextActive);
    writePresetsSafe(nextActive, next);
  };

  // 选中变化：ModBrowser 已完成父子/依赖联动，这里仅记录脏状态
  const handleToggle = (next: Set<string>) => {
    setSelected(withRequired(next));
    setDirty(true);
    setBanner(null);
  };

  /**
   * 清除当前预设模组的 `validatedMods` 校验缓存 → VCMI 下次启动重新校验。
   *
   * 适用场景：在游戏外替换过模组素材 / 增删文件（checksum 只覆盖 mod.json + config json，
   * 这类改动不会让它失效），或游戏报模组加载失败需要排查完整性。只读之外**唯一**的写操作
   * 是把这些模组从 `validatedMods` 里摘掉，不改动启用组合。
   */
  const handleRevalidate = async () => {
    const ids = [...baseMods, ...requiredMods, ...selected];
    const cleared = await clearModValidation(undefined, ids);
    setBanner({
      text:
        cleared > 0
          ? t.mods.revalidated.replace("{n}", String(cleared))
          : t.mods.revalidateNone,
      kind: "success",
    });
  };

  /* ----------------------- 导入游戏文件（底栏入口）事件 ----------------------- */

  /**
   * 弹层（ImportFilesDialog）导入模组 / HD 素材成功后广播 `vcmi-fv:mods-installed`：
   * 此处重扫 + **自动勾选**（与官方 launcher 安装后自动启用一致）+ 成功横幅。
   * 载荷在「本组件未挂载」或「初始化未就绪」时先进模块级队列，等就绪后统一消费。
   */
  const bootRef = useRef(boot);
  bootRef.current = boot;
  const selectedRef = useRef(selected);
  selectedRef.current = selected;
  const withRequiredRef = useRef(withRequired);
  withRequiredRef.current = withRequired;
  const tRef = useRef(t);
  tRef.current = t;

  /** 消费一批「导入完成」的模组：重扫 → 自动勾选 → 成功横幅 */
  const applyInstalledMods = async (mods: { modId: string; name: string }[]) => {
    try {
      const realMods = (await rescanRealMods()) as ModInfo[];
      setRootMods(realMods);
      if (bootRef.current === "ready") {
        setSelected(
          withRequiredRef.current(new Set([...selectedRef.current, ...mods.map((m) => m.modId)])),
        );
        setDirty(true);
      }
      const first = mods[0];
      if (first) {
        setBanner({
          text: tRef.current.mods.installOk
            .replace("{name}", first.name || first.modId)
            .replace("{id}", first.modId),
          kind: "success",
        });
      }
    } catch (e) {
      console.error("[ModView] 导入完成后重扫失败:", e);
    }
  };
  const applyInstalledModsRef = useRef(applyInstalledMods);
  applyInstalledModsRef.current = applyInstalledMods;

  useEffect(() => {
    const handler = (e: Event) => {
      const mods =
        (e as CustomEvent<{ mods?: { modId: string; name: string }[] }>).detail?.mods ?? [];
      if (mods.length === 0) return;
      if (bootRef.current !== "ready") {
        pendingInstalledMods.push(...mods);
        return;
      }
      void applyInstalledModsRef.current(mods);
    };
    window.addEventListener("vcmi-fv:mods-installed", handler);
    return () => window.removeEventListener("vcmi-fv:mods-installed", handler);
  }, []);

  // boot 就绪后消费「未挂载 / 未就绪期间」到达的导入结果
  useEffect(() => {
    if (boot !== "ready" || pendingInstalledMods.length === 0) return;
    const queued = pendingInstalledMods.splice(0);
    void applyInstalledModsRef.current(queued);
  }, [boot]);

  const handleSave = () => {
    if (isBuiltinPreset(activePreset)) {
      // 内置预设不能直接保存，弹出「另存为」弹窗，继承源固定为当前内置预设
      setBuiltinSaveOpen(activePreset);
      return;
    }
    // 转 VCMI 原生格式：根模组进 mods，与默认态（!keepDisabled）不同的子模组开关进 settings。
    // 基线/强制模组（baseMods/requiredMods）的子模组保持 VCMI 默认，不写 settings（managedRoots 排除它们）。
    const flatMap = new Map(flat.map((m) => [m.id, m]));
    const managedRoots = new Set(
      [...selected].filter((id) => {
        const m = flatMap.get(id);
        return m && !m.isSubmod && !requiredMods.includes(id);
      }),
    );
    const vcmi = toVcmiPreset(flat, [...baseMods, ...requiredMods, ...selected], managedRoots);
    const next = { ...presets, [activePreset]: { mods: vcmi.mods, settings: vcmi.settings ?? {} } };
    persistPresets(next, activePreset);
    setDirty(false);
  };

  const [builtinSaveOpen, setBuiltinSaveOpen] = useState<string | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<string | null>(null);

  /* ---------------- 模组仓库（获取与更新，mod-acquisition-update §9.1） ---------------- */
  // 索引条目（小写 id → 条目）；null = 尚未检查
  const [repoMap, setRepoMap] = useState<Map<string, ModRepoEntry> | null>(null);
  const [checkingUpdates, setCheckingUpdates] = useState(false);
  // P9（P10 简化：手动/静默统一底部悬浮进度条）：检查两阶段进度（index/meta）
  const [checkProgress, setCheckProgress] = useState<ModIndexProgress | null>(null);
  const [autoCheckOn, setAutoCheckOn] = useState(false);
  const [ledgers, setLedgers] = useState<Record<string, ModLedgerInfo>>({});
  // 启动器配置（阶段 1 读出）：管理页「安装位置」列需要用户/游戏模组目录前缀
  const cfgRef = useRef<{ userModsDir?: string; gameDir?: string } | null>(null);
  // P6（Q6 定案）：更新/安装行级队列——可连点多个入队，单 worker 串行执行，失败不中断
  const [modQueue, setModQueue] = useState<ModQueueItem[]>([]);
  const queueBusy = useRef(false);
  const repoMapRef = useRef(repoMap);
  repoMapRef.current = repoMap;
  const flatRef = useRef(flat);
  flatRef.current = flat;

  /** 可更新集合（仅当索引已拉取且预聚合了版本时有内容；官方索引无版本 → 空） */
  const repoUpdates = useMemo(
    () => (repoMap ? modUpdateMap(flat, repoMap) : {}),
    [repoMap, flat],
  );
  /** 仓库条目（抽屉展示用，含未安装/无版本条目） */
  const repoEntryOf = (id: string) => repoMap?.get(id.toLowerCase());

  /** 检查更新（入口在顶部预设栏「更多」菜单）：拉索引→重算可更新集；有缓存降级=绿横幅，彻底失败=红横幅；
   *  silent（启动自动检查）不弹横幅，只静默更新徽章。
   *  A1：`manual`（手动点检）忽略 TTL 新鲜度总是触发后台校验；`force` 阻塞全量（强制重新校验）。 */
  const handleCheckUpdates = async (force = false, silent = false, manual = false) => {
    if (!isTauri() || checkingUpdates) return;
    setCheckingUpdates(true);
    try {
      const idx = await fetchModIndex(force, manual);
      setRepoMap(modRepoIndex(idx.mods));
      if (idx.error && !silent) {
        setBanner({
          text: tRef.current.modsRepo.offline.replace("{msg}", idx.error),
          // 拿不到任何条目 = 真失败（红）；有缓存兜底 = 离线降级（绿）
          kind: idx.mods.length > 0 ? "success" : "error",
        });
      }
    } catch (e) {
      if (!silent) {
        setBanner({
          text: tRef.current.modsRepo.offline.replace("{msg}", backendErrorText(e)),
          kind: "error",
        });
      }
    } finally {
      setCheckingUpdates(false);
      // invoke resolve 是权威完成信号（事件丢帧也无害）：清进度，列表一次性刷新
      setCheckProgress(null);
    }
  };

  // P9：订阅检查进度事件（挂载期常驻；只在检查中由面板渲染，平时置 null 不显示）
  useEffect(() => {
    let unsub: (() => void) | null = null;
    let alive = true;
    void onModIndexProgress((p) => {
      if (alive) setCheckProgress(p);
    }).then((u) => {
      if (alive) unsub = u;
      else u();
    });
    return () => {
      alive = false;
      if (unsub) unsub();
    };
  }, []);

  // A1：SWR 后台校验发现远端索引变化——静默重拉（缓存已刷新，TTL 内秒回）并绿横幅提示
  useEffect(() => {
    let unsub: (() => void) | null = null;
    let alive = true;
    void onModIndexUpdated(() => {
      void (async () => {
        try {
          const idx = await fetchModIndex(false);
          if (!alive) return;
          setRepoMap(modRepoIndex(idx.mods));
          setBanner({ text: tRef.current.modsRepo.indexUpdated, kind: "success" });
        } catch {
          /* 静默：重拉失败（罕见）下次检查自会刷新 */
        }
      })();
    }).then((u) => {
      if (alive) unsub = u;
      else u();
    });
    return () => {
      alive = false;
      if (unsub) unsub();
    };
  }, []);

  // T1：机制同官方——仅当用户显式开启 autoCheckRepositories 时，启动就绪后自动跑一轮静默检查
  // （FV 默认关 = 纯手工；失败不打扰，徽章自然更新）。
  useEffect(() => {
    if (boot !== "ready" || !isTauri()) return;
    let cancelled = false;
    void (async () => {
      try {
        const on = await autoCheckReposEnabled();
        if (cancelled) return;
        setAutoCheckOn(on);
        if (on) await handleCheckUpdates(false, true);
      } catch {
        /* 自动检查失败静默（手工入口会展示错误） */
      }
    })();
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [boot]);

  /**
   * 仓库安装/更新（抽屉与批量弹窗共用的落盘后的状态收尾）：
   * - 更新（replaced=true）：选中集不动（启用状态保持）；
   * - 新装（replaced=false）：仅重扫入库，**不自动启用**（D1）。
   */
  const applyRepoResults = async (
    results: { modId: string; name: string; replaced: boolean }[],
    note?: string,
  ) => {
    if (results.length === 0) return;
    try {
      const realMods = (await rescanRealMods()) as ModInfo[];
      setRootMods(realMods);
      const first = results[0];
      const base = first.replaced
        ? tRef.current.modsRepo.updatedNote.replace("{name}", first.name || first.modId)
        : tRef.current.modsRepo.installedNote.replace("{name}", first.name || first.modId);
      setBanner({
        text: note ? `${base} —— ${note}` : base,
        kind: note ? "error" : "success",
      });
    } catch (e) {
      console.error("[ModView] 仓库安装后重扫失败:", e);
    }
  };

  const handleRepoInstall = async (modId: string): Promise<boolean> => {
    if (!isTauri()) return false;
    try {
      const r = await installModFromRepo(modId);
      // N12：镜像+无 sha256 等弱校验告警（r.warn）随安装结果一并展示
      await applyRepoResults(
        [{ modId: r.modId, name: r.modName, replaced: r.replaced }],
        r.warn && r.warn.length ? r.warn.join("；") : undefined,
      );
      return true;
    } catch (e) {
      const msg = backendErrorText(e);
      if (!msg.includes(MOD_DOWNLOAD_CANCELLED)) {
        setBanner({
          text: tRef.current.modsRepo.failed.replace("{name}", modId).replace("{msg}", msg),
          kind: "error",
        });
      }
      return false;
    }
  };

  /** 入队安装/更新（管理页行操作）；同 id 已在队（queued/running）则忽略 */
  const enqueueRepo = (modId: string, name: string) => {
    setModQueue((q) =>
      q.some((x) => x.id === modId && (x.state === "queued" || x.state === "running"))
        ? q
        : [...q.filter((x) => !(x.id === modId && (x.state === "done" || x.state === "fail"))), { id: modId, name, state: "queued" as const }],
    );
  };
  /** 取消尚未执行的排队项 */
  const dequeueRepo = (modId: string) => {
    setModQueue((q) => q.filter((x) => !(x.id === modId && x.state === "queued")));
  };
  // 队列 worker：单并发串行；完成/失败态留在行上，全部结束后刷新台账
  useEffect(() => {
    if (queueBusy.current) return;
    const next = modQueue.find((x) => x.state === "queued");
    if (!next) return;
    queueBusy.current = true;
    void (async () => {
      setModQueue((q) => q.map((x) => (x.id === next.id ? { ...x, state: "running" as const } : x)));
      const ok = await handleRepoInstall(next.id);
      if (ok) void listModLedgers().then(setLedgers).catch(() => {}); // 安装/更新后台账已变，刷新来源列
      setModQueue((q) =>
        q.map((x) => (x.id === next.id ? { ...x, state: ok ? ("done" as const) : ("fail" as const) } : x)),
      );
      queueBusy.current = false;
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [modQueue]);

  /* ---------------- 预设导入（剪贴板分享 §5.2：缺失模组自动补装） ---------------- */
  // 待处理导入：`missing` = 引用了未安装、且仓库可补装的根模组 id（大小写按索引条目）。
  const [pendingImport, setPendingImport] = useState<{
    name: string;
    mods: string[];
    settings: Record<string, Record<string, boolean>>;
    missing: string[];
  } | null>(null);
  const [importInstalling, setImportInstalling] = useState(false);
  const [importProgress, setImportProgress] = useState<{ done: number; total: number }>({ done: 0, total: 0 });
  const importAbortRef = useRef(false);
  const importCurrentRef = useRef("");

  /** 落地一份导入的预设：写入预设字典 + 切为当前 + 解析选中集合（`flatArr` 用于补装后拿最新模组）。 */
  const applyImportedPreset = (
    name: string,
    mods: string[],
    settings: Record<string, Record<string, boolean>>,
    flatArr: ModInfo[] = flat,
  ) => {
    const next = { ...presets, [name]: { mods, settings } };
    persistPresets(next, name);
    const flatSel = fromVcmiPreset(flatArr, { mods, settings });
    setSelected(withRequired(excludeBaseSubmods(flatArr, flatSel, baseMods)));
    setDirty(false);
  };

  /**
   * 导入入口：解析 JSON → 检测未安装的根模组（排除基线/保留）→ 惰性拉索引判断能否从仓库补装。
   * 无可补装缺失 → 直接落地；有缺失 → 弹出补装确认层（补装并导入 / 直接导入 / 取消）。
   * 与 D1 的区分：预设导入是玩家显式意图，补装后直接按预设启用（不走「安装即禁用」）。
   */
  const handleImport = async (json: string) => {
    let obj: {
      name?: string;
      mods?: unknown;
      settings?: Record<string, Record<string, boolean>>;
    };
    try {
      obj = JSON.parse(json);
    } catch {
      setBanner({ text: tRef.current.preset.importErr, kind: "error" });
      return;
    }
    if (!obj || !Array.isArray(obj.mods)) {
      setBanner({ text: tRef.current.preset.importErr, kind: "error" });
      return;
    }
    const name = (obj.name || "imported").trim() || "imported";
    const mods = obj.mods as string[];
    const settings = obj.settings ?? {};
    const installedLower = new Set(flat.map((m) => m.id.toLowerCase()));
    const missing = [
      ...new Set(
        mods.filter(
          (id) =>
            !baseMods.includes(id) &&
            !RESERVED_MOD_IDS.has(id) &&
            !installedLower.has(id.toLowerCase()),
        ),
      ),
    ];
    // 惰性确保有索引（缺索时无法判断可补装性）；拉取失败降级为直接导入
    let idx = repoMapRef.current;
    if (missing.length && !idx) {
      try {
        const r = await fetchModIndex(false);
        idx = modRepoIndex(r.mods);
        setRepoMap(idx);
      } catch {
        idx = null;
      }
    }
    const installable = idx ? missing.filter((id) => idx?.get(id.toLowerCase())) : [];
    if (installable.length === 0) {
      applyImportedPreset(name, mods, settings);
      return;
    }
    setPendingImport({ name, mods, settings, missing: installable });
  };

  const abortImportInstall = () => {
    importAbortRef.current = true;
    if (importCurrentRef.current) void cancelModDownload(importCurrentRef.current);
  };

  /** 逐个从仓库补装缺失模组 → 重扫 → 落地导入的预设（部分失败不阻断导入）。 */
  const confirmInstallAndImport = async () => {
    if (!pendingImport) return;
    setImportInstalling(true);
    importAbortRef.current = false;
    const list = pendingImport.missing;
    setImportProgress({ done: 0, total: list.length });
    let ok = 0;
    let failed = 0;
    for (let i = 0; i < list.length; i++) {
      if (importAbortRef.current) break;
      const id = list[i];
      importCurrentRef.current = id;
      try {
        await installModFromRepo(id);
        ok += 1;
      } catch (e) {
        const m = backendErrorText(e);
        failed += 1;
        if (m.includes(MOD_DOWNLOAD_CANCELLED)) break;
      }
      setImportProgress({ done: i + 1, total: list.length });
    }
    importCurrentRef.current = "";
    let flatFresh = flat;
    try {
      const realMods = (await rescanRealMods()) as ModInfo[];
      setRootMods(realMods);
      flatFresh = flattenMods(realMods).filter((m) => !RESERVED_MOD_IDS.has(m.id));
    } catch (e) {
      console.error("[ModView] 补装后重扫失败:", e);
    }
    applyImportedPreset(pendingImport.name, pendingImport.mods, pendingImport.settings, flatFresh);
    setImportInstalling(false);
    setBanner({
      text: failed
        ? tRef.current.preset.missingPartial.replace("{n}", String(failed))
        : tRef.current.preset.missingDone.replace("{n}", String(ok)),
      kind: failed ? "error" : "success",
    });
    setPendingImport(null);
  };

  /** 不补装、直接导入（缺失模组留在预设里，由 VCMI 自身忽略）。 */
  const importOnly = () => {
    if (!pendingImport) return;
    applyImportedPreset(pendingImport.name, pendingImport.mods, pendingImport.settings);
    setPendingImport(null);
  };


  /** 打开模组管理页（P5）：每次进入刷新安装台账（本地批量读，成本低；更新后来源/时间会变） */
  const openManage = () => {
    onModeChange?.("manage");
    setModQueue([]); // 上轮队列结果不残留（P6）
    if (isTauri())
      void listModLedgers()
        .then(setLedgers)
        .catch(() => {});
  };

  /** 管理页删除回传（P6/Q7）：成功则重扫 + 从选中集移除（含子模组）+ 刷台账；预设经既有 [selected] 链自动写盘 */
  const handleModDeleted = async (id: string, ok: boolean, name: string) => {
    if (!ok) {
      setBanner({ text: tRef.current.modsManage.deleteFail.replace("{name}", name), kind: "error" });
      return;
    }
    try {
      const realMods = (await rescanRealMods()) as ModInfo[];
      setRootMods(realMods);
      setSelected((prev) => new Set([...prev].filter((x) => x !== id && !x.startsWith(`${id}.`))));
      setBanner({ text: tRef.current.modsManage.deleteDone.replace("{name}", name), kind: "success" });
    } catch (e) {
      console.error("[ModView] 删除后重扫失败:", e);
    }
    if (isTauri()) void listModLedgers().then(setLedgers).catch(() => {});
  };

  /**
   * 蒙层防闪烁：等待很短时不该闪一下蒙层 —— 延迟 200ms 再显示。
   *（等待期间内容区是纯背景，与 wiki 页等待的观感一致。）
   */
  const [overlayVisible, setOverlayVisible] = useState(false);
  useEffect(() => {
    if (boot === "ready") {
      setOverlayVisible(false);
      return;
    }
    const timer = window.setTimeout(() => setOverlayVisible(true), 200);
    return () => window.clearTimeout(timer);
  }, [boot]);

  /**
   * 列表未就绪（读配置 → 扫描 → 预设解析）时**只渲染蒙层**（早返回，与 `WikiView` 同一范式）：
   * 被蒙层遮住的 PresetBar / ModBrowser 照样渲染、照样挂载卡片（图片 IPC 与 5s 重试不停），
   * 用户却完全看不见 —— 纯浪费（见 `docs/features/startup-responsiveness.zh-CN.md` U1）。
   *
   * ⚠️ 本页列表字段（名称 / 依赖 / 冲突 / 子模组层级）**全部来自 `scan_mods`，与 wiki 缓存无关**：
   * 蒙层只等列表就绪，**绝不**等缓存阶段（等缓存对列表零增益，见 `ModBrowser` 顶部注释）。
   */
  if (boot !== "ready") {
    return (
      <div className="relative flex h-full p-3">
        {overlayVisible && <LoadingOverlay title={t.mods.analyzing} hint={t.mods.analyzingHint} />}
      </div>
    );
  }

  /** 报告输入（只读快照：磁盘预设 + 当前扫描结果）。可更新集合 → `id: 远端版本`，
      无版本号的条目不进报告（避免"+0 个可更新"噪音）。 */
  const reportInput = (): PresetReportInput => {
    const p = presets[activePreset];
    const updates: Record<string, string> = {};
    for (const [id, u] of Object.entries(repoUpdates)) {
      const v = (u.entry?.version ?? "").trim();
      if (v) updates[id] = v;
    }
    return {
      presetLabel: isBuiltinPreset(activePreset)
        ? builtinPresetLabel(activePreset, lang)
        : activePreset,
      builtin: isBuiltinPreset(activePreset),
      mods: p?.mods ?? [],
      settings: p?.settings ?? {},
      allMods: allFlat,
      baseMods,
      requiredMods,
      updates,
      engineVersion: getEngineVersionSnapshot() ?? null,
      vcmiLang,
      lang,
      t: reportTextsFrom(t),
    };
  };

  /** 报告 Markdown：随模式切换重算（纯字符串拼接，开销可忽略） */
  const reportMd = reportOpen ? buildPresetReport(reportInput(), reportMode) : "";

  /** 复制报告（反馈走自动消失 Toast；失败带原因） */
  const copyReport = async () => {
    try {
      await navigator.clipboard.writeText(reportMd);
      setToast({ id: Date.now(), text: t.preset.reportCopied, kind: "success" });
    } catch (e) {
      setToast({
        id: Date.now(),
        text: t.preset.clipboardFail.replace("{msg}", backendErrorText(e)),
        kind: "error",
      });
    }
  };

  return (
    <div className="relative flex h-full flex-col">
      {/* 预设栏仅预设态展示；管理页自带顶部操作栏（返回+动作）。banner 为 fixed 浮层，两态共用 */}
      {mode === "presets" && (
      <PresetBar
        presetName={activePreset}
        presets={presets}
        dirty={dirty}
        loading={boot !== "ready"}
        lang={lang}
        onManage={openManage}
        manageBadge={autoCheckOn ? Object.keys(repoUpdates).length : 0}
        onSave={handleSave}
        onSwitch={(name) => {
          const p = presets[name];
          if (!p && !isBuiltinPreset(name)) return;
          setActivePreset(name);
          // 内置预设只补基线、不删磁盘上已注入的语言包等额外 mod（避免切预设时抹掉）
          const rawMods = isBuiltinPreset(name)
            ? ensureBaseline(p?.mods ?? [], builtinPresetMods(name, lang))
            : p.mods;
          const mods = isBuiltinPreset(name) ? resolvePresetIds(rawMods, flat) : rawMods;
          const flatSel = fromVcmiPreset(flat, { mods, settings: p?.settings ?? {} });
          setSelected(withRequired(excludeBaseSubmods(flat, flatSel, baseMods)));
          setDirty(false);
          writePresetsSafe(name, presets);
        }}
        onCreate={(name, inheritFrom) => {
          const srcMods =
            inheritFrom === "__pure__" ? baseMods : presets[inheritFrom]?.mods ?? baseMods;
          const srcSettings =
            inheritFrom === "__pure__" ? {} : presets[inheritFrom]?.settings ?? {};
          const next = { ...presets, [name]: { mods: [...srcMods], settings: { ...srcSettings } } };
          persistPresets(next, name);
          const flatSel = fromVcmiPreset(flat, { mods: srcMods, settings: srcSettings });
          setSelected(withRequired(excludeBaseSubmods(flat, flatSel, baseMods)));
          setDirty(false);
        }}
        onRename={(oldName, newName) => {
          if (!newName || newName === oldName || presets[newName]) return;
          if (isBuiltinPreset(oldName)) return;
          const next = { ...presets };
          next[newName] = presets[oldName];
          delete next[oldName];
          persistPresets(next, newName);
        }}
        onDelete={(name) => setDeleteTarget(name)}
        onImport={(json) => void handleImport(json)}
        onNotify={(msg, kind) => setToast({ id: Date.now(), text: msg, kind: kind ?? "success" })}
        onExport={() => {
          const p = presets[activePreset];
          return p ? { name: activePreset, mods: p.mods, settings: p.settings } : null;
        }}
        onReportOpen={() => setReportOpen(true)}
      />
      )}

      {/* 守卫/维护提示：fixed 浮层（顶栏下方居中），脱离文档流、不挤压页面布局。
          5 秒后自动关闭，点击内容可复制到剪贴板；下一次成功的勾选（handleToggle）也会自动清除。 */}
      {banner && (
        <Toast
          text={banner.text}
          kind={banner.kind}
          duration={5000}
          resetKey={banner.text}
          className="z-[60]"
          onDone={() => setBanner(null)}
          onClose={() => setBanner(null)}
        />
      )}

      {/* 轻提示（自动消失）：复制/粘贴等瞬时反馈。z-[160] 高于报告面板（z-[151]），
          保证在报告面板里点「复制 Markdown」也能看到反馈。 */}
      {toast && (
        <Toast
          text={toast.text}
          kind={toast.kind}
          resetKey={toast.id}
          onDone={() => setToast((cur) => (cur && cur.id === toast.id ? null : cur))}
        />
      )}

      {mode === "manage" ? (
        <ModManagerPanel
          mods={rootMods}
          repoUpdates={repoUpdates}
          repoMap={repoMap}
          ledgers={ledgers}
          checkingUpdates={checkingUpdates}
          checkProgress={checkProgress}
          autoCheckOn={autoCheckOn}
          queue={modQueue}
          userModsDir={cfgRef.current?.userModsDir ?? ""}
          gameModsDir={cfgRef.current?.gameDir ? `${cfgRef.current.gameDir.replace(/[/\\]+$/, "")}/Mods` : ""}
          onBack={() => onModeChange?.("presets")}
          onCheckUpdates={() => void handleCheckUpdates(false, false, true)}
          onEnqueue={enqueueRepo}
          onDequeue={dequeueRepo}
          onToggleAutoCheck={setAutoCheckOn}
          onRevalidate={() => void handleRevalidate()}
          onDeleted={(id, ok, name) => void handleModDeleted(id, ok, name)}
        />
      ) : (
      <ModBrowser
        mods={allFlat}
        selected={selected}
        baseMods={baseMods}
        lockedMods={lockedMods}
        loading={boot !== "ready"}
        onChange={handleToggle}
        onDependencyBlocked={(msg) => setBanner({ text: msg, kind: "error" })}
      />
      )}

      {/* 预设导入缺失补装确认层（剪贴板分享 §5.2）：z-[140] */}
      {pendingImport && (
        <>
          <div
            className="fixed inset-0 z-[139] bg-black/60 backdrop-blur-sm"
            onClick={importInstalling ? undefined : () => setPendingImport(null)}
          />
          <div className="fixed left-1/2 top-1/2 z-[140] flex max-h-[86vh] w-[min(92vw,520px)] -translate-x-1/2 -translate-y-1/2 flex-col overflow-hidden rounded-2xl border border-slate-700/60 bg-slate-950/97 shadow-2xl backdrop-blur-xl">
            <header className="border-b border-slate-800 px-5 py-4">
              <div className="text-base font-semibold">{t.preset.missingTitle}</div>
              <div className="mt-0.5 text-xs text-slate-400">
                {t.preset.missingIntro.replace("{n}", String(pendingImport.missing.length))}
              </div>
            </header>
            <div className="min-h-0 flex-1 overflow-y-auto px-5 py-3">
              <div className="flex flex-wrap gap-1.5">
                {pendingImport.missing.map((id) => (
                  <span
                    key={id}
                    className="rounded-md border border-slate-700/60 bg-slate-800/60 px-2 py-0.5 text-xs text-slate-200"
                    title={repoEntryOf(id)?.name ?? id}
                  >
                    {repoEntryOf(id)?.name ?? id}
                  </span>
                ))}
              </div>
              {importInstalling && (
                <div className="mt-4">
                  <div className="mb-1 text-xs text-slate-400">
                    {t.preset.installing
                      .replace("{done}", String(importProgress.done))
                      .replace("{total}", String(importProgress.total))}
                  </div>
                  <div className="h-1.5 w-full overflow-hidden rounded bg-slate-800">
                    <div
                      className="h-full bg-blue-500/80 transition-[width]"
                      style={{
                        width: `${importProgress.total > 0 ? Math.floor((importProgress.done / importProgress.total) * 100) : 0}%`,
                      }}
                    />
                  </div>
                </div>
              )}
            </div>
            <footer className="flex items-center justify-end gap-2 border-t border-slate-800 px-5 py-3.5">
              {importInstalling ? (
                <button
                  type="button"
                  onClick={abortImportInstall}
                  className="rounded-lg border border-red-500/50 px-4 py-2 text-sm font-medium text-red-300 hover:bg-red-500/15"
                >
                  {t.common.cancel}
                </button>
              ) : (
                <>
                  <button
                    type="button"
                    onClick={() => setPendingImport(null)}
                    className="rounded-lg px-3 py-2 text-sm text-slate-400 hover:bg-slate-800"
                  >
                    {t.common.cancel}
                  </button>
                  <button
                    type="button"
                    onClick={importOnly}
                    className="rounded-lg border border-slate-700/60 bg-slate-800/60 px-4 py-2 text-sm font-medium text-slate-300 hover:bg-slate-800"
                  >
                    {t.preset.missingOnly}
                  </button>
                  <button
                    type="button"
                    onClick={() => void confirmInstallAndImport()}
                    className="rounded-lg bg-gradient-to-br from-blue-500 to-indigo-500 px-4 py-2 text-sm font-semibold text-white shadow-lg shadow-blue-600/30"
                  >
                    {t.preset.missingInstall}
                  </button>
                </>
              )}
            </footer>
          </div>
        </>
      )}

      <ConfirmDialog
        open={!!deleteTarget}
        title={t.preset.confirmDelete.replace("{name}", deleteTarget || "")}
        description={t.preset.confirmDeleteDesc}
        confirmText={t.common.ok}
        cancelText={t.common.cancel}
        danger
        onCancel={() => setDeleteTarget(null)}
        onConfirm={() => {
          const name = deleteTarget;
          if (!name) return;
          if (isBuiltinPreset(name)) return;
          const next = { ...presets };
          delete next[name];
          let nextActive = activePreset;
          if (!next[activePreset]) {
            nextActive = BUILTIN_PRESETS[0].id;
            if (!next[nextActive]) nextActive = Object.keys(next)[0];
            const nextMods =
              next[nextActive]?.mods ?? builtinPresetMods(nextActive, lang) ?? [];
            const resolved = resolvePresetIds(nextMods, flat);
            setSelected(withRequired(resolved.filter((id) => !baseMods.includes(id))));
          }
          persistPresets(next, nextActive);
          setDeleteTarget(null);
        }}
      />

      <PresetSaveDialog
        open={!!builtinSaveOpen}
        inheritFrom={builtinSaveOpen ?? ""}
        inheritLabel={builtinSaveOpen ? builtinPresetLabel(builtinSaveOpen, lang) : ""}
        presets={Object.keys(presets)}
        onCancel={() => setBuiltinSaveOpen(null)}
        onConfirm={(name) => {
          if (!builtinSaveOpen || presets[name]) return;
          const next = {
            ...presets,
            [name]: { mods: [...baseMods, ...selected], settings: {} },
          };
          persistPresets(next, name);
          setDirty(false);
          setBuiltinSaveOpen(null);
        }}
      />

      {/* 报告（页内覆盖层）：盖住预设栏与列表（无需遮罩，面板不透明），
          顶部导航栏与底部工具栏保持可用；切页签会卸载本页，报告随之消失。 */}
      {reportOpen && (
        <ReportPanel
          markdown={reportMd}
          mode={reportMode}
          onModeChange={setReportMode}
          fullWidth={reportFullWidth}
          onToggleWidth={() => setReportFullWidth((v) => !v)}
          onCopy={() => void copyReport()}
          onClose={() => setReportOpen(false)}
        />
      )}
    </div>
  );
}
