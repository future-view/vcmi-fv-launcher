/**
 * 领域模组管理面板（realm-mods-management §5.2，P0-P2 全量）：
 * 「领域模组」tab 的管理视图——四标签页（已实装 / 可引用 / 可下载 / 可更新）。
 *
 * 语义（§2-§3，v3 §7.8）：引用 = 只申明领域依赖哪些「非领域目录」的模组——**纯台账申报、
 * 零物理动作**（user 源目录联接由激活领域时统一建立；game 源天然可见永不建链）；
 * 整合 = 复制本体到领域 Mods；领域位置 = 物理存在于领域 Mods 的模组（整合 ∪ 私有副本）。
 *
 * 落盘模型（§7.8）：引用 / 取消引用为纯台账（随「保存领域」落盘）；内容实体动作
 * （整合 / 删除 / 下载 / 导入）即时执行。本组件的 setBorrowed / setBundled / setPinned
 * 只改 RealmEditor 的编辑期状态，由 RealmEditor.performSave 随 update_realm 写入 realm.json。
 *
 * 数据源单一化：installed（list_realm_mods）由 RealmEditor 持有并传入，
 * 预设视图徽章与管理面板共用；内容实体动作后经 `reload` 回拉刷新（切换视图不重拉）。
 */
import { useEffect, useMemo, useRef, useState, useSyncExternalStore, type Dispatch, type SetStateAction } from "react";
import { AlertTriangle, Sparkles } from "lucide-react";
import { cn } from "@/lib/utils";
import { TYPE_ORDER } from "@/lib/minimapBase";
import { RESERVED_MOD_IDS } from "@/lib/modScope";
import type {
  ClosureInstallItem,
  ClosureRefItem,
  DepClosureResult,
  DepDefect,
  DepSolveResult,
  RootDepDefect,
} from "@/lib/modDeps";
import {
  changelogNewest,
  formatDownloadSize,
  isModUpdatable,
  modRepoIndex,
} from "@/lib/modRepo";
import { localizedDescription, localizedName, type ModInfo } from "@/lib/modScanner";
import { getVcmiLanguageSnapshot, subscribeVcmiLanguage } from "@/lib/vcmiLanguage";
import { fromVcmiPreset } from "@/lib/vcmiPreset";
import {
  MOD_DOWNLOAD_CANCELLED,
  cancelModDownload,
  installModFromRepo,
  listVcmiGameProcesses,
  onModDownloadProgress,
  onModIndexProgress,
  pickOpenDirectory,
  pickOpenPaths,
  readLauncherConfig,
  readModSettingsJson,
  realmModBundle,
  realmModDelete,
  realmModImport,
  realmModRef,
  realmModUnref,
  saveRealmModAutoCheck,
  type InstalledMod,
  type ModIndexProgress,
  type ModRepoEntry,
  type RealmImportResult,
  type RealmInstalledMod,
} from "@/lib/tauri";
import type { RealmModPin } from "@/lib/realm/types";
import { ModSourcePopover } from "@/features/mods/ModSourcePopover";
import { ModRowDrawer } from "@/features/mods/ModRowDrawer";
import type { Row } from "@/features/mods/ModManagerPanel";
import { ConfirmDialog } from "@/shared/ConfirmDialog";
import { DepClosureDialog } from "./DepClosureDialog";
import { DepResolveDialog } from "./DepResolveDialog";
import {
  DownloadableTable,
  InstalledTable,
  ReferableTable,
  UpdatableTable,
  pctOf,
  type DelDlgState,
  type DlMap,
  type QItem,
  type UnrefDlgState,
  type UpdRow,
} from "./RealmModTables";
import { installedToModInfo, realmModToModInfo } from "./editorShared";
import type { LangDict } from "@/i18n/types";
import { backendErrorText } from "@/lib/backendError";

export interface RealmModManagerPanelProps {
  t: LangDict;
  realmId: string;
  /** 领域已实装（list_realm_mods 结果，含子模组与缺失台账项） */
  installed: RealmInstalledMod[];
  /** 数据隔离是否生效（dataPath=realm 且 configPath=realm） */
  isolated: boolean;
  /** 全局已安装模组（可引用页数据源；含子模组） */
  globalMods: InstalledMod[];
  borrowed: Set<string>;
  bundled: Set<string>;
  pinned: RealmModPin[];
  setBorrowed: Dispatch<SetStateAction<Set<string>>>;
  setBundled: Dispatch<SetStateAction<Set<string>>>;
  setPinned: Dispatch<SetStateAction<RealmModPin[]>>;
  /** 从全部预设中清理这些根模组及其子模组（取消引用 / 删除的联动，§7.7） */
  purgePresets: (rootIds: string[]) => void;
  /** 从预设引用后的预设归属（§7.5 核对点 3）：create 缺省 = 合并进 `__realm__`；返回落定预设名 */
  adoptPresetRefs: (ids: string[], create?: string) => string;
  /** 刷新领域已实装（物理动作后回拉） */
  reload: () => Promise<void>;
  /** 仓库索引（null = 本轮尚未检查成功） */
  repoIndex: ModRepoEntry[] | null;
  repoBusy: boolean;
  /** 检查更新（`silent` = 静默：不弹完成/失败 toast，供「自动检测」触发） */
  onRepoCheck: (silent?: boolean) => void;
  showToast: (type: "success" | "error", msg: string) => void;
  /** 依赖缺陷求解结果（依赖处置体系，`lib/modDeps.ts`：行按钮 / 筛选 chip / 处理弹窗消费） */
  depSolve: DepSolveResult;
  /** 依赖处置（选项 A「实装 + 启用」）：把依赖补入受影响模组所在预设的激活集（编辑期状态，随保存落盘） */
  enableDepInPresets: (depId: string, presetNames: string[]) => void;
  /** 已忽略的依赖 id（小写）：缺陷求解 / 闭包静音；随「保存领域」落盘（`mods.depIgnores`） */
  depIgnores: Set<string>;
  /** 忽略依赖（编辑期状态）：写 depIgnores 静音；随「保存领域」落盘 */
  setDepIgnores: Dispatch<SetStateAction<Set<string>>>;
  /** 领域语言集合（小写：defaultLang + i18n 覆盖键）：语言分组 / 默认勾选依据 */
  langs: string[];
  /** 依赖闭包解析入口（一键解析计算核心；打开 / 实装变化时注入处置单） */
  resolveClosure: () => DepClosureResult;
  /** 预设页「去处理」携带的定位意图：落入「已实装」页 + 可选开启缺陷筛选 */
  jump?: { depFilter: boolean } | null;
  /** jump 消费完成回调（宿主清位，避免切视图重放） */
  onJumpHandled?: () => void;
  /** 返回「模组预设」视图 */
  onBack: () => void;
}

type ManageTab = "installed" | "referable" | "downloadable" | "updatable";

/** 根模组 id（`parent.child` → `parent`）。 */
const rootId = (id: string) => (id.includes(".") ? id.split(".")[0] : id);

export function RealmModManagerPanel(props: RealmModManagerPanelProps) {
  const {
    t,
    realmId,
    installed,
    isolated,
    globalMods,
    borrowed,
    bundled,
    pinned,
    setBorrowed,
    setBundled,
    setPinned,
    purgePresets,
    adoptPresetRefs,
    reload,
    repoIndex,
    repoBusy,
    onRepoCheck,
    showToast,
    depSolve,
    enableDepInPresets,
    depIgnores,
    setDepIgnores,
    langs,
    resolveClosure,
    jump,
    onJumpHandled,
    onBack,
  } = props;
  const m = t.realm.modsManage;
  const mm = t.modsManage;
  const vcmiLang = useSyncExternalStore(subscribeVcmiLanguage, getVcmiLanguageSnapshot) ?? "english";

  const [tab, setTab] = useState<ManageTab>("installed");
  const [query, setQuery] = useState("");
  const [typeFilter, setTypeFilter] = useState("all");
  /** 单发操作互斥（引用/整合/删除/更新确认执行中） */
  const [opBusy, setOpBusy] = useState(false);
  const [refAllBusy, setRefAllBusy] = useState(false);
  /** 行详情抽屉（同构 Row 复用模组管理页抽屉，§9-12） */
  const [detail, setDetail] = useState<{ row: Row; entry: ModRepoEntry | null } | null>(null);

  /* ---------------- 模组源弹层（抽取组件，与模组管理页共用） ---------------- */
  const [srcOpen, setSrcOpen] = useState(false);
  const srcWrapRef = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!srcOpen) return;
    const onDown = (e: MouseEvent) => {
      if (srcWrapRef.current && !srcWrapRef.current.contains(e.target as Node)) setSrcOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [srcOpen]);

  /* ---------------- 基础派生 ---------------- */
  const repoMap = useMemo(() => (repoIndex ? modRepoIndex(repoIndex) : null), [repoIndex]);
  /** 领域已装入（根模组）的 id 小写集合 */
  const realmLower = useMemo(() => new Set(installed.map((x) => x.id.toLowerCase())), [installed]);
  /** 领域已使用根模组（小写）：用于从可引用 / 可下载中排除 */
  const realmUsedRoots = useMemo(
    () => new Set(installed.map((x) => rootId(x.id).toLowerCase())),
    [installed],
  );
  /** 缺失台账项（引用/整合/副本在 user/game 与领域目录均解析不到 mod.json） */
  const missingRows = useMemo(() => installed.filter((x) => !x.metaOk), [installed]);
  const missingLower = useMemo(() => new Set(missingRows.map((x) => x.id.toLowerCase())), [missingRows]);
  const pinnedLower = useMemo(() => new Set(pinned.map((p) => p.id.toLowerCase())), [pinned]);
  /** 全局已安装模组（id 小写）→ 条目：「从模组预设引用」解析 id 用。
   * VCMI 预设写盘时 id 统一小写，磁盘目录可能保留驼峰（`marketoftime` → `marketOfTime`），
   * 故解析一律走小写索引，避免这类模组被静默丢弃（§7.5）。 */
  const gLowerMap = useMemo(() => new Map(globalMods.map((x) => [x.id.toLowerCase(), x])), [globalMods]);

  /** 依赖反查（§7.1：沿 depends 反查，递归到根模组口径）：返回传递闭包内依赖 target 的根模组 */
  const dependentsOf = (targetId: string): RealmInstalledMod[] => {
    const tid = targetId.toLowerCase();
    const roots = installed.filter((x) => !x.isSubmod);
    const picked = new Map<string, RealmInstalledMod>();
    let fringe = new Set<string>([tid]);
    let guard = 0;
    while (fringe.size && guard++ < 16) {
      const next = new Set<string>();
      for (const x of roots) {
        if (x.id.toLowerCase() === tid || picked.has(x.id)) continue;
        if ((x.depends ?? []).some((d) => fringe.has(rootId(d).toLowerCase()))) {
          picked.set(x.id, x);
          next.add(x.id.toLowerCase());
        }
      }
      fringe = next;
    }
    return [...picked.values()];
  };

  /* ---------------- 确认对话框状态 ---------------- */
  const [unrefDlg, setUnrefDlg] = useState<UnrefDlgState | null>(null);
  const [delDlg, setDelDlg] = useState<DelDlgState | null>(null);
  const [bundleDlg, setBundleDlg] = useState<{ mod: InstalledMod; reason: "game" | "diverged" } | null>(null);
  const [updDlg, setUpdDlg] = useState<UpdRow | null>(null);
  const [impDlg, setImpDlg] = useState<{ items: RealmImportResult[] } | null>(null);

  /* ---------------- 导入模组文件（§7.4） ---------------- */
  const [impMenuOpen, setImpMenuOpen] = useState(false);
  const impWrapRef = useRef<HTMLDivElement>(null);
  const [impBusy, setImpBusy] = useState(false);
  useEffect(() => {
    if (!impMenuOpen) return;
    const onDown = (e: MouseEvent) => {
      if (impWrapRef.current && !impWrapRef.current.contains(e.target as Node)) setImpMenuOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [impMenuOpen]);

  const pinFromInstall = (r: { modId: string; modName: string; version: string; sha256: string }) => {
    setPinned((prev) => [
      ...prev.filter((p) => p.id.toLowerCase() !== r.modId.toLowerCase()),
      {
        id: r.modId,
        name: r.modName,
        version: r.version,
        source: "repo",
        sha256: r.sha256,
        pinnedAt: new Date().toISOString().slice(0, 10),
      },
    ]);
  };

  /** 导入（zip / 目录）公共执行：overwrite=false 先探询冲突 */
  const runImport = async (paths: string[], overwrite: boolean) => {
    setImpBusy(true);
    try {
      const results = await realmModImport(realmId, paths, overwrite);
      const conflicts = results.filter((r) => !r.ok && r.exists);
      if (!overwrite && conflicts.length > 0) {
        setImpDlg({ items: conflicts });
        setImpBusy(false);
        return;
      }
      const ok = results.filter((r) => r.ok);
      const failed = results.filter((r) => !r.ok && !r.exists);
      if (ok.length > 0) {
        // 导入落 pinned 台账（source: "import"；条目由前端随保存落盘）
        const today = new Date().toISOString().slice(0, 10);
        setPinned((prev) => {
          const next = [...prev];
          for (const r of ok) {
            if (!r.modId) continue;
            const item: RealmModPin = {
              id: r.modId,
              name: r.name,
              version: r.version,
              source: "import",
              pinnedAt: today,
            };
            const i = next.findIndex((p) => p.id.toLowerCase() === r.modId!.toLowerCase());
            if (i >= 0) next[i] = item;
            else next.push(item);
          }
          return next;
        });
        // P4 轻告知 watch：reload 落定后自动求解——新缺陷由消费 effect 延迟补提示
        depWatchRef.current = {
          ids: new Set(ok.flatMap((r) => (r.modId ? [r.modId.toLowerCase()] : []))),
          atInstalled: installed,
        };
        showToast("success", m.impDone.replace("{n}", String(ok.length)));
        await reload();
      }
      if (failed.length > 0) {
        const reasons = failed
          .map((r) => {
            const why = r.reason === "not-mod" ? m.impReasonNotMod : (r.reason ?? "");
            return `· ${r.path.split(/[\\/]/).pop() || r.path}${why ? ` — ${why}` : ""}`;
          })
          .join("\n");
        showToast("error", `${m.impFailTitle}\n${reasons}`);
      }
      if (overwrite && conflicts.length > 0) {
        // 替换分支：conflicts 已在本次 overwrite=true 处理（ok 或 failed 已归类）
      }
      if (!overwrite && conflicts.length === 0) setImpMenuOpen(false);
    } catch (e) {
      showToast("error", `${m.impFailTitle}：${backendErrorText(e)}`);
    } finally {
      setImpBusy(false);
    }
  };

  const importFromZip = async () => {
    setImpMenuOpen(false);
    const files = await pickOpenPaths([{ name: "VCMI Mod", extensions: ["zip"] }]);
    if (files.length > 0) await runImport(files, false);
  };
  const importFromDir = async () => {
    setImpMenuOpen(false);
    const dir = await pickOpenDirectory();
    if (dir) await runImport([dir], false);
  };

  /* ---------------- 下载队列（可下载页安装，行级串行 worker） ---------------- */
  const [queue, setQueue] = useState<QItem[]>([]);
  const workerRef = useRef(false);
  /** `mod-download-progress` 事件累积（key = 模组 id 小写）：行内展示下载百分比与解包阶段。 */
  const [dlMap, setDlMap] = useState<DlMap>({});
  useEffect(() => {
    let unsub: (() => void) | null = null;
    let alive = true;
    void onModDownloadProgress((p) => {
      if (alive) {
        setDlMap((m0) => ({
          ...m0,
          [p.modId.toLowerCase()]: { received: p.received, total: p.total, done: !!p.done },
        }));
      }
    }).then((u) => {
      if (alive) unsub = u;
      else u();
    });
    return () => {
      alive = false;
      if (unsub) unsub();
    };
  }, []);
  /** 检查更新进度（mod-index-progress 两阶段事件，P9/P10 定案）：检查期间底部悬浮胶囊
   * 展示「拉取索引 / 补全元数据」进度；repoBusy 结束后清空（避免下轮检查闪现旧值）。 */
  const [repoProgress, setRepoProgress] = useState<ModIndexProgress | null>(null);
  useEffect(() => {
    let unsub: (() => void) | null = null;
    let alive = true;
    void onModIndexProgress((p) => {
      if (alive) setRepoProgress(p);
    }).then((u) => {
      if (alive) unsub = u;
      else u();
    });
    return () => {
      alive = false;
      if (unsub) unsub();
    };
  }, []);
  useEffect(() => {
    if (!repoBusy) setRepoProgress(null);
  }, [repoBusy]);
  /** 「自动检测更新」（launcher.json `realms.mod_auto_check`，默认开）：每次进入管理视图静默检查一次
   * ——面板挂载期去重（切走再进会重跑，由后端 TTL（30 分钟）缓存/SWR 兜底，近乎零成本），失败静默不打扰。 */
  const [autoCheckOn, setAutoCheckOn] = useState(true);
  const [autoCheckBusy, setAutoCheckBusy] = useState(false);
  const autoCheckedRef = useRef(false);
  useEffect(() => {
    if (autoCheckedRef.current) return;
    autoCheckedRef.current = true;
    void (async () => {
      let on = true;
      try {
        const cfg = await readLauncherConfig();
        on = cfg?.realmsModAutoCheck !== false;
      } catch {
        /* 配置读取失败按默认开处理 */
      }
      setAutoCheckOn(on);
      if (on) onRepoCheck(true);
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  /** 写盘成功才更新勾选态（失败不动，下次读取仍是磁盘真值）。 */
  const toggleAutoCheck = async (on: boolean) => {
    setAutoCheckBusy(true);
    try {
      await saveRealmModAutoCheck(on);
      setAutoCheckOn(on);
    } catch {
      /* 写失败不动开关 */
    } finally {
      setAutoCheckBusy(false);
    }
  };
  const queueOf = (id: string) => queue.find((x) => x.id.toLowerCase() === id.toLowerCase());
  /** 队列忙（有排队/执行中任务）：与删除 / 整合 / 取消引用等物理单发操作互斥。 */
  const queueBusy = queue.some((x) => x.state === "running" || x.state === "queued");
  /** 运行中的动作文案：下载百分比 → 安装中（解包）→ 兜底「执行中」。 */
  const progressLabel = (id: string, fallback: string) => {
    const dl = dlMap[id.toLowerCase()];
    if (dl?.done) return mm.qInstalling;
    if (dl && dl.total > 0) return mm.qDownloading.replace("{pct}", String(pctOf(dl)));
    return fallback;
  };
  /** 入队（同 id 排队/执行中忽略；已完成/失败的旧项被替换——支持失败后重试）。
   * `upd` 存在 = 更新任务（worker 走「取消引用 → 装副本 → 入台账」链路）。 */
  const enqueue = (id: string, name: string, upd?: UpdRow) => {
    const lower = id.toLowerCase();
    setDlMap((m0) => {
      if (!(lower in m0)) return m0;
      const n = { ...m0 };
      delete n[lower];
      return n;
    });
    setQueue((q) => {
      const cur = q.find((x) => x.id.toLowerCase() === lower);
      if (cur && (cur.state === "queued" || cur.state === "running")) return q;
      return [...q.filter((x) => x.id.toLowerCase() !== lower), { id, name, state: "queued", kind: upd ? "update" : "install", upd }];
    });
  };
  const dequeue = (id: string) => setQueue((q) => q.filter((x) => x.id.toLowerCase() !== id.toLowerCase()));
  /** 「取消」：排队中直接移除；下载中先请求后端取消（解包阶段不可取消，故不展示）。 */
  const cancelQueued = (id: string) => {
    if (queueOf(id)?.state === "running") void cancelModDownload(id);
    dequeue(id);
  };
  /** 安装/更新前的进程守卫：游戏本体运行中直接给出可读提示（后端亦有兜底守卫）。
   *  官方启动器不算——它不持有游戏状态，与领域操作可共存（§4.4 分级守卫）。 */
  const guardVcmi = async (): Promise<boolean> => {
    const procs = await listVcmiGameProcesses();
    if (procs.length > 0) {
      showToast("error", m.needExitGame.replace("{procs}", procs.join("、")));
      return false;
    }
    return true;
  };
  /** 可下载行播放入口：先过进程守卫再入队。 */
  const enqueueFromUi = async (entry: ModRepoEntry) => {
    if (!(await guardVcmi())) return;
    enqueue(entry.id, entry.name || entry.id);
  };
  /** 可更新行播放入口（确认弹窗确认后）：先过进程守卫再入队——与「可下载」同队列串行，
   * 可连点多个排队；执行开始后不可取消（引用态已取消引用，中途取消会悬空）。 */
  const enqueueUpdate = async (row: UpdRow) => {
    if (!(await guardVcmi())) return;
    enqueue(row.mod.id, shownNameOfRealm(row.mod), row);
  };
  useEffect(() => {
    if (workerRef.current) return;
    const next = queue.find((x) => x.state === "queued");
    if (!next) return;
    workerRef.current = true;
    void (async () => {
      setQueue((q) => q.map((x) => (x === next ? { ...x, state: "running" } : x)));
      const upd = next.upd;
      /** 更新任务：引用态已取消引用（供失败回滚判定） */
      let unrefDone = false;
      try {
        if (upd && upd.mod.source !== "realm") {
          // 更新任务（引用态）：先取消引用（删链接），再装领域副本（避免 .fv-old 交换穿透联接）
          await realmModUnref(realmId, [upd.mod.id]);
          unrefDone = true;
          setBorrowed((prev) => {
            const n = new Set(prev);
            n.delete(upd.mod.id);
            return n;
          });
        }
        const r = await installModFromRepo(next.id, `realm:${realmId}`);
        if (next.kind === "update" && upd) {
          // 与旧 doUpdate 同口径：引用态（刚取消引用）或已 pinned → 更新后转领域专属副本（入台账）
          if (upd.mod.source !== "realm" || pinnedLower.has(upd.mod.id.toLowerCase())) pinFromInstall(r);
          setQueue((q) => q.map((x) => (x === next ? { ...x, state: "done" } : x)));
          showToast("success", m.updDone.replace("{name}", r.modName || r.modId));
        } else {
          pinFromInstall(r);
          setQueue((q) => q.map((x) => (x === next ? { ...x, state: "done" } : x)));
          showToast("success", t.modsRepo.installedNote.replace("{name}", r.modName || r.modId));
        }
        // P4 轻告知 watch：reload 落定后自动求解（覆盖「安装」与「更新」两分支）
        depWatchRef.current = { ids: new Set([next.id.toLowerCase()]), atInstalled: installed };
        await reload();
      } catch (e) {
        const msg = backendErrorText(e);
        // 更新失败：引用态已取消——尽力恢复引用，避免模组悬空（恢复失败静默，由失败提示兜底）
        if (unrefDone && upd) {
          try {
            await realmModRef(realmId, [upd.mod.id]);
            setBorrowed((prev) => new Set([...prev, upd.mod.id]));
          } catch {
            /* 恢复失败：保留失败提示，用户可在「可引用」页手动重新引用 */
          }
        }
        if (msg.includes(MOD_DOWNLOAD_CANCELLED)) {
          // 用户取消：行已在点击「取消」时移出队列，无需再报错
        } else {
          setQueue((q) => q.map((x) => (x === next ? { ...x, state: "fail" } : x)));
          showToast(
            "error",
            next.kind === "update" && upd
              ? m.updFail.replace("{name}", upd.mod.name || upd.mod.id).replace("{msg}", msg)
              : t.modsRepo.failed.replace("{name}", next.name).replace("{msg}", msg),
          );
        }
      } finally {
        workerRef.current = false;
        // 触发下一轮（排队项继续执行）
        setQueue((q) => [...q]);
      }
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [queue]);

  /* ---------------- 行数据 ---------------- */
  /** 已实装（根模组） */
  const installedRoots = useMemo(() => installed.filter((x) => !x.isSubmod), [installed]);
  /** 可引用：全局安装中未被领域使用的根模组（user/game 源；排除基础模组） */
  const referable = useMemo(
    () =>
      globalMods
        .filter((x) => !x.isSubmod && !RESERVED_MOD_IDS.has(x.id) && !realmUsedRoots.has(x.id.toLowerCase()))
        .filter((x) => x.source === "user" || x.source === "game"),
    [globalMods, realmUsedRoots],
  );
  /** 可下载：仓库索引 − 已实装（大小写不敏感；父已实装则子模组条目一并排除） */
  const downloadable = useMemo(() => {
    if (!repoMap) return [] as { entry: ModRepoEntry; repair: boolean }[];
    const out: { entry: ModRepoEntry; repair: boolean }[] = [];
    for (const [key, e] of repoMap) {
      if (realmUsedRoots.has(key)) continue;
      const id = (e.id || key).toLowerCase();
      out.push({ entry: e, repair: missingLower.has(id) });
    }
    return out;
  }, [repoMap, realmUsedRoots, missingLower]);
  /** 可更新：已实装（含 pinned）∩ 仓库更高版本；子模组不参与 */
  const updatable = useMemo(() => {
    if (!repoMap) return [] as UpdRow[];
    const out: UpdRow[] = [];
    for (const x of installedRoots) {
      if (!x.metaOk) continue;
      const e = repoMap.get(x.id.toLowerCase());
      if (!e?.version) continue;
      if (isModUpdatable(x.version ?? "", e.version)) {
        out.push({ mod: x, entry: e, cur: x.version ?? "", next: e.version });
      }
    }
    return out;
  }, [repoMap, installedRoots]);

  /** 同构 Row（§9-12：复用 ModRowDrawer） */
  const rowOfInstalled = (x: RealmInstalledMod): Row => {
    const e = repoMap?.get(x.id.toLowerCase());
    const up = !!e && isModUpdatable(x.version ?? "", e.version ?? "");
    const mi = realmModToModInfo(x);
    return {
      id: x.id,
      name: localizedName(mi, vcmiLang) || x.name || x.id,
      type: x.type ?? "",
      desc: localizedDescription(mi, vcmiLang),
      author: x.author ?? "",
      installedVersion: x.version ?? "",
      latestVersion: e?.version ?? "",
      status: up ? "updatable" : "installed",
      modPath: "",
      dirName: x.diskName || x.id,
      location: x.source === "realm" ? "realm" : x.source === "user" ? "user" : "game",
    };
  };
  /** 「可引用」行（InstalledMod）→ 选词用轻量 ModInfo（i18n 块 → 原文回退） */
  const miOfInstalled = (x: InstalledMod): ModInfo =>
    ({ name: x.name || x.id, desc: x.desc ?? "", i18n: x.i18n }) as ModInfo;
  /** 已实装行显示名（i18n 选词 → 原文 → id）：表格列与抽屉行同口径 */
  const shownNameOfRealm = (x: RealmInstalledMod) =>
    localizedName(realmModToModInfo(x), vcmiLang) || x.name || x.id;
  const rowOfReferable = (x: InstalledMod): Row => {
    const e = repoMap?.get(x.id.toLowerCase());
    const mi = miOfInstalled(x);
    return {
      id: x.id,
      name: localizedName(mi, vcmiLang) || x.name || x.id,
      type: x.type ?? "",
      desc: localizedDescription(mi, vcmiLang),
      author: x.author ?? "",
      installedVersion: x.version ?? x.meta?.version ?? "",
      latestVersion: e?.version ?? "",
      status: "installed",
      modPath: "",
      dirName: x.id,
      location: x.source === "game" ? "game" : "user",
    };
  };
  const rowOfDownloadable = (e: ModRepoEntry): Row => ({
    id: e.id,
    name: localizedName({ name: e.name || e.id, i18n: e.i18n } as ModInfo, vcmiLang) || e.name || e.id,
    type: e.modType ?? "",
    desc: localizedDescription({ desc: e.description ?? "", i18n: e.i18n } as ModInfo, vcmiLang),
    author: e.author ?? "",
    installedVersion: "",
    latestVersion: e.version ?? "",
    status: "downloadable",
    modPath: "",
    dirName: e.id,
    location: "unknown",
  });

  /* ---------------- 筛选（四页共用 query + typeFilter，切页保持） ---------------- */
  const q = query.trim().toLowerCase();
  const match = (parts: (string | undefined | null)[]) =>
    !q || parts.filter(Boolean).join(" ").toLowerCase().includes(q);
  /** i18n 块（任意语言）的名称/描述全文：模组搜索支持多语言命中（不仅限于英文原名）。 */
  const i18nHay = (
    i18n?: Record<string, { name?: string; description?: string } | null> | null,
  ): (string | undefined)[] =>
    i18n ? Object.values(i18n).flatMap((v) => [v?.name, v?.description]) : [];
  const visibleInstalled = useMemo(
    () => installedRoots.filter((x) => match([x.id, x.name, x.diskName, x.desc, ...i18nHay(x.i18n)])),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [installedRoots, q],
  );
  const visibleReferable = useMemo(
    () => referable.filter((x) => match([x.id, x.name, x.desc, ...i18nHay(x.i18n)])),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [referable, q],
  );
  const visibleDownloadable = useMemo(
    () =>
      downloadable.filter(({ entry }) =>
        match([entry.id, entry.name, entry.description, ...i18nHay(entry.i18n)]),
      ),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [downloadable, q],
  );
  const visibleUpdatable = useMemo(
    () => updatable.filter(({ mod }) => match([mod.id, mod.name, mod.diskName, mod.desc, ...i18nHay(mod.i18n)])),
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [updatable, q],
  );
  /** 当前页出现过的类型 chips（TYPE_ORDER 固定序） */
  const types = useMemo(() => {
    let present: Set<string>;
    if (tab === "installed") present = new Set(visibleInstalled.map((x) => x.type ?? "").filter(Boolean));
    else if (tab === "referable") present = new Set(visibleReferable.map((x) => x.type ?? "").filter(Boolean));
    else if (tab === "downloadable") present = new Set(visibleDownloadable.map((x) => x.entry.modType ?? "").filter(Boolean));
    else present = new Set(visibleUpdatable.map((x) => x.mod.type ?? "").filter(Boolean));
    return TYPE_ORDER.filter((x) => present.has(x));
  }, [tab, visibleInstalled, visibleReferable, visibleDownloadable, visibleUpdatable]);
  const typeOk = (ty: string | undefined) => typeFilter === "all" || (ty ?? "") === typeFilter;
  /** 「依赖缺陷」筛选 toggle（操作栏；跳转意图可自动开启）——声明须先于 typedInstalled（filter 回调立即求值，const 晚声明会踩 TDZ） */
  const [depOnly, setDepOnly] = useState(false);
  const typedInstalled = visibleInstalled.filter(
    (x) => typeOk(x.type) && (!depOnly || depSolve.byRoot.has(x.id.toLowerCase())),
  );
  const typedReferable = visibleReferable.filter((x) => typeOk(x.type));
  const typedDownloadable = visibleDownloadable.filter((x) => typeOk(x.entry.modType));
  const typedUpdatable = visibleUpdatable.filter((x) => typeOk(x.mod.type));
  /* 缺陷清零（修复 / 忽略完）自动撤下「依赖缺陷」筛选：toggle 随缺陷数隐藏，避免空表悬挂 */
  useEffect(() => {
    if (depOnly && depSolve.defectRootCount === 0) setDepOnly(false);
  }, [depOnly, depSolve.defectRootCount]);
  const visibleCount =
    tab === "installed" ? typedInstalled.length
    : tab === "referable" ? typedReferable.length
    : tab === "downloadable" ? typedDownloadable.length
    : typedUpdatable.length;

  /** 切页时重置类型筛选（chips 集合随页变化，避免空结果困惑） */
  const switchTab = (k: ManageTab) => {
    setTab(k);
    setTypeFilter("all");
  };

  /* ---------------- 动作：引用 / 取消引用 / 整合 / 删除 ---------------- */
  const doRef = async (ids: string[]) => {
    setOpBusy(true);
    try {
      // 纯台账申报（v3 §7.8）：后端只校验来源存在性，不建任何链接；清单入内存后由
      // RealmEditor.realmInstalledView 合并展示（随保存落 realm.json；目录联接等实际
      // 挂载由「激活领域」时 apply_realm_mods 统一执行）。
      const results = await realmModRef(realmId, ids);
      const ok = results.filter((r) => r.ok);
      const missing = results.filter((r) => !r.ok);
      if (ok.length > 0) {
        setBorrowed((prev) => {
          const n = new Set(prev);
          ok.forEach((r) => n.add(r.id));
          return n;
        });
        showToast("success", m.actRefAllDone.replace("{n}", String(ok.length)));
      }
      if (missing.length > 0) {
        showToast("error", m.refMissing.replace("{list}", missing.map((r) => r.id).join("、")));
      }
    } catch (e) {
      showToast("error", t.modsRepo.failed.replace("{name}", ids.join("、")).replace("{msg}", backendErrorText(e)));
    } finally {
      setOpBusy(false);
    }
  };

  const doUnref = async (mod: RealmInstalledMod, cascadeIds: string[]) => {
    setOpBusy(true);
    try {
      // 台账移除（v3 §7.8）：后端仅在领域激活态下同步删目录联接（未激活无物理动作）
      const all = [mod.id, ...cascadeIds];
      const results = await realmModUnref(realmId, all);
      const ok = results.filter((r) => r.ok).map((r) => r.id);
      setBorrowed((prev) => {
        const n = new Set(prev);
        ok.forEach((id) => n.delete(id));
        return n;
      });
      purgePresets(ok);
      await reload();
      showToast("success", m.unrefDone.replace("{n}", String(ok.length)));
    } catch (e) {
      showToast("error", t.modsRepo.failed.replace("{name}", mod.id).replace("{msg}", backendErrorText(e)));
    } finally {
      setOpBusy(false);
    }
  };

  const doBundle = async (mod: InstalledMod, force: boolean) => {
    setOpBusy(true);
    try {
      const r = await realmModBundle(realmId, mod.id, force);
      if (r.ok) {
        if (r.skipped === "same") {
          showToast("success", m.bundleSame.replace("{name}", mod.name || mod.id));
        } else {
          setBundled((prev) => new Set(prev).add(r.modId));
          showToast("success", m.bundleDone.replace("{name}", mod.name || mod.id));
        }
        await reload();
      } else if (r.reason === "game" || r.reason === "diverged") {
        setBundleDlg({ mod, reason: r.reason });
      } else {
        showToast("error", t.modsRepo.failed.replace("{name}", mod.id).replace("{msg}", r.reason ?? ""));
      }
    } catch (e) {
      showToast("error", t.modsRepo.failed.replace("{name}", mod.id).replace("{msg}", backendErrorText(e)));
    } finally {
      setOpBusy(false);
    }
  };

  const doDelete = async (mod: RealmInstalledMod, kind: "bundled" | "pinned" | "local") => {
    setOpBusy(true);
    try {
      await realmModDelete(realmId, mod.id, kind);
      const lower = mod.id.toLowerCase();
      setBundled((prev) => {
        const n = new Set(prev);
        for (const x of [...n]) if (x.toLowerCase() === lower) n.delete(x);
        return n;
      });
      setPinned((prev) => prev.filter((p) => p.id.toLowerCase() !== lower));
      purgePresets([mod.id]);
      await reload();
      showToast("success", m.delDone.replace("{name}", mod.name || mod.id));
    } catch (e) {
      showToast("error", m.delFail.replace("{name}", mod.name || mod.id).replace("{msg}", backendErrorText(e)));
    } finally {
      setOpBusy(false);
    }
  };

  /* ---------------- 动作：更新（§7.3 分支） ----------------
   * 「更新」动作已并入下载队列（见上方 worker 的 update 分支）：确认弹窗 → enqueueUpdate 入队
   * → 单 worker 串行执行「取消引用 → 装副本 → 入台账」。可连点多个排队；
   * 执行开始后不可取消（引用态已取消引用，中途取消会悬空）。 */

  /* ---------------- 动作：全部引用（§7.6） ---------------- */
  const doRefAll = async () => {
    const ids = typedReferable.map((x) => x.id);
    if (ids.length === 0) {
      showToast("error", m.actRefAllNone);
      return;
    }
    setRefAllBusy(true);
    try {
      await doRef(ids);
    } finally {
      setRefAllBusy(false);
    }
  };

  /* ---------------- 从模组预设引用（§7.5） ---------------- */
  const [prefOpen, setPrefOpen] = useState(false);
  const [prefList, setPrefList] = useState<{ id: string; mods: string[]; sub: Record<string, Record<string, boolean>> }[] | null>(null);
  const [prefSel, setPrefSel] = useState<string | null>(null);
  const [prefAdopt, setPrefAdopt] = useState<"merge" | "create">("merge");
  const [prefBusy, setPrefBusy] = useState(false);
  const [prefResult, setPrefResult] = useState<{ ok: number; already: number; missing: string[]; created?: string } | null>(null);

  useEffect(() => {
    if (!prefOpen) return;
    let alive = true;
    setPrefList(null);
    setPrefSel(null);
    setPrefResult(null);
    setPrefAdopt("merge");
    (async () => {
      try {
        const s = (await readModSettingsJson()) as { presets?: Record<string, { mods?: unknown; settings?: unknown }> } | null;
        const raw = s?.presets ?? {};
        const out = Object.entries(raw).map(([id, val]) => ({
          id,
          mods: (Array.isArray(val?.mods) ? val.mods : []).filter(
            (x): x is string => typeof x === "string" && !!x,
          ),
          sub: (val?.settings && typeof val.settings === "object" ? val.settings : {}) as Record<string, Record<string, boolean>>,
        }));
        if (alive) {
          setPrefList(out);
          if (out.length > 0) setPrefSel(out[0].id);
        }
      } catch {
        if (alive) setPrefList([]);
      }
    })();
    return () => {
      alive = false;
    };
  }, [prefOpen]);

  const runPresetRef = async () => {
    const sel = prefList?.find((x) => x.id === prefSel);
    if (!sel) return;
    setPrefBusy(true);
    try {
      // 展开子模组开关 → 扁平选中集合（磁盘写法 id：根 + 启用的子模组）。
      // 预设写盘 id 可能小写而磁盘驼峰，解析一律走小写索引（`gLowerMap`）；
      // 解析不到的 id（含保留模组）如实进 missing，不再静默丢弃。
      const flat = fromVcmiPreset(globalMods.map(installedToModInfo), { mods: sel.mods, settings: sel.sub });
      const resolved: InstalledMod[] = [];
      const missing = new Set<string>();
      const seen = new Set<string>();
      for (const id of flat) {
        if (RESERVED_MOD_IDS.has(id)) continue;
        const gm = gLowerMap.get(id.toLowerCase());
        if (!gm) {
          missing.add(id);
          continue;
        }
        if (gm.type === "Core") continue;
        if (!seen.has(gm.id.toLowerCase())) {
          seen.add(gm.id.toLowerCase());
          resolved.push(gm);
        }
      }
      // 引用只认根模组（子模组随父整模组，§3.4）：未在本领域的走引用；
      // 已在领域使用的免引用、直接并入组合——否则「已引用数 + 已并入数 ≠ 组合模组数」。
      const refRoots = [
        ...new Set(
          resolved
            .filter((x) => !x.isSubmod)
            .map((x) => x.id)
            .filter((r) => !realmUsedRoots.has(r.toLowerCase())),
        ),
      ];
      const alreadyRoots = new Set(
        resolved
          .filter((x) => !x.isSubmod && realmUsedRoots.has(x.id.toLowerCase()))
          .map((x) => x.id.toLowerCase()),
      );
      let okIds: string[] = [];
      if (refRoots.length > 0) {
        const results = await realmModRef(realmId, refRoots);
        okIds = results.filter((r) => r.ok).map((r) => r.id);
        results.filter((r) => !r.ok).forEach((r) => missing.add(r.id));
        if (okIds.length > 0) {
          setBorrowed((prev) => {
            const n = new Set(prev);
            okIds.forEach((id) => n.add(id));
            return n;
          });
        }
        await reload();
      }
      // 预设归属（核对点 3）：合并进 __realm__ 或创建同名领域预设（重名自动加后缀）。
      // 并入的是**完整组合**（根 + 启用子模组）：新引用的根与已在本领域的根，
      // 其子模组随根一并并入，保证子模组粒度不丢失（§7.5）。
      const mergeRoots = new Set([...okIds, ...alreadyRoots].map((x) => x.toLowerCase()));
      const mergeIds = resolved
        .filter((x) => mergeRoots.has(rootId(x.id).toLowerCase()))
        .map((x) => x.id);
      let created: string | undefined;
      if (mergeIds.length > 0) {
        const name = adoptPresetRefs(mergeIds, prefAdopt === "create" ? sel.id : undefined);
        if (prefAdopt === "create") created = name;
      }
      setPrefResult({ ok: okIds.length, already: alreadyRoots.size, missing: [...missing], created });
    } catch (e) {
      showToast("error", t.modsRepo.failed.replace("{name}", sel.id).replace("{msg}", backendErrorText(e)));
    } finally {
      setPrefBusy(false);
    }
  };

  /* ---------------- 缺失清单 hover（§5.2.1 核对点 8） ---------------- */
  const [missingOpen, setMissingOpen] = useState(false);
  const jumpToDownloadable = (id: string) => {
    switchTab("downloadable");
    setQuery(id);
    setMissingOpen(false);
  };

  /* ---------------- 依赖处置（依赖处置体系） ---------------- */
  /** 处理弹窗目标（根模组 id 小写；null = 未打开） */
  const [depDlg, setDepDlg] = useState<string | null>(null);
  /** 行级处理状态（key = 依赖 id 小写）：busy / done / fail（不可得项不产生状态） */
  const [depStates, setDepStates] = useState<Record<string, "busy" | "done" | "fail">>({});
  /** 闭包处置弹窗数据（null = 未打开；打开时解析注入，实装 / 忽略变化后收敛刷新） */
  const [closureData, setClosureData] = useState<DepClosureResult | null>(null);
  /** 闭包行级执行状态（key = rootKey / dep 小写） */
  const [closureStates, setClosureStates] = useState<Record<string, "busy" | "done" | "fail">>({});
  const [closureBusy, setClosureBusy] = useState(false);
  /* 弹窗打开中：实装集 / 忽略集变化 → 重新解析处置单（执行后收敛、忽略即时摘除；全清自动收起） */
  useEffect(() => {
    if (!closureData) return;
    const next = resolveClosure();
    setClosureData(next.refs.length + next.installs.length + next.deads.length === 0 ? null : next);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [installed, depIgnores]);
  // 预设页「去处理」跳转意图：落入「已实装」页 + 开启缺陷筛选（清空搜索，避免旧词遮蔽缺陷行）
  useEffect(() => {
    if (!jump) return;
    setTab("installed");
    setTypeFilter("all");
    setQuery("");
    setDepOnly(jump.depFilter);
    onJumpHandled?.();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [jump]);
  /** 依赖根 id 归一化到磁盘写法（引用 / 安装按磁盘 id 操作；求解器输出为小写匹配键） */
  const canonId = (id: string): string => {
    const lc = id.toLowerCase();
    return (
      installed.find((z) => z.id.toLowerCase() === lc)?.id ??
      globalMods.find((z) => z.id.toLowerCase() === lc)?.id ??
      repoMap?.get(lc)?.id ??
      id
    );
  };
  /**
   * 依赖处置单条执行（选项 A「实装 + 启用」两段式）：
   * ① 实装：installed 源 → 引入台账（`realmModRef`）；repo 源 → 安装领域副本（`installModFromRepo` + 入台账）。
   * ② 补入激活集：把依赖声明 id 补入受影响模组所在预设（`enableDepInPresets`）——
   *    引擎启动只认预设激活集（`ModManager::getActiveMods`），仅实装不启用仍会被 BROKEN 剔除。
   */
  const resolveDepOne = async (d: DepDefect) => {
    if (d.source === "unavailable") return;
    const key = d.dep.toLowerCase();
    const presetNames = [...new Set(d.affected.flatMap((a) => a.enabledIn))];
    const target = canonId(d.depRoot);
    const lc = d.depRoot.toLowerCase();
    const shown =
      d.entry?.name ||
      installed.find((z) => z.id.toLowerCase() === lc)?.name ||
      globalMods.find((z) => z.id.toLowerCase() === lc)?.name ||
      target;
    setDepStates((s) => ({ ...s, [key]: "busy" }));
    try {
      if (d.source === "installed") {
        const results = await realmModRef(realmId, [target]);
        if (results.filter((r) => r.ok).length === 0) {
          throw new Error(m.refMissing.replace("{list}", target));
        }
        setBorrowed((prev) => new Set([...prev, target]));
      } else {
        if (!(await guardVcmi())) {
          setDepStates((s) => ({ ...s, [key]: "fail" }));
          return;
        }
        const r = await installModFromRepo(target, `realm:${realmId}`);
        pinFromInstall(r);
        await reload();
      }
      enableDepInPresets(d.dep, presetNames);
      setDepStates((s) => ({ ...s, [key]: "done" }));
      showToast(
        "success",
        (d.source === "installed" ? m.depRefDone : m.depInstallDone).replace("{name}", shown),
      );
    } catch (e) {
      setDepStates((s) => ({ ...s, [key]: "fail" }));
      showToast(
        "error",
        t.modsRepo.failed.replace("{name}", shown).replace("{msg}", backendErrorText(e)),
      );
    }
  };
  /** 依赖处置批量执行（同源串行：逐个处理、逐行反馈；逐个 reload 后求解实时收敛） */
  const resolveDepAll = async (kind: "installed" | "repo") => {
    const info = depDlg ? depSolve.byRoot.get(depDlg) : null;
    if (!info) return;
    for (const d of info.defects) {
      if (d.source !== kind) continue;
      const st = depStates[d.dep.toLowerCase()];
      if (st === "done" || st === "busy") continue;
      await resolveDepOne(d);
    }
  };
  /** 当前处理弹窗的缺陷快照（reload 后求解收敛；根修复完自动从 byRoot 消失 → 弹窗自动关闭） */
  const depDlgInfo = depDlg ? depSolve.byRoot.get(depDlg) ?? null : null;
  const depBusy = Object.values(depStates).some((s) => s === "busy");
  /**
   * 忽略依赖（处理弹窗 / 闭包弹窗共用）：写 `depIgnores`（小写）静音缺陷，随「保存领域」
   * 落盘（`mods.depIgnores` 恒写数组）。依据 VCMI 引擎语义（`ModDependenciesResolver`）：
   * 依赖缺失仅将受影响模组标记 BROKEN 剔除、不阻断启动——忽略 = 纯 UI 静音，无需激活拦截。
   */
  const ignoreDeps = (deps: string[]) => {
    setDepIgnores((prev) => new Set([...prev, ...deps.map((d) => d.toLowerCase())]));
    showToast("success", m.depIgnoreDone.replace("{n}", String(deps.length)));
  };
  /** 打开闭包处置弹窗（一键解析）：以当前实装 / 预设 / 忽略集解析一次注入处置单 */
  const openClosure = () => {
    setClosureStates({});
    setClosureData(resolveClosure());
  };
  /**
   * 闭包一键执行（依赖处置 · 一键解析）：
   * ① 引用：逐根 `realmModRef` 台账 + 补入受影响预设激活集（逐行反馈）；
   * ② 安装：进程守卫一次前置，`installModFromRepo` + 入 pinned 台账 + 补入激活集；
   * ③ 完成后 reload 收敛处置单（实装集变化 → 上方刷新 effect）+ 汇总 toast。
   * 单项失败逐行标记不中断（引擎容忍缺失，剩余项仍可继续）。
   */
  const runDepClosure = async (plan: { refs: ClosureRefItem[]; installs: ClosureInstallItem[] }) => {
    setClosureBusy(true);
    let okRefs = 0;
    let okInstalls = 0;
    let failed = 0;
    try {
      for (const r of plan.refs) {
        setClosureStates((s) => ({ ...s, [r.rootKey]: "busy" }));
        try {
          const results = await realmModRef(realmId, [r.rootId]);
          if (results.filter((x) => x.ok).length === 0) {
            throw new Error(m.refMissing.replace("{list}", r.rootId));
          }
          setBorrowed((prev) => new Set([...prev, r.rootId]));
          for (const dep of r.deps) enableDepInPresets(dep, r.presetNames);
          setClosureStates((s) => ({ ...s, [r.rootKey]: "done" }));
          okRefs++;
        } catch {
          setClosureStates((s) => ({ ...s, [r.rootKey]: "fail" }));
          failed++;
        }
      }
      if (plan.installs.length > 0 && !(await guardVcmi())) {
        setClosureStates((s) => {
          const n = { ...s };
          for (const it of plan.installs) n[it.dep.toLowerCase()] = "fail";
          return n;
        });
        failed += plan.installs.length;
      } else {
        for (const it of plan.installs) {
          const key = it.dep.toLowerCase();
          setClosureStates((s) => ({ ...s, [key]: "busy" }));
          try {
            const r = await installModFromRepo(canonId(it.depRoot), `realm:${realmId}`);
            pinFromInstall(r);
            enableDepInPresets(it.dep, it.presetNames);
            setClosureStates((s) => ({ ...s, [key]: "done" }));
            okInstalls++;
          } catch {
            setClosureStates((s) => ({ ...s, [key]: "fail" }));
            failed++;
          }
        }
      }
      await reload();
      showToast(
        "success",
        m.depClosureDone.replace("{refs}", String(okRefs)).replace("{installs}", String(okInstalls)),
      );
      if (failed > 0) showToast("error", m.depPartFail);
    } finally {
      setClosureBusy(false);
    }
  };
  /**
   * P4 导入 / 下载完成轻告知（不弹窗）：完成后自动求解 → 徽章/角标数据驱动刷新；
   * 有新缺陷时在完成 toast 之后延迟补一条（复用 RealmShell duplicateMods 的 2.8s 先例）。
   * 机制：内容动作（导入 / 安装 / 更新）在 `reload` 前登记待观察模组 id 快照；
   * reload 落定（installed 引用变化）后查求解结果，命中缺陷才补提示。
   */
  const depWatchRef = useRef<{ ids: Set<string>; atInstalled: RealmInstalledMod[] } | null>(null);
  const depWatchTimerRef = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(depWatchTimerRef.current), []);
  useEffect(() => {
    const w = depWatchRef.current;
    if (!w || w.atInstalled === installed) return; // reload 尚未落定
    depWatchRef.current = null;
    const hits = [...w.ids]
      .map((id) => depSolve.byRoot.get(id))
      .filter((x): x is RootDepDefect => x !== undefined);
    if (hits.length === 0) return;
    const total = hits.reduce((a, b) => a + b.total, 0);
    depWatchTimerRef.current = window.setTimeout(() => {
      showToast("success", m.depWatchNote.replace("{name}", hits[0].rootName).replace("{n}", String(total)));
    }, 2800);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [depSolve, installed]);

  /* ---------------- 渲染 ---------------- */
  const tabs: [ManageTab, string, number | null][] = [
    ["installed", m.tabInstalled, installedRoots.length],
    ["referable", m.tabReferable, referable.length],
    ["downloadable", m.tabDownloadable, downloadable.length],
    ["updatable", m.tabUpdatable, updatable.length],
  ];
  const selPreset = prefList?.find((x) => x.id === prefSel) ?? null;

  return (
    <div className="relative flex h-[70vh] min-h-0 flex-col gap-2">
      {/* 顶部：返回预设视图 + 标题 */}
      <div className="flex flex-none items-center gap-3">
        <button
          type="button"
          onClick={onBack}
          className="rounded border border-slate-700 px-2.5 py-1 text-xs text-slate-300 hover:bg-slate-800/50"
        >
          ← {m.back}
        </button>
        <h3 className="text-sm font-semibold text-white">{m.title}</h3>
      </div>

      {/* 数据隔离警示（§8.4） */}
      {!isolated && (
        <div className="flex-none rounded-lg border border-amber-500/50 bg-amber-500/10 px-3 py-2 text-xs leading-5 text-amber-200">
          {m.banner}
        </div>
      )}

      {/* 四标签页 + 搜索框（搜索为四页共用筛选，切页保留；放到标签行右侧常驻） */}
      <div className="flex flex-none items-center gap-1.5 text-xs">
        {tabs.map(([k, label, count]) => (
          <button
            key={k}
            type="button"
            onClick={() => switchTab(k)}
            className={cn(
              "rounded-lg border px-3 py-1.5 transition-colors",
              tab === k
                ? "border-blue-500/60 bg-blue-500/15 text-white"
                : "border-slate-700/50 text-slate-400 hover:text-white",
            )}
          >
            {label}
            {count !== null && count > 0 && (
              <span
                className={cn(
                  "ml-1.5 rounded-full px-1.5 text-[10px] font-semibold",
                  k === "updatable" ? "bg-amber-500/90 text-black" : "bg-slate-700/80 text-slate-300",
                )}
              >
                {count}
              </span>
            )}
          </button>
        ))}
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={mm.search}
          className="ml-auto w-64 rounded-lg border border-slate-700 bg-slate-900/60 px-3 py-1.5 text-xs text-white placeholder:text-slate-500 focus:border-blue-500 focus:outline-none"
        />
      </div>

      {/* 工具栏（按页变化） */}
      <div className="flex flex-none flex-wrap items-center gap-2">
        {tab === "installed" && (
          <>
            <div ref={impWrapRef} className="relative">
              <button
                type="button"
                onClick={() => setImpMenuOpen((o) => !o)}
                disabled={impBusy}
                className="rounded border border-emerald-600/70 px-2.5 py-1 text-xs text-emerald-300 hover:bg-emerald-600/15 disabled:opacity-40"
              >
                {impBusy ? t.modsRepo.checking : m.actImport}
              </button>
              {impMenuOpen && !impBusy && (
                <div className="absolute left-0 top-full z-[60] mt-1 w-44 overflow-hidden rounded-lg border border-slate-700 bg-slate-900 py-1 shadow-xl">
                  <button
                    type="button"
                    onClick={() => void importFromZip()}
                    className="block w-full px-3 py-1.5 text-left text-xs text-slate-200 hover:bg-slate-800"
                  >
                    {m.importZip}
                  </button>
                  <button
                    type="button"
                    onClick={() => void importFromDir()}
                    className="block w-full px-3 py-1.5 text-left text-xs text-slate-200 hover:bg-slate-800"
                  >
                    {m.importDir}
                  </button>
                </div>
              )}
            </div>
            <button
              type="button"
              onClick={() => setPrefOpen(true)}
              className="rounded border border-indigo-600 px-2.5 py-1 text-xs text-indigo-300 hover:bg-indigo-600/20"
            >
              {m.actRefFromPreset}
            </button>
          </>
        )}
        {tab === "referable" && (
          <>
            <button
              type="button"
              onClick={() => void doRefAll()}
              disabled={opBusy || queueBusy || refAllBusy || typedReferable.length === 0}
              className="rounded border border-indigo-600 px-2.5 py-1 text-xs text-indigo-300 hover:bg-indigo-600/20 disabled:opacity-40"
            >
              {refAllBusy ? m.refBusy : m.actRefAll}
            </button>
            <button
              type="button"
              onClick={() => setPrefOpen(true)}
              className="rounded border border-indigo-600/70 px-2.5 py-1 text-xs text-indigo-300 hover:bg-indigo-600/15"
            >
              {m.actRefFromPreset}
            </button>
          </>
        )}
        <div className="flex-1" />
        {/* 缺失 N 徽章 + hover 清单（已实装页，核对点 8） */}
        {tab === "installed" && missingRows.length > 0 && (
          <div
            className="relative"
            onMouseEnter={() => setMissingOpen(true)}
            onMouseLeave={() => setMissingOpen(false)}
          >
            <span className="cursor-default rounded-full border border-red-500/50 bg-red-500/10 px-2.5 py-1 text-xs text-red-300">
              {m.missingBtn.replace("{n}", String(missingRows.length))}
            </span>
            {missingOpen && (
              // 空隙桥接（外层 pt-1）：鼠标从徽章移向清单时经过的区域仍属本容器；
              // 若用 mt-1 外边距，4px 空隙会先触发 mouseleave 把清单卸载（与 ScopeBadge 同款修正）
              <div className="absolute right-0 top-full z-[70] pt-1">
                <div className="w-80 rounded-lg border border-slate-700 bg-slate-900 p-2 shadow-2xl">
                  <div className="mb-1 px-1 text-[11px] leading-4 text-slate-400">{m.missingHint}</div>
                  <div className="max-h-56 overflow-y-auto">
                    {missingRows.map((x) => (
                      <div key={x.id} className="flex items-center gap-2 rounded px-1 py-1 hover:bg-slate-800/60">
                        <span className="min-w-0 flex-1 truncate text-xs text-slate-200" title={`${x.name || x.id}（${x.diskName || x.id}）`}>
                          {x.name || x.id}
                        </span>
                        <button
                          type="button"
                          onClick={() => jumpToDownloadable(x.diskName || x.id)}
                          className="flex-none rounded border border-sky-500/60 px-1.5 py-0.5 text-[11px] text-sky-300 hover:bg-sky-500/15"
                        >
                          {m.missingGo}
                        </button>
                      </div>
                    ))}
                  </div>
                </div>
              </div>
            )}
          </div>
        )}
        {/* 依赖处置（已实装页且有缺陷时出现）：缺陷筛选 toggle + 一键解析（闭包处置 · 操作栏右侧） */}
        {tab === "installed" && depSolve.defectRootCount > 0 && (
          <>
            <button
              type="button"
              onClick={() => setDepOnly((v) => !v)}
              className={cn(
                "flex items-center gap-1 rounded border px-2.5 py-1 text-xs transition-colors",
                depOnly
                  ? "border-amber-500/60 bg-amber-500/15 text-amber-200"
                  : "border-slate-600 text-slate-200 hover:border-slate-500 hover:text-white",
              )}
            >
              <AlertTriangle className="h-3 w-3" />
              {m.depFilter}
              <span className="rounded-full bg-amber-500/25 px-1.5 text-[10px] text-amber-100">
                {depSolve.defectRootCount}
              </span>
            </button>
            <button
              type="button"
              onClick={openClosure}
              disabled={opBusy || queueBusy || closureBusy}
              className="flex items-center gap-1 rounded border border-sky-500/60 bg-sky-500/10 px-2.5 py-1 text-xs text-sky-200 hover:bg-sky-500/20 disabled:opacity-40"
            >
              <Sparkles className="h-3 w-3" />
              {m.depClosureBtn}
            </button>
          </>
        )}
        {tab !== "referable" && (
          <>
            {/* 「自动检测更新」（默认开，本地偏好）：打开本页自动静默检查一次；关闭后仅手动 */}
            <label className="flex cursor-pointer select-none items-center gap-1.5 text-xs text-slate-300">
              <input
                type="checkbox"
                checked={autoCheckOn}
                disabled={autoCheckBusy}
                onChange={(e) => void toggleAutoCheck(e.target.checked)}
                className="h-3.5 w-3.5 accent-blue-500"
              />
              {mm.autoCheckLabel}
            </label>
            <div ref={srcWrapRef} className="relative">
              <button
                type="button"
                onClick={() => setSrcOpen((o) => !o)}
                className="rounded border border-slate-600 px-2.5 py-1 text-xs text-slate-200 hover:border-slate-500 hover:text-white"
              >
                {mm.sourceTitle}
              </button>
              {srcOpen && <ModSourcePopover onClose={() => setSrcOpen(false)} onSaved={() => onRepoCheck()} />}
            </div>
            <button
              type="button"
              onClick={() => onRepoCheck()}
              disabled={repoBusy}
              className="rounded border border-slate-600 px-2.5 py-1 text-xs text-slate-200 hover:border-slate-500 hover:text-white disabled:opacity-40"
            >
              {repoBusy ? t.modsRepo.checking : t.modsRepo.check}
            </button>
          </>
        )}
      </div>

      {/* 分类筛选 + 统计 */}
      <div className="flex flex-none flex-wrap items-center gap-1.5 text-xs">
        <button
          type="button"
          onClick={() => setTypeFilter("all")}
          className={cn(
            "rounded-full border px-3 py-1 transition-colors",
            typeFilter === "all"
              ? "border-blue-500/60 bg-blue-500/15 text-white"
              : "border-slate-700/50 text-slate-400 hover:text-white",
          )}
        >
          {t.mods.all}
        </button>
        {types.map((tp) => (
          <button
            key={tp}
            type="button"
            onClick={() => setTypeFilter(tp)}
            className={cn(
              "rounded-full border px-3 py-1 transition-colors",
              typeFilter === tp
                ? "border-blue-500/60 bg-blue-500/15 text-white"
                : "border-slate-700/50 text-slate-400 hover:text-white",
            )}
          >
            {t.mods.typeLabels[tp] ?? tp}
          </button>
        ))}
      </div>

      {/* 表格区（四页签的表格体在 ./RealmModTables，props 与本地闭包同名） */}
      <div className="min-h-0 flex-1 overflow-y-auto rounded-lg border border-slate-800">
        {tab === "installed" && (
          <InstalledTable
            t={t}
            m={m}
            mm={mm}
            typedInstalled={typedInstalled}
            depSolve={depSolve}
            rowOfInstalled={rowOfInstalled}
            repoMap={repoMap}
            setDetail={setDetail}
            shownNameOfRealm={shownNameOfRealm}
            opBusy={opBusy}
            queueBusy={queueBusy}
            dependentsOf={dependentsOf}
            setDepStates={setDepStates}
            setDepDlg={setDepDlg}
            setUnrefDlg={setUnrefDlg}
            setDelDlg={setDelDlg}
          />
        )}

        {tab === "referable" && (
          <ReferableTable
            t={t}
            m={m}
            mm={mm}
            typedReferable={typedReferable}
            rowOfReferable={rowOfReferable}
            repoMap={repoMap}
            setDetail={setDetail}
            miOfInstalled={miOfInstalled}
            vcmiLang={vcmiLang}
            opBusy={opBusy}
            queueBusy={queueBusy}
            doRef={doRef}
            doBundle={doBundle}
          />
        )}

        {tab === "downloadable" && (
          <DownloadableTable
            t={t}
            m={m}
            mm={mm}
            typedDownloadable={typedDownloadable}
            queueOf={queueOf}
            dlMap={dlMap}
            rowOfDownloadable={rowOfDownloadable}
            setDetail={setDetail}
            vcmiLang={vcmiLang}
            enqueueFromUi={enqueueFromUi}
            progressLabel={progressLabel}
            cancelQueued={cancelQueued}
            repoMap={repoMap}
            repoBusy={repoBusy}
            onRepoCheck={onRepoCheck}
          />
        )}

        {tab === "updatable" && (
          <UpdatableTable
            t={t}
            m={m}
            mm={mm}
            typedUpdatable={typedUpdatable}
            queueOf={queueOf}
            dlMap={dlMap}
            rowOfInstalled={rowOfInstalled}
            setDetail={setDetail}
            shownNameOfRealm={shownNameOfRealm}
            opBusy={opBusy}
            setUpdDlg={setUpdDlg}
            progressLabel={progressLabel}
            dequeue={dequeue}
            repoMap={repoMap}
            repoBusy={repoBusy}
            onRepoCheck={onRepoCheck}
          />
        )}
      </div>

      {/* 底部统计 */}
      <div className="flex-none text-[11px] text-slate-500">{mm.summary.replace("{n}", String(visibleCount))}</div>

      {/* 检查更新进度悬浮胶囊（对齐 mods 页 P10-1 形态：固定悬浮、pointer-events-none、
          不占布局空间——触网等待期间给出两阶段进度，避免"卡住无反馈"的体感） */}
      {repoBusy && repoProgress && (
        <div className="pointer-events-none absolute bottom-5 left-1/2 z-40 flex w-[min(560px,86%)] -translate-x-1/2 items-center gap-3 rounded-full border border-slate-700 bg-slate-900/95 px-4 py-2.5 shadow-2xl backdrop-blur">
          <span className="shrink-0 text-xs text-slate-300">
            {repoProgress.phase === "index" ? mm.progressIndex : mm.progressMeta}
          </span>
          <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-slate-800">
            <div
              className="h-full rounded-full bg-sky-500 transition-all duration-200"
              style={{
                width: `${repoProgress.total > 0 ? Math.min(100, Math.round((repoProgress.done / repoProgress.total) * 100)) : 0}%`,
              }}
            />
          </div>
          <span className="shrink-0 text-xs tabular-nums text-slate-500">
            {repoProgress.done}/{repoProgress.total}
          </span>
        </div>
      )}

      {/* 行详情抽屉（同构 Row 复用） */}
      <ModRowDrawer row={detail?.row ?? null} entry={detail?.entry ?? null} onClose={() => setDetail(null)} />

      {/* 依赖处置弹窗（依赖处置体系）：缺失依赖清单 + 引入 / 安装（实装 + 补入预设激活集） */}
      {depDlgInfo && (
        <DepResolveDialog
          t={t}
          info={depDlgInfo}
          langs={langs}
          states={depStates}
          busy={depBusy}
          onClose={() => setDepDlg(null)}
          onResolve={(d) => void resolveDepOne(d)}
          onResolveAll={(kind) => void resolveDepAll(kind)}
          onIgnore={(dep) => ignoreDeps([dep])}
        />
      )}

      {/* 依赖闭包解析弹窗（依赖处置 · 一键解析）：多轮手工修复压缩为一次预览 + 一次执行 */}
      {closureData && (
        <DepClosureDialog
          t={t}
          data={closureData}
          langs={langs}
          states={closureStates}
          busy={closureBusy}
          onClose={() => {
            setClosureData(null);
            setClosureStates({});
          }}
          onRun={(plan) => void runDepClosure(plan)}
          onIgnore={ignoreDeps}
        />
      )}

      {/* 取消引用确认（§7.1：依赖清单 + 级联清理） */}
      {unrefDlg && (
        <ConfirmDialog
          open
          title={m.unrefTitle}
          description={`${unrefDlg.mod.name || unrefDlg.mod.id}${
            unrefDlg.deps.length > 0 ? `\n${m.unrefDeps.replace("{list}", unrefDlg.deps.map((d) => d.name || d.id).join("、"))}` : ""
          }\n${m.unrefBody}`}
          confirmText={t.common.ok}
          cancelText={t.common.cancel}
          danger
          onCancel={() => setUnrefDlg(null)}
          onConfirm={() => {
            const cascade = unrefDlg.cascade
              ? unrefDlg.deps.filter((d) => d.inBorrowed && d.source !== "realm").map((d) => d.id)
              : [];
            const mod = unrefDlg.mod;
            setUnrefDlg(null);
            void doUnref(mod, cascade);
          }}
        >
          {unrefDlg.deps.length > 0 && (
            <label className="mb-3 flex cursor-pointer items-center gap-2 text-xs text-slate-300">
              <input
                type="checkbox"
                checked={unrefDlg.cascade}
                onChange={(e) => setUnrefDlg({ ...unrefDlg, cascade: e.target.checked })}
                className="h-3.5 w-3.5 accent-blue-500"
              />
              {m.unrefCascade}
            </label>
          )}
        </ConfirmDialog>
      )}

      {/* 删除确认（§7.2：领域位置副本 + 预设联动清理） */}
      {delDlg && (
        <ConfirmDialog
          open
          title={m.delTitle}
          description={`${m.delBody
            .replace("{name}", delDlg.mod.name || delDlg.mod.id)
            .replace("{dir}", delDlg.mod.diskName || delDlg.mod.id)}${
            delDlg.deps.length > 0 ? `\n${m.delDeps.replace("{list}", delDlg.deps.map((d) => d.name || d.id).join("、"))}` : ""
          }`}
          confirmText={t.common.ok}
          cancelText={t.common.cancel}
          danger
          onCancel={() => setDelDlg(null)}
          onConfirm={() => {
            const { mod, kind } = delDlg;
            setDelDlg(null);
            void doDelete(mod, kind);
          }}
        />
      )}

      {/* 整合冲突确认（§9.3.3：game 恒可见重复挂载 / diverged 指纹不一致） */}
      {bundleDlg && (
        <ConfirmDialog
          open
          title={bundleDlg.reason === "game" ? m.bundleGameTitle : m.bundleDivTitle}
          description={(bundleDlg.reason === "game" ? m.bundleGameWarn : m.bundleDivWarn).replace(
            "{name}",
            bundleDlg.mod.name || bundleDlg.mod.id,
          )}
          confirmText={m.bundleForce}
          cancelText={t.common.cancel}
          danger
          onCancel={() => setBundleDlg(null)}
          onConfirm={() => {
            const mod = bundleDlg.mod;
            setBundleDlg(null);
            void doBundle(mod, true);
          }}
        />
      )}

      {/* 更新确认（§7.3：changelog + 体积 + 校验等级；引用态附「转为领域专属模组」说明） */}
      {updDlg && (
        <ConfirmDialog
          open
          title={m.updTitle}
          description={(() => {
            const size = formatDownloadSize(updDlg.entry.downloadSize);
            const logs = changelogNewest(updDlg.entry.changelog, 2);
            const head = `${shownNameOfRealm(updDlg.mod)}  v${updDlg.cur || "—"} → v${updDlg.next}`;
            const info = `${size ? `${m.updSize.replace("{size}", size)} · ` : ""}${
              updDlg.entry.sha256 ? m.updCheckOk : m.updCheckWeak
            }`;
            const log =
              logs.length > 0
                ? m.updLog.replace("{v}", logs[0].version).replace("{text}", logs[0].lines.slice(0, 3).join("；"))
                : t.modsRepo.noChangelog;
            const conv =
              updDlg.mod.source !== "realm"
                ? `\n${m.updConvert.replace(
                    "{loc}",
                    updDlg.mod.source === "game" ? mm.locationGame : mm.locationUser,
                  )}`
                : "";
            return `${head}\n${info}\n${log}${conv}`;
          })()}
          confirmText={mm.actUpdate}
          cancelText={t.common.cancel}
          onCancel={() => setUpdDlg(null)}
          onConfirm={() => {
            const row = updDlg;
            setUpdDlg(null);
            void enqueueUpdate(row);
          }}
        />
      )}

      {/* 导入冲突确认（§7.4：替换 / 跳过） */}
      {impDlg && (
        <ConfirmDialog
          open
          title={m.impConflictTitle}
          description={`${m.impConflictBody}\n${impDlg.items
            .map((r) => `· ${r.name || r.modId || r.path}${r.version ? ` v${r.version}` : ""}`)
            .join("\n")}`}
          confirmText={m.impConflictReplace}
          cancelText={m.impConflictSkip}
          onCancel={() => {
            const paths = impDlg.items.map((r) => r.path);
            setImpDlg(null);
            showToast("success", m.impSkipped.replace("{n}", String(paths.length)));
          }}
          onConfirm={() => {
            const paths = impDlg.items.map((r) => r.path);
            setImpDlg(null);
            void runImport(paths, true);
          }}
        />
      )}

      {/* 从模组预设引用（§7.5） */}
      {prefOpen && (
        <div className="fixed inset-0 z-[150] flex items-center justify-center bg-black/60" onClick={() => !prefBusy && setPrefOpen(false)}>
          <div
            className="flex max-h-[80vh] w-[34rem] flex-col rounded-xl border border-slate-700 bg-slate-900 p-4 shadow-2xl"
            onClick={(e) => e.stopPropagation()}
          >
            <h3 className="mb-2 text-sm font-semibold text-white">{m.prefTitle}</h3>
            <p className="mb-3 text-xs leading-5 text-slate-400">{m.prefHint}</p>
            {!prefList ? (
              <div className="py-8 text-center text-sm text-slate-500">{t.mods.loading}</div>
            ) : prefList.length === 0 ? (
              <div className="py-8 text-center text-sm text-slate-500">{m.prefEmpty}</div>
            ) : prefResult ? (
              <div className="space-y-2 py-2 text-sm">
                <div className="rounded-lg border border-emerald-500/40 bg-emerald-500/10 px-3 py-2 text-emerald-200">
                  {m.prefDone.replace("{n}", String(prefResult.ok))}
                </div>
                {prefResult.already > 0 && (
                  <div className="rounded-lg border border-sky-500/40 bg-sky-500/10 px-3 py-2 text-xs text-sky-200">
                    {m.prefInRealm.replace("{n}", String(prefResult.already))}
                  </div>
                )}
                {prefResult.missing.length > 0 && (
                  <div className="rounded-lg border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs leading-5 text-amber-200">
                    {m.prefMissing.replace("{n}", String(prefResult.missing.length))}
                    <div className="mt-1 text-[11px] text-amber-300/80">{m.prefMissingHint}</div>
                  </div>
                )}
                {prefResult.created && (
                  <div className="rounded-lg border border-indigo-500/40 bg-indigo-500/10 px-3 py-2 text-xs text-indigo-200">
                    {m.prefCreated.replace("{name}", prefResult.created)}
                  </div>
                )}
              </div>
            ) : (
              <>
                <div className="mb-3 min-h-0 flex-1 overflow-y-auto rounded-lg border border-slate-800">
                  {prefList.map((p) => (
                    <button
                      key={p.id}
                      type="button"
                      onClick={() => setPrefSel(p.id)}
                      className={cn(
                        "flex w-full items-center gap-2 border-b border-slate-800/60 px-3 py-2 text-left text-xs last:border-b-0",
                        prefSel === p.id ? "bg-indigo-500/15 text-white" : "text-slate-300 hover:bg-slate-800/50",
                      )}
                    >
                      <span className={cn("h-3 w-3 flex-none rounded-full border", prefSel === p.id ? "border-indigo-400 bg-indigo-500" : "border-slate-600")} />
                      <span className="min-w-0 flex-1 truncate">{p.id}</span>
                      <span className="flex-none text-[11px] text-slate-500">{m.prefCount.replace("{n}", String(p.mods.length))}</span>
                    </button>
                  ))}
                </div>
                <div className="mb-3 space-y-1.5 text-xs text-slate-300">
                  <div className="text-[11px] font-medium uppercase tracking-wide text-slate-500">{m.prefAdoptTitle}</div>
                  <label className="flex cursor-pointer items-center gap-2">
                    <input
                      type="radio"
                      checked={prefAdopt === "merge"}
                      onChange={() => setPrefAdopt("merge")}
                      className="h-3.5 w-3.5 accent-blue-500"
                    />
                    {m.prefAdoptMerge}
                  </label>
                  <label className="flex cursor-pointer items-center gap-2">
                    <input
                      type="radio"
                      checked={prefAdopt === "create"}
                      onChange={() => setPrefAdopt("create")}
                      className="h-3.5 w-3.5 accent-blue-500"
                    />
                    {m.prefAdoptCreate}
                    <span className="text-slate-500">{m.prefAdoptSuffix}</span>
                  </label>
                </div>
              </>
            )}
            <div className="mt-2 flex justify-end gap-2">
              {prefResult ? (
                <button
                  type="button"
                  onClick={() => setPrefOpen(false)}
                  className="rounded-lg bg-blue-600 px-3 py-1.5 text-sm font-semibold text-white hover:bg-blue-500"
                >
                  {t.common.ok}
                </button>
              ) : (
                <>
                  <button
                    type="button"
                    disabled={prefBusy}
                    onClick={() => setPrefOpen(false)}
                    className="rounded-lg border border-slate-600 px-3 py-1.5 text-sm text-slate-300 hover:text-white disabled:opacity-50"
                  >
                    {t.common.cancel}
                  </button>
                  <button
                    type="button"
                    disabled={prefBusy || !selPreset}
                    onClick={() => void runPresetRef()}
                    className="rounded-lg bg-blue-600 px-3 py-1.5 text-sm font-semibold text-white hover:bg-blue-500 disabled:opacity-50"
                  >
                    {prefBusy ? m.refBusy : m.prefRun}
                  </button>
                </>
              )}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
