/**
 * `modSettings.json` / `settings.json` 的原始读写，以及嵌套键 ↔ 扁平键转换。
 *
 * 拆分自原 `@/lib/tauri.ts` 单体；对外仍由该 barrel 统一导出，调用方无需改动。
 */
import { invoke } from "@tauri-apps/api/core";
import { feErr } from "@/lib/backendError";
import { isTauri } from "./core";

/** 读取 modSettings.json。
 * `configDir` 可选：传入领域配置目录时读取该领域隔离目录下的 modSettings.json（§3.5.1）。 */
export async function readModSettingsJson(configDir?: string): Promise<Record<string, unknown> | null> {
  if (!isTauri()) return null;
  try {
    return await invoke("read_mod_settings", configDir ? { configDir } : {});
  } catch {
    return null;
  }
}

/** 写回 modSettings.json */
export async function writeModSettingsJson(data: unknown): Promise<void> {
  if (!isTauri()) throw new Error(feErr("frontend_not_tauri"));
  return invoke("write_mod_settings", { data });
}

/** 读取 VCMI settings.json。configDir 可选：传入领域配置目录时读取该领域隔离目录（§3.5.1） */
export async function readVcmiSettings(configDir?: string): Promise<Record<string, unknown> | null> {
  if (!isTauri()) return null;
  try {
    return await invoke("read_vcmi_settings", { configDir: configDir ?? null });
  } catch {
    return null;
  }
}

/** 写回 VCMI settings.json（入参为平面键字典，写出前还原为嵌套结构）。configDir 可选：写入领域目录（§3.5.1） */
export async function writeVcmiSettings(data: unknown, configDir?: string): Promise<void> {
  if (!isTauri()) throw new Error(feErr("frontend_not_tauri"));
  const payload = isPlainObject(data) ? unflatten(data as Record<string, unknown>) : data;
  return invoke("write_vcmi_settings", { data: payload, configDir: configDir ?? null });
}

/* ----------------------- 嵌套 <-> 平面键 转换 ----------------------- */
// VCMI settings.json 是嵌套结构（"general":{...}），而前端 schema 使用平面键
// （"general.language"）。读写时在此处做转换，对外文件保持 VCMI 原生嵌套。

function isPlainObject(v: unknown): v is Record<string, unknown> {
  return typeof v === "object" && v !== null && !Array.isArray(v);
}

/** 把嵌套对象展开为平面键字典（"a":{"b":1} -> "a.b":1） */
export function flatten(obj: Record<string, unknown>, prefix = ""): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(obj)) {
    const key = prefix ? `${prefix}.${k}` : k;
    if (isPlainObject(v)) Object.assign(out, flatten(v, key));
    else out[key] = v;
  }
  return out;
}

/** 把平面键字典还原为嵌套对象（"a.b":1 -> {a:{b:1}}） */
export function unflatten(flat: Record<string, unknown>): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(flat)) {
    const parts = k.split(".");
    let cur = out;
    for (let i = 0; i < parts.length - 1; i++) {
      const p = parts[i];
      if (!isPlainObject(cur[p])) cur[p] = {};
      cur = cur[p] as Record<string, unknown>;
    }
    cur[parts[parts.length - 1]] = v;
  }
  return out;
}

/**
 * 判断平面键路径是否包含数组索引段（纯数字段，如 "logging.loggers.0"）。
 * 后端 remove_path 对数组索引执行缩位删除，易产生空洞/错位，故需特殊处理。
 */
export function hasArrayIndexSegment(key: string): boolean {
  return key.split(".").some((seg) => seg !== "" && /^\d+$/.test(seg));
}

/**
 * 把数组元素路径归并到其所属的「数组键」整键。
 * 例："logging.loggers.0.name" / "logging.loggers.0" → "logging.loggers"。
 * 用途：重置时按整键删除数组（而非逐元素缩位删除），避免破坏数组结构。
 * 非数组路径原样返回。
 */
export function collapseToArrayKey(key: string): string {
  const parts = key.split(".");
  const idx = parts.findIndex((seg) => seg !== "" && /^\d+$/.test(seg));
  if (idx === -1) return key;
  return parts.slice(0, idx).join(".");
}
