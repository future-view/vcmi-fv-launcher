import { useMemo, useState } from "react";
import { AlertTriangle } from "lucide-react";
import { useI18n } from "@/i18n";
import type { LangDict } from "@/i18n/types";
import { realmSelfHeal, repairGameMaps, type RealmSelfCheck } from "@/lib/tauri";
import { backendErrorText } from "@/lib/backendError";
import { refreshRealms } from "@/lib/realm/store";

interface Props {
  report: RealmSelfCheck;
  onClose: () => void;
}

/** 各 state 的诊断文案。**后端只回 status 枚举**，UI 文本一律走 i18n——
 *  后端 `message` 是中文硬编码，直接用会让英文界面露出中文。 */
function statusMessage(t: LangDict, status: string): string {
  switch (status) {
    case "realm_missing":
      return t.realm.selfCheckMsgRealmMissing;
    case "dirs_mismatch":
      return t.realm.selfCheckMsgDirsMismatch;
    case "active_mismatch":
      return t.realm.selfCheckMsgActiveMismatch;
    case "maps_orphan":
      return t.realm.selfCheckMsgMapsOrphan;
    case "maps_dangling":
      return t.realm.selfCheckMsgMapsDangling;
    case "maps_realboth":
      return t.realm.selfCheckMsgMapsRealboth;
    default:
      return t.realm.selfCheckDesc;
  }
}

/** 修复动作按钮文案（heal=自动自愈；其余为地图阀门的人工修复动作）。 */
function actionLabel(t: LangDict, action: string): string {
  switch (action) {
    case "unlink":
      return t.realm.selfCheckActionUnlink;
    case "merge":
      return t.realm.selfCheckActionMerge;
    case "revert":
      return t.realm.selfCheckActionRevert;
    default:
      return t.realm.selfCheckFix;
  }
}

/**
 * 崩溃自检（§4.5）弹窗：启动后若检测到领域状态残留/不一致，提示玩家处理。
 * 只在**需要玩家决策**时才出现——可安全自愈的 `maps_orphan` 已在 `App.tsx` 静默修复。
 * 按钮由后端报告的 `actions` 驱动：heal 走 `realm_self_heal`，
 * unlink / merge / revert 走 `repair_game_maps`（涉及删联接或搬动地图，必须玩家显式触发）。
 */
export function RealmSelfCheckDialog({ report, onClose }: Props) {
  const { t } = useI18n();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  // 无 actions 的老报告按 fixable 兜底为 heal，保证契约演进不丢按钮
  const actions = useMemo(
    () => (report.actions?.length ? report.actions : report.fixable ? ["heal"] : []),
    [report.actions, report.fixable],
  );
  const message = useMemo(() => statusMessage(t, report.status), [t, report.status]);

  const runAction = async (action: string) => {
    setBusy(true);
    setError("");
    try {
      if (action === "heal") {
        await realmSelfHeal();
      } else {
        const res = await repairGameMaps(action);
        if (!res.ok) {
          setError(backendErrorText(res.error) || t.realm.selfCheckFailed);
          setBusy(false);
          return;
        }
      }
      await refreshRealms();
      onClose();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="fixed inset-0 z-[120] flex items-center justify-center bg-black/60 p-4 backdrop-blur-sm">
      <div className="w-full max-w-md rounded-xl border border-amber-500/40 bg-slate-900 p-5 shadow-2xl">
        <div className="mb-3 flex items-start gap-2.5">
          <AlertTriangle className="mt-0.5 h-5 w-5 shrink-0 text-amber-400" />
          <div className="min-w-0">
            <h3 className="text-base font-semibold text-slate-100">{t.realm.selfCheckTitle}</h3>
            <p className="mt-1 text-sm leading-relaxed text-slate-400">{message}</p>
            {error && <p className="mt-2 text-xs leading-relaxed text-red-400">{error}</p>}
          </div>
        </div>
        <div className="flex flex-wrap justify-end gap-2">
          <button
            type="button"
            onClick={onClose}
            className="whitespace-nowrap rounded-lg border border-slate-700/60 bg-slate-800/60 px-4 py-2 text-sm font-medium text-slate-300 hover:bg-slate-800"
          >
            {t.realm.selfCheckIgnore}
          </button>
          {actions.map((a, i) => (
            <button
              key={a}
              type="button"
              onClick={() => runAction(a)}
              disabled={busy}
              className={
                i === actions.length - 1
                  ? "whitespace-nowrap rounded-lg bg-indigo-500/90 px-4 py-2 text-sm font-medium text-white hover:bg-indigo-500 disabled:opacity-50"
                  : "whitespace-nowrap rounded-lg border border-slate-700/60 bg-slate-800/60 px-4 py-2 text-sm font-medium text-slate-300 hover:bg-slate-800 disabled:opacity-50"
              }
            >
              {actionLabel(t, a)}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}
