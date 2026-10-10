//! 更新弹窗（普通更新，交互原型 A）：可关闭、可「稍后 / 跳过此版本」。
//!
//! 关闭弹窗**不中断**已开始的下载（后端继续，重开时从后端状态恢复现场）。
//! 强制更新请用 `ForcedUpdateOverlay`（全屏阻断、自动应用）。

import { useEffect } from "react";
import { ArrowRight, RefreshCw, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import {
  checkLauncherUpdate,
  setUpdatePref,
  type UpdateInfo,
} from "@/lib/tauri";
import { formatBytes, pickNotes, progressPercent } from "@/lib/updater";
import { renderInline } from "@/shared/Markdown";
import { useUpdateFlow } from "./updateFlow";

interface Props {
  info: UpdateInfo;
  open: boolean;
  onClose: () => void;
  /** 更新结果变化回传 App（跳过此版本后刷新红点；重开弹窗同步后端阶段） */
  onInfoChange: (info: UpdateInfo | null) => void;
  /** 跳过此版本后置回调：App 据此弹出「已跳过 + 撤销」提示（仅普通更新触发） */
  onSkip?: (version: string) => void;
}

export function UpdateDialog({ info, open, onClose, onInfoChange, onSkip }: Props) {
  const { lang, t } = useI18n();
  const flow = useUpdateFlow(info, false);

  // 打开时向后端核对一次真实阶段（下载中/已下载可恢复现场），同步给 App
  useEffect(() => {
    if (!open) return;
    let alive = true;
    void checkLauncherUpdate(false).then((i) => {
      if (alive && i) onInfoChange(i);
    });
    return () => {
      alive = false;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  if (!open) return null;

  const notes = pickNotes(info, lang);
  const pct = flow.progress ? progressPercent(flow.progress.received, flow.progress.total) : null;
  const busy = flow.ui === "downloading" || flow.ui === "applying" || flow.ui === "restarting";
  // 已下载态：安装包已落地，跳过语义弱化为「暂不应用」，避免"包都下好了还永久跳过"的违和
  const downloaded = flow.ui === "downloaded";
  const skipLabel = downloaded ? t.update.skipDefer : t.update.skipVersion;

  const skip = async () => {
    try {
      // 已下载态点「暂不应用」只是关闭、保留待应用标记（下次启动仍提示）；
      // 未下载态点「跳过此版本」才是永久跳过该版本（清红点/自动提醒）。
      if (!downloaded) {
        // autoCheck 传 null = 不改变用户既有开关（原先传 true 会在"跳过此版本"时
        // 把已关闭的自动检查强行打开）
        await setUpdatePref(null, info.latestVersion);
      }
      // 静默口径复查（尊重 skip）→ App 红点随之消失
      const i = await checkLauncherUpdate(false);
      onInfoChange(i);
    } catch (e) {
      console.error("跳过此版本失败", e);
    }
    // 仅「真正跳过」时给反馈 + 撤销入口；「暂不应用」不动 skip_version
    if (!downloaded) onSkip?.(info.latestVersion);
    onClose();
  };

  return (
    <div className="fixed inset-0 z-[200] flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm">
      <div className="w-full max-w-lg rounded-2xl border border-slate-700/60 bg-slate-900 shadow-2xl">
        {/* 头部 */}
        <div className="flex items-center justify-between border-b border-slate-800 px-5 py-3.5">
          <h3 className="text-base font-semibold text-slate-100">{t.update.title}</h3>
          <button
            type="button"
            onClick={onClose}
            title={t.update.later}
            aria-label={t.update.later}
            className="rounded p-1 text-slate-500 transition-colors hover:bg-slate-800 hover:text-slate-200"
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        <div className="px-5 py-4">
          {/* 版本对比 + 日期 + 重要徽标 */}
          <div className="flex flex-wrap items-center gap-2.5">
            <span className="font-mono text-lg font-semibold text-slate-300">
              {info.currentVersion}
            </span>
            {/* 仅确有更新时渲染箭头与目标版本：检查失败/无更新时 latestVersion 为空串，
                否则会渲染出「0.1.0 → 」的假更新样式 */}
            {info.hasUpdate && (
              <>
                <ArrowRight className="h-4 w-4 text-blue-400" />
                <span className="bg-gradient-to-r from-blue-400 to-indigo-400 bg-clip-text font-mono text-lg font-bold text-transparent">
                  {info.latestVersion}
                </span>
                <span className="text-xs text-slate-500">{info.publishedAt}</span>
                {info.critical && (
                  <span className="rounded-full bg-amber-500/15 px-2 py-0.5 text-xs font-medium text-amber-400">
                    {t.update.critical}
                  </span>
                )}
              </>
            )}
            {!info.hasUpdate && (
              <span className="text-sm text-slate-400">{t.update.upToDate}</span>
            )}
          </div>

          {/* 更新说明 */}
          <div className="mt-3 max-h-[40vh] overflow-y-auto pr-1">
            {!info.hasUpdate ? null : notes.length === 0 ? (
              <p className="text-sm text-slate-500">{t.update.notesEmpty}</p>
            ) : (
              <ul className="space-y-1.5">
                {notes.map((n, i) => (
                  <li key={i} className="flex items-start gap-2 text-sm text-slate-300">
                    <span className="mt-1.5 h-1.5 w-1.5 flex-none rounded-full bg-blue-400/70" />
                    <span className="min-w-0">{renderInline(n)}</span>
                  </li>
                ))}
              </ul>
            )}
          </div>

          {/* 下载进度 */}
          {flow.ui === "downloading" && flow.progress && (
            <div className="mt-4">
              <div className="h-2 w-full overflow-hidden rounded-full bg-slate-800">
                <div
                  className="h-full rounded-full bg-gradient-to-r from-blue-500 to-indigo-500 transition-all duration-200"
                  style={{ width: pct != null ? `${pct}%` : "35%" }}
                />
              </div>
              <div className="mt-1.5 flex justify-between text-xs text-slate-400">
                <span>
                  {pct != null ? `${pct}%` : t.update.downloading}
                </span>
                <span>
                  {formatBytes(flow.progress.received)} / {formatBytes(flow.progress.total)}
                </span>
              </div>
            </div>
          )}

          {/* 状态 / 错误反馈 */}
          {flow.ui === "downloaded" && (
            <p className="mt-3 text-sm text-emerald-400">✓ {t.update.readyRestart}</p>
          )}
          {flow.ui === "restarting" && (
            <p className="mt-3 text-sm text-blue-400">{t.update.restarting}</p>
          )}
          {/* 已下载态提示：安装包已落地保留，跳过只是「暂不应用」而非删除 */}
          {downloaded && (
            <p className="mt-3 text-xs text-slate-500">{t.update.downloadedNote}</p>
          )}
          {/* 纯配置更新包：已就地应用，不关窗 */}
          {flow.ui === "applied" && (
            <p className="mt-3 text-sm text-emerald-400">✓ {t.update.appliedNoRestart}</p>
          )}
          {flow.ui === "error" && (
            <div className="mt-3">
              <p className="text-sm text-red-400">
                {t.update.failed}
                {flow.err ? `：${flow.err}` : ""}
              </p>
              {info.configured && info.state === "error" && (
                <p className="mt-1 text-xs text-slate-500">{t.update.debugHint}</p>
              )}
            </div>
          )}
        </div>

        {/* 操作区 */}
        <div className="flex items-center justify-between gap-2 border-t border-slate-800 px-5 py-3.5">
          <div>
            {!info.hasUpdate ? null : flow.ui === "error" ? (
              <button
                type="button"
                onClick={() => void flow.start()}
                className="flex items-center gap-1.5 rounded-lg border border-slate-700 px-3 py-2 text-sm text-slate-300 transition-colors hover:bg-slate-800"
              >
                <RefreshCw className="h-3.5 w-3.5" />
                {t.update.retry}
              </button>
            ) : (
              <button
                type="button"
                onClick={() => void skip()}
                disabled={busy}
                className={cn(
                  "text-sm text-slate-500 transition-colors hover:text-slate-300",
                  busy && "cursor-not-allowed opacity-50",
                )}
              >
                {skipLabel}
              </button>
            )}
          </div>
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={onClose}
              disabled={flow.ui === "restarting"}
              className={cn(
                "rounded-lg border border-slate-700/60 bg-slate-800/60 px-4 py-2 text-sm font-medium text-slate-300 transition-colors hover:bg-slate-800",
                flow.ui === "restarting" && "cursor-not-allowed opacity-50",
              )}
            >
              {t.update.later}
            </button>
            {!info.hasUpdate ? null : flow.ui === "downloading" ? (
              <button
                type="button"
                onClick={() => void flow.cancel()}
                className="rounded-lg border border-slate-700 px-4 py-2 text-sm font-medium text-slate-300 transition-colors hover:bg-slate-800"
              >
                {t.update.cancelDownload}
              </button>
            ) : flow.ui === "downloaded" ? (
              <button
                type="button"
                onClick={() => void flow.apply()}
                className="rounded-lg bg-gradient-to-br from-blue-500 to-indigo-500 px-4 py-2 text-sm font-medium text-white shadow-md shadow-blue-600/30 transition-opacity hover:opacity-90"
              >
                {t.update.restartApply}
              </button>
            ) : flow.ui === "applied" ? (
              <button
                type="button"
                onClick={onClose}
                className="rounded-lg bg-gradient-to-br from-emerald-500 to-teal-500 px-4 py-2 text-sm font-medium text-white shadow-md shadow-emerald-600/30 transition-opacity hover:opacity-90"
              >
                {t.app.close}
              </button>
            ) : flow.ui === "error" ? (
              <button
                type="button"
                onClick={() => void flow.start()}
                className="rounded-lg bg-gradient-to-br from-blue-500 to-indigo-500 px-4 py-2 text-sm font-medium text-white shadow-md shadow-blue-600/30 transition-opacity hover:opacity-90"
              >
                {t.update.downloadNow}
              </button>
            ) : (
              <button
                type="button"
                onClick={() => void flow.start()}
                disabled={busy}
                className={cn(
                  "rounded-lg bg-gradient-to-br from-blue-500 to-indigo-500 px-4 py-2 text-sm font-medium text-white shadow-md shadow-blue-600/30 transition-opacity hover:opacity-90",
                  busy && "cursor-not-allowed opacity-50",
                )}
              >
                {flow.ui === "applying" ? t.update.applying : t.update.downloadNow}
              </button>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
