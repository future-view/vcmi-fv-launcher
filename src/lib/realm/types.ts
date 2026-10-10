/**
 * 领域（Realm）前端类型——对齐设计文档 §6「领域元数据规格（realm.json）」。
 *
 * 路径：`{realm-dir}/realm.json`，是领域元信息的**唯一权威**（§3.1「目录即领域」）。
 */

/** 领域私有副本台账条目（realm-mods-management §3.2）：
 * 维护者在领域沙箱内从仓库下载/更新、或导入模组文件所得的私有副本，
 * 实体存于 `{realm}/vcmi/Mods/<id>`，不参与引用建链/整合复制/激活清理
 * （`apply_realm_mods` 的 pinned 保护）。 */
export interface RealmModPin {
  id: string;
  name?: string;
  version?: string;
  /** `"repo"` = 从仓库下载/安装/更新；`"import"` = 导入模组文件 */
  source?: "repo" | "import";
  repositoryUrl?: string;
  downloadUrl?: string;
  sha256?: string;
  pinnedAt?: string;
}

/** 领域模组引用与整合（realm-mods-management §3 定稿）：
 * - `borrowed` = **引用清单**：只申明领域依赖哪些「非领域目录」的模组（user/game 源混合）。
 *   user 源建目录联接（领域隔离后 VCMI 读不到用户目录）；game 源仅登记不建链（天然可见）。
 * - `bundled` = 整合：复制模组本体到领域 `Mods/`（随 `.vcmirealm` 打包分发）。
 * - `pinned` = 领域私有副本台账（下载/导入所得，激活时永不清理）。
 * - `depIgnores` = 依赖忽略集（依赖处置）：不可得依赖的静音清单（**小写** id）。VCMI 引擎
 *   本身容忍依赖缺失（BROKEN 剔除不阻断启动），忽略 = 纯 UI 静音，不干预激活。
 *
 * `borrowAll` 字段已废弃移除（v2 定稿，不保留兼容读取）；由一次性「全部引用」动作替代。 */
export interface RealmMods {
  bundled?: string[];
  borrowed?: string[];
  pinned?: RealmModPin[];
  depIgnores?: string[];
}

/** 领域素材（路径相对 `{realm-dir}`）。字段名对齐设计 §6。 */
export interface RealmAssets {
  /** 壁纸图片：领域界面 / 登场演出背景（设计 §6 的 `wallpaper`） */
  wallpaper?: string;
  /** 封面图片：切换领域列表显示。未配置时用壁纸图片缩放生成（设计 §6 的 `cover`） */
  cover?: string;
  /** 背景音乐：领域主界面播放（设计 §6 的 `music`） */
  music?: string;
  /** 介绍音频：领域登场演出播放，支持 ogg / mp3（设计 §6 的 `introMusic`） */
  introMusic?: string;
  /** 介绍视频：领域登场演出 */
  introVideo?: string;
  /** 介绍文本（默认语言）：领域界面 / 登场演出显示，最大 1000 字 */
  introText?: string;
}

/** 主界面背景图片滤镜（主题化，作者设定；§6 的 `main.background`）。字段省略即取默认值。 */
export interface RealmBackgroundFilter {
  /** 亮度 0–200（默认 100） */
  brightness?: number;
  /** 模糊（px，默认 0） */
  blur?: number;
  /** 饱和度 0–200（默认 100） */
  saturate?: number;
  /** 灰度 0–100（默认 0） */
  grayscale?: number;
  /** 对比度 0–200（默认 100） */
  contrast?: number;
  /** 色相旋转（deg，默认 0） */
  hueRotate?: number;
}

/** 领域主界面配置（设计 §6 的 `main`） */
export interface RealmMain {
  /** 是否循环播放 `assets.music`（默认 true） */
  loopMusic?: boolean;
  /** 主界面文字蒙层透明度 0–100（默认 80）。0=全透明，100=不透明 */
  maskOpacity?: number;
  /** 主界面背景图片滤镜（主题化，作者设定） */
  background?: RealmBackgroundFilter;
}

/** 登场演出配置（§5.4 / §5.5）。是否登场由是否配置素材决定，故无主开关。 */
export interface RealmIntro {
  /** 是否允许跳过演出（ESC / 点击右下角）。设计 §6 拼写为 `skipable` */
  skipable?: boolean;
  /** 是否显示领域名称 */
  showRealmName?: boolean;
  /** 演出时长（毫秒）。仅在「无介绍视频」时生效；0/空 = 按字数自动推算 */
  durationMs?: number;
}

/** 单语言覆盖（设计 §6 的 `i18n.<lang>`）：按启动器语言覆盖顶层同名字段。
 * `assets` 可只写需要覆盖的项，从而支持「不同语言不同封面/介绍视频」。 */
export interface RealmI18nEntry {
  name?: string;
  description?: string;
  /** 领域标签（作者自定义，多个；用于切换领域 / 远程领域页的快速筛选）。
   * **键存在即声明**：空数组 = 「该语言无标签」的显式声明（发布时覆盖索引）；
   * 缺键 = 未声明（保留索引既有值）。编辑器保存时只会对有内容的语言条目写入本键。 */
  tags?: string[];
  assets?: RealmAssets;
}

/** 战役场景条目（H3C 头部内嵌场景表）。 */
export interface RealmMapScenario {
  /** 场景名（regionName）：官方战役为地区 / 关卡名，自制战役常填剧情梗概；可为空 */
  name: string;
  /** 子地图文件名（如 wan003.h3m）；磁盘上可为空 */
  mapFile: string;
  /** 场景默认难度 0..4（简单/普通/困难/专家/不可能），未知为 -1 */
  difficulty: number;
}

/** 领域专属地图清单项（B7 / 集成地图增强）。`file` 为收纳仓库内相对路径（含 maps/ 或 campaigns/ 前缀）。 */
export interface RealmMapEntry {
  file: string;
  /** `map` | `campaign` | `vmap` */
  kind?: string;
  /** VCMI 语言 identifier（chinese/english/…），未识别为 unknown */
  lang?: string;
  name?: string;
  description?: string;
  /** ROE / AB / SOD / HOTA / WOG / VCMI */
  version?: string;
  difficulty?: number;
  players?: number;
  humans?: number;
  sizeW?: number;
  sizeH?: number;
  sizeClass?: string;
  victory?: string;
  loss?: string;
  scenarioCount?: number;
  /** 战役场景表（仅 `kind === "campaign"`；index.json v2 起结构化） */
  scenarios?: RealmMapScenario[];
  size: number;
  /** 文件修改时间（unix 毫秒） */
  modifiedAt?: number;
}

/** 领域信息页配置（`realm.json` 的 `info`，设计 §9.3）。由领域编辑者预设，玩家可在页面临时改动。 */
export interface RealmInfo {
  /** 领域信息页是否默认展开**原版内容**（`core` 与 9 个原版城镇）。
   * 省略即 false：原版内容不属于领域自带内容，默认收起以降低信息密度。 */
  showCore?: boolean;
}

/** 阵营巡礼单牌组项（设计 docs/features/faction-showcase.zh-CN.md.3） */
export interface ShowcaseDeckEntry {
  /** 必填：缓存目录名 = mod 的 `sanitize_id`，用于定位缓存目录 */
  modId: string;
  /** 必填：阵营 id（factions[].factionId） */
  factionId: string;
  /** 字幕：VCMI 语言 identifier → 文案；缺省回退阵营名（§6.1） */
  caption?: Record<string, string>;
  /**
   * **阵营背景图**（相对 `{realm-dir}`，如 `assets/showcase/castle_bg.png`）。
   *
   * ⚠️ 它只替换播放页的**全窗口氛围底图**（那层 blur(30px) 的铺底），
   * **不替换幕布（城镇全景图）** —— 幕布上的建筑聚焦高亮是按原始 800×374 的
   * structures 坐标标定的，换底图会让高亮对不上位。
   */
  overrideBackground?: string;
  /** 单独覆盖本张的**焦点切换间隔**（ms）；缺省跟随领域级 */
  focusStepMs?: number;
  /** 单独覆盖本张的切换效果；缺省跟随领域级 */
  transition?: "fade" | "slide" | "dissolve";
  /**
   * **巡礼兵种**（有序，按勾选顺序）。非空即"只巡礼这些"；空则由领域级「兵种展示」决定。
   * 这些兵种会**自动带出各自的巢穴建筑**（进阶兵种取进阶巢穴）。
   */
  highlightCreatures?: string[];
  /**
   * **重点兵种**（`★` 标记，与 `highlightCreatures` 相互独立、通常是它的子集）。
   *
   * 播放页里这些兵种在兵种行左上角带 ★，聚焦到它们时放大展示区加金色光晕。
   * 与「巡礼兵种」分开的原因：进巡礼 ≠ 想强调，作者需要两个维度。
   */
  keyCreatures?: string[];
  /**
   * **重点建筑**（`★` 标记），与 `highlightBuildings` 相互独立。
   *
   * ⚠️ 与兵种保持**同一套语义**：`★ = 作者标记的重点`，而**不是**"参与巡礼"。
   * 从前建筑条的 ★ 表示"在巡礼序列里"，于是同一个符号在兵种行与建筑条含义不同；
   * 现在参与与否改用**透明度 + 更暗的边框**表达（不占符号），★ 只留给作者的重点标记。
   * 默认为空 → 默认**不打星**（巡礼建筑本来就不少，全打星等于没有重点）。
   */
  keyBuildings?: string[];
  /**
   * **重点英雄**（`★` 标记），与 `highlightHeroes` 相互独立。
   * 与兵种/建筑同一套语义：焦点项是★时，**巡礼区卡片加金色描边 + 外发光**。
   */
  keyHeroes?: string[];
  /** 高亮英雄 id（置顶） */
  highlightHeroes?: string[];
  /** 高亮建筑 id */
  highlightBuildings?: string[];
  /** 单独覆盖本张的停留时长（ms） */
  durationMs?: number;
  /** 本张音乐起始偏移（ms，跳过前奏直入主题句）。缺省 = 0（音乐来源仍由后端解析） */
  musicOffsetMs?: number;
  /** 本张音量 0–100。缺省 = 70 */
  musicVolume?: number;
  /** 本张改放领域背景音乐（不放阵营自己的音乐） */
  musicUseRealmBgm?: boolean;
  /** 本张音乐淡入淡出时长（ms）。缺省 = `MUSIC_FADE_MS`（800） */
  musicFadeMs?: number;
  /** 本张提前淡出时长（ms，切下一站前多久开始降音量；0 = 不提前淡出）。缺省 = `MUSIC_PRE_FADE_MS` */
  musicPreFadeMs?: number;
}

/** 阵营巡礼配置（`realm.json` 的 `showcase`，设计 §6.3） */
export interface RealmShowcase {
  /** 主开关；false = 主界面不显示入口（§6.1） */
  enabled?: boolean;
  /**
   * 登场演出结束后**直接进入阵营巡礼**（默认 false = 停在领域主界面，等玩家点按钮）。
   *
   * 只在"真的播过一场登场演出"之后触发一次（含中途 ESC / 「跳过」）；
   * 没有登场内容的领域不会误触发，**预览模式也不触发**（预览要停在主界面看效果）。
   */
  afterIntro?: boolean;
  /** true=自动推进；false=纯手动模式（§7.7） */
  autoPlay?: boolean;
  /**
   * 循环播放。省略/false（默认）= **每个阵营各展示一次后自动结束**（退出巡礼）；
   * true = 走完最后一站后从第一站重新开始。
   */
  loop?: boolean;
  /** 转场类型：fade=淡入淡出 / slide=推入 / dissolve=溶解（默认 fade，§9） */
  transition?: "fade" | "slide" | "dissolve";

  /** 中层：**每个焦点项**停留（ms，默认 3000，§7.6）。三类（兵种/建筑/英雄）共用 */
  focusStepMs?: number;
  /** `untilFocusExhausted=false` 时，焦点走完一轮是否从第一项重来（默认 true）。
   *  ⚠️ 只在「轮完一轮才切」**关闭**时才露面 —— 开着时停留时长恰好等于一轮，
   *  永远走不到第二轮，给了也是误导。 */
  focusLoop?: boolean;
  /** true（默认）= 焦点轮完一轮才切下一城镇（§7.6） */
  untilFocusExhausted?: boolean;
  /* 2026-09-21 删除 `maxFactionDurationMs`（「最大停留上限」）：停留时长由巡礼清单
     推算得出（格数 × 间隔），"防止单张过长"的自我防御阈值没有意义 —— 要短就减格子
     或调「焦点切换间隔」，两者都是作者可见的数字。 */
  /* 2026-09-21 删除（统一模型：英雄进主序列，不再有"独立轮换池"）：
     `focusWithBuildings` —— 建筑已是固定分类，此开关恒真；
     `heroRotation` / `heroRotationMs` —— 英雄不再旁支轮换；
     `heroClassBalanced` / `heroShuffle` / `heroCount` —— 轮换池概念消失
     （默认清单改由 `defaultHeroIds()` 给"每个职业各 1 个"）。 */

  /** 是否播放阵营自己的音乐（默认 true）。false = 整场不放阵营音乐（**没有音乐就安静**） */
  musicFaction?: boolean;
  /* 2026-09-21 删除（音乐细项**下沉到每个城镇抽屉**）：
     `musicCrossfade` —— 播放器从未读取（恒为交叉淡入），是个死开关；
     `musicFadeMs` / `musicOffsetMs` / `musicVolume` / `musicPreFadeMs` —— 领域级默认值没有
       界面入口（幽灵字段），改为**只在城镇抽屉里逐张可调**，缺省即代码常量。
     另删 `musicFallbackRealmBgm`：一般城镇都有专属音乐，把"没有就回退领域主 BGM"做成选项
     只增加理解成本 —— 现在恒定策略是**没有专属音乐就安静**。 */

  /** 自动生成时是否纳入原版 9 城镇（默认 false，§7.1）。
   *  ⚠️ 这是**公共选项**：两种模式都生效（手工模式下是"过滤不删除"，条目仍留在 `manual.deck` 里） */
  includeCore?: boolean;

  /**
   * 编排模式（**显式**记录，不再由"有没有手写牌组"推断）。
   *
   * 为什么必须显式：从前 `deck.length > 0` 即手工，于是"切回自动"只能把牌组**清空**才能生效
   * —— 手工编排的内容就此丢失，来回切换是**破坏性**的，作者不敢点。
   * 模式与数据分离后，切到自动只是**停用**手写牌组（数据保留），切回即恢复。
   * 缺省语义（老配置兼容）：有手写牌组 = manual，否则 auto（见 `normalizeShowcase`）。
   */
  mode?: "auto" | "manual";
  /**
   * **仅自动编排**生效的选项。
   *
   * 放进独立容器的意义：作用域成为**数据结构的属性**而不是 UI 的约定 ——
   * 将来新增"只影响自动"的选项时，漏归类就会在 `tsc` 里暴露；UI 也能据此自动标注作用域。
   */
  auto?: {
    /** 自动生成 deck 上限（默认 12，0=不限） */
    maxSlides?: number;
    /**
     * **随机种子**：有值时自动牌组按此种子洗牌（编辑器预览与运行时**共用**，保证"预览 = 实播"）。
     *
     * 缺省时按 `realm.id` 派生（`fallbackSeed`）—— 未显式随机过的领域也是"随机但稳定"的顺序。
     * 写入新种子的时机：点「换一批」/ 切回自动 / 重新开启阵营巡礼。
     */
    autoSeed?: number;
    /**
     * **每次进入巡礼都换一批**（默认 false）。
     *
     * 默认（false）= 顺序写死在 `autoSeed` 里，**每次播放都一样**（只有"换一批 / 切回自动 /
     * 重新开启"才会变）。开启后，每次**打开巡礼**都现算一个新种子：
     * - 只在**本次会话**生效，**不写回配置**（保存的顺序仍是 `autoSeed` 那一版）；
     * - 因此**编辑器里看到的顺序不再等于实际播放顺序**（编辑器仍按 `autoSeed` 显示）。
     */
    rerollOnEnter?: boolean;
  };
  /** **仅手工编排**生效的选项：手写牌组。切到自动时**保留不清空**（见 `mode`） */
  manual?: {
    /** 手写牌组；空/省略 = 自动生成（§7.1） */
    deck?: ShowcaseDeckEntry[];
  };

  /** 兵种展示：tiers=未升级（默认）/ upgraded=仅进阶 / all=全部。
   *  ⚠️ `highlight`（"仅高亮"）**已废弃**：现在"勾了巡礼兵种就以清单为准"是自动行为，
   *  该值只在老配置里可能残留，读取时按 `tiers` 处理（编辑器下拉里也显示为"未升级"）。 */
  creatureMode?: "tiers" | "upgraded" | "all" | "highlight";
  /** 英雄轮换池大小（0–4，默认 2）：同屏只显示 1 张卡，在池内按顺序/随机轮换 */
  heroCount?: number;
  /** 轮换池按职业均衡取样（保证每种职业都露脸，默认 true） */
  heroClassBalanced?: boolean;
  /** 轮换池随机洗牌，避免每次进入巡礼都是同一序列（默认 true） */
  heroShuffle?: boolean;
}

/** 数据隔离范围：realm=指向领域隔离目录（默认，隔离核心）；global=显式写回全局 VCMI 用户目录。 */
export type RealmDirScope = "realm" | "global";

/** 数据隔离配置（§3.2.1 / B7）：对应 dirs.json 的 5 个真实 key 的归属，以及游戏自带地图开关。 */
export interface RealmDirs {
  /** 用户数据目录（VCMI userDataPath） */
  dataPath?: RealmDirScope;
  /** 缓存目录（VCMI userCachePath） */
  cachePath?: RealmDirScope;
  /** 配置目录（VCMI userConfigPath） */
  configPath?: RealmDirScope;
  /** 日志目录（VCMI userLogsPath） */
  logsPath?: RealmDirScope;
  /** 存档目录（VCMI userSavePath） */
  savePath?: RealmDirScope;
  /** 游戏自带地图：show=显示，hide=屏蔽（§8.3）。这是屏蔽偏好的唯一权威 */
  gameMaps?: "show" | "hide";
  /** 配置共享（仅 configPath=realm 时生效）：true=复用全局同名文件（硬链接，实时双向）。
   * 覆盖 settings.json / keyBindingsConfig.json / persistentStorage.json / mapeditor.ini。 */
  configShare?: {
    /** 游戏设置 settings.json */
    settings?: boolean;
    /** 按键配置 keyBindingsConfig.json */
    keybinds?: boolean;
    /** 玩家进度/持久状态 persistentStorage.json */
    persistent?: boolean;
    /** 编辑器配置 mapeditor.ini */
    mapeditor?: boolean;
  };
}

export interface Realm {
  id: string;
  /** 领域版本，用于展示 */
  version?: string;
  /** 领域作者，用于展示 */
  author?: string;
  /** 创建日期 `YYYY-MM-DD`，自动生成 */
  createdAt?: string;
  /** 更新日期 `YYYY-MM-DD`，自动生成 */
  updateAt?: string;
  /** 领域名称（默认语言字符串）。其他语言见 `i18n.<lang>.name` */
  name?: string;
  /** 领域描述（默认语言字符串）。其他语言见 `i18n.<lang>.description` */
  description?: string;
  /** 顶层 `name` / `description` / `assets.introText` 所属语言（VCMI 语言标识）。
   * 它不是设计 §6 的字段，但编辑器需要它来判断"顶层字段属于哪个语言"。 */
  defaultLang?: string;
  /** 领域资源（默认语言） */
  assets?: RealmAssets;
  /** 是否显示游戏名称前缀（默认 false，§6）。
   * 开启后领域主界面与登场演出的领域标题带「游戏名：」前缀，游戏名按启动器界面语言取。 */
  showGameName?: boolean;
  /** 领域主界面配置（§6 的 `main`） */
  main?: RealmMain;
  /** 登场演出配置 */
  intro?: RealmIntro;
  /** 领域信息页配置（原版内容默认展开等，§9.3） */
  info?: RealmInfo;
  /** 阵营巡礼配置（设计 docs/features/faction-showcase.zh-CN.md） */
  showcase?: RealmShowcase;
  /** 领域激活时自动应用的模组预设 id（默认固定 `__realm__`，§3.3） */
  presetId?: string;
  /** 选定的模组引用（写入领域预设，§3.4） */
  mods?: RealmMods;
  /** 领域专属地图清单（与 `{realm-dir}/vcmi/Maps.vcmi-fv-realm/` 收纳仓库保持同步）+ 语言过滤配置 */
  maps?: {
    entries?: RealmMapEntry[];
    /** 游戏内语言过滤开关：开启后仅目标语言 + unknown 的地图在游戏内可见 */
    filterLang?: boolean;
    /** 过滤目标语言（VCMI identifier）；缺省时跟随启动器 UI 语言 */
    filterLangTarget?: string;
  };
  /** 数据隔离配置 */
  dirs?: RealmDirs;
  /** 领域标签（作者自定义，多个；用于切换领域 / 远程领域页的快速筛选）。
   * 顶层为 `defaultLang` 的兜底镜像（设计 §6）；逐语言标签见 `i18n.<lang>.tags`。
   * 编辑器保存时**恒写**（含空数组 = 显式清空；缺键 = 未声明、保留索引既有值）。 */
  tags?: string[];
  /** 来源标记：由远程仓库安装时写入，用于「有更新」判定；本地创建的领域没有此字段 */
  origin?: {
    repositoryUrl?: string;
    version?: string;
    sha256?: string;
    installedAt?: string;
  };
  /** 多语言覆盖：语言 id -> 覆盖字段。语言标识沿用 VCMI 标识（chinese / english / …） */
  i18n?: Record<string, RealmI18nEntry>;
  /** 编辑权（§6.2）：由后端 `list_realms` 依目录内 `lock` 计算，仅读态存在 */
  editable?: boolean;
  [key: string]: unknown;
}

export interface RealmList {
  realms: Realm[];
  activeRealm: string | null;
}
