#!/usr/bin/env node
/**
 * 版本号单一来源 = `src-tauri/Cargo.toml` 的 `[package].version`（同时是 `CARGO_PKG_VERSION`，
 * 即运行时 `UpdateInfo.currentVersion` 的来源；Tauri 要求它与 `tauri.conf.json.version` 必须一致）。
 *
 * 用法：
 *   node scripts/sync-version.mjs                 # 把 Cargo.toml 版本同步到 tauri.conf.json（消除两处不一致）
 *   node scripts/sync-version.mjs --bump patch    # 0.1.0 -> 0.1.1，并同步 tauri.conf.json
 *   node scripts/sync-version.mjs --bump minor    # 0.1.0 -> 0.2.0
 *   node scripts/sync-version.mjs --bump major    # 0.1.0 -> 1.0.0
 *   node scripts/sync-version.mjs --set 0.2.0     # 显式写入指定版本并同步 tauri.conf.json（供 release:pack 注入用）
 *
 * 原则：发版只需改 Cargo.toml（或一条 --bump/--set 命令），不必手改两处；本脚本负责把 tauri.conf.json 对齐，
 * 避免 `tauri build` 因版本不一致直接失败。不改动其他字段、保持 tauri.conf.json 既有格式（2 空格缩进）。
 *
 * ⚠️ 版本值必须是合法 semver（Cargo 强制）：`0.2.0` ✅ / `0.2.0-rc1` ✅ / `v1.0.0` ❌ / `2026-10-01` ❌。
 */

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, "..");
const CARGO = path.join(ROOT, "src-tauri", "Cargo.toml");
const CONF = path.join(ROOT, "src-tauri", "tauri.conf.json");

function readCargoVersion(cargo) {
  const afterPkg = cargo.split(/^\s*\[package\s*\]/m)[1] ?? cargo;
  const m = afterPkg.match(/^\s*version\s*=\s*"([^"]+)"/m);
  if (!m) throw new Error("无法从 src-tauri/Cargo.toml 解析 [package].version");
  return m[1];
}

function bumpVersion(ver, level) {
  const parts = ver.split(".").map((x) => parseInt(x, 10) || 0);
  while (parts.length < 3) parts.push(0);
  if (level === "major") {
    parts[0] += 1;
    parts[1] = 0;
    parts[2] = 0;
  } else if (level === "minor") {
    parts[1] += 1;
    parts[2] = 0;
  } else if (level === "patch") {
    parts[2] += 1;
  } else {
    throw new Error(`--bump 仅支持 major|minor|patch，收到：${level}`);
  }
  return parts.join(".");
}

/**
 * 改写 `[package]` 段内的 `version`（**行级扫描**）。
 *
 * ⚠️ 历史 bug：旧实现用 `/(^\s*\[package\s*\])([\s\S]*?)(?=\n\s*\[|$)/m` ——
 * `/m` 下的 `$` 会在**第一个行尾**就满足，非贪婪量词立刻停下，`body` 变成空串 ⇒
 * "写版本"是**静默空操作**（打印说改了、文件没变），`version:bump` 同样一直无效。
 * 行级扫描与 `$` 语义无关，且能明确报错而不是静默失败。
 */
function setCargoVersion(cargo, newVer) {
  const eol = cargo.includes("\r\n") ? "\r\n" : "\n";
  const lines = cargo.split(/\r?\n/);
  let inPkg = false;
  for (let i = 0; i < lines.length; i++) {
    if (/^\s*\[/.test(lines[i])) {
      if (/^\s*\[package\s*\]\s*$/.test(lines[i])) {
        inPkg = true;
        continue;
      }
      if (inPkg) break; // [package] 段已结束
      continue;
    }
    if (inPkg && /^\s*version\s*=/.test(lines[i])) {
      lines[i] = lines[i].replace(/^(\s*version\s*=\s*)"[^"]*"/, `$1"${newVer}"`);
      return lines.join(eol);
    }
  }
  throw new Error("无法在 src-tauri/Cargo.toml 的 [package] 段内找到 version 行");
}

/**
 * 改写 `tauri.conf.json` 顶层的 `"version"`（**定向替换，不做 JSON 往返**）。
 * JSON 往返（parse → stringify）会把 `["**"]` 之类的紧凑数组重排成多行，
 * 造成无意义的巨大 diff；这里只动那一行，改完仍用 JSON.parse 校验合法性。
 */
function setConfVersion(text, newVer) {
  const re = /^(\s{2}"version"\s*:\s*)"[^"]*"/m;
  if (!re.test(text)) throw new Error('无法在 src-tauri/tauri.conf.json 顶层找到 "version" 行');
  const next = text.replace(re, `$1"${newVer}"`);
  JSON.parse(next); // 保底：改完必须是合法 JSON
  return next;
}

function main() {
  let bump = null;
  let set = null;
  const argv = process.argv.slice(2);
  for (let i = 0; i < argv.length; i++) {
    if (argv[i] === "--bump") {
      bump = argv[++i];
      if (!bump) throw new Error("--bump 缺少取值（major|minor|patch）");
    } else if (argv[i] === "--set") {
      set = argv[++i];
      if (!set) throw new Error("--set 缺少取值（如 0.2.0）");
      if (!/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/.test(set)) {
        throw new Error(`--set 取值不是合法 semver：${set}（例：0.2.0 / 0.2.0-rc1）`);
      }
    } else {
      throw new Error(`未知参数：${argv[i]}`);
    }
  }
  if (bump && set) throw new Error("--bump 与 --set 互斥");

  let cargo = fs.readFileSync(CARGO, "utf8");
  let version = readCargoVersion(cargo);

  if (set) {
    if (set === version) {
      console.log(`[sync-version] Cargo.toml 版本已是 ${version}，无需改动`);
    } else {
      cargo = setCargoVersion(cargo, set);
      fs.writeFileSync(CARGO, cargo, "utf8");
      console.log(`[sync-version] Cargo.toml 版本 ${version} -> ${set}`);
      version = set;
    }
  }

  if (bump) {
    const next = bumpVersion(version, bump);
    cargo = setCargoVersion(cargo, next);
    fs.writeFileSync(CARGO, cargo, "utf8");
    version = next;
    console.log(`[sync-version] Cargo.toml 版本 ${bump} 递增 -> ${version}`);
  }

  const confText = fs.readFileSync(CONF, "utf8");
  const prev = JSON.parse(confText).version ?? "?";
  if (prev === version) {
    console.log(`[sync-version] tauri.conf.json 版本已为 ${version}，无需改动`);
  } else {
    fs.writeFileSync(CONF, setConfVersion(confText, version), "utf8");
    console.log(`[sync-version] tauri.conf.json 版本 ${prev} -> ${version}`);
  }
}

try {
  main();
} catch (e) {
  console.error(`[sync-version] 失败：${e.message}`);
  process.exit(1);
}
