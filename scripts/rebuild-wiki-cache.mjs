#!/usr/bin/env node
/**
 * Wiki 缓存**全量重建**驱动（跨平台 CLI 包装）。
 *
 * 目的：让「重建」可以**指定游戏目录 / 缓存目录**，且**包含所有子模组**，
 * 效果与启动器 `ensureWikiCacheBuilt` **完全一致**（同一套 Rust 构建逻辑 + 同款分批：
 * ① 全部 mod 阶段一（仅数据）→ ② 城镇批阶段二 → ③ 其余批阶段二）。
 * 重建后用 `node scripts/verify-wiki-cache.mjs` 校验产物是否齐全。
 *
 * 用法：
 *   node scripts/rebuild-wiki-cache.mjs                          # 默认游戏目录 + 默认缓存
 *   node scripts/rebuild-wiki-cache.mjs --game D:/Games/H3       # 指定游戏目录
 *   node scripts/rebuild-wiki-cache.mjs --cache D:/tmp/mods      # 输出到别的缓存目录（核对用）
 *   node scripts/rebuild-wiki-cache.mjs --user D:/MyGames/vcmi   # 指定用户数据目录（默认按 dirs.json 解析）
 *   node scripts/rebuild-wiki-cache.mjs --fresh                  # 先清空全部 mod.json 强制全量重建
 *   node scripts/rebuild-wiki-cache.mjs --jobs eco|max|N         # 并发：eco=缺省（不设线程数，后端按系统空闲
 *                                                                #   自动提档）/ max 或 0=用满全部逻辑核 / N=固定线程数
 *   node scripts/rebuild-wiki-cache.mjs --idle 300               # 模拟"系统空闲 300 秒"（验证空闲感知提档）
 *   node scripts/rebuild-wiki-cache.mjs --log                    # 输出 tracing 日志（耗时构成分析）
 *
 * 耗时构成分析（定位首次构建慢的热点，配合隔离缓存目录避免污染真实缓存）：
 *   node scripts/rebuild-wiki-cache.mjs --cache D:/tmp/wiki-verify --fresh --jobs max --log > rebuild.log 2>&1
 *   # --jobs eco 复现启动器默认档（用户在用电脑时 = cpus/8）；--jobs max 可在数分钟内拿到同一批构成数据
 *   # 日志关注：每批 `本批并发档位` 与 `本批耗时构成 Top15`（info），以及每 mod `耗时构成`（debug）
 *   # 若觉日志啰嗦：VCMI_WIKI_QUIET=1 可关闭每批诊断行（批次汇总行仍保留）
 *
 * 等价的环境变量：VCMI_GAME_DIR / VCMI_MOCK_CACHE_DIR / VCMI_USER_DATA_DIR /
 * VCMI_WIKI_REBUILD_FRESH / VCMI_WIKI_THREADS / VCMI_WIKI_IDLE_SECS / VCMI_WIKI_LOG / VCMI_WIKI_QUIET
 */
import { spawn } from "child_process";
import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(__dirname, "..");

function parseArgs(argv) {
  const out = {};
  for (let i = 2; i < argv.length; i++) {
    const a = argv[i];
    if (!a.startsWith("--")) continue;
    const key = a.slice(2);
    const next = argv[i + 1];
    if (next && !next.startsWith("--")) {
      out[key] = next;
      i++;
    } else {
      out[key] = "true";
    }
  }
  return out;
}
const args = parseArgs(process.argv);

// 默认值（与 Rust 测试内缺省一致；env 优先，其次 CLI）
const DEFAULT_GAME = "C:/Games/Heroes3-VCMI";

const gameDir = path.resolve(args.game || process.env.VCMI_GAME_DIR || DEFAULT_GAME);
// 缓存默认 = `<游戏目录>/.vcmi-fv-launcher/mods`（与启动器便携部署同一目录；
// 启动器实际用「exe 同级/.vcmi-fv-launcher/mods」，便携包中 exe 就在游戏目录里）
const DEFAULT_CACHE = path.join(gameDir, ".vcmi-fv-launcher", "mods");
const cacheDir = path.resolve(args.cache || process.env.VCMI_MOCK_CACHE_DIR || DEFAULT_CACHE);
// 复数别名：--cache-dir / --game-dir
const userDir = args.user || args["user-dir"] || process.env.VCMI_USER_DATA_DIR || "";
const fresh = args.fresh === "true" || process.env.VCMI_WIKI_REBUILD_FRESH === "1";
// --jobs：数字 / max|0（用满全部逻辑核）/ eco（缺省：不设线程数，由后端按系统空闲自动提档）
const jobsArg = args.jobs ?? "eco";
const jobsEnv = jobsArg === "max" || jobsArg === "0" ? "0" : jobsArg === "eco" ? "" : String(jobsArg);
// --idle：模拟"系统空闲 N 秒"（验证空闲感知提档；真实运行由 Win32 空闲检测给出）
const idleArg = args.idle ?? process.env.VCMI_WIKI_IDLE_SECS ?? "";
// --log：开 tracing（DEBUG）到 stdout，用于分析耗时构成
const withLog = args.log === "true" || !!process.env.VCMI_WIKI_LOG;

if (!fs.existsSync(gameDir)) {
  console.error(`游戏目录不存在：${gameDir}`);
  process.exit(2);
}
if (!fs.existsSync(path.join(gameDir, "Mods"))) {
  console.error(`游戏目录下没有 Mods 子目录：${gameDir}`);
  process.exit(2);
}

// ⚠️ 安全护栏（修复误删 realms 事故）：缓存目录必须是 `<...>/.vcmi-fv-launcher/mods`。
// 若指向了启动器数据根 `.vcmi-fv-launcher`（缺了 `/mods`），`--fresh` 会连 realms /
// launcher.json / runtime / state.json 一起删掉。提前拦下，避免灾难性清空。
{
  const leaf = path.basename(cacheDir);
  const parentLeaf = path.basename(path.dirname(cacheDir));
  if (leaf !== "mods" || parentLeaf !== ".vcmi-fv-launcher") {
    console.error(
      `❌ 缓存目录必须以 /.vcmi-fv-launcher/mods 结尾（实得：${cacheDir}）。\n` +
        `   指向启动器数据根 .vcmi-fv-launcher 会误删 realms/launcher.json/runtime/state.json。\n` +
        `   请用 --cache 指向正确的缓存子目录（即以 /mods 结尾）。`,
    );
    process.exit(2);
  }
}

console.log("=== Wiki 缓存全量重建（含全部子模组）===");
console.log(`游戏目录：${gameDir}`);
console.log(`缓存目录：${cacheDir}`);
console.log(`用户目录：${userDir || "(按 dirs.json 自动解析)"}`);
console.log(`强制清空：${fresh ? "是" : "否（增量）"}`);
console.log(
  `并发线程：${
    jobsArg === "eco"
      ? "ECO + 空闲感知（后端按系统空闲自动提档）"
      : jobsArg === "max"
        ? "全部逻辑核"
        : jobsArg
  }`,
);
console.log(`模拟空闲：${idleArg === "" ? "(不模拟，用真实键鼠空闲)" : `${idleArg}s`}`);
console.log(`日志输出：${withLog ? "开（DEBUG，含耗时构成）" : "关（加 --log 开启）"}`);
console.log("");

if (args["dry-run"] === "true") {
  console.log("（--dry-run：仅解析参数并校验路径，不执行重建）");
  console.log(`待执行：cargo test --manifest-path src-tauri/Cargo.toml rebuild_all_with_core -- --ignored --nocapture`);
  console.log(
    `环境变量：VCMI_GAME_DIR / VCMI_MOCK_CACHE_DIR${jobsEnv !== "" ? ` / VCMI_WIKI_THREADS=${jobsEnv}` : "（不设线程数，走空闲感知）"}${idleArg !== "" ? ` / VCMI_WIKI_IDLE_SECS=${idleArg}` : ""}${fresh ? " / VCMI_WIKI_REBUILD_FRESH=1" : ""}${userDir ? " / VCMI_USER_DATA_DIR" : ""}`,
  );
  process.exit(0);
}

const env = {
  ...process.env,
  VCMI_GAME_DIR: gameDir,
  VCMI_MOCK_CACHE_DIR: cacheDir,
  // 显式定档 → 线程数直接覆盖；eco → 让测试走 throttle=None（应用默认档 + 空闲感知）
  ...(jobsEnv !== "" ? { VCMI_WIKI_THREADS: jobsEnv } : { VCMI_WIKI_THROTTLE: "eco" }),
  ...(userDir ? { VCMI_USER_DATA_DIR: userDir } : {}),
  ...(fresh ? { VCMI_WIKI_REBUILD_FRESH: "1" } : {}),
  ...(withLog ? { VCMI_WIKI_LOG: "1" } : {}),
  ...(idleArg !== "" ? { VCMI_WIKI_IDLE_SECS: String(idleArg) } : {}),
};

const child = spawn(
  "cargo",
  [
    "test",
    "--manifest-path",
    path.join("src-tauri", "Cargo.toml"),
    "rebuild_all_with_core",
    "--",
    "--ignored",
    "--nocapture",
  ],
  { cwd: REPO, env, stdio: "inherit", shell: process.platform === "win32" },
);

child.on("exit", (code) => {
  if (code === 0) {
    console.log("\n✅ 重建完成。校验产物：");
    console.log(`  node scripts/verify-wiki-cache.mjs --cache "${cacheDir}"`);
  } else {
    console.error(`\n❌ 重建失败（退出码 ${code}）`);
  }
  process.exit(code ?? 1);
});
