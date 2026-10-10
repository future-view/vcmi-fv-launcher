import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Info, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n";
import {
  realmConfigDir,
  getModsCacheDir,
  generateTownPreview,
  readModSettingsJson,
  readTownCards,
  readModMetas,
  type TownCard,
  type InstalledMod,
} from "@/lib/tauri";
import { collectEnabledIds } from "@/lib/realm/enabledMods";
import { useWikiStagesDone, WikiBuildOverlay } from "@/shared/WikiBuildOverlay";
import { CORE_TOWN_IDS } from "@/lib/realm/coreTowns";
import { richToPlainText } from "@/lib/html";
import {
  buildEntry,
  fullImgUrl,
  loadModCache,
  loadWikiIndex,
  modIdKey,
  pickName,
  type WikiIndex,
} from "@/lib/modcache";
import type { ModInfo } from "@/lib/modScanner";
import { EntryDetail } from "@/features/wiki/detail/EntryDetail";
import { HdIcon } from "@/shared/HdIcon";
import { WikiPopupProvider } from "@/features/wiki/detail/WikiPopup";
import { DetailDrawer } from "@/features/mods/DetailDrawer";
import { AnchorSidebar } from "@/shared/AnchorSidebar";
import { MapsTab } from "@/features/realms/editor/tabs/MapsTab";
import { realmDescription, realmName } from "@/lib/realm/intro";
import type { Realm } from "@/lib/realm/types";

/** modType 分组顺序（§9.3.4）：城镇优先，其余沿用 ModView 既有分组的相对顺序。
 * 值对齐 VCMI mod.json 的 modType 枚举（与 `t.mods.typeLabels` 的 key 一致）。 */
const REALM_MODTYPE_ORDER = [
  "Town",
  "Creatures",
  "Heroes",
  "Artifacts",
  "Spells",
  "Skills",
  "Objects",
  "Mechanics",
  "Maps",
  "Campaigns",
  "Expansion",
  "Graphical",
  "Interface",
  "Music",
  "Sounds",
  "Resources",
  "Templates",
  "Translation",
  "Compatibility",
  "AI",
  "Core",
  "Demo",
  "Test",
  "Other",
];

/** 参与「有无内容」判定的类目（`contentCount` 里真正对应百科实体的键） */
const CONTENT_KEYS = [
  "factions", "creatures", "heroes", "spells", "artifacts", "skills", "buildings",
] as const;

/** 诊断用键：不参与任何类目展示（前端 `collectCounts` 同口径跳过） */
const DIAGNOSTIC_KEYS = new Set(["spellBuiltin", "skillBuiltin", "ref"]);

/** 是否为原版 scope（`core` 或 `core.<faction>`）。
 * 不能用 `startsWith("core")` —— 会误伤名为 `corecraft` 之类的第三方模组。 */
const isCoreId = (id: string) => {
  const k = modIdKey(id);
  return k === "core" || k.startsWith("core.");
};

/** 原版（`core` 与 9 个原版城镇）由「显示原版内容」开关控制是否纳入 */
const CORE_MOD_IDS = ["core"];

/** 侧栏「全部」项的伪类型 id（不会与真实 modType 冲突） */
const FILTER_ALL = "all";
/** 侧栏「地图包」项的伪类型 id */
const FILTER_MAPS = "maps";

/** 领域信息页（设计 §9.3，v2）。
 *
 * 关键口径：
 * 1. **启用集合 = `preset.mods` ⊕ `preset.settings` 中 true 的子模组**（VCMI 权威语义，
 *    见 `references/vcmi/lib/modding/ModManager.cpp::getActiveMods()`）。只读 `mods` 会漏掉
 *    全部子模组——实测 86 个启用城镇里只有 32 个是顶层城镇。
 * 2. **跳过无内容的模组**：启用集合里绝大多数是容器 / 兼容补丁（`contentCount` 全 0）。
 * 3. **首屏不阻塞**：A/B 与侧栏只依赖 `index.json` + `modSettings.json`（数百 KB），
 *    多语言名与城镇卡片走批量命令（`read_mod_metas` / `read_town_cards`）后台补齐。
 * 4. **侧栏即筛选器**（v2.1 改）：与工具条的类型 chips 曾是同一份数据、重复且占一整行；
 *    现合并为「侧栏 = 唯一类型控件」（含「全部」）。顺带取消了 scroll-spy 与锚点跳转：
 *    scroll-spy 在 sticky 工具条遮挡下会选错分组，而筛选天然只渲染一组，不再需要定位。
 * 5. 工具条**移出滚动区**：分组标题永远不会被 sticky 条压住。 */
export function InfoBody({ r }: { r: Realm }) {
  const { t } = useI18n();
  const lang = t.uiLang;
  const typeLabel = (type: string) =>
    (t.mods.typeLabels as unknown as Record<string, string>)[type] ?? type;

  const [state, setState] = useState<{
    loading: boolean;
    cacheDir: string | null;
    index: WikiIndex | null;
    /** 领域预设的启用集合（根模组 + settings 中 true 的子模组），原始大小写 */
    ids: string[];
    /** 共享全局配置时，启用组合来自全局的哪个预设（用于页头标注） */
    sourcePreset: string | null;
    isGlobal: boolean;
  }>({ loading: true, cacheDir: null, index: null, ids: [], sourcePreset: null, isGlobal: false });

  /** 模组元数据（多语言名/描述）：modId(小写) -> meta。后台补齐，不阻塞首屏 */
  const [metas, setMetas] = useState<Record<string, InstalledMod["meta"]>>({});
  /** 城镇卡片（后端批量返回最小字段） */
  const [cards, setCards] = useState<TownCard[]>([]);
  /** 本轮城镇卡片是否已返回：用来区分「还在加载」与「该模组确实没有城镇数据」 */
  const [cardsLoaded, setCardsLoaded] = useState(false);
  /** 按需合成的城镇预览图：key = `modId/factionId` → 图片相对路径 */
  const [townPreviews, setTownPreviews] = useState<Record<string, string>>({});
  /** 工具条 */
  const [q, setQ] = useState("");
  /** 侧栏当前的筛选（`all` / modType / `maps`）——侧栏与正文的唯一类型控件 */
  const [typeFilter, setTypeFilter] = useState<string>(FILTER_ALL);
  /** 原版内容开关：默认取领域配置（`realm.json` 的 `info.showCore`，由领域编辑者预设），玩家可临时改 */
  const [showCore, setShowCore] = useState<boolean>(() => r.info?.showCore === true);
  const [collapsed, setCollapsed] = useState<Record<string, boolean>>({});
  /** 城镇详情抽屉：当前查看的城镇（所属模组 id + 原始 faction 对象） */
  const [detail, setDetail] = useState<{ modId: string; obj: Record<string, unknown> } | null>(null);
  /** 模组详情抽屉（ⓘ）：由 index + meta 合成的 ModInfo */
  const [modDetail, setModDetail] = useState<ModInfo | null>(null);

  // 切换领域时套用该领域的默认开关与清空的筛选（组件不随 r.id 重挂载）
  useEffect(() => {
    setShowCore(r.info?.showCore === true);
    setQ("");
    setTypeFilter(FILTER_ALL);
    setCollapsed({});
    // 只在领域变化时重置：不跟随 r.info 变化，否则玩家手动改动会被 store 刷新冲掉
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [r.id]);

  /* ----------------------------- 阶段 0：首屏三件套（小数据，立即出内容） ----------------------------- */

  useEffect(() => {
    let alive = true;
    (async () => {
      const [dir, idx] = await Promise.all([getModsCacheDir(), loadWikiIndex()]);
      const isGlobal = r.dirs?.configPath === "global";
      // 领域共享全局配置时 `realm_config_dir` 返回的**就是全局配置目录**，
      // 因此这里读到的 modSettings.json 与 VCMI 实际使用的一致。
      const cfgDir = await realmConfigDir(r.id);
      const settings = cfgDir ? await readModSettingsJson(cfgDir) : null;
      const { ids, presetId } = collectEnabledIds(settings, r, isGlobal);
      if (alive) {
        setState({ loading: false, cacheDir: dir, index: idx, ids, sourcePreset: presetId, isGlobal });
      }
    })();
    return () => {
      alive = false;
    };
  }, [r.id, r.dirs?.configPath]);

  /* ----------------------------- 派生：过滤 / 分组 / 搜索 ----------------------------- */

  const infoByKey = useMemo(() => {
    const m = new Map<string, WikiIndex["mods"][number]>();
    state.index?.mods.forEach((x) => m.set(modIdKey(x.modId ?? ""), x));
    return m;
  }, [state.index]);

  /** 索引里的模组名 → 本地化文本（后端写入的是多语言对象；旧缓存/外部 mock 可能是单串） */
  const indexName = useCallback(
    (id: string) => {
      const n = infoByKey.get(modIdKey(id))?.name;
      if (!n) return "";
      return typeof n === "object" ? pickName(n, lang) : String(n);
    },
    [infoByKey, lang],
  );

  /** 名称（多语言）：modMeta → index.name → id */
  const nameOf = useCallback(
    (id: string) => {
      const meta = metas[modIdKey(id)];
      const fromMeta = meta?.name ? pickName(meta.name, lang) : "";
      if (fromMeta) return fromMeta;
      return indexName(id) || id;
    },
    [metas, indexName, lang],
  );

  /** 描述（多语言，已归一为纯文本）：modMeta → 空 */
  const descOf = useCallback(
    (id: string) => {
      const meta = metas[modIdKey(id)];
      if (!meta?.description) return "";
      return richToPlainText(pickName(meta.description, lang));
    },
    [metas, lang],
  );

  /** 描述原文（用于详情抽屉的富文本渲染，保留 Markdown/HTML 原样） */
  const rawDescOf = useCallback(
    (id: string) => {
      const meta = metas[modIdKey(id)];
      return meta?.description ? pickName(meta.description, lang) : "";
    },
    [metas, lang],
  );

  /** 启用集合 → 需要展示的模组（跳过无内容的容器/兼容补丁；开关控制原版） */
  const { visibleIds, hiddenCount } = useMemo(() => {
    const wanted: string[] = [];
    const seen = new Set<string>();
    const add = (id: string) => {
      const k = modIdKey(id);
      if (seen.has(k)) return;
      seen.add(k);
      wanted.push(id);
    };
    let hidden = 0;
    for (const id of state.ids) {
      const info = infoByKey.get(modIdKey(id));
      if (!info) continue;
      const cc = info.contentCount ?? {};
      const total = CONTENT_KEYS.reduce((a, k) => a + (cc[k] ?? 0), 0);
      if (total > 0 || info.type === "Town") add(id);
      else hidden += 1;
    }
    // 原版（core / core.<faction>）：不在预设里，由开关单独纳入
    if (showCore) {
      for (const id of [...CORE_MOD_IDS, ...CORE_TOWN_IDS]) {
        if (infoByKey.has(modIdKey(id))) add(id);
      }
    }
    return { visibleIds: wanted, hiddenCount: hidden };
  }, [state.ids, infoByKey, showCore]);

  /** 搜索命中（modId + 多语言名 + 描述纯文本） */
  const matches = useCallback(
    (id: string) => {
      const kw = q.trim().toLowerCase();
      if (!kw) return true;
      return (
        id.toLowerCase().includes(kw) ||
        nameOf(id).toLowerCase().includes(kw) ||
        descOf(id).toLowerCase().includes(kw)
      );
    },
    [q, nameOf, descOf],
  );

  const filteredIds = useMemo(() => visibleIds.filter((id) => matches(id)), [visibleIds, matches]);

  /** 按 modType 分组（城镇优先；组内原版优先） */
  const groups = useMemo(() => {
    const m = new Map<string, string[]>();
    for (const id of filteredIds) {
      const type = infoByKey.get(modIdKey(id))?.type || "Other";
      const list = m.get(type);
      if (list) list.push(id);
      else m.set(type, [id]);
    }
    const ordered = [...m.keys()].sort((a, b) => {
      const ia = REALM_MODTYPE_ORDER.indexOf(a);
      const ib = REALM_MODTYPE_ORDER.indexOf(b);
      return (ia < 0 ? REALM_MODTYPE_ORDER.length : ia) - (ib < 0 ? REALM_MODTYPE_ORDER.length : ib);
    });
    return ordered.map((type) => {
      // 「原版优先显示」：core* 排在同组最前
      const list = (m.get(type) ?? []).slice().sort((a, b) => {
        const ca = isCoreId(a) ? 0 : 1;
        const cb = isCoreId(b) ? 0 : 1;
        return ca - cb;
      });
      return { type, ids: list };
    });
  }, [filteredIds, infoByKey]);

  /* ----------------------------- 阶段 1：后台补齐多语言名/描述 ----------------------------- */

  useEffect(() => {
    if (state.loading || visibleIds.length === 0) return;
    let alive = true;
    (async () => {
      const m = await readModMetas(visibleIds);
      // 后端按传入 id 原样回键，这里统一按下比较键存，避免大小写不一致导致查不到
      const norm: Record<string, InstalledMod["meta"]> = {};
      for (const [k, v] of Object.entries(m)) norm[modIdKey(k)] = v;
      if (alive) setMetas((prev) => ({ ...prev, ...norm }));
    })();
    return () => {
      alive = false;
    };
  }, [state.loading, visibleIds]);

  /* ----------------------------- 阶段 2：城镇卡片（一次 IPC 取回最小字段） ----------------------------- */

  const townIds = useMemo(() => {
    const ids = new Set<string>();
    for (const g of groups) {
      if (g.type !== "Town") continue;
      // 折叠 / 未选中的城镇组不取卡片（拍板①：折叠组不加载）
      if (typeFilter !== FILTER_ALL && typeFilter !== "Town") continue;
      if (collapsed.Town) continue;
      g.ids.forEach((id) => ids.add(id));
    }
    return [...ids];
  }, [groups, collapsed, typeFilter]);

  // 就绪门（放在最靠前的数据 effect 之前：effect 需要它做守卫与依赖）。
  const ready = useWikiStagesDone(["data"]);

  useEffect(() => {
    // ⚠️ 未就绪不加载：此刻早返回只渲染蒙层，加载了也没人看（且 `readTownCards`
    // 依赖 data 产物，提前跑会拿到空结果并**因为依赖不变而不重跑**）。
    if (!ready) return;
    if (townIds.length === 0) {
      setCards([]);
      setCardsLoaded(true);
      return;
    }
    let alive = true;
    setCardsLoaded(false);
    (async () => {
      const cs = await readTownCards(townIds);
      if (!alive) return;
      setCards(cs);
      setCardsLoaded(true);
    })();
    return () => {
      alive = false;
    };
  }, [townIds, ready]);

  /** modId → 该模组的城镇卡片（保持分组内的原始顺序） */
  const cardsByMod = useMemo(() => {
    const m = new Map<string, TownCard[]>();
    for (const c of cards) {
      const k = modIdKey(c.modId);
      const arr = m.get(k);
      if (arr) arr.push(c);
      else m.set(k, [c]);
    }
    return m;
  }, [cards]);

  /* ----------------------------- 阶段 3：缺失预览图按需合成（并发 4） ----------------------------- */

  const attemptedPreviews = useRef<Set<string>>(new Set());
  useEffect(() => {
    const missing = cards.filter((c) => {
      if (c.preview) return false;
      // 原版 `core*` 只产出底图：其 `raw.json` 的 `rawConfigs` 没有 `towns`
      // （实测 core.castle 只有 mod/files/spells/translation/heroClasses），
      // `generate_town_preview` 必然失败。跳过可避免每次打开信息页白跑 9 次后端合成。
      if (isCoreId(c.modId)) return false;
      const key = `${c.modId}/${c.factionId}`;
      return !attemptedPreviews.current.has(key);
    });
    if (missing.length === 0) return;
    let alive = true;
    (async () => {
      // 并发 4：合成是后端重活（解码 DEF + 叠加 + webp 编码），串行会让首屏等太久
      const CONCURRENCY = 4;
      let cursor = 0;
      const worker = async () => {
        while (alive && cursor < missing.length) {
          const m = missing[cursor++];
          const key = `${m.modId}/${m.factionId}`;
          attemptedPreviews.current.add(key);
          const res = await generateTownPreview(m.modId, m.factionId);
          if (!alive) return;
          if (res?.ok && res.preview) {
            setTownPreviews((p) => ({ ...p, [key]: res.preview as string }));
          }
        }
      };
      await Promise.all(
        Array.from({ length: Math.min(CONCURRENCY, missing.length) }, () => worker()),
      );
    })();
    return () => {
      alive = false;
    };
  }, [cards]);

  /* ----------------------------- 城镇详情抽屉（点击卡片时按需读完整缓存） ----------------------------- */

  const townCacheRef = useRef<Record<string, Record<string, unknown> | null>>({});
  const openTown = async (c: TownCard) => {
    let cache = townCacheRef.current[c.modId];
    if (!cache) {
      cache = await loadModCache(c.modId);
      townCacheRef.current[c.modId] = cache;
    }
    const factions = (cache?.factions as Record<string, unknown>[] | undefined) ?? [];
    const obj = factions.find((f) => String(f.id ?? f.factionId ?? "") === c.factionId) ?? null;
    if (obj) setDetail({ modId: c.modId, obj });
  };

  /** 由 index + meta 合成 DetailDrawer 需要的 ModInfo（避免为此调 `list_installed_mods`） */
  const toModInfo = (id: string): ModInfo => {
    const info = infoByKey.get(modIdKey(id));
    const meta = metas[modIdKey(id)];
    const names = meta?.name ?? {};
    const descs = meta?.description ?? {};
    const langKeys = new Set([...Object.keys(names), ...Object.keys(descs)]);
    const i18n: Record<string, { name?: string; description?: string }> = {};
    langKeys.forEach((k) => {
      i18n[k] = { name: names[k], description: descs[k] };
    });
    return {
      id,
      diskName: info?.diskName ?? id,
      parentId: info?.parentId ?? id,
      // 与 `nameOf` 同口径取索引名（多语言对象需 pickName，直接 String() 会得到 `[object Object]`）
      name: indexName(id) || id,
      localizedName: nameOf(id),
      desc: rawDescOf(id),
      type: info?.type ?? "Other",
      version: meta?.version ?? undefined,
      author: meta?.author ?? undefined,
      depends: (meta?.depends as string[] | undefined) ?? [],
      softDepends: [],
      conflicts: (meta?.conflicts as string[] | undefined) ?? [],
      modPath: "",
      hasContentZip: false,
      contentZipEncrypted: false,
      isSubmod: id.includes("."),
      subs: [],
      factions:
        info?.type === "Town"
          ? (cardsByMod.get(modIdKey(id)) ?? []).map((c) => c.factionId)
          : undefined,
      i18n: langKeys.size > 0 ? i18n : undefined,
    };
  };

  /* ----------------------------- 渲染 ----------------------------- */

  const name = realmName(r, lang);
  const desc = realmDescription(r, lang);
  const mapEntries = r.maps?.entries ?? [];

  const sidebarItems = useMemo(() => {
    const items = [{ id: FILTER_ALL, label: t.mods.all, count: filteredIds.length }];
    for (const g of groups) items.push({ id: g.type, label: typeLabel(g.type), count: g.ids.length });
    items.push({ id: FILTER_MAPS, label: t.realm.infoGroupMaps, count: mapEntries.length });
    return items;
    // typeLabel 依赖 t.uiLang，已随 t 变化
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [groups, filteredIds.length, mapEntries.length, t]);

  if (state.loading) {
    return <div className="py-6 text-sm text-slate-400">{t.common.loading}</div>;
  }

  const showMaps = typeFilter === FILTER_ALL || typeFilter === FILTER_MAPS;
  const shownGroups =
    typeFilter === FILTER_ALL ? groups : groups.filter((g) => g.type === typeFilter);
  const nothingToShow =
    filteredIds.length === 0 && (!showMaps || mapEntries.length === 0);

  /** 非城镇模组的紧凑行：统计只显示 >0 且有翻译的类目 */
  const modRow = (id: string) => {
    const info = infoByKey.get(modIdKey(id));
    const cc = info?.contentCount ?? {};
    const labels = Object.entries(cc)
      .filter(([k, v]) => !DIAGNOSTIC_KEYS.has(k) && v > 0)
      .map(([k, v]) => {
        const lab = (t.wiki as unknown as Record<string, string>)[k];
        return lab ? `${lab} ${v}` : null;
      })
      .filter((x): x is string => !!x);
    const d = descOf(id);
    return (
      <div key={id} className="rounded-lg border border-slate-700/60 bg-slate-900/60 px-3 py-2">
        <div className="flex items-center gap-2.5">
          <span className="h-7 w-7 flex-none rounded bg-slate-800/80" />
          <span className="min-w-0 flex-1 truncate text-sm font-medium text-slate-100" title={nameOf(id)}>
            {nameOf(id)}
          </span>
          {labels.length > 0 && <span className="shrink-0 text-xs text-slate-400">{labels.join(" · ")}</span>}
          <button
            type="button"
            title={t.realm.infoModDetail}
            onClick={() => setModDetail(toModInfo(id))}
            className="shrink-0 rounded p-1 text-slate-500 hover:bg-slate-800 hover:text-slate-200"
          >
            <Info className="h-3.5 w-3.5" />
          </button>
        </div>
        {d && (
          <p className="mt-1 line-clamp-2 pl-9 text-xs text-slate-400" title={d}>
            {d}
          </p>
        )}
      </div>
    );
  };

  /** 城镇卡片（§9.3.5）：预览图铺满 + 城镇名称 + 描述（最多两行，无描述则不渲染该行）。
   * 数据未到达时渲染骨架占位；已到达但该模组没有城镇数据时给占位卡片，不留永久骨架。 */
  const townCards = (id: string) => {
    const cs = cardsByMod.get(modIdKey(id));
    if (!cs || cs.length === 0) {
      // 尚未返回 → 骨架
      if (!cardsLoaded) {
        return [
          <div key={id} className="animate-pulse overflow-hidden rounded-xl border border-slate-700/60 bg-slate-900/60">
            <div className="h-28 w-full bg-slate-800/60" />
            <div className="space-y-1.5 p-3">
              <div className="h-3 w-2/3 rounded bg-slate-800/80" />
              <div className="h-2.5 w-full rounded bg-slate-800/60" />
            </div>
          </div>,
        ];
      }
      // 已返回但没有城镇数据（如被标为 Town 的生物补充包）→ 占位卡片（模组粒度）
      return [
        <div key={id} className="overflow-hidden rounded-xl border border-slate-700/60 bg-slate-900/60">
          <div className="flex h-28 w-full items-center justify-center bg-slate-800/60 px-2 text-center text-xs text-slate-500">
            {id}
          </div>
          <div className="p-3">
            <div className="truncate text-sm font-medium text-slate-100" title={nameOf(id)}>
              {nameOf(id)}
            </div>
            {descOf(id) && (
              <p className="mt-1 line-clamp-2 text-xs text-slate-400">{descOf(id)}</p>
            )}
          </div>
        </div>,
      ];
    }
    return cs.map((c) => {
      const cname = pickName(c.name ?? undefined, lang) || c.factionId || c.modId;
      const rawDesc = c.description as Record<string, string> | string | undefined;
      const cdesc = richToPlainText(typeof rawDesc === "string" ? rawDesc : pickName(rawDesc, lang));
      // 优先合成图 → 已合成的结果 → 城镇底图。
      // 原版 `core.<faction>` 只有 `background`（没有 `preview`，也无法合成），
      // 缺了这层回退就会退化成"占位块 + faction id"（见 DetailDrawer 的同款口径）。
      const rel = c.preview ?? townPreviews[`${c.modId}/${c.factionId}`] ?? c.background ?? "";
      const img = rel ? fullImgUrl(state.cacheDir, rel) : "";
      return (
        <div
          key={`${c.modId}/${c.factionId}`}
          onClick={() => void openTown(c)}
          className="cursor-pointer overflow-hidden rounded-xl border border-slate-700/60 bg-slate-900/60 transition hover:border-blue-400/70 hover:shadow-[0_0_16px_rgba(96,165,250,0.2)]"
        >
          {img ? (
            <HdIcon src={img} alt="" className="h-28 w-full object-cover" loading="lazy" draggable={false} />
          ) : (
            <div className="flex h-28 w-full items-center justify-center bg-slate-800/60 px-2 text-center text-xs text-slate-500">
              {c.factionId || c.modId}
            </div>
          )}
          <div className="p-3">
            <div className="truncate text-sm font-medium text-slate-100" title={cname}>
              {cname}
            </div>
            {/* 无描述时不渲染该行（原版城镇本来就没有描述，避免整屏「暂无描述」噪音） */}
            {cdesc && (
              <p className="mt-1 line-clamp-2 text-xs text-slate-400" title={cdesc}>
                {cdesc}
              </p>
            )}
          </div>
        </div>
      );
    });
  };

  const groupSection = (type: string, ids: string[]) => {
    const isTown = type === "Town";
    // 单类型筛选下强制展开：否则侧栏点进去先是一片折叠头，还得再点一次
    const open = typeFilter !== FILTER_ALL || !collapsed[type];
    return (
      <section key={type} className="mb-5">
        <button
          type="button"
          disabled={typeFilter !== FILTER_ALL}
          onClick={() => setCollapsed((c) => ({ ...c, [type]: !c[type] }))}
          className="mb-3 flex w-full items-center gap-2 text-left text-sm font-semibold text-slate-200 disabled:cursor-default"
        >
          <span
            className={cn(
              "inline-block transition-transform",
              !open ? "-rotate-90" : "",
              typeFilter !== FILTER_ALL && "opacity-40",
            )}
          >
            ▾
          </span>
          {typeLabel(type)}
          <span className="text-xs font-normal text-slate-500">({ids.length})</span>
        </button>
        {open &&
          (isTown ? (
            <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4">
              {ids.flatMap((id) => townCards(id))}
            </div>
          ) : (
            <div className="space-y-1.5">{ids.map((id) => modRow(id))}</div>
          ))}
      </section>
    );
  };

  // ⚠️ **未就绪时只渲染蒙层，不渲染内容**（早返回）：本页会按需合成城镇预览图
  // （`generateTownPreview`）与拉取城镇卡片，蒙层期间这些渲染/请求用户都看不见 —— 纯浪费。
  if (!ready) {
    return (
      <div className="relative flex h-full min-h-0 w-full">
        <WikiBuildOverlay deps={["data"]} />
      </div>
    );
  }

  return (
    // 城镇详情抽屉复用百科的 `EntryDetail`，其 VCMI 式右键弹窗依赖这个 Provider（与百科页一致）
    <WikiPopupProvider
      contentLang={lang}
      t={t}
      cacheDir={state.cacheDir ?? ""}
      getModCache={(modId) => townCacheRef.current[modId] ?? {}}
    >
      <div className="relative flex h-full min-h-0 w-full">
        <AnchorSidebar items={sidebarItems} activeId={typeFilter} onSelect={setTypeFilter} />

        <div className="flex min-w-0 flex-1 flex-col">
          {/* C 工具条：搜索 + 原版开关 + 展开/收起。
              放在滚动区**之外**（不 sticky）：这样分组标题永远不会被它压住，
              也不需要为锚点跳转做「减去工具条高度」的偏移计算。 */}
          <div className="flex-none space-y-2 border-b border-slate-800/70 px-6 py-3">
            <input
              value={q}
              onChange={(e) => setQ(e.target.value)}
              placeholder={t.realm.infoSearch}
              className="w-full rounded-lg border border-slate-700/60 bg-slate-900/60 px-3 py-1.5 text-sm text-slate-200 placeholder:text-slate-500 focus:border-blue-500/60 focus:outline-none"
            />
            <div className="flex flex-wrap items-center justify-between gap-2 text-xs">
              <label className="flex cursor-pointer items-center gap-1.5 text-slate-400">
                <input
                  type="checkbox"
                  checked={showCore}
                  onChange={(e) => setShowCore(e.target.checked)}
                  className="h-3.5 w-3.5 accent-indigo-500"
                />
                {t.realm.infoShowCore}
              </label>
              {typeFilter === FILTER_ALL && (
                <div className="flex gap-2">
                  <button
                    type="button"
                    onClick={() => setCollapsed({})}
                    className="rounded border border-slate-700/50 px-2 py-1 text-slate-400 hover:text-white"
                  >
                    {t.realm.infoExpandAll}
                  </button>
                  <button
                    type="button"
                    onClick={() => setCollapsed(Object.fromEntries(groups.map((g) => [g.type, true])))}
                    className="rounded border border-slate-700/50 px-2 py-1 text-slate-400 hover:text-white"
                  >
                    {t.realm.infoCollapseAll}
                  </button>
                </div>
              )}
            </div>
          </div>

          <section className="min-h-0 flex-1 overflow-y-auto px-6 py-5">
            {/* A 基础信息卡片：名称 / 版本 / 作者 + 描述（最多两行） */}
            <section className="mb-4 rounded-xl border border-slate-700/60 bg-slate-900/50 p-4">
              <div className="grid grid-cols-1 gap-x-6 gap-y-1.5 text-sm sm:grid-cols-3">
                <div className="flex min-w-0 items-baseline gap-2">
                  <span className="flex-none text-slate-500">{t.realm.name}</span>
                  <span className="min-w-0 flex-1 truncate font-medium text-slate-200" title={name}>
                    {name || "—"}
                  </span>
                </div>
                <div className="flex items-baseline gap-2">
                  <span className="flex-none text-slate-500">{t.realm.version}</span>
                  <span className="min-w-0 flex-1 truncate text-slate-200" title={r.version}>
                    {r.version || "—"}
                  </span>
                </div>
                <div className="flex items-baseline gap-2">
                  <span className="flex-none text-slate-500">{t.realm.author}</span>
                  <span className="min-w-0 flex-1 truncate text-slate-200" title={r.author}>
                    {r.author || "—"}
                  </span>
                </div>
              </div>
              <p className="mt-2.5 line-clamp-2 text-sm leading-relaxed text-slate-300" title={desc}>
                {desc || "—"}
              </p>
            </section>

            {/* B 配置来源标注条：仅共享全局配置时显示（启用组合来自全局预设，改领域改不动它） */}
            {state.isGlobal && state.sourcePreset && (
              <div className="mb-4 rounded-lg border border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-300">
                ⓘ {t.realm.infoConfigGlobal.replace("{preset}", state.sourcePreset)}
              </div>
            )}

            {nothingToShow ? (
              <p className="text-sm text-slate-500">
                {visibleIds.length === 0 && mapEntries.length === 0
                  ? t.realm.infoNoContent
                  : t.realm.infoEmpty}
              </p>
            ) : (
              <>
                {shownGroups.map((g) => groupSection(g.type, g.ids))}

                {showMaps && mapEntries.length > 0 && (
                  <section className="mb-5">
                    <div className="mb-3 flex items-center gap-2 text-sm font-semibold text-slate-200">
                      {t.realm.infoGroupMaps}
                      <span className="text-xs font-normal text-slate-500">({mapEntries.length})</span>
                    </div>
                    {/* 只读分栏（确认点 9 / 原型屏四）：复用 MapsTab 的树/列表/详情，无管理按钮；
                        filterLang 开启时按「目标语言 + unknown」过滤显示，与游戏内可见口径一致 */}
                    <div className="h-[420px] overflow-hidden rounded-lg border border-slate-700/60 bg-slate-900/30 p-2">
                      <MapsTab
                        t={t}
                        realmId={r.id}
                        readOnly
                        applyFilter
                        filterLang={r.maps?.filterLang ?? false}
                        filterLangTarget={r.maps?.filterLangTarget ?? ""}
                      />
                    </div>
                  </section>
                )}

                {hiddenCount > 0 && typeFilter === FILTER_ALL && (
                  <p className="mt-2 text-xs text-slate-600">
                    {t.realm.infoHiddenZero.replace("{n}", String(hiddenCount))}
                  </p>
                )}
              </>
            )}
          </section>
        </div>
      </div>

      {/* 城镇详情抽屉：点击城镇卡片打开，复用百科的 `EntryDetail` 渲染城镇完整信息
          （名称 + 合成图 + 描述 + 兵种 / 英雄等表）。
          透明遮罩只用于「点击左侧关闭」：不改变右侧面板的可见性，但会吃掉页面交互。 */}
      {detail && (
        <>
          <div className="fixed inset-0 z-[149] bg-slate-950/40" onClick={() => setDetail(null)} />
          {/* 宽度取 560：生物表共 11 列（图片 / 名称 / 花费 + 8 项属性）约需 520px 内容宽，
              加内边距后 560 刚好放得下，避免出现横向滚动条（滚动查看表格体验很差）。 */}
          <div className="fixed right-0 top-0 z-[150] flex h-full w-[560px] max-w-[92vw] flex-col border-l border-slate-700/70 bg-slate-950/95 shadow-2xl">
            <div className="flex flex-none items-center justify-between gap-2 border-b border-slate-800 px-4 py-2.5">
              <span className="truncate text-sm font-semibold text-slate-100">
                {pickName(detail.obj.name as Record<string, string> | undefined, lang) ||
                  t.realm.realmInfo}
              </span>
              <button
                type="button"
                onClick={() => setDetail(null)}
                title={t.common.close}
                className="shrink-0 rounded p-1 text-slate-400 hover:bg-slate-800 hover:text-slate-100"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
            <div className="min-h-0 flex-1 overflow-y-auto p-4">
              <EntryDetail
                entry={buildEntry(
                  (townCacheRef.current[detail.modId] ?? {}) as Record<string, unknown>,
                  "factions",
                  detail.obj,
                )}
                contentLang={lang}
                t={t}
                cacheDir={state.cacheDir}
                modCache={townCacheRef.current[detail.modId] ?? undefined}
              />
            </div>
          </div>
        </>
      )}

      {/* 模组详情抽屉（ⓘ）：复用 mods 页的 DetailDrawer，`readOnly` 隐藏加入按钮 */}
      {modDetail && (
        <DetailDrawer
          mod={modDetail}
          selected={new Set()}
          onToggle={() => undefined}
          onClose={() => setModDetail(null)}
          readOnly
        />
      )}
    </WikiPopupProvider>
  );
}

/* ----------------------------- 启用集合推导（VCMI 权威语义） ----------------------------- */

