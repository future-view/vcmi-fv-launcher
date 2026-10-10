import { useEffect, useMemo, useState } from "react";
import type { ReactNode } from "react";
import { Download, RefreshCw, Settings2 } from "lucide-react";
import { cn } from "@/lib/utils";
import { compareModVersions } from "@/lib/modRepo";
import { topRealmTags } from "@/lib/realm/intro";
import { useI18n } from "@/i18n";
import {
  addRealmRepository,
  cancelRealmDownload,
  coverUrl,
  ensureRealmCovers,
  fetchRealmIndex,
  getRealmRepositories,
  installRemoteRealm,
  listInstalledMods,
  onRealmDownloadProgress,
  removeRealmRepository,
  testRealmRepository,
  REALM_DOWNLOAD_CANCELLED,
  type RealmRepositoryIndex,
  type RemoteRealm,
} from "@/lib/tauri";
import { backendErrorCode, backendErrorText } from "@/lib/backendError";

interface Props {
  /** 覆盖层内统一 Toast */
  showToast: (type: "success" | "error", msg: string) => void;
  /** 安装成功后回调：刷新本地领域列表 */
  onInstalled: () => Promise<void> | void;
}

/** 索引 i18n 块解析：匹配界面语言时整体覆盖 name/description/tags，无匹配按回退链兜底。 */
function fallbackLang(lang: string): string {
  return lang === "tchinese" ? "chinese" : "english";
}
function resolveRealm(
  r: RemoteRealm,
  lang: string,
): { name: string; description: string; tags: string[] } {
  const top = {
    name: typeof r.name === "string" ? r.name : "",
    description: typeof r.description === "string" ? r.description : "",
    tags: r.tags ?? [],
  };
  const i18n = r.i18n;
  if (!i18n || Object.keys(i18n).length === 0) return top;
  const block = i18n[lang] ?? i18n[fallbackLang(lang)] ?? i18n.english ?? null;
  if (!block) return top;
  return {
    name: block.name ?? top.name,
    description: block.description ?? top.description,
    tags: block.tags ?? top.tags,
  };
}

function fmtSize(n?: number): string {
  if (!n || n <= 0) return "";
  if (n >= 1024 * 1024 * 1024) return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
  if (n >= 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${Math.max(1, Math.round(n / 1024))} KB`;
}

/** 按领域 id 生成稳定的色相，用于无封面时的渐变占位块。 */
function hueOf(id: string): number {
  let h = 0;
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) % 360;
  return h;
}

function Modal({
  open,
  onClose,
  width = 560,
  children,
}: {
  open: boolean;
  onClose: () => void;
  width?: number;
  children: ReactNode;
}) {
  if (!open) return null;
  return (
    <div
      className="fixed inset-0 z-[300] grid place-items-center bg-slate-950/70 p-6 backdrop-blur-sm"
      onClick={onClose}
    >
      <div
        style={{ width }}
        className="max-h-[86vh] overflow-auto rounded-2xl border border-slate-700 bg-slate-900 p-6 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        {children}
      </div>
    </div>
  );
}

/**
 * 远程领域页（方案 v0.4：只有列表、没有详情页）。
 *
 * 列表按「有更新 / 未安装 / 已安装」三组展示，已安装组整卡禁用；
 * 支持名称检索与多标签筛选（chips 多选，OR 关系）；封面由后端下载后本地显示。
 */
export function RealmRepositoryPage({ showToast, onInstalled }: Props) {
  const { t } = useI18n();
  const lang = t.uiLang;
  const [index, setIndex] = useState<RealmRepositoryIndex | null>(null);
  const [loading, setLoading] = useState(true);
  const [covers, setCovers] = useState<Record<string, string>>({});
  const [installed, setInstalled] = useState<Set<string>>(new Set());
  const [kw, setKw] = useState("");
  const [tags, setTags] = useState<string[]>([]);
  const [progress, setProgress] = useState<Record<string, { received: number; total: number }>>({});
  const [repos, setRepos] = useState<string[]>([]);
  const [sourceOpen, setSourceOpen] = useState(false);
  const [srcInput, setSrcInput] = useState("");
  const [srcRes, setSrcRes] = useState<{ ok: boolean; text: string }>({ ok: true, text: "" });
  const [updateTarget, setUpdateTarget] = useState<RemoteRealm | null>(null);
  const [conflict, setConflict] = useState<RemoteRealm | null>(null);

  /** 拉取索引（force = 忽略 TTL）+ 封面缓存。失败且有缓存时后端会回退缓存。 */
  const load = async (force: boolean) => {
    setLoading(true);
    try {
      const data = await fetchRealmIndex(force);
      setIndex(data);
      const c = await ensureRealmCovers();
      setCovers(c);
    } catch (e) {
      setIndex(null);
      showToast("error", t.realm.remoteFetchFailed.replace("{msg}", backendErrorText(e)));
    } finally {
      setLoading(false);
    }
  };

  useEffect(() => {
    void load(false);
    void getRealmRepositories()
      .then((r) => setRepos(r.urls))
      .catch(() => setRepos([]));
    void listInstalledMods().then((mods) => setInstalled(new Set(mods.map((m) => m.id))));
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    let off: (() => void) | undefined;
    void onRealmDownloadProgress((p) => {
      setProgress((prev) => ({
        ...prev,
        [p.realmId]: { received: p.received, total: p.total || prev[p.realmId]?.total || 0 },
      }));
    }).then((fn) => {
      off = fn;
    });
    return () => {
      off?.();
    };
  }, []);

  const list = useMemo(() => {
    const k = kw.trim().toLowerCase();
    return (index?.realms ?? []).filter((r) => {
      if (k) {
        const rl = resolveRealm(r, lang);
        const hay = [rl.name, rl.description, r.id, r.author ?? ""]
          .join(" ")
          .toLowerCase();
        if (!hay.includes(k)) return false;
      }
      if (tags.length && !resolveRealm(r, lang).tags.some((x) => tags.includes(x))) return false;
      return true;
    });
  }, [index, kw, lang, tags]);

  /** 筛选栏标签：按界面语言解析索引条目标签（`resolveRealm` 回退链），只显示计数最多的
   * 前 `MAX_FILTER_TAGS` 个；已选标签始终补显（与切换领域页同口径，保证筛选总可被撤销）。 */
  const shownTags = useMemo(
    () => topRealmTags((index?.realms ?? []).map((r) => resolveRealm(r, lang).tags), tags),
    [index, lang, tags],
  );

  /** 界面语言切换后旧语言标签在新语言下无对应（筛选仍生效但无 chip 可取消），清空选择 */
  useEffect(() => {
    setTags([]);
  }, [lang]);

  const groups = useMemo(
    () => ({
      updatable: list.filter((r) => r.state === "updatable"),
      available: list.filter((r) => r.state === "available"),
      installed: list.filter((r) => r.state === "installed"),
    }),
    [list],
  );

  const doInstall = async (r: RemoteRealm, mode: "error" | "overwrite" | "rename") => {
    setProgress((p) => ({ ...p, [r.id]: { received: 0, total: r.size ?? 0 } }));
    try {
      const res = await installRemoteRealm(r.id, mode);
      await onInstalled();
      await load(false);
      const name = res.realmName || resolveRealm(r, lang).name || r.id;
      showToast("success", t.realm.installOk.replace("{name}", name));
      if (res.missingMods?.length) {
        showToast(
          "error",
          t.realm.installNeedsMods
            .replace("{name}", name)
            .replace("{mods}", res.missingMods.join("、")),
        );
      }
    } catch (e) {
      const msg = backendErrorText(e);
      if (msg.includes(REALM_DOWNLOAD_CANCELLED)) return; // 用户主动取消，不提示
      // 按**码**判断冲突（后端 id_conflict 带 realmId/realmName 负载），不能匹配本地化文案
      if (backendErrorCode(e) === "id_conflict") {
        setConflict(r);
        return;
      }
      showToast("error", msg);
    } finally {
      setProgress((p) => {
        const n = { ...p };
        delete n[r.id];
        return n;
      });
    }
  };

  const doAddSource = async () => {
    try {
      const r = await addRealmRepository(srcInput.trim());
      setRepos(r.urls);
      setSrcInput("");
      setSrcRes({ ok: true, text: "" });
      await load(true);
    } catch (e) {
      setSrcRes({ ok: false, text: backendErrorText(e) });
    }
  };

  const doRemoveSource = async (url: string) => {
    try {
      const r = await removeRealmRepository(url);
      setRepos(r.urls);
      await load(true);
    } catch (e) {
      showToast("error", backendErrorText(e));
    }
  };

  const doTestRepo = async (u: string) => {
    try {
      const r = await testRealmRepository(u);
      setSrcRes({
        ok: true,
        text: t.realm.remoteRepoTestOk
          .replace("{n}", String(r.realmCount))
          .replace("{ms}", String(r.elapsedMs)),
      });
    } catch (e) {
      setSrcRes({
        ok: false,
        text: t.realm.remoteRepoTestFail.replace("{msg}", backendErrorText(e)),
      });
    }
  };

  const toggleTag = (tag: string) => {
    setTags((prev) => (prev.includes(tag) ? prev.filter((x) => x !== tag) : [...prev, tag]));
  };

  const renderCard = (r: RemoteRealm) => {
    const p = progress[r.id];
    const downloading = !!p;
    const pct = p
      ? Math.min(99, Math.round((p.received / Math.max(1, p.total || r.size || 1)) * 100))
      : 0;
    const disabled = r.state === "installed";
    const coverPath = covers[r.id];
    const hue = hueOf(r.id);
    const missing = (r.requiresMods ?? []).filter((m) => !installed.has(m));
    const rl = resolveRealm(r, lang);

    return (
      <div
        key={r.id}
        className={cn(
          "group relative overflow-hidden rounded-xl border border-slate-800 bg-slate-900/50",
          disabled ? "pointer-events-none opacity-50" : "hover:border-slate-700",
        )}
      >
        <div
          className="relative aspect-video w-full"
          style={{
            background: coverPath
              ? undefined
              : `linear-gradient(135deg, hsl(${hue} 55% 32%), hsl(${(hue + 50) % 360} 50% 22%))`,
          }}
        >
          {coverPath ? (
            <img src={coverUrl(coverPath)} alt="" className="h-full w-full object-cover" />
          ) : (
            <div className="grid h-full w-full place-items-center">
              <span className="text-4xl font-extrabold text-white/80">
                {rl.name.slice(0, 1) || r.id.slice(0, 1)}
              </span>
            </div>
          )}
          <span className="absolute bottom-2 left-2 rounded bg-slate-950/75 px-1.5 py-0.5 text-[11px] text-slate-200">
            v{r.version ?? "-"}
          </span>
          {r.size ? (
            <span className="absolute bottom-2 right-2 rounded bg-slate-950/75 px-1.5 py-0.5 text-[11px] text-slate-200">
              {fmtSize(r.size)}
            </span>
          ) : null}
        </div>

        {r.state === "updatable" ? (
          <span className="absolute left-2 top-2 rounded bg-amber-500 px-1.5 py-0.5 text-[11px] font-bold text-slate-900">
            {/* 同版本但内容不同（发布事故兜底，见后端 `realm_update_state`）：
                「from → to」会写成「1.2.0 → 1.2.0」很怪，改说「内容已刷新」。 */}
            {compareModVersions(r.installedVersion ?? "", r.version ?? "") === 0
              ? t.realm.updatableTagContent
              : t.realm.updatableTag
                  .replace("{from}", r.installedVersion ?? "")
                  .replace("{to}", r.version ?? "")}
          </span>
        ) : null}
        {disabled ? (
          <span className="absolute left-2 top-2 rounded bg-slate-600/80 px-1.5 py-0.5 text-[11px] font-bold text-white">
            {t.realm.groupInstalled}
          </span>
        ) : null}
        {!disabled && missing.length > 0 ? (
          <span className="absolute right-2 top-2 rounded bg-amber-500/90 px-1.5 py-0.5 text-[11px] font-bold text-slate-900">
            {t.realm.needExternalMods.replace("{n}", String(missing.length))}
          </span>
        ) : null}
        {!disabled && (r.requiresMods ?? []).length > 0 && missing.length === 0 ? (
          <span className="absolute right-2 top-2 rounded bg-emerald-600/90 px-1.5 py-0.5 text-[11px] font-bold text-white">
            {t.realm.depsReady}
          </span>
        ) : null}

        <div className="px-3 pb-3 pt-2">
          <div className="truncate text-sm font-semibold text-foreground">
            {rl.name || r.id}
          </div>
          <p className="mt-1 line-clamp-2 min-h-[2.5rem] text-xs leading-relaxed text-muted">
            {rl.description}
          </p>
          {(rl.tags).length > 0 ? (
            <div className="mt-2 flex flex-wrap gap-1">
              {rl.tags.map((x) => (
                <span
                  key={x}
                  className="rounded bg-slate-700/50 px-1.5 py-0.5 text-[11px] text-slate-300"
                >
                  #{x}
                </span>
              ))}
            </div>
          ) : null}
        </div>

        <div className="flex items-center gap-2 px-3 pb-3">
          <span className="truncate text-[11px] text-muted">{r.author ?? ""}</span>
          <span className="flex-1" />
          {downloading ? (
            <button
              type="button"
              onClick={() => void cancelRealmDownload(r.id)}
              className="rounded border border-slate-700 px-2 py-1 text-xs text-slate-300 hover:bg-slate-800"
            >
              {t.realm.cancelDownload}
            </button>
          ) : disabled ? (
            <button type="button" disabled className="rounded border border-slate-800 px-2 py-1 text-xs text-slate-500">
              {t.realm.groupInstalled}
            </button>
          ) : r.state === "updatable" ? (
            <button
              type="button"
              onClick={() => setUpdateTarget(r)}
              className="rounded bg-amber-500/20 px-2 py-1 text-xs font-medium text-amber-300 hover:bg-amber-500/30"
            >
              {t.realm.updateRealm}
            </button>
          ) : (
            <button
              type="button"
              onClick={() => void doInstall(r, "error")}
              className="flex items-center gap-1 rounded bg-gradient-to-br from-blue-500 to-indigo-500 px-2.5 py-1 text-xs font-medium text-white"
            >
              <Download className="h-3 w-3" />
              {t.realm.download}
            </button>
          )}
        </div>

        {downloading ? (
          <div className="px-3 pb-3">
            <div className="h-1.5 overflow-hidden rounded bg-slate-700/50">
              <div
                className="h-full rounded bg-gradient-to-r from-blue-500 to-indigo-500 transition-[width]"
                style={{ width: `${pct}%` }}
              />
            </div>
            <div className="mt-1 flex justify-between text-[11px] text-muted">
              <span>{pct}%</span>
              <span>
                {fmtSize(p!.received)} / {fmtSize(p!.total || r.size || 0)}
              </span>
            </div>
          </div>
        ) : null}
      </div>
    );
  };

  const renderGroup = (
    title: string,
    cls: string,
    items: RemoteRealm[],
    note?: string,
  ) => {
    if (items.length === 0) return null;
    return (
      <>
        <div className="flex items-center gap-2 px-8 pt-5 text-sm font-semibold">
          <span className={cn("h-2 w-2 rounded-full", cls)} />
          {title}
          <span className="text-xs font-normal text-muted">{items.length}</span>
          <span className="h-px flex-1 bg-slate-800" />
        </div>
        {note ? <div className="px-8 pt-1 text-xs text-muted">{note}</div> : null}
        {/* 卡片网格：固定 3 列（与切换领域页一致）——列数不随分辨率 / 窗口变化，
            整页缩放下逻辑视口恒为设计画布 1280×720 */}
        <div className="grid grid-cols-3 gap-5 px-8 pt-3">
          {items.map(renderCard)}
        </div>
      </>
    );
  };

  return (
    <div className="flex h-full flex-col">
      {/* 仓库源信息条 */}
      <div className="mx-8 mt-4 flex items-center gap-3 rounded-xl border border-slate-800 bg-slate-900/50 px-4 py-2.5 text-xs">
        <span className="h-2 w-2 rounded-full bg-emerald-500 shadow-[0_0_8px] shadow-emerald-500" />
        <span className="rounded bg-blue-500/20 px-2 py-0.5 text-[11px] font-bold text-blue-200">
          {t.realm.remoteRepoCount.replace("{n}", String(repos.length))}
        </span>
        <span className="min-w-0 flex-1 truncate text-muted" title={repos[0] ?? ""}>
          {repos.length
            ? repos[0] +
              (repos.length > 1
                ? t.realm.remoteRepoMore.replace("{n}", String(repos.length - 1))
                : "")
            : t.realm.remoteReposEmpty}
        </span>
        <button
          type="button"
          onClick={() => {
            setSrcInput("");
            setSrcRes({ ok: true, text: "" });
            setSourceOpen(true);
          }}
          className="grid h-7 w-7 place-items-center rounded-lg border border-slate-700 text-muted hover:bg-slate-800 hover:text-foreground"
          title={t.realm.remoteReposTitle}
        >
          <Settings2 className="h-4 w-4" />
        </button>
        <button
          type="button"
          onClick={() => void load(true)}
          disabled={loading}
          className="grid h-7 w-7 place-items-center rounded-lg border border-slate-700 text-muted hover:bg-slate-800 hover:text-foreground disabled:opacity-50"
          title={t.realm.refresh}
        >
          <RefreshCw className={cn("h-4 w-4", loading && "animate-spin")} />
        </button>
      </div>

      {/* 检索（第一行） */}
      <div className="flex flex-none items-center px-8 pt-4 pb-2.5">
        <input
          value={kw}
          onChange={(e) => setKw(e.target.value)}
          placeholder={t.realm.searchRemote}
          className="w-64 rounded-lg border border-slate-700 bg-slate-900/70 px-3 py-1.5 text-sm placeholder:text-slate-500 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500/50"
        />
      </div>

      {/* 标签筛选（第二行，独立成行）：全量 chips 与搜索框同行时在长标签语言下会溢出换行
          （设计 §9.4）；只显示计数最多的前 6 个，已选标签补显。 */}
      {shownTags.length > 0 ? (
        <div className="flex flex-none flex-wrap items-center gap-1.5 px-8 pb-3">
          <button
            type="button"
            onClick={() => setTags([])}
            className={cn(
              "rounded-full border px-3 py-1 text-xs",
              tags.length === 0
                ? "border-transparent bg-gradient-to-br from-blue-500 to-indigo-500 text-white"
                : "border-slate-700 text-muted hover:bg-slate-800",
            )}
          >
            {t.realm.tagsAll}
          </button>
          {shownTags.map(({ tag, n }) => (
            <button
              key={tag}
              type="button"
              onClick={() => toggleTag(tag)}
              className={cn(
                "rounded-full border px-3 py-1 text-xs",
                tags.includes(tag)
                  ? "border-transparent bg-gradient-to-br from-blue-500 to-indigo-500 text-white"
                  : "border-slate-700 text-muted hover:bg-slate-800",
              )}
            >
              {tag}
              {n > 0 && <span className="ml-1 opacity-60">{n}</span>}
            </button>
          ))}
        </div>
      ) : null}

      {index?.error ? (
        <div className="mx-8 mb-1 rounded-lg border border-amber-500/30 bg-amber-500/10 px-4 py-2 text-xs text-amber-200">
          {t.realm.remoteOffline.replace("{msg}", index.error)}
        </div>
      ) : null}

      <div className="min-h-0 flex-1 overflow-y-auto pb-8">
        {loading && !index ? (
          <div className="grid h-full place-items-center text-sm text-muted">{t.common.loading}</div>
        ) : list.length === 0 ? (
          <div className="grid h-full place-items-center text-sm text-muted">
            {repos.length === 0 ? (
              <div className="flex flex-col items-center gap-3">
                <span>{t.realm.remoteReposNone}</span>
                <button
                  type="button"
                  onClick={() => {
                    setSrcInput("");
                    setSrcRes({ ok: true, text: "" });
                    setSourceOpen(true);
                  }}
                  className="rounded-lg bg-gradient-to-br from-blue-500 to-indigo-500 px-3 py-1.5 text-xs font-medium text-white"
                >
                  {t.realm.remoteRepoAdd}
                </button>
              </div>
            ) : (
              t.realm.remoteEmpty
            )}
          </div>
        ) : (
          <>
            {renderGroup(t.realm.groupUpdatable, "bg-amber-400", groups.updatable)}
            {renderGroup(t.realm.groupAvailable, "bg-blue-500", groups.available)}
            {renderGroup(
              t.realm.groupInstalled,
              "bg-slate-500",
              groups.installed,
              t.realm.groupInstalledNote,
            )}
          </>
        )}
      </div>

      {/* 仓库源管理 */}
      <Modal open={sourceOpen} onClose={() => setSourceOpen(false)} width={520}>
        <h3 className="mb-1 text-base font-semibold">{t.realm.remoteReposTitle}</h3>
        <p className="mb-4 text-xs leading-relaxed text-muted">{t.realm.remoteReposHint}</p>

        {repos.length === 0 ? (
          <div className="mb-4 rounded-lg border border-slate-800 bg-slate-950/40 px-3 py-2 text-xs text-muted">
            {t.realm.remoteReposEmpty}
          </div>
        ) : (
          <ul className="mb-4 max-h-[40vh] space-y-2 overflow-auto">
            {repos.map((u) => (
              <li
                key={u}
                className="flex items-center gap-2 rounded-lg border border-slate-800 bg-slate-900/50 px-3 py-2"
              >
                <span className="min-w-0 flex-1 truncate text-xs text-slate-200" title={u}>
                  {u}
                </span>
                <button
                  type="button"
                  onClick={() => void doTestRepo(u)}
                  className="rounded border border-slate-700 px-2 py-1 text-xs text-slate-300 hover:bg-slate-800"
                >
                  {t.realm.remoteRepoTest}
                </button>
                <button
                  type="button"
                  onClick={() => void doRemoveSource(u)}
                  className="rounded border border-slate-700 px-2 py-1 text-xs text-red-300 hover:bg-red-500/10"
                >
                  {t.realm.remoteRepoRemove}
                </button>
              </li>
            ))}
          </ul>
        )}

        <div className="flex items-center gap-2">
          <input
            value={srcInput}
            onChange={(e) => setSrcInput(e.target.value)}
            placeholder={t.realm.remoteTestPlaceholder}
            className="flex-1 rounded-lg border border-slate-700 bg-slate-950/70 px-3 py-2 text-xs"
          />
          <button
            type="button"
            onClick={() => void doTestRepo(srcInput.trim())}
            className="rounded-lg border border-slate-700 px-3 py-1.5 text-xs text-slate-300 hover:bg-slate-800"
          >
            {t.realm.remoteRepoTest}
          </button>
          <button
            type="button"
            onClick={() => void doAddSource()}
            className="rounded-lg bg-gradient-to-br from-blue-500 to-indigo-500 px-3 py-1.5 text-xs font-medium text-white"
          >
            {t.realm.remoteRepoAdd}
          </button>
        </div>
        <div
          className={cn(
            "mt-2 min-h-[18px] text-xs",
            srcRes.ok ? "text-emerald-400" : "text-red-400",
          )}
        >
          {srcRes.text}
        </div>
      </Modal>

      {/* 覆盖更新确认 */}
      <Modal open={updateTarget !== null} onClose={() => setUpdateTarget(null)} width={480}>
        <h3 className="mb-3 text-base font-semibold">{t.realm.updateConfirmTitle}</h3>
        <p className="text-sm leading-relaxed">
          {/* 同版本换包：不走「覆盖 v1.2.0」话术，改说明本次是内容校对重装 */}
          {compareModVersions(updateTarget?.installedVersion ?? "", updateTarget?.version ?? "") === 0
            ? t.realm.updateConfirmDescContent
            : t.realm.updateConfirmDesc
                .replace("{to}", updateTarget?.version ?? "")
                .replace("{from}", updateTarget?.installedVersion ?? "")}
        </p>
        <p className="mt-2 text-xs leading-relaxed text-amber-300">{t.realm.updateConfirmWarn}</p>
        <div className="mt-5 flex justify-end gap-2">
          <button
            type="button"
            onClick={() => setUpdateTarget(null)}
            className="rounded-lg border border-slate-700 px-3 py-1.5 text-xs text-slate-300 hover:bg-slate-800"
          >
            {t.common.cancel}
          </button>
          <button
            type="button"
            onClick={() => {
              const r = updateTarget;
              setUpdateTarget(null);
              if (r) void doInstall(r, "overwrite");
            }}
            className="rounded-lg bg-amber-500/20 px-3 py-1.5 text-xs font-medium text-amber-300 hover:bg-amber-500/30"
          >
            {t.realm.overwriteBtn}
          </button>
        </div>
      </Modal>

      {/* id 冲突：覆盖 / 自动重命名 */}
      <Modal open={conflict !== null} onClose={() => setConflict(null)} width={480}>
        <h3 className="mb-3 text-base font-semibold">{t.realm.conflictTitle}</h3>
        <p className="text-sm leading-relaxed">
          {t.realm.conflictDesc.replace("{name}", conflict?.id ?? "")}
        </p>
        <div className="mt-5 flex justify-end gap-2">
          <button
            type="button"
            onClick={() => setConflict(null)}
            className="rounded-lg border border-slate-700 px-3 py-1.5 text-xs text-slate-300 hover:bg-slate-800"
          >
            {t.common.cancel}
          </button>
          <button
            type="button"
            onClick={() => {
              const r = conflict;
              setConflict(null);
              if (r) void doInstall(r, "rename");
            }}
            className="rounded-lg border border-slate-700 px-3 py-1.5 text-xs text-slate-300 hover:bg-slate-800"
          >
            {t.realm.renameBtn}
          </button>
          <button
            type="button"
            onClick={() => {
              const r = conflict;
              setConflict(null);
              if (r) void doInstall(r, "overwrite");
            }}
            className="rounded-lg bg-red-500/20 px-3 py-1.5 text-xs font-medium text-red-300 hover:bg-red-500/30"
          >
            {t.realm.overwriteBtn}
          </button>
        </div>
      </Modal>
    </div>
  );
}
