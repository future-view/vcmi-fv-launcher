import { useState } from "react";
import { X } from "lucide-react";
import type { RealmI18nEntry } from "@/lib/realm/types";
import { Field } from "@/features/realms/editor/Field";
import type { LangDict } from "@/i18n/types";

/** 每语言的标签上限（控制筛选栏与卡片显示空间，见设计 §6 / 索引规范） */
const MAX_TAGS = 6;
/** 单个标签长度上限（按 Unicode 字符计：中文 / emoji 各算 1） */
const MAX_TAG_LEN = 16;
/** 归一化：首尾去空白、内部连续空白折为单个空格（标签内部允许空格，如 `High Resolution`） */
const normalizeTag = (s: string) => s.trim().replace(/\s+/g, " ");
/** 输入 / 粘贴按分隔符拆分：回车、英文逗号、中文逗号 */
const splitTagInput = (s: string) => s.split(/[,\uff0c\n]/);

export interface BasicTabProps {
  t: LangDict;
  /** 当前编辑语言的 i18n 条目（多语言文本） */
  curEntry: RealmI18nEntry;
  curLang: string;
  currentId: string;
  langLabel: (v: string) => string;
  author: string;
  setAuthor: React.Dispatch<React.SetStateAction<string>>;
  setCur: (patch: RealmI18nEntry) => void;
  version: string;
  setVersion: React.Dispatch<React.SetStateAction<string>>;
}

export function BasicTab(props: BasicTabProps) {
  const { t, curEntry, curLang, currentId, langLabel, author, setAuthor, setCur, version, setVersion } = props;
  // 标签编辑：草稿输入 + 就地校验提示（重复 / 超长 / 达上限）
  const [tagDraft, setTagDraft] = useState("");
  const [tagErr, setTagErr] = useState<string | null>(null);
  const tags = curEntry.tags ?? [];
  /** 提交若干原始片段（已按分隔符拆开）：逐段归一化、去空、校验后写入当前语言 */
  const commitTags = (raws: string[]) => {
    const next = [...tags];
    let err: string | null = null;
    for (const raw of raws) {
      const v = normalizeTag(raw);
      if (!v) continue; // 空段（连续分隔符）静默跳过
      if ([...v].length > MAX_TAG_LEN) {
        if (!err) err = t.realm.tagsLong.replace("{len}", String(MAX_TAG_LEN));
        continue;
      }
      if (next.includes(v)) {
        if (!err) err = t.realm.tagsDup;
        continue;
      }
      if (next.length >= MAX_TAGS) {
        err = t.realm.tagsFull.replace("{max}", String(MAX_TAGS));
        break;
      }
      next.push(v);
    }
    if (next.length !== tags.length) setCur({ tags: next });
    setTagErr(err);
  };
  return (
    <>
      <Field label={t.realm.realmId}>
        <input
          value={currentId}
          readOnly
          disabled
          placeholder={t.realm.realmIdPh}
          className="w-full rounded-lg border border-slate-700 bg-slate-900 px-3 py-2 text-sm text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500/50 disabled:opacity-50"
        />
        <p className="mt-1 text-xs text-muted">{t.realm.realmIdHint}</p>
      </Field>
      <div className="grid grid-cols-2 gap-3">
        <Field label={t.realm.version}>
          <input
            value={version}
            onChange={(e) => setVersion(e.target.value)}
            placeholder={t.realm.versionPh}
            className="w-full rounded-lg border border-slate-700 bg-slate-900 px-3 py-2 text-sm text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500/50"
          />
        </Field>
        <Field label={t.realm.author}>
          <input
            value={author}
            onChange={(e) => setAuthor(e.target.value)}
            placeholder={t.realm.authorPh}
            className="w-full rounded-lg border border-slate-700 bg-slate-900 px-3 py-2 text-sm text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500/50"
          />
        </Field>
      </div>
      {/* 单字段 + 语言切换器（§9.6.1 / 核对点 1）：名称/描述按当前语言编辑 */}
      <p className="text-xs text-muted">
        {t.realm.editingLang.replace("{lang}", langLabel(curLang))}
      </p>
      <Field label={t.realm.name}>
        <input
          value={curEntry.name ?? ""}
          onChange={(e) => setCur({ name: e.target.value })}
          placeholder={t.realm.namePh}
          className="w-full rounded-lg border border-slate-700 bg-slate-900 px-3 py-2 text-sm text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500/50"
        />
      </Field>
      <Field label={t.realm.description}>
        <textarea
          value={curEntry.description ?? ""}
          onChange={(e) => setCur({ description: e.target.value })}
          placeholder={t.realm.descPh}
          rows={3}
          className="w-full rounded-lg border border-slate-700 bg-slate-900 px-3 py-2 text-sm text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500/50"
        />
      </Field>
      {/* 标签（§6 `tags`，逐语言录入）：用于切换领域 / 远程领域页筛选；保存时恒写（空=显式清空） */}
      <Field label={t.realm.tags}>
        <div className="flex flex-wrap items-center gap-1.5 rounded-lg border border-slate-700 bg-slate-900 px-2 py-1.5 focus-within:ring-2 focus-within:ring-blue-500/50">
          {tags.map((x) => (
            <span
              key={x}
              className="flex items-center gap-1 rounded bg-slate-700/60 px-2 py-0.5 text-xs text-slate-200"
            >
              {x}
              <button
                type="button"
                onClick={() => setCur({ tags: tags.filter((v) => v !== x) })}
                title={t.realm.tagsRemove}
                className="rounded p-0.5 text-slate-400 hover:bg-red-500/20 hover:text-red-300"
              >
                <X className="h-3 w-3" />
              </button>
            </span>
          ))}
          <input
            value={tagDraft}
            onChange={(e) => {
              setTagErr(null);
              const v = e.target.value;
              // 输入或粘贴中出现分隔符 → 提交前面的片段，尾段留在输入框继续编辑
              if (/[,\uff0c\n]/.test(v)) {
                const parts = splitTagInput(v);
                const tail = parts.pop() ?? "";
                commitTags(parts);
                setTagDraft(tail);
              } else {
                setTagDraft(v);
              }
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter") {
                e.preventDefault();
                commitTags([tagDraft]);
                setTagDraft("");
              } else if (e.key === "Backspace" && !tagDraft && tags.length) {
                // 空输入框按退格：删除最后一个标签（常见 chip 输入交互）
                setCur({ tags: tags.slice(0, -1) });
              }
            }}
            disabled={tags.length >= MAX_TAGS}
            placeholder={
              tags.length >= MAX_TAGS
                ? t.realm.tagsFull.replace("{max}", String(MAX_TAGS))
                : t.realm.tagsPh
            }
            className="min-w-[10rem] flex-1 bg-transparent px-1 py-0.5 text-sm text-foreground placeholder:text-slate-500 focus-visible:outline-none disabled:cursor-not-allowed"
          />
          <span className={tags.length >= MAX_TAGS ? "text-xs text-amber-400" : "text-xs text-muted"}>
            {tags.length}/{MAX_TAGS}
          </span>
        </div>
        <p className="mt-1 text-xs text-muted">
          {t.realm.tagsHint.replace("{max}", String(MAX_TAGS)).replace("{len}", String(MAX_TAG_LEN))}
        </p>
        {tagErr && <p className="mt-1 text-xs text-red-400">{tagErr}</p>}
      </Field>
    </>
  );
}
