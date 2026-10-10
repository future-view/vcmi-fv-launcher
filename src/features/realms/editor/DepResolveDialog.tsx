/**
 * 依赖处置弹窗（依赖处置体系）：按「已实装根模组」聚合的缺失依赖清单 + 一键处理。
 *
 * 处理语义 = 实装（引入 / 安装）+ 补入受影响预设激活集（执行逻辑在宿主
 * `RealmModManagerPanel.resolveDepOne`）：引擎启动只认预设激活集
 * （`ModManager::getActiveMods`），仅实装不启用仍会被 BROKEN 剔除。
 * 未处理项保持缺失：游戏运行时引擎自行剔除受影响模组。
 *
 * 修复三档（本弹窗动作依据，求解见 `lib/modDeps.ts`）：
 * - `installed`（全局已装 → 引入）/ `repo`（仓库可得 → 安装）/
 *   `unavailable`（不可得 → 「忽略」写 `realm.json mods.depIgnores` 静音）；
 *   危害分级：🔴 active（受影响模组已在某预设启用）/ 🟠 latent（潜在）。
 *
 * 语言分组（决策：按领域配置语言处理，不锁死单一语言）：翻译包且语言不在
 * 领域语言集合 → 收进「其他语言包」折叠区（仍可手动安装、不参与批量口）；
 * 依据引擎语义（`isResolved`）：非当前语言的翻译模组启动即被 BROKEN 剔除。
 */
import { useMemo, useState } from "react";
import { AlertTriangle, Check, ChevronDown, Download, Link2, X } from "lucide-react";
import { cn } from "@/lib/utils";
import type { LangDict } from "@/i18n/types";
import { detectTranslationLang, type DepDefect, type RootDepDefect } from "@/lib/modDeps";

export interface DepResolveDialogProps {
  t: LangDict;
  /** 聚合缺陷（「已实装」行按钮打开时派生；忽略 / 处理后由宿主求解实时收敛） */
  info: RootDepDefect;
  /** 领域语言集合（小写：defaultLang + i18n 覆盖键）：其他语言翻译包折叠依据 */
  langs: string[];
  /** 行级处理状态（key = 依赖 id 小写） */
  states: Record<string, "busy" | "done" | "fail">;
  /** 任一处理进行中（禁用交互与关闭） */
  busy: boolean;
  onClose: () => void;
  /** 单条处理（引入 / 安装 + 补入激活集） */
  onResolve: (dep: DepDefect) => void;
  /** 批量处理（同源串行，逐个反馈） */
  onResolveAll: (kind: "installed" | "repo") => void;
  /** 忽略不可得依赖（写领域 `depIgnores` 静音；随「保存领域」落盘） */
  onIgnore: (dep: string) => void;
}

export function DepResolveDialog({
  t,
  info,
  langs,
  states,
  busy,
  onClose,
  onResolve,
  onResolveAll,
  onIgnore,
}: DepResolveDialogProps) {
  const m = t.realm.modsManage;
  const [otherOpen, setOtherOpen] = useState(false);
  const langSet = useMemo(() => new Set(langs.map((s) => s.toLowerCase())), [langs]);

  /** 其他语言翻译包：翻译包且已知语言不在领域集合 → 折叠区 */
  const isOtherLang = (d: DepDefect): boolean => {
    const lang = detectTranslationLang(d.entry?.id ?? d.depRoot, d.entry?.name);
    return lang !== null && lang !== "unknown" && !langSet.has(lang);
  };
  const primary = info.defects.filter((d) => !isOtherLang(d));
  const other = info.defects.filter((d) => isOtherLang(d));

  // 批量计数动态化：已处理（done）的不再计入，按钮随后消失；仅统计主区（折叠区语言包不进批量口）
  const refLeft = primary.filter(
    (d) => d.source === "installed" && states[d.dep.toLowerCase()] !== "done",
  ).length;
  const instLeft = primary.filter(
    (d) => d.source === "repo" && states[d.dep.toLowerCase()] !== "done",
  ).length;
  const affectedTip = (d: DepDefect) =>
    m.depAffectedTip
      .replace("{n}", String(d.affected.length))
      .replace("{list}", d.affected.map((a) => a.name).join("、"));

  /** 缺陷行（主列表与折叠区共用）：不可得行操作格 = 「忽略」 */
  const renderRow = (d: DepDefect) => {
    const st = states[d.dep.toLowerCase()];
    return (
      <div
        key={d.dep}
        className="flex items-center gap-2 rounded-lg border border-slate-800 bg-slate-950/40 px-3 py-2"
      >
        <span
          className={cn(
            "h-1.5 w-1.5 flex-none rounded-full",
            d.severity === "active" ? "bg-red-400" : "bg-amber-400/80",
          )}
          title={d.severity === "active" ? m.depSevActive : m.depSevLatent}
        />
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-1.5">
            <span className="min-w-0 truncate text-xs text-slate-200" title={d.dep}>
              {d.entry?.name || d.dep}
            </span>
            {d.source === "unavailable" && (
              <span className="flex-none rounded border border-slate-700 px-1 py-px text-[10px] text-slate-500">
                {m.depUnavailable}
              </span>
            )}
          </div>
          <div className="truncate text-[10px] text-slate-500" title={affectedTip(d)}>
            {affectedTip(d)}
          </div>
        </div>
        <span
          className={cn(
            "flex-none rounded px-1.5 py-0.5 text-[10px]",
            d.severity === "active"
              ? "bg-red-500/15 text-red-300"
              : "bg-amber-500/10 text-amber-300/80",
          )}
        >
          {d.severity === "active" ? m.depSevActive : m.depSevLatent}
        </span>
        {st === "done" ? (
          <span className="flex w-[76px] flex-none items-center justify-end gap-0.5 text-[11px] text-emerald-300">
            <Check className="h-3 w-3" />
            {m.depDone}
          </span>
        ) : st === "fail" ? (
          <span className="w-[76px] flex-none text-right text-[11px] text-red-400">{m.depFailTag}</span>
        ) : st === "busy" ? (
          <span className="w-[76px] flex-none text-right text-[11px] text-slate-400">{m.depBusy}</span>
        ) : d.source === "unavailable" ? (
          <button
            type="button"
            disabled={busy}
            onClick={() => onIgnore(d.dep)}
            className="w-[76px] flex-none rounded border border-slate-700 px-1.5 py-1 text-[11px] text-slate-400 hover:border-slate-500 hover:text-slate-200 disabled:opacity-40"
          >
            {m.depIgnore}
          </button>
        ) : (
          <button
            type="button"
            disabled={busy}
            onClick={() => onResolve(d)}
            className={cn(
              "flex w-[76px] flex-none items-center justify-center gap-0.5 rounded border px-1.5 py-1 text-[11px] disabled:opacity-40",
              d.source === "installed"
                ? "border-sky-500/60 text-sky-300 hover:bg-sky-500/15"
                : "border-emerald-500/60 text-emerald-300 hover:bg-emerald-500/15",
            )}
          >
            {d.source === "installed" ? (
              <Link2 className="h-3 w-3" />
            ) : (
              <Download className="h-3 w-3" />
            )}
            {d.source === "installed" ? m.depRef : m.depInstall}
          </button>
        )}
      </div>
    );
  };

  return (
    <div
      className="fixed inset-0 z-[100] flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm"
      onClick={() => {
        if (!busy) onClose();
      }}
    >
      <div
        className="flex max-h-[80vh] w-[560px] min-h-0 flex-col rounded-xl border border-slate-700/60 bg-slate-900 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        {/* 头部：标题 + 根模组名 */}
        <div className="flex flex-none items-center gap-2 px-5 pb-2 pt-4">
          <AlertTriangle className="h-4 w-4 flex-none text-amber-300" />
          <h3 className="flex-none text-sm font-semibold text-slate-100">{m.depTitle}</h3>
          <span className="min-w-0 flex-1 truncate text-xs text-slate-400" title={info.rootName}>
            {info.rootName}
          </span>
          <button
            type="button"
            onClick={onClose}
            disabled={busy}
            className="flex-none rounded p-1 text-slate-500 hover:bg-slate-800 hover:text-slate-200 disabled:opacity-40"
          >
            <X className="h-4 w-4" />
          </button>
        </div>
        {/* 摘要 */}
        <div className="flex-none px-5 pb-3 text-xs text-slate-400">
          {m.depSum.replace("{n}", String(info.total)).replace("{fixable}", String(info.fixable))}
        </div>
        {/* 缺失依赖清单 */}
        <div className="min-h-0 flex-1 overflow-y-auto px-5 pb-3">
          <div className="flex flex-col gap-1.5">
            {primary.map(renderRow)}
            {/* 其他语言包：折叠收纳（仍可逐条安装；引擎按当前语言自动禁用） */}
            {other.length > 0 && (
              <div className="rounded-lg border border-slate-800 bg-slate-950/30">
                <button
                  type="button"
                  onClick={() => setOtherOpen((v) => !v)}
                  className="flex w-full items-center gap-1.5 px-3 py-2 text-[11px] text-slate-500 hover:text-slate-300"
                >
                  <ChevronDown
                    className={cn("h-3 w-3 flex-none transition-transform", otherOpen && "rotate-180")}
                  />
                  <span className="flex-none">{m.depOtherLang.replace("{n}", String(other.length))}</span>
                </button>
                {otherOpen && (
                  <>
                    <div className="px-3 pb-1.5 text-[10px] leading-4 text-slate-500">
                      {m.depOtherLangHint.replace("{langs}", langs.join(" / "))}
                    </div>
                    <div className="flex flex-col gap-1.5 px-1.5 pb-1.5">{other.map(renderRow)}</div>
                  </>
                )}
              </div>
            )}
          </div>
        </div>
        {/* 底部：批量 + 关闭 */}
        <div className="flex flex-none items-center gap-2 border-t border-slate-800 px-5 py-3">
          {refLeft > 0 && (
            <button
              type="button"
              disabled={busy}
              onClick={() => onResolveAll("installed")}
              className="rounded border border-sky-500/60 px-2.5 py-1.5 text-xs text-sky-300 hover:bg-sky-500/15 disabled:opacity-40"
            >
              {m.depAllRef.replace("{n}", String(refLeft))}
            </button>
          )}
          {instLeft > 0 && (
            <button
              type="button"
              disabled={busy}
              onClick={() => onResolveAll("repo")}
              className="rounded border border-emerald-500/60 px-2.5 py-1.5 text-xs text-emerald-300 hover:bg-emerald-500/15 disabled:opacity-40"
            >
              {m.depAllInstall.replace("{n}", String(instLeft))}
            </button>
          )}
          <div className="flex-1" />
          <button
            type="button"
            onClick={onClose}
            disabled={busy}
            className="rounded-lg border border-slate-700/60 px-3 py-1.5 text-xs text-slate-300 hover:bg-slate-800/60 disabled:opacity-40"
          >
            {t.common.close}
          </button>
        </div>
        {/* 脚注：处理语义说明（实装 + 补入激活集） */}
        <div className="flex-none rounded-b-xl border-t border-slate-800/60 px-5 py-2 text-[10px] leading-4 text-slate-500">
          {m.depNote}
        </div>
      </div>
    </div>
  );
}
