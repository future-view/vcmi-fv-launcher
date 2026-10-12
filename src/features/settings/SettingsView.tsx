import { useEffect, useMemo, useRef, useState } from "react";
import { Search, Info, RotateCcw, RefreshCw, FolderOpen, ChevronDown } from "lucide-react";
import { cn } from "@/lib/utils";
import { ScopeBadge } from "./ScopeBadge";
import { AiRuntimeCard } from "./AiRuntimeCard";
import { HdEditionCard } from "./HdEditionCard";
import { MirrorRulesCard } from "./MirrorRulesCard";
import { useI18n } from "@/i18n";
import {
  SETTING_TAGS,
  ALL_ITEMS,
  OFFICIAL_DEFAULTS,
  type SettingItem,
  type SettingLevel,
} from "@/lib/settingsSchema";
import type { SettingTagId } from "@/generated/settingKeys";
import {
  loadVcmiSettings,
  writeVcmiSetting,
  resetVcmiSettings,
  getRuntimeInfo,
  listResolutions,
  listAiModels,
  openInExplorer,
  lastUpdateInfo,
  saveFvLauncherItem,
  saveNetworkMirrorRules,
  type RuntimeInfo,
  type Resolution,
  type AiModelInfo,
  type MirrorRule,
  type UpdateInfo,
} from "@/lib/tauri";
import { SettingsSidebar } from "./SettingsSidebar";
import { SettingCard } from "./SettingCard";
import { RuntimeInfoDialog } from "./RuntimeInfoDialog";
import { useSettingText, type SettingOption } from "./text";
import { ConfirmDialog } from "@/shared/ConfirmDialog";
import { Toast } from "@/shared/Toast";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/shared/ui/select";
import { saveSettingsPrefs, readLauncherConfig } from "@/lib/tauri";
import { backendErrorText } from "@/lib/backendError";

type ToastType = "success" | "error";
interface ToastState {
  type: ToastType;
  msg: string;
}

const LEVEL_RANK: Record<SettingLevel, number> = { common: 0, advanced: 1, all: 2 };

/**
 * 设置页（§3.5.1 通用组件）。启动器设置页与领域设置页**共用同一个组件**，
 * 差异全部由 props 表达，避免两套实现漂移：
 *
 * - `configDir`：读写哪个 `settings.json`。不传 = 全局用户配置；传入 = 该领域解析出的配置目录。
 * - `hideTitle`：隐藏工具条里的大标题。领域设置页用——它的「返回 + 标题」在**覆盖层左上角页头**
 *   （`RealmShell` 的 `PageHeaderBar`，与窗口控件同排，不占额外行），工具条再显示一次就重复了。
 *   启动器设置页不传：它上方是 `TopBar`，没有可挂页头的位置，标题就留在工具条里。
 * - `scope` / `scopePath`：作用域徽标（全局共享 / 领域独立）+ 其对应的配置目录，渲染在
 *   **右侧按钮群组里（「运行信息」左侧）**——放在标题旁会把「返回 | 标题 | 徽标 | 搜索框」挤成一行。
 *   两侧都传，含义由调用方给出。
 *
 * 离开设置页的方式由调用方决定，组件本身不管：领域侧是页头的「返回」，
 * 启动器侧是「再点 ⚙」或「点任一页签」。
 */
/** 最大公约数（把宽高比化简为最简分数标签） */
function gcd(a: number, b: number): number {
  return b === 0 ? a : gcd(b, a % b);
}

/** 把分辨率映射成常见屏幕比标签（带容差），否则退回最简分数（如 64:27） */
function aspectRatioLabel(w: number, h: number): string {
  const ratio = w / h;
  const known: Array<[string, number]> = [
    ["16:9", 16 / 9],
    ["16:10", 1.6],
    ["21:9", 21 / 9],
    ["32:9", 32 / 9],
    ["4:3", 4 / 3],
    ["5:4", 1.25],
    ["3:2", 1.5],
    ["5:3", 5 / 3],
    ["1:1", 1],
  ];
  let best = known[0];
  let bestDiff = Infinity;
  for (const [label, r] of known) {
    const d = Math.abs(ratio - r);
    if (d < bestDiff) {
      bestDiff = d;
      best = [label, r];
    }
  }
  if (bestDiff <= 0.02) return best[0];
  const g = gcd(w, h);
  return `${w / g}:${h / g}`;
}

/**
 * 把显示器枚举出的分辨率整理成分组下拉候选（方案 A+B）：
 * - 按屏幕比分组（16:9 / 16:10 / 4:3 …），组间排序：当前显示器所在比例组优先，其余按常用度兜底；
 * - 组内：当前显示器原生分辨率（`primary`）置顶，其余按像素总量升序；
 * - 标记 `primary` 供前端在原生项上挂「当前」徽标。
 */
function buildResolutionOptions(list: Resolution[]): SettingOption[] {
  const primary = list.find((r) => r.primary);
  const currentGroup = primary ? aspectRatioLabel(primary.width, primary.height) : null;
  const groups = new Map<string, Resolution[]>();
  for (const r of list) {
    const label = aspectRatioLabel(r.width, r.height);
    const arr = groups.get(label);
    if (arr) arr.push(r);
    else groups.set(label, [r]);
  }
  const preferred = ["16:9", "16:10", "21:9", "32:9", "4:3", "5:4", "3:2", "5:3", "1:1"];
  const rank = (label: string) =>
    label === currentGroup ? -1 : preferred.includes(label) ? preferred.indexOf(label) : 100;
  const ordered = [...groups.keys()].sort((a, b) => rank(a) - rank(b));
  const out: SettingOption[] = [];
  for (const label of ordered) {
    const items = (groups.get(label) ?? []).slice().sort((a, b) => {
      if (!!a.primary !== !!b.primary) return a.primary ? -1 : 1;
      return a.width * a.height - b.width * b.height;
    });
    for (const r of items) {
      out.push({
        v: `${r.width}x${r.height}`,
        text: `${r.width}×${r.height}`,
        group: label,
        primary: !!r.primary,
      });
    }
  }
  return out;
}

export function SettingsView({
  configDir,
  hideTitle,
  scope,
  scopePath,
  onOpenUpdate,
}: {
  configDir?: string;
  hideTitle?: boolean;
  scope?: "global" | "realm";
  scopePath?: string;
  /** 有传入才渲染「检查更新」按钮（更新是启动器级能力：领域设置页不传则不渲染） */
  onOpenUpdate?: () => void;
}) {
  const { t } = useI18n();
  const text = useSettingText();
  const [values, setValues] = useState<Record<string, unknown>>({});
  const [loading, setLoading] = useState(true);
  const [query, setQuery] = useState("");
  const [level, setLevel] = useState<SettingLevel>("common");
  const [showDesc, setShowDesc] = useState(false);
  const [activeTag, setActiveTag] = useState<SettingTagId>(SETTING_TAGS[0].id);
  const [runtime, setRuntime] = useState<RuntimeInfo | null>(null);
  const [runtimeOpen, setRuntimeOpen] = useState(false);
  const [resetOpen, setResetOpen] = useState(false);
  const [toast, setToast] = useState<ToastState | null>(null);
  const [resList, setResList] = useState<Resolution[]>([]);
  // AI 模型候选（组件目录扫描；空 = 组件未安装 / 无模型 → 下拉只剩「(不使用)」）
  const [aiModels, setAiModels] = useState<AiModelInfo[]>([]);
  // 右侧滚动容器 ref（侧栏点击后平滑滚动定位到对应标签分组）
  const scrollRef = useRef<HTMLElement | null>(null);
  // 「更多」菜单：收纳运行信息 / 检查更新 / 打开配置目录 / 重置（见设计 §2.2）
  const [menuOpen, setMenuOpen] = useState(false);
  const menuRef = useRef<HTMLDivElement | null>(null);
  // 最近一次检查更新结果（App 启动静默检查写入模块级缓存）：菜单内版本徽标 + 「更多」按钮红点（L2/L3）
  const [updateCached, setUpdateCached] = useState<UpdateInfo | null>(() => lastUpdateInfo());
  // 当前激活标签的 ref（供 scroll-spy 在回调中读取最新值，避免闭包过期）
  const activeTagRef = useRef(activeTag);
  useEffect(() => {
    activeTagRef.current = activeTag;
  }, [activeTag]);

  // 「更多」菜单：外点关闭（与 PresetBar「更多」菜单同一交互模式）；打开时刷新更新缓存
  useEffect(() => {
    if (!menuOpen) return;
    setUpdateCached(lastUpdateInfo());
    const handler = (e: MouseEvent) => {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) setMenuOpen(false);
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [menuOpen]);

  const showToast = (type: ToastType, msg: string) => {
    setToast({ type, msg });
  };

  useEffect(() => {
    let alive = true;
    (async () => {
      try {
        const loaded = await loadVcmiSettings(OFFICIAL_DEFAULTS, configDir);
        // `fv.*` 项（`FV启动器` 标签）来自 **launcher.json**，与 VCMI 设置不同源，
        // 故在 VCMI 值之上单独覆盖读回；未配置时保留 schema 里的默认值。
        const lc = await readLauncherConfig();
        if (lc) {
          loaded["fv.aiHdEnhance"] = lc.aiHdEnhance === true;
          loaded["fv.hdEnhance"] = lc.hdEnhance === true;
          loaded["fv.townPreview"] = lc.townPreview === true;
          if (lc.hdEditionUse) loaded["fv.hdEditionUse"] = lc.hdEditionUse;
          if (lc.hdEditionScale) loaded["fv.hdEditionScale"] = lc.hdEditionScale;
          if (lc.uiLang) loaded["fv.uiLang"] = lc.uiLang;
          if (lc.logLevel) loaded["fv.logLevel"] = lc.logLevel;
          // 模型未配置时置空串（= schema 默认「(不使用)」→ AI 不生效）
          loaded["fv.aiModel"] = lc.aiModel ?? "";
          // 网络代理：空串 = 直连（schema 默认）；有值则回填
          loaded["fv.networkProxy"] = lc.networkProxy ?? "";
          // GitHub 镜像规则表（L2）：缺省空数组 = 不用（默认不内置）
          loaded["fv.networkMirrorRules"] = lc.mirrorRules ?? [];
        }
        if (alive) setValues(loaded);
        // 分辨率候选（显示器枚举，按当前值筛选宽/高组合）
        try {
          const list = await listResolutions();
          if (alive && list.length) setResList(list);
        } catch {
          /* 离线环境下忽略，使用 schema 内置候选 */
        }
        // AI 模型候选（组件目录扫描）；空列表也要设 —— UI 据此只显示「(不使用)」
        try {
          const models = await listAiModels();
          if (alive) setAiModels(models);
        } catch {
          /* 组件未安装 / 非 Tauri：保持空列表 */
        }
      } catch (e) {
        showToast("error", t.settings.loadError);
        console.error(e);
      } finally {
        if (alive) setLoading(false);
      }
    })();
    return () => {
      alive = false;
    };
  }, []);

  const searching = query.trim().length > 0;

  // 搜索命中项（忽略级别）：只按当前界面语言那一本词典匹配（标签 + 详细描述 + 设置键）
  const matched = useMemo(() => {
    const q = query.trim().toLowerCase();
    return searching ? ALL_ITEMS.filter((it) => text.searchBlobOf(it).includes(q)) : [];
  }, [query, searching, text]);

  // 搜索模式下各标签命中数（Sidebar 徽标）
  const matchCounts = useMemo(() => {
    const m: Record<string, number> = {};
    if (searching) {
      for (const it of matched) {
        const tagId = SETTING_TAGS.find((tg) => tg.items.some((x) => x.id === it.id))?.id;
        if (tagId) m[tagId] = (m[tagId] ?? 0) + 1;
      }
    }
    return m;
  }, [matched, searching]);

  // 当前级别下「有可见项」的标签（设计文档：无可见项则不显示该标签栏）
  const visibleTags = useMemo(() => {
    return SETTING_TAGS.filter((tg) =>
      tg.items.some((it) => LEVEL_RANK[it.level] <= LEVEL_RANK[level])
    );
  }, [level]);

  // 搜索模式下，仅展示「有命中项」的标签（与官方 sidebarTags 一致，隐藏空标签）；否则用可见标签
  const sidebarTags = searching
    ? SETTING_TAGS.filter((tg) => (matchCounts[tg.id] ?? 0) > 0)
    : visibleTags;

  // 启动时从 launcher.json 读取已保存的视图偏好（可见级别 / 详细描述）
  useEffect(() => {
    (async () => {
      const cfg = await readLauncherConfig();
      if (cfg) {
        if (cfg.settingsLevel) setLevel(cfg.settingsLevel);
        if (cfg.settingsShowDesc !== undefined) setShowDesc(cfg.settingsShowDesc);
      }
    })();
  }, []);

  // 视图偏好变化时持久化到 launcher.json（下次打开可用）
  useEffect(() => {
    saveSettingsPrefs(level, showDesc);
  }, [level, showDesc]);

  // 若当前激活标签在可见标签中已不存在，自动跳到第一个可见标签
  useEffect(() => {
    if (!searching && !visibleTags.some((tg) => tg.id === activeTag)) {
      setActiveTag(visibleTags[0]?.id ?? SETTING_TAGS[0].id);
    }
  }, [visibleTags, activeTag, searching]);

  const handleSave = async (key: string, value: unknown) => {
    setValues((prev) => ({ ...prev, [key]: value }));
    try {
      // `fv.*` 分流到 launcher.json 通道，其余走 VCMI 设置（两者不同源，不可混写）
      if (key.startsWith("fv.")) {
        await saveFvLauncherItem(key, value);
      } else {
        await writeVcmiSetting(key, value, OFFICIAL_DEFAULTS, configDir);
      }
      showToast("success", t.settings.savedToast);
    } catch (e) {
      const msg = backendErrorText(e);
      showToast("error", t.settings.saveError.replace("{msg}", msg));
    }
  };

  // 镜像规则表保存（L2 专用卡）：失败 **上抛** 由卡片就地展示；成功回写 values + 绿条
  const saveMirrorRules = async (rules: MirrorRule[]) => {
    await saveNetworkMirrorRules(rules);
    setValues((prev) => ({ ...prev, ["fv.networkMirrorRules"]: rules }));
    showToast("success", t.settings.savedToast);
  };

  // 运行信息按当前配置目录取数（领域隔离后领域目录有自己的运行态，见设计 §六）
  const openRuntime = async () => {
    const info = await getRuntimeInfo(configDir);
    setRuntime(info);
    setRuntimeOpen(true);
  };

  const handleReset = async () => {
    setResetOpen(false);
    try {
      await resetVcmiSettings(OFFICIAL_DEFAULTS, configDir);
      const reloaded = await loadVcmiSettings(OFFICIAL_DEFAULTS, configDir);
      setValues(reloaded);
      showToast("success", t.settings.savedToast);
    } catch (e) {
      const msg = backendErrorText(e);
      showToast("error", t.settings.saveError.replace("{msg}", msg));
    }
  };

  // 侧栏点击：仅做滚动定位（不切换/锁定视图），并高亮当前锚点
  const handleSelect = (id: SettingTagId) => {
    setActiveTag(id);
    requestAnimationFrame(() => {
      document
        .getElementById(`tag-${id}`)
        ?.scrollIntoView({ behavior: "smooth", block: "start" });
    });
  };

  // 滚动联动：右侧滚动经过某分组时，左侧对应标签高亮（scroll-spy）。
  // 用 IntersectionObserver 监听当前可见分组，选取视口内最靠上的分组作为激活项。
  useEffect(() => {
    const root = scrollRef.current;
    if (!root || loading) return;
    const groups = (searching ? sidebarTags : visibleTags).map((tg) => `tag-${tg.id}`);
    if (groups.length === 0) return;
    const observer = new IntersectionObserver(
      (entries) => {
        let best: string | null = null;
        let bestTop = Infinity;
        for (const e of entries) {
          if (e.isIntersecting) {
            const top = e.boundingClientRect.top;
            if (top < bestTop) {
              bestTop = top;
              best = e.target.id;
            }
          }
        }
        if (best) {
          const id = best.slice(4); // 去掉 "tag-" 前缀
          if (id !== activeTagRef.current) setActiveTag(id as SettingTagId);
        }
      },
      { root, rootMargin: "0px 0px -70% 0px", threshold: 0 }
    );
    groups.forEach((id) => {
      const el = document.getElementById(id);
      if (el) observer.observe(el);
    });
    return () => observer.disconnect();
  }, [searching, sidebarTags, visibleTags, loading]);

  // 跨键控件（全屏/城镇生物互斥）一次写回多个键，逐键差量写回
  const handleSaveMulti = async (entries: Record<string, unknown>) => {
    setValues((prev) => ({ ...prev, ...entries }));
    try {
      for (const [k, v] of Object.entries(entries)) {
        await writeVcmiSetting(k, v, OFFICIAL_DEFAULTS, configDir);
      }
    } catch (e) {
      const msg = backendErrorText(e);
      showToast("error", t.settings.saveError.replace("{msg}", msg));
    }
  };

  // 设置项 id → 所属标签 id 的映射（搜索分组时用）
  const itemTagMap = useMemo(() => {
    const m: Record<string, string> = {};
    for (const tg of SETTING_TAGS) for (const it of tg.items) m[it.id] = tg.id;
    return m;
  }, []);

  // 渲染某项的候选覆盖（分辨率动态来自显示器枚举；缩放上限按主显示器动态计算）
  const candidateOverrideFor = (k: string): SettingOption[] | undefined => {
    if (k === "fv.aiModel") {
      // 首项「(不使用)」= 清除配置（AI 不生效）；文件名后的后缀标出"推荐 / 未验证"，
      // 让用户一眼看出清单内外的区别
      return [
        { v: "", text: text.word("disabledOption") },
        ...aiModels.map((m) => ({
          v: m.file,
          text: m.recommended
            ? text.word("recommendedName", { name: m.file })
            : m.known
              ? m.file
              : text.word("unverifiedName", { name: m.file }),
        })),
      ];
    }
    if (k === "video.resolution.width" && resList.length) {
      // 按屏幕比分组合并「当前显示器优先」（组间 + 组内排序、原生项挂 primary 标记）
      return buildResolutionOptions(resList);
    }
    if (k === "video.resolution.scaling") {
      // 上限 = round(100 * W*H / (800*600))，取显示器枚举中的最大分辨率
      let cap = 200;
      if (resList.length) {
        const max = resList.reduce((a, b) => (a.width * a.height >= b.width * b.height ? a : b));
        cap = Math.round((100 * max.width * max.height) / (800 * 600));
      }
      // 预设档位：125/150/175/200… 直至上限（高分屏适用，更多 >100% 档）
      const presets = [125, 150, 175, 200, 225, 250, 275, 300];
      const vals = presets.filter((v) => v <= cap);
      if (cap > 0 && !vals.includes(cap) && cap >= 125) vals.push(cap);
      return vals.map((v) => ({ v: String(v), text: `${v}%` }));
    }
    return undefined;
  };

  /** 计算某分组(标签)内被 altKey 覆盖、不应单独渲染的键集合 */
  const altCoveredIn = (items: SettingItem[]): Set<string> => {
    const set = new Set<string>();
    for (const it of items) if (it.altKey) set.add(it.altKey);
    return set;
  };

  // 依赖禁用：根据 item.disabledWhen 与当前 values 计算是否禁用
  const disabledFor = (it: SettingItem): boolean => {
    const dw = it.disabledWhen;
    if (!dw) return false;
    const dep = values[dw.key];
    if ("eq" in dw && dw.eq !== undefined) return dep === dw.eq;
    if ("neq" in dw && dw.neq !== undefined) return dep !== dw.neq;
    return false;
  };

  // —— 顶部区派生 ——
  // 作用域文案（「更多」菜单的重置确认框、运行信息弹窗共用）
  const scopeLabel = scope === "realm" ? t.settings.scopeRealm : t.settings.scopeGlobal;
  // 当前配置目录（作用域面板 /「打开配置目录」共用）：优先领域隔离目录，否则全局用户配置目录
  const scopeDir = configDir ?? scopePath;
  // 可选更新可用（L2 按钮红点 / L3 菜单徽标）：仅非强制
  const updateAvailable = !!updateCached?.hasUpdate && !updateCached.forced;

  return (
    <div className="flex h-full flex-col">
      {/* 顶部区（横跨左右分栏）：标题（可隐藏）+ 搜索 ｜ 视图控制 ｜ 作用域状态 + 「更多」菜单。
          v2 重构（设计 §二）：常驻控件只放"改变本页显示"者（同时作用于左栏与右栏）；
          维护类动作与危险操作收进菜单；只读状态不做按钮外观。 */}
      <div className="flex items-center gap-2 border-b border-slate-800 px-4 py-2.5">
        {!hideTitle && (
          <h2 className="mr-1 flex-none text-base font-semibold text-foreground">{t.settings.title}</h2>
        )}

        <div className="relative min-w-0 max-w-xs flex-1">
          <Search className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-slate-500" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder={t.settings.search}
            className="w-full rounded-lg border border-slate-700 bg-slate-900 py-1.5 pl-9 pr-3 text-sm text-foreground placeholder:text-slate-500 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500/50"
          />
        </div>

        <div className="ml-auto flex flex-none items-center gap-3">
          {/* 视图控制：显示级别 + 详细描述（同时作用于左栏标签与右栏卡片，属整页级控件） */}
          <div className="flex items-center gap-3">
            <div className="flex items-center gap-1.5">
              <span className="text-xs text-muted">{t.settings.levelLabel}</span>
              <Select value={level} onValueChange={(v) => setLevel(v as SettingLevel)}>
                <SelectTrigger className="h-9 w-28 bg-slate-900/60 text-sm text-foreground hover:bg-slate-800/60">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent className="z-[200] border-slate-700 bg-slate-900 text-slate-200">
                  <SelectItem value="common">{t.settings.levelCommon}</SelectItem>
                  <SelectItem value="advanced">{t.settings.levelAdvanced}</SelectItem>
                  <SelectItem value="all">{t.settings.levelAll}</SelectItem>
                </SelectContent>
              </Select>
            </div>

            <label className="flex cursor-pointer items-center gap-1.5 text-sm text-muted">
              <input
                type="checkbox"
                checked={showDesc}
                onChange={(e) => setShowDesc(e.target.checked)}
                className="h-3.5 w-3.5 accent-blue-500"
              />
              {t.settings.showDesc}
            </label>
          </div>

          {/* 分隔：视图控制 ｜ 页面配置 */}
          <span aria-hidden className="h-4 w-px bg-slate-700/70" />

          {/* 页面配置：作用域状态（只读小字 + 说明面板）+「更多」菜单 */}
          <div className="flex items-center gap-3">
            {scope && (
              <ScopeBadge
                variant={scope}
                hint={scope === "global" ? t.settings.scopeGlobalHint : t.settings.scopeRealmHint}
                path={scopePath}
              />
            )}

            <div className="relative" ref={menuRef}>
              <button
                type="button"
                onClick={() => setMenuOpen((o) => !o)}
                className="relative flex items-center gap-1.5 rounded-lg border border-slate-700 px-2.5 py-1.5 text-sm text-slate-300 hover:bg-slate-800/50"
              >
                {t.settings.more}
                <ChevronDown className="h-3.5 w-3.5" />
                {/* L2：有可选更新时按钮右上角红点（纯装饰，不挤压布局） */}
                {updateAvailable && (
                  <span className="absolute -right-0.5 -top-0.5 h-1.5 w-1.5 rounded-full bg-blue-500" />
                )}
              </button>

              {menuOpen && (
                <div className="absolute right-0 z-[120] mt-1 w-64 overflow-hidden rounded-lg border border-slate-700 bg-slate-900 shadow-xl">
                  {/* 运行信息（按当前配置目录取数） */}
                  <MenuItem
                    icon={<Info className="h-4 w-4" />}
                    label={t.settings.runtimeInfo}
                    desc={t.settings.moreRuntimeDesc}
                    onClick={() => {
                      setMenuOpen(false);
                      void openRuntime();
                    }}
                  />

                  {/* 检查更新（仅启动器设置页渲染：更新是启动器级能力，领域页不该管） */}
                  {onOpenUpdate && (
                    <MenuItem
                      icon={<RefreshCw className="h-4 w-4" />}
                      label={t.update.checkNow}
                      desc={t.settings.moreUpdateCurrent.replace(
                        "{version}",
                        updateCached?.currentVersion ?? "—",
                      )}
                      badge={
                        updateAvailable
                          ? t.settings.moreUpdateAvailable.replace(
                              "{version}",
                              updateCached?.latestVersion ?? "",
                            )
                          : undefined
                      }
                      onClick={() => {
                        setMenuOpen(false);
                        onOpenUpdate();
                      }}
                    />
                  )}

                  {/* 打开配置目录（与作用域面板同源目录） */}
                  {scopeDir && (
                    <MenuItem
                      icon={<FolderOpen className="h-4 w-4" />}
                      label={t.settings.moreOpenConfigDir}
                      desc={t.settings.moreOpenConfigDirDesc}
                      onClick={() => {
                        setMenuOpen(false);
                        void openInExplorer(scopeDir);
                      }}
                    />
                  )}

                  <div className="my-1 border-t border-slate-800" />

                  {/* 重置为默认：危险操作，红字 + 与上方分隔（设计 §2.2） */}
                  <MenuItem
                    icon={<RotateCcw className="h-4 w-4" />}
                    label={t.settings.reset}
                    desc={t.settings.moreResetDesc}
                    danger
                    onClick={() => {
                      setMenuOpen(false);
                      setResetOpen(true);
                    }}
                  />
                </div>
              )}
            </div>
          </div>
        </div>
      </div>

      <div className="flex min-h-0 flex-1">
        <SettingsSidebar
          tags={sidebarTags}
          activeTag={activeTag}
          onSelect={handleSelect}
          matchCounts={matchCounts}
          searching={searching}
        />

        <section ref={scrollRef} className="min-w-0 flex-1 overflow-y-auto p-4">
          {loading ? (
            <div className="flex h-full items-center justify-center text-muted">
              {t.common.loading}…
            </div>
          ) : searching && matched.length === 0 ? (
            <div className="flex h-full items-center justify-center text-muted">
              {t.settings.noResult}
            </div>
          ) : (
            // 右侧为「分组长页」：按可视级别（非搜索）或按命中分组（搜索）逐个标签渲染，
            // 每个标签一个带锚点 id 的分组区块，左侧点击仅平滑滚动定位到此。
            (searching ? sidebarTags : visibleTags).map((tg) => {
              const items = searching
                ? matched.filter((it) => itemTagMap[it.id] === tg.id)
                : tg.items.filter((it) => LEVEL_RANK[it.level] <= LEVEL_RANK[level]);
              const altCovered = altCoveredIn(items);
              return (
                <div key={tg.id} id={`tag-${tg.id}`} className="mb-6 scroll-mt-4">
                  <div className="mb-3 flex items-center gap-2">
                    <h3 className="text-sm font-semibold uppercase tracking-wide text-slate-400">
                      {text.tagLabel(tg.id)}
                    </h3>
                  </div>
                  <div className="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">
                    {items
                      .filter((it) => !altCovered.has(it.k))
                      .map((it) => (
                        <SettingCard
                          key={it.id}
                          item={it}
                          value={values[it.k] ?? it.val}
                          showDesc={showDesc}
                          highlight={searching}
                          disabled={disabledFor(it)}
                          candidatesOverride={candidateOverrideFor(it.k)}
                          value2={it.altKey ? values[it.altKey] : undefined}
                          onEdit={(v) => handleSave(it.k, v)}
                          onEditMulti={handleSaveMulti}
                        />
                      ))}
                  </div>
                  {/* 专用卡列（HD 素材状态 / AI 运行库 / 镜像规则表）：仅挂在「FV启动器」标签下，
                      三卡均为「实验性 / 非常用」能力，整列仅在「全部」档展示 ——
                      与其服务的开关同层（fv.hdEnhance / fv.ai* / fv.hdEdition* 等亦为「全部」级，
                      本标签无「进阶」项），避免低档位下卡片引用不可见开关（悬空引用） */}
                  {tg.id === "fvLauncher" && level === "all" && (
                    <div className="mt-3 max-w-xl space-y-3">
                      <HdEditionCard />
                      <AiRuntimeCard />
                      <MirrorRulesCard
                        value={(values["fv.networkMirrorRules"] as MirrorRule[] | undefined) ?? []}
                        onSave={saveMirrorRules}
                      />
                    </div>
                  )}
                </div>
              );
            })
          )}
        </section>
      </div>

      <RuntimeInfoDialog
        info={runtime}
        open={runtimeOpen}
        onClose={() => setRuntimeOpen(false)}
        scopeLabel={scopeLabel}
        sourcePath={scopeDir}
      />

      <ConfirmDialog
        open={resetOpen}
        title={t.settings.resetTitle.replace("{scope}", scopeLabel)}
        description={t.settings.resetDesc}
        confirmText={t.common.ok}
        onCancel={() => setResetOpen(false)}
        onConfirm={handleReset}
      />

      {toast && (
        <Toast
          text={toast.msg}
          kind={toast.type}
          duration={2600}
          resetKey={toast.msg}
          className="z-[200]"
          placement="bottom"
          onDone={() => setToast(null)}
        />
      )}
    </div>
  );
}

/**
 * 「更多」菜单项：图标 + 主文案 + 一行小字说明（可选右侧蓝标徽章；`danger` 为红字危险项）。
 * 一行说明用于消除"点上去才知道是什么"的猜测成本（v2 重构的菜单收纳项均为低频操作）。
 */
function MenuItem({
  icon,
  label,
  desc,
  badge,
  danger,
  onClick,
}: {
  icon: React.ReactNode;
  label: string;
  desc: string;
  badge?: string;
  danger?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex w-full items-start gap-2.5 px-3 py-2 text-left transition-colors hover:bg-slate-800"
    >
      <span className={cn("mt-0.5 flex-none", danger ? "text-red-400" : "text-slate-400")}>
        {icon}
      </span>
      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-2">
          <span className={cn("text-sm", danger ? "text-red-400" : "text-slate-200")}>{label}</span>
          {badge && (
            <span className="rounded bg-blue-500/15 px-1.5 py-0.5 text-[10px] font-medium text-blue-300">
              {badge}
            </span>
          )}
        </span>
        <span className="mt-0.5 block text-[11px] text-slate-500">{desc}</span>
      </span>
    </button>
  );
}
