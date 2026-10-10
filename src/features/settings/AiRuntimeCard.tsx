//! 设置页「FV启动器」标签下的**外挂运行时组件**卡片（首个组件 = AI 运行库）。
//!
//! 三种状态：
//! - 已安装（文件齐备 + sha256 全中）→ 显示版本 + 「移除」
//! - 未安装且配了远程包 → 「安装」+ 进度条 + 「取消」
//! - 未安装且**没配**远程包 → 只显示提示（无法在线安装，只能由发行方内置）
//!
//! ⚠️ 组件**永远是可选项**：未安装时 AI 高清增强自动回退传统算法，绝不阻塞其它功能。
//! ⚠️ 非 Tauri（浏览器 mock）时 `fetchRuntimeStatus` 返回 null → 整卡不渲染。

import { useEffect, useRef, useState } from "react";
import { Download, Loader2, Trash2 } from "lucide-react";

import { useI18n } from "@/i18n";
import {
  AI_RUNTIME_ID,
  cancelRuntime,
  fetchRuntimeStatus,
  installRuntime,
  onRuntimeProgress,
  uninstallRuntime,
  type RuntimeStatus,
} from "@/lib/tauri";
import { formatBytes, progressPercent } from "@/lib/updater";
import { backendErrorText } from "@/lib/backendError";

export function AiRuntimeCard() {
  const { t } = useI18n();
  const [st, setSt] = useState<RuntimeStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [received, setReceived] = useState(0);
  const [total, setTotal] = useState(0);
  const [error, setError] = useState<string | null>(null);
  // 卸载确认：二次点击才真正执行（避免误删）
  const confirmRemoveRef = useRef(false);

  useEffect(() => {
    let alive = true;
    let unsub: (() => void) | undefined;
    void (async () => {
      const s = await fetchRuntimeStatus(AI_RUNTIME_ID);
      if (alive) setSt(s);
      unsub = await onRuntimeProgress((p) => {
        if (p.id !== AI_RUNTIME_ID) return;
        setReceived(p.received);
        setTotal(p.total);
      });
    })();
    return () => {
      alive = false;
      unsub?.();
    };
  }, []);

  const doInstall = async () => {
    setBusy(true);
    setError(null);
    setReceived(0);
    setTotal(0);
    try {
      setSt(await installRuntime(AI_RUNTIME_ID));
    } catch (e) {
      setError(backendErrorText(e));
    } finally {
      setBusy(false);
    }
  };

  const doRemove = async () => {
    if (!confirmRemoveRef.current) {
      confirmRemoveRef.current = true;
      window.setTimeout(() => {
        confirmRemoveRef.current = false;
      }, 3000);
      return;
    }
    confirmRemoveRef.current = false;
    setBusy(true);
    setError(null);
    try {
      setSt(await uninstallRuntime(AI_RUNTIME_ID));
    } catch (e) {
      setError(backendErrorText(e));
    } finally {
      setBusy(false);
    }
  };

  // 未配置该组件 / 浏览器模式 → 不渲染
  if (!st) return null;

  const pct = progressPercent(received, total);
  const stateText = st.installed
    ? st.version
      ? t.runtime.installed.replace("{version}", st.version)
      : t.runtime.installedNoVer
    : st.has_package
      ? t.runtime.missing
      : t.runtime.noPackage;

  return (
    <div className="rounded-lg border border-slate-700/60 bg-slate-800/40 p-3">
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <div className="whitespace-nowrap text-sm font-medium text-slate-100">
            {t.runtime.title}
          </div>
          <div className="mt-0.5 text-xs text-slate-400">{stateText}</div>
        </div>

        <div className="flex shrink-0 items-center gap-2">
          {st.installed ? (
            <button
              type="button"
              onClick={doRemove}
              disabled={busy}
              className="inline-flex items-center gap-1 whitespace-nowrap rounded border border-slate-600 px-2 py-1 text-xs text-slate-300 hover:border-rose-500/60 hover:text-rose-300 disabled:opacity-50"
            >
              <Trash2 className="h-3.5 w-3.5" />
              {t.runtime.remove}
            </button>
          ) : busy ? (
            <button
              type="button"
              onClick={() => void cancelRuntime(AI_RUNTIME_ID)}
              className="inline-flex items-center gap-1 whitespace-nowrap rounded border border-slate-600 px-2 py-1 text-xs text-slate-300 hover:border-amber-500/60"
            >
              <Loader2 className="h-3.5 w-3.5 animate-spin" />
              {t.runtime.cancel}
            </button>
          ) : st.has_package ? (
            <button
              type="button"
              onClick={doInstall}
              className="inline-flex items-center gap-1 whitespace-nowrap rounded border border-emerald-600/60 bg-emerald-600/15 px-2 py-1 text-xs text-emerald-300 hover:bg-emerald-600/25"
            >
              <Download className="h-3.5 w-3.5" />
              {t.runtime.install}
            </button>
          ) : null}
        </div>
      </div>

      {busy && (
        <div className="mt-2">
          <div className="h-1.5 w-full overflow-hidden rounded bg-slate-700">
            <div
              className="h-full bg-emerald-500 transition-all"
              style={{ width: `${pct ?? 0}%` }}
            />
          </div>
          <div className="mt-1 text-xs text-slate-400">
            {t.runtime.installing}
            {received > 0 && ` ${formatBytes(received)}${total > 0 ? ` / ${formatBytes(total)}` : ""}`}
          </div>
        </div>
      )}

      <div className="mt-2 text-xs leading-relaxed text-slate-500">{t.runtime.hint}</div>
      {error && (
        <div className="mt-1 text-xs text-rose-400">
          {t.runtime.failed.replace("{msg}", error)}
        </div>
      )}
    </div>
  );
}
