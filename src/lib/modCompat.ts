/**
 * 引擎版本兼容性判定（纯逻辑，无 React / 无 Tauri）。
 *
 * **复刻 VCMI 引擎语义**，不要凭直觉改：
 * - 判定入口 `ModDescription::isCompatible`（`references/vcmi/lib/modding/ModDescription.cpp`）：
 *   `min` 与 `max` 各自与「引擎版本」做一次 `CModVersion::compatible`，两者都通过才算兼容。
 * - `CModVersion::compatible(other, checkMinor=true, checkPatch=true)`
 *   （`references/vcmi/lib/modding/CModVersion.cpp`）的规则：
 *   1. **major 必须相等**（这是最容易误判的一条：`compatibility.max = "1.7"` 的模组在 1.8 上
 *      判为**不兼容**，而不是"向下兼容"）；
 *   2. 只有**双方都写了**的段才比较（任一侧缺失该段 = `Any` = 跳过比较）。
 *      所以 `min: "1.7"`（只写到 minor）不比较 patch，`min: "1"` 只比较 major；
 *   3. 比较是"区间包含"：`min` 要求 引擎 ≥ min，`max` 要求 引擎 ≤ max。
 */

/** 解析后的版本号：`null` 段 = VCMI 的 `Any`（未声明 / 不可解析）。 */
export interface EngineVersion {
  major: number;
  minor: number | null;
  patch: number | null;
}

/** mod.json 的 `compatibility` 字段（磁盘原文，空串视为未声明）。 */
export interface ModCompatibility {
  min?: string | null;
  max?: string | null;
}

/**
 * 解析版本字符串。
 *
 * - 超过 3 段（如 PE FileVersion 的 `1.8.0.0`）**截取前 3 段**：VCMI 的 `fromString`
 *   对 >3 段直接返回空版本，但引擎版本来自 exe 文件版本，必然带第 4 段，故此处放宽。
 * - 任一段含非数字 → 该段起视为未声明（与 VCMI 的 `Any` 一致）。
 */
export function parseVcmiVersion(raw: string | null | undefined): EngineVersion | null {
  const s = (raw ?? "").trim();
  if (!s) return null;
  const segs = s.split(".");
  if (segs.length === 0) return null;
  const nums: (number | null)[] = [];
  for (let i = 0; i < Math.min(segs.length, 3); i++) {
    const seg = segs[i].trim();
    if (!/^\d+$/.test(seg)) {
      nums.push(null);
      break;
    }
    nums.push(Number(seg));
  }
  if (nums.length === 0 || nums[0] === null) return null;
  return {
    major: nums[0],
    minor: nums.length > 1 ? nums[1] : null,
    patch: nums.length > 2 ? nums[2] : null,
  };
}

/** 单侧区间判定：`bound` 为 min 时要求 `engine >= bound`，为 max 时要求 `engine <= bound`。 */
function withinBound(
  engine: EngineVersion,
  bound: EngineVersion,
  side: "min" | "max",
): boolean {
  if (engine.major !== bound.major) return false;
  // 只有双方都写了 minor 才比较（VCMI：doCheckMinor = checkMinor && 双方 minor != Any）
  if (bound.minor === null || engine.minor === null) return true;
  if (side === "min" && engine.minor < bound.minor) return false;
  if (side === "max" && engine.minor > bound.minor) return false;
  // patch 仅在 minor 可比且相等时才需区分
  if (bound.patch === null || engine.patch === null) return true;
  if (engine.minor !== bound.minor) return true;
  if (side === "min" && engine.patch < bound.patch) return false;
  if (side === "max" && engine.patch > bound.patch) return false;
  return true;
}

/**
 * 模组是否与当前引擎版本兼容。
 *
 * 返回 `true` 的三种情形：未声明 `compatibility` / 区间成立 / **引擎版本未知**
 * （`Unknown`、空串、非数字）——宁可放过也不要误报为"不兼容"。
 */
export function isModCompatible(
  engineVersion: string | null | undefined,
  compatibility: ModCompatibility | null | undefined,
): boolean {
  if (!compatibility) return true;
  const min = parseVcmiVersion(compatibility.min);
  const max = parseVcmiVersion(compatibility.max);
  if (!min && !max) return true;
  const engine = parseVcmiVersion(engineVersion);
  if (!engine) return true;
  if (min && !withinBound(engine, min, "min")) return false;
  if (max && !withinBound(engine, max, "max")) return false;
  return true;
}

/**
 * 兼容区间的可读文本（供 tooltip / 详情展示），如 `≥ 1.7.0`、`≤ 1.8`、`1.6.0 – 1.8`。
 * 无区间时返回 `null`（调用方不渲染这一行）。
 */
export function compatRangeText(
  compatibility: ModCompatibility | null | undefined,
): string | null {
  if (!compatibility) return null;
  const min = (compatibility.min ?? "").trim();
  const max = (compatibility.max ?? "").trim();
  if (!min && !max) return null;
  if (min && max) return `${min} – ${max}`;
  if (min) return `≥ ${min}`;
  return `≤ ${max}`;
}
