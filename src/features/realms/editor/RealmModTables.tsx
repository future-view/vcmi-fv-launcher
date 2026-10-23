/**
 * 领域模组管理面板的**四张页签表格**（已实装 / 可引用 / 可下载 / 可更新）
 * 与下载队列的行级小工具。
 *
 * 从 `RealmModManagerPanel.tsx` 抽出：那一块是「给定行数据 + 回调 → 表格 DOM」的纯展示，
 * 不含任何 import 期副作用与状态；抽成独立模块后主组件只关心数据装配与动作编排。
 *
 * ⚠️ 约定：各表格的 props **刻意沿用主组件里的同名局部变量**（`typedInstalled` / `queueOf`
 * / `dlMap` / `progressLabel` …），使 JSX 主体搬过来**一字不改** —— 拆分只改"变量从哪来"，
 * 不改"怎么渲"。新增列时仍按同一口径补 prop，不要把主组件闭包再塞回来。
 */
import type { Dispatch, SetStateAction } from "react";
import { AlertTriangle } from "lucide-react";
import type { Row } from "@/features/mods/ModManagerPanel";
import { localizedName, type ModInfo } from "@/lib/modScanner";
import type { DepSolveResult } from "@/lib/modDeps";
import type {
  InstalledMod,
  ModRepoEntry,
  RealmInstalledMod,
} from "@/lib/tauri";
import type { LangDict } from "@/i18n/types";

/* ---------------- 行模型 / 队列（主组件与表格共用） ---------------- */

/** 可更新行模型 */
export interface UpdRow {
  mod: RealmInstalledMod;
  entry: ModRepoEntry;
  cur: string;
  next: string;
}

/** 下载/更新队列项（主组件内部 worker 串行执行；失败不中断后续） */
export interface QItem {
  id: string;
  name: string;
  state: "queued" | "running" | "done" | "fail";
  /** 任务类型：install = 可下载页安装；update = 可更新行升级（引用态先取消引用再装副本） */
  kind: "install" | "update";
  /** update 任务上下文（kind === "update" 时必填） */
  upd?: UpdRow;
}

/** 行级下载进度（key = mod id 小写） */
export type DlMap = Record<string, { received: number; total: number; done: boolean }>;

/** 取消引用确认框的状态 */
export interface UnrefDlgState {
  mod: RealmInstalledMod;
  deps: RealmInstalledMod[];
  cascade: boolean;
}

/** 删除确认框的状态 */
export interface DelDlgState {
  mod: RealmInstalledMod;
  kind: "bundled" | "pinned" | "local";
  deps: RealmInstalledMod[];
}

export function queueLabel(q: QItem, m: LangDict["modsManage"]): string {
  if (q.state === "queued") return m.qQueued;
  if (q.state === "running") return m.qRunning;
  if (q.state === "done") return m.qDone;
  return m.qFail;
}

/** 下载进度百分比（total 未知时返回 0）。 */
export const pctOf = (dl: { received: number; total: number }) =>
  dl.total > 0 ? Math.min(100, Math.floor((dl.received / dl.total) * 100)) : 0;

/** 「可下载 / 可更新」尚未检查仓库时的缺省说明：解释空页原因 + 内联「检查更新」入口。 */
export function RepoNotChecked({
  t,
  m,
  busy,
  onCheck,
}: {
  t: LangDict;
  m: LangDict["realm"]["modsManage"];
  busy: boolean;
  onCheck: () => void;
}) {
  return (
    <div className="mx-auto flex max-w-md flex-col items-center gap-2">
      <div className="text-sm text-slate-300">{m.repoEmptyTitle}</div>
      <div className="text-xs leading-5 text-slate-500">{m.repoEmptyHint}</div>
      <button
        type="button"
        onClick={onCheck}
        disabled={busy}
        className="rounded border border-slate-600 px-3 py-1 text-xs text-slate-200 hover:border-slate-500 hover:text-white disabled:opacity-40"
      >
        {busy ? t.modsRepo.checking : t.modsRepo.check}
      </button>
    </div>
  );
}

/* ---------------- 已实装 ---------------- */

export function InstalledTable({
  t,
  m,
  mm,
  typedInstalled,
  depSolve,
  rowOfInstalled,
  repoMap,
  setDetail,
  shownNameOfRealm,
  opBusy,
  queueBusy,
  dependentsOf,
  setDepStates,
  setDepDlg,
  setUnrefDlg,
  setDelDlg,
}: {
  t: LangDict;
  m: LangDict["realm"]["modsManage"];
  mm: LangDict["modsManage"];
  typedInstalled: RealmInstalledMod[];
  depSolve: DepSolveResult;
  rowOfInstalled: (x: RealmInstalledMod) => Row;
  repoMap: Map<string, ModRepoEntry> | null;
  setDetail: (v: { row: Row; entry: ModRepoEntry | null } | null) => void;
  shownNameOfRealm: (x: RealmInstalledMod) => string;
  opBusy: boolean;
  queueBusy: boolean;
  dependentsOf: (id: string) => RealmInstalledMod[];
  setDepStates: Dispatch<SetStateAction<Record<string, "busy" | "done" | "fail">>>;
  setDepDlg: (v: string | null) => void;
  setUnrefDlg: (v: UnrefDlgState | null) => void;
  setDelDlg: (v: DelDlgState | null) => void;
}) {
  return (
    <table className="w-full table-fixed border-separate border-spacing-0 text-sm">
      <colgroup>
        <col className="w-[28%]" />
        <col className="w-[14%]" />
        <col className="w-[8%]" />
        <col className="w-[9%]" />
        <col className="w-[12%]" />
        <col className="w-[29%]" />
      </colgroup>
      <thead className="sticky top-0 z-10">
        <tr className="bg-slate-900 text-left text-xs text-slate-500">
          <th className="border-b border-slate-800 px-3 py-2 font-normal">{mm.colMod}</th>
          <th className="border-b border-slate-800 px-2 py-2 font-normal">{mm.colDirName}</th>
          <th className="border-b border-slate-800 px-2 py-2 font-normal">{mm.colType}</th>
          <th className="border-b border-slate-800 px-2 py-2 font-normal">{m.colVersion}</th>
          <th className="border-b border-slate-800 px-2 py-2 font-normal">{mm.colLocation}</th>
          <th className="border-b border-slate-800 px-2 py-2 font-normal">{mm.colAction}</th>
        </tr>
      </thead>
      <tbody>
        {typedInstalled.map((x) => {
          const isRealm = x.source === "realm";
          const canUnref = x.inBorrowed;
          const depInfo = depSolve.byRoot.get(x.id.toLowerCase());
          return (
            <tr
              key={`${x.source}:${x.id}`}
              onClick={() => setDetail({ row: rowOfInstalled(x), entry: repoMap?.get(x.id.toLowerCase()) ?? null })}
              className="group cursor-pointer text-slate-300 hover:bg-slate-800/40"
            >
              <td className="border-b border-slate-800/60 px-3 py-2">
                <div className="flex min-w-0 items-center gap-2">
                  <span className="truncate text-white decoration-sky-400/60 transition-colors group-hover:text-sky-300 group-hover:underline">
                    {shownNameOfRealm(x)}
                  </span>
                  {!x.metaOk && (
                    <span className="shrink-0 rounded border border-red-500/50 bg-red-500/10 px-1.5 py-0.5 text-[10px] text-red-300">
                      {t.mods.missing}
                    </span>
                  )}
                </div>
              </td>
              <td className="truncate border-b border-slate-800/60 px-2 py-2 text-xs text-slate-500" title={x.diskName || x.id}>
                {x.diskName || x.id}
              </td>
              <td className="border-b border-slate-800/60 px-2 py-2 text-xs text-slate-400">
                {x.type ? (t.mods.typeLabels[x.type] ?? x.type) : "—"}
              </td>
              <td className="border-b border-slate-800/60 px-2 py-2 tabular-nums">{x.version || "—"}</td>
              <td className="border-b border-slate-800/60 px-2 py-2 text-xs text-slate-400">
                {isRealm ? mm.locationRealm : x.source === "user" ? mm.locationUser : mm.locationGame}
              </td>
              <td className="border-b border-slate-800/60 px-2 py-2">
                <div className="flex flex-wrap items-center gap-1.5 text-xs" onClick={(e) => e.stopPropagation()}>
                  {/* 依赖处置（依赖处置体系）：仅缺陷行显示；N = 缺失依赖项数 */}
                  {depInfo && (
                    <button
                      type="button"
                      disabled={opBusy || queueBusy}
                      onClick={() => {
                        setDepStates({});
                        setDepDlg(x.id.toLowerCase());
                      }}
                      className="flex items-center gap-1 rounded border border-amber-500/60 bg-amber-500/10 px-2 py-1 text-amber-200 hover:bg-amber-500/20 disabled:opacity-50"
                    >
                      <AlertTriangle className="h-3 w-3" />
                      {m.depRowBtn}
                      <span className="rounded bg-amber-500/30 px-1 text-[10px]">{depInfo.total}</span>
                    </button>
                  )}
                  {canUnref && (
                    <button
                      type="button"
                      disabled={opBusy || queueBusy}
                      onClick={() =>
                        setUnrefDlg({ mod: x, deps: dependentsOf(x.id), cascade: dependentsOf(x.id).length > 0 })
                      }
                      className="rounded border border-slate-600 px-2 py-1 text-slate-300 hover:text-white disabled:opacity-50"
                    >
                      {m.actUnref}
                    </button>
                  )}
                  {isRealm && (
                    <button
                      type="button"
                      disabled={opBusy || queueBusy}
                      onClick={() => {
                        const kind = x.inPinned ? "pinned" : x.inBundled ? "bundled" : "local";
                        setDelDlg({ mod: x, kind, deps: dependentsOf(x.id) });
                      }}
                      className="rounded border border-red-500/50 px-2 py-1 text-red-300 hover:bg-red-500/10 disabled:opacity-50"
                    >
                      {m.actDelete}
                    </button>
                  )}
                </div>
              </td>
            </tr>
          );
        })}
        {typedInstalled.length === 0 && (
          <tr>
            <td colSpan={6} className="px-2 py-10 text-center text-slate-500">
              {mm.empty}
            </td>
          </tr>
        )}
      </tbody>
    </table>
  );
}

/* ---------------- 可引用 ---------------- */

export function ReferableTable({
  t,
  m,
  mm,
  typedReferable,
  rowOfReferable,
  repoMap,
  setDetail,
  miOfInstalled,
  vcmiLang,
  opBusy,
  queueBusy,
  doRef,
  doBundle,
}: {
  t: LangDict;
  m: LangDict["realm"]["modsManage"];
  mm: LangDict["modsManage"];
  typedReferable: InstalledMod[];
  rowOfReferable: (x: InstalledMod) => Row;
  repoMap: Map<string, ModRepoEntry> | null;
  setDetail: (v: { row: Row; entry: ModRepoEntry | null } | null) => void;
  miOfInstalled: (x: InstalledMod) => ModInfo;
  vcmiLang: string;
  opBusy: boolean;
  queueBusy: boolean;
  doRef: (ids: string[]) => unknown;
  doBundle: (mod: InstalledMod, force: boolean) => unknown;
}) {
  return (
    <table className="w-full table-fixed border-separate border-spacing-0 text-sm">
      <colgroup>
        <col className="w-[30%]" />
        <col className="w-[15%]" />
        <col className="w-[8%]" />
        <col className="w-[10%]" />
        <col className="w-[13%]" />
        <col className="w-[24%]" />
      </colgroup>
      <thead className="sticky top-0 z-10">
        <tr className="bg-slate-900 text-left text-xs text-slate-500">
          <th className="border-b border-slate-800 px-3 py-2 font-normal">{mm.colMod}</th>
          <th className="border-b border-slate-800 px-2 py-2 font-normal">{mm.colDirName}</th>
          <th className="border-b border-slate-800 px-2 py-2 font-normal">{mm.colType}</th>
          <th className="border-b border-slate-800 px-2 py-2 font-normal">{m.colVersion}</th>
          <th className="border-b border-slate-800 px-2 py-2 font-normal">{mm.colLocation}</th>
          <th className="border-b border-slate-800 px-2 py-2 font-normal">{mm.colAction}</th>
        </tr>
      </thead>
      <tbody>
        {typedReferable.map((x) => (
          <tr
            key={x.id}
            onClick={() => setDetail({ row: rowOfReferable(x), entry: repoMap?.get(x.id.toLowerCase()) ?? null })}
            className="group cursor-pointer text-slate-300 hover:bg-slate-800/40"
          >
            <td className="border-b border-slate-800/60 px-3 py-2">
              <span className="truncate text-white decoration-sky-400/60 group-hover:text-sky-300 group-hover:underline">
                {localizedName(miOfInstalled(x), vcmiLang) || x.name || x.id}
              </span>
            </td>
            <td className="truncate border-b border-slate-800/60 px-2 py-2 text-xs text-slate-500" title={x.id}>
              {x.id}
            </td>
            <td className="border-b border-slate-800/60 px-2 py-2 text-xs text-slate-400">
              {x.type ? (t.mods.typeLabels[x.type] ?? x.type) : "—"}
            </td>
            <td className="border-b border-slate-800/60 px-2 py-2 tabular-nums">{x.version || x.meta?.version || "—"}</td>
            <td className="border-b border-slate-800/60 px-2 py-2 text-xs text-slate-400">
              {x.source === "game" ? mm.locationGame : mm.locationUser}
            </td>
            <td className="border-b border-slate-800/60 px-2 py-2">
              <div className="flex flex-wrap items-center gap-1.5 text-xs" onClick={(e) => e.stopPropagation()}>
                <button
                  type="button"
                  disabled={opBusy || queueBusy}
                  onClick={() => void doRef([x.id])}
                  className="rounded border border-indigo-500/60 bg-indigo-500/10 px-2 py-1 text-indigo-200 hover:bg-indigo-500/20 disabled:opacity-50"
                >
                  {m.actRef}
                </button>
                <button
                  type="button"
                  disabled={opBusy || queueBusy}
                  onClick={() => void doBundle(x, false)}
                  className="rounded border border-fuchsia-500/60 bg-fuchsia-500/10 px-2 py-1 text-fuchsia-200 hover:bg-fuchsia-500/20 disabled:opacity-50"
                >
                  {m.actBundle}
                </button>
              </div>
            </td>
          </tr>
        ))}
        {typedReferable.length === 0 && (
          <tr>
            <td colSpan={6} className="px-2 py-10 text-center text-slate-500">
              {mm.empty}
            </td>
          </tr>
        )}
      </tbody>
    </table>
  );
}

/* ---------------- 可下载 ---------------- */

export function DownloadableTable({
  t,
  m,
  mm,
  typedDownloadable,
  queueOf,
  dlMap,
  rowOfDownloadable,
  setDetail,
  vcmiLang,
  enqueueFromUi,
  progressLabel,
  cancelQueued,
  repoMap,
  repoBusy,
  onRepoCheck,
}: {
  t: LangDict;
  m: LangDict["realm"]["modsManage"];
  mm: LangDict["modsManage"];
  typedDownloadable: { entry: ModRepoEntry; repair: boolean }[];
  queueOf: (id: string) => QItem | undefined;
  dlMap: DlMap;
  rowOfDownloadable: (e: ModRepoEntry) => Row;
  setDetail: (v: { row: Row; entry: ModRepoEntry | null } | null) => void;
  vcmiLang: string;
  enqueueFromUi: (entry: ModRepoEntry) => Promise<void>;
  progressLabel: (id: string, fallback: string) => string;
  cancelQueued: (id: string) => void;
  repoMap: Map<string, ModRepoEntry> | null;
  repoBusy: boolean;
  onRepoCheck: () => void;
}) {
  return (
    <table className="w-full table-fixed border-separate border-spacing-0 text-sm">
      <colgroup>
        <col className="w-[32%]" />
        <col className="w-[16%]" />
        <col className="w-[8%]" />
        <col className="w-[10%]" />
        <col className="w-[12%]" />
        <col className="w-[22%]" />
      </colgroup>
      <thead className="sticky top-0 z-10">
        <tr className="bg-slate-900 text-left text-xs text-slate-500">
          <th className="border-b border-slate-800 px-3 py-2 font-normal">{mm.colMod}</th>
          <th className="border-b border-slate-800 px-2 py-2 font-normal">{mm.colDirName}</th>
          <th className="border-b border-slate-800 px-2 py-2 font-normal">{mm.colType}</th>
          <th className="border-b border-slate-800 px-2 py-2 font-normal">{m.colCurVersion}</th>
          <th className="border-b border-slate-800 px-2 py-2 font-normal">{m.colSource}</th>
          <th className="border-b border-slate-800 px-2 py-2 font-normal">{mm.colAction}</th>
        </tr>
      </thead>
      <tbody>
        {typedDownloadable.map(({ entry, repair }) => {
          const qq = queueOf(entry.id);
          const dl = dlMap[entry.id.toLowerCase()];
          return (
            <tr
              key={entry.id}
              onClick={() => setDetail({ row: rowOfDownloadable(entry), entry })}
              className="group cursor-pointer text-slate-300 hover:bg-slate-800/40"
            >
              <td className="border-b border-slate-800/60 px-3 py-2">
                <div className="flex min-w-0 items-center gap-2">
                  <span className="truncate text-white decoration-sky-400/60 group-hover:text-sky-300 group-hover:underline">
                    {localizedName({ name: entry.name || entry.id, i18n: entry.i18n } as ModInfo, vcmiLang) || entry.name || entry.id}
                  </span>
                  {repair && (
                    <span className="shrink-0 rounded border border-red-500/50 bg-red-500/10 px-1.5 py-0.5 text-[10px] text-red-300">
                      {m.missingTag}
                    </span>
                  )}
                </div>
              </td>
              <td className="truncate border-b border-slate-800/60 px-2 py-2 text-xs text-slate-500" title={entry.id}>
                {entry.id}
              </td>
              <td className="border-b border-slate-800/60 px-2 py-2 text-xs text-slate-400">
                {entry.modType ? (t.mods.typeLabels[entry.modType] ?? entry.modType) : "—"}
              </td>
              <td className="border-b border-slate-800/60 px-2 py-2 tabular-nums">{entry.version || "—"}</td>
              <td className="border-b border-slate-800/60 px-2 py-2 text-xs text-slate-400">
                {entry.sourceKind === "extra" ? mm.sourceExtra : mm.sourceDefault}
              </td>
              <td className="border-b border-slate-800/60 px-2 py-2">
                <div className="flex flex-wrap items-center gap-1.5 text-xs" onClick={(e) => e.stopPropagation()}>
                  <button
                    type="button"
                    onClick={() => void enqueueFromUi(entry)}
                    disabled={qq?.state === "queued" || qq?.state === "running"}
                    className="rounded border border-sky-500/60 bg-sky-500/10 px-2 py-1 text-sky-200 hover:bg-sky-500/20 disabled:opacity-50"
                  >
                    {qq
                      ? qq.state === "running"
                        ? progressLabel(entry.id, mm.qRunning)
                        : queueLabel(qq, mm)
                      : mm.actInstall}
                  </button>
                  {(qq?.state === "queued" || (qq?.state === "running" && dl && !dl.done)) && (
                    <button
                      type="button"
                      onClick={() => cancelQueued(entry.id)}
                      className="rounded border border-slate-600 px-2 py-1 text-slate-400 hover:text-white"
                    >
                      {mm.actCancel}
                    </button>
                  )}
                </div>
                {qq?.state === "running" && dl && !dl.done && dl.total > 0 && (
                  <div className="mt-1 h-1 w-28 overflow-hidden rounded bg-slate-700">
                    <div
                      className="h-full bg-sky-400 transition-[width] duration-200"
                      style={{ width: `${pctOf(dl)}%` }}
                    />
                  </div>
                )}
              </td>
            </tr>
          );
        })}
        {typedDownloadable.length === 0 && (
          <tr>
            <td colSpan={6} className="px-2 py-10 text-center text-slate-500">
              {repoMap ? mm.empty : <RepoNotChecked t={t} m={m} busy={repoBusy} onCheck={onRepoCheck} />}
            </td>
          </tr>
        )}
      </tbody>
    </table>
  );
}

/* ---------------- 可更新 ---------------- */

export function UpdatableTable({
  t,
  m,
  mm,
  typedUpdatable,
  queueOf,
  dlMap,
  rowOfInstalled,
  setDetail,
  shownNameOfRealm,
  opBusy,
  setUpdDlg,
  progressLabel,
  dequeue,
  repoMap,
  repoBusy,
  onRepoCheck,
}: {
  t: LangDict;
  m: LangDict["realm"]["modsManage"];
  mm: LangDict["modsManage"];
  typedUpdatable: UpdRow[];
  queueOf: (id: string) => QItem | undefined;
  dlMap: DlMap;
  rowOfInstalled: (x: RealmInstalledMod) => Row;
  setDetail: (v: { row: Row; entry: ModRepoEntry | null } | null) => void;
  shownNameOfRealm: (x: RealmInstalledMod) => string;
  opBusy: boolean;
  setUpdDlg: (v: UpdRow | null) => void;
  progressLabel: (id: string, fallback: string) => string;
  dequeue: (id: string) => void;
  repoMap: Map<string, ModRepoEntry> | null;
  repoBusy: boolean;
  onRepoCheck: () => void;
}) {
  return (
    <table className="w-full table-fixed border-separate border-spacing-0 text-sm">
      <colgroup>
        <col className="w-[28%]" />
        <col className="w-[14%]" />
        <col className="w-[8%]" />
        <col className="w-[9%]" />
        <col className="w-[9%]" />
        <col className="w-[12%]" />
        <col className="w-[20%]" />
      </colgroup>
      <thead className="sticky top-0 z-10">
        <tr className="bg-slate-900 text-left text-xs text-slate-500">
          <th className="border-b border-slate-800 px-3 py-2 font-normal">{mm.colMod}</th>
          <th className="border-b border-slate-800 px-2 py-2 font-normal">{mm.colDirName}</th>
          <th className="border-b border-slate-800 px-2 py-2 font-normal">{mm.colType}</th>
          <th className="border-b border-slate-800 px-2 py-2 font-normal">{m.colCurVersion}</th>
          <th className="border-b border-slate-800 px-2 py-2 font-normal">{m.colLatest}</th>
          <th className="border-b border-slate-800 px-2 py-2 font-normal">{mm.colLocation}</th>
          <th className="border-b border-slate-800 px-2 py-2 font-normal">{mm.colAction}</th>
        </tr>
      </thead>
      <tbody>
        {typedUpdatable.map((row) => {
          const qq = queueOf(row.mod.id);
          const busyRow = qq?.state === "running";
          const dl = dlMap[row.mod.id.toLowerCase()];
          return (
            <tr
              key={row.mod.id}
              onClick={() => setDetail({ row: rowOfInstalled(row.mod), entry: row.entry })}
              className="group cursor-pointer text-slate-300 hover:bg-slate-800/40"
            >
              <td className="border-b border-slate-800/60 px-3 py-2">
                <span className="truncate text-white decoration-sky-400/60 group-hover:text-sky-300 group-hover:underline">
                  {shownNameOfRealm(row.mod)}
                </span>
              </td>
              <td className="truncate border-b border-slate-800/60 px-2 py-2 text-xs text-slate-500" title={row.mod.diskName || row.mod.id}>
                {row.mod.diskName || row.mod.id}
              </td>
              <td className="border-b border-slate-800/60 px-2 py-2 text-xs text-slate-400">
                {row.mod.type ? (t.mods.typeLabels[row.mod.type] ?? row.mod.type) : "—"}
              </td>
              <td className="border-b border-slate-800/60 px-2 py-2 tabular-nums">{row.cur || "—"}</td>
              <td className="border-b border-slate-800/60 px-2 py-2 font-semibold tabular-nums text-amber-300">{row.next}</td>
              <td className="border-b border-slate-800/60 px-2 py-2 text-xs text-slate-400">
                {row.mod.source === "realm" ? mm.locationRealm : row.mod.source === "user" ? mm.locationUser : mm.locationGame}
              </td>
              <td className="border-b border-slate-800/60 px-2 py-2">
                <div className="flex flex-wrap items-center gap-1.5 text-xs" onClick={(e) => e.stopPropagation()}>
                  <button
                    type="button"
                    disabled={opBusy || qq?.state === "queued" || qq?.state === "running"}
                    onClick={() => setUpdDlg(row)}
                    className="rounded border border-amber-500/60 bg-amber-500/10 px-2 py-1 text-amber-200 hover:bg-amber-500/20 disabled:opacity-50"
                  >
                    {qq
                      ? busyRow
                        ? progressLabel(row.mod.id, mm.qRunning)
                        : queueLabel(qq, mm)
                      : mm.actUpdate}
                  </button>
                  {qq?.state === "queued" && (
                    <button
                      type="button"
                      onClick={() => dequeue(row.mod.id)}
                      className="rounded border border-slate-600 px-2 py-1 text-slate-400 hover:text-white"
                    >
                      {mm.actCancel}
                    </button>
                  )}
                  {busyRow && dl && !dl.done && dl.total > 0 && (
                    <div className="h-1 w-24 overflow-hidden rounded bg-slate-700">
                      <div
                        className="h-full bg-amber-400 transition-[width] duration-200"
                        style={{ width: `${pctOf(dl)}%` }}
                      />
                    </div>
                  )}
                </div>
              </td>
            </tr>
          );
        })}
        {typedUpdatable.length === 0 && (
          <tr>
            <td colSpan={7} className="px-2 py-10 text-center text-slate-500">
              {repoMap ? t.modsRepo.emptyUpdates : <RepoNotChecked t={t} m={m} busy={repoBusy} onCheck={onRepoCheck} />}
            </td>
          </tr>
        )}
      </tbody>
    </table>
  );
}
