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
  | "glossary"
  | "mods";

export interface WikiEntry {
  /** 类别 */
  category: WikiCategory;
  /** 条目 id（生物/英雄/宝物/法术/技能 id，或 factionId/modId） */
  id: string;
  /** 来源 mod id */
  modId: string;
  /** 多语言名：VCMI 语言键 → 文本（chinese / tchinese / english / …），**不限于中英** */
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
    /** VCMI modId：磁盘目录名**小写**归一（与 VCMI 主键同形态，见 `modIdKey`） */
    modId: string;
    /** 磁盘目录原名（保留大小写，可能含空格）——磁盘定位用它，不要用 `modId` */
    diskName?: string;
    /**
     * 模组磁盘位置的**来源根标识**：`game`（游戏目录 `Mods/`）/ `user`（用户数据 `Mods/`）
     * / `realm`（领域 `vcmi/Mods`）/ `abs`（未命中已知根，此时 `modPathRel` 是绝对路径）。
     *
     * 与 `modPathRel` 配对使用：按当前 `launcher.json` 解出对应根的绝对路径，拼上 `modPathRel`
     * 才是最终路径。**刻意不存绝对路径** —— 缓存目录常被整体复制复用（换机器/换路径），
     * 绝对路径随即失效；而"相对 `launcher.json`"也不成立（它与游戏目录可能跨盘符，
     * Windows 无法表示跨盘相对路径）。详见后端 `modpath_root_rel`。
     */
    modPathRoot?: string;
    /** 相对 `modPathRoot` 的路径（`/` 分隔，跨平台可移植） */
    modPathRel?: string;
    /** 缓存相对目录 `mods/<sanitize(modId)>`（后端 `index.rs` 写入） */
    cacheDir?: string;
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
 * 的 `to_lower_copy`；`getModDescription` 甚至 `assert` 入参已小写）。
 *
 * 后端 `norm_mod_id` 现已按同一口径归一，正常情况下入参**已经是小写**，本函数是幂等的。
 * 保留它只为兜底两类非当前口径的来源：
 * ① 旧版本生成的缓存索引 / 外部 mock（`modId` 仍是磁盘原名如 `Fairytale`）；
 * ② 手写的 `modSettings.json`（大小写不受控）。
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

/**
 * 词汇表专用取词：当前语言 → 英文 → auto → 中文 → 任意可用语言。
 *
 * 其它类目坚持「当前语言 → 英文」（与 VCMI 一致，见 [`pickName`]），但 glossary
 * 是跨 mod 的概念词条，实测常见形态是**只提供 `chinese.json`**（如 Expansion）。
 * 严格按 VCMI 口径时英文界面只剩翻译 key / 实体 id，用户看到的是
 * `vcmi.wiki.glossary.proverb.name`。词条没有实体标识语义，显示任一可用语言
 * 远好于显示 key；这不是给缺中文的实体伪造语言键。
 */
export function pickNameAny(name: Record<string, string> | undefined, lang: string): string {
  if (!name) return "";
  const l = contentLangKey(lang);
  for (const k of [l, "english", "auto", "chinese"]) {
    const v = name[k];
    if (v && v.trim().length > 0) return v;
  }
  for (const v of Object.values(name)) {
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
