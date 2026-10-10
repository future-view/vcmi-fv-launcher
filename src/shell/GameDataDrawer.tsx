import { useEffect, useState } from "react";
import { useI18n } from "@/i18n";
import { Button } from "@/shared/ui/button";
import { cn } from "@/lib/utils";
import {
  getRealDirs,
  getCompileInfo,
  gamedataHealthCheck,
  openInExplorer,
  type VcmiDirs,
  type CompileInfo,
  type GamedataHealth,
} from "@/lib/tauri";

interface GameDataDrawerProps {
  onClose: () => void;
}

export function GameDataDrawer({ onClose }: GameDataDrawerProps) {
  const { t } = useI18n();
  const [dirs, setDirs] = useState<VcmiDirs | null>(null);
  const [info, setInfo] = useState<CompileInfo | null>(null);
  // 数据完整性体检：`undefined` = 检查中，`null` = 无法检查（非 Tauri / 游戏目录未就绪）
  const [health, setHealth] = useState<GamedataHealth | null | undefined>(undefined);

  useEffect(() => {
    getRealDirs().then(setDirs).catch(() => {});
    getCompileInfo().then(setInfo).catch(() => {});
    gamedataHealthCheck().then(setHealth).catch(() => setHealth(null));
  }, []);

  const runHealthCheck = () => {
    setHealth(undefined);
    gamedataHealthCheck().then(setHealth).catch(() => setHealth(null));
  };

  const fallback: VcmiDirs = {
    ready: false,
    exeDir: "—",
    gameDir: "—",
    userDir: "—",
    userConfigDir: "—",
    logsDir: "—",
    userModsDir: "—",
  };
  const d = dirs ?? fallback;

  return (
    <>
      <div className="fixed inset-0 z-[70] bg-black/55 backdrop-blur-sm" onClick={onClose} />
      <aside className="fixed inset-y-0 right-0 z-[71] flex h-screen w-[440px] max-w-[92vw] flex-col border-l border-slate-700/40 bg-slate-950/95 backdrop-blur-xl shadow-2xl">
        <header className="flex items-center gap-3 border-b border-slate-800 p-5">
          <div className="grid h-11 w-11 flex-none place-items-center rounded-xl bg-slate-800 text-xl">🖥</div>
          <div className="min-w-0 flex-1">
            <div className="text-lg font-semibold">{t.data.title}</div>
            <div className="text-xs text-slate-400">{t.data.sub}</div>
          </div>
          <Button variant="ghost" size="sm" onClick={onClose}>
            {t.common.close} ✕
          </Button>
        </header>
        <div className="flex-1 space-y-6 overflow-y-auto p-5">
          {/* 编译信息 */}
          <section>
            <h3 className="mb-2 border-l-2 border-sky-400 pl-2 text-[13px] font-semibold text-slate-300">
              {t.data.compileSection}
            </h3>
            <div className="space-y-2.5">
              <DrawerRow
                label={t.data.gameVersion}
                value={info?.vcmiVersion ?? t.common.loading}
              />
              {/* 内置核心 mod 的资产版本：与引擎版本不是一回事（实测 1.5 vs 1.7.x），
                  分开显示可避免把 `1.5` 误当引擎版本去判断模组兼容性。 */}
              <DrawerRow
                label={t.data.coreModVersion}
                value={info?.coreModVersion ?? "—"}
              />
              <DrawerRow
                label={t.data.os}
                value={info?.osVersion ?? t.common.loading}
              />
            </div>
          </section>

          {/* 数据目录 */}
          <section>
            <h3 className="mb-2 border-l-2 border-sky-400 pl-2 text-[13px] font-semibold text-slate-300">
              {t.data.dataSection}
            </h3>
            <div className="space-y-2.5">
              <DrawerRow
                label={t.data.gameDataDir}
                value={d.gameDir}
                onOpen={() => openInExplorer(d.gameDir)}
              />
              <DrawerRow
                label={t.data.userDataDir}
                value={d.userDir}
                onOpen={() => openInExplorer(d.userDir)}
              />
              <DrawerRow
                label={t.data.logsDir}
                value={d.logsDir}
                onOpen={() => openInExplorer(d.logsDir)}
              />
              <DrawerRow
                label={t.data.userConfigDir}
                value={d.userConfigDir}
                onOpen={() => openInExplorer(d.userConfigDir)}
              />
            </div>
          </section>

          {/* 数据完整性体检（只读）：判定依据见后端 `vcmi::health`（lod 条目 / 核心 mod / 资源容器） */}
          <section>
            <div className="mb-2 flex items-center gap-2">
              <h3 className="border-l-2 border-sky-400 pl-2 text-[13px] font-semibold text-slate-300">
                {t.data.healthSection}
              </h3>
              {health && (
                <span
                  className={cn(
                    "rounded px-1.5 py-0.5 text-[10px]",
                    health.ok ? "bg-emerald-500/15 text-emerald-300" : "bg-red-500/15 text-red-300",
                  )}
                >
                  {health.ok ? t.data.healthOk : t.data.healthBad}
                </span>
              )}
              <button
                type="button"
                onClick={runHealthCheck}
                disabled={health === undefined}
                className="ml-auto text-[11px] text-sky-400 hover:text-sky-300 disabled:opacity-40"
              >
                {health === undefined ? t.data.healthChecking : t.data.healthCheck}
              </button>
            </div>
            <p className="mb-2 text-[11px] text-slate-500">{t.data.healthSub}</p>
            {health === undefined && (
              <div className="rounded-xl border border-slate-700/40 bg-slate-900/40 p-3 text-xs text-slate-500">
                {t.data.healthChecking}
              </div>
            )}
            {health === null && (
              <div className="rounded-xl border border-slate-700/40 bg-slate-900/40 p-3 text-xs text-slate-500">
                {t.data.healthUnavailable}
              </div>
            )}
            {health && (
              <div className="space-y-2">
                {health.items.map((it) => (
                  <div
                    key={it.id}
                    className={cn(
                      "flex items-center gap-3 rounded-xl border p-3",
                      it.ok
                        ? "border-slate-700/40 bg-slate-900/40"
                        : it.level === "fatal"
                          ? "border-red-500/40 bg-red-950/30"
                          : "border-amber-500/40 bg-amber-950/20",
                    )}
                  >
                    <div className="w-[104px] flex-none text-xs text-slate-500">
                      {t.data.healthItems[it.id] ?? it.id}
                    </div>
                    <div
                      className="min-w-0 flex-1 truncate font-mono text-[11.5px] text-slate-300"
                      title={it.detail}
                    >
                      {it.detail}
                    </div>
                    <span
                      className={cn(
                        "flex-none text-[11px]",
                        it.ok ? "text-emerald-400" : it.level === "fatal" ? "text-red-400" : "text-amber-400",
                      )}
                    >
                      {it.ok ? "✓" : it.level === "fatal" ? t.data.healthFatal : t.data.healthWarn}
                    </span>
                  </div>
                ))}
                {health.roots.length > 0 && (
                  <div className="truncate text-[11px] text-slate-500" title={health.roots.join("\n")}>
                    {t.data.healthRoots}: {health.roots.join(" · ")}
                  </div>
                )}
              </div>
            )}
          </section>
        </div>
      </aside>
    </>
  );
}

function DrawerRow({
  label,
  value,
  onOpen,
}: {
  label: string;
  value: string;
  onOpen?: () => void;
}) {
  const { t } = useI18n();
  return (
    <div className="flex items-center gap-3 rounded-xl border border-slate-700/40 bg-slate-900/40 p-3">
      <div className="w-[96px] flex-none text-xs text-slate-500">{label}</div>
      <div className="min-w-0 flex-1 truncate font-mono text-[12.5px] text-slate-200" title={value}>
        {value}
      </div>
      {onOpen && (
        <Button variant="outline" size="sm" onClick={onOpen}>
          {t.launch.open}
        </Button>
      )}
    </div>
  );
}
