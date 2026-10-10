import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import {
  AlertTriangle,
  ChevronDown,
  ChevronRight,
  Info,
  Minus,
  Plus,
} from "lucide-react";
import { useI18n } from "@/i18n";
import { cn } from "@/lib/utils";
import { compatRangeText, isModCompatible } from "@/lib/modCompat";
import {
  getEngineVersionSnapshot,
  loadEngineVersion,
  subscribeEngineVersion,
} from "@/lib/engineVersion";
import { vcmiLangName } from "@/lib/vcmiLangs";
import {
  getVcmiLanguageSnapshot,
  loadVcmiLanguage,
  modBaseLanguage,
  subscribeVcmiLanguage,
} from "@/lib/vcmiLanguage";
import { isTauri, getModsCacheDir, readModWikiCache } from "@/lib/tauri";
import { fullImgUrl } from "@/lib/modcache";
import { getWikiStatus } from "@/lib/modcache/status";
import { useWikiStageDone } from "@/shared/WikiBuildOverlay";
import { getTownPreviewEnabled, subscribeTownPreview } from "@/lib/townPreview";
import { useInViewport } from "@/shared/useInViewport";
import { townLoadingInc, townLoadingDec } from "@/lib/townLoading";
import type { ModInfo } from "@/lib/modScanner";
import { localizedName, localizedDescription } from "@/lib/modScanner";
import { richToPlainText } from "@/lib/html";
import { TYPE_COLORS } from "@/lib/minimapBase";
import { HdIcon } from "@/shared/HdIcon";

interface ModListItemProps {
  mod: ModInfo;
  lang: string;
  isSelected?: boolean;
  isChild?: boolean;
  isMissing?: boolean;
  isBase?: boolean;
  /** 全局强制模组：锁定（不显示开关，展示「强制」角标，不可取消） */
  locked?: boolean;
  hasConflict?: boolean;
  showTypeTag?: boolean;
  expandable?: boolean;
  expanded?: boolean;
  /** 城镇大图卡片模式（仅城镇标签页下使用） */
  townCard?: boolean;
  /** 城镇分组中的父卡片：显示「主体」角标与描边光晕 */
  isParent?: boolean;
  /** 紧凑行小图标仍用 creatureBackground；仓库可更新角标文案（`↑ x.y.z`，空 = 无更新） */
  updateTip?: string;
  /** 依赖缺陷角标（hover 说明文案；空 = 无角标）。依赖处置体系：见 `lib/modDeps.ts`。 */
  depTip?: string;
  onToggle?: () => void;
  onDetail?: () => void;
  onClick?: () => void;
  onExpandToggle?: () => void;
  parentName?: string;
}

export function ModListItem({
  mod,
  lang,
  isSelected = false,
  isChild = false,
  isMissing = false,
  isBase = false,
  locked = false,
  hasConflict = false,
  showTypeTag = true,
  expandable = false,
  expanded = false,
  townCard = false,
  isParent = false,
  updateTip = "",
  depTip = "",
  onToggle,
  onDetail,
  onClick,
  onExpandToggle,
  parentName,
}: ModListItemProps) {
  const { t } = useI18n();
  // 城镇背景预览开关（默认关闭）：只控制 **hover 浮层**，城镇 tab 的大卡片背景不受影响。
  // 紧凑行 + 开关关闭时干脆不读大图缓存，省掉每行的 IPC 与"未就绪 5s 重试"轮询。
  const previewEnabled = useSyncExternalStore(subscribeTownPreview, getTownPreviewEnabled);
  // 视口懒加载：数百行同时挂载时只让**可见行**发起图片 IPC（否则挂载瞬间就是数百次读缓存，
  // 首次无缓存时全部失败并进入重试）。滚到附近（rootMargin 300px）即开始加载。
  const { ref: rootRef, inView } = useInViewport<HTMLDivElement>();
  const townIcon = useTownIcon(mod, isMissing || townCard || !inView);
  const townBg = useTownBackground(mod, isMissing || (!previewEnabled && !townCard) || !inView);
  // hover 预览三态：hidden（不渲染）→ in（淡入）→ out（淡出后卸载）。
  // hidden 与 out 的中间态是为了让移出时有淡出动画，而不是"啪一下消失"。
  const [preview, setPreview] = useState<"hidden" | "in" | "out">("hidden");
  const previewTimer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(previewTimer.current), []);
  const title = localizedName(mod, lang);
  // 引擎版本兼容性（模块级单次加载的版本号 + 本地 `compatibility` 区间，判定见 lib/modCompat.ts）：
  // 加载完成前 `engineVersion === undefined` → 不判定，避免误报；完成后订阅触发重渲染补齐角标。
  const engineVersion = useSyncExternalStore(subscribeEngineVersion, getEngineVersionSnapshot);
  useEffect(() => {
    void loadEngineVersion();
  }, []);
  const incompatTip =
    engineVersion && !isModCompatible(engineVersion, mod.compatibility)
      ? t.mods.incompatTip
          .replace("{range}", compatRangeText(mod.compatibility) ?? "?")
          .replace("{ver}", engineVersion)
      : undefined;
  // 翻译模组语言不符（VCMI 会把 `modType: "Translation"` 且基准语言 ≠ 游戏语言的模组判为 BROKEN
  // 并从预设中移除，见 lib/vcmiLanguage.ts 模块头）。必须用 `rawType` 原文：
  // `type` 是后端按关键词**推断**的，推断成 Translation 的非翻译模组会造成误报。
  const vcmiLanguage = useSyncExternalStore(subscribeVcmiLanguage, getVcmiLanguageSnapshot);
  useEffect(() => {
    void loadVcmiLanguage();
  }, []);
  const langTip =
    mod.rawType === "Translation" &&
    !!vcmiLanguage &&
    modBaseLanguage(mod.language) !== vcmiLanguage
      ? t.mods.langMismatchTip
          .replace("{lang}", vcmiLangName(modBaseLanguage(mod.language)))
          .replace("{cur}", vcmiLangName(vcmiLanguage))
      : undefined;
  // 正向角标：翻译模组基准语言与当前游戏语言一致（建议启用，见 lang-mods 设计文档）
  const langMatch =
    mod.rawType === "Translation" &&
    !!vcmiLanguage &&
    modBaseLanguage(mod.language) === vcmiLanguage;
  // 列表/卡片只显示纯文本：mod.json 描述可能含 HTML（实测 `<h1 style=...>`），
  // 直接渲染会露出标签串。详情抽屉里再按 VCMI 口径渲染富文本（见 `shared/Markdown.tsx`）。
  const desc = richToPlainText(localizedDescription(mod, lang));
  const typeLabel = t.mods.typeLabels[mod.type] ?? mod.type;
  // 真城镇判定：type 为 Town **且** 后端透传的 factions 非空。
  // 后端 classify_mod 对显式 modType 直接采信（commands.rs），导致生物补充包
  // （如「树塔老生物」modType 标 Town）等被误标为 Town，但它们并无 factions 数据。
  // factions 由 scan_mod_dir 透传（mod.json 声明优先，否则探测 Content/config/factions）。
  const isTown =
    mod.type === "Town" && !isMissing && (mod.factions?.length ?? 0) > 0;

  // 大图（卡片或 hover 弹出）只用真正的城镇背景图（preview 合成图 / background）。
  // 不可用 townIcon(creatureBackground) 回退：生物补充包/修复包等有兵种背景但无城镇数据，
  // 回退会让它们被误渲染成城镇大图卡片（mod.type 不可靠，见后端 classify_mod 显式 modType 优先）。
  const bigImage = isTown ? townBg : null;
  // 紧凑行小图标仍用 creatureBackground
  const smallImage = isTown ? townIcon : null;

  // hover 预览：150ms hover intent —— 快速掠过多行不会触发；移出先淡出再卸载，
  // 避免"城镇 ↔ 非城镇"来回移动时的闪烁（开关关闭时直接不进这段逻辑）。
  const showPreviewSoon = () => {
    if (!previewEnabled || !isTown) return;
    window.clearTimeout(previewTimer.current);
    previewTimer.current = window.setTimeout(() => setPreview("in"), PREVIEW_DELAY_MS);
  };
  const hidePreview = () => {
    window.clearTimeout(previewTimer.current);
    if (preview === "hidden") return;
    setPreview("out");
    previewTimer.current = window.setTimeout(() => setPreview("hidden"), PREVIEW_FADE_MS);
  };

  // 城镇大图卡片：真城镇（含子模组城镇）统一以同款卡片呈现。
  // 有无背景预览图、有无描述，容器高宽（aspect-ratio 800/374）保持一致；
  // 无图时不渲染字体图标占位，改用纯色渐变；操作按钮统一收进右上角紧凑区。
  if (townCard && isTown) {
    return (
      <div
        ref={rootRef}
        onClick={(e) => {
          if ((e.target as HTMLElement).closest("button")) return;
          onClick?.();
        }}
        className={cn(
          "group relative flex flex-col overflow-hidden rounded-xl border bg-slate-900/60 transition-all",
          hasConflict ? "border-red-500/40" : "border-slate-700/50",
          isMissing && "opacity-70",
          isParent && "ring-1 ring-indigo-400/40",
          "hover:ring-2 hover:ring-indigo-400/60",
        )}
        style={{ aspectRatio: "800 / 374" }}
      >
        {/* 背景层：有城市大图显示；无图用纯色渐变占位（不渲染字体图标） */}
        {bigImage ? (
          <HdIcon
            src={bigImage}
            alt=""
            className="absolute inset-0 h-full w-full object-cover"
            draggable={false}
          />
        ) : (
          <div className="absolute inset-0 bg-gradient-to-br from-slate-800 to-slate-900" />
        )}
        {/* 渐变遮罩，保证文字可读 */}
        <div className="absolute inset-0 bg-gradient-to-t from-black/80 via-black/30 to-black/10" />

        {/* 顶栏：标签（左） + 操作（右上，紧凑） */}
        <div className="relative z-10 flex items-start justify-between gap-2 p-3">
          <div className="flex flex-wrap items-center gap-1">
            {showTypeTag && (
              <span
                className="rounded bg-black/40 px-2 py-0.5 text-[10px] font-medium backdrop-blur-sm"
                style={{ color: TYPE_COLORS[mod.type] ?? "#fff" }}
              >
                {typeLabel}
              </span>
            )}
            {isMissing && (
              <span className="flex items-center gap-0.5 rounded bg-red-500/80 px-1.5 py-0.5 text-[10px] text-white">
                <AlertTriangle className="h-3 w-3" />
                {t.mods.missing}
              </span>
            )}
            {isBase && (
              <span className="rounded border border-green-400/60 bg-black/40 px-1.5 py-0.5 text-[10px] text-green-300 backdrop-blur-sm">
                {t.mods.base}
              </span>
            )}
            {isParent && (
              <span className="rounded border border-indigo-400/60 bg-black/40 px-1.5 py-0.5 text-[10px] text-indigo-300 backdrop-blur-sm">
                {t.mods.parentBadge}
              </span>
            )}
            {incompatTip && !isMissing && (
              <span
                className="flex items-center gap-0.5 rounded border border-amber-400/60 bg-black/40 px-1.5 py-0.5 text-[10px] text-amber-300 backdrop-blur-sm"
                title={incompatTip}
              >
                <AlertTriangle className="h-3 w-3" />
                {t.mods.incompatible}
              </span>
            )}
            {/* 依赖缺陷角标（依赖处置体系）：缺必需依赖的模组启用后会被引擎剔除 */}
            {depTip && !isMissing && (
              <span
                className="flex items-center gap-0.5 rounded border border-orange-400/60 bg-black/40 px-1.5 py-0.5 text-[10px] text-orange-300 backdrop-blur-sm"
                title={depTip}
              >
                <AlertTriangle className="h-3 w-3" />
                {t.mods.depTag}
              </span>
            )}
            {langTip && !isMissing && (
              <span
                className="flex items-center gap-0.5 rounded border border-sky-400/60 bg-black/40 px-1.5 py-0.5 text-[10px] text-sky-300 backdrop-blur-sm"
                title={langTip}
              >
                <AlertTriangle className="h-3 w-3" />
                {t.mods.langMismatch}
              </span>
            )}
            {langMatch && !isMissing && (
              <span
                className="rounded border border-green-400/60 bg-black/40 px-1.5 py-0.5 text-[10px] text-green-300 backdrop-blur-sm"
                title={t.mods.langMatch}
              >
                {t.mods.langMatch}
              </span>
            )}
            {/* 仓库可更新角标（mod-acquisition-update §9.1）；与冲突/不兼容共存时排在其后 */}
            {updateTip && !isMissing && (
              <span
                className="rounded bg-amber-500/90 px-1.5 py-0.5 text-[10px] font-medium text-white backdrop-blur-sm"
                title={updateTip}
              >
                {updateTip}
              </span>
            )}
            {hasConflict && !isMissing && (
              <span className="rounded bg-red-500/80 px-1.5 py-0.5 text-[10px] text-white">⚠</span>
            )}
          </div>
          <div className="flex flex-none items-center gap-1">
            <button
              onClick={(e) => {
                e.stopPropagation();
                onDetail?.();
              }}
              className="rounded-md bg-black/40 p-1.5 text-white/90 backdrop-blur-sm hover:bg-black/60"
              title={t.mods.detail}
            >
              <Info className="h-3.5 w-3.5" />
            </button>
            {!isMissing && !isBase && !locked && onToggle && (
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  onToggle();
                }}
                className={cn(
                  "flex items-center gap-1 rounded-md px-2 py-1 text-[11px] font-medium backdrop-blur-sm",
                  isSelected
                    ? "bg-red-500/30 text-red-100 hover:bg-red-500/50"
                    : "bg-blue-500/40 text-white hover:bg-blue-500/60",
                )}
              >
                {isSelected ? (
                  <>
                    <Minus className="h-3 w-3" />
                    {t.mods.remove}
                  </>
                ) : (
                  <>
                    <Plus className="h-3 w-3" />
                    {t.mods.add}
                  </>
                )}
              </button>
            )}
            {!isMissing && locked && (
              <span className="flex items-center gap-1 rounded-md bg-amber-500/20 px-2 py-1 text-[11px] font-medium text-amber-200">
                {t.mods.forced}
              </span>
            )}
          </div>
        </div>

        {/* 底部：标题（固定1行）+ 描述（固定最大2行）+ 子模组归属 */}
        <div className="relative z-10 mt-auto flex flex-col gap-0.5 p-3">
          <div
            className="h-[22px] truncate text-[15px] font-bold leading-[22px] text-white drop-shadow"
            title={title}
          >
            {title}
          </div>
          <p
            className="h-[32px] line-clamp-2 text-xs leading-[16px] text-slate-200/90 drop-shadow"
            title={desc}
          >
            {desc || t.mods.noDescription}
          </p>
          {isChild && parentName && (
            <p className="h-[14px] text-[10px] leading-[14px] text-slate-300/80">
              {t.mods.childOf.replace("{parent}", parentName)}
            </p>
          )}
        </div>
      </div>
    );
  }

  // 紧凑行模式（全部 / 其它类型标签页）
  return (
    <div
      ref={rootRef}
      onClick={(e) => {
        if ((e.target as HTMLElement).closest("button")) return;
        onClick?.();
      }}
      className={cn(
        "group relative flex items-center gap-3 rounded-xl border border-slate-700/50 bg-slate-900/50 p-3 transition-colors",
        "hover:bg-slate-800/60",
        // 子模组的缩进 / 导轨由外层容器统一承担（方案 A），这里不再逐行 ml / border-l
        hasConflict && "border-red-500/40",
        isMissing && "opacity-70",
      )}
      onMouseEnter={showPreviewSoon}
      onMouseLeave={hidePreview}
    >
      {/* 定宽前置槽（w-4）：有子模组放展开按钮，无子模组留等宽占位。
          目的——让父行 / 叶子行 / 子行的图标与标题起点落在同一条竖线上（父子视觉对齐）。 */}
      <div className="flex w-4 flex-none justify-center">
        {expandable && (
          <button
            onClick={(e) => {
              e.stopPropagation();
              onExpandToggle?.();
            }}
            className="text-slate-500 hover:text-slate-200"
            title={expanded ? t.mods.collapse : t.mods.expand}
          >
            {expanded ? (
              <ChevronDown className="h-4 w-4" />
            ) : (
              <ChevronRight className="h-4 w-4" />
            )}
          </button>
        )}
      </div>

      {/* 图标（城镇提供小预览图标） */}
      <div
        className={cn(
          "flex h-12 w-12 flex-none items-center justify-center overflow-hidden rounded-lg",
          smallImage ? "bg-transparent" : "bg-slate-800",
        )}
      >
        {smallImage ? (
          <HdIcon src={smallImage} alt="" className="h-full w-full object-cover" draggable={false} />
        ) : (
          <TypeIcon type={mod.type} />
        )}
      </div>

      {/* 文本 */}
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span
            className={cn(
              "truncate text-[14px] font-semibold",
              isMissing ? "text-slate-400" : "text-slate-100",
            )}
            title={title}
          >
            {title}
          </span>
          {showTypeTag && (
            <span
              className="flex-none whitespace-nowrap rounded bg-slate-800 px-1.5 py-0.5 text-[10px] font-medium text-slate-300"
              style={{ color: TYPE_COLORS[mod.type] ?? "#94a3b8" }}
            >
              {typeLabel}
            </span>
          )}
          {isMissing && (
            <span
              className="flex flex-none items-center gap-0.5 whitespace-nowrap rounded bg-red-500/15 px-1.5 py-0.5 text-[10px] text-red-400"
              title={t.mods.missingTip}
            >
              <AlertTriangle className="h-3 w-3" />
              {t.mods.missing}
            </span>
          )}
          {hasConflict && !isMissing && (
            <span
              className="flex-none whitespace-nowrap rounded bg-red-500/15 px-1.5 py-0.5 text-[10px] text-red-400"
              title={t.mods.conflictTip}
            >
              ⚠
            </span>
          )}
          {incompatTip && !isMissing && (
            <span
              className="flex flex-none items-center gap-0.5 whitespace-nowrap rounded bg-amber-500/15 px-1.5 py-0.5 text-[10px] text-amber-300"
              title={incompatTip}
            >
              <AlertTriangle className="h-3 w-3" />
              {t.mods.incompatible}
            </span>
          )}
          {langTip && !isMissing && (
            <span
              className="flex flex-none items-center gap-0.5 whitespace-nowrap rounded bg-sky-500/15 px-1.5 py-0.5 text-[10px] text-sky-300"
              title={langTip}
            >
              <AlertTriangle className="h-3 w-3" />
              {t.mods.langMismatch}
            </span>
          )}
          {langMatch && !isMissing && (
            <span
              className="flex-none whitespace-nowrap rounded bg-green-500/15 px-1.5 py-0.5 text-[10px] text-green-300"
              title={t.mods.langMatch}
            >
              {t.mods.langMatch}
            </span>
          )}
          {/* 依赖缺陷角标（依赖处置体系）：缺必需依赖的模组启用后会被引擎剔除 */}
          {depTip && !isMissing && (
            <span
              className="flex flex-none items-center gap-0.5 whitespace-nowrap rounded bg-orange-500/15 px-1.5 py-0.5 text-[10px] text-orange-300"
              title={depTip}
            >
              <AlertTriangle className="h-3 w-3" />
              {t.mods.depTag}
            </span>
          )}
          {isBase && (
            <span className="flex-none whitespace-nowrap rounded border border-green-500/40 px-1.5 py-0.5 text-[10px] text-green-400">
              {t.mods.base}
            </span>
          )}
          {/* 仓库可更新角标（§9.1，同城镇卡片） */}
          {updateTip && !isMissing && (
            <span
              className="flex-none whitespace-nowrap rounded bg-amber-500/90 px-1.5 py-0.5 text-[10px] font-medium text-white"
              title={updateTip}
            >
              {updateTip}
            </span>
          )}
        </div>
        <p
          className={cn(
            "line-clamp-2 text-xs",
            isMissing ? "text-slate-500" : "text-slate-400",
          )}
          title={desc}
        >
          {desc || t.mods.noDescription}
        </p>
        {parentName && (
          <p className="mt-0.5 text-[10px] text-slate-500">
            {t.mods.childOf.replace("{parent}", parentName)}
          </p>
        )}
      </div>

      {/* 操作 */}
      <div className="flex flex-none items-center gap-1.5">
        <button
          onClick={(e) => {
            e.stopPropagation();
            onDetail?.();
          }}
          className="rounded-lg p-2 text-slate-400 hover:bg-slate-800 hover:text-slate-100"
          title={t.mods.detail}
        >
          <Info className="h-4 w-4" />
        </button>
        {!isMissing && !isBase && !locked && onToggle && (
          <button
            onClick={(e) => {
              e.stopPropagation();
              onToggle();
            }}
            className={cn(
              "flex items-center gap-1 rounded-lg px-2.5 py-1.5 text-xs font-medium transition-colors",
              isSelected
                ? "bg-red-500/15 text-red-300 hover:bg-red-500/25"
                : "bg-blue-500/15 text-blue-300 hover:bg-blue-500/25",
            )}
          >
            {isSelected ? (
              <>
                <Minus className="h-3.5 w-3.5" />
                {t.mods.remove}
              </>
            ) : (
              <>
                <Plus className="h-3.5 w-3.5" />
                {t.mods.add}
              </>
            )}
          </button>
        )}
        {!isMissing && locked && (
          <span className="flex items-center gap-1 rounded-lg bg-amber-500/20 px-2.5 py-1.5 text-xs font-medium text-amber-200">
            {t.mods.forced}
          </span>
        )}
      </div>

      {/* 城镇大图 hover 弹出层：延迟 150ms 淡入，移出淡出后才卸载 */}
      {preview !== "hidden" && previewEnabled && bigImage && (
        <div
          className={cn(
            "pointer-events-none fixed left-1/2 top-4 z-[120] w-[480px] max-w-[90vw] -translate-x-1/2 overflow-hidden rounded-2xl border border-slate-700/60 shadow-2xl transition-opacity duration-150",
            preview === "in" ? "opacity-100" : "opacity-0",
          )}
        >
          <HdIcon src={bigImage} alt="" className="aspect-[800/374] w-full object-cover" />
          <div className="absolute bottom-0 left-0 right-0 bg-gradient-to-t from-black/70 to-transparent p-3">
            <div className="text-sm font-semibold text-white">{title}</div>
            <div className="text-[11px] text-slate-300">{t.mods.previewClose}</div>
          </div>
        </div>
      )}
    </div>
  );
}

/** 字段优先级常量（模块级，保证 useEffect 依赖引用稳定） */
const ICON_FIELDS = ["creatureBackground"];
/** hover 大图预览：显示延迟（hover intent）与淡入淡出时长（ms） */
const PREVIEW_DELAY_MS = 150;
const PREVIEW_FADE_MS = 150;
/** 大图优先取合成预览图 townscreen/*.webp（含全建筑），回退纯背景 */
const BG_FIELDS = ["preview", "background"];

/** 城镇类型模组：从 wiki 缓存读取城镇大图/兵种背景图（不解析原始 mod） */
function useTownCacheImage(
  mod: ModInfo,
  disabled: boolean,
  fields: string[],
): string | null {
  const [url, setUrl] = useState<string | null>(null);
  // 素材阶段完成 → 重新读取，补齐此前未就绪的预览。
  // ⚠️ 用 `useWikiStageDone("assets")` 而**不是**裸订阅 `busy`：
  // 城镇预览的前提是**素材**就绪，没必要等高清（hd）跑完 —— 否则开了高清的用户
  // 要多等几十分钟才看到预览（而 hd 只是增强项）。
  // 同时它是 boolean 快照，进度数字变化不会触发重渲染。
  const ready = useWikiStageDone("assets");
  useEffect(() => {
    if (disabled || mod.type !== "Town" || !mod.id) {
      setUrl(null);
      return;
    }
    if (!isTauri()) {
      setUrl(null);
      return;
    }
    let cancelled = false;
    let timer: number | undefined;

    // 不再因「构建中」阻塞读取：已生成的缓存立即显示，未生成的显示占位。
    // 未取到图时低频重试（带随机抖动错开请求峰值），实现图片随构建逐个出现。
    const load = async (): Promise<boolean> => {
      if (cancelled) return true;
      townLoadingInc();
      try {
        const [cacheDir, cache] = await Promise.all([getModsCacheDir(), readModWikiCache(mod.id)]);
        if (cancelled) return true;
        if (!cache?.ok || !cacheDir) return false;
        const data = cache.data as Record<string, unknown> | undefined;
        const factions = (data?.factions as Record<string, unknown>[]) ?? [];
        // 按优先级取首个非空字段（大图：合成预览图 → 纯背景）
        let rel = "";
        for (const f of fields) {
          const v = factions[0]?.[f] as string | undefined;
          if (v && v.length > 0) {
            rel = v;
            break;
          }
        }
        setUrl(rel ? fullImgUrl(cacheDir, rel) : null);
        return !!rel;
      } catch {
        setUrl(null);
        return false;
      } finally {
        townLoadingDec();
      }
    };

    // 未取到图时**指数退避**重试（5s → 10s → 20s，上限 30s，+ 抖动）：
    // 首次启动是"无缓存 + 构建中"，数百张卡片会同时失败并重试，固定 5s 轮询会造成周期性 IPC 尖峰。
    // 正常路径由 `useWikiStageDone("assets")` 的阶段完成跃迁统一补齐，退避只是兜底。
    let attempt = 0;
    const run = async () => {
      const got = await load();
      if (got || cancelled) return;
      if (getWikiStatus().busy) {
        attempt += 1;
        const delay = Math.min(5000 * 2 ** (attempt - 1), 30000) + Math.random() * 2000;
        timer = window.setTimeout(run, delay);
      }
    };
    run();

    return () => {
      cancelled = true;
      if (timer !== undefined) window.clearTimeout(timer);
      townLoadingDec();
    };
  }, [mod, disabled, fields, ready]);
  return url;
}

/** 城镇类型模组：阵营图标（小图标，读取 wiki 缓存） */
function useTownIcon(mod: ModInfo, disabled: boolean): string | null {
  return useTownCacheImage(mod, disabled, ICON_FIELDS);
}

/** 城镇类型模组：城镇大图（优先含全建筑的合成预览图，读取 wiki 缓存） */
function useTownBackground(mod: ModInfo, disabled: boolean): string | null {
  return useTownCacheImage(mod, disabled, BG_FIELDS);
}

function TypeIcon({ type, size = 20 }: { type: string; size?: number }) {
  const color = TYPE_COLORS[type] ?? "#94a3b8";
  switch (type) {
    case "Town":
      return (
        <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth="1.5">
          <path d="M3 21h18M5 21V7l7-4 7 4v14M8 11h8M8 15h8" />
        </svg>
      );
    case "Translation":
      return (
        <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth="1.5">
          <circle cx="12" cy="12" r="10" />
          <path d="M2 12h20M12 2a15.3 15.3 0 0 1 4 10 15.3 15.3 0 0 1-4 10 15.3 15.3 0 0 1-4-10 15.3 15.3 0 0 1 4-10z" />
        </svg>
      );
    case "Heroes":
      return (
        <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth="1.5">
          <path d="M12 2l2.4 7.2h7.6l-6 4.8 2.4 7.2-6-4.8-6 4.8 2.4-7.2-6-4.8h7.6z" />
        </svg>
      );
    case "Music":
    case "Sounds":
      return (
        <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth="1.5">
          <path d="M9 18V5l12-2v13" />
          <circle cx="6" cy="18" r="3" />
          <circle cx="18" cy="16" r="3" />
        </svg>
      );
    case "Mechanics":
      return (
        <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth="1.5">
          <circle cx="12" cy="12" r="3" />
          <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 1 1-4 0v-.09A1.65 1.65 0 0 0 7 18.4l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 1 1 0-4h.09A1.65 1.65 0 0 0 4.6 7l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 1 1 4 0v.09a1.65 1.65 0 0 0 1 1.51h.09a1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 1 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
        </svg>
      );
    default:
      return (
        <svg width={size} height={size} viewBox="0 0 24 24" fill="none" stroke={color} strokeWidth="1.5">
          <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z" />
          <polyline points="3.27 6.96 12 12.01 20.73 6.96" />
          <line x1="12" y1="22.08" x2="12" y2="12" />
        </svg>
      );
  }
}
