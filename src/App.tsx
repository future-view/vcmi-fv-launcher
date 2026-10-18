import { useCallback, useEffect, useRef, useState } from "react";
import { I18nProvider, useI18n } from "./i18n";
import { TopBar } from "@/shell/TopBar";
import { BottomBar } from "@/shell/BottomBar";
import { ModView } from "@/features/mods/ModView";
import { SettingsView } from "@/features/settings/SettingsView";
import { WikiView } from "@/features/wiki/WikiView";
import { SetupWizard } from "@/features/setup/SetupWizard";
import { InitTopBar } from "@/shell/InitTopBar";
import { GameDataDrawer } from "@/shell/GameDataDrawer";
import { HelpDrawer } from "@/shell/HelpDrawer";
import { ImportFilesDialog } from "@/shell/ImportFilesDialog";
import { detectVcmiPaths, isDebug, isTauri, realmSelfCheck, realmSelfHeal, getActiveRealm, activateRealm, deactivateRealm, realmDir, isVcmiRunning, buildRealmBundledWikiByIds, checkLauncherUpdate, markUpdatePrompted, closeWindow, applyLauncherUpdate, setUpdatePref, type VcmiDirs, type RealmSelfCheck, type UpdateInfo } from "@/lib/tauri";
import { ensureWikiCacheBuilt } from "@/lib/modcache/status";
import { getRealmById, refreshRealms, useRealms } from "@/lib/realm/store";
import { shouldPlayIntro } from "@/lib/realm/intro";
import type { Realm } from "@/lib/realm/types";
import { RealmSelfCheckDialog } from "@/features/realms/RealmSelfCheckDialog";
import { RealmShell } from "@/features/realms/RealmShell";
import { RealmIntroOverlay } from "@/features/realms/RealmIntroOverlay";
import { syncLanguageMods } from "@/lib/langMods";
import { installGlobalLogging, logInfo, logWarn } from "@/lib/log";
import { UpdateDialog } from "@/features/settings/UpdateDialog";
import { ForcedUpdateOverlay } from "@/features/settings/ForcedUpdateOverlay";
import { backendErrorText } from "@/lib/backendError";
import { Toast } from "@/shared/Toast";

// 尽早安装全局错误钩子：window.onerror / unhandledrejection → 启动器日志文件
installGlobalLogging();

/** 底栏「导入游戏文件」直接接受的扩展名（与后端 `vcmi::game_import` 白名单一致；`.zip` 由后端自动识别） */
const IMPORT_FILE_EXTS = new Set(["zip", "h3m", "vmap", "h3c", "vcmp", "vsgm1", "json", "exe"]);

/**
 * 拖放预过滤：白名单文件直收；无扩展名 / 非常规扩展名（如目录名含空格与点）
 * 视为**目录候选**（HD 版安装目录）放行；其余（.pdf/.txt/.png 等）忽略。
 */
function isImportCandidate(p: string): boolean {
  const tail = p.split(/[\\/]/).filter(Boolean).pop() ?? "";
  if (!tail) return false;
  const dot = tail.lastIndexOf(".");
  if (dot <= 0) return true;
  const ext = tail.slice(dot + 1).toLowerCase();
  if (!/^[a-z0-9]{1,5}$/.test(ext)) return true;
  return IMPORT_FILE_EXTS.has(ext);
}

/** 启动器主界面的页签视图（领域不再是页签，而是**顶级页面**）。 */
type ViewKey = "mods" | "wiki" | "settings";

/** 全局提示条：领域页与启动器主界面都会用到，故抽成独立组件；可附带一个动作按钮（如语言模组「从仓库安装」）。
 * 统一用 `Toast` 实现，自动消失 + 点击可复制。 */
function NoticeBar({
  msg,
  action,
  onClose,
}: {
  msg: string;
  action?: { label: string; run: () => void };
  onClose: () => void;
}) {
  return (
    <Toast
      text={msg}
      kind="warning"
      duration={6000}
      resetKey={msg}
      className="top-[70px] z-[300]"
      action={action}
      onDone={onClose}
      onClose={onClose}
    />
  );
}

function AppContent() {
  const { t, vcmiLang } = useI18n();
  const { realms: realmList, activeRealm } = useRealms();
  const activeRealmObj = realmList.find((r) => r.id === activeRealm) ?? null;
  const [view, setView] = useState<ViewKey>("mods");
  /**
   * 「设置」不是页签（§9.0.1 v1.10）：由右上角控件区的 ⚙ 进入，**再点 ⚙**（或点任一页签）离开。
   * ⚙ 是 toggle，故这里记住最近一个非设置视图，用于复位——固定跳回「模组」会丢上下文。
   */
  const prevViewRef = useRef<ViewKey>("mods");
  const changeView = (v: ViewKey) => {
    if (v !== "settings") prevViewRef.current = v;
    setView(v);
  };
  /**
   * ModView 子语境（预设编排 presets / 模组管理 manage）。提升到此处持有：
   * ① 启动前弹窗「前往模组管理」需从外部切入 manage 页；
   * ② 领域退出后 ModView 因 dirs 变化重挂载，提升后 mode 不被重置（受控于 App）。
   */
  const [modsMode, setModsMode] = useState<"presets" | "manage">("presets");
  /** ⚙：切入设置视图 / 从设置视图复位；有可选更新且满足节流条件时顺带弹出更新弹窗 */
  const [updateInfo, setUpdateInfo] = useState<UpdateInfo | null>(null);
  const [updateOpen, setUpdateOpen] = useState(false);
  const toggleSettings = () => {
    if (view === "settings") {
      setView(prevViewRef.current);
      return;
    }
    setView("settings");
    // 可选更新的自动提醒（感知阶梯 L4）：由后端 `shouldPrompt` 判定
    //（每版本首次发现立即提醒；同版本 7 天节流，见 settings-panel-design §2.3）。
    // 展示即计时：就地置 false（会话内不再重复弹）+ 落盘记录（跨会话由后端节流接管）。
    if (updateInfo?.hasUpdate && !updateInfo.forced && updateInfo.shouldPrompt) {
      setUpdateOpen(true);
      setUpdateInfo({ ...updateInfo, shouldPrompt: false });
      void markUpdatePrompted(updateInfo.latestVersion);
    }
  };
  const [dirs, setDirs] = useState<VcmiDirs | null>(null);
  const [checking, setChecking] = useState(true);
  const [showData, setShowData] = useState(false);
  const [showHelp, setShowHelp] = useState(false);
  const [modsLoading, setModsLoading] = useState(true);
  const [debug, setDebug] = useState(false);
  /** isDebug 是否已解析（避免调试构建下强制更新阻断层闪现） */
  const [debugReady, setDebugReady] = useState(false);
  const [selfCheck, setSelfCheck] = useState<RealmSelfCheck | null>(null);
  // 领域页（§9.2.2）与登场演出（§5.4）
  const [shellRealm, setShellRealm] = useState<Realm | null>(null);
  const [introRealm, setIntroRealm] = useState<Realm | null>(null);
  const [introRoot, setIntroRoot] = useState("");
  // 启动游戏后领域页淡出（§9.2.3）；VCMI 退出后自动淡回
  const [shellFaded, setShellFaded] = useState(false);
  /**
   * **顶级页面模式**：领域页与启动器主界面**互斥渲染**。
   *
   * 旧实现把领域做成「叠在主界面之上的覆盖层」，两层组件同时挂载，
   * 于是 ModView 与领域预设管理会同时写同一份 `modSettings.json`
   * （领域共享全局配置时尤其严重）：ModView 整体写回预设字典，
   * 会把 `activePreset=__realm__` 改回 `__official__` 并删掉 `__realm__` 预设。
   * 改为互斥渲染后，领域模式下 ModView 根本不挂载，冲突从结构上不可能发生。
   */
  const [realmMode, setRealmMode] = useState(false);
  // 全局提示（如语言模组缺失、领域激活失败）；action = 可选动作按钮（语言模组「从仓库安装」）
  const [globalNotice, setGlobalNotice] = useState<{
    msg: string;
    action?: { label: string; run: () => void };
  } | null>(null);
  /**
   * 语言模组同步的「重跑信号」。
   *
   * `useRealms()` 的 `activeRealm` 在 `refreshRealms()` 时即被置位，而它发生在
   * `activate_realm` **之前**——若只依赖 `activeRealmObj`，语言同步可能在领域预设
   * 尚未落盘时就跑掉（此时目标无预设 → 跳过），之后再无触发时机。
   * 故激活完成后显式自增该值，确保同步发生在配置初始化之后。
   */
  const [langSyncNonce, setLangSyncNonce] = useState(0);
  // —— 导入游戏文件（底栏「游戏数据」左侧入口；应用级拖放见下方 effect）——
  const [importOpen, setImportOpen] = useState(false);
  const [importSeed, setImportSeed] = useState<string[]>([]);
  const [importSeedNonce, setImportSeedNonce] = useState(0);
  const [importDragOver, setImportDragOver] = useState(false);

  /** 拖放 / 入口预填：打开弹层或向已打开弹层追加（nonce 驱动弹层内会话追加） */
  const addImportPaths = useCallback((list: string[]) => {
    const accepted = list.filter(isImportCandidate);
    if (accepted.length === 0) return;
    setImportSeed(accepted);
    setImportSeedNonce((n) => n + 1);
    setImportOpen(true);
  }, []);

  const ready = !!dirs?.ready;

  /** 进入领域页：装载领域主界面，并按 §9.1 的播放策略决定是否播放登场演出。 */
  const openRealm = async (r: Realm, forceIntro = false) => {
    setShellRealm(r);
    setRealmMode(true);
    // 整合模组的资料补建百科缓存：文件在激活时才落到 `{realm-dir}/vcmi/Mods/`，
    // 且不在全局构建的扫描范围（只扫全局用户目录 + 游戏目录）。增量构建，重复调用很快。
    void buildRealmBundledWikiByIds(r.id, r.mods?.bundled ?? []);
    if (!forceIntro && !(await shouldPlayIntro(r))) return;
    try {
      const dir = await realmDir(r.id);
      setIntroRoot(dir);
      setIntroRealm(r);
    } catch {
      /* 目录解析失败则跳过演出，不影响进入领域 */
    }
  };

  /**
   * 启动时恢复领域（§9.1）：存在 `active_realm` 则激活并直接进入领域页。
   *
   * 便携启动允许 launcher.json 预置 `realms.active` 直入领域主界面，
   * 但**激活逻辑不能丢**：改写 dirs.json、播种领域预设、落地借用模组全在 `activate_realm` 里。
   * 否则 dirs.json 会停留在游戏原厂配置，领域形同未激活。`activate_realm` 幂等，可重复执行。
   */
  const restoreRealm = async () => {
    await refreshRealms();
    const id = await getActiveRealm();
    if (!id) return;
    const r = getRealmById(id);
    if (!r) return;
    const res = await activateRealm(id);
    if (!res.ok) {
      // 失败不阻断进入（例如 VCMI 正在运行时进程守卫会拦截），但要明确告知。
      console.warn("[realm] 启动恢复激活领域失败:", res.error);
      logWarn("realm", `启动恢复激活领域失败：${res.error ?? ""}`);
      setGlobalNotice({ msg: t.app.realmActivateFailed.replace("{msg}", backendErrorText(res.error)) });
    }
    await openRealm(r);
    // 激活已完成、领域预设已落盘 → 触发一次语言模组同步（见 langSyncNonce 注释）
    setLangSyncNonce((n) => n + 1);
  };

  /**
   * 崩溃自检（§4.5）：**必须在领域激活之后**执行，否则会把刚激活的领域误判为残留记录。
   *
   * 分级处理，避免用模态框打断玩家：
   * - `maps_orphan`（地图联接缺失 = 恢复透明态）：意图明确且可安全自愈 → **静默修好**，
   *   只记日志 / 轻提示。历史上它是唯一会「每次启动都弹」的分支（领域退回全局态后必然命中）；
   * - 其余异常（涉及「回到官方预设 vs 重新进入领域」的选择，或需要人工决定删联接 / 合并地图）
   *   才交给 `RealmSelfCheckDialog` 呈现。
   */
  const runSelfCheck = async () => {
    try {
      const r = await realmSelfCheck();
      if (!r || r.ok) return;
      if (r.status === "maps_orphan" && r.fixable) {
        await realmSelfHeal();
        await refreshRealms();
        logInfo("realm", "已静默重建游戏地图目录联接（恢复透明态）");
        return;
      }
      setSelfCheck(r);
    } catch {
      /* 自检失败不影响使用 */
    }
  };

  /**
   * 启动编排（**严格串行**，消除旧实现的并行竞态）：
   *   1. **目录初始化**：`detectVcmiPaths()` 读取 `launcher.json`（无论来自初始化向导保存
   *      还是便携包预置），按其 `game_dir` + `user_data_dir` 解析全部路径，并创建
   *      `<user_data_dir>/{cache,config,logs,Saves,Mods}`；
   *      无激活领域且 `redirect_user_data` 时还会接管 `game_dir/config/dirs.json`。
   *      未就绪（无有效配置）→ 交给初始化向导，不进入后续步骤。
   *   2. 领域恢复（有 active_realm → activate_realm：改写 dirs.json / 播种预设 / 落地模组）
   *   3. 崩溃自检（依赖 2 的结果，必须在激活之后）
   *   4. 后台 wiki 缓存构建（增量，不阻塞 UI）
   */
  const boot = async () => {
    logInfo("boot", "启动编排开始：目录探测/初始化");
    try {
      const d = await detectVcmiPaths();
      setDirs(d);
      if (!d?.ready) {
        logInfo("boot", "目录未就绪，进入初始化向导");
        return; // 未就绪 → 交给初始化向导
      }
      logInfo("boot", "目录就绪，开始领域恢复");
      await restoreRealm();
      await runSelfCheck();
      void ensureWikiCacheBuilt().catch(() => {});
      // 启动静默检查更新（不 await：不打断启动编排，失败静默）
      void checkUpdateSilently();
      logInfo("boot", "启动编排完成");
    } finally {
      setChecking(false);
    }
  };

  /** 初始化向导完成：重新走一遍完整启动编排（目录初始化 → 领域恢复 → 自检 → 缓存）。 */
  const handleWizardReady = async () => {
    setChecking(true);
    await boot();
  };

  /**
   * 打开领域页：
   * - 已有激活领域 → 加载它并直接进入领域主界面（而不是落到「切换领域」页）；
   * - 都没有 → 进入领域浏览 / 切换页（由 `RealmShell` 在 `realm=null` 时渲染）。
   */
  const openRealmPage = async () => {
    if (shellRealm) {
      setRealmMode(true);
      return;
    }
    const id = await getActiveRealm();
    if (id) {
      let r = getRealmById(id);
      if (!r) {
        await refreshRealms();
        r = getRealmById(id);
      }
      if (r) {
        await openRealm(r);
        return;
      }
    }
    setRealmMode(true);
  };

  /** 进入某领域（领域浏览页的「进入」按钮，§9.4）：激活 → 刷新 → 打开领域主界面。 */
  const handleEnterRealm = async (id: string): Promise<boolean> => {
    const res = await activateRealm(id);
    if (!res.ok) return false;
    await refreshRealms();
    const r = getRealmById(id);
    if (r) await openRealm(r);
    else setRealmMode(true);
    // 激活已完成 → 触发一次语言模组同步（见 langSyncNonce 注释）
    setLangSyncNonce((n) => n + 1);
    return true;
  };

  /** 退出领域：停用并回到启动器主界面（领域页与主界面互斥，退出即切页）。 */
  const handleExitRealm = async () => {
    const res = await deactivateRealm();
    if (!res.ok) return;
    await refreshRealms();
    setShellFaded(false);
    setShellRealm(null);
    setRealmMode(false);
  };

  /** 关闭领域浏览页（未激活任何领域）→ 回到启动器主界面。 */
  const closeRealmPage = () => {
    setShellRealm(null);
    setRealmMode(false);
  };

  useEffect(() => {
    isDebug()
      .then((d) => {
        setDebug(d);
        setDebugReady(true);
      })
      .catch(() => {
        setDebug(false);
        setDebugReady(true);
      });
  }, []);

  // 调试构建检测到强制更新：不阻断，仅提示条（保护 tauri dev 开发流程）
  useEffect(() => {
    if (debugReady && debug && updateInfo?.forced && updateInfo.hasUpdate) {
      setGlobalNotice({ msg: `${t.update.forcedTitle} · ${t.update.debugHint}` });
    }
  }, [debugReady, debug, updateInfo, t]);

  /** 已下载待应用：一键重启并应用（复用 UpdateDialog 的落地逻辑，免重复实现） */
  const applyPendingUpdate = useCallback(async () => {
    setGlobalNotice(null);
    try {
      const res = await applyLauncherUpdate();
      if (res.ok) {
        if (res.restarted === false) {
          // 纯配置包已就地应用：打开弹窗展示「已应用，重启后生效」
          setUpdateOpen(true);
        } else {
          // 含 exe：后端已 spawn 替换脚本，延迟关窗等其按 PID 等进程退出
          window.setTimeout(() => void closeWindow(), 800);
        }
      } else {
        setUpdateOpen(true); // 失败交给弹窗展示错误
      }
    } catch {
      setUpdateOpen(true);
    }
  }, []);

  /** 跳过某版本后的反馈：提示已跳过，并提供「撤销」一次恢复提醒（清掉 skip_version） */
  const onSkipUpdate = useCallback((version: string) => {
    setGlobalNotice({
      msg: t.update.skippedToast.replace("{ver}", version),
      action: {
        label: t.update.undoSkip,
        run: async () => {
          try {
            await setUpdatePref(null, ""); // 清空 skip_version → 红点/自动提醒恢复
            const i = await checkLauncherUpdate(false);
            if (i) setUpdateInfo(i);
          } catch {
            /* 撤销失败静默：用户可在设置中手动检查更新 */
          }
          setGlobalNotice(null);
        },
      },
    });
  }, [t]);

  /** 启动静默检查更新：失败静默（清单不可达绝不阻塞使用）；结果供红点 / 阻断 / 弹窗派生 */
  const checkUpdateSilently = async () => {
    const info = await checkLauncherUpdate(false);
    if (!info) return;
    setUpdateInfo(info);
    // 「更新包已就绪」：跨会话恢复的未竟事务，启动期直接提示，不受 7 天节流
    if (info.readyToApply && !info.forced) {
      // 纯配置包（无 exe）：启动期就地静默应用，无需提示用户重启（风险低、用户已点击更新）
      if (info.kind === "config") {
        try {
          await applyLauncherUpdate();
        } catch {
          /* 失败则落到下方提示条兜底 */
        }
        const refreshed = await checkLauncherUpdate(false);
        setUpdateInfo(refreshed ?? info);
        if (refreshed?.readyToApply && !refreshed.forced) {
          setGlobalNotice({
            msg: t.update.pendingApply.replace("{ver}", refreshed.latestVersion),
            action: { label: t.update.restartApply, run: applyPendingUpdate },
          });
        }
        return;
      }
      setGlobalNotice({
        msg: t.update.pendingApply.replace("{ver}", info.latestVersion),
        action: { label: t.update.restartApply, run: applyPendingUpdate },
      });
    }
  };

  useEffect(() => {
    // 窗口初始尺寸由 Rust 在显示前定好（见 lib/tauri.ts 顶部说明），前端只跑启动编排
    void boot();
  }, []);

  // 全局禁用右键菜单（自定义无边框窗口避免系统右键）
  useEffect(() => {
    const prevent = (e: MouseEvent) => e.preventDefault();
    document.addEventListener("contextmenu", prevent);
    return () => document.removeEventListener("contextmenu", prevent);
  }, []);

  /**
   * 应用级拖放（**唯一监听**，替代旧 ModView 的 zip 直装）：
   * - `over`/`leave` 驱动导入弹层整层高亮（Tauri 无落点判定，与官方「拖到窗口任意位置」一致）；
   * - `drop` 过滤后预填列表（弹层未开则打开）；
   * - 领域页（与主界面互斥）→ 导入目标恒为全局用户目录，只给全局提示。
   */
  const realmModeRef = useRef(realmMode);
  realmModeRef.current = realmMode;
  const tRef = useRef(t);
  tRef.current = t;
  useEffect(() => {
    if (!isTauri()) return;
    let unlisten: (() => void) | undefined;
    let cancelled = false;
    void (async () => {
      try {
        const { getCurrentWebview } = await import("@tauri-apps/api/webview");
        const un = await getCurrentWebview().onDragDropEvent((e) => {
          if (cancelled) return;
          if (e.payload.type === "over") {
            if (!realmModeRef.current) setImportDragOver(true);
            return;
          }
          if (e.payload.type === "leave") {
            setImportDragOver(false);
            return;
          }
          // drop
          setImportDragOver(false);
          if (realmModeRef.current) {
            setGlobalNotice({ msg: tRef.current.importFiles.realmBlocked });
            return;
          }
          addImportPaths(e.payload.paths ?? []);
        });
        unlisten = un;
      } catch {
        /* 非桌面端或不支持拖放：静默降级，按钮仍可用 */
      }
    })();
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, [addImportPaths]);

  /**
   * 语言层：把「当前语言匹配的翻译类模组」（按类型+语言发现，不按名字）同步进生效预设——
   * 领域模式取 `realm.json` 的 `presetId`，非领域取 `activePreset`。
   *
   * 同时会先同步 VCMI 的 `general.language`（否则翻译模组会被 VCMI 判为语言不匹配而移除，
   * 见 `src/langMods.ts` 头注释）。幂等，覆盖三个时机：
   * 切换界面语言 / 激活领域 / 启动恢复领域。
   *
   * 「完全没装匹配语言包」的仓库安装建议不在此处处理，改由启动前的 Tier 3 检查
   * （`checkLaunchLangMod` + 启动弹窗）在按「启动游戏」时给出。
   */
  useEffect(() => {
    if (!ready) return;
    void syncLanguageMods({
      vcmiLang,
      realmId: activeRealmObj?.id ?? null,
      presetId: activeRealmObj?.presetId,
    }).then((res) => {
      // 语言层失败必须让用户看见：否则 VCMI 会在启动瞬间把翻译模组移除，
      // 表现为「启动器是中文、游戏内却仍是英文」的静默失效。
      if (!res.ok && res.error) {
        console.warn("[langMods] 语言同步失败:", res.error);
        logWarn("langMods", `语言同步失败：${res.error}`);
        setGlobalNotice({ msg: t.app.vcmiLangSettingFailed.replace("{msg}", res.error) });
      }
    });
  }, [ready, vcmiLang, activeRealmObj?.id, activeRealmObj?.presetId, langSyncNonce]);

  /**
   * 领域页淡出期间轮询 VCMI 进程：先观测到「运行中」，再观测到「已退出」时淡回（§9.2.3）。
   * 这样玩家关掉游戏后会自动回到领域主界面。
   */
  useEffect(() => {
    if (!shellFaded) return;
    let sawRunning = false;
    let stopped = false;
    const timer = window.setInterval(async () => {
      if (stopped) return;
      const running = await isVcmiRunning();
      if (running) {
        sawRunning = true;
        return;
      }
      if (sawRunning) setShellFaded(false);
    }, 3000);
    return () => {
      stopped = true;
      window.clearInterval(timer);
    };
  }, [shellFaded]);

  if (checking) {
    return (
      <div className="flex h-screen w-screen items-center justify-center bg-slate-950 text-slate-400">
        <div className="animate-pulse">VCMI-FV</div>
      </div>
    );
  }

  // 初始化未完成：仅渲染向导，主界面（ModView 等）不挂载，
  // 避免未保存就执行扫描 Mods / 写 modSettings / 提取城镇图等磁盘副作用
  if (!ready) {
    return (
      <div className="flex h-screen flex-col bg-slate-950 text-slate-100">
        {/* 首启标题栏：与主界面同一套视觉模板（品牌 + 窗口三键）。
            不挂它的话窗口没有任何最小化/关闭入口，只能 Alt+F4 退出。 */}
        <InitTopBar />
        {debug && (
          <div className="fixed left-0 right-0 top-[60px] z-[60] flex items-center justify-center gap-2 bg-amber-500/90 px-3 py-1 text-center text-[11px] font-medium text-black">
            <span>{t.app.debugMode}</span>
            <button
              type="button"
              className="ml-1 rounded bg-black/20 px-1.5 py-0.5 hover:bg-black/30"
              onClick={() => setDebug(false)}
            >
              {t.app.close}
            </button>
          </div>
        )}
        <div className="flex min-h-0 flex-1 items-center justify-center p-6">
          <SetupWizard onReady={handleWizardReady} />
        </div>
      </div>
    );
  }

  // ============ 强制更新阻断（最高优先级，v1.1） ============
  // 清单 minSupported 判定（缓存离线同样生效）；调试构建不阻断（仅提示条，见上方 effect）。
  // 主界面 / 领域界面 / 红点全部不渲染，只允许「立即更新」或「退出启动器」。
  if (debugReady && !debug && updateInfo?.forced && updateInfo.hasUpdate) {
    return (
      <ForcedUpdateOverlay
        info={updateInfo}
        onExit={() => {
          void closeWindow();
        }}
      />
    );
  }

  // ============ 顶级页面：领域页（§9.2.0） ============
  // 与启动器主界面**互斥**：此处不再渲染主界面，ModView 不会挂载，
  // 因此不会出现「ModView 与领域预设争抢同一份 modSettings.json」的问题。
  if (realmMode) {
    return (
      <div className="relative h-screen w-screen overflow-hidden bg-slate-950">
        <RealmShell
          realm={shellRealm}
          onEnter={handleEnterRealm}
          onExitRealm={handleExitRealm}
          onExitToLauncher={closeRealmPage}
          // 语言包预检弹窗「前往模组管理」：领域内没有模组页，故先退出领域再切「模组」页并进入管理语境。
          // 这是用户明确的导航意图：即便停用领域失败也强制离开（否则会卡在领域页、changeView 被领域分支覆盖），
          // 故不复用 handleExitRealm（其在 deactivate 失败时会 early-return 而不退出）。
          onGoToMods={async () => {
            await deactivateRealm().catch(() => undefined);
            await refreshRealms().catch(() => undefined);
            setShellFaded(false);
            setShellRealm(null);
            setRealmMode(false);
            changeView("mods");
            setModsMode("manage");
          }}
          onReplayIntro={async (r) => openRealm(r, true)}
          introActive={!!introRealm}
          hidden={shellFaded}
          updateBadge={!!updateInfo?.hasUpdate && !updateInfo.forced}
        />

        {/* 淡出期间的手动回退入口，避免进程探测失效时无处可回 */}
        {shellFaded && shellRealm && (
          <button
            type="button"
            onClick={() => setShellFaded(false)}
            className="fixed bottom-24 right-6 z-[150] rounded-lg border border-slate-700 bg-slate-900/90 px-3 py-1.5 text-xs text-slate-300 shadow-xl backdrop-blur-sm hover:bg-slate-800"
          >
            {t.realm.backToRealm}
          </button>
        )}

        {/* 登场演出（§5.4）：叠加在领域主界面之上 */}
        {introRealm && (
          <RealmIntroOverlay
            realm={introRealm}
            root={introRoot}
            onFinish={() => setIntroRealm(null)}
          />
        )}

        {globalNotice && <NoticeBar {...globalNotice} onClose={() => setGlobalNotice(null)} />}
      </div>
    );
  }

  // ============ 顶级页面：启动器主界面 ============
  // 此处不存在激活领域（有激活领域时走上面的领域页），
  // 因此设置页读写全局 VCMI 配置，不再需要「领域激活通栏」这类兼容逻辑。
  return (
    <>
      <div className="flex h-screen w-screen flex-col overflow-hidden">
        <TopBar
          view={view}
          onViewChange={changeView}
          onToggleSettings={toggleSettings}
          realmOpen={false}
          onOpenRealm={openRealmPage}
          settingsBadge={!!updateInfo?.hasUpdate && !updateInfo.forced}
          settingsBadgeTitle={
            !!updateInfo?.readyToApply && !updateInfo.forced
              ? t.update.redDotReady
              : t.update.redDotTooltip
          }
        />
        <div className="min-h-0 flex-1 pt-[60px] pb-[64px]">
          {view === "mods" ? (
            <ModView
              key={dirs?.userModsDir || dirs?.gameDir || "ready"}
              onModsLoadingChange={setModsLoading}
              mode={modsMode}
              onModeChange={setModsMode}
            />
          ) : view === "wiki" ? (
            <WikiView />
          ) : (
            // 启动器设置页恒为全局作用域：不传 onBack（离开靠再点 ⚙ / 点页签），
            // scope 与领域设置页共用同一套徽标渲染
            <SettingsView
              scope="global"
              scopePath={dirs?.userConfigDir || undefined}
              onOpenUpdate={() => {
                // 「检查更新」= 手动检查（force=true，绕过 24h 清单 TTL 缓存）：
                // 先强制重拉并回填结果，再开弹窗 —— 否则弹窗只展示启动静默检查的
                //（可能来自缓存）结果，出现「空目标版本」的假更新弹窗。
                void checkLauncherUpdate(true).then((i) => {
                  if (i) setUpdateInfo(i);
                  setUpdateOpen(true);
                });
              }}
            />
          )}
        </div>
        <BottomBar
          onShowData={() => setShowData(true)}
          onShowHelp={() => setShowHelp(true)}
          onShowImport={() => setImportOpen(true)}
          // 语言包预检弹窗「前往模组管理」：切到「模组」视图并进入「模组管理」子语境（ModManagerPanel）。
          // 即便当前已在「模组」页（默认视图，且 ModView 默认处于预设编排态），也要把 mode 切到 manage——
          // 否则弹窗关闭后停留在预设态，看起来"点了没反应"。
          // （领域页不渲染此 BottomBar，由 RealmShell 的 onGoToMods 负责「退出领域 + 切模组页 + 进管理语境」）
          onGoToMods={() => {
            changeView("mods");
            setModsMode("manage");
          }}
          modsLoading={modsLoading}
        />
      </div>

      {/* 导入弹层（z-[150]，高于「游戏数据」抽屉与确认框） */}
      <ImportFilesDialog
        open={importOpen}
        dragActive={importDragOver}
        seedPaths={importSeed}
        seedNonce={importSeedNonce}
        onClose={() => {
          setImportOpen(false);
          // 种子路径是一次性消费：关闭即清空，否则下次从底栏打开时旧列表会被重新灌入
          setImportSeed([]);
        }}
      />

      {/* 抽屉在根层渲染，避免被 footer 的 stacking context 裁剪 */}
      {showData && <GameDataDrawer onClose={() => setShowData(false)} />}
      {showHelp && <HelpDrawer onClose={() => setShowHelp(false)} />}
      {selfCheck && <RealmSelfCheckDialog report={selfCheck} onClose={() => setSelfCheck(null)} />}
      {/* 普通更新弹窗（App 顶层挂载：⚙ 自动弹 / 设置页按钮共用；强制更新走上面的 overlay） */}
      {updateInfo && !updateInfo.forced && (
        <UpdateDialog
          info={updateInfo}
          open={updateOpen}
          onClose={() => setUpdateOpen(false)}
          onInfoChange={setUpdateInfo}
          onSkip={onSkipUpdate}
        />
      )}
      {globalNotice && <NoticeBar {...globalNotice} onClose={() => setGlobalNotice(null)} />}
    </>
  );
}

export default function App() {
  return (
    <I18nProvider>
      <AppContent />
    </I18nProvider>
  );
}
