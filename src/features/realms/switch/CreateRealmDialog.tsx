import { useEffect, useState } from "react";
import { useI18n } from "@/i18n";
import { checkRealmId } from "@/lib/tauri";
import { createRealmSafe } from "@/lib/realm/store";
import type { Realm } from "@/lib/realm/types";
import { backendErrorCode, backendErrorText } from "@/lib/backendError";

/**
 * 新建领域第一步（「先定 ID 再编辑」）：确定领域 ID 与名称后**立即创建领域**
 * ——后端随即建立 `realms/<id>/` 并落盘 `realm.json`，因此进入编辑器后
 * 地图上传、素材导入、整合模组、导出等依赖领域目录的能力全部即时可用。
 */

/** 领域 ID 合法字符集，与后端 `sanitize_realm_id` 保持一致（字母/数字/下划线/连字符）。 */
const ID_RE = /^[A-Za-z0-9_-]+$/;
const ID_MAX = 64;

type IdError = "" | "empty" | "invalid" | "occupied";

interface Props {
  onCancel: () => void;
  /** 创建成功：回传后端返回的领域（含 id），由父级打开编辑器 */
  onCreated: (realm: Realm) => void;
  showToast: (type: "success" | "error", msg: string) => void;
}

export function CreateRealmDialog({ onCancel, onCreated, showToast }: Props) {
  const { t, lang } = useI18n();
  const [id, setId] = useState("");
  const [name, setName] = useState("");
  const [err, setErr] = useState<IdError>("");
  const [checking, setChecking] = useState(false);
  const [busy, setBusy] = useState(false);

  // 实时校验：字符集本地判定，占用情况防抖 400ms 问后端（领域目录是否已存在）。
  useEffect(() => {
    const v = id.trim();
    if (!v) {
      setErr("");
      setChecking(false);
      return;
    }
    if (!ID_RE.test(v) || v.length > ID_MAX) {
      setErr("invalid");
      setChecking(false);
      return;
    }
    let alive = true;
    setChecking(true);
    const timer = window.setTimeout(async () => {
      try {
        const r = await checkRealmId(v);
        if (!alive) return;
        setErr(r.available ? "" : r.reason === "occupied" ? "occupied" : "invalid");
      } catch {
        if (alive) setErr("");
      } finally {
        if (alive) setChecking(false);
      }
    }, 400);
    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
  }, [id]);

  const errText = (() => {
    if (err === "empty") return t.realm.idErrEmpty;
    if (err === "invalid") return t.realm.idErrInvalid;
    if (err === "occupied") return t.realm.idOccupied;
    return "";
  })();

  const canSubmit = !!id.trim() && !checking && err === "" && !busy;

  const submit = async () => {
    const v = id.trim();
    if (!v) {
      setErr("empty");
      return;
    }
    if (!ID_RE.test(v) || v.length > ID_MAX) {
      setErr("invalid");
      return;
    }
    setBusy(true);
    try {
      const created = await createRealmSafe({
        id: v,
        // 顶层 name 为默认语言字符串（设计 §6），其他语言后续在编辑器的 i18n 中补充
        name: name.trim(),
        defaultLang: lang,
      });
      if (created) {
        onCreated(created);
        return;
      }
      showToast("error", t.realm.createFailed);
    } catch (e) {
      // 按**码**判断 ID 冲突（匹配本地化文案在非中文界面会失配）
      showToast(
        "error",
        backendErrorCode(e) === "realm_id_taken"
          ? t.realm.idOccupied
          : t.realm.error.replace("{msg}", backendErrorText(e)),
      );
    } finally {
      setBusy(false);
    }
  };

  return (
    /* 蒙层**不响应点击关闭**：创建领域需先填 ID / 名称，误触蒙层会丢失输入，
       只能通过「取消」按钮或 Esc 退出。 */
    <div className="fixed inset-0 z-[150] flex items-center justify-center bg-black/65 p-4 backdrop-blur-sm">
      <div
        className="w-full max-w-md space-y-4 rounded-xl border border-slate-800 bg-slate-950 p-5 shadow-2xl"
        onKeyDown={(e) => {
          if (e.key === "Enter" && canSubmit) void submit();
          if (e.key === "Escape") onCancel();
        }}
      >
        <h3 className="text-base font-semibold text-foreground">{t.realm.createTitle}</h3>

        <div className="space-y-1.5">
          <label className="text-sm text-slate-200">{t.realm.realmId}</label>
          <input
            autoFocus
            value={id}
            onChange={(e) => setId(e.target.value)}
            placeholder={t.realm.createIdPh}
            className="w-full rounded-lg border border-slate-700 bg-slate-900 px-3 py-2 text-sm text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500/50"
          />
          {errText ? (
            <p className="text-xs text-red-400">{errText}</p>
          ) : (
            <p className="text-xs text-muted">
              {checking ? "…" : t.realm.createIdHint}
            </p>
          )}
        </div>

        <div className="space-y-1.5">
          <label className="text-sm text-slate-200">{t.realm.name}</label>
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder={t.realm.createNamePh}
            className="w-full rounded-lg border border-slate-700 bg-slate-900 px-3 py-2 text-sm text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500/50"
          />
        </div>

        <div className="flex items-center justify-end gap-2 pt-1">
          <button
            type="button"
            onClick={onCancel}
            className="rounded-lg border border-slate-700 px-3 py-1.5 text-sm text-slate-300 hover:bg-slate-800/50"
          >
            {t.common.cancel}
          </button>
          <button
            type="button"
            disabled={!canSubmit}
            onClick={() => void submit()}
            className="rounded-lg bg-gradient-to-br from-blue-500 to-indigo-500 px-4 py-1.5 text-sm font-medium text-white shadow-md shadow-blue-600/40 hover:opacity-90 disabled:opacity-50"
          >
            {busy ? t.realm.creating : t.common.ok}
          </button>
        </div>
      </div>
    </div>
  );
}
