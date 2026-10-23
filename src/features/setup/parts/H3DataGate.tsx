import { useI18n } from "@/i18n";
import type { H3DataKind, H3DataStatus } from "@/lib/tauri";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { fmtBytes, h3SourceBlockText, h3UnsupportedText, type CopyProgressState } from "@/features/setup/setupShared";

/**
 * 原版数据内联闸门（向导第 ③ 步本体，**固定步骤**）：
 * - 就绪：一行绿色状态（含命中根：游戏目录 / 用户数据目录）；
 * - 缺失 / 探测失败：就地展开数据目录选择 + 复制 + 重试；
 * - 检测中：占位"检测中…"，避免状态行忽隐忽现造成布局跳动。
 */
export function H3DataGate({
  h3,
  h3Checking,
  h3Probed,
  h3DataDir,
  setH3DataDir,
  h3Candidates,
  sourceKind,
  sourceChecking,
  copyMp3,
  setCopyMp3,
  copyMaps,
  setCopyMaps,
  lodTarget,
  setLodTarget,
  gameDir,
  userDataDir,
  onChooseDir,
  onCopy,
  onRedetect,
  copying,
  pct,
  progress,
}: {
  h3: H3DataStatus | null;
  h3Checking: boolean;
  h3Probed: boolean;
  h3DataDir: string;
  setH3DataDir: (v: string) => void;
  h3Candidates: string[];
  /** 待复制**源**目录的数据模态（`null` = 未检/无法识别） */
  sourceKind: H3DataKind | null;
  sourceChecking: boolean;
  copyMp3: boolean;
  setCopyMp3: (v: boolean) => void;
  copyMaps: boolean;
  setCopyMaps: (v: boolean) => void;
  /** `.lod` 复制目标：游戏目录 Data（默认，领域安全）/ 用户数据目录 Data */
  lodTarget: "game" | "user";
  setLodTarget: (v: "game" | "user") => void;
  gameDir: string;
  userDataDir: string;
  onChooseDir: () => void | Promise<unknown>;
  onCopy: () => void | Promise<unknown>;
  onRedetect: () => void | Promise<unknown>;
  copying: boolean;
  pct: number;
  progress: CopyProgressState;
}) {
  const { t } = useI18n();
  /** 目标（当前游戏/用户目录）与源（待复制目录）的不支持原因，文案同源 */
  const unsupportedTarget = h3UnsupportedText(h3?.kind, t);
  const unsupportedSource = h3SourceBlockText(sourceKind, t);

  /**
   * 进度标题：`copy_extra` 逐段搬 `Mp3` / `Maps`，每段各自有分母。
   * 不区分阶段时用户会看到两次一模一样的「正在复制数据文件 xx%」（用户实测反馈）。
   */
  const copyStageLabel =
    progress.stage === "mp3"
      ? t.setup.h3CopyingMp3
      : progress.stage === "maps"
        ? t.setup.h3CopyingMaps
        : t.setup.h3Copying;

  // 检测中（尚未有结果）：占位，不展开操作区
  if (h3Checking && !h3Probed) {
    return (
      <p className="break-words text-xs text-slate-500">
        <span className="mr-1.5 text-slate-400">○</span>
        {t.setup.h3Title}: {t.setup.h3Checking}
      </p>
    );
  }

  // 就绪：一行绿色状态（说明命中在哪个根）；demo 数据单独说明（VCMI 放行但功能受限）
  if (h3Probed && h3?.found) {
    return (
      <p className="break-words text-xs text-emerald-400">
        <span className="mr-1.5">●</span>
        {h3.kind === "demo"
          ? t.setup.h3DemoReady
          : t.setup.h3FoundIn.replace("{arg}", h3.inGame ? t.setup.gameDir : t.setup.userDataDir)}
      </p>
    );
  }

  // 探测失败：禁用的原因 + 重试
  if (!h3Probed) {
    return (
      <div className="rounded-xl border border-slate-700 bg-slate-950/40 p-3">
        <p className="break-words text-xs text-amber-300">
          <span className="mr-1.5">●</span>
          {t.setup.h3Title}: {t.setup.h3ProbeFailed}
        </p>
        <div className="mt-2">
          <Button
            type="button"
            variant="outline"
            onClick={onRedetect}
            disabled={h3Checking}
            className="border-slate-700 text-slate-300 hover:bg-slate-800"
          >
            {t.setup.h3Retry}
          </Button>
        </div>
      </div>
    );
  }

  // 缺失：就地展开（目标固定游戏目录 Data，与用户数据目录选择无关）
  return (
    <div className="rounded-xl border border-amber-800/60 bg-amber-950/20 p-3">
      <p className="break-words text-xs text-amber-200">
        <span className="mr-1.5">●</span>
        {/* 版本不支持优先于"缺文件"：HD 只会缺 .lod、RoE/AB 根本不缺文件，报"缺文件"会误导 */}
        {unsupportedTarget ??
          t.setup.h3Missing.replace("{arg}", h3?.missing.join("、") || "—")}
      </p>
      <p className="mt-1 break-words text-xs text-slate-400">
        {unsupportedTarget ? t.setup.h3VersionHint : t.setup.h3MissingHint}
      </p>

      <div className="mt-2">
        <div className="mb-1 flex items-baseline justify-between gap-2">
          <label className="text-sm font-medium text-slate-300">{t.setup.h3DataDir}</label>
          <span className="min-w-0 break-words text-xs text-slate-500">
            {t.setup.h3DataDirHint}
          </span>
        </div>
        <div className="flex gap-2">
          <Input
            value={h3DataDir}
            onChange={(e) => setH3DataDir(e.target.value)}
            placeholder={h3Candidates[0] ?? ""}
            className="min-w-0 flex-1 bg-slate-950/60 text-slate-200 placeholder:text-slate-600"
          />
          <Button
            type="button"
            variant="outline"
            onClick={onChooseDir}
            className="shrink-0 border-slate-700 text-slate-300 hover:bg-slate-800"
          >
            {t.setup.choose}
          </Button>
        </div>
        {/* 源目录版本提示：选完立刻判（RoE/AB/HD 复制过去也起不来，禁止白搬） */}
        {sourceChecking && (
          <p className="mt-1 break-words text-xs text-slate-500">{t.setup.h3Checking}</p>
        )}
        {!sourceChecking && unsupportedSource && (
          <p className="mt-1 break-words text-xs text-amber-300">{unsupportedSource}</p>
        )}
        {/* 复制目标：默认游戏目录（领域共享）；用户数据目录仅在不适用领域隔离时选 */}
        <div className="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1">
          <span className="text-xs text-slate-400">{t.setup.h3CopyTarget}</span>
          <label className="flex cursor-pointer items-center gap-1.5 text-xs text-slate-300">
            <input
              type="radio"
              name="h3-lod-target"
              checked={lodTarget === "game"}
              onChange={() => setLodTarget("game")}
              disabled={copying}
              className="h-3.5 w-3.5 accent-sky-500"
            />
            {t.setup.h3TargetGame}
          </label>
          <label className="flex cursor-pointer items-center gap-1.5 text-xs text-slate-300">
            <input
              type="radio"
              name="h3-lod-target"
              checked={lodTarget === "user"}
              onChange={() => setLodTarget("user")}
              disabled={copying}
              className="h-3.5 w-3.5 accent-sky-500"
            />
            {t.setup.h3TargetUser}
          </label>
        </div>
        {lodTarget === "user" && (
          <p className="mt-1 break-words text-xs text-amber-400">{t.setup.h3TargetUserWarn}</p>
        )}
        <p className="mt-1 break-words text-xs text-slate-500">
          {t.setup.h3Target.replace(
            "{arg}",
            lodTarget === "game" ? `${gameDir}\\Data` : `${userDataDir}\\Data`,
          )}
        </p>

        {/* Data 内的 .lod/.snd/.vid 全搬（官方同口径，含 VIDEO.VID）；Mp3/Maps 由下面两个开关控制 */}
        <p className="mt-1 break-words text-xs text-slate-500">{t.setup.h3CopyScope}</p>
        <div className="mt-2 flex flex-wrap items-center gap-x-4 gap-y-1">
          <label className="flex cursor-pointer items-center gap-1.5 text-xs text-slate-300">
            <input
              type="checkbox"
              checked={copyMp3}
              onChange={(e) => setCopyMp3(e.target.checked)}
              disabled={copying}
              className="h-3.5 w-3.5 accent-sky-500"
            />
            {t.setup.h3CopyMp3}
          </label>
          <label className="flex cursor-pointer items-center gap-1.5 text-xs text-slate-300">
            <input
              type="checkbox"
              checked={copyMaps}
              onChange={(e) => setCopyMaps(e.target.checked)}
              disabled={copying}
              className="h-3.5 w-3.5 accent-sky-500"
            />
            {t.setup.h3CopyMaps}
          </label>
        </div>
        <p className="mt-1 break-words text-xs text-slate-500">{t.setup.h3CopyHint}</p>

        {h3Candidates.length > 0 && (
          <p className="mt-1 break-words text-xs text-slate-500">
            {t.setup.h3Candidates.replace("{arg}", h3Candidates.join("；"))}
          </p>
        )}
      </div>

      {copying ? (
        <div className="mt-2">
          <div className="text-xs text-slate-200">
            {copyStageLabel.replace("{arg}", `${pct}%`)}
          </div>
          <div className="mt-1.5 h-1.5 w-full overflow-hidden rounded bg-slate-800">
            <div
              className="h-full bg-sky-500 transition-[width] duration-200"
              style={{ width: `${pct}%` }}
            />
          </div>
          <div className="mt-1.5 break-words text-xs text-slate-400">
            {t.setup.copyBytes.replace(
              "{arg}",
              `${fmtBytes(progress.doneBytes)} / ${fmtBytes(progress.totalBytes)}`,
            )}
            {" · "}
            {t.setup.copyFiles.replace("{arg}", `${progress.doneFiles} / ${progress.totalFiles}`)}
          </div>
        </div>
      ) : (
        <div className="mt-2 flex flex-wrap items-center gap-3">
          <Button
            type="button"
            onClick={onCopy}
            disabled={!h3DataDir.trim() || sourceChecking || !!unsupportedSource}
            className="bg-sky-600 text-white hover:bg-sky-500"
          >
            {t.setup.h3Copy}
          </Button>
          <Button
            type="button"
            variant="outline"
            onClick={onRedetect}
            disabled={h3Checking}
            className="border-slate-700 text-slate-300 hover:bg-slate-800"
          >
            {t.setup.h3Redetect}
          </Button>
        </div>
      )}
    </div>
  );
}
