/**
 * 远程领域仓库（索引、下载、封面）。
 *
 * 拆分自原 `@/lib/tauri.ts` 单体；对外仍由该 barrel 统一导出，调用方无需改动。
 */
import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { feErr } from "@/lib/backendError";
import { isTauri } from "./core";
import { ImportRealmResult } from "./realm-maps";

/* ----------------------------- 远程领域仓库 ----------------------------- */

/** 远程领域索引条目（后端已补齐 `id` / `tags` / `state` / `installedVersion`）。 */
export interface RemoteRealm {
  id: string;
  /** 领域版本 */
  version?: string;
  author?: string;
  /** 领域名称（默认语言字符串，即 `defaultLang` 对应语言）。其他语言见 `i18n.<lang>.name` */
  name?: string;
  /** 领域描述（默认语言字符串）。其他语言见 `i18n.<lang>.description` */
  description?: string;
  /** 领域标签（默认语言）。其他语言见 `i18n.<lang>.tags` */
  tags?: string[];
  /** 顶层 `name` / `description` / `tags` 所属语言（VCMI 语言标识），缺省 chinese */
  defaultLang?: string;
  /** 逐语言覆盖块：语言 id -> 覆盖 `name` / `description` / `tags`。
   *  匹配界面语言时整体覆盖对应字段；无匹配按回退链兜底到顶层值。 */
  i18n?: Record<string, { name?: string; description?: string; tags?: string[] }>;
  /** 封面图地址（由**后端**下载后本地缓存，前端不直连，规避图床防盗链） */
  cover?: string;
  download?: string;
  sha256?: string;
  /** 包体积（字节） */
  size?: number;
  /** 依赖的外部模组（不随领域分发，需玩家本机已安装） */
  requiresMods?: string[];
  /** 该 realm 所属仓库源地址（聚合多源时由后端打上，用于校验/下载定位） */
  repositoryUrl?: string;
  gameEdition?: string;
  /** `available` 未安装 / `installed` 已安装且最新 / `updatable` 有更新 */
  state: "available" | "installed" | "updatable";
  installedVersion?: string | null;
}

export interface RealmRepositoryIndex {
  realms: RemoteRealm[];
  /** 各仓库源的拉取状态（成功/失败、领域数量、缓存时间、错误原因） */
  sources?: RealmRepositorySource[];
  /** 聚合层面的错误：仅当所有源都失败且无任何缓存时非空 */
  error?: string | null;
}

export interface RealmRepositorySource {
  url: string;
  /** 本次拉取是否成功（成功或有可用缓存均为 true） */
  ok: boolean;
  error?: string | null;
  fetchedAt?: number | null;
  realmCount: number;
}

/** 当前仓库源列表（不触发网络请求）。 */
export async function getRealmRepositories(): Promise<{ urls: string[] }> {
  if (!isTauri()) return { urls: [] };
  return await invoke<{ urls: string[] }>("get_realm_repositories");
}

/** 新增一个仓库源地址（去重、校验 https + .json）。 */
export async function addRealmRepository(url: string): Promise<{ urls: string[] }> {
  if (!isTauri()) throw new Error(feErr("frontend_desktop_only", "配置领域仓库源"));
  return await invoke<{ urls: string[] }>("add_realm_repository", { url });
}

/** 移除一个仓库源地址。 */
export async function removeRealmRepository(url: string): Promise<{ urls: string[] }> {
  if (!isTauri()) throw new Error(feErr("frontend_desktop_only", "配置领域仓库源"));
  return await invoke<{ urls: string[] }>("remove_realm_repository", { url });
}

/** 测试索引地址是否可用（不写入配置）。 */
export async function testRealmRepository(
  url: string,
): Promise<{ ok: boolean; url: string; realmCount: number; elapsedMs: number }> {
  if (!isTauri()) throw new Error(feErr("frontend_desktop_only", "测试领域仓库源"));
  return await invoke("test_realm_repository", { url });
}

/** 拉取远程领域列表。`force` 为 true 时忽略缓存 TTL 强制刷新。 */
export async function fetchRealmIndex(force = false): Promise<RealmRepositoryIndex> {
  if (!isTauri()) {
    return { realms: [], sources: [], error: feErr("frontend_mock_unsupported", "远程领域") };
  }
  return await invoke<RealmRepositoryIndex>("fetch_realm_index", { force });
}

/** 下载并安装远程领域（后端**强制**校验 sha256 后才导入）。conflict 语义同本地导入。 */
export async function installRemoteRealm(
  realmId: string,
  conflict: "error" | "overwrite" | "rename" = "error",
): Promise<ImportRealmResult> {
  if (!isTauri()) throw new Error(feErr("frontend_desktop_only", "下载远程领域"));
  return await invoke<ImportRealmResult>("install_remote_realm", { realmId, conflict });
}

/** 取消正在进行的领域下载。 */
export async function cancelRealmDownload(realmId: string): Promise<void> {
  if (!isTauri()) return;
  await invoke("cancel_realm_download", { realmId });
}

/** 下载并缓存远程领域封面，返回 `领域 id → 本地图片绝对路径`。 */
export async function ensureRealmCovers(): Promise<Record<string, string>> {
  if (!isTauri()) return {};
  try {
    return await invoke<Record<string, string>>("ensure_realm_covers");
  } catch {
    return {};
  }
}

/** 封面本地绝对路径 → `<img>` 可用 URL（§5.4：本地资源必须经 convertFileSrc）。 */
export function coverUrl(absPath: string | undefined): string {
  return absPath ? convertFileSrc(absPath) : "";
}

export interface RealmDownloadProgress {
  realmId: string;
  received: number;
  total: number;
  done?: boolean;
}

/** 订阅领域下载进度，返回取消订阅函数。 */
export async function onRealmDownloadProgress(
  cb: (p: RealmDownloadProgress) => void,
): Promise<() => void> {
  if (!isTauri()) return () => {};
  const { listen } = await import("@tauri-apps/api/event");
  return await listen<RealmDownloadProgress>("realm-download-progress", (e) => cb(e.payload));
}

/** 取消下载的哨兵错误串（后端约定）：前端识别后不弹错误提示。 */
export const REALM_DOWNLOAD_CANCELLED = "REALM_DOWNLOAD_CANCELLED";
