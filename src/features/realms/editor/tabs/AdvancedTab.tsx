import { cn } from "@/lib/utils";
import type { RealmDirs, RealmDirScope } from "@/lib/realm/types";
import type { LangDict } from "@/i18n/types";

export interface AdvancedTabProps {
  t: LangDict;
  borrowed: Set<string>;
  bundled: Set<string>;
  dirs: RealmDirs;
  setDirs: React.Dispatch<React.SetStateAction<RealmDirs>>;
  setShare: (k: keyof NonNullable<RealmDirs["configShare"]>, v: boolean) => void;
  /** 领域信息页是否默认展开原版内容（写入 `realm.json` 的 `info.showCore`） */
  infoShowCore: boolean;
  setInfoShowCore: React.Dispatch<React.SetStateAction<boolean>>;
}

export function AdvancedTab(props: AdvancedTabProps) {
  const { t, borrowed, bundled, dirs, setDirs, setShare, infoShowCore, setInfoShowCore } = props;
  return (
    <>

      {/* 领域信息页展示偏好（§9.3）：只影响信息页的默认展开状态，不影响游戏内容 */}
      <div className="rounded-lg border border-slate-800 bg-slate-900/40 p-3">
        <div className="mb-2 text-sm font-medium text-foreground">{t.realm.realmInfo}</div>
        <label className="flex cursor-pointer items-center gap-2 text-sm text-foreground">
          <input
            type="checkbox"
            checked={infoShowCore}
            onChange={(e) => setInfoShowCore(e.target.checked)}
            className="h-4 w-4 accent-blue-500"
          />
          {t.realm.infoShowCore}
        </label>
        <p className="mt-1 text-xs text-muted">{t.realm.infoShowCoreNote}</p>
      </div>

      {/* 地图阀门与整合地图已移至独立的「集成地图」标签页（MapsTab） */}

      {/* 数据隔离（§3.2.1 / B7）：5 个 dirs.json 真实 key 的归属 */}
      <div className="rounded-lg border border-slate-800 bg-slate-900/40 p-3">
        <div className="mb-2 text-sm font-medium text-foreground">{t.realm.dataIsolation}</div>
        <div className="space-y-1.5">
          {(
            [
              ["dataPath", t.realm.dirData],
              ["cachePath", t.realm.dirCache],
              ["configPath", t.realm.dirConfig],
              ["logsPath", t.realm.dirLogs],
              ["savePath", t.realm.dirSave],
            ] as const
          ).map(([k, label]) => (
            <div key={k} className="flex items-center justify-between gap-2">
              <span className="text-sm text-slate-300">{label}</span>
              <div className="flex shrink-0 overflow-hidden rounded-lg border border-slate-700 text-xs">
                <button
                  type="button"
                  onClick={() => setDirs((d) => ({ ...d, [k]: "realm" as RealmDirScope }))}
                  className={cn(
                    "px-2.5 py-1",
                    (dirs[k] ?? "realm") === "realm"
                      ? "bg-blue-500/90 text-white"
                      : "text-slate-300 hover:bg-slate-800/50",
                  )}
                >
                  {t.realm.scopeRealm}
                </button>
                <button
                  type="button"
                  onClick={() => setDirs((d) => ({ ...d, [k]: "global" as RealmDirScope }))}
                  className={cn(
                    "px-2.5 py-1",
                    (dirs[k] ?? "realm") === "global"
                      ? "bg-blue-500/90 text-white"
                      : "text-slate-300 hover:bg-slate-800/50",
                  )}
                >
                  {t.realm.scopeGlobal}
                </button>
              </div>
            </div>
          ))}
        </div>
        {dirs.configPath === "realm" && (
          <div className="mt-3 border-t border-slate-800 pt-3">
            <div className="mb-2 text-xs font-medium text-slate-400">{t.realm.configShareTitle}</div>
            <div className="space-y-1.5">
              {(
                [
                  ["settings", t.realm.cfgSettings],
                  ["keybinds", t.realm.cfgKeybinds],
                  ["persistent", t.realm.cfgPersistent],
                  ["mapeditor", t.realm.cfgMapeditor],
                ] as const
              ).map(([k, label]) => {
                const on = dirs.configShare?.[k] ?? true;
                return (
                  <div key={k} className="flex items-center justify-between gap-2">
                    <span className="text-sm text-slate-300">{label}</span>
                    <div className="flex shrink-0 overflow-hidden rounded-lg border border-slate-700 text-xs">
                      <button
                        type="button"
                        onClick={() => setShare(k, true)}
                        className={cn(
                          "px-2.5 py-1",
                          on
                            ? "bg-blue-500/90 text-white"
                            : "text-slate-300 hover:bg-slate-800/50",
                        )}
                      >
                        {t.realm.shareGlobal}
                      </button>
                      <button
                        type="button"
                        onClick={() => setShare(k, false)}
                        className={cn(
                          "px-2.5 py-1",
                          !on
                            ? "bg-blue-500/90 text-white"
                            : "text-slate-300 hover:bg-slate-800/50",
                        )}
                      >
                        {t.realm.isolateRealm}
                      </button>
                    </div>
                  </div>
                );
              })}
            </div>
            <p className="mt-1.5 text-xs text-muted">{t.realm.configShareHint}</p>
          </div>
        )}
        {dirs.dataPath === "global" && (
          <p className="mt-2 text-xs text-amber-300/80">{t.realm.isolationTip}</p>
        )}
      </div>

      {/* 整合数据（模组）：精简为借用/整合计数报告；地图管理已移至「集成地图」标签页 */}
      <div className="rounded-lg border border-slate-800 bg-slate-900/40 p-3">
        <div className="mb-2 text-sm font-medium text-foreground">{t.realm.integratedData}</div>
        <p className="text-xs text-muted">
          {t.realm.advancedModsSummary
            .replace("{borrowed}", String(borrowed.size))
            .replace("{bundled}", String(bundled.size))}
        </p>
      </div>
    </>
  );
}
