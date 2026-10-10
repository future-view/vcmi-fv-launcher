/**
 * 领域所需素材的**加急构建**（S1）。
 *
 * ## 背景（开发者确认点）
 *
 * - 首次启动 / 便携运行换机器时，缓存可能尚未构建，而「领域信息」「阵营巡礼」都依赖素材；
 * - **不接受文字降级**（体验差）→ 缺素材时应当"先建再进"；
 * - 也不能让用户干等全局构建（652 个 mod 的阶段二可能数分钟）→ 只建**本领域所需**的那些。
 *
 * ## 流程（[`ensureRealmAssets`]）
 *
 * ```
 * 1. 算出领域所需 mod（core.* ∪ 启用集合 ∪ bundled/borrowed）
 * 2. checkRealmAssets 抽样判定是否已就绪 → 就绪直接返回
 * 3. 不就绪：暂停全局后台构建 → 跑领域加急批次（独立进度通道）→ 恢复全局构建
 * ```
 *
 * 纯 TS 逻辑层（**禁止 import react**）。
 */

// ⚠️ 本模块**不再引入高清相关 API**：领域加急刻意只生成标清素材（见 `ensureRealmAssets`），
// 高清由全局阶段三在后台补。
import { collectEnabledIds } from "@/lib/realm/enabledMods";
import type { Realm } from "@/lib/realm/types";
import {
  buildRealmCacheUrgent,
  checkRealmAssets,
  fetchRealmBuildProgress,
  readLauncherConfig,
  readModSettingsJson,
  realmConfigDir,
  scanRealMods,
} from "@/lib/tauri";
import { buildCoreEntries, flattenTree, toBatchPayload, type BatchPayload } from "./buildTargets";
import { ensureWikiCacheBuilt, pauseWikiBuild } from "./status";

/** 领域所需素材的 modId 集合（规范化后的比较键 → 用于去重） */
function realmAssetIdSet(realm: Realm, enabledIds: string[]): Set<string> {
  const ids = new Set<string>();
  // 原版 9 族 + core：与全局构建同源（见 `buildTargets`），加急队列同样需要它们
  for (const f of ["castle", "rampart", "tower", "inferno", "necropolis", "dungeon", "stronghold", "fortress", "conflux"]) {
    ids.add(`core.${f}`);
  }
  ids.add("core");
  for (const id of enabledIds) ids.add(id);
  // 领域携带清单：bundled（随包复制）与 borrowed（目录联接借用）
  for (const id of realm.mods?.bundled ?? []) ids.add(id);
  for (const id of realm.mods?.borrowed ?? []) ids.add(id);
  return ids;
}

/** 取领域启用组合（`modSettings.json` 的 `__realm__` 预设）；读不到返回空数组 */
async function realmEnabledIds(realm: Realm, realmId: string): Promise<string[]> {
  const configDir = await realmConfigDir(realmId);
  const settings = await readModSettingsJson(configDir ?? undefined);
  if (!settings) return [];
  const isGlobal = realm.dirs?.configPath === "global";
  return collectEnabledIds(settings, realm, isGlobal).ids;
}

/** 原版伪模组 id（`core` / `core.<faction>`）：与后端 `realms::transfer::is_core_mod_id` 同口径。 */
function isCoreModId(id: string): boolean {
  const l = id.toLowerCase();
  return l === "core" || l.startsWith("core.");
}

/**
 * 领域所需素材的**构建目标**（含 `modPath`，可直接交给加急队列）。
 *
 * - `core.*` 由 [`buildCoreEntries`] 产出（其 `modPath` 指向 `<gameDir>/Mods/vcmi`）；
 * - 其余 mod 从 `scan_mods` 结果里按**小写 id** 匹配（modSettings 写规范小写，
 *   而缓存目录/扫描结果沿用磁盘原名，如 `HuaXia`），匹配到的用**磁盘原名**作为 modId
 *   —— 缓存目录名就是它，传小写会落到另一个目录。
 *
 * @param opts.excludeCore 排除原版伪模组（**仅导出打包路径用**）：原版素材由接收方
 *   本机游戏数据生成，随包分发会白增约 58 MB，且缓存键含 `sourceHash` / `h3DataKey` /
 *   `h3CoreHash` / `schemaHash`，跨机器必然失配。领域激活/加急构建**不能**排除，
 *   否则原版阵营的巡礼没图。
 */
export async function realmAssetTargets(
  realm: Realm,
  realmId: string,
  opts?: { excludeCore?: boolean },
): Promise<BatchPayload[]> {
  const cfg = await readLauncherConfig();
  const gameDir = cfg?.gameDir ?? "";
  const ids = realmAssetIdSet(realm, await realmEnabledIds(realm, realmId));

  const core = buildCoreEntries(gameDir).filter((e) => ids.has(e.modId.toLowerCase()));
  const out: BatchPayload[] = [...core];

  const scanned = toBatchPayload(flattenTree((await scanRealMods()) as Record<string, unknown>[]));
  const byId = new Map(scanned.map((m) => [m.modId.toLowerCase(), m]));
  for (const id of ids) {
    if (isCoreModId(id)) continue; // 已由 coreEntries 处理（或按 excludeCore 整类排除）
    const hit = byId.get(id.toLowerCase());
    if (hit) out.push(hit);
  }
  return opts?.excludeCore ? out.filter((e) => !isCoreModId(e.modId)) : out;
}

export interface RealmAssetsResult {
  /** 全部所需素材均可用 */
  ok: boolean;
  /** 抽样判定为缺失的 modId（构建后仍失败的那些） */
  missing: string[];
  /** 是否因为用户「返回」而中断 */
  cancelled: boolean;
}

/**
 * 确保领域所需素材就绪；不就绪则跑**加急队列**。
 *
 * @param onProgress 进度回调（事件 + 400ms 轮询兜底，双通道保证进度条不会停）
 *
 * 注意：本函数**不做 UI**，由调用方（RealmShell 的进度页）负责展示与「返回」按钮；
 * 「返回」只需调 `cancelRealmCacheBuild()`，本函数会在批次结束时返回。
 */
export async function ensureRealmAssets(
  realm: Realm,
  realmId: string,
  onProgress?: (done: number, total: number) => void,
): Promise<RealmAssetsResult> {
  const targets = await realmAssetTargets(realm, realmId);
  if (targets.length === 0) return { ok: true, missing: [], cancelled: false };

  // 就绪检查：只抽样"一定会显示"的图，快（见后端 `check_realm_assets`）
  const chk = await checkRealmAssets(targets.map((t) => t.modId));
  if (chk.ready) return { ok: true, missing: [], cancelled: false };

  // 暂停全局后台构建：两个批次同时跑会争抢 CPU，且用户此刻关心的是本领域进度
  await pauseWikiBuild();

  // ⚠️ **加急批次刻意不生成高清**（不传 `hd` 参数）：
  // 阵营巡礼只需**标清**素材即可播放，而高清生成耗时是标清的数倍 ——
  // 让用户为"增强项"卡在进度条上是最差的体验。高清由全局阶段三在后台补，
  // 生成完成后由 `clearHdMissing()` + 重载自动切到高清（渐进增强）。

  // 事件可能丢失 → 400ms 轮询兜底（与全局构建同一套策略）
  const timer = window.setInterval(() => {
    void fetchRealmBuildProgress().then((p) => {
      if (p && p.total > 0) onProgress?.(p.done, p.total);
    });
  }, 400);

  let res: Awaited<ReturnType<typeof buildRealmCacheUrgent>> = null;
  try {
    res = await buildRealmCacheUrgent(
      (await readLauncherConfig())?.gameDir ?? "",
      targets,
      onProgress,
    );
  } finally {
    window.clearInterval(timer);
    // 恢复全局构建（增量续跑）。⚠️ 全局 busy 时该调用会被忽略 —— 领域素材已就绪，
    // 剩余部分下次启动继续补，不影响用户当前使用。
    void ensureWikiCacheBuilt().catch(() => {});
  }

  if (!res) return { ok: false, missing: chk.missing, cancelled: false };
  // 二次校验：构建"成功"不等于素材一定齐（个别 mod 可能本身缺图）
  const after = await checkRealmAssets(targets.map((t) => t.modId));
  return { ok: after.ready, missing: after.missing, cancelled: res.failed.length > 0 };
}
