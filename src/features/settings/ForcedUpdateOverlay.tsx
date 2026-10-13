//! 强制更新全屏阻断层（交互原型 B）：
//!
//! - `current < 清单 minSupported` 时由 `App` 在**最外层**渲染（主界面 / 领域界面都不渲染）；
//! - 无关闭按钮、无「稍后 / 跳过此版本」，只有「立即更新」与「退出启动器」；
//! - 下载校验完成后**自动应用重启**（用户已确认的设计决策）；
//! - 清单不可达时 App 不会走到这里（防误伤：放行）。
//!
//! 注意：调试构建（`is_debug`）由 App 侧保证不进入本组件，仅显示提示条。

import { ArrowRight, DownloadCloud, RefreshCw } from "lucide-react";
import { cn } from "@/lib/utils";
import brandIcon from "@/assets/brand-icon.png";
import { useI18n } from "@/i18n";
import type { UpdateInfo } from "@/lib/tauri";
import { formatBytes, pickNotes, progressPercent } from "@/lib/updater";
import { renderInline } from "@/shared/Markdown";
import { useUpdateFlow } from "./updateFlow";

interface Props {
  info: UpdateInfo;
  onExit: () => void;
}

export function ForcedUpdateOverlay({ info, onExit }: Props) {
  const { lang, t } = useI18n();
  const flow = useUpdateFlow(info, true);
  const notes = pickNotes(info, lang);
  const pct = flow.progress ? progressPercent(flow.progress.received, flow.progress.total) : null;
  const busy = flow.ui === "downloading" || flow.ui === "applying" || flow.ui === "restarting";
  // manual 模式（ADR 0008 D7）：应用内替换不可用 → 阻断层改为引导手动下载
  // （per-target minSupported 落在 manual 平台上时的诚实行为）
  const manual = info.updateMode === "manual" && !!info.downloadUrl;

  return (
    <div className="flex h-screen w-screen flex-col items-center justify-center bg-slate-950 px-6">
      {/* 顶部拖拽区（领域模式之外唯一拖拽区由本层承担，保证窗口可拖动） */}
      <div data-tauri-drag-region className="absolute inset-x-0 top-0 h-10 cursor-move" />

      <div className="flex w-full max-w-md flex-col items-center text-center">
        <div className="h-16 w-16 overflow-hidden rounded-2xl shadow-lg shadow-blue-600/30">
          <img src={brandIcon} alt="VCMI" className="h-full w-full object-cover" draggable={false} />
        </div>
        <h1 className="mt-5 text-xl font-semibold text-slate-100">{t.update.forcedTitle}</h1>
        <p className="mt-1.5 text-sm text-slate-400">{t.update.forcedDesc}</p>

        {/* 版本对比（等宽大字号） */}
        <div className="mt-6 flex items-center gap-3">
          <span className="font-mono text-2xl text-slate-500">{info.currentVersion}</span>
          <ArrowRight className="h-5 w-5 text-blue-400" />
          <span className="bg-gradient-to-r from-blue-400 to-indigo-400 bg-clip-text font-mono text-2xl font-bold text-transparent">
            {info.latestVersion}
          </span>
        </div>

        {/* 更新说明 */}
        {notes.length > 0 && (
          <ul className="mt-5 max-h-[30vh] space-y-1.5 overflow-y-auto text-left">
            {notes.map((n, i) => (
              <li key={i} className="flex items-start gap-2 text-sm text-slate-300">
                <span className="mt-1.5 h-1.5 w-1.5 flex-none rounded-full bg-blue-400/70" />
                <span className="min-w-0">{renderInline(n)}</span>
              </li>
            ))}
          </ul>
        )}

        {/* 下载进度 */}
        {flow.ui === "downloading" && flow.progress && (
          <div className="mt-6 w-full">
            <div className="h-2 w-full overflow-hidden rounded-full bg-slate-800">
              <div
                className="h-full rounded-full bg-gradient-to-r from-blue-500 to-indigo-500 transition-all duration-200"
                style={{ width: pct != null ? `${pct}%` : "35%" }}
              />
            </div>
            <div className="mt-1.5 flex justify-between text-xs text-slate-400">
              <span>{pct != null ? `${pct}%` : t.update.downloading}</span>
              <span>
                {formatBytes(flow.progress.received)} / {formatBytes(flow.progress.total)}
              </span>
            </div>
          </div>
        )}

        {/* 状态 / 错误 */}
        {flow.ui === "applying" && <p className="mt-6 text-sm text-blue-400">{t.update.applying}</p>}
        {flow.ui === "restarting" && <p className="mt-6 text-sm text-blue-400">{t.update.restarting}</p>}
        {/* ⚠️ 强制更新理应推送**含 exe** 的包（否则达不到"强制升级"目的）；
            万一推送了纯配置包，这里给出口，避免用户卡在无操作入口的界面 */}
        {flow.ui === "applied" && (
          <p className="mt-6 text-sm text-emerald-400">✓ {t.update.appliedNoRestart}</p>
        )}
        {flow.ui === "error" && (
          <p className="mt-6 max-w-md break-all text-sm text-red-400">
            {t.update.failed}
            {flow.err ? `：${flow.err}` : ""}
          </p>
        )}

        {/* 操作区：唯一主按钮 + 退出（下载中主按钮变「取消下载」） */}
        <div className="mt-8 flex w-full max-w-xs flex-col gap-2.5">
          {manual ? (
            <a
              href={info.downloadUrl}
              target="_blank"
              rel="noreferrer noopener"
              className="flex whitespace-nowrap items-center justify-center gap-2 rounded-xl bg-gradient-to-br from-blue-500 to-indigo-500 px-4 py-2.5 text-sm font-medium text-white shadow-md shadow-blue-600/30 transition-opacity hover:opacity-90"
            >
              <DownloadCloud className="h-4 w-4" />
              {t.update.openDownload}
            </a>
          ) : flow.ui === "downloading" ? (
            <button
              type="button"
              onClick={() => void flow.cancel()}
              className="whitespace-nowrap rounded-xl border border-slate-700 px-4 py-2.5 text-sm font-medium text-slate-300 transition-colors hover:bg-slate-800"
            >
              {t.update.cancelDownload}
            </button>
          ) : flow.ui === "error" ? (
            <button
              type="button"
              onClick={() => void flow.start()}
              className="flex whitespace-nowrap items-center justify-center gap-2 rounded-xl bg-gradient-to-br from-blue-500 to-indigo-500 px-4 py-2.5 text-sm font-medium text-white shadow-md shadow-blue-600/30 transition-opacity hover:opacity-90"
            >
              <RefreshCw className="h-4 w-4" />
              {t.update.retry}
            </button>
          ) : flow.ui === "downloaded" ? (
            <button
              type="button"
              disabled
              className="flex cursor-wait whitespace-nowrap items-center justify-center gap-2 rounded-xl bg-gradient-to-br from-blue-500 to-indigo-500 px-4 py-2.5 text-sm font-medium text-white opacity-80"
            >
              <DownloadCloud className="h-4 w-4 animate-pulse" />
              {t.update.applying}
            </button>
          ) : flow.ui === "applied" ? (
            <button
              type="button"
              onClick={onExit}
              className="whitespace-nowrap rounded-xl bg-gradient-to-br from-emerald-500 to-teal-500 px-4 py-2.5 text-sm font-medium text-white shadow-md shadow-emerald-600/30 transition-opacity hover:opacity-90"
            >
              {t.app.close}
            </button>
          ) : (
            <button
              type="button"
              onClick={() => void flow.start()}
              disabled={busy}
              className={cn(
                "whitespace-nowrap rounded-xl bg-gradient-to-br from-blue-500 to-indigo-500 px-4 py-2.5 text-sm font-medium text-white shadow-md shadow-blue-600/30 transition-opacity hover:opacity-90",
                busy && "cursor-not-allowed opacity-50",
              )}
            >
              {t.update.downloadNow}
            </button>
          )}
          <button
            type="button"
            onClick={onExit}
            disabled={flow.ui === "restarting" || flow.ui === "applying"}
            className={cn(
              "whitespace-nowrap rounded-xl px-4 py-2 text-sm text-slate-500 transition-colors hover:text-slate-300",
              (flow.ui === "restarting" || flow.ui === "applying") &&
                "cursor-not-allowed opacity-50",
            )}
          >
            {t.update.exitLauncher}
          </button>
        </div>
      </div>
    </div>
  );
}
