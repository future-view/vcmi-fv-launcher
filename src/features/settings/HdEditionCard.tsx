//! 设置页「FV启动器」标签下的 **HD 素材来源** 状态卡（官方 HD 版素材 mod `hd-edition` 的 `.pak`）。
//!
//! 只读展示：是否检测到、来源目录、可用档位、配置是否关闭，并提示"改动后需重建素材缓存"。
//! 探测走 `hd_edition_status`（后端只读，不注入构建上下文）。
//!
//! ⚠️ 非 Tauri（浏览器 mock）时 `hdEditionStatus()` 返回 null → 整卡不渲染。

import { useCallback, useEffect, useState } from "react";
import { RefreshCw } from "lucide-react";

import { useI18n } from "@/i18n";
import { hdEditionStatus, type HdEditionStatus } from "@/lib/tauri";

export function HdEditionCard() {
  const { t } = useI18n();
  const [st, setSt] = useState<HdEditionStatus | null>(null);
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    setBusy(true);
    try {
      setSt(await hdEditionStatus());
    } finally {
      setBusy(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  // 导入弹层生成 / 替换 `hd-edition` 后即时刷新（事件由 ImportFilesDialog 广播）
  useEffect(() => {
    const handler = (e: Event) => {
      const mods = (e as CustomEvent<{ mods?: { modId: string }[] }>).detail?.mods ?? [];
      if (mods.some((m) => m.modId === "hd-edition")) void load();
    };
    window.addEventListener("vcmi-fv:mods-installed", handler);
    return () => window.removeEventListener("vcmi-fv:mods-installed", handler);
  }, [load]);

  if (!st) return null;

  const scales = st.scales.map((s) => `x${s}`).join(", ");
  return (
    <div className="glass rounded-xl p-4">
      <div className="flex items-center justify-between gap-3">
        <div className="text-sm font-medium">{t.hdEdition.title}</div>
        <button
          type="button"
          onClick={() => void load()}
          disabled={busy}
          className="inline-flex items-center gap-1 rounded-md border border-border/60 px-2 py-1 text-xs text-muted-foreground transition-colors hover:text-foreground disabled:opacity-50"
        >
          <RefreshCw className={busy ? "h-3.5 w-3.5 animate-spin" : "h-3.5 w-3.5"} />
          {t.hdEdition.refresh}
        </button>
      </div>
      <div className="mt-2 space-y-1 text-xs leading-relaxed text-muted-foreground">
        <p>{t.hdEdition.hint}</p>
        {!st.enabled ? (
          <p className="text-warning">{t.hdEdition.disabled}</p>
        ) : st.found ? (
          <>
            <p className="break-all">{t.hdEdition.found.replace("{dir}", st.dir ?? "")}</p>
            <p>{t.hdEdition.scales.replace("{scales}", scales || "-")}</p>
          </>
        ) : (
          <p>{t.hdEdition.missing}</p>
        )}
        <p>{t.hdEdition.rebuildHint}</p>
      </div>
    </div>
  );
}
