#!/usr/bin/env node
/**
 * 从设置项结构生成「设置面板文案的键清单」`src/generated/settingKeys.ts`。
 *
 * 为什么需要它：`LangDict.settings` 的 `tags` / `items` 段是 `Record<SettingItemId, …>` 这类
 * **穷举映射**，词典与设置项结构的对齐关系因此完全由 `tsc` 把关（CI 的 `npx tsc --noEmit`）：
 *  - 新增设置项忘了补译 → 编译失败；
 *  - 删了设置项残留译文 → 编译失败（多余的键不在联合类型里）；
 *  - 新增一门语言 → 编译器逐条列出全部待译文案。
 * 于是仓库里不再需要单独的多语言完整性脚本（原 `verify-enum-labels.mjs` 的职责已被此机制取代）。
 *
 * 用法：node scripts/gen-setting-keys.mjs      （改了 settingsSchema.ts 之后跑一次）
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const repo = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SCHEMA = path.join(repo, "src/lib/settingsSchema.ts");
const OUT = path.join(repo, "src/generated/settingKeys.ts");

/** 候选值即 VCMI 语言标识的设置项：显示名由 `src/lib/vcmiLangs.ts`（语言无关）提供，不需要译名 */
const LANG_KEYS = new Set(["general.language", "general.gameDataLanguage", "fv.uiLang"]);

/** 取值不写在 `o/candidates` 里的跨键控件（`fullscreenGroup` / `creatureDisplay` 三态） */
const EXTRA_ENUM_VALUES = {
  "video.fullscreen": ["windowed", "borderless", "exclusive"],
  "gameTweaks.availableCreaturesAsDwellingLabel": ["none", "growth", "available"],
};

const src = fs.readFileSync(SCHEMA, "utf8");
if (src.includes("l: L(")) {
  console.error("✗ settingsSchema.ts 里仍有 `l: L(...)` 文案 —— 设置项文本必须放在 src/i18n 词典里。");
  process.exit(1);
}

const tagIds = [];
const itemIds = [];
/** 设置项 id -> 候选原始值清单 */
const valuesById = new Map();

for (const line of src.split("\n")) {
  // 标签行：`  { id: "general", icon: "Cog", items: GENERAL },`
  const tag = line.match(/^\s*\{\s*id:\s*"([^"]+)",\s*icon:/);
  if (tag) {
    tagIds.push(tag[1]);
    continue;
  }
  // 设置项行：`  { k: "...", id: "...", ctl: ... }`
  const item = line.match(/^\s*\{\s*k:\s*"[^"]+",\s*id:\s*"([^"]+)"/);
  if (!item) continue;
  const id = item[1];
  itemIds.push(id);
  const values = new Set();
  for (const field of ["o", "enabledO", "candidates"]) {
    const at = line.indexOf(`${field}: [`);
    if (at === -1) continue;
    const end = line.indexOf("]", at);
    for (const m of line.slice(at, end + 1).matchAll(/"((?:[^"\\]|\\.)*)"/g)) values.add(JSON.parse(`"${m[1]}"`));
  }
  if (values.size) valuesById.set(id, [...values]);
}

for (const [id, extra] of Object.entries(EXTRA_ENUM_VALUES)) {
  valuesById.set(id, [...new Set([...(valuesById.get(id) ?? []), ...extra])]);
}

const enumKeys = [];
for (const [id, values] of valuesById) {
  if (LANG_KEYS.has(id)) continue; // 语言选项不产译名键（走 VCMI_LANGS 的本地化名）
  for (const v of values) enumKeys.push(`${id}.${v}`);
}
enumKeys.sort();

const literalList = (names) => names.map((n) => `  ${JSON.stringify(n)},`).join("\n");

const out = `/**
 * ⚠️ 自动生成，请勿手工修改 —— 数据源 \`src/lib/settingsSchema.ts\`，重新生成 \`npm run gen:setting-keys\`。
 *
 * 这些键清单是 \`src/i18n/types.ts\` 里 \`LangDict.settings\` 各段的**穷举依据**：
 * 设置面板的文案只存在于 i18n 词典（\`src/i18n/settingsText.*.ts\`），本文件只提供「该有哪些键」。
 * 生成于设置项结构之后，若忘记重跑，词典与结构的不一致会以 \`tsc\` 编译错误的形式暴露。
 */

/** 设置页左侧标签 id（共 ${tagIds.length} 个） */
export const SETTING_TAG_IDS = [
${literalList(tagIds)}
] as const;
export type SettingTagId = (typeof SETTING_TAG_IDS)[number];

/** 设置项 id —— 与 VCMI \`settings.json\` 的 key 一一对应（共 ${itemIds.length} 项） */
export const SETTING_ITEM_IDS = [
${literalList(itemIds)}
] as const;
export type SettingItemId = (typeof SETTING_ITEM_IDS)[number];

/** 需要译名的取值键：\`\${设置项 id}.\${原始值}\`（共 ${enumKeys.length} 条；语言选项除外） */
export const SETTING_ENUM_KEYS = [
${literalList(enumKeys)}
] as const;
export type SettingEnumKey = (typeof SETTING_ENUM_KEYS)[number];

/** 取值原始值集合（通用兜底词 \`valueWords\` 的键域；允许只覆盖其中一部分，未覆盖者原样显示） */
export const SETTING_VALUE_WORDS = [
${literalList([...new Set(enumKeys.map((k) => k.slice(k.indexOf(".") < 0 ? 0 : k.lastIndexOf(".") + 1)))].sort())}
] as const;
export type SettingValueWord = (typeof SETTING_VALUE_WORDS)[number];

/** 语言选项类设置项：其候选显示名来自 \`src/lib/vcmiLangs.ts\`，不进词典 */
export const SETTING_LANG_OPTION_KEYS = [
${literalList([...LANG_KEYS])}
] as const;
export type SettingLangOptionKey = (typeof SETTING_LANG_OPTION_KEYS)[number];
`;

fs.writeFileSync(OUT, out.replace(/\r?\n$/, "\n"), "utf8");
console.log(`✓ ${path.relative(repo, OUT)}：标签 ${tagIds.length} · 设置项 ${itemIds.length} · 取值键 ${enumKeys.length}`);
if (tagIds.length !== 13 || itemIds.length < 150) {
  console.error("!! 数量与预期不符，请检查 settingsSchema.ts 的行格式（每个设置项一行）");
  process.exit(1);
}
