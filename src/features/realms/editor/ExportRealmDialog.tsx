import { useEffect, useMemo, useState } from "react";
import { useI18n } from "@/i18n";
import { cn } from "@/lib/utils";
import { formatBytes } from "@/lib/updater";
import {
  cancelRealmExport,
  exportRealm,
  onRealmExportProgress,
  openInExplorer,
  pickOpenDirectory,
  pickSavePath,
  previewRealmPublish,
  readLauncherConfig,
  setRealmPublishTarget,
  type ExportRealmResult,
  type RealmExportProgress,
  type RealmPublishPreview,
  type RealmUrlSource,
} from "@/lib/tauri";
import { backendErrorText } from "@/lib/backendError";

/** 弹窗状态机：填写 → 进行中 → 结果 / 失败（失败可重试回 idle）。 */
type Phase = "idle" | "running" | "result" | "error";

interface ExportRealmDialogProps {
  open: boolean;
  /** 领域 id（默认文件名、进度事件过滤、仓库子目录名都用它） */
  realmId: string;
  onClose: () => void;
  /** 宿主算出「本领域所需素材 mod」（`realmAssetTargets({ excludeCore: true })` 的产物） */
  resolveCacheMods: () => Promise<string[]>;
  /** 一次成功导出后回调（宿主可据此提示 / 刷新） */
  onExported?: (r: ExportRealmResult) => void;
}

/** 会导致分发失败的告警（红色）；其余按黄色「需注意」处理 */
const RED_WARNING_CODES = new Set(["over-download-max", "bad-id"]);

/** 导出分支：分享 = 只出包文件（可选索引片段）；发布 = 包 + 封面 + index.json 直写仓库工作树。 */
type ExportMode = "share" | "publish";

/** i18n 占位填充（`{done}` / `{arg}` 之类） */
function fill(tpl: string, map: Record<string, string | number>): string {
  let out = tpl;
  for (const [k, v] of Object.entries(map)) out = out.replace(`{${k}}`, String(v));
  return out;
}

/** 绝对路径 → 相对仓库根（「将写入」清单里显示 `realms/<id>/…` 比整串绝对路径好读） */
function relToRepo(repoDir: string, path: string): string {
  const base = repoDir.trim().replace(/[\\/]+$/, "").replace(/\\/g, "/");
  const p = path.replace(/\\/g, "/");
  return p.toLowerCase().startsWith(`${base.toLowerCase()}/`) ? p.slice(base.length + 1) : p;
}

/** 文件路径 → 所在目录（`open_in_explorer` 直传文件会被系统当成“打开文件”）。 */
function dirOf(path: string): string {
  const i = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  return i > 0 ? path.slice(0, i) : path;
}

/**
 * 「导出领域」弹窗（§9.6.1 / §7.1 / §7.3）。
 *
 * 三态设计：
 * - `idle`：填路径与选项。**蒙层不可点关闭**（避免误触丢设置），只走取消按钮 / ESC；
 * - `running`：后端已移出主线程跑重 IO，这里读 `realm-export-progress` 显示阶段与进度，
 *   并提供「取消导出」（不可点蒙层/ESC 关闭，防止以为关掉窗口就是取消）；
 * - `result` / `error`：结构化呈现产物与发布自检告警，取代原先只有一条 toast 的无反馈体验。
 */
export function ExportRealmDialog({
  open,
  realmId,
  onClose,
  resolveCacheMods,
  onExported,
}: ExportRealmDialogProps) {
  const { t } = useI18n();
  const [phase, setPhase] = useState<Phase>("idle");
  const [mode, setMode] = useState<ExportMode>("share");
  const [exportPath, setExportPath] = useState("");
  const [includeCache, setIncludeCache] = useState(false);
  /** 分享分支的索引片段（**默认关**：发给朋友时多一个 .index.json 是噪声） */
  const [includeFragment, setIncludeFragment] = useState(false);
  const [repoDir, setRepoDir] = useState("");
  /** 下载前缀的**人工覆盖值**（只存本机）；默认空 = 用后端推导结果 */
  const [urlBase, setUrlBase] = useState("");
  const [editingUrl, setEditingUrl] = useState(false);
  const [preview, setPreview] = useState<RealmPublishPreview | null>(null);
  const [previewErr, setPreviewErr] = useState("");
  const [previewing, setPreviewing] = useState(false);
  const [progress, setProgress] = useState<RealmExportProgress | null>(null);
  const [result, setResult] = useState<ExportRealmResult | null>(null);
  const [errMsg, setErrMsg] = useState("");
  /** 回填 idle 态的一次性提示（「已取消导出」/「已自举 urlBase」） */
  const [notice, setNotice] = useState("");
  const [cancelling, setCancelling] = useState(false);

  const busy = phase === "running";
  const publishing = mode === "publish";
  /** 发布分支拿到预览才算可导出：目录无效 / index.json 坏了 / 降级发布（versionRel = down）
      都在预览处就地报错并禁用，不放行——降级检测已前移到后端打包之前，这里只是 UX 前置。 */
  const canExport = publishing
    ? !!preview && preview.indexParseOk && preview.versionRel !== "down" && !previewing
    : exportPath.trim().length > 0;
  /** 「将写入」清单的目录前缀：从后端给的包绝对路径反推，前端不再抄一遍仓库布局约定 */
  const willWriteDir = preview ? relToRepo(repoDir, preview.pkgPath).replace(/[^/]*$/, "") : "";

  // 打开时复位并回填上次的发布目标与分支（launcher.json `realms.publish_*`）
  useEffect(() => {
    if (!open) return;
    setPhase("idle");
    setProgress(null);
    setResult(null);
    setErrMsg("");
    setNotice("");
    setCancelling(false);
    setEditingUrl(false);
    setPreview(null);
    setPreviewErr("");
    setExportPath(`${realmId}.vcmirealm`);
    let alive = true;
    void readLauncherConfig().then((cfg) => {
      if (!alive) return;
      const dir = (cfg?.realmsPublishRepoDir ?? "").trim();
      setRepoDir(dir);
      setUrlBase((cfg?.realmsPublishUrlBase ?? "").trim());
      // 默认用上次实际选的分支（便携包要求偏好随目录走，故存配置不存 localStorage）
      setMode(dir && cfg?.realmsPublishMode === "publish" ? "publish" : "share");
    });
    return () => {
      alive = false;
    };
  }, [open, realmId]);

  /**
   * 发布分支：改目录 / 改前缀覆盖后防抖预览——点导出**之前**就要把会动哪三个文件、
   * 下载地址是什么显示出来（取代原先那个要求用户凭空填的「下载 URL 前缀」输入框）。
   */
  useEffect(() => {
    if (!open || phase !== "idle" || !publishing) return;
    const dir = repoDir.trim();
    if (!dir) {
      setPreview(null);
      setPreviewErr("");
      return;
    }
    const timer = setTimeout(() => {
      setPreviewing(true);
      void previewRealmPublish(dir, realmId, urlBase.trim() || null)
        .then((p) => {
          setPreview(p);
          setPreviewErr("");
        })
        .catch((e: unknown) => {
          setPreview(null);
          setPreviewErr(backendErrorText(e));
        })
        .finally(() => setPreviewing(false));
    }, 250);
    return () => clearTimeout(timer);
  }, [open, phase, publishing, repoDir, realmId, urlBase]);

  // ESC 关闭：进行中禁用（关掉窗口不等于取消导出）
  useEffect(() => {
    if (!open || busy) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, busy, onClose]);

  // 进度订阅（仅进行中挂载；以 invoke resolve 为权威完成信号，事件丢帧无害）
  useEffect(() => {
    if (!busy) return;
    let unsub: (() => void) | null = null;
    let alive = true;
    void onRealmExportProgress((p) => {
      if (alive && p.realmId === realmId) setProgress(p);
    }).then((fn) => {
      if (!alive) fn();
      else unsub = fn;
    });
    return () => {
      alive = false;
      if (unsub) unsub();
    };
  }, [busy, realmId]);

  const phaseText = useMemo(() => {    switch (progress?.phase) {
      case "prepare":
        return t.realm.exportPhasePrepare;
      case "collect":
        return t.realm.exportPhaseCollect;
      case "pack":
        return t.realm.exportPhasePack;
      case "index":
        return t.realm.exportPhaseIndex;
      case "publish":
        return t.realm.exportPhasePublish;
      default:
        return t.realm.exportPhasePrepare;
    }
  }, [progress?.phase, t]);

  /** 优先按字节算（大包耗时集中在写 zip），没有总量时退化成条目计数。 */
  const percent = useMemo(() => {
    if (!progress) return 0;
    if (progress.bytesTotal > 0) {
      return Math.min(100, Math.round((progress.bytesDone / progress.bytesTotal) * 100));
    }
    return progress.total > 0
      ? Math.min(100, Math.round((progress.done / progress.total) * 100))
      : 0;
  }, [progress]);

  const pickPath = async () => {
    const p = await pickSavePath(`${realmId}.vcmirealm`, [
      { name: t.realm.realmFile, extensions: ["vcmirealm"] },
    ]);
    if (p) setExportPath(p);
  };

  const pickRepo = async () => {
    const p = await pickOpenDirectory();
    if (!p) return;
    const dir = p.replace(/[\\/]+$/, "");
    setRepoDir(dir);
    // 选完就先持久化：用户可能只换目录不导出，下次不该重选
    void setRealmPublishTarget(dir, urlBase.trim() || null, mode).catch(() => {});
  };

  const run = async () => {
    if (!canExport) return;
    // 发布分支的目标路径由仓库约定决定（`realms/<id>/<id>.vcmirealm`），不再让用户手改
    const dest = publishing
      ? (preview?.pkgPath ?? "")
      : exportPath.trim() || `${realmId}.vcmirealm`;
    if (!dest) return;
    setProgress(null);
    setErrMsg("");
    setPhase("running");
    try {
      // 附带素材缓存时先算"本领域所需 mod"（启用集合 + bundled/borrowed），
      // 与领域激活/巡礼同源，但**排除原版 core.***（见 `realmAssetTargets`）。
      const cacheMods = includeCache ? await resolveCacheMods() : undefined;
      const r = await exportRealm(realmId, dest, {
        includeShowcaseCache: includeCache,
        // 发布必然写仓库 index.json；分享按勾选决定是否另出片段
        includeIndex: publishing ? true : includeFragment,
        cacheMods,
        repoDir: publishing ? repoDir.trim() : undefined,
        urlBase: publishing ? urlBase.trim() || undefined : undefined,
      });
      if (r.cancelled) {
        // 取消：半成品已被后端删除，回到可编辑态让用户改选项重试
        setPhase("idle");
        setProgress(null);
        setNotice(t.realm.exportCancelled);
        return;
      }
      void setRealmPublishTarget(
        repoDir.trim() || null,
        urlBase.trim() || null,
        mode,
      ).catch(() => {});
      // 自举：推导出的前缀已被后端写进仓库 index.json，回 idle 后预览会改口为「来自声明」
      if (r.repo?.urlBootstrapped) setNotice(t.realm.exportUrlBootstrapped);
      setResult(r);
      setPhase("result");
      onExported?.(r);
    } catch (e) {
      setErrMsg(backendErrorText(e));
      setPhase("error");
    } finally {
      setCancelling(false);
    }
  };

  const requestCancel = async () => {
    setCancelling(true);
    try {
      await cancelRealmExport(realmId);
    } catch {
      setCancelling(false);
    }
  };

  if (!open) return null;

  const repo = result?.repo ?? null;
  const warnings = result?.warnings ?? [];

  return (
    /* 蒙层**不挂 onClick**：导出选项（路径 / 两个勾选 / 仓库目录）误触丢失的代价比多点一下高，
       与通用 ConfirmDialog 一致，只走「取消 / X / ESC」。 */
    <div className="fixed inset-0 z-[145] flex items-center justify-center bg-black/65 p-4 backdrop-blur-sm">
      <div className="w-full max-w-md rounded-xl border border-slate-700/60 bg-slate-900 p-5 shadow-2xl">
        <h3 className="mb-3 text-base font-semibold text-slate-100">
          {phase === "result"
            ? t.realm.exportDoneTitle
            : phase === "error"
              ? t.realm.exportFailedTitle
              : t.realm.exportTitle}
        </h3>

        {phase === "idle" && (
          <>
            <p className="mb-3 text-sm text-slate-400">{t.realm.exportDesc}</p>
            {notice && (
              <p className="mb-3 rounded-md border border-slate-700 bg-slate-800/60 px-2.5 py-2 text-[11px] text-slate-300">
                {notice}
              </p>
            )}
            {/* 分支二选一：取代原先「生成远程仓库索引 / 写入本地仓库」两个需互相解释的勾选。
                选哪个就只出现哪一组控件；素材勾选两分支共用，放在分支内容之后。 */}
            <div className="mb-1 grid grid-cols-2 gap-1 rounded-lg border border-slate-700 bg-slate-800/60 p-1">
              {(["share", "publish"] as const).map((m) => (
                <button
                  key={m}
                  type="button"
                  onClick={() => setMode(m)}
                  className={cn(
                    "rounded-md px-2 py-1.5 text-xs font-medium transition-colors",
                    mode === m
                      ? "bg-blue-600/80 text-white"
                      : "text-slate-400 hover:text-slate-200",
                  )}
                >
                  {m === "share" ? t.realm.exportModeShare : t.realm.exportModePublish}
                </button>
              ))}
            </div>
            <p className="mb-3 text-[11px] leading-relaxed text-slate-500">
              {t.realm.exportModeHint}
            </p>

            {mode === "share" ? (
              <div className="mb-3 flex items-center gap-2">
                <input
                  value={exportPath}
                  onChange={(e) => setExportPath(e.target.value)}
                  placeholder={`${realmId}.vcmirealm`}
                  className="min-w-0 flex-1 rounded-lg border border-slate-700 bg-slate-800 px-3 py-2 text-sm text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500/50"
                />
                <button
                  type="button"
                  onClick={pickPath}
                  className="shrink-0 rounded-lg border border-slate-700 px-2.5 py-2 text-sm text-slate-200 hover:bg-slate-800/50"
                >
                  {t.realm.exportPickPath}
                </button>
              </div>
            ) : (
              <div className="mb-3 space-y-2">
                <div className="flex items-center gap-2">
                  <input
                    value={repoDir}
                    onChange={(e) => setRepoDir(e.target.value)}
                    placeholder={t.realm.exportPublishRepoPlaceholder}
                    className="min-w-0 flex-1 rounded-lg border border-slate-700 bg-slate-800 px-2.5 py-2 text-sm text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500/50"
                  />
                  <button
                    type="button"
                    onClick={pickRepo}
                    className="shrink-0 rounded-lg border border-slate-700 px-2.5 py-2 text-sm text-slate-200 hover:bg-slate-800/50"
                  >
                    {t.realm.exportPublishPick}
                  </button>
                </div>
                {!repoDir.trim() ? (
                  <p className="text-[11px] text-amber-300/80">{t.realm.exportNeedRepoDir}</p>
                ) : previewing ? (
                  <p className="text-[11px] text-slate-500">{t.realm.exportPreviewLoading}</p>
                ) : previewErr ? (
                  <p className="break-all rounded-md border border-red-500/40 bg-red-500/10 px-2 py-1.5 text-[11px] leading-relaxed text-red-200">
                    {previewErr}
                  </p>
                ) : preview && !preview.indexParseOk ? (
                  <p className="rounded-md border border-red-500/40 bg-red-500/10 px-2 py-1.5 text-[11px] leading-relaxed text-red-200">
                    {t.realm.exportRepoIndexBroken}
                  </p>
                ) : null}
                {/* 降级发布：后端会在打包前硬拒（§7.3）；这里提前标红并禁用按钮，
                    免得用户等到打包完才失败。有意回滚 → 先改仓库 index.json 的 version 再发。 */}
                {preview && preview.indexParseOk && preview.versionRel === "down" && (
                  <p className="rounded-md border border-red-500/40 bg-red-500/10 px-2 py-1.5 text-[11px] leading-relaxed text-red-200">
                    {fill(t.realm.exportPreviewVersionDown, {
                      from: preview.prevVersion ?? "",
                      to: preview.nextVersion,
                    })}
                  </p>
                )}
                {preview && preview.indexParseOk && (
                  <div className="space-y-1.5 rounded-lg border border-slate-700/70 bg-slate-800/40 px-2.5 py-2">
                    <p className="text-[11px] font-medium text-slate-400">
                      {t.realm.exportWillWrite}
                    </p>
                    <ul className="space-y-0.5 font-mono text-[11px] text-slate-300">
                      <li className="truncate" title={preview.pkgPath}>
                        {relToRepo(repoDir, preview.pkgPath)}
                      </li>
                      {preview.coverName && (
                        <li className="truncate text-slate-400">
                          {willWriteDir}
                          {preview.coverName}
                        </li>
                      )}
                      <li className="truncate text-slate-400">{t.realm.exportIndexMerge}</li>
                    </ul>
                    {/* 分片预估：只在预计会切分时露面（常态小包零噪声）。
                        分片是自动行为、不给用户选项——这里只负责「知情」，见设计 §9。 */}
                    {(preview.estimatedPartCount ?? 0) > 0 && (
                      <p className="pt-1 text-[11px] leading-relaxed text-slate-400">
                        {fill(t.realm.exportShardEstimate, {
                          count: preview.estimatedPartCount ?? 0,
                        })}
                      </p>
                    )}
                    {/* 下载地址：只读显示推导结果 + 「改」覆盖入口。
                        不再要求用户凭空填 URL——把「填空」换成「确认」，出错率天差地别。 */}
                    <div className="pt-1">
                      <div className="flex items-baseline gap-2">
                        <span className="flex-none text-[11px] text-slate-500">
                          {t.realm.exportDownloadUrl}
                        </span>
                        <span
                          className={cn(
                            "min-w-0 flex-1 truncate font-mono text-[11px]",
                            preview.downloadUrl ? "text-slate-300" : "text-amber-300/90",
                          )}
                          title={preview.downloadUrl ?? ""}
                        >
                          {preview.downloadUrl ?? t.realm.exportUrlSourceNone}
                        </span>
                        <button
                          type="button"
                          onClick={() => setEditingUrl((v) => !v)}
                          className="shrink-0 rounded border border-slate-700 px-1.5 py-0.5 text-[11px] text-slate-400 hover:bg-slate-800/60 hover:text-slate-200"
                        >
                          {t.realm.exportEditUrl}
                        </button>
                      </div>
                      <p className="mt-0.5 text-[11px] leading-relaxed text-slate-500">
                        {urlSourceText(t, preview.urlSource)}
                      </p>
                      {editingUrl && (
                        <div className="mt-1.5">
                          <input
                            value={urlBase}
                            onChange={(e) => setUrlBase(e.target.value)}
                            placeholder={t.realm.exportPublishUrlBase}
                            className="w-full rounded-lg border border-slate-700 bg-slate-800 px-2.5 py-1.5 text-xs text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500/50"
                          />
                          <p className="mt-1 text-[11px] leading-relaxed text-slate-500">
                            {t.realm.exportUrlOverrideHint}
                          </p>
                        </div>
                      )}
                    </div>
                  </div>
                )}
              </div>
            )}
            {/* 素材缓存：两分支共用，默认**不带**（体积大；原版阵营素材已排除，见 realmAssetTargets） */}
            <label className="mb-2 flex cursor-pointer items-start gap-2 text-xs text-slate-300">
              <input
                type="checkbox"
                checked={includeCache}
                onChange={(e) => setIncludeCache(e.target.checked)}
                className="mt-0.5 h-3.5 w-3.5 flex-none accent-blue-500"
              />
              <span className="min-w-0">
                <span className="font-medium">{t.realm.exportIncludeCache}</span>
                <span className="mt-0.5 block text-[11px] leading-relaxed text-slate-500">
                  {t.realm.exportIncludeCacheHint}
                </span>
              </span>
            </label>
            {/* 索引片段：仅分享分支需要（发布分支直接写仓库索引，片段是多余产物） */}
            {mode === "share" && (
              <label className="mb-1 flex cursor-pointer items-start gap-2 text-xs text-slate-300">
                <input
                  type="checkbox"
                  checked={includeFragment}
                  onChange={(e) => setIncludeFragment(e.target.checked)}
                  className="mt-0.5 h-3.5 w-3.5 flex-none accent-blue-500"
                />
                <span className="min-w-0">
                  <span className="font-medium">{t.realm.exportFragment}</span>
                  <span className="mt-0.5 block text-[11px] leading-relaxed text-slate-500">
                    {t.realm.exportFragmentHint}
                  </span>
                </span>
              </label>
            )}
          </>
        )}

        {busy && (
          <div className="mb-4">
            <p className="mb-2 text-sm text-slate-300">{phaseText}</p>
            <div className="h-1.5 overflow-hidden rounded-full bg-slate-800">
              <div
                className={cn(
                  "h-full rounded-full bg-blue-500 transition-[width] duration-200",
                  !progress && "animate-pulse bg-slate-600",
                )}
                style={{ width: `${progress ? percent : 8}%` }}
              />
            </div>
            <div className="mt-2 flex items-center justify-between gap-2 text-[11px] text-slate-500">
              <span>
                {progress
                  ? fill(t.realm.exportProgressFiles, { done: progress.done, total: progress.total })
                  : ""}
              </span>
              <span>
                {progress && progress.bytesTotal > 0
                  ? fill(t.realm.exportProgressBytes, {
                      done: formatBytes(progress.bytesDone),
                      total: formatBytes(progress.bytesTotal),
                    })
                  : ""}
              </span>
            </div>
            {progress?.current && (
              <p className="mt-1 truncate text-[11px] text-slate-600" title={progress.current}>
                {progress.current}
              </p>
            )}
          </div>
        )}

        {phase === "result" && result && (
          <div className="mb-4 max-h-[52vh] space-y-1.5 overflow-y-auto text-xs">
            <ResultRow label={t.realm.exportFieldPath} value={result.path} mono />
            <ResultRow label={t.realm.exportFieldSize} value={formatBytes(result.size)} />
            <ResultRow label={t.realm.exportFieldFiles} value={String(result.fileCount)} />
            {result.showcaseCache && (
              <ResultRow label={t.realm.exportIncludeCache} value={t.common.ok} />
            )}
            {repo ? (
              <>
                <ResultRow
                  label={t.realm.exportPublishRepo}
                  value={
                    repo.entryState === "created"
                      ? t.realm.exportRepoCreated
                      : repo.entryState === "updated"
                        ? t.realm.exportRepoUpdated
                        : t.realm.exportRepoUnchanged
                  }
                />
                <ResultRow label={t.realm.exportFieldIndex} value={repo.indexPath} mono />
                {repo.coverPath ? <ResultRow label={t.realm.exportFieldCover} value={repo.coverPath} mono /> : null}
                {/* 分片产物（0 = 未分片）：片数 + 各片相对仓库的路径（路径由后端给，前端不拼名） */}
                {(repo.partCount ?? 0) > 0 && (
                  <ResultRow label={t.realm.exportFieldParts} value={String(repo.partCount ?? 0)} />
                )}
                {(repo.parts?.length ?? 0) > 0 && (
                  <ul className="space-y-0.5 font-mono text-[11px] text-slate-400">
                    {repo.parts?.map((p) => (
                      <li key={p.path} className="truncate" title={p.path}>
                        {relToRepo(repo.repoDir, p.path)}
                      </li>
                    ))}
                  </ul>
                )}
                {repo.entryState === "updated" && (
                  <p className="pt-1 text-[11px] leading-relaxed text-slate-500">
                    {fill(t.realm.exportRepoChanged, { fields: repo.changedFields.join(", ") })}
                  </p>
                )}
                {/* 自举：地址已固化进仓库，下次发布与协作者都不再需要推导 */}
                {repo.urlBootstrapped && (
                  <p className="pt-1 text-[11px] leading-relaxed text-emerald-300/80">
                    {t.realm.exportUrlBootstrapped}
                  </p>
                )}
              </>
            ) : result.indexPath ? (
              <ResultRow label={t.realm.exportFieldIndex} value={result.indexPath} mono />
            ) : null}
            {warnings.length > 0 && (
              <ul className="mt-2 space-y-1">
                {warnings.map((w, i) => (
                  <li
                    key={`${w.code}-${i}`}
                    className={cn(
                      "rounded-md border px-2 py-1.5 text-[11px] leading-relaxed",
                      RED_WARNING_CODES.has(w.code)
                        ? "border-red-500/40 bg-red-500/10 text-red-200"
                        : "border-amber-500/30 bg-amber-500/10 text-amber-200",
                    )}
                  >
                    {warningText(t, w.code, w.arg ?? "")}
                  </li>
                ))}
              </ul>
            )}
          </div>
        )}

        {phase === "error" && (
          <div className="mb-4">
            <p className="mb-2 text-sm text-red-300">{t.realm.exportFail}</p>
            <p className="max-h-[40vh] overflow-y-auto break-all rounded-md border border-red-500/40 bg-red-500/10 px-2.5 py-2 text-[11px] leading-relaxed text-red-200">
              {exportErrorText(t, errMsg)}
            </p>
          </div>
        )}

        <div className="flex items-center justify-end gap-2">
          {phase === "result" && (repo || result?.indexPath) && (
            <button
              type="button"
              onClick={() =>
                void openInExplorer(repo ? repo.repoDir : dirOf(result?.indexPath ?? "")).catch(
                  () => {},
                )
              }
              className="mr-auto rounded-lg border border-slate-700 px-3 py-1.5 text-xs text-slate-300 hover:bg-slate-800/50"
            >
              {repo ? t.realm.exportOpenRepo : t.realm.exportOpenIndex}
            </button>
          )}
          {phase === "idle" && (
            <>
              <button
                type="button"
                onClick={onClose}
                className="rounded-lg border border-slate-700 px-3 py-1.5 text-sm text-slate-300 hover:bg-slate-800/50"
              >
                {t.common.cancel}
              </button>
              <button
                type="button"
                onClick={run}
                disabled={!canExport}
                className="rounded-lg bg-gradient-to-br from-blue-500 to-indigo-500 px-3 py-1.5 text-sm font-medium text-white shadow-md shadow-blue-600/40 hover:opacity-90 disabled:cursor-not-allowed disabled:opacity-40"
              >
                {t.realm.exportConfirm}
              </button>
            </>
          )}
          {busy && (
            <button
              type="button"
              onClick={requestCancel}
              disabled={cancelling}
              className="rounded-lg border border-slate-700 px-3 py-1.5 text-sm text-slate-300 hover:bg-slate-800/50 disabled:opacity-50"
            >
              {t.realm.exportCancel}
            </button>
          )}
          {phase === "result" && (
            <>
              <button
                type="button"
                onClick={() => {
                  setResult(null);
                  setProgress(null);
                  setNotice("");
                  setPhase("idle");
                }}
                className="rounded-lg border border-slate-700 px-3 py-1.5 text-sm text-slate-300 hover:bg-slate-800/50"
              >
                {t.realm.exportAgain}
              </button>
              <button
                type="button"
                onClick={onClose}
                className="rounded-lg bg-gradient-to-br from-blue-500 to-indigo-500 px-3 py-1.5 text-sm font-medium text-white shadow-md shadow-blue-600/40 hover:opacity-90"
              >
                {t.common.close}
              </button>
            </>
          )}
          {phase === "error" && (
            <>
              <button
                type="button"
                onClick={onClose}
                className="rounded-lg border border-slate-700 px-3 py-1.5 text-sm text-slate-300 hover:bg-slate-800/50"
              >
                {t.common.close}
              </button>
              <button
                type="button"
                onClick={() => setPhase("idle")}
                className="rounded-lg bg-gradient-to-br from-blue-500 to-indigo-500 px-3 py-1.5 text-sm font-medium text-white shadow-md shadow-blue-600/40 hover:opacity-90"
              >
                {t.realm.exportRetry}
              </button>
            </>
          )}
        </div>
      </div>
    </div>
  );
}

/** 结果态一行：标签 + 值（`mono` 用于路径，过长省略号 + title 全文） */
function ResultRow({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex items-start gap-2">
      <span className="w-24 flex-none text-slate-500">{label}</span>
      <span
        className={cn("min-w-0 flex-1 break-all text-slate-200", mono && "font-mono text-[11px]")}
        title={value}
      >
        {value}
      </span>
    </div>
  );
}

/** 前缀来源 → 本地文案（后端只回 source 枚举，文案留在 i18n 侧）。 */
function urlSourceText(t: ReturnType<typeof useI18n>["t"], src: RealmUrlSource): string {
  const r = t.realm;
  switch (src) {
    case "declared":
      return r.exportUrlSourceDeclared;
    case "git":
      return r.exportUrlSourceGit;
    case "entries":
      return r.exportUrlSourceEntries;
    case "manual":
      return r.exportUrlSourceManual;
    default:
      return r.exportUrlSourceNone;
  }
}

/** 告警 code → 本地文案（后端只回 code + 已格式化的补充信息，文案留在 i18n 侧）。 */
function warningText(t: ReturnType<typeof useI18n>["t"], code: string, arg: string): string {
  const r = t.realm;
  switch (code) {
    case "over-download-max":
      return fill(r.exportWarnOverDownloadMax, { arg });
    case "over-git-max":
      return fill(r.exportWarnOverGitMax, { arg });
    case "bundled":
      return fill(r.exportWarnBundled, { arg });
    case "bad-id":
      return fill(r.exportWarnBadId, { arg });
    case "no-url-base":
      return r.exportWarnNoUrlBase;
    case "version-unchanged":
      return fill(r.exportWarnVersionUnchanged, { arg });
    case "pkg-unchanged":
      return fill(r.exportWarnPkgUnchanged, { arg });
    case "copied-into-repo":
      return fill(r.exportWarnCopiedIntoRepo, { arg });
    // 分片（包 >45 MiB 时自动切分，非错误——见 realms-sharded-package 设计 §9）
    case "sharded":
      return fill(r.exportWarnSharded, { arg });
    case "over-total-hint":
      return fill(r.exportWarnOverTotalHint, { arg });
    case "shard-needs-url-base":
      return fill(r.exportWarnShardNeedsUrlBase, { arg });
    case "shard-failed":
      return r.exportWarnShardFailed;
    default:
      return arg ? `${code}: ${arg}` : code;
  }
}

/**
 * 错误消息 → 本地化文案。后端可能抛**结构化错误**（`{code,from,to}` 的 JSON 串，
 * 如降级发布的 `version_downgrade`），其余（普通字符串）原样显示。
 */
function exportErrorText(t: ReturnType<typeof useI18n>["t"], msg: string): string {
  try {
    const v = JSON.parse(msg) as { code?: string; from?: string; to?: string };
    if (v && v.code === "version_downgrade") {
      return fill(t.realm.exportErrVersionDowngrade, { from: v.from ?? "", to: v.to ?? "" });
    }
  } catch {
    // 非结构化错误：走原样显示
  }
  return msg;
}
