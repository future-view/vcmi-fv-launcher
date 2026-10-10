import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { Search } from "lucide-react";
import { useI18n } from "@/i18n";
import { cn } from "@/lib/utils";

import type { LangDict } from "@/i18n/types";
import { getModsCacheDir, getRuntimeInfo, isTauri } from "@/lib/tauri";
import { visibleModKeySet } from "@/lib/modcache/buildTargets";
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
  const [entries, setEntries] = useState<WikiEntry[]>([]);
  const [selected, setSelected] = useState<WikiEntry | null>(null);
  // 导航历史栈（对齐 VCMI wiki navHistory）：内容区链接/列表点选前压栈，返回按钮回退
  const [navStack, setNavStack] = useState<WikiEntry[]>([]);
  // 「全部模组」选中后按需加载该 mod 的完整缓存（含多语言 description / version），
  // 供右侧详情栏展示描述（索引条目本身不含 description，需读 cache.json）。
  const [modCacheMap, setModCacheMap] = useState<Record<string, Record<string, unknown>>>({});
  const [loading, setLoading] = useState(false);
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
        setEntries(result);
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
      for (const mid of modIds) {
        let mod = cacheRef.current[mid];
        if (!mod) {
          mod = (await loadModCache(mid)) ?? undefined;
          if (mod) cacheRef.current[mid] = mod;
        }
        if (!mod) continue;
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
          result.sort((a, b) => Number((a.data as Record<string, unknown>).index ?? 0) - Number((b.data as Record<string, unknown>).index ?? 0));
        }
        // 建筑：中立建筑置顶，避免被 370+ 条城镇建筑淹没（用户实测"看不到中立建筑"）
        if (cat === "buildings") {
          result.sort((a, b) => {
            const an = String((a.data as Record<string, unknown>).factionId ?? "") === "neutral" ? 0 : 1;
            const bn = String((b.data as Record<string, unknown>).factionId ?? "") === "neutral" ? 0 : 1;
            return an - bn;
          });
        }
        // 左栏计数 = 实际聚合条目数（原版法术等已跨 mod 去重，与列表一致）
        setCounts((prev) => ({ ...prev, [cat]: result.length }));
        setEntries(result);
      setLoading(false);
      return result;
    },
    [indexed, visibleKeys],
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

  // 返回上一条目
  const navigateBack = useCallback(() => {
    if (navStack.length === 0) return;
    const prev = navStack[navStack.length - 1];
    setNavStack((s) => s.slice(0, -1));
    setSelected(prev);
  }, [navStack]);

  // 联动跳转：根据 (modId, category, id) 定位条目并切换详情
  const navigateTo = useCallback(
    async (target: WikiLinkTarget) => {
      // 1. 当前已加载列表里查找
      const hit = entries.find(
        (e) => e.category === target.category && e.modId === target.modId && e.id === target.id,
      );
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

  // 搜索过滤
  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return entries;
    return entries.filter((e) => {
      const zh = pickName(e.name, "chinese").toLowerCase();
      const en = pickName(e.name, "english").toLowerCase();
      return zh.includes(q) || en.includes(q) || e.id.toLowerCase().includes(q);
    });
  }, [entries, query]);

  // 切换类目/搜索时重置分页窗口
  useEffect(() => {
    setVisibleCount(300);
  }, [category, query]);

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
          <div className="relative">
            <Search className="pointer-events-none absolute left-2.5 top-2.5 h-4 w-4 text-slate-500" />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder={t.wiki.searchPlaceholder}
              className="w-full rounded-lg border border-slate-700 bg-slate-800/70 py-2 pl-8 pr-3 text-sm text-slate-100 outline-none placeholder:text-slate-500 focus:border-blue-500"
            />
          </div>
        </div>
        <div
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
                // 等级（class）：数字 0-4 → 珍藏/宝物/...；字符串 → class+PascalCase 键本地化
                const cnum = Number(ed.class);
                sub =
                  Number.isFinite(cnum) && String(ed.class ?? "") !== ""
                    ? [t.wiki.artifact.classTreasure, t.wiki.artifact.classMinor, t.wiki.artifact.classMajor, t.wiki.artifact.classRelic, t.wiki.artifact.classScroll][cnum] ??
                      t.wiki.artifact.classOther
                    : (() => {
                        const raw = String(ed.class ?? "");
                        const key =
                          "class" + raw.charAt(0).toUpperCase() + raw.slice(1).toLowerCase();
                        return (
                          (t.wiki.artifact as Record<string, string>)[key] ?? raw
                        );
                      })();
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
              return (
                <EntryRow
                  key={`${e.modId}:${e.id}`}
                  entry={e}
                  cacheDir={cacheDir}
                  name={name}
                  sub={sub}
                  alignment={alignment}
                  active={selected?.id === e.id && selected?.modId === e.modId}
                  onClick={() => pushAndSelect(e)}
                />
              );
            })
          )}
        </div>
      </section>

      {/* 右栏：详情 */}
      <section className="relative min-w-0 flex-1 overflow-y-auto rounded-xl border border-slate-800/60 bg-slate-900/40">
        {/* 返回按钮（对齐 VCMI wiki navHistory/backButton） */}
        {navStack.length > 0 && (
          <button
            type="button"
            onClick={navigateBack}
            className="absolute right-3 top-3 z-10 rounded-lg border border-slate-700/60 bg-slate-800/80 px-2.5 py-1 text-xs text-slate-300 hover:bg-slate-700 hover:text-slate-100"
            title={t.wiki.sections.back}
          >
            ← {navStack.length}
          </button>
        )}
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
      </section>

    </div>
  </WikiPopupProvider>
  );
}

/** 中栏条目行：VCMI WikiListItem::showPopupWindow —— 右键弹 subtitle 文本气泡 */
function EntryRow({
  entry,
  cacheDir,
  name,
  sub,
  alignment,
  active,
  onClick,
}: {
  entry: WikiEntry;
  cacheDir: string | null;
  name: string;
  sub: string;
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
      // VCMI WikiListItem::showPopupWindow：条目行右键 = subtitle 文本气泡（居中于光标）
      {...(sub ? triggers((p) => popup.showText(sub, p)) : {})}
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
