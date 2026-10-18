/**
 * 「导入游戏文件」单页四态弹层（底栏「游戏数据」左侧入口）。
 *
 * 设计定稿（docs/features/game-import.zh-CN.md）：
 * - **单页四态**：空态（缺省描述 + 拖放区）→ 有文件态（自动预检徽标 + `[改判▾]`）
 *   → 执行中（禁关闭）→ 结果态（就地切换汇总）；
 * - 弃用四步向导与「按类型分 tab」：核心输入是 zip / 拖放，拖入前用户并不知道内容，
 *   这正是「自动识别」的用武之地；「支持哪些类型」用常显的缺省描述区满足（可折叠）；
 * - 拖放为一级交互：窗口级 `onDragDropEvent`（App 唯一监听）驱动——`over` 时整层
 *   描边高亮 + 文案切换；Tauri 无落点判定，故高亮对象是整个弹层而非局部（与官方
 *   「拖到窗口任意位置」一致）；
 * - zip 识别透明化：徽标展示识别结论 + 计数，未识别时徽标注明「按模组尝试」，
 *   逐项 `[改判▾]`（自动 / 模组 / 地图包 / 存档包）即重预检。
 *
 * 会话语义：每次「关闭 → 打开」重置全部状态并消费 App 预填的种子路径；
 * 打开期间种子路径的 nonce 变化 → 追加合并（drag drop 持续可用）。
 */

import { Fragment, useEffect, useMemo, useRef, useState } from "react";
import { FolderOpen, HelpCircle, Loader2, Upload, X } from "lucide-react";

import { useI18n } from "@/i18n";
import { realmName } from "@/lib/realm/intro";
import { useRealms } from "@/lib/realm/store";
import {
  importGameFiles,
  isVcmiRunning,
  openInExplorer,
  pickDirectory,
  pickOpenPaths,
  type GameImportConflict,
  type GameImportDirPreview,
  type GameImportExecItem,
  type GameImportExecReport,
  type GameImportOverride,
  type GameImportPreviewItem,
  type GameImportPreviewReport,
  type GameImportZipPreview,
} from "@/lib/tauri";
import { Button } from "@/shared/ui/button";
import { cn } from "@/lib/utils";
import { backendErrorText } from "@/lib/backendError";

/** 「选择文件」对话框支持的全部扩展名（首位分组聚合用；与后端 `classify_input` 白名单一致） */
const ALL_IMPORT_EXTS = ["h3m", "vmap", "h3c", "vcmp", "zip", "json", "vsgm1", "exe"];

/** 扩展名 → 后端 kind：预检未返回时供过滤器兜底，避免筛选视图瞬间闪空 */
const EXT_KIND: Record<string, string> = {
  h3m: "map",
  vmap: "map",
  h3c: "campaign",
  vcmp: "campaign",
  vsgm1: "save",
  zip: "zip",
  json: "config",
  exe: "chronicles",
};

interface ImportFilesDialogProps {
  open: boolean;
  /** 全窗拖放悬停中（App 的 onDragDropEvent 驱动，弹层打开时整层描边高亮） */
  dragActive: boolean;
  /** App 预填列表（打开时消费；打开期间 nonce 变化 → 追加） */
  seedPaths: string[];
  seedNonce: number;
  onClose: () => void;
}

/** 去重（Windows 路径大小写不敏感；保留首次出现的原样路径） */
function dedupePaths(list: string[]): string[] {
  const seen = new Set<string>();
  const out: string[] = [];
  for (const p of list) {
    const key = p.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    out.push(p);
  }
  return out;
}

function baseName(p: string): string {
  return p.split(/[\\/]/).filter(Boolean).pop() ?? p;
}

/** 归一化扩展名（小写、去前导点；后端 `ext` 即小写无点，路径推导时统一） */
function normalizeExt(s: string): string {
  return s.trim().toLowerCase().replace(/^\./, "");
}

/** 从路径推导扩展名（目录 / 无扩展名 → 空串） */
function pathExt(p: string): string {
  const base = baseName(p);
  const i = base.lastIndexOf(".");
  return i > 0 ? base.slice(i + 1) : "";
}

function formatSize(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
}

/** 执行项 dest 语义：map/campaign/save/config 为文件路径（取父目录）；zip(mod/maps/saves) 与 hd 已是目录 */
function execDestDir(it: GameImportExecItem): string | null {
  if (!it.dest) return null;
  if (it.kind === "map" || it.kind === "campaign" || it.kind === "save" || it.kind === "config") {
    return it.dest.replace(/[\\/][^\\/]*$/, "") || it.dest;
  }
  return it.dest;
}

export function ImportFilesDialog({
  open,
  dragActive,
  seedPaths,
  seedNonce,
  onClose,
}: ImportFilesDialogProps) {
  const { t, lang } = useI18n();
  const { realms, activeRealm } = useRealms();
  const activeRealmObj = realms.find((r) => r.id === activeRealm) ?? null;
  const activeName = activeRealmObj ? realmName(activeRealmObj, lang) : "";

  const [paths, setPaths] = useState<string[]>([]);
  const [conflict, setConflict] = useState<GameImportConflict>("skip");
  const [overrides, setOverrides] = useState<Record<string, GameImportOverride>>({});
  const [phase, setPhase] = useState<"select" | "executing" | "done">("select");
  const [preview, setPreview] = useState<GameImportPreviewReport | null>(null);
  const [scanning, setScanning] = useState(false);
  const [exec, setExec] = useState<GameImportExecReport | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [typesOpen, setTypesOpen] = useState(true);
  const [filter, setFilter] = useState("all");
  const [vcmiRunning, setVcmiRunning] = useState(false);
  // 执行失败后回到 select 时强制重预检（paths/overrides 未变，需显式触发）
  const [refreshNonce, setRefreshNonce] = useState(0);

  const openedRef = useRef(false);
  const seedNonceRef = useRef(seedNonce);
  const seqRef = useRef(0);
  const executingRef = useRef(false);
  executingRef.current = phase === "executing";
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const prevPathCountRef = useRef(0);
  /** 追加文件后保持滚到底，直到本次预检结束（预检完成会加高条目，需二次跟随） */
  const stickBottomRef = useRef(false);

  // —— 会话重置：关闭 → 打开时清空并消费种子；打开期间 nonce 变化 → 追加 ——
  useEffect(() => {
    if (open && !openedRef.current) {
      openedRef.current = true;
      seedNonceRef.current = seedNonce;
      setPaths(dedupePaths(seedPaths));
      setConflict("skip");
      setOverrides({});
      setPhase("select");
      setPreview(null);
      setScanning(false);
      setExec(null);
      setError(null);
      setTypesOpen(true);
      setFilter("all");
      void isVcmiRunning().then(setVcmiRunning).catch(() => setVcmiRunning(false));
    } else if (!open && openedRef.current) {
      openedRef.current = false;
    }
  }, [open, seedPaths, seedNonce]);

  useEffect(() => {
    if (!open || phase === "executing") return;
    if (seedNonce === seedNonceRef.current) return;
    seedNonceRef.current = seedNonce;
    if (seedPaths.length === 0) return;
    setPaths((prev) => dedupePaths([...prev, ...seedPaths]));
    // 结果态再拖入 → 回到有文件态重新预检
    if (phase === "done") {
      setPhase("select");
      setExec(null);
    }
  }, [open, phase, seedNonce, seedPaths]);

  // —— 预检（有文件态）：paths / overrides 变化即 dryRun；seq 防竞态 ——
  useEffect(() => {
    if (!open || phase !== "select") return;
    if (paths.length === 0) {
      setPreview(null);
      setScanning(false);
      setError(null);
      return;
    }
    const seq = ++seqRef.current;
    setScanning(true);
    importGameFiles({ paths, conflict, dryRun: true, overrides })
      .then((rep) => {
        if (seq !== seqRef.current) return;
        setPreview(rep);
        setScanning(false);
        setError(null);
      })
      .catch((e: unknown) => {
        if (seq !== seqRef.current) return;
        setPreview(null);
        setScanning(false);
        setError(
          t.importFiles.preflightFail.replace("{msg}", backendErrorText(e)),
        );
      });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, phase, paths, overrides, refreshNonce]);

  // —— Esc 关闭（执行中禁用）——
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !executingRef.current) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  // —— 滚动区：追加文件时滚到底（含预检完成后的高度变化）；切到结果态回顶部 ——
  useEffect(() => {
    if (paths.length > prevPathCountRef.current) stickBottomRef.current = true;
    prevPathCountRef.current = paths.length;
  }, [paths.length]);

  useEffect(() => {
    if (!stickBottomRef.current) return;
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [paths.length, preview]);

  useEffect(() => {
    if (!scanning) stickBottomRef.current = false;
  }, [scanning]);

  useEffect(() => {
    if (phase !== "done") return;
    const el = scrollRef.current;
    if (el) el.scrollTop = 0;
  }, [phase]);

  // —— 选入文件后自动收起缺省描述（不再占列表空间）；入口转为头部类型徽标 ——
  useEffect(() => {
    if (paths.length > 0) setTypesOpen(false);
  }, [paths.length]);

  const previewByPath = useMemo(
    () => new Map((preview?.items ?? []).map((it) => [it.path, it])),
    [preview],
  );

  /** 描述区与头部类型徽标严格二选一（结果态两者皆无） */
  const descVisible = phase !== "done" && typesOpen;
  const badgeVisible = phase !== "done" && !typesOpen;

  const addPaths = (list: string[]) => setPaths((prev) => dedupePaths([...prev, ...list]));

  const removePath = (p: string) => {
    setPaths((prev) => prev.filter((x) => x !== p));
    setOverrides((prev) => {
      if (!(p in prev)) return prev;
      const next = { ...prev };
      delete next[p];
      return next;
    });
  };

  /** 列表视图过滤器：按识别类型（kind）归类，与对话框扩展名分组互不影响 */
  const filters: { key: string; label: string; kinds?: string[] }[] = [
    { key: "all", label: t.importFiles.filterAll },
    { key: "map", label: t.importFiles.filterMap, kinds: ["map"] },
    { key: "campaign", label: t.importFiles.filterCampaign, kinds: ["campaign"] },
    { key: "save", label: t.importFiles.filterSave, kinds: ["save"] },
    { key: "mod", label: t.importFiles.filterMod, kinds: ["zip"] },
    { key: "config", label: t.importFiles.filterConfig, kinds: ["config"] },
    { key: "chronicles", label: t.importFiles.filterChronicles, kinds: ["chronicles"] },
    { key: "hd", label: t.importFiles.filterHd, kinds: ["hd"] },
    { key: "dir", label: t.importFiles.filterDir, kinds: ["dir"] },
  ];

  /** 条目类型：优先后端识别结果，预检期间按扩展名兜底 */
  const kindOf = (p: string) =>
    previewByPath.get(p)?.kind ?? EXT_KIND[normalizeExt(pathExt(p))] ?? "unsupported";

  const filterKinds = filters.find((f) => f.key === filter)?.kinds;
  /** 有文件态列表按筛选过滤显示；导入始终包含全部已选文件 */
  const visiblePaths = filterKinds
    ? paths.filter((p) => filterKinds.includes(kindOf(p)))
    : paths;

  /**
   * 「选择文件」——全类型的唯一入口（HD 素材直接选 `HOMM3 2.0.exe` 本体，或拖入整个目录）。
   * 分组固定：首位聚合全部扩展名，避免「全部」状态下对话框退化为 (*.*)。
   */
  const handlePickFiles = async () => {
    const picked = await pickOpenPaths(
      [
        { name: t.importFiles.dlgFilterAll, extensions: ALL_IMPORT_EXTS },
        { name: t.importFiles.filterMap, extensions: ["h3m", "vmap"] },
        { name: t.importFiles.filterCampaign, extensions: ["h3c", "vcmp"] },
        { name: t.importFiles.filterSave, extensions: ["vsgm1"] },
        { name: t.importFiles.filterMod, extensions: ["zip"] },
        { name: t.importFiles.filterConfig, extensions: ["json"] },
        { name: t.importFiles.dlgFilterExe, extensions: ["exe"] },
      ],
      t.importFiles.pickFilesTitle,
    );
    if (picked.length) addPaths(picked);
  };

  /**
   * 「选择目录…」——递归导入目录内受支持文件（后端按类型分流、保留子目录结构）。
   *
   * 原生对话框无法在同一会话里混选文件与目录（directory 与 filters 互斥），故保留两个入口。
   */
  const handlePickDir = async () => {
    const dir = await pickDirectory(t.importFiles.pickDirTitle);
    if (dir) addPaths([dir]);
  };

  // —— 执行导入 ——
  const handleStart = async () => {
    if (paths.length === 0 || phase === "executing") return;
    setPhase("executing");
    setError(null);
    seqRef.current++; // 使 in-flight 预检失效
    try {
      const rep = await importGameFiles({ paths, conflict, dryRun: false, overrides });
      setExec(rep);
      setPhase("done");
      // 已导入项即刻清理：结果态只展示 exec，再拖入新文件时列表从干净开始
      setPaths([]);
      setPreview(null);
      setOverrides({});
      if (rep.modsInstalled.length > 0) {
        window.dispatchEvent(
          new CustomEvent("vcmi-fv:mods-installed", { detail: { mods: rep.modsInstalled } }),
        );
      }
    } catch (e: unknown) {
      setPhase("select");
      setRefreshNonce((n) => n + 1);
      setError(t.importFiles.execFail.replace("{msg}", backendErrorText(e)));
    }
  };

  // —— 徽标 ——
  const kindBadge = (kind: string | undefined): { label: string; cls: string } => {
    switch (kind) {
      case "map":
        return { label: t.importFiles.kindMap, cls: "border-indigo-500/50 bg-indigo-500/10 text-indigo-200" };
      case "campaign":
        return { label: t.importFiles.kindCampaign, cls: "border-violet-500/50 bg-violet-500/10 text-violet-200" };
      case "zip":
        return { label: t.importFiles.kindZip, cls: "border-sky-500/50 bg-sky-500/10 text-sky-200" };
      case "config":
        return { label: t.importFiles.kindConfig, cls: "border-amber-500/50 bg-amber-500/10 text-amber-200" };
      case "save":
        return { label: t.importFiles.kindSave, cls: "border-teal-500/50 bg-teal-500/10 text-teal-200" };
      case "chronicles":
        return { label: t.importFiles.kindChronicles, cls: "border-orange-500/50 bg-orange-500/10 text-orange-200" };
      case "hd":
        return { label: t.importFiles.kindHd, cls: "border-fuchsia-500/50 bg-fuchsia-500/10 text-fuchsia-200" };
      case "dir":
        return { label: t.importFiles.kindDir, cls: "border-slate-400/50 bg-slate-400/10 text-slate-200" };
      default:
        return { label: t.importFiles.kindNone, cls: "border-slate-600/60 bg-slate-700/30 text-slate-400" };
    }
  };

  const zipBadgeText = (z: GameImportZipPreview): string => {
    if (z.detected === "maps") {
      const n = t.importFiles.zipMapsCount.replace("{n}", String(z.mapCount));
      return `${t.importFiles.zipAsMaps} ${n}`;
    }
    if (z.detected === "saves") {
      const n = t.importFiles.zipSavesCount.replace("{n}", String(z.saveCount));
      return `${t.importFiles.zipAsSaves} ${n}`;
    }
    let text = z.hasModJson ? t.importFiles.zipAsMod : t.importFiles.zipTryMod;
    if (z.name) text += ` · ${z.name}${z.version ? ` v${z.version}` : ""}`;
    return text;
  };

  /** 目录明细：仅列出非零类型（用类型全名，避免单字徽标歧义） */
  const dirBreakdown = (d: GameImportDirPreview): string => {
    const parts: string[] = [];
    const push = (n: number, label: string) => {
      if (n > 0) parts.push(`${label} ${n}`);
    };
    push(d.mapCount, t.importFiles.typeMapT);
    push(d.campaignCount, t.importFiles.typeCampaignT);
    push(d.saveCount, t.importFiles.typeSaveT);
    push(d.configCount, t.importFiles.typeConfigT);
    push(d.zipCount, t.importFiles.typeZipT);
    return parts.join(" · ");
  };

  /** 目录项统计行：「6 个文件 · 地图 2 · 存档 1 … · 忽略 3」 */
  const dirText = (d: GameImportDirPreview): string => {
    const total = d.mapCount + d.campaignCount + d.saveCount + d.configCount + d.zipCount;
    return [
      t.importFiles.dirFiles.replace("{n}", String(total)),
      dirBreakdown(d),
      d.ignored > 0 ? t.importFiles.dirIgnored.replace("{n}", String(d.ignored)) : "",
    ]
      .filter(Boolean)
      .join(" · ");
  };

  const statusBadge = (status: string): { label: string; cls: string } => {
    switch (status) {
      case "added":
        return { label: t.importFiles.statusAdded, cls: "border-emerald-500/40 bg-emerald-500/10 text-emerald-300" };
      case "replaced":
        return { label: t.importFiles.statusReplaced, cls: "border-emerald-500/40 bg-emerald-500/10 text-emerald-300" };
      case "skipped":
        return { label: t.importFiles.statusSkipped, cls: "border-slate-600/50 bg-slate-700/30 text-slate-300" };
      case "unsupported":
        return { label: t.importFiles.statusUnsupported, cls: "border-slate-600/50 bg-slate-700/30 text-slate-400" };
      default:
        return { label: t.importFiles.statusFailed, cls: "border-red-500/40 bg-red-500/10 text-red-300" };
    }
  };

  const typeRows: [string, string][] = [
    [t.importFiles.typeMapT, t.importFiles.typeMapD],
    [t.importFiles.typeCampaignT, t.importFiles.typeCampaignD],
    [t.importFiles.typeZipT, t.importFiles.typeZipD],
    [t.importFiles.typeConfigT, t.importFiles.typeConfigD],
    [t.importFiles.typeSaveT, t.importFiles.typeSaveD],
    [t.importFiles.typeChroniclesT, t.importFiles.typeChroniclesD],
    [t.importFiles.typeHdT, t.importFiles.typeHdD],
    [t.importFiles.typeDirT, t.importFiles.typeDirD],
  ];

  /** 类型清单表格（空态描述区与头部徽标气泡共用） */
  const typeTable = (
    <div className="grid grid-cols-[5.5rem_1fr] gap-x-3 gap-y-1.5 text-xs">
      {typeRows.map(([k, d]) => (
        <Fragment key={k}>
          <span className="text-slate-300">{k}</span>
          <span className="min-w-0 break-words text-slate-400">{d}</span>
        </Fragment>
      ))}
    </div>
  );

  const conflicts = preview?.summary.conflicts ?? 0;
  const conflictActionLabel =
    conflict === "skip"
      ? t.importFiles.conflictActionSkip
      : conflict === "overwrite"
        ? t.importFiles.conflictActionOverwrite
        : t.importFiles.conflictActionRename;

  const openDirs = useMemo(() => {
    if (!exec) return [] as string[];
    const dirs = new Set<string>();
    for (const it of exec.items) {
      const d = execDestDir(it);
      if (d) dirs.add(d);
    }
    return [...dirs];
  }, [exec]);

  const hasHdDone =
    !!exec?.items.some((i) => i.kind === "hd" && (i.status === "added" || i.status === "replaced"));
  const hasConfigDone =
    !!exec?.items.some(
      (i) => i.kind === "config" && (i.status === "added" || i.status === "replaced"),
    );
  const hasChronicles = !!exec?.items.some((i) => i.kind === "chronicles");
  const hasDoneHints =
    !!exec && (exec.modsInstalled.length > 0 || hasHdDone || hasConfigDone || hasChronicles);

  if (!open) return null;

  return (
    <div className="fixed inset-0 z-[150] flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm">
      <div
        role="dialog"
        aria-modal="true"
        className={cn(
          "relative flex max-h-[86vh] w-full max-w-3xl flex-col overflow-hidden rounded-2xl border bg-slate-900 shadow-2xl transition-colors",
          dragActive ? "border-indigo-400/80" : "border-slate-700/60",
        )}
      >
        {/* 拖放高亮（Tauri 无落点判定 → 高亮整层，与「拖到窗口任意位置」一致） */}
        {dragActive && (
          <div className="pointer-events-none absolute inset-0 z-20 flex items-center justify-center rounded-2xl border-2 border-dashed border-indigo-400 bg-indigo-500/10">
            <span className="rounded-xl bg-slate-950/85 px-4 py-2 text-sm font-medium text-indigo-200 shadow-xl">
              {t.importFiles.dropActive}
            </span>
          </div>
        )}

        {/* 头部 */}
        <div className="flex items-start justify-between gap-3 border-b border-slate-800/80 px-5 py-4">
          <div className="min-w-0">
            <h3 className="text-base font-semibold text-slate-100">{t.importFiles.title}</h3>
            <p className="mt-0.5 text-xs text-slate-400">
              {phase === "executing"
                ? t.importFiles.executing
                : phase === "done"
                  ? t.importFiles.resultTitle
                  : t.importFiles.sub}
            </p>
          </div>
          <div className="flex flex-none items-center gap-2">
            {/* 类型徽标：描述收起后（有文件态）的类型唯一入口；hover / 聚焦弹出清单气泡 */}
            {badgeVisible && (
              <div className="group relative flex-none">
                <button
                  type="button"
                  className="flex items-center gap-1 rounded-md border border-slate-700/60 bg-slate-800/40 px-2 py-1 text-[11px] text-slate-400 transition-colors group-hover:border-slate-600 group-hover:text-slate-200"
                >
                  <HelpCircle className="h-3.5 w-3.5" />
                  {t.importFiles.typesBadge.replace("{n}", String(typeRows.length))}
                </button>
                <div className="pointer-events-none absolute right-0 top-full z-30 mt-2 hidden w-[440px] max-w-[70vw] rounded-xl border border-slate-700 bg-slate-900 px-4 py-3 shadow-2xl group-hover:block group-focus-within:block">
                  <div className="mb-2 text-[11px] font-semibold uppercase tracking-wide text-slate-400">
                    {t.importFiles.typesTitle}
                  </div>
                  {typeTable}
                </div>
              </div>
            )}
            <button
              type="button"
              onClick={onClose}
              disabled={phase === "executing"}
              title={t.importFiles.close}
              className="rounded p-1.5 text-slate-500 transition-colors hover:bg-slate-800 hover:text-slate-200 disabled:cursor-not-allowed disabled:opacity-40"
            >
              <X className="h-4 w-4" />
            </button>
          </div>
        </div>

        {/* 执行中：顶部细进度条（无逐字节进度，二期加事件） */}
        {phase === "executing" && (
          <div className="h-0.5 w-full overflow-hidden bg-slate-800">
            <div className="h-full w-1/3 animate-pulse rounded-full bg-indigo-400" />
          </div>
        )}

        {/* 主体：固定块 + 独立滚动列表（列表再长，操作栏/过滤器/冲突策略恒可见） */}
        <div className="flex min-h-0 flex-1 flex-col">
          {/* 固定：缺省描述（空态默认展开；选入文件后自动收起为头部徽标） */}
          {descVisible && (
            <section className="flex-none border-b border-slate-800/70 px-5 py-3">
              <div className="rounded-xl border border-slate-700/50 bg-slate-950/50 p-3">
                <div className="mb-2 flex items-center justify-between">
                  <h4 className="text-[11px] font-semibold uppercase tracking-wide text-slate-400">
                    {t.importFiles.typesTitle}
                  </h4>
                  <button
                    type="button"
                    onClick={() => setTypesOpen(false)}
                    className="text-[11px] text-slate-500 transition-colors hover:text-slate-300"
                  >
                    {t.importFiles.typesCollapse} ▲
                  </button>
                </div>
                {typeTable}
              </div>
            </section>
          )}

          {/* 固定：操作栏 + 拖放提示 + 已选计数（结果态隐藏） */}
          {phase !== "done" && (
            <section className="flex-none border-b border-slate-800/70 px-5 py-3">
              <div className="flex flex-wrap items-center gap-2">
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => void handlePickFiles()}
                  disabled={phase === "executing"}
                >
                  {t.importFiles.pickFiles}
                </Button>
                <Button
                  size="sm"
                  variant="outline"
                  onClick={() => void handlePickDir()}
                  disabled={phase === "executing"}
                >
                  <FolderOpen className="h-3.5 w-3.5" />
                  {t.importFiles.pickDir}
                </Button>
                <div className="ml-auto flex flex-wrap items-center gap-1 text-[11px] text-slate-500">
                  <span className="mr-0.5">{t.importFiles.filterLabel}</span>
                  {filters.map((f) => (
                    <button
                      key={f.key}
                      type="button"
                      onClick={() => setFilter(f.key)}
                      className={cn(
                        "rounded-full border px-2 py-0.5 transition-colors",
                        filter === f.key
                          ? "border-indigo-500/60 bg-indigo-500/15 text-indigo-200"
                          : "border-slate-700/60 text-slate-400 hover:border-slate-600 hover:text-slate-200",
                      )}
                    >
                      {f.label}
                    </button>
                  ))}
                </div>
              </div>
              <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1.5">
                {/* 拖放提示仅空态需要；有文件态让位给计数行（列表更高） */}
                {paths.length === 0 ? (
                  <div className="flex min-w-0 flex-1 items-center gap-2 rounded-xl border border-dashed border-slate-700/70 px-3 py-2 text-xs text-slate-400">
                    <Upload className="h-3.5 w-3.5 flex-none" />
                    <span className="min-w-0">{t.importFiles.dropHint}</span>
                  </div>
                ) : (
                  <div className="ml-auto flex flex-none items-center gap-2 text-xs">
                    <span className="text-slate-400">
                      {t.importFiles.selectedCount.replace("{n}", String(paths.length))}
                    </span>
                    {filterKinds && (
                      <span className="text-slate-500">
                        · {t.importFiles.filterShowing.replace("{n}", String(visiblePaths.length))}
                      </span>
                    )}
                    {scanning && <span className="text-slate-500">{t.importFiles.scanning}</span>}
                    <button
                      type="button"
                      onClick={() => {
                        setPaths([]);
                        setOverrides({});
                      }}
                      disabled={phase === "executing"}
                      className="text-[11px] text-slate-500 transition-colors hover:text-slate-300 disabled:cursor-not-allowed disabled:opacity-40"
                    >
                      {t.importFiles.clear}
                    </button>
                  </div>
                )}
              </div>
            </section>
          )}

          {/* 固定：结果汇总（结果态） */}
          {phase === "done" && exec && (
            <div className="flex-none border-b border-slate-800/70 px-5 py-3">
              <div className="rounded-xl border border-slate-700/50 bg-slate-950/50 px-3 py-2.5 text-sm text-slate-200">
                {t.importFiles.resultSummary
                  .replace("{added}", String(exec.added))
                  .replace("{replaced}", String(exec.replaced))
                  .replace("{skipped}", String(exec.skipped))
                  .replace("{failed}", String(exec.failed))}
              </div>
            </div>
          )}

          {/* 滚动区：文件列表 / 结果列表（唯一滚动内容） */}
          <div ref={scrollRef} className="min-h-0 flex-1 overflow-y-auto px-5 py-3">
            {/* 有文件态：文件列表（按筛选过滤显示；导入仍为全部已选） */}
            {phase !== "done" && paths.length > 0 && (
              <div className="space-y-1.5">
                {visiblePaths.map((p) => {
                  const item: GameImportPreviewItem | undefined = previewByPath.get(p);
                  const badge = kindBadge(item?.kind);
                  const metaText = item?.meta
                    ? [item.meta.name, item.meta.lang, item.meta.sizeClass].filter(Boolean).join(" · ")
                    : null;
                  const hdText =
                    item?.hd && item.hd.valid
                      ? [
                          item.hd.language || item.hd.languageCode
                            ? t.importFiles.hdLang.replace(
                                "{lang}",
                                item.hd.language ?? item.hd.languageCode ?? "",
                              )
                            : "",
                          t.importFiles.hdPaks
                            .replace("{found}", String(item.hd.paks.found))
                            .replace("{total}", String(item.hd.paks.total)),
                          item.hd.missing.length > 0
                            ? t.importFiles.hdMissing.replace("{list}", item.hd.missing.join(", "))
                            : "",
                        ]
                          .filter(Boolean)
                          .join(" · ")
                      : null;
                  return (
                    <div
                      key={p}
                      className={cn(
                        "rounded-xl border border-slate-800/60 px-3 py-2",
                        item && !item.ok && "opacity-70",
                      )}
                    >
                      <div className="flex items-center gap-2.5">
                        <span
                          className={cn(
                            "inline-flex h-6 min-w-[2.4rem] flex-none items-center justify-center rounded-md border px-1.5 text-[11px] font-semibold",
                            badge.cls,
                          )}
                        >
                          {badge.label}
                        </span>
                        <span className="min-w-0 flex-1 truncate text-sm text-slate-200" title={p}>
                          {baseName(p)}
                        </span>
                        <span className="flex-none text-[11px] text-slate-500">
                          {item ? formatSize(item.size) : ""}
                        </span>
                        {item?.conflict && (
                          <span className="flex-none text-amber-400" title={t.importFiles.conflictLabel}>
                            ⚠
                          </span>
                        )}
                        <button
                          type="button"
                          onClick={() => removePath(p)}
                          disabled={phase === "executing"}
                          title={t.importFiles.removeItem}
                          className="flex-none rounded p-0.5 text-slate-500 transition-colors hover:bg-slate-800 hover:text-slate-200 disabled:cursor-not-allowed disabled:opacity-40"
                        >
                          <X className="h-3.5 w-3.5" />
                        </button>
                      </div>

                      {item && (
                        <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 pl-[4.9rem] text-[11px]">
                          {metaText && <span className="text-slate-400">{metaText}</span>}
                          {item.zip && (
                            <span className="text-sky-300/90">{zipBadgeText(item.zip)}</span>
                          )}
                          {hdText && <span className="text-fuchsia-300/90">{hdText}</span>}
                          {item.dir && <span className="text-slate-300">{dirText(item.dir)}</span>}
                          {item.inno !== undefined && (
                            <span className={item.inno ? "text-emerald-300/90" : "text-slate-400"}>
                              {item.inno ? t.importFiles.innoYes : t.importFiles.innoNo}
                            </span>
                          )}
                          {!item.ok && (
                            <span className="text-slate-400">
                              {item.kind === "unsupported"
                                ? t.importFiles.unsupportedNote
                                : item.reason}
                              {item.kind === "unsupported" && item.reason ? (
                                <span className="ml-1 text-slate-500">· {item.reason}</span>
                              ) : null}
                            </span>
                          )}
                          {item.zip && item.ok && (
                            <label className="flex items-center gap-1.5 text-slate-400">
                              <span>{t.importFiles.overrideLabel}</span>
                              <select
                                value={overrides[p] ?? ""}
                                disabled={phase === "executing"}
                                onChange={(e) => {
                                  const v = e.target.value as GameImportOverride | "";
                                  setOverrides((prev) => {
                                    const next = { ...prev };
                                    if (!v) delete next[p];
                                    else next[p] = v;
                                    return next;
                                  });
                                }}
                                className="rounded-md border border-slate-700/70 bg-slate-900 px-1.5 py-0.5 text-[11px] text-slate-300 outline-none focus:border-indigo-500/60 disabled:cursor-not-allowed disabled:opacity-40"
                              >
                                <option value="">{t.importFiles.overrideAuto}</option>
                                <option value="mod">{t.importFiles.overrideMod}</option>
                                <option value="maps">{t.importFiles.overrideMaps}</option>
                                <option value="saves">{t.importFiles.overrideSaves}</option>
                              </select>
                            </label>
                          )}
                        </div>
                      )}

                      {item?.target && item.ok && (
                        <div className="mt-0.5 truncate pl-[4.9rem] text-[11px] text-slate-500" title={item.target}>
                          → {item.target}
                        </div>
                      )}
                    </div>
                  );
                })}
                {visiblePaths.length === 0 && (
                  <div className="px-1 py-6 text-center text-xs text-slate-500">
                    {t.importFiles.filterEmpty}
                  </div>
                )}
              </div>
            )}

            {/* 结果态：结果列表（可滚动） */}
            {phase === "done" && exec && (
              <div className="space-y-1.5">
                {exec.items.map((it) => {
                  const kb = kindBadge(it.kind);
                  const sb = statusBadge(it.status);
                  return (
                    <div
                      key={it.path}
                      className="flex items-center gap-2.5 rounded-xl border border-slate-800/60 px-3 py-2"
                    >
                      <span
                        className={cn(
                          "inline-flex h-6 min-w-[2.4rem] flex-none items-center justify-center rounded-md border px-1.5 text-[11px] font-semibold",
                          kb.cls,
                        )}
                      >
                        {kb.label}
                      </span>
                      <span className="min-w-0 flex-1 truncate text-sm text-slate-200" title={it.path}>
                        {baseName(it.path)}
                      </span>
                      {it.reason && (
                        <span
                          className="max-w-[40%] flex-none truncate text-[11px] text-slate-500"
                          title={it.reason}
                        >
                          {it.reason}
                        </span>
                      )}
                      <span
                        className={cn(
                          "flex-none rounded-md border px-2 py-0.5 text-[11px] font-medium",
                          sb.cls,
                        )}
                      >
                        {sb.label}
                      </span>
                    </div>
                  );
                })}
              </div>
            )}
          </div>

          {/* 固定：冲突策略（恒可见；从列表尾移出） */}
          {phase !== "done" && paths.length > 0 && (
            <div className="flex-none border-t border-slate-800/70 px-5 py-2.5">
              <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
                <div className="min-w-0 text-xs">
                  {conflicts > 0 && (
                    <span className="text-amber-300">
                      {t.importFiles.conflictCount
                        .replace("{n}", String(conflicts))
                        .replace("{action}", conflictActionLabel)}
                    </span>
                  )}
                  {conflicts > 0 && conflict === "rename" && (
                    <span className="ml-2 text-[11px] text-slate-500">
                      {t.importFiles.conflictRenameNote}
                    </span>
                  )}
                </div>
                <div className="flex items-center gap-3 text-xs text-slate-300">
                  <span className="text-slate-400">{t.importFiles.conflictLabel}</span>
                  {(
                    [
                      ["skip", t.importFiles.conflictSkip],
                      ["overwrite", t.importFiles.conflictOverwrite],
                      ["rename", t.importFiles.conflictRename],
                    ] as [GameImportConflict, string][]
                  ).map(([key, label]) => (
                    <label key={key} className="flex cursor-pointer items-center gap-1.5">
                      <input
                        type="radio"
                        name="import-conflict"
                        checked={conflict === key}
                        disabled={phase === "executing"}
                        onChange={() => setConflict(key)}
                        className="accent-indigo-500"
                      />
                      {label}
                    </label>
                  ))}
                </div>
              </div>
            </div>
          )}

          {/* 固定：结果提示与打开目录（结果态） */}
          {phase === "done" && exec && (hasDoneHints || openDirs.length > 0) && (
            <div className="flex-none border-t border-slate-800/70 px-5 py-3">
              {/* 结果提示：模组保存 / HD 滤镜 / 配置重启 / 历代记模组已生成 */}
              <div className="space-y-1.5 text-[11px]">
                {exec.modsInstalled.length > 0 && (
                  <div className="text-amber-300/90">{t.importFiles.modsSavedHint}</div>
                )}
                {hasHdDone && <div className="text-fuchsia-300/90">{t.importFiles.hdFilterHint}</div>}
                {hasConfigDone && <div className="text-slate-400">{t.importFiles.configRestartHint}</div>}
                {hasChronicles && (
                  <div className="text-slate-400">{t.importFiles.chroniclesHint}</div>
                )}
              </div>
              {openDirs.length > 0 && (
                <div className="mt-2.5 flex flex-wrap items-center gap-1.5">
                  {/* 目标目录 chips：引导标签承担动词语义，chip 只放目录名（完整路径走 title） */}
                  <span className="text-[11px] text-slate-500">{t.importFiles.openDirLabel}</span>
                  {openDirs.map((d) => (
                    <button
                      key={d}
                      type="button"
                      title={d}
                      onClick={() => void openInExplorer(d)}
                      className="flex items-center gap-1 rounded-md border border-slate-700/70 px-2 py-0.5 text-[11px] text-slate-300 transition-colors hover:border-slate-600 hover:text-slate-100"
                    >
                      <FolderOpen className="h-3 w-3" />
                      {baseName(d)}
                    </button>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>

        {/* 页脚：警示 + 操作按钮 */}
        <div className="flex items-center justify-between gap-3 border-t border-slate-800/80 bg-slate-950/40 px-5 py-3">
          <div className="min-w-0 flex-1 space-y-0.5 text-[11px] leading-relaxed">
            {activeName && (
              <div className="text-amber-300/90">{t.importFiles.realmWarn.replace("{realm}", activeName)}</div>
            )}
            {vcmiRunning && <div className="text-amber-300/90">{t.importFiles.vcmiRunningWarn}</div>}
            {error && <div className="break-words text-red-300">{error}</div>}
          </div>
          <div className="flex flex-none items-center gap-2">
            {phase === "select" && (
              <>
                <Button size="sm" variant="outline" onClick={onClose}>
                  {t.importFiles.cancel}
                </Button>
                <Button
                  size="sm"
                  variant="primary"
                  disabled={paths.length === 0 || scanning}
                  onClick={() => void handleStart()}
                >
                  {t.importFiles.start}
                </Button>
              </>
            )}
            {phase === "executing" && (
              <>
                <Button size="sm" variant="outline" disabled>
                  {t.importFiles.cancel}
                </Button>
                <Button size="sm" variant="primary" disabled>
                  <Loader2 className="h-3.5 w-3.5 animate-spin" />
                  {t.importFiles.executing}
                </Button>
              </>
            )}
            {phase === "done" && (
              <Button size="sm" variant="primary" onClick={onClose}>
                {t.importFiles.close}
              </Button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
