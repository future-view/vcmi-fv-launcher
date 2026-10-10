#!/usr/bin/env node
/**
 * 同步 VCMI 配置定义（JSON Schema）到启动器内置资源。
 *
 * 目的（提取协议 L1）：启动器对模组配置的读取/写入以**内置的 vcmi-jsonSchema** 为准，
 * 不依赖开发机的 `references/vcmi`（发行版不含）。后续 VCMI 升级时重跑本脚本即可更新，
 * 无需重新编译（运行时优先读用户覆盖目录，其次读内置资源目录）。
 *
 * 用法：node scripts/sync-vcmi-schemas.mjs [--src <schemas 目录>] [--vcmirepo <VCMI 源码目录>]
 *
 * 产出：
 *   src-tauri/resources/vcmi-schemas/*.json      配置 schema（排除 lobbyProtocol 联机协议）
 *   src-tauri/resources/vcmi-schemas/manifest.json  { vcmiVersion, syncedAt, contentHash, files[] }
 */
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { execSync } from "node:child_process";

const args = process.argv.slice(2);
const argVal = (flag, dflt) => {
  const i = args.indexOf(flag);
  return i >= 0 && args[i + 1] ? args[i + 1] : dflt;
};

const srcDir = argVal("--src", "references/vcmi/config/schemas");
const vcmiRepo = argVal("--vcmirepo", "references/vcmi");
const dstDir = "src-tauri/resources/vcmi-schemas";

if (!fs.existsSync(srcDir)) {
  console.error(`源目录不存在：${srcDir}（需要 VCMI 源码的 config/schemas）`);
  process.exit(1);
}

const sha256 = (buf) => crypto.createHash("sha256").update(buf).digest("hex");

// VCMI 版本：优先 git describe（源码仓库），回退 mod.json，再回退未知
function detectVcmiVersion() {
  try {
    const v = execSync(`git -C "${vcmiRepo}" describe --tags --always 2>/dev/null`, { encoding: "utf8" }).trim();
    if (v) return v;
  } catch { /* ignore */ }
  for (const p of ["references/Heroes3-VCMI/Mods/vcmi/mod.json", "references/vcmi/Mods/vcmi/mod.json"]) {
    try {
      const m = JSON.parse(fs.readFileSync(p, "utf8").replace(/^\s*\/\/.*$/gm, ""));
      if (m.version) return String(m.version);
    } catch { /* ignore */ }
  }
  return "unknown";
}

fs.mkdirSync(dstDir, { recursive: true });
const files = [];
const hashes = [];
for (const e of fs.readdirSync(srcDir, { withFileTypes: true })) {
  if (!e.isFile() || !e.name.endsWith(".json")) continue;
  const src = path.join(srcDir, e.name);
  const buf = fs.readFileSync(src);
  fs.writeFileSync(path.join(dstDir, e.name), buf);
  files.push(e.name);
  hashes.push(`${e.name}:${sha256(buf)}`);
}
// 清理目标目录里已不在源中的旧文件（保持与源一致）
for (const e of fs.readdirSync(dstDir, { withFileTypes: true })) {
  if (e.isFile() && e.name.endsWith(".json") && e.name !== "manifest.json" && !files.includes(e.name)) {
    fs.rmSync(path.join(dstDir, e.name));
    console.log(`  移除旧文件 ${e.name}`);
  }
}

const contentHash = sha256(hashes.sort().join("\n")).slice(0, 16);
const manifest = {
  note: "VCMI 配置定义（JSON Schema）内置快照。由 scripts/sync-vcmi-schemas.mjs 生成，勿手改。",
  vcmiVersion: detectVcmiVersion(),
  syncedAt: new Date().toISOString(),
  contentHash,
  fileCount: files.length,
  files: files.sort(),
};
fs.writeFileSync(path.join(dstDir, "manifest.json"), JSON.stringify(manifest, null, 2) + "\n");

console.log(`已同步 ${files.length} 个 schema → ${dstDir}`);
console.log(`   VCMI 版本: ${manifest.vcmiVersion}`);
console.log(`   contentHash: ${contentHash}`);
console.log(`   提示：manifest.contentHash 变化时应递增 WIKI_CACHE_VERSION 并复核 mod-cache-protocol.json`);
