/**
 * 「领域模组」tab 双视图容器（realm-mods-management §4.3 / §5.1）：
 * - 视图 A「模组预设」（默认）：预设条 + `ModBrowser`（数据源 = 领域已实装）；
 * - 视图 B「模组管理」：`RealmModManagerPanel` 四标签页（已实装 / 可引用 / 可下载 / 可更新）。
 *
 * 视图状态 `view` 由 `RealmEditor` 持有（§4.4：切走 tab 再回来保持，非路由）；
 * 原「模组引用抽屉」与「一键借用全部」职责已移入管理视图，本组件不再持有引用状态。
 */
import { useState } from "react";
import { AlertTriangle, Info } from "lucide-react";
import { cn } from "@/lib/utils";
import { ModBrowser } from "@/features/mods/ModBrowser";
import type { ModInfo } from "@/lib/modScanner";
import type { LangDict } from "@/i18n/types";
import type { DepSolveResult } from "@/lib/modDeps";
import { REALM_PRESET_ID } from "../editorShared";
import { RealmModManagerPanel, type RealmModManagerPanelProps } from "../RealmModManagerPanel";

/** 双视图标识（§4.4：非路由；切 tab 保持） */
export type ModsViewMode = "presets" | "manage";

export interface ModsTabProps {
  t: LangDict;
  /* ---------------- 视图 A：预设条 ---------------- */
  presets: Record<string, string[]>;
  activePreset: string;
  setActivePreset: React.Dispatch<React.SetStateAction<string>>;
  newPresetName: string;
  setNewPresetName: React.Dispatch<React.SetStateAction<string>>;
  addingPreset: boolean;
  setAddingPreset: React.Dispatch<React.SetStateAction<boolean>>;
  addPreset: () => void;
  deletePreset: () => void;
  /** 重命名模组组合（`__realm__` 为契约 id，不可重命名） */
  renamePreset: (from: string, to: string) => void;
  /* ---------------- 视图 A：模组双栏浏览 ---------------- */
  /** 领域范围模组（§5.1 数据源收紧为「已实装」：bundled ∪ pinned ∪ borrowed ∪ 引擎基础模组） */
  realmMods: ModInfo[];
  realmBaseMods: string[];
  realmSelected: Set<string>;
  /** 领域预设是否已从领域 modSettings.json 读出（false = 加载中，禁止改动） */
  presetsReady: boolean;
  handleRealmSelection: (next: Set<string>) => void;
  /** 必需依赖守卫触发：禁用会破坏依赖时回调（用于 toast 提示） */
  onDependencyBlocked?: (message: string) => void;
  /* ---------------- 双视图切换 ---------------- */
  view: ModsViewMode;
  setView: (v: ModsViewMode) => void;
  /** 依赖缺陷求解结果（操作栏徽章 / 告知弹层 / 树内角标消费，`lib/modDeps.ts`） */
  depSolve: DepSolveResult;
  /** 「去处理」：切到管理视图「已实装」页（可附开启依赖缺陷筛选） */
  onGoManage: (opts?: { depFilter?: boolean }) => void;
  /** 管理视图 props（t / onBack 由本组件注入） */
  manage: Omit<RealmModManagerPanelProps, "t" | "onBack">;
}

export function ModsTab(props: ModsTabProps) {
  const {
    t,
    presets,
    activePreset,
    setActivePreset,
    newPresetName,
    setNewPresetName,
    addingPreset,
    setAddingPreset,
    addPreset,
    deletePreset,
    renamePreset,
    realmMods,
    realmBaseMods,
    realmSelected,
    presetsReady,
    handleRealmSelection,
    onDependencyBlocked,
    view,
    setView,
    depSolve,
    onGoManage,
    manage,
  } = props;
  const m = t.realm.modsManage;
  // 依赖缺陷告知弹层（操作栏「依赖处置」按钮 → 缺陷清单 + 去处理）
  const [depPop, setDepPop] = useState(false);
  // 缺陷清单（实际危害排前）：弹层与按钮徽章共用
  const defectRoots = [...depSolve.byRoot.values()].sort((a, b) =>
    a.severity === b.severity ? 0 : a.severity === "active" ? -1 : 1,
  );
  // 领域预设读出前：禁止切预设 / 增删组合 / 改选中，避免空选中被当成权威组合
  const busy = !presetsReady;
  /** 重命名模组组合（§5.1）：`__realm__` 为契约 id，不可重命名；重名 / 空名不迁移 */
  const [renaming, setRenaming] = useState(false);
  const [renameName, setRenameName] = useState("");
  const openRename = () => {
    setRenameName(activePreset);
    setRenaming(true);
    setAddingPreset(false);
  };
  const confirmRename = () => {
    const n = renameName.trim();
    if (n && n !== activePreset && !presets[n]) renamePreset(activePreset, n);
    setRenaming(false);
  };

  /* ---------------- 视图 B：模组管理（§5.2 四标签页） ---------------- */
  if (view === "manage") {
    return <RealmModManagerPanel t={t} {...manage} onBack={() => setView("presets")} />;
  }

  /* ---------------- 视图 A：模组预设 ---------------- */
  // 数据源统计（ⓘ hover；根模组口径对齐「已实装」表格）与缺省页判据
  const roots = manage.installed.filter((x) => !x.isSubmod);
  const posCount = roots.filter((x) => x.source === "realm").length;
  const userCount = roots.filter((x) => x.source === "user").length;
  const gameCount = roots.filter((x) => x.source === "game").length;
  const empty = roots.length === 0;

  return (
    <>
      {/* 预设管理条 +「模组管理 →」入口 + 独立 ⓘ（§5.1：点击入口与信息展示职责分离） */}
      <div className="flex flex-wrap items-center gap-2 rounded-lg border border-slate-800 bg-slate-900/40 p-2">
        <span className="text-xs text-muted">{t.realm.presetLabel}</span>
        {renaming ? (
          <>
            <input
              value={renameName}
              onChange={(e) => setRenameName(e.target.value)}
              onKeyDown={(e) => e.key === "Enter" && confirmRename()}
              autoFocus
              className="w-40 rounded border border-slate-700 bg-slate-900 px-2 py-1 text-xs text-foreground"
            />
            <button
              type="button"
              onClick={confirmRename}
              disabled={!renameName.trim() || renameName.trim() === activePreset || !!presets[renameName.trim()]}
              className="rounded border border-blue-600 px-2 py-1 text-xs text-blue-300 hover:bg-blue-600/20 disabled:opacity-40"
            >
              {t.common.ok}
            </button>
            <button
              type="button"
              onClick={() => setRenaming(false)}
              className="rounded border border-slate-700 px-2 py-1 text-xs text-slate-300 hover:bg-slate-800/50"
            >
              {t.common.cancel}
            </button>
          </>
        ) : (
          <select
            value={activePreset}
            onChange={(e) => setActivePreset(e.target.value)}
            disabled={busy}
            className="rounded border border-slate-700 bg-slate-900 px-2 py-1 text-xs text-foreground disabled:opacity-40"
          >
            {(Object.keys(presets).length === 0 ? [activePreset] : Object.keys(presets)).map((pid) => (
              <option key={pid} value={pid}>
                {pid}
              </option>
            ))}
          </select>
        )}
        {addingPreset ? (
          <>
            <input
              value={newPresetName}
              onChange={(e) => setNewPresetName(e.target.value)}
              placeholder={t.realm.modPreset}
              className="w-40 rounded border border-slate-700 bg-slate-900 px-2 py-1 text-xs text-foreground"
            />
            <button
              type="button"
              onClick={addPreset}
              disabled={!newPresetName.trim() || !!presets[newPresetName.trim()]}
              className="rounded border border-blue-600 px-2 py-1 text-xs text-blue-300 hover:bg-blue-600/20 disabled:opacity-40"
            >
              {t.common.ok}
            </button>
            <button
              type="button"
              onClick={() => {
                setAddingPreset(false);
                setNewPresetName("");
              }}
              className="rounded border border-slate-700 px-2 py-1 text-xs text-slate-300 hover:bg-slate-800/50"
            >
              {t.common.cancel}
            </button>
          </>
        ) : (
          <button
            type="button"
            onClick={() => {
              setAddingPreset(true);
              setRenaming(false);
            }}
            disabled={busy}
            className="rounded border border-slate-700 px-2 py-1 text-xs text-slate-300 hover:bg-slate-800/50 disabled:opacity-40"
          >
            {t.realm.presetNew}
          </button>
        )}
        <button
          type="button"
          onClick={openRename}
          disabled={busy || activePreset === REALM_PRESET_ID}
          className="rounded border border-slate-700 px-2 py-1 text-xs text-slate-300 hover:bg-slate-800/50 disabled:opacity-40"
        >
          {t.preset.rename}
        </button>
        <button
          type="button"
          onClick={deletePreset}
          disabled={busy || Object.keys(presets).length <= 1}
          className="rounded border border-slate-700 px-2 py-1 text-xs text-slate-300 hover:bg-slate-800/50 disabled:opacity-40"
        >
          {t.realm.presetDelete}
        </button>
        <div className="flex-1" />
        {/* 依赖处置（依赖处置体系）：有缺依赖的已实装模组时点亮；轻量弹层列缺陷清单 + 去处理 */}
        <span className="relative">
          <button
            type="button"
            onClick={() => setDepPop((v) => !v)}
            disabled={depSolve.defectRootCount === 0}
            className={cn(
              "flex items-center gap-1 rounded border px-2.5 py-1 text-xs",
              depSolve.defectRootCount > 0
                ? "border-amber-500/60 text-amber-300 hover:bg-amber-500/10"
                : "border-slate-700 text-slate-500 opacity-50",
            )}
          >
            <AlertTriangle className="h-3.5 w-3.5" />
            {m.depBtn}
            {depSolve.defectRootCount > 0 && (
              <span className="rounded bg-amber-500/30 px-1 text-[10px] text-amber-100">
                {depSolve.defectRootCount}
              </span>
            )}
          </button>
          {depPop && (
            <>
              {/* 点击外部关闭的透明遮罩（z 低于弹层） */}
              <div className="fixed inset-0 z-[60]" onClick={() => setDepPop(false)} />
              <div className="absolute right-0 top-full z-[70] mt-1 w-80 rounded-lg border border-slate-700 bg-slate-900 p-3 text-left shadow-2xl">
                <div className="mb-1 flex items-center gap-1.5 text-xs font-medium text-amber-300">
                  <AlertTriangle className="h-3.5 w-3.5" />
                  {m.depTitle}
                </div>
                <p className="mb-2 text-[11px] leading-5 text-slate-400">
                  {m.depDesc.replace("{n}", String(depSolve.defectRootCount))}
                </p>
                <div className="mb-2 max-h-[200px] overflow-y-auto rounded border border-slate-800 bg-slate-950/50">
                  {defectRoots.map((r) => (
                    <div
                      key={r.rootId}
                      className="flex items-center gap-1.5 border-b border-slate-800/60 px-2 py-1.5 text-[11px] last:border-b-0"
                    >
                      <span
                        className={cn(
                          "h-1.5 w-1.5 flex-none rounded-full",
                          r.severity === "active" ? "bg-red-400" : "bg-amber-400/80",
                        )}
                      />
                      <span className="min-w-0 flex-1 truncate text-slate-200" title={r.rootName}>
                        {r.rootName}
                      </span>
                      <span className="flex-none text-slate-500">
                        {m.depCount.replace("{n}", String(r.total))}
                      </span>
                      <span
                        className={cn(
                          "flex-none rounded px-1 text-[10px]",
                          r.severity === "active"
                            ? "bg-red-500/20 text-red-300"
                            : "bg-amber-500/15 text-amber-300/90",
                        )}
                      >
                        {r.severity === "active" ? m.depSevActive : m.depSevLatent}
                      </span>
                    </div>
                  ))}
                </div>
                <button
                  type="button"
                  onClick={() => {
                    setDepPop(false);
                    onGoManage({ depFilter: true });
                  }}
                  className="w-full rounded bg-amber-500/90 px-2 py-1.5 text-xs font-medium text-white hover:bg-amber-500"
                >
                  {m.depGo} →
                </button>
              </div>
            </>
          )}
        </span>
        <button
          type="button"
          onClick={() => setView("manage")}
          className="rounded border border-indigo-600 px-2.5 py-1 text-xs text-indigo-300 hover:bg-indigo-600/20"
        >
          {m.actManage} →
          {roots.length > 0 && (
            <span className="ml-1 rounded bg-indigo-500/30 px-1 text-[10px]">{roots.length}</span>
          )}
        </button>
        <span className="group relative flex items-center">
          <Info className="h-4 w-4 cursor-help text-slate-500 hover:text-slate-300" />
          <span className="pointer-events-none absolute right-0 top-full z-[70] mt-1 hidden w-80 rounded-lg border border-slate-700 bg-slate-900 p-2.5 text-left shadow-2xl group-hover:block">
            <span className="mb-1 block text-xs font-medium text-slate-200">{m.scopeTitle}</span>
            <span className="block text-[11px] leading-5 text-slate-400">
              {m.scopeLine
                .replace("{pos}", String(posCount))
                .replace("{user}", String(userCount))
                .replace("{game}", String(gameCount))}
            </span>
            <span className="mt-1 block text-[11px] leading-5 text-slate-500">{m.scopeHint}</span>
          </span>
        </span>
      </div>

      {empty ? (
        /* 空状态缺省页（§5.1 补充点 c）：概念卡 + 步骤引导；已实装 > 0 后不再出现 */
        <div className="flex h-[70vh] min-h-0 flex-col items-center justify-center gap-5 overflow-y-auto rounded-lg border border-slate-800 bg-slate-900/30 p-6">
          <h3 className="text-base font-semibold text-white">{m.emptyTitle}</h3>
          <div className="grid w-full max-w-3xl grid-cols-3 gap-3">
            {[
              [m.emptyRefT, m.emptyRefD, "border-sky-500/40"],
              [m.emptyBundleT, m.emptyBundleD, "border-fuchsia-500/40"],
              [m.emptyLocalT, m.emptyLocalD, "border-emerald-500/40"],
            ].map(([title, desc, tone]) => (
              <div key={title} className={`rounded-lg border bg-slate-900/60 p-3 text-left ${tone}`}>
                <div className="mb-1 text-sm font-medium text-white">{title}</div>
                <div className="text-xs leading-5 text-slate-400">{desc}</div>
              </div>
            ))}
          </div>
          <div className="flex items-center gap-2 text-xs text-slate-400">
            <span>{m.emptyStep1}</span>
            <span className="text-slate-600">·</span>
            <span>{m.emptyStep2}</span>
          </div>
          <button
            type="button"
            onClick={() => setView("manage")}
            className="rounded-lg bg-indigo-500/90 px-4 py-2 text-sm font-semibold text-white hover:bg-indigo-500"
          >
            {m.emptyGo} →
          </button>
        </div>
      ) : (
        <div className="h-[70vh] flex min-h-0 flex-col overflow-hidden rounded-lg border border-slate-800">
          <ModBrowser
            mods={realmMods}
            selected={realmSelected}
            baseMods={realmBaseMods}
            loading={busy}
            onChange={handleRealmSelection}
            onDependencyBlocked={onDependencyBlocked}
            defectIds={depSolve.defectModIds}
          />
        </div>
      )}
    </>
  );
}
