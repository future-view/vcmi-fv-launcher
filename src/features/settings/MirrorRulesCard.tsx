//! 设置页「FV启动器」标签专用卡：**L2 GitHub 镜像规则表**（方案 `docs/features/network-egress.zh-CN.md` §3.2.7）。
//!
//! - 摘要态：每条规则一行（`① 域名 · 前缀/换形 · n 主机 · 启用`），点「编辑规则」**就地展开**（刻意不用弹层）；
//! - 编辑态：逐规则卡片——启用/备注/↑↓ 排序（D4 按钮）/删除二次确认/hosts 标签输入 + GitHub 全家桶预填/
//!   mode 下拉/前缀或模板输入 + **raw+release 双样例实时预览**（D6，暴露 template 边界）；
//! - 校验：前端镜像后端 `normalize_rule` 做即时提示；保存走 `onSave`（后端权威校验，错误就地展示）；
//! - 默认空、**不内置镜像**（N10：镜像域名会腐化，由用户自填）。
import { useState } from "react";
import { ArrowDown, ArrowUp, Plus, X } from "lucide-react";

import { useI18n } from "@/i18n";
import { GITHUB_MIRROR_HOSTS as GITHUB_HOSTS } from "@/lib/mirrorRules";
import type { MirrorRule } from "@/lib/tauri";
import { cn } from "@/lib/utils";
import { backendErrorText } from "@/lib/backendError";

/** 预览用真实样例（§3.2.3 例 1 / 例 2）：raw 与 release 双样例（D6） */
const SAMPLE_RAW = "https://raw.githubusercontent.com/vcmi/vcmi-mods-repository/develop/vcmi-1.7.json";
const SAMPLE_RELEASE = "https://github.com/herosix/wog/releases/download/v1.1.1/WOG.zip";
// 「预填 GitHub 全家桶」的 5 个域见 `@/lib/mirrorRules`（与首启向导的一键镜像规则同源）。
/** 占位符（与后端 `rewrite_url` 同一套） */
const TOKENS = ["{owner}", "{repo}", "{ref}", "{path}", "{url}", "{host}"] as const;
/** 规则条数上限（与后端 `set_network_mirror_rules` 同口径） */
const MAX_RULES = 10;

interface Props {
  value: MirrorRule[];
  /** 持久化并即时刷新进程级规则；失败 throw（错误由本卡就地展示） */
  onSave: (rules: MirrorRule[]) => Promise<void>;
}

/** 前端镜像后端 `normalize_rule` 的必填校验（不做美化，保存时后端仍权威归一） */
function ruleErrors(r: MirrorRule): string[] {
  const errs: string[] = [];
  if (r.hosts.length === 0) errs.push("hosts");
  if (r.mode === "prefix") {
    const p = (r.prefix ?? "").trim();
    if (!p.startsWith("http://") && !p.startsWith("https://")) errs.push("prefix");
  } else {
    const t = (r.template ?? "").trim();
    if ((!t.startsWith("http://") && !t.startsWith("https://")) || !t.includes("{")) errs.push("template");
  }
  return errs;
}

/** 全局替换（不用 replaceAll：项目 TS lib 目标较低） */
const sub = (s: string, a: string, b: string) => s.split(a).join(b);

/** 前端复刻 `infra::http::rewrite_url`（仅预览用；权威在后端） */
function previewRewrite(url: string, r: MirrorRule): string | null {
  try {
    const u = new URL(url);
    const segs = u.pathname.split("/").filter(Boolean);
    if (r.mode === "prefix") {
      const p = (r.prefix ?? "").trim();
      if (!p) return null;
      return `${p.endsWith("/") ? p : `${p}/`}${url}`;
    }
    let t = r.template ?? "";
    t = sub(sub(t, "{url}", url), "{host}", u.host);
    for (const [token, idx] of [
      ["{owner}", 0],
      ["{repo}", 1],
      ["{ref}", 2],
    ] as const) {
      if (!t.includes(token)) continue;
      const v = segs[idx];
      if (v === undefined) return null;
      t = sub(t, token, v);
    }
    if (t.includes("{path}")) {
      if (segs.length < 3) return null;
      t = sub(t, "{path}", segs.slice(3).join("/"));
    }
    return t;
  } catch {
    return null;
  }
}

/** 摘要里的镜像域名（解析失败退回原文） */
function mirrorDomain(r: MirrorRule): string {
  const v = (r.mode === "prefix" ? r.prefix ?? "" : r.template ?? "").trim();
  try {
    return new URL(v).host;
  } catch {
    return v || "—";
  }
}

export function MirrorRulesCard({ value, onSave }: Props) {
  const { t } = useI18n();
  const m = t.settings.mirrorRules;
  const [expanded, setExpanded] = useState(false);
  const [draft, setDraft] = useState<MirrorRule[]>([]);
  const [confirmDel, setConfirmDel] = useState(-1);
  const [hostInput, setHostInput] = useState<Record<number, string>>({});
  const [saving, setSaving] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const openEdit = () => {
    setDraft(value.map((r) => ({ ...r, hosts: [...r.hosts] })));
    setConfirmDel(-1);
    setErr(null);
    setExpanded(true);
  };
  const patch = (i: number, p: Partial<MirrorRule>) =>
    setDraft((d) => d.map((r, k) => (k === i ? { ...r, ...p } : r)));
  const move = (i: number, dir: -1 | 1) =>
    setDraft((d) => {
      const j = i + dir;
      if (j < 0 || j >= d.length) return d;
      const out = [...d];
      [out[i], out[j]] = [out[j], out[i]];
      return out;
    });
  const addHost = (i: number) => {
    const v = (hostInput[i] ?? "").trim().toLowerCase().replace(/,+$/, "");
    if (!v) return;
    const r = draft[i];
    if (!r.hosts.includes(v)) patch(i, { hosts: [...r.hosts, v] });
    setHostInput((h) => ({ ...h, [i]: "" }));
  };
  const addRule = () =>
    setDraft((d) =>
      d.length >= MAX_RULES
        ? d
        : [...d, { enabled: true, note: "", hosts: [...GITHUB_HOSTS], mode: "prefix", prefix: "" }],
    );

  const invalid = draft.some((r) => ruleErrors(r).length > 0);
  const save = async () => {
    if (invalid || saving) return;
    setSaving(true);
    setErr(null);
    try {
      await onSave(draft);
      setExpanded(false);
    } catch (e) {
      setErr(backendErrorText(e));
    } finally {
      setSaving(false);
    }
  };

  const dot = (i: number) => ["①", "②", "③", "④", "⑤", "⑥", "⑦", "⑧", "⑨", "⑩"][i] ?? `${i + 1}.`;

  /* ---------------- 摘要态 ---------------- */
  if (!expanded) {
    return (
      <div className="glass rounded-xl p-4">
        <div className="flex items-center justify-between gap-3">
          <div className="text-sm font-medium">{m.title}</div>
          <button
            type="button"
            onClick={openEdit}
            className="inline-flex items-center gap-1 rounded-md border border-border/60 px-2 py-1 text-xs transition-colors hover:bg-slate-800/60"
          >
            {m.edit}
          </button>
        </div>
        {value.length === 0 ? (
          <div className="mt-2 text-xs text-muted-foreground">{m.empty}</div>
        ) : (
          <ul className="mt-2 space-y-1">
            {value.map((r, i) => (
              <li key={i} className="text-xs text-slate-300">
                <span>{dot(i)} </span>
                <span className="font-mono">{mirrorDomain(r)}</span>
                <span> · {r.mode === "prefix" ? m.modePrefix : m.modeTemplate}</span>
                <span> · {r.hosts.length} {m.unitHosts}</span>
                <span className={r.enabled ? "" : "text-slate-500"}>
                  {" · "}{r.enabled ? m.badgeEnabled : m.badgeDisabled}
                </span>
                {r.note ? <span className="text-slate-500"> · {r.note}</span> : null}
              </li>
            ))}
          </ul>
        )}
        <div className="mt-2 text-[11px] leading-4 text-slate-500">{m.guide}</div>
      </div>
    );
  }

  /* ---------------- 编辑态（就地展开） ---------------- */
  return (
    <div className="glass rounded-xl p-4">
      <div className="flex items-center justify-between gap-3">
        <div className="text-sm font-medium">{m.title}</div>
        <div className="text-xs text-slate-500">{draft.length} / {MAX_RULES}</div>
      </div>
      <div className="mt-1 text-[11px] leading-4 text-slate-500">{m.guide}</div>

      <div className="mt-3 space-y-3">
        {draft.map((r, i) => {
          const errs = ruleErrors(r);
          const raw = previewRewrite(SAMPLE_RAW, r);
          const rel = previewRewrite(SAMPLE_RELEASE, r);
          return (
            <div key={i} className="rounded-lg border border-slate-700/70 bg-slate-900/40 p-3">
              <div className="flex items-center gap-2">
                <label className="flex items-center gap-1.5 text-xs">
                  <input
                    type="checkbox"
                    checked={r.enabled}
                    onChange={(e) => patch(i, { enabled: e.target.checked })}
                  />
                  {m.enabled}
                </label>
                <input
                  value={r.note ?? ""}
                  onChange={(e) => patch(i, { note: e.target.value })}
                  placeholder={m.noteLabel}
                  className="h-7 w-36 rounded-md border border-slate-700 bg-slate-900/60 px-2 text-xs outline-none focus:border-slate-500"
                />
                <div className="ml-auto flex items-center gap-1">
                  <button type="button" title={m.up} onClick={() => move(i, -1)} className="rounded p-1 hover:bg-slate-800 disabled:opacity-30" disabled={i === 0}>
                    <ArrowUp className="h-3.5 w-3.5" />
                  </button>
                  <button type="button" title={m.down} onClick={() => move(i, 1)} className="rounded p-1 hover:bg-slate-800 disabled:opacity-30" disabled={i === draft.length - 1}>
                    <ArrowDown className="h-3.5 w-3.5" />
                  </button>
                  <button
                    type="button"
                    onClick={() =>
                      confirmDel === i
                        ? (setDraft((d) => d.filter((_, k) => k !== i)), setConfirmDel(-1))
                        : setConfirmDel(i)
                    }
                    className={cn(
                      "rounded-md border px-2 py-1 text-xs",
                      confirmDel === i
                        ? "border-red-500/60 bg-red-600/20 text-red-300"
                        : "border-slate-700 text-slate-400 hover:bg-slate-800",
                    )}
                  >
                    {confirmDel === i ? m.confirmDel : m.del}
                  </button>
                </div>
              </div>

              {/* 生效主机（chips + 回车添加 + GitHub 预填） */}
              <div className="mt-2 text-xs text-slate-400">{m.hostsLabel}</div>
              <div className="mt-1 flex flex-wrap items-center gap-1.5">
                {r.hosts.map((h) => (
                  <span key={h} className="inline-flex items-center gap-1 rounded-full border border-slate-600 bg-slate-800/70 px-2 py-0.5 font-mono text-[11px]">
                    {h}
                    <button type="button" onClick={() => patch(i, { hosts: r.hosts.filter((x) => x !== h) })}>
                      <X className="h-3 w-3 text-slate-400 hover:text-slate-200" />
                    </button>
                  </span>
                ))}
                <input
                  value={hostInput[i] ?? ""}
                  onChange={(e) => setHostInput((s) => ({ ...s, [i]: e.target.value }))}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" || e.key === ",") {
                      e.preventDefault();
                      addHost(i);
                    }
                  }}
                  placeholder={m.hostInput}
                  className="h-6 w-44 rounded border border-slate-700 bg-slate-900/60 px-2 text-[11px] outline-none focus:border-slate-500"
                />
                <button
                  type="button"
                  onClick={() => patch(i, { hosts: Array.from(new Set([...r.hosts, ...GITHUB_HOSTS])) })}
                  className="rounded-md border border-slate-700 px-2 py-0.5 text-[11px] text-slate-300 hover:bg-slate-800"
                >
                  {m.presetGithub}
                </button>
              </div>
              {errs.includes("hosts") && <div className="mt-1 text-[11px] text-red-400">{m.errHosts}</div>}

              {/* 模式 + 目标 */}
              <div className="mt-2 flex items-center gap-2">
                <span className="text-xs text-slate-400">{m.modeLabel}</span>
                <select
                  value={r.mode}
                  onChange={(e) => patch(i, { mode: e.target.value as MirrorRule["mode"] })}
                  className="h-7 rounded-md border border-slate-700 bg-slate-900/60 px-1 text-xs outline-none"
                >
                  <option value="prefix">{m.modePrefix}</option>
                  <option value="template">{m.modeTemplate}</option>
                </select>
              </div>
              {r.mode === "prefix" ? (
                <>
                  <div className="mt-1.5 text-xs text-slate-400">{m.prefixLabel}</div>
                  <input
                    value={r.prefix ?? ""}
                    onChange={(e) => patch(i, { prefix: e.target.value })}
                    placeholder="https://v4.gh-proxy.org/"
                    className="mt-1 h-7 w-full rounded-md border border-slate-700 bg-slate-900/60 px-2 font-mono text-xs outline-none focus:border-slate-500"
                  />
                  {errs.includes("prefix") && <div className="mt-1 text-[11px] text-red-400">{m.errPrefix}</div>}
                </>
              ) : (
                <>
                  <div className="mt-1.5 text-xs text-slate-400">{m.templateLabel}</div>
                  <input
                    value={r.template ?? ""}
                    onChange={(e) => patch(i, { template: e.target.value })}
                    placeholder="https://cdn.jsdelivr.net/gh/{owner}/{repo}@{ref}/{path}"
                    className="mt-1 h-7 w-full rounded-md border border-slate-700 bg-slate-900/60 px-2 font-mono text-xs outline-none focus:border-slate-500"
                  />
                  <div className="mt-1 flex flex-wrap items-center gap-1 text-[11px] text-slate-500">
                    <span>{m.templateHint}</span>
                    {TOKENS.map((tk) => (
                      <button
                        key={tk}
                        type="button"
                        onClick={() => patch(i, { template: `${r.template ?? ""}${tk}` })}
                        className="rounded border border-slate-700 px-1 font-mono hover:bg-slate-800"
                      >
                        {tk}
                      </button>
                    ))}
                  </div>
                  {errs.includes("template") && <div className="mt-1 text-[11px] text-red-400">{m.errTemplate}</div>}
                </>
              )}

              {/* 双样例实时预览（D6） */}
              <div className="mt-2 space-y-1 rounded-md border border-slate-800 bg-slate-950/50 p-2 text-[11px]">
                <div className="text-slate-500">{m.previewTitle}</div>
                <div>
                  <span className="text-slate-400">{m.previewRaw}: </span>
                  {raw ? (
                    <span className="font-mono text-emerald-400/90">{raw} <span className="text-emerald-500">{m.okParse}</span></span>
                  ) : (
                    <span className="text-red-400">{m.failParse}</span>
                  )}
                </div>
                <div>
                  <span className="text-slate-400">{m.previewRelease}: </span>
                  {rel ? (
                    <span className="font-mono text-slate-300">{rel}</span>
                  ) : (
                    <span className="text-red-400">{m.failParse}</span>
                  )}
                </div>
              </div>
            </div>
          );
        })}
      </div>

      <button
        type="button"
        onClick={addRule}
        disabled={draft.length >= MAX_RULES}
        className="mt-3 inline-flex items-center gap-1 rounded-md border border-dashed border-slate-600 px-2 py-1 text-xs text-slate-300 hover:bg-slate-800 disabled:opacity-40"
      >
        <Plus className="h-3.5 w-3.5" /> {m.addRule}
        {draft.length >= MAX_RULES ? `（${m.limit}）` : ""}
      </button>

      <div className="mt-2 space-y-1 text-[11px] leading-4 text-slate-500">
        <div>{m.warnTls}</div>
        <div>{m.budgetHint}</div>
      </div>
      {err && <div className="mt-2 rounded-md border border-red-500/50 bg-red-950/40 px-2 py-1 text-xs text-red-300">{err}</div>}

      <div className="mt-3 flex items-center justify-end gap-2">
        <button
          type="button"
          onClick={() => setExpanded(false)}
          className="rounded-md border border-slate-700 px-3 py-1.5 text-xs text-slate-300 hover:bg-slate-800"
        >
          {m.cancel}
        </button>
        <button
          type="button"
          onClick={() => void save()}
          disabled={invalid || saving}
          className="rounded-md bg-sky-700 px-3 py-1.5 text-xs font-medium text-white hover:bg-sky-600 disabled:opacity-40"
        >
          {saving ? "…" : m.save}
        </button>
      </div>
    </div>
  );
}
