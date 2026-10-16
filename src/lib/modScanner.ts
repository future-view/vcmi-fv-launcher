import { classifyMod } from "./minimapBase";

/**
 * 模组信息结构（纯类型与本地化取值，浏览器/Tauri 前端可用，无 Node 依赖）。
 * 磁盘读取、zip 解包、编码解码等 Node 专属逻辑见 modScannerNode.ts。
 */
export interface ModInfo {
  /** VCMI modId：磁盘目录名**小写**归一，子 mod 用 `parent.child`（与 VCMI 主键同一形态） */
  id: string;
  /** 磁盘目录原名（保留大小写；可能含空格）——磁盘定位用它，不要用 `id` */
  diskName: string;
  parentId: string; // 顶层父 mod id（点号前第一段 或 自身）
  name: string; // 英文名
  localizedName: string; // 按语言取词后的名字
  desc: string;
  /** 分类结果（可能是按 id/名称/描述**推断**出来的，见后端 classify_mod） */
  type: string;
  /**
   * `mod.json` 的 `modType` **原文**（未推断；未声明时为 null）。
   * 需要"与引擎同一判定口径"的逻辑（如翻译模组语言不符）必须用本字段，而不是 `type`。
   */
  rawType?: string | null;
  version?: string;
  author?: string;
  depends: string[];
  softDepends: string[];
  conflicts: string[];
  /**
   * 引擎兼容区间（`mod.json` 的 `compatibility`）：`{ min, max }`，缺省 / 空串 = 未声明。
   * 判定见 `lib/modCompat.ts`（复刻 VCMI `CModVersion::compatible`）。
   */
  compatibility?: { min?: string | null; max?: string | null } | null;
  /**
   * 该 mod 的**磁盘目录绝对路径**（后端扫描回传）。
   *
   * 这是唯一可靠的磁盘定位依据：`diskName` 只能表达"名字"，表达不了**来源根**
   * （游戏目录 `Mods/` 与 userDataPath 下的 `Mods/` 会同时挂载，可能有同名模组），
   * 子模组的磁盘布局（`<父>/Mods/<子>`）也无法从 `id`/`diskName` 推出。
   */
  modPath: string;
  hasContentZip: boolean;
  contentZipEncrypted: boolean;
  isSubmod: boolean;
  subs: ModInfo[]; // 子 mod（仅顶层 mod 携带）
  /** 子模组默认是否保持禁用（VCMI mod.json 的 keepDisabled：true=安装后不自动启用，对应默认不激活）。 */
  keepDisabled?: boolean;
  language?: string;
  /** 是否为基础游戏语言包（提供 content/translation/{game,campaigns,maps,chronicles}.json）。后端透传，用于区分"基础语言包"与"内容汉化包"。 */
  baseTranslation?: boolean;
  factions?: string[]; // 城镇阵营列表（由后端透传）
  /** 多语言块，键为 VCMI 语言标识（chinese/english/tchinese/...），由后端透传 */
  i18n?: Record<string, { name?: string; description?: string } | null>;
}

export interface ScanManifest {
  mods: ModInfo[];
  scannedAt: number;
}

interface RawModJson {
  name?: string;
  description?: string;
  modType?: string;
  version?: string;
  author?: string;
  depends?: string[];
  softDepends?: string[];
  conflicts?: string[];
  language?: string;
  [key: string]: unknown; // 各语言键（chinese/english/...）
}

/**
 * 按 VCMI 语言标识取词：优先该语言的 name；
 * 繁体中文(tchinese)无专属文本时回退简体中文；再无则回退 english；最后回退默认英文名。
 */
export function localizedName(
  mod: Pick<ModInfo, "name" | "i18n">,
  vcmiLang: string,
): string {
  const block = mod.i18n?.[vcmiLang];
  if (block?.name) return block.name;
  if (vcmiLang === "tchinese" && mod.i18n?.chinese?.name) return mod.i18n.chinese.name;
  if (mod.i18n?.english?.name) return mod.i18n.english.name;
  return mod.name || "";
}

export function localizedDescription(
  mod: Pick<ModInfo, "desc" | "i18n">,
  vcmiLang: string,
): string {
  const block = mod.i18n?.[vcmiLang];
  if (block?.description) return block.description;
  if (vcmiLang === "tchinese" && mod.i18n?.chinese?.description) return mod.i18n.chinese.description!;
  if (mod.i18n?.english?.description) return mod.i18n.english.description;
  return mod.desc || "";
}

/**
 * 归一化 mod id：**磁盘目录名小写**，多词保留空格，子 mod 用点号分层。
 *
 * 与 VCMI 一致（`lib/modding/ModManager.cpp` 对目录名做 `to_lower_copy`）：modId 是全局主键，
 * `modSettings.json` 的 presets、依赖/冲突、跨模组引用 `<modId>:<entityId>` 都用这个形态。
 * 实测 572 个模组的 presets 里含大写字母的 id 为 0 个 —— VCMI 侧确实小写。
 *
 * ⚠️ 原始目录名保留在 `ModInfo.diskName`（可能含空格与大写）；**磁盘定位请用 diskName，
 * 不要用 id**（小写后在大小写敏感的文件系统上必然找不到目录）。
 */
export function normalizeModId(diskName: string): string {
  return diskName.trim().toLowerCase();
}

/**
 * 计算 submod 在 modSettings settings[父][子] 中的键。
 * VCMI 子 ID 为磁盘目录名小写归一化、点号分层、保留空格。
 */
export function submodSettingKey(parentId: string, subDiskName: string): string {
  const normalized = subDiskName.trim().toLowerCase().replace(/\s+/g, " ");
  return `${parentId}.${normalized}`;
}

/** 将顶层 mod + 全部子 mod 展平为可勾选单元列表 */
export function flattenMods(rootMods: ModInfo[]): ModInfo[] {
  const out: ModInfo[] = [];
  const walk = (m: ModInfo) => {
    out.push(m);
    m.subs.forEach(walk);
  };
  rootMods.forEach(walk);
  return out;
}
