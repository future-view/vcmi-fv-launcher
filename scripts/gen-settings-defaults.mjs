#!/usr/bin/env node
/**
 * 从 VCMI 官方 settings schema（docs/plan/settings.json）自动提取：
 *  1. SETTING_SCHEMA_DEFAULTS  —— 每个叶子字段的官方默认值（平面键 "general.language"）
 *  2. SETTING_SCHEMA_ENUM_OPTIONS —— 每个枚举字段的候选项数组（平面键）
 *
 * 目标：让启动器的默认值 / 枚举候选跟随 VCMI 源码迭代，消除「手工逐行录入默认值」的硬编码逻辑。
 * 产品决策（如 useSavePrefix 故意改 false）由 src/lib/settingsSchema.ts 中的 SETTING_DEFAULT_OVERRIDES 覆盖层承接，
 * 不在此文件内硬编码。
 *
 * 用法：node scripts/gen-settings-defaults.mjs
 * 输出：src/generated/settingDefaults.ts（自动生成，勿手改）
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(__dirname, "..");
const schemaPath = path.join(repo, "docs", "config", "settings_vcmi-1.7.5.json");
const outPath = path.join(repo, "src", "generated", "settingDefaults.ts");

// schema 文件顶部带 // 行注释，先剔除再解析（JSON.parse 不支持注释）
const raw = fs.readFileSync(schemaPath, "utf8");
const stripped = raw
  .split("\n")
  .filter((line) => !/^\s*\/\//.test(line))
  .join("\n");
const schema = JSON.parse(stripped);

/** 递归遍历 schema 的 properties，收集默认值与枚举 */
function walk(node, prefix, defaults, enums, stringKeys) {
  if (!node || typeof node !== "object") return;
  const props = node.properties;
  if (!props) return;
  for (const [key, sub] of Object.entries(props)) {
    const flat = prefix ? `${prefix}.${key}` : key;
    if (!sub || typeof sub !== "object") continue;
    // 叶子字段：带 default 且非对象类型 → 记录默认值
    if ("default" in sub && sub.type !== "object") {
      defaults[flat] = sub.default;
    }
    // 字符串枚举 → 记录候选项
    if (Array.isArray(sub.enum) && sub.enum.every((e) => typeof e === "string")) {
      enums[flat] = sub.enum;
    }
    // 字符串类型（含带 default 的自由字符串键，如 video.driver / logging level）→ 记录为「已知字符串键」
    // 用于校验脚本区分「UI 自定义的枚举（合法）」与「真正过期的标签键」
    if (sub.type === "string" && "default" in sub) {
      stringKeys[flat] = true;
    }
    // 递归子对象
    if (sub.type === "object") {
      walk(sub, flat, defaults, enums, stringKeys);
    }
  }
}

const defaults = {};
const enums = {};
const stringKeys = {};
walk(schema, "", defaults, enums, stringKeys);

const json = (v) => JSON.stringify(v, null, 2);

const header = `// 本文件由 scripts/gen-settings-defaults.mjs 自动生成，请勿手工修改。
// 数据源：docs/plan/settings.json（VCMI 官方 settings schema）。
// 重新生成：npm run gen:settings
`;

const content =
  header +
  `export const SETTING_SCHEMA_DEFAULTS: Record<string, unknown> = ${json(defaults)};\n\n` +
  `export const SETTING_SCHEMA_ENUM_OPTIONS: Record<string, string[]> = ${json(enums)};\n\n` +
  `// 带默认值的字符串类型键（含自由字符串，如 driver / 日志等级），用于校验脚本识别「UI 自定义枚举」避免误报
  export const SETTING_SCHEMA_STRING_KEYS: Record<string, true> = ${json(stringKeys)};\n`;

fs.mkdirSync(path.dirname(outPath), { recursive: true });
fs.writeFileSync(outPath, content, "utf8");

const dCount = Object.keys(defaults).length;
const eCount = Object.keys(enums).length;
console.log(`✓ 已生成 ${outPath}`);
console.log(`  默认值键：${dCount} 个`);
console.log(`  枚举候选键：${eCount} 个`);
