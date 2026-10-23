import { useI18n } from "@/i18n";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { Switch } from "@/shared/ui/switch";

/**
 * 可选内容（向导第 ③ 步底部）：高清版素材 / 历代记战役。
 *
 * 两个开关**默认关闭**，勾上后才展开对应目录/文件选择——避免第 ③ 步信息过载。
 * 安装是**点击即装**（保存前显式传 `gameDir` / `userDataDir`），成功后才锁定该行。
 */
export function OptionalContent({
  hdInstallOn,
  setHdInstallOn,
  hdInstallDir,
  hdInstalled,
  hdReady,
  onChooseHdDir,
  onInstallHd,
  chrInstallOn,
  setChrInstallOn,
  chrInstallFile,
  chrInstalled,
  chrReady,
  onChooseChrFile,
  onInstallChr,
  optTarget,
  setOptTarget,
  gameDir,
  userDataDir,
  installing,
}: {
  hdInstallOn: boolean;
  setHdInstallOn: (v: boolean) => void;
  hdInstallDir: string;
  hdInstalled: boolean | null;
  /** 已就绪（本次装好，或盘上已存在）：显示绿色结论，收起开关与安装控件 */
  hdReady: boolean;
  onChooseHdDir: () => void | Promise<unknown>;
  onInstallHd: () => void | Promise<unknown>;
  chrInstallOn: boolean;
  setChrInstallOn: (v: boolean) => void;
  chrInstallFile: string;
  chrInstalled: boolean | null;
  chrReady: boolean;
  onChooseChrFile: () => void | Promise<unknown>;
  onInstallChr: () => void | Promise<unknown>;
  /** 安装目标根：游戏目录（默认，领域共享）/ 用户数据目录 */
  optTarget: "game" | "user";
  setOptTarget: (v: "game" | "user") => void;
  gameDir: string;
  userDataDir: string;
  /**
   * 正在安装的项（`null` = 空闲）。**只让对应按钮**显示「安装中…」：
   * 早先只有一个 boolean，装历代记时已装好的高清素材按钮也会变成「安装中…」（用户实测反馈）。
   */
  installing: "hd" | "chr" | null;
}) {
  const { t } = useI18n();
  const busy = installing !== null;
  const targetRoot = optTarget === "game" ? gameDir : userDataDir;
  const allReady = hdReady && chrReady;
  return (
    <div className="mt-3 rounded-xl border border-slate-800 bg-slate-950/40 p-3">
      {/* 安装目标：默认游戏目录（领域共享）；用户数据目录随领域隔离，选中给警示。
          两项都已就绪时这个选择没有意义，直接不渲染（用户实测反馈）。 */}
      {!allReady && (
        <>
          <div className="mb-2 flex flex-wrap items-center gap-x-4 gap-y-1">
            <span className="text-xs text-slate-400">{t.setup.optTarget}</span>
            <label className="flex cursor-pointer items-center gap-1.5 text-xs text-slate-300">
              <input
                type="radio"
                name="opt-target"
                checked={optTarget === "game"}
                onChange={() => setOptTarget("game")}
                disabled={busy}
                className="h-3.5 w-3.5 accent-sky-500"
              />
              {t.setup.h3TargetGame}
            </label>
            <label className="flex cursor-pointer items-center gap-1.5 text-xs text-slate-300">
              <input
                type="radio"
                name="opt-target"
                checked={optTarget === "user"}
                onChange={() => setOptTarget("user")}
                disabled={busy}
                className="h-3.5 w-3.5 accent-sky-500"
              />
              {t.setup.h3TargetUser}
            </label>
            <span className="min-w-0 break-words font-mono text-xs text-slate-500">
              {targetRoot || "—"}
            </span>
          </div>
          {optTarget === "user" && (
            <p className="mb-2 break-words text-xs text-amber-400">{t.setup.h3TargetUserWarn}</p>
          )}
        </>
      )}

      {/* 已就绪（盘上已有或本次装好）⇒ 只留一行绿色结论，不再给开关与安装控件 */}
      {hdReady ? (
        <p className="break-words text-sm text-emerald-400">{t.setup.hdReady}</p>
      ) : (
        <>
          <label className="flex items-start justify-between gap-3">
            <span className="min-w-0 flex-1">
              <span className="block break-words text-sm font-medium text-slate-300">
                {t.setup.hdInstall}
              </span>
              {hdInstallOn && (
                <span className="mt-0.5 block break-words text-xs text-slate-500">
                  {t.setup.hdInstallHint}
                </span>
              )}
            </span>
            <Switch checked={hdInstallOn} onCheckedChange={setHdInstallOn} disabled={busy} />
          </label>
          {hdInstallOn && (
            <div className="mt-2 flex gap-2">
              <Input
                value={hdInstallDir}
                readOnly
                placeholder={t.setup.hdInstallPlaceholder}
                disabled={busy}
                className="min-w-0 flex-1 bg-slate-950/60 text-slate-200 placeholder:text-slate-600"
              />
              <Button
                type="button"
                variant="outline"
                onClick={onChooseHdDir}
                disabled={busy}
                className="shrink-0 border-slate-700 text-slate-300 hover:bg-slate-800"
              >
                {t.setup.choose}
              </Button>
              <Button
                type="button"
                onClick={onInstallHd}
                disabled={busy || !hdInstallDir.trim()}
                className="shrink-0 bg-sky-600 text-white hover:bg-sky-500"
              >
                {installing === "hd" ? t.setup.installing : t.setup.install}
              </Button>
            </div>
          )}
          {hdInstalled === false && (
            <p className="mt-1 break-words text-xs text-red-400">{t.setup.installFailed}</p>
          )}
        </>
      )}

      {chrReady ? (
        <p className="mt-3 break-words text-sm text-emerald-400">{t.setup.chrReady}</p>
      ) : (
        <>
          <label className="mt-3 flex items-start justify-between gap-3">
            <span className="min-w-0 flex-1">
              <span className="block break-words text-sm font-medium text-slate-300">
                {t.setup.chrInstall}
              </span>
              {chrInstallOn && (
                <span className="mt-0.5 block break-words text-xs text-slate-500">
                  {t.setup.chrInstallHint}
                </span>
              )}
            </span>
            <Switch checked={chrInstallOn} onCheckedChange={setChrInstallOn} disabled={busy} />
          </label>
          {chrInstallOn && (
            <div className="mt-2 flex gap-2">
              <Input
                value={chrInstallFile}
                readOnly
                placeholder={t.setup.chrInstallPlaceholder}
                disabled={busy}
                className="min-w-0 flex-1 bg-slate-950/60 text-slate-200 placeholder:text-slate-600"
              />
              <Button
                type="button"
                variant="outline"
                onClick={onChooseChrFile}
                disabled={busy}
                className="shrink-0 border-slate-700 text-slate-300 hover:bg-slate-800"
              >
                {t.setup.choose}
              </Button>
              <Button
                type="button"
                onClick={onInstallChr}
                disabled={busy || !chrInstallFile.trim()}
                className="shrink-0 bg-sky-600 text-white hover:bg-sky-500"
              >
                {installing === "chr" ? t.setup.installing : t.setup.install}
              </Button>
            </div>
          )}
          {chrInstalled === false && (
            <p className="mt-1 break-words text-xs text-amber-400">{t.setup.chrInstallFailed}</p>
          )}
        </>
      )}
    </div>
  );
}
