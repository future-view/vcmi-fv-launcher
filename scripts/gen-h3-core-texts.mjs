/**
 * 从任意 H3 游戏目录（各语言版本）提取原版文案，输出 JSON 到 src-tauri/data/h3-core/。
 *
 * 解析规则复刻 VCMI `lib/texts/CLegacyConfigParser.cpp` 与各 handler 的 `loadLegacyData()`：
 *   - 字段以 \t 分隔，行尾 \r\n（\r 终止字符串、\n 终止行）
 *   - " 包裹的字段可含 \t 与换行，"" 表示一个字面引号
 *   - 空 entry（跳过空格后为 \n/\r/\t/EOF）需跳过
 *   - 数字支持逗号小数点
 *
 * 用法：
 *   node scripts/gen-h3-core-texts.mjs --game C:/Games/Heroes3-EN [--lang en] [--encoding cp1252]
 *   node scripts/gen-h3-core-texts.mjs --game C:/Games/Heroes3-VCMI \
 *        --config C:/Games/Heroes3-VCMI/config --lang chinese --encoding gbk
 *
 * 参数：
 *   --game      H3 游戏目录（需含 Data/H3bitmap.lod），必需
 *   --config    VCMI config 目录（提供实体 id→index 映射）。缺省用 references/vcmi/config（标准 SOD）
 *   --lang      输出语言键（en / chinese / ...）。缺省按 --encoding 与目录名推断
 *   --encoding  TXT 编码（gbk / cp1252 / utf8 / ...）。缺省自动探测
 *   --out       输出目录，默认 src-tauri/data/h3-core
 */
import fs from "fs";
import path from "path";
import zlib from "zlib";
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
if (!args.game) {
  console.error("缺少 --game <H3 游戏目录>");
  process.exit(1);
}
const GAME_DIR = path.resolve(args.game);
const CONFIG_DIR = path.resolve(args.config || path.join(REPO, "references", "vcmi", "config"));
const OUT_DIR = path.resolve(args.out || path.join(REPO, "src-tauri", "data", "h3-core"));
// 人工撰写的原版城镇描述（来自百科，中英双语），由 h3-core 注入到 `faction.core.<id>.description`。
// 独立于生成产物（`data/h3-core/<lang>.factions.json`）保存，保证重跑脚本不会覆盖。
const DESC_PATH = path.resolve(REPO, "src-tauri", "data", "h3-core-src", "factionDescriptions.json");
const ENCODING = args.encoding || "auto";

/* ------------------------------ LOD 读取 ------------------------------ */

/**
 * LOD 档案读取。注意：部分条目 compressedSize == fullSize 但内容仍是 zlib 流
 * （如 TERRNAME.TXT），因此必须按 magic 探测，不能只信标记位。
 */
class LodArchive {
  constructor(filePath) {
    this.path = filePath;
    this.entries = new Map();
    const data = fs.readFileSync(filePath);
    const total = data.readUInt32LE(8);
    let pos = 0x5c;
    for (let i = 0; i < total && pos + 32 <= data.length; i++) {
      const nameBytes = data.slice(pos, pos + 16);
      const zero = nameBytes.indexOf(0);
      const name = nameBytes.slice(0, zero < 0 ? 16 : zero).toString("utf8").toUpperCase();
      const offset = data.readUInt32LE(pos + 16);
      const fullSize = data.readUInt32LE(pos + 20);
      const compressedSize = data.readUInt32LE(pos + 28);
      if (name) this.entries.set(name, { offset, fullSize, compressedSize });
      pos += 32;
    }
    this.data = data;
  }

  read(name) {
    const e = this.entries.get(String(name).trim().toUpperCase());
    if (!e) return null;
    const size = e.compressedSize > 0 ? e.compressedSize : e.fullSize;
    const buf = this.data.slice(e.offset, e.offset + size);
    // magic 探测优先于标记位
    if (buf.length >= 2 && buf[0] === 0x78 && (buf[1] === 0x9c || buf[1] === 0x01 || buf[1] === 0xda)) {
      try {
        return zlib.inflateSync(buf);
      } catch {
        /* 落到下一种尝试 */
      }
    }
    if (e.compressedSize > 0 && e.compressedSize !== e.fullSize) {
      try {
        return zlib.inflateSync(buf);
      } catch {
        /* ignore */
      }
    }
    return buf;
  }
}

/** 按候选文件名在游戏的 lod 中查找（H3bitmap.lod 优先，回退 H3ab_bmp.lod） */
function makeLodResolver(gameDir) {
  const archives = [];
  for (const name of ["H3bitmap.lod", "H3ab_bmp.lod"]) {
    const p = path.join(gameDir, "Data", name);
    if (fs.existsSync(p)) {
      try {
        archives.push(new LodArchive(p));
      } catch (err) {
        console.warn(`  ! 无法解析 ${name}: ${err.message}`);
      }
    }
  }
  return (fileName) => {
    for (const arch of archives) {
      const buf = arch.read(fileName);
      if (buf) return buf;
    }
    return null;
  };
}

/* ------------------------------ 编码解码 ------------------------------ */

function decodeBuffer(buf, enc) {
  if (enc === "auto") {
    // 中文版本常见：出现 GBK 双字节（首字节 0x81-0xFE 且次字节 0x40-0xFE）
    let gbkish = 0;
    const limit = Math.min(buf.length, 4096);
    for (let i = 0; i + 1 < limit; i++) {
      const b = buf[i];
      if (b >= 0x81 && b <= 0xfe && buf[i + 1] >= 0x40 && buf[i + 1] <= 0xfe) gbkish++;
    }
    enc = gbkish > 8 ? "gbk" : "cp1252";
  }
  try {
    return { text: new TextDecoder(enc).decode(buf), encoding: enc };
  } catch {
    return { text: buf.toString("latin1"), encoding: "latin1" };
  }
}

/* --------------------- CLegacyConfigParser 复刻 --------------------- */

class LegacyParser {
  constructor(text) {
    this.s = text;
    this.i = 0;
  }

  get end() {
    return this.s.length;
  }

  extractQuotedPart() {
    this.i++; // skip opening quote
    const begin = this.i;
    while (this.i < this.end && this.s[this.i] !== '"' && this.s[this.i] !== "\t") this.i++;
    const part = this.s.slice(begin, this.i);
    this.i++; // skip closing quote（或停在的 \t，与 C++ 一致）
    return part;
  }

  extractQuotedString() {
    let ret = "";
    for (;;) {
      ret += this.extractQuotedPart();
      if (this.i < this.end && this.s[this.i] === '"') {
        ret += '"';
      } else if (this.i < this.end && this.s[this.i] !== "\t" && this.s[this.i] !== "\r") {
        const begin = this.i;
        while (this.i < this.end && this.s[this.i] !== "\t" && this.s[this.i] !== "\r" && this.s[this.i] !== '"') {
          this.i++;
        }
        ret += this.s.slice(begin, this.i);
        if (this.i >= this.end || this.s[this.i] !== '"') return ret;
      } else {
        return ret;
      }
    }
  }

  extractNormalString() {
    const begin = this.i;
    while (this.i < this.end && this.s[this.i] !== "\t" && this.s[this.i] !== "\r") this.i++;
    return this.s.slice(begin, this.i);
  }

  readRawString() {
    if (this.i >= this.end || this.s[this.i] === "\n") return "";
    const ret = this.s[this.i] === '"' ? this.extractQuotedString() : this.extractNormalString();
    this.i++;
    return ret;
  }

  readString() {
    return this.readRawString();
  }

  readNumber() {
    const input = this.readRawString().trim();
    if (!input) return 0;
    const v = Number.parseFloat(input.replace(",", "."));
    return Number.isFinite(v) ? v : 0;
  }

  isNextEntryEmpty() {
    let j = this.i;
    while (j < this.end && this.s[j] === " ") j++;
    return j >= this.end || this.s[j] === "\n" || this.s[j] === "\r" || this.s[j] === "\t";
  }

  endLine() {
    while (this.i < this.end && this.s[this.i] !== "\n") this.readString();
    this.i++;
    return this.i < this.end;
  }

  skipLines(count) {
    for (let i = 0; i < count; i++) this.endLine();
  }
}

/* --------------------- VCMI config 读取（id→index） --------------------- */

/** 宽容 JSON：剥离 // 与 /* *\/ 注释（逐字符扫描，跟踪字符串状态，避免误伤字符串内的 //） */
function stripJsonComments(text) {
  let out = "";
  let inString = false;
  let escaped = false;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (inString) {
      out += c;
      if (escaped) escaped = false;
      else if (c === "\\") escaped = true;
      else if (c === '"') inString = false;
      continue;
    }
    if (c === '"') {
      inString = true;
      out += c;
    } else if (c === "/" && text[i + 1] === "/") {
      while (i < text.length && text[i] !== "\n") i++;
      if (i < text.length) out += "\n";
    } else if (c === "/" && text[i + 1] === "*") {
      i += 2;
      while (i < text.length && !(text[i] === "*" && text[i + 1] === "/")) i++;
      i++;
    } else {
      out += c;
    }
  }
  // 去尾逗号（JSON5 容忍）
  return out.replace(/,(\s*[}\]])/g, "$1");
}

function parseVcmiJson(text) {
  return JSON.parse(stripJsonComments(text));
}

function readJsonIfExists(p) {
  try {
    if (!fs.existsSync(p)) return null;
    return parseVcmiJson(fs.readFileSync(p, "utf8"));
  } catch (err) {
    console.warn(`  ! 解析失败 ${p}: ${err.message}`);
    return null;
  }
}

/** 从 config 目录收集 实体 index → id 映射（目录下的所有 json 文件按分类归并） */
function collectIndexMap(configDir, subDir) {
  const map = new Map();
  const dir = subDir ? path.join(configDir, subDir) : configDir;
  if (!fs.existsSync(dir)) return map;
  const files = subDir
    ? fs.readdirSync(dir).filter((f) => f.endsWith(".json"))
    : [];
  for (const f of files) {
    const j = readJsonIfExists(path.join(dir, f));
    if (!j) continue;
    for (const [id, val] of Object.entries(j)) {
      if (val && typeof val === "object" && Number.isFinite(val.index)) {
        map.set(val.index, id);
      }
    }
  }
  return map;
}

/** 单文件 config（artifacts.json / skills.json / terrains.json） */
function collectIndexMapSingle(configDir, file) {
  const map = new Map();
  const j = readJsonIfExists(path.join(configDir, file));
  if (!j) return map;
  for (const [id, val] of Object.entries(j)) {
    if (val && typeof val === "object" && Number.isFinite(val.index)) map.set(val.index, id);
  }
  return map;
}

/* ------------------------------ 常量表 ------------------------------ */

// NFaction::names（references/vcmi/lib/constants/StringConstants.h:74），末两个为 VCMI 扩展
const FACTION_ORDER = [
  "castle",
  "rampart",
  "tower",
  "inferno",
  "necropolis",
  "dungeon",
  "stronghold",
  "fortress",
  "conflux",
  "neutral",
  "random",
];

// EBuildingType::names（references/vcmi/lib/constants/StringConstants.h:57），索引即建筑 id
const BUILDING_NAMES = [
  "mageGuild1", "mageGuild2", "mageGuild3", "mageGuild4", "mageGuild5",
  "tavern", "shipyard", "fort", "citadel", "castle",
  "villageHall", "townHall", "cityHall", "capitol", "marketplace",
  "resourceSilo", "blacksmith", "special1", "horde1", "horde1Upgr",
  "ship", "special2", "special3", "special4", "horde2",
  "horde2Upgr", "grail", "extraTownHall", "extraCityHall", "extraCapitol",
  "dwellingLvl1", "dwellingLvl2", "dwellingLvl3", "dwellingLvl4", "dwellingLvl5",
  "dwellingLvl6", "dwellingLvl7", "dwellingUpLvl1", "dwellingUpLvl2", "dwellingUpLvl3",
  "dwellingUpLvl4", "dwellingUpLvl5", "dwellingUpLvl6", "dwellingUpLvl7", "dwellingLvl8",
  "dwellingUpLvl8",
];

/* ------------------------------ 提取器 ------------------------------ */

/** 生物：CRTRAITS.TXT */
function extractCreatures(resolve, encoding, indexMap) {
  const raw = resolve("CRTRAITS.TXT");
  if (!raw) return null;
  const { text } = decodeBuffer(raw, encoding);
  const p = new LegacyParser(text);
  p.endLine(); // header line 1
  let namesCount = 2;
  const s1 = p.readString();
  const s2 = p.readString();
  if (s1 !== "Singular" || s2 !== "Plural") {
    console.warn(`  ! CRTRAITS 表头异常: ${s1}/${s2}`);
  }
  if (p.readString() === "Plural2") namesCount = 3;
  p.endLine();

  const dataSize = indexMap.size ? Math.max(...indexMap.keys()) + 1 : 0;
  const out = {};
  for (let i = 0; i < dataSize; i++) {
    while (p.isNextEntryEmpty()) {
      if (!p.endLine()) break;
    }
    const singular = p.readString();
    if (namesCount === 3) p.readString();
    const plural = p.readString();
    // 7 项招募花费，顺序与 VCMI CCreatureHandler 一致：木/汞/矿/硫/晶/宝/金
    const costRes = ["wood", "mercury", "ore", "sulfur", "crystal", "gems", "gold"];
    const cost = {};
    costRes.forEach((k) => {
      const v = p.readNumber();
      if (v) cost[k] = v;
    });
    // 原版生物的**数值属性只存在于 CRTRAITS.TXT**（config/creatures/*.json 里没有
    // attack/hitPoints/speed/damage/cost 等字段），故必须随文案一并烘进 h3-core，
    // 否则 Wiki 里原版生物属性全为 0。
    const fightValue = p.readNumber();
    const aiValue = p.readNumber();
    const growth = p.readNumber();
    const horde = p.readNumber();
    const hitPoints = p.readNumber();
    const speed = p.readNumber();
    const attack = p.readNumber();
    const defense = p.readNumber();
    const damageMin = p.readNumber();
    const damageMax = p.readNumber();
    const shots = p.readNumber();
    p.readNumber(); // spellPoints（非 ROE）
    const advMin = p.readNumber();
    const advMax = p.readNumber();
    const abilityText = p.readString();
    p.endLine();

    const id = indexMap.get(i);
    if (!id) continue;
    out[id] = {
      name: singular,
      plural,
      abilityText,
      // 数值属性（语言无关）：由 Rust 侧在构建 core 生物时并入配置
      stats: {
        attack,
        defense,
        hitPoints,
        speed,
        shots,
        growth,
        horde,
        aiValue,
        combatValue: fightValue,
        damage: { min: damageMin, max: damageMax },
        cost,
        advMapAmount: { min: advMin, max: advMax },
      },
    };
  }
  return out;
}

/** 城镇：TOWNTYPE.TXT（名称）+ TOWNNAME.TXT（随机名） */
function extractFactions(resolve, encoding, lang) {
  const raw = resolve("TOWNTYPE.TXT");
  if (!raw) return null;
  const ORDER = FACTION_ORDER;
  const { text } = decodeBuffer(raw, encoding);
  const p = new LegacyParser(text);
  const out = {};
  let idx = 0;
  do {
    const name = p.readString();
    if (!name) break;
    const id = ORDER[idx] ?? `faction${idx}`;
    out[id] = { name };
    idx++;
  } while (p.endLine());

  // 随机城镇名（TOWNNAME.TXT）：每镇 NAMES_PER_TOWN 个
  const rawNames = resolve("TOWNNAME.TXT");
  if (rawNames) {
    const np = new LegacyParser(decodeBuffer(rawNames, encoding).text);
    let town = 0;
    const perTown = 16; // NAMES_PER_TOWN
    for (;;) {
      const names = [];
      for (let i = 0; i < perTown; i++) {
        const n = np.readString();
        if (n) names.push(n);
        if (!np.endLine()) break;
      }
      if (!names.length) break;
      const id = ORDER[town];
      if (id && out[id]) out[id].townNames = names;
      town++;
      if (np.i >= np.end) break;
    }
  }

  // 人工撰写的原版城镇描述（来自百科，中英双语），注入为 `description`。
  // 与 `resources`/`spellSchools` 的官方译名同理，走独立的 `factionDescriptions.json`，
  // 由脚本按当前语言挑选 `en`/`zh` 写入，避免直接改生成产物被覆盖。
  let descMap = {};
  try {
    descMap = JSON.parse(fs.readFileSync(DESC_PATH, "utf8"));
  } catch {
    console.warn(`  ! 未找到城镇描述表 ${DESC_PATH}，跳过 description`);
  }
  for (const id of Object.keys(out)) {
    const d = descMap[id];
    if (d && typeof d === "object") {
      const text = d[lang] || d.en || "";
      if (text) out[id].description = text;
    }
  }
  return out;
}

/**
 * 由 creatures 配置 indexMap（index → VCMI id）构建「H3 生物名 → VCMI id」归一化映射。
 * 用于把 HOTRAITS.TXT 里的初始部队生物名（如 StoneGargoyle）解析为 wiki 用的 camelCase id
 * （如 stoneGargoyle），否则前端按 id 查不到生物、无法显示图标。
 * 原版生物的 VCMI id 即「H3 名去空格 + 驼峰」，故对两者都做小写去非字母数字归一化后相等即可匹配；
 * 不依赖 CRTRAITS 行序（增强版 creatures index 与 CRTRAITS 行序并不对齐）。
 */
function buildCreatureNameToId(creaturesIndexMap) {
  const out = new Map();
  const norm = (s) => String(s).toLowerCase().replace(/[^a-z0-9]/g, "");
  for (const id of creaturesIndexMap.values()) {
    out.set(norm(id), id);
  }
  return out;
}

/** 英雄：HOTRAITS.TXT（名）+ HEROBIOS.TXT（传记）+ HEROSPEC.TXT（特长） */
function extractHeroes(resolve, encoding, indexMap, creatureNameToId) {
  const rawTraits = resolve("HOTRAITS.TXT");
  if (!rawTraits) return null;
  const traits = new LegacyParser(decodeBuffer(rawTraits, encoding).text);
  traits.skipLines(2); // header

  const rawBio = resolve("HEROBIOS.TXT");
  const bio = rawBio ? new LegacyParser(decodeBuffer(rawBio, encoding).text) : null;

  const rawSpec = resolve("HEROSPEC.TXT");
  const spec = rawSpec ? new LegacyParser(decodeBuffer(rawSpec, encoding).text) : null;
  if (spec) spec.skipLines(2); // header

  const dataSize = indexMap.size ? Math.max(...indexMap.keys()) + 1 : 0;
  const out = {};
  for (let i = 0; i < dataSize; i++) {
    // HOTRAITS.TXT（增强版）列结构：
    //   第 0 列 Name，随后 3 组「Low \t High \t Type(creature)」即初始部队（无主属性列）。
    //   主属性（attack/defense/...）不在此文件中，由 VCMI 从 heroConfig.general / heroClass 派生，
    //   原版英雄配置里通常缺省 → wiki 中主属性回退为 0 属已知数据限制。
    const name = traits.readString();
    const biography = bio ? bio.readString() : "";
    const specName = spec ? spec.readString() : "";
    const specTooltip = spec ? spec.readString() : "";
    const specDescription = spec ? spec.readString() : "";
    const army = [];
    for (let x = 0; x < 3; x++) {
      const low = traits.readNumber();
      const high = traits.readNumber();
      let creature = traits.readString();
      if (creature && creature.trim()) {
        // HOTRAITS 给的是 H3 生物名（如 StoneGargoyle），归一化后映射到 VCMI camelCase id
        const key = creature.toLowerCase().replace(/[^a-z0-9]/g, "");
        const cid = creatureNameToId.get(key);
        if (cid) creature = cid;
        army.push({ creature, min: low, max: high });
      }
    }
    traits.endLine();
    if (bio) bio.endLine();
    if (spec) spec.endLine();

    const id = indexMap.get(i);
    if (!id) continue;
    out[id] = {
      name,
      biography,
      specialty: { name: specName, tooltip: specTooltip, description: specDescription },
      army,
    };
  }
  return out;
}

/** 英雄职业：HCTRAITS.TXT（第 0 列为职业名，2-5 列为初始四维主属性）
 *  表头：增强版 HCTRAITS.TXT 只有 2 个物理行（第 2 行把列名与上一行标题合并），
 *  故从数据第 1 行（Knight/骑士）开始读取。若未来遇到 3 行表头的版本，
 *  可改回 skipLines(3) 或在此做首字段探测。 */
function extractHeroClasses(resolve, encoding) {
  const raw = resolve("HCTRAITS.TXT");
  if (!raw) return null;
  const p = new LegacyParser(decodeBuffer(raw, encoding).text);
  p.skipLines(2); // 2 行表头（增强版）
  const out = {};
  let idx = 0;
  for (;;) {
    const name = p.readString();
    if (!name) break;
    // HCTRAITS 列顺序：Name / Aggression / Attack / Defense / Spell Power / Knowledge / ...
    p.readNumber(); // Aggression（成长概率相关，非初始属性，跳过）
    const attack = Math.round(p.readNumber());
    const defense = Math.round(p.readNumber());
    const spellPower = Math.round(p.readNumber());
    const knowledge = Math.round(p.readNumber());
    out[`class${idx}`] = {
      name,
      stats: { attack, defense, spellPower, knowledge },
    };
    idx++;
    if (!p.endLine()) break;
  }
  return out;
}

/** 宝物：ARTRAITS.TXT + ARTEVENT.TXT */
function extractArtifacts(resolve, encoding, indexMap) {
  const raw = resolve("ARTRAITS.TXT");
  if (!raw) return null;
  const p = new LegacyParser(decodeBuffer(raw, encoding).text);
  const rawEvent = resolve("ARTEVENT.TXT");
  const events = rawEvent ? new LegacyParser(decodeBuffer(rawEvent, encoding).text) : null;

  p.endLine(); // 表头第 1 行
  for (let i = 0; i < 7; i++) p.readString();
  const isRoe = p.readString() !== "Misc 5";
  p.endLine(); // 表头第 2 行结束

  // 槽位列顺序与 VCMI CArtHandler.cpp 的 ART_POS_LIST 完全一致（19 列；ROE 少 MISC5 一列）。
  // 此前误用 10/11 列，导致 class 被当成槽位、description 读成空串（历史数据描述全为 " "）。
  const SLOT_NAMES = [
    "SPELLBOOK", "MACH4", "MACH3", "MACH2", "MACH1",
    "MISC5", "MISC4", "MISC3", "MISC2", "MISC1",
    "FEET", "LEFT_RING", "RIGHT_RING", "TORSO",
    "LEFT_HAND", "RIGHT_HAND", "NECK", "SHOULDERS", "HEAD",
  ].filter((s) => !(isRoe && s === "MISC5"));
  const CLASS_MAP = { S: "SPECIAL", T: "TREASURE", N: "MINOR", J: "MAJOR", R: "RELIC" };

  const dataSize = indexMap.size ? Math.max(...indexMap.keys()) + 1 : 0;
  const out = {};
  for (let i = 0; i < dataSize; i++) {
    const name = p.readString();
    const event = events ? events.readString() : "";
    const value = p.readNumber();
    const slots = [];
    for (const slot of SLOT_NAMES) {
      if (p.readString() === "x") slots.push(slot);
    }
    const clsRaw = p.readString().trim();
    const cls = CLASS_MAP[clsRaw[0]] ?? clsRaw;
    const description = p.readString();
    p.endLine();
    if (events) events.endLine();

    const id = indexMap.get(i);
    if (!id) continue;
    out[id] = { name, event, value, slots, class: cls, description };
  }
  return out;
}

/** 法术：SPTRAITS.TXT（分 adventure / battle 两段，均取名称 + 4 级描述） */
function extractSpells(resolve, encoding, indexMap) {
  const raw = resolve("SPTRAITS.TXT");
  if (!raw) return null;
  const p = new LegacyParser(decodeBuffer(raw, encoding).text);
  const LEVELS = 4; // SPELL_SCHOOL_LEVELS

  const readSection = () => {
    const rows = [];
    do {
      const name = p.readString();
      if (!name) break;
      p.readString(); // 缩写名（未使用）
      const level = p.readNumber(); // level（数值，语言无关，随文案一并烘进 h3-core）
      // 学校标记（列 3-6）：earth/water/fire/air，值为 "x"/空 —— 同样语言无关
      const schoolCols = [];
      for (let s = 0; s < 4; s++) schoolCols.push(p.readString());
      for (let c = 0; c < LEVELS; c++) p.readNumber(); // costs
      p.readNumber(); // power
      for (let c = 0; c < LEVELS; c++) p.readNumber(); // powers
      for (let f = 0; f < 9; f++) p.readNumber(); // 各族 gainChance
      for (let c = 0; c < LEVELS; c++) p.readNumber(); // AI values
      const descriptions = [];
      for (let c = 0; c < LEVELS; c++) descriptions.push(p.readString());
      p.readString(); // attributes（JSON 中已有）
      const schoolNames = ["earth", "water", "fire", "air"];
      const schools = schoolNames.filter((_, i) => (schoolCols[i] ?? "").trim() === "x");
      rows.push({ name, level, schools, descriptions });
    } while (p.endLine() && !p.isNextEntryEmpty());
    return rows;
  };

  p.skipLines(5); // header
  const adventure = readSection();
  p.skipLines(3);
  const battle = readSection();

  const out = {};
  let idx = 0;
  for (const row of [...adventure, ...battle]) {
    // SPTRAITS 的解析顺序即 VCMI 的 spell index（legacyData.size()）
    const id = indexMap.get(idx) ?? `spell${idx}`;
    out[id] = {
      name: row.name,
      level: row.level ?? 0,
      schools: row.schools ?? [],
      levelDescriptions: {
        basic: row.descriptions[1] ?? "",
        advanced: row.descriptions[2] ?? "",
        expert: row.descriptions[3] ?? "",
      },
    };
    idx++;
  }
  return out;
}

/** 技能：SSTRAITS.TXT（名称 + 3 级描述） */
function extractSkills(resolve, encoding, indexMap) {
  const raw = resolve("SSTRAITS.TXT");
  if (!raw) return null;
  const p = new LegacyParser(decodeBuffer(raw, encoding).text);
  p.skipLines(2); // header
  const out = {};
  let idx = 0;
  do {
    const name = p.readString();
    if (!name) break;
    const basic = p.readString();
    const advanced = p.readString();
    const expert = p.readString();
    const id = indexMap.get(idx) ?? `skill${idx}`;
    out[id] = { name, basic, advanced, expert };
    idx++;
  } while (p.endLine());
  return out;
}

/** 资源名（H3 官方译名）：
 *  VCMI `config/resources.json` **不含 name 字段**（仅 index/price），翻译池也没有
 *  `resource.core.*.name`（实测 0 条），故中文/英文名按 H3 官方固定译名烘入 h3-core。 */
const RESOURCE_NAMES = {
  wood: { zh: "木材", en: "Wood" },
  mercury: { zh: "水银", en: "Mercury" },
  ore: { zh: "矿石", en: "Ore" },
  sulfur: { zh: "硫磺", en: "Sulfur" },
  crystal: { zh: "水晶", en: "Crystal" },
  gems: { zh: "宝石", en: "Gems" },
  gold: { zh: "黄金", en: "Gold" },
  mitril: { zh: "秘银", en: "Mitril" },
};

/** 法术派系名（H3 官方译名）：
 *  `config/spellSchools.json` 的 name 只有英文原文，翻译池无 `spellSchool.core.*.name`（实测 0 条）。 */
const SPELL_SCHOOL_NAMES = {
  air: { zh: "气系", en: "Air" },
  fire: { zh: "火系", en: "Fire" },
  earth: { zh: "土系", en: "Earth" },
  water: { zh: "水系", en: "Water" },
};

/** 由「官方译名表 + config 的 index 映射」生成 `{<id>: {index, name}}` */
function namesFromTable(indexMap, table, lang, label) {
  const out = {};
  const isZh = lang === "chinese" || lang === "tchinese";
  for (const [index, id] of indexMap.entries()) {
    const t = table[id];
    const name = t ? (isZh ? t.zh : t.en) : id;
    out[id] = { index, name };
  }
  if (!Object.keys(out).length) {
    console.warn(`  ! ${label}: 无 index 映射（跳过）`);
    return null;
  }
  return out;
}

/** 地形：TERRNAME.TXT */
function extractTerrains(resolve, encoding, indexMap) {
  const raw = resolve("TERRNAME.TXT");
  if (!raw) return null;
  const p = new LegacyParser(decodeBuffer(raw, encoding).text);
  const out = {};
  let idx = 0;
  do {
    const name = p.readString();
    if (!name) break;
    const id = indexMap.get(idx) ?? `terrain${idx}`;
    out[id] = { name };
    idx++;
  } while (p.endLine());
  return out;
}

/**
 * 从 BUILDING.TXT 解析一行的 7 项资源花费。
 * 列顺序：Wood Mercury Ore Sulfur Crystal Gems Gold
 */
const __FACTION_ORDER = FACTION_ORDER;
const __BUILDING_NAMES = BUILDING_NAMES;
function parseBuildingCostLine(line) {
  const parts = line.split("\t");
  const nums = parts.slice(0, 7).map((s) => {
    const v = Number(String(s).replace(/,/g, ".").trim());
    return Number.isFinite(v) && v > 0 ? Math.round(v) : 0;
  });
  const [wood, mercury, ore, sulfur, crystal, gems, gold] = nums;
  const cost = {};
  if (wood) cost.wood = wood;
  if (mercury) cost.mercury = mercury;
  if (ore) cost.ore = ore;
  if (sulfur) cost.sulfur = sulfur;
  if (crystal) cost.crystal = crystal;
  if (gems) cost.gems = gems;
  if (gold) cost.gold = gold;
  return cost;
}

/** 建筑花费：BUILDING.TXT（VCMI 从该文件读取原版城镇建筑 cost） */
function extractBuildingCosts(resolve, encoding) {
  const raw = resolve("BUILDING.TXT");
  if (!raw) return null;
  const lines = decodeBuffer(raw, encoding).text.split(/\r?\n/);
  let i = 0;
  const nextNonEmpty = () => {
    while (i < lines.length && lines[i].trim() === "") i++;
    return i < lines.length ? lines[i++] : null;
  };
  const peek = () => {
    let j = i;
    while (j < lines.length && lines[j].trim() === "") j++;
    return j < lines.length ? lines[j] : null;
  };

  const out = {};
  const put = (key, cost) => {
    if (Object.keys(cost).length > 0) out[key] = cost;
  };

  // BUILDING.TXT 仅含 9 个真实城镇；FACTION_ORDER 末尾的 neutral/random 是 VCMI 扩展，不在此文件
  const TOWNS = FACTION_ORDER.slice(0, 9);

  // 表头
  nextNonEmpty(); // Wood\tMercury\t...
  nextNonEmpty(); // Special Buildings

  // 1) 每镇特色建筑（17..25），读到空行结束
  for (let town = 0; town < TOWNS.length; town++) {
    nextNonEmpty(); // town name
    let b = 0;
    while (i < lines.length && lines[i].trim() !== "") {
      const cost = parseBuildingCostLine(lines[i]);
      if (b < 9) {
        const id = BUILDING_NAMES[17 + b];
        if (id) put(`${TOWNS[town]}.${id}`, cost);
      }
      i++;
      b++;
    }
    i++; // skip empty line
  }

  // 跳过“未使用/扩展建筑”的额外行直到 Neutral Buildings，再跳过表头和随后的空行
  while (i < lines.length && !/^Neutral Buildings/i.test(lines[i].trim())) i++;
  i++; // skip "Neutral Buildings" header
  while (i < lines.length && lines[i].trim() === "") i++; // skip blank lines

  // 2) 通用建筑（0..16）
  let b = 0;
  while (i < lines.length && lines[i].trim() !== "") {
    const cost = parseBuildingCostLine(lines[i]);
    const id = BUILDING_NAMES[b];
    if (id) {
      // 通用建筑：写入 common.<id>，同时复制到 9 个原版城镇便于 Rust 直接查 <faction>.<id>
      put(`common.${id}`, cost);
      for (const f of TOWNS) put(`${f}.${id}`, cost);
    }
    i++;
    b++;
  }
  i++; // skip empty line

  // 3) 巢穴建筑（30..43）
  while (i < lines.length && !/^Dwellings/i.test(lines[i].trim())) i++;
  i++; // skip "Dwellings" header
  while (i < lines.length && lines[i].trim() === "") i++; // skip blank lines
  for (let town = 0; town < TOWNS.length; town++) {
    nextNonEmpty(); // town name
    let d = 0;
    while (i < lines.length && lines[i].trim() !== "") {
      const cost = parseBuildingCostLine(lines[i]);
      const id = BUILDING_NAMES[30 + d];
      if (id) put(`${TOWNS[town]}.${id}`, cost);
      i++;
      d++;
    }
    i++; // skip empty line
  }

  return Object.keys(out).length ? out : null;
}

/** 建筑：BLDGSPEC / BLDGNEUT / DWELLING（名称 + 描述） */
function extractBuildings(resolve, encoding) {
  const out = {};
  const put = (key, name, description) => {
    if (name) out[key] = { name, description };
  };

  // BLDGSPEC：每镇 9 个特色(17..25) + grail(26) + resourceSilo(15)
  const rawSpec = resolve("BLDGSPEC.TXT");
  if (rawSpec) {
    const p = new LegacyParser(decodeBuffer(rawSpec, encoding).text);
    for (let town = 0; town < FACTION_ORDER.length; town++) {
      let any = false;
      for (let b = 0; b < 9; b++) {
        const name = p.readString();
        const description = p.readString();
        if (name) {
          any = true;
          put(`${FACTION_ORDER[town]}.${BUILDING_NAMES[17 + b]}`, name, description);
        }
        if (!p.endLine()) break;
      }
      if (!any) break;
      for (const idx of [26, 15]) {
        const name = p.readString();
        const description = p.readString();
        put(`${FACTION_ORDER[town]}.${BUILDING_NAMES[idx]}`, name, description);
        if (!p.endLine()) break;
      }
      if (p.i >= p.end) break;
    }
  }

  // BLDGNEUT：通用建筑 0..14 → ship(20) → 每镇 blacksmith(16)
  const rawNeut = resolve("BLDGNEUT.TXT");
  if (rawNeut) {
    const p = new LegacyParser(decodeBuffer(rawNeut, encoding).text);
    for (let b = 0; b < 15; b++) {
      const name = p.readString();
      const description = p.readString();
      put(`common.${BUILDING_NAMES[b]}`, name, description);
      if (!p.endLine()) break;
    }
    p.skipLines(3); // silo / blacksmith / moat（未使用）
    const shipName = p.readString();
    const shipDesc = p.readString();
    put(`common.${BUILDING_NAMES[20]}`, shipName, shipDesc);
    p.endLine();
    for (let town = 0; town < FACTION_ORDER.length; town++) {
      const name = p.readString();
      const description = p.readString();
      put(`${FACTION_ORDER[town]}.${BUILDING_NAMES[16]}`, name, description);
      if (!p.endLine()) break;
    }
  }

  // DWELLING：每镇 14 个巢穴(30..43)
  const rawDwell = resolve("DWELLING.TXT");
  if (rawDwell) {
    const p = new LegacyParser(decodeBuffer(rawDwell, encoding).text);
    for (let town = 0; town < FACTION_ORDER.length; town++) {
      let any = false;
      for (let b = 0; b < 14; b++) {
        const name = p.readString();
        const description = p.readString();
        if (name) {
          any = true;
          put(`${FACTION_ORDER[town]}.${BUILDING_NAMES[30 + b]}`, name, description);
        }
        if (!p.endLine()) break;
      }
      if (!any) break;
      if (p.i >= p.end) break;
    }
  }
  return Object.keys(out).length ? out : null;
}

/* ------------------------------ 主流程 ------------------------------ */

function inferLang(encoding, gameDir, resolvedEncoding) {
  if (args.lang) return args.lang;
  const enc = resolvedEncoding || encoding;
  if (enc === "gbk" || enc === "gb18030" || enc === "big5") return "chinese";
  if (enc === "cp1251" || enc === "koi8-r") return "russian";
  if (/[-_](ru|rus)$/i.test(gameDir)) return "russian";
  if (/[-_](cn|zh|chs)$/i.test(gameDir)) return "chinese";
  return "english";
}

function main() {
  if (!fs.existsSync(GAME_DIR)) {
    console.error(`游戏目录不存在: ${GAME_DIR}`);
    process.exit(1);
  }
  const resolve = makeLodResolver(GAME_DIR);
  if (!resolve("CRTRAITS.TXT")) {
    console.error(`未在 ${path.join(GAME_DIR, "Data")} 找到 H3bitmap.lod 内的 CRTRAITS.TXT`);
    process.exit(1);
  }

  // 先探测一次编码（用 CRTRAITS）
  const probe = decodeBuffer(resolve("CRTRAITS.TXT"), ENCODING);
  const lang = inferLang(ENCODING, GAME_DIR, probe.encoding);
  console.log(`游戏目录: ${GAME_DIR}`);
  console.log(`config  : ${CONFIG_DIR}`);
  console.log(`编码    : ${probe.encoding}  语言: ${lang}`);

  const indexMaps = {
    creatures: collectIndexMap(CONFIG_DIR, "creatures"),
    heroes: collectIndexMap(CONFIG_DIR, "heroes"),
    artifacts: collectIndexMapSingle(CONFIG_DIR, "artifacts.json"),
    skills: collectIndexMapSingle(CONFIG_DIR, "skills.json"),
    terrains: collectIndexMapSingle(CONFIG_DIR, "terrains.json"),
    spells: collectIndexMap(CONFIG_DIR, "spells"),
    resources: collectIndexMapSingle(CONFIG_DIR, "resources.json"),
    spellSchools: collectIndexMapSingle(CONFIG_DIR, "spellSchools.json"),
  };

  const encoding = probe.encoding;
  const creatureNameToId = buildCreatureNameToId(indexMaps.creatures);
  const datasets = {
    creatures: extractCreatures(resolve, encoding, indexMaps.creatures),
    factions: extractFactions(resolve, encoding, lang),
    heroes: extractHeroes(resolve, encoding, indexMaps.heroes, creatureNameToId),
    heroClasses: extractHeroClasses(resolve, encoding),
    artifacts: extractArtifacts(resolve, encoding, indexMaps.artifacts),
    spells: extractSpells(resolve, encoding, indexMaps.spells),
    skills: extractSkills(resolve, encoding, indexMaps.skills),
    terrains: extractTerrains(resolve, encoding, indexMaps.terrains),
    buildings: extractBuildings(resolve, encoding),
    buildingCosts: extractBuildingCosts(resolve, encoding),
    // 资源名 / 法术派系名：LOD 与翻译池均无中文源，由官方译名表 + config index 生成
    resources: namesFromTable(indexMaps.resources, RESOURCE_NAMES, lang, "resources"),
    spellSchools: namesFromTable(indexMaps.spellSchools, SPELL_SCHOOL_NAMES, lang, "spellSchools"),
  };

  fs.mkdirSync(OUT_DIR, { recursive: true });
  const summary = {};
  for (const [key, data] of Object.entries(datasets)) {
    if (!data) {
      console.warn(`  - ${key}: 无数据（跳过）`);
      continue;
    }
    const file = path.join(OUT_DIR, `${lang}.${key}.json`);
    fs.writeFileSync(file, JSON.stringify(data, null, 2), "utf8");
    summary[key] = Object.keys(data).length;
    console.log(`  + ${path.relative(REPO, file)} (${Object.keys(data).length} 条)`);
  }

  // 把本地绝对路径收敛成不泄露个人目录的形式：
  // 若在仓库内→写仓库相对路径；若在 gameDir 内→写 `<gameDir 末级>/...`；否则仅保留末级目录名（版本标识）。
  const gameBase = path.basename(path.resolve(GAME_DIR));
  function toRepoSafe(p) {
    const abs = path.resolve(p);
    const relRepo = path.relative(REPO, abs);
    if (!relRepo.startsWith("..") && !path.isAbsolute(relRepo)) return relRepo;
    const relGame = path.relative(path.resolve(GAME_DIR), abs);
    if (!relGame.startsWith("..") && !path.isAbsolute(relGame)) return `${gameBase}/${relGame}`;
    return path.basename(abs);
  }

  const metaPath = path.join(OUT_DIR, `meta.${lang}.json`);
  fs.writeFileSync(
    metaPath,
    JSON.stringify(
      {
        lang,
        encoding,
        gameDir: toRepoSafe(GAME_DIR),
        configDir: toRepoSafe(CONFIG_DIR),
        generatedAt: new Date().toISOString(),
        source: "H3 legacy TXT in Data/*.lod (via VCMI CLegacyConfigParser rules)",
        counts: summary,
      },
      null,
      2,
    ),
    "utf8",
  );
  console.log(`  + ${path.relative(REPO, metaPath)}`);
}

main();
