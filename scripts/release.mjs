#!/usr/bin/env node
/**
 * VCMI-FV 启动器发行工具（**唯一入口**，含产物提取）。
 *
 * 设计依据：`docs/guides/release.zh-CN.md` + `docs/decisions/0007-version-injection-at-pack.zh-CN.md`
 *
 *   pack     本地发包：**写入版本**（ADR 0007）→ 构建 → 提取产物 → 生成随包预置补丁 → 写发行目录与条目片段
 *            ⚠️ **不改** `releases/index.json` 的版本条目（索引只写「已确认上传成功」的版本）
 *            `--version <semver>` 定版并注入 Cargo.toml + tauri.conf.json；`--temp-version` 改为构建后还原；
 *            `--test` = 一键测试包（临时注入 + 不进更新链 + 不出安装包 + 自动 `<现版本>-dev.<日期>`）
 *   publish  发布到 GitHub Releases（建 tag/release → 流式传资产 → 用 API 结果更新索引）
 *   preview  本地预演：生成**派生索引** releases/index.local.json（下载地址改写到本地静态服务）
 *            —— 用于在 publish 之前跑通「发现更新 → 下载 → 应用」；⚠️ 永不写 releases/index.json
 *   verify   索引契约自检（结构 / 投影一致性 / 预置一致性 / ChangeLog 一致性 / --online 可达性）
 *   status   索引版本 vs 本地已打包目录
 *
 * 关键不变式（勿破坏）：
 *   1. **索引只由 publish 追加版本条目**；pack 最多自举 repo/download/preset 配置区块。
 *   2. `versions[]` 是 `releases[]` 的**投影**（生成物，勿手改）；`releases[].notes` 派生自 `ChangeLog.md`。
 *   3. **契约版本必须等于 exe 编译版本**（`Cargo.toml`）；`--version` 只是发行标识（tag / Release title）。
 *      两者不一致时只有 `--no-update-chain`（不进更新链）一条出路。
 *   4. token 只从环境变量 / `.env.local` / `--token-file` 读，**绝不落盘**，输出一律脱敏。
 *   5. 便携 zip **只含程序文件**（exe + 组件 + 预置补丁），绝不含 `.vcmi-fv-launcher/` 下的用户数据。
 *
 * 用法见 `node scripts/release.mjs --help`。
 */

import crypto from "node:crypto";
import fs from "node:fs";
import http from "node:http";
import https from "node:https";
import path from "node:path";
import zlib from "node:zlib";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, "..");

/* ────────────────────────────── 常量 ────────────────────────────── */

const EXE_NAME = "vcmi-fv-launcher.exe";
const PRODUCT = "VCMI-FV-Launcher";
/** 预置补丁在包内的相对路径（必须与 `src-tauri/src/updater/config_patch.rs::PATCH_REL` 一致） */
const PATCH_REL = ".vcmi-fv-launcher/launcher.patch.json";
/** 随包预置补丁的源目录（生成物但**提交进 git**：Tauri `bundle.resources` 构建期要读） */
const PRESET_DIR = path.join(ROOT, "src-tauri", "resources", "release-preset");
const INDEX_REL = "releases/index.json";
const INDEX_PATH = path.join(ROOT, "releases", "index.json");
const LATEST_PATH = path.join(ROOT, "releases", "latest.json");
/** 本地预演的**派生索引**（永不进 git：`.gitignore` 已忽略 `releases/*` 中除 index.json 外的文件） */
const LOCAL_INDEX_REL = "releases/index.local.json";
const LOCAL_INDEX_PATH = path.join(ROOT, "releases", "index.local.json");
const DEFAULT_PREVIEW_PORT = 8765;
const DEFAULT_CHANGELOG = path.join(ROOT, "ChangeLog.md");
const SCHEMA = "vcmi-fv-launcher-release-index/1";
const GH_API_VERSION = "2022-11-28";
/** 安装包"本次构建产出"判定容差：同一次构建里 bundle 产物与 exe 的 mtime 会差几十秒 */
const BUNDLE_MTIME_TOLERANCE_MS = 5 * 60 * 1000;
/** 资产名 / tag / 发行标识的字符集（GitHub 对非 ASCII 资产名与 URL 编码极不友好） */
const NAME_RE = /^[A-Za-z0-9._-]+$/;
const ASCII_RE = /^[\x20-\x7E]+$/;
/** ChangeLog 语言块别名（其余一律用 VCMI 语言标识） */
const LANG_ALIAS = { 中文: "chinese", 英文: "english", 繁体中文: "tchinese", 繁體中文: "tchinese" };

/* ────────────────────────────── 日志与基础工具 ────────────────────────────── */

function log(msg) {
  console.log(`[release] ${msg}`);
}
function warn(msg) {
  console.warn(`[release] ⚠️ ${msg}`);
}
function die(msg) {
  throw new Error(msg);
}
function humanSize(n) {
  if (!n || n <= 0) return "0";
  if (n < 1024 * 1024) return `${Math.max(1, Math.round(n / 1024))} KB`;
  return `${(n / 1024 / 1024).toFixed(1)} MB`;
}
function todayLocal() {
  const d = new Date();
  const p = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}
function nowIso() {
  return new Date().toISOString();
}
function abs(p) {
  return path.isAbsolute(p) ? p : path.resolve(ROOT, p);
}

/* ────────────────────────────── 版本比较 ────────────────────────────── */

/**
 * 点分版本逐段比较（-1/0/1）。与 `src-tauri/src/infra/version.rs` 同口径（去 v 前缀 → 按 `.` 分段 →
 * 每段取前导数字 → 缺段补 0；后缀不参与），保证脚本判定与启动器一致。
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

/* ────────────────────────────── 哈希 / JSON ────────────────────────────── */

function sha256File(p) {
  const buf = fs.readFileSync(p);
  return { hash: crypto.createHash("sha256").update(buf).digest("hex"), size: buf.length };
}

function readJson(p, what) {
  try {
    return JSON.parse(fs.readFileSync(p, "utf8"));
  } catch (e) {
    die(`${what} 解析失败（不覆盖既有文件）：${e.message}`);
  }
}

/** 原子写：临时文件 + rename（避免半截 JSON） */
function writeJsonAtomic(p, obj) {
  fs.mkdirSync(path.dirname(p), { recursive: true });
  const tmp = `${p}.tmp-${process.pid}`;
  fs.writeFileSync(tmp, `${JSON.stringify(obj, null, 2)}\n`, "utf8");
  fs.renameSync(tmp, p);
}

/** 只在内容变化时写（保持 git diff 干净） */
function writeJsonIfChanged(p, obj) {
  const next = `${JSON.stringify(obj, null, 2)}\n`;
  if (fs.existsSync(p) && fs.readFileSync(p, "utf8") === next) return false;
  writeJsonAtomic(p, obj);
  return true;
}

/* ────────────────────────────── 手写 ZIP（零依赖；原 package-release.mjs） ────────────────────────────── */

const CRC_TABLE = (() => {
  const t = new Uint32Array(256);
  for (let i = 0; i < 256; i++) {
    let c = i;
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    t[i] = c >>> 0;
  }
  return t;
})();

function crc32(buf) {
  let c = 0xffffffff;
  for (let i = 0; i < buf.length; i++) c = CRC_TABLE[(c ^ buf[i]) & 0xff] ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

/** entries: [{ name, data }]（name 用 `/` 分隔）。小文件自动退化为 store。 */
function buildZip(entries) {
  const parts = [];
  const central = [];
  let offset = 0;
  for (const e of entries) {
    const nameBuf = Buffer.from(e.name, "utf8");
    const crc = crc32(e.data);
    const deflated = zlib.deflateRawSync(e.data, { level: 9 });
    const useDeflate = deflated.length < e.data.length;
    const payload = useDeflate ? deflated : e.data;
    const method = useDeflate ? 8 : 0;

    const lfh = Buffer.alloc(30);
    lfh.writeUInt32LE(0x04034b50, 0);
    lfh.writeUInt16LE(20, 4);
    lfh.writeUInt16LE(0, 6);
    lfh.writeUInt16LE(method, 8);
    lfh.writeUInt16LE(0, 10);
    lfh.writeUInt16LE(0x21, 12); // 固定日期 1980-01-01
    lfh.writeUInt32LE(crc, 14);
    lfh.writeUInt32LE(payload.length, 18);
    lfh.writeUInt32LE(e.data.length, 22);
    lfh.writeUInt16LE(nameBuf.length, 26);
    lfh.writeUInt16LE(0, 28);
    parts.push(lfh, nameBuf, payload);

    const cd = Buffer.alloc(46);
    cd.writeUInt32LE(0x02014b50, 0);
    cd.writeUInt16LE(20, 4);
    cd.writeUInt16LE(20, 6);
    cd.writeUInt16LE(0, 8);
    cd.writeUInt16LE(method, 10);
    cd.writeUInt16LE(0, 12);
    cd.writeUInt16LE(0x21, 14);
    cd.writeUInt32LE(crc, 16);
    cd.writeUInt32LE(payload.length, 20);
    cd.writeUInt32LE(e.data.length, 24);
    cd.writeUInt16LE(nameBuf.length, 28);
    cd.writeUInt16LE(0, 30);
    cd.writeUInt16LE(0, 32);
    cd.writeUInt16LE(0, 34);
    cd.writeUInt16LE(0, 36);
    cd.writeUInt32LE(0, 38);
    cd.writeUInt32LE(offset, 42);
    central.push(cd, nameBuf);

    offset += lfh.length + nameBuf.length + payload.length;
  }
  const cdBuf = Buffer.concat(central);
  const eocd = Buffer.alloc(22);
  eocd.writeUInt32LE(0x06054b50, 0);
  eocd.writeUInt16LE(entries.length, 8);
  eocd.writeUInt16LE(entries.length, 10);
  eocd.writeUInt32LE(cdBuf.length, 12);
  eocd.writeUInt32LE(offset, 16);
  // ⚠️ 中央目录必须一起拼进来（曾漏掉 cdBuf → 文件比结构与声明短一截，任何解压器都判"损坏"，
  //    而体积/sha256 看起来完全正常，极难发现）；拼完立刻做结构自检。
  const buf = Buffer.concat([...parts, cdBuf, eocd]);
  assertZipStructurallyValid(buf, entries.length);
  return buf;
}

/**
 * ZIP 结构自检（防"能算哈希、不能解压"的坏包静默出厂）：
 *   1. 末尾 22 字节必须是 EOCD；2. `cdOffset + cdSize + 22 === 文件长度`；
 *   3. 条目数与中央目录每一项都能在其 `localHeaderOffset` 处看到本地头签名 `PK\x03\x04`。
 */
function assertZipStructurallyValid(buf, expectedEntries) {
  const len = buf.length;
  const eocdOff = len - 22;
  if (eocdOff < 0 || buf.readUInt32LE(eocdOff) !== 0x06054b50) {
    die("ZIP 自检失败：末尾找不到 EOCD 签名（写入器有 bug）");
  }
  const count = buf.readUInt16LE(eocdOff + 10);
  const cdSize = buf.readUInt32LE(eocdOff + 12);
  const cdOff = buf.readUInt32LE(eocdOff + 16);
  if (count !== expectedEntries) {
    die(`ZIP 自检失败：中央目录条目数 ${count} ≠ 期望 ${expectedEntries}`);
  }
  if (cdOff + cdSize + 22 !== len) {
    die(
      `ZIP 自检失败：cdOffset(${cdOff}) + cdSize(${cdSize}) + 22 ≠ 文件长度(${len})` +
        `（差 ${len - (cdOff + cdSize + 22)} 字节 —— 中央目录很可能没写进文件）`,
    );
  }
  let p = cdOff;
  for (let i = 0; i < count; i++) {
    if (buf.readUInt32LE(p) !== 0x02014b50) die(`ZIP 自检失败：第 ${i + 1} 个中央目录项签名错误`);
    const nameLen = buf.readUInt16LE(p + 28);
    const extraLen = buf.readUInt16LE(p + 30);
    const commentLen = buf.readUInt16LE(p + 32);
    const localOff = buf.readUInt32LE(p + 42);
    if (buf.readUInt32LE(localOff) !== 0x04034b50) {
      die(`ZIP 自检失败：第 ${i + 1} 个条目的本地头偏移 ${localOff} 处没有本地头签名`);
    }
    p += 46 + nameLen + extraLen + commentLen;
  }
  if (p !== cdOff + cdSize) die(`ZIP 自检失败：中央目录实际长度 ${p - cdOff} ≠ 声明 ${cdSize}`);
}

/* ────────────────────────────── 文件遍历 ────────────────────────────── */

function walk(dir, base = dir) {
  const out = [];
  if (!fs.existsSync(dir)) return out;
  // ⚠️ 必须排序：`fs.readdirSync` 的顺序不保证（不同机器/文件系统可能不同），
  //    而 zip 的条目顺序会影响最终字节 ⇒ CI 与本地产出的包必须顺序一致。
  const entries = fs.readdirSync(dir, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0));
  for (const e of entries) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) out.push(...walk(p, base));
    else out.push({ abs: p, rel: path.relative(base, p).split(path.sep).join("/") });
  }
  return out;
}

function firstMatch(dir, exts) {
  if (!fs.existsSync(dir)) return null;
  for (const f of fs.readdirSync(dir)) {
    if (exts.some((e) => f.toLowerCase().endsWith(e))) return path.join(dir, f);
  }
  return null;
}

function readCargoVersion() {
  const cargo = fs.readFileSync(path.join(ROOT, "src-tauri", "Cargo.toml"), "utf8");
  const afterPkg = cargo.split(/^\s*\[package\s*\]/m)[1] ?? cargo;
  const m = afterPkg.match(/^\s*version\s*=\s*"([^"]+)"/m);
  if (!m) die("无法从 src-tauri/Cargo.toml 解析 [package].version");
  return m[1];
}

function git(args, { quiet = true } = {}) {
  const r = spawnSync("git", args, { cwd: ROOT, encoding: "utf8", shell: false });
  if (r.status !== 0) {
    if (quiet) return "";
    die(`git ${args.join(" ")} 失败：${(r.stderr || "").trim()}`);
  }
  return (r.stdout || "").trim();
}

/* ────────────────────────────── 索引 ────────────────────────────── */

function loadIndex() {
  if (!fs.existsSync(INDEX_PATH)) return null;
  const v = readJson(INDEX_PATH, `索引 ${INDEX_REL}`);
  if (v.schemaVersion !== 1 || typeof v.schema !== "string") {
    die(`索引 schema 不认识（${v.schema} / v${v.schemaVersion}）：请升级发行脚本`);
  }
  if (!Array.isArray(v.releases)) v.releases = [];
  if (!Array.isArray(v.versions)) v.versions = [];
  return v;
}

/** 解析 owner/repo/branch/provider：CLI > 索引 repo > git remote > 报错（含操作指引） */
function resolveRepo(opts, index) {
  const r = index?.repo ?? {};
  const provider = (opts.provider || r.provider || "github").toLowerCase();
  let owner = opts.owner || r.owner || "";
  let name = opts.repo || r.name || "";
  let branch = opts.branch || r.defaultBranch || "";
  if (!owner || !name || !branch) {
    const url = git(["remote", "get-url", "origin"]);
    const m = url && url.match(/github\.com[:/]([^/]+)\/([^/]+?)(?:\.git)?$/i);
    if (m) {
      owner = owner || m[1];
      name = name || m[2];
      branch = branch || git(["rev-parse", "--abbrev-ref", "HEAD"]) || "master";
    }
  }
  if (!owner || !name) {
    die(
      "无法确定 GitHub 仓库（owner/repo）。请任选其一：\n" +
        "  ① 传参：--owner <账号> --repo <仓库名> [--branch master]\n" +
        "  ② 配置远端：git remote add origin https://github.com/<账号>/<仓库名>.git\n" +
        "  ③ 在 releases/index.json 顶部 repo 区块里写死（首次自举后即固化）",
    );
  }
  if (!branch) branch = "master";
  const host = provider === "gitee" ? "gitee.com" : "github.com";
  const rawBase =
    provider === "gitee"
      ? `https://gitee.com/${owner}/${name}/raw/${branch}/`
      : `https://raw.githubusercontent.com/${owner}/${name}/${branch}/`;
  return {
    provider,
    owner,
    name,
    defaultBranch: branch,
    host,
    releaseBase: `https://${host}/${owner}/${name}/releases`,
    indexUrl: `${rawBase}${INDEX_REL}`,
  };
}

function resolveDownload(index, repo) {
  const d = index?.download ?? {};
  // ⚠️ 索引里的 download.base 只在「与解析出的仓库是同一个」时采用：
  //    占位符/换仓库的场景必须回落到新仓库的 releaseBase，否则条目 URL 会指向旧地址。
  const sameRepo =
    (!index?.repo?.owner || index.repo.owner === repo.owner) &&
    (!index?.repo?.name || index.repo.name === repo.name);
  const base = String((sameRepo && d.base) || repo.releaseBase).replace(/\/+$/, "");
  const tpl = String(d.pathTemplate || "{base}/download/{tag}/{name}");
  if (!tpl.includes("{name}")) die("download.pathTemplate 必须含 {name} 占位符");
  if (!base.startsWith("https://")) die(`download.base 必须是 https 地址，收到：${base}`);
  const sample = assetUrl({ base, tpl }, "v0.0.0", "x.zip", "0.0.0");
  if (!sample.startsWith("https://")) die(`download.pathTemplate 展开后不是 https：${sample}`);
  return { base, tpl };
}

function assetUrl(dl, tag, name, release, dir = "") {
  return dl.tpl
    .replace(/\{base\}/g, dl.base)
    .replace(/\{tag\}/g, tag)
    .replace(/\{name\}/g, encodeURIComponent(name))
    .replace(/\{release\}/g, release)
    // {dir} = pack 目录名：本地预演用（静态服务以 releases/ 为根，资产在 `/<dir>/<name>`）
    .replace(/\{dir\}/g, dir);
}

/** GitHub 原生相对路径（相对 releaseBase）—— 即「提取 /releases/<path>」 */
function nativePath(tag, name) {
  return `download/${tag}/${name}`;
}

function deepMerge(base, over) {
  if (over === null || over === undefined) return base;
  if (Array.isArray(over) || typeof over !== "object") return over;
  const out = { ...(base && typeof base === "object" && !Array.isArray(base) ? base : {}) };
  for (const [k, v] of Object.entries(over)) out[k] = deepMerge(out[k], v);
  return out;
}

/**
 * 预置策略：缺省 = `{ base: { update.manifest_url: <repo.indexUrl> }, flavors: { ai: … } }`。
 * ⚠️ `manifest_url` 缺省（或仍等于索引自己声明的 indexUrl，即自举产物）→ 跟随解析出的仓库；
 *    开发者显式改成别的地址（如自建 CDN 索引）→ **原样尊重**。
 */
function resolvePreset(index, repo) {
  const p = index?.preset ?? {};
  const base = deepMerge({}, p.base ?? {});
  const oldUrl = index?.repo?.indexUrl;
  const auto = !base.update?.manifest_url || (oldUrl && base.update.manifest_url === oldUrl);
  if (auto) base.update = { ...(base.update ?? {}), manifest_url: repo.indexUrl };
  const flavors = p.flavors && typeof p.flavors === "object" ? p.flavors : {};
  return { base, flavors };
}

function presetForFlavor(preset, flavor) {
  const over = flavor && preset.flavors[flavor] ? preset.flavors[flavor] : {};
  return deepMerge(preset.base, over);
}

function presetFileFor(flavor) {
  return path.join(PRESET_DIR, flavor ? `launcher.patch.${flavor}.json` : "launcher.patch.json");
}

/** 本次构建实际使用的预置文件：该 flavor 未声明时退化为 base（不因此让构建失败） */
function activePresetFile(flavor) {
  const f = presetFileFor(flavor);
  return fs.existsSync(f) ? f : presetFileFor("");
}

/** 生成随包预置补丁：**索引里声明的 flavor 全部刷新**（内容不变则不写）；返回变化清单 */
function writePresetFiles(preset) {
  fs.mkdirSync(PRESET_DIR, { recursive: true });
  const changed = [];
  const want = new Map([["", preset.base]]);
  for (const f of Object.keys(preset.flavors)) want.set(f, presetForFlavor(preset, f));
  for (const [f, obj] of want) {
    if (writeJsonIfChanged(presetFileFor(f), obj)) {
      changed.push(f ? `launcher.patch.${f}.json` : "launcher.patch.json");
    }
  }
  // 索引已不再声明的 flavor → 删掉残留文件（生成物；避免陈旧预置被构建读走）
  for (const name of fs.readdirSync(PRESET_DIR)) {
    const m = name.match(/^launcher\.patch\.(.+)\.json$/);
    if (m && !want.has(m[1])) {
      fs.rmSync(path.join(PRESET_DIR, name), { force: true });
      changed.push(`删除 ${name}（索引未声明 flavors.${m[1]}）`);
    }
  }
  return changed;
}

/** 自举：补齐 repo / download / preset 配置区块（**绝不写版本条目**） */
function bootstrapIndex(index, repo, dl, preset) {
  const before = JSON.stringify(index);
  index.schema = index.schema ?? SCHEMA;
  index.schemaVersion = index.schemaVersion ?? 1;
  index.product = index.product ?? PRODUCT;
  index.channel = index.channel ?? "stable";
  index.latest = index.latest ?? null;
  index.minSupported = index.minSupported ?? null;
  index.repo = {
    provider: repo.provider,
    owner: repo.owner,
    name: repo.name,
    defaultBranch: repo.defaultBranch,
    releaseBase: repo.releaseBase,
    indexUrl: repo.indexUrl,
  };
  index.download = {
    base: dl.base,
    pathTemplate: dl.tpl,
    note: "换自建源 / CDN / Gitee 只改这里；{base} {tag} {name} {release} 为占位符",
  };
  index.preset = {
    base: preset.base,
    ...(Object.keys(preset.flavors).length ? { flavors: preset.flavors } : {}),
  };
  // ⚠️ `updatedAt` 只在**内容真的变了**时刷新：否则每天跑一次 pack 都会产生一个无意义的
  //    "只改了日期"的 diff（用户会以为索引变了）
  const changed = before !== JSON.stringify(index);
  if (changed) index.updatedAt = todayLocal();
  return changed;
}

/** `versions[]` 投影（生成物；§4.4） */
function projectVersions(index) {
  const rows = (index.releases ?? []).filter((r) => r && r.updateVersion);
  const seen = new Set();
  return rows
    .map((r) => {
      const a = (r.assets ?? []).find((x) => x.role === "update");
      if (!a) die(`发行 ${r.release} 缺少 role=update 的资产，无法投影`);
      if (seen.has(r.updateVersion)) die(`契约版本 ${r.updateVersion} 在 releases[] 中重复（同一版本只能有一条进更新链）`);
      seen.add(r.updateVersion);
      return {
        version: r.updateVersion,
        download: a.url,
        sha256: a.sha256,
        size: a.size,
        publishedAt: r.publishedAt || "",
        critical: !!r.critical,
        format: String(a.name).toLowerCase().endsWith(".zip") ? "zip" : "exe",
        kind: "full",
        notes: { ...(r.notes ?? {}) },
      };
    })
    .sort((a, b) => cmpVersion(b.version, a.version));
}

function refreshProjection(index) {
  index.versions = projectVersions(index);
  index.latest = index.versions.length ? index.versions[0].version : null;
  index.updatedAt = todayLocal();
  return index;
}

/* ────────────────────────────── ChangeLog 解析 ────────────────────────────── */

function normalizeLang(token) {
  const t = String(token).trim();
  if (LANG_ALIAS[t]) return LANG_ALIAS[t];
  if (/^[A-Za-z][A-Za-z0-9_-]*$/.test(t)) return t.toLowerCase();
  return null;
}

function normalizeVersionToken(token) {
  let t = String(token).trim();
  const br = t.match(/^\[(.+)\]$/);
  if (br) t = br[1].trim();
  t = t.split(/\s+/)[0]; // 去掉 `- 2026-09-30` 之类的日期尾巴
  t = t.replace(/^[vV]+/, "");
  return t;
}

/**
 * 解析 ChangeLog（严格语法，§5.6.3）：
 *   `## <版本>` 节 → `### <lang>` 块 → `- ` 条目
 * 版本节内出现其它非空行 / 语言块内出现非列表行 → **报错**（避免「写了却没生效」）。
 * 返回 { sections: Map<version, { langs, raw, order }> }
 */
function parseChangeLog(text, fromFile) {
  const lines = String(text).replace(/\r\n?/g, "\n").split("\n");
  const sections = new Map();
  let cur = null; // { version, langs, rawLines }
  let curLang = null;
  let curRaw = null;

  const where = (i) => `${fromFile}:${i + 1}`;

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i];
    const h2 = line.match(/^##\s+(.+?)\s*$/);
    const h3 = line.match(/^###\s+(.+?)\s*$/);
    if (h2) {
      cur = { version: normalizeVersionToken(h2[1]), langs: {}, rawLines: [], order: sections.size };
      if (!cur.version) die(`${where(i)}：版本节标题为空`);
      if (sections.has(cur.version)) die(`${where(i)}：版本节 ${cur.version} 重复`);
      sections.set(cur.version, cur);
      curLang = null;
      curRaw = null;
      continue;
    }
    if (h3) {
      if (!cur) die(`${where(i)}：语言块出现在任何 "## <版本>" 节之前`);
      const lang = normalizeLang(h3[1]);
      if (!lang) die(`${where(i)}：语言标识非法 "${h3[1]}"（用 VCMI 标识，如 chinese / english）`);
      if (cur.langs[lang]) die(`${where(i)}：版本 ${cur.version} 内语言块 ${lang} 重复`);
      cur.langs[lang] = [];
      curLang = lang;
      cur.rawLines.push(line);
      continue;
    }
    if (!cur) continue; // 文件头部（标题 / 说明）不属任何版本节，忽略
    if (!line.trim()) {
      if (curLang) cur.langs[curLang].push(""); // 占位，稍后过滤
      cur.rawLines.push(line);
      continue;
    }
    if (!curLang) {
      die(
        `${where(i)}：版本 ${cur.version} 节内出现非语言块内容："${line.trim().slice(0, 40)}"\n` +
          "  版本节只允许 `### <lang>` 块与其内的列表项；自由文本/表格请用 --body-file。",
      );
    }
    const item = line.match(/^\s*(?:[-*]|\d+[.)])\s+(.+?)\s*$/);
    if (!item) {
      die(`${where(i)}：语言块 ${curLang} 内出现非列表行："${line.trim().slice(0, 40)}"（列表项请用 "- " 开头）`);
    }
    cur.langs[curLang].push(item[1]);
    cur.rawLines.push(line);
  }
  for (const s of sections.values()) {
    for (const [lang, arr] of Object.entries(s.langs)) {
      const clean = arr.map((x) => x.trim()).filter(Boolean);
      if (!clean.length) delete s.langs[lang];
      else s.langs[lang] = clean;
    }
    s.raw = s.rawLines.join("\n").trim();
    delete s.rawLines;
  }
  return { sections };
}

/** 取某版本的说明：先按发行标识，再按契约版本 */
function pickSection(changelog, release, contractVersion) {
  for (const key of [release, contractVersion]) {
    if (key && changelog.sections.has(String(key))) return changelog.sections.get(String(key));
  }
  return null;
}

function splitNotes(text) {
  return String(text)
    .split(";")
    .map((x) => x.trim())
    .filter(Boolean);
}

/**
 * 解析更新说明（§5.6.4 优先级）：
 *   1. --notes（最高优先级；语言由 --notes-lang 指定）
 *   2. --notes-file（.md 走本解析器；.json 直接给 map）
 *   3. ChangeLog.md 对应版本节
 *   4. 都没有 → 报错（--allow-empty-notes 放行）
 */
function resolveNotes(opts, ctx) {
  // `ctx.allowMissing`：不进更新链的包（测试/预发行）缺说明只告警 —— 它们不会出现在更新弹窗里
  const empty = opts.allowEmptyNotes || !!ctx.allowMissing;
  if (opts.notes) {
    const notes = { [opts.notesLang]: splitNotes(opts.notes) };
    if (!notes[opts.notesLang].length && !empty) die("--notes 内容为空");
    return { notes, source: "cli", section: null };
  }
  if (opts.notesFile) {
    const p = abs(opts.notesFile);
    if (!fs.existsSync(p)) die(`--notes-file 不存在：${opts.notesFile}`);
    const text = fs.readFileSync(p, "utf8");
    if (/\.json$/i.test(p)) {
      const v = readJson(p, "--notes-file(JSON)");
      const notes = {};
      for (const [lang, arr] of Object.entries(v)) {
        if (!Array.isArray(arr)) die(`--notes-file 的 ${lang} 必须是字符串数组`);
        const clean = arr.map((x) => String(x).trim()).filter(Boolean);
        if (clean.length) notes[normalizeLang(lang) ?? lang.toLowerCase()] = clean;
      }
      if (!Object.keys(notes).length && !empty) die("--notes-file(JSON) 里没有任何条目");
      return { notes, source: "file", section: null };
    }
    const cl = parseChangeLog(text, opts.notesFile);
    const sec = pickSection(cl, ctx.release, ctx.contractVersion);
    if (!sec || !Object.keys(sec.langs).length) {
      if (empty) return { notes: {}, source: "file", section: null };
      die(`--notes-file 里没有 ${ctx.release}（或 ${ctx.contractVersion}）的说明节`);
    }
    return { notes: sec.langs, source: "file", section: sec };
  }
  const clPath = opts.changelog ? abs(opts.changelog) : DEFAULT_CHANGELOG;
  if (fs.existsSync(clPath)) {
    const sec = pickSection(parseChangeLog(fs.readFileSync(clPath, "utf8"), path.relative(ROOT, clPath)), ctx.release, ctx.contractVersion);
    if (sec && Object.keys(sec.langs).length) {
      return { notes: sec.langs, source: "changelog", section: sec };
    }
  }
  if (empty) return { notes: {}, source: "changelog", section: null };
  die(
    `没有可用的更新说明：请任选其一\n` +
      `  ① 在 ${path.relative(ROOT, clPath)} 里加一节 "## v${ctx.release}"（推荐）\n` +
      `  ② --notes "第一条;第二条"（--notes-lang 指定语言，缺省 chinese）\n` +
      `  ③ --notes-file <说明.md|说明.json>\n` +
      `  ④ --allow-empty-notes（不推荐：更新弹窗里将没有任何说明）`,
  );
}

function buildBody(opts, notes, section) {
  if (opts.bodyFile) {
    const p = abs(opts.bodyFile);
    if (!fs.existsSync(p)) die(`--body-file 不存在：${opts.bodyFile}`);
    return fs.readFileSync(p, "utf8").trim();
  }
  if (opts.bodyLang) {
    const lang = normalizeLang(opts.bodyLang) ?? opts.bodyLang;
    const lines = notes[lang];
    if (!lines) die(`--body-lang ${opts.bodyLang} 在说明里不存在（可用：${Object.keys(notes).join(" / ") || "无"}）`);
    return `### ${lang}\n\n${lines.map((x) => `- ${x}`).join("\n")}`;
  }
  if (section?.raw) return section.raw;
  const langs = Object.keys(notes);
  if (!langs.length) return "";
  return langs.map((lang) => `### ${lang}\n\n${notes[lang].map((x) => `- ${x}`).join("\n")}`).join("\n\n");
}

/* ────────────────────────────── 产物提取（原 package-release.mjs） ────────────────────────────── */

function stamp() {
  const d = new Date();
  const p = (n) => String(n).padStart(2, "0");
  return `${d.getFullYear()}${p(d.getMonth() + 1)}${p(d.getDate())}-${p(d.getHours())}`;
}

/**
 * 把 `src-tauri/target/<profile>` 的产物整理到 `releases/<version>_<stamp>[_flavor]/`。
 * ⚠️ 便携 zip **只含程序文件**（exe + 组件 + 预置补丁），绝不含 `.vcmi-fv-launcher/` 下的用户数据。
 * ⚠️ 安装包只在"本次构建产出"（mtime ≥ exe − 5min）时才复制：bundle 失败/被跳过不会报错，
 *    旧包进新目录（时间戳新、内容旧）比缺文件更危险。
 */
function extractArtifacts({ profile, flavor, zip }) {
  const profileDir = path.join(ROOT, "src-tauri", "target", profile);
  const exe = path.join(profileDir, EXE_NAME);
  if (!fs.existsSync(exe)) {
    die(
      `未找到 ${path.relative(ROOT, exe)}（profile=${profile}）。\n` +
        `  请先构建，或去掉 --no-build（pack 缺省会自己跑构建）。`,
    );
  }
  const version = readCargoVersion();
  const ts = stamp();
  const dirName = `${version}_${ts}${flavor ? `_${flavor}` : ""}`;
  const outDir = path.join(ROOT, "releases", dirName);
  fs.rmSync(outDir, { recursive: true, force: true });
  fs.mkdirSync(outDir, { recursive: true });

  const artifacts = {};
  const warnings = [];
  const warnPkg = (msg) => {
    warnings.push(msg);
    warn(msg);
  };

  // 0) 外挂组件（仅 --flavor ai；源 `target/<profile>/.vcmi-fv-launcher/runtime/<id>/**`）
  const componentFiles = [];
  let componentBytes = 0;
  if (flavor === "ai") {
    const rtRoot = path.join(profileDir, ".vcmi-fv-launcher", "runtime");
    if (!fs.existsSync(rtRoot)) {
      die(
        `内置 AI 版缺少组件：${path.relative(ROOT, rtRoot)} 不存在。\n` +
          `  请用 \`npm run release:pack -- --flavor ai\`（会自动带 -c tauri.ai-runtime.conf.json 完整构建）。`,
      );
    }
    for (const e of fs.readdirSync(rtRoot, { withFileTypes: true })) {
      if (!e.isDirectory()) continue;
      for (const f of walk(path.join(rtRoot, e.name))) {
        componentFiles.push({ name: `.vcmi-fv-launcher/runtime/${e.name}/${f.rel}`, abs: f.abs });
        componentBytes += fs.statSync(f.abs).size;
      }
    }
  }
  // 0b) 随包预置补丁（**所有** flavor 都带；由索引 preset 生成）
  const presetPath = activePresetFile(flavor || "");
  if (!fs.existsSync(presetPath)) {
    die(`缺少随包预置补丁 ${path.relative(ROOT, presetPath)}（应已由 pack 生成；请勿手工删除）`);
  }
  const presetFiles = [{ name: PATCH_REL, abs: presetPath }];

  // 1) 主程序
  const exeDest = path.join(outDir, EXE_NAME);
  fs.copyFileSync(exe, exeDest);
  artifacts.exe = { file: EXE_NAME, ...sha256File(exeDest) };
  artifacts.exe.sizeHuman = humanSize(artifacts.exe.size);

  // 2) 便携 zip
  let portable = null;
  if (zip) {
    const entries = [{ name: EXE_NAME, data: fs.readFileSync(exe) }];
    for (const c of componentFiles) entries.push({ name: c.name, data: fs.readFileSync(c.abs) });
    for (const p of presetFiles) entries.push({ name: p.name, data: fs.readFileSync(p.abs) });
    const zipName = `portable_${version}_${ts}${flavor ? `_${flavor}` : ""}.zip`;
    const zipDest = path.join(outDir, zipName);
    fs.writeFileSync(zipDest, buildZip(entries));
    portable = { file: zipName, ...sha256File(zipDest) };
    portable.sizeHuman = humanSize(portable.size);
    portable.entries = entries.map((e) => e.name);
    artifacts.portable = portable;
  }

  // 3) 安装包（可能不存在：网络受限 / bundle 被跳过）
  const exeMtimeMs = fs.statSync(exe).mtimeMs;
  const bundleBuiltAt = { nsis: null, msi: null };
  const takeBundleArtifact = (kind, exts) => {
    const src = firstMatch(path.join(profileDir, "bundle", kind), exts);
    if (!src) return null;
    const mtimeMs = fs.statSync(src).mtimeMs;
    if (mtimeMs + BUNDLE_MTIME_TOLERANCE_MS < exeMtimeMs) {
      warnPkg(
        `跳过陈旧的 ${kind}：${path.basename(src)}（${new Date(mtimeMs).toISOString()}）早于本次 exe` +
          `（${new Date(exeMtimeMs).toISOString()}）—— 本次构建没有生成安装包` +
          `（离线缺 wix / NSIS 工具，或用了 --no-bundle）`,
      );
      return null;
    }
    bundleBuiltAt[kind] = new Date(mtimeMs).toISOString();
    return src;
  };

  const nsis = takeBundleArtifact("nsis", [".exe"]);
  if (nsis) {
    const dest = path.join(outDir, path.basename(nsis));
    fs.copyFileSync(nsis, dest);
    artifacts.nsis = { file: path.basename(nsis), ...sha256File(dest) };
    artifacts.nsis.sizeHuman = humanSize(artifacts.nsis.size);
  }
  // ⚠️ AI 版**不看 msi**：`tauri.ai-runtime.conf.json` 里 `bundle.targets = ["nsis"]`
  //    （WiX 侧不打包 `.vcmi-fv-launcher` 点开头目录）—— 出 MSI 等于产出"标称 AI 版却不含组件"的包。
  if (flavor !== "ai") {
    const msi = takeBundleArtifact("msi", [".msi"]);
    if (msi) {
      const dest = path.join(outDir, path.basename(msi));
      fs.copyFileSync(msi, dest);
      artifacts.msi = { file: path.basename(msi), ...sha256File(dest) };
      artifacts.msi.sizeHuman = humanSize(artifacts.msi.size);
    }
  }

  // 4) release.json（顶层 sha256/size = 主更新产物）
  const primary = portable ?? artifacts.exe;
  const info = {
    version,
    sha256: primary.hash,
    size: primary.size,
    format: portable ? "zip" : "exe",
    kind: "full",
    // 预置补丁指纹（进索引 releases[].preset，便于事后核对"这版包带了什么配置"）
    preset: { file: path.basename(presetPath), sha256: sha256File(presetPath).hash },
    profile,
    flavor: flavor || null,
    dir: dirName,
    builtAt: nowIso(),
    bundleBuiltAt,
    ...(flavor === "ai" ? { componentBytes } : {}),
    ...(warnings.length ? { warnings } : {}),
    exe: EXE_NAME,
    sizeHuman: primary.sizeHuman,
    artifacts,
  };
  writeJsonAtomic(path.join(outDir, "release.json"), info);
  return { dirName, outDir, version, info };
}

/* ────────────────────────────── GitHub REST ────────────────────────────── */

let TOKEN_VALUE = "";

function mask(s) {
  let out = String(s ?? "");
  if (TOKEN_VALUE && TOKEN_VALUE.length >= 8) out = out.split(TOKEN_VALUE).join("***");
  return out.replace(/\b(gh[pousr]_[A-Za-z0-9]{16,}|github_pat_[A-Za-z0-9_]{20,})\b/g, "***");
}

function resolveToken(opts) {
  const use = (v, from) => {
    TOKEN_VALUE = String(v).trim();
    log(`token 来源：${from}`);
    return TOKEN_VALUE;
  };
  if (opts.token) return use(opts.token, "--token（⚠️ 会进 shell 历史，仅调试用）");
  for (const k of ["GITHUB_TOKEN", "GH_TOKEN"]) {
    if (process.env[k] && process.env[k].trim()) return use(process.env[k], `环境变量 ${k}`);
  }
  const envLocal = path.join(ROOT, ".env.local");
  if (fs.existsSync(envLocal)) {
    for (const line of fs.readFileSync(envLocal, "utf8").split(/\r?\n/)) {
      const m = line.match(/^\s*(GITHUB_TOKEN|GH_TOKEN)\s*=\s*(.*)$/);
      if (m) {
        const v = m[2].trim().replace(/^["']|["']$/g, "");
        if (v) return use(v, ".env.local");
      }
    }
  }
  if (opts.tokenFile) {
    const p = abs(opts.tokenFile);
    if (!fs.existsSync(p)) die(`--token-file 不存在：${opts.tokenFile}`);
    const v = fs.readFileSync(p, "utf8").trim();
    if (!v) die(`--token-file 内容为空：${opts.tokenFile}`);
    return use(v, `--token-file ${opts.tokenFile}`);
  }
  die(
    "缺少 GitHub token（需要 Contents: Read and write 权限）。任选其一：\n" +
      "  ① export GITHUB_TOKEN=<token>     （Windows: setx GITHUB_TOKEN <token> 后重开终端）\n" +
      "  ② 在仓库根 .env.local 里加一行 GITHUB_TOKEN=<token>（该文件已被 .gitignore 忽略）\n" +
      "  ③ --token-file <文件>（内容为整行 token）",
  );
}

function ghRequest({ host = "api.github.com", method = "GET", urlPath, body, token, extraHeaders = {} }) {
  return new Promise((resolve, reject) => {
    const payload =
      body == null ? null : Buffer.from(typeof body === "string" ? body : JSON.stringify(body), "utf8");
    const headers = {
      Authorization: `Bearer ${token}`,
      Accept: "application/vnd.github+json",
      "X-GitHub-Api-Version": GH_API_VERSION,
      "User-Agent": "vcmi-fv-release-script",
      ...extraHeaders,
    };
    if (payload) headers["Content-Length"] = payload.length;
    const req = https.request({ host, method, path: urlPath, headers, timeout: 60_000 }, (res) => {
      const chunks = [];
      res.on("data", (c) => chunks.push(c));
      res.on("end", () =>
        resolve({
          status: res.statusCode,
          headers: res.headers,
          text: Buffer.concat(chunks).toString("utf8"),
        }),
      );
    });
    req.on("timeout", () => req.destroy(new Error("请求超时（60s）")));
    req.on("error", reject);
    if (payload) req.write(payload);
    req.end();
  });
}

function ghError(res, what) {
  let detail = "";
  try {
    const v = JSON.parse(res.text || "{}");
    detail = [v.message, Array.isArray(v.errors) ? v.errors.map((e) => e.message || JSON.stringify(e)).join("；") : ""]
      .filter(Boolean)
      .join(" — ");
  } catch {
    detail = (res.text || "").slice(0, 300);
  }
  const hint =
    res.status === 401
      ? "（token 无效或已过期）"
      : res.status === 403
        ? "（权限不足或触发限流：token 需 Contents: Read and write）"
        : res.status === 404
          ? "（仓库不存在，或 token 无权访问）"
          : "";
  return new Error(`${what} 失败：HTTP ${res.status} ${hint} ${mask(detail)}`.trim());
}

function ghJson(res, what) {
  if (res.status >= 200 && res.status < 300) {
    try {
      return JSON.parse(res.text || "{}");
    } catch (e) {
      die(`${what} 返回了非法 JSON：${mask(e.message)}`);
    }
  }
  throw ghError(res, what);
}

async function ghRetry(fn, what, times = 3) {
  let last = null;
  for (let i = 0; i < times; i++) {
    try {
      return await fn();
    } catch (e) {
      last = e;
      const retriable = !/HTTP (4\d\d)/.test(String(e.message));
      if (!retriable || i === times - 1) break;
      const wait = 1000 * 3 ** i;
      warn(`${what} 第 ${i + 1} 次失败（${mask(e.message)}），${wait / 1000}s 后重试…`);
      await new Promise((r) => setTimeout(r, wait));
    }
  }
  throw last;
}

function ghUploadAsset({ repo, token, releaseId, name, file }) {
  return new Promise((resolve, reject) => {
    const stat = fs.statSync(file);
    const req = https.request(
      {
        host: "uploads.github.com",
        method: "POST",
        path: `/repos/${repo.owner}/${repo.name}/releases/${releaseId}/assets?name=${encodeURIComponent(name)}`,
        headers: {
          Authorization: `Bearer ${token}`,
          Accept: "application/vnd.github+json",
          "X-GitHub-Api-Version": GH_API_VERSION,
          "User-Agent": "vcmi-fv-release-script",
          "Content-Type": "application/octet-stream",
          "Content-Length": stat.size,
        },
        timeout: 30 * 60_000,
      },
      (res) => {
        const chunks = [];
        res.on("data", (c) => chunks.push(c));
        res.on("end", () =>
          resolve({ status: res.statusCode, headers: res.headers, text: Buffer.concat(chunks).toString("utf8") }),
        );
      },
    );
    req.on("timeout", () => req.destroy(new Error("上传超时（30min）")));
    req.on("error", reject);
    const rs = fs.createReadStream(file);
    rs.on("error", reject);
    rs.pipe(req);
  });
}

/* ────────────────────────────── 通用 HTTP（匿名探测 / 远端索引） ────────────────────────────── */

function httpGet(url, { method = "GET", redirects = 5, timeout = 30_000 } = {}) {
  return new Promise((resolve, reject) => {
    const req = https.request(url, { method, timeout }, (res) => {
      if ([301, 302, 303, 307, 308].includes(res.statusCode) && res.headers.location && redirects > 0) {
        res.resume();
        const next = new URL(res.headers.location, url).toString();
        return resolve(httpGet(next, { method: "HEAD" === method ? "HEAD" : "GET", redirects: redirects - 1, timeout }));
      }
      const chunks = [];
      res.on("data", (c) => chunks.push(c));
      res.on("end", () =>
        resolve({
          status: res.statusCode,
          headers: res.headers,
          body: Buffer.concat(chunks),
          finalUrl: url,
        }),
      );
    });
    req.on("timeout", () => req.destroy(new Error("请求超时")));
    req.on("error", reject);
    req.end();
  });
}

/* ────────────────────────────── 版本注入与命名（ADR 0007） ────────────────────────────── */

/** 合法 semver（Cargo 强制；`v1.0.0` / `2026-10-01` 之类会被 cargo build 拒绝） */
const SEMVER_RE = /^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/;
/** 临时注入的快照/标记目录（`.tmp/` 已在 .gitignore 中，不会污染仓库） */
const INJ_DIR = path.join(ROOT, ".tmp", "version-injection");
const INJ_MARKER = path.join(INJ_DIR, "state.json");
/** 版本注入会改写的文件（相对仓库根） */
const VERSION_FILES = ["src-tauri/Cargo.toml", "src-tauri/Cargo.lock", "src-tauri/tauri.conf.json"];

function snapName(rel) {
  return rel.replace(/[\\/]/g, "__");
}

/** 临时注入"待还原"标记：进程异常退出时由顶层 finally 兜底还原 */
let PENDING_TEMP_RESTORE = false;

/** 展开命名模板（未知占位符直接报错，避免静默生成错名字） */
function expandTemplate(tpl, vars, what) {
  const unknown = [...String(tpl).matchAll(/\{(\w+)\}/g)].map((m) => m[1]).find((k) => !(k in vars));
  if (unknown) {
    die(
      `${what} 模板含未知占位符 {${unknown}}（可用：${Object.keys(vars)
        .map((x) => `{${x}}`)
        .join(" ")}）`,
    );
  }
  return String(tpl).replace(/\{(\w+)\}/g, (_, k) => String(vars[k] ?? ""));
}

/**
 * 发行标识 / tag / title（ADR 0007 V2）：**CLI > 索引 `naming` 模板 > 内置缺省**。
 * 缺省 `release = <契约版本>` / `tag = v<release>` / `title = "<product> <release>"` ⇒ 零参数。
 */
function resolveNaming(index, opts, version) {
  const n = index?.naming && typeof index.naming === "object" ? index.naming : {};
  const vars = { version, date: todayLocal(), release: "", flavor: opts.flavor || "" };
  const release = opts.version || expandTemplate(n.release ?? "{version}", vars, "naming.release");
  if (!NAME_RE.test(release) || release.length > 64) {
    die(`发行标识非法（只允许 A-Za-z0-9._- ，≤64）："${release}"（来自 --version 或索引 naming.release）`);
  }
  vars.release = release;
  const tag = opts.tag || expandTemplate(n.tag ?? "v{release}", vars, "naming.tag");
  if (!NAME_RE.test(tag) || tag.length > 100) {
    die(`tag 非法（只允许 A-Za-z0-9._- ，≤100）："${tag}"（来自 --tag 或索引 naming.tag）`);
  }
  const title = (opts.title || expandTemplate(n.title ?? `${PRODUCT} {release}`, vars, "naming.title"))
    .replace(/\s+/g, " ")
    .trim();
  return { release, tag, title, templated: Object.keys(n).length > 0 && !opts.version && !opts.tag && !opts.title };
}

/** 单调性守卫：更新器只在 `current < version` 时提示 ⇒ 版本号不可回退、同号重发无效 */
function checkMonotonic(index, version, { allow = false } = {}) {
  const versions = (index?.versions ?? []).map((v) => v.version).filter(Boolean);
  if (!versions.length) return null;
  const max = versions.sort((a, b) => cmpVersion(b, a))[0];
  if (cmpVersion(version, max) > 0) return null;
  const msg = `版本 ${version} 不高于索引里的最新版 ${max}（同号重发对已装用户无效、版本号不可回退）`;
  if (allow) {
    warn(`${msg} —— 已放行（本包不进更新链 / 显式 --allow-downgrade）`);
    return max;
  }
  die(
    `${msg}\n  请任选其一：\n` +
      `    ① 用一个更高的版本号；\n` +
      `    ② --allow-downgrade（明确知道后果）；\n` +
      `    ③ --no-update-chain（本包不进更新链，仅作手工分发）`,
  );
}

/** 把当前 3 个版本文件快照到 `.tmp/`（供临时注入还原） */
function snapshotVersionFiles(version) {
  fs.rmSync(INJ_DIR, { recursive: true, force: true });
  fs.mkdirSync(INJ_DIR, { recursive: true });
  const present = {};
  for (const rel of VERSION_FILES) {
    const p = path.join(ROOT, rel);
    present[rel] = fs.existsSync(p);
    if (present[rel]) fs.copyFileSync(p, path.join(INJ_DIR, snapName(rel)));
  }
  writeJsonAtomic(INJ_MARKER, { version, at: nowIso(), present });
}

/** 还原被临时注入改写的版本文件（幂等；无标记则什么都不做） */
function restoreVersionFiles({ quiet = false } = {}) {
  if (!fs.existsSync(INJ_MARKER)) return false;
  const st = readJson(INJ_MARKER, "版本注入标记");
  const present = st.present ?? {};
  for (const rel of VERSION_FILES) {
    const dst = path.join(ROOT, rel);
    const bak = path.join(INJ_DIR, snapName(rel));
    if (present[rel] && fs.existsSync(bak)) fs.copyFileSync(bak, dst);
    else if (present[rel] === false) fs.rmSync(dst, { force: true });
  }
  fs.rmSync(INJ_DIR, { recursive: true, force: true });
  if (!quiet) log(`已还原被临时注入改写的版本文件（Cargo.toml 回到 ${readCargoVersion()}）`);
  return true;
}

/** 兜底：上次 pack 的临时注入若因中断未还原（含 Ctrl+C / 崩溃），这里恢复 */
function cleanupStaleInjection() {
  if (!fs.existsSync(INJ_MARKER)) return;
  const st = readJson(INJ_MARKER, "版本注入标记");
  warn(`检测到上一次「临时注入」未还原（注入版本 ${st.version ?? "?"}，${st.at ?? "?"}）→ 自动还原`);
  restoreVersionFiles({ quiet: true });
}

/* ────────────────────────────── 子命令：pack ────────────────────────────── */

function runStep(cmd, args, label, shell = false) {
  log(`${label}…`);
  const r = spawnSync(cmd, args, { cwd: ROOT, stdio: "inherit", shell });
  if (r.error) die(`${label} 启动失败：${r.error.message}`);
  if (r.status !== 0) die(`${label} 失败（退出码 ${r.status}）`);
}

function classifyAsset(name) {
  const low = name.toLowerCase();
  if (low.startsWith("portable_") && low.endsWith(".zip")) return "update";
  if (name === EXE_NAME) return "binary";
  if (low.endsWith(".exe") || low.endsWith(".msi")) return "installer";
  return "extra";
}

function buildAssetsFromDir({ dir, dirLabel, info, repo, dl, tag, releaseId }) {
  const skip = new Set(["release.json", "index.entry.json"]);
  const byFile = new Map();
  for (const a of Object.values(info?.artifacts ?? {})) {
    if (a?.file) byFile.set(a.file, a);
  }
  const assets = [];
  for (const name of fs.readdirSync(dir).sort()) {
    if (skip.has(name) || name.includes(".tmp-")) continue;
    if (!ASCII_RE.test(name)) die(`资产名含非 ASCII 字符：${name}（GitHub 对 URL 编码极不友好，请改构建命名）`);
    const p = path.join(dir, name);
    if (!fs.statSync(p).isFile()) continue;
    const known = byFile.get(name);
    const { hash, size } = known?.hash ? { hash: known.hash, size: known.size } : sha256File(p);
    assets.push({
      name,
      role: classifyAsset(name),
      path: nativePath(tag, name),
      url: assetUrl(dl, tag, name, releaseId),
      size,
      sha256: hash,
      _dir: dir,
      _label: dirLabel,
    });
  }
  return assets;
}

function cmdPack(opts) {
  // 兜底：上次 pack 的临时注入若因中断（Ctrl+C / 崩溃）未还原，这里先恢复
  cleanupStaleInjection();

  const index0 = loadIndex();
  const fileVersion = readCargoVersion();

  const noChain = !!(opts.noUpdateChain || opts.test);
  /** `--test` = 临时注入 + 不进更新链 + 不出安装包（一键测试包） */
  const temp = !!(opts.tempVersion || opts.test);
  const noBundle = !!(opts.noBundle || opts.test);
  if (opts.critical && noChain) die("--critical 与 --no-update-chain 互斥（不进更新链的版本不该标重要更新）");
  if (temp && opts.noBuild) {
    die(
      `${opts.test ? "--test" : "--temp-version"} 需要构建（不构建就不会注入版本）。\n` +
        `  只想复用现有产物请去掉该开关（版本取文件里的值）。`,
    );
  }

  // ── 本次契约版本（ADR 0007）：CLI >（--test 自动 `<现版本>-dev.<日期>`）> 文件里的版本 ──
  const targetVersion = opts.version
    ? String(opts.version)
    : opts.test
      ? `${fileVersion}-dev.${todayLocal().replace(/-/g, "")}`
      : fileVersion;
  if (!SEMVER_RE.test(targetVersion)) {
    die(
      `版本 "${targetVersion}" 不是合法 semver（Cargo 强制 major.minor.patch[-pre][+build]）。\n` +
        `  可用：0.2.0 ✅ / 0.2.0-rc1 ✅ / 2026.10.1 ✅\n` +
        `  不可用：v1.0.0 ❌ / 2026-10-01 ❌（自由文本请用 --tag / --title）`,
    );
  }
  const needInject = targetVersion !== fileVersion;
  if (opts.noBuild && needInject) {
    die(
      `--no-build 下不能改版本（不构建 ⇒ exe 里仍是 ${fileVersion}）。\n` +
        `  要改版本请去掉 --no-build；只想换发行标识请用 --tag / --title。`,
    );
  }
  // Windows 安装包（WiX/MSI）只接受**纯数字**版本：`0.1.0-dev.1` 会让 bundle 直接失败
  // （实测 `optional pre-release identifier in app version must be numeric-only ... for msi target`）
  if (/[+-]/.test(targetVersion) && !noBundle) {
    die(
      `版本 ${targetVersion} 带 prerelease/build 后缀，**打不出 Windows 安装包**（WiX 限制）。\n` +
        `  请任选其一：\n` +
        `    ① 用纯数字版本：--version 0.2.0（正式发行推荐）\n` +
        `    ② 加 --no-bundle：只出便携包（exe + zip），不出 NSIS/MSI —— 测试包建议直接用 --test`,
    );
  }
  if (needInject) {
    checkMonotonic(index0, targetVersion, { allow: noChain || !!opts.allowDowngrade });
    if (temp) {
      snapshotVersionFiles(targetVersion);
      PENDING_TEMP_RESTORE = true;
      const onSig = (sig) => {
        warn(`收到 ${sig}：还原临时注入的版本文件…`);
        restoreVersionFiles({ quiet: true });
        process.exit(130);
      };
      process.once("SIGINT", () => onSig("SIGINT"));
      process.once("SIGTERM", () => onSig("SIGTERM"));
    }
    runStep(
      process.execPath,
      [path.join(__dirname, "sync-version.mjs"), "--set", targetVersion],
      `写入版本号 ${fileVersion} → ${targetVersion}${temp ? "（临时，构建后还原）" : "（持久，需随发行提交）"}`,
    );
    // ⚠️ 断言注入**真的**落到两个文件里（历史上曾出现"打印成功、文件没变"的静默失败）
    const afterCargo = readCargoVersion();
    const afterConf = readJson(path.join(ROOT, "src-tauri", "tauri.conf.json"), "tauri.conf.json").version;
    if (afterCargo !== targetVersion || afterConf !== targetVersion) {
      die(
        `版本写入未生效：Cargo.toml=${afterCargo} / tauri.conf.json=${afterConf} / 期望 ${targetVersion}\n` +
          `  请检查 scripts/sync-version.mjs（Cargo 与 tauri.conf 必须一致，否则 tauri build 会失败）`,
      );
    }
  }
  const contractVersion = readCargoVersion();

  // ── 发行标识 / tag / title：CLI > 索引 naming 模板 > 内置缺省 ──
  const naming = resolveNaming(index0, opts, contractVersion);
  const releaseId = naming.release;
  const tag = naming.tag;

  // ── CI / 标签守卫（早失败，别浪费一次构建）──
  // HEAD 正好停在某个 tag 上，却与本次 tag 不一致 ⇒ 几乎总是"打了标签但忘了定版"。
  // CI 从 tag 构建时若不拦，会把 v0.2.0 标签下的 0.1.0 代码静默发成 v0.1.0 的发行。
  const headTag = git(["describe", "--tags", "--exact-match", "HEAD"]);
  if (headTag && headTag !== tag) {
    if (opts.allowTagMismatch) {
      warn(`HEAD 上的 tag 是 ${headTag}，本次 tag 是 ${tag} —— 已按 --allow-tag-mismatch 放行`);
    } else {
      die(
        `HEAD 正好在 tag ${headTag} 上，但本次要发的 tag 是 ${tag}（版本 ${contractVersion}）。\n` +
          `  这通常意味着「打了标签但忘了定版」。请任选其一：\n` +
          `    ① 用与该标签一致的版本：--version <版本>（并提交）；\n` +
          `    ② --allow-tag-mismatch（确认是有意为之）；\n` +
          `    ③ 从分支（非标签）构建。`,
      );
    }
  }

  const repo = resolveRepo(opts, index0);
  const dl = resolveDownload(index0, repo);
  const preset = resolvePreset(index0, repo);
  const profile = opts.profile || "release";
  if (profile !== "release") die(`发行只允许 --profile release（收到 ${profile}）；测试包请用 --test（临时注入、不进更新链）`);
  /** 标记"target 里的产物是测试版本"（防 --no-build 把测试 exe 当正式包发出去） */
  const TEST_BUILD_MARK = path.join(ROOT, "src-tauri", "target", profile, ".fv-test-build.json");

  // 1) 随包预置补丁（**必须早于构建**：bundle.resources 构建期要读它）
  const presetChanged = writePresetFiles(preset);
  const presetPath = activePresetFile(opts.flavor || "");
  const presetSha = sha256File(presetPath).hash;
  if (presetChanged.length) log(`已更新随包预置：${presetChanged.join(" / ")}（记得一并提交）`);
  if (opts.flavor && !fs.existsSync(presetFileFor(opts.flavor))) {
    warn(`索引未声明 preset.flavors.${opts.flavor}：本次 ${opts.flavor} 包只用 base 预置（不会默认启用该 flavor 的功能）`);
  }

  // 2) 构建
  if (!opts.noBuild) {
    // 先落标记：构建中途失败时 target/ 里也已是被注入版本编译出的产物，标记必须存在
    if (temp && needInject) writeJsonAtomic(TEST_BUILD_MARK, { version: contractVersion, at: nowIso() });
    runStep(process.execPath, [path.join(__dirname, "sync-version.mjs")], "同步版本号");
    const buildArgs = ["run", "tauri", "--", "build"];
    if (opts.flavor === "ai") buildArgs.push("--config", "src-tauri/tauri.ai-runtime.conf.json");
    if (noBundle) buildArgs.push("--no-bundle");
    runStep("npm", buildArgs, `构建（release${opts.flavor ? ` / ${opts.flavor}` : ""}${noBundle ? "，不出安装包" : ""}）`, true);
    if (!temp) fs.rmSync(TEST_BUILD_MARK, { force: true }); // 正式构建 → 清掉测试标记
  } else {
    // ⚠️ 防坑（fail-closed）：`--test` 构建会把 target 里的 exe/安装包换成**测试版本**，
    //    再用 --no-build 复用它们，就会产出"条目是正式版、exe 内部是测试版"的不一致包 ——
    //    这种包 publish 出去无法被察觉，所以这里**直接拒绝**。
    if (fs.existsSync(TEST_BUILD_MARK)) {
      const st = readJson(TEST_BUILD_MARK, "测试构建标记");
      die(
        `target/${profile} 里的产物来自上次 --test 构建（版本 ${st.version}，${st.at}），不是正式版本。\n` +
          `  请去掉 --no-build 重新构建；确知无碍时可删除标记后重试：\n` +
          `    rm src-tauri/target/${profile}/.fv-test-build.json`,
      );
    }
    if (presetChanged.length) {
      warn("--no-build 下预置补丁内容有变化：本次 zip 会用新补丁，但安装包/exe 用的是上次构建时的补丁");
    }
    log("跳过构建（--no-build）：直接用 src-tauri/target 里的现有产物");
  }

  // 3) 提取产物（内部读一次 Cargo.toml 版本 → 必须在"临时注入还原"之前）
  const ex = extractArtifacts({ profile, flavor: opts.flavor, zip: true });
  const info = ex.info;

  // 临时注入：产物已提取（版本已写进 release.json / 文件名），立刻把源码还原回原版本
  if (temp && needInject) {
    restoreVersionFiles();
    PENDING_TEMP_RESTORE = false;
  }

  // 4) 说明（**不进更新链**的包缺说明只告警：测试/预发行不该因 ChangeLog 没写而卡住）
  const notesRes = resolveNotes(opts, { release: releaseId, contractVersion, allowMissing: noChain });
  if (!Object.keys(notesRes.notes).length) {
    warn(`没有更新说明（ChangeLog.md 里没有 "${releaseId}" 节）—— 本包不进更新链，按测试/预发行处理`);
  }

  // 5) 条目
  let assets = buildAssetsFromDir({
    dir: ex.outDir,
    dirLabel: ex.dirName,
    info,
    repo,
    dl,
    tag,
    releaseId,
  });
  const updates = assets.filter((a) => a.role === "update");
  if (updates.length !== 1) {
    die(`本目录应有且仅有 1 个 update 资产（portable_*.zip），实际 ${updates.length} 个：${updates.map((a) => a.name).join("、")}`);
  }

  const entry = {
    release: releaseId,
    title: naming.title,
    tag,
    target: opts.target || git(["rev-parse", "--abbrev-ref", "HEAD"]) || repo.defaultBranch,
    commit: opts.target && /^[0-9a-f]{7,40}$/i.test(opts.target) ? opts.target : git(["rev-parse", "HEAD"]),
    page: `${repo.releaseBase}/tag/${tag}`,
    updateVersion: noChain ? null : contractVersion,
    prerelease: noChain,
    channel: index0?.channel || "stable",
    publishedAt: opts.publishedAt || "",
    critical: !!opts.critical,
    notesSource: notesRes.source,
    notes: notesRes.notes,
    assets: assets.map(({ _dir, _label, ...rest }) => rest),
    preset: { file: path.basename(presetPath), sha256: presetSha },
    pack: { dir: ex.dirName, profile, flavor: opts.flavor || null, builtAt: info.builtAt },
  };

  // 6) 片段 + latest.json
  const fragment = { release: entry, version: null, generatedAt: nowIso() };
  const probeIndex = index0
    ? { ...index0, releases: [...index0.releases.filter((r) => r.release !== releaseId), entry] }
    : { releases: [entry] };
  fragment.version = noChain ? null : projectVersions(probeIndex)[0] ?? null;
  writeJsonAtomic(path.join(ex.outDir, "index.entry.json"), fragment);
  writeJsonIfChanged(LATEST_PATH, {
    dir: ex.dirName,
    version: ex.version,
    release: releaseId,
    tag,
    profile,
    flavor: opts.flavor || null,
    builtAt: info.builtAt,
  });

  // 7) 索引自举（**只补配置区块，绝不写版本条目** —— D3）
  const index = index0 ?? {
    schema: SCHEMA,
    schemaVersion: 1,
    product: PRODUCT,
    channel: "stable",
    latest: null,
    minSupported: null,
    versions: [],
    releases: [],
  };
  const bootChanged = bootstrapIndex(index, repo, dl, preset);
  if (!index0 || bootChanged) {
    writeJsonAtomic(INDEX_PATH, index);
    log(`已自举 ${INDEX_REL}（仅 repo/download/preset 配置区块；**未**写本版本条目 —— 只有 publish 才写）`);
  }

  // 8) 输出
  console.log("");
  log(
    `版本：${contractVersion}` +
      (needInject ? `（由 ${fileVersion} 注入${temp ? "，源码已还原" : "，持久写入 —— 需随发行提交"}）` : "（取自文件）"),
  );
  log(`发行标识：${releaseId}   tag：${tag}${naming.templated ? "（来自索引 naming 模板）" : ""}`);
  log(`标题：${naming.title}`);
  log(`发行目录：${path.relative(ROOT, ex.outDir).replace(/\\/g, "/")}`);
  for (const a of entry.assets) {
    log(`  ${a.role.padEnd(9)} ${a.name}  ${humanSize(a.size)}  ${a.sha256.slice(0, 12)}…`);
  }
  log(`说明来源：${entry.notesSource}（语言：${Object.keys(entry.notes).join(" / ") || "无"}）`);
  log(`随包预置：${entry.preset.file}  ${presetSha.slice(0, 12)}…  ${opts.flavor === "ai" ? "（含 AI 默认启用配置）" : ""}`);
  if (noChain) {
    warn("不进更新链（--no-update-chain / --test）：只作 GitHub 发行 + 索引档案，用户不会收到更新提示");
  }
  console.log("");
  log("下一步（本地自测通过后）：");
  log(`  npm run release:publish -- --version ${releaseId}`);
  warn("此时 releases/index.json 尚未变更；未推送索引前，对所有用户都不生效。");
}

/* ────────────────────────────── 子命令：publish ────────────────────────────── */

function findPackedEntry(version, dirOpt) {
  if (dirOpt) {
    const d = abs(dirOpt);
    const f = path.join(d, "index.entry.json");
    if (!fs.existsSync(f)) die(`${dirOpt} 下没有 index.entry.json（该目录不是 pack 产出的发行目录）`);
    return { dir: d, fragment: readJson(f, "条目片段") };
  }
  const root = path.join(ROOT, "releases");
  const hits = [];
  for (const e of fs.readdirSync(root, { withFileTypes: true })) {
    if (!e.isDirectory()) continue;
    const f = path.join(root, e.name, "index.entry.json");
    if (!fs.existsSync(f)) continue;
    try {
      const frag = JSON.parse(fs.readFileSync(f, "utf8"));
      if (frag?.release?.release === version) hits.push({ dir: path.join(root, e.name), fragment: frag });
    } catch {
      warn(`跳过坏片段：releases/${e.name}/index.entry.json`);
    }
  }
  if (!hits.length) {
    die(
      `没有找到 --version ${version} 的本地发行目录（releases/*/index.entry.json）。\n` +
        `  请先跑：npm run release:pack -- --version ${version}`,
    );
  }
  hits.sort((a, b) => fs.statSync(b.dir).mtimeMs - fs.statSync(a.dir).mtimeMs);
  return hits[0]; // 取最新打包的那份
}

async function cmdPublish(opts) {
  // 缺省 = 文件里的版本（与 pack 对称：本地定版后，发布也可以零参数）
  const releaseId = opts.version ? String(opts.version) : readCargoVersion();
  if (!opts.version) log(`未传 --version → 使用文件里的版本 ${releaseId}`);

  const index = loadIndex();
  if (!index) {
    die(
      `${INDEX_REL} 尚未初始化。请先跑 pack（它会自举 repo/download/preset 配置区块）：\n` +
        `  npm run release:pack -- --version ${releaseId} --owner <账号> --repo <仓库名>`,
    );
  }
  const repo = resolveRepo(opts, index);
  const dl = resolveDownload(index, repo);
  const preset = resolvePreset(index, repo);
  if (repo.provider !== "github") die(`publish 目前只支持 GitHub（当前 repo.provider=${repo.provider}）`);

  const { dir, fragment } = findPackedEntry(releaseId, opts.dir);
  const entry = fragment.release;
  const tag = opts.tag || entry.tag || `v${releaseId}`;
  entry.tag = tag;
  if (opts.title) entry.title = opts.title;
  if (opts.critical) entry.critical = true;
  if (opts.noUpdateChain) {
    if (entry.updateVersion) warn(`改为不进更新链（原 updateVersion=${entry.updateVersion}）`);
    entry.updateVersion = null;
    entry.prerelease = true;
  }
  if (opts.replaceVersion) log("--replace-version：允许覆盖索引中同契约版本的既有条目");

  // 上传资产 = 主目录 + --attach 附加目录（如 AI 版）
  const dirs = [{ dir, label: path.basename(dir), fragment }];
  for (const extra of opts.attach) {
    const d = abs(extra);
    if (!fs.existsSync(d)) die(`--attach 目录不存在：${extra}`);
    const fj = path.join(d, "release.json");
    if (!fs.existsSync(fj)) die(`--attach 目录缺少 release.json（不是 pack 产出的目录）：${extra}`);
    dirs.push({ dir: d, label: path.basename(d), fragment: { release: readJson(fj, "--attach release.json") } });
  }
  const uploads = [];
  for (const d of dirs) {
    let assets = d.fragment.release.assets;
    if (!Array.isArray(assets)) {
      assets = buildAssetsFromDir({
        dir: d.dir,
        dirLabel: d.label,
        info: d.fragment.release,
        repo,
        dl,
        tag,
        releaseId,
      });
    }
    for (const a of assets) uploads.push({ ...a, _dir: d.dir, _label: d.label });
  }
  // 更新源资产唯一性
  let updateAssets = uploads.filter((a) => a.role === "update");
  if (opts.updateAsset) {
    for (const a of uploads) {
      if (a.name === opts.updateAsset) a.role = "update";
      else if (a.role === "update") a.role = "extra";
    }
    updateAssets = uploads.filter((a) => a.role === "update");
    if (!updateAssets.length) die(`--update-asset ${opts.updateAsset} 不存在于待上传资产中`);
  }
  if (updateAssets.length !== 1) {
    die(
      `更新源资产必须唯一，实际 ${updateAssets.length} 个：${updateAssets.map((a) => a.name).join("、")}\n` +
        `  多版本包（如纯净版 + AI 版）请用 --update-asset <文件名> 指定谁进更新链。`,
    );
  }

  // 重算指纹（防「打包后手工替换过文件」）
  const mismatch = [];
  for (const a of uploads) {
    const p = path.join(a._dir, a.name);
    if (!fs.existsSync(p)) {
      mismatch.push(`${a.name}（文件不存在）`);
      continue;
    }
    const { hash, size } = sha256File(p);
    if (hash !== a.sha256 || size !== a.size) mismatch.push(`${a.name}（sha256/size 与条目不符）`);
    a.sha256 = hash;
    a.size = size;
    a.path = nativePath(tag, a.name);
    a.url = assetUrl(dl, tag, a.name, releaseId);
  }
  if (mismatch.length) die(`资产指纹校验失败（打包后文件被改过？）：\n  - ${mismatch.join("\n  - ")}`);
  // 预置补丁指纹
  const presetPath = activePresetFile(entry.pack?.flavor || "");
  if (fs.existsSync(presetPath) && entry.preset?.sha256) {
    const cur = sha256File(presetPath).hash;
    if (cur !== entry.preset.sha256) {
      warn(`随包预置补丁与 pack 时不一致（${entry.preset.file}），已按当前内容更新索引记录`);
      entry.preset.sha256 = cur;
    }
  }

  // 说明（若来源是 ChangeLog，md 改了就跟着更新）
  const notesRes = resolveNotes(opts, { release: releaseId, contractVersion: entry.updateVersion || fragment.release.updateVersion });
  if (JSON.stringify(notesRes.notes) !== JSON.stringify(entry.notes)) {
    log(`更新说明已刷新（来源 ${notesRes.source}）：语言 ${Object.keys(notesRes.notes).join(" / ") || "无"}`);
    entry.notes = notesRes.notes;
    entry.notesSource = notesRes.source;
  }
  if (!Object.keys(entry.notes || {}).length && !opts.allowEmptyNotes) {
    die("更新说明为空。请补 ChangeLog.md 或传 --notes / --allow-empty-notes");
  }

  const headSha = git(["rev-parse", "HEAD"]);
  const dirty = git(["status", "--porcelain"]);
  console.log("");
  log(`仓库：${repo.owner}/${repo.name}    tag：${tag}    target：${headSha.slice(0, 10)}`);
  log(`将上传 ${uploads.length} 个资产：`);
  for (const a of uploads) {
    log(`  ${a.role.padEnd(9)} ${a._label}/${a.name}  ${humanSize(a.size)}  ${a.sha256.slice(0, 12)}…`);
  }
  if (dirty) {
    warn("工作区有未提交改动 —— tag 将指向当前 HEAD，请确认它就是产出这批二进制的提交：");
    for (const line of dirty.split("\n").slice(0, 10)) warn(`  ${line}`);
  }

  const body = buildBody(opts, entry.notes, notesRes.section);
  if (opts.dryRun) {
    log("--dry-run：不联网、不写盘。Release 正文预览：");
    console.log("────────────");
    console.log(body || "(空)");
    console.log("────────────");
    log(`索引将 upsert 发行 ${releaseId}（updateVersion=${entry.updateVersion ?? "null"}）并重投影 versions[]`);
    log("已跳过：创建 release / 上传资产 / 写索引");
    return;
  }

  const token = resolveToken(opts);
  if (!token) die("token 为空");

  // 1) 校验仓库与权限
  await ghRetry(
    async () =>
      ghJson(
        await ghRequest({ urlPath: `/repos/${repo.owner}/${repo.name}`, token }),
        "读取仓库",
      ),
    "读取仓库",
    1,
  );
  log("仓库可达、token 有效");

  // 2) 建 release / 复用
  let rel = null;
  const found = await ghRequest({
    urlPath: `/repos/${repo.owner}/${repo.name}/releases/tags/${encodeURIComponent(tag)}`,
    token,
  });
  if (found.status === 200) {
    rel = JSON.parse(found.text);
    if (opts.replaceRelease) {
      log(`已存在 tag ${tag} 的发行（id=${rel.id}），--replace-release：删除后重建`);
      const del = await ghRequest({
        method: "DELETE",
        urlPath: `/repos/${repo.owner}/${repo.name}/releases/${rel.id}`,
        token,
      });
      if (del.status >= 300) throw ghError(del, "删除既有发行");
      rel = null;
    } else if (opts.resume) {
      log(`已存在 tag ${tag} 的发行（id=${rel.id}），--resume：续传缺失资产`);
    } else {
      die(
        `tag ${tag} 的发行已存在（id=${rel.id}）。请任选其一：\n` +
          `  ① --resume          续传（跳过已存在且一致的资产，只补差值）\n` +
          `  ② --replace-release 删除该发行后重建（tag 保留）\n` +
          `  ③ --tag <新 tag>    换一个 tag`,
      );
    }
  } else if (found.status !== 404) {
    throw ghError(found, "查询发行");
  }
  if (!rel) {
    rel = await ghRetry(
      async () =>
        ghJson(
          await ghRequest({
            method: "POST",
            urlPath: `/repos/${repo.owner}/${repo.name}/releases`,
            token,
            body: {
              tag_name: tag,
              target_commitish: opts.target || headSha,
              name: entry.title || `${PRODUCT} ${releaseId}`,
              body,
              draft: false,
              prerelease: !!entry.prerelease,
            },
          }),
          "创建发行",
        ),
      "创建发行",
    );
    log(`已创建发行：${rel.html_url}`);
  }

  // 3) 上传资产
  const listed = await ghJson(
    await ghRequest({ urlPath: `/repos/${repo.owner}/${repo.name}/releases/${rel.id}/assets`, token }),
    "列出资产",
  );
  const remote = new Map((listed || []).map((a) => [a.name, a]));
  for (const a of uploads) {
    const exist = remote.get(a.name);
    if (exist) {
      if (opts.replace) {
        log(`  ${a.name}：已存在 → --replace 先删除`);
        const del = await ghRequest({
          method: "DELETE",
          urlPath: `/repos/${repo.owner}/${repo.name}/releases/assets/${exist.id}`,
          token,
        });
        if (del.status >= 300) throw ghError(del, `删除资产 ${a.name}`);
      } else if (opts.resume) {
        const digestOk = exist.digest ? exist.digest === `sha256:${a.sha256}` : null;
        const same = digestOk === null ? exist.size === a.size : digestOk;
        log(`  ${a.name}：已存在（size ${exist.size}${exist.digest ? " + digest" : ""}）→ ${same ? "跳过" : "补齐"}`);
        if (same) {
          a.url = exist.browser_download_url || a.url;
          continue;
        }
        const del = await ghRequest({
          method: "DELETE",
          urlPath: `/repos/${repo.owner}/${repo.name}/releases/assets/${exist.id}`,
          token,
        });
        if (del.status >= 300) throw ghError(del, `删除不一致资产 ${a.name}`);
      } else {
        die(
          `资产 ${a.name} 已存在于该发行。请任选其一：--resume（跳过一致项）/ --replace（先删再传）`,
        );
      }
    }
    const p = path.join(a._dir, a.name);
    log(`  上传 ${a.name}（${humanSize(a.size)}）…`);
    const res = await ghRetry(async () => {
      const r = await ghUploadAsset({ repo, token, releaseId: rel.id, name: a.name, file: p });
      // 5xx / 网络抖动 → 抛错以触发重试；4xx 交给 ghJson 报中文错误
      if (r.status >= 500) throw new Error(`上传 ${a.name} 失败：HTTP ${r.status}`);
      return r;
    }, `上传 ${a.name}`);
    const up = ghJson(res, `上传 ${a.name}`);
    if (up.digest && up.digest !== `sha256:${a.sha256}`) {
      die(`资产 ${a.name} 上传后 digest 不符（本地 ${a.sha256.slice(0, 12)}… / 远端 ${up.digest}）`);
    }
    if (up.browser_download_url) {
      if (up.browser_download_url !== a.url) {
        warn(`${a.name}：生效 URL 与预测不一致，已按 API 返回值写入索引`);
        a.url = up.browser_download_url;
      }
    }
  }

  // 4) 回填 + 合并索引
  entry.assets = uploads.map(({ _dir, _label, ...rest }) => rest);
  entry.page = rel.html_url || entry.page;
  entry.commit = headSha;
  entry.target = opts.target || entry.target;
  entry.publishedAt = opts.publishedAt || entry.publishedAt || todayLocal();
  entry.title = entry.title || `${PRODUCT} ${releaseId}`;
  if (opts.noUpdateChain) entry.updateVersion = null;

  const exists = index.releases.find((r) => r.release === releaseId);
  if (exists && !opts.resume && !opts.replaceRelease) {
    log(`索引中已有同名发行 ${releaseId}，按 upsert 覆盖其条目`);
  }
  if (entry.updateVersion) {
    const dup = index.releases.find((r) => r.release !== releaseId && r.updateVersion === entry.updateVersion);
    if (dup) {
      if (!opts.replaceVersion) {
        die(
          `契约版本 ${entry.updateVersion} 已被发行 ${dup.release} 占用（更新器按版本号定位，重复会导致行为不确定）。\n` +
            `  请任选其一：① bump 版本号后重新 pack；② --replace-version（把更新链指向本次发行）`,
        );
      }
      warn(`契约版本 ${entry.updateVersion} 原本指向发行 ${dup.release}，已被本次发行接管（--replace-version）`);
    }
  }
  index.releases = [...index.releases.filter((r) => r.release !== releaseId), entry];
  index.releases.sort((a, b) => cmpVersion(b.release, a.release));
  if (opts.minSupported) {
    const cur = index.minSupported;
    if (cur && cmpVersion(opts.minSupported, cur) < 0) {
      die(`minSupported 只升不降（当前 ${cur}，收到 ${opts.minSupported}）；确需回退请手工改索引`);
    }
    // ⚠️ 强制更新必须推送**含 exe** 的包，否则用户被阻断后仍拿不到新程序（launcher-auto-update §5.5.2）。
    const upd = (entry.assets ?? []).find((a) => a.role === "update");
    if (upd && upd.name.toLowerCase().endsWith(".zip")) {
      const info = fs.existsSync(path.join(dir, "release.json"))
        ? readJson(path.join(dir, "release.json"), "release.json")
        : null;
      const entries = info?.artifacts?.portable?.entries;
      if (!entries) {
        warn("无法确认更新包是否含 exe（release.json 缺 artifacts.portable.entries）—— 强制更新必须含 exe");
      } else if (!entries.includes(EXE_NAME)) {
        die(
          `--min-supported（强制更新）要求本次更新包**含 ${EXE_NAME}**，但 portable 包内没有它。\n` +
            `  纯配置包达不到"强制升级"的目的；请改用 pack 产出的含 exe 便携包。`,
        );
      }
    }
    index.minSupported = opts.minSupported;
  }
  refreshProjection(index);
  writeJsonAtomic(INDEX_PATH, index);

  console.log("");
  log(`已发布：${rel.html_url}`);
  log(`索引已更新：${INDEX_REL}（latest=${index.latest} / versions=${index.versions.length} / releases=${index.releases.length}）`);
  console.log("");
  log("🔴 下一步（推送后对所有用户生效）：");
  log("  git add releases/index.json && git commit -m \"release: <版本>\" && git push");
  log("  提示：raw CDN 有分钟级缓存；启动器侧另有 24h TTL（设置页「检查更新」会立即拉取）。");
}

/* ────────────────────────────── 子命令：preview（本地预演，不推送） ────────────────────────────── */

/** 扫 `releases/<dir>/index.entry.json` →（发行标识 ⇒ 本地目录名）：只有这些发行能由本地服务提供 */
function localPackDirs() {
  const root = path.join(ROOT, "releases");
  const map = new Map();
  if (!fs.existsSync(root)) return map;
  for (const e of fs.readdirSync(root, { withFileTypes: true })) {
    if (!e.isDirectory()) continue;
    const f = path.join(root, e.name, "index.entry.json");
    if (!fs.existsSync(f)) continue;
    try {
      const r = JSON.parse(fs.readFileSync(f, "utf8"))?.release;
      if (r?.release) map.set(r.release, e.name);
    } catch {
      warn(`跳过坏片段：releases/${e.name}/index.entry.json`);
    }
  }
  return map;
}

/** 预演静态服务：以 `releases/` 为根（`/index.local.json` + `/<dir>/<asset>`） */
function serveLocal({ port }) {
  const root = path.resolve(ROOT, "releases");
  const MIME = {
    ".json": "application/json; charset=utf-8",
    ".zip": "application/zip",
    ".exe": "application/octet-stream",
    ".msi": "application/x-msi",
    ".md": "text/markdown; charset=utf-8",
    ".txt": "text/plain; charset=utf-8",
  };
  return new Promise((resolve, reject) => {
    const server = http.createServer((req, res) => {
      let rel;
      try {
        rel = decodeURIComponent(new URL(req.url, `http://127.0.0.1:${port}`).pathname).replace(/^\/+/, "");
      } catch {
        res.writeHead(400).end("bad request");
        return;
      }
      const file = path.resolve(root, rel);
      if (file !== root && !file.startsWith(root + path.sep)) {
        res.writeHead(403).end("forbidden");
        return;
      }
      fs.stat(file, (err, st) => {
        if (err || !st.isFile()) {
          res.writeHead(404, { "content-type": "text/plain; charset=utf-8" }).end("404");
          return;
        }
        res.writeHead(200, {
          "content-type": MIME[path.extname(file).toLowerCase()] ?? "application/octet-stream",
          "content-length": st.size,
          // 预演期间索引与资产频繁变动：不返回 ETag + 禁用缓存，避免命中启动器侧的 24h TTL
          "cache-control": "no-store",
        });
        if (req.method === "HEAD") {
          res.end();
          return;
        }
        fs.createReadStream(file).pipe(res);
      });
    });
    server.on("error", reject);
    server.listen(port, "127.0.0.1", () => resolve(server));
  });
}

/**
 * 本地预演：生成**派生索引** `releases/index.local.json`（把下载地址改写到本地静态服务），
 * 可顺带起一个零依赖服务 —— 用于在 publish 之前跑通「旧版 → 发现更新 → 下载 → 应用」整条链。
 *
 * ⚠️ 永不写 `releases/index.json`：索引里出现的必须可下载，未发布的包写进真索引 = 全网 404。
 */
async function cmdPreview(opts) {
  const releaseId = opts.version ? String(opts.version) : readCargoVersion();
  if (!opts.version) log(`未传 --version → 使用文件里的版本 ${releaseId}`);
  const index = loadIndex();
  if (!index) die(`${INDEX_REL} 尚未初始化。请先跑：npm run release:pack -- --version ${releaseId}`);

  const { dir, fragment } = findPackedEntry(releaseId, opts.dir);
  const entry = JSON.parse(JSON.stringify(fragment.release));
  const tag = opts.tag || entry.tag || `v${releaseId}`;
  entry.tag = tag;
  if (opts.title) entry.title = opts.title;
  if (opts.critical) entry.critical = true;
  if (opts.noUpdateChain) {
    entry.updateVersion = null;
    entry.prerelease = true;
  }
  entry.publishedAt = opts.publishedAt || entry.publishedAt || todayLocal();
  entry.title = entry.title || `${PRODUCT} ${releaseId}`;

  const port = Number(opts.port || DEFAULT_PREVIEW_PORT);
  const base = String(opts.base || `http://127.0.0.1:${port}`).replace(/\/+$/, "");
  // 本地形状：服务以 releases/ 为根 ⇒ 资产在 `/<dir>/<name>`
  const dl = { base, tpl: "{base}/{dir}/{name}" };

  const dirs = localPackDirs();
  const local = JSON.parse(JSON.stringify(index));
  local.releases = [...local.releases.filter((r) => r.release !== releaseId), entry];
  for (const r of local.releases) {
    const d = dirs.get(r.release);
    // 本地没有该发行的目录 → 保持索引里的原地址（只参与版本比较，latest 不会指向它）
    if (!d) continue;
    r.assets = (r.assets ?? []).map((a) => ({
      ...a,
      path: `${d}/${a.name}`,
      url: assetUrl(dl, r.tag, a.name, r.release, d),
    }));
    for (const a of r.assets) {
      if (!fs.existsSync(path.join(ROOT, "releases", d, a.name))) {
        warn(`本地缺少资产 releases/${d}/${a.name} —— 预演时该地址会 404`);
      }
    }
  }
  // 明文 http 必须**显式放行**：更新器默认只认 https，且读的是**顶层** allowHttp
  // （不是 download.allowHttp —— src-tauri/src/updater/manifest.rs::download_scheme_ok）
  local.allowHttp = true;
  local.download = {
    ...(local.download ?? {}),
    base,
    pathTemplate: dl.tpl,
    note: "release:preview 派生索引：地址已改写到本地预演服务（勿提交）",
  };
  refreshProjection(local);
  local.preview = { generatedAt: nowIso(), release: releaseId, dir: path.basename(dir), base };
  writeJsonAtomic(LOCAL_INDEX_PATH, local);

  console.log("");
  log(`派生索引已生成：${LOCAL_INDEX_REL}（latest=${local.latest ?? "无"} / versions=${local.versions.length}）`);
  log(`真实索引 ${INDEX_REL} **未被修改**（版本条目仍只有 publish 能写）`);
  console.log("");
  log("被测启动器（一个**旧版本**的 release 包，且是 release 构建）→ 把「更新源」指向本地预演：");
  log("① 直接编辑 launcher.json（推荐，绕过 UI 的 https 限制）：");
  log("    文件：<启动器目录>/.vcmi-fv-launcher/launcher.json（exe 同级）");
  log(`    配置键：update.manifest_url = "${base}/index.local.json"`);
  log("    片段（合并进现有文件，不要整份替换；该文件是 JSON5，允许注释/尾逗号）：");
  log(`      { "update": { "manifest_url": "${base}/index.local.json" } }`);
  log("② 或用设置页（需先给启动器进程设 VCMI_FV_UPDATER_ALLOW_HTTP=1，UI 默认拒绝 http）：");
  log(`    设置页「更新源」填 ${base}/index.local.json —— 改完重启启动器让环境变量生效`);
  log("  然后点「检查更新」（force，绕过 24h TTL 缓存）");
  console.log("");
  log("前置条件（不满足就测不出东西）：");
  log("  ① 被测 exe 必须是 release 构建（debug 构建不检查、也不应用更新）");
  log(`  ② 被测 exe 版本必须低于 ${entry.updateVersion ?? "（本条目不进更新链）"}`);
  log("  ③ 同一包装过一次后不再提示：删 .vcmi-fv-launcher/state.json 里的 appliedUpdateSha / appliedUpdateVersion 再测");
  log("  ④ 更新后随包预置会把「更新源」改回官方地址 —— 第二轮测试要重新填一次");
  log("  ⑤ 环境若有 HTTP(S)_PROXY，本地 127.0.0.1 可能被代理吞掉：NO_PROXY=127.0.0.1,localhost");
  log("  完整端到端验收步骤见 docs/guides/release.zh-CN.md §9.1");
  if (!entry.updateVersion) {
    console.log("");
    warn("本条目 updateVersion 为空（--no-update-chain / --test）⇒ 不进 versions[]，**测不到更新提示**");
  }

  if (!opts.serve) {
    console.log("");
    log(`未带 --serve：请用任意静态服务以 releases/ 为根暴露（如 python -m http.server ${port} --directory releases）`);
    return null;
  }
  const server = await serveLocal({ port });
  log(`预演服务已启动：${base}（根 = releases/，Ctrl+C 停止）`);
  await new Promise(() => {}); // 服务句柄保活：进程不退出，直到 Ctrl+C
  return server;
}

/* ────────────────────────────── 子命令：verify ────────────────────────────── */

function readMigrationFloor() {
  const p = path.join(ROOT, "src-tauri", "src", "updater", "migrate.rs");
  if (!fs.existsSync(p)) return null;
  const m = fs.readFileSync(p, "utf8").match(/MIGRATION_FLOOR\s*:\s*&str\s*=\s*"([^"]+)"/);
  return m ? m[1] : null;
}

function deepEqual(a, b) {
  return JSON.stringify(a) === JSON.stringify(b);
}

async function cmdVerify(opts) {
  const errors = [];
  const warns = [];
  const err = (m) => errors.push(m);
  const wrn = (m) => warns.push(m);
  const ok = (m) => log(`✓ ${m}`);

  const index = loadIndex();
  if (!index) {
    log(`${INDEX_REL} 尚未初始化（首次 \`release:pack\` 会自举）。`);
    log("✓ 无索引可校验（不算失败）");
    return { errors, warns };
  }

  // 1) 结构
  const PLACEHOLDER_RE = /^(OWNER|REPO|TODO|<.+>)$/i;
  if (!index.repo?.owner || !index.repo?.name) err("repo.owner / repo.name 为空（URL 推导不可靠）");
  else if (PLACEHOLDER_RE.test(index.repo.owner) || PLACEHOLDER_RE.test(index.repo.name)) {
    err(
      `repo 仍是占位符（${index.repo.owner}/${index.repo.name}）：` +
        `请跑 \`npm run release:pack -- --owner <账号> --repo <仓库名>\` 完成自举（会一并重写随包预置）`,
    );
  } else ok(`repo：${index.repo.owner}/${index.repo.name}（${index.repo.defaultBranch}）`);
  if (index.schema !== SCHEMA) err(`schema 期望 ${SCHEMA}，实际 ${index.schema}`);
  // 1b) naming 模板（ADR 0007）：非法模板会在 pack 时才炸，这里提前拦
  if (index.naming) {
    try {
      const nm = resolveNaming(index, {}, index.latest || "0.0.0");
      ok(`naming 模板：release=${nm.release} / tag=${nm.tag} / title=${nm.title}（试算）`);
    } catch (e) {
      err(`naming 模板非法：${mask(e.message)}`);
    }
  }

  // 2) versions 契约
  if (!index.versions.length) wrn("versions[] 为空：启动器会静默认为「无更新」（尚未发布任何版本）");
  const seen = new Set();
  for (const v of index.versions) {
    const at = `versions[${v.version}]`;
    if (seen.has(v.version)) err(`${at}：版本号重复`);
    seen.add(v.version);
    if (!/^https:\/\//.test(v.download || "")) err(`${at}：download 不是 https 地址`);
    if (!/^[0-9a-f]{64}$/.test(v.sha256 || "")) err(`${at}：sha256 不是 64 位小写 hex`);
    if (!(v.size > 0)) err(`${at}：size 必须为正整数`);
    if (!v.format) err(`${at}：缺少 format（缺它会按单 exe 处理 zip → 启动失败）`);
    if (!v.notes || !Object.keys(v.notes).length) wrn(`${at}：没有更新说明（弹窗里会是空的）`);
    if (v.publishedAt && !/^\d{4}-\d{2}-\d{2}$/.test(v.publishedAt)) wrn(`${at}：publishedAt 不是 YYYY-MM-DD`);
  }
  if (!errors.length) ok(`versions[]：${index.versions.length} 条，字段齐备`);

  // 3) latest / 投影一致
  if (index.versions.length && index.latest !== index.versions[0].version) {
    err(`latest(${index.latest}) 与 versions[] 最大版本(${index.versions[0].version}) 不一致`);
  }
  // 3b) versions[] 必须严格递减（更新器只认 current < version；乱序/并列会让"是否有更新"失真）
  for (let i = 1; i < index.versions.length; i++) {
    if (cmpVersion(index.versions[i].version, index.versions[i - 1].version) >= 0) {
      err(
        `versions[] 第 ${i} 与第 ${i + 1} 条不满足严格递减：` +
          `${index.versions[i - 1].version} → ${index.versions[i].version}`,
      );
      break;
    }
  }
  let projected;
  try {
    projected = projectVersions(index);
    if (!deepEqual(projected, index.versions)) {
      err("versions[] 与 releases[] 的投影不一致（跑 `release:verify --fix` 重算）");
      for (let i = 0; i < Math.max(projected.length, index.versions.length); i++) {
        const a = index.versions[i];
        const b = projected[i];
        if (JSON.stringify(a) !== JSON.stringify(b)) {
          err(`  第 ${i + 1} 条差异：索引 ${a?.version ?? "(缺)"} / 投影 ${b?.version ?? "(缺)"}`);
          break;
        }
      }
    } else {
      ok("versions[] 与 releases[] 投影一致");
    }
  } catch (e) {
    err(`投影失败：${mask(e.message)}`);
  }

  // 4) releases 条目
  for (const r of index.releases) {
    const at = `releases[${r.release}]`;
    if (!NAME_RE.test(r.release || "")) err(`${at}：release 标识字符集非法`);
    if (!NAME_RE.test(r.tag || "")) err(`${at}：tag 字符集非法`);
    const ups = (r.assets ?? []).filter((a) => a.role === "update");
    if (r.updateVersion) {
      if (ups.length !== 1) err(`${at}：进更新链的发行必须有且仅有 1 个 role=update 资产（实际 ${ups.length}）`);
      if (ups[0] && !ups[0].name.toLowerCase().endsWith(".zip")) wrn(`${at}：更新资产不是 zip`);
    } else if (ups.length) {
      wrn(`${at}：不进更新链却标记了 role=update 资产`);
    }
    for (const a of r.assets ?? []) {
      if (!ASCII_RE.test(a.name || "")) err(`${at}：资产名含非 ASCII（${a.name}）`);
      if (!a.url?.startsWith("https://")) err(`${at}：资产 ${a.name} 的 url 不是 https`);
      if (!/^[0-9a-f]{64}$/.test(a.sha256 || "")) err(`${at}：资产 ${a.name} 的 sha256 非法`);
      if (!a.path) wrn(`${at}：资产 ${a.name} 缺少 path（GitHub 原生相对路径）`);
    }
    if (r.notesSource && !["changelog", "file", "cli"].includes(r.notesSource)) {
      wrn(`${at}：notesSource=${r.notesSource} 不认识`);
    }
  }
  if (!index.releases.length) wrn("releases[] 为空");

  // 5) minSupported
  if (index.minSupported) {
    if (cmpVersion(index.minSupported, index.latest || "0.0.0") > 0) {
      err(`minSupported(${index.minSupported}) 高于 latest(${index.latest})`);
    }
    const floor = readMigrationFloor();
    if (floor && cmpVersion(index.minSupported, floor) < 0) {
      wrn(`minSupported(${index.minSupported}) 低于代码里的 MIGRATION_FLOOR(${floor})：建议保持一致`);
    }
    if (floor && cmpVersion(index.minSupported, floor) > 0) {
      wrn(`minSupported(${index.minSupported}) 高于 MIGRATION_FLOOR(${floor})：确认已 bump 迁移地板`);
    }
  }

  // 6) 预置补丁一致性
  if (index.repo?.indexUrl) {
    const preset = resolvePreset(index, index.repo);
    for (const flavor of ["", ...Object.keys(preset.flavors)]) {
      const p = presetFileFor(flavor);
      if (!fs.existsSync(p)) {
        if (flavor) wrn(`缺少 ${path.relative(ROOT, p)}（索引声明了 flavors.${flavor}，但未生成补丁）`);
        continue;
      }
      const want = flavor ? presetForFlavor(preset, flavor) : preset.base;
      const have = readJson(p, path.relative(ROOT, p));
      if (!deepEqual(have, want)) {
        err(`${path.relative(ROOT, p)} 与索引 preset 不一致（跑 \`release:pack\` 重新生成）`);
      }
    }
    if (!errors.length) ok("随包预置补丁与索引 preset 一致");
  } else {
    wrn("索引缺 repo.indexUrl：跳过随包预置一致性校验");
  }

  // 7) ChangeLog 一致性
  const clPath = opts.changelog ? abs(opts.changelog) : DEFAULT_CHANGELOG;
  if (fs.existsSync(clPath)) {
    let cl = null;
    try {
      cl = parseChangeLog(fs.readFileSync(clPath, "utf8"), path.relative(ROOT, clPath));
      ok(`ChangeLog 解析通过：${cl.sections.size} 个版本节`);
    } catch (e) {
      err(`ChangeLog 解析失败：${mask(e.message)}`);
    }
    if (cl) {
      let missing = 0;
      for (const r of index.releases) {
        if (r.notesSource !== "changelog") continue;
        const sec = pickSection(cl, r.release, r.updateVersion);
        if (!sec) {
          missing++;
          continue;
        }
        if (!deepEqual(sec.langs, r.notes)) {
          err(`releases[${r.release}].notes 与 ChangeLog.md 不一致（跑 \`release:verify --fix\` 或重新 publish）`);
        }
      }
      if (missing) wrn(`索引里有 ${missing} 个版本在 ${path.relative(ROOT, clPath)} 中没有说明节（历史版本可忽略）`);
      if (!missing) ok("索引 notes 与 ChangeLog 一致");
    }
  } else {
    wrn(`${path.relative(ROOT, clPath)} 不存在（更新说明只能靠 --notes / --notes-file）`);
  }

  // 8) 本地片段
  const root = path.join(ROOT, "releases");
  const localDirs = fs.existsSync(root)
    ? fs.readdirSync(root, { withFileTypes: true }).filter((e) => e.isDirectory()).map((e) => e.name)
    : [];
  let unpub = 0;
  for (const d of localDirs) {
    const f = path.join(root, d, "index.entry.json");
    if (!fs.existsSync(f)) continue;
    try {
      const frag = JSON.parse(fs.readFileSync(f, "utf8"));
      const hit = index.releases.find((r) => r.release === frag.release?.release);
      if (!hit) unpub++;
      else if (!deepEqual(hit.assets, frag.release.assets)) wrn(`releases/${d} 的片段与索引条目不一致（索引可能已被 publish 更新）`);
    } catch {
      /* 已在 findPackedEntry 里提示过 */
    }
  }
  if (unpub) wrn(`有 ${unpub} 个已打包但未发布到索引的版本（跑 release:publish 或忽略）`);
  ok(`本地发行目录：${localDirs.length} 个`);

  // 9) --online
  if (opts.online) {
    for (const v of index.versions) {
      const a = index.releases.find((r) => r.updateVersion === v.version)?.assets?.find((x) => x.role === "update");
      if (!a) continue;
      try {
        const res = await httpGet(a.url, { method: "HEAD" });
        const len = Number(res.headers["content-length"] ?? 0);
        if (res.status !== 200) err(`在线探测 ${v.version}：HTTP ${res.status}（${a.url}）`);
        else if (len && len !== a.size) err(`在线探测 ${v.version}：体积不符（远端 ${len} / 索引 ${a.size}）`);
        else ok(`在线探测 ${v.version}：200 / ${humanSize(len || a.size)}`);
      } catch (e) {
        err(`在线探测 ${v.version} 失败：${mask(e.message)}`);
      }
      // 配了 CDN（download.base ≠ repo.releaseBase）→ **双探**：CDN 生效时也要确认原生地址可用
      // （便于"CDN 挂掉回落到 GitHub"以及审计两端内容是否同步；原生地址不可达只告警，不算失败）
      const nativeBase = (index.repo?.releaseBase || "").replace(/\/+$/, "");
      const dlBase = (index.download?.base || index.repo?.releaseBase || "").replace(/\/+$/, "");
      if (nativeBase && a.path && dlBase !== nativeBase) {
        const nativeUrl = `${nativeBase}/${a.path}`;
        try {
          const r2 = await httpGet(nativeUrl, { method: "HEAD" });
          if (r2.status !== 200) wrn(`原生地址探测 ${v.version}：HTTP ${r2.status}（${nativeUrl}）`);
          else ok(`原生地址探测 ${v.version}：200（${nativeBase.split("/")[2]}）`);
        } catch (e) {
          wrn(`原生地址探测 ${v.version} 失败：${mask(e.message)}`);
        }
      }
    }
    if (index.repo?.indexUrl) {
      try {
        const bust = index.repo.indexUrl + (index.repo.indexUrl.includes("?") ? "&" : "?") + `t=${Date.now()}`;
        const res = await httpGet(bust);
        if (res.status !== 200) {
          wrn(`远端索引拉取失败：HTTP ${res.status}`);
        } else {
          const remote = JSON.parse(res.body.toString("utf8"));
          const localIds = new Set(index.releases.map((r) => r.release));
          const remoteIds = new Set((remote.releases ?? []).map((r) => r.release));
          const ahead = [...localIds].filter((x) => !remoteIds.has(x));
          const behind = [...remoteIds].filter((x) => !localIds.has(x));
          if (ahead.length) wrn(`本地索引领先远端 ${ahead.length} 个版本（${ahead.join(" / ")}）—— 记得 push`);
          if (behind.length) wrn(`远端索引领先本地 ${behind.length} 个版本（${behind.join(" / ")}）—— 记得 pull`);
          if (!ahead.length && !behind.length) ok("远端索引与本地一致");
        }
      } catch (e) {
        wrn(`远端索引比对失败：${mask(e.message)}`);
      }
    }
  }

  // --fix：只重建派生物（投影 + ChangeLog 派生的 notes），绝不改档案字段
  let didFix = false;
  if (opts.fix && errors.length) {
    let fixedNotes = 0;
    try {
      if (index.releases.some((r) => r.notesSource === "changelog") && fs.existsSync(clPath)) {
        const cl = parseChangeLog(fs.readFileSync(clPath, "utf8"), path.relative(ROOT, clPath));
        for (const r of index.releases) {
          if (r.notesSource !== "changelog") continue;
          const sec = pickSection(cl, r.release, r.updateVersion);
          if (sec && !deepEqual(sec.langs, r.notes)) {
            r.notes = sec.langs;
            fixedNotes++;
          }
        }
      }
      refreshProjection(index);
      writeJsonAtomic(INDEX_PATH, index);
      didFix = true;
      log(`--fix：已重算派生物并写回 ${INDEX_REL}（notes 修正 ${fixedNotes} 条 + versions[] 重投影）`);
    } catch (e) {
      err(`--fix 失败：${mask(e.message)}`);
    }
  }

  console.log("");
  for (const w of warns) warn(w);
  for (const e of errors) console.error(`[release] ✗ ${e}`);
  log(`校验结果：${errors.length ? `${errors.length} 个错误` : "无错误"}${warns.length ? `，${warns.length} 个提示` : ""}`);
  return { errors, warns, didFix };
}

/* ────────────────────────────── 子命令：status ────────────────────────────── */

function cmdStatus() {
  const index = loadIndex();
  if (!index) {
    log(`${INDEX_REL} 尚未初始化（首次 \`release:pack\` 会自举配置区块）。`);
  } else {
    console.log("");
    log(`索引 ${INDEX_REL}：latest=${index.latest ?? "(无)"}  minSupported=${index.minSupported ?? "-"}  versions=${index.versions.length}`);
    if (index.repo?.owner) log(`repo：${index.repo.owner}/${index.repo.name}@${index.repo.defaultBranch}`);
    // 当前有效版本 + 该版本会用的命名（零参数构建时用的就是这些值）
    try {
      const eff = readCargoVersion();
      const nm = resolveNaming(index, {}, eff);
      log(`当前有效版本：${eff}（取自文件） →  release=${nm.release}  tag=${nm.tag}  title=${nm.title}`);
    } catch (e) {
      warn(`无法试算命名：${mask(e.message)}`);
    }
    if (index.releases.length) {
      console.log("");
      console.log(
        ["发行".padEnd(14), "tag".padEnd(18), "更新链".padEnd(10), "资产".padEnd(6), "说明"].join(" "),
      );
      console.log("─".repeat(72));
      for (const r of [...index.releases].sort((a, b) => cmpVersion(b.release, a.release))) {
        const notes = Object.keys(r.notes ?? {}).join("/") || "-";
        console.log(
          [
            String(r.release).padEnd(14),
            String(r.tag).padEnd(18),
            String(r.updateVersion ?? "—").padEnd(10),
            String((r.assets ?? []).length).padEnd(6),
            `${notes}  ${r.publishedAt || ""}`,
          ].join(" "),
        );
      }
    }
  }

  const root = path.join(ROOT, "releases");
  const dirs = fs.existsSync(root)
    ? fs.readdirSync(root, { withFileTypes: true }).filter((e) => e.isDirectory()).map((e) => e.name).sort()
    : [];
  console.log("");
  log(`本地发行目录（releases/）：${dirs.length} 个`);
  for (const d of dirs) {
    const f = path.join(root, d, "index.entry.json");
    if (!fs.existsSync(f)) {
      console.log(`  ${d.padEnd(28)} 无条目片段（旧产物 / 非 pack 产出）`);
      continue;
    }
    try {
      const frag = JSON.parse(fs.readFileSync(f, "utf8"));
      const rel = frag.release ?? {};
      const inIndex = index?.releases?.some((r) => r.release === rel.release);
      console.log(
        `  ${d.padEnd(28)} release=${rel.release}  updateVersion=${rel.updateVersion ?? "—"}  ` +
          `${(rel.assets ?? []).length} 个资产  ${inIndex ? "已在索引" : "⚠️ 未发布"}`,
      );
    } catch {
      console.log(`  ${d.padEnd(28)} 条目片段解析失败`);
    }
  }
}

/* ────────────────────────────── CLI ────────────────────────────── */

const USAGE = `
VCMI-FV 启动器发行工具（唯一入口）

用法：
  node scripts/release.mjs pack     [选项]    本地发包（构建 → 提取 → 预置补丁 → 条目片段）
  node scripts/release.mjs publish  [选项]    发布到 GitHub Releases 并更新 releases/index.json
  node scripts/release.mjs preview  [选项]    本地预演：派生索引 + 静态服务（不推送、不写真索引）
  node scripts/release.mjs verify   [选项]    索引契约自检
  node scripts/release.mjs status             索引版本 vs 本地已打包目录

pack：
  --version <semver>     本次发行的**契约版本**（ADR 0007）：缺省 = Cargo.toml 里的版本；
                         与文件不同时，**构建前写入** Cargo.toml + tauri.conf.json（持久，需随发行提交）
  --temp-version         临时注入：构建后自动还原版本文件（零源码改动；tag 复现需带 --version）
  --test                 一键测试包 = --temp-version + --no-update-chain + --no-bundle
                         + 自动版本 <现版本>-dev.<日期>（不改源码、不进更新链、不出安装包）
  --tag <t>              tag（缺省 v<release>；可用索引 naming.tag 模板）
  --title <t>            Release title（缺省 "<product> <release>"；可用索引 naming.title 模板）
  --allow-downgrade      允许版本不高于索引里的最新版（默认拒绝：更新器只认 current < version）
  --allow-tag-mismatch   HEAD 停在某个 tag 上但与本 tag 不一致时放行（默认拒绝，防 CI 发错版本）
  --flavor <ai>          构建 AI 版（含组件；作为同一 Release 的附件发布）
  --no-build             跳过构建，只提取 src-tauri/target 里的现有产物（与版本注入互斥）
  --no-bundle            不出 NSIS/MSI 安装包（⚠️ Windows 安装包只支持**纯数字**版本，
                         0.2.0-rc1 这类必须加本开关，否则 bundler 直接失败）
  --profile <p>          仅允许 release（缺省）

publish：
  --version <semver>     缺省 = 文件里的版本（定版后发布也可零参数）
  --dir <path>           指定本地发行目录（缺省按 --version 自动匹配）
  --attach <path>        附加上传目录（可多次；如 AI 版目录）
  --update-asset <name>  指定哪个资产进更新链（多包时必填）
  --resume               续传：跳过已存在且一致的资产
  --replace              同名资产先删再传
  --replace-release      同 tag 发行已存在时删除后重建
  --replace-version      允许覆盖索引中同契约版本的既有条目
  --min-supported <v>    抬高强制更新门槛（只升不降）
  --dry-run              只打印计划，不联网、不写盘

preview（本地预演：publish 之前跑通「发现更新 → 下载 → 应用」整条链）：
  --version <semver>     缺省 = 文件里的版本
  --dir <path>           指定本地发行目录（缺省按 --version 自动匹配）
  --serve                同时起内置静态服务（根 = releases/）
  --port <n>             端口（缺省 8765）
  --base <url>           只生成派生索引、改用这个 base（自带 nginx / python 静态服务时）
  ⚠️ 只写 releases/index.local.json（git 已忽略），**永不**写 releases/index.json

说明（pack / publish / preview 通用）：
  --notes <文本>         （最高优先级）更新说明，";" 分行
  --notes-lang <lang>    配合 --notes，缺省 chinese
  --notes-file <path>    .md（同 ChangeLog 语法）或 .json（{"<lang>": ["…"]}）
  --body-file <md>        GitHub Release 正文（缺省 = ChangeLog 节原文）
  --body-lang <lang>     正文只取该语言
  --critical             标为重要更新（与 --no-update-chain 互斥）
  --no-update-chain      不进自动更新链（预发行/手工分发）
  --allow-empty-notes    允许空更新说明（默认报错）
  --changelog <path>     指定 ChangeLog（缺省仓库根 ChangeLog.md）
  --published-at <date>  发布日期（YYYY-MM-DD）
  --target <sha|branch>  target_commitish（缺省当前 HEAD sha）

仓库 / 凭据：
  --owner <账号>  --repo <仓库名>  --branch <分支>  --provider <github|gitee>
                                    （缺省 github；publish 目前只支持 github，gitee 仅用于自建下载源）
  --token <t> / --token-file <path>（缺省读 GITHUB_TOKEN / GH_TOKEN / .env.local）

verify：
  --online               匿名探测下载地址 + 比对远端索引
  --fix                  重算派生物（versions[] 投影 + ChangeLog 派生的 notes）后写回

CI / 换机器（版本、仓库、说明、预置全部来自已提交的文件 ⇒ 零参数）：
  git checkout <tag> && npm run release:pack        # 正式版
  npm run release:pack -- --test                    # 本地测试包（不改源码、不进更新链）
`;

function parseArgs(argv) {
  const BOOL = new Set([
    "critical",
    "no-update-chain",
    "allow-empty-notes",
    "no-build",
    "no-bundle",
    "temp-version",
    "allow-downgrade",
    "allow-tag-mismatch",
    "test",
    "dry-run",
    "resume",
    "replace",
    "replace-release",
    "replace-version",
    "online",
    "fix",
    "serve",
    "help",
  ]);
  const VALUE = new Set([
    "version",
    "tag",
    "title",
    "flavor",
    "profile",
    "notes",
    "notes-lang",
    "notes-file",
    "body-file",
    "body-lang",
    "changelog",
    "published-at",
    "target",
    "owner",
    "repo",
    "branch",
    "token",
    "token-file",
    "provider",
    "dir",
    "attach",
    "update-asset",
    "min-supported",
    "port",
    "base",
  ]);
  const out = {
    cmd: null,
    notesLang: "chinese",
    flavor: "",
    profile: "release",
    attach: [],
  };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (!a.startsWith("--")) {
      if (!out.cmd) out.cmd = a;
      else die(`多余的位置参数：${a}`);
      continue;
    }
    const key = a.slice(2);
    const camel = key.replace(/-([a-z])/g, (_, c) => c.toUpperCase());
    if (BOOL.has(key)) {
      out[camel] = true;
    } else if (VALUE.has(key)) {
      const v = argv[++i];
      if (v === undefined) die(`参数 --${key} 缺少取值`);
      if (key === "attach") out.attach.push(v);
      else out[camel] = v;
    } else {
      die(`未知参数：${a}（用 --help 查看用法）`);
    }
  }
  return out;
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (!opts.cmd || opts.help || opts.cmd === "--help") {
    console.log(USAGE.trim());
    return;
  }
  switch (opts.cmd) {
    case "pack":
      cmdPack(opts);
      break;
    case "publish":
      await cmdPublish(opts);
      break;
    case "preview":
      await cmdPreview(opts);
      break;
    case "verify": {
      const first = await cmdVerify(opts);
      let errors = first.errors;
      // --fix 写回后**重新校验**：报告与退出码必须是修复后的真实状态（否则"修好了却报失败"）
      if (opts.fix && first.didFix) {
        log("--fix 已写回，重新校验…");
        errors = (await cmdVerify({ ...opts, fix: false })).errors;
      }
      if (errors.length) process.exitCode = 1;
      break;
    }
    case "status":
      cmdStatus();
      break;
    default:
      die(`未知子命令：${opts.cmd}（可用：pack / publish / preview / verify / status）`);
  }
}

try {
  await main();
} catch (e) {
  console.error(`[release] 失败：${mask(e?.message ?? e)}`);
  // ⚠️ 用 exitCode 而不是 process.exit()：后者会跳过 finally，留下未还原的临时注入
  process.exitCode = 1;
} finally {
  if (PENDING_TEMP_RESTORE) {
    warn("未走完正常流程：还原临时注入的版本文件");
    restoreVersionFiles({ quiet: true });
  }
}
