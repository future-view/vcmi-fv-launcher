import { useEffect, useRef, useState } from "react";
import { Loader2 } from "lucide-react";
import { useI18n } from "@/i18n";
import type { Realm } from "@/lib/realm/types";
import { ensureRealmAssets } from "@/lib/modcache/realmAssets";
import { cancelRealmCacheBuild } from "@/lib/tauri";

interface Props {
  realm: Realm;
  realmId: string;
  /** 素材就绪后要进入的页面 */
  target: "info" | "showcase";
  onDone: (target: "info" | "showcase") => void;
  onBack: () => void;
}

/**
 * 领域素材准备页（S1）。
 *
 * **只在"当前领域所需素材未就绪"时才会被看到**：`ensureRealmAssets` 先做抽样就绪检查，
 * 已就绪时几乎立即回调 `onDone` —— 用户感知为"直接进入"，不会闪进度条。
 *
 * 进度条读的是**本领域专属**通道（后端 `BuildSlot::realm` + `realm-cache-build-progress`），
 * 与启动时的全局后台构建互不干扰。
 *
 * 「返回」= 取消加急构建（已分发的 mod 会跑完，不影响已完成的缓存）→ 回领域主界面。
 * 用户也可以在原目标页正常使用（缺图处由 `HdImg` 占位兜底）。
 */
export function RealmAssetsPage({ realm, realmId, target, onDone, onBack }: Props) {
  const { t } = useI18n();
  const [progress, setProgress] = useState<{ done: number; total: number } | null>(null);
  const [failed, setFailed] = useState(false);
  const aliveRef = useRef(true);
  // 回调放 ref：父组件每次渲染都会传新函数，直接进依赖数组会让 effect 反复重跑（甚至死循环）
  const onDoneRef = useRef(onDone);
  onDoneRef.current = onDone;

  useEffect(() => {
    aliveRef.current = true;
    void (async () => {
      const res = await ensureRealmAssets(realm, realmId, (done, total) => {
        if (aliveRef.current && total > 0) setProgress({ done, total });
      });
      if (!aliveRef.current) return;
      if (res.missing.length === 0) {
        onDoneRef.current(target);
      } else {
        // 仍缺个别 mod（例如素材本身缺失）：不把用户卡在这里，给提示后自动放行
        setFailed(true);
        window.setTimeout(() => {
          if (aliveRef.current) onDoneRef.current(target);
        }, 1800);
      }
    })();
    return () => {
      aliveRef.current = false;
    };
    // ⚠️ 只依赖 realmId/target：realm 对象与回调都可能是新引用
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [realmId, target]);

  const pct =
    progress && progress.total > 0
      ? Math.max(2, Math.min(100, Math.round((progress.done / progress.total) * 100)))
      : 0;

  const handleBack = () => {
    void cancelRealmCacheBuild();
    onBack();
  };

  return (
    <div className="flex h-full items-center justify-center px-8">
      <div className="w-full max-w-[440px] rounded-2xl border border-slate-700/60 bg-slate-900/70 p-7 text-center backdrop-blur-sm">
        <h2 className="text-lg font-bold text-white">{t.realm.assetsPreparing}</h2>
        <p className="mt-2 text-xs leading-relaxed text-slate-400">{t.realm.assetsHint}</p>

        <div className="mt-6 h-2 w-full overflow-hidden rounded-full bg-slate-800">
          <div
            className="h-full rounded-full bg-[var(--gold,#F5C56B)] transition-all duration-300"
            style={{ width: `${pct}%` }}
          />
        </div>

        <div className="mt-2 flex items-center justify-center gap-2 text-xs text-slate-400">
          {pct > 0 ? null : <Loader2 className="h-3.5 w-3.5 animate-spin" />}
          <span>
            {progress && progress.total > 0
              ? `${progress.done} / ${progress.total}`
              : t.realm.assetsBuilding}
          </span>
        </div>

        {failed && (
          <div className="mt-3 text-xs text-amber-300/90">{t.realm.assetsFailed}</div>
        )}

        <button
          type="button"
          onClick={handleBack}
          className="mt-7 rounded-lg border border-slate-600/70 px-4 py-1.5 text-sm text-slate-200 hover:bg-slate-800/70"
        >
          {t.realm.assetsBack}
        </button>
      </div>
    </div>
  );
}
