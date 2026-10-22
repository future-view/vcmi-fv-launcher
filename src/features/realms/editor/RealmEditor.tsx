import { useEffect, useMemo, useRef, useState } from "react";
import { ChevronDown } from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n, UI_LANG_OPTIONS } from "@/i18n";
import { StageGate } from "@/shared/StageGate";
import { realmAssetTargets } from "@/lib/modcache/realmAssets";
import { ExportRealmDialog } from "@/features/realms/editor/ExportRealmDialog";
import {
  listInstalledMods,
  listRealmMods,
  pickOpenPath,
  importRealmAsset,
  checkRealmModConflicts,
  realmDir,
  nativeMapsStatus,
  applyRealmMapsValve,
  realmAssetUrl,
  realmConfigDir,
  readModSettingsJson,
  writeRealmPresets,
  type InstalledMod,
  type RealmInstalledMod,
  type ModConflict,
} from "@/lib/tauri";
import { updateRealmSafe, setRealmMapsBlockedSafe } from "@/lib/realm/store";
import { REALM_MEDIA_ASSET_KEYS, realmMediaAssets } from "@/lib/realm/intro";
import type {
  Realm,
  RealmAssets,
  RealmBackgroundFilter,
  RealmDirs,
  RealmI18nAssets,
  RealmI18nEntry,
  RealmMapEntry,
  RealmShowcase,
} from "@/lib/realm/types";
import { type ModInfo } from "@/lib/modScanner";
import { RESERVED_MOD_IDS } from "@/lib/modScope";
import { fetchModIndex, onModIndexUpdated, type ModRepoEntry } from "@/lib/tauri";
import type { RealmModPin } from "@/lib/realm/types";
import { fromVcmiPreset, toVcmiPreset } from "@/lib/vcmiPreset";
import { solveDepClosure, solveRealmDeps, type DepClosureResult } from "@/lib/modDeps";
import { ConfirmDialog } from "@/shared/ConfirmDialog";
import { RealmIntroOverlay } from "@/features/realms/RealmIntroOverlay";
import { RealmShell } from "@/features/realms/RealmShell";
import { ShowcaseOverlay } from "@/features/realms/showcase/ShowcaseOverlay";
import { EditorProps, REALM_PRESET_ID, realmModToModInfo } from "./editorShared";
import { LanguagePicker } from "./LanguagePicker";
import { BasicTab, type BasicTabProps } from "./tabs/BasicTab";
import { AdvancedTab, type AdvancedTabProps } from "./tabs/AdvancedTab";
import { MapsTab } from "./tabs/MapsTab";
import { ModsTab, type ModsTabProps, type ModsViewMode } from "./tabs/ModsTab";
import { IntroTab, type IntroTabProps } from "./tabs/IntroTab";
import { ShowcaseTab } from "./tabs/ShowcaseTab";
import { backendErrorText } from "@/lib/backendError";

export function RealmEditor({ editing, onCancel, onSaved, showToast, registerBack }: EditorProps) {
  const { t, lang } = useI18n();
  // 数据隔离配置（§3.2.1 / B7）：5 个 dirs key 的归属 + 游戏地图开关 + 配置共享阀门。
  const defaultConfigShare = {
    settings: true,
    keybinds: true,
    persistent: true,
    mapeditor: true,
  };
  const [dirs, setDirs] = useState<RealmDirs>(
    editing.dirs
      ? {
          ...editing.dirs,
          configShare: { ...defaultConfigShare, ...(editing.dirs.configShare ?? {}) },
        }
      : {
          dataPath: "realm",
          cachePath: "global",
          configPath: "realm",
          logsPath: "global",
          savePath: "realm",
          gameMaps: "show",
          configShare: defaultConfigShare,
        },
  );
  /** 配置共享阀门：切换某文件复用全局 / 隔离领域（§配置共享）。 */
  const setShare = (k: keyof NonNullable<RealmDirs["configShare"]>, v: boolean) =>
    setDirs((d) => ({
      ...d,
      configShare: { ...(d.configShare ?? defaultConfigShare), [k]: v },
    }));
  const blockMaps = dirs.gameMaps === "hide";
  const [saving, setSaving] = useState(false);

  // 多语言（设计 §6，新模型）：全部文本放 `i18n`，默认语言也写入 `i18n[defaultLang]`；
  // 顶层 name/description/assets 仅作兜底（兼容旧读取方）。编辑器不再把 defaultLang 从 i18nMap 中剥离。
  const [defaultLang, setDefaultLang] = useState<string>(editing.defaultLang ?? lang);
  /** 当前正在编辑的语言（§9.6.1：单字段 + 语言切换器） */
  const [curLang, setCurLang] = useState<string>(editing.defaultLang ?? lang);
  // 默认语言作为 i18nMap 的一个普通条目编辑：初值优先取 i18n[defaultLang]，回退到顶层字段。
  const seedDefault: RealmI18nEntry = {
    name: editing.i18n?.[editing.defaultLang ?? lang]?.name ?? editing.name ?? "",
    description: editing.i18n?.[editing.defaultLang ?? lang]?.description ?? editing.description ?? "",
    // 标签：i18n[defaultLang] 优先、顶层兜底（与 name/description 同款回退；恒写语义见 pruneI18n）
    tags: editing.i18n?.[editing.defaultLang ?? lang]?.tags ?? editing.tags ?? [],
    // 只承接**文本类**素材：媒体是语言无关的（见下方 `media`），不再进 i18n，
    // 否则保存时会把媒体写进语言块、发布推导封面（只读顶层）拿不到 → 静默丢封面。
    assets: {
      introText: editing.i18n?.[editing.defaultLang ?? lang]?.assets?.introText ?? editing.assets?.introText,
    },
  };
  const [i18nMap, setI18nMap] = useState<Record<string, RealmI18nEntry>>(() => {
    const dl = editing.defaultLang ?? lang;
    const others = Object.fromEntries(
      Object.entries(editing.i18n ?? {}).filter(([l]) => l !== dl),
    );
    return { [dl]: seedDefault, ...others };
  });
  /** 当前编辑语言是否为「默认语言」（默认语言现在也是 i18nMap 的一个条目） */
  const isBaseLang = curLang === defaultLang;
  const curEntry: RealmI18nEntry = i18nMap[curLang] ?? {};
  /** 顶层（默认语言）字段，从 i18nMap[defaultLang] 派生，供回退显示与落盘兜底 */
  const baseEntry: RealmI18nEntry = i18nMap[defaultLang] ?? {};
  const baseName = baseEntry.name ?? "";
  const baseDesc = baseEntry.description ?? "";
  /**
   * 媒体素材（**语言无关**）：壁纸 / 封面 / 音乐 / 介绍音频 / 介绍视频只存一份、写顶层 `assets`。
   *
   * 拆分前它们跟着「当前编辑语言」写进 `i18n.<lang>.assets`，而发布推导封面只读顶层
   * （`plan_publish`），于是"只在非默认语言配过封面"会静默丢封面；且 `import_realm_asset`
   * 按 kind 固定文件名，分语言根本存不下（后传覆盖先传）。故改为单一真相源。
   * 初值走 `realmMediaAssets`：旧形状（媒体仍在语言块）也能读回来，不至于一保存就抹掉。
   */
  const [media, setMedia] = useState<RealmAssets>(() => realmMediaAssets(editing));
  /** 媒体写入：patch 里空串 = 清除该项（删除键，不写空值） */
  const patchMedia = (patch: RealmAssets) => {
    setMedia((m) => {
      const next: RealmAssets = { ...m };
      for (const [k, v] of Object.entries(patch)) {
        const key = k as keyof RealmAssets;
        if (!v) delete next[key];
        else next[key] = v;
      }
      return next;
    });
  };
  /** 当前编辑语言的介绍文本（唯一分语言的素材） */
  const curIntroText = curEntry.assets?.introText ?? "";
  /** 写入当前编辑语言的某字段（默认语言与非默认语言统一写入 i18nMap[curLang]） */
  const setCur = (patch: RealmI18nEntry) => {
    setI18nMap((m) => ({
      ...m,
      [curLang]: { ...(m[curLang] ?? {}), ...patch },
    }));
  };
  /** 写入当前编辑语言的介绍文本：空值删除该键，从而回退显示默认语言的文本 */
  const setIntroText = (v: string) => {
    const next: RealmI18nAssets = { ...(curEntry.assets ?? {}) };
    if (v) next.introText = v;
    else delete next.introText;
    setCur({ assets: next });
  };

  // 模组选择（realm-mods-management §2）：引用（user 源建链接 / game 源登记）/ 整合（复制本体）。
  // 「领域已实装」列表（`realmInstalled`，后端 `list_realm_mods` 目录扫描 + 台账补漏）是
  // 预设视图浏览区与管理视图四标签页的单一数据源。
  const [installed, setInstalled] = useState<InstalledMod[]>([]);
  const [realmInstalled, setRealmInstalled] = useState<RealmInstalledMod[]>([]);
  const [borrowed, setBorrowed] = useState<Set<string>>(
    () => new Set(editing.mods?.borrowed ?? []),
  );
  const [bundled, setBundled] = useState<Set<string>>(
    () => new Set(editing.mods?.bundled ?? []),
  );
  // 领域私有副本台账（mod-acquisition-update §4.4）：维护者在领域沙箱内从仓库更新/新装的模组；
  // 保存时随 realm.json `mods.pinned` 落盘，`apply_realm_mods` 据此不参与联接/复制/清理。
  const [pinnedMods, setPinnedMods] = useState<RealmModPin[]>(
    () => (editing.mods?.pinned ?? []).filter((p) => !!p?.id),
  );
  // 依赖忽略集（依赖处置：不可得依赖的静音清单，**小写** id）：随 realm.json `mods.depIgnores`
  // 落盘。依据 VCMI 引擎语义：依赖缺失仅 BROKEN 剔除、不阻断启动——忽略 = 纯 UI 静音。
  const [depIgnores, setDepIgnores] = useState<Set<string>>(
    () => new Set((editing.mods?.depIgnores ?? []).map((s) => s.toLowerCase())),
  );
  // 仓库索引（获取与更新：仅 RealmEditor 提供维护者入口，C4）
  const [repoIndex, setRepoIndex] = useState<ModRepoEntry[] | null>(null);
  const [repoBusy, setRepoBusy] = useState(false);
  // 启用层（§3.3）：领域预设 —— 每个预设独立记录「启用哪些模组」，由 modSettings.json 维护。
  // 与携带层（borrowed / bundled）解耦：携带 = 联接 / 复制 / 导出，启用 = VCMI 实际启用的组合。
  const [presets, setPresets] = useState<Record<string, string[]>>({});
  const [activePreset, setActivePreset] = useState(editing.presetId || REALM_PRESET_ID);
  // 领域预设尚未从领域 modSettings.json 读出：就绪前 ModsTab 禁止改动选择，
  // 避免「空选中」被当成权威组合（与 Mods 页面的 boot 阶段机同款问题）。
  const [presetsReady, setPresetsReady] = useState(false);
  const [addingPreset, setAddingPreset] = useState(false);
  const [newPresetName, setNewPresetName] = useState("");
  // 「领域模组」tab 双视图（realm-mods-management §4.4）：视图状态由本组件持有（非路由），
  // 切走其它 tab 再回来保持；A = 模组预设（默认），B = 模组管理（四标签页）。
  const [modsView, setModsView] = useState<ModsViewMode>("presets");
  /** 管理视图定位意图（预设页「依赖处置 → 去处理」）：落入「已实装」页 + 可选开启缺陷筛选。 */
  const [manageJump, setManageJump] = useState<{ depFilter: boolean } | null>(null);
  const openManage = (opts?: { depFilter?: boolean }) => {
    setModsView("manage");
    setManageJump({ depFilter: !!opts?.depFilter });
  };
  // 领域登场：素材预览根目录（已保存领域才有；新建态用 pending 绝对路径，无需 root）
  const [assetRoot, setAssetRoot] = useState("");

  // §9.6.1 四个标签页：基本信息 / 领域登场 / 领域模组 / 高级选项
  const [tab, setTab] = useState<"basic" | "intro" | "mods" | "adv" | "maps" | "showcase">("basic");

  // 基本信息（§9.6.1）：领域 ID 由创建弹窗确定且只读 / 版本 / 作者
  const [version, setVersion] = useState((editing.version as string) ?? "");
  const [author, setAuthor] = useState((editing.author as string) ?? "");

  // 登场演出（§5.4 / §5.5）：是否登场由是否配置素材决定（§9.6.1 核对点 3），故无主开关。
  // 素材分两类：媒体（wallpaper/cover/music/introMusic/introVideo）语言无关、见上方 `media`；
  // 介绍文本（introText）随语言覆盖、见 `curIntroText`。
  const [introSeconds, setIntroSeconds] = useState(
    editing.intro?.durationMs ? editing.intro.durationMs / 1000 : 5,
  );
  // 设计 §6 拼写为 `skipable`；`showRealmName` 控制登场时是否显示领域名称
  const [introSkippable, setIntroSkippable] = useState(editing.intro?.skipable !== false);
  const [introShowName, setIntroShowName] = useState(editing.intro?.showRealmName !== false);
  // §6 `showGameName`：领域主界面与登场演出的领域标题是否带游戏名前缀
  const [showGameName, setShowGameName] = useState(editing.showGameName === true);
  // §9.3 `info.showCore`：领域信息页是否**默认**展开原版内容（玩家仍可在页面临时开关）
  const [infoShowCore, setInfoShowCore] = useState(editing.info?.showCore === true);
  // §6 `main.loopMusic`：领域主界面的背景音乐是否循环（未配置时默认循环）
  const [loopMusic, setLoopMusic] = useState(editing.main?.loopMusic !== false);
  // §6 `main.maskOpacity`：主界面文字蒙层透明度（默认 80）；`main.background`：背景图片滤镜（主题化，作者设定）
  const [maskOpacity, setMaskOpacity] = useState(editing.main?.maskOpacity ?? 80);
  const [background, setBackground] = useState<RealmBackgroundFilter | undefined>(editing.main?.background);
  // 「主界面外观」小节默认收起，让介绍音频/视频/文本先曝光
  const [appearanceOpen, setAppearanceOpen] = useState(false);
  // 整合模组重名冲突（§9.3.3）：非空时弹窗提示改名
  const [modConflicts, setModConflicts] = useState<ModConflict[] | null>(null);
  // 阵营巡礼配置（设计 docs/features/faction-showcase.zh-CN.md）
  const [showcase, setShowcase] = useState<RealmShowcase | undefined>(editing.showcase);

  // 后端操作使用的领域 ID：即 editing.id（ID 在创建时确定且不可修改，重命名入口已移除）
  const currentId = editing.id;
  /** 已把领域预设合并进借用清单的领域 id（同一领域只合并一次，见下方 mods tab 的 effect） */
  const presetMergedRef = useRef<string | null>(null);
  /**
   * 加载期的**归一化**（预设懒加载 / borrowed·bundled 收敛）会写表单，需要**重算基线**。
   *
   * ⚠️ 为什么不就地算：两条归一化**可能同时发生**，各自拿"自己的新值 + 对方的旧值"算基线
   * 会互相漏掉 → 统一置位，等这一轮 state 全部落地后，用**当前值**重算一次（见下方 effect）。
   */
  const normPendingRef = useRef(false);
  /**
   * 归一化发生**之前**表单是否已有用户改动。
   *
   * ⚠️ 归一化后重算基线是为了「让归一化本身不算改动」，但**整份重算**会把归一化之前
   * 用户已做的编辑一并洗白 —— 症状就是「把默认语言切成英文后，保存一直置灰、切换不生效」。
   * 故：归一化前若表单已 dirty，则**保留基线不动**，让用户的改动继续可见 / 可保存。
   */
  const normHadEditsRef = useRef(false);
  // B5 预览：临时全屏覆盖层，播放登场演出 + 领域主界面，不产生持久改动
  const [previewOpen, setPreviewOpen] = useState(false);
  // 巡礼预览：从「巡礼」Tab 直接全屏打开播放页，跳过登场演出（改完立刻能看到效果）。
  // 领域对象存 state 而非每次渲染现造 —— 否则 `ShowcaseOverlay` 的 `[realm]` 依赖会每次重跑。
  const [previewShowcaseOpen, setPreviewShowcaseOpen] = useState(false);
  const [previewShowcaseRealm, setPreviewShowcaseRealm] = useState<Realm | null>(null);
  const [previewPhase, setPreviewPhase] = useState<"intro" | "shell">("intro");
  /**
   * 预览范围（预览按钮组）：
   * - `full`：完整流程 —— 登场演出（若已配置）→ 领域主界面（其中的「阵营巡礼」可点）
   * - `introOnly`：只看登场演出，演完即退出
   * - `shellOnly`：跳过登场，直接看领域主界面（「阵营巡礼」照常可用）
   */
  const [previewMode, setPreviewMode] = useState<"full" | "introOnly" | "shellOnly">("full");
  /** 预览按钮组的下拉是否展开 */
  const [previewMenuOpen, setPreviewMenuOpen] = useState(false);
  const [previewRealm, setPreviewRealm] = useState<Realm | null>(null);
  const [previewRoot, setPreviewRoot] = useState("");
  // 领域专属地图清单（设计 §6 `maps.entries`）：初值取 realm.json，避免未打开高级选项时保存被清空。
  // 清单的加载/变更由 MapsTab 负责，通过 onMapsChanged 回传同步；语言过滤配置同步由 onFilterChange 回传。
  const [maps, setMaps] = useState<RealmMapEntry[]>(editing.maps?.entries ?? []);
  const [mapFilter, setMapFilter] = useState<{ enabled: boolean; target: string }>({
    enabled: editing.maps?.filterLang ?? false,
    target: editing.maps?.filterLangTarget ?? "",
  });
  // §8.3.4「整理游戏自带地图」：首次开启屏蔽时弹确认框（避免对游戏目录的不可逆改动无提示）
  const [showOrganize, setShowOrganize] = useState(false);
  const [organizeBusy, setOrganizeBusy] = useState(false);
  const [organized, setOrganized] = useState(true);

  // 导出弹窗（§9.6.1：导出仅「高级选项」显示，先校验已保存且无差异）。
  // 弹窗内部自持三态（idle / running / result）与选项，这里只保留开关与领域上下文。
  const [exportOpen, setExportOpen] = useState(false);

  // 语言标签 / 可见语言（默认语言在前，其余为其后启用的覆盖语言）
  const langLabel = (v: string) => UI_LANG_OPTIONS.find((l) => l.value === v)?.label ?? v;
  const visibleLangs = [defaultLang, ...Object.keys(i18nMap).filter((l) => l !== defaultLang)];
  /** 勾选 / 取消勾选某语言。默认语言必选（取消无效），取消时丢弃该语言的覆盖值。 */
  const toggleLang = (v: string) => {
    if (v === defaultLang) return; // 默认语言不可取消
    if (i18nMap[v] !== undefined) {
      setI18nMap((m) => {
        const c = { ...m };
        delete c[v];
        return c;
      });
      if (curLang === v) setCurLang(defaultLang);
    } else {
      setI18nMap((m) => ({ ...m, [v]: {} }));
      setCurLang(v); // 启用即切换到该语言编辑（设计 §9.6.1）
    }
  };
  /**
   * 设为默认语言：只切换「默认语言」标记与当前编辑语言，**不移动任何语言条目的内容**。
   *
   * ⚠️ 历史缺陷：旧实现把「原默认语言」与「目标语言」的条目内容互换，导致
   * 「英文语言下显示原中文内容」——每个语言 key 永久拥有自己的内容（设计 §6 的
   * `i18n.<lang>`），改变默认语言只是换个兜底 / 顶层镜像来源，不该改写其它语言的数据。
   * 若目标语言尚无条目，则补一个空条目，保证默认语言在 i18nMap 中始终可编辑
   * （绝不沿用原默认语言的内容，否则会污染目标语言）。
   */
  const setAsDefaultLang = (v: string) => {
    if (v === defaultLang) return;
    setI18nMap((m) => (m[v] !== undefined ? m : { ...m, [v]: {} }));
    setDefaultLang(v);
    setCurLang(v);
  };
  /** 去掉空字符串，避免把空值（如 `{"cover":""}`）写进 realm.json。**只处理媒体字段**
   * （语言无关，落顶层 `assets`；介绍文本见 `pruneI18nAssets`）。 */
  const pruneMedia = (a: RealmAssets): RealmAssets | undefined => {
    const out: RealmAssets = {};
    for (const k of REALM_MEDIA_ASSET_KEYS) {
      const v = (a[k] ?? "").trim();
      if (v) out[k] = v;
    }
    return Object.keys(out).length ? out : undefined;
  };
  /** 语言块内的素材只剩「介绍文本」：媒体一律归顶层（见 `RealmI18nAssets`） */
  const pruneI18nAssets = (a: RealmI18nAssets): RealmI18nAssets | undefined =>
    (a.introText ?? "").trim() ? { introText: a.introText } : undefined;
  /** 只保留有实际内容的语言条目；每个 entry 内也只保留非空字段。
   * 默认语言（defaultLang）也在 i18n 中落盘（新模型），故不再跳过；无内容则不强写（顶层已兜底）。
   * 标签例外：条目一旦保留（有其他内容，或 tags 本身有值）就**恒写** `tags`（含空数组）——
   * 空数组是「该语言无标签」的显式声明（发布时覆盖索引），缺键会被下游当作「未声明」保留索引旧值。 */
  const pruneI18n = (m: Record<string, RealmI18nEntry>) => {
    const out: Record<string, RealmI18nEntry> = {};
    for (const [l, e] of Object.entries(m)) {
      const entry: RealmI18nEntry = {};
      const name = (e.name ?? "").trim();
      const desc = (e.description ?? "").trim();
      const assets = e.assets ? pruneI18nAssets(e.assets) : undefined;
      const tags = Array.isArray(e.tags) ? e.tags : [];
      if (name) entry.name = name;
      if (desc) entry.description = desc;
      if (assets) entry.assets = assets;
      if (Object.keys(entry).length || tags.length) {
        entry.tags = [...tags];
        out[l] = entry;
      }
    }
    return Object.keys(out).length ? out : undefined;
  };

  /**
   * 启用层的**稳定序列化**：每个预设的模组列表先**去重 + 排序**再比较。
   * ⚠️ 不这么做会把"同一份组合、只是顺序不同"判成改动 —— 磁盘值经 `fromVcmiPreset`
   * 展开子模组后，顺序本就可能与 realm.json 里落盘的不同。
   */
  const presetsSig = (ps: Record<string, string[]>) =>
    JSON.stringify(Object.fromEntries(Object.entries(ps).map(([k, v]) => [k, [...new Set(v)].sort()])));

  /**
   * 表单签名（"是否有未保存改动"的判据）。排序后再比较，避免"只是顺序不同"被判成改动。
   *
   * ⚠️ 覆盖面要求（v1.26）：**所有会被保存写入的编辑态字段都要在签名里**——
   * 保存按钮以 `dirty` 为可点门控，漏字段会出现"改了按钮却不亮"的死角。
   * 已知唯一例外：`maps` 只比较文件名集合（地图元数据不可编辑；全量比较会因挂载时
   * 后端重扫的 `modifiedAt` 等抖动而误报）。
   */
  const signatureOf = () =>
    JSON.stringify({
      defaultLang,
      baseName, baseDesc,
      // 媒体素材（语言无关）单独计入：改壁纸 / 封面也要能触发保存
      media: JSON.stringify(media),
      i18n: JSON.stringify(i18nMap),
      version, author, blockMaps, dirs: JSON.stringify(dirs),
      borrowed: [...borrowed].sort(),
      bundled: [...bundled].sort(),
      // 启用层（§3.3）：预设改动也要计入未保存差异，否则新建 / 切换组合后无法保存
      presetId: activePreset,
      presets: presetsSig(presets),
      introSeconds, introSkippable, introShowName,
      // v1.26 补全：显示游戏名称 / 主界面配置（循环音乐、遮罩强度、背景滤镜）
      showGameName,
      loopMusic, maskOpacity, background: JSON.stringify(background),
      infoShowCore,
      maps: maps.map((m) => m.file).sort(),
      // 地图语言过滤（MapsTab）与模组台账（私有副本 / 依赖忽略）同样参与保存
      mapFilter: JSON.stringify(mapFilter),
      pinned: JSON.stringify(pinnedMods),
      depIgnores: [...depIgnores].sort(),
      showcase: JSON.stringify(showcase),
    });
  const [baselineSig, setBaselineSig] = useState(signatureOf);
  const dirty = signatureOf() !== baselineSig;
  /** `dirty` 的最新值镜像：归一化的异步任务里读闭包 `dirty` 可能拿到旧值。 */
  const dirtyRef = useRef(dirty);
  dirtyRef.current = dirty;

  /**
   * 加载期归一化（预设懒加载 / borrowed·bundled 收敛）**落地后重算基线**。
   *
   * ⚠️ 故意**不写依赖数组**：要等这一轮所有归一化 state 都应用完再算，用当前值算才准。
   * 由 `normPendingRef` 把门 —— 只有真的发生过归一化才重算，避免吞掉用户的真实改动。
   */
  useEffect(() => {
    if (!normPendingRef.current) return;
    normPendingRef.current = false;
    // ⚠️ 归一化前已有用户改动时**绝不**重算基线：整份重算会把用户的编辑（如「默认语言
    // 切成英文」）一并洗白，表现为保存按钮明明该亮却一直置灰。归一化写回的正是磁盘既有值，
    // 保留在 diff 里也不会产生脏数据。
    if (normHadEditsRef.current) {
      normHadEditsRef.current = false;
      return;
    }
    setBaselineSig(signatureOf());
  });

  /** 刷新「领域已实装」（`list_realm_mods`）：挂载时拉取，物理动作后由管理面板经 `reload` 回拉。 */
  const reloadRealmInstalled = async () => {
    try {
      setRealmInstalled(await listRealmMods(currentId));
    } catch {
      /* 非致命：保留上次结果 */
    }
  };

  useEffect(() => {
    listInstalledMods().then(setInstalled).catch(() => setInstalled([]));
    void reloadRealmInstalled();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [currentId]);

  /**
   * 解析领域素材根目录（`{realm-dir}`，壁纸 / BGM / 自定义底图都相对它存放）。
   *
   * ⚠️ 必须**常驻解析，不能只在「领域登场」tab 里解析** —— 它同时供「巡礼」Tab 的预览使用
   * （`ShowcaseOverlay` 的 `root`）。旧写法在非 intro tab 里把 root 清空，导致从巡礼 Tab 预览时：
   * - 领域背景音乐解析为 `""` → 「本张改放领域背景音乐」静默无声（阵营音乐不受影响，因为它走
   *   后端解析出的绝对路径）；
   * - `overrideBackground`（自定义底图）也解析不出来 → 预览里看到的是缓存底图。
   */
  useEffect(() => {
    let alive = true;
    realmDir(currentId)
      .then((d) => alive && setAssetRoot(d))
      .catch(() => alive && setAssetRoot(""));
    return () => {
      alive = false;
    };
  }, [currentId]);

  /**
   * 「领域已实装」展示层（§7.8 落盘模型的 UI 补账）：磁盘真值（`list_realm_mods`，
   * 读已落盘的 realm.json 台账）∪ 编辑期内存引用（已点「引用 / 全部引用」但尚未保存）。
   *
   * - game 源引用**物理上不建链**（仅登记台账）：保存前只能由内存补展示，否则「全部引用」后
   *   只有 user 源（建了目录联接）立刻可见，看起来像“只引用了 N 个”；
   * - 反向：磁盘台账里仍在、但编辑期已取消引用的引用型条目（保存后才消失）从视图隐藏，
   *   保证视图始终呈现「保存后的预期状态」。
   */
  const realmInstalledView = useMemo<RealmInstalledMod[]>(() => {
    const memLower = new Set([...borrowed].map((x) => x.toLowerCase()));
    const gByLower = new Map(installed.map((x) => [x.id.toLowerCase(), x]));
    const diskLower = new Set(realmInstalled.map((x) => x.id.toLowerCase()));
    const out: RealmInstalledMod[] = [];
    /** 根段（点号前第一段）：台账只登记根模组，子模组随根判定在 / 不在（§3.4）。 */
    const seg = (id: string) => id.split(".")[0].toLowerCase();
    for (const x of realmInstalled) {
      // 引用型条目（user/game 源、非整合/私有副本）：存在性与 inBorrowed 以内存台账为准
      const isRefKind =
        (x.source === "user" || x.source === "game") && !x.inBundled && !x.inPinned;
      if (isRefKind && !memLower.has(seg(x.id))) continue; // 待移除（未保存），不展示
      out.push(isRefKind ? { ...x, inBorrowed: true } : x);
    }
    // 内存新增引用（尚未落盘）：从全局安装列表补元数据合成展示条目
    const pushedRoots = new Set<string>();
    for (const id of borrowed) {
      const lower = id.toLowerCase();
      if (diskLower.has(lower)) continue;
      const g = gByLower.get(lower);
      if (!g) continue;
      pushedRoots.add(lower);
      out.push({
        id,
        // 磁盘原名优先（`id` 是小写归一后的 VCMI modId，不能当目录名用）
        diskName: g.diskName || id,
        name: g.name,
        desc: g.desc ?? "",
        type: g.type,
        rawType: g.rawType ?? null,
        version: g.version ?? g.meta?.version ?? null,
        author: g.author ?? g.meta?.author ?? null,
        depends: g.depends ?? [],
        softDepends: g.softDepends ?? [],
        conflicts: g.conflicts ?? [],
        factions: g.factions,
        i18n: g.i18n,
        isSubmod: g.isSubmod ?? false,
        keepDisabled: g.keepDisabled,
        isLink: g.source !== "game",
        hasLedger: false,
        source: g.source === "game" ? "game" : "user",
        inBorrowed: true,
        inBundled: false,
        inPinned: false,
        metaOk: true,
      });
    }
    // 内存新增引用的**子模组**（尚未落盘）：v3 纯台账模型下引用零物理痕迹，后端展开
    // 要等激活 / 重扫才可见；这里按同一份全局安装列表补出子模组条目，保证右栏父子树
    // 与「保存后的预期状态」一致（§5.1）。
    const haveLower = new Set(out.map((x) => x.id.toLowerCase()));
    for (const s of installed) {
      if (!s.isSubmod) continue;
      const lower = s.id.toLowerCase();
      if (haveLower.has(lower)) continue;
      if (!pushedRoots.has(seg(s.id))) continue;
      out.push({
        id: s.id,
        // 磁盘原名优先（理由同上）
        diskName: s.diskName || s.id,
        name: s.name,
        desc: s.desc ?? "",
        type: s.type,
        rawType: s.rawType ?? null,
        version: s.version ?? s.meta?.version ?? null,
        author: s.author ?? s.meta?.author ?? null,
        depends: s.depends ?? [],
        softDepends: s.softDepends ?? [],
        conflicts: s.conflicts ?? [],
        factions: s.factions,
        i18n: s.i18n,
        isSubmod: true,
        keepDisabled: s.keepDisabled,
        isLink: s.source !== "game",
        hasLedger: false,
        source: s.source === "game" ? "game" : "user",
        inBorrowed: true,
        inBundled: false,
        inPinned: false,
        metaOk: true,
      });
    }
    return out;
  }, [realmInstalled, borrowed, installed]);

  // 领域模组 tab（realm-mods-management §5.1）：浏览区数据源收紧为「领域已实装」——
  // 展示层结果中解析成功（metaOk）的项（bundled ∪ pinned ∪ borrowed ∪ 引擎基础模组）；
  // 缺失引用（metaOk=false）只进管理视图的「缺失」清单，不进浏览区。
  const realmMods = useMemo<ModInfo[]>(
    () => realmInstalledView.filter((m) => m.metaOk).map(realmModToModInfo),
    [realmInstalledView],
  );
  /** 领域模组 tab 的基线模组（引擎核心，永远启用且不可取消），与 Mods 页面口径一致。 */
  const realmBaseMods = useMemo(
    () => [...new Set([...realmMods.filter((m) => m.type === "Core").map((m) => m.id), ...RESERVED_MOD_IDS])],
    [realmMods],
  );
  /** 领域模组 tab 的受控选中集合 = **当前预设启用的模组**（§3.3 启用层）。
   * 不再等于 borrowed ∪ bundled —— 携带清单只决定联接 / 复制 / 导出。 */
  const realmSelected = useMemo<Set<string>>(
    () => new Set(presets[activePreset] ?? []),
    [presets, activePreset],
  );
  /**
   * 依赖缺陷求解（「依赖处置」体系，`lib/modDeps.ts`）：三源（已实装 ∪ 全局已装 ∪
   * 仓库索引）+ 预设表 → 按根聚合的缺陷 / 受影响模组集 / 危害分级（🔴 已启用 vs 🟠 潜在）。
   * 供预设页徽章与告知弹层、树内角标、已实装行按钮与筛选、处理弹窗消费。
   */
  const depSolve = useMemo(
    () =>
      solveRealmDeps({
        installed: realmInstalledView,
        globalMods: installed,
        repoIndex,
        presets,
        ignoredDeps: depIgnores,
      }),
    [realmInstalledView, installed, repoIndex, presets, depIgnores],
  );
  /** 领域语言集合（小写：defaultLang + i18n 覆盖键）：依赖处置的语言分组 / 默认勾选依据。 */
  const depLangs = useMemo(
    () =>
      [...new Set([defaultLang, ...Object.keys(i18nMap)].map((s) => s.toLowerCase()))].filter(Boolean),
    [defaultLang, i18nMap],
  );
  /** 依赖闭包解析（一键解析）：与 `depSolve` 同源输入 + 忽略集；管理面板打开 / 实装变化时重新解析注入。 */
  const resolveClosure = (): DepClosureResult =>
    solveDepClosure({
      installed: realmInstalledView,
      globalMods: installed,
      repoIndex,
      presets,
      ignoredDeps: depIgnores,
    });
  /** 领域模组 id -> ModInfo 查表（根模组解析 / 预设转换共用）。 */
  const realmModMap = useMemo(() => new Map(realmMods.map((m) => [m.id, m])), [realmMods]);
  /** 解析任意模组 id（根或子模组）到其**根模组** id：引用 / 整合只认根模组，
   * 子模组随父整模组一起被引用 / 复制，不单独存在。
   * ⚠️ `ModInfo.parentId` 语义是「点号前第一段**或自身**」（见 modScanner），
   * 不能拿 `parentId !== id` 判断是否子模组；缺失项（不在本表中）按 id 点号兜底。 */
  const rootOf = (id: string): string => {
    let cur = id;
    for (;;) {
      const m = realmModMap.get(cur);
      if (m?.isSubmod && m.parentId && m.parentId !== cur) {
        cur = m.parentId;
        continue;
      }
      if (!m && cur.includes(".")) {
        cur = cur.split(".")[0];
        continue;
      }
      return cur;
    }
  };
  /** 归一化 legacy 携带集合：把旧版本可能带入的带点子模组 id（如 `P.S1`）收敛到根模组，
   * 保证 borrowed / bundled 里只存在根模组 id（§3.4）。 */
  useEffect(() => {
    if (realmMods.length === 0) return;
    const norm = (prev: Set<string>) => {
      const nb = new Set<string>();
      prev.forEach((id) => nb.add(rootOf(id)));
      if (nb.size === prev.size && [...nb].every((x) => prev.has(x))) return prev;
      return nb;
    };
    const nb = norm(borrowed);
    const nu = norm(bundled);
    if (nb === borrowed && nu === bundled) return; // 干净数据：什么都不做（也不重渲染）
    /**
     * 这是**老数据收敛**（带点子模组 id → 根模组），同样不是用户改动 ——
     * 基线跟着重算，否则老领域一进编辑器就被判"有未保存改动"。
     */
    normHadEditsRef.current = normHadEditsRef.current || dirtyRef.current;
    normPendingRef.current = true;
    setBorrowed(nb);
    setBundled(nu);
  }, [realmMods, realmModMap]);
  /** 拉取仓库索引（A1：TTL 内复用缓存秒回；过期走 SWR——先返回缓存、后台校验，
   * 远端真变经 `mod-index-updated` 刷新，不再阻塞等待；面板底部仅在阻塞触网时显示进度胶囊）。
   * `silent` = 静默模式（「自动检测更新」触发）：不弹完成/失败 toast，不阻断其它功能；
   * 手动点检（`silent=false`）忽略 TTL 新鲜度，总是触发后台条件校验（manual）。 */
  const handleRealmRepoCheck = async (silent = false) => {
    setRepoBusy(true);
    try {
      const idx = await fetchModIndex(false, !silent);
      setRepoIndex(idx.mods);
      if (silent) return;
      if (idx.error) {
        showToast("error", t.modsRepo.offline.replace("{msg}", idx.error));
      } else {
        showToast("success", t.modsRepo.checkDone.replace("{n}", String(idx.mods.length)));
      }
    } catch (e) {
      if (!silent) showToast("error", t.modsRepo.offline.replace("{msg}", backendErrorText(e)));
    } finally {
      setRepoBusy(false);
    }
  };

  // A1：SWR 后台校验发现远端索引变化——静默重拉（缓存已刷新，TTL 内秒回）并 toast 提示
  useEffect(() => {
    let unsub: (() => void) | null = null;
    let alive = true;
    void onModIndexUpdated(() => {
      void (async () => {
        try {
          const idx = await fetchModIndex(false);
          if (!alive) return;
          setRepoIndex(idx.mods);
          showToast("success", t.modsRepo.indexUpdated);
        } catch {
          /* 静默：重拉失败（罕见）下次检查自会刷新 */
        }
      })();
    }).then((u) => {
      if (alive) unsub = u;
      else u();
    });
    return () => {
      alive = false;
      if (unsub) unsub();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  /** 领域模组 tab 选中变化（由 ModBrowser 完成父子/依赖联动后回传）。
   * 只更新**当前预设的启用组合**（§3.3 启用层）——引用 / 整合是管理视图（视图 B）里的
   * 显式动作；勾选 / 取消勾选不再自动触发引用（realm-mods-management §5.1）。 */
  const handleRealmSelection = (next: Set<string>) => {
    setPresets((ps) => ({ ...ps, [activePreset]: [...next] }));
  };
  /** 素材预览 URL：领域素材根目录 + 相对路径（`v` 取自当前编辑语言的 assets） */
  const assetPreview = (rel?: string) =>
    assetRoot && rel ? realmAssetUrl(assetRoot, rel) : "";

  // 进入地图相关 tab 时查询地图阀门是否已整理（§8.3.4，供屏蔽开关首次开启时弹确认）
  useEffect(() => {
    if (tab === "adv" || tab === "maps") {
      nativeMapsStatus()
        .then((r) => setOrganized(r.organized))
        .catch(() => setOrganized(false));
    }
  }, [tab, currentId]);

  // 启动器 UI 语言变更时（激活领域态）重建领域地图阀门：
  // filterLangTarget 未显式指定时阀门跟随 ui_lang（§0.4）；非激活/游戏运行中静默忽略
  useEffect(() => {
    applyRealmMapsValve(currentId).catch(() => {
      /* 非激活领域或 VCMI 运行中：下次激活时应用 */
    });
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [lang]);

  // 领域模组 tab：从领域 modSettings.json 加载**全部预设**（§3.3 多预设）。
  // 预设是「启用组合」的权威来源，与 realm.json 的 borrowed / bundled（携带清单）解耦：
  // 携带清单决定联接 / 复制 / 导出，预设决定 VCMI 实际启用哪些模组。
  // 同一领域只加载一次（`presetMergedRef`），避免覆盖用户进入 tab 后的手动改动。
  useEffect(() => {
    // 「巡礼」Tab 也要用当前启用组合来推导自动牌组（`realmSelected`），
    // 所以不能只在「模组」Tab 才加载预设 —— 否则巡礼会拿到空集合、退化成全量城镇。
    if (tab !== "mods" && tab !== "showcase") return;
    if (presetMergedRef.current === currentId) {
      // 同一领域已合并过：presets / activePreset 已是权威值，直接放开交互
      setPresetsReady(true);
      return;
    }
    // 换了领域（尚未合并过）：重新进入加载态，避免沿用上一个领域已就绪的标记
    setPresetsReady(false);
    let alive = true;
    const job = (async () => {
      const cfgDir = await realmConfigDir(currentId);
      const settings = cfgDir ? await readModSettingsJson(cfgDir) : null;
      const raw = settings?.presets as
        | Record<string, { mods?: unknown; settings?: unknown } | undefined>
        | undefined;
      // 领域范围模组树（`realmMods`）此时应已就绪，用它派生子模组树；
      // 若为空则退化为仅根模组（与旧行为一致，不丢数据）。
      const tree = realmMods;
      const next: Record<string, string[]> = {};
      for (const [pid, val] of Object.entries(raw ?? {})) {
        const mods = (Array.isArray(val?.mods) ? val.mods : []).filter(
          (x): x is string => typeof x === "string" && !!x && !RESERVED_MOD_IDS.has(x),
        );
        const sub = (
          val?.settings && typeof val.settings === "object" ? val.settings : {}
        ) as Record<string, Record<string, boolean>>;
        const flatSel = fromVcmiPreset(tree, { mods, settings: sub });
        next[pid] = flatSel.filter((id) => !RESERVED_MOD_IDS.has(id));
      }
      const ap =
        typeof settings?.activePreset === "string" && next[settings.activePreset]
          ? settings.activePreset
          : editing.presetId || REALM_PRESET_ID;
      // 旧领域（还没写入过预设）：用携带清单初始化当前预设，保证已有勾选不丢
      if (!next[ap] || next[ap].length === 0) {
        next[ap] = [...new Set([...borrowed, ...bundled])];
      }
      if (!alive) return;
      /**
       * ⚠️ 这次是**从磁盘补齐表单**，不是用户改动 —— 基线要跟着重算，
       * 否则"进一次领域模组 / 阵营巡礼 tab 就提示有未保存改动"（实测反馈）：
       * `presets` 初始是 `{}`，加载后变成真实预设表，签名必然变化。
       *
       * 只在"预设层还没被用户动过"（仍是空表）时置位 —— 用户改过就**不**重算，
       * 免得把真实改动当成基线吞掉。
       */
      if (Object.keys(presets).length === 0) {
        normHadEditsRef.current = normHadEditsRef.current || dirtyRef.current;
        normPendingRef.current = true;
      }
      presetMergedRef.current = currentId;
      setActivePreset(ap);
      setPresets(next);
    })();
    // 成功与否都要放开：失败时沿用内置 / 携带清单兜底，不能让 tab 卡在加载态
    const markReady = () => {
      if (alive) setPresetsReady(true);
    };
    job.then(markReady, (e) => {
      console.error("[RealmEditor] 加载领域模组预设失败:", e);
      markReady();
    });
    return () => {
      alive = false;
    };
  }, [tab, currentId]);

  /** 新建模组组合：以输入的名称为预设 id，创建后即切换过去（空组合，自行勾选模组） */
  const addPreset = () => {
    const id = newPresetName.trim();
    if (!id || presets[id]) return;
    setPresets((ps) => ({ ...ps, [id]: [] }));
    setActivePreset(id);
    setNewPresetName("");
    setAddingPreset(false);
  };
  /** 删除当前模组组合（至少保留一个），并切到剩下的第一个 */
  const deletePreset = () => {
    const rest = Object.keys(presets).filter((p) => p !== activePreset);
    if (rest.length === 0) return;
    setPresets((ps) => {
      const next = { ...ps };
      delete next[activePreset];
      return next;
    });
    setActivePreset(rest[0]);
  };
  /** 重命名模组组合：键值整体迁移并保持激活态（`__realm__` 为契约 id，不可重命名） */
  const renamePreset = (from: string, to: string) => {
    const n = to.trim();
    if (!n || n === from || from === REALM_PRESET_ID || presets[n]) return;
    setPresets((ps) => {
      const next: Record<string, string[]> = {};
      for (const [k, v] of Object.entries(ps)) next[k === from ? n : k] = v;
      return next;
    });
    setActivePreset((cur) => (cur === from ? n : cur));
  };

  /** 从**全部预设**清理这些根模组及其子模组（realm-mods-management §7.7）：
   * 「取消引用 / 删除」的联动——物理层已移除的模组不应在启用组合里残留。 */
  const purgePresets = (rootIds: string[]) => {
    if (rootIds.length === 0) return;
    const roots = new Set(rootIds.map((x) => x.toLowerCase()));
    setPresets((ps) => {
      const next: Record<string, string[]> = {};
      for (const [k, v] of Object.entries(ps)) {
        next[k] = v.filter((id) => !roots.has(rootOf(id).toLowerCase()));
      }
      return next;
    });
  };

  /**
   * 「从模组预设引用」的预设归属（realm-mods-management §7.5 核对点 3）：
   * - `create` 缺省：合并进当前领域的 `__realm__` 组合（去重）；
   * - `create` 给定时：以该名新建领域组合（重名自动加 `-2` / `-3` 后缀），并切换过去。
   * 返回**落定**的组合名（供结果提示）。
   */
  const adoptPresetRefs = (ids: string[], create?: string): string => {
    if (create) {
      let name = create;
      let i = 2;
      while (presets[name]) name = `${create}-${i++}`;
      setPresets((ps) => ({ ...ps, [name]: [...new Set([...(ps[name] ?? []), ...ids])] }));
      setActivePreset(name);
      return name;
    }
    setPresets((ps) => ({
      ...ps,
      [REALM_PRESET_ID]: [...new Set([...(ps[REALM_PRESET_ID] ?? []), ...ids])],
    }));
    return REALM_PRESET_ID;
  };

  /**
   * 依赖处置（选项 A「实装 + 启用」）：把依赖补入**受影响子模组所在预设**的激活集。
   *
   * 引擎启动只认预设激活集（`ModManager::getActiveMods` = mods 数组 + settings 为 true
   * 的子模组；`addNewModsToPreset` 对 root 直接跳过、绝不自动启用）——仅实装不启用，
   * 受影响子模组在游戏里仍被 BROKEN 剔除。补入用**依赖声明的精确 id**：子模组 id 由
   * `toVcmiPreset` 自动带出父根模组（见 `vcmiPreset.ts` 的子模组分支）。
   * id 规范到磁盘写法（realmMods 权威）；编辑期状态，随「保存领域」落盘。
   */
  const enableDepInPresets = (depId: string, presetNames: string[]) => {
    if (presetNames.length === 0) return;
    const canonical =
      realmMods.find((m) => m.id.toLowerCase() === depId.toLowerCase())?.id ?? depId;
    setPresets((ps) => {
      let changed = false;
      const next = { ...ps };
      for (const p of presetNames) {
        const cur = next[p];
        if (!cur || cur.some((v) => v.toLowerCase() === canonical.toLowerCase())) continue;
        next[p] = [...cur, canonical];
        changed = true;
      }
      return changed ? next : ps;
    });
  };

  /**
   * 选择并导入登场演出素材（§5.4）。
   * 资源必须落在 `{realm-dir}/assets/` 下，才能随 `.vcmirealm` 导出打包（§7.1）。
   * 已有领域立即复制；新建领域暂存，待保存取得 id 后再复制。
   */
  const pickAsset = async (
    kind: "wallpaper" | "cover" | "music" | "introMusic" | "introVideo",
    pickKind: "video" | "cover" | "wallpaper" | "music" | "introMusic",
  ) => {
    const p = await pickOpenPath(
      pickKind === "video"
        ? [{ name: t.realm.introVideo, extensions: ["mp4", "webm", "mov", "mkv"] }]
        : pickKind === "music" || pickKind === "introMusic"
          ? [{ name: t.realm.music, extensions: ["mp3", "ogg", "wav", "m4a", "flac"] }]
          : [{ name: t.realm.introCover, extensions: ["png", "jpg", "jpeg", "webp"] }],
    );
    if (!p) return;
    // 领域目录已在创建时建立，素材可直接复制进 `{realm-dir}/assets/`（无需保存后再补写）。
    // 媒体**语言无关**：一律写顶层 `assets`（`media`），不随当前编辑语言分家——
    // 文件名本就按 kind 固定（`assets/wallpaper.<ext>`），分语言存不下且发布推导只认顶层。
    const rel = await importRealmAsset(currentId, p, pickKind);
    patchMedia({ [kind]: rel });
  };

  /**
   * 整合模组（`mods.bundled`）的重名校验（§9.3.3）。
   *
   * 只在**确有风险**时才打扰玩家，避免「整合即弹窗」的噪音——判定下沉到后端
   * `check_realm_mod_conflicts`，比对来源目录与领域内副本的目录指纹：
   * - `game`：来源在游戏目录（永远可见）→ 进入领域会与领域副本重复挂载同 ID；
   * - `diverged`：领域副本与来源内容已分叉 → 共用同一份按 modId 键控的百科缓存，会串味。
   * 领域内尚未整合（无副本）时不算冲突，因为复制过去内容必然一致。
   */
  const handleSave = async () => {
    const hasName =
      baseName.trim().length > 0 ||
      Object.values(i18nMap).some((e) => (e.name ?? "").trim().length > 0);
    if (!hasName) {
      showToast("error", t.realm.needName);
      return;
    }
    if (bundled.size > 0) {
      const conflicts = await checkRealmModConflicts(currentId, [...bundled]);
      if (conflicts.length > 0) {
        setModConflicts(conflicts);
        return;
      }
    }
    await performSave();
  };

  /** 保存：领域已存在（ID 在创建时确定），只需把表单深合并回 `realm.json` */
  const performSave = async () => {
    setSaving(true);
    try {
      const data: Record<string, unknown> = {
        // 顶层为默认语言（设计 §6）
        name: baseName.trim() || undefined,
        description: baseDesc.trim() || undefined,
        // 标签（顶层 = 默认语言镜像）：**恒写**（含空数组 = 显式清空）——缺键会被下游当作
        // 「未声明」而保留索引既有值；与 depIgnores 同款恒写理由（见下）。
        tags: [...(baseEntry.tags ?? [])],
        defaultLang,
        version: version.trim() || undefined,
        author: author.trim() || undefined,
        // 媒体素材（语言无关）：只写顶层，不进 i18n
        assets: pruneMedia(media),
        // §6 `showGameName`：领域主界面与登场演出的标题带游戏名前缀
        showGameName,
        // §6 `main`：领域主界面配置
        main: { loopMusic, maskOpacity, background },
        // 登场演出（§5.4）：秒数输入 → 毫秒存储；是否登场由素材决定（§9.6.1 核对点 3），故无 enabled
        intro: {
          skipable: introSkippable,
          showRealmName: introShowName,
          durationMs: Math.max(0, Math.round(introSeconds * 1000)),
        },
        // §9.3 领域信息页：是否默认展开原版内容（玩家可在页面临时开关）
        info: { showCore: infoShowCore },
        presetId: activePreset,
        mods: {
          // 携带集合只存根模组 id（子模组随父整模组，§3.4）：写入前再收敛一次。
          // pinned 里的 id 是领域私有副本，从 borrowed/bundled 剔除（§4.4，防被联接/复制覆盖）。
          borrowed: [
            ...new Set([...borrowed].filter((x) => !pinnedMods.some((p) => p.id === x)).map(rootOf)),
          ],
          bundled: [
            ...new Set([...bundled].filter((x) => !pinnedMods.some((p) => p.id === x)).map(rootOf)),
          ],
          // 私有副本台账（保存后 `apply_realm_mods` 据此保护不被清理/覆盖）
          pinned: pinnedMods.length > 0 ? pinnedMods : undefined,
          // 依赖忽略集（静音）：**恒写数组**（含空）——顶层键深合并下缺省会残留旧值，
          // 清空删不掉；排序保证落盘稳定 diff。
          depIgnores: [...depIgnores].sort(),
        },
        // 地图清单由后端随目录同步，这里回写保证 realm.json 与磁盘一致（设计 §6）；
        // 语言过滤配置一并回写（后端 set_realm_map_filter 也会即时落盘，此处兜底）
        maps: {
          entries: maps,
          filterLang: mapFilter.enabled,
          filterLangTarget: mapFilter.target || undefined,
        },
        // 屏蔽地图的唯一权威是 dirs.gameMaps（§8.3）
        dirs: { ...dirs, gameMaps: blockMaps ? "hide" : "show" },
        // 其他语言的覆盖
        i18n: pruneI18n(i18nMap),
        // 阵营巡礼配置（设计 docs/features/faction-showcase.zh-CN.md）
        ...(showcase ? { showcase } : {}),
      };
      // 启用层：把**全部预设**写入领域 modSettings.json（§3.3 多预设）。
      // 与 realm.json 的 mods（携带清单）分开维护，保存领域不再覆盖玩家调整过的启用组合。
      // 启用层：把**全部预设**写入领域 modSettings.json（§3.3 多预设）。
      // 转 VCMI 原生格式：根模组进 mods，与默认态（`!keepDisabled`）不同的子模组开关进 settings；
      // 基线模组（realmBaseMods）的子模组保持 VCMI 默认，不写 settings（managedRoots 排除它们）。
      // 与 realm.json 的 mods（携带清单）分开维护，保存领域不再覆盖玩家调整过的启用组合。
      await writeRealmPresets(
        currentId,
        Object.fromEntries(
          Object.entries(presets).map(([k, v]) => {
            const flatMap = new Map(realmMods.map((m) => [m.id, m]));
            const managedRoots = new Set(
              v.filter((id) => {
                const m = flatMap.get(id);
                return m && !m.isSubmod && !realmBaseMods.includes(id);
              }),
            );
            const conv = toVcmiPreset(realmMods, v, managedRoots);
            return [k, { mods: conv.mods, settings: conv.settings ?? {} }];
          }),
        ),
        activePreset,
      );
      await updateRealmSafe(currentId, data);
      // §7.8 落盘模型（v3）：引用 / 取消引用为纯台账申报——保存只写 realm.json，
      // 不做物理同步；目录联接等实际挂载由「激活领域」时 `apply_realm_mods` 统一执行。
      void reloadRealmInstalled();
      setBaselineSig(signatureOf());
      onSaved();
    } catch (e) {
      showToast("error", t.realm.error.replace("{msg}", backendErrorText(e)));
    } finally {
      setSaving(false);
    }
  };

  // 落盘屏蔽地图偏好（若领域已在激活态，切换需要即时生效）
  const applyBlock = async (v: boolean) => {
    try {
      await setRealmMapsBlockedSafe(currentId, v);
    } catch {
      /* 非激活态忽略：仅持久化偏好，激活时再套用 */
    }
  };

  // 屏蔽游戏自带地图（§8.3.4）：先更新本地偏好；首次开启且尚未整理时弹出确认框，
  // 返回：有未保存改动时先确认（领域本身已创建，放弃的只是本页未保存的改动）
  const [confirmLeave, setConfirmLeave] = useState(false);
  const handleBack = () => {
    if (dirty) setConfirmLeave(true);
    else onCancel();
  };

  // 「返回 + 标题」已上提为覆盖层左上角悬浮页头（§9.2.4，v1.16），由 `RealmShell` 渲染；
  // 把返回处理（含未保存确认）注册给宿主，供页头的返回按钮触发。
  useEffect(() => {
    registerBack(handleBack);
    return () => registerBack(null);
  }, [registerBack, handleBack]);

  // 确认后再真正落盘（触发 one-time 收纳 + 阀门切换）；已整理过则直接落盘。
  const requestBlock = (v: boolean) => {
    setDirs((d) => ({ ...d, gameMaps: v ? "hide" : "show" }));
    if (v && !organized) {
      setShowOrganize(true);
      return;
    }
    applyBlock(v);
  };

  /** 导出校验（§9.6.1）：必须已保存、无未保存差异。 */
  const handleExportClick = () => {
    if (dirty) {
      showToast("error", t.realm.exportDirty);
      return;
    }
    setExportOpen(true);
  };
  /**
   * 导出弹窗算「本领域所需素材 mod」（启用集合 + bundled/borrowed + core.*），
   * 与领域激活/巡礼同一口径（`lib/modcache/realmAssets.ts`），但**排除原版 core.***
   * （原版素材由接收方本机生成，随包分发会白增约 58 MB 且跨版本必然失配）。
   * 导出前已校验「已保存且无差异」，故此处读磁盘预设与编辑态一致。
   */
  const resolveExportCacheMods = async () => {
    const targets = await realmAssetTargets(editing, currentId, { excludeCore: true });
    return targets.map((m) => m.modId);
  };

  /** 预览用临时领域对象（取自编辑期状态，不落盘）。登场演出与巡礼预览共用。 */
  const buildPreviewRealm = (): Realm => ({
    id: currentId,
    name: baseName,
    description: baseDesc,
    defaultLang,
    assets: pruneMedia(media),
    showGameName,
    main: { loopMusic, maskOpacity, background },
    intro: {
      skipable: introSkippable,
      showRealmName: introShowName,
      durationMs: Math.max(0, Math.round(introSeconds * 1000)),
    },
    info: { showCore: infoShowCore },
    presetId: activePreset,
    mods: {
      borrowed: [...borrowed],
      bundled: [...bundled],
      pinned: pinnedMods.length > 0 ? pinnedMods : undefined,
    },
    maps: { entries: maps },
    dirs,
    i18n: pruneI18n(i18nMap),
    // 阵营巡礼（docs/features/faction-showcase.zh-CN.md）：预览必须带上编辑期配置，
    // 否则只会走全部默认值，看不到刚配的节奏 / 牌组 / 音乐偏移。
    showcase,
  });

  /**
   * 预览（B5）：临时全屏覆盖层，不产生任何持久改动。
   *
   * - `full`：登场演出（`RealmIntroOverlay` 内部对"无内容"会直接结束）→ 领域主界面；
   * - `introOnly`：只演登场，演完在 `onFinish` 里直接退出；
   * - `shellOnly`：跳过登场，首帧就落在领域主界面。
   *
   * 三条路径都使用**编辑期**的 realm 对象（`buildPreviewRealm()`），所以未保存的改动也能预览到。
   */
  const startPreview = async (mode: "full" | "introOnly" | "shellOnly") => {
    setPreviewMenuOpen(false);
    const realmObj: Realm = buildPreviewRealm();
    const root = await realmDir(currentId);
    setPreviewRealm(realmObj);
    setPreviewRoot(root);
    setPreviewMode(mode);
    setPreviewPhase(mode === "shellOnly" ? "shell" : "intro");
    setPreviewOpen(true);
  };

  // 四个标签页的 props 打包（§9.6.1）。放在此处是为了让被引用的
  // state / useMemo / 处理函数都已初始化（它们都是 const，不存在提升）。
  // 显式标注为各自的 Props 类型：漏传 / 多传都会被 `tsc` 直接拦下。
  const basicTabProps: BasicTabProps = {
    t,
    curEntry,
    curLang,
    currentId,
    langLabel,
    author,
    setAuthor,
    setCur,
    version,
    setVersion,
  };

  const advTabProps: AdvancedTabProps = {
    t,
    borrowed,
    bundled,
    dirs,
    setDirs,
    setShare,
    infoShowCore,
    setInfoShowCore,
  };

  const mapsTabProps = {
    t,
    realmId: currentId,
    disabled: dirs.dataPath === "global",
    blockMaps,
    requestBlock,
    filterLang: mapFilter.enabled,
    filterLangTarget: mapFilter.target,
    onFilterChange: (enabled: boolean, target: string) =>
      setMapFilter({ enabled, target }),
    onMapsChanged: (entries: RealmMapEntry[]) => setMaps(entries),
    showToast,
  };

  const modsTabProps: ModsTabProps = {
    t,
    presets,
    activePreset,
    setActivePreset,
    newPresetName,
    setNewPresetName,
    addingPreset,
    setAddingPreset,
    addPreset,
    deletePreset,
    renamePreset,
    realmMods,
    realmBaseMods,
    realmSelected,
    presetsReady,
    handleRealmSelection,
    onDependencyBlocked: (msg) => showToast("error", msg),
    view: modsView,
    setView: setModsView,
    // 依赖缺陷求解 + 「去处理」跳转（预设页操作栏徽章 / 告知弹层 / 树内角标消费）
    depSolve,
    onGoManage: openManage,
    // 管理视图（realm-mods-management §5.2）：物理动作即时执行，台账由本组件持有、
    // 随「保存领域」落盘（§7.8）；`reload` 回拉刷新已实装列表。
    manage: {
      realmId: currentId,
      // 展示层（磁盘真值 ∪ 内存未落盘引用）：见 `realmInstalledView`
      installed: realmInstalledView,
      isolated: dirs.dataPath === "realm" && dirs.configPath === "realm",
      globalMods: installed,
      borrowed,
      bundled,
      pinned: pinnedMods,
      setBorrowed,
      setBundled,
      setPinned: setPinnedMods,
      purgePresets,
      adoptPresetRefs,
      // 依赖处置（选项 A）：求解结果 + 受影响预设激活集补入 + 预设页跳转意图
      depSolve,
      enableDepInPresets,
      depIgnores,
      setDepIgnores,
      langs: depLangs,
      resolveClosure,
      jump: manageJump,
      onJumpHandled: () => setManageJump(null),
      reload: reloadRealmInstalled,
      repoIndex,
      repoBusy,
      onRepoCheck: (silent?: boolean) => void handleRealmRepoCheck(silent),
      showToast,
    },
  };

  const introTabProps: IntroTabProps = {
    t,
    // 媒体（语言无关）与介绍文本（分语言）分开传：前者只写顶层，后者写 i18n[curLang]
    media,
    patchMedia,
    introText: curIntroText,
    setIntroText,
    curLang,
    langLabel,
    isBaseLang,
    assetPreview,
    pickAsset,
    introSeconds,
    setIntroSeconds,
    introSkippable,
    setIntroSkippable,
    introShowName,
    setIntroShowName,
    // §6 `showGameName`（v1.25 由「基本信息」迁入「领域登场」共用组）
    showGameName,
    setShowGameName,
    loopMusic,
    setLoopMusic,
    maskOpacity,
    setMaskOpacity,
    background,
    setBackground,
    appearanceOpen,
    setAppearanceOpen,
  };

  return (
    <div className="relative flex h-full flex-col">
      {/* ⚠️ **刻意不在根容器挂构建蒙层**（2026-09-23 修正）：
          6 个 Tab 里只有「阵营巡礼」依赖 `data`（`readWikiIndex`/`readTownCards`/`readShowcaseSlides`
          全来自 `cache.json`），basic / intro / mods / adv / maps 不依赖或仅降级可用。
          而根蒙层会连**操作行（保存 / 预览）与 Tab 栏一起遮住**，用户只能干等。
          改为**在需要的区块上包 `<StageGate>`**（见下方巡礼 Tab）。
          ⇒ **后续改版**：哪个区块要等，就在哪个区块上包一层，本文件根容器无需改动。 */}
      {/* 操作行（§9.6.1）：「返回 + 标题」已上提至覆盖层左上角悬浮页头（v1.16），
          此处只留右侧操作。刻意**不与下方标签栏合并**：操作组在俄 / 德 / 波等语言下
          很长，与 6 个标签挤一行会溢出。 */}
      <div className="flex items-center gap-2 border-b border-slate-800 px-4 py-2.5">
        <div className="flex-1" />
        {/* 多语言（§9.6.1）：头部常驻，切换「当前编辑语言」并管理启用语言 */}
        <LanguagePicker
          selected={visibleLangs}
          currentLang={curLang}
          defaultLang={defaultLang}
          onToggle={toggleLang}
          onSelect={setCurLang}
          onSetDefault={setAsDefaultLang}
        />
        {/* 导出 */}
        {(
          <button
            type="button"
            onClick={handleExportClick}
            className="shrink-0 rounded-lg border border-slate-700 px-2.5 py-1.5 text-sm text-slate-200 hover:bg-slate-800/50"
          >
            {t.realm.export}
          </button>
        )}
        {/* 预览按钮组：主按钮 = 完整流程；下拉可选"只预览某一段" */}
        <div className="relative shrink-0">
          <div className="flex items-stretch overflow-hidden rounded-lg border border-blue-500/60">
            <button
              type="button"
              onClick={() => void startPreview("full")}
              className="px-2.5 py-1.5 text-sm font-medium text-blue-200 hover:bg-blue-500/15"
            >
              {t.realm.preview}
            </button>
            <button
              type="button"
              aria-label={t.realm.preview}
              aria-expanded={previewMenuOpen}
              onClick={() => setPreviewMenuOpen((v) => !v)}
              className={cn(
                "border-l border-blue-500/60 px-1.5 text-blue-200 hover:bg-blue-500/15",
                previewMenuOpen && "bg-blue-500/15",
              )}
            >
              <ChevronDown className="h-4 w-4" />
            </button>
          </div>
          {previewMenuOpen && (
            <>
              {/* 透明遮罩：点任意处关闭菜单（`fixed` 覆盖全屏，不参与布局） */}
              <div className="fixed inset-0 z-[80]" onClick={() => setPreviewMenuOpen(false)} />
              <div className="absolute right-0 top-full z-[81] mt-1 w-48 overflow-hidden rounded-lg border border-slate-700 bg-slate-900 py-1 shadow-2xl">
                {(
                  [
                    ["introOnly", t.realm.previewIntro],
                    ["shellOnly", t.realm.previewHome],
                    ["showcaseOnly", t.realm.previewShowcase],
                  ] as const
                ).map(([k, label]) => (
                  <button
                    key={k}
                    type="button"
                    onClick={() => {
                      if (k === "showcaseOnly") {
                        setPreviewMenuOpen(false);
                        setPreviewShowcaseRealm(buildPreviewRealm());
                        setPreviewShowcaseOpen(true);
                        return;
                      }
                      void startPreview(k);
                    }}
                    className="block w-full px-3 py-1.5 text-left text-sm text-slate-200 hover:bg-slate-800"
                  >
                    {label}
                  </button>
                ))}
              </div>
            </>
          )}
        </div>
        {/* 保存常驻头部（§9.6.1）：v1.26 由「确定」改「保存」——文案恒为「保存」，
            无未保存改动时置灰不可点，有改动才高亮；保存中显示「保存中…」。 */}
        <button
          type="button"
          onClick={handleSave}
          disabled={saving || !dirty}
          className={cn(
            "shrink-0 rounded-lg px-4 py-1.5 text-sm font-medium",
            dirty
              ? "bg-gradient-to-br from-blue-500 to-indigo-500 text-white shadow-md shadow-blue-600/40 hover:opacity-90 disabled:opacity-60"
              : "cursor-not-allowed bg-slate-800 text-slate-500",
          )}
        >
          {saving ? t.common.saving : t.common.save}
        </button>
      </div>

      {/* 标签栏（§9.6.1） */}
      <div className="flex flex-none items-center gap-2 border-b border-slate-800 px-8 py-2.5">
        {(
          [
            ["basic", t.realm.tabBasic],
            ["intro", t.realm.tabIntro],
            ["mods", t.realm.tabMods2],
            ["adv", t.realm.tabAdvanced],
            ["maps", t.realm.tabMaps],
            ["showcase", t.realm.tabShowcase],
          ] as const
        ).map(([k, label]) => (
          <button
            key={k}
            type="button"
            onClick={() => setTab(k)}
            className={cn(
              "rounded-lg px-3 py-1.5 text-sm font-medium transition-colors",
              tab === k
                ? "bg-blue-500/20 text-blue-200"
                : "text-slate-400 hover:bg-slate-800/70 hover:text-white",
            )}
          >
            {label}
          </button>
        ))}
      </div>

      <div className="relative min-h-0 flex-1 overflow-y-auto p-4">
        <div
          className={cn(
            "mx-auto space-y-4",
            tab === "mods" ? "w-full max-w-6xl" : tab === "maps" ? "h-full w-full max-w-6xl" : "max-w-xl",
          )}
        >
          {tab === "basic" && <BasicTab {...basicTabProps} />}
          {tab === "adv" && <AdvancedTab {...advTabProps} />}
          {tab === "maps" && <MapsTab {...mapsTabProps} />}
          {tab === "mods" && <ModsTab {...modsTabProps} />}
          {tab === "intro" && <IntroTab {...introTabProps} />}
          {tab === "showcase" && (
            /* 只有这个 Tab 依赖缓存（`data` + `assets`）；用它自己的区块包裹，
               顶栏返回 / 保存 / 预览与其余 5 个 Tab 全程可用。
               后续若本 Tab 改为只需要 `data`，改这里的 `deps` 即可。 */
            <StageGate deps={["data", "assets"]}>
              <ShowcaseTab
                showcase={showcase}
                onChange={setShowcase}
                realmId={currentId}
                enabledIds={presetsReady ? realmSelected : undefined}
                presetsReady={presetsReady}
                curLang={curLang}
                onError={(msg) => showToast("error", msg)}
              />
            </StageGate>
          )}
        </div>
      </div>

      {/* 未保存离开确认（领域已创建，放弃的只是本页改动） */}
      <ConfirmDialog
        open={confirmLeave}
        title={t.realm.unsavedTitle}
        description={t.realm.unsavedDesc}
        confirmText={t.realm.discard}
        cancelText={t.realm.keepEditing}
        danger
        onCancel={() => setConfirmLeave(false)}
        onConfirm={() => {
          setConfirmLeave(false);
          onCancel();
        }}
      />

      {/* 整合模组重名提示（§9.3.3）：建议改名后再整合，玩家可强制继续 */}
      <ConfirmDialog
        open={modConflicts !== null}
        title={t.realm.modConflictTitle}
        description={
          t.realm.modConflictDesc +
          (modConflicts ?? [])
            .map((c) => {
              const name = installed.find((m) => m.id === c.id)?.name ?? c.id;
              const why =
                c.reason === "game" ? t.realm.modConflictGame : t.realm.modConflictDiverged;
              return `\n· ${name}（${c.id}）— ${why}`;
            })
            .join("")
        }
        confirmText={t.realm.modConflictProceed}
        cancelText={t.realm.modConflictBack}
        danger
        onCancel={() => setModConflicts(null)}
        onConfirm={async () => {
          setModConflicts(null);
          await performSave();
        }}
      />

      {/* 导出领域（§9.6.1 / §7.1 / §7.3）：先校验已保存无差异，再由弹窗接管三态
          （路径与选项 → 带进度与取消的打包中 → 产物与发布自检告警的结果态）。
          蒙层不可点关，避免误触丢设置；重 IO 已在后端移出主线程，窗口全程可响应。 */}
      <ExportRealmDialog
        open={exportOpen}
        realmId={currentId}
        onClose={() => setExportOpen(false)}
        resolveCacheMods={resolveExportCacheMods}
        onExported={(r) => showToast("success", t.realm.exportOk.replace("{path}", r.path))}
      />

      {/* 巡礼预览：直开播放页（不经过登场演出），用编辑期状态，不落盘 */}
      {previewShowcaseOpen && previewShowcaseRealm && (
        <ShowcaseOverlay
          realm={previewShowcaseRealm}
          lang={t.uiLang}
          root={assetRoot}
          enabledIds={presetsReady ? realmSelected : undefined}
          onExit={() => {
            setPreviewShowcaseOpen(false);
            setPreviewShowcaseRealm(null);
          }}
        />
      )}

      {/* B5 预览：临时全屏覆盖层播放登场演出 + 领域主界面，不产生持久改动（不激活、不落盘） */}
      {previewOpen && previewRealm && (
        <div className="fixed inset-0 z-[200] bg-black">
          {previewPhase === "intro" ? (
            <RealmIntroOverlay
              realm={previewRealm}
              root={previewRoot}
              preview
              onFinish={() => {
                // 只预览登场：演完即退出；其余模式接着进领域主界面
                if (previewMode === "introOnly") {
                  setPreviewOpen(false);
                  setPreviewPhase("intro");
                  return;
                }
                setPreviewPhase("shell");
              }}
            />
          ) : (
            <RealmShell
              realm={previewRealm}
              preview
              // 编辑期启用集合：预览里的「阵营巡礼」要用它，否则读磁盘旧值 → 城镇列表不准
              enabledIds={presetsReady ? realmSelected : undefined}
              onClosePreview={() => {
                setPreviewOpen(false);
                setPreviewPhase("intro");
              }}
              onEnter={async () => false}
              onExitRealm={async () => {}}
              onReplayIntro={() => {}}
            />
          )}
        </div>
      )}

      {/* 整理游戏自带地图（§8.3.4）：居中模态框，无标题栏 / 关闭，进行中禁用取消 */}
      {showOrganize && (
        <div
          className="fixed inset-0 z-[150] flex items-center justify-center bg-black/65 p-4 backdrop-blur-sm"
          onClick={(e) => {
            if (!organizeBusy && e.target === e.currentTarget) setShowOrganize(false);
          }}
        >
          <div className="w-full max-w-md rounded-xl border border-slate-700/60 bg-slate-900 p-5 shadow-2xl">
            <h3 className="mb-2 text-base font-semibold text-slate-100">{t.realm.orgTitle}</h3>
            <p className="mb-3 text-sm text-slate-400">{t.realm.orgBody}</p>
            <ul className="mb-4 space-y-1.5 text-sm text-slate-300">
              <li>· {t.realm.orgP1}</li>
              <li>· {t.realm.orgP2}</li>
              <li>· {t.realm.orgP3}</li>
            </ul>
            <div className="flex justify-end gap-2">
              <button
                type="button"
                disabled={organizeBusy}
                onClick={() => setShowOrganize(false)}
                className="rounded-lg border border-slate-700 px-3 py-1.5 text-sm text-slate-300 hover:bg-slate-800 disabled:opacity-50"
              >
                {t.common.cancel}
              </button>
              <button
                type="button"
                disabled={organizeBusy}
                onClick={async () => {
                  setOrganizeBusy(true);
                  try {
                    await applyBlock(true);
                    setOrganized(true);
                  } catch (e) {
                    showToast("error", t.realm.error.replace("{msg}", backendErrorText(e)));
                  } finally {
                    setOrganizeBusy(false);
                    setShowOrganize(false);
                  }
                }}
                className="rounded-lg bg-gradient-to-br from-blue-500 to-indigo-500 px-3 py-1.5 text-sm font-medium text-white shadow-md shadow-blue-600/40 hover:opacity-90 disabled:opacity-50"
              >
                {organizeBusy ? t.realm.orgDoing : t.realm.orgStart}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
