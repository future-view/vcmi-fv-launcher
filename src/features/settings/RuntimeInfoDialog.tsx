import { Info, X } from "lucide-react";
import type { RuntimeInfo } from "@/lib/tauri";
import { useI18n } from "@/i18n";

interface RuntimeInfoDialogProps {
  info: RuntimeInfo | null;
  open: boolean;
  onClose: () => void;
  /** 作用域文案（全局共享 / 领域独立）：拼进标题，明确"这份数据来自哪个作用域" */
  scopeLabel?: string;
  /** 该作用域对应的配置目录（数据来源；长路径截断，hover 显示完整） */
  sourcePath?: string;
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between border-b border-slate-800 py-2 last:border-0">
      <span className="text-sm text-muted">{label}</span>
      <span className="font-mono text-sm text-foreground">{value}</span>
    </div>
  );
}

/** 分组标题：本配置目录（跟随作用域）/ 环境（全局） */
function GroupTitle({ children }: { children: React.ReactNode }) {
  return (
    <h4 className="mb-1 mt-3 text-[11px] font-semibold uppercase tracking-wide text-slate-500 first:mt-0">
      {children}
    </h4>
  );
}

/**
 * 运行信息弹窗（设计文档 §六）：只读键值表。
 *
 * v2 重构（2026-09-29）：入口从工具条 ℹ 按钮改为「更多」菜单；**按当前配置目录取数**
 * （`get_runtime_info(config_dir)`，与设置读写同一作用域）—— 领域隔离后领域目录有自己的
 * `settings.json` / `modSettings.json` 运行态，若读全局会显示过期值。
 * 内容按「本配置目录（跟随作用域）/ 环境（全局）」两组呈现，字段来源一目了然。
 */
export function RuntimeInfoDialog({
  info,
  open,
  onClose,
  scopeLabel,
  sourcePath,
}: RuntimeInfoDialogProps) {
  const { t } = useI18n();
  if (!open) return null;

  const res = info?.recommendedResolution;
  const resText = res ? `${res.width} × ${res.height}` : "—";

  return (
    <div
      className="fixed inset-0 z-[150] flex items-center justify-center bg-black/70 p-4 backdrop-blur-sm"
      onClick={onClose}
    >
      <div
        className="w-[min(92vw,420px)] rounded-2xl border border-slate-700/50 bg-slate-900 p-5 shadow-2xl"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="mb-3 flex items-center gap-2">
          <Info className="h-5 w-5 text-blue-400" />
          <h3 className="text-base font-semibold text-foreground">
            {t.settings.runtimeInfo}
            {scopeLabel ? ` · ${scopeLabel}` : ""}
          </h3>
          <button
            type="button"
            onClick={onClose}
            className="ml-auto rounded-md p-1 text-slate-400 hover:bg-slate-800 hover:text-foreground"
            aria-label={t.settings.runtimeClose}
          >
            <X className="h-4 w-4" />
          </button>
        </div>

        {/* 数据来源（有 configDir 时给出完整目录；否则退回只读提示） */}
        {sourcePath ? (
          <p className="mb-1 truncate text-xs text-muted" title={sourcePath}>
            {t.settings.runtimeSource}: {sourcePath}
          </p>
        ) : (
          <p className="mb-1 text-xs text-muted">{t.settings.runtimeHint}</p>
        )}

        <div>
          {/* 本配置目录：跟随作用域（读当前 configDir 的 settings.json / modSettings.json） */}
          <GroupTitle>{t.settings.runtimeGroupConfig}</GroupTitle>
          <Row label={t.settings.runtimeLastMap} value={info?.lastMap ? info.lastMap : "—"} />
          <Row label={t.settings.runtimeLastSave} value={info?.lastSave ? info.lastSave : "—"} />
          <Row
            label={t.settings.runtimeGameDataLang}
            value={info?.gameDataLanguage || "auto"}
          />
          <Row
            label={t.settings.runtimeModsEnabled}
            value={info ? String(info.modsEnabled) : "—"}
          />
          <Row
            label={t.settings.runtimeSetupCompleted}
            value={info?.setupCompleted ? "✓" : "—"}
          />
          <Row
            label={t.settings.runtimeMainWindow}
            value={
              info?.mainWindow?.geometry
                ? `${info.mainWindow.geometry.x ?? 0}, ${info.mainWindow.geometry.y ?? 0} · ${info.mainWindow.geometry.width ?? 0}×${info.mainWindow.geometry.height ?? 0}`
                : "—"
            }
          />

          {/* 环境：与作用域无关（引擎探测 / 磁盘扫描 / 显示器） */}
          <GroupTitle>{t.settings.runtimeGroupEnv}</GroupTitle>
          <Row label={t.settings.runtimeVersion} value={info?.vcmiVersion ?? "—"} />
          <Row
            label={t.settings.runtimeModsInstalled}
            value={info ? String(info.modsInstalled) : "—"}
          />
          <Row label={t.settings.runtimeResolution} value={resText} />
        </div>

        <button
          type="button"
          onClick={onClose}
          className="mt-4 w-full rounded-lg bg-slate-800 py-2 text-sm text-foreground hover:bg-slate-700"
        >
          {t.settings.runtimeClose}
        </button>
      </div>
    </div>
  );
}
