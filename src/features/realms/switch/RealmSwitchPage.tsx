import { useEffect, useMemo, useState } from "react";
import { MoreHorizontal, Globe } from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import { realmDir, realmAssetUrl, listVcmiGameProcesses, listVcmiProcesses, pickNativeLauncherProcs, killVcmi } from "@/lib/tauri";
import { isLauncherNoticeMuted, setLauncherNoticeMuted } from "@/lib/uiPrefs";
import { deleteRealmSafe, useRealms } from "@/lib/realm/store";
import { realmAsset, realmDescription, realmName, realmTags, topRealmTags } from "@/lib/realm/intro";
import type { Realm } from "@/lib/realm/types";
import { ConfirmDialog } from "@/shared/ConfirmDialog";

interface Props {
  /** 点击「进入」：激活并打开该领域（由 App 决定是否播放登场演出）。返回 false 表示未成功。 */
  onEnter: (id: string) => Promise<boolean>;
  /** 点击「新建」 */
  onNew: () => void;
  /** 点击「导入」 */
  onImport: () => Promise<void>;
  /** 点击「远程领域」：进入在线领域列表页 */
  onRemote: () => void;
  /** 点击「调整」 */
  onEdit: (r: Realm) => void;
  /** 提示（覆盖层内统一 Toast） */
  showToast: (type: "success" | "error", msg: string) => void;
}

/** 领域封面：把 `assets/...` 相对路径解析为可显示 URL（§5.4 必须经 convertFileSrc）。 */
function RealmCover({ realmId, rel }: { realmId: string; rel?: string }) {
  const [src, setSrc] = useState("");
  useEffect(() => {
    if (!rel) return;
    let alive = true;
    realmDir(realmId)
      .then((root) => {
        if (alive) setSrc(realmAssetUrl(root, rel));
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [realmId, rel]);
  if (!src) {
    return (
      <div className="grid h-full w-full place-items-center bg-slate-800/60 text-[11px] text-slate-500">
        —
      </div>
    );
  }
  return (
    <img
      src={src}
      alt=""
      className="h-full w-full object-cover"
      draggable={false}
      /* 文件缺失时回退到上面的「—」灰块占位（而不是 broken 图标）：
         清空 `src` 即复用已存在的 `!src` 分支。 */
      onError={() => setSrc("")}
    />
  );
}

/**
 * 切换领域页（§9.2.4 / §9.4）：领域列表 + 搜索 + 新建 / 导入 + 两种空态。
 *
 * 卡片为 16:9 整卡封面（无封面则回退壁纸，均无则显示占位），名称与描述
 * 叠放在卡片底部的渐变蒙层上（类似模组页的城镇卡片）；`description` 仅在本页显示；
 * 操作按钮右上角浮动；「调整」仅在有编辑权（`editable`）时显示（§6.2），删除走「更多」菜单。
 */
export function RealmSwitchPage({ onEnter, onNew, onImport, onRemote, onEdit, showToast }: Props) {
  const { t } = useI18n();
  const { realms, activeRealm } = useRealms();
  const [kw, setKw] = useState("");
  const [tags, setTags] = useState<string[]>([]);
  const [moreId, setMoreId] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState<Realm | null>(null);
  /** 进程守卫拦下的「进入」请求（含具体进程名，供弹窗展示与「结束游戏」后重试） */
  const [blocked, setBlocked] = useState<{ realm: Realm; procs: string[] } | null>(null);
  /** 软提示：**只有官方启动器**在运行时的一次性提醒（不阻断，勾选「忽略提醒」后不再出现） */
  const [notice, setNotice] = useState<{ realm: Realm; procs: string[] } | null>(null);
  /** 软提示里的「忽略提醒」勾选（每次打开重置，确认时才落盘到 localStorage） */
  const [muteNotice, setMuteNotice] = useState(false);

  const lang = t.uiLang;
  /** 筛选栏标签（多选筛选，OR 关系）：按**界面语言**解析逐语言标签
   * （与远程领域页同口径，`realmTags` 回退链；顶层仅作兜底），只显示计数最多的
   * 前 `MAX_FILTER_TAGS` 个；已选标签始终补显（保证筛选总可被撤销）。 */
  const shownTags = useMemo(
    () => topRealmTags(realms.map((r) => realmTags(r, lang)), tags),
    [realms, lang, tags],
  );

  /** 界面语言切换后旧语言标签在新语言下无对应（筛选仍生效但无 chip 可取消），清空选择 */
  useEffect(() => {
    setTags([]);
  }, [lang]);

  const list = useMemo(() => {
    const k = kw.trim().toLowerCase();
    return realms.filter((r) => {
      if (k) {
        // 领域搜索：对「名称 + 描述」模糊搜索，覆盖顶层与全部语言（§9.4）
        const hay = [
          r.name ?? "",
          r.description ?? "",
          ...Object.values(r.i18n ?? {}).flatMap((e) => [e.name ?? "", e.description ?? ""]),
          r.id,
        ]
          .join(" ")
          .toLowerCase();
        if (!hay.includes(k)) return false;
      }
      if (tags.length && !realmTags(r, lang).some((x) => tags.includes(x))) return false;
      return true;
    });
  }, [realms, kw, lang, tags]);

  const nameOf = (r: Realm) => realmName(r, lang);
  const descOf = (r: Realm) => realmDescription(r, lang);
  /** 卡片封面：缩略图优先，缺失时回退封面图（§9.4） */
  // 媒体语言无关：封面 / 壁纸都只存顶层一份，无需按界面语言取值
  const coverOf = (r: Realm) => realmAsset(r, "cover") || realmAsset(r, "wallpaper");

  /**
   * 进程守卫（§4.4 **分级**）：只有**游戏本体**（client / server / mapeditor）在运行时
   * 才阻断切换 —— 它们持有存档与游戏状态，运行中改写 `dirs.json` 会把存档 / 设置写到
   * 切换前的目录。官方启动器 `VCMI_launcher.exe` **不阻断**（可共存）：它的路径在自身
   * 启动时已快照，切换领域不会破坏它已打开的数据，只是它之后的改动可能落在切换前的目录。
   *
   * ⚠️ 这里**取具体进程名**再弹窗，而不是只弹一句提示：
   * 只说"VCMI 正在运行"用户无从判断该关掉什么 —— 实测最常见的是**上一次启动后残留**的
   * `VCMI_client.exe`（游戏已退出但进程没退干净），也有可能是启动器自己（旧实现用
   * `tasklist` 全文 `contains` 判定，会把自己算进去；现在后端已改为精确匹配 + 排除自身）。
   *
   * 只开官方启动器时走**软提示**（不阻断）：它不持有游戏状态，但路径快照让它之后的改动
   * 落在切换前的目录，需要玩家知情；勾「忽略提醒」后写 localStorage，后续直接进。
   */
  const handleEnter = async (r: Realm) => {
    const procs = await listVcmiGameProcesses();
    if (procs.length > 0) {
      setBlocked({ realm: r, procs });
      return;
    }
    if (isLauncherNoticeMuted()) {
      await enterNow(r);
      return;
    }
    const launcher = pickNativeLauncherProcs(await listVcmiProcesses());
    if (launcher.length > 0) {
      setMuteNotice(false);
      setNotice({ realm: r, procs: launcher });
      return;
    }
    await enterNow(r);
  };

  /** 真正进入（守卫通过后，或用户选择「结束游戏」并结束后） */
  const enterNow = async (r: Realm) => {
    const ok = await onEnter(r.id);
    if (!ok) showToast("error", t.realm.enterFailed);
  };

  const handleDelete = async () => {
    const target = confirmDelete;
    setConfirmDelete(null);
    if (!target) return;
    if (target.id === activeRealm) {
      showToast("error", t.realm.deleteActiveForbidden);
      return;
    }
    await deleteRealmSafe(target.id);
    showToast("success", t.realm.deleteOk);
  };

  return (
    <div className="flex h-full flex-col">
      {/* 工具栏第一行：搜索 + 新建 / 导入 / 远程领域（§9.2.4：空态下也必须保持可用） */}
      <div className="flex flex-none items-center gap-2 px-8 pt-4 pb-2.5">
        <input
          value={kw}
          onChange={(e) => setKw(e.target.value)}
          placeholder={t.realm.searchRealms}
          className="w-64 rounded-lg border border-slate-700 bg-slate-900/70 px-3 py-1.5 text-sm text-foreground placeholder:text-slate-500 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500/50"
        />
        <span className="flex-1" />
        <button
          type="button"
          onClick={onNew}
          className="rounded-lg bg-gradient-to-br from-blue-500 to-indigo-500 px-3 py-1.5 text-sm font-medium text-white shadow-md shadow-blue-600/40 hover:opacity-90"
        >
          {t.realm.newRealm}
        </button>
        <button
          type="button"
          onClick={onImport}
          className="rounded-lg border border-slate-700 px-3 py-1.5 text-sm text-slate-300 hover:bg-slate-800/50"
        >
          {t.realm.importRealm}
        </button>
        <button
          type="button"
          onClick={onRemote}
          className="rounded-lg border border-emerald-600/50 bg-emerald-500/10 px-3 py-1.5 text-sm text-emerald-300 hover:bg-emerald-500/20"
        >
          {t.realm.remoteRealm}
        </button>
      </div>

      {/* 工具栏第二行：标签筛选独立成行——全量 chips 与操作按钮挤一行时，俄 / 德 / 波等
          长标签语言下会溢出换行（设计 §9.4）；独立行后全宽 1216px 有保底，
          `flex-wrap` 仅作极端兜底。 */}
      {shownTags.length > 0 ? (
        <div className="flex flex-none flex-wrap items-center gap-1.5 px-8 pb-3">
          <button
            type="button"
            onClick={() => setTags([])}
            className={cn(
              "rounded-full border px-3 py-1 text-xs",
              tags.length === 0
                ? "border-transparent bg-gradient-to-br from-blue-500 to-indigo-500 text-white"
                : "border-slate-700 text-slate-400 hover:bg-slate-800/50",
            )}
          >
            {t.realm.tagsAll}
          </button>
          {shownTags.map(({ tag, n }) => (
            <button
              key={tag}
              type="button"
              onClick={() =>
                setTags((prev) =>
                  prev.includes(tag) ? prev.filter((x) => x !== tag) : [...prev, tag],
                )
              }
              className={cn(
                "rounded-full border px-3 py-1 text-xs",
                tags.includes(tag)
                  ? "border-transparent bg-gradient-to-br from-blue-500 to-indigo-500 text-white"
                  : "border-slate-700 text-slate-400 hover:bg-slate-800/50",
              )}
            >
              {tag}
              {n > 0 && <span className="ml-1 opacity-60">{n}</span>}
            </button>
          ))}
        </div>
      ) : null}

      <div className="min-h-0 flex-1 overflow-y-auto px-8 pb-8">
        {/* 空态一：还没有任何领域（§9.2.4）：插画 + 说明 + 两个主行动按钮 */}
        {realms.length === 0 ? (
          <div className="flex h-full flex-col items-center justify-center gap-4 text-center">
            {/* 空态插画（§9.2.4 列表区第一项） */}
            <div className="grid h-20 w-20 place-items-center rounded-2xl bg-gradient-to-br from-indigo-500/30 to-blue-500/20 text-blue-300 shadow-inner ring-1 ring-blue-400/30">
              <Globe className="h-10 w-10" />
            </div>
            <p className="text-lg font-medium text-foreground">{t.realm.empty}</p>
            <p className="max-w-md text-sm text-muted">{t.realm.emptySub}</p>
            <div className="mt-2 flex items-center gap-3">
              <button
                type="button"
                onClick={onNew}
                className="rounded-lg bg-gradient-to-br from-blue-500 to-indigo-500 px-4 py-2 text-sm font-medium text-white shadow-md shadow-blue-600/40 hover:opacity-90"
              >
                {t.realm.createRealm}
              </button>
              <button
                type="button"
                onClick={onImport}
                className="rounded-lg border border-slate-700 px-4 py-2 text-sm text-slate-300 hover:bg-slate-800/50"
              >
                {t.realm.importRealm}
              </button>
            </div>
          </div>
        ) : /* 空态二：搜索无结果（与空态是不同文案，§9.2.4） */
        list.length === 0 ? (
          <div className="flex h-full flex-col items-center justify-center gap-3 text-center">
            <p className="text-base text-foreground">
              {t.realm.noMatch.replace("{kw}", kw.trim())}
            </p>
            <button
              type="button"
              onClick={() => setKw("")}
              className="rounded-lg border border-slate-700 px-3 py-1.5 text-sm text-slate-300 hover:bg-slate-800/50"
            >
              {t.realm.clearSearch}
            </button>
          </div>
        ) : (
          /* 卡片网格：**固定 3 列**（§9.4）。不用 auto-fill 自适应——整页缩放（`installUiScale`）
             使各分辨率下逻辑视口恒为设计画布 1280×720，固定 3 列 ⇒ 列数与卡片比例在所有屏幕
             一致；自适应反而会在超宽屏（21:9）/ 小于画布的小窗口跳列数。 */
          <div className="grid grid-cols-3 gap-5">
            {list.map((r) => {
              const isActive = activeRealm === r.id;
              return (
                <div
                  key={r.id}
                  className={cn(
                    "group relative aspect-video w-full overflow-hidden rounded-xl border",
                    isActive
                      ? "border-blue-500/70 ring-1 ring-blue-500/30"
                      : "border-slate-800 hover:border-slate-700",
                  )}
                >
                  {/* 封面铺满整卡（§9.4：16:9，类似模组页的城镇卡片） */}
                  <RealmCover realmId={r.id} rel={coverOf(r)} />

                  {/* 底部黑透蒙层 + 名称 / 描述（description 仅在本页显示）。
                      文本区用**均一半透明黑底**而非纯渐变：纯渐变在名称 / 描述所处高度黑度不足
                      （≈50~70%），亮背景（雪地 / 白云）的高频细节下白字仍难读；
                      上缘再以 40px 同色渐变过渡，消除蒙层硬边。 */}
                  <div className="pointer-events-none absolute inset-x-0 bottom-0 h-10 bg-gradient-to-t from-black/65 to-transparent" />
                  <div className="absolute inset-x-0 bottom-0 bg-black/65 px-3 pb-2.5 pt-2">
                    <div className="truncate text-sm font-semibold text-white" title={nameOf(r)}>
                      {nameOf(r)}
                    </div>
                    <p className="line-clamp-2 min-h-[2.5rem] text-xs leading-relaxed text-slate-200">
                      {descOf(r)}
                    </p>
                  </div>

                  {isActive && (
                    <span className="absolute left-2 top-2 rounded bg-blue-500/90 px-1.5 py-0.5 text-[11px] font-medium text-white">
                      {t.realm.active}
                    </span>
                  )}
                  {/* 右上角浮动操作：调整 / 更多 / 进入（§9.4） */}
                  <div className="absolute right-2 top-2 flex items-center gap-1">
                    {r.editable !== false && (
                      <button
                        type="button"
                        onClick={() => onEdit(r)}
                        className="rounded bg-slate-900/85 px-2 py-1 text-xs text-slate-200 backdrop-blur-sm hover:bg-slate-800"
                      >
                        {t.realm.adjust}
                      </button>
                    )}
                    <div className="relative">
                      <button
                        type="button"
                        onClick={(e) => {
                          e.stopPropagation();
                          setMoreId(moreId === r.id ? null : r.id);
                        }}
                        className="flex h-7 w-7 items-center justify-center rounded bg-slate-900/85 text-slate-200 backdrop-blur-sm hover:bg-slate-800"
                        title={t.realm.more}
                      >
                        <MoreHorizontal className="h-4 w-4" />
                      </button>
                      {moreId === r.id && (
                        <div className="absolute right-0 top-8 z-10 w-28 overflow-hidden rounded-lg border border-slate-700 bg-slate-900 shadow-xl">
                          <button
                            type="button"
                            onClick={() => {
                              setMoreId(null);
                              setConfirmDelete(r);
                            }}
                            className="w-full px-3 py-2 text-left text-xs text-red-400 hover:bg-red-500/10"
                          >
                            {t.realm.deleteRealm}
                          </button>
                        </div>
                      )}
                    </div>
                    <button
                      type="button"
                      onClick={() => handleEnter(r)}
                      className="rounded bg-slate-900/85 px-2 py-1 text-xs text-slate-200 backdrop-blur-sm hover:bg-slate-800"
                    >
                      {t.realm.activate}
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </div>

      <ConfirmDialog
        open={confirmDelete !== null}
        title={t.realm.deleteRealm}
        description={t.realm.deleteConfirm}
        confirmText={t.common.ok}
        cancelText={t.common.cancel}
        danger
        onCancel={() => setConfirmDelete(null)}
        onConfirm={handleDelete}
      />

      {/* 进程守卫（§4.4）：报出**具体进程名**，并允许直接结束游戏后重试进入
          （「结束游戏」只结束游戏本体，不动官方启动器） */}
      <ConfirmDialog
        open={blocked !== null}
        title={t.realm.vcmiRunningTitle}
        description={
          blocked
            ? `${t.realm.vcmiRunningHint}\n\n${t.realm.vcmiRunningProcesses.replace("{list}", blocked.procs.join("、"))}`
            : undefined
        }
        confirmText={t.realm.killVcmi}
        cancelText={t.common.cancel}
        onCancel={() => setBlocked(null)}
        onConfirm={() => {
          const target = blocked;
          setBlocked(null);
          if (!target) return;
          void (async () => {
            await killVcmi();
            // `tasklist` 有延迟：等进程真正退干净再进，否则守卫会再次命中
            window.setTimeout(() => void enterNow(target.realm), 600);
          })();
        }}
      />

      {/* 官方启动器仍在运行（§4.4 分级守卫）：**不阻断**，只说明它仍指向切换前的配置目录；
          勾「忽略提醒」写 localStorage，之后不再弹（见 `lib/uiPrefs`） */}
      <ConfirmDialog
        open={notice !== null}
        title={t.realm.launcherRunningTitle}
        description={
          notice
            ? `${t.realm.launcherRunningHint}\n\n${t.realm.vcmiRunningProcesses.replace("{list}", notice.procs.join("、"))}`
            : undefined
        }
        confirmText={t.realm.enterAnyway}
        cancelText={t.common.cancel}
        onCancel={() => setNotice(null)}
        onConfirm={() => {
          const target = notice;
          setNotice(null);
          if (!target) return;
          if (muteNotice) setLauncherNoticeMuted(true);
          void enterNow(target.realm);
        }}
      >
        <label className="mb-3 flex cursor-pointer items-center gap-2 text-xs text-slate-300">
          <input
            type="checkbox"
            checked={muteNotice}
            onChange={(e) => setMuteNotice(e.target.checked)}
            className="h-3.5 w-3.5 accent-blue-500"
          />
          {t.realm.muteLauncherNotice}
        </label>
      </ConfirmDialog>
    </div>
  );
}

