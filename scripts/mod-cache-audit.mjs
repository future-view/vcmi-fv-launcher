#!/usr/bin/env node
/**
 * 提取协议审计器（L2）：把"配置里有、缓存里没有"的字段检查产品化。
 *
 * 三种模式：
 *   --ci            仅校验协议覆盖率：内置 schema 的文本字段必须 100% 被 mod-cache-protocol.json
 *                   登记（或在 ignoredPaths 白名单）→ 不通过退出码 1（CI 阻断）。
 *   --schema-only   列出每个 schema 的文本字段与协议登记情况（人工排查用）。
 *   --game-dir <d>  扫描真实模组配置：统计"配置里出现、协议未登记"的文本字段路径（含命中 mod 数）。
 *
 * 依赖：
 *   src-tauri/resources/vcmi-schemas/*.json      内置 schema（sync-vcmi-schemas.mjs 生成）
 *   src-tauri/resources/mod-cache-protocol.json       提取协议表（字段登记 + 缓存映射）
 */
import fs from "node:fs";
import path from "node:path";
import yauzl from "yauzl";

const args = process.argv.slice(2);
const has = (f) => args.includes(f);
const argVal = (f, d) => { const i = args.indexOf(f); return i >= 0 && args[i + 1] ? args[i + 1] : d; };

const SCHEMA_DIR = "src-tauri/resources/vcmi-schemas";
const PROTOCOL = "src-tauri/resources/mod-cache-protocol.json";

/** VCMI 的 schema 含整行 // 注释与尾逗号 */
function looseParse(t) {
  const cleaned = t.replace(/^\s*\/\/.*$/gm, "").replace(/\/\/[^\n"\\]*$/gm, "").replace(/,\s*([}\]])/g, "$1");
  try { return JSON.parse(cleaned); } catch { return null; }
}
const TEXT_RE = /(^|\.)(name|description|biography|specialty|tooltip|text|texts|event|desc|title|message)(\.|$|\[\])/i;

/** 递归收集 schema 中的文本类字符串字段路径（`*`=additionalProperties，`[]`=数组项） */
function collectTextPaths(node, prefix, acc, depth = 0, seenRefs = new Set()) {
  if (!node || typeof node !== "object" || depth > 8) return;
  if (node.type === "string" || node.format === "textFile") {
    if (TEXT_RE.test(prefix) || /name|description|text|biography|specialty|tooltip|event/i.test(prefix.split(".").pop() ?? "")) {
      acc.add(prefix || "(root)");
    }
    return;
  }
  if (node.$ref && typeof node.$ref === "string") {
    // 只展开本文件内的 definitions 引用
    return node.$ref;
  }
  if (node.type === "array" && node.items) {
    collectTextPaths(node.items, `${prefix}[]`, acc, depth + 1, seenRefs);
    return;
  }
  for (const comb of ["oneOf", "anyOf", "allOf"]) {
    if (Array.isArray(node[comb])) for (const v of node[comb]) collectTextPaths(v, prefix, acc, depth + 1, seenRefs);
  }
  if (node.properties) {
    for (const [k, v] of Object.entries(node.properties)) {
      const p = prefix ? `${prefix}.${k}` : k;
      // $ref 展开（definitions）
      if (v && v.$ref && typeof v.$ref === "string" && v.$ref.startsWith("#/definitions/")) {
        acc.add(`${p} → ${v.$ref.replace("#/definitions/", "$def:")}`);
        continue;
      }
      collectTextPaths(v, p, acc, depth + 1, seenRefs);
    }
  }
  if (node.additionalProperties && typeof node.additionalProperties === "object") {
    collectTextPaths(node.additionalProperties, prefix ? `${prefix}.*` : "*", acc, depth + 1, seenRefs);
  }
}

// ---- 加载协议 ----
if (!fs.existsSync(PROTOCOL)) {
  console.error(`缺少协议表 ${PROTOCOL}（请先创建）`);
  process.exit(2);
}
const protocol = JSON.parse(fs.readFileSync(PROTOCOL, "utf8"));
/** schema 文件 → 该类型登记的配置路径集合（去掉 * / [] 后的前缀匹配用原样） */
const registeredBySchema = new Map();
for (const [cat, ent] of Object.entries(protocol.entities)) {
  const paths = new Set(ent.ignoredPaths ?? []);
  for (const tf of ent.textFields ?? []) for (const p of tf.configPaths) paths.add(p);
  registeredBySchema.set(ent.schema, { cat, paths });
}

// ---- schema-only / ci 模式 ----
if (has("--schema-only") || has("--ci")) {
  const outOfScope = new Set(Object.keys(protocol.outOfScope?.schemas ?? {}));
  const files = fs.readdirSync(SCHEMA_DIR).filter((f) => f.endsWith(".json") && f !== "manifest.json");
  let violations = 0;
  let declaredOut = 0;
  const lines = [];
  for (const f of files) {
    if (outOfScope.has(f)) { declaredOut += 1; continue; } // 已声明不纳入（outOfScope）
    const s = looseParse(fs.readFileSync(path.join(SCHEMA_DIR, f), "utf8"));
    if (!s) { lines.push(`### ${f} — 解析失败（跳过）`); continue; }
    const acc = new Set();
    collectTextPaths(s, "", acc);
    const reg = registeredBySchema.get(f);
    const list = [...acc].filter((p) => !p.includes("→ $def:"));
    const refs = [...acc].filter((p) => p.includes("→ $def:"));
    const unreg = reg ? list.filter((p) => !isRegistered(p, reg.paths)) : list;
    if (!reg) {
      lines.push(`### ${f} — 协议未登记该 schema（${list.length} 个文本字段）`);
      for (const p of list) lines.push(`    · ${p}`);
      continue;
    }
    lines.push(`### ${f} → ${reg.cat}（文本字段 ${list.length}，未登记 ${unreg.length}）`);
    for (const p of unreg) { lines.push(`    ✗ ${p}`); violations += 1; }
    for (const p of list.filter((x) => !unreg.includes(x))) lines.push(`    ✓ ${p}`);
    for (const r of refs) lines.push(`    ~ ${r}（引用 definitions，按展开处登记）`);
  }
  console.log(lines.join("\n"));
  if (declaredOut) console.log(`（outOfScope 声明跳过 ${declaredOut} 个 schema）`);
  if (has("--ci")) {
    const required = Object.keys(protocol.entities);
    const missing = [];
    for (const cat of required) {
      const ent = protocol.entities[cat];
      if (!fs.existsSync(path.join(SCHEMA_DIR, ent.schema))) missing.push(`${cat} → ${ent.schema} 不存在`);
    }
    if (violations || missing.length) {
      console.error(`\n[CI] 协议覆盖失败：未登记文本字段 ${violations} 个${missing.length ? `，缺失 schema ${missing.length} 个` : ""}`);
      process.exit(1);
    }
    console.log("\n[CI] 协议覆盖检查通过 ✓");
  }
  process.exit(0);
}

/** 登记匹配：精确路径，或登记含 `*`/`[]` 通配 */
function isRegistered(p, paths) {
  if (paths.has(p)) return true;
  for (const r of paths) {
    if (r === p) return true;
    const re = new RegExp("^" + r.replace(/[.+?^${}()|[\]\\]/g, "\\$&").replace(/\*/g, "[^.]+").replace(/\[\]/g, "\\[\\]") + "$");
    if (re.test(p)) return true;
    // 子路径也算覆盖（如登记 text 覆盖 text.name）
    if (p.startsWith(r + ".")) return true;
  }
  return false;
}

// ---- game-dir 模式：扫真实配置 ----
const gameDir = argVal("--game-dir", "");
if (!gameDir) {
  console.log("（未指定 --game-dir，跳过真实配置扫描）");
  process.exit(0);
}
const modsRoot = path.join(gameDir, "Mods");
if (!fs.existsSync(modsRoot)) {
  console.error(`未找到 ${modsRoot}`);
  process.exit(2);
}
function walk(d, acc = [], depth = 0, max = 9) {
  if (depth > max || !fs.existsSync(d)) return acc;
  for (const e of fs.readdirSync(d, { withFileTypes: true })) {
    const p = path.join(d, e.name);
    if (e.isDirectory()) walk(p, acc, depth + 1, max);
    else acc.push(p);
  }
  return acc;
}
// 路径 → 归属类型（按目录段名，容错大小写/单复数）
const CAT_DIRS = {
  artifacts: /(^|\/)(artifact|artifacts)(\/|$)/i,
  creatures: /(^|\/)(creature|creatures)(\/|$)/i,
  heroes: /(^|\/)(hero|heroes)(\/|$)/i,
  heroClasses: /(^|\/)(heroclass|heroclasses|classes)(\/|$)/i,
  spells: /(^|\/)(spell|spells)(\/|$)/i,
  skills: /(^|\/)(skill|skills)(\/|$)/i,
  factions: /(^|\/)(faction|factions)(\/|$)/i,
  terrains: /(^|\/)(terrain|terrains)(\/|$)/i,
};
const hits = new Map(); // cat → Map(path → count)
/** 文本字段判定：只看**字段名末段**（避免 `specialty.base.type` 这类中间段误报） */
const TEXT_KEY = /^(name|desc|description|biography|tooltip|event|message|title|text|infix)s?$/i;
function scanStrings(obj, prefix, cat, depth = 0) {
  if (!obj || typeof obj !== "object" || depth > 6) return;
  if (Array.isArray(obj)) { obj.slice(0, 3).forEach((v) => scanStrings(v, `${prefix}[]`, cat, depth + 1)); return; }
  for (const [k, v] of Object.entries(obj)) {
    const p = prefix ? `${prefix}.${k}` : k;
    if (typeof v === "string") {
      if (TEXT_KEY.test(k)) {
        if (!hits.has(cat)) hits.set(cat, new Map());
        hits.get(cat).set(p, (hits.get(cat).get(p) ?? 0) + 1);
      }
    } else if (v && typeof v === "object") scanStrings(v, p, cat, depth + 1);
  }
}
function handle(text, rel) {
  const v = looseParse(text);
  if (!v || typeof v !== "object" || Array.isArray(v)) return;
  for (const [cat, re] of Object.entries(CAT_DIRS)) {
    if (!re.test(rel)) continue;
    const keys = Object.keys(v);
    // `{<id>: {...}, <id2>: {...}}`：逐实体扫描（路径从实体内部开始，与协议登记形态一致）；
    // 跨 mod 引用（`core:xxx`）跳过
    const isEntityMap = keys.length > 0 && keys.every((k) => v[k] && typeof v[k] === "object" && !Array.isArray(v[k]));
    if (isEntityMap) {
      for (const k of keys) {
        if (k.includes(":")) continue;
        scanStrings(v[k], "", cat);
      }
    } else {
      scanStrings(v, "", cat);
    }
  }
}

console.log(`扫描模组配置：${modsRoot}`);
for (const f of walk(modsRoot).filter((f) => /\.json$/i.test(f) && /config/i.test(f))) {
  try { handle(fs.readFileSync(f, "utf8"), f); } catch { /* ignore */ }
}
const zips = walk(modsRoot).filter((f) => /\.zip$/i.test(f));
let zi = 0;
for (const z of zips) {
  zi += 1;
  await new Promise((resolve) => {
    yauzl.open(z, { lazyEntries: true }, (err, zip) => {
      if (err || !zip) return resolve();
      zip.readEntry();
      zip.on("entry", (entry) => {
        const name = entry.fileName.replace(/\\/g, "/");
        if (!name.toLowerCase().endsWith(".json") || !/config/i.test(name)) return zip.readEntry();
        zip.openReadStream(entry, (e2, rs) => {
          if (e2 || !rs) return zip.readEntry();
          const chunks = [];
          rs.on("data", (c) => chunks.push(c));
          rs.on("end", () => {
            try { handle(Buffer.concat(chunks).toString("utf8"), name); } catch { /* ignore */ }
            zip.readEntry();
          });
        });
      });
      zip.on("end", resolve);
    });
  });
  if (zi % 200 === 0) console.log(`  zip ${zi}/${zips.length}`);
}

console.log("\n== 配置里出现但协议未登记的文本字段（按命中数） ==");
let total = 0;
for (const [cat, m] of hits) {
  const ent = Object.values(protocol.entities).find((e) => e.cat === cat || e.schema.startsWith(cat.slice(0, 4)));
  const reg = ent ? new Set([...(ent.ignoredPaths ?? []), ...(ent.textFields ?? []).flatMap((t) => t.configPaths)]) : new Set();
  const unreg = [...m.entries()].filter(([p]) => !isRegistered(p, reg)).sort((a, b) => b[1] - a[1]);
  if (!unreg.length) { console.log(`\n### ${cat} ✓ 全部已登记`); continue; }
  console.log(`\n### ${cat}（未登记 ${unreg.length} 种路径）`);
  for (const [p, n] of unreg.slice(0, 25)) { console.log(`  ${String(n).padStart(5)} × ${p}`); total += 1; }
}
console.log(`\n合计未登记路径种类：${total}`);
