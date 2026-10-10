import { useEffect, useMemo, useState } from "react";
import {
  ChevronDown,
  ChevronRight,
  FileText,
  Folder,
  FolderInput,
  ListOrdered,
  MoreVertical,
  Search,
  Trash2,
  Upload,
  X,
} from "lucide-react";
import { cn } from "@/lib/utils";
import type { LangDict } from "@/i18n/types";
import type { RealmMapEntry } from "@/lib/realm/types";
import { vcmiLangName, vcmiLangOptions } from "@/lib/vcmiLangs";
import {
  applyRealmMapsValve,
  deleteRealmMap,
  importRealmMapsDir,
  listRealmMaps,
  moveRealmMap,
  pickOpenDirectory,
  pickOpenPath,
  setRealmMapFilter,
  uploadRealmMap,
  type RealmMapImportReport,
} from "@/lib/tauri";
import { ConfirmDialog } from "@/shared/ConfirmDialog";
import { backendErrorCode, backendErrorText } from "@/lib/backendError";

/** 集成地图统一页面（v5 三栏：目录树 / 直属列表 / 详情面板）。
 * - 编辑器模式：工具栏（引入 / 搜索 / 屏蔽阀门 / 语言过滤）+ 完整管理能力；
 * - 信息页只读模式（readOnly）：无管理按钮，仅树 + 列表 + 详情，可按语言过滤显示。 */
export interface MapsTabProps {
  t: LangDict;
  realmId: string;
  /** dataPath === "global" 时禁用管理操作（整合地图不会被 VCMI 读取） */
  disabled?: boolean;
  /** 信息页只读模式 */
  readOnly?: boolean;
  /** 只读模式下按 filterLang 过滤显示（目标语言 + unknown 恒显示） */
  applyFilter?: boolean;
  blockMaps?: boolean;
  requestBlock?: (v: boolean) => void;
  filterLang: boolean;
  /** 过滤目标语言（VCMI identifier）；"" = 跟随启动器 UI 语言 */
  filterLangTarget: string;
  onFilterChange?: (enabled: boolean, target: string) => void;
  /** 地图清单变化（引入/删除/移动后）回传给编辑器，保持保存快照一致 */
  onMapsChanged?: (entries: RealmMapEntry[]) => void;
  showToast?: (kind: "success" | "error" | "info", msg: string) => void;
}

type KindSel = "all" | "maps" | "campaigns";

interface TreeNode {
  seg: string;
  children: TreeNode[];
  count: number;
}

function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}

/** 仓库相对路径 → { 类型分区, 目录段, 文件名 } */
function splitFile(file: string) {
  const segs = file.split("/");
  return { kind: segs[0] ?? "maps", dirs: segs.slice(1, -1), name: segs[segs.length - 1] ?? file };
}

export function MapsTab(props: MapsTabProps) {
  const {
    t,
    realmId,
    disabled = false,
    readOnly = false,
    applyFilter = false,
    blockMaps,
    requestBlock,
    filterLang,
    filterLangTarget,
    onFilterChange,
    onMapsChanged,
    showToast,
  } = props;

  const [entries, setEntries] = useState<RealmMapEntry[]>([]);
  const [loading, setLoading] = useState(true);
  // 当前选中节点：类型分区 + 类型后的目录段（[lang, subdir...]）
  const [selKind, setSelKind] = useState<KindSel>("all");
  const [selPath, setSelPath] = useState<string[]>([]);
  const [expanded, setExpanded] = useState<Set<string>>(new Set());
  const [selFile, setSelFile] = useState<string | null>(null);
  const [menuFile, setMenuFile] = useState<string | null>(null);
  const [searchOpen, setSearchOpen] = useState(false);
  const [query, setQuery] = useState("");
  // 引入弹窗（三步）
  const [importOpen, setImportOpen] = useState(false);
  const [importStep, setImportStep] = useState<"pick" | "preview" | "done">("pick");
  const [importDir, setImportDir] = useState("");
  const [report, setReport] = useState<RealmMapImportReport | null>(null);
  const [keepStructure, setKeepStructure] = useState(true);
  const [conflict, setConflict] = useState<"skip" | "overwrite" | "rename">("skip");
  const [busy, setBusy] = useState(false);
  // 移动 / 删除
  const [moveFile, setMoveFile] = useState<string | null>(null);
  const [moveDest, setMoveDest] = useState<string>("");
  const [delFile, setDelFile] = useState<string | null>(null);

  const err = (e: unknown) =>
    showToast?.("error", t.realm.error.replace("{msg}", backendErrorText(e)));

  /** 加载清单（重建 index.json） */
  const refresh = async () => {
    setLoading(true);
    try {
      const r = await listRealmMaps(realmId);
      const list = r.entries ?? [];
      setEntries(list);
      onMapsChanged?.(list);
    } catch (e) {
      err(e);
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    refresh();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [realmId]);

  /** 语言显示名（unknown → i18n 文案） */
  const langName = (seg: string) => (seg === "unknown" ? t.realm.mapsLangUnknown : vcmiLangName(seg));

  /** 由 entries 派生目录树（两大分区：maps / campaigns） */
  const tree = useMemo(() => {
    const roots: Record<string, TreeNode> = {
      maps: { seg: "maps", children: [], count: 0 },
      campaigns: { seg: "campaigns", children: [], count: 0 },
    };
    const insert = (root: TreeNode, segs: string[]) => {
      root.count += 1;
      let cur = root;
      for (const seg of segs) {
        let next = cur.children.find((c) => c.seg === seg);
        if (!next) {
          next = { seg, children: [], count: 0 };
          cur.children.push(next);
        }
        next.count += 1;
        cur = next;
      }
    };
    for (const e of entries) {
      const { kind, dirs } = splitFile(e.file);
      const root = roots[kind] ?? roots.maps;
      insert(root, dirs);
    }
    const sortTree = (n: TreeNode) => {
      n.children.sort((a, b) => a.seg.localeCompare(b.seg));
      n.children.forEach(sortTree);
    };
    Object.values(roots).forEach(sortTree);
    return roots;
  }, [entries]);

  // 默认展开两大分区（语言层可见，子目录懒展开）
  useEffect(() => {
    setExpanded((prev) => {
      if (prev.has("maps") || prev.has("campaigns")) return prev;
      const next = new Set(prev);
      next.add("maps");
      next.add("campaigns");
      return next;
    });
  }, []);

  const toggleNode = (key: string) =>
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });

  /** 信息页语言过滤：目标语言（或 UI 语言回退）+ unknown 恒显示 */
  const visibleEntries = useMemo(() => {
    if (!applyFilter || !filterLang) return entries;
    return entries.filter((e) => !e.lang || e.lang === "unknown" || e.lang === filterLangTarget);
  }, [entries, applyFilter, filterLang, filterLangTarget]);

  /** 当前选中目录下的直属子目录与直属文件（「全部」根节点不列文件，仅作汇总入口） */
  const { subDirs, directFiles } = useMemo(() => {
    if (selKind === "all") {
      return { subDirs: [] as { seg: string; count: number }[], directFiles: [] as RealmMapEntry[] };
    }
    const inSel = visibleEntries.filter((e) => {
      const d = splitFile(e.file);
      return d.kind === selKind && selPath.every((p, i) => d.dirs[i] === p);
    });
    const segIdx = selPath.length;
    const segs = new Map<string, number>();
    const files: RealmMapEntry[] = [];
    for (const e of inSel) {
      const d = splitFile(e.file);
      if (d.dirs.length === segIdx) files.push(e);
      else if (d.dirs.length > segIdx) {
        const seg = d.dirs[segIdx];
        segs.set(seg, (segs.get(seg) ?? 0) + 1);
      }
    }
    return {
      subDirs: [...segs.entries()].map(([seg, count]) => ({ seg, count })).sort((a, b) => a.seg.localeCompare(b.seg)),
      directFiles: files.sort((a, b) => splitFile(a.file).name.localeCompare(splitFile(b.file).name)),
    };
  }, [visibleEntries, selKind, selPath]);

  /** 「全部」节点汇总：类型/语言分布 + 总大小（不列文件） */
  const summary = useMemo(() => {
    let maps = 0;
    let campaigns = 0;
    let totalSize = 0;
    const langs = new Map<string, number>();
    for (const e of visibleEntries) {
      if (e.kind === "campaign") campaigns += 1;
      else maps += 1;
      totalSize += e.size ?? 0;
      const l = e.lang || "unknown";
      langs.set(l, (langs.get(l) ?? 0) + 1);
    }
    const byLang = [...langs.entries()].sort((a, b) => b[1] - a[1]);
    return { maps, campaigns, totalSize, byLang };
  }, [visibleEntries]);

  /** 搜索结果（平铺，附所属目录；与类型/语言筛选叠加） */
  const searchResults = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return [];
    return visibleEntries.filter((e) => {
      const { name, dirs } = splitFile(e.file);
      return (
        name.toLowerCase().includes(q) ||
        (e.name ?? "").toLowerCase().includes(q) ||
        dirs.some((d) => d.toLowerCase().includes(q))
      );
    });
  }, [query, visibleEntries]);

  const selectedEntry = useMemo(
    () => visibleEntries.find((e) => e.file === selFile) ?? entries.find((e) => e.file === selFile) ?? null,
    [visibleEntries, entries, selFile],
  );

  /** 面包屑段显示名 */
  const crumbName = (kind: KindSel, seg: string, idx: number) =>
    idx === 0 ? langName(seg) : seg;

  /** 移动候选目录：同类型分区下的全部目录（排除当前所在目录） */
  const moveCandidates = useMemo(() => {
    if (!moveFile) return [];
    const src = splitFile(moveFile);
    const dirs = new Set<string>();
    for (const e of entries) {
      const d = splitFile(e.file);
      if (d.kind !== src.kind) continue;
      dirs.add(d.dirs.join("/"));
    }
    const srcDir = src.dirs.join("/");
    return [...dirs].filter((d) => d !== srcDir).sort();
  }, [moveFile, entries]);

  // ── 操作 ────────────────────────────────────────────────────────────

  const handleImportFile = async () => {
    const p = await pickOpenPath([{ name: t.realm.mapList, extensions: ["h3m", "h3c", "vmap"] }]);
    if (!p) return;
    setBusy(true);
    try {
      await uploadRealmMap(realmId, p);
      await refresh();
      showToast?.("success", t.common.ok);
      setImportOpen(false);
    } catch (e) {
      err(e);
    } finally {
      setBusy(false);
    }
  };

  const handleBrowseDir = async () => {
    const p = await pickOpenDirectory();
    if (p) setImportDir(p);
  };

  const handleScan = async () => {
    if (!importDir.trim()) return;
    setBusy(true);
    try {
      const r = await importRealmMapsDir(realmId, importDir.trim(), keepStructure, conflict, true);
      setReport(r);
      setImportStep("preview");
    } catch (e) {
      err(e);
    } finally {
      setBusy(false);
    }
  };

  const handleImportConfirm = async () => {
    if (!importDir.trim()) return;
    setBusy(true);
    try {
      const r = await importRealmMapsDir(realmId, importDir.trim(), keepStructure, conflict, false);
      setReport(r);
      setImportStep("done");
      await refresh();
    } catch (e) {
      err(e);
    } finally {
      setBusy(false);
    }
  };

  const handleDelete = async () => {
    if (!delFile) return;
    try {
      await deleteRealmMap(realmId, delFile);
      if (selFile === delFile) setSelFile(null);
      await refresh();
    } catch (e) {
      err(e);
    } finally {
      setDelFile(null);
      setMenuFile(null);
    }
  };

  const handleMove = async () => {
    if (!moveFile || !moveDest) return;
    try {
      await moveRealmMap(realmId, moveFile, moveDest);
      await refresh();
    } catch (e) {
      err(e);
    } finally {
      setMoveFile(null);
      setMoveDest("");
      setMenuFile(null);
    }
  };

  /** 切换语言过滤（后端立即持久化 + 激活领域时重建阀门） */
  const changeFilter = async (enabled: boolean, target: string) => {
    onFilterChange?.(enabled, target);
    try {
      await setRealmMapFilter(realmId, enabled, target || undefined);
    } catch (e) {
      // 按**码**判断「VCMI 运行中」守卫（匹配本地化文案在非中文界面会失配）
      showToast?.(
        "info",
        backendErrorCode(e) === "realm_guard_vcmi_running"
          ? t.realm.mapsVcmiRunning
          : backendErrorText(e),
      );
    }
  };

  // ── 渲染辅助 ────────────────────────────────────────────────────────

  const kindLabel = (k: KindSel) =>
    k === "all" ? t.realm.mapsTreeAll : k === "maps" ? t.realm.mapsTreeMaps : t.realm.mapsTreeCampaigns;

  const renderTreeNode = (kind: KindSel, node: TreeNode, path: string[], depth: number): React.ReactNode => {
    const key = `${kind}|${path.join("/")}`;
    const isOpen = expanded.has(key);
    const isSel = selKind === kind && selPath.join("/") === path.join("/");
    // 语言层用本地化名，更深目录用原始名
    const label = depth === 1 ? langName(node.seg) : node.seg;
    return (
      <div key={key}>
        <div
          className={cn(
            "flex cursor-pointer items-center gap-1 rounded px-1.5 py-1 text-xs",
            isSel ? "bg-blue-500/20 text-blue-200" : "text-slate-300 hover:bg-slate-800/60",
          )}
          style={{ paddingLeft: `${depth * 12 + 4}px` }}
          onClick={() => {
            setSelKind(kind);
            setSelPath(path);
            setSelFile(null);
          }}
        >
          {node.children.length > 0 ? (
            <button
              type="button"
              className="shrink-0 text-slate-500 hover:text-slate-200"
              onClick={(ev) => {
                ev.stopPropagation();
                toggleNode(key);
              }}
            >
              {isOpen ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
            </button>
          ) : (
            <span className="w-3 shrink-0" />
          )}
          {depth === 0 ? <Folder size={13} className="shrink-0 text-slate-500" /> : null}
          <span className="min-w-0 flex-1 truncate">{label}</span>
          <span className="shrink-0 text-[10px] text-slate-500">{node.count}</span>
        </div>
        {isOpen &&
          node.children.map((c) => renderTreeNode(kind, c, [...path, c.seg], depth + 1))}
      </div>
    );
  };

  const diffLabel = (d?: number) => {
    const arr = [t.realm.mapsDiff0, t.realm.mapsDiff1, t.realm.mapsDiff2, t.realm.mapsDiff3, t.realm.mapsDiff4];
    return d !== undefined && d >= 0 && d <= 4 ? arr[d] : "—";
  };

  const vicLabel = (v?: string) => {
    const m: Record<string, string> = {
      standard: t.realm.mapVicStandard,
      artifact: t.realm.mapVicArtifact,
      gather_troop: t.realm.mapVicGatherTroop,
      gather_resource: t.realm.mapVicGatherResource,
      build_city: t.realm.mapVicBuildCity,
      build_grail: t.realm.mapVicBuildGrail,
      beat_hero: t.realm.mapVicBeatHero,
      capture_city: t.realm.mapVicCaptureCity,
      beat_monster: t.realm.mapVicBeatMonster,
      take_dwellings: t.realm.mapVicTakeDwellings,
      take_mines: t.realm.mapVicTakeMines,
      transport_item: t.realm.mapVicTransportItem,
      eliminate_all_monsters: t.realm.mapVicEliminateMonsters,
      survive_days: t.realm.mapVicSurviveDays,
    };
    return (v && m[v]) || "—";
  };

  const lossLabel = (v?: string) => {
    const m: Record<string, string> = {
      standard: t.realm.mapLossStandard,
      lose_castle: t.realm.mapLossCastle,
      lose_hero: t.realm.mapLossHero,
      time_expires: t.realm.mapLossTime,
    };
    return (v && m[v]) || "—";
  };

  const langBadge = (lang?: string) => (
    <span className="shrink-0 rounded bg-slate-800 px-1 py-0.5 text-[10px] text-slate-300">
      {lang === "unknown" || !lang ? t.realm.mapsLangUnknown : langName(lang)}
    </span>
  );

  const kindBadge = (e: RealmMapEntry) => (
    <span
      className={cn(
        "shrink-0 rounded px-1 py-0.5 text-[10px] font-medium",
        e.kind === "campaign" ? "bg-amber-500/20 text-amber-300" : "bg-sky-500/20 text-sky-300",
      )}
    >
      {e.kind === "campaign" ? t.realm.mapsTreeCampaigns : e.kind === "vmap" ? "VCMI" : t.realm.mapsTreeMaps}
    </span>
  );

  /** 行内 ⋮ 菜单（移动 / 删除；只读模式不显示） */
  const renderMenu = (e: RealmMapEntry) => {
    if (readOnly || disabled) return null;
    return (
      <div className="relative shrink-0">
        <button
          type="button"
          className="rounded p-0.5 text-slate-400 hover:bg-slate-800 hover:text-white"
          onClick={(ev) => {
            ev.stopPropagation();
            setMenuFile(menuFile === e.file ? null : e.file);
          }}
        >
          <MoreVertical size={14} />
        </button>
        {menuFile === e.file && (
          <div className="absolute right-0 top-6 z-30 w-32 rounded-md border border-slate-700 bg-slate-900 py-1 shadow-xl">
            <button
              type="button"
              className="flex w-full items-center gap-1.5 px-2.5 py-1.5 text-xs text-slate-200 hover:bg-slate-800"
              onClick={(ev) => {
                ev.stopPropagation();
                setMoveFile(e.file);
                setMoveDest("");
              }}
            >
              <FolderInput size={13} />
              {t.realm.mapsMoveTo}
            </button>
            <button
              type="button"
              className="flex w-full items-center gap-1.5 px-2.5 py-1.5 text-xs text-red-400 hover:bg-slate-800"
              onClick={(ev) => {
                ev.stopPropagation();
                setDelFile(e.file);
              }}
            >
              <Trash2 size={13} />
              {t.realm.deleteMap}
            </button>
          </div>
        )}
      </div>
    );
  };

  /** 列表行（固定列序）：
   * - 地图：类型tag - 玩家数 - 尺寸 - 版本 - 名称 - 语言 - 大小
   * - 战役：类型tag - 名称 - 场景数 - 语言 - 大小 */
  const renderRow = (e: RealmMapEntry, showDir: boolean) => {
    const { name, dirs } = splitFile(e.file);
    const isCampaign = e.kind === "campaign";
    const players =
      e.players !== undefined && e.players >= 0 ? `${e.humans ?? 0}/${e.players}` : "—";
    return (
      <div
        key={e.file}
        className={cn(
          "flex cursor-pointer items-center gap-2 rounded-md border px-2 py-1.5 text-xs",
          selFile === e.file
            ? "border-blue-500/50 bg-blue-500/10"
            : "border-slate-800 bg-slate-900/60 hover:bg-slate-800/50",
        )}
        onClick={() => setSelFile(e.file)}
      >
        {kindBadge(e)}
        {isCampaign ? (
          <span
            className="w-10 shrink-0 text-center text-slate-300"
            title={t.realm.mapsDetailScenarios}
          >
            {(e.scenarioCount ?? 0) > 0 && (
              <ListOrdered size={11} className="mr-0.5 inline align-[-1px] text-slate-500" />
            )}
            {e.scenarioCount ?? 0}
          </span>
        ) : (
          <>
            <span className="w-9 shrink-0 text-center text-slate-400" title={t.realm.mapsDetailPlayers}>
              {players}
            </span>
            <span className="w-7 shrink-0 text-center font-medium text-slate-300" title={t.realm.mapsDetailPlayers}>
              {e.sizeClass || "—"}
            </span>
            <span className="w-10 shrink-0 text-center text-slate-400" title={t.realm.mapsDetailVersion}>
              {e.version || "—"}
            </span>
          </>
        )}
        <span className="min-w-0 flex-1 truncate text-foreground" title={e.file}>
          {e.name || name}
        </span>
        {showDir && !isCampaign && (
          <span className="hidden max-w-[10rem] shrink-0 truncate text-[10px] text-slate-500 lg:block">
            {dirs.join("/") || "—"}
          </span>
        )}
        {langBadge(e.lang)}
        <span className="w-16 shrink-0 text-right tabular-nums text-slate-500">
          {formatBytes(e.size)}
        </span>
        {renderMenu(e)}
      </div>
    );
  };

  // ── 主体 ────────────────────────────────────────────────────────────

  return (
    <div className="flex h-full min-h-0 flex-col gap-2" onClick={() => setMenuFile(null)}>
      {/* 工具栏（一行；只读模式不显示） */}
      {!readOnly && (
        <div className="flex flex-none flex-wrap items-center justify-between gap-2">
          <div className="flex items-center gap-2">
            <button
              type="button"
              title={t.realm.mapsImportTip}
              disabled={disabled}
              onClick={() => {
                setImportDir("");
                setReport(null);
                setImportStep("pick");
                setImportOpen(true);
              }}
              className="flex items-center gap-1.5 rounded border border-indigo-600 px-2.5 py-1.5 text-xs text-indigo-300 hover:bg-indigo-600/20 disabled:cursor-not-allowed disabled:opacity-40"
            >
              <Upload size={14} />
              {t.realm.mapsImport}
            </button>
            <button
              type="button"
              onClick={() => setSearchOpen((v) => !v)}
              className={cn(
                "flex items-center gap-1.5 rounded border px-2.5 py-1.5 text-xs",
                searchOpen
                  ? "border-blue-500 bg-blue-500/20 text-blue-200"
                  : "border-slate-700 text-slate-300 hover:bg-slate-800/50",
              )}
            >
              <Search size={14} />
              {t.realm.mapsSearch}
            </button>
          </div>
          <div className="flex flex-wrap items-center justify-end gap-3" onClick={(ev) => ev.stopPropagation()}>
            {/* 屏蔽游戏自带地图阀门 */}
            {requestBlock && (
              <label
                className="flex cursor-pointer items-center gap-1.5 text-xs text-foreground"
                title={t.realm.blockMapsHint}
              >
                <input
                  type="checkbox"
                  checked={blockMaps ?? false}
                  onChange={(e) => requestBlock(e.target.checked)}
                  className="h-3.5 w-3.5 accent-blue-500"
                />
                {t.realm.blockMaps}
              </label>
            )}
            {/* 语言过滤 */}
            <label
              className="flex cursor-pointer items-center gap-1.5 text-xs text-foreground"
              title={t.realm.mapsFilterHint}
            >
              <input
                type="checkbox"
                checked={filterLang}
                onChange={(e) => changeFilter(e.target.checked, filterLangTarget)}
                className="h-3.5 w-3.5 accent-blue-500"
              />
              {t.realm.mapsFilterLang}
            </label>
            {filterLang && (
              <select
                value={filterLangTarget}
                onChange={(e) => changeFilter(true, e.target.value)}
                className="rounded border border-slate-700 bg-slate-900 px-1.5 py-1 text-xs text-foreground"
              >
                {vcmiLangOptions().map((l) => (
                  <option key={l.id} value={l.id}>
                    {l.native}
                  </option>
                ))}
              </select>
            )}
          </div>
        </div>
      )}

      {/* 搜索条（平铺结果） */}
      {searchOpen && (
        <div className="flex flex-none items-center gap-2">
          <input
            autoFocus
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t.realm.mapsSearch}
            className="min-w-0 flex-1 rounded border border-slate-700 bg-slate-900 px-2 py-1.5 text-xs text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500/50"
          />
          <button
            type="button"
            className="rounded p-1 text-slate-400 hover:bg-slate-800 hover:text-white"
            onClick={() => {
              setSearchOpen(false);
              setQuery("");
            }}
          >
            <X size={14} />
          </button>
        </div>
      )}

      {/* 三栏主体 */}
      <div className="flex min-h-0 flex-1 gap-2">
        {/* 左：目录树（搜索时不显示） */}
        {!searchOpen && (
          <div className="w-52 flex-none overflow-y-auto rounded-lg border border-slate-800 bg-slate-900/40 p-1.5">
            <div
              className={cn(
                "flex cursor-pointer items-center gap-1 rounded px-1.5 py-1 text-xs",
                selKind === "all" ? "bg-blue-500/20 text-blue-200" : "text-slate-300 hover:bg-slate-800/60",
              )}
              onClick={() => {
                setSelKind("all");
                setSelPath([]);
                setSelFile(null);
              }}
            >
              <span className="min-w-0 flex-1">{t.realm.mapsTreeAll}</span>
              <span className="shrink-0 text-[10px] text-slate-500">{visibleEntries.length}</span>
            </div>
            {(["maps", "campaigns"] as const).map((k) => {
              const root = tree[k];
              return (
                <div key={k}>
                  <div
                    className={cn(
                      "flex cursor-pointer items-center gap-1 rounded px-1.5 py-1 text-xs",
                      selKind === k && selPath.length === 0
                        ? "bg-blue-500/20 text-blue-200"
                        : "text-slate-300 hover:bg-slate-800/60",
                    )}
                    onClick={() => {
                      setSelKind(k);
                      setSelPath([]);
                      setSelFile(null);
                    }}
                  >
                    <button
                      type="button"
                      className="shrink-0 text-slate-500 hover:text-slate-200"
                      onClick={(ev) => {
                        ev.stopPropagation();
                        toggleNode(k);
                      }}
                    >
                      {expanded.has(k) ? <ChevronDown size={12} /> : <ChevronRight size={12} />}
                    </button>
                    <span className="min-w-0 flex-1 truncate">{kindLabel(k)}</span>
                    <span className="shrink-0 text-[10px] text-slate-500">{root.count}</span>
                  </div>
                  {expanded.has(k) && root.children.map((c) => renderTreeNode(k, c, [c.seg], 1))}
                </div>
              );
            })}
          </div>
        )}

        {/* 中：直属列表 / 搜索平铺结果 */}
        <div className="flex min-w-0 flex-1 flex-col gap-1.5">
          {!searchOpen && (
            <div className="flex flex-none flex-wrap items-center gap-1 text-xs text-slate-400">
              <button
                type="button"
                className="hover:text-white"
                onClick={() => {
                  setSelKind("all");
                  setSelPath([]);
                }}
              >
                {t.realm.mapsTreeAll}
              </button>
              {selKind !== "all" && (
                <>
                  <span>›</span>
                  <button
                    type="button"
                    className="hover:text-white"
                    onClick={() => setSelPath([])}
                  >
                    {kindLabel(selKind)}
                  </button>
                  {selPath.map((p, i) => (
                    <span key={i} className="flex items-center gap-1">
                      <span>›</span>
                      <button
                        type="button"
                        className="hover:text-white"
                        onClick={() => setSelPath(selPath.slice(0, i + 1))}
                      >
                        {crumbName(selKind, p, i)}
                      </button>
                    </span>
                  ))}
                </>
              )}
              <span className="ml-auto text-[10px] text-slate-500">
                {selKind === "all" ? visibleEntries.length : subDirs.length + directFiles.length}
              </span>
            </div>
          )}
          <div className="min-h-0 flex-1 space-y-1.5 overflow-y-auto pr-1">
            {searchOpen ? (
              query.trim() && searchResults.length === 0 ? (
                <p className="text-xs text-muted">{t.realm.mapsSearchEmpty}</p>
              ) : (
                searchResults.map((e) => renderRow(e, true))
              )
            ) : loading ? (
              <p className="text-xs text-muted">…</p>
            ) : selKind === "all" ? (
              /* 「全部」：仅显示汇总信息（大数字统计 + 语言分布），浏览进左树分区 */
              <div className="mx-auto mt-6 w-full max-w-md rounded-xl border border-slate-800 bg-slate-900/40 p-6">
                <div className="grid grid-cols-3 gap-4 text-center">
                  <div>
                    <p className="text-3xl font-semibold tabular-nums text-sky-300">{summary.maps}</p>
                    <p className="mt-1.5 text-xs text-muted">{t.realm.mapsTreeMaps}</p>
                  </div>
                  <div className="border-x border-slate-800">
                    <p className="text-3xl font-semibold tabular-nums text-amber-300">{summary.campaigns}</p>
                    <p className="mt-1.5 text-xs text-muted">{t.realm.mapsTreeCampaigns}</p>
                  </div>
                  <div>
                    <p className="text-3xl font-semibold tabular-nums text-foreground">{formatBytes(summary.totalSize)}</p>
                    <p className="mt-1.5 text-xs text-muted">{t.realm.mapsTotalSize}</p>
                  </div>
                </div>
                {summary.byLang.length > 0 && (
                  <div className="mt-5 border-t border-slate-800 pt-4 text-center">
                    <p className="mb-2 text-xs text-muted">{t.realm.mapsLangDist}</p>
                    <div className="flex flex-wrap justify-center gap-2">
                      {summary.byLang.map(([l, n]) => (
                        <span key={l} className="rounded bg-slate-800 px-2.5 py-1 text-xs text-slate-200">
                          {langName(l)} × {n}
                        </span>
                      ))}
                    </div>
                  </div>
                )}
                <p className="mt-5 border-t border-slate-800 pt-4 text-center text-xs text-muted">
                  {t.realm.mapsSummaryHint}
                </p>
              </div>
            ) : subDirs.length === 0 && directFiles.length === 0 ? (
              <p className="text-xs text-muted">{t.realm.mapsEmpty}</p>
            ) : (
              <>
                {subDirs.map((d) => (
                  <div
                    key={d.seg}
                    className="flex cursor-pointer items-center gap-2 rounded-md border border-slate-800 bg-slate-900/40 px-2 py-1.5 text-xs text-slate-300 hover:bg-slate-800/50"
                    onClick={() => {
                      setSelPath([...selPath, d.seg]);
                      setSelFile(null);
                    }}
                  >
                    <Folder size={14} className="shrink-0 text-slate-500" />
                    <span className="min-w-0 flex-1 truncate">
                      {selPath.length === 0 ? langName(d.seg) : d.seg}
                    </span>
                    <span className="shrink-0 text-[10px] text-slate-500">{d.count}</span>
                  </div>
                ))}
                {directFiles.map((e) => renderRow(e, false))}
              </>
            )}
          </div>
        </div>
      </div>

      {/* 详情抽屉（overlay 弹层：蒙层点击关闭；z 序高于右上角窗口控件 z-[140]、低于引入/移动弹窗） */}
      {selectedEntry && (
        <>
          <div
            className="fixed inset-0 z-[142] bg-black/50"
            onClick={() => setSelFile(null)}
          />
          <aside
            className="fixed right-0 top-0 z-[142] flex h-full w-96 max-w-[92vw] flex-col border-l border-slate-700 bg-slate-900 shadow-2xl"
            onClick={(e) => e.stopPropagation()}
          >
            <div className="min-h-0 flex-1 overflow-y-auto p-5 text-sm">
            <div className="mb-3 flex items-start justify-between gap-1">
              <div className="min-w-0 flex-1 text-base font-semibold text-foreground">
                {selectedEntry.name || splitFile(selectedEntry.file).name}
                {selectedEntry.sizeClass && (
                  <span className="ml-2 rounded bg-slate-800 px-1.5 py-0.5 text-[10px] font-medium text-slate-300">
                    {selectedEntry.sizeClass}
                  </span>
                )}
              </div>
              <button
                type="button"
                className="rounded p-0.5 text-slate-500 hover:bg-slate-800 hover:text-white"
                onClick={() => setSelFile(null)}
              >
                <X size={14} />
              </button>
            </div>
            {selectedEntry.modifiedAt ? (
              <p className="mb-1.5 text-slate-400">
                {t.realm.mapsDetailDate}：
                {new Date(selectedEntry.modifiedAt).toLocaleDateString()}
              </p>
            ) : null}
            <p className="mb-1.5 text-slate-400">
              {t.realm.mapsDetailVersion}：{selectedEntry.version || "—"}
              {" · "}
              {t.realm.mapsDetailLang}：{langBadge(selectedEntry.lang)}
            </p>
            {selectedEntry.description && (
              <p className="mb-3 whitespace-pre-line border-t border-slate-800 pt-3 leading-relaxed text-slate-300">
                {selectedEntry.description}
              </p>
            )}
            <p className="mb-1.5 text-slate-400">
              {t.realm.mapsDetailVictory}：{vicLabel(selectedEntry.victory)}
            </p>
            <p className="mb-1.5 text-slate-400">
              {t.realm.mapsDetailLoss}：{lossLabel(selectedEntry.loss)}
            </p>
            <p className="mb-1.5 text-slate-400">
              {t.realm.mapsDetailDifficulty}：{diffLabel(selectedEntry.difficulty)}
            </p>
            {selectedEntry.players !== undefined && selectedEntry.players >= 0 && (
              <p className="mb-1.5 text-slate-400">
                {t.realm.mapsDetailPlayers}：{selectedEntry.humans ?? 0} / {selectedEntry.players}
              </p>
            )}
            {selectedEntry.kind === "campaign" &&
              (selectedEntry.scenarioCount ?? 0) > 0 && (
                <div className="mt-3 border-t border-slate-800 pt-3">
                  <p className="mb-1.5 font-medium text-slate-200">
                    {t.realm.mapsDetailScenarios}（{selectedEntry.scenarioCount}）
                  </p>
                  <ol className="list-decimal space-y-1 pl-5 text-slate-400">
                    {(selectedEntry.scenarios ?? []).map((s, i) => {
                      // 场景名可为空（作者未填）：回退地图文件名 → 「#序号」
                      const label = s.name || s.mapFile || `#${i + 1}`;
                      return (
                        <li key={i} className="flex items-baseline gap-2" title={label}>
                          <span className="min-w-0 flex-1 truncate">{label}</span>
                          {s.mapFile && s.name && (
                            <span className="shrink-0 font-mono text-[10px] text-slate-600">
                              {s.mapFile}
                            </span>
                          )}
                          {s.difficulty >= 0 && (
                            <span
                              className="shrink-0 text-[10px] text-slate-500"
                              title={t.realm.mapsDetailDifficulty}
                            >
                              {diffLabel(s.difficulty)}
                            </span>
                          )}
                        </li>
                      );
                    })}
                  </ol>
                </div>
              )}
            </div>
          </aside>
        </>
      )}

      {/* 引入弹窗（三步） */}
      {importOpen && (
        <div className="fixed inset-0 z-[150] flex items-center justify-center bg-black/60 p-4">
          <div className="w-full max-w-lg rounded-xl border border-slate-700 bg-slate-900 p-4 shadow-2xl">
            {importStep === "pick" && (
              <>
                <h3 className="mb-3 text-sm font-semibold text-foreground">{t.realm.mapsImport}</h3>
                <div className="mb-3 flex items-center gap-2">
                  <span className="shrink-0 text-xs text-slate-400">{t.realm.mapsDirLabel}</span>
                  <input
                    value={importDir}
                    onChange={(e) => setImportDir(e.target.value)}
                    className="min-w-0 flex-1 rounded border border-slate-700 bg-slate-900 px-2 py-1.5 text-xs text-foreground"
                  />
                  <button
                    type="button"
                    onClick={handleBrowseDir}
                    className="shrink-0 rounded border border-slate-700 px-2 py-1.5 text-xs text-slate-300 hover:bg-slate-800/50"
                  >
                    {t.realm.mapsBrowse}
                  </button>
                </div>
                <button
                  type="button"
                  onClick={handleImportFile}
                  className="mb-3 text-xs text-indigo-300 hover:underline"
                >
                  {t.realm.mapsImportTip}
                </button>
                <div className="flex justify-end gap-2">
                  <button
                    type="button"
                    onClick={() => setImportOpen(false)}
                    className="rounded-lg border border-slate-700 px-3 py-1.5 text-xs text-slate-300 hover:bg-slate-800"
                  >
                    {t.common.cancel}
                  </button>
                  <button
                    type="button"
                    disabled={!importDir.trim() || busy}
                    onClick={handleScan}
                    className="rounded-lg bg-blue-500 px-3 py-1.5 text-xs font-medium text-white hover:opacity-90 disabled:opacity-40"
                  >
                    {t.realm.mapsScan}
                  </button>
                </div>
              </>
            )}
            {importStep === "preview" && report && (
              <>
                <h3 className="mb-2 text-sm font-semibold text-foreground">
                  {t.realm.mapsFound.replace("{dir}", importDir).replace("{n}", String(report.total))}
                </h3>
                <p className="mb-1 text-xs text-muted">
                  {t.realm.mapsKindDist}：
                  {Object.entries(report.byKind)
                    .map(([k, v]) => `${k} ${v}`)
                    .join(" | ")}
                </p>
                <p className="mb-2 text-xs text-muted">
                  {t.realm.mapsLangDist}：
                  {Object.entries(report.byLang)
                    .map(([k, v]) => `${langName(k)} ${v}`)
                    .join(" | ")}
                </p>
                {report.conflicts.length > 0 && (
                  <div className="mb-2 max-h-28 overflow-y-auto rounded border border-amber-600/40 bg-amber-500/10 p-2">
                    <p className="mb-1 text-xs text-amber-300">
                      {t.realm.mapsConflicts.replace("{n}", String(report.conflicts.length))}
                    </p>
                    {report.conflicts.map((c) => (
                      <p key={c} className="truncate text-[10px] text-amber-200/80" title={c}>
                        {c}
                      </p>
                    ))}
                  </div>
                )}
                {report.conflicts.length > 0 && (
                  <div className="mb-2 flex items-center gap-3 text-xs text-slate-300">
                    {(
                      [
                        ["skip", t.realm.mapsConflictSkip],
                        ["overwrite", t.realm.mapsConflictOverwrite],
                        ["rename", t.realm.mapsConflictRename],
                      ] as const
                    ).map(([v, label]) => (
                      <label key={v} className="flex cursor-pointer items-center gap-1">
                        <input
                          type="radio"
                          checked={conflict === v}
                          onChange={() => setConflict(v)}
                          className="accent-blue-500"
                        />
                        {label}
                      </label>
                    ))}
                  </div>
                )}
                <label className="mb-3 flex cursor-pointer items-center gap-1.5 text-xs text-foreground">
                  <input
                    type="checkbox"
                    checked={keepStructure}
                    onChange={(e) => setKeepStructure(e.target.checked)}
                    className="h-3.5 w-3.5 accent-blue-500"
                  />
                  {t.realm.mapsKeepStructure}
                </label>
                <div className="flex justify-end gap-2">
                  <button
                    type="button"
                    onClick={() => setImportOpen(false)}
                    className="rounded-lg border border-slate-700 px-3 py-1.5 text-xs text-slate-300 hover:bg-slate-800"
                  >
                    {t.common.cancel}
                  </button>
                  <button
                    type="button"
                    disabled={busy}
                    onClick={handleImportConfirm}
                    className="rounded-lg bg-blue-500 px-3 py-1.5 text-xs font-medium text-white hover:opacity-90 disabled:opacity-40"
                  >
                    {t.realm.mapsImportConfirm}
                  </button>
                </div>
              </>
            )}
            {importStep === "done" && report && (
              <>
                <h3 className="mb-3 text-sm font-semibold text-foreground">
                  {t.realm.mapsImportDone
                    .replace("{a}", String(report.added))
                    .replace("{s}", String(report.skipped))
                    .replace("{f}", String(report.failed))}
                </h3>
                <div className="flex justify-end">
                  <button
                    type="button"
                    onClick={() => setImportOpen(false)}
                    className="rounded-lg bg-blue-500 px-3 py-1.5 text-xs font-medium text-white hover:opacity-90"
                  >
                    {t.common.ok}
                  </button>
                </div>
              </>
            )}
          </div>
        </div>
      )}

      {/* 移动弹窗 */}
      {moveFile && (
        <div className="fixed inset-0 z-[150] flex items-center justify-center bg-black/60 p-4">
          <div className="w-full max-w-sm rounded-xl border border-slate-700 bg-slate-900 p-4 shadow-2xl">
            <h3 className="mb-3 text-sm font-semibold text-foreground">{t.realm.mapsMoveTitle}</h3>
            <div className="mb-3 max-h-56 space-y-1 overflow-y-auto">
              {moveCandidates.length === 0 ? (
                <p className="text-xs text-muted">{t.realm.mapsEmpty}</p>
              ) : (
                moveCandidates.map((d) => (
                  <label
                    key={d}
                    className={cn(
                      "flex cursor-pointer items-center gap-2 rounded px-2 py-1 text-xs",
                      moveDest === d ? "bg-blue-500/20 text-blue-200" : "text-slate-300 hover:bg-slate-800/60",
                    )}
                  >
                    <input
                      type="radio"
                      checked={moveDest === d}
                      onChange={() => setMoveDest(d)}
                      className="accent-blue-500"
                    />
                    <Folder size={13} className="shrink-0 text-slate-500" />
                    <span className="min-w-0 flex-1 truncate">
                      {splitFile(`${d}/x`).dirs.length ? d : d || "/"}
                    </span>
                  </label>
                ))
              )}
            </div>
            <div className="flex justify-end gap-2">
              <button
                type="button"
                onClick={() => setMoveFile(null)}
                className="rounded-lg border border-slate-700 px-3 py-1.5 text-xs text-slate-300 hover:bg-slate-800"
              >
                {t.common.cancel}
              </button>
              <button
                type="button"
                disabled={!moveDest}
                onClick={handleMove}
                className="rounded-lg bg-blue-500 px-3 py-1.5 text-xs font-medium text-white hover:opacity-90 disabled:opacity-40"
              >
                {t.common.ok}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* 删除确认 */}
      <ConfirmDialog
        open={!!delFile}
        title={t.realm.mapsDeleteConfirmTitle}
        description={t.realm.mapsDeleteConfirmDesc.replace(
          "{name}",
          delFile ? (entries.find((e) => e.file === delFile)?.name || splitFile(delFile).name) : "",
        )}
        danger
        confirmText={t.realm.deleteMap}
        cancelText={t.common.cancel}
        onConfirm={handleDelete}
        onCancel={() => setDelFile(null)}
      />
    </div>
  );
}
