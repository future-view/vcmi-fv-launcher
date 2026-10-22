import { useCallback, useEffect, useRef, useState } from "react";
import { SkipForward } from "lucide-react";
import { useI18n } from "@/i18n";
import { markIntroPlayed, realmAssetUrl, updateRealm } from "@/lib/tauri";
import { refreshRealms } from "@/lib/realm/store";
import {
  realmAsset,
  realmIntroText,
  realmTitle,
} from "@/lib/realm/intro";
import type { Realm } from "@/lib/realm/types";

interface Props {
  realm: Realm;
  /** 领域目录绝对路径，用于把 `assets/...` 解析为可播放 URL（§5.4） */
  root: string;
  onFinish: () => void;
  /** 预览模式（B5）：仅展示，不记录播放时间、不产生持久改动 */
  preview?: boolean;
}

/** 出场时长（毫秒）：无视频时生效；为 0 时按滚动距离与 60px/s 反推（§5.5）。 */
function scrollDuration(contentH: number, areaH: number, durationMs?: number): number {
  if (durationMs && durationMs > 0) return durationMs;
  return ((contentH + areaH) / 60) * 1000;
}

/**
 * 登场演出（Intro Sequence）全屏覆盖层（§5.4 / §5.5）。
 *
 * - 覆盖 `TopBar`(z-50) / `BottomBar`(z-40)，故用 z-[130]；演出生效期间不显示窗口控件（§9.2.1）。
 * - 有介绍视频 → 铺满播放（muted autoplay playsinline），播完结束；无视频 → 封面图 + 文本滚动。
 * - 跳过：右下角按钮或 ESC（受 `intro.skipable` 约束）。
 */
export function RealmIntroOverlay({ realm, root, onFinish, preview }: Props) {
  const { t } = useI18n();
  const [phase, setPhase] = useState<"in" | "out">("in");
  const [narrow, setNarrow] = useState(false);
  const areaRef = useRef<HTMLDivElement | null>(null);
  const contentRef = useRef<HTMLDivElement | null>(null);
  const videoRef = useRef<HTMLVideoElement | null>(null);
  const audioRef = useRef<HTMLAudioElement | null>(null);
  /** 收尾幂等：视频 / 音频 / 滚动 / 兜底定时器都可能触发 finish，只放行第一次 */
  const finishedRef = useRef(false);

  const intro = realm.intro;
  const lang = t.uiLang;
  // 媒体素材**语言无关**（只存顶层 `assets`）；分语言的只有介绍文本（下取 `lang`）
  const videoSrc = realmAssetUrl(root, realmAsset(realm, "introVideo"));
  const coverSrc = realmAssetUrl(root, realmAsset(realm, "wallpaper"));
  const musicSrc = realmAssetUrl(root, realmAsset(realm, "introMusic"));
  const text = realmIntroText(realm, lang);
  // §6 `showGameName`：开启后标题带游戏名前缀，游戏名按界面语言取
  const name = realmTitle({
    r: realm,
    uiLang: lang,
    gameName: t.realm.gameName,
    template: t.realm.gameNameTitle,
  });
  const skippable = intro?.skipable !== false;
  const showName = intro?.showRealmName !== false;
  const hasVideo = !!videoSrc;
  // 介绍音频仅在「无介绍视频」时播放：有视频时以视频自带音轨为准
  const hasMusic = !!musicSrc && !hasVideo;
  /** 探测到的媒体实际时长（毫秒）：有视频 / 音频时以它为准，并写回 `intro.durationMs` */
  const [mediaMs, setMediaMs] = useState<number | null>(null);

  /** 淡出 200ms 后收尾（§5.5 转场）。 */
  const finish = useCallback(() => {
    if (finishedRef.current) return;
    finishedRef.current = true;
    setPhase("out");
    window.setTimeout(onFinish, 200);
  }, [onFinish]);

  // 记录播放时间（§9.1：24 小时内不再重复播放；「重播」也会刷新此戳）。
  // 预览模式（B5）下跳过，避免产生持久改动。
  useEffect(() => {
    if (!preview) markIntroPlayed(realm.id).catch(() => {});
  }, [realm.id, preview]);

  // 时长自动计算（§5.4）：探测到视频 / 音频的真实时长后写回 `intro.durationMs`，
  // 后续播放直接读取即可，不必重复探测。预览模式不写；只读领域写回失败也不影响演出。
  useEffect(() => {
    if (preview || mediaMs == null) return;
    const cur = intro?.durationMs ?? 0;
    if (Math.abs(cur - mediaMs) < 500) return; // 差异过小，不值得写
    updateRealm(realm.id, { intro: { durationMs: Math.round(mediaMs) } })
      .then(() => refreshRealms())
      .catch(() => {});
  }, [preview, mediaMs, realm.id, intro?.durationMs]);

  // 带声自动播放（§5.4）：视频 / 音频不再默认静音。
  // 若被 WebView 自动播放策略拦截：视频回退静音重试（画面不能丢），音频则直接跳过演出。
  useEffect(() => {
    const v = videoRef.current;
    if (!v) return;
    v.play().catch(() => {
      v.muted = true;
      v.play().catch(() => finish());
    });
  }, [videoSrc, finish]);

  useEffect(() => {
    const a = audioRef.current;
    if (!hasMusic || !a) return;
    a.play().catch(() => finish());
  }, [hasMusic, musicSrc, finish]);

  // 窄窗口（<720px）时介绍文本改为静态显示 + 滚动条（§5.5 最小可读宽度）
  useEffect(() => {
    const onResize = () => setNarrow(window.innerWidth < 720);
    onResize();
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);

  // 跳过：ESC（受 skippable 约束）
  useEffect(() => {
    if (!skippable) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") finish();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [skippable, finish]);

  // 文本滚动：仅在「无视频」且非窄窗口时启动（§5.2 / §5.5）
  useEffect(() => {
    if (hasVideo || narrow || !text) return;
    const area = areaRef.current;
    const content = contentRef.current;
    if (!area || !content) return;
    const areaH = area.clientHeight;
    const contentH = content.scrollHeight;
    // 时长优先级：媒体实际时长 > `intro.durationMs` > 按滚动距离推算（60px/s，§5.5）
    const duration = scrollDuration(contentH, areaH, mediaMs ?? intro?.durationMs);
    let raf = 0;
    let start = 0;
    const tick = (ts: number) => {
      if (!start) start = ts;
      const p = Math.min(1, (ts - start) / duration);
      content.style.transform = `translateY(${areaH - p * (contentH + areaH)}px)`;
      if (p < 1) raf = requestAnimationFrame(tick);
      else finish();
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [hasVideo, narrow, text, mediaMs, intro?.durationMs, finish]);

  // 既无视频、也无介绍音频、也无文本：直接进入（不留白屏）
  useEffect(() => {
    if (!hasVideo && !hasMusic && !text) finish();
  }, [hasVideo, hasMusic, text, finish]);

  return (
    <div
      className="fixed inset-0 z-[130] overflow-hidden bg-slate-950"
      style={{
        opacity: phase === "in" ? 1 : 0,
        transition: `opacity ${phase === "in" ? 300 : 200}ms ease-out`,
      }}
    >
      {/* 背景层：视频 > 壁纸图 > 默认渐变（§5.2）。视频带声播放，不再默认静音 */}
      {hasVideo ? (
        <video
          ref={videoRef}
          src={videoSrc}
          className="absolute inset-0 h-full w-full object-cover"
          autoPlay
          playsInline
          onEnded={finish}
          onError={finish}
          onLoadedMetadata={(e) => {
            const d = e.currentTarget.duration;
            if (Number.isFinite(d) && d > 0) setMediaMs(d * 1000);
          }}
        />
      ) : coverSrc ? (
        <img
          src={coverSrc}
          alt=""
          className="absolute inset-0 h-full w-full object-cover"
          /* 封面文件缺失时不显示 broken 图标：隐藏自身露出底层深色底
             （演出的视频/音频分支不受影响）。 */
          onError={(e) => {
            e.currentTarget.style.visibility = "hidden";
          }}
        />
      ) : (
        <div className="absolute inset-0 bg-gradient-to-br from-slate-900 via-indigo-950 to-slate-950" />
      )}

      {/* 介绍音频（§6 `assets.introMusic`）：无介绍视频时随演出播放，播完即结束 */}
      {hasMusic && (
        <audio
          ref={audioRef}
          src={musicSrc}
          autoPlay
          onEnded={finish}
          onError={finish}
          onLoadedMetadata={(e) => {
            const d = e.currentTarget.duration;
            if (Number.isFinite(d) && d > 0) setMediaMs(d * 1000);
          }}
          className="hidden"
        />
      )}

      {/* 领域名称：左上角距边 40px（§5.5）。登场演出不显示领域描述。
          标题自带紧贴文字宽度的圆角药丸蒙层，与领域主界面标题风格统一 */}
      <div className="absolute left-10 top-10 max-w-[70%]">
        {showName && name && (
          <h1
            className="inline-block max-w-full truncate rounded-xl bg-slate-900/80 px-4 py-1 font-bold text-white backdrop-blur-sm"
            style={{ fontSize: 48, textShadow: "0 2px 12px rgba(0,0,0,0.65)" }}
          >
            {name}
          </h1>
        )}
      </div>

      {/* 介绍文本：无视频时滚动（60px/s），宽度 ≤60%，底部距边 48px。
          整块滚动区加黑透底 + 白字，避免被壁纸 / 视频画面污染文本显示。
          字号（2026-09-22 二修，对标原版战役过场字幕的 3.3~3.7% 屏高占比）：
          `clamp(18px, 3.5vh, 34px)`。
          ⚠️ 首修用 2.9vh 是错的——实测窗口 ~1113×612（非最大化，zoom=1）下只有 17.7px，
          与旧 16px 仅差 1.7px，肉眼不可辨；且"最大化 16:9 视口高恒 ≈720"的假设不成立于小窗。
          3.5vh 在任意窗口高度下都直接对齐原版比例：612 高 → 21.4px、720 高 → 25.2px。
          18px 下限兜底极小窗；34px 上限防超高视口（如浏览器全屏 1080）失控。
          ⚠️ 为什么这里**可以**用 `vh` 而巡礼（ShowcaseOverlay）不行：
          本覆盖层是 `fixed inset-0` 的真视口元素，只有 `installUiScale` 的 zoom 一级缩放；
          巡礼是 1280×720 设计稿画布再套一层 `scale`，vh 与它不同源，故那边用画布内常量。 */}
      {!hasVideo && text && (
        <div
          ref={areaRef}
          className="absolute bottom-12 left-10 w-[60%] overflow-hidden rounded-xl bg-black/45 px-5 py-4 backdrop-blur-sm"
          style={{ maxHeight: "42%" }}
        >
          <div
            ref={contentRef}
            className="whitespace-pre-wrap text-slate-100"
            style={{
              fontSize: "clamp(18px, 3.5vh, 34px)",
              lineHeight: 1.8,
              textShadow: "0 1px 8px rgba(0,0,0,0.6)",
              // 窄窗口：静态显示 + 滚动条，不做自动滚动
              maxHeight: narrow ? "42vh" : undefined,
              overflowY: narrow ? "auto" : undefined,
              transform: narrow ? undefined : "translateY(100%)",
            }}
          >
            {text}
          </div>
        </div>
      )}

      {/* 跳过：右下角距边 32px（§5.5） */}
      {skippable && (
        <button
          type="button"
          onClick={finish}
          className="absolute bottom-8 right-8 flex items-center gap-1.5 rounded-lg border border-white/25 bg-black/35 px-3 py-1.5 text-sm text-white/90 backdrop-blur-sm hover:bg-black/55"
        >
          <SkipForward className="h-4 w-4" />
          {t.realm.introSkip}
        </button>
      )}
    </div>
  );
}
