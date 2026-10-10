/**
 * 模组管理页详情抽屉（P10）：行点击统一查看模组信息——已安装与可下载同一入口，
 * 形态对齐官方 qt 启动器的仓库详情面板（名称/类型/版本/描述/作者/大小/stars/仓库地址）。
 *
 * 外壳沿用 DetailDrawer 的交互约定：点蒙层关闭、右侧滑出、z-[150/151]（压在窗口控件之上）。
 * 动作（安装/更新/打开目录/删除）仍留在管理页行操作列——本抽屉只呈现信息。
 */
import { useI18n } from "@/i18n";
import { cn } from "@/lib/utils";
import { changelogNewest, formatDownloadSize } from "@/lib/modRepo";
import { toMarkdown } from "@/lib/html";
import type { ModRepoEntry } from "@/lib/tauri";
import { Markdown } from "@/shared/Markdown";
import type { ModQueueItem, Row } from "./ModManagerPanel";

interface ModRowDrawerProps {
  row: Row | null;
  /** 仓库条目（可下载/可更新行有；纯已装且索引未覆盖时 null） */
  entry: ModRepoEntry | null;
  queueItem?: ModQueueItem;
  onClose: () => void;
}

/** 信息行：左标签右值（值为空返回 null 不占行） */
function Field({ k, v, accent }: { k: string; v?: string | null; accent?: boolean }) {
  if (!v) return null;
  return (
    <div className="flex items-start gap-3 py-1 text-sm">
      <span className="w-24 shrink-0 text-xs leading-6 text-slate-500">{k}</span>
      <span className={cn("min-w-0 flex-1 break-words leading-6", accent ? "font-semibold text-amber-300" : "text-slate-200")}>
        {v}
      </span>
    </div>
  );
}

export function ModRowDrawer({ row, entry, queueItem, onClose }: ModRowDrawerProps) {
  const { t } = useI18n();
  const m = t.modsManage;
  if (!row) return null;
  const statusText =
    row.status === "updatable" ? m.stateUpdatable : row.status === "downloadable" ? m.stateDownloadable : m.stateInstalled;
  const locationText =
    row.location === "user"
      ? m.locationUser
      : row.location === "game"
        ? m.locationGame
        : row.location === "realm"
          ? m.locationRealm
          : null;
  const desc = row.desc || entry?.description || "";
  const author = row.author || entry?.author || "";
  const size = entry ? formatDownloadSize(entry.downloadSize) : "";
  const stars = typeof entry?.githubStars === "number" ? String(entry.githubStars) : "";
  /** 更新日志（索引 changelog 版本倒序，最多 15 版）——「可更新」行的核心决策信息（新版改了什么）。 */
  const logs = changelogNewest(entry?.changelog, 15);

  return (
    <>
      <div className="fixed inset-0 z-[150] bg-black/55 backdrop-blur-sm" onClick={onClose} />
      <aside className="fixed bottom-0 right-0 top-0 z-[151] flex w-[440px] max-w-[92vw] flex-col border-l border-slate-700/40 bg-slate-950/95 backdrop-blur-xl">
        <header className="flex items-center gap-3 border-b border-slate-800 p-5">
          <div className="min-w-0 flex-1">
            <div className="truncate text-lg font-semibold text-white">{row.name}</div>
            <div className="mt-0.5 flex items-center gap-2 text-xs text-slate-400">
              {row.type && <span>{t.mods.typeLabels[row.type] ?? row.type}</span>}
              <span
                className={cn(
                  "rounded-full border px-2 py-0.5",
                  row.status === "updatable"
                    ? "border-amber-500/50 bg-amber-500/10 text-amber-300"
                    : row.status === "downloadable"
                      ? "border-sky-500/50 bg-sky-500/10 text-sky-300"
                      : "border-slate-600 text-slate-400",
                )}
              >
                {statusText}
                {queueItem ? ` · ${queueItem.state === "running" ? m.qRunning : queueItem.state === "queued" ? m.qQueued : queueItem.state === "done" ? m.qDone : m.qFail}` : ""}
              </span>
            </div>
          </div>
          <button onClick={onClose} className="grid h-8 w-8 flex-none place-items-center rounded-lg text-slate-500 hover:bg-slate-800 hover:text-white">
            ✕
          </button>
        </header>

        <div className="min-h-0 flex-1 overflow-y-auto p-5">
          {/* 版本与安装信息 */}
          <h3 className="mb-1 text-xs font-semibold uppercase tracking-wider text-slate-500">{m.colStatus}</h3>
          <Field k={t.modsRepo.installedVer} v={row.installedVersion || null} />
          <Field k={m.colUpgradable} v={row.status === "updatable" ? row.latestVersion : row.latestVersion || null} accent={row.status === "updatable"} />
          <Field k={m.colLocation} v={locationText} />
          <Field k={m.colDirName} v={row.dirName || null} />

          {/* 安装台账（仓库安装来源可追溯） */}
          {row.ledger && (
            <>
              <h3 className="mb-1 mt-4 text-xs font-semibold uppercase tracking-wider text-slate-500">{m.sourceRepo}</h3>
              <Field k={m.sourceRepo} v={row.ledger.repositoryUrl || row.ledger.source || null} />
              <Field k={m.badgeHint} v={row.ledger.via && row.ledger.via !== "direct" ? row.ledger.via : null} />
            </>
          )}

          {/* 更新日志（抽屉 v2）：已装且索引无 changelog 数据时整块隐藏；可下载/可更新行
              即使无数据也显示占位——「可更新」场景下让用户先看到新版改了什么，再往下读简介。 */}
          {(logs.length > 0 || row.status !== "installed") && (
            <>
              <h3 className="mb-1 mt-4 text-xs font-semibold uppercase tracking-wider text-slate-500">{m.drawerChangelog}</h3>
              {logs.length > 0 ? (
                <div className="space-y-2">
                  {logs.map((g) => (
                    <div key={g.version}>
                      <div className="text-xs font-semibold text-slate-300">v{g.version}</div>
                      {g.lines.length > 0 && (
                        <ul className="mt-0.5 list-disc space-y-0.5 pl-5 text-[13px] leading-relaxed text-slate-400">
                          {g.lines.map((ln, i) => (
                            <li key={i}>{ln}</li>
                          ))}
                        </ul>
                      )}
                    </div>
                  ))}
                </div>
              ) : (
                <div className="text-[13px] text-slate-500">{t.modsRepo.noChangelog}</div>
              )}
            </>
          )}

          {/* 简介：作者写的富文本（实测含 `<br>` / `<h2>` 等标签）经 `toMarkdown` 重铸后
              交给 `Markdown` 渲染（输出 React 元素，不走 innerHTML）——与 mods 页 `DetailDrawer`
              同口径；描述缺失（官方索引未 enrich 到的条目）时给占位，保证各页抽屉一致。 */}
          <h3 className="mb-1 mt-4 text-xs font-semibold uppercase tracking-wider text-slate-500">{m.drawerDesc}</h3>
          {desc ? (
            <Markdown source={toMarkdown(desc)} className="text-[13px] leading-relaxed" />
          ) : (
            <div className="text-[13px] text-slate-500">{t.mods.noDescription}</div>
          )}

          {/* 仓库信息（对齐官方 qt 详情面板字段） */}
          {(author || size || stars || entry) && (
            <>
              <h3 className="mb-1 mt-4 text-xs font-semibold uppercase tracking-wider text-slate-500">{m.drawerRepo}</h3>
              <Field k={m.drawerAuthor} v={author || null} />
              <Field k={m.drawerSize} v={size || null} />
              <Field k={m.drawerStars} v={stars || null} />
              {entry && <Field k={m.sourceRepo} v={entry.repositoryUrl || null} />}
            </>
          )}
        </div>
      </aside>
    </>
  );
}
