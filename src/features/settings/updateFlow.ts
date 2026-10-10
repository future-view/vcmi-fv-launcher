//! 更新流程 hook（UpdateDialog 与 ForcedUpdateOverlay 共用）。
//!
//! 状态机：idle → downloading → downloaded → applying → restarting；
//! 取消回 idle（可重试），任何失败回 error（可重试）。
//! 下载在后端是**阻塞命令**，前端只负责进度展示与按钮分发；
//! `autoApply=true`（强制更新）时下载校验完成即自动应用，不再等用户确认。

import { useCallback, useEffect, useState } from "react";
import {
  applyLauncherUpdate,
  cancelLauncherUpdate,
  closeWindow,
  downloadLauncherUpdate,
  onUpdateProgress,
  UPDATE_DOWNLOAD_CANCELLED,
  type UpdateInfo,
} from "@/lib/tauri";
import { backendErrorText } from "@/lib/backendError";

export type UpdateUiState =
  | "idle"
  | "downloading"
  | "downloaded"
  | "applying"
  | "restarting"
  /** **纯配置更新包**已就地应用（`restarted === false`）—— 不关窗，提示重启后生效 */
  | "applied"
  | "error";

export interface UpdateFlow {
  ui: UpdateUiState;
  progress: { received: number; total: number } | null;
  err: string | null;
  start: () => Promise<void>;
  cancel: () => Promise<void>;
  apply: () => Promise<void>;
}

export function useUpdateFlow(info: UpdateInfo, autoApply: boolean): UpdateFlow {
  // 弹窗重开时从后端状态恢复现场（后端 check 已把 Downloading/Downloaded 映射进 info.state）
  const [ui, setUi] = useState<UpdateUiState>(() =>
    info.state === "downloaded" ? "downloaded" : "idle",
  );
  const [progress, setProgress] = useState<{ received: number; total: number } | null>(null);
  const [err, setErr] = useState<string | null>(null);

  const apply = useCallback(async () => {
    setErr(null);
    setUi("applying");
    const res = await applyLauncherUpdate();
    if (res.ok) {
      if (res.restarted === false) {
        // 纯配置更新包：已就地应用 —— **不要关窗**（关窗反而打断用户）
        if (res.failed?.length) {
          console.warn("[update] 个别文件被占用，重启启动器后生效：", res.failed);
        }
        setUi("applied");
        return;
      }
      setUi("restarting");
      // 后端已启动 detached 替换脚本：延迟等 IPC 响应送达后再关窗（脚本按 PID 等进程退出）
      window.setTimeout(() => {
        void closeWindow();
      }, 800);
    } else {
      setErr(backendErrorText(res.error ?? ""));
      setUi("error");
    }
  }, []);

  const start = useCallback(async () => {
    setErr(null);
    setUi("downloading");
    setProgress({ received: 0, total: info.size ?? 0 });
    const un = await onUpdateProgress((p) => setProgress({ received: p.received, total: p.total }));
    const res = await downloadLauncherUpdate();
    un();
    setProgress(null);
    if (res.ok) {
      if (autoApply) {
        await apply();
      } else {
        setUi("downloaded");
      }
    } else if (res.error === UPDATE_DOWNLOAD_CANCELLED) {
      setUi("idle"); // 用户取消：回首态可重试
    } else {
      setErr(backendErrorText(res.error ?? ""));
      setUi("error");
    }
  }, [info.size, autoApply, apply]);

  const cancel = useCallback(async () => {
    await cancelLauncherUpdate();
  }, []);

  // 卸载时若仍在下载（弹窗关闭不中断下载），无需特殊处理——后端继续，重开时恢复现场
  useEffect(() => () => setProgress(null), []);

  return { ui, progress, err, start, cancel, apply };
}
