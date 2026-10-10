import { ChevronDown } from "lucide-react";
import { cn } from "@/lib/utils";
import type { RealmAssets, RealmBackgroundFilter } from "@/lib/realm/types";
import { FilterSlider } from "../FilterSlider";
import { Field } from "../Field";
import type { LangDict } from "@/i18n/types";

export interface IntroTabProps {
  t: LangDict;
  /** 当前编辑语言的资源（已按「默认语言回退」合并后的结果） */
  curAssets: RealmAssets;
  curLang: string;
  langLabel: (v: string) => string;
  isBaseLang: boolean;
  isFallbackAsset: (k: keyof RealmAssets) => boolean;
  assetPreview: (rel?: string) => string;
  pickAsset: (kind: "wallpaper" | "cover" | "music" | "introMusic" | "introVideo", pickKind: "video" | "cover" | "wallpaper" | "music" | "introMusic") => Promise<void>;
  setCurAssets: (patch: RealmAssets) => void;
  introSeconds: number;
  setIntroSeconds: React.Dispatch<React.SetStateAction<number>>;
  introSkippable: boolean;
  setIntroSkippable: React.Dispatch<React.SetStateAction<boolean>>;
  introShowName: boolean;
  setIntroShowName: React.Dispatch<React.SetStateAction<boolean>>;
  /** §6 `showGameName`：两页标题带游戏名前缀（v1.25 由「基本信息」迁入，共用组内） */
  showGameName: boolean;
  setShowGameName: React.Dispatch<React.SetStateAction<boolean>>;
  loopMusic: boolean;
  setLoopMusic: React.Dispatch<React.SetStateAction<boolean>>;
  maskOpacity: number;
  setMaskOpacity: React.Dispatch<React.SetStateAction<number>>;
  background: RealmBackgroundFilter | undefined;
  setBackground: React.Dispatch<React.SetStateAction<RealmBackgroundFilter | undefined>>;
  appearanceOpen: boolean;
  setAppearanceOpen: React.Dispatch<React.SetStateAction<boolean>>;
}

/**
 * 「领域登场」标签页（v1.25 重排）。
 *
 * 按两个**渲染页面**的实际消费关系分三组（分组口径与预览菜单
 * 「预览领域登场」「预览领域主界面」一一对应）：
 * - 共用配置：壁纸 / 封面 / 介绍文本 / 标题显示开关（两页均生效）；
 * - 领域主界面：背景音乐 / 循环 / 文字遮罩强度 / 主界面外观（滤镜折叠区）；
 * - 登场演出：介绍视频 / 介绍音频 / 演出秒数 / 允许跳过。
 */
export function IntroTab(props: IntroTabProps) {
  const { t, curAssets, curLang, langLabel, isBaseLang, isFallbackAsset, assetPreview, pickAsset, setCurAssets, introSeconds, setIntroSeconds, introSkippable, setIntroSkippable, introShowName, setIntroShowName, showGameName, setShowGameName, loopMusic, setLoopMusic, maskOpacity, setMaskOpacity, background, setBackground, appearanceOpen, setAppearanceOpen } = props;
  return (
    <>
      {/* 标签页说明：三组的分工 */}
      <p className="text-xs text-muted">{t.realm.introTabHint}</p>

      {/* ── 共用配置（领域主界面 + 登场演出同时生效，仅出现一次）── */}
      <div className="space-y-3 rounded-lg border border-slate-800 bg-slate-900/40 p-3">
        <div>
          <div className="text-sm font-medium text-foreground">{t.realm.introCommon}</div>
          <p className="mt-1 text-xs text-muted">{t.realm.introCommonHint}</p>
        </div>

        {/* 壁纸图片（§9.6.1：预览 + 上传；用于领域界面 / 登场演出背景，对应设计 §6 的 `wallpaper`）。素材按当前编辑语言存放。 */}
        <Field label={t.realm.introCover}>
          <div className="flex items-center gap-3">
            {assetPreview(curAssets.wallpaper) && (
              <img
                src={assetPreview(curAssets.wallpaper)}
                alt=""
                className="h-16 w-16 shrink-0 rounded object-contain"
                draggable={false}
                /* 素材文件缺失时隐藏（保留占位空间，避免布局跳动），不出 broken 图标 */
                onError={(e) => {
                  e.currentTarget.style.visibility = "hidden";
                }}
              />
            )}
            <div className="flex min-w-0 flex-1 items-center gap-2">
              <button
                type="button"
                onClick={() => pickAsset("wallpaper", "wallpaper")}
                className="shrink-0 rounded border border-slate-700 px-2.5 py-1 text-xs text-slate-300 hover:bg-slate-800/50"
              >
                {t.realm.introPickCover}
              </button>
              <span className="min-w-0 flex-1 truncate text-xs text-muted">
                {curAssets.wallpaper || t.realm.introNoCover}
              </span>
              {isFallbackAsset("wallpaper") && (
                <span className="shrink-0 rounded bg-slate-700/60 px-1.5 py-0.5 text-[10px] text-slate-400">
                  {t.realm.followDefault}
                </span>
              )}
              {curAssets.wallpaper && (
                <button
                  type="button"
                  onClick={() => setCurAssets({ wallpaper: "" })}
                  className="shrink-0 text-xs text-red-400 hover:underline"
                >
                  {t.realm.introClear}
                </button>
              )}
            </div>
          </div>
          <p className="mt-1 text-xs text-muted">{t.realm.introCoverHint}</p>
        </Field>

        {/* 封面图片（可选：用于切换领域列表；不上传时自动用壁纸图片生成，对应设计 §6 的 `cover`，§9.6.1/§9.6.2）。素材按当前编辑语言存放。 */}
        <Field label={t.realm.introThumb}>
          <div className="flex items-center gap-3">
            {assetPreview(curAssets.cover) && (
              <img
                src={assetPreview(curAssets.cover)}
                alt=""
                className="h-16 w-16 shrink-0 rounded object-contain"
                draggable={false}
                /* 素材文件缺失时隐藏（保留占位空间，避免布局跳动），不出 broken 图标 */
                onError={(e) => {
                  e.currentTarget.style.visibility = "hidden";
                }}
              />
            )}
            <div className="flex min-w-0 flex-1 items-center gap-2">
              <button
                type="button"
                onClick={() => pickAsset("cover", "cover")}
                className="shrink-0 rounded border border-slate-700 px-2.5 py-1 text-xs text-slate-300 hover:bg-slate-800/50"
              >
                {t.realm.introPickThumb}
              </button>
              <span className="min-w-0 flex-1 truncate text-xs text-muted">
                {curAssets.cover || t.realm.introNoThumb}
              </span>
              {isFallbackAsset("cover") && (
                <span className="shrink-0 rounded bg-slate-700/60 px-1.5 py-0.5 text-[10px] text-slate-400">
                  {t.realm.followDefault}
                </span>
              )}
              {curAssets.cover && (
                <button
                  type="button"
                  onClick={() => setCurAssets({ cover: "" })}
                  className="shrink-0 text-xs text-red-400 hover:underline"
                >
                  {t.realm.introClear}
                </button>
              )}
            </div>
          </div>
          <p className="mt-1 text-xs text-muted">{t.realm.introThumbHint}</p>
        </Field>

        {/* 介绍文本（单字段 + 语言切换器，§9.6.1）：领域主界面左下角文本框 / 登场演出滚动字幕。
            严格按界面语言显示，缺失时隐藏。 */}
        <Field label={`${t.realm.introText}（${langLabel(curLang)}）`}>
          <textarea
            value={curAssets.introText ?? ""}
            onChange={(e) => setCurAssets({ introText: e.target.value })}
            placeholder={t.realm.introTextPh}
            rows={6}
            className="w-full rounded-lg border border-slate-700 bg-slate-900 px-3 py-2 text-sm text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500/50"
          />
          {!isBaseLang && !(curAssets.introText ?? "").trim() && (
            <p className="mt-1.5 text-xs text-amber-400/90">
              {t.realm.introTextFallbackHint.replace("{lang}", langLabel(curLang))}
            </p>
          )}
        </Field>

        {/* 标题显示（两页共用）：领域名称显隐（§6 `intro.showRealmName`）+ 游戏名前缀（顶层 `showGameName`，v1.25 由「基本信息」迁入） */}
        <div className="space-y-1.5">
          <label className="flex cursor-pointer items-center gap-2 text-sm text-foreground">
            <input
              type="checkbox"
              checked={introShowName}
              onChange={(e) => setIntroShowName(e.target.checked)}
              className="h-4 w-4 accent-blue-500"
            />
            {t.realm.introShowName}
          </label>
          <label className="flex cursor-pointer items-center gap-2 text-sm text-foreground">
            <input
              type="checkbox"
              checked={showGameName}
              onChange={(e) => setShowGameName(e.target.checked)}
              className="h-4 w-4 accent-blue-500"
            />
            {t.realm.showGameName}
          </label>
          <p className="text-xs text-muted">{t.realm.showGameNameHint}</p>
        </div>
      </div>

      {/* ── 领域主界面（§6 `main`）── */}
      <div className="space-y-3 rounded-lg border border-slate-800 bg-slate-900/40 p-3">
        <div className="text-sm font-medium text-foreground">{t.realm.realmHome}</div>

        {/* 背景音乐（`assets.music`）：覆盖层根部常驻播放，切二级页不中断 */}
        <Field label={t.realm.music}>
          <div className="flex min-w-0 items-center gap-2">
            <button
              type="button"
              onClick={() => pickAsset("music", "music")}
              className="shrink-0 rounded border border-slate-700 px-2.5 py-1 text-xs text-slate-300 hover:bg-slate-800/50"
            >
              {t.realm.pickMusic}
            </button>
            <span className="min-w-0 flex-1 truncate text-xs text-muted">
              {curAssets.music || t.realm.noMusic}
            </span>
            {isFallbackAsset("music") && (
              <span className="shrink-0 rounded bg-slate-700/60 px-1.5 py-0.5 text-[10px] text-slate-400">
                {t.realm.followDefault}
              </span>
            )}
            {curAssets.music && (
              <button
                type="button"
                onClick={() => setCurAssets({ music: "" })}
                className="shrink-0 text-xs text-red-400 hover:underline"
              >
                {t.realm.introClear}
              </button>
            )}
          </div>
          <p className="mt-1 text-xs text-muted">{t.realm.musicHint}</p>
        </Field>

        <label className="flex cursor-pointer items-center gap-2 text-sm text-foreground">
          <input
            type="checkbox"
            checked={loopMusic}
            onChange={(e) => setLoopMusic(e.target.checked)}
            className="h-4 w-4 accent-blue-500"
          />
          {t.realm.loopMusic}
        </label>
        <p className="-mt-1 text-xs text-muted">{t.realm.loopMusicHint}</p>

        {/* 文字遮罩强度（`main.maskOpacity`）：主界面标题 / 文本框 / 菜单按钮的局部蒙层 */}
        <Field label={t.realm.maskOpacity}>
          <div className="flex items-center gap-3">
            <input
              type="range"
              min={0}
              max={100}
              value={maskOpacity}
              onChange={(e) => setMaskOpacity(Number(e.target.value))}
              className="h-1.5 flex-1 cursor-pointer appearance-none rounded-full bg-slate-700 accent-blue-500"
            />
            <span className="w-10 text-right text-xs tabular-nums text-muted">{maskOpacity}</span>
          </div>
          <p className="mt-1 text-xs text-muted">{t.realm.maskOpacityHint}</p>
        </Field>

        {/* 主界面外观（主题化）：子折叠区，默认收起，只含 6 项背景滤镜 */}
        <div className="rounded-lg border border-slate-800 bg-slate-950/40">
          <button
            type="button"
            onClick={() => setAppearanceOpen((v) => !v)}
            className="flex w-full items-center justify-between px-3 py-2 text-sm font-medium text-foreground"
          >
            <span>{t.realm.homeAppearance}</span>
            <ChevronDown className={cn("h-4 w-4 transition-transform", !appearanceOpen && "-rotate-90")} />
          </button>
          {appearanceOpen && (
            <div className="space-y-3 px-3 pb-3">
              <p className="text-xs text-muted">{t.realm.appearanceHint}</p>
              <FilterSlider label={t.realm.filterBrightness} hint={t.realm.filterBrightnessHint} min={0} max={200} value={background?.brightness ?? 100} unit="%" onChange={(v) => setBackground((b) => ({ ...(b ?? {}), brightness: v }))} />
              <FilterSlider label={t.realm.filterBlur} hint={t.realm.filterBlurHint} min={0} max={20} value={background?.blur ?? 0} unit="px" onChange={(v) => setBackground((b) => ({ ...(b ?? {}), blur: v }))} />
              <FilterSlider label={t.realm.filterSaturate} hint={t.realm.filterSaturateHint} min={0} max={200} value={background?.saturate ?? 100} unit="%" onChange={(v) => setBackground((b) => ({ ...(b ?? {}), saturate: v }))} />
              <FilterSlider label={t.realm.filterGrayscale} hint={t.realm.filterGrayscaleHint} min={0} max={100} value={background?.grayscale ?? 0} unit="%" onChange={(v) => setBackground((b) => ({ ...(b ?? {}), grayscale: v }))} />
              <FilterSlider label={t.realm.filterContrast} hint={t.realm.filterContrastHint} min={0} max={200} value={background?.contrast ?? 100} unit="%" onChange={(v) => setBackground((b) => ({ ...(b ?? {}), contrast: v }))} />
              <FilterSlider label={t.realm.filterHueRotate} hint={t.realm.filterHueRotateHint} min={0} max={360} value={background?.hueRotate ?? 0} unit="°" onChange={(v) => setBackground((b) => ({ ...(b ?? {}), hueRotate: v }))} />
            </div>
          )}
        </div>
      </div>

      {/* ── 登场演出（§5.4 / §5.5）：是否登场由是否配置素材决定（§9.6.1 核对点 3），故无主开关 ── */}
      <div className="space-y-3 rounded-lg border border-slate-800 bg-slate-900/40 p-3">
        <div>
          <div className="text-sm font-medium text-foreground">{t.realm.introGroup}</div>
          <p className="mt-1 text-xs text-muted">{t.realm.introHint}</p>
        </div>

        {/* 介绍视频（§9.6.1：预览 + 上传；§9.6.2 玩家文案） */}
        <Field label={t.realm.introVideo}>
          <div className="flex items-center gap-3">
            {assetPreview(curAssets.introVideo) && (
              <video
                src={assetPreview(curAssets.introVideo)}
                muted
                className="h-16 w-28 shrink-0 rounded object-contain"
              />
            )}
            <div className="flex min-w-0 flex-1 items-center gap-2">
              <button
                type="button"
                onClick={() => pickAsset("introVideo", "video")}
                className="shrink-0 rounded border border-slate-700 px-2.5 py-1 text-xs text-slate-300 hover:bg-slate-800/50"
              >
                {t.realm.introPickVideo}
              </button>
              <span className="min-w-0 flex-1 truncate text-xs text-muted">
                {curAssets.introVideo || t.realm.introNoVideo}
              </span>
              {isFallbackAsset("introVideo") && (
                <span className="shrink-0 rounded bg-slate-700/60 px-1.5 py-0.5 text-[10px] text-slate-400">
                  {t.realm.followDefault}
                </span>
              )}
              {curAssets.introVideo && (
                <button
                  type="button"
                  onClick={() => setCurAssets({ introVideo: "" })}
                  className="shrink-0 text-xs text-red-400 hover:underline"
                >
                  {t.realm.introClear}
                </button>
              )}
            </div>
          </div>
          <p className="mt-1 text-xs text-muted">{t.realm.introVideoHint}</p>
        </Field>

        {/* 介绍音频（§6 `assets.introMusic`）：无介绍视频时随登场演出播放 */}
        <Field label={t.realm.introMusic}>
          <div className="flex min-w-0 items-center gap-2">
            <button
              type="button"
              onClick={() => pickAsset("introMusic", "introMusic")}
              className="shrink-0 rounded border border-slate-700 px-2.5 py-1 text-xs text-slate-300 hover:bg-slate-800/50"
            >
              {t.realm.pickIntroMusic}
            </button>
            <span className="min-w-0 flex-1 truncate text-xs text-muted">
              {curAssets.introMusic || t.realm.noIntroMusic}
            </span>
            {isFallbackAsset("introMusic") && (
              <span className="shrink-0 rounded bg-slate-700/60 px-1.5 py-0.5 text-[10px] text-slate-400">
                {t.realm.followDefault}
              </span>
            )}
            {curAssets.introMusic && (
              <button
                type="button"
                onClick={() => setCurAssets({ introMusic: "" })}
                className="shrink-0 text-xs text-red-400 hover:underline"
              >
                {t.realm.introClear}
              </button>
            )}
          </div>
          <p className="mt-1 text-xs text-muted">{t.realm.introMusicHint}</p>
        </Field>

        {/* 演出秒数（`intro.durationMs`，§5.4）：无介绍视频 / 音频时生效 */}
        <Field label={t.realm.introSeconds}>
          <input
            type="number"
            min={0}
            value={introSeconds}
            onChange={(e) => setIntroSeconds(Number(e.target.value) || 0)}
            className="w-full rounded-lg border border-slate-700 bg-slate-900 px-3 py-2 text-sm text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-blue-500/50"
          />
          <p className="mt-1 text-xs text-muted">{t.realm.introSecondsHint}</p>
        </Field>

        <label className="flex cursor-pointer items-center gap-2 text-sm text-foreground">
          <input
            type="checkbox"
            checked={introSkippable}
            onChange={(e) => setIntroSkippable(e.target.checked)}
            className="h-4 w-4 accent-blue-500"
          />
          {t.realm.introSkippable}
        </label>
      </div>
    </>
  );
}
