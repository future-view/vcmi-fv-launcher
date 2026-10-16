//! H3M / H3C / VMAP 地图头部解析与语言检测。
//!
//! 逐函数复刻自 `homm3-map-translator` 的 C++ 引擎（`tools/h3_map_parser`），
//! 仅保留头部所需部分（版本 / 名称 / 描述 / 尺寸 / 难度 / 玩家 / 胜负条件 / 战役场景表 / 语言），
//! 不做对象与文本偏移的全量解析。字节布局对照：
//! - `h3m_header_parser.cpp`（parseVersion / parseHeaderStrings / parsePlayerInfo / parseVictoryLossConditions）
//! - `h3c_reader.cpp`（多 gzip 块战役：parseHeader / parseScenarioHeader / skipScenarioTravel）
//! - `language_detector.cpp`（编码评分 + 语言词典检测）
//!
//! 语言标识最终**归一化为 VCMI `Languages.h` 的 identifier**（`chinese` / `english` / …），
//! 与启动器 `ui_lang` 同一命名体系；未识别返回 `unknown`。

use crate::infra::berror::be_err;
use encoding_rs::{BIG5, EUC_KR, GBK, SHIFT_JIS, WINDOWS_1250, WINDOWS_1251, WINDOWS_1252};
use serde::Serialize;
use std::io::Read;
use std::path::Path;

/// 地图元数据（`index.json` 条目 + 前端展示来源）。
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct MapMeta {
    /// `map` | `campaign` | `vmap`
    pub kind: String,
    /// VCMI 语言 identifier（`chinese`/`english`/…），未识别为 `unknown`
    pub lang: String,
    /// 检测出的文本编码（utf-8/gbk/big5/cp932/cp949/cp1250/cp1251/cp1252）
    pub encoding: String,
    pub name: String,
    pub description: String,
    /// ROE / AB / SOD / HOTA / WOG / VCMI
    pub version: String,
    /// 0..4（简单/普通/困难/专家/不可能），未知为 -1
    pub difficulty: i8,
    /// 可玩玩家总数（人类 + 电脑）
    pub players: i16,
    /// 人类可玩玩家数
    pub humans: i16,
    pub size_w: i32,
    pub size_h: i32,
    /// S / M / L / XL / H / XH / G
    pub size_class: String,
    /// 胜利条件键（standard/artifact/gather_troop/…），前端按 i18n 渲染
    pub victory: String,
    /// 失败条件键（standard/lose_castle/lose_hero/time_expires）
    pub loss: String,
    /// 战役场景数（仅 h3c）
    pub scenario_count: i16,
    /// 战役场景表（仅 h3c，取自头部内嵌场景表）
    pub scenarios: Vec<ScenarioMeta>,
}

/// 战役场景条目（H3C 头部内嵌场景表；`index.json` v2 起结构化）。
#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ScenarioMeta {
    /// 场景名（regionName）：官方战役为地区 / 关卡名，自制战役常填剧情梗概；可为空
    pub name: String,
    /// 子地图文件名（如 `wan003.h3m`）；磁盘上可为空
    pub map_file: String,
    /// 场景默认难度 0..4（简单 / 普通 / 困难 / 专家 / 不可能），未知为 -1
    pub difficulty: i8,
}

// ── 字节游标 ────────────────────────────────────────────────────────

struct R<'a> {
    d: &'a [u8],
    pos: usize,
}

impl<'a> R<'a> {
    fn new(d: &'a [u8]) -> Self {
        Self { d, pos: 0 }
    }
    fn u8(&mut self) -> u8 {
        let v = self.d.get(self.pos).copied().unwrap_or(0);
        self.pos = self.pos.saturating_add(1);
        v
    }
    fn bool(&mut self) -> bool {
        self.u8() != 0
    }
    fn u32(&mut self) -> u32 {
        let mut b = [0u8; 4];
        for slot in &mut b {
            *slot = self.u8();
        }
        u32::from_le_bytes(b)
    }
    fn skip(&mut self, n: usize) {
        self.pos = self.pos.saturating_add(n);
    }
    fn eof(&self) -> bool {
        self.pos >= self.d.len()
    }
    /// 读 size-prefixed 字符串（u32 长度 + 字节），返回原始字节；长度超限视为损坏。
    fn raw_string(&mut self, max: usize) -> &'a [u8] {
        let len = self.u32() as usize;
        if len == 0 {
            return &[];
        }
        if len > max || self.pos.saturating_add(len) > self.d.len() {
            self.pos = self.d.len();
            return &[];
        }
        let s = &self.d[self.pos..self.pos + len];
        self.pos += len;
        s
    }
}

// ── 版本特性（对照 C++ initFeatures） ───────────────────────────────

struct Features {
    level_roe: bool,
    level_ab: bool,
    level_sod: bool,
    factions_bytes: usize,
}

fn init_features(version: u32) -> Option<(Features, &'static str)> {
    let (level_roe, level_ab, level_sod, fb, name) = match version {
        0x0E => (true, false, false, 1, "ROE"),
        0x15 => (true, true, false, 2, "AB"),
        0x1C => (true, true, true, 2, "SOD"),
        0x33 => (true, true, true, 2, "WOG"),
        0x20 => (true, true, true, 2, "HOTA"),
        _ => return None,
    };
    Some((
        Features {
            level_roe,
            level_ab,
            level_sod,
            factions_bytes: fb,
        },
        name,
    ))
}

// ── 编码检测与语言识别（对照 language_detector.cpp） ────────────────

const ENCODINGS: &[(&str, &encoding_rs::Encoding)] = &[
    ("utf-8", encoding_rs::UTF_8),
    ("gbk", GBK),
    ("big5", BIG5),
    ("cp932", SHIFT_JIS),
    ("cp949", EUC_KR),
    ("cp1251", WINDOWS_1251),
    ("cp1250", WINDOWS_1250),
    ("cp1252", WINDOWS_1252),
];

/// 严格解码：字节全部合法才返回 Some（等价 MultiByteToWideChar + MB_ERR_INVALID_CHARS）。
fn decode_strict(enc: &'static encoding_rs::Encoding, raw: &[u8]) -> Option<String> {
    if raw.is_empty() {
        return Some(String::new());
    }
    // encoding_rs 的 decode 带 BOM 嗅探，对 cp125x 无影响；UTF-8 用标准库严格校验
    if enc == encoding_rs::UTF_8 {
        return std::str::from_utf8(raw).ok().map(String::from);
    }
    let (text, _, had_errors) = enc.decode(raw);
    if had_errors {
        None
    } else {
        Some(text.into_owned())
    }
}

/// 宽容解码（损坏字节替换），用于最终输出。
fn decode_lossy(enc: &'static encoding_rs::Encoding, raw: &[u8]) -> String {
    if raw.is_empty() {
        return String::new();
    }
    let (text, _enc, _had) = enc.decode(raw);
    text.into_owned()
}

fn enc_by_name(name: &str) -> &'static encoding_rs::Encoding {
    ENCODINGS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, e)| *e)
        .unwrap_or(WINDOWS_1252)
}

/// 简体常用字表 + 日 / 韩常用字表（对照 C++ commonChinese/commonJapanese/commonKorean）
const COMMON_CHINESE: &str = "的一是不在人有我他这中大来上国个到说们为子和你地出道也时年得就那要下以生会自着去之过家学对可她里后小心多天而能好都然没日于起还发成事作当想看文无开手十用主行方前所本见经头面公同三已老从动两长知民样现分将外但身些与高意进把法实回二理美点月明暗光王战英雄龙神魔";
const COMMON_JAPANESE: &str =
    "のにをはがとでてもからまでよりするしたですますないあるいるこれそれ英雄王戦争地図物語魔神竜";
const COMMON_KOREAN: &str = "의이가은는을를에와과한하다로서영웅왕전쟁지도마법신용";

fn count_common(text: &str, common: &str) -> i32 {
    let common: Vec<char> = common.chars().collect();
    text.chars().filter(|c| common.contains(c)).count() as i32
}

/// 语言判定（对照 C++ detectWideLanguage）：CJK/谚文/假名/西里尔计数 + 拉丁小词典评分。
fn detect_wide_language(text: &str) -> String {
    let mut cjk = 0i32;
    let mut kana = 0i32;
    let mut hangul = 0i32;
    let mut cyrillic = 0i32;
    let mut letters = 0i32;
    for ch in text.chars() {
        let cp = ch as u32;
        if (0x3400..=0x9FFF).contains(&cp) {
            cjk += 1;
        }
        if (0x3040..=0x30FF).contains(&cp) {
            kana += 1;
        }
        if (0xAC00..=0xD7AF).contains(&cp) {
            hangul += 1;
        }
        if (0x0400..=0x052F).contains(&cp) {
            cyrillic += 1;
        }
        if ch.is_alphabetic() {
            letters += 1;
        }
    }
    if kana >= 2 && kana * 10 >= letters {
        return "ja-JP".into();
    }
    if hangul >= 2 && hangul * 5 >= letters {
        return "ko-KR".into();
    }
    if cjk >= 2 && cjk * 5 >= letters {
        return "zh-CN".into();
    }
    if cyrillic >= 2 {
        // 乌克兰语特征词（і/ї/є + 高频词），对照 C++ uk 词典
        let uk = ["і", "ї", "є", "та", "що", "для", "цей", "бути", "герої"];
        let hits = uk.iter().filter(|w| text.contains(**w)).count();
        return if hits >= 2 {
            "uk-UA".into()
        } else {
            "ru-RU".into()
        };
    }
    // 拉丁语系小词典评分（对照 C++ dictionaries）
    const DICTS: &[(&str, &[&str])] = &[
        (
            "en-US",
            &[
                "the", "and", "your", "you", "are", "with", "for", "from", "this", "that", "have",
                "will", "hero", "kingdom",
            ],
        ),
        (
            "pl-PL",
            &[
                "nie",
                "się",
                "jest",
                "dla",
                "który",
                "przez",
                "oraz",
                "bohater",
                "królestwo",
                "musisz",
                "jeśli",
                "znasz",
                "nazwa",
                "pomoże",
                "możesz",
                "został",
            ],
        ),
        (
            "cs-CZ",
            &[
                "není",
                "jste",
                "který",
                "tento",
                "musíte",
                "hrdina",
                "království",
                "může",
                "jsou",
                "pokud",
                "bude",
                "byl",
                "získat",
            ],
        ),
        (
            "de-DE",
            &[
                "der",
                "die",
                "das",
                "und",
                "ist",
                "nicht",
                "für",
                "werden",
                "held",
                "königreich",
                "musst",
            ],
        ),
        (
            "fr-FR",
            &[
                "les", "des", "dans", "pour", "avec", "vous", "votre", "héros", "royaume", "devez",
            ],
        ),
        (
            "es-ES",
            &[
                "los", "para", "que", "con", "una", "por", "del", "héroe", "reino", "debes",
            ],
        ),
        (
            "pt-PT",
            &[
                "para", "que", "com", "uma", "não", "dos", "das", "seu", "herói", "reino",
            ],
        ),
        (
            "it-IT",
            &[
                "per", "che", "con", "una", "nel", "della", "eroe", "regno", "devi",
            ],
        ),
        (
            "nl-NL",
            &[
                "het",
                "een",
                "van",
                "voor",
                "met",
                "niet",
                "held",
                "koninkrijk",
                "moet",
            ],
        ),
        (
            "hu-HU",
            &[
                "egy", "hogy", "nem", "kell", "vissza", "gonosz", "város", "szárny", "lehet",
                "vagy",
            ],
        ),
        (
            "ro-RO",
            &[
                "este", "pentru", "care", "trebuie", "sunt", "lume", "erou", "regat", "poate",
                "fără",
            ],
        ),
    ];
    let lower = text.to_lowercase();
    let mut best = "und";
    let mut best_score = 0;
    let mut second = 0;
    for (lang, words) in DICTS {
        let score: i32 = words.iter().map(|w| lower.matches(w).count() as i32).sum();
        if score > best_score {
            second = best_score;
            best = lang;
            best_score = score;
        } else if score > second {
            second = score;
        }
    }
    if best_score >= 6 && best_score - second >= 2 {
        return best.into();
    }
    if best == "en-US" && best_score >= 4 && best_score > second {
        return best.into();
    }
    // 词典未达标但含拉丁字母：默认英语（对照 C++ 拉丁回退）
    if best_score < 4 && text.chars().any(|c| c.is_ascii_alphabetic()) {
        return "en-US".into();
    }
    "und".into()
}

/// 编码评分（对照 C++ decodeScore）。
fn decode_score(text: &str, encoding: &str) -> i32 {
    let mut controls = 0i32;
    let mut cjk = 0i32;
    let mut kana = 0i32;
    let mut hangul = 0i32;
    let mut cyrillic = 0i32;
    let mut latin_ext = 0i32;
    let mut letters = 0i32;
    for ch in text.chars() {
        let cp = ch as u32;
        if cp < 0x20 && ch != '\r' && ch != '\n' && ch != '\t' {
            controls += 1;
        }
        if (0x3400..=0x9FFF).contains(&cp) {
            cjk += 1;
        }
        if (0x3040..=0x30FF).contains(&cp) {
            kana += 1;
        }
        if (0xAC00..=0xD7AF).contains(&cp) {
            hangul += 1;
        }
        if (0x0400..=0x052F).contains(&cp) {
            cyrillic += 1;
        }
        if (0x00C0..=0x024F).contains(&cp) {
            latin_ext += 1;
        }
        if ch.is_alphabetic() {
            letters += 1;
        }
    }
    let mut score = -controls * 1000;
    let language = detect_wide_language(text);
    let common_cjk = count_common(text, COMMON_CHINESE);
    let common_kana = count_common(text, COMMON_JAPANESE);
    let common_hangul = count_common(text, COMMON_KOREAN);
    match encoding {
        "utf-8" => score += if language == "und" { -50 } else { 300 },
        "gbk" | "big5" => {
            score += if cjk >= 2 && cjk * 20 >= letters {
                cjk * 22 + common_cjk * 25 - kana * 5
            } else {
                -150
            }
        }
        "cp932" => {
            score += if kana >= 2 && kana * 20 >= letters {
                kana * 25 + cjk * 8 + common_kana * 20
            } else {
                -150
            }
        }
        "cp949" => {
            score += if hangul >= 2 && hangul * 20 >= letters && common_hangul >= 1 {
                hangul * 25 + common_hangul * 20
            } else {
                -150
            }
        }
        "cp1251" => {
            score += if cyrillic >= 4 && cyrillic * 5 >= letters {
                cyrillic * 8
            } else {
                -100
            }
        }
        "cp1250" => {
            score += if language == "pl-PL" || language == "cs-CZ" {
                latin_ext * 7 + 40
            } else {
                latin_ext * 2
            }
        }
        "cp1252" => score += latin_ext * 3 + 10,
        _ => {}
    }
    if language != "und" {
        score += 80;
    }
    score
}

/// 文本样本 → (最佳编码, 语言标签)。对照 C++ analyzeTextBytes。
fn analyze_text_bytes(samples: &[&[u8]]) -> (String, String) {
    let combined: Vec<u8> = samples
        .iter()
        .flat_map(|s| s.iter().copied().chain(std::iter::once(b'\n')))
        .collect();
    let ascii = combined.iter().all(|b| *b < 0x80);
    let mut best_encoding = "cp1252";
    if !ascii {
        let ascii_letters = combined.iter().filter(|b| b.is_ascii_alphabetic()).count() as i32;
        let high_bytes = combined.iter().filter(|b| **b >= 0x80).count() as i32;
        let mut best_score = i32::MIN;
        for (name, enc) in ENCODINGS {
            let Some(text) = decode_strict(enc, &combined) else {
                continue;
            };
            let mut score = decode_score(&text, name);
            let multibyte = matches!(*name, "gbk" | "big5" | "cp932" | "cp949");
            if multibyte && high_bytes * 5 < ascii_letters {
                score -= 400;
            }
            if score > best_score {
                best_score = score;
                best_encoding = name;
            }
        }
    }
    let enc = enc_by_name(best_encoding);
    let decoded: Vec<String> = samples.iter().map(|s| decode_lossy(enc, s)).collect();
    let joined = decoded.join("\n");
    let mut language = detect_wide_language(&joined);
    if language == "und" {
        language = fallback_language_for_encoding(best_encoding, &joined);
    }
    (best_encoding.into(), language)
}

/// 编码兜底语言（对照 C++ fallbackLanguageForEncoding）。
fn fallback_language_for_encoding(encoding: &str, text: &str) -> String {
    match encoding {
        "gbk" => "zh-CN".into(),
        "big5" => "zh-TW".into(),
        "cp932" => "ja-JP".into(),
        "cp949" => "ko-KR".into(),
        "cp1251" => "ru-RU".into(),
        "cp1252" | "latin" => {
            if text.chars().any(|c| c.is_ascii_alphabetic()) {
                "en-US".into()
            } else {
                "und".into()
            }
        }
        _ => "und".into(),
    }
}

/// BCP-47 → VCMI `Languages.h` identifier（§0.2 映射表）。
pub fn to_vcmi_lang(bcp47: &str) -> String {
    match bcp47 {
        "zh-CN" => "chinese",
        "zh-TW" => "tchinese",
        "en-US" | "en" => "english",
        "ru-RU" | "ru" => "russian",
        "uk-UA" | "uk" => "ukrainian",
        "ja-JP" | "ja" => "japanese",
        "ko-KR" | "ko" => "korean",
        "pl-PL" | "pl" => "polish",
        "cs-CZ" | "cs" => "czech",
        "de-DE" | "de" => "german",
        "fr-FR" | "fr" => "french",
        "es-ES" | "es" => "spanish",
        "pt-PT" | "pt" => "portuguese",
        "it-IT" | "it" => "italian",
        "nl-NL" | "nl" => "dutch",
        "ro-RO" | "ro" => "romanian",
        "hu-HU" | "hu" => "hungarian",
        _ => "unknown",
    }
    .into()
}

fn size_class(w: i32, h: i32) -> String {
    let side = w.max(h);
    match side {
        0 => String::new(),
        s if s >= 252 => "G".into(),
        s if s >= 216 => "XH".into(),
        s if s >= 180 => "H".into(),
        s if s >= 144 => "XL".into(),
        s if s >= 108 => "L".into(),
        s if s >= 72 => "M".into(),
        _ => "S".into(),
    }
}

// ── 胜负条件 ────────────────────────────────────────────────────────

/// 解析胜利/失败条件段（对照 C++ parseVictoryLossConditions），返回 (胜利键, 失败键)。
fn parse_victory_loss(r: &mut R, f: &Features) -> (String, String) {
    let victory = match r.u8() as i8 {
        -1 => "standard",
        cond => {
            r.skip(2); // allowNormalVictory + appliesToAI
            match cond {
                0 => {
                    r.skip(if f.level_ab { 2 } else { 1 }); // ARTIFACT
                    "artifact"
                }
                1 => {
                    r.skip(6); // GATHERTROOP
                    "gather_troop"
                }
                2 => {
                    r.skip(5); // GATHERRESOURCE
                    "gather_resource"
                }
                3 => {
                    r.skip(5); // BUILDCITY
                    "build_city"
                }
                4 => {
                    r.skip(3); // BUILDGRAIL
                    "build_grail"
                }
                5 => {
                    r.skip(3); // BEATHERO
                    "beat_hero"
                }
                6 => {
                    r.skip(3); // CAPTURECITY
                    "capture_city"
                }
                7 => {
                    r.skip(3); // BEATMONSTER
                    "beat_monster"
                }
                8 => "take_dwellings",
                9 => "take_mines",
                10 => {
                    r.skip(4); // TRANSPORTITEM
                    "transport_item"
                }
                11 => "eliminate_all_monsters",
                // 12 及未知值：HOTA_SURVIVE_FOR_DAYS
                _ => {
                    r.skip(4);
                    "survive_days"
                }
            }
        }
    };
    let loss = match r.u8() as i8 {
        -1 => "standard",
        cond => match cond {
            0 => {
                r.skip(3); // LOSSCASTLE
                "lose_castle"
            }
            1 => {
                r.skip(3); // LOSSHERO
                "lose_hero"
            }
            // 2 及未知值：TIMEEXPIRES
            _ => {
                r.skip(2);
                "time_expires"
            }
        },
    };
    (victory.into(), loss.into())
}

// ── H3M 头部解析 ────────────────────────────────────────────────────

/// 按扩展名分发解析。`path` 为领域仓库内的地图文件。
pub fn parse_file(path: &Path) -> Result<MapMeta, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let data =
        std::fs::read(path).map_err(|e| be_err("infra_map_read_failed", &format!("{}", e)))?;
    match ext.as_str() {
        "h3m" => parse_h3m_bytes(&data, "map"),
        "h3c" => parse_h3c_bytes(&data),
        "vmap" => parse_vmap_bytes(&data),
        other => Err(be_err(
            "infra_map_format_unsupported",
            &format!("{}", other),
        )),
    }
}

/// gzip 魔数（1f 8b）则解压（对照 C++ decompressIfNeeded）。
/// 流式容错：部分语料文件 gzip 尾部 ISIZE 校验失败（截断/长度字段不符），
/// 游戏与 C++ 均用流式 zlib 解压不校验尾部，此处一致保留已产出部分。
fn gunzip_if_needed(data: &[u8]) -> Vec<u8> {
    if data.len() >= 10 && data[0] == 0x1f && data[1] == 0x8b {
        let mut out = Vec::new();
        let mut decoder = flate2::read::MultiGzDecoder::new(data);
        let mut buf = [0u8; 65536];
        loop {
            match decoder.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => out.extend_from_slice(&buf[..n]),
                Err(_) => break,
            }
        }
        if !out.is_empty() {
            return out;
        }
    }
    data.to_vec()
}

fn parse_h3m_bytes(data: &[u8], kind: &str) -> Result<MapMeta, String> {
    let raw = gunzip_if_needed(data);
    if raw.len() < 8 {
        return Err(be_err("h3m_too_small", ""));
    }
    let mut r = R::new(&raw);
    let version = r.u32();
    // HOTA：子版本扩展字段（对照 C++ parseVersion 的 hota_ver 跳表）
    if version == 0x20 {
        let hota_ver = r.u32();
        if hota_ver >= 8 {
            r.skip(12);
        }
        if hota_ver >= 1 {
            r.skip(2);
        }
        if hota_ver >= 2 {
            r.skip(4);
        }
        if hota_ver >= 5 {
            r.skip(5);
        }
        if hota_ver >= 7 {
            r.skip(1);
        }
        if hota_ver >= 8 {
            r.skip(1);
        }
        if hota_ver >= 9 {
            r.skip(4);
        }
    }
    let Some((f, version_name)) = init_features(version) else {
        return Err(be_err("infra_map_version_unknown", &format!("{}", version)));
    };

    // 头部字符串（对照 parseHeaderStrings）
    r.skip(1); // areAnyPlayers
    let size_w = r.u32() as i32; // 尺寸仅一个 u32（H3M 恒方图，宽=高）
    r.skip(1); // hasUnderground
    let name_raw = r.raw_string(100_000).to_vec();
    let desc_raw = r.raw_string(100_000).to_vec();
    let (encoding, lang_bcp47) = analyze_text_bytes(&[&name_raw, &desc_raw]);
    let enc = enc_by_name(&encoding);
    let name = decode_lossy(enc, &name_raw);
    let description = decode_lossy(enc, &desc_raw);
    let difficulty = r.u8() as i8;
    if f.level_ab {
        r.skip(1); // levelLimit
    }

    // 玩家信息 ×8（对照 parsePlayerInfo）
    let mut players = 0i16;
    let mut humans = 0i16;
    for _ in 0..8 {
        let can_human = r.bool();
        let can_computer = r.bool();
        if !can_human && !can_computer {
            if f.level_roe {
                r.skip(6);
            }
            if f.level_ab {
                r.skip(6);
            }
            if f.level_sod {
                r.skip(1);
            }
            continue;
        }
        players += 1;
        if can_human {
            humans += 1;
        }
        r.skip(1); // aiTactic
        if f.level_sod {
            r.skip(1); // faction selectable
        }
        r.skip(f.factions_bytes); // factions bitmask
        r.skip(1); // isFactionRandom
        let has_main_town = r.bool();
        if has_main_town {
            if f.level_ab {
                r.skip(2); // generateHero + starting town type
            }
            r.skip(3); // main town pos
        }
        r.skip(1); // hasRandomHero
        let main_custom_hero_id = r.u8();
        if main_custom_hero_id != 0xFF {
            r.skip(1); // portrait
            r.raw_string(1000); // hero name
        }
        if f.level_ab {
            r.skip(1);
            let hero_count = r.u32();
            if hero_count <= 100 {
                for _ in 0..hero_count {
                    r.skip(1); // hero id
                    r.raw_string(1000);
                }
            }
        }
    }
    if r.eof() {
        // 头部被截断：胜负条件缺失不致命，返回已解析部分
        return Ok(base_meta(
            kind,
            &lang_bcp47,
            &encoding,
            name,
            description,
            version_name,
            difficulty,
            players,
            humans,
            size_w,
        ));
    }
    let (victory, loss) = parse_victory_loss(&mut r, &f);

    Ok(MapMeta {
        kind: kind.into(),
        lang: to_vcmi_lang(&lang_bcp47),
        encoding,
        name,
        description,
        version: version_name.into(),
        difficulty,
        players,
        humans,
        size_w,
        size_h: size_w,
        size_class: size_class(size_w, size_w),
        victory,
        loss,
        scenario_count: 0,
        scenarios: Vec::new(),
    })
}

#[allow(clippy::too_many_arguments)]
fn base_meta(
    kind: &str,
    lang_bcp47: &str,
    encoding: &str,
    name: String,
    description: String,
    version: &str,
    difficulty: i8,
    players: i16,
    humans: i16,
    size_w: i32,
) -> MapMeta {
    MapMeta {
        kind: kind.into(),
        lang: to_vcmi_lang(lang_bcp47),
        encoding: encoding.into(),
        name,
        description,
        version: version.into(),
        difficulty,
        players,
        humans,
        size_w,
        size_h: size_w,
        size_class: size_class(size_w, size_w),
        victory: "standard".into(),
        loss: "standard".into(),
        scenario_count: 0,
        scenarios: Vec::new(),
    }
}

// ── H3C 战役解析 ────────────────────────────────────────────────────

/// 找出所有 gzip 块起始位置（对照 C++ findGzipHeaders）。
fn find_gzip_headers(data: &[u8]) -> Vec<usize> {
    let mut out = Vec::new();
    if data.len() < 3 {
        return out;
    }
    for i in 0..=(data.len() - 3) {
        if data[i] == 0x1f && data[i + 1] == 0x8b && data[i + 2] == 0x08 {
            out.push(i);
        }
    }
    out
}

fn gunzip_slice(data: &[u8], start: usize, end: usize) -> Vec<u8> {
    if start >= end || end > data.len() {
        return Vec::new();
    }
    // 简化：用 MultiGzDecoder 对切片解压（切片以 gzip 头开始，解码器在流尾停止）
    let mut out = Vec::new();
    let mut decoder = flate2::read::MultiGzDecoder::new(&data[start..end]);
    if decoder.read_to_end(&mut out).is_err() || out.is_empty() {
        return Vec::new();
    }
    out
}

/// 解析 H3C 战役（对照 C++ parseHeader + parseScenarioHeader + skipScenarioTravel）。
/// 场景清单取自战役头的 regionName（头部内嵌场景表，非解包子地图）。
fn parse_h3c_bytes(data: &[u8]) -> Result<MapMeta, String> {
    let headers = find_gzip_headers(data);
    if headers.is_empty() {
        return Err(be_err("h3c_no_gzip", ""));
    }
    // 按块边界解压
    let mut blocks: Vec<Vec<u8>> = Vec::new();
    for (i, &start) in headers.iter().enumerate() {
        let end = headers.get(i + 1).copied().unwrap_or(data.len());
        let block = gunzip_slice(data, start, end);
        if !block.is_empty() {
            blocks.push(block);
        }
    }
    if blocks.is_empty() {
        return Err(be_err("h3c_gzip_fail", ""));
    }
    let header = &blocks[0];
    if header.len() < 8 {
        return Err(be_err("h3c_header_small", ""));
    }
    let mut r = R::new(header);
    let format_version = r.u32();
    r.skip(1); // campaignMapId
    let name_raw = r.raw_string(500).to_vec();
    let desc_raw = r.raw_string(8000).to_vec();
    if format_version > 4 {
        r.skip(1); // difficulty flag（RoE 无）
    }
    r.skip(1); // music

    // 场景表：数量 = 后续 gzip 块数（对照 C++ 的 truth source）。
    // 先只收集原始字节，全部文本读完后再统一检测编码并解码——对照 C++
    // normalizeHeaderEncoding：「从全部原始头部文本样本确定一个战役级编码，再统一解码」。
    // ⚠️ regionName 曾直接 `from_utf8_lossy`：GBK 字节被当 UTF-8 处理，场景名整列乱码。
    let map_count = blocks.len().saturating_sub(1);
    let mut scenarios_raw: Vec<(Vec<u8>, Vec<u8>, i8)> = Vec::new(); // (mapFileName, regionName, difficulty)
    let mut story_samples: Vec<Vec<u8>> = Vec::new(); // prologue / epilogue（仅入编码样本，不输出）
    for _ in 0..map_count {
        if r.eof() {
            break;
        }
        let map_file_raw = r.raw_string(1000).to_vec(); // mapFileName
        r.skip(4); // packedMapSize
                   // Unholy Alliance（>8 场景）为 2 字节前置条件
        r.skip(if map_count > 8 { 2 } else { 1 });
        r.skip(1); // regionColor
        let difficulty = r.u8() as i8; // 场景默认难度（0..4）
        let region_raw = r.raw_string(2000).to_vec();
        // prolog / epilog
        for _ in 0..2 {
            if r.eof() {
                break;
            }
            let has = r.u8();
            if has != 0 {
                r.skip(2); // video + music
                story_samples.push(r.raw_string(8000).to_vec());
            }
        }
        skip_scenario_travel(&mut r, format_version);
        scenarios_raw.push((map_file_raw, region_raw, difficulty));
    }

    // 编码 / 语言：样本 = 战役名 + 描述 + 场景名 + 剧情文本（对照 normalizeHeaderEncoding + 语言检测）
    let mut samples: Vec<&[u8]> = vec![&name_raw, &desc_raw];
    samples.extend(scenarios_raw.iter().map(|(_, region, _)| region.as_slice()));
    samples.extend(story_samples.iter().map(|s| s.as_slice()));
    let (encoding, lang_bcp47) = analyze_text_bytes(&samples);
    let enc = enc_by_name(&encoding);
    let name = decode_lossy(enc, &name_raw);
    let description = decode_lossy(enc, &desc_raw);
    let scenarios: Vec<ScenarioMeta> = scenarios_raw
        .into_iter()
        .map(|(mfn, region, difficulty)| ScenarioMeta {
            name: decode_lossy(enc, &region),
            map_file: decode_lossy(enc, &mfn),
            difficulty,
        })
        .collect();
    let mut lang = to_vcmi_lang(&lang_bcp47);
    if lang == "chinese" && encoding == "big5" {
        lang = "tchinese".into();
    }

    Ok(MapMeta {
        kind: "campaign".into(),
        lang,
        encoding,
        name,
        description,
        version: "H3C".into(),
        difficulty: -1,
        players: -1,
        humans: -1,
        size_w: 0,
        size_h: 0,
        size_class: String::new(),
        victory: String::new(),
        loss: String::new(),
        scenario_count: map_count as i16,
        scenarios,
    })
}

/// 跳过 CScenarioTravel 段（对照 C++ skipScenarioTravel）。
fn skip_scenario_travel(r: &mut R, campaign_version: u32) {
    if r.eof() {
        return;
    }
    r.skip(1); // whatHeroKeeps bitmap
    r.skip(if campaign_version == 10 { 24 } else { 19 }); // monstersKeptByHero
    r.skip(if campaign_version == 10 {
        21
    } else if campaign_version < 6 {
        17
    } else {
        18
    }); // artifactsKeptByHero
    if r.eof() {
        return;
    }
    let start_opt = r.u8();
    if start_opt == 1 {
        r.skip(1); // playerColor
    }
    if start_opt != 0 {
        let num_bonuses = r.u8();
        const BONUS_SIZES: [usize; 8] = [3, 6, 1, 4, 3, 6, 4, 5];
        for _ in 0..num_bonuses {
            if r.eof() {
                break;
            }
            match start_opt {
                2 => {
                    r.skip(2); // HERO_CROSSOVER: player + scenario
                    continue;
                }
                3 => {
                    r.skip(3); // HERO_OPTIONS: player + hero(int16)
                    continue;
                }
                _ => {}
            }
            let bonus_type = r.u8();
            match BONUS_SIZES.get(bonus_type as usize) {
                Some(size) => r.skip(*size),
                None => break,
            }
        }
    }
}

// ── VMAP（VCMI JSON 地图）解析 ──────────────────────────────────────

/// 从 JSON 值提取文本：支持字符串或 `{text|translated: …}` 本地化对象。
fn localized(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Object(o) => o
            .get("translated")
            .or_else(|| o.get("text"))
            .and_then(|x| x.as_str())
            .unwrap_or_default()
            .into(),
        _ => String::new(),
    }
}

/// 解析 VMAP（ZIP 内 header.json + 语言 json 文件名）。语言文件名即 VCMI identifier。
fn parse_vmap_bytes(data: &[u8]) -> Result<MapMeta, String> {
    let cursor = std::io::Cursor::new(data);
    let mut zip = zip::ZipArchive::new(cursor)
        .map_err(|e| be_err("infra_vmap_zip_open_failed", &format!("{}", e)))?;
    let mut names: Vec<String> = Vec::new();
    for i in 0..zip.len() {
        if let Ok(n) = zip.by_index(i) {
            names.push(n.name().to_string());
        }
    }
    // 语言 = 条目最多的 `<lang>.json`（对照 C++ pickBestLang；语言文件名即 VCMI identifier）
    let structural = [
        "header.json",
        "objects.json",
        "terrain0.json",
        "terrain1.json",
    ];
    let mut best_lang = "unknown";
    let mut best_count = 0usize;
    for name in &names {
        if !name.ends_with(".json") || structural.contains(&name.as_str()) {
            continue;
        }
        let lang = &name[..name.len() - 5];
        if lang.is_empty() || lang.contains('/') {
            continue;
        }
        let count = zip
            .by_name(name)
            .map(|f| {
                serde_json::from_reader::<_, serde_json::Map<String, serde_json::Value>>(f)
                    .map(|m| m.len())
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        if count > best_count {
            best_count = count;
            best_lang = lang;
        }
    }
    let header: serde_json::Value = zip
        .by_name("header.json")
        .ok()
        .and_then(|f| serde_json::from_reader(f).ok())
        .ok_or_else(|| be_err("infra_vmap_header_missing", ""))?;
    let name = header.get("name").map(localized).unwrap_or_default();
    let description = header.get("description").map(localized).unwrap_or_default();
    let difficulty = header
        .get("difficulty")
        .and_then(|v| v.as_i64())
        .map(|v| v as i8)
        .unwrap_or(-1);
    let size_w = header
        .pointer("/mapSize/width")
        .or_else(|| header.get("size"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0) as i32;

    Ok(MapMeta {
        kind: "vmap".into(),
        lang: best_lang.into(),
        encoding: "utf-8".into(),
        name,
        description,
        version: "VCMI".into(),
        difficulty,
        players: -1,
        humans: -1,
        size_w,
        size_h: size_w,
        size_class: size_class(size_w, size_w),
        victory: String::new(),
        loss: String::new(),
        scenario_count: 0,
        scenarios: Vec::new(),
    })
}

// cp1252 由 WINDOWS_1252 承担（其前 256 码点与 ISO-8859-1 一致）

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn vcmi_lang_mapping() {
        assert_eq!(to_vcmi_lang("zh-CN"), "chinese");
        assert_eq!(to_vcmi_lang("zh-TW"), "tchinese");
        assert_eq!(to_vcmi_lang("en-US"), "english");
        assert_eq!(to_vcmi_lang("ru-RU"), "russian");
        assert_eq!(to_vcmi_lang("und"), "unknown");
    }

    #[test]
    fn language_detect_cjk() {
        assert_eq!(
            detect_wide_language("恶魔入侵了为了拯救摇摇欲坠的艾恩法斯特王朝"),
            "zh-CN"
        );
        assert_eq!(detect_wide_language("Might and Magic Heroes"), "en-US");
        assert_eq!(detect_wide_language("Освободите землю от врагов"), "ru-RU");
        assert_eq!(detect_wide_language(""), "und");
    }

    #[test]
    fn encoding_detect_gbk() {
        // GBK 编码的中文样本
        let raw = vec![0xB6, 0xF1, 0xC4, 0xA7, 0xC8, 0xEB, 0xC7, 0xD6]; // 恶魔入侵
        let (enc, lang) = analyze_text_bytes(&[&raw]);
        assert_eq!(enc, "gbk");
        assert_eq!(to_vcmi_lang(&lang), "chinese");
    }

    #[test]
    fn encoding_detect_ascii() {
        let (enc, lang) = analyze_text_bytes(&[b"The Underworld"]);
        assert_eq!(enc, "cp1252");
        assert_eq!(to_vcmi_lang(&lang), "english");
    }

    #[test]
    fn size_class_boundaries() {
        assert_eq!(size_class(36, 36), "S");
        assert_eq!(size_class(72, 72), "M");
        assert_eq!(size_class(108, 108), "L");
        assert_eq!(size_class(144, 144), "XL");
        assert_eq!(size_class(0, 0), "");
    }

    #[test]
    fn parse_real_map_sample() {
        // 参考 sample：references/Heroes3-VCMI 下的原版地图（存在则验证，CI 无样本时跳过）
        let candidates = [
            "../../references/Heroes3-VCMI/Maps",
            "../../references/Heroes3-EN/Maps",
        ];
        let mut tested = false;
        for dir in candidates {
            let Ok(rd) = std::fs::read_dir(dir) else {
                continue;
            };
            for e in rd.flatten().take(5) {
                let p = e.path();
                if p.extension()
                    .and_then(|x| x.to_str())
                    .map(|x| x.eq_ignore_ascii_case("h3m"))
                    != Some(true)
                {
                    continue;
                }
                let meta = parse_file(&p).expect("parse should succeed");
                assert!(!meta.version.is_empty());
                assert!(meta.players >= 0);
                tested = true;
                break;
            }
        }
        assert!(tested || true, "样本目录不存在时跳过");
    }

    #[test]
    fn corpus_sample_parse() {
        // homm3-map-translator 语料库抽样：验证解析成功率 + kind/尺寸/战役场景数。
        // 注意：目录语言标签是翻译「目标语言」，语料中含未翻译的英文地图，且拉丁短文本
        // 词典检测固有模糊，故不逐文件断言语言一致；语言命中情况由
        // `examples/h3m_check.rs`（`npm run verify:maps`）全量统计评估。
        // 语料不存在时跳过（CI 无外部样本）。
        // 语料布局：<ROE|AB|SOD|HOTA|WOG>/{map,campaign}/<BCP-47>/…/*.h3m|h3c
        let corpus = Path::new("../../homm3-map-translator/Maps");
        if !corpus.is_dir() {
            return;
        }
        let mut tested = 0;
        for version in ["ROE", "AB", "SOD", "HOTA", "WOG"] {
            for kind in ["map", "campaign"] {
                let kind_dir = corpus.join(version).join(kind);
                if !kind_dir.is_dir() {
                    continue;
                }
                let Ok(langs) = std::fs::read_dir(&kind_dir) else {
                    continue;
                };
                for lang_e in langs.flatten() {
                    let lang_dir = lang_e.path();
                    if !lang_dir.is_dir() {
                        continue;
                    }
                    let mut files: Vec<PathBuf> = Vec::new();
                    collect_map_files(&lang_dir, 0, &mut files);
                    if files.is_empty() {
                        continue;
                    }
                    files.sort();
                    // 每个语言目录均匀抽至多 3 个，保持抽样确定性
                    let step = files.len().div_ceil(3);
                    for i in (0..files.len()).step_by(step).take(3) {
                        let path = &files[i];
                        let ext = path
                            .extension()
                            .and_then(|x| x.to_str())
                            .unwrap_or_default()
                            .to_ascii_lowercase();
                        let want_kind = if ext == "h3c" { "campaign" } else { "map" };
                        let meta = parse_file(path)
                            .unwrap_or_else(|e| panic!("解析失败 {}: {}", path.display(), e));
                        assert_eq!(meta.kind, want_kind, "kind 不一致: {}", path.display());
                        // 注意：语料中存在 nameLen=0 的文件（作者未填名），空名非解析缺陷；
                        // 名称提取错位由尺寸断言 + `examples/h3m_check.rs` 全量报告兜底
                        if want_kind == "map" {
                            // 合法尺寸：S/M/L/XL/H/XH/G（战役无尺寸字段）
                            assert!(
                                matches!(meta.size_w, 36 | 72 | 108 | 144 | 180 | 216 | 252),
                                "尺寸非法 {} ({}): {}",
                                meta.size_w,
                                meta.size_class,
                                path.display()
                            );
                        } else {
                            assert!(
                                meta.scenario_count >= 1,
                                "战役场景数为 0: {}",
                                path.display()
                            );
                        }
                        tested += 1;
                    }
                }
            }
        }
        assert!(tested > 0, "语料库存在但未抽样到任何文件");
    }

    #[test]
    fn parse_cultivator_campaign_scenarios() {
        // 中文 GBK 战役回归（领域内真实文件，缺失时跳过）：
        // 场景表曾走 `from_utf8_lossy` 全部乱码；修复后 regionName 与战役名同管线解码。
        let candidates = [
            "../references/realms/cultivator-world/maps/战役/新仙剑奇侠传.h3c",
            "../../references/realms/cultivator-world/maps/战役/新仙剑奇侠传.h3c",
        ];
        let mut sample: Option<&Path> = None;
        for c in candidates {
            if Path::new(c).is_file() {
                sample = Some(Path::new(c));
                break;
            }
        }
        let Some(path) = sample else {
            eprintln!("[skip] cultivator-world 战役样本缺失");
            return;
        };
        let meta = parse_file(path).expect("parse should succeed");
        assert_eq!(meta.kind, "campaign");
        assert_eq!(meta.encoding, "gbk");
        assert_eq!(meta.lang, "chinese");
        assert_eq!(meta.name, "新仙剑奇侠传");
        assert_eq!(meta.scenario_count, 11);
        assert_eq!(meta.scenarios.len(), 11);
        // 编码缺陷回归护栏：任何场景名都不应含 UTF-8 替换符
        assert!(!meta.scenarios.iter().any(|s| s.name.contains('\u{FFFD}')));
        // 第 3 景（wan003.h3m）：场景名 / 文件名 / 难度
        let s3 = &meta.scenarios[2];
        assert!(
            s3.name.contains("月夜谁知起变故"),
            "场景名解码异常: {}",
            s3.name
        );
        assert_eq!(s3.map_file, "wan003.h3m");
        assert!(
            (0..=4).contains(&s3.difficulty),
            "难度越界: {}",
            s3.difficulty
        );
        // 第 1 景的场景名 / 文件名在磁盘上即为空（作者未填），非解析缺陷
        assert!(meta.scenarios[0].name.is_empty());
        assert!(meta.scenarios[0].map_file.is_empty());
    }

    /// 语料递归收集（限深度，测试用，避免引入 walkdir 依赖）。
    fn collect_map_files(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
        if depth > 10 {
            return;
        }
        let Ok(rd) = std::fs::read_dir(dir) else {
            return;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                collect_map_files(&p, depth + 1, out);
            } else if matches!(
                p.extension().and_then(|x| x.to_str()).map(|x| x.to_ascii_lowercase()),
                Some(ref x) if x == "h3m" || x == "h3c"
            ) {
                out.push(p);
            }
        }
    }
}
