import { useEffect, useMemo, useState } from "react";
import { useI18n } from "@/i18n";
import { Button } from "@/shared/ui/button";
import { launchGame, launchEditor, launchNativeLauncher } from "@/lib/tauri";
import { backendErrorText } from "@/lib/backendError";
import { useRealms } from "@/lib/realm/store";
import { realmName } from "@/lib/realm/intro";
import { getTownLoading, subscribeTownLoading } from "@/lib/townLoading";
import { PreLaunchCheckDialog } from "@/features/prelaunch/PreLaunchCheckDialog";
import {
  usePreLaunchChecks,
  type PreLaunchStep,
} from "@/features/prelaunch/usePreLaunchChecks";
import { makeLangPackStep } from "@/features/prelaunch/LangPackStepCard";

interface BottomBarProps {
  onShowData: () => void;
  onShowHelp: () => void;
  /** 「导入游戏文件」弹层入口（位于「游戏数据」左侧） */
  onShowImport: () => void;
  /** 前往「模组」页（语言包预检弹窗里「前往模组管理」的引导出口） */
  onGoToMods?: () => void;
  modsLoading?: boolean;
}

export function BottomBar({
  onShowData,
  onShowHelp,
  onShowImport,
  onGoToMods,
  modsLoading = false,
}: BottomBarProps) {
  const { t, vcmiLang } = useI18n();
  const [toast, setToast] = useState<string | null>(null);
  const [townLoading, setTownLoading] = useState(false);
  /** 启动按钮态：idle / checking（本地扫描中）/ launching（拉起游戏中） */
  const [launchState, setLaunchState] = useState<"idle" | "checking" | "launching">("idle");
  /** 启动前检测编排（通用 step 框架） */
  const prelaunch = usePreLaunchChecks();

  useEffect(() => {
    setTownLoading(getTownLoading() > 0);
    return subscribeTownLoading((n) => setTownLoading(n > 0));
  }, []);

  const showToast = (msg: string) => {
    setToast(msg);
    setTimeout(() => setToast(null), 2200);
  };

  // §9.2.0：领域激活时底部区要给出可见提示（此时「启动游戏」启动的仍是当前领域环境）
  const { realms, activeRealm } = useRealms();
  const activeRealmObj = realms.find((r) => r.id === activeRealm) ?? null;

  const proceedLaunch = async () => {
    const res = await launchGame();
    showToast(backendErrorText(res.error) || t.launch.launcherLaunching);
  };

  // 弹窗内动作的统一收尾：**先关弹窗再执行**。
  // ① 启动游戏后弹窗必须消失（否则游戏关掉回来仍停在"启动前检测"）；
  // ② 「前往模组管理」切换页面后，弹窗若不关会以 z-150 遮罩盖住新页面，看起来"点了没反应"。
  const dismissLangCheckAndProceed = () => {
    prelaunch.close();
    void proceedLaunch();
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
        realmId: activeRealmObj?.id ?? null,
        realmName: activeRealmObj ? realmName(activeRealmObj, t.uiLang) : undefined,
        onProceed: dismissLangCheckAndProceed,
        onGoToMods: dismissLangCheckAndGoToMods,
        onBusyChange: prelaunch.setBusy,
      }),
    ],
    // 闭包已捕获最新 dismiss/proceed；仅依赖影响 step 语义的入参
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [vcmiLang, activeRealmObj, t.launch.stepLangPack, t.uiLang, prelaunch.setBusy],
  );

  const handleLaunch = async () => {
    if (modsLoading) {
      showToast(t.mods.loading);
      return;
    }
    // 城镇图片仍在异步提取时，先弹窗提示用户等待或继续启动
    if (townLoading) {
      const confirmed = window.confirm(t.mods.loadingConfirm);
      if (!confirmed) {
        return;
      }
    }
    // 两层加载归属（docs/features/lang-mods §10.2）：本地扫描阶段按钮显示"检查中…"，避免"点了没反应"
    setLaunchState("checking");
    const res = await prelaunch.run(prelaunchSteps);
    if (res === "proceed") {
      // 全部 ok → 不弹窗，按钮转"启动中…"并持续 loading 直到拉起
      setLaunchState("launching");
      await proceedLaunch();
      setLaunchState("idle");
    } else {
      // 有问题 → 弹窗已开，按钮恢复常态（取消 loading），由弹窗接管进度
      setLaunchState("idle");
    }
  };

  const handleEditor = async () => {
    const res = await launchEditor();
    showToast(backendErrorText(res.error) || t.launch.editorLaunching);
  };

  const handleNativeLauncher = async () => {
    const res = await launchNativeLauncher();
    showToast(backendErrorText(res.error) || t.launch.nativeLauncher);
  };

  // §9.2.0：领域激活时底部区要给出可见提示（此时「启动游戏」启动的仍是当前领域环境）
  const activeName = activeRealmObj ? realmName(activeRealmObj, t.uiLang) : "";

  return (
    <footer className="fixed bottom-0 left-0 right-0 z-40 flex h-[64px] items-center justify-between border-t border-slate-800/70 bg-slate-950/85 px-5 backdrop-blur-xl select-none">
      <div className="flex items-center gap-3">
        {/* §9.2.0：领域激活时底部区必须给出可见提示——此时「启动游戏」启动的仍是当前领域环境 */}
        {activeName && (
          <span
            className="flex items-center gap-1.5 rounded-lg border border-blue-500/40 bg-blue-500/10 px-2.5 py-1.5 text-xs text-blue-200"
            title={t.realm.bottomActiveRealm.replace("{name}", activeName)}
          >
            <span className="h-1.5 w-1.5 flex-none rounded-full bg-blue-400" />
            <span className="max-w-[160px] truncate">
              {t.realm.bottomActiveRealm.replace("{name}", activeName)}
            </span>
          </span>
        )}
        {/* 入口顺序对齐设计：[导入游戏文件] [游戏数据] [帮助]（indigo 描边区别于导航类按钮） */}
        <Button
          variant="outline"
          size="sm"
          onClick={onShowImport}
          className="border-indigo-500/50 text-indigo-200 hover:bg-indigo-500/10"
        >
          {t.importFiles.button}
        </Button>
        <Button variant="outline" size="sm" onClick={onShowData}>
          {t.launch.gameData}
        </Button>
        <Button variant="outline" size="sm" onClick={onShowHelp}>
          {t.launch.help}
        </Button>
      </div>

      <div className="flex items-center gap-3">
        <Button variant="outline" size="sm" onClick={handleNativeLauncher}>
          {t.launch.nativeLauncher}
        </Button>
        <Button variant="outline" size="sm" onClick={handleEditor}>
          {t.launch.editor}
        </Button>
        <Button
          variant="success"
          size="sm"
          onClick={handleLaunch}
          disabled={modsLoading || launchState !== "idle"}
          className={modsLoading || launchState !== "idle" ? "cursor-not-allowed opacity-50" : ""}
        >
          {launchState === "idle"
            ? t.launch.launcher
            : launchState === "checking"
              ? t.launch.checking
              : t.launch.launching}
        </Button>
      </div>

      {/* Toast */}
      {toast && (
        <div className="fixed bottom-20 left-1/2 z-[95] -translate-x-1/2 rounded-xl bg-green-500/95 px-5 py-2.5 text-sm text-white shadow-xl">
          {toast}
        </div>
      )}

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
    </footer>
  );
}
