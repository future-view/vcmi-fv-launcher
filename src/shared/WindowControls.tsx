import { useEffect, useState } from "react";
import { Settings } from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import {
  closeWindow,
  isWindowMaximized,
  minimizeWindow,
  onWindowResized,
  toggleMaximizeWindow,
} from "@/lib/tauri";

interface WindowControlsProps {
  /** 传入则显示「设置」按钮；不传则只有窗口按钮 */
  onSettings?: () => void;
  /** 「设置」按钮的 title（主界面传「设置」，领域界面传「领域设置」） */
  settingsTitle?: string;
  /** 设置视图处于激活态时高亮「设置」按钮（主界面用；领域覆盖层不传） */
  settingsActive?: boolean;
  /** 有新版本可用：⚙ 左上角显示呼吸红点（纯装饰 pointer-events-none，不影响按钮命中） */
  settingsBadge?: boolean;
  /** 红点 hover 说明（拼在按钮 title 前部） */
  settingsBadgeTitle?: string;
  /** 「设置」与「最小化」之间的附加控件（如「多语言」下拉），由调用方决定是否提供 */
  extra?: React.ReactNode;
}

/**
 * 窗口控件（§9.2.0）：设置（可选）/ 附加控件（可选）/ 最小化 / 最大化 / 关闭。
 *
 * `TopBar` 与领域覆盖层 `RealmShell` **共用**本组件；登场演出界面不使用任何窗口控件（§9.2.1）。
 *
 * 最大化状态以**系统真值**为准（`isMaximized` + `onResized`）：按钮点击后系统消息是异步的，
 * 用户也可能通过双击拖拽区（`data-tauri-drag-region` 的内建行为）最大化，本地布尔值会脱节。
 */
export function WindowControls({
  onSettings,
  settingsTitle,
  settingsActive,
  settingsBadge,
  settingsBadgeTitle,
  extra,
}: WindowControlsProps) {
  const { t } = useI18n();
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    let alive = true;
    let unlisten: (() => void) | null = null;
    const sync = () => {
      void isWindowMaximized().then((m) => {
        if (alive) setMaximized(m);
      });
    };
    sync();
    void onWindowResized(sync).then((fn) => {
      // 订阅是异步建立的：期间若已卸载，立即退订，避免泄漏
      if (alive) unlisten = fn;
      else fn();
    });
    return () => {
      alive = false;
      unlisten?.();
    };
  }, []);

  const handleMaximize = async () => {
    await toggleMaximizeWindow();
    // onResized 之外的双保险：最大化后立即读一次真实状态（系统消息可能晚于本次 await 到达）
    window.setTimeout(() => {
      void isWindowMaximized().then(setMaximized);
    }, 150);
  };

  return (
    <div className="flex flex-none items-center gap-1">
      {onSettings && (
        <button
          type="button"
          onClick={onSettings}
          title={
            settingsBadge && settingsBadgeTitle
              ? `${settingsBadgeTitle} · ${settingsTitle ?? t.realm.settings}`
              : (settingsTitle ?? t.realm.settings)
          }
          className={cn(
            "relative flex h-8 w-8 cursor-pointer items-center justify-center rounded-lg transition-colors",
            settingsActive
              ? "bg-slate-700/70 text-white"
              : "text-slate-400 hover:bg-slate-700/60 hover:text-white",
          )}
        >
          <Settings className="h-4 w-4" />
          {/* 更新红点：absolute + pointer-events-none，不参与命中测试，点击行为完全不变 */}
          {settingsBadge && (
            <span className="pointer-events-none absolute right-0.5 top-0.5 h-2 w-2 animate-pulse rounded-full bg-red-500 shadow shadow-red-500/60" />
          )}
        </button>
      )}
      {extra}
      <button
        type="button"
        onClick={minimizeWindow}
        title={t.common.minimize}
        className="flex h-8 w-8 cursor-pointer items-center justify-center rounded-lg text-slate-400 transition-colors hover:bg-slate-700/60 hover:text-white"
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
          <path d="M5 12h14" />
        </svg>
      </button>
      <button
        type="button"
        onClick={handleMaximize}
        title={maximized ? t.common.restore : t.common.maximize}
        className="flex h-8 w-8 cursor-pointer items-center justify-center rounded-lg text-slate-400 transition-colors hover:bg-slate-700/60 hover:text-white"
      >
        {maximized ? (
          // 还原：两个错开的方框
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <rect x="8" y="3" width="13" height="13" rx="1" />
            <path d="M19 16v3a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h3" />
          </svg>
        ) : (
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
            <rect x="4" y="4" width="16" height="16" rx="1" />
          </svg>
        )}
      </button>
      <button
        type="button"
        onClick={closeWindow}
        title={t.common.close}
        className="flex h-8 w-8 items-center justify-center rounded-lg text-slate-400 transition-colors hover:bg-red-500/20 hover:text-red-400"
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round">
          <path d="M18 6 6 18M6 6l12 12" />
        </svg>
      </button>
    </div>
  );
}
