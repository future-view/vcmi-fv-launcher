/**
 * 模组仓库（获取与更新）的前端纯逻辑层：版本比较 / 更新判定 / 条目匹配。
 *
 * 与后端权威实现 `src-tauri/src/infra/version.rs` 的 `compare`（模组与领域**共用**）
 * **同一口径**：去 v 前缀 → 按 `.` 分段 → 每段取**前导数字**（"2b"→2、无数字→0）
 * → 逐段比较、缺段补 0；后缀不参与（`1.0.0-rc1` = `1.0.0`，宁缺勿错）。
 * 仅更新版本更高者才算可更新，设计见 `docs/features/mods-repository.zh-CN.md`。
 * IPC 封装在 `lib/tauri.ts`。
 */
import type { ModInfo } from "./modScanner";
import type { ModRepoEntry } from "./tauri";

/** 点分版本逐段比较（对齐后端 `infra::version::compare`，返回 -1/0/1）。 */
export function compareModVersions(a: string, b: string): number {
  const seg = (s: string) =>
    (s ?? "")
      .trim()
      .replace(/^[vV]+/, "")
      .split(".")
      .map((p) => {
        const m = p.trim().match(/^\d+/);
        return m ? Number(m[0]) : 0;
      });
  const x = seg(a);
  const y = seg(b);
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    const l = x[i] ?? 0;
    const r = y[i] ?? 0;
    if (l !== r) return l < r ? -1 : 1;
  }
  return 0;
}

/** 更新判定：仅新版本更高才算可更新（本地领先仓库时不提示降级）。 */
export function isModUpdatable(installedVersion: string | undefined, latest: string): boolean {
  const cur = (installedVersion ?? "").trim();
  const next = (latest ?? "").trim();
  if (!cur || !next) return false;
  if (cur === next) return false;
  return compareModVersions(cur, next) < 0;
}

/** 小写索引（id 匹配大小写不敏感，对齐 VCMI 目录语义）。 */
export function modRepoIndex(entries: ModRepoEntry[]): Map<string, ModRepoEntry> {
  const m = new Map<string, ModRepoEntry>();
  for (const e of entries) {
    const key = (e.id ?? "").toLowerCase();
    if (key) m.set(key, e);
  }
  return m;
}

/**
 * 可更新集合：`id → { entry, installedVersion }`。
 * 子模组（id 含点）不参与——它们随父模组整包更新。
 */
export function modUpdateMap(
  mods: Pick<ModInfo, "id" | "version" | "isSubmod">[],
  repo: Map<string, ModRepoEntry>,
): Record<string, { entry: ModRepoEntry; installedVersion: string }> {
  const out: Record<string, { entry: ModRepoEntry; installedVersion: string }> = {};
  for (const m of mods) {
    if (m.isSubmod) continue;
    const e = repo.get(m.id.toLowerCase());
    if (!e || !isModUpdatable(m.version, e.version)) continue;
    out[m.id] = { entry: e, installedVersion: m.version ?? "" };
  }
  return out;
}

/**
 * 仓库版本信息（不要求已安装）：`id → entry`，仅保留**可解析**的远端版本。
 * FV 聚合索引缺 version 时无法判定「已装/有更新」，条目只用于展示来源。
 */
export function repoVersionOf(entry: ModRepoEntry | undefined): string {
  if (!entry) return "";
  // 预聚合元数据优先；缺失时不猜（官方索引需懒取 mod.json，M3 的 build-mod-index 补全）
  return (entry.version ?? "").trim();
}

/** 下载体积展示：官方条目是 MB 数字串（如 "4.062"）；无值返回 null。 */
export function formatDownloadSize(raw: unknown): string | null {
  if (raw === null || raw === undefined || raw === "") return null;
  const n = typeof raw === "number" ? raw : Number(String(raw).trim());
  if (!Number.isFinite(n) || n <= 0) return null;
  if (n >= 1024) return `${(n / 1024).toFixed(1)} GB`;
  return `${n.toFixed(1)} MB`;
}

/** changelog 最新版本条目（版本倒序取前 N 行，供弹窗/抽屉摘要）。 */
export function changelogNewest(
  changelog: Record<string, string[]> | null | undefined,
  maxVersions = 3,
): { version: string; lines: string[] }[] {
  if (!changelog) return [];
  const versions = Object.keys(changelog).sort((a, b) => compareModVersions(b, a));
  return versions.slice(0, maxVersions).map((v) => ({
    version: v,
    lines: Array.isArray(changelog[v]) ? changelog[v].map(String) : [],
  }));
}
