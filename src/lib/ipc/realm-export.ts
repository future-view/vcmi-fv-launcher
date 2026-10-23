/**
 * 领域导出与发布（打包、进度、发布目标）。
 *
 * 拆分自原 `@/lib/tauri.ts` 单体；对外仍由该 barrel 统一导出，调用方无需改动。
 */
import { invoke } from "@tauri-apps/api/core";
import { feErr } from "@/lib/backendError";
import { isTauri } from "./core";

/** 已安装模组列表（供领域借用 / 整合选择，设计 §3.4）。返回 { mods: {id,name}[] } */
export interface InstalledMod {
  /** VCMI modId：磁盘目录名**小写**归一（与 VCMI 主键同形态） */
  id: string;
  /**
   * 磁盘目录原名（保留大小写、可能含空格）。
   *
   * ⚠️ 磁盘定位必须用它，**不要用 `id`** —— `id` 是小写归一后的 VCMI modId，
   * 在大小写敏感的文件系统（Linux/macOS）上 `join(id)` 找不到目录。
   * 缺省（旧后端）时只能以 `id` 兜底。
   */
  diskName?: string;
  /** 模组目录绝对路径（后端扫描回传；子模组的 `<父>/Mods/<子>` 布局靠它才准） */
  modPath?: string;
  name: string;
  /** 模组描述（mod.json 的 `description`，英文原文）。领域信息页等非 wiki 场景曾用此字段 */
  desc?: string;
  /** `mod.json` 多语言块（scan 透传，与领域已实装同口径）：名称/描述按界面语言选词，
   *  领域「可引用」页的行名称与抽屉简介依赖它做本地化 */
  i18n?: Record<string, { name?: string; description?: string } | null>;
  /** `mod.json` 的 `version` / `author` 原文（scan 透传；缓存未构建时的回退值） */
  version?: string | null;
  author?: string | null;
  /** 模组元数据（**缓存投影 cache.json 的 modMeta**）：多语言名/描述 + 作者/版本/类型/依赖。
   *  缓存未构建时缺失——此时只能回退 `name` / `desc`（英文原文）。 */
  meta?: {
    modId?: string;
    name?: Record<string, string>;
    description?: Record<string, string>;
    author?: string;
    version?: string;
    modType?: string;
    depends?: unknown[];
    conflicts?: unknown[];
    compatibility?: Record<string, unknown>;
  };
  /** 来源目录（§9.6.1）："user"=玩家安装（全局用户目录）；"game"=游戏自带（游戏目录 Mods，永远可见） */
  source?: "user" | "game";
  /** 模组类型（modType，由后端 classify_mod 识别），供领域模组 tab 复用 Mods 页面的类型筛选 */
  type?: string;
  /** `mod.json` 的 `modType` **原文**（未推断；用于翻译模组语言判定等需与引擎同口径的场景） */
  rawType?: string | null;
  /** 引擎兼容区间（`mod.json` 的 `compatibility`），透传给领域模组列表做兼容提示 */
  compatibility?: { min?: string | null; max?: string | null } | null;
  /** 翻译类模组的基准语言（`mod.json` 的 `language`；官方判定缺失时视为 english） */
  language?: string | null;
  /** 顶层父 mod id（子模组指向父；顶层为自身），供复用 Mods 页面的父子树布局 */
  parentId?: string;
  /** 是否为子模组 */
  isSubmod?: boolean;
  /** 子模组默认是否保持禁用（VCMI mod.json 的 keepDisabled：true=安装后不自动启用） */
  keepDisabled?: boolean;
  /** 城镇阵营列表（真城镇才有），供类型筛选/缩略图 */
  factions?: string[];
  /** 必需依赖（VCMI mod.json 的 `depends`）：禁用本模组前需校验仍有启用模组依赖它（§3.3 依赖守卫） */
  depends?: string[];
  /** 可选依赖（VCMI mod.json 的 `softDepends`）：缺失不影响加载，仅增强 */
  softDepends?: string[];
  /** 互斥模组（VCMI mod.json 的 `conflicts`）：同时启用即冲突 */
  conflicts?: string[];
}
export async function listInstalledMods(): Promise<InstalledMod[]> {
  if (!isTauri()) return [];
  try {
    const r = await invoke<{ mods: InstalledMod[] }>("list_installed_mods");
    return r.mods || [];
  } catch {
    return [];
  }
}

/** 导出领域快照（§7.1）：打包为 .vcmirealm（ZIP）。dest 为目标文件绝对路径。 */
export interface ExportRealmResult {
  ok: boolean;
  /** 用户中途取消（半成品已删除，`path` 仅供展示） */
  cancelled?: boolean;
  /** 导出的 `.vcmirealm` 路径 */
  path: string;
  /** 同步导出的领域索引片段路径（`<stem>.index.json`，可并入仓库 index.json 用于分发） */
  indexPath?: string;
  /** 直写本地仓库的结果（§7.3）；未开该模式为 null */
  repo?: RealmPublishResult | null;
  /** 发布自检告警（code 供 i18n 映射，arg 是已格式化的补充信息） */
  warnings?: RealmPublishWarning[];
  fileCount: number;
  /** 包内条目校验和（`realm.json.checksum`，导入侧校验用） */
  checksum: string;
  /** **整个包文件的 SHA-256**（仓库索引 `sha256` 字段用，与下载校验同源） */
  sha256?: string;
  /** 产出的 `.vcmirealm` 字节数 */
  size?: number;
  /** 打包前的源内容总字节数（进度分母） */
  bytesTotal?: number;
  /** 是否附带了模组巡礼素材缓存 */
  showcaseCache?: boolean;
  /** 分片片数（0 = 未分片）。仅发布分支可能产生分片（分享分支恒为 0） */
  partCount?: number;
}

/** 发布自检告警（与后端 `realms::transfer::warn` 同构）。 */
export interface RealmPublishWarning {
  /** `over-download-max` | `over-git-max` | `bundled` | `bad-id` | `no-url-base` | `version-unchanged` | `pkg-unchanged` | `copied-into-repo` */
  code: string;
  /** 已格式化的补充信息（体积 MB / 模组清单 / 落地路径等） */
  arg?: string;
}

/** 下载地址前缀的来源（与后端 `realms::transfer::URL_SOURCE_*` 同值）。 */
export type RealmUrlSource = "manual" | "declared" | "git" | "entries" | "none";

/** 直写本地领域仓库工作树的产物摘要（§7.3）。 */
export interface RealmPublishResult {
  /** 仓库根（含 index.json 的那级） */
  repoDir: string;
  /** 包在仓库内的落地路径 `realms/<id>/<file>` */
  pkgPath: string;
  /** 封面落地路径；领域未配封面时为 null */
  coverPath?: string | null;
  /** 被合并写回的 index.json */
  indexPath: string;
  /** 本次对仓库条目的动作：新增 / 字段有变化 / 完全重复（条目已存在且无字段变化） */
  entryState: "created" | "updated" | "unchanged";
  /** 值发生变化的条目字段名（供结果态摘要展示） */
  changedFields: string[];
  /** 本次 download 的前缀来源 */
  urlSource: RealmUrlSource;
  /** 本次是否把推导结果回写成了仓库的 `urlBase`（自举） */
  urlBootstrapped?: boolean;
  /** 分片片数（0 = 未分片，包未超过单片上限） */
  partCount?: number;
  /** 分片产物清单（仅分片时非空；路径由后端给出，前端不自行拼名） */
  parts?: RealmPartInfo[];
}

/** 分片产物（`docs/features/realms-sharded-package.zh-CN.md` §5）。 */
export interface RealmPartInfo {
  /** 分片在仓库工作树内的绝对路径 */
  path: string;
  size: number;
  sha256: string;
}

/**
 * 「发布到领域仓库」的落地预览（§7.3）。
 *
 * 弹窗在用户点「导出」**之前**就要把会动哪三个文件、下载地址是什么显示出来，
 * 让用户确认而不是猜——这是取消「下载 URL 前缀」手填框的前提。
 * 推导与实际发布共用后端的 `plan_publish`，两边说法必然一致。
 */
export interface RealmPublishPreview {
  repoDir: string;
  /** 仓库是否已有 index.json */
  indexExists: boolean;
  /** 已有但解析失败：发布会中止（绝不覆盖坏数据），弹窗需提前标红 */
  indexParseOk: boolean;
  /** 包的落地路径 `realms/<id>/<id>.vcmirealm` */
  pkgPath: string;
  /** 封面落地文件名；领域未配封面（或封面实体缺失）为 null */
  coverName?: string | null;
  /** 推导出的 URL 前缀（带末尾 `/`）；推不出为 null */
  urlBase?: string | null;
  urlSource: RealmUrlSource;
  /** 最终写进索引的 download 地址；无地址为 null */
  downloadUrl?: string | null;
  /** 仓库里是否已有该领域条目（预告 created 还是 updated） */
  entryExists: boolean;
  /** 仓库既有条目版本；无条目 / 手工条目缺 version 时为 null */
  prevVersion?: string | null;
  /** 本次将发布的版本（取 realm.json，与索引条目同口径） */
  nextVersion: string;
  /** 版本关系：none = 无基准；down = 降级（发布将被阻断）；up = 升版；same = 同版 */
  versionRel: "none" | "down" | "up" | "same";
  /**
   * 分片预估片数：0 = 预计不分包（包未超过单片上限）。
   * 由后端按待打包内容字节数估算（不读文件内容），**实际片数以打包后为准**。
   */
  estimatedPartCount?: number;
  /** 单片上限字节数（提示语用，避免前端写死 45 MiB） */
  partMaxBytes?: number;
}

/** 导出进度（事件 `realm-export-progress`）。 */
export interface RealmExportProgress {
  realmId: string;
  /** prepare = 算素材清单；collect = 扫盘；pack = 写 zip；index = 算索引；publish = 写仓库 */
  phase: "prepare" | "collect" | "pack" | "index" | "publish";
  done: number;
  total: number;
  bytesDone: number;
  bytesTotal: number;
  /** 当前正在打包的条目相对路径 */
  current?: string;
}

/**
 * 导出领域快照（§7.1）。
 *
 * `opts`（S2）：
 * - `includeShowcaseCache`（默认 false）：把「阵营巡礼」所需素材缓存打进包
 *   （`showcase-cache/<modId>/{cache.json,images/}`），接收方导入后**无需重新采集**；
 *   体积随之增大。**原版伪模组（core / core.<faction>）不随包分发**（由接收方本机生成）。
 * - `repoDir`：给出即走「发布到领域仓库」分支（§7.3）——包与封面落 `realms/<id>/`、
 *   直接合并仓库 `index.json`，此时 `includeIndex` 恒为真（发布必然写索引）。
 * - `includeIndex`：仅分享分支有意义——true 时另写 `<stem>.index.json` 片段，
 *   供把包传到自己服务器后手工并入远程仓库。
 * - `urlBase`：仓库 URL 前缀的**人工覆盖值**（只存本机，不写回仓库）；留空由后端按
 *   「仓库声明 > git remote+分支 > 既有条目」推导。
 *
 * 后端已移出主线程执行（重 IO），进度经 `onRealmExportProgress` 推送，可用
 * `cancelRealmExport` 中途取消。
 */
export async function exportRealm(
  id: string,
  dest: string,
  opts?: {
    includeShowcaseCache?: boolean;
    includeIndex?: boolean;
    /** 要打包的缓存 mod（`realmAssetTargets` 的产物，见 `lib/modcache/realmAssets.ts`） */
    cacheMods?: string[];
    /** 本地领域仓库工作树根：给出即走「直写仓库」发布流程 */
    repoDir?: string;
    /** download / cover 的 URL 前缀；留空由后端从既有条目推导 */
    urlBase?: string;
  },
): Promise<ExportRealmResult> {
  if (!isTauri()) throw new Error(feErr("frontend_desktop_only", "导出领域"));
  return await invoke<ExportRealmResult>("export_realm", {
    realmId: id,
    dest,
    includeShowcaseCache: opts?.includeShowcaseCache ?? false,
    includeIndex: opts?.includeIndex ?? true,
    cacheMods: opts?.cacheMods ?? null,
    repoDir: opts?.repoDir?.trim() ? opts.repoDir.trim() : null,
    urlBase: opts?.urlBase?.trim() ? opts.urlBase.trim() : null,
  });
}

/** 订阅领域导出进度（事件 `realm-export-progress`），返回取消订阅函数。 */
export async function onRealmExportProgress(
  cb: (p: RealmExportProgress) => void,
): Promise<() => void> {
  if (!isTauri()) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  return await listen<RealmExportProgress>("realm-export-progress", (e) => cb(e.payload));
}

/** 请求取消正在进行的领域导出（中止点在打包循环，随后 `exportRealm` 返回 `cancelled: true`）。 */
export async function cancelRealmExport(id: string): Promise<void> {
  if (!isTauri()) return;
  await invoke("cancel_realm_export", { realmId: id });
}

/**
 * 记住「导出领域」的上次仓库目录、前缀覆盖与分支选择（传空 = 清空）。
 * 走 `launcher.json` 而非 localStorage：便携包要求偏好随目录走。
 */
export async function setRealmPublishTarget(
  repoDir: string | null,
  urlBase: string | null,
  mode: "share" | "publish" | null,
): Promise<void> {
  if (!isTauri()) return;
  await invoke("set_realm_publish_target", { repoDir, urlBase, mode });
}

/**
 * 预览「发布到领域仓库」会写什么（只读本地小文件，无网络无写入）。
 * 目录为空返回 null；目录不存在 / 不是领域仓库时后端报错，由调用方就地显示。
 */
export async function previewRealmPublish(
  repoDir: string,
  realmId: string,
  urlBase?: string | null,
): Promise<RealmPublishPreview | null> {
  if (!isTauri()) return null;
  const dir = repoDir.trim();
  if (!dir) return null;
  return await invoke<RealmPublishPreview>("preview_realm_publish", {
    repoDir: dir,
    realmId,
    urlBase: urlBase?.trim() ? urlBase.trim() : null,
  });
}
