import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowLeft } from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import {
  launchGame,
  realmConfigDir,
  realmDir,
  realmAssetUrl,
  checkRealmModsReady,
  pickOpenPath,
  importRealm,
  RealmImportError,
  getBgmMuted,
  setBgmMuted,
  buildRealmBundledWiki,
} from "@/lib/tauri";
import { refreshRealms, useRealms } from "@/lib/realm/store";
import { realmAsset, realmName } from "@/lib/realm/intro";
import type { Realm } from "@/lib/realm/types";
import { PreLaunchCheckDialog } from "@/features/prelaunch/PreLaunchCheckDialog";
import {
  usePreLaunchChecks,
  type PreLaunchStep,
} from "@/features/prelaunch/usePreLaunchChecks";
import { makeLangPackStep } from "@/features/prelaunch/LangPackStepCard";
import { WindowControls } from "@/shared/WindowControls";
import { LanguageMenuButton } from "@/shared/LanguageMenuButton";
import { SettingsView } from "@/features/settings/SettingsView";
import { ConfirmDialog } from "@/shared/ConfirmDialog";
import { Toast } from "@/shared/Toast";
import { RealmSwitchPage } from "@/features/realms/switch/RealmSwitchPage";
import { RealmRepositoryPage } from "@/features/realms/repository/RealmRepositoryPage";
import { RealmEditor } from "@/features/realms/editor/RealmEditor";
import { ShowcaseOverlay } from "@/features/realms/showcase/ShowcaseOverlay";
import { CreateRealmDialog } from "@/features/realms/switch/CreateRealmDialog";
import { PageHeaderBar } from "./PageHeaderBar";
import { HomePage } from "./HomePage";
import { InfoBody } from "./InfoBody";
import { RealmAssetsPage } from "./RealmAssetsPage";
import { backendErrorText } from "@/lib/backendError";

type RealmPage =
  | "home"
  | "info"
  /** 领域素材准备页（S1）：缺素材时先建再进，见 `RealmAssetsPage` */
  | "assets"
  | "switch"
  | "editor"
  | "settings"
  | "repository"
  | "showcase";

interface Props {
  /** 当前激活领域；为 null（未进入任何领域）时只能停留在「切换领域」页 */
  realm: Realm | null;
  /** 覆盖层整体淡出（由 App 控制） */
  hidden?: boolean;
  /** 进入某领域（激活 + 打开；由 App 决定是否播放登场演出）。返回 false 表示未成功。 */
  onEnter: (id: string) => Promise<boolean>;
  /** 退出领域：App 负责还原并关闭覆盖层 */
  onExitRealm: () => Promise<void>;
  /**
   * 关闭覆盖层回到启动器主界面（**未激活任何领域**时「切换领域」页的返回出口）。
   * 与 `onExitRealm` 的区别：后者要跑停用流程（还原 dirs.json / 卸联接），
   * 这里只是关掉浏览页，没有任何持久化副作用。
   */
  onExitToLauncher?: () => void;
  /**
   * 前往「模组」页（语言包预检弹窗里「前往模组管理」的引导出口）。
   * 领域内没有模组管理页，因此由 App 实现为「退出领域 + 切到模组页」。
   */
  onGoToMods?: () => void;
  /** 重播登场演出 */
  onReplayIntro: (r: Realm) => void;
  /** 登场演出是否正在播放（叠加在领域主界面之上）。为 true 时暂停主界面背景音乐，结束后恢复 */
  introActive?: boolean;
  /** 预览模式（B5）：展示领域主界面但不进入领域，由「返回编辑」退出 */
  preview?: boolean;
  onClosePreview?: () => void;
  /** 有新版本可用：领域主界面 ⚙ 显示红点（仅提示；⚙ 点击行为不变，仍是领域设置页） */
  updateBadge?: boolean;
  /**
   * 透传「本领域启用的模组集合」（编辑器预览用）。编辑期改动尚未落盘，
   * 阵营巡礼若自己去读 `modSettings.json` 会读到旧值/读不到 → 退化成全量城镇。
   */
  enabledIds?: Set<string> | null;
}

/**
 * 领域全屏覆盖层 RealmShell（§9.2.0）。
 *
 * - 在 `App.tsx` **根层**渲染，不套用 `pt-[60px] pb-[64px]`。
 * - 承载 5 个页面：领域主界面 / 领域信息 / 切换领域 / 新建·编辑领域 / 领域设置。
 * - 控件分档（§9.2.0）：主界面 ⚙─✕；其余二级页面 ─ ✕；登场演出无控件（由 App 单独渲染）。
 */
export function RealmShell({
  realm: realmProp,
  hidden,
  onEnter,
  onExitRealm,
  onExitToLauncher,
  onGoToMods,
  onReplayIntro,
  introActive,
  preview,
  onClosePreview,
  updateBadge,
  enabledIds,
}: Props) {
  const { t, vcmiLang } = useI18n();
  const store = useRealms();
  /**
   * 以 store 中的最新领域为准。App 传入的是「进入领域时的快照」，编辑器保存后
   * 不会自动更新，壁纸 / 背景音乐 / 游戏名等改动就看不到。预览模式例外——
   * 它要显示尚未保存的编辑结果，不能让 store 里的旧值覆盖。
   */
  const realm =
    realmProp && !preview
      ? (store.realms.find((r) => r.id === realmProp.id) ?? realmProp)
      : realmProp;
  const [page, setPage] = useState<RealmPage>("home");
  /** 「领域信息 / 阵营巡礼」的**素材守门**目标页（S1）：先过准备页，就绪后跳回这里 */
  const [assetsTarget, setAssetsTarget] = useState<"info" | "showcase">("info");
  // 领域目录：用于把 `assets/...` 解析为可加载 URL（§5.4 的壁纸 / 背景音乐）
  const [root, setRoot] = useState("");
  const [editorOpen, setEditorOpen] = useState(false);
  const [editing, setEditing] = useState<Realm | null>(null);
  /** 编辑器注册的返回处理（含未保存确认），供悬浮页头的 editor case 触发（v1.16） */
  const editorBackRef = useRef<(() => void) | null>(null);
  // 新建领域：「先定 ID 再编辑」——弹窗提交后才真正建目录并进入编辑器
  const [createOpen, setCreateOpen] = useState(false);
  const [settingsDir, setSettingsDir] = useState<string | null>(null);
  const [phase, setPhase] = useState<"checking" | "launching" | null>(null);
  /** 游戏已启动：在领域主界面之上显示「启动中」弹出层（§9.2.3）。
   * 不再淡出整个覆盖层（否则会露出底下的 Mods 页），期间暂停背景音乐。 */
  const [launching, setLaunching] = useState(false);
  // 启动前问题（§9.2.3）：bundled 缺失 / borrowed 缺失 / 冲突，分别给不同出口
  const [launchIssue, setLaunchIssue] = useState<{
    kind: "bundled" | "borrowed" | "conflict";
    items: string[];
  } | null>(null);
  const [launchError, setLaunchError] = useState<string | null>(null);
  /** 启动前检测编排（通用 step 框架） */
  const prelaunch = usePreLaunchChecks();
  const [toast, setToast] = useState<{ type: "success" | "error" | "info"; msg: string } | null>(
    null,
  );
  const [conflict, setConflict] = useState<{ realmId: string; realmName: string } | null>(null);
  const [importPath, setImportPath] = useState<string | null>(null);

  /**
   * 登场演出结束 → 若作者开启「演出后直接进入阵营巡礼」，自动跳一次。
   *
   * 判据是 `introActive` **由 true 变 false**（而不是"它现在是 false"）：
   * 没有登场内容的领域 `introActive` 恒为 false，若只看当前值就会一进领域就跳走。
   * 中途 ESC / 点「跳过」同样算"演出结束" → 一并跳转（作者的意图就是"别停在我这"）。
   */
  const introPlayedRef = useRef(false);
  useEffect(() => {
    if (introActive) {
      introPlayedRef.current = true;
      return;
    }
    if (!introPlayedRef.current) return;
    introPlayedRef.current = false;
    if (!realm || preview) return;
    // 缺 `showcase` 字段（早于该功能的老领域）一律视为未开启，不自动进入
    if (!realm.showcase || realm.showcase.enabled === false) return;
    if (realm.showcase?.afterIntro !== true) return;
    // 同样受**素材守门**保护（S1）：缺素材时先进准备页，就绪后自动进入巡礼
    setAssetsTarget("showcase");
    setPage("assets");
  }, [introActive, realm, preview]);

  // 进入 / 切换到另一个领域后，页面要回到领域主界面（§9.4）。
  // 否则从「切换领域」页点「进入」时 `page` 仍停留在 "switch"，
  // 表现为只把领域标记为激活、界面却停在切换页。
  useEffect(() => {
    setPage("home");
  }, [realm?.id]);

  // 未进入任何领域时，默认停留在「切换领域」页；但「新建领域」编辑器可在无领域时打开
  const curPage: RealmPage = realm
    ? page
    : editorOpen && page === "editor"
      ? "editor"
      : page === "repository"
        ? "repository"
        : "switch";

  const showToast = (type: "success" | "error" | "info", msg: string) => {
    setToast({ type, msg });
  };

  // 领域目录（§3.1）：壁纸 / 背景音乐等素材相对它存放，需经 convertFileSrc 才能加载（§5.4）
  useEffect(() => {
    if (!realm) {
      setRoot("");
      return;
    }
    let alive = true;
    realmDir(realm.id)
      .then((d) => alive && setRoot(d))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [realm?.id]);

  /** 真正启动 VCMI（§9.2.3 阶段 3：文案改「正在启动 VCMI…」，成功后覆盖层淡出）。 */
  const doLaunch = async () => {
    setPhase("launching");
    const r = await launchGame();
    setPhase(null);
    if (r.ok) {
      setLaunching(true);
      return;
    }
    // 阶段 4：启动失败 → 模态框显示具体原因 + 「重试」
    setLaunchError(backendErrorText(r.error) || t.realm.launchFailedMsg);
  };

  // 弹窗内动作的统一收尾：**先关弹窗再执行**。
  // ① 启动游戏后弹窗必须消失（否则游戏关掉回来仍停在"启动前检测"）；
  // ② 「前往模组管理」会退出领域并切页，弹窗若不关会以 z-150 遮罩盖住新页面，看起来"点了没反应"。
  const dismissLangCheckAndProceed = () => {
    prelaunch.close();
    void doLaunch();
  };
  const dismissLangCheckAndGoToMods = () => {
    prelaunch.close();
    onGoToMods?.();
  };

  // 启动前检测步骤（当前仅语言包一项；未来依赖完整性/版本匹配直接在此追加 step）
  const prelaunchSteps: PreLaunchStep[] = useMemo(
    () => [
      makeLangPackStep({
        label: t.launch.stepLangPack,
        vcmiLang,
        realmId: realm?.id ?? null,
        // 语言包统一装到**全局** mod 目录：语言层由启动器在启动前保证可见（必要时在领域隔离目录
        // 建临时联接），无需领域私有副本，也不写领域数据 —— 多领域共享一份。
        realmName: realm ? realmName(realm, t.uiLang) : undefined,
        onProceed: dismissLangCheckAndProceed,
        onGoToMods: dismissLangCheckAndGoToMods,
        onBusyChange: prelaunch.setBusy,
      }),
    ],
    // 闭包已捕获最新 dismiss/doLaunch；仅依赖影响 step 语义的入参
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [vcmiLang, realm, t.launch.stepLangPack, t.uiLang, prelaunch.setBusy],
  );

  /**
   * 启动游戏（§9.2.3）：点击 → loading「正在检查…」→ 按 bundled / borrowed / 冲突
   * 分别校验 → 仍有问题则弹模态框（**必须给「仍然启动」逃生口**）→ 否则直接启动。
   */
  const handlePlay = async () => {
    if (!realm) return;
    // ⚠️ 预览模式**绝不能真启动游戏**：
    // ① 预览的承诺是"仅展示、不产生改动"（`previewNotice`），而启动游戏会真的拉起源进程；
    // ② 更隐蔽的后果：`VCMI_client.exe` / `VCMI_server.exe` 一旦跑起来，随后从「切换领域」
    //    进入任何领域都会被**进程守卫**拦下，弹「VCMI 正在运行」——
    //    用户以为是"刚编辑完配置导致的"，实际是预览时误点了启动游戏。
    if (preview) {
      showToast("info", t.realm.previewNoLaunch);
      return;
    }
    setPhase("checking");
    const res = await checkRealmModsReady(realm.id);
    if (res.conflicts.length > 0) {
      setPhase(null);
      setLaunchIssue({ kind: "conflict", items: res.conflicts.map((c) => c.id) });
      return;
    }
    if (res.missingBundled.length > 0) {
      setPhase(null);
      setLaunchIssue({ kind: "bundled", items: res.missingBundled });
      return;
    }
    if (res.missingBorrowed.length > 0) {
      setPhase(null);
      setLaunchIssue({ kind: "borrowed", items: res.missingBorrowed });
      return;
    }
    // 硬校验全过 → 进入启动前检测（通用 step 框架：语言包等；本地扫描即报告先行）
    const pre = await prelaunch.run(prelaunchSteps);
    setPhase(null);
    if (pre === "proceed") {
      await doLaunch();
    }
    // 否则弹窗已开（报告先行，按钮恢复常态），由弹窗接管进度
  };

  /** 导入领域快照（§7.2）：先以 "error" 尝试，遇 id 冲突弹窗让用户选择。 */
  const finishImport = async (path: string, kind: "error" | "overwrite" | "rename") => {
    const res = await importRealm(path, kind);
    await refreshRealms();
    // 整合模组随包解包、不在全局已安装列表里：把它们的资料补进百科缓存
    // （否则领域信息页拿不到描述、整合的城镇也没有预览图）。不 await，避免导入等待构建。
    if (res.bundledMods?.length) {
      void buildRealmBundledWiki(res.bundledMods);
    }
    if (res.needsMods) {
      showToast(
        "error",
        t.realm.importNeedsMods
          .replace("{name}", res.realmName)
          .replace("{mods}", res.missingMods.join("、")),
      );
    } else {
      showToast("success", t.realm.importOk.replace("{name}", res.realmName));
    }
    // 同名提示（§9.3.3）：重名会共用百科缓存导致资料串味。不阻断导入，
    // 且延后到上一条 toast 结束再显示，避免互相覆盖。
    if (res.duplicateMods?.length) {
      const mods = res.duplicateMods.join("、");
      window.setTimeout(() => {
        showToast("error", t.realm.importDupMods.replace("{mods}", mods));
      }, 2800);
    }
  };

  const handleImport = async () => {
    const p = await pickOpenPath([{ name: t.realm.exportFilter, extensions: ["vcmirealm"] }]);
    if (!p) return;
    try {
      await finishImport(p, "error");
    } catch (e) {
      if (e instanceof RealmImportError && e.code === "id_conflict") {
        setImportPath(p);
        setConflict({ realmId: e.realmId || "", realmName: e.realmName || "" });
        return;
      }
      showToast("error", t.realm.error.replace("{msg}", backendErrorText(e)));
    }
  };

  const openSettings = async () => {
    if (!realm) return;
    const dir = await realmConfigDir(realm.id);
    if (!dir) {
      showToast("error", t.realm.settingsDirMissing);
      return;
    }
    setSettingsDir(dir);
    setPage("settings");
  };

  /** 切换当前领域背景音乐开关（仅对当前领域有效、不随导出走，写入 `.bgmMuted`）。
   * 先乐观更新本地状态立即停/续播，再落盘到当前启动器。 */
  const handleToggleBgm = async () => {
    if (!realm) return;
    const next = !bgmMuted;
    setBgmMutedState(next);
    try {
      await setBgmMuted(realm.id, next);
    } catch {
      // 落盘失败则回滚本地状态
      setBgmMutedState(!next);
    }
  };

  /**
   * 进入「领域信息」/「阵营巡礼」前先过**素材守门**（S1）：
   * 记住目标页 → 进准备页；`RealmAssetsPage` 会先做抽样就绪检查，
   * **已就绪时立刻跳转**（用户感知为直接进入，不会闪进度条）。
   */
  const openWithAssets = (target: "info" | "showcase") => {
    setAssetsTarget(target);
    setPage("assets");
  };

  // 控件分档：只有领域主界面有「设置」（§9.2.0）
  const showSettingsBtn = curPage === "home";

  // 壁纸渲染在覆盖层根部（铺满整个窗口，含顶部控件区），而非 HomePage 内部——
  // 否则 body 的 pt-10 让位区会露出一条黑边。仅领域主界面显示。
  const wallpaperSrc = realm ? realmAssetUrl(root, realmAsset(realm, "wallpaper")) : "";

  // 背景音乐（§6 `assets.music` / `main.loopMusic`）：提升到覆盖层根部常驻渲染。
  // 领域信息 / 设置等二级页会替换掉 HomePage，若 audio 留在 HomePage 内会随其卸载而中断；
  // 背景音乐不属于某个页面，而属于整个领域覆盖层——切页不中断，回到主界面继续播放。
  const musicUrl = realm ? realmAssetUrl(root, realmAsset(realm, "music")) : "";
  const loopMusic = realm?.main?.loopMusic !== false;
  // 玩家运行时开关：仅对当前领域有效、不随 realm 导出走，状态存于点文件 `.bgmMuted`。
  // 进入/切换领域时从当前启动器读取（本地状态，避免 realm.json 被用户态污染）。
  const [bgmMuted, setBgmMutedState] = useState(false);
  useEffect(() => {
    if (!realm) {
      setBgmMutedState(false);
      return;
    }
    let alive = true;
    getBgmMuted(realm.id)
      .then((m) => alive && setBgmMutedState(m))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [realm?.id]);
  const bgmRef = useRef<HTMLAudioElement | null>(null);
  useEffect(() => {
    const a = bgmRef.current;
    if (!a) return;
    a.loop = loopMusic;
    // 关闭背景音乐 / 登场演出播放中 / 游戏启动中：暂停；否则播放。
    // 三者结束后自动恢复（避免与 introMusic / introVideo 或游戏声音重叠）。
    //
    // ⚠️ **不再**因为"进入了阵营巡礼"而暂停：巡礼会**接管同一个元素**（`realmBgmRef`）——
    //    要么接续播放（巡礼要用领域 BGM 时），要么由它自己淡出暂停（要用阵营音乐 / 要安静时）。
    //    若这里 pause + 巡礼另起一个元素，主题曲就会**从头开始**（实测反馈）。
    //    `curPage` 仍留在依赖里：退出巡礼时本 effect 重跑 → 自动恢复播放（从同一时间线继续）。
    if (bgmMuted || introActive || launching) a.pause();
    else a.play().catch(() => {});
  }, [musicUrl, loopMusic, bgmMuted, introActive, launching, curPage]);
  const bgm = musicUrl ? (
    <audio ref={bgmRef} src={musicUrl} loop={loopMusic} className="hidden" />
  ) : null;

  // 主界面背景图片滤镜（主题化，作者设定）：合成 CSS filter 套到壁纸 <img>（编辑器预览 / 实际主界面共用）
  const bgFilter = (() => {
    const b = realm?.main?.background;
    if (!b) return undefined;
    const parts: string[] = [];
    if (b.brightness !== undefined) parts.push(`brightness(${b.brightness}%)`);
    if (b.blur !== undefined) parts.push(`blur(${b.blur}px)`);
    if (b.saturate !== undefined) parts.push(`saturate(${b.saturate}%)`);
    if (b.grayscale !== undefined) parts.push(`grayscale(${b.grayscale}%)`);
    if (b.contrast !== undefined) parts.push(`contrast(${b.contrast}%)`);
    if (b.hueRotate !== undefined) parts.push(`hue-rotate(${b.hueRotate}deg)`);
    return parts.length ? parts.join(" ") : undefined;
  })();

  // B5 预览模式：仅展示领域主界面 + 顶部「返回编辑」，不进入领域、不产生持久改动
  if (preview && realm) {
    return (
      <div
        className={cn(
          "fixed inset-0 z-[125] flex flex-col bg-gradient-to-br from-slate-950 via-slate-900 to-indigo-950",
          hidden && "pointer-events-none opacity-0",
        )}
      >
        {root && wallpaperSrc && (
          <>
            <img
              src={wallpaperSrc}
              alt=""
              className="absolute inset-0 h-full w-full object-cover"
              style={bgFilter ? { filter: bgFilter } : undefined}
              draggable={false}
              /* 与真机同源（v1.26）：root 守卫 + 文件缺失隐藏 / 加载成功复位 */
              onLoad={(e) => {
                e.currentTarget.style.visibility = "visible";
              }}
              onError={(e) => {
                e.currentTarget.style.visibility = "hidden";
              }}
            />
          </>
        )}
        {bgm}
        <div className="relative flex items-center gap-2 border-b border-slate-800 bg-black/40 px-4 py-2.5 backdrop-blur-sm">
          <button
            type="button"
            onClick={() => onClosePreview?.()}
            className="flex items-center gap-1.5 rounded-lg border border-slate-700 px-2.5 py-1.5 text-sm text-slate-300 hover:bg-slate-800/50"
          >
            <ArrowLeft className="h-4 w-4" />
            {t.realm.previewExit}
          </button>
          <h2 className="text-base font-semibold text-foreground">
            {realmName(realm, t.uiLang)}
          </h2>
          <div className="flex-1" />
          <span className="text-xs text-muted">{t.realm.previewNotice}</span>
        </div>
        <div className="relative min-h-0 flex-1 pt-4">
          {/* ⚠️ 预览分支曾把 HomePage 的所有回调都置空、且**根本没传 `onShowcase`** ——
              于是"预览领域主界面"上看不到「阵营巡礼」按钮（而它就是靠这个 prop 决定是否渲染的）。
              现在补上，并让预览内的巡礼真正可播（退出回主界面，不落到真实领域的页面上）。
              v1.26：入口渲染与真机同口径 —— `showcase.enabled === false`（含新建领域默认）时
              同样不传 `onShowcase`，预览里隐藏（此前预览无条件显示，与真机行为不一致）。 */}
          {page === "showcase" ? (
            <ShowcaseOverlay
              realm={realm}
              lang={t.uiLang}
              root={root}
              enabledIds={enabledIds}
              // 接管主界面的 BGM 元素：主题曲**接续播放**，不出两段时间线
              realmBgmRef={bgmRef}
              bgmMuted={bgmMuted}
              onToggleMute={handleToggleBgm}
              onExit={() => setPage("home")}
            />
          ) : (
            <HomePage
              realm={realm}
              root={root}
              phase={null}
              preview
              onPlay={() => {}}
              onInfo={() => {}}
              onSwitch={() => {}}
              onExit={() => {}}
              onReplay={() => {}}
              bgmMuted={false}
              onToggleBgm={() => {}}
              // 巡礼入口与真机同口径（v1.26）：缺字段 / 开关关闭 / 新建领域默认时不渲染
              onShowcase={realm.showcase && realm.showcase.enabled !== false ? () => setPage("showcase") : undefined}
            />
          )}
        </div>
      </div>
    );
  }

  const body = (() => {
    if (!realm && curPage !== "editor" && curPage !== "repository") {
      return (
        <RealmSwitchPage
          onEnter={onEnter}
          onNew={() => setCreateOpen(true)}
          onImport={handleImport}
          onRemote={() => setPage("repository")}
          onEdit={(r) => {
            setEditing(r);
            setEditorOpen(true);
            setPage("editor");
          }}
          showToast={showToast}
        />
      );
    }

    switch (curPage) {
      case "editor":
        // 编辑器始终基于已存在的领域（新建流程已先 create_realm）
        return editorOpen && editing ? (
          <RealmEditor
            editing={editing}
            registerBack={(fn) => {
              editorBackRef.current = fn;
            }}
            onCancel={() => {
              setEditorOpen(false);
              setPage(realm ? "home" : "switch");
            }}
            onSaved={async () => {
              // 保存后**停留**在编辑器（v1.26 修正）：只刷新领域列表（让「切换领域」页 /
              // 壁纸等后续读取看到最新值），不关闭覆盖层、不跳页——保存已是可重复动作，
              // 完成后由 `dirty` 自动回落把按钮置灰；离开走返回（含未保存确认）。
              await refreshRealms();
            }}
            showToast={showToast}
          />
        ) : null;

      case "settings":
        // 「返回 + 标题」交给覆盖层左上角页头（与窗口控件同排，不占额外行）；
        // 作用域徽标由 SettingsView 渲染在工具条右侧按钮群组里。故工具条要 hideTitle，
        // 否则标题会在页头与工具条各出现一次。
        return settingsDir ? (
          <SettingsView
            configDir={settingsDir}
            hideTitle
            scope={realm?.dirs?.configPath === "global" ? "global" : "realm"}
            scopePath={settingsDir}
          />
        ) : null;

      case "info":
        // 滚动**下沉到 InfoBody 内部**（对齐设置面板的「侧栏 + 右侧独立滚动区」）：
        // 锚点侧栏与 scroll-spy 的 IntersectionObserver 需要一个稳定的滚动容器作为 root，
        // 否则长页面（实测全展开 8+ 屏）会完全失去位置感。
        //
        // ⚠️ 必须用 `h-full` 而不是 `flex-1`：本节点是块级父容器（`relative min-h-0 flex-1 pt-10`）
        // 的子节点，不是 flex 容器的子项 —— `flex-1` 在块级父下完全失效，高度会撑成内容高，
        // 于是 `overflow-y-auto` 拿不到可滚动高度 → 整页既滚不动也定位不了。
        return (
          <div className="flex h-full min-h-0">
            <InfoBody r={realm} />
          </div>
        );

      case "switch":
        return (
          <RealmSwitchPage
            onEnter={onEnter}
            onNew={() => setCreateOpen(true)}
            onImport={handleImport}
            onRemote={() => setPage("repository")}
            onEdit={(r) => { setEditing(r); setEditorOpen(true); setPage("editor"); }}
            showToast={showToast}
          />
        );

      case "repository":
        return (
          <RealmRepositoryPage
            showToast={showToast}
            onInstalled={async () => {
              await refreshRealms();
            }}
          />
        );

      default:
        return <HomePage realm={realm} root={root} phase={phase} onPlay={handlePlay} onInfo={() => setPage("info")} onSwitch={() => setPage("switch")} onExit={onExitRealm} onReplay={onReplayIntro} bgmMuted={bgmMuted} onToggleBgm={handleToggleBgm} onShowcase={realm.showcase && realm.showcase.enabled !== false ? () => openWithAssets("showcase") : undefined} />;
      case "assets":
        return realm ? (
          <RealmAssetsPage
            realm={realm}
            realmId={realm.id}
            target={assetsTarget}
            onDone={(tg) => setPage(tg)}
            onBack={() => setPage("home")}
          />
        ) : null;
      case "showcase":
        return realm ? (
          <ShowcaseOverlay
            realm={realm}
            lang={t.uiLang}
            root={root}
            realmBgmRef={bgmRef}
            bgmMuted={bgmMuted}
            onToggleMute={handleToggleBgm}
            enabledIds={enabledIds}
            onExit={() => setPage("home")}
          />
        ) : null;
    }
  })();

  // 二级页面页头（与右上角窗口控件同一行）：按当前页面给出标题与返回动作。
  // 主界面不在此列；编辑器（v1.16 起）在此列——返回转发给编辑器内部注册的 `handleBack`。
  const pageHeader: { title: string; onBack?: () => void; backLabel?: string } | null = (() => {
    switch (curPage) {
      case "info":
        return { title: t.realm.realmInfo, onBack: () => setPage("home") };
      case "assets":
        // 素材准备页：页头返回 = 取消并回主界面（页面内也有同样的按钮，见 `RealmAssetsPage`）
        return { title: t.realm.assetsPreparing, onBack: () => setPage("home") };
      case "switch":
        // 未进入任何领域时，「返回」的语义是「回启动器主界面」（§9.0.1）；
        // 从领域内进来时才是「回领域主界面」。
        return realm
          ? { title: t.realm.switchRealm, onBack: () => setPage("home") }
          : {
              title: t.realm.switchRealm,
              onBack: onExitToLauncher,
              backLabel: t.app.backToLauncher,
            };
      case "repository":
        return { title: t.realm.remoteTitle, onBack: () => setPage("switch") };
      case "settings":
        // 设置页的「返回 + 标题」放回左上角页头：顶部区一行已有搜索 / 视图偏好 / 作用域状态 /
        // 「更多」菜单（v2 重构后运行信息 / 检查更新 / 重置等动作类控件已收进菜单），
        // 再塞返回和标题会挤成一条（且左上角 40px 带本来空着）。
        if (!settingsDir) return null;
        return {
          title: `${realmName(realm, t.uiLang)} · ${t.realm.settings}`,
          onBack: () => {
            setSettingsDir(null);
            setPage("home");
          },
        };
      case "editor":
        // 编辑器：「返回 + 标题」上提为与其它二级页一致的悬浮页头（v1.16）。
        // 返回转发给编辑器注册的 `handleBack`（含未保存改动的确认弹窗）。
        return {
          title: t.realm.editRealm,
          onBack: () => editorBackRef.current?.(),
          backLabel: t.realm.back,
        };
      default:
        return null;
    }
  })();

  return (
    <div
      className={cn(
        "fixed inset-0 z-[125] flex flex-col bg-gradient-to-br from-slate-950 via-slate-900 to-indigo-950 transition-opacity duration-300",
        hidden && "pointer-events-none opacity-0",
      )}
    >
      {/* 顶部拖拽条：领域模式下 `App` 不渲染 `TopBar`，而它是全项目唯一的拖拽区
          （`data-tauri-drag-region`），缺此条则窗口完全无法拖动。
          窗口控件（⚙/多语言/最小化/最大化/关闭）**嵌套进本拖拽区并显式 no-drag**
          （与 TopBar 同款做法）：否则全宽拖拽区会盖在控件上方，导致 hover 落到
          move 光标、点击被吞。nest + no-drag 让控件可点击且 hover 显示小手光标，
          拖拽区其余部分仍可拖动窗口。内容区从 `pt-10` 起，故本条只覆盖顶部 40px。
          顺带获得双击切换最大化（Tauri 对 drag region 的内建行为）。 */}
      <div data-tauri-drag-region className="absolute inset-x-0 top-0 z-[135] h-10 cursor-move">
        {/* 控件容器：no-drag 使其脱离拖拽区，可点击、hover 显示小手光标 */}
        <div
          style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
          className="absolute right-4 top-3 z-[140] rounded-xl bg-black/30 px-1 py-0.5 backdrop-blur-sm"
        >
          <WindowControls
            onSettings={showSettingsBtn ? openSettings : undefined}
            settingsTitle={t.realm.settings}
            settingsBadge={updateBadge}
            settingsBadgeTitle={t.update.redDotTooltip}
            extra={<LanguageMenuButton />}
          />
        </div>
        {/* 二级页面页头（返回 + 标题，§9.2.4）：与右侧控件同理，必须**嵌套进拖拽区 + no-drag**——
            页头若作为拖拽区的重叠兄弟节点，`app-region: drag` 会按矩形吞掉光标与点击，
            返回按钮 hover 无小手（实测）。 */}
        {pageHeader && (
          <PageHeaderBar
            title={pageHeader.title}
            onBack={pageHeader.onBack}
            backLabel={pageHeader.backLabel}
          />
        )}
      </div>
      {/* 壁纸背景（§9.2.2）：铺满整个窗口（含顶部控件让位区）。
          领域信息页同样保留壁纸（朦胧可见）以维持沉浸感，并加深蒙层保证卡片文字可读。
          主题化滤镜（§6 `main.background`）与编辑器预览同一口径套在此 `<img>` 上
          （v1.25 修复：此前仅预览分支生效，实际主界面未套）。
          v1.26 修复：条件加 `root` 守卫 —— `root` 异步解析完成前 `wallpaperSrc` 是
          `convertFileSrc("/assets/…")` 形式的**非空坏 URL**，`<img>` 会先以它挂载并在
          请求失败时触发 `onError` 隐藏；若该失败先于 root 就绪到达，之后 src 修正、
          图片实际加载成功也不会复位（内联样式残留）→ 主界面壁纸"永久消失"。 */}
      {root && (curPage === "home" || curPage === "info") && wallpaperSrc && (
        <>
          <img
            src={wallpaperSrc}
            alt=""
            className="absolute inset-0 h-full w-full object-cover"
            style={bgFilter ? { filter: bgFilter } : undefined}
            draggable={false}
            /* 自备素材的路径存在但文件缺失时，`<img>` 会渲染成 broken 图标；
               隐藏自身即可露出底层深色渐变（与"未配置壁纸"同款观感）。
               `onLoad` 复位（v1.26）：隐藏是写在 DOM 上的一次性内联样式，
               换图 / 文件补齐后加载成功必须能恢复显示。 */
            onLoad={(e) => {
              e.currentTarget.style.visibility = "visible";
            }}
            onError={(e) => {
              e.currentTarget.style.visibility = "hidden";
            }}
          />
          {/* 主界面不再加全局蒙层（标题由 HomePage 自带局部蒙层保护，§9.2.2）；
              领域信息页文字密集，保留加深 + 模糊蒙层保证可读 */}
          {curPage === "info" && (
            <div className="absolute inset-0 bg-slate-950/80 backdrop-blur-sm" />
          )}
        </>
      )}
      {bgm}

      {/* 游戏启动中弹出层（§9.2.3）：停留在领域主界面之上，不再淡出覆盖层露出 Mods 页。
          期间暂停背景音乐；可关闭返回领域主界面。 */}
      {launching && (
        <div className="fixed inset-0 z-[145] flex items-center justify-center bg-black/70 p-4 backdrop-blur-sm">
          <div className="w-full max-w-sm rounded-xl border border-slate-700/60 bg-slate-900 p-6 text-center shadow-2xl">
            <div className="text-base font-semibold text-slate-100">{t.realm.launching}</div>
            <p className="mt-2 text-sm text-slate-400">{t.realm.launchingHint}</p>
            <button
              type="button"
              onClick={() => setLaunching(false)}
              className="mt-5 w-full rounded-lg border border-slate-700 px-3 py-2 text-sm text-slate-200 hover:bg-slate-800"
            >
              {t.realm.backToRealm}
            </button>
          </div>
        </div>
      )}

      {/* 二级页面页头已上移进顶部拖拽条（嵌套 + no-drag，见上方注释），此处不再单独渲染 */}

      {/* 窗口控件已上移进顶部拖拽条（嵌套 + no-drag，见上方注释），此处不再单独渲染 */}

      {/* relative：让内容绘制在 absolute 壁纸层之上 */}
      <div className="relative min-h-0 flex-1 pt-10">{body}</div>

      {/* 新建领域：先确定 ID + 名称并立即创建（领域目录与 realm.json 落盘），再进编辑器 */}
      {createOpen && (
        <CreateRealmDialog
          onCancel={() => setCreateOpen(false)}
          onCreated={(r) => {
            setCreateOpen(false);
            setEditing(r);
            setEditorOpen(true);
            setPage("editor");
          }}
          showToast={showToast}
        />
      )}

      {/* 启动前问题：居中模态框，按类型给不同出口，「仍然启动」是有意保留的逃生口（§9.2.3） */}
      {launchIssue && (
        <div
          className="fixed inset-0 z-[145] flex items-center justify-center bg-black/65 p-4 backdrop-blur-sm"
          onClick={() => setLaunchIssue(null)}
        >
          <div
            className="w-full max-w-md rounded-xl border border-slate-700/60 bg-slate-900 p-5 shadow-2xl"
            onClick={(e) => e.stopPropagation()}
          >
            <h3 className="mb-2 text-base font-semibold text-slate-100">{t.realm.launchIssueTitle}</h3>
            <p className="mb-3 text-sm text-slate-400">
              {launchIssue.kind === "conflict"
                ? t.realm.launchConflictDesc
                : launchIssue.kind === "bundled"
                  ? t.realm.launchMissingBundledDesc
                  : t.realm.launchMissingBorrowedDesc}
            </p>
            <ul className="mb-4 max-h-40 space-y-0.5 overflow-y-auto text-sm text-slate-300">
              {launchIssue.items.map((id) => (
                <li key={id}>· {id}</li>
              ))}
            </ul>
            <div className="flex flex-wrap justify-end gap-2">
              <button
                type="button"
                onClick={() => setLaunchIssue(null)}
                className="rounded-lg border border-slate-700 px-3 py-1.5 text-sm text-slate-300 hover:bg-slate-800"
              >
                {t.common.cancel}
              </button>
              {launchIssue.kind === "conflict" && realm && (
                <button
                  type="button"
                  onClick={() => {
                    setLaunchIssue(null);
                    setEditing(realm);
                    setEditorOpen(true);
                    setPage("editor");
                  }}
                  className="rounded-lg border border-slate-700 px-3 py-1.5 text-sm text-slate-200 hover:bg-slate-800"
                >
                  {t.realm.viewModCombo}
                </button>
              )}
              {launchIssue.kind === "borrowed" && (
                <button
                  type="button"
                  onClick={async () => {
                    setLaunchIssue(null);
                    await onExitRealm();
                  }}
                  className="rounded-lg border border-slate-700 px-3 py-1.5 text-sm text-slate-200 hover:bg-slate-800"
                >
                  {t.realm.backToOfficialInstall}
                </button>
              )}
              <button
                type="button"
                onClick={async () => {
                  setLaunchIssue(null);
                  await doLaunch();
                }}
                className="rounded-lg bg-red-500/90 px-3 py-1.5 text-sm font-medium text-white hover:bg-red-500"
              >
                {t.realm.launchAnyway}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* 启动失败：显示具体原因 + 「重试」（§9.2.3 阶段 4） */}
      <ConfirmDialog
        open={launchError !== null}
        title={t.realm.launchFailedTitle}
        description={launchError ?? ""}
        confirmText={t.realm.retry}
        cancelText={t.common.close}
        danger
        onCancel={() => setLaunchError(null)}
        onConfirm={async () => {
          setLaunchError(null);
          await doLaunch();
        }}
      />

      {/* 启动前检测弹窗（通用 step 框架）：步骤条总览 + 各检查项卡片。
          后续新增检查项只需在 prelaunchSteps 追加一个 step，无需改动外壳。 */}
      <PreLaunchCheckDialog
        open={prelaunch.open}
        title={t.launch.langModCheck}
        steps={prelaunchSteps}
        states={prelaunch.states}
        closeDisabled={prelaunch.busy}
        onClose={prelaunch.close}
      />

      {/* 导入 id 冲突（§7.2） */}
      <ConfirmDialog
        open={conflict !== null}
        title={t.realm.importConflictTitle}
        description={t.realm.importConflictDesc.replace("{name}", conflict?.realmName ?? "")}
        confirmText={t.realm.importOverwrite}
        cancelText={t.realm.importRename}
        onCancel={async () => {
          const p = importPath;
          setConflict(null);
          setImportPath(null);
          try {
            await finishImport(p!, "rename");
          } catch (e) {
            showToast("error", t.realm.error.replace("{msg}", backendErrorText(e)));
          }
        }}
        onConfirm={async () => {
          const p = importPath;
          setConflict(null);
          setImportPath(null);
          try {
            await finishImport(p!, "overwrite");
          } catch (e) {
            showToast("error", t.realm.error.replace("{msg}", backendErrorText(e)));
          }
        }}
      />

      {toast && (
        <Toast
          text={toast.msg}
          kind={toast.type}
          duration={2600}
          resetKey={toast.msg}
          className="z-[200]"
          placement="bottom"
          onDone={() => setToast(null)}
        />
      )}
    </div>
  );
}
