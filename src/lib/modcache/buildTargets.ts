/**
 * 缓存构建的**目标清单**构造 —— 全局后台构建（`status.ts`）与领域加急队列
 * （`realmAssets.ts`）共用。
 *
 * 为什么必须共用：原版伪模组（`core` / `core.<faction>`）的定义、子模组展开规则、
 * 与后端 `BatchWikiMod`（serde flatten）对齐的字段集 —— 任何一处两套实现出现差异，
 * 都会表现为"某个通道建过、另一个通道不认"的错位
 * （最典型：加急队列漏了 core → 原版 9 族在巡礼里没有图）。
 *
 * **读侧（百科页）同样复用本模块**：`index.json` 是「已缓存清单」，只合并不删除
 * （理由见 `modcache/batch.rs` 写索引处的注释），因此要把条目映射回「当前上下文可见的模组」，
 * 必须复用同一份 id 全集算法 —— 否则 `core.*`、子模组的 id 口径一分叉就会漏项。
 *
 * 纯 TS 逻辑层（**禁止 import react**）。
 */

import { isTauri, readLauncherConfig, scanRealMods } from "@/lib/tauri";
import { CORE_TOWN_IDS } from "@/lib/realm/coreTowns";
import { modIdKey } from "./index";

/** 提交给后端的单个构建项（字段需与后端 `BatchWikiMod` 的 flatten extra 对齐） */
export interface BatchPayload {
  modId: string;
  modPath: string;
  diskName: string;
  parentId: string;
  type: string;
  name: string;
  contentZipEncrypted: boolean;
}

/**
 * 递归展开 mod 树（含全部子模组）。
 *
 * 子模组的城镇数据独立存放于自己的目录（如 hota/mods/cove 港口种族、factory 工厂种族、
 * bulwark 坚垒），父模组主目录往往只有翻译/音效而无城镇数据。
 * 不展开则子模组永远不会生成缓存 → 城镇页子城镇无背景图、wiki 页缺子模组数据。
 */
export function flattenTree(list: Record<string, unknown>[]): Record<string, unknown>[] {
  const out: Record<string, unknown>[] = [];
  const walk = (m: Record<string, unknown>) => {
    out.push(m);
    (Array.isArray(m.subs) ? (m.subs as Record<string, unknown>[]) : []).forEach(walk);
  };
  list.forEach(walk);
  return out;
}

/** `scan_mods` 的节点 → 构建载荷（过滤掉缺 id / 缺路径的坏节点） */
export function toBatchPayload(list: Record<string, unknown>[]): BatchPayload[] {
  return list
    .map((m) => ({
      modId: String(m.id ?? m.modId ?? ""),
      modPath: String(m.modPath ?? m.path ?? ""),
      type: String(m.type ?? ""),
      diskName: String(m.diskName ?? ""),
      parentId: String(m.parentId ?? ""),
      name: String(m.name ?? ""),
      contentZipEncrypted: Boolean(m.contentZipEncrypted ?? false),
    }))
    .filter((m) => m.modId && m.modPath);
}

/**
 * 当前上下文**可见**的 modId 全集（[`modIdKey`] 小写键集合）。
 *
 * ## 为什么叫「可见」而不是「已安装」
 *
 * 领域（realm）激活会改写 `dirs.json` 的 `userDataPath`，借用（borrowed）/ 随包（bundled）
 * 模组只在对应领域可见；切回全局视图又换成另一批。而 `scan_mods` 扫的是
 * `AppPaths`（游戏目录 Mods + 当前用户目录 Mods），**天然反映当前上下文**，
 * 所以这里不必知道"现在是哪个领域"——这正是它比在缓存里记 `context` 标记更省事的地方。
 *
 * ## 用途：给「已缓存」集合做交集
 *
 * `index.json` 只增不删（卸载 / 切领域后的旧条目永久留在索引里），wiki 页若直接拿它当
 * 安装清单，就会出现**幽灵条目**与虚高计数，且实体类目会对每条索引做一次 IPC 读 cache。
 * 用本集合求交即可解决，且**自愈**（换上下文立刻生效，不像落盘的软标记会腐坏）。
 *
 * @returns `null` = 扫描失败 / 结果为空（非 Tauri、游戏目录未配置等）
 *          → 调用方**必须不筛选**，退化为此前行为，不可据此把列表清空。
 */
export async function visibleModKeySet(): Promise<Set<string> | null> {
  // 非 Tauri（浏览器自测）：无 scan 能力；若仅凭 core 伪模组构造非空集合，
  // 会把百科页过滤到只剩官方 9 城，违背下方「非 Tauri → 不筛选」的取舍。
  if (!isTauri()) return null;
  try {
    const cfg = await readLauncherConfig();
    const scanned = toBatchPayload(flattenTree((await scanRealMods()) as Record<string, unknown>[]));
    const keys = new Set<string>();
    // `core` / `core.<faction>` 是构造出来的伪模组，不在任何扫描结果里，必须并入
    for (const e of buildCoreEntries(cfg?.gameDir ?? "")) keys.add(modIdKey(e.modId));
    for (const m of scanned) keys.add(modIdKey(m.modId));
    return keys.size > 0 ? keys : null;
  } catch {
    return null;
  }
}

/**
 * 原版伪模组条目：**父 core + 9 个 `core.<faction>` 子模组**。
 *
 * 为什么需要：`scan_mods` 因 `RESERVED_SCOPES(["core","vcmi"])` 跳过了基础 mod，
 * 而原版 9 城镇/生物/英雄只存在于游戏根 `config/factions|creatures|heroes`。
 * 拆成子模组后，每个 `core.<faction>` 的缓存形态与第三方 town mod 完全一致
 * （factions/creatures/heroes），前端可按普通 town mod 渲染；
 * 父 core 承载全局与无归属实体（法术/技能/宝物/地形/中立生物等），不持有城镇。
 * 后端由 modId 本身识别作用域（见 `parse_core_mod`），无需额外参数。
 *
 * ⚠️ `name` **一律留空**：显示名由后端从 h3-core 多语言文案表派生
 * （`modcache::index::core_mod_display_name`，随内容语言切换），前端不携带任何语言标签
 * —— 早期此处写死中文枚举表，导致英文界面也显示中文（且中英文同值）。
 *
 * `gameDir` 为空时返回 `[]`（尚未配置游戏目录）。
 */
export function buildCoreEntries(gameDir: string): BatchPayload[] {
  if (!gameDir) return [];
  const root = gameDir.replace(/[\\/]$/, "");
  return [
    {
      modId: "core",
      modPath: `${root}/Mods/vcmi`,
      type: "Core",
      diskName: "vcmi",
      parentId: "",
      name: "",
      contentZipEncrypted: false,
    },
    // 9 个原版城镇：id 复用领域侧同一份清单（`CORE_TOWN_IDS`），不再维护第三份枚举
    ...CORE_TOWN_IDS.map((id) => ({
      modId: id,
      modPath: `${root}/Mods/vcmi`,
      // 归入 Town 批次：与第三方城镇模组同批优先构建，尽快出城镇大图
      type: "Town",
      diskName: id.slice("core.".length),
      parentId: "core",
      name: "",
      contentZipEncrypted: false,
    })),
  ];
}
