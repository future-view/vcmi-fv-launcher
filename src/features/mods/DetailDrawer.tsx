import { useEffect, useState } from "react";
import { useI18n } from "@/i18n";
import type { ModInfo } from "@/lib/modScanner";
import { localizedDescription, localizedName } from "@/lib/modScanner";
import { Button } from "@/shared/ui/button";
import { cn } from "@/lib/utils";
import { isTauri, getModsCacheDir, readModWikiCache } from "@/lib/tauri";
import { fullImgUrl } from "@/lib/modcache";
import { toMarkdown } from "@/lib/html";
import { HdIcon } from "@/shared/HdIcon";
import { Markdown } from "@/shared/Markdown";
import { compatRangeText } from "@/lib/modCompat";

interface DetailDrawerProps {
  mod: ModInfo | null;
  selected: Set<string>;
  onToggle: (id: string) => void;
  onClose: () => void;
  /** 只读模式（领域信息页复用）：隐藏底部「加入 / 已加入」按钮 */
  readOnly?: boolean;
  /* P6（T3 定案）：仓库「获取与更新」区块与安装/更新动作已移除，统一迁至模组管理页 */
}

export function DetailDrawer({
  mod,
  selected,
  onToggle,
  onClose,
  readOnly,
}: DetailDrawerProps) {
  const { t, lang } = useI18n();
  if (!mod) return null;
  const joined = selected.has(mod.id);
  const isTown = mod.type === "Town" && !!mod.modPath;
  const compatRange = compatRangeText(mod.compatibility);

  return (
    <>
      {/* z-index 必须高于领域覆盖层（`z-[125]`）与窗口控件（`z-[140]`）：
          领域信息页复用本抽屉时，原先的 z-70/71 会让「多语言 / 最小化 / 关闭」三个
          系统按钮压在抽屉右上角，把它自己的关闭按钮盖住导致点不到。 */}
      <div className="fixed inset-0 z-[150] bg-black/55 backdrop-blur-sm" onClick={onClose} />
      <aside className="fixed bottom-0 right-0 top-0 z-[151] flex w-[440px] max-w-[92vw] flex-col border-l border-slate-700/40 bg-slate-950/95 backdrop-blur-xl">
        <header className="flex items-center gap-3 border-b border-slate-800 p-5">
          <div className="grid h-12 w-12 flex-none place-items-center rounded-xl bg-slate-800 text-2xl">
            {iconFor(mod.type)}
          </div>
          <div className="min-w-0 flex-1">
            <div className="truncate text-lg font-semibold">{localizedName(mod, lang)}</div>
            <div className="text-xs text-slate-400">
              {mod.type} · v{mod.version ?? "—"}
            </div>
            {/* 引擎兼容区间（mod.json 的 compatibility，判定语义见 lib/modCompat.ts）。
                不兼容与否由卡片角标提示，这里只回答"它到底要求什么版本"。 */}
            {compatRange && (
              <div className="text-[11px] text-slate-500">
                {t.mods.compatLabel}: {compatRange}
              </div>
            )}
          </div>
          <Button variant="ghost" size="sm" onClick={onClose}>
            {t.common.close} ✕
          </Button>
        </header>

        <div className="flex-1 space-y-4 overflow-y-auto p-5 text-sm text-slate-300">
          {isTown && (
            <Section label={t.mods.townBackground}>
              <TownBackgroundImage mod={mod} />
            </Section>
          )}

          <Section label={t.mods.detail}>
            {/* 富文本：mod.json 描述由作者自由填写（实测含 `<h1 style=...>`）。
                按 VCMI 官方口径「HTML → Markdown 重铸 → 无 HTML 渲染」处理，
                输出为 React 元素，不走 innerHTML（见 `lib/html.ts` 与 `shared/Markdown.tsx`）。 */}
            <div className="rounded-lg border border-slate-700/50 bg-slate-900/50 p-3">
              <Markdown
                source={toMarkdown(localizedDescription(mod, lang))}
                className="text-[13px] leading-relaxed"
              />
              {!localizedDescription(mod, lang) && (
                <div className="text-[13px] text-slate-500">{t.mods.noDescription}</div>
              )}
            </div>
          </Section>

          {/* 「已加入」是模组页的选择态概念，只读抽屉里恒为 0、标签也不成立，直接不渲染 */}
          {!readOnly && (
            <Section label={`${t.mods.joined} (${mod.depends.length})`}>
              <div className="flex flex-wrap gap-2">
                {mod.depends.length === 0 && <Pill>—</Pill>}
                {mod.depends.map((d) => (
                  <Pill key={d}>{d}</Pill>
                ))}
              </div>
            </Section>
          )}

          {mod.conflicts.length > 0 && (
            <Section label={`${t.mods.partial} (${mod.conflicts.length})`}>
              <div className="flex flex-wrap gap-2">
                {mod.conflicts.map((c) => (
                  <Pill key={c} danger>
                    {c}
                  </Pill>
                ))}
              </div>
            </Section>
          )}

          {/* 无可展示子模组时整段不渲染（领域信息页的只读抽屉里 `subs` 恒为空，
              原先会留下一整块只有「－」的噪音） */}
          <Section label={t.mods.submods} hidden={mod.subs.length === 0}>
            <div className="flex flex-wrap gap-2">
              {mod.subs.map((s) => (
                <Pill key={s.id} ok={selected.has(s.id)}>
                  {s.name}
                  {selected.has(s.id) ? " ✓" : ""}
                </Pill>
              ))}
            </div>
          </Section>

          {mod.contentZipEncrypted && (
            <div className="rounded-lg border border-amber-500/40 bg-amber-500/10 p-3 text-[12px] text-amber-300">
              {/* ⚠️ 文案走 i18n（`t.mods.encryptedZipHint`），不要内联 `lang === "chinese" ? … : …`：
                  那种写法只覆盖两语言，将来加语言必然漏。
                  也⚠️ 不要断言"背景图已回退为色块"：加密只影响**从原始包提取**新素材，
                  缓存里已有的素材照常显示。 */}
              🔒 {t.mods.encryptedZipHint}
            </div>
          )}
        </div>

        {/* 只读模式（领域信息页）：不提供加入/移除，只作详情浏览（P6：仓库动作已迁至模组管理页） */}
        {!readOnly && (
          <footer className="flex items-center gap-2 border-t border-slate-800 p-4">
            <Button
              variant={joined ? "outline" : "primary"}
              className="flex-1"
              onClick={() => onToggle(mod.id)}
            >
              {joined ? `✓ ${t.mods.joined}` : `+ ${t.mods.add}`}
            </Button>
          </footer>
        )}
      </aside>
    </>
  );
}

function Section({
  label,
  children,
  hidden,
}: {
  label: string;
  children: React.ReactNode;
  /** 内容为空时整段不渲染（避免「子模组」这类区块留下只有「－」的空壳） */
  hidden?: boolean;
}) {
  if (hidden) return null;
  return (
    <div>
      <h4 className="mb-2 text-[13px] font-semibold text-slate-200">{label}</h4>
      {children}
    </div>
  );
}

/** 从 wiki 缓存读取城镇背景大图（不解析原始 mod） */
function useTownBackgroundWiki(mod: ModInfo): string | null {
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    if (!isTauri() || !mod.id) {
      setUrl(null);
      return;
    }
    let cancelled = false;
    (async () => {
      const [cacheDir, cache] = await Promise.all([getModsCacheDir(), readModWikiCache(mod.id)]);
      if (cancelled) return;
      if (!cache?.ok || !cacheDir) return;
      const data = cache.data as Record<string, unknown> | undefined;
      const factions = (data?.factions as Record<string, unknown>[]) ?? [];
      // 优先含全建筑的合成预览图 townscreen/*.webp，回退纯背景
      const rel =
        (factions[0]?.preview as string) || (factions[0]?.background as string) || "";
      if (rel) setUrl(fullImgUrl(cacheDir, rel));
    })();
    return () => {
      cancelled = true;
    };
  }, [mod.id]);
  return url;
}

/** 城镇背景图（Section 内展示） */
function TownBackgroundImage({ mod }: { mod: ModInfo }) {
  const { t } = useI18n();
  const url = useTownBackgroundWiki(mod);
  return (
    <div className="overflow-hidden rounded-lg border border-slate-700/50 bg-gradient-to-br from-slate-800/50 via-slate-900/60 to-slate-950">
      {url ? (
        <HdIcon
          src={url}
          alt=""
          className="aspect-[800/374] w-full object-cover"
          draggable={false}
          placeholderClassName="aspect-[800/374] w-full"
        />
      ) : (
        /* 无图模式：原来借用"暂无描述"文案（语义不符，用户会以为 mod 没写描述），
           改为与详情头一致的占位。 */
        <div className="relative grid aspect-[800/374] w-full place-items-center">
          <div className="absolute inset-0 bg-[radial-gradient(ellipse_at_center,rgba(99,102,241,0.14),transparent_70%)]" />
          <span className="relative text-[11px] tracking-wide text-slate-500">
            {t.wiki.preparingBg}
          </span>
        </div>
      )}
    </div>
  );
}


function Pill({
  children,
  ok,
  danger,
}: {
  children: React.ReactNode;
  ok?: boolean;
  danger?: boolean;
}) {
  return (
    <span
      className={cn(
        "rounded-full border px-2.5 py-1 text-xs",
        danger
          ? "border-red-500/50 bg-red-500/10 text-red-300"
          : ok
            ? "border-green-500/50 bg-green-500/10 text-green-300"
            : "border-slate-600/60 bg-slate-800/50 text-slate-300",
      )}
    >
      {children}
    </span>
  );
}

function iconFor(type: string): string {
  switch (type) {
    case "Town":
      return "🏰";
    case "Expansion":
      return "📦";
    case "Heroes":
      return "🦸";
    default:
      return "🧩";
  }
}
