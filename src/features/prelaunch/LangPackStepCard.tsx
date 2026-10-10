import { useEffect, useState } from "react";
import { useI18n } from "@/i18n";
import { Button } from "@/shared/ui/button";
import {
  installModFromRepo,
  rescanRealMods,
  type ModRepoEntry,
} from "@/lib/tauri";
import { backendErrorText } from "@/lib/backendError";
import { vcmiLangName } from "@/lib/vcmiLangs";
import {
  syncLanguageMods,
  setSessionAutoEnable,
  setSessionSkipLangCheck,
  scanLangPack,
  resolveLangPackRepo,
  type LaunchLangModCheck,
} from "@/lib/langMods";
import { type PreLaunchStep, type StepViewProps } from "./usePreLaunchChecks";

/**
 * 「语言包」检查项 step 卡片 —— 启动前检测的其中一类（见 `docs/features/lang-mods` §10）。
 *
 * 数据来源分两层：
 * - `scan.data`（`LaunchLangModCheck`）：本地扫描结论（ok / 已装未启 / 未装），毫秒级、已先行展示；
 * - `resolve.data`（`ModRepoEntry[]`）：弹窗打开后异步补全的仓库候选，期间 `resolving` 为真。
 *
 * 三态：
 * - `installed-disabled`：已装未启 → 「启用并启动」+ 可选「以后自动启用」。
 * - `missing` + 候选：逐条「下载并启用」。
 * - `missing` + 无候选/超时：引导「前往模组管理」+ 逃生口。
 *
 * 防唠叨（会话级，不落盘）：选「跳过并启动」后本会话不再预检；勾「以后自动启用」后本会话静默启用。
 */
interface LangPackStepCardProps extends StepViewProps {
  vcmiLang: string;
  realmId?: string | null;
  scope?: string;
  realmName?: string;
  onProceed: () => void;
  onGoToMods?: () => void;
  onBusyChange?: (busy: boolean) => void;
}

export function LangPackStepCard({
  scan,
  resolve,
  resolving,
  onProceed,
  onGoToMods,
  onBusyChange,
  vcmiLang,
  realmId,
  scope,
  realmName,
}: LangPackStepCardProps) {
  const { t } = useI18n();
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [autoEnable, setAutoEnable] = useState(false);

  // 上报忙碌态给外壳（禁用关闭）。入参为稳定的 setState，不会造成重复触发。
  useEffect(() => {
    onBusyChange?.(busy);
  }, [busy, onBusyChange]);

  const check = scan.data as LaunchLangModCheck;
  const isDisabled = check.status === "installed-disabled";
  const candidates = (resolve?.data as ModRepoEntry[] | undefined) ?? undefined;
  const langLabel = vcmiLangName(vcmiLang);

  // 结论标题（本地已知，立即显示）
  const title =
    check.status === "ok"
      ? t.launch.reportLangOk.replace("{lang}", langLabel)
      : isDisabled
        ? t.launch.langModTitleDisabled.replace("{lang}", langLabel)
        : t.launch.langModTitleMissing.replace("{lang}", langLabel);

  // 「跳过并启动」：记下本会话不再预检，然后照常启动（逃生口，不算硬阻断）
  const handleSkipLaunch = () => {
    setSessionSkipLangCheck(true);
    onProceed();
  };

  const handleEnable = async () => {
    setBusy(true);
    setErr(null);
    try {
      if (autoEnable) setSessionAutoEnable(true);
      // force：用户刚点了「启用并启动」，启用是当下显式意图，必须绕过"尊重显式关闭"闸门
      await syncLanguageMods({ vcmiLang, realmId, presetId: undefined, force: true });
      onProceed();
    } catch (e) {
      setErr(backendErrorText(e));
    } finally {
      setBusy(false);
    }
  };

  const handleInstall = async (modId: string) => {
    setBusy(true);
    setErr(null);
    try {
      await installModFromRepo(modId, scope);
      // 安装后必须重扫：否则刚装的模组不在扫描缓存里，下一步 syncLanguageMods 会启用空集合
      await rescanRealMods();
      await syncLanguageMods({ vcmiLang, realmId, presetId: undefined, force: true });
      onProceed();
    } catch (e) {
      setErr(backendErrorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div>
      {/* 本检查项结论（警告色） */}
      <div className="mb-3 text-sm font-semibold text-amber-300">{title}</div>

      {/* ① 检测到了什么（本地已知，立即显示） */}
      <div className="mb-3 space-y-1 text-sm text-slate-300">
        <div>{t.launch.langModCurLang.replace("{lang}", langLabel)}</div>
        {realmName && <div>{t.launch.langModCurRealm.replace("{name}", realmName)}</div>}
        {/* 已装未启时，明确告诉用户「将启用哪个语言包」（名称（id）） */}
        {isDisabled && check.matchingLabels.length > 0 && (
          <div>{t.launch.langModDetected.replace("{mods}", check.matchingLabels.join("、"))}</div>
        )}
      </div>

      {/* ② 不管会怎样 + ③ 建议怎么做 */}
      <div className="mb-4 space-y-2 text-sm leading-relaxed text-slate-300">
        <div>{t.launch.langModConsequence}</div>
        {isDisabled ? (
          <div className="text-slate-400">{t.launch.langModSuggestEnable}</div>
        ) : (
          <div>{t.launch.langModNotInstalled}</div>
        )}
      </div>

      {/* 方案区：未装时由 resolve 异步补全（报告先行，联网不阻塞） */}
      {check.status === "missing" && (
        <div className="mb-4">
          {resolving ? (
            <div className="flex items-center gap-2 text-sm text-slate-300">
              <svg
                className="h-4 w-4 animate-spin text-sky-300"
                viewBox="0 0 24 24"
                fill="none"
                aria-hidden="true"
              >
                <circle cx="12" cy="12" r="9" stroke="currentColor" strokeWidth="3" opacity="0.3" />
                <path
                  d="M21 12a9 9 0 0 0-9-9"
                  stroke="currentColor"
                  strokeWidth="3"
                  strokeLinecap="round"
                />
              </svg>
              {t.launch.resolvingRepo}
            </div>
          ) : candidates && candidates.length > 0 ? (
            <>
              <div className="mb-2 text-sm text-slate-300">{t.launch.langModRepoHint}</div>
              <div className="max-h-[180px] space-y-2 overflow-auto">
                {candidates.map((c) => (
                  <div
                    key={c.id}
                    className="flex items-center justify-between gap-3 rounded-lg border border-slate-700 bg-slate-800/60 px-3 py-2"
                  >
                    <div className="min-w-0">
                      <div className="truncate text-sm text-slate-100">{c.name || c.id}</div>
                      <div className="truncate text-xs text-slate-400">{c.id}</div>
                    </div>
                    <Button
                      size="sm"
                      variant="success"
                      disabled={busy}
                      onClick={() => handleInstall(c.id)}
                    >
                      {t.launch.langModInstall}
                    </Button>
                  </div>
                ))}
              </div>
            </>
          ) : (
            <div className="text-sm leading-relaxed text-slate-300">
              {t.launch.langModRepoUnreachable.replace("{lang}", langLabel)}
            </div>
          )}
        </div>
      )}

      {err && (
        <div className="mb-3 rounded-lg bg-red-500/10 px-3 py-2 text-xs text-red-300">{err}</div>
      )}

      {/* 会话级「以后自动启用」：勾选后本会话内静默启用、不再预检 */}
      {isDisabled && (
        <label className="mb-4 flex items-center gap-2 text-xs text-slate-400">
          <input
            type="checkbox"
            checked={autoEnable}
            disabled={busy}
            onChange={(e) => setAutoEnable(e.target.checked)}
            className="h-3.5 w-3.5 accent-green-500"
          />
          <span>{t.launch.langModAutoEnable.replace("{lang}", langLabel)}</span>
        </label>
      )}

      <div className="flex items-center justify-end gap-3">
        {busy && <span className="text-xs text-slate-400">{t.launch.langModInstalling}</span>}
        {/* 未装且无候选/超时：给「前往模组管理」的引导出口 */}
        {check.status === "missing" && !(candidates && candidates.length > 0) && onGoToMods && (
          <Button size="sm" variant="outline" disabled={busy} onClick={onGoToMods}>
            {realmId ? t.launch.langModGoToModsRealm : t.launch.langModGoToMods}
          </Button>
        )}
        {/* 逃生口：始终可跳过启动——本检查是建议性提示，不是硬阻断。
            警告色（amber）刻意与「启用并启动」的正向绿区分，提示这是"带后果的跳过"。 */}
        <Button size="sm" variant="warning" disabled={busy} onClick={handleSkipLaunch}>
          {t.launch.preLaunchSkipAndLaunch}
        </Button>
        {isDisabled && (
          <Button size="sm" variant="success" disabled={busy} onClick={handleEnable}>
            {t.launch.langModEnable}
          </Button>
        )}
      </div>
    </div>
  );
}

/**
 * 构造语言包检查项（`PreLaunchStep`）。`label` 由调用方用 i18n 提供（本函数非组件，无法 `useI18n`）。
 * 业务动作（onProceed / onGoToMods / onBusyChange）在构造时闭包进 `Component`，外壳只注入 scan/resolve 状态。
 */
export function makeLangPackStep(args: {
  label: string;
  vcmiLang: string;
  realmId?: string | null;
  scope?: string;
  realmName?: string;
  onProceed: () => void;
  onGoToMods?: () => void;
  onBusyChange?: (busy: boolean) => void;
}): PreLaunchStep {
  return {
    id: "lang-pack",
    label: args.label,
    Component: (props) => (
      <LangPackStepCard
        {...props}
        vcmiLang={args.vcmiLang}
        realmId={args.realmId}
        scope={args.scope}
        realmName={args.realmName}
        onProceed={args.onProceed}
        onGoToMods={args.onGoToMods}
        onBusyChange={args.onBusyChange}
      />
    ),
    scan: () =>
      scanLangPack({ vcmiLang: args.vcmiLang, realmId: args.realmId }).then((check) => ({
        // 语言包问题都是"建议性"，severity 统一 warn（不阻断，始终给逃生口）
        severity: check.status === "ok" ? "ok" : "warn",
        title: "",
        data: check,
      })),
    resolve: async (scan) => {
      const candidates = await resolveLangPackRepo(args.vcmiLang);
      return { severity: scan.severity, data: candidates };
    },
  };
}
