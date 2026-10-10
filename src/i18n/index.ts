import React, { createContext, useCallback, useContext, useEffect, useMemo, useState } from "react";
import { zhCN } from "./zh-CN";
import { en } from "./en";
import { ru } from "./ru";
import { pl } from "./pl";
import { de } from "./de";
import type { LangDict } from "./types";
import { UI_LANG_FALLBACK, UI_LANG_IDS, type UiLang } from "./langs";
import { isTauri, readLauncherConfig, setUiLang } from "@/lib/tauri";
import { vcmiLangName, vcmiLangTag } from "@/lib/vcmiLangs";

export type { UiLang };

// 启动器界面词典以 VCMI 语言标识为 key（与 VCMI 游戏 / mod 多语言体系同源，
// 例如 chinese / english / tchinese / russian / polish / ukrainian / czech）。
// `Record<UiLang, LangDict>` 让「登记了却没写词典」直接编译失败；
// 而 `LangDict` 内 `settings.items` 等段是穷举映射，新语言会被编译器逐条列出待译文案。
const dicts: Record<UiLang, LangDict> = {
  chinese: zhCN,
  english: en,
  russian: ru,
  polish: pl,
  german: de,
};

// 界面语言下拉选项：标签取各语言的**本地化名**（`src/lib/vcmiLangs.ts`，语言无关），
// 因此新增一门语言 = 在 langs.ts 登记 + 补一本词典，不必再抄一遍中文名。
export const UI_LANG_OPTIONS: { value: UiLang; label: string }[] = UI_LANG_IDS.map((id) => ({
  value: id,
  label: vcmiLangName(id),
}));

// 当某 VCMI 语言没有对应界面译本时，回退到哪本界面词典。
// 这是单向兜底（仅决定用哪本界面词典），不污染游戏语言 / mod 取词 / 预设匹配；
// 其余未登记的语言统一回退 `UI_LANG_FALLBACK`，无需逐个列举。
const UI_FALLBACK: Partial<Record<string, UiLang>> = {
  tchinese: "chinese",
};

/** 取当前界面应使用的词典 key（VCMI 语言标识），缺失则回退 */
export function uiLangFor(vcmiLang: string): UiLang {
  return dicts[vcmiLang as UiLang] ? (vcmiLang as UiLang) : (UI_FALLBACK[vcmiLang] ?? UI_LANG_FALLBACK);
}

/** 是否为受支持的界面语言（有对应词典） */
function isSupportedLang(v: string): v is UiLang {
  return !!dicts[v as UiLang];
}

interface I18nCtx {
  /** 当前 VCMI 语言标识（唯一事实来源，决定游戏内语言、mod 取词、预设匹配、界面词典） */
  vcmiLang: string;
  /** 当前界面词典 key（由 vcmiLang 派生，可能经 UI_FALLBACK 回退） */
  lang: string;
  t: LangDict;
  setVcmiLang: (v: string) => void;
}

const Ctx = createContext<I18nCtx>({
  vcmiLang: "chinese",
  lang: "chinese",
  t: zhCN,
  setVcmiLang: () => {},
});

/**
 * 解析系统语言：优先匹配中文 / 俄文 / 波兰文 / 德文，否则回退英文，结果以 VCMI 标识表示。
 *
 * **仅作为「launcher.json 尚无 ui.lang」时的初始默认**（如首次运行、初始化向导阶段）。
 * 语言持久化已改由 `launcher.json` 的 `ui.lang` 承载，**不再使用 localStorage**——
 * WebView 的 localStorage 绑定系统用户数据目录、不随便携包走，导致便携包语言无法统一。
 */
function detectSystemLang(): UiLang {
  const navLangs =
    typeof navigator !== "undefined" ? (navigator.languages?.slice() ?? [navigator.language]) : [];
  for (const raw of navLangs) {
    const l = (raw || "").toLowerCase();
    if (l.startsWith("zh") || l.startsWith("cn")) return "chinese";
    if (l.startsWith("ru")) return "russian";
    if (l.startsWith("pl")) return "polish";
    if (l.startsWith("de")) return "german";
  }
  // 无匹配一律保底英文
  return "english";
}

export function I18nProvider({ children }: { children: React.ReactNode }) {
  const [vcmiLang, setVcmiLangState] = useState<string>(detectSystemLang);

  // 启动时以 launcher.json 的 `ui.lang` 为准（便携包可预置）。
  // 读取发生在 App 的启动遮罩期间，用户看不到中间态闪烁。
  useEffect(() => {
    if (!isTauri()) return;
    let alive = true;
    readLauncherConfig()
      .then((cfg) => {
        const saved = cfg?.uiLang;
        if (alive && saved && isSupportedLang(saved)) setVcmiLangState(saved);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  /**
   * 切换语言：更新界面并持久化到 launcher.json。
   *
   * 初始化向导阶段配置尚未就绪，`set_ui_lang` 会失败 → 静默忽略
   * （该阶段的语言随后由 `writeLauncherConfig({ uiLang })` 一并落盘）。
   */
  const setVcmiLang = useCallback((v: string) => {
    setVcmiLangState(v);
    void setUiLang(v).catch(() => {});
  }, []);

  useEffect(() => {
    document.documentElement.lang = vcmiLangTag(vcmiLang) ?? "en";
  }, [vcmiLang]);

  const value = useMemo<I18nCtx>(() => {
    const lang = uiLangFor(vcmiLang);
    return { vcmiLang, lang, t: dicts[lang], setVcmiLang };
  }, [vcmiLang, setVcmiLang]);

  return React.createElement(Ctx.Provider, { value }, children);
}

export function useI18n() {
  return useContext(Ctx);
}
