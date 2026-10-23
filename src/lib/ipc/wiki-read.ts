/**
 * Wiki 缓存读取侧（cache.json / index.json / 展示卡 / 封面音频）。
 *
 * 拆分自原 `@/lib/tauri.ts` 单体；对外仍由该 barrel 统一导出，调用方无需改动。
 */
import { invoke } from "@tauri-apps/api/core";
import { isTauri, mockCacheBase } from "./core";
import { InstalledMod } from "./realm-export";

async function fetchMockCacheJson(rel: string): Promise<WikiReadResult | null> {
  try {
    const r = await fetch(`${mockCacheBase()}/${rel}`, { cache: "no-cache" });
    if (!r.ok) return null;
    const data = await r.json();
    return { ok: true, data } as WikiReadResult;
  } catch {
    return null;
  }
}

/* ----------------------------- Wiki 数据源（mods/{modId}/cache.json + index.json） ----------------------------- */

/** 读取单个 mod 的 Wiki 缓存（命中缓存时不再解析原始 mod） */
export async function readModWikiCache(modId: string): Promise<WikiReadResult | null> {
  // 浏览器自测模式：直接读 mock 缓存目录的 cache.json
  if (!isTauri()) return fetchMockCacheJson(`${modId}/cache.json`);
  try {
    // 后端返回 **JSON 原文**（不在 Rust 侧解析）：省掉一次整份解析（1.7 MB ≈ 33 ms），
    // 且后端可缓存原文、命令层只传字符串。`JSON.parse` 本来就不可避免。
    const text = await invoke<string>("read_mod_cache", { modId });
    if (!text) return null;
    return { ok: true, data: JSON.parse(text) as Record<string, unknown> } as WikiReadResult;
  } catch {
    return null;
  }
}

/** 城镇预览合成（后台按需申请）：叠加 structures 到 townScreen 背景，结果缓存并写回 preview */
export async function generateTownPreview(
  modId: string,
  factionId: string
): Promise<{ ok: boolean; cached?: boolean; preview?: string; error?: string } | null> {
  if (!isTauri()) return null;
  try {
    return await invoke("generate_town_preview", { modId, factionId });
  } catch {
    return null;
  }
}

/** 批量构建所有 Town mod 的 Wiki 缓存并生成 mods/index.json 索引 */
export async function buildAllWikiCache(mods: unknown[]): Promise<WikiBuildAllResult | null> {
  if (!isTauri()) return null;
  try {
    return await invoke<WikiBuildAllResult>("build_all_mod_cache", { mods });
  } catch {
    return null;
  }
}

/** 读取 mods/index.json 索引 */
export async function readWikiIndex(): Promise<WikiReadResult | null> {
  // 浏览器自测模式：读 mock 缓存目录的 index.json
  if (!isTauri()) return fetchMockCacheJson("index.json");
  try {
    return await invoke<WikiReadResult>("read_cache_index");
  } catch {
    return null;
  }
}

/** 城镇卡片（领域信息页专用，后端只回传必要字段，避免读整份 cache.json） */
export interface TownCard {
  modId: string;
  factionId: string;
  /** 多语言名称 {chinese, english, ...} */
  name?: Record<string, string> | null;
  /** 多语言描述，或纯字符串 */
  description?: Record<string, string> | string | null;
  /** 合成预览图的相对路径（images/...，铺满全部建筑），可能为空 */
  preview?: string | null;
  /** 城镇底图相对路径（images/...）。原版 `core.<faction>` 只有它（合成图需 raw towns，原版没有） */
  background?: string | null;
}

/**
 * 批量读取**城镇卡片**最小字段。
 *
 * 替代"逐个 `read_mod_cache` 读整份 cache.json"：本例 86 个城镇合计 31 MB，
 * 走本命令只有数十 KB，领域信息页因此不必再做视口懒加载。
 */
export async function readTownCards(ids: string[]): Promise<TownCard[]> {
  if (!isTauri() || ids.length === 0) return [];
  try {
    const r = await invoke<{ cards?: TownCard[] }>("read_town_cards", { ids });
    return r?.cards ?? [];
  } catch {
    return [];
  }
}

/** 阵营巡礼单 slide 载荷（裁剪后的 cache 数据，设计 §8.1） */
export interface ShowcaseSlide {
  modId: string;
  factionId: string;
  name?: Record<string, string>;
  description?: Record<string, string>;
  /** 幕布：preview ?? background（相对缓存根，走 fullImgUrl） */
  backdrop?: string;
  /** 阵营音乐：磁盘已有 → 绝对路径；否则 null */
  music?: string | null;
  /** zip 内音乐描述（前端调 ensureShowcaseAudio 异步抽取）；无则 null */
  musicZip?: { modId: string; rel: string } | null;
  /** 城镇结构幕布坐标 + 金色描边图（§5.9），用于高亮罩住整栋建筑 */
  structures: { id: string; x: number; y: number; z: number; w: number; h: number; image: string }[];
  /** `upgradedFrom` = 升级来源（空数组/undefined = 基础形态），前端据此区分未升级 / 仅进阶 */
  creatures: {
    id: string;
    name?: Record<string, string>;
    /** 描述（多语言，与 `name` 同构）。取不到时省略 —— 前端显示「暂无描述」，**不编造** */
    description?: Record<string, string>;
    /**
     * 该生物的**巢穴建筑名**（多语言 → 名称数组）。
     * 来源：城镇配置里 `building.creatures` 的反查（`town.rs` 的 `creature_dwellings`）。
     * 前端据此做「招募建筑」的**权威匹配**（按名称比对建筑清单），
     * 比"按 level 猜 id"稳 —— 猜 id 在第三方模组上常落空，表现就是"没有招募建筑"。
     */
    dwellingNames?: Record<string, string[]>;
    level?: number;
    image?: string;
    imageLarge?: string;
    upgradedFrom?: string[];
  }[];
  creatureTotal: number;
  /** `className` 在缓存里是 VCMI 多语言对象（与 name 同构），个别模组也可能是纯字符串 */
  heroes: {
    id: string;
    name?: Record<string, string>;
    /** 描述（多语言）；原版英雄为特长说明（`specialty`） */
    description?: Record<string, string>;
    className?: Record<string, string> | string;
    image?: string;
  }[];
  buildings: {
    id: string;
    name?: Record<string, string>;
    /** 描述（多语言） */
    description?: Record<string, string>;
    image?: string;
  }[];
  /** 后端解析到的 VCMI AudioPath（诊断用；null = 该阵营配置里没有 musicTheme） */
  musicTheme?: string | null;
  /** 数据缺失原因（缓存未构建 / faction 不存在） */
  missing?: string;
}

/** 阵营巡礼请求项（前端带入 modPath 用于解析音乐，设计 §8.1） */
export interface ShowcaseEntry {
  modId: string;
  factionId: string;
  modPath?: string;
}

/** 读取阵营巡礼 slide 载荷（批量裁剪，避免读整份 cache.json） */
export async function readShowcaseSlides(entries: ShowcaseEntry[]): Promise<ShowcaseSlide[]> {
  if (!isTauri() || entries.length === 0) return [];
  try {
    const r = await invoke<{ slides?: ShowcaseSlide[] }>("read_showcase_slides", { entries });
    return r?.slides ?? [];
  } catch {
    return [];
  }
}

/** zip 内音乐单条目抽取落盘，返回绝对路径（null = 跳过/失败） */
export async function ensureShowcaseAudio(
  modPath: string,
  modId: string,
  rel: string,
): Promise<string | null> {
  if (!isTauri()) return null;
  try {
    const r = await invoke<{ ok: boolean; path?: string | null }>("ensure_showcase_audio", {
      modPath,
      modId,
      rel,
    });
    return r?.path ?? null;
  } catch {
    return null;
  }
}

/**
 * 批量读取**模组元数据**（`cache.json.modMeta`：多语言名/描述 + 作者/版本/类型/依赖）。
 *
 * 替代 `listInstalledMods()` 里"每个模组各读一次整份 cache.json"的开销
 * （实测 652 个 mod ≈ 126 MB）。只传真正需要的 id 列表。
 */
export async function readModMetas(
  ids: string[],
): Promise<Record<string, InstalledMod["meta"]>> {
  if (!isTauri() || ids.length === 0) return {};
  try {
    const r = await invoke<{ metas?: Record<string, InstalledMod["meta"]> }>("read_mod_metas", {
      ids,
    });
    return r?.metas ?? {};
  } catch {
    return {};
  }
}

/** 获取 mods 缓存根目录绝对路径（用于拼接 convertFileSrc 完整图片路径） */
let modsCacheDirPromise: Promise<string | null> | null = null;

export async function getModsCacheDir(): Promise<string | null> {
  if (!isTauri()) return null;
  // **模块级单次记忆**：后端每次都会 `create_dir_all` + 磁盘 IO，而列表页每张卡片都会调用
  // （数百张卡片 = 数百次 IPC + 建目录）。目录由游戏 / 用户目录推导，会话内不会变，可安全记忆。
  if (!modsCacheDirPromise) {
    modsCacheDirPromise = invoke<{ ok: boolean; cacheDir?: string }>("get_mods_cache_dir")
      .then((res) => res?.cacheDir ?? null)
      .catch(() => {
        modsCacheDirPromise = null; // 失败不缓存，下次调用可重试
        return null;
      });
  }
  return modsCacheDirPromise;
}

export interface WikiReadResult {
  ok: boolean;
  data?: unknown;
  error?: string;
}

export interface WikiBuildAllResult {
  ok: boolean;
  built?: number;
  cached?: number;
  skipped?: number;
  encrypted?: number;
  total?: number;
  indexPath?: string;
  totalIndexed?: number;
}
