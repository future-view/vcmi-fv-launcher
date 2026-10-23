import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { Search, X } from "lucide-react";
import { useI18n } from "@/i18n";
import { cn } from "@/lib/utils";

import type { LangDict } from "@/i18n/types";
import { getModsCacheDir, getRuntimeInfo, isTauri } from "@/lib/tauri";
import { visibleModKeySet } from "@/lib/modcache/buildTargets";
import { vcmiLangTag } from "@/lib/vcmiLangs";
import {
  buildEntry,
  contentLangKey,
  loadModCache,
  loadWikiIndex,
  modIdKey,
  pickName,
  pickNameAny,
  type WikiCategory,
  type WikiEntry,
  type WikiLinkTarget,
} from "@/lib/modcache";
import { useWikiStagesDone, WikiBuildOverlay } from "@/shared/WikiBuildOverlay";
import { LoadingOverlay } from "@/shared/LoadingOverlay";
import { EntryDetail } from "@/features/wiki/detail/EntryDetail";
import { releaseDefAnimationCache } from "@/lib/def/animationCache";
import { WikiPopupProvider } from "@/features/wiki/detail/WikiPopup";
import { WikiFilterPanel } from "@/features/wiki/WikiFilterPanel";
import { EntryRow } from "@/features/wiki/EntryRow";
import {
  DEFAULT_SORT,
  collectBidNames,
  collectClassNames,
  normalizeSort,
  sortEntries,
} from "@/features/wiki/sort";
import type { SortContext, SortDim, SortSpec } from "@/features/wiki/sort";
import { readWikiSort, writeWikiSort } from "@/lib/uiPrefs";
import { artifactClassLabel, WIKI_FILTERS } from "@/features/wiki/filters";

const CATEGORIES: { key: WikiCategory; label: (t: LangDict) => string }[] = [
  { key: "factions", label: (t) => t.wiki.factions },
  { key: "creatures", label: (t) => t.wiki.creatures },
  { key: "heroes", label: (t) => t.wiki.heroes },
  { key: "artifacts", label: (t) => t.wiki.artifacts },
  { key: "spells", label: (t) => t.wiki.spells },
  { key: "skills", label: (t) => t.wiki.skills },
  { key: "buildings", label: (t) => t.wiki.buildings },
  { key: "terrains", label: (t) => t.wiki.terrains },
  { key: "glossary", label: (t) => t.wiki.glossary },
  { key: "mods", label: (t) => t.wiki.allMods },
];

/**
 * mod 完整缓存（`cache.json` 解析结果）的**内存上界**（条数）。
 *
 * 装载一个类目要遍历全部可见 mod（572 个），但装载完成后只留得下"最近用过的少数几个"
 * 就够了：列表条目各自持有自己的 `data`（`WikiEntry.data` 只引用该条，不随缓存淘汰消失），
 * 详情栏需要完整 mod 对象的只可能是**当前选中**那一个。
 * 取 24：远大于"一次会话里来回点选的 mod 数"，又足以把常驻堆从上百 MB 压到几 MB。
 */
const MOD_CACHE_CAP = 24;

/**
 * 跨 mod **按 id 去重**的类目（覆盖规则见 `loadAllCategories` 内的 flag 说明）。
 *
 * `glossary` 也走这条：VCMI 用 `assembleFromFiles` 把各 mod 的同名 key 合成一张全局表，
 * 前端按 id 去重等价；mod 自定义条目覆盖 core 的同名条目。
 */
const DEDUP_CATS: WikiCategory[] = ["spells", "skills", "artifacts", "creatures", "glossary"];

/** **逐 mod 直接入列**的类目（不去重，各自 mod 的条目都保留）。 */
const DIRECT_CATS: WikiCategory[] = ["factions", "heroes", "buildings"];

/**
 * 父模组页**聚合**的类目（与 `ModDetail` 的 SECTIONS 同一组）。
 *
 * `terrains` 刻意不在内：它是游戏级共享数据（每个 mod 缓存里都是同一份），
 * 聚合只会把同一批地形重复拼几遍。
 */
const AGG_CATS: WikiCategory[] = ["factions", "heroes", "creatures", "artifacts", "spells", "skills"];

/** `mods` 类目中栏的一行（树形展开后的扁平行） */
type ModRow = { entry: WikiEntry; depth: number; hasChildren: boolean };

/** 一次装载的产出：**全部类目** → 条目数组（见 `loadAllCategories` / `catEntriesRef`）。 */
type WikiCategoryMap = Map<WikiCategory, WikiEntry[]>;

/** 写入并维持上界（`Map` 的迭代顺序即插入顺序，队首 = 最久未用）。 */
function putModCache(
  m: Map<string, Record<string, unknown>>,
  modId: string,
  mod: Record<string, unknown>,
) {
  m.delete(modId);
  m.set(modId, mod);
  while (m.size > MOD_CACHE_CAP) {
    const oldest = m.keys().next().value;
    if (oldest === undefined) break;
    m.delete(oldest);
  }
}



export function WikiView() {
  const { t, lang } = useI18n();
  // ⚠️ 这里**只订阅粗粒度的"阶段完成"布尔量**，绝不订阅整个 `WikiBuildStatus`：
  // 构建期进度每 ~120ms emit 一次，在顶层订阅会让整棵子树（含长列表）跟着重渲染，
  // 而列表内容在阶段完成前并不会变 —— 这些渲染纯属浪费。
  // 需要实时进度的只有蒙层里那几条进度条，它们在自己的组件内订阅（见 `WikiBuildOverlay`）。
  const ready = useWikiStagesDone(["data", "assets"]);
  // 内容语言跟随 UI 语言（切换语言即同步刷新，保底英文），
  // 与 VCMI mod 多语言 schema（chinese/english/tchinese）对齐。
  const contentLang = contentLangKey(lang);
  const [category, setCategory] = useState<WikiCategory>("factions");
  const [query, setQuery] = useState("");
  // 筛选：维度 key → 选中值（空串 / 缺省 = 全部）。切类目时清空，对齐 VCMI 切类目清空搜索框的语义
  const [filters, setFilters] = useState<Record<string, string>>({});
  //
  // 排序（双槽：主排序 + 次排序），**按类目各记一组**（localStorage `wikiSort`）。
  //
  // 与筛选刻意不同：**切类目不清排序**。筛选的维度随类目变化，沿用无意义；排序是视图偏好，
  // 清了会让用户每次切类目都要重设。按类目记而非全局单记，是因为维度池随类目变化
  // （生物有等级、英雄有职业、建筑有 bid）——全局记会频繁遇到"在生物选的等级切到英雄不存在"。
  const [sortSpecs, setSortSpecs] = useState<Record<WikiCategory, SortSpec>>(() => {
    const stored = readWikiSort();
    const out = {} as Record<WikiCategory, SortSpec>;
    for (const k of Object.keys(DEFAULT_SORT) as WikiCategory[]) {
      // localStorage 的形状是宽松的（string），合法性由 normalizeSort 运行时裁剪
      out[k] = normalizeSort(k, stored[k] as Partial<SortSpec> | undefined);
    }
    return out;
  });
  // 阵营 id → 多语言名（cache.json 顶层 `factionNames`，跨 mod 合并）：供「按阵营筛选」显示名字
  const [factionNames, setFactionNames] = useState<Record<string, Record<string, string>>>({});
  // modId（小写归一）→ 多语言名（取自索引条目名）：供「来源模组」筛选显示名字
  const [modNames, setModNames] = useState<Record<string, Record<string, string>>>({});
  const [entries, setEntries] = useState<WikiEntry[]>([]);
  const [selected, setSelected] = useState<WikiEntry | null>(null);
  // 导航历史栈（对齐 VCMI wiki navHistory）：内容区链接/列表点选前压栈，返回按钮回退
  const [navStack, setNavStack] = useState<WikiEntry[]>([]);
  // 「选中条目所属 mod」的完整缓存（含多语言 description / version / creatures / heroes
  // / resources 等**整份 cache.json**）：索引条目与 `WikiEntry.data` 都不含这些，
  // 详情栏按需加载后从这里取（见下方 effect；有界 LRU）。
  const [modCacheMap, setModCacheMap] = useState<Record<string, Record<string, unknown>>>({});
  const [loading, setLoading] = useState(false);
  /**
   * 索引**已就绪**（`applyIndex` 完成过一次）—— 与 `indexed` 是否为**空集**是两个状态：
   * 后者无法区分"还没读"与"读完了但确实没有条目"。
   *
   * 装载（`loadAllCategories`）必须等它：曾经挂载即装载，而 `indexed` 还是空集
   * （`applyIndex` 的 IPC 未回，启动瞬间还要与预检抢同一份 `scan_mods`）→ 装载出
   * **空壳**（`mods` 类目有 289 条、实体类目全 0）并写进 `catEntriesRef` 缓存 +
   * 覆盖左栏真实计数 → 用户看到"分类全 0 + 未找到匹配条目"好几分钟，直到索引
   * 姗姗就绪才重装（2026-10-21 真机实测，见 startup-responsiveness 文档）。
   */
  const [indexDone, setIndexDone] = useState(false);
  /**
   * **首次装载完成**（`loadAllCategories` 跑完一轮）。
   *
   * 缓存已生成（`ready=true`）到装载完成之间仍有数秒（遍历 572 个 mod 的 cache.json，
   * 约 32 MB IPC + 解析；HDD 上更久）。这期间不渲染任何内容 —— 早返回只显示
   * loading 蒙层（`t.wiki.loading`）：宁可明确告知"在加载"，也不能渲染空状态
   * 让用户以为没有数据。
   */
  const [bootReady, setBootReady] = useState(false);
  /**
   * 中栏列表**已装载完成**的类目（`loadAllCategories` 异步，装载期间 `entries` 还是上一个类目的数据）。
   *
   * 只有「返回后定位并滚动到恢复行」需要区分这一点：靠 `entries[0].category` 推断在
   * "上一个类目恰好为空" 时会误判成"目标不在列表"，所以显式记一个已装载类目。
   */
  const [loadedCategory, setLoadedCategory] = useState<WikiCategory | null>(null);
  // 列表分页渲染：数万条目时避免一次性挂载全部 DOM（卡死防护）
  const [visibleCount, setVisibleCount] = useState(300);
  const [indexed, setIndexed] = useState<Set<string>>(new Set());
  /**
   * 当前上下文**可见**的 modId 小写键集合（`null` = 未知 → 不筛选）。
   *
   * `index.json` 是「已缓存清单」而非「安装清单」：它只合并不删除（见后端 `batch.rs`），
   * 所以卸载 / 切领域后旧条目仍在里面。这里把它与实时扫描求交，
   * 是 wiki 侧唯一需要知道"哪些缓存还有效"的地方。
   */
  const [visibleKeys, setVisibleKeys] = useState<Set<string> | null>(null);
  const [cacheDir, setCacheDir] = useState<string | null>(null);
  // `mods` 类目的树展开态（默认**全折叠**：101 个 mod 全展开等于没折叠）。
  // 点父行 = 选中 + 展开；点 chevron = 只切展开，不改选中。
  const [expandedMods, setExpandedMods] = useState<Set<string>>(() => new Set());
  /**
   * 父模组页的**聚合结果**：自身 + 全部后代子模组的各类目条目合并成一份虚拟缓存，
   * 外加「条目 → 来源子模组」映射（行尾来源标签 + 跳转目标）。
   *
   * 为什么在前端做而不是后端：实测 1.83 发行版全部 101 份缓存合计 **6.0 MB**，
   * 最大一族 hota 的 28 个子模组合计 **0.7 MB** —— 点一次父模组的代价可忽略，
   * 且可以按父 modId memo，重复点击不重复读。后端预聚合则会让父缓存成倍膨胀。
   */
  const [modAggregate, setModAggregate] = useState<{
    modId: string;
    cache: Record<string, unknown>;
    sources: Record<string, { modId: string; label: string }>;
  } | null>(null);
  const [counts, setCounts] = useState<Record<string, number>>({});
  // 任一 mod 缓存缺失原版 H3 数据文件：百科图标/背景将无图（提示用户补放 Data/*.lod）
  const [h3Missing, setH3Missing] = useState(false);
  // ⚠️ **有界 LRU**（`MOD_CACHE_CAP`）。此前是无界普通对象：装载会为**每一个**
  // 可见 mod 各读一次整份 `cache.json` 并永久留在内存里 —— 572 个 mod、缓存 JSON 合计
  // 32 MB，解析成 JS 对象后是几百 MB 的常驻堆。
  // 现在**列表数据由 `catEntriesRef` 承担**（见其说明），本缓存只负责详情页 ——
  // 真正还需要"整份 mod 对象"的只有**当前选中条目所属的那一个**（详情栏要读
  // `sharedIcons` / `resources` / `factionNames` / 同类目数组），其余 mod 的数据已被
  // `WikiEntry.data` 各自持有（条目只引用自己那一条，不受缓存淘汰影响）。
  const cacheRef = useRef<Map<string, Record<string, unknown>>>(new Map());
  /**
   * **全类目**的「id → 条目」索引（懒建，数据源是 `catEntriesRef` —— 它已经一次装载了
   * 全部类目，建索引只是加一张 Map，不额外读盘）。
   *
   * 为什么必须是"全类目"而不是"当前类目"：跨模组引用无处不在，而引用场景往往**不在**
   * 当前类目上——
   * - 在城镇页右键生物行弹生物窗（当前类目是 `factions`，查 `creatures`）；
   * - 宝物详情页查"组成组件"（组件常在 `core`，与母宝物不同 mod）；
   * - 生物详情页查"可升级为"（升级目标常在 core / 别的城镇 mod）。
   * 旧实现只保留**当前类目**的一张索引（`cat` 不等就查不到），这些场景全部落空 →
   * 表现为"可升级为只剩原始 id 且无图""宝物组成组件三格全空"。
   */
  const entryByIdRef = useRef<Map<WikiCategory, Map<string, WikiEntry>>>(new Map());
  /**
   * **全部类目**的装载结果（`WikiCategory` → 条目数组）。
   *
   * 为什么必须是"全部"而不是"按需 + LRU"：装载任何类目都要遍历**全部可见 mod**
   * （572 个）各读一份 `cache.json`（合计约 32 MB + JSON 解析，本地 SSD 上 1~3 秒）。
   * 若按需装载，切到每个**新**类目都要重付这笔钱 —— 实测用户感受就是"切换分类变慢很多"。
   * 而 `cacheRef` 的 24 条 LRU 无法代劳（容量上不可能覆盖 572 个 mod，装载时会一路淘汰）。
   *
   * 所以改为**一次装载产出全部类目**：遍历 mod 只发生一次，之后所有类目切换都只是
   * 从内存取数组。二者职责因此清晰分离：
   * - 本 ref → 服务**列表**（条目各自持有 `data`，不需要整份 mod 对象）；
   * - `cacheRef`（24 条）/ `modCacheMap` → 服务**详情页**（要整份 mod 对象的
   *   `sharedIcons` / `resources` / `factionNames` / `structures`）。
   *
   * 内存：只保留**列表实体的引用**，比"所有 mod 的完整缓存"小（后者还含
   * `modMeta` / `sharedIcons` / `buildingAnimations` 等非列表字段）。
   */
  const catEntriesRef = useRef<Map<WikiCategory, WikiEntry[]> | null>(null);
  /**
   * `catEntriesRef` 的有效性键。涵盖所有会改变装载结果的因素：
   * - `epoch`：后台补图完成后自增（图片字段由 null 变有值）
   * - `contentLang`：列表排序按当前语言的显示名（`sortByName`）
   * - `indexed.size` / 可见集合大小：安装或启停模组后条目集合会变
   */
  const catCacheKeyRef = useRef<string>("");
  // 右栏详情滚动容器：切换条目时要把滚动位置归零（见下方「切换条目后右栏回到顶部」）
  const detailScrollRef = useRef<HTMLDivElement | null>(null);
  // 中栏列表滚动容器 + 「返回后需要把列表滚到恢复行」的待办标记（见下方「返回后把中栏滚到恢复的高亮行」）
  const listScrollRef = useRef<HTMLDivElement | null>(null);
  const pendingListScrollRef = useRef(false);
  /**
   * 中栏**按类目**记忆的滚动位置（语义见下方「切换类目后恢复中栏滚动位置」）。
   * 键缺失 / 值为 0 都表示"该类目从顶部开始"。只在用户滚动时写入（`onScroll`）。
   */
  const listScrollMemoRef = useRef<Map<WikiCategory, number>>(new Map());
  /** 待恢复滚动位置的类目：切类目时置位，恢复成功（或判定无需恢复）后清掉。 */
  const pendingRestoreRef = useRef<WikiCategory | null>(null);
  // 缓存代数：后台构建（尤其阶段二补图）完成后自增，驱动当前分类重新拉取 mod.json，
  // 否则 cacheRef 里阶段一的旧对象（图片字段为 null）会一直用到刷新页面为止。
  const [epoch, setEpoch] = useState(0);
  /**
   * 装载有效性的键（`catCacheKeyRef` 的候选值）：涵盖所有会改变装载结果的因素 ——
   * `epoch`、`contentLang`、`indexed.size`、可见集合大小（见 `catCacheKeyRef` 说明）。
   *
   * 提到 `useMemo` 是为了让调用方能**按同一个键判重**（见 `loadInflightRef`）。
   */
  const catCacheKey = useMemo(
    () => `${epoch}|${contentLang}|${indexed.size}|${visibleKeys ? visibleKeys.size : -1}`,
    [epoch, contentLang, indexed, visibleKeys],
  );
  // 数据重载（后台补图 / 换显示语言 / 装卸载模组）后条目集合与排序都变了，
  // 记下来的行位置不再指向同一批条目 → 清空，各类目重新从顶部开始。
  useEffect(() => {
    listScrollMemoRef.current.clear();
  }, [catCacheKey]);
  /**
   * 正在跑的那一轮装载（`key` = 它对应的 `catCacheKey`）。
   *
   * ⚠️ 必须有这个判重，否则"切类目"会把机器读穿：装载要遍历全部可见 mod 各读一份
   * `cache.json`（572 份 / 约 32 MB，HDD 上十几秒），而装载期间 `catEntriesRef` 还是
   * `null`（命中不了缓存）—— 此前 `category` 直接进 effect 的 deps，于是**用户每点一次
   * 左栏就再起一轮全量读盘**。点得越多越慢（HDD 上多轮并发互相拖慢），表现就是
   * "切分类再也没有数据"（2026-10-16 实测）。
   */
  const loadInflightRef = useRef<{ key: string; p: Promise<WikiCategoryMap> } | null>(null);

  // 获取缓存根目录（拼图片完整路径）+ 订阅后台构建状态
  useEffect(() => {
    getModsCacheDir().then(setCacheDir);
  }, []);

  /**
   * 读索引并对齐到「当前可见集合」。
   *
   * ⚠️ `visibleKeys` 为 `null`（扫描失败 / 非 Tauri / 尚未配置游戏目录）时**不筛选**：
   * 宁可显示少量陈旧条目，也不能把页面清空 —— 这是这条路径唯一的失效模式，必须保守。
   */
  const applyIndex = useCallback(async () => {
    const [idx, keys] = await Promise.all([loadWikiIndex(), visibleModKeySet()]);
    setVisibleKeys(keys);
    // 索引就绪：装载 effect 以此为闸门（见 `indexDone` 注释）。
    // ⚠️ 必须在 `if (!idx) return` **之前**置位 —— "读不到索引"同样是就绪
    // （没有数据可显示），否则闸门永远不开、boot 蒙层永驻。
    setIndexDone(true);
    if (!idx) return;
    const mods = keys ? idx.mods.filter((m) => keys.has(modIdKey(m.modId))) : idx.mods;
    setCounts(collectCounts(mods));
    setIndexed(new Set(mods.map((m) => m.modId)));
    setH3Missing(mods.some((m) => m.h3DataMissing));
    // modId（小写归一）→ 多语言名：供「来源模组」筛选显示名字，顺带在此复用索引条目，
    // 不对每个类目额外发 IPC。索引 name 为多语言对象；兼容旧缓存 / 外部 mock 的单串回退。
    setModNames(
      Object.fromEntries(
        mods.map((m) => [
          modIdKey(m.modId),
          typeof m.name === "object" && m.name
            ? (m.name as Record<string, string>)
            : { chinese: String(m.name ?? m.modId), english: String(m.name ?? m.modId) },
        ]),
      ),
    );
  }, []);

  // 首次进入：读 index；构建中则订阅状态，完成后自动刷新
  useEffect(() => {
    // 已是火后即忘：组件卸载后 setState 无副作用（React 18 不再告警），只需吞掉异常
    void applyIndex().catch(() => {});
  }, [applyIndex]);

  /**
   * 离开百科时释放动画位图（`ImageBitmap` 不受 GC 管辖，只能显式 `close()`）。
   *
   * **切分类 / 切到后台都不在此释放**：
   * - 同分类内频繁点选时保留缓存才有意义；
   * - 后台标签页的 rAF 只是暂停，恢复后播放器仍要 `drawImage` 这些位图，
   *   提前 `close()` 会让恢复后第一帧抛错。
   * 残留由 `animationCache` 的 60s 前台空闲定时器兜底。
   */
  useEffect(
    () => () => {
      releaseDefAnimationCache();
    },
    [],
  );

  // 就绪（**data + assets 都完成**）后才加载索引 —— 不跟进度数字刷新。
  // ⚠️ 刻意不在 data 完成时就先加载一次：那时用户仍看不到内容（蒙层在等 assets），
  // 多加载一次只会白跑一遍解析与首屏渲染，随后又被丢弃。
  useEffect(() => {
    if (!ready) return;
    reloadAll();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [ready]);

  const reloadAll = useCallback(async () => {
    // 构建完成后丢弃内存里的旧 mod 缓存（阶段一对象图片字段为 null），强制重新读取
    cacheRef.current.clear();
    // 缓存重建 / 切换领域后源数据可能已变 → 动画位图一并作废（并释放显存）
    releaseDefAnimationCache();
    setEpoch((n) => n + 1);
    await applyIndex();
  }, [applyIndex]);

  /**
   * 中栏列表的统一排序：按「当前界面语言的显示名」升序。
   *
   * 对齐 VCMI `WikiWindow.cpp` —— 官方对**每个**分类都做了 `std::sort(a.name < b.name)`
   * （town / hero / creature / artifact / spell / skill / terrain 与 glossary；mods 见
   * `WikiModContent.cpp`）。启动器此前漏了这一步，而数据到达顺序来自 Rust 侧
   * `std::collections::HashMap` 的迭代序（`modcache/batch.rs` 的索引聚合、
   * `modcache/collect.rs` 的实体合并），每次进程运行、每次缓存重建都不同 ——
   * 这就是「中栏顺序总是不固定」的根因，在此兜底为全序。
   *
   * 名称回退链与列表显示一致（当前语言 → english → id），避免缺名条目在语言切换后乱序；
   * 比较交给 `Intl.Collator`（1420 条生物逐个 `localeCompare` 会明显变慢，且中文需按拼音）。
   */
  const nameCollator = useMemo(
    () => new Intl.Collator(vcmiLangTag(contentLang) ?? "en", { numeric: true, sensitivity: "base" }),
    [contentLang],
  );
  const sortByName = useCallback(
    (a: WikiEntry, b: WikiEntry) => {
      // glossary 与列表显示同一口径（任意可用语言回退），否则英文界面排序键全空
      const nameOf = (e: WikiEntry) =>
        (e.category === "glossary"
          ? pickNameAny(e.name, contentLang)
          : pickName(e.name, contentLang)) ||
        pickName(e.name, "english") ||
        e.id;
      return nameCollator.compare(nameOf(a), nameOf(b));
    },
    [nameCollator, contentLang],
  );

  /**
   * 按给定排序配置重排一个类目的列表（**原地**）。
   *
   * 供两处共用：① `loadAllCategories` 装载完毕时；② 用户在浮层切换排序时。
   * 切换排序走这条而不是重新装载 —— 重排不需要再读一遍 572 个 mod 的 cache.json
   * （32 MB + 解析），零 IPC。
   *
   * `factionNames` 由调用方传入：装载途中要用刚收集的 `namesAcc`（state 那时还没更新），
   * 切换排序时用 `factionNames` state。
   */
  const sortList = useCallback(
    (
      cat: WikiCategory,
      list: WikiEntry[],
      spec: SortSpec,
      fnames: Record<string, Record<string, string>>,
    ) => {
      // 主排序为名称 → 保持原有名称序（含 english 回退），确保"按名称"模式零回归。
      // buildings 额外保留「中立置顶」：默认模式要解决"370+ 城镇建筑淹没中立建筑"的
      // 实测问题；分组模式下无城镇条目由「无值排最后」接管，两者相反是刻意的。
      if (spec.primary === "name") {
        if (cat === "buildings") {
          list.sort((a, b) => {
            const an =
              String((a.data as Record<string, unknown>).factionId ?? "") === "neutral" ? 0 : 1;
            const bn =
              String((b.data as Record<string, unknown>).factionId ?? "") === "neutral" ? 0 : 1;
            return an - bn || sortByName(a, b);
          });
          return;
        }
        list.sort(sortByName);
        return;
      }
      const ctx: SortContext = {
        lang: contentLang,
        t,
        factionNames: fnames,
        classNames: cat === "heroes" ? collectClassNames(list) : {},
        bidNames: cat === "buildings" ? collectBidNames(list) : {},
      };
      sortEntries(list, spec, ctx, nameCollator);
    },
    [sortByName, contentLang, t, nameCollator],
  );

  /**
   * 一次装载**全部类目**的条目（聚合各 mod），返回 `类目 → 条目数组`。
   *
   * 为什么不是"按需装载单个类目"：装载要遍历全部可见 mod（572 个）各读一份 `cache.json`，
   * 按需装载会让每个**新**类目都重付这笔钱（约 32 MB + 解析）。改为一次产出全部类目后，
   * 切换分类（无论新旧）都只是从内存取数组（见 `catEntriesRef` 的说明）。
   */
  const loadAllCategories = useCallback(async (key: string): Promise<WikiCategoryMap> => {
    // ── 命中：直接复用，零 IPC ──
    if (catCacheKeyRef.current !== key) {
      // 数据源变了（补图完成 / 换语言 / 增删模组）→ 旧条目可能过期，整体作废
      catEntriesRef.current = null;
      entryByIdRef.current.clear();
      catCacheKeyRef.current = key;
    }
    if (catEntriesRef.current) return catEntriesRef.current;

    setLoading(true);
    const out = new Map<WikiCategory, WikiEntry[]>();
    {
      // ── `mods` 类目：读 `index.json`（不遍历 mod 的 cache.json）──
      const list: WikiEntry[] = [];
      const idx = await loadWikiIndex();
      if (idx) {
        // 与「当前可见集合」求交：剔除已卸载 / 其它领域的缓存条目（索引只增不删）
        const mods = visibleKeys ? idx.mods.filter((m) => visibleKeys.has(modIdKey(m.modId))) : idx.mods;
        for (const m of mods) {
          list.push({
            category: "mods",
            id: m.modId,
            modId: m.modId,
            // pickName 只认 VCMI 语言键（chinese/english），勿用 en/zh-CN。
            // 后端索引 name 为多语言对象（modMeta.name）；兼容旧缓存/外部 mock 的单串回退。
            name:
              typeof m.name === "object" && m.name
                ? (m.name as Record<string, string>)
                : { chinese: String(m.name ?? m.modId), english: String(m.name ?? m.modId) },
            data: { ...m },
          });
        }
      }
      list.sort(sortByName);
      // 只保留「百科里有内容」的模组（对齐 VCMI `WikiModContent.cpp:97-112` 的 `modHasContent`）：
      // 实测 572 个 mod 里 309 个是字体 / 翻译 / 纯 AI 调参 / 地图对象补丁 / 兼容补丁，
      // 它们七类实体全为空 —— 列进百科只是噪音，点开也没有任何内容可看。
      out.set("mods", pruneModsWithoutContent(list));
    }
    // 只遍历「当前可见」的 mod：索引条目随时间累积（卸载后仍保留），
    // 而下面每条都要一次 IPC 读 cache.json —— 不筛选的话，开销会随使用时长单调增长。
    const modIds = Array.from(indexed).filter((id) => !visibleKeys || visibleKeys.has(modIdKey(id)));
    let processed = 0;
    // 跨 mod 去重的累加器：**每个类目一份**（此前只用一份 `spellMap`/`entityMap`，
    // 因为每次只装载一个类目；现在一次装载全部，混用会把不同类目的条目串在一起）。
    type DedupEntry = { mod: Record<string, unknown>; item: Record<string, unknown>; flag: boolean };
    const dedupAcc: Record<string, Map<string, DedupEntry>> = {
      spells: new Map(),
      skills: new Map(),
      artifacts: new Map(),
      creatures: new Map(),
      glossary: new Map(),
    };
    // 逐 mod 直接入列的类目容器
    const directAcc: Record<string, WikiEntry[]> = {
      factions: [],
      heroes: [],
      buildings: [],
    };
    // 地形是游戏级数据（**每个 mod 缓存里都是同一份**），必须跨 mod 按 id 去重；
    // nativeTowns 按 mod 分散（各 mod 只报自己的本族城镇），去重时合并城镇列表。
    // 此前按单个 mod 建 map → 每个 mod 各出 10 条，17 个 mod 就是 170 条假重复。
    const terrainAcc = new Map<string, { mod: Record<string, unknown>; obj: Record<string, unknown> }>();
    // 阵营 id → 多语言名（cache.json 顶层 `factionNames`）：与分类数据同一次读取拿到，
    // 无须为取标签额外发 IPC；跨 mod 合并，后见者覆盖。
    const namesAcc: Record<string, Record<string, string>> = {};
    for (const mid of modIds) {
      let mod = cacheRef.current.get(mid);
      if (!mod) {
        mod = (await loadModCache(mid)) ?? undefined;
        if (mod) putModCache(cacheRef.current, mid, mod);
      }
      if (!mod) continue;
      if (mod.factionNames && typeof mod.factionNames === "object") {
        Object.assign(namesAcc, mod.factionNames as Record<string, Record<string, string>>);
      }

      // ── 逐 mod 直接入列的类目（城镇/英雄/建筑）──
      for (const c of DIRECT_CATS) {
        for (const item of (mod[c] as unknown[]) ?? []) {
          const obj = item as Record<string, unknown>;
          // 跨 mod 引用条目（如 `core:dimensionDoor`、`hota.factory:factory`）是补丁引用，
          // 构建期已标记 `ref:true`；列表默认不展示（计数同样已排除）
          if (obj.ref) continue;
          directAcc[c].push(buildEntry(mod, c, obj));
        }
      }

      // ── 跨 mod 按 id 去重的类目（法术/技能/宝物/生物）──
      // 覆盖规则统一为"旧标记为真、新标记为假 → 替换"：
      //   spells/skills 的标记 = `builtin`（非 builtin 覆盖 builtin → mod 覆盖原版优先）
      //   artifacts/creatures 的标记 = 来自 core（mod 定义优先于原版，避免同名重复入列）
      for (const c of DEDUP_CATS) {
        const isSpellKind = c === "spells" || c === "skills";
        const flagOf = (o: Record<string, unknown>) =>
          isSpellKind ? !!o.builtin : mid === "core" || mid.startsWith("core.");
        const acc = dedupAcc[c];
        for (const item of (mod[c] as unknown[]) ?? []) {
          const o = item as Record<string, unknown>;
          // 跨 mod 引用条目（VCMI `<modId>:<entityId>` 补丁语法）：无独立数据，不进入列表
          if (o.ref) continue;
          const id = String(o.id ?? o.factionId ?? "");
          if (!id) continue;
          const flag = flagOf(o);
          const ex = acc.get(id);
          if (ex) {
            if (ex.flag && !flag) acc.set(id, { mod, item: o, flag });
            continue;
          }
          acc.set(id, { mod, item: o, flag });
        }
      }

      // ── 地形（游戏级数据，跨 mod 按 id 去重 + 多语言合并）──
      for (const item of (mod.terrains as unknown[]) ?? []) {
        const o = { ...(item as Record<string, unknown>) };
        const id = String(o.id ?? "");
        if (!id) continue;
        const prev = terrainAcc.get(id);
        if (!prev) {
          terrainAcc.set(id, { mod, obj: o });
          continue;
        }
        // **多语言合并**：同名地形可能被多个 mod 定义、各带部分语言
        // （A 有中文、B 有英文），合并后任意界面语言都能取到；
        // 不做"某语言优先"（会偏袒中文，对以外文为主的模组不公平）。
        // 语言中立的质量规则：已有值为空或就是实体 id 原文时，用新值替换。
        const mergeNames = (a: unknown, b: unknown) => {
          const acc: Record<string, string> = { ...((a as Record<string, string>) ?? {}) };
          for (const [k, v] of Object.entries((b as Record<string, string>) ?? {})) {
            const old = acc[k];
            if ((!old || old === id) && typeof v === "string" && v && v !== id) acc[k] = v;
          }
          return acc;
        };
        prev.obj.name = mergeNames(prev.obj.name, o.name);
        if (!prev.obj.image && o.image) prev.obj.image = o.image;
        if (!prev.obj.tiles && o.tiles) prev.obj.tiles = o.tiles;
        const towns = [
          ...((prev.obj.nativeTowns as string[]) ?? []),
          ...((o.nativeTowns as string[]) ?? []),
        ];
        prev.obj.nativeTowns = [...new Set(towns)];
      }

      // 每 8 个 mod 让出一帧，避免长任务阻塞交互（卡死防护）。
      // 取 8（原 24）：单个 cache.json 解析是同步长任务（32MB / 289 份），
      // 24 连跑会让单帧主线程占用达到秒级 —— WebView 无法及时处理
      // `WM_SETCURSOR` 时 Windows 会显示忙碌光标（真机反馈：装载/构建期
      // 光标不定期转圈）。8 连跑的单帧占用降到亚百毫秒级。
      processed += 1;
      if (processed % 8 === 0) {
        await new Promise((r) => setTimeout(r, 0));
      }
    }

    // ── 产出各累加器的条目数组（排序口径与原「按类目装载」完全一致）──
    // 一次产出全部类目意味着这一段是**同步长任务**（上万个 `buildEntry` + 排序），
    // 故每处理一个类目让出一帧，避免一次性卡住交互（与上面 mod 循环同款防护）。
    const yieldFrame = () => new Promise((r) => setTimeout(r, 0));
    //
    // 排序走共享的 `sortList`（与浮层切换排序是同一条代码路径，避免两处口径漂移）。
    // `namesAcc` 是本次装载刚收集的跨 mod 名字表 —— 此时 `factionNames` state 还没更新。
    const applySort = (cat: WikiCategory, list: WikiEntry[]) =>
      sortList(cat, list, sortSpecs[cat], namesAcc);
    for (const c of DIRECT_CATS) {
      const list = directAcc[c];
      applySort(c, list);
      out.set(c, list);
      await yieldFrame();
    }
    for (const c of DEDUP_CATS) {
      const list: WikiEntry[] = [];
      for (const { mod, item } of dedupAcc[c].values()) {
        list.push(buildEntry(mod, c, item));
      }
      applySort(c, list);
      out.set(c, list);
      await yieldFrame();
    }
    {
      // 地形：跨 mod 去重结果一次性入列（每个 mod 只贡献本族 nativeTowns）。
      // 按显示名升序：原按 `index` 排序是启动器自创，官方 `WikiWindow.cpp` 的地形列表
      // 同样按 `a.name < b.name`，此处对齐官方。
      const list: WikiEntry[] = [];
      for (const { mod, obj } of terrainAcc.values()) {
        list.push(buildEntry(mod, "terrains", obj));
      }
      list.sort(sortByName);
      out.set("terrains", list);
      await yieldFrame();
    }
    // 左栏计数 = 实际聚合条目数（原版法术等已跨 mod 去重，与列表一致）。
    // 一次装载产出全部类目，故一次写全；此前是每装载一个类目写一次。
    setCounts((prev) => {
      const next = { ...prev };
      for (const [c, list] of out) next[c] = list.length;
      return next;
    });
    // 阵营名映射：合并各 mod 的顶层表（渲染时按 id 取词）
    setFactionNames((prev) => ({ ...prev, ...namesAcc }));
    // ⚠️ 装载期间数据源可能又变了（`key` 由调用方给出、`catCacheKeyRef` 已被新一轮改写）：
    // 这份结果已经过期，**不能**写进缓存（否则新一轮的装载结果会被它顶掉，页面一直显示旧数据）。
    if (catCacheKeyRef.current === key) catEntriesRef.current = out;
    setLoading(false);
    return out;
  }, [
    indexed,
    visibleKeys,
    sortByName,
    sortList,
    // `sortSpecs` 进 deps 是**必需**的：用户切换排序后若再切类目，新一轮装载必须用新配置，
    // 否则新类目会退回上一次的排序（缓存的 useCallback 捕获的是旧值）。
    sortSpecs,
    // `contentLang` 进 deps 是**必需**的：闭包内的 `sortByName` 要按当前语言排序。
    // 有效性键（含 `epoch`）由调用方以 `key` 传入，见 `catCacheKey`。
    contentLang,
  ]);

  /**
   * 取（必要时启动）某一轮装载 —— **按 `key` 单飞**（single-flight）。
   *
   * 同一 `key` 已有装载在跑 → 直接复用那个 promise，不再起第二轮（见 `loadInflightRef`）。
   * 切类目因此在装载期间也是零成本（只是换个数组取），不会再叠出多轮全量读盘。
   */
  const ensureCategoriesLoaded = useCallback(
    (key: string): Promise<WikiCategoryMap> => {
      const inflight = loadInflightRef.current;
      if (inflight && inflight.key === key) return inflight.p;
      const rec: { key: string; p: Promise<WikiCategoryMap> } = {
        key,
        p: loadAllCategories(key),
      };
      // 收尾时清掉登记（只清自己那条：期间可能已被新一轮覆盖）
      rec.p = rec.p.finally(() => {
        if (loadInflightRef.current === rec) loadInflightRef.current = null;
      });
      loadInflightRef.current = rec;
      return rec.p;
    },
    [loadAllCategories],
  );

  /**
   * 按 (类目, id) 查条目 —— **全类目**索引（数据源 `catEntriesRef`，见 `entryByIdRef` 注释）。
   *
   * 消费者：详情组件的跨模组引用解析（生物升级链、宝物组成组件/所属组合、
   * 英雄初始部队的生物头像与战争机器判定），以及弹窗（`WikiPopup`）。
   * 索引未就绪（首次装载中）时返回 `undefined`，调用方各自回退到 mod 本地查找。
   */
  const lookupEntry = useCallback((cat: WikiCategory, id: string): WikiEntry | undefined => {
    const src = catEntriesRef.current;
    if (!src) return undefined;
    let m = entryByIdRef.current.get(cat);
    if (!m) {
      m = new Map((src.get(cat) ?? []).map((e) => [String(e.id), e]));
      entryByIdRef.current.set(cat, m);
    }
    return m.get(id);
  }, []);

  // 压栈跳转：记录当前选中，再切换（对齐 VCMI navigateTo + navHistory）
  const pushAndSelect = useCallback(
    (e: WikiEntry | null) => {
      if (!e) return;
      setNavStack((s) => (selected && (selected.id !== e.id || selected.category !== e.category || selected.modId !== e.modId) ? [...s.slice(-49), selected] : s));
      setSelected(e);
    },
    [selected],
  );

  /**
   * 返回上一条目（对齐 VCMI WebsiteWindow::navigateBack）。
   *
   * ⚠️ 必须**连同来源类目一起还原**：`navStack` 里的条目带自己的 `category`，
   * 而跳转时 `navigateTo` 会 `setCategory` 切走。只 `setSelected(prev)` 的话，
   * 会出现「左栏高亮建筑 / 中栏是建筑列表 / 右栏却显示城镇」三栏互相矛盾的状态
   * （VCMI 原实现 `WikiWindow.cpp:1786-1802` 就是取 `prev.category` 切回去的）。
   *
   * 搜索词处理与 VCMI 保持一致：仅在**类目真的变了**时清空
   * （VCMI 的返回不清搜索框 `:1774-1841`，只有切类目 `:1668` 才清）；
   * 清空的目的是让还原的条目不被旧类目的搜索词过滤掉，从而能在中栏高亮。
   */
  const navigateBack = useCallback(() => {
    if (navStack.length === 0) return;
    const prev = navStack[navStack.length - 1];
    setNavStack((s) => s.slice(0, -1));
    if (prev.category !== selected?.category) setQuery("");
    pendingListScrollRef.current = true;
    setCategory(prev.category);
    setSelected(prev);
  }, [navStack, selected]);

  // 联动跳转：根据 (modId, category, id) 定位条目并切换详情
  const navigateTo = useCallback(
    async (target: WikiLinkTarget) => {
      // 标记"跳转后要把中栏滚到新条目"：链接跳转（升级链 / 模组条目等）与"返回"不同，
      // 不设这个标记时右栏换了条目、中栏还停在原地，用户看不到自己跳到了哪一行。
      pendingListScrollRef.current = true;
      // 1. 当前已加载列表里查找。modId 精确命中失败时，退化为"同分类按 id 找"：
      // 生物详情页的升级链只有目标 id，modId 借用当前条目的（同 mod 是常态，跨 mod
      // 升级会指错）。creatures / artifacts / spells / skills / terrains 均已跨 mod
      // 按 id 去重，同 id 只有一条，按 id 兜底不会指错对象。
      const hit =
        entries.find(
          (e) => e.category === target.category && e.modId === target.modId && e.id === target.id,
        ) ?? entries.find((e) => e.category === target.category && e.id === target.id);
      if (hit) {
        setCategory(target.category);
        setQuery("");
        pushAndSelect(hit);
        return;
      }
      // 2. 从缓存取该 mod 的对应类目对象
      let mod = cacheRef.current.get(target.modId);
      if (!mod) {
        mod = (await loadModCache(target.modId)) ?? undefined;
        if (mod) putModCache(cacheRef.current, target.modId, mod);
      }
      const arr = mod ? ((mod[target.category] as unknown[]) ?? []) : [];
      const obj = arr.find((it) => {
        const o = it as Record<string, unknown>;
        return String(o.id ?? o.factionId ?? "") === target.id;
      }) as Record<string, unknown> | undefined;
      if (obj) {
        setCategory(target.category);
        setQuery("");
        pushAndSelect(buildEntry(mod ?? { modId: target.modId }, target.category, obj));
        return;
      }
      // buildings 嵌套在 factions[].buildings（mod 顶层无此数组）
      if (target.category === "buildings" && mod) {
        for (const f of (mod.factions as unknown[]) ?? []) {
          const bs = ((f as Record<string, unknown>).buildings as unknown[]) ?? [];
          const hitB = bs.find((it) => String((it as Record<string, unknown>).id ?? "") === target.id);
          if (hitB) {
            setCategory("buildings");
            setQuery("");
            pushAndSelect(buildEntry(mod, "buildings", hitB as Record<string, unknown>));
            return;
          }
        }
      }
      // 3. 找不到对应条目（数据缺失），仅切换类目
      setCategory(target.category);
      setQuery("");
    },
    [entries, pushAndSelect],
  );

  useEffect(() => {
    // ⚠️ 只**取**当前类目，不重新装载：装载一次产出全部类目（`ensureCategoriesLoaded`
    // 按 `catCacheKey` 单飞，切类目不会叠出第二轮全量读盘），之后每次切类目都命中其
    // 内部缓存 → 零 IPC（这正是"切分类不再卡"的关键）。
    //
    // 三道闸门（都缺一不可，见各处注释）：
    // 1. `ready`：后台构建未完成时不装载 —— 蒙层期用户看不到内容，白跑一遍全量读盘；
    // 2. `indexDone`：索引未就绪不装载 —— 否则 `indexed` 还是空集，会产出
    //    "只有 mods 类目"的空壳并写进 `catEntriesRef`（左栏全 0 + 未找到匹配条目）；
    // 3. `indexed.size > 0`：真空索引（确实没有任何缓存）直接放行，让页面显示
    //    `t.wiki.empty` —— 没有数据就不该永远转圈。
    let cancelled = false;
    if (!ready || !indexDone) return;
    if (indexed.size === 0) {
      setBootReady(true);
      return;
    }
    void (async () => {
      const map = await ensureCategoriesLoaded(catCacheKey);
      // ⚠️ 必须判 `cancelled`：类目可能在装载期间被切走，此时这份结果属于**上一个类目**，
      // 再 `setEntries` 会把当前类目的列表顶掉（用户看到"切了分类，列表却没变"）。
      if (cancelled) return;
      const list = map.get(category) ?? [];
      // 跨模组引用索引由 `lookupEntry` 从 `catEntriesRef` 懒建（全类目，不再限定当前类目）
      setEntries(list);
      setLoadedCategory(category);
      // 首次装载完成 → 撤 boot loading 蒙层（见 `bootReady` 注释）
      setBootReady(true);
    })().catch(() => {
      // 装载失败：同样放行（显示 empty 好过永驻蒙层），异常已在 ensureCategoriesLoaded 内吞掉
      if (!cancelled) setBootReady(true);
    });
    return () => {
      cancelled = true;
    };
  }, [ready, indexDone, indexed.size, category, catCacheKey, ensureCategoriesLoaded]);

  // 数据重载（如后台阶段二补图完成）后刷新当前详情对象，避免详情页停留在旧的无图对象
  useEffect(() => {
    if (!selected) return;
    const hit = entries.find((e) => e.modId === selected.modId && e.id === selected.id);
    if (hit && hit !== selected) setSelected(hit);
  }, [entries, selected]);

  /**
   * 切换条目后右栏回到顶部。
   *
   * ⚠️ 右栏 `<section>` 是**跨条目复用的同一个滚动容器**：DOM 节点不变，
   * 浏览器会保留 `scrollTop`。城镇详情页很长，从 A 城镇滚到中段再点 B 城镇，
   * 新内容会停在中段（甚至底部），体验上是"点了没反应/直接跳到末尾"。
   * 只在**条目身份真的变了**时归零：后台构建补图后 `selected` 会被同 id 的新对象
   * 替换（见上方刷新 effect），那时不该打断用户已滚动的位置。
   */
  const selectedKey = selected ? `${selected.modId}:${selected.category}:${selected.id}` : "";
  // 用 layout effect：在浏览器绘制新详情**之前**归零，否则会闪一帧"滚到中段"的旧位置
  useLayoutEffect(() => {
    const el = detailScrollRef.current;
    if (el) el.scrollTop = 0;
  }, [selectedKey]);

  // 「切换类目后按类目记忆 / 恢复中栏滚动位置」的两个 effect 见下方 `filtered` 之后

  /**
   * 选中条目后按需加载**该 mod 的完整缓存**（`cache.json` 全文），供右侧详情栏使用。
   *
   * ## 为什么必须按需（2026-10-16 修复「城镇详情页没有生物/英雄表」）
   *
   * `cacheRef` 是**有界 LRU（24 条）**，而一次装载要遍历**全部**可见 mod（572 个，
   * 见 `loadAllCategories`）—— 装载完只剩最后 24 个 mod 在里面。详情页要的
   * `creatures` / `heroes` / `resources` / `statIcons` / `factionNames` 都在整份
   * `cache.json` 里，而 `WikiEntry.data` 只持有**该条目自己那一条**，取不到其余数组。
   * 原先这条按需加载只对 `mods` 类目生效，于是 `factions` / `creatures` / `heroes` /
   * `artifacts` / `spells` / `buildings` 的详情页有 96% 的概率拿到 `undefined`
   * （572 个 mod 里只有 24 个在 LRU 里）→ 生物表 / 英雄表整段不渲染。
   *
   * 代价：每次选中**新** mod 的条目多一次 IPC（本地读一份 cache.json，MB 级）——
   * `mods` 类目一向如此，且 `modCacheMap` 同样是有界 LRU，内存不增长。
   */
  useEffect(() => {
    if (!selected) return;
    // LRU 里已有该 mod（同 mod 的连续点选：一个 town mod 的多个 faction、
    // 一个生物 mod 的几十条生物）→ 渲染处的 `?? cacheRef.current.get(...)` 直接兜底，
    // 不必再发一次 IPC。
    if (cacheRef.current.has(selected.modId)) return;
    let cancelled = false;
    void loadModCache(selected.modId).then((c) => {
      if (cancelled) return;
      // 与 `cacheRef` 同理：**只留最近几个**。此前的 `{...m, [id]: c}` 会随"逐个点开
      // 全部模组"无限增长 —— 每份都是整份 `cache.json`，几百个就是几百 MB。
      setModCacheMap((m) => {
        const next = new Map(Object.entries(m));
        next.delete(selected.modId);
        next.set(selected.modId, c ?? {});
        while (next.size > MOD_CACHE_CAP) {
          const oldest = next.keys().next().value;
          if (oldest === undefined) break;
          next.delete(oldest);
        }
        return Object.fromEntries(next);
      });
    });
    return () => {
      cancelled = true;
    };
  }, [selected]);

  // 搜索过滤（**只做搜索**）。
  // 匹配条目的**全部语言名**而不是枚举某几种语言：界面语言已达 5 种且会继续增加，任何"写死中/英"的口径在新增语言时
  // 都会静默失效（只改文案救不了）。`id` 一并参与匹配，便于按 jsonKey 定位。
  // 这套「跨语言可搜」是**有意超出 VCMI 官方**的：官方只对当前语言显示名做模糊匹配
  // （`WikiWindow.cpp` 的 `isFuzzyMatch(filter, entry.name)`），而用户常常只记得另一种语言名。
  //
  // ⚠️ 搜索必须与筛选**分开**：筛选浮层要用「搜索后的条目」当基准去算每个维度的选项与计数
  // （见 `WikiFilterPanel`）。合并成一个 memo 的话，浮层只能拿到全量条目，
  // 计数会和「当前列表实际有多少条」对不上。
  const searched = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return entries;
    return entries.filter(
      (e) =>
        e.id.toLowerCase().includes(q) ||
        Object.values(e.name).some((v) => typeof v === "string" && v.toLowerCase().includes(q)),
    );
  }, [entries, query]);

  // 筛选过滤：在搜索结果之上按维度求交集。
  // 维度按类目取用（`WIKI_FILTERS`，见 filters.ts）：同一维度单选、不同维度取交集（AND）。
  const filtered = useMemo(() => {
    const activeSpecs = WIKI_FILTERS[category].filter((s) => filters[s.key]);
    if (!activeSpecs.length) return searched;
    return searched.filter((e) => activeSpecs.every((s) => s.match(e, filters[s.key])));
  }, [searched, category, filters]);

  /**
   * `mods` 类目的**父子结构**（数据源 = 平铺的 `entries`，每条的 `data.parentId`）。
   *
   * `parentId` 的语义见 `lib/modScanner`：点号前第一段，顶层模组为自身（或空）。
   * 父模组**不在当前可见集合**（卸载 / 缓存缺失）时把该条按根展示 —— 否则整棵子树
   * 会凭空消失，用户连它自己都看不到。
   *
   * 只依赖 `entries`（不依赖搜索/筛选）：结构是数据属性，不该随查询变化；
   * 聚合 effect 也才能稳定，否则每敲一个字都要重新合并一次后代内容。
   */
  const modStructure = useMemo(() => {
    if (category !== "mods") return null;
    const byKey = new Map<string, WikiEntry>();
    for (const e of entries) byKey.set(modIdKey(e.modId), e);
    const roots: WikiEntry[] = [];
    const childrenOf = new Map<string, WikiEntry[]>();
    const parentOf = new Map<string, string>();
    for (const e of entries) {
      const key = modIdKey(e.modId);
      const pid = modIdKey(String((e.data as Record<string, unknown>).parentId ?? ""));
      if (!pid || pid === key || !byKey.has(pid)) {
        roots.push(e);
        continue;
      }
      parentOf.set(key, pid);
      const arr = childrenOf.get(pid);
      if (arr) arr.push(e);
      else childrenOf.set(pid, [e]);
    }
    // `entries` 已按显示名排序，故 roots / childrenOf 内的顺序天然是名称序
    return { roots, childrenOf, parentOf };
  }, [category, entries]);

  /** 搜索 / 模组类型筛选生效时，中栏改为**扁平命中列表**（见 `modRows`）。 */
  const modsFlat =
    category === "mods" && (query.trim() !== "" || WIKI_FILTERS.mods.some((s) => filters[s.key]));

  /**
   * 中栏要渲染的模组行（DFS 序，只下钻到已展开的节点；默认全折叠 —— 101 个模组
   * 全展开等于没折叠）。点父行 = 选中 + 展开；折叠只由 chevron 负责。
   *
   * 搜索 / 筛选时改扁平：树里"父模组没命中、子模组命中"会被折叠藏起来，
   * 而此刻用户要的是"哪些匹配项"，不是看层级。
   */
  const modRows = useMemo<ModRow[]>(() => {
    if (!modStructure) return [];
    if (modsFlat) return filtered.map((entry) => ({ entry, depth: 0, hasChildren: false }));
    const out: ModRow[] = [];
    const walk = (list: WikiEntry[], depth: number) => {
      for (const e of list) {
        const kids = modStructure.childrenOf.get(modIdKey(e.modId)) ?? [];
        out.push({ entry: e, depth, hasChildren: kids.length > 0 });
        if (kids.length > 0 && expandedMods.has(modIdKey(e.modId))) walk(kids, depth + 1);
      }
    };
    walk(modStructure.roots, 0);
    return out;
  }, [modStructure, modsFlat, filtered, expandedMods]);

  /** 中栏实际行数（分页 / 滚动定位用）：模组类是树形扁平行，其余类目就是 `filtered`。 */
  const rowCount = category === "mods" ? modRows.length : filtered.length;

  /** 分页窗口内的行。非 `mods` 类目包一层 `ModRow` 形状，渲染处只认这一种。 */
  const visibleRows = useMemo<ModRow[]>(() => {
    const rows: ModRow[] =
      category === "mods"
        ? modRows
        : filtered.map((entry) => ({ entry, depth: 0, hasChildren: false }));
    return rows.slice(0, visibleCount);
  }, [category, modRows, filtered, visibleCount]);

  /**
   * 选中的模组若在折叠的子树里（返回 / 跳转 / 搜索后清空搜索），逐级把祖先展开：
   * 否则中栏高亮行根本不存在，"返回后滚动到恢复行"也无从定位。
   */
  useEffect(() => {
    if (category !== "mods" || selected?.category !== "mods" || !modStructure) return;
    const need = new Set<string>();
    let cur: string | undefined = modIdKey(selected.modId);
    for (let i = 0; i < 16 && cur; i += 1) {
      need.add(cur);
      cur = modStructure.parentOf.get(cur);
    }
    setExpandedMods((prev) => {
      let changed = false;
      const next = new Set(prev);
      for (const k of need) {
        if (!next.has(k)) {
          next.add(k);
          changed = true;
        }
      }
      return changed ? next : prev;
    });
  }, [category, selected, modStructure]);

  /**
   * **父模组页聚合**：选中带子模组的模组时，把它自己 + 全部后代的条目合并成一份
   * 虚拟缓存，另建「条目 → 来源子模组」表（行尾标签 + 点击跳转用）。
   *
   * 为什么在前端做而不是后端：实测 1.83 发行版 101 份缓存合计 **6.0 MB**，最大一族
   * `hota` 的 28 个子模组合计 **0.7 MB** —— 点一次父模组的代价可忽略，且能按父
   * modId 复用；后端预聚合会让父缓存成倍膨胀，而绝大多数用户从不点父模组页。
   *
   * 叶子模组不做聚合（返回 `null`）：沿用既有的单 mod 缓存路径，不凭空造对象。
   */
  useEffect(() => {
    if (category !== "mods" || selected?.category !== "mods" || !modStructure) {
      setModAggregate(null);
      return;
    }
    const selfKey = modIdKey(selected.modId);
    // 逐级收集后代（顺带记下每个子模组的多语言名：来源标签不必再查一次索引）
    const descendants: { modId: string; name: Record<string, string> }[] = [];
    const seenKeys = new Set<string>([selfKey]);
    const walk = (key: string, depth: number) => {
      if (depth > 8) return; // 异常数据里的自引用 / 环
      for (const child of modStructure.childrenOf.get(key) ?? []) {
        const ck = modIdKey(child.modId);
        if (seenKeys.has(ck)) continue;
        seenKeys.add(ck);
        descendants.push({ modId: child.modId, name: child.name });
        walk(ck, depth + 1);
      }
    };
    walk(selfKey, 0);
    if (descendants.length === 0) {
      setModAggregate(null);
      return;
    }
    let cancelled = false;
    void (async () => {
      const ids = [selected.modId, ...descendants.map((d) => d.modId)];
      const caches = await Promise.all(
        ids.map(async (mid) => {
          const hit = cacheRef.current.get(mid);
          if (hit) return hit;
          const c = await loadModCache(mid);
          if (c) putModCache(cacheRef.current, mid, c);
          return c;
        }),
      );
      if (cancelled) return;
      const selfCache = caches[0] ?? {};
      // 父模组自己的元数据（modMeta / defines / resources）原样保留：详情页标题、
      // 描述、版本都取自这里
      const cache: Record<string, unknown> = { ...selfCache };
      const sources: Record<string, { modId: string; label: string }> = {};
      for (const cat of AGG_CATS) {
        const merged: unknown[] = [];
        const seenIds = new Set<string>();
        caches.forEach((c, i) => {
          for (const item of (c?.[cat] as unknown[]) ?? []) {
            const o = item as Record<string, unknown>;
            // 跨 mod 补丁引用没有自己的数据（见 `loadAllCategories` 同款过滤）
            if (o.ref) continue;
            const id = String(o.id ?? o.factionId ?? "");
            if (!id || seenIds.has(id)) continue;
            seenIds.add(id);
            merged.push(o);
            // 自己（i === 0）的条目不加来源标签；同 id 时父模组优先，也落在这一支
            if (i > 0) {
              sources[`${cat}:${id}`] = {
                modId: ids[i],
                label:
                  pickName(modNames[modIdKey(ids[i])], contentLang) ||
                  pickName(descendants[i - 1].name, contentLang) ||
                  ids[i],
              };
            }
          }
        });
        if (merged.length > 0) cache[cat] = merged;
      }
      // `defines` 是"**本 mod 自己**定义了什么"，聚合后必须按"任意后代定义了"重算：
      // 否则父模组自己没定义生物的类目会被整节隐藏（子模组的生物就看不见了）。
      const selfDefines = selfCache.defines as Record<string, boolean> | undefined;
      if (selfDefines && typeof selfDefines === "object") {
        const defines: Record<string, boolean> = { ...selfDefines };
        for (const cat of AGG_CATS) {
          if (!defines[cat] && ((cache[cat] as unknown[]) ?? []).length > 0) defines[cat] = true;
        }
        cache.defines = defines;
      }
      // 资源表 / 共享图标是游戏级数据：父模组缓存里缺时从后代借一份（否则详情页
      // 的花费图标、法术图标会整片空掉）
      if (!cache.resources) {
        const withRes = caches.find((c) => c?.resources);
        if (withRes) cache.resources = withRes.resources;
      }
      if (!cache.sharedIcons) {
        const withIcons = caches.find((c) => c?.sharedIcons);
        if (withIcons) cache.sharedIcons = withIcons.sharedIcons;
      }
      setModAggregate({ modId: selected.modId, cache, sources });
    })();
    return () => {
      cancelled = true;
    };
  }, [category, selected, modStructure, modNames, contentLang]);

  // 切换类目/搜索时重置分页窗口
  useEffect(() => {
    setVisibleCount(300);
  }, [category, query]);

  /**
   * 返回后把中栏滚到恢复的高亮行（对齐 VCMI `WikiWindow.cpp:1836 elementList->scrollTo(foundIdx)`）。
   *
   * 时序陷阱：中栏内容由 `loadAllCategories()` **异步**填充。`setCategory` 之后、装载完成之前，
   * `entries` 仍是上一个类目的数据 —— 此刻既找不到目标行，也不代表「目标不在列表里」，
   * 必须等 `loadedCategory` 追平 `category` 再定位（见 `loadedCategory` 的说明，勿用
   * `entries[0].category` 推断："上一个类目恰好为空" 时会误判成目标丢失）。
   * 行数超出分页窗口（`visibleCount`）时先扩容，等下一次 render 再滚。
   */
  useEffect(() => {
    if (!pendingListScrollRef.current) return;
    if (loadedCategory !== category) return; // 列表尚未装载完当前类目 → 继续等
    // 模组类是树形扁平行：按**可见行**定位，否则折叠子树里的目标永远找不到
    const idx =
      category === "mods"
        ? modRows.findIndex((r) => r.entry.modId === selected?.modId && r.entry.id === selected?.id)
        : filtered.findIndex((e) => e.modId === selected?.modId && e.id === selected?.id);
    if (idx < 0) {
      // 模组类目：目标可能只是还被折叠在子树里（祖先展开是上一轮 effect 里的状态更新），
      // 等下一次 render 再试；其余类目 / 模组确实不在列表（数据缺失或被跨 mod 去重）则放弃。
      if (category === "mods" && selected && entries.some((e) => e.modId === selected.modId)) return;
      pendingListScrollRef.current = false;
      return;
    }
    if (idx >= visibleCount) {
      setVisibleCount(idx + 300);
      return;
    }
    pendingListScrollRef.current = false;
    listScrollRef.current?.querySelector('[data-wiki-active="true"]')?.scrollIntoView({ block: "nearest" });
  }, [filtered, modRows, entries, selected, visibleCount, category, loadedCategory]);

  /**
   * 切换类目后**按类目记忆 / 恢复**中栏滚动位置。
   *
   * ## 语义（2026-10-17 确认）
   * - **没打开过的类目** → 从顶部开始；
   * - **打开过、重新激活的类目** → 回到上次离开的位置（每个类目各记各的）；
   * - 链接跳转 / 返回（`pendingListScrollRef`）优先滚到目标行，此时**不使用**记忆值。
   *
   * ## 为什么必须显式实现（此前这两条都是副作用）
   * 中栏是**跨类目复用的同一个滚动容器**（见下方 `ref={listScrollRef}` 的 `div`），
   * DOM 节点不变 → 浏览器保留 `scrollTop`：
   * - 没打开过的类目要走真装载，期间渲染 4 行骨架 → 内容高度骤降把 `scrollTop` **夹**到 0，
   *   看着像"回到顶部"，实则是被夹出来的 —— 一旦装载变快或去掉骨架屏，这条就漂；
   * - 打开过的类目命中 `catEntriesRef`、条目整体替换且中间没有 paint → 位置保留（恰好是想要的）。
   *
   * 现在两条都由代码显式决定，不依赖渲染时序的巧合。
   *
   * 位置说明：必须排在 `filtered` 之后 —— 恢复前可能要先按记忆位置把分页窗口开够。
   */
  // ① 切类目 → 标记"待恢复"。必须排在 ② 之前：同一轮提交里先置位，② 才能在同一 tick 命中。
  useLayoutEffect(() => {
    pendingRestoreRef.current = category;
  }, [category]);

  // ② 等当前类目的条目真的渲染出来后再写回记忆位置
  useLayoutEffect(() => {
    if (pendingRestoreRef.current === null) return; // 已恢复过 / 已被跳转接管 → 不再打扰用户后续的滚动
    // 跳转 / 返回排在前面（见上方「返回后把中栏滚到恢复的高亮行」）：本次不恢复，
    // 并**顺手取消**本次激活的恢复资格，否则后续 `visibleCount` 变大时会把记忆位置硬塞回来。
    if (pendingListScrollRef.current) {
      pendingRestoreRef.current = null;
      return;
    }
    if (pendingRestoreRef.current !== category) return;
    if (loadedCategory !== category) return; // 列表尚未换成当前类目 → 继续等
    const el = listScrollRef.current;
    if (!el) return;
    const saved = listScrollMemoRef.current.get(category) ?? 0;
    if (saved <= 0) {
      el.scrollTop = 0;
      pendingRestoreRef.current = null;
      return;
    }
    // 记忆位置可能落在分页窗口之外（新类目会把窗口复位到 300 条）→ 先把窗口开够，
    // 下一次 render 再写入；直接写会被当前内容高度夹掉。
    const rendered = Math.min(rowCount, visibleCount);
    if (rendered > 0 && saved > el.scrollHeight - el.clientHeight && visibleCount < rowCount) {
      const rowH = el.scrollHeight / rendered;
      const need = Math.ceil(saved / Math.max(rowH, 1)) + 50;
      setVisibleCount((c) => Math.min(Math.max(c, need), rowCount));
      return;
    }
    el.scrollTop = saved;
    pendingRestoreRef.current = null;
  }, [category, loadedCategory, visibleCount, rowCount]);

  // ⚠️ **未就绪时只渲染蒙层，不渲染任何内容**（早返回）：
  // 被蒙层遮住的内容照样会渲染、跑 effect、拉数据（列表、图片、甚至按需合成预览），
  // 用户却完全看不见 —— 纯浪费，首次构建期间还会造成前端卡顿（实测）。
  // 只有这一处蒙层、没有"内容 + 蒙层"并存，才真正省下这些开销。
  //
  // 两级闸门，对应两段不同的等待：
  // ① `!ready` —— 后台**构建**缓存中（`WikiBuildOverlay`：真实三段进度条）；
  // ② `!bootReady` —— 缓存已在磁盘上，正在**装载**（`LoadingOverlay`：遍历 572 个
  //    mod 的 cache.json，数秒；无真实进度可报，只给 spinner + 说明）。
  // ② 曾是空缺：`ready` 跃迁后直接渲染空 UI（左栏全 0 + "未找到匹配条目"），
  // 用户读作"没有数据"（2026-10-21 真机实测投诉）。
  if (!ready) {
    return (
      <div className="relative flex h-full p-3">
        <WikiBuildOverlay deps={["data", "assets"]} />
      </div>
    );
  }
  if (!bootReady) {
    return (
      <div className="relative flex h-full p-3">
        <LoadingOverlay title={t.wiki.loading} hint={t.wiki.loadingHint} />
      </div>
    );
  }

  return (
    <WikiPopupProvider
      contentLang={contentLang}
      t={t}
      cacheDir={cacheDir}
      getModCache={(modId) => cacheRef.current.get(modId)}
      getEntry={lookupEntry}
      onNavigate={navigateTo}
    >
      <div className="relative flex h-full gap-3 p-3">
      {h3Missing && (
        <div className="absolute inset-x-3 top-3 z-30 rounded-lg border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs leading-relaxed text-amber-200">
          {t.wiki.h3DataMissing}
        </div>
      )}
      {/* 左栏：类目 */}
      <aside className="w-[190px] flex-none rounded-xl border border-slate-800/60 bg-slate-900/60 p-2">
        <p className="px-2 pb-2 text-xs font-semibold uppercase tracking-wider text-slate-500">
          {t.wiki.categories}
        </p>
        <nav className="space-y-0.5">
          {CATEGORIES.map((c) => (
            <button
              key={c.key}
              onClick={() => {
                setCategory(c.key);
                setSelected(null);
                setQuery("");
                // 筛选随类目清空：维度按类目定义，沿用上一个类目的选择没有意义
                setFilters({});
              }}
              className={cn(
                "flex w-full items-center justify-between rounded-lg px-3 py-2 text-sm transition-colors",
                category === c.key
                  ? "border-l-2 border-blue-500 bg-slate-800/70 text-white"
                  : "text-slate-400 hover:bg-slate-800/50 hover:text-white",
              )}
            >
              <span>{c.label(t)}</span>
              {counts[c.key] !== undefined && (
                <span className="rounded bg-slate-800 px-1.5 text-[11px] text-slate-400">
                  {counts[c.key]}
                </span>
              )}
            </button>
          ))}
        </nav>
      </aside>

      {/* 中栏：条目 + 搜索 */}
      <section className="flex w-[300px] flex-none flex-col overflow-hidden rounded-xl border border-slate-800/60 bg-slate-900/60">
        <div className="border-b border-slate-800/60 p-2">
          {/* 头部：输入框（flex-1）+ 固宽工具槽。槽位宽度**不随类目变化** ——
              筛选维度按类目变化的部分全部收进浮层（见 WikiFilterPanel），
              以保证任何类目下输入框宽度与列表上沿都恒定。 */}
          <div className="flex items-center gap-1.5">
            <div className="group relative min-w-0 flex-1">
              <Search className="pointer-events-none absolute left-2.5 top-2.5 h-4 w-4 text-slate-500" />
              <input
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder={t.wiki.searchPlaceholder}
                className={cn(
                  "w-full rounded-lg border border-slate-700 bg-slate-800/70 py-2 pl-8 text-sm text-slate-100 outline-none placeholder:text-slate-500 focus:border-blue-500",
                  // 有内容时右侧留出 X 图标的位置
                  query ? "pr-8" : "pr-3",
                )}
              />
              {/* 清空按钮：**仅在有输入时出现**（没内容可清时不占位、不分散注意），
                  hover / focus 时才显形 —— 与输入框的 hover 语义一致，不额外引入常驻元素。
                  VCMI 的 CTextInput 没有这个按钮（它的做法是搜索框旁一个常驻 hint 标签，
                  见 WikiWindow.cpp:1032-1034），这里是我们补的可用性改进。 */}
              {query && (
                <button
                  type="button"
                  onClick={() => setQuery("")}
                  title={t.wiki.searchClear}
                  aria-label={t.wiki.searchClear}
                  className="absolute right-1.5 top-1/2 flex h-6 w-6 -translate-y-1/2 items-center justify-center rounded text-slate-500 opacity-0 transition-opacity hover:text-slate-200 focus:opacity-100 group-hover:opacity-100 group-focus-within:opacity-100"
                >
                  <X className="h-3.5 w-3.5" />
                </button>
              )}
            </div>
            <WikiFilterPanel
              key={category}
              specs={WIKI_FILTERS[category]}
              // 基准是**搜索后、筛选前**的条目：维度选项与计数都基于它（见 WikiFilterPanel 的联动说明）
              entries={searched}
              filters={filters}
              onChange={(k, v) => setFilters((f) => ({ ...f, [k]: v }))}
              onReset={() => setFilters({})}
              contentLang={contentLang}
              factionNames={factionNames}
              modNames={modNames}
              sortSpec={sortSpecs[category]}
              sortCategory={category}
              onSortChange={(spec) => {
                setSortSpecs((prev) => {
                  const next = { ...prev, [category]: spec };
                  // 持久化放在 updater 里：functional update 保证连续切换时不丢中间态。
                  // 幂等写，重复调用无害。
                  writeWikiSort(next);
                  return next;
                });
                // 内存重排：不重新装载（省 572 个 cache.json 的 32MB + 解析）。
                //
                // ⚠️ 必须**换数组引用**，不能原地 sort：`searched` / `filtered` 都是 useMemo，
                // 按引用比较依赖 —— 原地排序内容变了但引用不变，memo 不失效 → 列表不刷新
                // （2026-10-20 实测"点了排序列表没反应"就是这个原因）。
                // `entries` 与 `catEntriesRef` 里的数组本就是同一引用，故两处都要换成新的。
                const cur = catEntriesRef.current;
                if (cur) {
                  const reordered = [...(cur.get(category) ?? [])];
                  sortList(category, reordered, spec, factionNames);
                  cur.set(category, reordered);
                  setEntries(reordered);
                }
                // 排序变了，恢复的滚动位置会指向另一个条目 → 直接回顶部
                listScrollRef.current?.scrollTo({ top: 0 });
              }}
              t={t}
            />
          </div>
        </div>
        <div
          ref={listScrollRef}
          className="min-h-0 flex-1 overflow-y-auto"
          onScroll={(ev) => {
            const el = ev.currentTarget;
            // 记下当前类目的滚动位置（切走再切回时按它恢复）。
            // ⚠️ 只在用户可见的滚动期间记录：`loading` 时渲染的是 4 行骨架，内容变矮会把
            // `scrollTop` 夹到 0 —— 那是滚动容器的高度限制，不是用户的位置，不能覆盖记忆值。
            if (!loading) listScrollMemoRef.current.set(category, el.scrollTop);
            if (el.scrollTop + el.clientHeight >= el.scrollHeight - 80) {
              setVisibleCount((c) => (c < rowCount ? c + 300 : c));
            }
          }}
        >
          {loading ? (
            <div className="space-y-2 p-3">
              {[0, 1, 2, 3].map((i) => (
                <div key={i} className="h-10 animate-pulse rounded-lg bg-slate-800/60" />
              ))}
            </div>
          ) : rowCount === 0 ? (
            <p className="p-4 text-sm text-slate-500">{t.wiki.empty}</p>
          ) : (
            visibleRows.map((row) => {
              const e = row.entry;
              // 词汇表常见"只有中文翻译"的 mod：英文界面按任意可用语言回退，
              // 否则会显示 `vcmi.wiki.glossary.proverb.name` 这类翻译 key。
              const name =
                (e.category === "glossary"
                  ? pickNameAny(e.name, contentLang)
                  : pickName(e.name, contentLang)) ||
                pickName(e.name, "english") ||
                e.id;
              // 副标题语义化（对齐 VCMI WikiListItem：英雄→职业、生物→等级、宝物→等级、法术→学派、Mod→类型、其余→来源 mod）
              const ed = e.data as Record<string, unknown>;
              let sub = "";
              if (e.category === "heroes") {
                sub = pickName((ed.className as Record<string, string>) ?? {}, contentLang) || String(ed.class ?? "");
              } else if (e.category === "creatures") {
                sub = Number(ed.level) > 0 ? `T${ed.level}` : "";
              } else if (e.category === "artifacts") {
                // 等级：与「宝物等级」筛选项共用同一份实现（见 filters.ts 的 artifactClassLabel），
                // 避免"列表显示的等级"与"筛选项文字"各写一遍而漂移
                sub = artifactClassLabel(ed.class, t);
              } else if (e.category === "spells" || e.category === "skills") {
                // VCMI 法术/技能列表行只有图标+名称，无副标题
                sub = "";
              } else if (e.category === "factions") {
                sub = pickName((ed.description as Record<string, string>) ?? {}, contentLang);
              } else if (e.category === "buildings") {
                // 建筑条目来自哪个城镇/阵营
                sub =
                  pickName((ed.factionName as Record<string, string>) ?? {}, contentLang) ||
                  String(ed.factionId ?? "");
              } else if (e.category === "terrains") {
                // 地形：显示移动消耗（负数 = 不可通行）+ 本土城镇，代替无意义的来源 modId
                const mc = Number(ed.moveCost ?? -1);
                sub = mc < 0 ? t.wiki.terrainDetail.impassable : `${t.wiki.terrainDetail.moveCost} ${mc}%`;
              } else if (e.category === "glossary") {
                // VCMI 的 glossary 列表行只有名称，无副标题（`WikiWindow.cpp` 未填 subtitle）
                sub = "";
              } else if (e.category === "mods") {
                // 模组类型多语言（与 VCMI modType 枚举对齐，见 i18n typeLabels）
                const rawType = String(ed.type ?? "");
                sub = (t.mods.typeLabels as Record<string, string>)[rawType] ?? rawType;
              } else {
                // skills / 其余
                sub = e.modId;
              }
              const alignment = e.category === "factions" ? String(ed.alignment ?? "") : "";
              // 右键气泡文本（VCMI `WikiListItem::showPopupWindow` 显示的就是副标题，
              // 我们换成"描述类"字段）：副标题（T6 / 狂化人 / 中级宝物 / 北境）是**分类
              // 信息**，作为气泡没有信息量 —— 用户右键一行想知道的是"这东西是什么"。
              // 取值口径与各类目的详情页保持一致（含两种历史数据形态的兼容）：
              //   factions / creatures / artifacts / buildings → description
              //   heroes → biography
              //   spells  → levelDescriptions.basic（原版是纯字符串，需剥 {标题} 前缀）
              //   skills  → levels.basic
              // 全部取不到时回退条目名，保证永不空挂（有内容才挂触发器）。
              const levelText = (v: unknown) => {
                const raw =
                  typeof v === "string"
                    ? v
                    : pickName((v as Record<string, string>) ?? {}, contentLang);
                // 剥掉 H3 富文本的 {标题} 标记（与 SpellDetail 同口径）
                return raw.replace(/^\{[^}]*\}\s*/, "").trim();
              };
              const descField =
                e.category === "heroes"
                  ? ed.biography
                  : e.category === "spells"
                    ? (ed.levelDescriptions as Record<string, unknown> | undefined)?.basic
                    : e.category === "skills"
                      ? (ed.levels as Record<string, unknown> | undefined)?.basic
                      : e.category === "factions" ||
                          e.category === "creatures" ||
                          e.category === "artifacts" ||
                          e.category === "buildings"
                        ? ed.description
                        : undefined;
              const tip = levelText(descField) || name;
              return (
                <EntryRow
                  key={`${e.modId}:${e.id}`}
                  entry={e}
                  cacheDir={cacheDir}
                  name={name}
                  sub={sub}
                  tip={tip}
                  alignment={alignment}
                  active={selected?.id === e.id && selected?.modId === e.modId}
                  depth={row.depth}
                  hasChildren={row.hasChildren}
                  expanded={expandedMods.has(modIdKey(e.modId))}
                  onToggleExpand={() =>
                    setExpandedMods((prev) => {
                      const next = new Set(prev);
                      const k = modIdKey(e.modId);
                      if (next.has(k)) next.delete(k);
                      else next.add(k);
                      return next;
                    })
                  }
                  onClick={() => {
                    // 点父行 = 选中 + 展开（折叠只交给 chevron，避免"点一下又收起来"）
                    if (row.hasChildren) {
                      setExpandedMods((prev) =>
                        prev.has(modIdKey(e.modId))
                          ? prev
                          : new Set(prev).add(modIdKey(e.modId)),
                      );
                    }
                    pushAndSelect(e);
                  }}
                />
              );
            })
          )}
        </div>
      </section>

      {/* 右栏：详情。**两层结构**：外层卡片只做定位（返回按钮挂这里，不滚动），内层才是滚动核 */}
      <section className="relative flex min-w-0 flex-1 flex-col overflow-hidden rounded-xl border border-slate-800/60 bg-slate-900/40">
        {/* 返回按钮（对齐 VCMI `WikiWindow.cpp:793-801` 的 backButton：位置固定、不显示级数、
            无历史时不出现）。
            ⚠️ 必须挂**卡片层**而非滚动核内：挂在滚动核里会随内容滚走 ——
            城镇详情页很长，滚下去恰好是最想返回的时候，按钮却已滚出视口。
            目标条目名只进 `title`（tooltip），不进布局：否则名字会压在当前详情标题上方，
            既容易被误读成"当前条目"，又会随每次跳转在同一位置反复换字。 */}
        {navStack.length > 0 && (
          <button
            type="button"
            onClick={navigateBack}
            className="absolute right-3 top-3 z-10 rounded-lg border border-slate-700/60 bg-slate-800/80 px-2.5 py-1 text-xs text-slate-300 hover:bg-slate-700 hover:text-slate-100"
            title={`${t.wiki.sections.back}: ${pickName(navStack[navStack.length - 1].name, contentLang) || navStack[navStack.length - 1].id}`}
          >
            ← {t.wiki.sections.back}
          </button>
        )}
        <div ref={detailScrollRef} className="min-h-0 flex-1 overflow-y-auto">
          {selected ? (
            <EntryDetail
              key={`${selected.modId}:${selected.id}`}
              entry={selected}
              contentLang={contentLang}
              t={t}
              cacheDir={cacheDir}
              // 带子模组的模组页用**聚合缓存**（自身 + 全部后代）；叶子模组 / 其它类目
              // 沿用单 mod 缓存。聚合结果未就绪时先显示自身那份，随后自动补全。
              modCache={
                selected.category === "mods" && modAggregate?.modId === selected.modId
                  ? modAggregate.cache
                  : (modCacheMap[selected.modId] ?? cacheRef.current.get(selected.modId))
              }
              // 行尾「来源子模组」标签 + 点击跳转目标（仅聚合态有值）
              sourceLabels={
                selected.category === "mods" && modAggregate?.modId === selected.modId
                  ? modAggregate.sources
                  : undefined
              }
              getEntry={lookupEntry}
              onNavigate={navigateTo}
            />
          ) : (
            // 未选中条目时的右栏空态。对齐 VCMI `vcmi.wiki.content.placeholder`
            // （`WikiWindow.cpp:1195`：未选中时 contentBox->setText(...)）——
            // 官方用的是**指令式句子**而非名词/图标：它同时回答"这面板是干嘛的"
            // 和"我该做什么"。此前这里显示 `t.wiki.search`（"搜索"），那是搜索框的
            // 提示词，放在内容区只会让人以为要在这里输入。
            <div className="flex h-full items-center justify-center px-6 text-center text-sm leading-relaxed text-slate-600">
              {t.wiki.contentPlaceholder}
            </div>
          )}
        </div>
      </section>

    </div>
  </WikiPopupProvider>
  );
}


/** 索引 `contentCount` 中代表"百科里有实体"的键（对齐 VCMI `collectScopes` 的七类）。 */
const MOD_CONTENT_KEYS = [
  "factions",
  "heroes",
  "creatures",
  "artifacts",
  "spells",
  "skills",
  // 原版法术 / 技能在计数里单独拆成 builtin（见 `collectCounts`），它们同样是 core 的真实条目
  "spellBuiltin",
  "skillBuiltin",
] as const;

/**
 * 剔除**没有任何百科内容**的模组（对齐 VCMI `WikiModContent.cpp:97-112` 的 `modHasContent`）。
 *
 * 判定用索引里的 `contentCount`（构建期已排除 `ref` 补丁引用），而不是"缓存存在与否"：
 * 字体包 / 翻译 / 纯 AI 调参 / 地图对象补丁的 cache.json 一样存在，但七类实体全为 0。
 * 实测该发行版 572 个 mod 里 309 个属于这类噪音。
 *
 * ⚠️ 自身无内容、但**后代有内容**的父模组必须保留：树形列表靠它挂住子模组
 * （实测 `hota` 本身七类全空，28 个城镇 / 扩展子模组却都有内容）。
 */
function pruneModsWithoutContent(list: WikiEntry[]): WikiEntry[] {
  const hasOwn = new Map<string, boolean>();
  const childrenOf = new Map<string, string[]>();
  for (const e of list) {
    const key = modIdKey(e.modId);
    const cc = (e.data as Record<string, unknown>).contentCount as
      | Record<string, number>
      | undefined;
    // ⚠️ 旧索引 / 外部 mock 可能**根本没有** `contentCount`：此时必须保守地当成"有内容"，
    // 否则整个模组列表会被清空（与 `visibleKeys === null` 不筛选同一条失效原则）。
    hasOwn.set(
      key,
      !cc || typeof cc !== "object" ? true : MOD_CONTENT_KEYS.some((k) => (cc[k] ?? 0) > 0),
    );
  }
  for (const e of list) {
    const key = modIdKey(e.modId);
    const pid = modIdKey(String((e.data as Record<string, unknown>).parentId ?? ""));
    if (!pid || pid === key || !hasOwn.has(pid)) continue;
    const arr = childrenOf.get(pid);
    if (arr) arr.push(key);
    else childrenOf.set(pid, [key]);
  }
  const keep = new Map<string, boolean>();
  const visit = (key: string, depth: number): boolean => {
    const hit = keep.get(key);
    if (hit !== undefined) return hit;
    keep.set(key, false); // 先占位：异常数据里的环不至于递归不返回
    let ok = hasOwn.get(key) ?? false;
    if (!ok && depth < 16) {
      for (const child of childrenOf.get(key) ?? []) {
        if (visit(child, depth + 1)) {
          ok = true;
          break;
        }
      }
    }
    keep.set(key, ok);
    return ok;
  };
  return list.filter((e) => visit(modIdKey(e.modId), 0));
}

function collectCounts(mods: { contentCount?: Record<string, number> }[]): Record<string, number> {
  const counts: Record<string, number> = {};
  // 原版法术/技能（builtin）注入到每个 mod，全局只计一次
  let builtinSpells = 0;
  let builtinSkills = 0;
  // 地形是游戏级数据（每个 mod 缓存里都是同一份），按 mod 求和会得到 N× 重复计数
  let terrains = 0;
  for (const m of mods) {
    const cc = m.contentCount ?? {};
    for (const k of Object.keys(cc)) {
      if (k === "spellBuiltin") {
        builtinSpells = Math.max(builtinSpells, cc[k] ?? 0);
        continue;
      }
      if (k === "skillBuiltin") {
        builtinSkills = Math.max(builtinSkills, cc[k] ?? 0);
        continue;
      }
      if (k === "terrains") {
        terrains = Math.max(terrains, cc[k] ?? 0);
        continue;
      }
      // 跨 mod 引用诊断计数（见后端 contentCount.ref）：不参与任何类目展示
      if (k === "ref") continue;
      counts[k] = (counts[k] ?? 0) + (cc[k] ?? 0);
    }
  }
  counts["spells"] = (counts["spells"] ?? 0) + builtinSpells;
  counts["skills"] = (counts["skills"] ?? 0) + builtinSkills;
  if (terrains > 0) counts["terrains"] = terrains;
  return counts;
}
