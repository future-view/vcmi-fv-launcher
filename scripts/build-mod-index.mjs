#!/usr/bin/env node
/**
 * FV 聚合模组索引生成脚本：把官方仓库索引（`vcmi-*.json`）升级成 FV 预聚合索引。
 *
 * 动机（见 docs/features/mods-repository.zh-CN.md / C2）：
 * - 官方索引条目**不带 version / sha256 / changelog**，启动器只能对每个模组逐条拉 mod.json
 *   才能判定「可更新」，140 个模组的启动开销不可接受；且无法做 sha256 强校验。
 * - 本脚本离线把上述字段**预聚合**进每条的 `fv` 扩展对象，产出的索引 URL 填进
 *   设置页 `launcher.extraRepositoryURL`（或替换默认仓库）即可让启动器：
 *     · 直接读 `fv.version` 做更新检测（不再逐条触网）；
 *     · `--fetch-sha` 时补 `fv.sha256` → 安装走强校验（C2 升级）。
 *
 * `fv` 字段契约（后端 `repository/mods.rs::normalize_mod_entry` 消费，字段名严格对齐）：
 *   { version, name, changelog, sha256? }
 * 原始官方字段（mod/download/downloadSize/githubStars/screenshots/descriptionURL）原样保留。
 *
 * 用法：
 *   node scripts/build-mod-index.mjs [选项]
 *
 * 常用选项：
 *   --in <path|url>    官方索引（缺省拉 vcmi-1.7.json；可给本地文件离线复用）
 *   --out <path>       输出（缺省 ./fv-mod-index.json）
 *   --fetch-sha        逐条下载 zip 计算 sha256（慢，产强校验索引；缺省省略 sha256 → 弱校验）
 *   --mod-json-base <url>  当条目缺少 mod.json 绝对 URL 时的兜底前缀（一般不需要）
 *   --concurrency <n>  并发数（缺省 6；sha 下载建议 ≤ 4）
 *   --only <id,id>     只处理指定模组 id（调试 / 增量补录）
 *   --dry              只打印摘要，不写文件
 */
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(__dirname, "..");

const OFFICIAL_INDEX_DEFAULT =
  "https://raw.githubusercontent.com/vcmi/vcmi-mods-repository/develop/vcmi-1.7.json";
const UA = "vcmi-fv-launcher/build-mod-index";

/* ----------------------------- 参数 ----------------------------- */

function parseArgs(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (!a.startsWith("--")) continue;
    const key = a.slice(2);
    const next = argv[i + 1];
    if (next !== undefined && !next.startsWith("--")) {
      out[key] = next;
      i++;
    } else {
      out[key] = true;
    }
  }
  return out;
}

const args = parseArgs(process.argv.slice(2));
const inSrc = String(args.in ?? OFFICIAL_INDEX_DEFAULT);
const outPath = path.resolve(String(args.out ?? path.join(repo, "fv-mod-index.json")));
const fetchSha = args["fetch-sha"] === true;
const modJsonBase = args["mod-json-base"] ? String(args["mod-json-base"]) : "";
const concurrency = Math.max(1, parseInt(String(args.concurrency ?? (fetchSha ? 4 : 6)), 10) || 6);
const only = args.only
  ? String(args.only).split(",").map((s) => s.trim().toLowerCase()).filter(Boolean)
  : null;
const dry = args.dry === true;

/* ----------------------------- 工具 ----------------------------- */

const isUrl = (s) => /^https?:\/\//i.test(s);

/** 带重试与超时的 GET（返回文本）。 */
async function getText(url, { tries = 3, timeoutMs = 30000 } = {}) {
  let lastErr;
  for (let t = 0; t < tries; t++) {
    const ctrl = new AbortController();
    const timer = setTimeout(() => ctrl.abort(), timeoutMs);
    try {
      const resp = await fetch(url, { signal: ctrl.signal, headers: { "user-agent": UA } });
      if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
      return await resp.text();
    } catch (e) {
      lastErr = e;
      await new Promise((r) => setTimeout(r, 400 * (t + 1)));
    } finally {
      clearTimeout(timer);
    }
  }
  throw lastErr instanceof Error ? lastErr : new Error(String(lastErr));
}

/** 流式下载并计算 sha256（不落盘，边下边哈希），返回 {sha256, bytes}。 */
async function streamSha256(url, { timeoutMs = 120000 } = {}) {
  const ctrl = new AbortController();
  const timer = setTimeout(() => ctrl.abort(), timeoutMs);
  try {
    const resp = await fetch(url, { signal: ctrl.signal, headers: { "user-agent": UA } });
    if (!resp.ok) throw new Error(`HTTP ${resp.status}`);
    const h = crypto.createHash("sha256");
    let bytes = 0;
    for await (const chunk of resp.body) {
      const buf = Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk);
      h.update(buf);
      bytes += buf.length;
    }
    return { sha256: h.digest("hex"), bytes };
  } finally {
    clearTimeout(timer);
  }
}

/** 解析索引里的 mod.json URL：绝对直用；相对则拼 modJsonBase。 */
function resolveModJsonUrl(entry) {
  const raw = typeof entry.mod === "string" ? entry.mod.trim() : "";
  if (!raw) return "";
  if (isUrl(raw)) return raw;
  if (modJsonBase) return `${modJsonBase.replace(/\/$/, "")}/${raw.replace(/^\//, "")}`;
  return "";
}

/** 从 mod.json 抽 name / version / changelog（缺失即不写入，绝不猜测）。 */
function pickFvFromModJson(modJson) {
  const out = {};
  const name = typeof modJson.name === "string" ? modJson.name.trim() : "";
  const version = modJson.modVersion ?? modJson.version;
  const versionStr =
    typeof version === "string" ? version.trim() : Array.isArray(version) ? version.join(".") : "";
  if (name) out.name = name;
  if (versionStr) out.version = versionStr;
  if (modJson.changelog && typeof modJson.changelog === "object" && !Array.isArray(modJson.changelog)) {
    out.changelog = modJson.changelog;
  }
  return out;
}

/** 简易并发池：固定 worker 数消费任务队列，单任务异常不中断整体。 */
async function runPool(items, worker, limit) {
  let cursor = 0;
  const errors = [];
  const runners = Array.from({ length: Math.min(limit, items.length) }, async () => {
    while (cursor < items.length) {
      const idx = cursor++;
      try {
        await worker(items[idx], idx);
      } catch (e) {
        errors.push({ idx, err: e instanceof Error ? e.message : String(e) });
      }
    }
  });
  await Promise.all(runners);
  return errors;
}

/* ----------------------------- 主流程 ----------------------------- */

async function loadIndex() {
  if (isUrl(inSrc)) {
    process.stdout.write(`· 拉取官方索引 ${inSrc}\n`);
    return JSON.parse(await getText(inSrc, { timeoutMs: 60000 }));
  }
  const p = path.resolve(inSrc);
  process.stdout.write(`· 读取本地索引 ${p}\n`);
  return JSON.parse(fs.readFileSync(p, "utf8"));
}

async function main() {
  const index = await loadIndex();
  const available = index?.availableMods;
  if (!available || typeof available !== "object") {
    console.error("✗ 索引缺少 availableMods 字段（不是模组仓库索引）");
    process.exit(1);
  }

  let ids = Object.keys(available);
  if (only) ids = ids.filter((id) => only.includes(id.toLowerCase()));

  process.stdout.write(
    `· 待聚合 ${ids.length} 个模组${fetchSha ? "（含 sha256 下载，较慢）" : "（省略 sha256 → 弱校验）"}\n`,
  );

  let done = 0;
  // 聚合过程中的逐条告警（mod.json 拉取失败 / sha 下载失败 / URL 不可解析）。
  // 必须在 runPool 之前声明：worker 闭包会向其 push，否则落入 `const errors = await runPool()` 的 TDZ。
  const warnings = [];
  const poolErrors = await runPool(
    ids,
    async (id) => {
      const entry = available[id];
      const fv = {};

      const mjUrl = resolveModJsonUrl(entry);
      if (mjUrl) {
        try {
          const mj = JSON.parse(await getText(mjUrl));
          Object.assign(fv, pickFvFromModJson(mj));
        } catch (e) {
          warnings.push({ id, err: `mod.json 拉取失败：${e.message}` });
        }
      } else {
        warnings.push({ id, err: "条目缺少可解析的 mod.json URL（跳过 version/changelog）" });
      }

      if (fetchSha && typeof entry.download === "string" && entry.download) {
        try {
          const { sha256 } = await streamSha256(entry.download);
          fv.sha256 = sha256;
        } catch (e) {
          warnings.push({ id, err: `sha256 下载失败：${e.message}` });
        }
      }

      if (Object.keys(fv).length) entry.fv = fv;
      done += 1;
      if (done % 10 === 0 || done === ids.length) {
        process.stdout.write(`  进度 ${done}/${ids.length}\n`);
      }
    },
    concurrency,
  );

  const errors = [...warnings, ...poolErrors.map((e) => ({ id: ids[e.idx], err: e.err }))];

  const versioned = ids.filter((id) => available[id]?.fv?.version).length;
  const withSha = ids.filter((id) => available[id]?.fv?.sha256).length;
  const output = {
    ...index,
    _fvMeta: {
      generatedAt: new Date().toISOString(),
      source: inSrc,
      modCount: Object.keys(available).length,
      versionedCount: versioned,
      sha256Count: withSha,
      validation: withSha > 0 ? "strong" : "weak",
    },
  };

  process.stdout.write(
    `· 完成：${versioned}/${ids.length} 有版本，${withSha} 有 sha256（校验等级 ${withSha > 0 ? "strong" : "weak"}）\n`,
  );
  if (errors.length) {
    process.stdout.write(`· 告警 ${errors.length} 条（不影响聚合，缺字段者降级为弱校验）：\n`);
    for (const e of errors.slice(0, 20)) process.stdout.write(`  - ${e.id ?? e.idx}: ${e.err}\n`);
    if (errors.length > 20) process.stdout.write(`  … 另有 ${errors.length - 20} 条\n`);
  }

  if (dry) {
    process.stdout.write("· --dry：未写文件\n");
    return;
  }
  fs.writeFileSync(outPath, JSON.stringify(output, null, 2) + "\n", "utf8");
  process.stdout.write(`✓ 已写出 ${outPath}\n`);
  process.stdout.write(
    "  下一步：把该文件的 raw URL 填进 设置 → 启动器 → 额外仓库地址，并勾选启用。\n",
  );
}

main().catch((e) => {
  console.error("✗ 生成失败:", e instanceof Error ? e.stack || e.message : String(e));
  process.exit(1);
});
