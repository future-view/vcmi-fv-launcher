/**
 * Wiki 数据模型与加载。
 * 仅从 `.vcmi-fv-launcher/mods/{modId}/cache.json` + `mods/index.json` 读取，不解析原始 mod。
 */
import { convertFileSrc } from "@tauri-apps/api/core";
import { imgCandidates } from "@/lib/hd";
import { isTauri, readModWikiCache, readWikiIndex } from "@/lib/tauri";

export type WikiCategory =
  | "factions"
  | "buildings"
  | "creatures"
  | "heroes"
  | "artifacts"
  | "spells"
  | "skills"
  | "terrains"
  | "mods";

export interface WikiEntry {
  /** 类别 */
  category: WikiCategory;
  /** 条目 id（生物/英雄/宝物/法术/技能 id，或 factionId/modId） */
  id: string;
  /** 来源 mod id */
  modId: string;
  /** 多语言名 {zh-CN, en} */
  name: Record<string, string>;
  /** 图标相对路径（images/...），需 convertFileSrc 转换 */
  image?: string;
  /** 原始配置对象（完整字段） */
  data: Record<string, unknown>;
}

/** 联动跳转目标（用于 wiki 条目间的关联跳转） */
export interface WikiLinkTarget {
  modId: string;
  category: WikiCategory;
  id: string;
}

/** 把单个 mod 的某类目对象转为 WikiEntry（列表与联动跳转复用） */
export function buildEntry(
  mod: Record<string, unknown>,
  cat: WikiCategory,
  obj: Record<string, unknown>,
): WikiEntry {
  return {
    category: cat,
    id: String(obj.id ?? obj.factionId ?? ""),
    modId: String(mod.modId ?? ""),
    name: (obj.name as Record<string, string>) ?? {},
    image:
      (obj.image as string) ??
      (cat === "factions" ? (obj.background as string) : undefined),
    data: obj,
  };
}

export interface WikiIndex {
  generatedAt?: number;
  version?: number;
  mods: {
    modId: string;
    diskName?: string;
    /** 顶层父 mod id（子模组指向父；顶层为自身），由后端 `batch.rs` 写入 */
    parentId?: string;
    type?: string;
    /** 多语言模组名 {chinese, english, ...}（后端从 cache.json 的 modMeta.name 提取；旧缓存/外部 mock 可能为单串，前端已兼容） */
    name?: Record<string, string> | string;
    encrypted?: boolean;
    /** 原版 H3 数据文件缺失：百科图标/背景将无法提取（提示用户补放 Data/*.lod） */
    h3DataMissing?: boolean;
    contentCount?: Record<string, number>;
  }[];
}

/**
 * modId 的**比较键**（小写）。
 *
 * VCMI 的规范 modId 是「磁盘目录名小写、点号分层」（见 `references/vcmi/lib/modding/ModManager.cpp`
 * 的 `to_lower_copy`；`getModDescription` 甚至 `assert` 入参已小写）。因此 `modSettings.json`
 * 里写的是 `fairytale`，而缓存索引沿用磁盘原名 `Fairytale` —— 两侧**必须按小写比较**才能命中。
 *
 * 只用于查表比较；传给后端 / 拼路径时仍用原始 id（后端有大小写无关解析）。
 */
export function modIdKey(id: string): string {
  return (id ?? "").toLowerCase();
}

/** 内容语言 key 映射：UI lang (chinese/english) → VCMI mod 多语言 key。
 * 与 VCMI 的 mod.json 多语言能力对齐（chinese/english/tchinese/...），不使用自定义 zh-CN/en。 */
export function contentLangKey(uiLang: string): string {
  const l = (uiLang || "").toLowerCase();
  if (l === "chinese" || l === "zh-cn" || l === "zh") return "chinese";
  if (l === "tchinese" || l === "zh-tw" || l === "zh-hant") return "tchinese";
  return "english";
}

/** 从多语言字典取文本，带回退。
 * name 的 key 为 VCMI 语言 key（chinese/english/tchinese/.../auto）。
 *
 * 回退顺序严格对齐 VCMI：**当前语言 → 英文（VCMI 翻译基准语言）**。
 * 不做「任意语言」兜底，也不在英文界面回退到中文——否则会出现捷克语残留、
 * 或英文界面显示中文等与 VCMI 不一致的表现。 */
export function pickName(name: Record<string, string> | undefined, lang: string): string {
  if (!name) return "";
  const l = contentLangKey(lang);
  for (const k of [l, "english"]) {
    const v = name[k];
    if (v && v.trim().length > 0) return v;
  }
  return "";
}

/** 读取 wiki 索引 */
export async function loadWikiIndex(): Promise<WikiIndex | null> {
  const res = await readWikiIndex();
  if (!res?.ok || !res.data) return null;
  return res.data as WikiIndex;
}

/** 读取单个 mod 的缓存数据，返回各类型条目 */
export async function loadModCache(modId: string): Promise<Record<string, unknown> | null> {
  const res = await readModWikiCache(modId);
  if (!res?.ok || !res.data) return null;
  return res.data as Record<string, unknown>;
}

/** 把 images/ 相对路径拼成缓存根目录下的绝对路径，供 convertFileSrc 使用。
 * 浏览器自测模式（非 Tauri）下改走 vite mock-cache 中间件的 URL（rel 已含 <modId>/ 前缀）。 */
export function fullImgUrl(cacheDir: string | null, rel?: string): string {
  if (!rel) return "";
  const norm = rel.replace(/\\/g, "/");
  if (!isTauri()) return `/mock-cache/mods/${norm}`;
  if (!cacheDir) return rel;
  return convertFileSrc(`${cacheDir}/${norm}`);
}

/**
 * **高清优先**的候选 URL 列表（`[高清, 原图]`），供 `HdImg` 逐个尝试。
 *
 * 高清变体（`@Nx.webp`）是素材阶段的**可选产物**（倍率由显示器推导，见 `lib/hd.ts`）：
 * - `rate < 2`（低分屏）→ 只返回原图；
 * - 该高清变体已被记为缺失 → 只返回原图（不重复 404）；
 * - 原素材**始终存在**，故列表末项一定是可用的兜底。
 */
export function fullImgCandidates(
  cacheDir: string | null,
  rel: string | undefined,
  rate: number,
): string[] {
  const base = fullImgUrl(cacheDir, rel);
  if (!base) return [];
  return imgCandidates(base, rate);
}
