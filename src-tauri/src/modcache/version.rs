//! 模组缓存版本常量（**唯一定义点**）。
//!
//! - [`WIKI_CACHE_VERSION`]：缓存结构 / 提取逻辑变更时递增 → 旧缓存整体失效重建；
//! - [`ASSETS_VERSION`]：仅素材提取逻辑变更时递增 → 可使旧缓存只重跑素材阶段。
//!
//! 缓存键 = `v{WIKI_CACHE_VERSION}:{sourceHash}:{h3DataKey}:{h3CoreHash}:{schemaHash}`，
//! 命中判定为**字符串相等**（`build/mod.rs`）。版本号只作失效标记，**不参与数值比较**；
//! `h3CoreHash` / `schemaHash` 自 v77/v85 起并入键，数据源/schema 变化会自动触发重建，
//! 与版本号解耦，开发者不必每次都手动 bump 版本号。

/// 解析原版伪模组的 id 约定，返回 `(是否 core 系, 阵营作用域)`：
/// - `"core"`            → `(true, None)`：父模组，只承载**全局/无归属**数据（无城镇）；
/// - `"core.<faction>"`  → `(true, Some(faction))`：子模组，只承载该阵营的城镇/生物/英雄，
///   形态与第三方 town mod 完全一致；
/// - 其他                → `(false, None)`：普通第三方 mod。

/// 素材提取版本号：素材提取逻辑（编码策略/取帧规则/落盘位置等）变更时递增，
/// 使只需重跑阶段二补图，而不必重新解析配置。
///
/// 历史（v1→v5）已随 2026-09-16 重置废弃：期间工作包括原版城镇背景合成图、
/// 原版生物/英雄图片回退（TWCRPORT/PortraitsLarge/UN32/UN44 等）、头像回退改为引用全局产物、
/// 原版图片落盘从全局 `_shared/` 收编进各自 `core.<faction>/images/`。
///
/// - 1 → 2（2026-09-23）：合并两项同日改动 ——
///   ① **城镇幕布去重**：`preview` 不再独立合成落 `images/townscreen/`，改为复用 `build_towns`
///   产出的 `images/town/<fid>.webp`（跨文件并集口径更完整；实测 84 城镇冗余约 42 MB）；
///   ② **幕布接入官方 HD 素材**（"选项 A"）：底图命中 `hd-edition` 的 `.pak` 时，幕布基准图
///   改为"HD 底图 + 结构帧（HD 命中用 HD，否则 1x 整数放大）"的合成图（尺寸 = 原版 × 档位，
///   x2/x3），编码走有损 q85；未命中 HD 的城镇逐字节不变。
///   ③ **兵种背景接入 HD**（`creatureBackground`，`TPCAS*`）：命中即用 HD 图（有损 q85）。
///   递增后清空 `images/` 并重跑阶段二，使所有城镇幕布 / 兵种背景按新口径重建。
///
/// - 3 → 4（2026-09-24）：**HD 结构帧的绘制偏移对齐 VCMI**
///   （`HdImageLoader::getImage`：`setMargins((defMargins - spriteOffset) * scale)`）。
///   旧口径命中 HD 帧后返回偏移 `(0,0)`、直接刻在结构 `x/y` 上 —— 对 lm/tm 非零的帧是错的。
///   实例：城堡船坞 `TBCSDKNN`（DEF `tm=53`、pak `spriteOffsetY=2`）被上移 51px，
///   正好压在战神像（grail，`z` 更低）身上。修正后按 **DEF 的 lm/tm 减去 pak 帧的
///   `spriteOffset`** 定位。递增后清空 `images/` 重跑阶段二，让所有幕布按新偏移重建。
pub(crate) const ASSETS_VERSION: u32 = 4;

/// Wiki / 模组缓存版本号：缓存结构 / 提取逻辑变更时递增，使旧缓存整体失效重建。
///
/// - 2026-09-16 重置为 `1`（不再兼容 1→86 期间的旧缓存）；`modPath` 采用相对路径编码
///   （`gameMods:`/`userMods:`/`realm:<id>:`），base 取自 `launcher.json` 权威配置与 `.vcmi-fv-launcher` 根，
///   不写死绝对路径、不依赖 `VcmiPaths::detect()` 重探测游戏目录。
///
/// 逐版变更日志已废弃，v1 区间（约 v49→v86）主要覆盖的工作范围，按主题归类如下：
///
/// - **原版数据补全（core / core.<faction> 伪模组）**：9 原版城镇、生物、英雄、建筑花费、
///   英雄初始部队与四维（HOTRAITS/HCTRAITS/BUILDING.TXT）、地形、资源、法术派系、
///   h3-core 文案表（含城镇描述）；
/// - **多语言文本提取**：翻译祖先模组链、键大小写归一、配置内嵌文本、mod.json 语言块、
///   逐语言交错合并、技能/法术分级描述键序、builtin 法术/技能收敛到 core；
/// - **采集容错**：路径大小写/单复数/聚合文件、类型目录 `-`/`_` 后缀、城镇变体目录、
///   建筑图标按资源真实类型分派、失败可观测（`iconFailures` / `audit.json`）；
/// - **提取协议**：内置 VCMI schema 权威源（`resources/vcmi-schemas`）、协议表 + 常驻审计
///   （objects/resources/schools）、`mapObject` 小写键修复、regex 通配匹配修复；
/// - **命名重构**：`wiki` → `modcache`，产物 `cache.json`/`raw.json`，
///   投影新增 `modMeta`/`resources`/`objects`。
///
/// 细节见 git 历史与 `docs/plan/` 下「百科」相关复核报告。
///
/// - v2：`factions[].structures`（城镇结构幕布坐标 + 金色描边 `border` 图，供 hover 高亮
///   罩住整栋建筑，设计 §5.9）；描边图落在 `<mod>/images/townborder/<faction>-<structure>.webp`。
/// - 2026-09-21 → `3`：生物缓存新增 `dwellings` 的**消费方**要求 —— 巡礼页的「招募建筑」
///   现在按**名称**匹配（`showcase.rs` 把 `creatures[].dwellings` 作为 `dwellingNames` 下发），
///   而它来自"城镇配置 `building.creatures` 反查"。该字段若在旧缓存里缺失，
///   联动就只能退回"按 level 猜 id"（第三方模组常猜空 → 表现为没有招募建筑）。
///   递增版本确保**一次性重建**后该字段必然存在。
pub(crate) const WIKI_CACHE_VERSION: u32 = 1;
