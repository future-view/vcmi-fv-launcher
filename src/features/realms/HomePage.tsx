import { Play, Info, ArrowLeftRight, LogOut, Sparkles } from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import { hasIntroContent, realmIntroText, realmName, realmTitle } from "@/lib/realm/intro";
import type { Realm } from "@/lib/realm/types";
import { MenuButton } from "./MenuButton";

/**
 * 领域主界面（§9.2.2）：壁纸背景 + 领域名称 + 介绍文本（左侧贴底）+ 右侧定宽菜单。
 *
 * - 壁纸（`assets.wallpaper`）铺满整个界面，未配置时回退到覆盖层的默认渐变；
 *   标题、介绍文本、菜单按钮均自带局部暗色蒙层，保证白色文字在任意壁纸上可读。
 * - 介绍文本贴左下角（对齐 §9.2.2 原型的文本框位置），带半透明黑底蒙层。
 * - 背景音乐（`assets.music`）由覆盖层根部常驻播放（切到领域信息等二级页不中断），
 *   `main.loopMusic` 控制是否循环（默认循环）。
 */
export function HomePage({
  realm,
  root,
  phase,
  preview,
  onPlay,
  onInfo,
  onSwitch,
  onExit,
  onReplay,
  bgmMuted,
  onToggleBgm,
  onShowcase,
}: {
  realm: Realm;
  /** 领域目录：用于解析 `assets/...`（§5.4） */
  root: string;
  phase: "checking" | "launching" | null;
  preview?: boolean;
  onPlay: () => void;
  onInfo: () => void;
  onSwitch: () => void;
  onExit: () => void;
  onReplay: (r: Realm) => void;
  /** 当前领域是否已静音背景音乐 */
  bgmMuted: boolean;
  /** 切换背景音乐开关（仅对当前领域有效） */
  onToggleBgm: () => void;
  /** 进入阵营巡礼（独立页模式 A） */
  onShowcase?: () => void;
}) {
  const { t } = useI18n();
  const lang = t.uiLang;
  const name = realmName(realm, lang);
  // §6 `showGameName`：开启后标题带游戏名前缀，游戏名按界面语言取
  const title = realmTitle({
    r: realm,
    uiLang: lang,
    gameName: t.realm.gameName,
    template: t.realm.gameNameTitle,
  });
  // §6 `showRealmName`：关闭时不显示领域名称（主界面与登场演出一致）
  const showName = realm.intro?.showRealmName !== false;
  const introText = realmIntroText(realm, lang);
  // §6 `main.maskOpacity`：主界面文字蒙层透明度（默认 80）
  const maskOpacity = realm.main?.maskOpacity ?? 80;

  return (
    <div className="relative flex h-full flex-col">
      {/* 壁纸与背景音乐均由覆盖层根部（RealmShell）渲染，此处只负责主界面内容 */}

      <div className="relative z-10 flex min-h-0 flex-1 flex-col">
        {showName && title && (
          <div className="min-w-0 px-8 pt-4">
            <h1
              className="inline-block max-w-full truncate rounded-xl px-4 py-1 text-[48px] font-bold leading-tight text-white drop-shadow-[0_2px_10px_rgba(0,0,0,0.85)] backdrop-blur-sm"
              style={{ backgroundColor: `rgba(15,23,42,${maskOpacity / 100})` }}
              title={name}
            >
              {title}
            </h1>
          </div>
        )}

        {/* 介绍文本贴左下角；右侧菜单自顶部排布（§9.2.2 原型） */}
        <div className="flex min-h-0 flex-1 items-stretch justify-between gap-10 px-8 pb-10 pt-6">
          <div className="flex min-h-0 flex-1 flex-col justify-end">
            {introText ? (
              <div className="max-h-[55vh] w-[92%] max-w-[560px] overflow-y-auto rounded-xl p-5 text-[15px] leading-relaxed text-white backdrop-blur-sm" style={{ backgroundColor: `rgba(15,23,42,${maskOpacity / 100})` }}>
                <div className="whitespace-pre-wrap">{introText}</div>
              </div>
            ) : null}
          </div>

          <div className="flex w-[240px] flex-none flex-col gap-3">
            <MenuButton
              icon={<Play className="h-4 w-4" />}
              label={phase === "checking" ? t.realm.checking : phase === "launching" ? t.realm.launching : t.realm.playGame}
              onClick={onPlay}
              disabled={phase !== null}
              bgOpacity={maskOpacity}
            />
            <MenuButton icon={<Info className="h-4 w-4" />} label={t.realm.realmInfo} onClick={onInfo} bgOpacity={maskOpacity} />
            {/* 「阵营巡礼」紧跟「领域信息」：两者都是"看这个领域有什么"，
                排在「切换领域」这种"离开当前领域"的动作之前更符合预期 */}
            {onShowcase && (
              <MenuButton icon={<Sparkles className="h-4 w-4" />} label={t.realm.showcase} onClick={onShowcase} bgOpacity={maskOpacity} />
            )}
            <MenuButton icon={<ArrowLeftRight className="h-4 w-4" />} label={t.realm.switchRealm} onClick={onSwitch} bgOpacity={maskOpacity} />
            <MenuButton icon={<LogOut className="h-4 w-4" />} label={t.realm.exitRealm} onClick={onExit} bgOpacity={maskOpacity} />
          </div>
        </div>
      </div>

      {!preview && (
        <div className="absolute bottom-5 right-7 z-20 flex items-center gap-2">
          {hasIntroContent(realm) && (
            <button
              type="button"
              onClick={() => onReplay(realm)}
              className="rounded-lg border border-slate-700/70 bg-slate-900/80 px-3 py-1.5 text-xs text-slate-300 backdrop-blur-sm hover:bg-slate-800/70"
            >
              {t.realm.introReplay}
            </button>
          )}
          <button
            type="button"
            onClick={onToggleBgm}
            className="flex items-center gap-2 rounded-lg border border-slate-700/70 bg-slate-900/80 px-3 py-1.5 text-xs text-slate-300 backdrop-blur-sm hover:bg-slate-800/70"
            title={bgmMuted ? t.realm.bgmOn : t.realm.bgmOff}
          >
            <span
              className={cn(
                "h-3.5 w-6 rounded-full p-0.5 transition-colors",
                bgmMuted ? "bg-slate-600" : "bg-emerald-500",
              )}
            >
              <span
                className={cn(
                  "block h-2.5 w-2.5 rounded-full bg-white transition-transform",
                  bgmMuted ? "" : "translate-x-2.5",
                )}
              />
            </span>
            {bgmMuted ? t.realm.bgmOn : t.realm.bgmOff}
          </button>
        </div>
      )}
    </div>
  );
}
