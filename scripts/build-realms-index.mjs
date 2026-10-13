#!/usr/bin/env node
/**
 * 领域仓库索引生成脚本：读取 `.vcmirealm` 包 → 生成 / 更新 `index.json` 的条目。
 *
 * 会自动从包内 `realm.json` 提取（无需手工抄写）：
 *   id / version / author / name / description / tags / requiresMods(← mods.borrowed)
 * 并计算：
 *   sha256（整体哈希，启动器强制校验）/ size（字节）
 *
 * 版本变更检测（与启动器发布端同口径，见 src-tauri/src/infra/version.rs）：
 *   版本降级 → 跳过该条目并报错（拒绝降低仓库版本；确要回滚请手工改 index.json）；
 *   同版包变 / 升版同包 → 黄色告警（不阻断，索引照常写）。
 *
 * 合并规则（与启动器发布端 merge_repo_index 同口径）：
 *   包内「未声明」（空串 / 空数组 / null）的可推导字段不动索引既有值——手工维护的
 *   author / description 与自定义扩展字段不会被无配置的发布抹掉；`tags` 例外：
 *   键存在即声明（数组含空 = 显式清空，编辑器删光标签可生效），缺键才保留既有值；
 *   `i18n` 按「语言级 + 块内字段级」两级合并（手工加译的语言与块内缺键的手工字段保留）；
 *   `parts` 与 `download` 互斥且完全派生：分片时写 `parts` 并删除 `download`，
 *   未分片时写 `download` 并删除 `parts`（与启动器 `merge_repo_index` 同口径）。
 *
 * 用法：
 *   node scripts/build-realms-index.mjs --in <包文件或目录> [选项]
 *
 * 常用选项：
 *   --in <path>            必填。.vcmirealm 文件，或包含 .vcmirealm 的目录（递归一层）
 *   --index <path>         现有 index.json（缺省 ./index.json；不存在则新建）
 *   --out <path>           输出路径（缺省同 --index）
 *   --download-base <url>  下载 URL 前缀，支持 {tag} 占位
 *                          缺省 https://gitee.com/futureview/vcmi-fv-launcher-realms/releases/download/{tag}/
 *   --tag <tag>            Release 标签，替换 {tag}（缺省用领域 version）
 *   --cover-base <url>     封面 URL 前缀（如 .../raw/master/covers）；给了才写 cover
 *   --covers-dir <dir>     本地封面目录，用于推断扩展名（配合 --cover-base）
 *   --part-size <bytes>    单片上限（缺省 45 MiB，与启动器一致）；超过即切成 <name>.partNNN
 *   --dry                  只打印结果，不写文件
 *
 * 示例：
 *   node scripts/build-realms-index.mjs --in ./dist-realms --tag v1.2.0
 *   node scripts/build-realms-index.mjs --in ./a.vcmirealm --index ../index.json --cover-base https://gitee.com/x/y/raw/master/covers
 */
import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";
import { fileURLToPath } from "node:url";
import yauzl from "yauzl";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repo = path.resolve(__dirname, "..");

/** Gitee 附件单文件上限：超过启动器会在下载时直接放弃 */
const PACKAGE_MAX_BYTES = 100 * 1024 * 1024;
/** 单片上限：与启动器 `REALM_PART_MAX_BYTES` 同值（45 MiB），见 docs/features/realms-sharded-package.zh-CN.md */
const DEFAULT_PART_MAX_BYTES = 45 * 1024 * 1024;
const DEFAULT_DOWNLOAD_BASE =
  "https://gitee.com/futureview/vcmi-fv-launcher-realms/releases/download/{tag}/";

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
const input = args.in ? path.resolve(String(args.in)) : "";
if (!input) {
  console.error("✗ 缺少 --in <包文件或目录>");
  process.exit(1);
}
// 只给了 --out 时，把它同时当作输入索引，避免每次都从空白重建而丢掉手工字段
const fallbackIndex = path.join(repo, "index.json");
const indexPath = path.resolve(String(args.index ?? args.out ?? fallbackIndex));
const outPath = path.resolve(String(args.out ?? indexPath));
const downloadBase = String(args["download-base"] ?? DEFAULT_DOWNLOAD_BASE);
const tag = args.tag ? String(args.tag) : "";
const coverBase = args["cover-base"] ? String(args["cover-base"]) : "";
const coversDir = args["covers-dir"] ? path.resolve(String(args["covers-dir"])) : "";
const dry = args.dry === true;
const partSizeRaw = Number(args["part-size"] ?? DEFAULT_PART_MAX_BYTES);
const partSize =
  Number.isFinite(partSizeRaw) && partSizeRaw > 0
    ? Math.floor(partSizeRaw)
    : DEFAULT_PART_MAX_BYTES;

/* ----------------------------- 工具 ----------------------------- */

/** 本地日期 `YYYY-MM-DD`（用 UTC 会在东八区凌晨前后显示差一天，索引里它是展示字段） */
function todayLocal() {
  const d = new Date();
  const p = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

function fmtSize(n) {
  if (n >= 1024 * 1024 * 1024) return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`;
  if (n >= 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`;
  return `${Math.max(1, Math.round(n / 1024))} KB`;
}

/**
 * 点分版本逐段比较（-1/0/1）。与启动器权威实现 `src-tauri/src/infra/version.rs` 同口径：
 * 去 v 前缀 → 按 `.` 分段 → 每段取前导数字（"2b"→2、无数字→0）→ 逐段比较、缺段补 0；
 * 后缀不参与（`1.0.0-rc1` = `1.0.0`，宁缺勿错）。
 */
function cmpVersion(a, b) {
  const seg = (s) =>
    String(s ?? "")
      .trim()
      .replace(/^[vV]+/, "")
      .split(".")
      .map((p) => {
        const m = p.trim().match(/^\d+/);
        return m ? Number(m[0]) : 0;
      });
  const x = seg(a);
  const y = seg(b);
  for (let i = 0; i < Math.max(x.length, y.length); i++) {
    const l = x[i] ?? 0;
    const r = y[i] ?? 0;
    if (l !== r) return l < r ? -1 : 1;
  }
  return 0;
}

function sha256File(file) {
  return new Promise((resolve, reject) => {
    const h = crypto.createHash("sha256");
    const s = fs.createReadStream(file);
    s.on("data", (c) => h.update(c));
    s.on("end", () => resolve(h.digest("hex")));
    s.on("error", reject);
  });
}

/**
 * 按字节区间切分整包（与启动器 `split_realm_package` 同口径）：`<name>.part001`… 落在源文件同目录。
 * 逐片算 sha256；**数组序即拼接顺序**（启动器下载端按序拼回后校验整包 sha256）。
 */
async function splitPackage(file, size) {
  const dir = path.dirname(file);
  const name = path.basename(file);
  const total = fs.statSync(file).size;
  const count = Math.max(1, Math.ceil(total / size));
  const out = [];
  let idx = 0;
  let ws = null;
  let hasher = null;
  let written = 0;
  const openPart = () => {
    idx += 1;
    const pname = `${name}.part${String(idx).padStart(3, "0")}`;
    ws = fs.createWriteStream(path.join(dir, pname));
    hasher = crypto.createHash("sha256");
    written = 0;
    out.push({ name: pname, size: 0, sha256: "" });
  };
  const closePart = () =>
    new Promise((resolve, reject) => {
      ws.end((err) => {
        if (err) return reject(err);
        out[idx - 1].sha256 = hasher.digest("hex");
        resolve();
      });
    });

  openPart();
  for await (const chunk of fs.createReadStream(file)) {
    let off = 0;
    while (off < chunk.length) {
      const slice = chunk.subarray(off, off + (size - written));
      hasher.update(slice);
      ws.write(slice);
      written += slice.length;
      off += slice.length;
      out[idx - 1].size += slice.length;
      if (written >= size && idx < count) {
        await closePart();
        openPart();
      }
    }
  }
  await closePart();
  return out;
}

function openZip(file) {
  return new Promise((resolve, reject) => {
    yauzl.open(file, { lazyEntries: true, autoClose: true }, (err, zf) => {
      if (err) reject(err);
      else resolve(zf);
    });
  });
}

function readEntry(zf, entry) {
  return new Promise((resolve, reject) => {
    zf.openReadStream(entry, (err, stream) => {
      if (err) return reject(err);
      const chunks = [];
      stream.on("data", (c) => chunks.push(c));
      stream.on("end", () => resolve(Buffer.concat(chunks)));
      stream.on("error", reject);
    });
  });
}

/** 读取包内 realm.json 与 ZIP 注释（启动器要求注释必须为 `vcmirealm`）。 */
async function readRealmPackage(file) {
  const zf = await openZip(file);
  return await new Promise((resolve, reject) => {
    let meta = null;
    zf.on("entry", async (entry) => {
      try {
        if (entry.fileName === "realm.json") {
          const buf = await readEntry(zf, entry);
          meta = JSON.parse(buf.toString("utf8"));
        }
        zf.readEntry();
      } catch (e) {
        reject(e);
      }
    });
    zf.on("end", () => resolve({ meta, comment: zf.comment ?? "" }));
    zf.on("error", reject);
    zf.readEntry();
  });
}

/** 收集待处理的 .vcmirealm 文件 */
function collectPackages(p) {
  if (fs.statSync(p).isFile()) return [p];
  const out = [];
  for (const e of fs.readdirSync(p, { withFileTypes: true })) {
    if (e.isFile() && e.name.toLowerCase().endsWith(".vcmirealm")) {
      out.push(path.join(p, e.name));
    } else if (e.isDirectory()) {
      const sub = path.join(p, e.name);
      for (const f of fs.readdirSync(sub)) {
        if (f.toLowerCase().endsWith(".vcmirealm")) out.push(path.join(sub, f));
      }
    }
  }
  return out.sort();
}

/** 「包内未声明」判定：null / 空串 / 空数组（空对象同判）→ 不写回索引，保留手工值。 */
function blank(v) {
  return (
    v == null ||
    v === "" ||
    (Array.isArray(v) && v.length === 0) ||
    (typeof v === "object" && !Array.isArray(v) && Object.keys(v).length === 0)
  );
}

/**
 * 从 realm.json 重建索引条目的多语言部分（i18n 块结构，与 realm.json 同构）：
 * - 顶层 `name` / `description` / `tags` 视为 `defaultLang` 值（tags 缺键 = 未声明）
 * - `i18n.<lang>` 逐语言覆盖 name / description / tags（编辑器已支持逐语言标签录入）
 * 开发者可在生成的索引里复制某个 `i18n.<lang>` 块即可加译，defaultLang 兜底。
 */
function buildI18n(meta) {
  const defLang = meta?.defaultLang || "chinese";
  const top = {
    name: typeof meta?.name === "string" ? meta.name : "",
    description: typeof meta?.description === "string" ? meta.description : "",
    tags: Array.isArray(meta?.tags) ? meta.tags : undefined,
  };
  const i18n = {};
  if (meta?.i18n && typeof meta.i18n === "object") {
    for (const [lang, v] of Object.entries(meta.i18n)) {
      if (!v) continue;
      const block = {};
      if (typeof v.name === "string" && v.name) block.name = v.name;
      if (typeof v.description === "string" && v.description) block.description = v.description;
      // 标签：数组即声明（含空数组 = 显式清空）；非数组（缺失 / null）不写出
      if (Array.isArray(v.tags)) block.tags = v.tags;
      if (Object.keys(block).length) i18n[lang] = block;
    }
  }
  return {
    defaultLang: defLang,
    name: top.name,
    description: top.description,
    tags: top.tags,
    i18n: Object.keys(i18n).length ? i18n : undefined,
  };
}

/** 推断封面扩展名（配合 --covers-dir，缺省 png） */
function coverExt(id) {
  if (coversDir && fs.existsSync(coversDir)) {
    const hit = fs
      .readdirSync(coversDir)
      .find((f) => f.toLowerCase().startsWith(`${id.toLowerCase()}.`));
    if (hit) return path.extname(hit).slice(1).toLowerCase();
  }
  return "png";
}

/* ----------------------------- 主流程 ----------------------------- */

const packages = collectPackages(input);
if (packages.length === 0) {
  console.error(`✗ 没有找到 .vcmirealm 包：${input}`);
  process.exit(1);
}

// 已有索引作为基底：脚本只覆盖"可从包推导"的字段，手工字段（如 gameEdition）得以保留
let index = { schemaVersion: 1, updatedAt: "", realms: {} };
if (fs.existsSync(indexPath)) {
  try {
    index = JSON.parse(fs.readFileSync(indexPath, "utf8"));
    if (!index.realms || typeof index.realms !== "object") index.realms = {};
  } catch (e) {
    console.error(`✗ 现有索引解析失败：${indexPath}\n  ${e.message}`);
    process.exit(1);
  }
}

const rows = [];
let warned = 0;

for (const file of packages) {
  const name = path.basename(file);
  const size = fs.statSync(file).size;
  const sha256 = await sha256File(file);

  let meta = null;
  let comment = "";
  try {
    const r = await readRealmPackage(file);
    meta = r.meta;
    comment = r.comment;
  } catch (e) {
    console.error(`✗ 跳过 ${name}：读取包失败（${e.message}）`);
    warned++;
    continue;
  }
  if (!meta) {
    console.error(`✗ 跳过 ${name}：包内缺少 realm.json（不是启动器导出的领域包？）`);
    warned++;
    continue;
  }

  const id = String(meta.id ?? "");
  if (!id || id !== id.replace(/[^A-Za-z0-9_-]/g, "")) {
    console.error(`✗ 跳过 ${name}：realm.json 的 id 非法（只允许字母数字与 _ -）："${id}"`);
    warned++;
    continue;
  }
  // 与启动器导入端一致的包格式校验（ZIP 注释魔数）
  if (comment && comment.trim() !== "vcmirealm") {
    console.warn(`⚠ ${name}：ZIP 注释为 "${comment.trim()}"，启动器导入会拒绝（应为 vcmirealm）`);
    warned++;
  }
  // 是否需要分片：包超过单片上限（缺省 45 MiB）→ 切成 <name>.partNNN，索引改走 `parts`
  const sharded = size > partSize;
  if (!sharded && size > PACKAGE_MAX_BYTES) {
    console.warn(`⚠ ${name}：${fmtSize(size)} 超过 100 MB，启动器下载时会放弃（超单片上限的包会自动分片，不受此限）`);
    warned++;
  }
  if (Array.isArray(meta.mods?.bundled) && meta.mods.bundled.length > 0) {
    console.warn(`⚠ ${name}：整合了模组 ${meta.mods.bundled.join("、")}，远程领域不应整合（易超 100 MB）`);
    warned++;
  }

  const version = String(meta.version ?? "1.0.0");
  // 版本变更检测（与启动器发布端同口径）：必须在修改 entry 之前做——
  // `prev` 就是 index.realms[id] 的引用，先改再判会把既有版本读成新版本。
  const prev =
    index.realms[id] && typeof index.realms[id] === "object" ? index.realms[id] : null;
  const prevVersion = typeof prev?.version === "string" ? prev.version : "";
  const prevSha = typeof prev?.sha256 === "string" ? prev.sha256 : "";
  const rel = prevVersion ? cmpVersion(version, prevVersion) : 0;
  if (prevVersion && rel < 0) {
    console.error(
      `✗ 跳过 ${name}：版本降级 v${prevVersion} → v${version}（拒绝降低仓库版本；确要回滚请先手工改 index.json 的 version）`,
    );
    warned++;
    rows.push({ id, version, size, sha256, existed: true, skipped: "版本降级" });
    continue;
  }
  if (prevVersion && rel === 0 && prevSha && prevSha !== sha256) {
    console.warn(`⚠ ${name}：版本号仍是 v${version} 但包内容已变；接收方按版本判定「已安装」，不会提示更新`);
    warned++;
  }
  if (prevVersion && rel > 0 && prevSha && prevSha === sha256) {
    console.warn(`⚠ ${name}：版本号升到 v${version} 但包内容与旧包一致；接收方只能看到版本号变化，实际无更新`);
    warned++;
  }
  const entry = prev ?? {};

  const base = downloadBase.replace("{tag}", tag || version);
  // 「包内没声明」≠「清空仓库」：空值不写回，保留索引里手工维护的值（与启动器同口径）；
  // `tags` 例外：键存在即声明（含空数组 = 显式清空）。本条目已有 version / download /
  // sha256 / size 直接从包更新，不受影响。
  entry.version = version;
  if (!blank(meta.author)) entry.author = meta.author;
  const i18nPart = buildI18n(meta);
  if (i18nPart.name) entry.name = i18nPart.name;
  if (i18nPart.description) entry.description = i18nPart.description;
  if (i18nPart.defaultLang) entry.defaultLang = i18nPart.defaultLang;
  if (i18nPart.i18n !== undefined) {
    // i18n 两级合并：语言级（其余语言 / 手工加译保留）+ 块内字段级（未声明的键保留旧值）；
    // 块内 `tags` 与顶层同口径：数组（含空）即声明
    const oldI18n = entry.i18n && typeof entry.i18n === "object" ? entry.i18n : {};
    const nextI18n = { ...oldI18n };
    for (const [lang, block] of Object.entries(i18nPart.i18n)) {
      const oldBlock =
        oldI18n[lang] && typeof oldI18n[lang] === "object" ? oldI18n[lang] : {};
      const merged = { ...oldBlock };
      for (const [k, v] of Object.entries(block)) {
        if (!blank(v) || (k === "tags" && Array.isArray(v))) merged[k] = v;
      }
      if (Object.keys(merged).length) nextI18n[lang] = merged;
    }
    if (Object.keys(nextI18n).length) entry.i18n = nextI18n;
  }
  // `tags` 键存在即声明：数组（含空数组 = 显式清空）照常写回；缺键（undefined）才保留既有值
  if (Array.isArray(meta.tags)) entry.tags = meta.tags;
  // `parts` 与 `download` 互斥且完全派生（与启动器 `merge_repo_index` 同口径）：
  // 分片条目只写 parts，单包条目只写 download，另一侧一律删除（否则来回切换会残留矛盾字段）。
  const prefix = base.endsWith("/") ? base : `${base}/`;
  if (sharded) {
    const parts = await splitPackage(file, partSize);
    entry.parts = parts.map((p) => ({ url: `${prefix}${p.name}`, size: p.size, sha256: p.sha256 }));
    delete entry.download;
    console.warn(
      `⚠ ${name}：${fmtSize(size)} 超过单片上限 ${fmtSize(partSize)}，已切分为 ${parts.length} 个分片` +
        `（整包仍留在目录里，可自行删除或交由启动器下次发布清理）`,
    );
    warned++;
  } else {
    entry.download = `${prefix}${name}`;
    delete entry.parts;
  }
  // 条目级 sha256 / size 恒为**整包**（拼回后才比对），分片与否语义不变
  entry.sha256 = sha256;
  entry.size = size;
  const reqMods = Array.isArray(meta.mods?.borrowed) ? meta.mods.borrowed : [];
  if (reqMods.length) entry.requiresMods = reqMods;
  if (coverBase) {
    entry.cover = `${coverBase.replace(/\/$/, "")}/${id}.${coverExt(id)}`;
  }
  if (entry.gameEdition === undefined) entry.gameEdition = "any";

  const existed = !!index.realms[id];
  index.realms[id] = entry;
  rows.push({ id, version, size, sha256, existed });
}

index.schemaVersion = 1;
index.updatedAt = todayLocal();

/* ----------------------------- 输出 ----------------------------- */

console.log("");
console.log("生成的索引条目：");
console.log("─".repeat(96));
console.log(
  ["领域 id".padEnd(26), "版本".padEnd(10), "体积".padEnd(11), "sha256".padEnd(18), "状态"].join(" "),
);
console.log("─".repeat(96));
for (const r of rows) {
  console.log(
    [
      r.id.padEnd(26),
      r.version.padEnd(10),
      fmtSize(r.size).padEnd(11),
      `${r.sha256.slice(0, 12)}…`.padEnd(18),
      r.skipped ? `✗ 跳过（${r.skipped}）` : r.existed ? "更新" : "新增",
    ].join(" "),
  );
}
console.log("─".repeat(96));
console.log(`共 ${rows.length} 个条目${warned ? `，${warned} 条警告` : ""}`);

if (dry) {
  console.log("\n--dry：未写入文件。内容预览：\n");
  console.log(JSON.stringify(index, null, 2));
} else {
  fs.mkdirSync(path.dirname(outPath), { recursive: true });
  fs.writeFileSync(outPath, `${JSON.stringify(index, null, 2)}\n`, "utf8");
  console.log(`\n✓ 已写入：${outPath}`);
  console.log("  推送到 Gitee 后最长 60 秒生效；启动器内可点「↻ 刷新」立即拉取。");
}
