import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { Search } from "lucide-react";
import { useI18n } from "@/i18n";
import { cn } from "@/lib/utils";

import type { LangDict } from "@/i18n/types";
import { getModsCacheDir, getRuntimeInfo, isTauri } from "@/lib/tauri";
import { visibleModKeySet } from "@/lib/modcache/buildTargets";
import { vcmiLangTag } from "@/lib/vcmiLangs";
import {
  buildEntry,
  contentLangKey,
  fullImgUrl,
  loadModCache,
  loadWikiIndex,
  modIdKey,
  pickName,
  type WikiCategory,
  type WikiEntry,
  type WikiLinkTarget,
} from "@/lib/modcache";
import { useWikiStagesDone, WikiBuildOverlay } from "@/shared/WikiBuildOverlay";
import { EntryDetail } from "@/features/wiki/detail/EntryDetail";
import { WikiPopupProvider } from "@/features/wiki/detail/WikiPopup";
import { useWikiPopup, usePopupTriggers } from "@/features/wiki/detail/WikiPopupContext";
import { HdIcon } from "@/shared/HdIcon";
import { WikiFilterPanel } from "@/features/wiki/WikiFilterPanel";
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
  { key: "mods", label: (t) => t.wiki.allMods },
];

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
  // 阵营 id → 多语言名（cache.json 顶层 `factionNames`，跨 mod 合并）：供「按阵营筛选」显示名字
  const [factionNames, setFactionNames] = useState<Record<string, Record<string, string>>>({});
  // modId（小写归一）→ 多语言名（取自索引条目名）：供「来源模组」筛选显示名字
  const [modNames, setModNames] = useState<Record<string, Record<string, string>>>({});
  const [entries, setEntries] = useState<WikiEntry[]>([]);
  const [selected, setSelected] = useState<WikiEntry | null>(null);
  // 导航历史栈（对齐 VCMI wiki navHistory）：内容区链接/列表点选前压栈，返回按钮回退
  const [navStack, setNavStack] = useState<WikiEntry[]>([]);
  // 「全部模组」选中后按需加载该 mod 的完整缓存（含多语言 description / version），
  // 供右侧详情栏展示描述（索引条目本身不含 description，需读 cache.json）。
  const [modCacheMap, setModCacheMap] = useState<Record<string, Record<string, unknown>>>({});
  const [loading, setLoading] = useState(false);
  /**
   * 中栏列表**已装载完成**的类目（`loadCategory` 异步，装载期间 `entries` 还是上一个类目的数据）。
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
  const [counts, setCounts] = useState<Record<string, number>>({});
  // 任一 mod 缓存缺失原版 H3 数据文件：百科图标/背景将无图（提示用户补放 Data/*.lod）
  const [h3Missing, setH3Missing] = useState(false);
  const cacheRef = useRef<Record<string, Record<string, unknown>>>({});
  // 右栏详情滚动容器：切换条目时要把滚动位置归零（见下方「切换条目后右栏回到顶部」）
  const detailScrollRef = useRef<HTMLDivElement | null>(null);
  // 中栏列表滚动容器 + 「返回后需要把列表滚到恢复行」的待办标记（见下方「返回后把中栏滚到恢复的高亮行」）
  const listScrollRef = useRef<HTMLDivElement | null>(null);
  const pendingListScrollRef = useRef(false);
  // 缓存代数：后台构建（尤其阶段二补图）完成后自增，驱动当前分类重新拉取 mod.json，
  // 否则 cacheRef 里阶段一的旧对象（图片字段为 null）会一直用到刷新页面为止。
  const [epoch, setEpoch] = useState(0);

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
    cacheRef.current = {};
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
    (a: WikiEntry, b: WikiEntry) =>
      nameCollator.compare(
        pickName(a.name, contentLang) || pickName(a.name, "english") || a.id,
        pickName(b.name, contentLang) || pickName(b.name, "english") || b.id,
      ),
    [nameCollator, contentLang],
  );

  // 按需加载目标类目的条目（聚合各 mod）
  const loadCategory = useCallback(
    async (cat: WikiCategory) => {
      setLoading(true);
      const result: WikiEntry[] = [];
      if (cat === "mods") {
        const idx = await loadWikiIndex();
        if (idx) {
          // 与「当前可见集合」求交：剔除已卸载 / 其它领域的缓存条目（索引只增不删）
          const mods = visibleKeys ? idx.mods.filter((m) => visibleKeys.has(modIdKey(m.modId))) : idx.mods;
          for (const m of mods) {
            result.push({
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
        result.sort(sortByName);
        setEntries(result);
        setLoadedCategory(cat);
        setLoading(false);
        return result;
      }
      // 只遍历「当前可见」的 mod：索引条目随时间累积（卸载后仍保留），
      // 而下面每条都要一次 IPC 读 cache.json —— 不筛选的话，开销会随使用时长单调增长。
      const modIds = Array.from(indexed).filter((id) => !visibleKeys || visibleKeys.has(modIdKey(id)));
      let processed = 0;
      // 地形是游戏级数据（**每个 mod 缓存里都是同一份**），必须跨 mod 按 id 去重；
      // nativeTowns 按 mod 分散（各 mod 只报自己的本族城镇），去重时合并城镇列表。
      // 此前按单个 mod 建 map → 每个 mod 各出 10 条，17 个 mod 就是 170 条假重复。
      const terrainAcc = new Map<string, { mod: Record<string, unknown>; obj: Record<string, unknown> }>();
      // 法术/技能/宝物/生物：跨 mod 按 id 去重（builtin/原版收敛到 core，mod 定义优先覆盖原版）
      const spellMap = new Map<string, { mod: Record<string, unknown>; item: Record<string, unknown>; builtin: boolean }>();
      const entityMap = new Map<string, { mod: Record<string, unknown>; item: Record<string, unknown>; core: boolean }>();
      // 阵营 id → 多语言名（cache.json 顶层 `factionNames`）：与分类数据同一次读取拿到，
      // 无须为取标签额外发 IPC；跨 mod 合并，后见者覆盖。
      const namesAcc: Record<string, Record<string, string>> = {};
      for (const mid of modIds) {
        let mod = cacheRef.current[mid];
        if (!mod) {
          mod = (await loadModCache(mid)) ?? undefined;
          if (mod) cacheRef.current[mid] = mod;
        }
        if (!mod) continue;
        if (mod.factionNames && typeof mod.factionNames === "object") {
          Object.assign(namesAcc, mod.factionNames as Record<string, Record<string, string>>);
        }
        const arr = (mod[cat] as unknown[]) ?? [];
        if (cat === "terrains") {
          for (const item of arr) {
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
              const out: Record<string, string> = { ...((a as Record<string, string>) ?? {}) };
              for (const [k, v] of Object.entries((b as Record<string, string>) ?? {})) {
                const old = out[k];
                if ((!old || old === id) && typeof v === "string" && v && v !== id) out[k] = v;
              }
              return out;
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
          processed += 1;
          if (processed % 24 === 0) await new Promise((r) => setTimeout(r, 0));
          continue;
        }
        // 法术/技能：builtin 已收敛到 core，按 id 去重（非 builtin 覆盖 builtin → mod 覆盖原版优先）
        if (cat === "spells" || cat === "skills") {
          for (const item of arr) {
            const o = item as Record<string, unknown>;
            // 跨 mod 引用条目（VCMI `<modId>:<entityId>` 补丁语法）：无独立数据，不进入列表
            if (o.ref) continue;
            const id = String(o.id ?? "");
            if (!id) continue;
            const builtin = !!o.builtin;
            const ex = spellMap.get(id);
            if (ex) {
              if (ex.builtin && !builtin) spellMap.set(id, { mod, item: o, builtin });
              continue;
            }
            spellMap.set(id, { mod, item: o, builtin });
          }
          processed += 1;
          if (processed % 24 === 0) await new Promise((r) => setTimeout(r, 0));
          continue;
        }
        // 宝物/生物：按 id 跨 mod 去重（mod 定义优先于 core 原版，避免同名实体重复入列）
        if (cat === "artifacts" || cat === "creatures") {
          for (const item of arr) {
            const obj = item as Record<string, unknown>;
            if (obj.ref) continue;
            const id = String(obj.id ?? obj.factionId ?? "");
            if (!id) continue;
            const isCore = mid === "core" || mid.startsWith("core.");
            const ex = entityMap.get(id);
            if (ex) {
              if (ex.core && !isCore) entityMap.set(id, { mod, item: obj, core: isCore });
              continue;
            }
            entityMap.set(id, { mod, item: obj, core: isCore });
          }
          processed += 1;
          if (processed % 24 === 0) await new Promise((r) => setTimeout(r, 0));
          continue;
        }
        // 其余类目（英雄/城镇/建筑）逐 mod 入列
        for (const item of arr) {
          const obj = item as Record<string, unknown>;
          // 跨 mod 引用条目（如 `core:dimensionDoor`、`hota.factory:factory`）是补丁引用，
          // 构建期已标记 `ref:true`；列表默认不展示（计数同样已排除）
          if (obj.ref) continue;
          result.push(buildEntry(mod, cat, obj));
        }
        // 每 24 个 mod 让出一帧，避免长任务阻塞交互（卡死防护）
        processed += 1;
        if (processed % 24 === 0) {
          await new Promise((r) => setTimeout(r, 0));
        }
        }
        // 法术/技能/宝物/生物：跨 mod 去重结果一次性入列
        if (cat === "spells" || cat === "skills") {
          for (const { mod, item } of spellMap.values()) {
            result.push(buildEntry(mod, cat, item));
          }
        }
        if (cat === "artifacts" || cat === "creatures") {
          for (const { mod, item } of entityMap.values()) {
            result.push(buildEntry(mod, cat, item));
          }
        }
        // 地形：跨 mod 去重结果一次性入列（每个 mod 只贡献本族 nativeTowns）
        if (cat === "terrains") {
          for (const { mod, obj } of terrainAcc.values()) {
            result.push(buildEntry(mod, cat, obj));
          }
        }
        // 建筑：中立建筑置顶，避免被 370+ 条城镇建筑淹没（用户实测"看不到中立建筑"）。
        // 置顶语义**优先于**名称序，组内再按显示名升序。
        if (cat === "buildings") {
          result.sort((a, b) => {
            const an = String((a.data as Record<string, unknown>).factionId ?? "") === "neutral" ? 0 : 1;
            const bn = String((b.data as Record<string, unknown>).factionId ?? "") === "neutral" ? 0 : 1;
            return an - bn || sortByName(a, b);
          });
        } else {
          // 其余分类统一按显示名升序（含地形：原按 `index` 排序是启动器自创，
          // 官方 `WikiWindow.cpp` 的地形列表同样按 `a.name < b.name`，此处对齐官方）
          result.sort(sortByName);
        }
        // 左栏计数 = 实际聚合条目数（原版法术等已跨 mod 去重，与列表一致）
        setCounts((prev) => ({ ...prev, [cat]: result.length }));
        // 阵营名映射：合并各 mod 的顶层表（渲染时按 id 取词）
        setFactionNames((prev) => ({ ...prev, ...namesAcc }));
        setEntries(result);
      setLoadedCategory(cat);
      setLoading(false);
      return result;
    },
    [indexed, visibleKeys, sortByName],
  );

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
      let mod = cacheRef.current[target.modId];
      if (!mod) {
        mod = (await loadModCache(target.modId)) ?? undefined;
        if (mod) cacheRef.current[target.modId] = mod;
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
    // mods 分支在 loadCategory 内部处理（读 index.json），同样需要加载
    loadCategory(category);
  }, [category, loadCategory, epoch]);

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

  // 「全部模组」选中后按需加载该 mod 完整缓存（含 description），供右侧栏展示
  useEffect(() => {
    if (selected?.category !== "mods") return;
    let cancelled = false;
    void loadModCache(selected.modId).then((c) => {
      if (!cancelled) setModCacheMap((m) => ({ ...m, [selected.modId]: c ?? {} }));
    });
    return () => {
      cancelled = true;
    };
  }, [selected]);

  // 搜索过滤（**只做搜索**）。
  //
  // 搜索匹配**条目的全部语言名**（`name` 是 VCMI 多语言对象，键为 chinese / tchinese / english / …），
  // 而不是枚举某几种语言：界面语言已达 5 种且会继续增加，任何"写死中/英"的口径在新增语言时
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

  // 切换类目/搜索时重置分页窗口
  useEffect(() => {
    setVisibleCount(300);
  }, [category, query]);

  /**
   * 返回后把中栏滚到恢复的高亮行（对齐 VCMI `WikiWindow.cpp:1836 elementList->scrollTo(foundIdx)`）。
   *
   * 时序陷阱：中栏内容由 `loadCategory(category)` **异步**填充。`setCategory` 之后、装载完成之前，
   * `entries` 仍是上一个类目的数据 —— 此刻既找不到目标行，也不代表「目标不在列表里」，
   * 必须等 `loadedCategory` 追平 `category` 再定位（见 `loadedCategory` 的说明，勿用
   * `entries[0].category` 推断："上一个类目恰好为空" 时会误判成目标丢失）。
   * 行数超出分页窗口（`visibleCount`）时先扩容，等下一次 render 再滚。
   */
  useEffect(() => {
    if (!pendingListScrollRef.current) return;
    if (loadedCategory !== category) return; // 列表尚未装载完当前类目 → 继续等
    const idx = filtered.findIndex((e) => e.modId === selected?.modId && e.id === selected?.id);
    if (idx < 0) {
      // 目标确实不在当前列表（数据缺失 / 被跨 mod 去重），无法定位，放弃
      pendingListScrollRef.current = false;
      return;
    }
    if (idx >= visibleCount) {
      setVisibleCount(idx + 300);
      return;
    }
    pendingListScrollRef.current = false;
    listScrollRef.current?.querySelector('[data-wiki-active="true"]')?.scrollIntoView({ block: "nearest" });
  }, [filtered, selected, visibleCount, category, loadedCategory]);

  // ⚠️ **未就绪时只渲染蒙层，不渲染任何内容**（早返回）：
  // 被蒙层遮住的内容照样会渲染、跑 effect、拉数据（列表、图片、甚至按需合成预览），
  // 用户却完全看不见 —— 纯浪费，首次构建期间还会造成前端卡顿（实测）。
  // 只有这一处蒙层、没有"内容 + 蒙层"并存，才真正省下这些开销。
  if (!ready) {
    return (
      <div className="relative flex h-full p-3">
        <WikiBuildOverlay deps={["data", "assets"]} />
      </div>
    );
  }

  return (
    <WikiPopupProvider
      contentLang={contentLang}
      t={t}
      cacheDir={cacheDir}
      getModCache={(modId) => cacheRef.current[modId]}
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
            <div className="relative min-w-0 flex-1">
              <Search className="pointer-events-none absolute left-2.5 top-2.5 h-4 w-4 text-slate-500" />
              <input
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder={t.wiki.searchPlaceholder}
                className="w-full rounded-lg border border-slate-700 bg-slate-800/70 py-2 pl-8 pr-3 text-sm text-slate-100 outline-none placeholder:text-slate-500 focus:border-blue-500"
              />
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
              t={t}
            />
          </div>
        </div>
        <div
          ref={listScrollRef}
          className="min-h-0 flex-1 overflow-y-auto"
          onScroll={(ev) => {
            const el = ev.currentTarget;
            if (el.scrollTop + el.clientHeight >= el.scrollHeight - 80) {
              setVisibleCount((c) => (c < filtered.length ? c + 300 : c));
            }
          }}
        >
          {loading ? (
            <div className="space-y-2 p-3">
              {[0, 1, 2, 3].map((i) => (
                <div key={i} className="h-10 animate-pulse rounded-lg bg-slate-800/60" />
              ))}
            </div>
          ) : filtered.length === 0 ? (
            <p className="p-4 text-sm text-slate-500">{t.wiki.empty}</p>
          ) : (
            filtered.slice(0, visibleCount).map((e) => {
              const name = pickName(e.name, contentLang) || pickName(e.name, "english") || e.id;
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
                  onClick={() => pushAndSelect(e)}
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
              modCache={modCacheMap[selected.modId] ?? cacheRef.current[selected.modId]}
              onNavigate={navigateTo}
            />
          ) : (
            <div className="flex h-full items-center justify-center text-sm text-slate-600">
              {t.wiki.search}
            </div>
          )}
        </div>
      </section>

    </div>
  </WikiPopupProvider>
  );
}

/** 中栏条目行：VCMI WikiListItem::showPopupWindow —— 右键弹文本气泡 */
function EntryRow({
  entry,
  cacheDir,
  name,
  sub,
  tip,
  alignment,
  active,
  onClick,
}: {
  entry: WikiEntry;
  cacheDir: string | null;
  name: string;
  sub: string;
  /** 右键气泡文本：描述/传记类字段，回退条目名（见 WikiView 里的取值口径） */
  tip: string;
  alignment: string;
  active: boolean;
  onClick: () => void;
}) {
  const { t } = useI18n();
  const popup = useWikiPopup();
  const triggers = usePopupTriggers();
  return (
    <button
      type="button"
      onClick={onClick}
      // 返回后中栏要滚到恢复的行：列表容器据此查询高亮行（见 scrollListToSelection）
      data-wiki-active={active ? "true" : undefined}
      // VCMI WikiListItem::showPopupWindow：条目行右键 = 文本气泡（居中于光标）。
      // 显示 `tip`（描述/传记）而不是副标题 —— 副标题是分类信息（T6 / 狂化人），
      // 作为气泡没有信息量；`tip` 一定非空（回退了条目名），故恒挂触发器。
      {...triggers((p) => popup.showText(tip, p))}
      className={cn(
        "flex w-full items-center gap-2 border-l-2 px-3 py-2 text-left transition-colors",
        active ? "border-blue-500 bg-blue-500/15" : "border-transparent hover:bg-slate-800/50",
      )}
    >
      <EntryIcon entry={entry} cacheDir={cacheDir} />
      <span className="min-w-0 flex-1">
        <span className="flex items-center justify-between gap-2">
          <span className="min-w-0 flex-1 truncate text-sm text-slate-100">{name}</span>
          {alignment ? (
            <span
              className={cn(
                "flex-none rounded px-1.5 py-0.5 text-[10px] font-medium",
                alignment.toLowerCase() === "good"
                  ? "bg-emerald-500/20 text-emerald-300"
                  : alignment.toLowerCase() === "evil"
                    ? "bg-red-500/20 text-red-300"
                    : "bg-slate-600/30 text-slate-400",
              )}
            >
              {t.wiki.alignment[alignment.toLowerCase() as keyof typeof t.wiki.alignment] ?? alignment}
            </span>
          ) : null}
        </span>
        {sub && sub !== name ? (
          <span className="mt-0.5 block truncate text-[11px] text-slate-500">{sub}</span>
        ) : null}
      </span>
    </button>
  );
}

function EntryIcon({ entry, cacheDir }: { entry: WikiEntry; cacheDir: string | null }) {
  if (!entry.image) {
    // 无图条目（如中立建筑）用名称前两字代替 id 前缀，避免整屏相同的 "NE"/"CO"
    const label = (entry.name.chinese || entry.name.english || entry.id).slice(0, 2);
    return (
      <div className="flex h-8 w-8 flex-none items-center justify-center rounded bg-slate-800 text-[10px] text-slate-500">
        {label.toUpperCase()}
      </div>
    );
  }
  return (
    <HdIcon
      src={fullImgUrl(cacheDir, entry.image)}
      alt={entry.id}
      className="h-10 w-10 flex-none rounded object-contain"
      draggable={false}
      loading="lazy"
    />
  );
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
