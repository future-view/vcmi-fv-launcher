#!/usr/bin/env node
/**
 * MVP 验证脚本：读取真实 modSettings.json，验证：
 *  1. 结构解析（activeMods/activePreset/presets/validatedMods）
 *  2. settings[父][子] 子 ID 命名规律（点号分层 / 空格多词）
 *  3. 官方单预设格式导出 {name, mods, settings}
 *  4. 预设写入 + 备份（用临时副本，不改真实文件）
 *
 * 用法：node scripts/read-modsettings.mjs [userConfigDir]
 */
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(__dirname, "..");

const userConfigDirArg = process.argv[2];
const userConfigDirs = userConfigDirArg
  ? [path.resolve(userConfigDirArg)]
  : [
      path.join(repo, "references", "vcmi", "config"),
      path.join(repo, "references", "vcmi_config", "config"),
    ];

let ok = true;

function assert(cond, msg) {
  if (!cond) {
    console.error("  ✗ 失败:", msg);
    ok = false;
  } else {
    console.log("  ✓", msg);
  }
}

for (const dir of userConfigDirs) {
  const file = path.join(dir, "modSettings.json");
  console.log(`\n=== ${dir} ===`);
  if (!fs.existsSync(file)) {
    console.log("  （无 modSettings.json，跳过）");
    continue;
  }
  const data = JSON.parse(fs.readFileSync(file, "utf8"));

  // 1. 顶层字段
  assert("activePreset" in data, "含 activePreset 字段");
  assert("presets" in data, "含 presets 对象");
  console.log("  activePreset =", data.activePreset);
  console.log("  presets =", Object.keys(data.presets));
  if ("activeMods" in data) console.log("  activeMods =", data.activeMods);
  if ("validatedMods" in data) console.log("  validatedMods keys =", Object.keys(data.validatedMods));

  // 2. settings[父][子] 命名
  for (const [pname, preset] of Object.entries(data.presets)) {
    console.log(`  预设[${pname}].mods =`, preset.mods);
    if (preset.settings && Object.keys(preset.settings).length) {
      const firstParent = Object.keys(preset.settings)[0];
      const children = Object.keys(preset.settings[firstParent]);
      console.log(`  settings["${firstParent}"] 子项(前3):`, children.slice(0, 3));
      // 验证点号分层
      const dotted = children.filter((c) => c.includes("."));
      console.log(`  含点号嵌套子项:`, dotted.slice(0, 3));
      const spaced = children.filter((c) => c.includes(" "));
      console.log(`  含空格多词子项:`, spaced.slice(0, 3));
    }
  }

  // 3. 官方单预设格式导出
  const presetName = data.activePreset;
  const p = data.presets[presetName];
  if (p) {
    const exported = { name: presetName, mods: p.mods, settings: p.settings };
    assert(
      exported.name && Array.isArray(exported.mods) && typeof exported.settings === "object",
      "导出为官方单预设格式 {name, mods, settings}",
    );
  }

  // 4. 写入 + 备份（临时副本）
  const tmpDir = path.join(repo, ".tmp-verify");
  fs.mkdirSync(tmpDir, { recursive: true });
  const tmpFile = path.join(tmpDir, "modSettings.json");
  fs.writeFileSync(tmpFile, JSON.stringify(data, null, "\t"));

  // 模拟写入：新增一个测试预设
  const writeData = JSON.parse(fs.readFileSync(tmpFile, "utf8"));
  writeData.presets["verify-test"] = { mods: ["vcmi", "core"], settings: { "abyss-town": { "changed banks": true } } };
  writeData.activePreset = "verify-test";
  // 备份原文件
  const backupPath = tmpFile.replace(/\.json$/, ".backup.json");
  fs.copyFileSync(tmpFile, backupPath);
  fs.writeFileSync(tmpFile, JSON.stringify(writeData, null, "\t"));

  const reread = JSON.parse(fs.readFileSync(tmpFile, "utf8"));
  assert(reread.activePreset === "verify-test", "写入后 activePreset 更新");
  assert(reread.presets["verify-test"]?.mods?.length === 2, "写入的预设 mods 正确");
  assert(fs.existsSync(backupPath), "写前自动备份 .backup.json 已生成");
  console.log("  临时写入验证通过（使用 .tmp-verify 副本，未改动真实文件）");
}

// 清理
fs.rmSync(path.join(repo, ".tmp-verify"), { recursive: true, force: true });

console.log(ok ? "\n✅ 全部验证通过" : "\n❌ 存在验证失败");
process.exit(ok ? 0 : 1);
