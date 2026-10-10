/**
 * 模组源弹层（从 `ModManagerPanel` 抽取，realm-mods-management §10.2）：
 * 就地编辑 vcmiSettings 的 4 个仓库键（默认源开关/地址、额外源开关/地址），与官方启动器互通。
 *
 * 使用方式（两处共用）：
 * - 模组管理页（`ModManagerPanel` 顶部工具栏「模组源」按钮）；
 * - 领域管理页（`RealmModManagerPanel` 已实装/可下载/可更新工具栏，§5.2）。
 *
 * 父组件用 `relative` 容器包裹触发按钮 + 本组件（绝对定位弹层）；
 * 失焦关闭由父组件处理（沿用 `document mousedown` 点外部关闭模式）。
 */
import { useEffect, useState } from "react";
import { useI18n } from "@/i18n";
import { readVcmiSettings, writeVcmiSettings } from "@/lib/tauri";

/** 官方默认仓库地址（与后端 `mods.rs` 的 `OFFICIAL_REPO_FALLBACK` 同值）：
 * 设置里缺 `defaultRepositoryURL` 键时官方启动器也用它，弹层同样回显，避免“空输入框”假象。 */
export const DEFAULT_REPO_URL =
  "https://raw.githubusercontent.com/vcmi/vcmi-mods-repository/develop/vcmi-1.7.json";

interface ModSourceState {
  defOn: boolean;
  defUrl: string;
  exOn: boolean;
  exUrl: string;
}

export function ModSourcePopover({ onClose, onSaved }: { onClose: () => void; onSaved?: () => void }) {
  const { t } = useI18n();
  const m = t.modsManage;
  const [src, setSrc] = useState<ModSourceState | null>(null);
  const [busy, setBusy] = useState(false);

  // 打开即读 vcmiSettings（缺键 = 默认源启用 / 额外源禁用，回显实际生效地址）
  useEffect(() => {
    let alive = true;
    (async () => {
      try {
        const s = (await readVcmiSettings()) as { launcher?: Record<string, unknown> } | null;
        const l = s?.launcher ?? {};
        if (!alive) return;
        setSrc({
          defOn: typeof l.defaultRepositoryEnabled === "boolean" ? l.defaultRepositoryEnabled : true,
          defUrl:
            typeof l.defaultRepositoryURL === "string" && l.defaultRepositoryURL
              ? l.defaultRepositoryURL
              : DEFAULT_REPO_URL,
          exOn: typeof l.extraRepositoryEnabled === "boolean" ? l.extraRepositoryEnabled : false,
          exUrl: typeof l.extraRepositoryURL === "string" ? l.extraRepositoryURL : "",
        });
      } catch {
        if (alive) setSrc({ defOn: true, defUrl: DEFAULT_REPO_URL, exOn: false, exUrl: "" });
      }
    })();
    return () => {
      alive = false;
    };
  }, []);

  const save = async () => {
    if (!src) return;
    setBusy(true);
    try {
      await writeVcmiSettings({
        launcher: {
          defaultRepositoryEnabled: src.defOn,
          defaultRepositoryURL: src.defUrl,
          extraRepositoryEnabled: src.exOn,
          extraRepositoryURL: src.exUrl,
        },
      });
      onClose();
      onSaved?.();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="absolute right-0 z-[70] mt-1 w-[38rem] rounded-lg border border-slate-700 bg-slate-900 p-4 shadow-2xl">
      <div className="mb-3 flex items-center justify-between">
        <h3 className="text-sm font-semibold text-white">{m.sourceTitle}</h3>
        <button onClick={onClose} className="text-slate-500 hover:text-white">
          ✕
        </button>
      </div>
      {!src ? (
        <div className="py-4 text-center text-sm text-slate-500">{t.modsRepo.checking}</div>
      ) : (
        <div className="space-y-3 text-sm">
          <label className="flex items-center gap-2 text-slate-300">
            <input
              type="checkbox"
              checked={src.defOn}
              onChange={(e) => setSrc({ ...src, defOn: e.target.checked })}
              className="h-3.5 w-3.5 accent-blue-500"
            />
            {m.sourceDefault}
          </label>
          {src.defOn && (
            <input
              value={src.defUrl}
              onChange={(e) => setSrc({ ...src, defUrl: e.target.value })}
              title={src.defUrl}
              placeholder={DEFAULT_REPO_URL}
              className="w-full rounded-lg border border-slate-700 bg-slate-950/60 px-2 py-1.5 text-xs text-white placeholder:text-slate-600 focus:border-blue-500 focus:outline-none"
            />
          )}
          <label className="flex items-center gap-2 text-slate-300">
            <input
              type="checkbox"
              checked={src.exOn}
              onChange={(e) => setSrc({ ...src, exOn: e.target.checked })}
              className="h-3.5 w-3.5 accent-blue-500"
            />
            {m.sourceExtra}
          </label>
          {src.exOn && (
            <input
              value={src.exUrl}
              onChange={(e) => setSrc({ ...src, exUrl: e.target.value })}
              title={src.exUrl}
              placeholder="https://…/fv-mod-index.json"
              className="w-full rounded-lg border border-slate-700 bg-slate-950/60 px-2 py-1.5 text-xs text-white placeholder:text-slate-600 focus:border-blue-500 focus:outline-none"
            />
          )}
          <div className="flex justify-end gap-2 pt-1">
            <button
              onClick={() => void save()}
              disabled={busy || (src.defOn && !src.defUrl.trim()) || (src.exOn && !src.exUrl.trim())}
              className="rounded-lg bg-blue-600 px-3 py-1.5 text-xs font-semibold text-white hover:bg-blue-500 disabled:opacity-50"
            >
              {busy ? t.modsRepo.checking : t.preset.save}
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
