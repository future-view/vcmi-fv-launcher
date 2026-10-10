import { getIntroState } from "@/lib/tauri";
import type { Realm } from "./types";

/** 24 小时（毫秒）——§9.1 的登场演出重复播放间隔 */
export const INTRO_REPLAY_INTERVAL_MS = 24 * 3600 * 1000;

/**
 * 领域多语言字段取值（设计 §6：顶层为默认语言，`i18n.<lang>` 按启动器语言覆盖）。
 *
 * 回退链：当前界面语言的 `i18n` 覆盖 → 顶层值（= `defaultLang`）→ chinese → english → 任意非空。
 *
 * @param base 顶层字段值（默认语言），如 `realm.name` / `realm.description` / `realm.assets.introText`
 * @param i18n 各语言覆盖，如 `realm.i18n?.chinese?.name`
 */
export function pickRealmText(opts: {
  base?: string;
  i18n?: Record<string, string | undefined>;
  defaultLang?: string;
  uiLang: string;
}): string {
  const { base = "", i18n, defaultLang, uiLang } = opts;
  const pick = (lang?: string) => (lang && i18n?.[lang]) || "";
  return (
    pick(uiLang) ||
    base ||
    pick(defaultLang) ||
    pick("chinese") ||
    pick("english") ||
    (Object.values(i18n ?? {}).find((v) => !!v) ?? "")
  );
}

/** 领域名称（多语言回退） */
export function realmName(r: Realm | undefined | null, uiLang: string): string {
  if (!r) return "";
  return (
    pickRealmText({
      base: r.name,
      i18n: mapI18n(r, (e) => e.name),
      defaultLang: r.defaultLang,
      uiLang,
    }) || r.id
  );
}

/** 领域描述（多语言回退） */
export function realmDescription(r: Realm | undefined | null, uiLang: string): string {
  if (!r) return "";
  return pickRealmText({
    base: r.description,
    i18n: mapI18n(r, (e) => e.description),
    defaultLang: r.defaultLang,
    uiLang,
  });
}

/**
 * 领域标签（多语言回退，**数组语义**）。
 *
 * 与 `pickRealmText` 同款回退链（界面语言 i18n 覆盖 → 顶层 → defaultLang → chinese → english），
 * 但「空数组」是有效的**显式声明**（该语言确实无标签、不再落回），只有**键不存在**才继续回退
 * —— 故不能用 `||` 击穿，逐级用 `??` 判定。`i18n.<lang>.tags` 与顶层 `tags` 的键存在
 * 语义见 realm.json 规范（设计 §6）。
 */
export function realmTags(r: Realm | undefined | null, uiLang: string): string[] {
  if (!r) return [];
  const fromI18n = (lang?: string): string[] | undefined => {
    const t = lang ? r.i18n?.[lang]?.tags : undefined;
    return Array.isArray(t) ? t : undefined;
  };
  return (
    fromI18n(uiLang) ??
    (Array.isArray(r.tags) ? r.tags : undefined) ??
    fromI18n(r.defaultLang) ??
    fromI18n("chinese") ??
    fromI18n("english") ??
    []
  );
}

/** 筛选栏最多展示的标签数（按领域计数取前 N）。领域越来越多时标签随之膨胀，
 *  全量 chips 在俄 / 德 / 波等长标签语言下会挤爆一行（设计 §9.4）。 */
export const MAX_FILTER_TAGS = 6;

/**
 * 标签筛选的显示集合：按领域计数取前 `limit` 个（降序，并列按字母序，返回 `{ tag, n }`）。
 *
 * `selected` 中的标签**始终包含**（补显在末尾，数据里已不存在时计数为 0）：
 * 否则已选标签掉出前 N 后 chip 消失、筛选仍生效却无法取消。
 */
export function topRealmTags(
  tagLists: string[][],
  selected: string[],
  limit = MAX_FILTER_TAGS,
): { tag: string; n: number }[] {
  const m = new Map<string, number>();
  for (const list of tagLists) for (const x of list) m.set(x, (m.get(x) ?? 0) + 1);
  const top = [...m.entries()]
    .sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]))
    .slice(0, limit)
    .map(([tag, n]) => ({ tag, n }));
  const shown = new Set(top.map((x) => x.tag));
  for (const s of selected) {
    if (!shown.has(s)) {
      shown.add(s);
      top.push({ tag: s, n: m.get(s) ?? 0 });
    }
  }
  return top;
}

/** 登场介绍文本（按当前界面语言；默认语言也允许放在 `i18n[defaultLang]`，顶层 `assets.introText` 仅兜底）
 *
 * 介绍文本是长文本，跨语言回退会在英文 UI 下出现大段中文等违和场景，
 * 因此只取「当前界面语言」的 `i18n.<uiLang>.assets.introText`，缺失时回退顶层 `assets.introText`
 * （即默认语言），再缺失则返回空、由界面隐藏文本框。不再要求默认语言必须放在顶层。
 */
export function realmIntroText(r: Realm | undefined | null, uiLang: string): string {
  if (!r) return "";
  const t = mapI18n(r, (e) => e.assets?.introText)[uiLang] ?? r.assets?.introText ?? "";
  return t.trim();
}

/** 领域素材路径（多语言回退：`i18n.<lang>.assets` 可覆盖封面/视频/音乐，实现分语言素材） */
export function realmAsset(
  r: Realm | undefined | null,
  kind: "wallpaper" | "cover" | "music" | "introMusic" | "introVideo",
  uiLang: string,
): string {
  if (!r) return "";
  return pickRealmText({
    base: r.assets?.[kind],
    i18n: mapI18n(r, (e) => e.assets?.[kind]),
    defaultLang: r.defaultLang,
    uiLang,
  });
}

/** 把 `realm.i18n` 映射成 `Record<lang, 某字段>` 供回退链使用。 */
function mapI18n(
  r: Realm,
  get: (e: NonNullable<Realm["i18n"]>[string]) => string | undefined,
): Record<string, string | undefined> {
  const out: Record<string, string | undefined> = {};
  for (const [lang, entry] of Object.entries(r.i18n ?? {})) {
    out[lang] = get(entry);
  }
  return out;
}

/**
 * 领域是否存在「有内容」的登场演出（§9.1 / §9.6.1 核对点 3）。
 * 是否登场由是否配置素材决定：有介绍视频、或介绍音频、或壁纸图、或任意语言有介绍文本即视为有演出。
 */
export function hasIntroContent(r: Realm | undefined | null): boolean {
  const assets = r?.assets;
  if (!assets) return false;
  if (assets.introVideo) return true;
  if (assets.introMusic) return true;
  if (assets.wallpaper) return true;
  const anyText =
    (assets.introText ?? "").trim().length > 0 ||
    Object.values(r?.i18n ?? {}).some((e) => (e.assets?.introText ?? "").trim().length > 0);
  return anyText;
}

/**
 * 领域标题（§6 `showGameName`）：开启时标题带游戏名前缀，如「魔法门之英雄无敌Ⅲ：光明陨落」。
 *
 * 游戏名与连接模板均由 i18n 提供（**按启动器界面语言取**：中文界面用全角冒号，英文用半角），
 * 故此处只做占位符替换，不内置任何硬编码游戏名。
 */
export function realmTitle(opts: {
  r: Realm | undefined | null;
  uiLang: string;
  /** 游戏名，如「魔法门之英雄无敌Ⅲ」 */
  gameName: string;
  /** 标题模板，如 `{game}：{name}` */
  template: string;
}): string {
  const name = realmName(opts.r, opts.uiLang);
  if (!opts.r?.showGameName || !name) return name;
  return opts.template.replace("{game}", opts.gameName).replace("{name}", name);
}

/**
 * 按 §9.1 的播放策略判定本次进入领域是否播放登场演出：
 * 有内容且距上次播放超过 24 小时 → 播放；24 小时内已播过 → 不播（提供重播入口）。
 */
export async function shouldPlayIntro(r: Realm | undefined | null): Promise<boolean> {
  if (!hasIntroContent(r)) return false;
  try {
    const st = await getIntroState(r!.id);
    if (st.playedAt == null) return true;
    return Date.now() - st.playedAt * 1000 > INTRO_REPLAY_INTERVAL_MS;
  } catch {
    return true; // 读取失败按「未播放过」处理，宁可播放
  }
}
