/**
 * 模组管理页（P5 建立，P6 全面升级）：与「预设编排」解耦的模组库维护视图。
 *
 * 设计（docs/features/mods-repository.zh-CN.md/§6.6，T3/T4/T5 + P6 定案）：
 * - 预设页零仓库信息（P6/Q1）：安装/更新/角标全部只存在于本页；
 * - 状态筛选（P6）：全部/已安装/可下载/可更新——默认选中随「自动检查开启或本轮已检查」切换；
 * - 行级队列（P6/Q6）：可连点多个「安装/更新」入队，父级单 worker 串行执行，失败不中断；
 * - 操作列（P6/Q7）：打开目录（open_in_explorer）、删除（物理删除，输入目录名二次确认，
 *   游戏目录额外红字警示）；领域私有副本不在本表内（Q3 定案）；
 * - 就地配置（P6）：「自动检测更新」开关与「模组源」弹层直接读写 vcmiSettings（与官方互通）；
 * - 强制重新校验收进「…」溢出菜单（P6/Q4）。
 */
import { useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { useI18n } from "@/i18n";
import { cn } from "@/lib/utils";
import { localizedDescription, localizedName, type ModInfo } from "@/lib/modScanner";
import { getVcmiLanguageSnapshot, subscribeVcmiLanguage } from "@/lib/vcmiLanguage";
import { TYPE_ORDER } from "@/lib/minimapBase";
import {
  deleteMod,
  openInExplorer,
  writeVcmiSettings,
  type ModIndexProgress,
  type ModLedgerInfo,
  type ModRepoEntry,
} from "@/lib/tauri";
import { ModRowDrawer } from "./ModRowDrawer";
import { ModSourcePopover } from "./ModSourcePopover";

/** 队列项（父级 ModView 持有并串行执行；本组件只渲染行状态） */
export interface ModQueueItem {
  id: string;
  name: string;
  state: "queued" | "running" | "done" | "fail";
}

interface ModManagerPanelProps {
  /** 根模组（子模组随父整包更新，不单独管理） */
  mods: ModInfo[];
  /** 可更新集合（与检查更新结果同源） */
  repoUpdates: Record<string, { entry: ModRepoEntry; installedVersion: string }>;
  /** 全量仓库条目（null = 本轮尚未检查成功——默认落在「已安装」筛选） */
  repoMap: Map<string, ModRepoEntry> | null;
  /** 安装台账（key = 目录名小写）；无条目 = 随包/本地 */
  ledgers: Record<string, ModLedgerInfo>;
  checkingUpdates: boolean;
  /** P9/P10：两阶段检查进度（手动/静默均显示，底部悬浮胶囊不抖表格） */
  checkProgress: ModIndexProgress | null;
  autoCheckOn: boolean;
  queue: ModQueueItem[];
  userModsDir: string;
  gameModsDir: string;
  onBack: () => void;
  onCheckUpdates: () => void;
  onEnqueue: (id: string, name: string) => void;
  onDequeue: (id: string) => void;
  /** 「自动检测更新」开关写盘成功后回传（父级同步 autoCheckOn 态） */
  onToggleAutoCheck: (on: boolean) => void;
  onRevalidate: () => void;
  /** 删除动作结果（父级负责重扫 + 横幅） */
  onDeleted: (id: string, ok: boolean, name: string) => void;
}

type StatusFilter = "all" | "installed" | "downloadable" | "updatable";

/** 行模型：已安装（用户/游戏目录）或可下载（仓库未装条目）；导出供详情抽屉复用 */
export interface Row {
  id: string;
  name: string;
  type: string;
  /** 描述（参与搜索；可下载行由后端 enrich 从 mod.json 回填） */
  desc: string;
  /** 作者（已装行来自扫描；可下载行抽屉取 entry.author） */
  author: string;
  installedVersion: string;
  latestVersion: string;
  status: "installed" | "downloadable" | "updatable";
  modPath: string;
  dirName: string;
  location: "user" | "game" | "realm" | "unknown";
  ledger?: ModLedgerInfo;
}

const normPath = (p: string) => p.replace(/\\/g, "/").toLowerCase().replace(/\/+$/, "");

/** 台账出口展示：mirror:<域名> 只留域名；direct/cache 等原样小标签。 */
function viaLabel(via: string): string {
  if (via.startsWith("mirror:")) return via.slice("mirror:".length);
  return via;
}

export function ModManagerPanel({
  mods,
  repoUpdates,
  repoMap,
  ledgers,
  checkingUpdates,
  checkProgress,
  autoCheckOn,
  queue,
  userModsDir,
  gameModsDir,
  onBack,
  onCheckUpdates,
  onEnqueue,
  onDequeue,
  onToggleAutoCheck,
  onRevalidate,
  onDeleted,
}: ModManagerPanelProps) {
  const { t } = useI18n();
  const m = t.modsManage;
  const vcmiLang = useSyncExternalStore(subscribeVcmiLanguage, getVcmiLanguageSnapshot) ?? "english";
  const [query, setQuery] = useState("");
  // 默认筛选（P6 定案）：自动检查开启、或本轮已手工检查过（repoMap 非 null）→「全部」；否则「已安装」
  const [status, setStatus] = useState<StatusFilter>(() =>
    autoCheckOn || repoMap !== null ? "all" : "installed",
  );
  const [typeFilter, setTypeFilter] = useState("all");
  const [moreOpen, setMoreOpen] = useState(false);
  const [srcOpen, setSrcOpen] = useState(false);
  const srcWrapRef = useRef<HTMLDivElement>(null);
  const moreWrapRef = useRef<HTMLDivElement>(null);
  const [autoBusy, setAutoBusy] = useState(false);
  const [del, setDel] = useState<{ id: string; dirName: string; path: string; isGame: boolean } | null>(null);
  const [delInput, setDelInput] = useState("");
  const [delBusy, setDelBusy] = useState(false);
  // P10：行点击统一开详情抽屉（已装/可下载同一入口；动作列 stopPropagation 不串发）
  const [detail, setDetail] = useState<Row | null>(null);

  const updateCount = Object.keys(repoUpdates).length;
  const queueOf = (id: string) => queue.find((x) => x.id === id);

  // 弹层失焦自动关闭（P7）：点击弹层与触发按钮以外的任意处收起
  useEffect(() => {
    if (!srcOpen) return;
    const onDown = (e: MouseEvent) => {
      if (srcWrapRef.current && !srcWrapRef.current.contains(e.target as Node)) setSrcOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [srcOpen]);
  useEffect(() => {
    if (!moreOpen) return;
    const onDown = (e: MouseEvent) => {
      if (moreWrapRef.current && !moreWrapRef.current.contains(e.target as Node)) setMoreOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    return () => document.removeEventListener("mousedown", onDown);
  }, [moreOpen]);

  /* ---------------- 行数据：已安装 ∪ 可下载 ---------------- */
  const rows = useMemo<Row[]>(() => {
    const installedIds = new Set(mods.map((x) => x.id.toLowerCase()));
    const userRoot = normPath(userModsDir);
    const gameRoot = normPath(gameModsDir);
    const out: Row[] = mods.map((x) => {
      const up = repoUpdates[x.id];
      const p = normPath(x.modPath);
      return {
        id: x.id,
        name: localizedName(x, vcmiLang) || x.name,
        type: x.type,
        // P10.1：描述也按启动器语言选词（mod.json i18n 块，与名称同口径）
        desc: localizedDescription(x, vcmiLang) || x.desc || "",
        author: x.author ?? "",
        installedVersion: x.version ?? "",
        latestVersion: up ? up.entry.version : (repoMap?.get(x.id.toLowerCase())?.version ?? ""),
        status: up ? "updatable" : "installed",
        modPath: x.modPath,
        dirName: x.diskName || x.id,
        location: userRoot && p.startsWith(userRoot) ? "user" : gameRoot && p.startsWith(gameRoot) ? "game" : "unknown",
        ledger: ledgers[x.id.toLowerCase()],
      };
    });
    if (status !== "installed" && repoMap) {
      for (const [key, e] of repoMap) {
        if (installedIds.has(key)) continue;
        out.push({
          id: e.id || key,
          // P10.1：可下载行同样按启动器语言选词（enrich 回填的 mod.json i18n 块）
          name:
            localizedName({ name: e.name || key, i18n: e.i18n }, vcmiLang) || e.name || key,
          // enrich 从 mod.json 回填（与已安装扫描同走 classify_mod）；回填前为空，列显示 —
          type: e.modType ?? "",
          desc: localizedDescription({ desc: e.description ?? "", i18n: e.i18n }, vcmiLang),
          author: "",
          installedVersion: "",
          latestVersion: e.version ?? "",
          status: "downloadable",
          modPath: "",
          dirName: "",
          location: "unknown",
        });
      }
    }
    return out;
  }, [mods, repoUpdates, repoMap, ledgers, vcmiLang, userModsDir, gameModsDir, status]);

  /** 分类 chips：出现过的类型按 TYPE_ORDER 固定序 */
  const types = useMemo(() => {
    const present = new Set(rows.filter((r) => r.type).map((r) => r.type));
    return TYPE_ORDER.filter((x) => present.has(x));
  }, [rows]);

  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    return rows
      .filter((r) => {
        if (status === "installed" && r.status === "downloadable") return false;
        if (status === "downloadable" && r.status !== "downloadable") return false;
        if (status === "updatable" && r.status !== "updatable") return false;
        if (typeFilter !== "all" && r.type !== typeFilter) return false;
        if (q && !`${r.id} ${r.name} ${r.dirName} ${r.desc}`.toLowerCase().includes(q)) return false;
        return true;
      })
      .sort(
        (a, b) =>
          (a.status === "updatable" ? 0 : a.status === "installed" ? 1 : 2) -
            (b.status === "updatable" ? 0 : b.status === "installed" ? 1 : 2) ||
          a.id.localeCompare(b.id),
      );
  }, [rows, status, typeFilter, query]);

  /* ---------------- 动作 ---------------- */
  const toggleAuto = async (on: boolean) => {
    setAutoBusy(true);
    try {
      await writeVcmiSettings({ launcher: { autoCheckRepositories: on } });
      onToggleAutoCheck(on);
    } catch {
      /* 写失败不动开关（下次读取仍是磁盘真值） */
    } finally {
      setAutoBusy(false);
    }
  };

  const openSourceEditor = () => {
    setSrcOpen((o) => !o);
  };

  const confirmDelete = async () => {
    if (!del) return;
    setDelBusy(true);
    const ok = await deleteMod(del.path).then(() => true).catch(() => false);
    setDelBusy(false);
    setDel(null);
    setDelInput("");
    onDeleted(del.id, ok, del.dirName);
  };

  const statusChip: [StatusFilter, string][] = [
    ["all", m.statusAll],
    ["installed", m.statusInstalled],
    ["downloadable", m.statusDownloadable],
    ["updatable", m.statusUpdatable],
  ];

  return (
    <div className="relative flex min-h-0 flex-1 flex-col">
      {/* 顶部操作栏（P6：批量更新弹窗已移除，更新走行级队列） */}
      <div className="flex items-center gap-3 px-4 py-3">
        <button
          onClick={onBack}
          className="rounded-lg border border-slate-700 px-3 py-1.5 text-sm text-slate-300 transition-colors hover:border-slate-600 hover:text-white"
        >
          ← {m.back}
        </button>
        <h2 className="text-lg font-semibold text-white">{m.title}</h2>
        <div className="ml-auto flex items-center gap-2">
          <label className="flex cursor-pointer select-none items-center gap-1.5 text-sm text-slate-300">
            <input
              type="checkbox"
              checked={autoCheckOn}
              disabled={autoBusy}
              onChange={(e) => void toggleAuto(e.target.checked)}
              className="h-3.5 w-3.5 accent-blue-500"
            />
            {m.autoCheckLabel}
          </label>
          <div ref={srcWrapRef} className="relative">
            <button
              onClick={() => void openSourceEditor()}
              className="rounded-lg border border-slate-600 px-3 py-1.5 text-sm text-slate-200 transition-colors hover:border-slate-500 hover:text-white"
            >
              {m.sourceTitle}
            </button>
            {/* 模组源弹层（ModSourcePopover，与领域管理页共用；点外部自动关闭） */}
            {srcOpen && <ModSourcePopover onClose={() => setSrcOpen(false)} />}
          </div>
          <button
            onClick={onCheckUpdates}
            disabled={checkingUpdates}
            className="relative rounded-lg border border-slate-600 px-3 py-1.5 text-sm text-slate-200 transition-colors hover:border-slate-500 hover:text-white disabled:opacity-50"
          >
            {checkingUpdates ? t.modsRepo.checking : t.modsRepo.check}
            {updateCount > 0 && !checkingUpdates && (
              <span
                title={m.badgeHint.replace("{n}", String(updateCount))}
                className="absolute -right-1.5 -top-1.5 rounded-full bg-amber-500/90 px-1.5 text-[10px] font-semibold leading-4 text-slate-950"
              >
                {updateCount}
              </span>
            )}
          </button>
          <div ref={moreWrapRef} className="relative">
            <button
              onClick={() => setMoreOpen((o) => !o)}
              className="rounded-lg border border-slate-600 px-2.5 py-1.5 text-sm text-slate-200 transition-colors hover:border-slate-500 hover:text-white"
            >
              …
            </button>
            {moreOpen && (
              <div className="absolute right-0 z-[60] mt-1 w-44 overflow-hidden rounded-lg border border-slate-700 bg-slate-900 shadow-xl">
                <button
                  className="w-full px-3 py-2 text-left text-sm text-slate-200 hover:bg-slate-800"
                  onClick={() => {
                    setMoreOpen(false);
                    onRevalidate();
                  }}
                >
                  {t.mods.revalidate}
                </button>
              </div>
            )}
          </div>
        </div>
      </div>

      {/* 状态筛选栏 + 搜索 */}
      <div className="flex flex-wrap items-center gap-1.5 px-4 pb-2 text-xs">
        {statusChip.map(([k, label]) => (
          <button
            key={k}
            onClick={() => setStatus(k)}
            className={cn(
              "rounded-full border px-3 py-1 transition-colors",
              status === k
                ? "border-blue-500/60 bg-blue-500/15 text-white"
                : "border-slate-700/50 text-slate-400 hover:text-white",
            )}
          >
            {label}
            {k === "updatable" && updateCount > 0 && (
              <span className="ml-1.5 rounded-full bg-amber-500/90 px-1.5 font-semibold text-black">{updateCount}</span>
            )}
          </button>
        ))}
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={m.search}
          className="ml-auto w-72 rounded-lg border border-slate-700 bg-slate-900/60 px-3 py-1.5 text-sm text-white placeholder:text-slate-500 focus:border-blue-500 focus:outline-none"
        />
      </div>

      {/* 分类筛选栏（复用预设页 TYPE_ORDER 固定排序） */}
      {types.length > 0 && (
        <div className="flex flex-wrap items-center gap-1.5 px-4 pb-2 text-xs">
          <button
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
          <span className="ml-auto text-slate-500">{m.summary.replace("{n}", String(visible.length))}</span>
        </div>
      )}

      {/* 列表 */}
      <div className="min-h-0 flex-1 overflow-y-auto px-4 pb-4">
        <table className="w-full table-fixed border-separate border-spacing-0 text-sm">
          <colgroup>
            <col className="w-[26%]" />
            <col className="w-[8%]" />
            <col className="w-[10%]" />
            <col className="w-[10%]" />
            <col className="w-[9%]" />
            <col className="w-[9%]" />
            <col className="w-[12%]" />
            <col className="w-[16%]" />
          </colgroup>
          <thead className="sticky top-0 z-10">
            <tr className="bg-slate-900 text-left text-xs text-slate-500">
              <th className="border-b border-slate-800 px-2 py-2 font-normal">{m.colMod}</th>
              <th className="border-b border-slate-800 px-2 py-2 font-normal">{m.colType}</th>
              <th className="border-b border-slate-800 px-2 py-2 font-normal">{t.modsRepo.installedVer}</th>
              <th className="border-b border-slate-800 px-2 py-2 font-normal">{m.colUpgradable}</th>
              <th className="border-b border-slate-800 px-2 py-2 font-normal">{m.colStatus}</th>
              <th className="border-b border-slate-800 px-2 py-2 font-normal">{m.colLocation}</th>
              <th className="border-b border-slate-800 px-2 py-2 font-normal">{m.colDirName}</th>
              <th className="border-b border-slate-800 px-2 py-2 font-normal">{m.colAction}</th>
            </tr>
          </thead>
          <tbody>
            {visible.map((r) => {
              const q = queueOf(r.id);
              return (
                <tr
                  key={`${r.status}:${r.id}`}
                  onClick={() => setDetail(r)}
                  className="group cursor-pointer text-slate-300 hover:bg-slate-800/40"
                >
                  <td className="border-b border-slate-800/60 px-2 py-2">
                    <div className="flex min-w-0 items-center gap-2">
                      {/* P10.3/A：名称链接化——hover 整行时变主题蓝+下划线，持久暗示「可点开详情」 */}
                      <span className="truncate text-white decoration-sky-400/60 transition-colors group-hover:text-sky-300 group-hover:underline">{r.name}</span>
                      {r.ledger && (
                        <span
                          className={cn(
                            "shrink-0 rounded border px-1.5 py-0.5 text-[10px]",
                            r.ledger.check === "strong"
                              ? "border-emerald-500/50 bg-emerald-500/10 text-emerald-300"
                              : "border-amber-500/50 bg-amber-500/10 text-amber-300",
                          )}
                          title={`${m.sourceRepo}${r.ledger.via && r.ledger.via !== "direct" ? ` · ${viaLabel(r.ledger.via)}` : ""}`}
                        >
                          {r.ledger.check === "strong" ? t.modsRepo.verified : t.modsRepo.weak}
                        </span>
                      )}
                    </div>
                  </td>
                  <td className="border-b border-slate-800/60 px-2 py-2 text-xs text-slate-400">
                    {r.type ? (t.mods.typeLabels[r.type] ?? r.type) : "—"}
                  </td>
                  <td className="border-b border-slate-800/60 px-2 py-2 tabular-nums">{r.installedVersion || "—"}</td>
                  <td className="border-b border-slate-800/60 px-2 py-2 tabular-nums">
                    {r.status === "updatable" ? (
                      <span className="font-semibold text-amber-300">{r.latestVersion || "—"}</span>
                    ) : (
                      r.latestVersion || "—"
                    )}
                  </td>
                  <td className="border-b border-slate-800/60 px-2 py-2 text-xs">
                    {r.status === "updatable" ? (
                      <span className="text-amber-300">{m.stateUpdatable}</span>
                    ) : r.status === "downloadable" ? (
                      <span className="text-sky-300">{m.stateDownloadable}</span>
                    ) : (
                      <span className="text-slate-400">{m.stateInstalled}</span>
                    )}
                  </td>
                  <td className="border-b border-slate-800/60 px-2 py-2 text-xs text-slate-400">
                    {r.location === "user"
                      ? m.locationUser
                      : r.location === "game"
                        ? m.locationGame
                        : r.location === "realm"
                          ? m.locationRealm
                          : "—"}
                  </td>
                  <td className="truncate border-b border-slate-800/60 px-2 py-2 text-xs text-slate-500">{r.dirName || "—"}</td>
                  <td className="border-b border-slate-800/60 px-2 py-2">
                    {/* 动作列不串发行点击抽屉（P10） */}
                    <div
                      className="flex flex-wrap items-center gap-1.5 text-xs"
                      onClick={(e) => e.stopPropagation()}
                    >
                      {r.status === "downloadable" && (
                        <button
                          onClick={() => onEnqueue(r.id, r.name)}
                          disabled={q?.state === "queued" || q?.state === "running"}
                          className="rounded border border-sky-500/60 bg-sky-500/10 px-2 py-1 text-sky-200 hover:bg-sky-500/20 disabled:opacity-50"
                        >
                          {q ? queueLabel(q, m) : m.actInstall}
                        </button>
                      )}
                      {r.status === "updatable" && (
                        <button
                          onClick={() => onEnqueue(r.id, r.name)}
                          disabled={q?.state === "queued" || q?.state === "running"}
                          className="rounded border border-amber-500/60 bg-amber-500/10 px-2 py-1 text-amber-200 hover:bg-amber-500/20 disabled:opacity-50"
                        >
                          {q ? queueLabel(q, m) : m.actUpdate}
                        </button>
                      )}
                      {q?.state === "queued" && (
                        <button
                          onClick={() => onDequeue(r.id)}
                          className="rounded border border-slate-600 px-2 py-1 text-slate-400 hover:text-white"
                        >
                          {m.actCancel}
                        </button>
                      )}
                      {r.status !== "downloadable" && (
                        <>
                          <button
                            onClick={() => void openInExplorer(r.modPath)}
                            className="rounded border border-slate-600 px-2 py-1 text-slate-300 hover:text-white"
                          >
                            {m.actOpen}
                          </button>
                          <button
                            onClick={() => {
                              setDel({ id: r.id, dirName: r.dirName, path: r.modPath, isGame: r.location === "game" });
                              setDelInput("");
                            }}
                            className="rounded border border-red-500/50 px-2 py-1 text-red-300 hover:bg-red-500/10"
                          >
                            {m.actDelete}
                          </button>
                        </>
                      )}
                    </div>
                  </td>
                </tr>
              );
            })}
            {visible.length === 0 && (
              <tr>
                <td colSpan={8} className="px-2 py-10 text-center text-slate-500">
                  {m.empty}
                </td>
              </tr>
            )}
          </tbody>
        </table>
      </div>

      {/* 检查进度悬浮胶囊（P10-1）：固定悬浮在表格底部，手动/静默均显示；
          不占布局空间——避免进度条出现/消失引起表格整体位移抖动 */}
      {checkingUpdates && checkProgress && (
        <div className="pointer-events-none absolute bottom-5 left-1/2 z-40 flex w-[min(560px,86%)] -translate-x-1/2 items-center gap-3 rounded-full border border-slate-700 bg-slate-900/95 px-4 py-2.5 shadow-2xl backdrop-blur">
          <span className="shrink-0 text-xs text-slate-300">
            {checkProgress.phase === "index" ? m.progressIndex : m.progressMeta}
          </span>
          <div className="h-1.5 flex-1 overflow-hidden rounded-full bg-slate-800">
            <div
              className="h-full rounded-full bg-sky-500 transition-all duration-200"
              style={{
                width: `${checkProgress.total > 0 ? Math.min(100, Math.round((checkProgress.done / checkProgress.total) * 100)) : 0}%`,
              }}
            />
          </div>
          <span className="shrink-0 text-xs tabular-nums text-slate-500">
            {checkProgress.done}/{checkProgress.total}
          </span>
        </div>
      )}

      {/* 行详情抽屉（P10-2）：已装/可下载统一入口，点蒙层关闭 */}
      <ModRowDrawer
        row={detail}
        entry={detail ? repoMap?.get(detail.id.toLowerCase()) ?? null : null}
        queueItem={detail ? queueOf(detail.id) : undefined}
        onClose={() => setDetail(null)}
      />

      {/* 删除二次确认（P6/Q7：输入目录名匹配才能确认；游戏目录额外警示） */}
      {del && (
        <div className="fixed inset-0 z-[150] flex items-center justify-center bg-black/60" onClick={() => setDel(null)}>
          <div className="w-[26rem] rounded-xl border border-slate-700 bg-slate-900 p-4 shadow-2xl" onClick={(e) => e.stopPropagation()}>
            <h3 className="mb-2 text-sm font-semibold text-white">{m.delTitle}</h3>
            {del.isGame && <div className="mb-2 rounded-lg border border-red-500/50 bg-red-500/10 p-2 text-xs text-red-300">{m.delGameWarn}</div>}
            <p className="mb-2 text-xs leading-5 text-slate-400">{m.delHint.replace("{name}", del.dirName)}</p>
            <input
              value={delInput}
              onChange={(e) => setDelInput(e.target.value)}
              placeholder={del.dirName}
              className="mb-3 w-full rounded-lg border border-slate-700 bg-slate-950/60 px-2 py-1.5 text-sm text-white placeholder:text-slate-600 focus:border-red-500 focus:outline-none"
            />
            <div className="flex justify-end gap-2">
              <button
                onClick={() => {
                  setDel(null);
                  setDelInput("");
                }}
                className="rounded-lg border border-slate-600 px-3 py-1.5 text-sm text-slate-300 hover:text-white"
              >
                {m.actCancel}
              </button>
              <button
                onClick={() => void confirmDelete()}
                disabled={delInput !== del.dirName || delBusy}
                className="rounded-lg bg-red-600 px-3 py-1.5 text-sm font-semibold text-white hover:bg-red-500 disabled:opacity-50"
              >
                {delBusy ? t.modsRepo.checking : m.delConfirm}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

/** 队列态按钮文案 */
function queueLabel(q: ModQueueItem, m: { qQueued: string; qRunning: string; qDone: string; qFail: string }): string {
  if (q.state === "queued") return m.qQueued;
  if (q.state === "running") return m.qRunning;
  if (q.state === "done") return m.qDone;
  return m.qFail;
}

