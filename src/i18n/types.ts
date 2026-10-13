import type { UiLang } from "./langs";
import type {
  SettingEnumKey,
  SettingItemId,
  SettingTagId,
  SettingValueWord,
} from "@/generated/settingKeys";

/** 界面语言标识的唯一来源是 `src/i18n/langs.ts` 的登记表；词典注册表与之双向约束（缺词典 = 编译失败） */
export type { UiLang };

/** 设置项文案：标签 + 详细描述（描述默认折叠，勾选「详细描述」才显示） */
export interface SettingItemText {
  l: string;
  d: string;
}

/** 控件局部文案：与具体设置项无关的少数几条 */
export interface SettingsWords {
  /** 空值选项（如未选 AI 超分模型 = 不启用） */
  defaultOption: string;
  /** 动态候选首项「不使用」 */
  disabledOption: string;
  /** 动态候选：清单内的推荐模型（占位符 {name} 为文件名） */
  recommendedName: string;
  /** 动态候选：清单外自备模型，未验证（占位符 {name} 为文件名） */
  unverifiedName: string;
  /** 分辨率下拉：当前显示器原生分辨率标记 */
  currentDisplay: string;
}

/**
 * 设置面板文案的五段正文：由各语言的 `src/i18n/settingsText.<lang>.ts` 提供，
 * 在各语言词典（`zh-CN.ts` / `en.ts` / `ru.ts` …）的 `settings` 段展开合并 —— 只是文件切分，**不是第二套词典**。
 */
export type SettingsText = Pick<LangDict["settings"], "tags" | "items" | "enums" | "valueWords" | "words">;

export interface LangDict {
  uiLang: UiLang;
  /** 后端结构化错误码词典：code → 本地化模板，模板中用 {arg} 占位 */
  beErr: Record<string, string>;
  app: {
    brand: string;
    tabMods: string;
    tabWiki: string;
    tabSettings: string;
    tabRealms: string;
    language: string;
    debugMode: string;
    close: string;
    /** 语言模组缺失提示（占位符 {mods} 为模组 id 列表） */
    langModsMissing: string;
    /** 语言模组缺失提示条上的「从仓库安装」动作（mod-acquisition-update §7） */
    langModsInstall: string;
    /** 启动恢复领域时激活失败提示（占位符 {msg} 为后端错误） */
    realmActivateFailed: string;
    /** 领域浏览（未激活领域）页返回启动器主界面的入口 */
    backToLauncher: string;
    /** 语言设置同步失败提示（占位符 {msg} 为后端错误） */
    vcmiLangSettingFailed: string;
  };
  /** HD 素材来源（官方 HD 版素材 mod `hd-edition`）：设置页「FV启动器」标签下的状态卡 */
  hdEdition: {
    /** 卡片标题 */
    title: string;
    /** 说明：只在构建期读取 pak，基准图仍是原版素材 */
    hint: string;
    /** 已检测到（占位符 {dir} 为来源目录） */
    found: string;
    /** 未检测到 */
    missing: string;
    /** 可用档位（占位符 {scales} 形如 "x3, x2"） */
    scales: string;
    /** 已在设置中关闭 */
    disabled: string;
    /** 重新探测按钮 */
    refresh: string;
    /** 改档位 / 开关后需重建素材缓存的提示 */
    rebuildHint: string;
  };
  /** 外挂运行时组件（设置页「FV启动器」标签下的组件卡片） */
  runtime: {
    /** 卡片标题 */
    title: string;
    /** 说明：组件是可选的，未安装时相关功能自动回退 */
    hint: string;
    missing: string;
    /** 已安装（占位符 {version} 可能为 unknown → 回退 installedNoVer） */
    installed: string;
    installedNoVer: string;
    install: string;
    installing: string;
    cancel: string;
    remove: string;
    /** 未配置远程包（无法在线安装） */
    noPackage: string;
    /** 安装失败（占位符 {msg}） */
    failed: string;
  };
  wiki: {
    categories: string;
    factions: string;
    buildings: string;
    creatures: string;
    heroes: string;
    artifacts: string;
    spells: string;
    skills: string;
    terrain: string;
    terrains: string;
    terrainDetail: {
      moveCost: string;
      nativeTowns: string;
      impassable: string;
    };
    /** 技能/法术等级术语（VCMI NSecondarySkill / spellSchool：none/basic/advanced/expert） */
    skillLevels: {
      none: string;
      basic: string;
      advanced: string;
      expert: string;
    };
    /** 法术详情属性行标签（SpellDetail；" · Lv n" 前缀为设计约定保留） */
    spell: {
      type: string;
      school: string;
      level: string;
      mana: string;
      cost: string;
    };
    allMods: string;
    search: string;
    searchPlaceholder: string;
    /** 中栏「筛选」浮层：维度按类目声明，见 features/wiki/filters.ts */
    filter: {
      title: string;
      reset: string;
      all: string;
      /** 该类目暂无可筛选维度时的按钮提示 */
      none: string;
      byAlignment: string;
      byFaction: string;
      byCreatureLevel: string;
      bySpellLevel: string;
      bySchool: string;
      byCombined: string;
      byModType: string;
      combinedYes: string;
      combinedNo: string;
      levelWarMachine: string;
      byHeroClass: string;
      byArtifactClass: string;
      byCreatureTier: string;
      byAbility: string;
      byMod: string;
      tierBase: string;
      tierUpgraded: string;
      /** 选项搜索无结果 */
      noOption: string;
      byHeroSkill: string;
      byArtifactSlot: string;
      byBuildingDwelling: string;
      dwellingYes: string;
      dwellingNo: string;
    };
    empty: string;
    noImage: string;
    enableAnimation: string;
    generation: string;
    generating: string;
    pauseBuild: string;
    /** 构建阶段名（进度条标签） */
    stageData: string;
    stageAssets: string;
    stageHd: string;
    /**
     * 「何时可用」模板 —— 用**依赖阶段名列表**填充 `{stages}`，再补一句 `readyHintRest`。
     *
     * ⚠️ 不要为每种依赖组合各写一句（`data` / `data+assets` / `data+assets+hd` / 仅 `assets`…
     * 组合会越写越多且必然漏）。模板 + 阶段名拼接才能覆盖任意组合，见
     * `shared/WikiBuildOverlay.tsx::readyHint`。
     */
    readyHintTemplate: string;
    /** 阶段名之间的分隔符（中文「、」/ 英文 ", "） */
    stageSeparator: string;
    /** 模板后补充的一句：其余阶段在后台继续（不阻塞本页） */
    readyHintRest: string;
    /** 缓存素材未就绪时的占位文案（城镇背景等） */
    preparingBg: string;
    h3DataMissing: string;
    animation: string;
    alignment: {
      good: string;
      evil: string;
      neutral: string;
    };
    stats: {
      level: string;
      attack: string;
      defense: string;
      damage: string;
      health: string;
      speed: string;
      shots: string;
      mana: string;
      ai: string;
      growth: string;
      horde: string;
      doubleWide: string;
      mapAmount: string;
      upgradeOf: string;
      upgradeTo: string;
      yes: string;
    };
    creature: {
      abilities: string;
      attack: string;
      defense: string;
      damage: string;
      health: string;
      speed: string;
      shots: string;
      mana: string;
      morale: string;
      luck: string;
    };
    bonusSource: {
      creature: string;
      spell: string;
      hero: string;
      experience: string;
      commander: string;
      other: string;
    };
    hero: {
      spellPower: string;
      knowledge: string;
      biography: string;
      specialty: string;
      startingArmy: string;
      warMachine: string;
      secondarySkills: string;
      spells: string;
      spellBook: string;
      noSpellBook: string;
    };
    artifact: {
      classTreasure: string;
      classMinor: string;
      classMajor: string;
      classRelic: string;
      classScroll: string;
      /** 特殊宝物（VCMI ArtifactClass::SPECIAL，如战役/剧情宝物） */
      classSpecial: string;
      classOther: string;
      slotHead: string;
      slotShoulders: string;
      slotNeck: string;
      slotRightHand: string;
      slotLeftHand: string;
      slotTorso: string;
      slotRightRing: string;
      slotLeftRing: string;
      slotFeet: string;
      slotMisc: string;
      slotWarMachine: string;
      slotSpellbook: string;
      slotBackpack: string;
      headerClass: string;
      headerPrice: string;
      headerSlots: string;
      combined: string;
      components: string;
      partOf: string;
      /** 宝物效果条目无名称时的回退（原硬编码 "Effect"） */
      effect: string;
      effects: string;
    };
    sections: {
      buildings: string;
      creatures: string;
      heroes: string;
      startingArmy: string;
      gender: string;
      back: string;
      name: string;
      cost: string;
      specialty: string;
    };
  };
  mods: {
    leftTitle: string;
    leftSub: string;
    rightTitle: string;
    rightSub: string;
    search: string;
    all: string;
    unjoinedOnly: string;
    partial: string;
    showAll: string;
    joined: string;
    town: string;
    expansion: string;
    noSub: string;
    submods: string;
    detail: string;
    add: string;
    remove: string;
    loading: string;
    /** 模组列表分析中（启动蒙层主标题，见 docs/features/startup-responsiveness.zh-CN.md U1） */
    analyzing: string;
    /** 蒙层副提示：列表就绪后即可操作 */
    analyzingHint: string;
    loadingConfirm: string;
    emptyLeft: string;
    emptyRight: string;
    childOf: string;
    children: string;
    expand: string;
    /** 强制（全局必装模组，不可取消）角标 */
    forced: string;
    collapse: string;
    missing: string;
    missingTip: string;
    conflictTip: string;
    base: string;
    noDescription: string;
    /** Content.zip 加密提示（无法从原始包提取素材；已缓存素材不受影响） */
    encryptedZipHint: string;
    previewLarge: string;
    previewClose: string;
    /** 工具栏开关：hover 是否弹出城镇背景大图（默认关闭，持久化在 launcher.json） */
    townPreview: string;
    townBackground: string;
    parentBadge: string;
    /** 必需依赖守卫提示：禁用 {name} 会破坏 {by} 的 required depends */
    depBlocked: string;
    /** 启用失败：{name} 缺少必需依赖 {dep}（VCMI 不会启用缺依赖的模组） */
    depMissing: string;
    /** 依赖缺陷角标（依赖处置体系）：缺必需依赖、启用后会被引擎剔除 */
    depTag: string;
    /** 依赖缺陷角标 hover 说明 */
    depTagTip: string;
    /** 启用守卫分层处置：已自动跳过缺依赖的子模组（{list}） */
    depSkipSubs: string;
    /** 启用失败：{name} 与 {by} 互斥（conflicts） */
    conflictBlocked: string;
    /** 右栏视图切换：主体树（默认，可移除） */
    viewTree: string;
    /** 右栏视图切换：按类型分类（只读） */
    viewGrouped: string;
    /** 分类视图为只读时的提示 */
    readonlyHint: string;
    /** 树视图中「父模组未选中」的独立子模组分组标题 */
    orphanSubs: string;
    /** 与当前 VCMI 引擎版本不兼容（`compatibility.min/max` 区间不成立） */
    incompatible: string;
    /** 不兼容原因提示：要求 {range}，当前 {ver} */
    incompatTip: string;
    /** 启用失败：{name} 要求 VCMI {range}，当前 {ver} */
    incompatBlocked: string;
    /** 详情抽屉的兼容要求行：{range} */
    compatRange: string;
    /** 兼容要求行的标签 */
    compatLabel: string;
    /** 翻译模组基准语言与当前游戏语言不符（VCMI 会忽略该模组） */
    langMismatch: string;
    /** 翻译模组基准语言与当前游戏语言一致（建议启用） */
    langMatch: string;
    /** 语言不符提示：该模组对应 {lang}，当前 {cur} */
    langMismatchTip: string;
    /** 启用失败：{name} 是 {lang} 翻译模组，当前游戏语言 {cur} */
    langBlocked: string;
    /** 模组导入成功（`vcmi-fv:mods-installed` 事件横幅）：已安装 {name}（{id}） */
    installOk: string;
    /** 预设菜单：清除 validatedMods 校验缓存，强制 VCMI 重新校验 */
    revalidate: string;
    /** 校验缓存已清除（{n} 个模组） */
    revalidated: string;
    /** 没有可清除的校验缓存 */
    revalidateNone: string;
    /** 与 VCMI modType 枚举对齐的标签，完整 23 类 + all/Core */
    typeLabels: Record<string, string>;
  };
  /** 模组仓库（获取与更新）：索引/检查/安装/批量更新弹窗文案 */
  modsRepo: {
    /** 工具区按钮：检查更新 */
    check: string;
    /** 检查中 */
    checking: string;
    /** 检查完成：仓库索引共 {n} 个模组 */
    checkDone: string;
    /** 拉取失败（离线/网络） */
    offline: string;
    /** 仓库未提供版本（需 FV 聚合索引，M3 的 build-mod-index 补全） */
    noVersions: string;
    /** 类型标签栏尾部的「可更新」过滤 chip */
    tab: string;
    /** 可更新角标：{new} */
    badge: string;
    /** 过滤后空列表提示 */
    emptyUpdates: string;
    /** 抽屉「获取与更新」区块标题 */
    section: string;
    installedVer: string;
    latestVer: string;
    size: string;
    stars: string;
    /** 校验等级徽标（C2） */
    verified: string;
    weak: string;
    /** 来源仓库：{host} */
    source: string;
    /** 索引未预聚合时的 changelog 占位 */
    noChangelog: string;
    /** A1：SWR 后台校验发现远端索引更新后的提示（模组页横幅 / 领域面板 toast） */
    indexUpdated: string;
    /** 底部动作：更新到 {new} */
    updateTo: string;
    /** 底部动作：安装（D1：不启用） */
    installD1: string;
    /** 仓库新装完成横幅（不自动启用）：{name} */
    installedNote: string;
    /** 仓库更新完成横幅：{name} */
    updatedNote: string;
    /** 安装/更新失败：{name} {msg} */
    failed: string;
    /** 批量更新弹窗标题：{n} */
    dialogTitle: string;
    dialogIntro: string;
    /** 列表行版本跳转：{cur} → {next} */
    rowVersions: string;
    /** 进度文本：{name} {pct}%（{done}/{total}） */
    progress: string;
    /** 固定底栏主按钮 */
    start: string;
    /** 更新完成横幅：{n} */
    done: string;
    /** changelog 展开/收起 */
    changelogShow: string;
    changelogHide: string;
  };
  /** 模组管理页（P5 建立，P6 全面升级：状态/分类筛选、行级队列、删除、模组源就地配置） */
  modsManage: {
    /** 顶部入口按钮与页标题 */
    title: string;
    /** 返回预设页 */
    back: string;
    /** 搜索框占位 */
    search: string;
    /** 空列表 */
    empty: string;
    /** 列表统计：{n} */
    summary: string;
    /** 入口/检查按钮徽章 tooltip：{n} */
    badgeHint: string;
    /** 表头 */
    colMod: string;
    colType: string;
    colUpgradable: string;
    colStatus: string;
    colLocation: string;
    colDirName: string;
    colAction: string;
    /** 状态筛选 chips */
    statusAll: string;
    statusInstalled: string;
    statusDownloadable: string;
    statusUpdatable: string;
    /** 行状态列 */
    stateInstalled: string;
    stateDownloadable: string;
    stateUpdatable: string;
    /** 安装位置 */
    locationUser: string;
    locationGame: string;
    /** 安装位置：领域模组目录（realm-mods-management：领域位置） */
    locationRealm: string;
    /** 行操作 */
    actInstall: string;
    actUpdate: string;
    actOpen: string;
    actDelete: string;
    actCancel: string;
    /** 队列态文案 */
    qQueued: string;
    qRunning: string;
    qDone: string;
    qFail: string;
    /** 下载中（{pct}） */
    qDownloading: string;
    /** 下载完成、解包安装阶段 */
    qInstalling: string;
    /** 删除二次确认（输入目录名匹配才可确认） */
    delTitle: string;
    delHint: string;
    delGameWarn: string;
    delConfirm: string;
    deleteDone: string;
    deleteFail: string;
    /** 模组源弹层（与官方共用 vcmiSettings 键，就地编辑） */
    sourceTitle: string;
    sourceDefault: string;
    sourceExtra: string;
    /** 台账来源小标签（仓库） */
    sourceRepo: string;
    /** 「自动检测更新」开关（launcher.autoCheckRepositories） */
    autoCheckLabel: string;
    /** P9 检查进度两阶段文案 */
    progressIndex: string;
    progressMeta: string;
    /** P10 行详情抽屉字段标签 */
    drawerDesc: string;
    drawerAuthor: string;
    drawerSize: string;
    drawerStars: string;
    drawerRepo: string;
    /** 更新日志区块标题（行详情抽屉 v2：索引 changelog 版本倒序） */
    drawerChangelog: string;
  };
  /**
   * 导入游戏文件（底栏「游戏数据」左侧入口 + 单页四态弹层）。
   * 由原 `mods.install*` 迁移扩展而来；模组成功横幅仍复用 `mods.installOk`。
   */
  importFiles: {
    /** 底栏入口按钮 */
    button: string;
    title: string;
    sub: string;
    /** 缺省描述清单（默认展开；收起后由头部类型徽标承载） */
    typesTitle: string;
    typesCollapse: string;
    typesExpand: string;
    /** 头部类型徽标（{n} = 类型数）：hover / 聚焦弹出完整清单气泡 */
    typesBadge: string;
    typeMapT: string;
    typeMapD: string;
    typeCampaignT: string;
    typeCampaignD: string;
    typeZipT: string;
    typeZipD: string;
    typeConfigT: string;
    typeConfigD: string;
    typeSaveT: string;
    typeSaveD: string;
    typeChroniclesT: string;
    typeChroniclesD: string;
    typeHdT: string;
    typeHdD: string;
    typeDirT: string;
    typeDirD: string;
    /** 拖放与选择 */
    dropHint: string;
    dropActive: string;
    pickFiles: string;
    pickFilesTitle: string;
    /** 目录入口（递归导入目录内受支持文件） */
    pickDir: string;
    pickDirTitle: string;
    /** 「选择文件」原生对话框分组名（平台会自动追加扩展名列表；首位聚合全部类型） */
    dlgFilterAll: string;
    dlgFilterExe: string;
    /** 列表视图过滤 chips（按后端识别类型归类；与对话框分组互不影响） */
    filterLabel: string;
    filterAll: string;
    filterMap: string;
    filterCampaign: string;
    filterSave: string;
    filterMod: string;
    filterConfig: string;
    filterChronicles: string;
    filterHd: string;
    filterDir: string;
    /** 列表头 */
    selectedCount: string;
    /** 筛选显示计数（{n} = 筛选后可见项数）：仅非「全部」筛选时显示 */
    filterShowing: string;
    /** 筛选下无匹配项时的列表占位提示 */
    filterEmpty: string;
    clear: string;
    removeItem: string;
    scanning: string;
    /** 冲突策略 */
    conflictLabel: string;
    conflictSkip: string;
    conflictOverwrite: string;
    conflictRename: string;
    conflictRenameNote: string;
    /** 冲突计数：冲突 {n} 项（将{action}） */
    conflictCount: string;
    conflictActionSkip: string;
    conflictActionOverwrite: string;
    conflictActionRename: string;
    /** 类型徽标（单字缩写） */
    kindDir: string;
    kindMap: string;
    kindCampaign: string;
    kindZip: string;
    kindConfig: string;
    kindSave: string;
    kindChronicles: string;
    kindHd: string;
    kindNone: string;
    /** zip 识别徽标与计数 */
    zipAsMod: string;
    zipAsMaps: string;
    zipAsSaves: string;
    zipTryMod: string;
    zipMapsCount: string;
    zipSavesCount: string;
    /** zip 改判下拉 */
    overrideLabel: string;
    overrideAuto: string;
    overrideMod: string;
    overrideMaps: string;
    overrideSaves: string;
    /** HD 目录预检 */
    hdLang: string;
    hdPaks: string;
    hdMissing: string;
    /** 目录递归统计（{n} = 可导入文件数 / 忽略数） */
    dirFiles: string;
    dirIgnored: string;
    /** 历代记识别结论 */
    innoYes: string;
    innoNo: string;
    /** 执行中 */
    executing: string;
    hdCopying: string;
    /** 结果态 */
    resultTitle: string;
    resultSummary: string;
    /** 结果态目标目录 chips 的引导标签（具体目录名作为 chip 文本） */
    openDirLabel: string;
    modsSavedHint: string;
    hdFilterHint: string;
    chroniclesHint: string;
    openNative: string;
    configRestartHint: string;
    /** 逐项状态徽标 */
    statusAdded: string;
    statusReplaced: string;
    statusSkipped: string;
    statusFailed: string;
    statusUnsupported: string;
    /** 不支持项注记（列表徽标旁） */
    unsupportedNote: string;
    /** 页脚按钮 */
    start: string;
    cancel: string;
    close: string;
    /** 顶部警告条 */
    realmWarn: string;
    vcmiRunningWarn: string;
    /** 领域模式下整窗拖放被拦截的全局提示 */
    realmBlocked: string;
    /** 报错（后端原文透传） */
    preflightFail: string;
    execFail: string;
  };
  preset: {
    save: string;
    more: string;
    new: string;
    rename: string;
    /** 内置预设禁用项的 title 提示（不可重命名） */
    builtinNoRename: string;
    /** 内置预设禁用项的 title 提示（不可删除） */
    builtinNoDelete: string;
    import: string;
    export: string;
    report: string;
    delete: string;
    newName: string;
    inheritFrom: string;
    /** 新建预设弹窗：起始内容区块标题 */
    startFrom: string;
    /** 新建预设弹窗：起始内容二选一——空白 */
    blank: string;
    /** 新建预设弹窗：起始内容二选一——复制现有预设 */
    copyExisting: string;
    /** 新建预设弹窗：空白选项说明（不复制任何预设的勾选） */
    blankNote: string;
    /** 新建预设弹窗：复制选项说明（{name}=来源预设显示名，{n}=模组数） */
    copyNote: string;
    search: string;
    current: string;
    copy: string;
    nameExists: string;
    exportHint: string;
    importHint: string;
    reportTitle: string;
    reportGenerated: string;
    /** 报告标题拼接（{title} / {name} 占位） */
    reportHeading: string;
    reportModeBrief: string;
    reportModeFull: string;
    reportCopy: string;
    reportCopied: string;
    reportWidthCentered: string;
    reportWidthFull: string;
    reportEngine: string;
    reportGameLang: string;
    reportPresetKind: string;
    reportKindBuiltin: string;
    reportKindUser: string;
    /** 启用总数（{n}） */
    reportTotal: string;
    /** 可更新 / 未安装提示（{n}） */
    reportUpdatesHint: string;
    reportMissingHint: string;
    reportOverview: string;
    reportColItem: string;
    reportColCount: string;
    reportStatEnabled: string;
    reportStatBase: string;
    reportStatRequired: string;
    reportStatChosen: string;
    reportStatSettings: string;
    reportStatUpdates: string;
    reportStatMissing: string;
    reportSectionMods: string;
    /** 分组小标题（{type} / {n}） */
    reportGroupHeading: string;
    reportColMod: string;
    reportColId: string;
    reportColVersion: string;
    reportColNote: string;
    reportNoteBase: string;
    reportNoteRequired: string;
    /** 可更新备注（{v} = 远端版本） */
    reportNoteUpdate: string;
    reportNoteLangMismatch: string;
    reportNoteIncompat: string;
    reportUpdateMark: string;
    reportSettingsTitle: string;
    reportColParent: string;
    reportColSub: string;
    reportColState: string;
    reportStateOn: string;
    reportStateOff: string;
    reportMissingTitle: string;
    reportMissingNote: string;
    /** 精简模式缺失提示（{n}） */
    reportBriefMissing: string;
    importOk: string;
    importErr: string;
    saveOk: string;
    confirmDelete: string;
    confirmDeleteDesc: string;
    keepOne: string;
    inheritHint: string;
    copyClipboard: string;
    copiedOk: string;
    pasteFromClipboard: string;
    clipboardFail: string;
    clipboardEmpty: string;
    missingTitle: string;
    missingIntro: string;
    missingInstall: string;
    missingOnly: string;
    installing: string;
    missingDone: string;
    missingPartial: string;
  };
  launch: {
    launcher: string;
    launcherLaunching: string;
    nativeLauncher: string;
    nativeLauncherLaunching: string;
    editor: string;
    editorLaunching: string;
    gameData: string;
    help: string;
    language: string;
    unsaved: string;
    open: string;
    /** 启动前预检弹窗标题前缀（点明"为什么弹这个"） */
    langModCheck: string;
    /** 标题：未启用 {lang} 语言包 */
    langModTitleDisabled: string;
    /** 标题：未找到 {lang} 语言包 */
    langModTitleMissing: string;
    /** 当前游戏语言：{lang} */
    langModCurLang: string;
    /** 当前领域：{name}（仅领域模式显示） */
    langModCurRealm: string;
    /** 已装未启时，将启用的语言包（名称（id）），多个用「、」连接 */
    langModDetected: string;
    /** 后果：不启用时游戏内文字的表现 */
    langModConsequence: string;
    /** 本机未安装匹配的翻译模组 */
    langModNotInstalled: string;
    /** 建议：启用后再启动 */
    langModSuggestEnable: string;
    /** 可从模组仓库获取一个： */
    langModRepoHint: string;
    /** 仓库不可用时说明：{lang} */
    langModRepoUnreachable: string;
    /** 「启用并启动」按钮 */
    langModEnable: string;
    /** 「下载并启用」按钮 */
    langModInstall: string;
    /** 启动前检查的逃生口按钮：「跳过并启动」（通用词，不限语言/检查项） */
    preLaunchSkipAndLaunch: string;
    /** 勾选：以后自动启用 {lang} 语言包（不再提示） */
    langModAutoEnable: string;
    /** 「前往模组管理」按钮 */
    langModGoToMods: string;
    /** 领域模式下「前往模组管理」按钮（将退出当前领域） */
    langModGoToModsRealm: string;
    /** 下载中提示 */
    langModInstalling: string;
    /** 启动按钮：本地扫描中 */
    checking: string;
    /** 启动按钮：拉起游戏中 */
    launching: string;
    /** 步骤条节点标签：语言包 */
    stepLangPack: string;
    /** 未装语言包时，弹窗内正在异步查询可下载候选 */
    resolvingRepo: string;
    /** 报告：已启用与游戏语言匹配的语言包（{lang}） */
    reportLangOk: string;
  };
  data: {
    title: string;
    sub: string;
    compileSection: string;
    dataSection: string;
    gameVersion: string;
    /** 内置核心 mod 的资产版本（不是引擎版本，仅供诊断） */
    coreModVersion: string;
    os: string;
    dataDir: string;
    gameDataDir: string;
    userDataDir: string;
    logsDir: string;
    userConfigDir: string;
    userModsDir: string;
    /** 游戏数据完整性体检（lod 条目 / 核心 mod / 资源容器，只读） */
    healthSection: string;
    healthSub: string;
    healthOk: string;
    healthBad: string;
    healthFatal: string;
    healthWarn: string;
    healthCheck: string;
    healthChecking: string;
    healthUnavailable: string;
    healthRoots: string;
    /** 体检项名称（id → 文案），id 见后端 `vcmi::health` */
    healthItems: Record<string, string>;
  };
  setup: {
    title: string;
    subtitle: string;
    gameDir: string;
    userDataDir: string;
    userConfigDir: string;
    userModsDir: string;
    gameDirHint: string;
    userDataDirHint: string;
    userConfigDirHint: string;
    userModsDirHint: string;
    choose: string;
    language: string;
    languageHint: string;
    autoDetect: string;
    save: string;
    error: string;
    missingClient: string;
    missingUserData: string;
    derived: string;
  };
  help: {
    title: string;
    /** 头部副标题（离线文档说明） */
    docHint: string;
    /** 目录区标题 */
    toc: string;
    searchPlaceholder: string;
    noResult: string;
    /** 外部链接提示（首版纯文本，不可点击） */
    linkExternalHint: string;
  };
  builtinPreset: {
    builtinTag: string;
  };
  settings: {
    title: string;
    search: string;
    openEditor: string;
    reset: string;
    /** 可见级别三档下拉 */
    levelLabel: string;
    levelCommon: string;
    levelAdvanced: string;
    levelAll: string;
    /** 详细描述开关 */
    showDesc: string;
    /** 运行信息弹窗 */
    runtimeInfo: string;
    /** 缩放联动组 */
    auto: string;
    preset: string;
    customPercent: string;
    runtimeVersion: string;
    runtimeModsInstalled: string;
    runtimeModsEnabled: string;
    runtimeResolution: string;
    runtimeGameDataLang: string;
    runtimeLastMap: string;
    runtimeLastSave: string;
    runtimeSetupCompleted: string;
    runtimeMainWindow: string;
    runtimeClose: string;
    runtimeHint: string;
    /** 标签名：键 = `SETTING_TAGS[].id`，由 `npm run gen:setting-keys` 穷举，新增标签必须补译 */
    tags: Record<SettingTagId, string>;
    /** 每个设置项的标签与详细描述：键 = 设置项 id（= VCMI `settings.json` 的 key） */
    items: Record<SettingItemId, SettingItemText>;
    /** 取值的专属显示名：`${设置项 id}.${原始值}`；缺项则兜底到 `valueWords`，再缺则原样显示 */
    enums: Partial<Record<SettingEnumKey, string>>;
    /** 通用取值兜底词（auto / off / hardware / nearest …）；语言选项除外，见 `src/lib/vcmiLangs.ts` */
    valueWords: Partial<Record<SettingValueWord, string>>;
    /** 控件局部文案（不使用 / 默认 / 推荐 / 未验证） */
    words: SettingsWords;
    /** 控件局部提示 */
    jsonInvalid: string;
    btnListCustom: string;
    /** 搜索无结果显示 */
    noResult: string;
    /** 重置确认标题与描述 */
    resetTitle: string;
    resetDesc: string;
    /** 保存中/已保存提示 */
    savedToast: string;
    saveError: string;
    /** 实时读取失败提示 */
    loadError: string;
    /** 作用域徽标（标题右侧）：全局共享 / 领域独立（§3.5.1 通用组件，两种入口共用） */
    scopeGlobal: string;
    scopeRealm: string;
    /** 作用域徽标 hover 说明（domain-agnostic：启动器设置页与领域设置页都会用到） */
    scopeGlobalHint: string;
    scopeRealmHint: string;
    /** 作用域面板：复制路径按钮与复制完成就地反馈 */
    scopeCopyPath: string;
    scopeCopied: string;
    /** 顶部区「更多」菜单（收纳运行信息 / 检查更新 / 打开配置目录 / 重置等维护类操作） */
    more: string;
    moreRuntimeDesc: string;
    moreUpdateCurrent: string;
    moreUpdateAvailable: string;
    moreOpenConfigDir: string;
    moreOpenConfigDirDesc: string;
    moreResetDesc: string;
    /** 运行信息弹窗：来源标注与分组标题（本配置目录 / 环境） */
    runtimeSource: string;
    runtimeGroupConfig: string;
    runtimeGroupEnv: string;
    /** 设置页「FV启动器」专用卡：L2 GitHub 镜像规则表（方案 §3.2.7；不进 settings schema，文案自成一套） */
    mirrorRules: {
      title: string;
      /** 空态副标题 */
      empty: string;
      edit: string;
      save: string;
      cancel: string;
      addRule: string;
      limit: string;
      del: string;
      confirmDel: string;
      up: string;
      down: string;
      enabled: string;
      noteLabel: string;
      hostsLabel: string;
      /** 主机输入框 placeholder */
      hostInput: string;
      presetGithub: string;
      modeLabel: string;
      modePrefix: string;
      modeTemplate: string;
      prefixLabel: string;
      templateLabel: string;
      /** 占位符行说明（点击插入） */
      templateHint: string;
      previewTitle: string;
      previewRaw: string;
      previewRelease: string;
      okParse: string;
      failParse: string;
      errHosts: string;
      errPrefix: string;
      errTemplate: string;
      badgeEnabled: string;
      badgeDisabled: string;
      /** 量词：n 主机 */
      unitHosts: string;
      /** 顶部引导（顺序/抗腐化） */
      guide: string;
      /** ⚠️ TLS 中转告警 */
      warnTls: string;
      /** 超时预算提示 */
      budgetHint: string;
    };
  };
  update: {
    /** 弹窗 / 设置页按钮标题 */
    title: string;
    checkNow: string;
    upToDate: string;
    notConfigured: string;
    latest: string;
    current: string;
    /** 「重要更新」琥珀徽标（清单 critical） */
    critical: string;
    notesEmpty: string;
    downloadNow: string;
    downloading: string;
    verifying: string;
    readyRestart: string;
    pendingApply: string;
    restartApply: string;
    restarting: string;
    /** 纯配置更新包**就地应用**后（无需重启）的提示 */
    appliedNoRestart: string;
    /** 强制更新 downloaded 后自动应用时的文案 */
    applying: string;
    /** 操作区「稍后」按钮的短标签（如 Later / 稍后），不可含说明性长句 */
    later: string;
    /** 「稍后」按钮下方的静默说明（如「7 天内不再提醒」），从按钮文案中拆出，避免长 locale 撑爆按钮 */
    laterHint: string;
    skipVersion: string;
    skipDefer: string;
    downloadedNote: string;
    /** manual 模式（ADR 0008 D7）：该平台不应用内更新，引导手动下载的说明 */
    manualHint: string;
    /** manual 模式主按钮：打开下载页 */
    openDownload: string;
    skippedToast: string;
    undoSkip: string;
    redDotReady: string;
    cancelDownload: string;
    openFolder: string;
    retry: string;
    failed: string;
    debugHint: string;
    /** ⚙ 红点 tooltip */
    redDotTooltip: string;
    /** 强制更新全屏阻断层 */
    forcedTitle: string;
    forcedDesc: string;
    exitLauncher: string;
  };
  common: {
    ok: string;
    cancel: string;
    close: string;
    minimize: string;
    /** 最大化窗口（窗口控件按钮 title） */
    maximize: string;
    /** 还原窗口大小（已最大化时的按钮 title） */
    restore: string;
    back: string;
    loading: string;
    /** 保存（编辑器顶栏常驻按钮；v1.26 起无未保存改动时置灰不可点） */
    save: string;
    saving: string;
    /** 全选（多选组标题按钮） */
    selectAll: string;
    /** 全不选（取消全部勾选） */
    selectNone: string;
    /** 提示条点击复制提示（title 属性） */
    copyTip: string;
    /** 提示条复制成功后的简短反馈 */
    tipCopied: string;
  };
  realm: {
    title: string;
    newRealm: string;
    editRealm: string;
    name: string;
    description: string;
    multiLang: string;
    multiLangOff: string;
    multiLangHint: string;
    langAdd: string;
    langAllAdded: string;
    langRemove: string;
    langRemoveConfirm: string;
    followDefault: string;
    importDupMods: string;
    presetNew: string;
    presetDelete: string;
    setDefault: string;
    default: string;
    /** 当前语言未设置登场文本时的编辑器提示 */
    introTextFallbackHint: string;
    // B5 预览
    preview: string;
    previewIntro: string;
    previewExit: string;
    previewNotice: string;
    previewHome: string;
    previewShowcase: string;
    /** 预览模式点了「启动游戏」的提示 */
    previewNoLaunch: string;
    // B7 高级选项：数据隔离 + 整合数据
    dataIsolation: string;
    isolationTip: string;
    dirData: string;
    dirCache: string;
    dirConfig: string;
    dirLogs: string;
    dirSave: string;
    scopeRealm: string;
    scopeGlobal: string;
    // 配置共享阀门（§配置共享）：默认复用全局，实时双向
    shareGlobal: string;
    isolateRealm: string;
    configShareTitle: string;
    configShareHint: string;
    cfgSettings: string;
    cfgKeybinds: string;
    cfgPersistent: string;
    cfgMapeditor: string;
    integratedData: string;
    advancedModsSummary: string;
    mapList: string;
    noMaps: string;
    deleteMap: string;
    mapsNeedSave: string;
    // B8 重命名
    idOccupied: string;
    renameActiveErr: string;
    blockMaps: string;
    blockMapsHint: string;
    orgTitle: string;
    orgBody: string;
    orgP1: string;
    orgP2: string;
    orgP3: string;
    orgStart: string;
    orgDoing: string;
    activate: string;
    deactivate: string;
    active: string;
    settings: string;
    delete: string;
    deleteConfirm: string;
    empty: string;
    emptySub: string;
    createRealm: string;
    createTitle: string;
    createIdPh: string;
    createIdHint: string;
    createNamePh: string;
    idErrEmpty: string;
    idErrInvalid: string;
    creating: string;
    unsavedTitle: string;
    unsavedDesc: string;
    discard: string;
    keepEditing: string;
    activateOk: string;
    deactivateOk: string;
    error: string;
    back: string;
    needName: string;
    vcmiRunningTitle: string;
    vcmiRunningHint: string;
    /** 进程守卫：检测到的具体进程名（`{list}` 替换） */
    vcmiRunningProcesses: string;
    killVcmi: string;
    /** 软提示（不阻断）：官方启动器仍在运行的标题 */
    launcherRunningTitle: string;
    /** 软提示：官方启动器仍作用于切换前的配置目录 */
    launcherRunningHint: string;
    /** 软提示的「仍要进入」按钮 */
    enterAnyway: string;
    /** 软提示的「忽略提醒」勾选（localStorage 持久化） */
    muteLauncherNotice: string;
    selfCheckTitle: string;
    selfCheckDesc: string;
    selfCheckFix: string;
    selfCheckIgnore: string;
    selfCheckHealed: string;
    /** 自检诊断文案：上次进入的领域目录已丢失（realm_missing） */
    selfCheckMsgRealmMissing: string;
    /** 自检诊断文案：领域切换未完成（dirs_mismatch） */
    selfCheckMsgDirsMismatch: string;
    /** 自检诊断文案：残留的领域激活记录（active_mismatch） */
    selfCheckMsgActiveMismatch: string;
    /** 自检诊断文案：地图目录联接缺失（maps_orphan） */
    selfCheckMsgMapsOrphan: string;
    /** 自检诊断文案：地图联接悬空（maps_dangling） */
    selfCheckMsgMapsDangling: string;
    /** 自检诊断文案：游戏目录出现了新的自带地图（maps_realboth） */
    selfCheckMsgMapsRealboth: string;
    /** 地图阀门修复动作：删除悬空联接 */
    selfCheckActionUnlink: string;
    /** 地图阀门修复动作：合并新出现的自带地图并重建联接 */
    selfCheckActionMerge: string;
    /** 地图阀门修复动作：把游戏目录彻底还原为未整理形态 */
    selfCheckActionRevert: string;
    /** 修复动作失败时的兜底提示 */
    selfCheckFailed: string;
    export: string;
    exportFilter: string;
    exportOk: string;
    exportNeedSaved: string;
    exportDirty: string;
    exportFail: string;
    exportTitle: string;
    exportDesc: string;
    exportPickPath: string;
    /** 导出：附带阵营巡礼素材缓存（**默认关**，体积大；不含原版阵营素材） */
    exportIncludeCache: string;
    exportIncludeCacheHint: string;
    /** 导出：分支二选一（取代原先「生成远程仓库索引 / 写入本地仓库」两个需互相解释的勾选） */
    exportModeShare: string;
    exportModePublish: string;
    /** 分段控件下的一句话解释 */
    exportModeHint: string;
    /** 分享分支：另出 `<文件名>.index.json` 片段（**默认关**，仅仓库不在本机时才需要） */
    exportFragment: string;
    exportFragmentHint: string;
    exportConfirm: string;
    /** 发布分支（§7.3）：仓库目录字段 */
    exportPublishRepo: string;
    exportPublishRepoHint: string;
    exportPublishPick: string;
    /** 仓库目录输入框的占位提示（**只描述形态，不得写真实路径 / 不得带开发者目录信息**） */
    exportPublishRepoPlaceholder: string;
    /** 发布分支：「将写入」清单（让用户扫一眼就知道动哪三个文件） */
    exportWillWrite: string;
    exportIndexMerge: string;
    exportDownloadUrl: string;
    /** 前缀来源说明（与后端 `URL_SOURCE_*` 一一对应）；文案不得写死单一托管站 */
    exportUrlSourceDeclared: string;
    exportUrlSourceGit: string;
    exportUrlSourceEntries: string;
    exportUrlSourceManual: string;
    exportUrlSourceNone: string;
    /** 前缀只读预览旁的覆盖入口（把「让用户填空」换成「让用户确认」） */
    exportEditUrl: string;
    exportPublishUrlBase: string;
    exportUrlOverrideHint: string;
    /** 自举成功的一次性提示（推导结果已写进仓库，后续发布无需再填） */
    exportUrlBootstrapped: string;
    /** 发布分支的就地校验 / 加载态 */
    exportNeedRepoDir: string;
    exportRepoIndexBroken: string;
    exportPreviewLoading: string;
    /** 预览期发现降级发布（发布按钮会被禁用）：{from} 仓库既有版本、{to} 本次版本 */
    exportPreviewVersionDown: string;
    /** 导出进度阶段文案（与后端 `realm-export-progress` 的 phase 一一对应） */
    exportPhasePrepare: string;
    exportPhaseCollect: string;
    exportPhasePack: string;
    exportPhaseIndex: string;
    exportPhasePublish: string;
    /** 进度计数行：{done}/{total} 个条目 */
    exportProgressFiles: string;
    /** 进度字节行：{done} / {total} */
    exportProgressBytes: string;
    /** 进行中按钮文案（区别于普通「取消」） */
    exportCancel: string;
    exportCancelled: string;
    exportDoneTitle: string;
    exportFailedTitle: string;
    /** 降级发布被后端硬拒（结构化错误 `version_downgrade` 的文案）：{from}/{to} */
    exportErrVersionDowngrade: string;
    exportRetry: string;
    exportAgain: string;
    /** 结果态字段标签 */
    exportFieldPath: string;
    exportFieldSize: string;
    exportFieldFiles: string;
    exportFieldIndex: string;
    exportFieldCover: string;
    /** 仓库条目动作 */
    exportRepoCreated: string;
    exportRepoUpdated: string;
    /** 重复发布的条目动作：所有可推导字段与仓库一致（不再是笼统的 updated） */
    exportRepoUnchanged: string;
    /** 变更字段摘要：{fields} */
    exportRepoChanged: string;
    /** 发布自检告警（按 code 映射；{arg} 是后端已格式化的补充信息）。
     * 文案不得写死单一托管站（启动器面向 Gitee / GitHub / 自建镜像） */
    exportWarnOverDownloadMax: string;
    exportWarnOverGitMax: string;
    exportWarnBundled: string;
    exportWarnBadId: string;
    exportWarnNoUrlBase: string;
    exportWarnVersionUnchanged: string;
    /** 版本号已变但包与旧包逐字节相同（黄） */
    exportWarnPkgUnchanged: string;
    exportWarnCopiedIntoRepo: string;
    /* —— 分片分发（docs/features/realms-sharded-package.zh-CN.md）—— */
    /** 「将写入」区的预估提示行：{count} = 预估片数（仅超阈值时显示） */
    exportShardEstimate: string;
    /** 结果态「分片」字段标签 */
    exportFieldParts: string;
    /** 已自动分片（信息级）：{arg} = 「N × ≤45.0 MB」 */
    exportWarnSharded: string;
    /** 分片后整包仍 >100MB：提示仓库体积增长（黄）：{arg} = 整包体积 */
    exportWarnOverTotalHint: string;
    /** 推不出 URL 前缀 ⇒ 给不出分片地址，已退回整包（黄）：{arg} = 整包体积 */
    exportWarnShardNeedsUrlBase: string;
    /** 切分失败，已按整包发布（黄） */
    exportWarnShardFailed: string;
    /** 结果态动作 chips */
    exportOpenRepo: string;
    exportOpenIndex: string;
    realmFile: string;
    importRealm: string;
    importOk: string;
    importNeedsMods: string;
    importConflictTitle: string;
    importConflictDesc: string;
    importOverwrite: string;
    importRename: string;
    introTitle: string;
    introEnabled: string;
    /** 「领域登场」标签页顶部说明（按领域主界面 / 登场演出两个渲染页面分组后） */
    introTabHint: string;
    introHint: string;
    /** 「共用配置」分组标题（领域主界面与登场演出共用） */
    introCommon: string;
    /** 「共用配置」分组说明 */
    introCommonHint: string;
    /** 「登场演出」分组标题（编辑器内） */
    introGroup: string;
    introVideo: string;
    introCover: string;
    introThumb: string;
    introPickVideo: string;
    introPickCover: string;
    introNoVideo: string;
    introNoCover: string;
    introPickThumb: string;
    introNoThumb: string;
    realmId: string;
    realmIdPh: string;
    realmIdHint: string;
    version: string;
    versionPh: string;
    author: string;
    authorPh: string;
    introClear: string;
    introText: string;
    introSeconds: string;
    introSkippable: string;
    introShowName: string;
    /** 游戏名（§6 `showGameName` 的前缀文本），按界面语言取 */
    gameName: string;
    /** 带游戏名的标题模板，如 `{game}：{name}`（中文全角冒号 / 英文半角） */
    gameNameTitle: string;
    showGameName: string;
    showGameNameHint: string;
    music: string;
    introMusic: string;
    pickMusic: string;
    pickIntroMusic: string;
    noMusic: string;
    noIntroMusic: string;
    musicHint: string;
    introMusicHint: string;
    loopMusic: string;
    loopMusicHint: string;
    /** 「领域主界面」小节标题（编辑器内，§6 `main`） */
    realmHome: string;
    /** 「主界面外观（主题化）」可折叠小节标题 */
    homeAppearance: string;
    /** 主界面文字蒙罩强度标签 */
    maskOpacity: string;
    /** 主界面外观小节提示 */
    appearanceHint: string;
    /** 背景滤镜：亮度 */
    filterBrightness: string;
    /** 背景滤镜：模糊 */
    filterBlur: string;
    /** 背景滤镜：饱和度 */
    filterSaturate: string;
    /** 背景滤镜：灰度 */
    filterGrayscale: string;
    /** 背景滤镜：对比度 */
    filterContrast: string;
    /** 背景滤镜：色相旋转 */
    filterHueRotate: string;
    /** 文字蒙层透明度说明 */
    maskOpacityHint: string;
    /** 亮度滤镜说明 */
    filterBrightnessHint: string;
    /** 模糊滤镜说明 */
    filterBlurHint: string;
    /** 饱和度滤镜说明 */
    filterSaturateHint: string;
    /** 灰度滤镜说明 */
    filterGrayscaleHint: string;
    /** 对比度滤镜说明 */
    filterContrastHint: string;
    /** 色相旋转滤镜说明 */
    filterHueRotateHint: string;
    introSkip: string;
    introReplay: string;
    bgmOff: string;
    bgmOn: string;
    readOnly: string;
    readOnlyHint: string;
    modConflictTitle: string;
    modConflictDesc: string;
    modConflictProceed: string;
    modConflictBack: string;
    modConflictGame: string;
    modConflictDiverged: string;
    playGame: string;
    realmInfo: string;
    switchRealm: string;
    exitRealm: string;
    checking: string;
    launching: string;
    launchingHint: string;
    modsMissingTitle: string;
    modsMissingDesc: string;
    switchEmpty: string;
    infoId: string;
    infoDescTitle: string;
    infoGroupTown: string;
    infoGroupOther: string;
    infoGroupMaps: string;
    infoNoContent: string;
    // Faction Showcase（design §11 + editor §10）
    showcase: string;
    tabShowcase: string;
    showcaseLoading: string;
    showcaseEmpty: string;
    /** 巡礼控制条计数（如「1 / 5 阵营」；{i}=当前、{n}=总数） */
    showcaseFactionCount: string;
    showcasePrev: string;
    showcaseNext: string;
    showcasePlay: string;
    showcasePause: string;
    showcaseSkip: string;
    showcaseEditor: string;
    showcaseEnable: string;
    showcaseEnableHint: string;
    showcasePlayback: string;
    showcaseAuto: string;
    showcaseManualMode: string;
    showcaseFactionDuration: string;
    showcaseFocusRotation: string;
    showcaseFocusStep: string;
    showcaseFocusHint: string;
    showcaseUntilFocus: string;
    showcaseMaxFactionDuration: string;
    /** 「实际停留」说明：`{v}` 会被替换成计算出的时长 */
    showcaseStayRelation: string;
    showcaseTransition: string;
    showcaseTransitionFade: string;
    showcaseTransitionSlide: string;
    showcaseTransitionDissolve: string;
    showcaseCreatureMode: string;
    showcaseCreatureTiers: string;
    showcaseCreatureAll: string;
    showcaseCreatureHighlight: string;
    showcaseHeroCount: string;
    showcaseMusic: string;
    showcaseMusicFade: string;
    showcaseMusicOffset: string;
    showcaseMusicOffsetHint: string;
    showcaseNoMusic: string;
    showcaseDeckHint: string;
    showcaseOrder: string;
    showcaseAutoHint: string;
    showcaseAddFaction: string;
    showcaseIncludeCore: string;
    /** 公共选项说明：关闭后手工编排也不带上原版城镇 */
    showcaseIncludeCoreHint: string;
    /** 手工列表里被公共选项挡掉的行标签 */
    showcaseCoreExcluded: string;
    /** 手工列表底部：共 N 个原版城镇被挡掉（`{n}` 替换） */
    showcaseCoreExcludedNote: string;
    /** 自动编排「换一批」（重新随机顺序） */
    showcaseReroll: string;
    showcaseAutoRandomHint: string;
    showcaseEdit: string;
    showcaseDelete: string;
    showcaseCreatures: string;
    showcaseHeroes: string;
    showcaseBuildings: string;
    showcaseCaption: string;
    showcaseOverrideBg: string;
    /** 上传自定义底图 */
    showcaseOverrideBgPick: string;
    /** 自定义底图的尺寸 / 格式 / 大小建议 */
    showcaseOverrideBgHint: string;
    showcaseHighlightCreatures: string;
    showcaseHighlightHeroes: string;
    showcaseHighlightBuildings: string;
    showcaseNoFactionToAdd: string;
    showcaseMaxSlides: string;
    showcaseMusicCrossfade: string;
    /** 巡礼内静音开关按钮的提示 */
    showcaseMute: string;
    showcaseUnmute: string;
    /** 「循环播放」开关与其说明 */
    showcaseLoop: string;
    showcaseLoopHint: string;
    // —— 逐项描述（§10：让作者不必猜每个选项的用途） ——
    showcaseFactionDurationHint: string;
    showcaseFocusStepHint: string;
    showcaseUntilFocusHint: string;
    showcaseFocusWithBuildingsHint: string;
    showcaseTransitionHint: string;
    showcaseCreatureModeHint: string;
    showcaseHeroCountHint: string;
    showcaseHeroRotationHint: string;
    showcaseMusicCrossfadeHint: string;
    showcaseMusicFadeHint: string;
    showcaseMaxSlidesHint: string;
    showcaseHighlightCreaturesHint: string;
    /** 一键把「兵种展示」设为「仅高亮」 */
    showcaseHighlightCreaturesApply: string;
    /** 播放器里"重点"标记的短词（★ 后面跟的文字） */
    showcaseKeyMark: string;
    /** 编辑器里 ★ 列的按钮提示 */
    showcaseKeyMarkTitle: string;
    /** 本张预计播放时间（抽屉里「音乐」之前的那块） */
    showcaseStayEstimate: string;
    /** 预计播放时间的构成说明（{n} 焦点格数，{s} 每格秒数） */
    showcaseStayEstimateHint: string;
    /** 被「最大停留上限」截断（{v} = 一轮实际需要的时长） */
    showcaseStayCapped: string;
    /** 列表行：作者定制过本张的角标 */
    showcaseCustomized: string;
    showcaseCustomizedHint: string;
    /** 列表行第二行的音乐来源 */
    showcaseMusicOwn: string;
    showcaseMusicOwnHint: string;
    showcaseMusicFallback: string;
    showcaseMusicFallbackHint: string;
    /** 「最大停留上限」的三种状态说明 */
    showcaseCapIsTheStay: string;
    showcaseCapClipping: string;
    showcaseCapIdle: string;
    /** 配置项作用域徽标 */
    showcaseScopeShared: string;
    showcaseScopeAuto: string;
    showcaseScopeManual: string;
    /** 两种模式的差异说明（写在切换处） */
    showcaseModeExplainAuto: string;
    showcaseModeExplainManual: string;
    /** 自动态下手工清单仍被保留的提示 + 清空入口 */
    showcaseManualKept: string;
    showcaseClearManual: string;
    /** 「包含原版 9 城镇」在手工模式下的语义补充 */
    showcaseIncludeCoreManual: string;
    /** 「进阶设置」折叠区（仅手工编排可展开） */
    showcaseAdvanced: string;
    showcaseAdvancedHint: string;
    /** 自动编排下提示"切到手工可解锁" */
    showcaseAdvancedAutoHint: string;
    /** 自动编排下提示"进阶项里有 {n} 项改过值，仍会生效" */
    showcaseAdvancedTweaked: string;
    /** 播放页：素材加载中的提示（就绪前不计时） */
    showcaseLoadingAssets: string;
    /** 登场演出结束后直接进入阵营巡礼（默认关闭） */
    showcaseAfterIntro: string;
    showcaseAfterIntroHint: string;
    /** 播放页：已暂停提示（含"从剩余时间继续"的说明） */
    showcasePaused: string;
    /** 手工查看模式：状态文字 / 进入提示 / 继续 / 继续（最后一张）/ 重播本张 */
    showcaseInspecting: string;
    showcaseInspectHint: string;
    showcaseResume: string;
    showcaseResumeLast: string;
    showcaseReplayHint: string;
    /** 进度指示：剩余秒数 */
    showcaseRemain: string;
    /** 手工查看详情卡 */
    showcaseNoDesc: string;
    /** 巡礼等级 Lv.0 的称谓（"Lv.n" 前缀为设计原型约定保留） */
    showcaseLevelZero: string;
    /** 编辑器巡礼列表拖拽手柄的悬浮提示 */
    showcaseDragHint: string;
    /** 秒（音乐偏移/淡入时长单位；"ms" 为设计约定保留） */
    showcaseSeconds: string;
    /** 分（巡礼时长人读文案，如「2 分 30 秒」中的「分」） */
    showcaseMinutes: string;
    showcaseInTour: string;
    showcaseNotInTour: string;
    showcaseUpgradedFrom: string;
    /** 巡礼区次块标题：招募建筑 */
    showcaseLinkedDwelling: string;
    /** 标签计数含义（参与/全部） */
    showcaseTabCountHint: string;
    /** 自动播放下的标签说明：列表只列参与巡礼的项 */
    showcaseTabAutoHint: string;
    /** 控制簇里的键盘提示（替代旧的浮动提示条） */
    showcaseKeyHint: string;
    /** 巡礼顺序说明（编辑器） */
    showcaseTourOrder: string;
    /** 只巡礼勾选的兵种 */
    showcaseCreaturesOnly: string;
    showcaseCreaturesOnlyHint: string;
    /** 逐组搜索框占位文案 */
    showcaseSearchCreature: string;
    showcaseSearchHero: string;
    showcaseSearchBuilding: string;
    /** 多选列表的三个批量按钮 */
    showcaseSelectAll: string;
    showcaseRandomPick: string;
    showcaseClearAll: string;
    /** 重点建筑是否也进入焦点序列 */
    showcaseBuildingsInFocus: string;
    showcaseBuildingsInFocusHint: string;
    /** 焦点一轮耗时估算（`{v}` 替换） */
    showcaseCycleEstimate: string;
    showcaseApplyEstimate: string;
    showcaseCycleEstimateUnknown: string;
    /** 「每个城镇停留」改为自动计算后的只读展示（`{v}` 替换） */
    showcaseStayAuto: string;
    showcaseStayAutoHint: string;
    /** 全部阵营跑完一趟的总时长（`{v}` 替换） */
    showcaseTotalDuration: string;
    /** 编辑器里"自动项"标签（如巡礼兵种带出的巢穴建筑） */
    showcaseAutoTag: string;
    /** 每阵营覆盖项的说明 */
    showcaseDeckFocusStepHint: string;
    showcaseDeckTransitionHint: string;
    showcaseFollowRealm: string;
    showcaseHighlightHeroesHint: string;
    showcaseHighlightBuildingsHint: string;
    showcaseDeckDurationHint: string;
    showcaseDeckHeroCountHint: string;
    showcaseMusicUseBgmHint: string;
    showcaseResetDeck: string;
    showcasePreview: string;
    showcaseDeckModeAuto: string;
    showcaseDeckModeManual: string;
    showcaseToManual: string;
    showcaseToManualHint: string;
    showcaseManualNote: string;
    showcaseSearchFaction: string;
    showcaseFocusWithBuildings: string;
    showcaseCreatureUpgraded: string;
    showcaseHeroClassBalanced: string;
    showcaseHeroClassBalancedHint: string;
    showcaseHeroShuffle: string;
    showcaseHeroShuffleHint: string;
    showcaseHeroRotation: string;
    showcaseMusicVolume: string;
    showcaseMusicUseBgm: string;
    showcaseMusicFaction: string;
    showcaseMusicFactionHint: string;
    /** 无专属音乐时改用领域背景音乐（默认关 = 静音）。⚠️ 与列表行的「回退BGM」角标是两回事，故另起名 */
    showcaseMusicFallbackRealm: string;
    showcaseMusicFallbackRealmHint: string;
    /** 播放页：建筑列表里"招募建筑"的提示（它们已在兵种那一步展示过） */
    showcaseDwellingCovered: string;
    /** 焦点走完一轮是否重来（**只在关闭「轮完一轮才切」时**出现） */
    showcaseFocusLoop: string;
    showcaseFocusLoopHint: string;
    /** 自动编排：每次进入巡礼都换一批（默认关闭 = 顺序固定） */
    showcaseRerollEachEntry: string;
    showcaseRerollEachEntryHint: string;
    /** 播放页「退出巡礼」按钮的 tooltip（说明去向与 ESC） */
    showcaseSkipHint: string;
    showcaseMusicPreFade: string;
    showcaseMusicPreFadeHint: string;
    showcaseAudition: string;
    /** 试听按钮激活态（「■ 停止」的文案部分；■ 为符号保留） */
    showcaseAuditionStop: string;
    showcaseAuditionHint: string;
    showcaseEditLang: string;
    showcaseEditLangHint: string;
    showcaseLoadingPreset: string;
    // —— 领域素材准备页（加急构建，S1） ——
    /** 页面标题 / 准备中 */
    assetsPreparing: string;
    /** 尚无进度数据时的说明 */
    assetsBuilding: string;
    /** 准备页说明（只采集本领域所需素材 / 完成后自动进入 / 可先返回） */
    assetsHint: string;
    /** 「返回领域主界面」按钮 */
    assetsBack: string;
    /** 构建完成但仍有缺失 mod 时的提示 */
    assetsFailed: string;
    // —— 领域信息页 v2（启用集合 + 锚点导航） ——
    infoConfigGlobal: string;
    infoSearch: string;
    infoShowCore: string;
    infoShowCoreNote: string;
    infoCoreLoading: string;
    infoCoreSection: string;
    infoEmpty: string;
    infoHiddenZero: string;
    infoExpandAll: string;
    infoCollapseAll: string;
    infoModDetail: string;
    infoNameZh: string;
    infoNameEn: string;

    infoMods: string;
    infoModsValue: string;
    infoBlockMaps: string;
    infoCreated: string;
    infoNo: string;
    backToRealm: string;
    adjust: string;
    more: string;
    searchRealms: string;
    noMatch: string;
    clearSearch: string;
    deleteRealm: string;
    deleteActiveForbidden: string;
    deleteOk: string;
    // 远程领域仓库（在线领域列表）
    remoteRealm: string;
    exportIndexOk: string;
    remoteTitle: string;
    remoteBack: string;
    searchRemote: string;
    groupUpdatable: string;
    groupAvailable: string;
    groupInstalled: string;
    groupInstalledNote: string;
    remoteEmpty: string;
    remoteOffline: string;
    remoteFetchFailed: string;
    remoteRepoCount: string;
    remoteReposEmpty: string;
    remoteRepoMore: string;
    remoteReposTitle: string;
    remoteReposHint: string;
    remoteRepoTest: string;
    remoteRepoRemove: string;
    remoteRepoAdd: string;
    remoteRepoTestOk: string;
    remoteRepoTestFail: string;
    remoteReposNone: string;
    remoteTestPlaceholder: string;
    download: string;
    cancelDownload: string;
    updateRealm: string;
    updateConfirmTitle: string;
    updateConfirmDesc: string;
    /** 同版本换包（发布事故兜底）时的确认文案 */
    updateConfirmDescContent: string;
    updateConfirmWarn: string;
    conflictTitle: string;
    conflictDesc: string;
    overwriteBtn: string;
    renameBtn: string;
    installOk: string;
    installNeedsMods: string;
    needExternalMods: string;
    depsReady: string;
    updatableTag: string;
    /** 同版本换包（发布事故兜底）时的角标文案 */
    updatableTagContent: string;
    tagsAll: string;
    refresh: string;
    infoDesc: string;
    launchIssueTitle: string;
    launchConflictDesc: string;
    launchMissingBundledDesc: string;
    launchMissingBorrowedDesc: string;
    launchAnyway: string;
    viewModCombo: string;
    backToOfficialInstall: string;
    launchFailedTitle: string;
    /** 启动失败的具体原因兜底（RealmShell 阶段 4 模态框；后端未给消息时） */
    launchFailedMsg: string;
    /** 领域设置目录定位失败 */
    settingsDirMissing: string;
    /** 进入领域失败（RealmSwitchPage；后端未给消息时） */
    enterFailed: string;
    /** 创建领域失败（CreateRealmDialog；后端返回空时） */
    createFailed: string;
    retry: string;
    settingsRealmBanner: string;
    bottomActiveRealm: string;
    tabBasic: string;
    tabIntro: string;
    tabMods2: string;
    tabAdvanced: string;
    tabMaps: string;
    // 集成地图标签页（统一页面，v5）
    mapsImport: string;
    mapsImportTip: string;
    mapsSearch: string;
    mapsSearchEmpty: string;
    mapsFilterLang: string;
    mapsFilterTarget: string;
    mapsFilterHint: string;
    mapsTreeAll: string;
    mapsTreeMaps: string;
    mapsTreeCampaigns: string;
    mapsLangUnknown: string;
    mapsEmpty: string;
    mapsMoveTo: string;
    mapsMoveTitle: string;
    mapsDeleteConfirmTitle: string;
    mapsDeleteConfirmDesc: string;
    mapsDirLabel: string;
    mapsBrowse: string;
    mapsScan: string;
    mapsFound: string;
    mapsKindDist: string;
    mapsLangDist: string;
    /** 「全部」节点汇总：总大小 */
    mapsTotalSize: string;
    /** 「全部」节点汇总：提示去左侧分区浏览 */
    mapsSummaryHint: string;
    mapsConflicts: string;
    mapsConflictSkip: string;
    mapsConflictOverwrite: string;
    mapsConflictRename: string;
    mapsKeepStructure: string;
    mapsImportConfirm: string;
    mapsImportDone: string;
    mapsVcmiRunning: string;
    mapsDetailVersion: string;
    mapsDetailLang: string;
    mapsDetailDesc: string;
    mapsDetailVictory: string;
    mapsDetailLoss: string;
    mapsDetailDifficulty: string;
    mapsDetailPlayers: string;
    mapsDetailScenarios: string;
    mapsDetailDate: string;
    mapsDiff0: string;
    mapsDiff1: string;
    mapsDiff2: string;
    mapsDiff3: string;
    mapsDiff4: string;
    mapVicStandard: string;
    mapVicArtifact: string;
    mapVicGatherTroop: string;
    mapVicGatherResource: string;
    mapVicBuildCity: string;
    mapVicBuildGrail: string;
    mapVicBeatHero: string;
    mapVicCaptureCity: string;
    mapVicBeatMonster: string;
    mapVicTakeDwellings: string;
    mapVicTakeMines: string;
    mapVicTransportItem: string;
    mapVicEliminateMonsters: string;
    mapVicSurviveDays: string;
    mapLossStandard: string;
    mapLossCastle: string;
    mapLossHero: string;
    mapLossTime: string;
    // 领域模组 tab（§9.6.1 核对点 2）
    modPreset: string;
    presetName: string;
    hintMods: string;
    srcGame: string;
    srcUser: string;
    modDetailEmpty: string;
    modGameFixed: string;
    presetLabel: string;
    /**
     * 领域模组管理视图（realm-mods-management §5.2：四标签页，P0-P2 全量）。
     * 通用列表词汇（search/col/location/q 系列）复用 `modsManage`；
     * 本命名空间自含管理动作与确认框差异键。占位符：{n}/{name}/{list}/{dir}/{loc}。
     */
    modsManage: {
      title: string;
      /** 返回预设视图按钮（← 前缀由组件渲染） */
      back: string;
      /** 数据隔离警示横幅正文（!isolated 时显示） */
      banner: string;
      tabInstalled: string;
      tabReferable: string;
      tabDownloadable: string;
      tabUpdatable: string;
      actImport: string;
      actRefFromPreset: string;
      actRefAll: string;
      /** {n} */
      actRefAllDone: string;
      actRefAllNone: string;
      refBusy: string;
      importZip: string;
      importDir: string;
      /** {n} */
      impDone: string;
      impFailTitle: string;
      impReasonNotMod: string;
      impConflictTitle: string;
      impConflictBody: string;
      impConflictReplace: string;
      impConflictSkip: string;
      /** {n} */
      impSkipped: string;
      /** 缺失徽章：{n} */
      missingBtn: string;
      missingHint: string;
      missingGo: string;
      /** 可下载页行内缺失标记 */
      missingTag: string;
      colVersion: string;
      colCurVersion: string;
      colLatest: string;
      colSource: string;
      actRef: string;
      actUnref: string;
      actBundle: string;
      actDelete: string;
      unrefTitle: string;
      unrefBody: string;
      /** {list} */
      unrefDeps: string;
      unrefCascade: string;
      /** {n} */
      unrefDone: string;
      /** {list} */
      refMissing: string;
      delTitle: string;
      /** {name} {dir} */
      delBody: string;
      /** {list} */
      delDeps: string;
      /** {name} */
      delDone: string;
      /** {name} {msg} */
      delFail: string;
      /** {name} */
      bundleDone: string;
      /** {name} */
      bundleSame: string;
      bundleGameTitle: string;
      /** {name} */
      bundleGameWarn: string;
      bundleDivTitle: string;
      /** {name} */
      bundleDivWarn: string;
      bundleForce: string;
      updTitle: string;
      /** {loc} */
      updConvert: string;
      /** {name} */
      updDone: string;
      /** 下载体积约 {size} */
      updSize: string;
      /** 更新包提供完整性校验 */
      updCheckOk: string;
      /** 该来源未提供校验信息 */
      updCheckWeak: string;
      /** 更新内容（{v}）：{text} */
      updLog: string;
      /** 更新 {name} 失败：{msg} */
      updFail: string;
      /** 缺省说明：尚未检查仓库（标题） */
      repoEmptyTitle: string;
      /** 缺省说明：尚未检查仓库（正文） */
      repoEmptyHint: string;
      /** 安装/更新前进程守卫（{procs}） */
      needExitGame: string;
      prefTitle: string;
      prefHint: string;
      prefEmpty: string;
      /** {n} */
      prefCount: string;
      prefAdoptTitle: string;
      prefAdoptMerge: string;
      prefAdoptCreate: string;
      prefAdoptSuffix: string;
      prefRun: string;
      /** {n} */
      prefDone: string;
      /** {n} */
      prefInRealm: string;
      /** {n} */
      prefMissing: string;
      prefMissingHint: string;
      /** {name} */
      prefCreated: string;
      /** 预设视图入口按钮（→ 由组件渲染） */
      actManage: string;
      /** ⓘ hover：数据源标题 */
      scopeTitle: string;
      /** ⓘ hover：{pos} {user} {game} */
      scopeLine: string;
      scopeHint: string;
      emptyTitle: string;
      emptyStep1: string;
      emptyStep2: string;
      emptyGo: string;
      emptyRefT: string;
      emptyRefD: string;
      emptyBundleT: string;
      emptyBundleD: string;
      emptyLocalT: string;
      emptyLocalD: string;
      /* ---------------- 依赖处置（deps）：预设页徽章 / 告知弹层 / 已实装行 / 处理弹窗 ---------------- */
      /** 预设页操作栏按钮：依赖处置 */
      depBtn: string;
      /** 告知弹层标题 */
      depTitle: string;
      /** 告知弹层正文：{n} = 缺陷模组数 */
      depDesc: string;
      /** 危害标记：影响已启用组合 */
      depSevActive: string;
      /** 危害标记：潜在影响 */
      depSevLatent: string;
      /** 清单计数：{n} 项 */
      depCount: string;
      /** 告知弹层底部按钮：去处理（→ 由组件渲染） */
      depGo: string;
      /** 已实装表筛选 chip：依赖缺陷 */
      depFilter: string;
      /** 已实装行操作按钮（N = 缺失依赖项数由组件渲染） */
      depRowBtn: string;
      /** 处理弹窗摘要：{n} 项缺失依赖、{fixable} 项可快速处理 */
      depSum: string;
      /** 处理弹窗影响列 hover：影响 {n} 个模组：{list} */
      depAffectedTip: string;
      /** 处理动作：引入（全局已装） */
      depRef: string;
      /** 处理动作：安装（仓库可得） */
      depInstall: string;
      /** 不可得标记（引入 / 安装都没有） */
      depUnavailable: string;
      /** 批量按钮：全部引入（{n}） */
      depAllRef: string;
      /** 批量按钮：全部安装（{n}） */
      depAllInstall: string;
      /** 行级状态：处理中… */
      depBusy: string;
      /** 行级状态：已处理 */
      depDone: string;
      /** 行级状态：失败 */
      depFailTag: string;
      /** toast：已引入 {name} 并补入受影响预设激活集 */
      depRefDone: string;
      /** toast：已安装 {name} 并补入受影响预设激活集 */
      depInstallDone: string;
      /** toast：部分依赖处理失败 */
      depPartFail: string;
      /** 处理弹窗脚注（实装 + 补入激活集语义说明） */
      depNote: string;
      /** P4 轻告知：导入 / 下载完成后检出依赖缺陷（{name} {n}） */
      depWatchNote: string;
      /* ---------------- 依赖处置 · 第四轮（闭包解析 / 语言分组 / 忽略） ---------------- */
      /** 忽略依赖（写入 `realm.json mods.depIgnores`，不再计入缺陷） */
      depIgnore: string;
      /** toast：已忽略 {n} 项依赖（随保存落盘） */
      depIgnoreDone: string;
      /** 处理弹窗语言分组：其他语言包（{n}） */
      depOtherLang: string;
      /** 处理弹窗语言分组提示：领域语言（{langs}）优先 */
      depOtherLangHint: string;
      /** 入口按钮：一键解析全部依赖 */
      depClosureBtn: string;
      /** 闭包弹窗标题 */
      depClosureTitle: string;
      /** 闭包弹窗摘要（{refs} 引用数 / {items} 子模组 / {resolves} 解决 / {installs} 安装 / {deads} 无解） */
      depClosureSum: string;
      /** 引用组标题（{n}） */
      depClosureRefGroup: string;
      /** 安装组标题（{n}） */
      depClosureInstallGroup: string;
      /** 无解组标题（{n}） */
      depClosureDeadGroup: string;
      /** 引用行：解决 {n} 项 */
      depClosureResolves: string;
      /** 引用行：{n} 个子模组 */
      depClosureItems: string;
      /** 执行按钮：一键执行 */
      depClosureRun: string;
      /** 执行按钮副标：引用 {refs} · 安装 {installs} */
      depClosureRunTip: string;
      /** 空态：没有可自动处理的依赖 */
      depClosureEmpty: string;
      /** 无解组提示文案 */
      depClosureDeadHint: string;
      /** 语言勾选说明：默认勾选与领域语言匹配的翻译（{langs}） */
      depClosureLangNote: string;
      /** 执行中… */
      depClosureBusy: string;
      /** toast：闭包解析完成（{refs} {installs}） */
      depClosureDone: string;
    };
    // 高级选项：地图区块（阀门）
    integratedMaps: string;
    // 多语言：单字段 + 语言切换器（§9.6.1 核对点 1）
    editingLang: string;
    editingNow: string;
    editingNowShort: string;
    // §9.6.2 字段玩家文案
    namePh: string;
    descPh: string;
    /** 标签（§6 `tags`，逐语言录入，编辑器基本信息页） */
    tags: string;
    tagsPh: string;
    tagsHint: string;
    tagsFull: string;
    tagsDup: string;
    tagsLong: string;
    tagsRemove: string;
    introTextPh: string;
    introSecondsHint: string;
    introCoverHint: string;
    introThumbHint: string;
    introVideoHint: string;
    modDetailTitle: string;
  };
}
