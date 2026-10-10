#!/usr/bin/env node
/**
 * IPC 契约一致性检查（可进 CI）：比对
 *   ① 前端 `invoke("x")` 字面量
 *   ② 后端 `#[tauri::command]` 函数（src-tauri/src/*.rs）
 *   ③ `lib.rs` 的 `generate_handler!` 注册表
 *
 * 检查项：
 *   - 前端调用了但后端未定义/未注册（调用必然失败）
 *   - 后端定义了但未注册（幽灵命令）
 *   - 前端未调用（僵尸 API 候选，仅提示不阻断）
 *   - 前端传参与后端参数名不一致（camelCase ↔ snake_case）
 *
 * 用途：src-tauri 目录重构期间的保护网——命令名或参数被改动时立刻暴露。
 * 见 docs/architecture/backend.zh-CN.md。
 *
 * 用法：node scripts/check-invoke-parity.mjs        # 有差异退出码 1
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(__dirname, "..");
const TAURI_SRC = path.join(REPO, "src-tauri", "src");
const FE_FILE = path.join(REPO, "src", "lib", "tauri.ts");

/** Tauri 注入参数（不出现在 IPC 参数里） */
const INJECT_PREFIXES = [
  "State<", "tauri::State<", "AppHandle", "tauri::AppHandle", "&AppHandle", "&tauri::AppHandle",
  "Window", "tauri::Window", "WebviewWindow", "tauri::WebviewWindow", "&Window", "&tauri::Window",
];

const camel = (s) => s.replace(/_([a-z0-9])/g, (_, c) => c.toUpperCase());

/**
 * 去掉 Rust/TS 源码中的注释（跳过字符串字面量，避免 http:// 之类被误伤）。
 *
 * ⚠️ Rust 的生命周期（`&'static str`）不能被当作字符串起点 —— 否则会吞掉大段代码，
 * 使注释剥离失效（曾导致 `build_mods_wiki_batch` 的 `throttle`/`phase` 参数解析丢失）。
 */
function stripComments(src) {
  let out = "";
  let i = 0;
  while (i < src.length) {
    const c = src[i];
    if (c === '"' || c === "`") {
      const q = c;
      out += c; i++;
      while (i < src.length) {
        out += src[i];
        if (src[i] === "\\") { i++; out += src[i] ?? ""; i++; continue; }
        if (src[i] === q) { i++; break; }
        i++;
      }
      continue;
    }
    if (c === "'") {
      if (src[i + 2] === "'") { out += src.slice(i, i + 3); i += 3; continue; } // 'x' 字符字面量
      if (src[i + 1] === "\\") {
        let j = i + 2;
        while (j < src.length && src[j] !== "'") j++;
        out += src.slice(i, j + 1); i = j + 1; continue; // '\t' / '\'' 转义字面量
      }
      out += c; i++; continue; // 生命周期（&'static 等）：原样保留
    }
    if (c === "/" && src[i + 1] === "/") { while (i < src.length && src[i] !== "\n") i++; continue; }
    if (c === "/" && src[i + 1] === "*") { i += 2; while (i < src.length && !(src[i] === "*" && src[i + 1] === "/")) i++; i += 2; continue; }
    out += c; i++;
  }
  return out;
}

/** 从 `(` 起按深度扫描到匹配的 `)` */
function scanParens(text, openIdx) {
  let depth = 0;
  for (let i = openIdx; i < text.length; i++) {
    if (text[i] === "(") depth++;
    else if (text[i] === ")") { depth--; if (depth === 0) return text.slice(openIdx + 1, i); }
  }
  return "";
}

function splitTop(text) {
  const out = [];
  let depth = 0, cur = "";
  for (const ch of text) {
    if ("(<[{".includes(ch)) depth++;
    else if (")>]}".includes(ch)) depth--;
    if (ch === "," && depth === 0) { out.push(cur); cur = ""; } else cur += ch;
  }
  if (cur.trim()) out.push(cur);
  return out;
}

/** 递归收集 src-tauri/src 下的全部 .rs 文件（模块已按 `infra/vcmi/wiki/commands/...` 分层） */
function rustFiles(dir) {
  const out = [];
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) out.push(...rustFiles(p));
    else if (e.name.endsWith(".rs")) out.push(p);
  }
  return out;
}

/** ① 后端定义 */
function readBackend() {
  const map = new Map();
  for (const file of rustFiles(TAURI_SRC)) {
    const raw = fs.readFileSync(file, "utf8");
    const text = stripComments(raw);
    let idx = 0;
    while ((idx = text.indexOf("#[tauri::command]", idx)) !== -1) {
      idx += 17;
      const after = text.slice(idx, idx + 800);
      const m = after.match(/(?:pub\s+)?(?:async\s+)?fn\s+([a-z0-9_]+)\s*\(/);
      if (!m) continue;
      const parenIdx = idx + after.indexOf("(", after.indexOf(m[0]));
      const inner = scanParens(text, parenIdx);
      const params = splitTop(inner)
        .map((p) => p.replace(/\/\/[^\n]*/g, "").trim()) // 双保险：剥离参数间的行注释
        .filter(Boolean)
        .map((p) => p.match(/^(?:mut\s+)?([a-z0-9_]+)\s*:\s*([\s\S]+)$/))
        .filter(Boolean)
        .filter((mm) => !INJECT_PREFIXES.some((pre) => mm[2].trim().startsWith(pre)))
        .map((mm) => camel(mm[1]));
      map.set(m[1], { file: path.relative(REPO, file), params });
    }
  }
  return map;
}

/** ② lib.rs 注册表 */
function readRegistry() {
  const lib = stripComments(fs.readFileSync(path.join(TAURI_SRC, "lib.rs"), "utf8"));
  const start = lib.indexOf("generate_handler!");
  const body = start === -1 ? "" : lib.slice(start, lib.indexOf("])", start));
  const names = new Set();
  for (const line of body.split("\n")) {
    const t = line.trim().replace(/,$/, "");
    if (/^[a-z0-9_]+$/.test(t)) names.add(t);
  }
  return names;
}

/** ③ 前端 invoke */
function readFrontend() {
  const text = stripComments(fs.readFileSync(FE_FILE, "utf8"));
  const map = new Map();
  let search = 0;
  while (true) {
    const i = text.indexOf("invoke", search);
    if (i === -1) break;
    search = i + 6;
    const nextCh = text[i + 6];
    if (nextCh !== "(" && nextCh !== "<") continue;
    let p = i + 6;
    if (text[p] === "<") {
      let depth = 0;
      while (p < text.length) {
        if (text[p] === "<") depth++;
        else if (text[p] === ">") { depth--; if (depth === 0) { p++; break; } }
        p++;
      }
    }
    if (text[p] !== "(") continue;
    const inner = scanParens(text, p);
    const args = splitTop(inner);
    const nameMatch = (args[0] || "").trim().match(/^"([a-z0-9_]+)"$/);
    if (!nameMatch) continue;
    const name = nameMatch[1];
    const rec = map.get(name) || { keys: new Set(), dynamic: false };
    if (args.length > 1) {
      const obj = args.slice(1).join(",").trim();
      if (obj.startsWith("{") && obj.endsWith("}")) {
        for (const entry of splitTop(obj.slice(1, -1))) {
          const key = entry.trim().split(":")[0].trim();
          if (/^[a-zA-Z_][a-zA-Z0-9_]*$/.test(key)) rec.keys.add(key);
          else if (entry.includes("...")) rec.dynamic = true;
        }
      } else rec.dynamic = true;
    }
    map.set(name, rec);
  }
  return map;
}

const backend = readBackend();
const registry = readRegistry();
const frontend = readFrontend();

const problems = [];
const warnings = [];

for (const [name, fe] of frontend) {
  const be = backend.get(name);
  if (!be) { problems.push(`[前端→后端] 调用了不存在的命令：${name}`); continue; }
  if (!registry.has(name)) problems.push(`[注册表] 后端定义但未注册（前端会调用失败）：${name}`);
  if (!fe.dynamic) {
    const missing = be.params.filter((k) => !fe.keys.has(k));
    const extra = [...fe.keys].filter((k) => !be.params.includes(k));
    if (missing.length || extra.length) {
      problems.push(`[参数] ${name}: 后端=${JSON.stringify(be.params)} 前端=${JSON.stringify([...fe.keys])} 缺失=${JSON.stringify(missing)} 多余=${JSON.stringify(extra)}`);
    }
  }
}
for (const name of backend.keys()) if (!frontend.has(name)) warnings.push(`[僵尸候选] 后端定义了但前端未调用：${name}`);
for (const name of registry) if (!backend.has(name)) problems.push(`[注册表] 注册了不存在的命令：${name}`);

console.log(`后端命令 ${backend.size} 个，注册表 ${registry.size} 项，前端 invoke ${frontend.size} 个。`);
if (warnings.length) {
  console.log("\n告警（不阻断）：");
  for (const w of warnings) console.log("  " + w);
}
if (problems.length) {
  console.error("\n发现问题（IPC 契约不一致）：");
  for (const p of problems) console.error("  " + p);
  process.exit(1);
}
console.log("\n✅ IPC 契约一致。");
