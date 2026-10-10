/**
 * 依赖闭包解析弹窗（依赖处置 · 一键解析）：把「逐条修复不收敛」的多轮手工操作
 * 压缩为一次预览 + 一次执行。
 *
 * 数据由宿主打开 / 重新解析时注入（`solveDepClosure` 输出，`lib/modDeps.ts`）：
 * - 「将引用」：全局已装的依赖根（**默认全选**——用户只需父模组引入即可）；
 * - 「需安装」：仓库可得项，按**领域语言集合**默认勾选匹配的翻译包；其余语言包
 *   折叠收纳（依据引擎语义：翻译模组语言不匹配时启动即 BROKEN 剔除）；
 * - 「无法解决」：任何来源都不存在的死结，提供「忽略」（写 `realm.json mods.depIgnores`，
 *   纯 UI 静音——VCMI 引擎本身容忍依赖缺失，BROKEN 剔除不阻断启动）。
 *
 * 执行 = 引用（批量台账）+ 安装（仓库副本）+ 对每项补入受影响预设激活集
 * （与 `resolveDepOne` 同语义，执行逻辑在宿主 `RealmModManagerPanel.runDepClosure`）。
 */
import { useEffect, useMemo, useState } from "react";
import { AlertTriangle, Check, ChevronDown, Download, Link2, Loader2, Sparkles, X } from "lucide-react";
import { cn } from "@/lib/utils";
import type { LangDict } from "@/i18n/types";
import type { ClosureInstallItem, ClosureRefItem, DepClosureResult } from "@/lib/modDeps";

export interface DepClosureDialogProps {
  t: LangDict;
  /** 闭包处置单（打开时 / 执行完成后由宿主重新解析注入） */
  data: DepClosureResult;
  /** 领域语言集合（**小写**：defaultLang + i18n 覆盖键）：安装区默认勾选匹配项 */
  langs: string[];
  /** 执行中的行级状态（key = rootKey 小写 / dep 小写） */
  states: Record<string, "busy" | "done" | "fail">;
  /** 执行中（禁用交互与关闭） */
  busy: boolean;
  onClose: () => void;
  /** 一键执行（勾选的引用根 + 安装项，宿主串行执行并逐行反馈） */
  onRun: (plan: { refs: ClosureRefItem[]; installs: ClosureInstallItem[] }) => void;
  /** 忽略（写 depIgnores）：单条 / 全部（死结清单） */
  onIgnore: (deps: string[]) => void;
}

export function DepClosureDialog({
  t,
  data,
  langs,
  states,
  busy,
  onClose,
  onRun,
  onIgnore,
}: DepClosureDialogProps) {
  const m = t.realm.modsManage;
  const langSet = useMemo(() => new Set(langs.map((s) => s.toLowerCase())), [langs]);

  /** 选择状态（默认全选引用 = 决策点 1；安装按语言默认勾选） */
  const [selRefs, setSelRefs] = useState<Set<string>>(new Set());
  const [selInstalls, setSelInstalls] = useState<Set<string>>(new Set());
  const [otherOpen, setOtherOpen] = useState(false);

  /** 处置单快照变化（首次打开 / 执行后重新解析）→ 按默认策略重置勾选 */
  useEffect(() => {
    setSelRefs(new Set(data.refs.map((r) => r.rootKey)));
    setSelInstalls(
      new Set(
        data.installs
          .filter((it) => !it.lang || it.lang === "unknown" || langSet.has(it.lang))
          .map((it) => it.dep.toLowerCase()),
      ),
    );
  }, [data, langSet]);

  const installMain = data.installs.filter(
    (it) => !it.lang || it.lang === "unknown" || langSet.has(it.lang),
  );
  const installOther = data.installs.filter(
    (it) => it.lang && it.lang !== "unknown" && !langSet.has(it.lang),
  );
  const refItemTotal = data.refs.reduce((acc, r) => acc + r.itemCount, 0);
  const runnable = selRefs.size + selInstalls.size;
  const allEmpty = data.refs.length === 0 && data.installs.length === 0;

  const toggle = (set: Set<string>, key: string) => {
    const n = new Set(set);
    if (n.has(key)) n.delete(key);
    else n.add(key);
    return n;
  };
  const affectedTip = (names: string[]) =>
    m.depAffectedTip.replace("{n}", String(names.length)).replace("{list}", names.join("、"));
  /** 行级状态格（busy / done / fail；与处理弹窗同一视觉语言） */
  const stateCell = (st: "busy" | "done" | "fail" | undefined) =>
    st === "done" ? (
      <span className="flex w-[64px] flex-none items-center justify-end gap-0.5 text-[11px] text-emerald-300">
        <Check className="h-3 w-3" />
        {m.depDone}
      </span>
    ) : st === "fail" ? (
      <span className="w-[64px] flex-none text-right text-[11px] text-red-400">{m.depFailTag}</span>
    ) : st === "busy" ? (
      <span className="w-[64px] flex-none text-right text-[11px] text-slate-400">{m.depBusy}</span>
    ) : null;
  /** 安装行（主列表与折叠区共用；禁用 done 行） */
  const installRow = (it: ClosureInstallItem) => {
    const lc = it.dep.toLowerCase();
    const checked = selInstalls.has(lc);
    const st = states[lc];
    return (
      <label
        key={lc}
        className="flex cursor-pointer items-center gap-2 border-b border-slate-800/40 px-3 py-1.5 last:border-b-0 hover:bg-slate-800/30"
      >
        <input
          type="checkbox"
          checked={checked}
          disabled={busy || st === "done"}
          onChange={() => setSelInstalls((s) => toggle(s, lc))}
          className="h-3.5 w-3.5 flex-none accent-emerald-500"
        />
        <span className="min-w-0 flex-1 truncate text-xs text-slate-200" title={affectedTip(it.affectedNames)}>
          {it.name}
        </span>
        {it.lang && it.lang !== "unknown" && (
          <span className="flex-none rounded bg-slate-800 px-1.5 py-0.5 text-[10px] uppercase text-slate-400">
            {it.lang}
          </span>
        )}
        {stateCell(st)}
      </label>
    );
  };

  return (
    <div
      className="fixed inset-0 z-[110] flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm"
      onClick={() => {
        if (!busy) onClose();
      }}
    >
      <div
        className="flex max-h-[82vh] w-[720px] min-h-0 flex-col rounded-xl border border-slate-700/60 bg-slate-900 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        {/* 头部：标题 + 关闭 */}
        <div className="flex flex-none items-center gap-2 px-5 pb-2 pt-4">
          <Sparkles className="h-4 w-4 flex-none text-sky-300" />
          <h3 className="flex-none text-sm font-semibold text-slate-100">{m.depClosureTitle}</h3>
          <span className="min-w-0 flex-1" />
          <button
            type="button"
            onClick={onClose}
            disabled={busy}
            className="flex-none rounded p-1 text-slate-500 hover:bg-slate-800 hover:text-slate-200 disabled:opacity-40"
          >
            <X className="h-4 w-4" />
          </button>
        </div>
        {/* 摘要：完整代价预览（引用 / 解决 / 安装 / 无解） */}
        <div className="flex-none px-5 pb-3 text-xs leading-5 text-slate-400">
          {m.depClosureSum
            .replace("{refs}", String(data.refs.length))
            .replace("{items}", String(refItemTotal))
            .replace("{resolves}", String(data.resolvedCount))
            .replace("{installs}", String(data.installs.length))
            .replace("{deads}", String(data.deads.length))}
        </div>
        {/* 主体：三组处置单 */}
        <div className="min-h-0 flex-1 overflow-y-auto px-5 pb-3">
          {allEmpty ? (
            <div className="py-10 text-center text-xs text-slate-500">{m.depClosureEmpty}</div>
          ) : (
            <div className="flex flex-col gap-3">
              {/* ① 将引用：默认全选 */}
              {data.refs.length > 0 && (
                <section className="rounded-lg border border-sky-900/50 bg-sky-950/10">
                  <header className="flex items-center gap-2 border-b border-slate-800/60 px-3 py-2">
                    <Link2 className="h-3.5 w-3.5 flex-none text-sky-300" />
                    <span className="flex-none text-xs font-medium text-sky-200">
                      {m.depClosureRefGroup.replace("{n}", String(data.refs.length))}
                    </span>
                    <span className="flex-none text-[10px] text-slate-500">
                      {m.depClosureItems.replace("{n}", String(refItemTotal))}
                    </span>
                    <span className="min-w-0 flex-1" />
                    <button
                      type="button"
                      disabled={busy}
                      onClick={() =>
                        setSelRefs((s) =>
                          s.size === data.refs.length ? new Set() : new Set(data.refs.map((r) => r.rootKey)),
                        )
                      }
                      className="flex-none rounded border border-slate-700 px-1.5 py-0.5 text-[10px] text-slate-400 hover:text-slate-200 disabled:opacity-40"
                    >
                      {selRefs.size === data.refs.length ? t.common.selectNone : t.common.selectAll}
                    </button>
                  </header>
                  {data.refs.map((r) => {
                    const checked = selRefs.has(r.rootKey);
                    const st = states[r.rootKey];
                    return (
                      <label
                        key={r.rootKey}
                        className="flex cursor-pointer items-center gap-2 border-b border-slate-800/40 px-3 py-1.5 last:border-b-0 hover:bg-slate-800/30"
                      >
                        <input
                          type="checkbox"
                          checked={checked}
                          disabled={busy || st === "done"}
                          onChange={() => setSelRefs((s) => toggle(s, r.rootKey))}
                          className="h-3.5 w-3.5 flex-none accent-sky-500"
                        />
                        <span
                          className="min-w-0 flex-1 truncate text-xs text-slate-200"
                          title={r.deps.join("、")}
                        >
                          {r.rootName}
                        </span>
                        {r.itemCount > 0 && (
                          <span className="flex-none rounded bg-slate-800 px-1.5 py-0.5 text-[10px] text-slate-400">
                            {m.depClosureItems.replace("{n}", String(r.itemCount))}
                          </span>
                        )}
                        <span className="flex-none rounded bg-sky-500/15 px-1.5 py-0.5 text-[10px] text-sky-300">
                          {m.depClosureResolves.replace("{n}", String(r.resolves))}
                        </span>
                        {stateCell(st)}
                      </label>
                    );
                  })}
                </section>
              )}
              {/* ② 需安装：语言匹配默认勾选；其他语言包折叠 */}
              {data.installs.length > 0 && (
                <section className="rounded-lg border border-emerald-900/50 bg-emerald-950/10">
                  <header className="flex items-center gap-2 border-b border-slate-800/60 px-3 py-2">
                    <Download className="h-3.5 w-3.5 flex-none text-emerald-300" />
                    <span className="flex-none text-xs font-medium text-emerald-200">
                      {m.depClosureInstallGroup.replace("{n}", String(data.installs.length))}
                    </span>
                    <span className="min-w-0 flex-1" />
                    {installMain.length > 0 && (
                      <button
                        type="button"
                        disabled={busy}
                        onClick={() =>
                          setSelInstalls((s) => {
                            const n = new Set(s);
                            const keys = installMain.map((it) => it.dep.toLowerCase());
                            const allOn = keys.every((k) => n.has(k));
                            for (const k of keys) {
                              if (allOn) n.delete(k);
                              else n.add(k);
                            }
                            return n;
                          })
                        }
                        className="flex-none rounded border border-slate-700 px-1.5 py-0.5 text-[10px] text-slate-400 hover:text-slate-200 disabled:opacity-40"
                      >
                        {installMain.every((it) => selInstalls.has(it.dep.toLowerCase()))
                          ? t.common.selectNone
                          : t.common.selectAll}
                      </button>
                    )}
                  </header>
                  {installMain.length > 0 && (
                    <div className="border-b border-slate-800/40 px-3 pt-1.5 text-[10px] text-slate-500">
                      {m.depClosureLangNote.replace("{langs}", langs.join(" / "))}
                    </div>
                  )}
                  {installMain.map(installRow)}
                  {installOther.length > 0 && (
                    <div className="border-t border-slate-800/40">
                      <button
                        type="button"
                        onClick={() => setOtherOpen((v) => !v)}
                        className="flex w-full items-center gap-1.5 px-3 py-1.5 text-[11px] text-slate-500 hover:text-slate-300"
                      >
                        <ChevronDown
                          className={cn("h-3 w-3 flex-none transition-transform", otherOpen && "rotate-180")}
                        />
                        <span className="flex-none">{m.depOtherLang.replace("{n}", String(installOther.length))}</span>
                      </button>
                      {otherOpen && (
                        <>
                          <div className="px-3 pb-1 text-[10px] leading-4 text-slate-500">
                            {m.depOtherLangHint.replace("{langs}", langs.join(" / "))}
                          </div>
                          {installOther.map(installRow)}
                        </>
                      )}
                    </div>
                  )}
                </section>
              )}
              {/* ③ 无法解决：静态标记 + 忽略（写 depIgnores） */}
              {data.deads.length > 0 && (
                <section className="rounded-lg border border-slate-800 bg-slate-950/30">
                  <header className="flex items-center gap-2 border-b border-slate-800/60 px-3 py-2">
                    <AlertTriangle className="h-3.5 w-3.5 flex-none text-slate-400" />
                    <span className="flex-none text-xs font-medium text-slate-300">
                      {m.depClosureDeadGroup.replace("{n}", String(data.deads.length))}
                    </span>
                  </header>
                  <div className="px-3 pt-1.5 text-[10px] leading-4 text-slate-500">{m.depClosureDeadHint}</div>
                  {data.deads.map((d) => (
                    <div
                      key={d.dep.toLowerCase()}
                      className="flex items-center gap-2 border-b border-slate-800/40 px-3 py-1.5 last:border-b-0"
                    >
                      <span className="h-1.5 w-1.5 flex-none rounded-full bg-slate-500/70" />
                      <span className="min-w-0 flex-1 truncate text-xs text-slate-300" title={affectedTip(d.affectedNames)}>
                        {d.name}
                      </span>
                      <button
                        type="button"
                        disabled={busy}
                        onClick={() => onIgnore([d.dep])}
                        className="flex-none rounded border border-slate-700 px-1.5 py-1 text-[11px] text-slate-400 hover:text-slate-200 disabled:opacity-40"
                      >
                        {m.depIgnore}
                      </button>
                    </div>
                  ))}
                </section>
              )}
            </div>
          )}
        </div>
        {/* 底部：忽略全部无解 + 执行 + 关闭 */}
        <div className="flex flex-none items-center gap-2 border-t border-slate-800 px-5 py-3">
          {data.deads.length > 0 && (
            <button
              type="button"
              disabled={busy}
              onClick={() => onIgnore(data.deads.map((d) => d.dep))}
              className="rounded border border-slate-700/60 px-2.5 py-1.5 text-xs text-slate-400 hover:text-slate-200 disabled:opacity-40"
            >
              {`${m.depIgnore} (${data.deads.length})`}
            </button>
          )}
          <div className="flex-1" />
          <button
            type="button"
            disabled={busy || runnable === 0}
            onClick={() =>
              onRun({
                refs: data.refs.filter((r) => selRefs.has(r.rootKey)),
                installs: data.installs.filter((it) => selInstalls.has(it.dep.toLowerCase())),
              })
            }
            className="flex items-center gap-1.5 rounded-lg bg-sky-500/90 px-3 py-1.5 text-xs font-medium text-white hover:bg-sky-500 disabled:opacity-40"
          >
            {busy ? (
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
            ) : (
              <Sparkles className="h-3.5 w-3.5" />
            )}
            {busy ? m.depClosureBusy : m.depClosureRun}
            {!busy && runnable > 0 && (
              <span className="rounded bg-white/15 px-1 text-[10px]">
                {m.depClosureRunTip
                  .replace("{refs}", String(selRefs.size))
                  .replace("{installs}", String(selInstalls.size))}
              </span>
            )}
          </button>
          <button
            type="button"
            onClick={onClose}
            disabled={busy}
            className="rounded-lg border border-slate-700/60 px-3 py-1.5 text-xs text-slate-300 hover:bg-slate-800/60 disabled:opacity-40"
          >
            {t.common.close}
          </button>
        </div>
      </div>
    </div>
  );
}
