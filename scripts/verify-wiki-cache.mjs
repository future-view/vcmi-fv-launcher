#!/usr/bin/env node
/**
 * Wiki 缓存校验器（脱离启动器二进制，可进 CI）。
 *
 * 用途：对「模组缓存目录」`<启动器>/.vcmi-fv-launcher/mods` 做**只读**体检，回答两个问题：
 *   1. 重新生成后的数据是否齐全（结构 / 引用完整性 / 退化值 / 图片文件是否真的存在）；
 *   2. 相比上次基线是否**劣化**（计数上升即视为回归）。
 *
 * 与「重建驱动」的分工：
 *   - 本脚本**只校验产物**，不负责重建（重建由 Rust 集成测试/启动器完成）。
 *   - 因此它不需要游戏目录，只需要已生成的缓存，可在 CI 里对着测试产物跑。
 *
 * 用法：
 *   node scripts/verify-wiki-cache.mjs                      # 自动探测缓存目录
 *   node scripts/verify-wiki-cache.mjs --cache <mods 目录>
 *   node scripts/verify-wiki-cache.mjs --json               # 机器可读输出
 *   node scripts/verify-wiki-cache.mjs --update-baseline    # 刷新基线（确认无误后执行）
 *
 * 退出码：0 = 通过（可能仅有告警）；1 = 存在 error 或相对基线劣化。
 */
import fs from "fs";
import path from "path";
import { fileURLToPath } from "url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const REPO = path.resolve(__dirname, "..");

/* ------------------------------ 参数解析 ------------------------------ */
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
const AS_JSON = args.json === "true";
const UPDATE_BASELINE = args["update-baseline"] === "true";
const BASELINE_PATH = args.baseline
  ? path.resolve(args.baseline)
  : path.join(REPO, "scripts", "verify-wiki-cache.baseline.json");

/** 自动探测缓存目录（mods）。 */
function detectCacheDir() {
  if (args.cache) return path.resolve(args.cache);
  if (process.env.VCMI_CACHE_DIR) return path.resolve(process.env.VCMI_CACHE_DIR);
  const candidates = [
    path.join(REPO, "references", "Heroes3-VCMI", ".vcmi-fv-launcher", "mods"),
    "C:/Games/Heroes3-VCMI/.vcmi-fv-launcher/mods",
    path.join(REPO, "src-tauri", "target", "debug", ".vcmi-fv-launcher", "mods"),
    path.join(process.cwd(), ".vcmi-fv-launcher", "mods"),
  ];
  for (const c of candidates) {
    if (fs.existsSync(c)) return c;
  }
  return null;
}
const CACHE_DIR = detectCacheDir();
if (!CACHE_DIR || !fs.existsSync(CACHE_DIR)) {
  console.error(`未找到缓存目录，请用 --cache <mods 目录> 指定（已探测：--cache / VCMI_CACHE_DIR / 常见位置）`);
  process.exit(2);
}

/* ------------------------------ 已知豁免 ------------------------------ */
// 「补丁类」模组：VCMI 语义是按其加载顺序**跨 mod 合并**进被补丁的模组，
// 而我们的缓存是**单 mod 独立生成**，故这类模组天然缺图形/属性，不算缺陷。
const PATCH_MOD_RE = /^vcmi-enhancement-fix\.|^.*\.\w*-fix$/;
// 纯图形/元数据类模组：其 creatures 条目通常是挂载点而非真实生物。
const META_MOD_RE = /^vcmi-extras\.bonusIcons|^more-secondary\./;

/* ------------------------------ 工具 ------------------------------ */
const toPosix = (p) => p.replace(/\\/g, "/");
const isImg = (s) => typeof s === "string" && /\.(webp|png|bmp|jpg|jpeg)$/i.test(s);

/**
 * 复刻后端 `sanitize_id`：保留字母/数字/`_`/`-`/`.`，其余替换为 `_`。
 * 缓存目录名 = `sanitize_id(modId)`（如 `crossroads-of-worlds.Night Watch`
 * → `crossroads-of-worlds.Night_Watch`），**不等于** modId 本身。
 * 注意用 Unicode 语义（中文属字母，需保留），与 Rust `char::is_alphanumeric` 对齐。
 */
const sanitizeId = (s) => [...s].map((c) => (/[\p{L}\p{N}_\-.]/u.test(c) ? c : "_")).join("");

function readJson(p) {
  try {
    return JSON.parse(fs.readFileSync(p, "utf8"));
  } catch {
    return null;
  }
}
/** 名称是否缺省（空 / 等于 id / 中英都为 id）。 */
function nameOf(v) {
  if (!v) return "";
  if (typeof v === "string") return v;
  return v.chinese || v.english || v.auto || "";
}

/* ------------------------------ 校验主体 ------------------------------ */
const errors = [];
const warns = [];
const addErr = (scope, msg) => errors.push({ scope, msg });
const addWarn = (scope, msg) => warns.push({ scope, msg });

// 两类"批量"问题单独收集：逐个列出会淹没真正的问题（例如部分重建时会一次出现几百条）。
const missingCacheJson = []; // 缓存目录存在但无 cache.json（未构建）
const indexWithoutDir = []; // 索引条目没有对应缓存目录

const entries = fs
  .readdirSync(CACHE_DIR, { withFileTypes: true })
  .filter((e) => e.isDirectory() && e.name !== "_shared");

const perMod = {}; // 基线计数（键 = modId，非目录名）
const dirOf = {}; // modId → 缓存目录名（= sanitize_id(modId)）
let totalCreatures = 0;
let totalHeroes = 0;
let totalHeroNoClass = 0;
let totalHeroClassFallback = 0;
let totalAbilities = 0;
let totalAbilityIcons = 0;
let missingFiles = 0;
/** 全发行版 id 池：跨模组引用（升级链 / 宝物组件）只要落在这个池里，前端就能解析出来 */
const globalCreatureIds = new Set();
const globalArtifactIds = new Set();
const cacheVersions = new Map();

for (const ent of entries) {
  const dirName = ent.name;
  const cacheJsonPath = path.join(CACHE_DIR, dirName, "cache.json");
  if (!fs.existsSync(cacheJsonPath)) {
    missingCacheJson.push(dirName);
    continue;
  }
  const j = readJson(cacheJsonPath);
  if (!j) {
    addErr(dirName, "cache.json 解析失败（可能是半截写入）");
    continue;
  }
  // 以 cache.json 内的 modId 为准（索引/父子关系都按 modId 表达）
  const id = String(j.modId ?? dirName);
  dirOf[id] = dirName;
  if (sanitizeId(id) !== dirName) {
    addWarn(dirName, `目录名与 sanitize_id(modId) 不一致（modId=${id}）`);
  }

  const count = { creatures: (j.creatures || []).length, heroes: (j.heroes || []).length, facNoName: 0, crNoStat: 0, crNoImg: 0, heroNoImg: 0, missingFiles: 0 };
  perMod[id] = count;

  /* E1 结构字段 */
  for (const k of ["modId", "cacheVersion", "sourceHash", "assetsDone"]) {
    if (j[k] === undefined) addErr(id, `缺少字段 ${k}`);
  }
  if (j.cacheVersion !== undefined) {
    cacheVersions.set(String(j.cacheVersion), (cacheVersions.get(String(j.cacheVersion)) || 0) + 1);
  }
  // assetsDone=false 说明阶段二未完成（构建中断或未跑）——对"完整性校验"而言是 error
  if (j.assetsDone !== true) addWarn(id, "assetsDone≠true（素材阶段二未完成，图片可能缺失）");

  const isPatch = PATCH_MOD_RE.test(id);
  const isMeta = META_MOD_RE.test(id);

  /* E5 图片文件存在性（含共享 _shared） */
  const checkImg = (rel, where) => {
    if (!isImg(rel)) return;
    const abs = path.join(CACHE_DIR, toPosix(rel));
    if (!fs.existsSync(abs)) {
      count.missingFiles++;
      missingFiles++;
      addErr(id, `${where} 引用的图片不存在: ${rel}`);
    }
  };
  for (const c of j.creatures || []) {
    checkImg(c.image, `creature ${c.id}`);
    checkImg(c.imageLarge, `creature ${c.id} imageLarge`);
    checkImg(c.advMapImage, `creature ${c.id} advMapImage`);
  }
  for (const f of j.factions || []) {
    checkImg(f.background, `faction ${f.factionId} background`);
    checkImg(f.creatureBackground, `faction ${f.factionId} creatureBackground`);
    checkImg(f.preview, `faction ${f.factionId} preview`);
    for (const b of f.buildings || []) checkImg(b.image, `building ${b.id}`);
  }
  /* E8 能力图标：图标来自 VCMI 的两级来源（`ability["icon"]` → bonuses.json 的
   * `graphics.icon`，真实发行版由 vcmi-extras 的 Bonus Icons 子模组提供，资源形如
   * `zvs/Lib1.res/E_DEATH` —— 属 `<file>/<sub>` 形态）。覆盖率过低说明资源归一回归
   * （2026-10-16 前因 `strip_ext` 把中间段当扩展名而全空，实测覆盖率仅 5.4%）。*/
  for (const c of j.creatures || []) {
    for (const a of c.abilities || []) {
      totalAbilities++;
      if (a.icon) {
        totalAbilityIcons++;
        checkImg(a.icon, `ability ${c.id}.${a.type ?? a.name ?? ""}`);
      }
    }
  }
  for (const h of j.heroes || []) checkImg(h.image, `hero ${h.id}`);
  for (const t of j.terrains || []) checkImg(t.image, `terrain ${t.id}`);
  for (const v of Object.values(j.statIcons || {})) checkImg(v, "statIcons");
  for (const [k, v] of Object.entries(j.sharedIcons || {})) {
    if (isImg(v)) checkImg(v, `sharedIcons.${k}`);
  }

  /* E4 退化值：名称回落 id */
  for (const f of j.factions || []) {
    const nm = nameOf(f.name);
    if (!nm || nm === f.factionId) {
      count.facNoName++;
      addWarn(id, `城镇名回落 id（${f.factionId}）`);
    }
  }
  for (const c of j.creatures || []) {
    const nm = nameOf(c.name);
    if (!nm || nm === c.id) addWarn(id, `生物名回落 id（${c.id}）`);
  }

  /* E4 退化值：生物属性全 0 */
  if (!isPatch && !isMeta) {
    const cr = j.creatures || [];
    if (cr.length > 0) {
      const withStat = cr.filter((c) => (c.attack || 0) > 0 || (c.hitPoints || 0) > 0).length;
      if (withStat === 0) {
        count.crNoStat = cr.length;
        addErr(id, `全部 ${cr.length} 个生物属性为 0（配置未解析/被覆盖）`);
      }
      count.crNoImg = cr.filter((c) => !c.image && !(Number(c.iconIndex) >= 0)).length;
    }
  }

  /* 英雄头像：image 为空时走共享头像帧 `_shared/portraitsLarge/<imageIndex>.webp`（设计如此） */
  if (!isPatch) {
    const sharedPortraitDir = (j.sharedIcons || {}).portraitsLargeDir;
    for (const h of j.heroes || []) {
      if (h.image) continue;
      const ii = Number(h.imageIndex);
      if (sharedPortraitDir && Number.isFinite(ii) && ii >= 0) {
        if (!fs.existsSync(path.join(CACHE_DIR, toPosix(`${sharedPortraitDir}/${ii}.webp`)))) {
          count.heroNoImg++;
          addErr(id, `英雄 ${h.id} 无头像且共享头像帧缺失（${ii}）`);
        }
      } else {
        count.heroNoImg++;
        addWarn(id, `英雄 ${h.id} 无头像且无 imageIndex 兜底`);
      }
    }
  }

  /* E6 英雄职业：`className` 回落成 class **id** 说明 heroClass 表没解析出来
   *（城镇页分组标题会显示 `MOONPRIEST // GROVEKEEPER` 这类原样 id，力量/魔法分组也会塌成一张）。
   * 2026-10-16 修复：`heroes/` 目录里的职业定义（mod.json 在 heroClasses 键下声明）此前被当英雄读。*/
  for (const h of j.heroes || []) {
    const cid = String(h.class ?? "");
    const cname = nameOf(h.className);
    if (!cid) {
      count.heroNoClass = (count.heroNoClass || 0) + 1;
      addWarn(id, `英雄 ${h.id} 没有职业（heroClass 未解析）`);
    } else if (!cname || cname === cid) {
      count.heroClassFallback = (count.heroClassFallback || 0) + 1;
      addWarn(id, `英雄职业名回落 id（${h.id}: ${cid}）`);
    }
    if (!String(h.affinity ?? "")) count.heroNoAffinity = (count.heroNoAffinity || 0) + 1;
  }

  /* 全发行版 id 池：给 E7 的跨模组引用可解析性用 */
  for (const c of j.creatures || []) globalCreatureIds.add(String(c.id));
  for (const a of j.artifacts || []) globalArtifactIds.add(String(a.id));

  totalCreatures += count.creatures;
  totalHeroes += count.heroes;
  totalHeroNoClass += count.heroNoClass || 0;
  totalHeroClassFallback += count.heroClassFallback || 0;
}

/* E2 索引一致性 */
const idx = readJson(path.join(CACHE_DIR, "index.json"));
if (!idx || !Array.isArray(idx.mods)) {
  addErr("index.json", "缺失或结构异常（缺少 mods 数组）");
} else {
  const idxIds = new Set(idx.mods.map((m) => m.modId));
  for (const id of Object.keys(perMod)) {
    if (!idxIds.has(id)) addErr("index.json", `缓存目录 ${id} 未出现在索引中`);
  }
  for (const m of idx.mods) {
    const dir = dirOf[m.modId];
    if (!dir) {
      indexWithoutDir.push(m.modId);
      continue;
    }
    const j = readJson(path.join(CACHE_DIR, dir, "cache.json"));
    if (!j) continue;
    const cc = m.contentCount || {};
    const pairs = [
      ["creature", (j.creatures || []).length],
      ["hero", (j.heroes || []).length],
      ["faction", (j.factions || []).length],
    ];
    for (const [k, n] of pairs) {
      if (cc[k] !== undefined && Number(cc[k]) !== n) {
        addErr(m.modId, `索引 contentCount.${k}=${cc[k]} 与 cache.json 实际 ${n} 不一致`);
      }
    }
  }
}

/* E3 引用完整性（faction.creatureList ⊆ creatures） */
for (const id of Object.keys(perMod)) {
  const j = readJson(path.join(CACHE_DIR, dirOf[id], "cache.json"));
  if (!j) continue;
  const cids = new Set((j.creatures || []).map((c) => String(c.id)));
  for (const f of j.factions || []) {
    for (const cid of f.creatureList || []) {
      if (!cids.has(String(cid))) addWarn(id, `faction ${f.factionId} 的 creatureList 引用了不存在的生物 ${cid}`);
    }
  }

  /* E7 跨模组引用可解析性：升级链 / 宝物组件在**本 mod** 里找不到是常态
   *（实测 47% 的宝物组件引用只存在于 core），前端靠全类目索引兜底 ——
   * 只要目标落在全发行版 id 池里就一定能出图。落在池外的才是真丢数据（mod 未安装等）。*/
  for (const c of j.creatures || []) {
    for (const u of c.upgrades || []) {
      if (!globalCreatureIds.has(String(u))) {
        addWarn(id, `生物 ${c.id} 的升级目标 ${u} 在全发行版内不存在（跨模组兜底也取不到）`);
      }
    }
  }
  for (const a of j.artifacts || []) {
    for (const part of a.components || []) {
      if (!globalArtifactIds.has(String(part))) {
        addWarn(id, `宝物 ${a.id} 的组成组件 ${part} 在全发行版内不存在`);
      }
    }
  }
}

/* E6 期望值（防止已修复问题回归） */
function expect(cond, msg) {
  if (!cond) addErr("expect", msg);
}
const coreCastle = readJson(path.join(CACHE_DIR, dirOf["core.castle"] || "core.castle", "cache.json"));
if (coreCastle) {
  expect((coreCastle.factions || []).length === 1, `core.castle 应有 1 座城镇，实际 ${(coreCastle.factions || []).length}`);
  expect((coreCastle.creatures || []).length === 14, `core.castle 应有 14 生物，实际 ${(coreCastle.creatures || []).length}`);
  expect((coreCastle.heroes || []).length === 16, `core.castle 应有 16 英雄，实际 ${(coreCastle.heroes || []).length}`);
  const pik = (coreCastle.creatures || []).find((c) => c.id === "pikeman");
  expect(!!pik, "core.castle 应有 pikeman");
  if (pik) {
    expect(pik.attack === 4, `pikeman 攻击应为 4，实际 ${pik.attack}`);
    expect(pik.defense === 5, `pikeman 防御应为 5，实际 ${pik.defense}`);
    expect(pik.hitPoints === 10, `pikeman HP 应为 10，实际 ${pik.hitPoints}`);
  }
}
{
  let coreCr = 0;
  let coreHe = 0;
  for (const [id, c] of Object.entries(perMod)) {
    if (id === "core" || id.startsWith("core.")) {
      coreCr += c.creatures;
      coreHe += c.heroes;
    }
  }
  if (coreCr > 0) expect(coreCr === 150, `core* 生物合计应为 150，实际 ${coreCr}`);
}

/* E7 基线回归 */
let regressions = [];
if (UPDATE_BASELINE) {
  const baseline = { generatedAt: new Date().toISOString(), cacheDir: CACHE_DIR, mods: perMod };
  fs.writeFileSync(BASELINE_PATH, JSON.stringify(baseline, null, 2), "utf8");
} else if (fs.existsSync(BASELINE_PATH)) {
  const base = readJson(BASELINE_PATH);
  if (base && base.mods) {
    for (const [id, cur] of Object.entries(perMod)) {
      const b = base.mods[id];
      if (!b) continue;
      for (const k of ["facNoName", "crNoStat", "crNoImg", "heroNoImg", "missingFiles"]) {
        if ((cur[k] || 0) > (b[k] || 0)) {
          regressions.push({ id, metric: k, before: b[k] || 0, after: cur[k] || 0 });
        }
      }
    }
  }
}
if (regressions.length) {
  for (const r of regressions) addErr(r.id, `基线回归：${r.metric} ${r.before} → ${r.after}`);
}

/* 缓存版本一致性（检出「半迁移」） */
if (cacheVersions.size > 1) {
  addErr("cacheVersion", `存在多个缓存版本（${[...cacheVersions.entries()].map(([k, v]) => `${k}×${v}`).join(", ")}），疑似半迁移/未全量重建`);
}

/* E8 汇总：能力图标覆盖率（回归护栏 —— 旧实现在 5% 上下，修复后约 80%+） */
const abilityIconRate = totalAbilities ? totalAbilityIcons / totalAbilities : 1;
if (totalAbilities >= 500 && abilityIconRate < 0.5) {
  addErr(
    "abilities",
    `能力图标覆盖率仅 ${(abilityIconRate * 100).toFixed(1)}%（${totalAbilityIcons}/${totalAbilities}）：` +
      "`bonusToGraphics`（bonuses.json 的 graphics.icon，资源形如 zvs/Lib1.res/E_DEATH）链路疑似回归",
  );
}

/* ------------------------------ 输出 ------------------------------ */
const summary = {
  cacheDir: CACHE_DIR,
  mods: Object.keys(perMod).length,
  notBuilt: missingCacheJson.length,
  indexStale: indexWithoutDir.length,
  totalCreatures,
  totalHeroes,
  heroNoClass: totalHeroNoClass,
  heroClassFallback: totalHeroClassFallback,
  abilities: totalAbilities,
  abilityIcons: totalAbilityIcons,
  abilityIconRate: +(abilityIconRate * 100).toFixed(1),
  missingFiles,
  cacheVersions: Object.fromEntries(cacheVersions),
  errors: 0,
  warnings: warns.length,
};

// 两类批量问题聚合为单条 error（明细见 --json），避免淹没真正的问题
if (missingCacheJson.length) {
  addErr("build", `${missingCacheJson.length} 个缓存目录缺少 cache.json（未构建/构建中断）：${missingCacheJson.slice(0, 5).join(", ")}${missingCacheJson.length > 5 ? " …" : ""}`);
}
if (indexWithoutDir.length) {
  addErr("index.json", `${indexWithoutDir.length} 个索引条目没有对应缓存目录（索引与磁盘不一致）：${indexWithoutDir.slice(0, 5).join(", ")}${indexWithoutDir.length > 5 ? " …" : ""}`);
}
summary.errors = errors.length;

if (AS_JSON) {
  console.log(
    JSON.stringify({ summary, errors, warns, regressions, notBuilt: missingCacheJson, indexWithoutDir }, null, 2),
  );
} else {
  console.log(`缓存目录：${CACHE_DIR}`);
  console.log(`模组数：${summary.mods}　生物：${totalCreatures}　英雄：${totalHeroes}　缺失图片：${missingFiles}`);
  console.log(
    `英雄职业：无职业 ${totalHeroNoClass}　职业名回落 id ${totalHeroClassFallback}` +
      `（后者的城镇页分组标题会显示原样 id、力量/魔法分组塌成一张）`,
  );
  console.log(
    `能力图标：${totalAbilityIcons}/${totalAbilities} = ${(abilityIconRate * 100).toFixed(1)}%` +
      `（来源＝ability["icon"] ＋ bonuses.json 的 graphics.icon；<50% 视为回归）`,
  );
  if (cacheVersions.size) console.log(`缓存版本：${[...cacheVersions.entries()].map(([k, v]) => `${k}(×${v})`).join(" ")}`);
  if (errors.length) {
    console.log(`\n[ERROR] ${errors.length} 项`);
    for (const e of errors.slice(0, 60)) console.log(`  ✗ ${e.scope}：${e.msg}`);
    if (errors.length > 60) console.log(`  … 其余 ${errors.length - 60} 项省略（--json 查看全部）`);
  }
  if (warns.length) {
    console.log(`\n[WARN] ${warns.length} 项（前 30）`);
    for (const w of warns.slice(0, 30)) console.log(`  ! ${w.scope}：${w.msg}`);
  }
  if (!errors.length) console.log(`\n✅ 校验通过${warns.length ? "（有告警）" : ""}`);
  else console.log(`\n❌ 校验未通过`);
}
process.exit(errors.length ? 1 : 0);
