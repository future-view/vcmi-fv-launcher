//! 宽容 JSON 解析（JSON5 子集）。
//!
//! VCMI 及其配置文件（`mod.json` / `town.json` / `dirs.json` / `modSettings.json` 等）
//! 允许 JSON5 风格写法：行内 `//` 注释、`\'` 转义、无引号键、尾逗号。
//! 手工编辑的 `launcher.json` / `realm.json` 也常带注释与尾逗号。
//!
//! 策略：**标准 JSON 优先**（避免预处理破坏合法 JSON），失败再走宽容预处理。
//!
//! 两个入口：
//! - [`parse_json_lenient`]：返回 `Result`。**写回前必须用它并据错中止**——
//!   把解析失败静默降级为空对象会在回写时清空用户已有全部数据。
//! - [`read_json_file_lenient`]：读文件便利函数，文件不存在返回 `Ok(None)`。

use crate::infra::berror::be_err;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// 宽容解析 JSON5 子集；返回 `Err` 表示确实无法解析（**不要**降级为空对象）。
pub fn parse_json_lenient(text: &str) -> Result<Value, String> {
    // 标准 JSON 优先：能直接解析就直接返回（避免预处理破坏合法 JSON）
    if let Ok(v) = serde_json::from_str::<Value>(text) {
        return Ok(v);
    }
    let cleaned = sanitize_vcmi_json(text);
    serde_json::from_str::<Value>(&cleaned)
        .map_err(|e| be_err("cmd_json_parse_failed", &format!("{}", e)))
}

/// 读取文件并宽容解析。
///
/// - 文件不存在 → `Ok(None)`
/// - 读文件失败 / 解析失败 → `Err`（调用方应报错，不得静默降级）
pub fn read_json_file_lenient(path: &Path) -> Result<Option<Value>, String> {
    if !path.is_file() {
        return Ok(None);
    }
    let text = fs::read_to_string(path)
        .map_err(|e| be_err("infra_file_read_failed", &format!("{}", e)))?;
    parse_json_lenient(&text).map(Some)
}

/// 将 VCMI 风格的 JSON 预处理为合法 JSON：
/// 1. 去掉行内 `//` 注释（不在字符串内时才删）。
/// 2. 将字符串内的 `\'` 转义替换为字面 `'`（VCMI 允许 `'` 作为转义引号）。
/// 3. 去除对象/数组末尾多余的逗号。
/// 4. 修正前导零整数（`083` → `83`，见 [`fix_leading_zero_numbers`]）。
fn sanitize_vcmi_json(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let mut in_str = false;
    while i < chars.len() {
        let c = chars[i];
        if in_str {
            // 处理转义
            if c == '\\' && i + 1 < chars.len() {
                let nxt = chars[i + 1];
                if nxt == '\'' {
                    out.push('\'');
                    i += 2;
                    continue;
                }
                out.push(c);
                out.push(nxt);
                i += 2;
                continue;
            }
            out.push(c);
            if c == '"' {
                in_str = false;
            }
            i += 1;
        } else if c == '"' {
            in_str = true;
            out.push(c);
            i += 1;
        } else if c == '/' && i + 1 < chars.len() && chars[i + 1] == '/' {
            // 跳过直到行尾
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else {
            out.push(c);
            i += 1;
        }
    }
    fix_leading_zero_numbers(&strip_trailing_commas(&out))
}

/// 修正**前导零整数**：`083` → `83`、`-007` → `-7`、`00` → `0`。
///
/// 为什么需要：标准 JSON 语法禁止数字带前导零，而 VCMI 的 `JsonParser` 对数字走
/// `strtol` 式解析，`083` 就是 83（实测老中文包/苦修士等城镇配置里大量出现
/// `"y" : 083`、`"x" : 07` 这类手工对齐写法）。后果不是"这一个数错"，而是
/// **整份文件**解析失败 —— `parse_json_lenient` 返回 `Err` → `parse_vcmi_json`
/// 降级成 `Null` → `build_town_config` 得到空对象 → faction_id 为空 → 被
/// `read_town_configs` 的 `retain(!faction_id.is_empty())` 整条丢弃。
///
/// 实测 `8-Dervish-town`（苦修士）的 `town/structures.json` 第 31 行 `"y" : 083`：
/// 该文件承载 39 个建筑的屏幕位置与 `animation`，整份丢掉后 `TownConfig.structures`
/// 为空 → `build_towns` 不做结构叠加 → 城镇幕布图只剩裸底图（用户报"城镇背景图没有建筑"）。
///
/// 改动范围刻意收紧：
/// - **只动 token 的整数部分**：`083` → `83`、`083.5` → `83.5`、`083e2` → `83e2`；
///   小数部分与指数部分原样保留（`0.083` 本来就合法，`083.5` 的 `.5` 不该被动）。
/// - **字符串字面量内一律不碰**：资源路径里全是 `0tbelmage.def`、`"083"` 这类内容，
///   误改会直接弄坏资源引用。
///
/// 数字 token 的起点判定：前一个**有效字符**属于 `{` `[` `,` `:` `-` 或位于文件开头
/// （`-` 覆盖 `-083` 的负号形态；后者作为 token 起点时会把负号也纳入收集）。
fn fix_leading_zero_numbers(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let mut in_str = false;
    // 上一个非空白字符（进入字符串后统一记为 `"`，避免把字符串内容当成数字上下文）
    let mut prev: Option<char> = None;
    while i < chars.len() {
        let c = chars[i];
        if in_str {
            out.push(c);
            if c == '\\' && i + 1 < chars.len() {
                out.push(chars[i + 1]);
                i += 2;
                continue;
            }
            if c == '"' {
                in_str = false;
                prev = Some('"');
            }
            i += 1;
            continue;
        }
        if c == '"' {
            in_str = true;
            out.push(c);
            prev = Some('"');
            i += 1;
            continue;
        }
        let at_value_start = c.is_ascii_digit()
            && matches!(
                prev,
                None | Some('{') | Some('[') | Some(',') | Some(':') | Some('-')
            );
        if at_value_start {
            let mut j = i;
            while j < chars.len()
                && (chars[j].is_ascii_digit()
                    || matches!(chars[j], '.' | 'e' | 'E' | '+' | '-'))
            {
                j += 1;
            }
            let token: String = chars[i..j].iter().collect();
            // 拆成「整数部分 | 小数部分 | 指数部分」，只对整数部分去前导零：
            // `083` → `83`、`083.5` → `83.5`、`083e2` → `83e2`。
            let (mantissa, exp) = match token.find(['e', 'E']) {
                Some(k) => (&token[..k], &token[k..]),
                None => (token.as_str(), ""),
            };
            let (int_part, frac) = match mantissa.find('.') {
                Some(k) => (&mantissa[..k], &mantissa[k..]),
                None => (mantissa, ""),
            };
            if int_part.len() > 1 && int_part.starts_with('0') {
                let trimmed = int_part.trim_start_matches('0');
                out.push_str(if trimmed.is_empty() { "0" } else { trimmed });
            } else {
                out.push_str(int_part);
            }
            out.push_str(frac);
            out.push_str(exp);
            prev = Some(chars[j - 1]);
            i = j;
            continue;
        }
        if !c.is_whitespace() {
            prev = Some(c);
        }
        out.push(c);
        i += 1;
    }
    out
}

/// 去除尾逗号（`,` 后跟可选空白再跟 `}` 或 `]`），逐字符扫描。
fn strip_trailing_commas(s: &str) -> String {
    let v: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < v.len() {
        let c = v[i];
        if c == ',' && i + 1 < v.len() {
            let mut j = i + 1;
            while j < v.len() && matches!(v[j], ' ' | '\t' | '\n' | '\r') {
                j += 1;
            }
            if j < v.len() && (v[j] == '}' || v[j] == ']') {
                i = j;
                continue;
            }
        }
        out.push(c);
        i += 1;
    }
    out
}

/// 统一解析 VCMI 模组配置文件（mod.json / town.json / faction.json / creatures 等）。
/// VCMI `JsonParser` 在 JSON5 模式下容忍：行内 `//` 注释、`\'` 转义、无引号键、尾逗号。
/// `parse_vcmi_json` 先尝试标准 JSON，失败再走宽容预处理，故合法 JSON 也不受影响。
///
/// 这是读取 VCMI 模组配置的唯一入口：scan 阶段（识别 modType / factions）与 build 阶段
/// （读取 town / creature 配置）共用同一解析器。此前 scan 用另一套"只去尾逗号"的
/// `read_mod_json`，遇到含 `//` 注释的 mod.json 会解析失败、误判 modType，与 build 阶段
/// 表现不一致——这是城镇不全的根因之一。
pub(crate) fn read_vcmi_json(path: &Path) -> Value {
    // 「尽力而为」：文件缺失 / 解析失败一律返回 Null（模组配置允许缺失）
    crate::infra::json::read_json_file_lenient(path)
        .ok()
        .flatten()
        .unwrap_or(Value::Null)
}

pub(crate) fn read_vcmi_json_bytes(bytes: &[u8]) -> Value {
    let text = String::from_utf8_lossy(bytes);
    parse_vcmi_json(&text)
}

pub(crate) fn parse_vcmi_json(text: &str) -> Value {
    // 宽容解析（JSON5 子集：去 // 注释 + 处理 \' 转义 + 去尾逗号）见 `crate::infra::json`。
    // 这里是「尽力而为」包装：失败返回 Null（模组配置允许缺失/损坏）。
    crate::infra::json::parse_json_lenient(text).unwrap_or(Value::Null)
}

pub(crate) fn collect_all_json_files(dir: &Path, out: &mut Vec<PathBuf>) {
    if let Ok(entries) = fs::read_dir(dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                collect_all_json_files(&p, out);
            } else if p
                .file_name()
                .map(|n| n.to_string_lossy().ends_with(".json"))
                .unwrap_or(false)
            {
                out.push(p);
            }
        }
    }
}

/// 段名与类型名的等价判定（含单复数容错与别名）。
fn type_name_eq(s: &str, t: &str) -> bool {
    if s == t {
        return true;
    }
    // 单复数容错：artifact ↔ artifacts、creature ↔ creatures、hero ↔ heroes…
    if t.ends_with('s') && s == t.trim_end_matches('s') {
        return true;
    }
    if s.ends_with('s') && s.trim_end_matches('s') == t {
        return true;
    }
    // heroClasses 的目录别名 classes（HuaXia 等把职业配置放 config/<mod>/classes/）
    if t == "heroclasses" && s == "classes" {
        return true;
    }
    false
}

/// 配置目录名/文件名与类型名的容错匹配。
///
/// VCMI 生态里存在**大小写与单复数写法差异**，严格段相等（`s == type_subdir`）会整批漏采：
/// - 实测 `config/artifact/ffArt1.json`（单数目录）→ 8 个宝物漏采；
/// - `config/<mod>/Creatures/<x>.json`（大写目录）→ 5 个生物漏采；
/// - `config/<mod>/classes/*.json`（heroClasses 别名，HuaXia）；
/// - **分隔符后缀**目录：`config/Abode-creatures/`、`config/Abode-heroes/`
///   （家园 mod 的写法）→ 生物/英雄整批漏采（index `creatures:0, heroes:0`）。
///   末段与类型名等价才命中（不做"包含即命中"，避免误判）。
pub(crate) fn type_seg_match(seg: &str, type_subdir: &str) -> bool {
    let s = seg.to_lowercase();
    let t = type_subdir.to_lowercase();
    if type_name_eq(&s, &t) {
        return true;
    }
    // `<前缀>-<类型>` / `<前缀>_<类型>`：取最后一个 `-`/`_` 之后的部分比对
    let tail = s.rsplit(|c| c == '-' || c == '_').next().unwrap_or(&s);
    tail != s && type_name_eq(tail, &t)
}

/// 该路径是否为指定类型的实体配置（目录段容错 + 聚合文件名匹配）。
///
/// 聚合文件名（如 `config/<mod>/heroes.json`）也算命中，但**排除 town 配置目录**：
/// `<faction>/town/spells.json` 是魔法行会概率表（`{ "<fid>": { "town": { "guildSpells": ... } } }`），
/// 若按聚合文件采入会生成以 faction id 为 id 的伪实体。
///
/// ⚠️ **「其实属于别的类型」一票否决**（2026-10-16 修正）：判定只看**最后一个 `config` 段
/// 之后**的子路径（锚点之前是模组目录名 / 游戏目录名，与实体类型无关）。
///
/// 起因：`type_seg_match` 的 `<前缀>-<类型>` 变体会命中**模组目录名本身** —— 实测
/// `immortal-cultivation-heroes`（及子模组 `L-skills` / `artifactsplus`）把
/// `config/skills/skXx.json`、`config/artifacts/*.json` 整批采成"英雄"，产出无头像、
/// 无职业、特长只有一个 ✦ 的幽灵行（`heroes` 分类 62/67 的无图条目全属此类）；
/// `new-monsters-pack.kurek-creatures.*`（目录名 `…-creatures`）对"生物"同理。
///
/// 取舍：**只在"锚点之后的子路径明确命中另一个已知类型目录"时才否决**；子路径没有
/// 类型目录时（如 `config/cherub/cherub.json`、`config/mapspells/accuracy.json`）仍沿用
/// 旧的模组目录名容错——它们正是靠这条容错才被采到的真实实体（实测 404+94 个文件），
/// 一刀切锚定会把这些真实生物/法术整批弄丢。
pub(crate) fn is_type_config_path(rel_lower: &str, type_subdir: &str) -> bool {
    let all: Vec<&str> = rel_lower.split('/').collect();
    let fname = all.last().copied().unwrap_or("");
    // 取**最后一个** `config` 段之后的部分做"属于别的类型"判定；没有 `config` 段时不否决。
    let start = all
        .iter()
        .rposition(|s| *s == "config")
        .map(|i| i + 1)
        .unwrap_or(0);
    let segs = &all[start..];
    let t = type_subdir.to_lowercase();
    let is_other = |name: &str| -> bool {
        KNOWN_TYPE_DIRS
            .iter()
            .any(|k| type_seg_match(name, k) && !type_name_eq(&k.to_lowercase(), &t))
    };
    // ① 目录段命中别的类型目录（如 `config/skills/…` 采"英雄"）→ 否决
    let other_dir = segs[..segs.len().saturating_sub(1)].iter().any(|s| is_other(s));
    // ② 聚合文件名本身就是别的类型名（如 `config/x/creatures.json` 采"英雄"）→ 否决
    let other_agg = is_other(fname.trim_end_matches(".json"));
    if other_dir || other_agg {
        return false;
    }
    // ⚠️ 只在**最后一个 `config` 段之后**找类型目录，不能扫全路径：
    // 文件系统路径里还有 `Mods/<mod>/…` 这些段，其中**模组目录名**可能恰好等于某个类型名——
    // 实测 `Mods/hota/Mods/artifacts/Content/config/artiSpells/fakeHornSpell.json`：
    // 该 mod 的目录名就是 `artifacts`，扫全路径会把 `artiSpells/` 里的**法术**当成宝物收录
    // （百科里出现一件"召唤夺心魔"、无价格无槽位的假宝物）。
    let in_dir = segs[..segs.len().saturating_sub(1)]
        .iter()
        .any(|s| type_seg_match(s, type_subdir));
    let is_agg = fname.trim_end_matches(".json") == t
        && !segs
            .iter()
            .any(|s| matches!(*s, "town" | "towns" | "translation"));
    in_dir || is_agg
}

/// 已知的**实体类型目录名**（用于「其实属于别的类型」判定，见 [`is_type_config_path`]）。
const KNOWN_TYPE_DIRS: &[&str] = &[
    "creatures",
    "heroes",
    "heroClasses",
    "spells",
    "artifacts",
    "skills",
    "objects",
    "bonuses",
    "terrains",
    "factions",
    "buildings",
    "town",
];

/// 递归收集 `config/**/<type_subdir>/*.json` 的文件路径（含大小写/单复数容错与聚合文件）。
pub(crate) fn collect_json_files(dir: &Path, type_subdir: &str, out: &mut Vec<PathBuf>) {
    // 走**目录清单缓存**（见 `infra::dirscan`）：原实现每个类型都 `fs::read_dir` 全量递归一遍，
    // 而同一次构建里会对同一个 `Content/` 收集 6+ 种类型（creatures/heroes/spells/skills/
    // artifacts×2…），加上城镇目录索引与 raw 全量归一，约 9 次重复遍历。
    // 改为"只递归一次 + 纯内存过滤"，且**匹配口径完全不变**（仍是绝对路径小写归一）。
    let listing = crate::infra::dirscan::dir_listing(dir);
    for i in 0..listing.len() {
        let abs = listing.abs(i);
        // 仍传**绝对路径**：`is_type_config_path` 内部已把匹配锚定到最后一个 `config` 段之后
        // （见其文档），mod 目录名不再参与匹配；而"路径里根本没有 `config` 段"的兜底场景
        // （如游戏根 `config/<sub>/x.json` 之外的写法）仍需整条路径。
        if !abs.ends_with(".json") {
            continue;
        }
        if is_type_config_path(abs, type_subdir) {
            out.push(listing.path(i).as_ref().to_path_buf());
        }
    }
}

#[cfg(test)]
mod leading_zero_tests {
    use super::*;

    fn parse(text: &str) -> Value {
        parse_json_lenient(text).unwrap_or_else(|e| panic!("应能解析: {e}"))
    }

    #[test]
    fn 前导零整数被归一() {
        // 实测形态：苦修士 town/structures.json 的 `"y" : 083`
        let v = parse(r#"{ "x": 132, "y": 083, "z": -2 }"#);
        assert_eq!(v["y"], 83);
        assert_eq!(v["z"], -2);
    }

    #[test]
    fn 负号与多位前导零() {
        let v = parse(r#"{ "a": -083, "b": 007, "c": 00, "d": 0 }"#);
        assert_eq!(v["a"], -83);
        assert_eq!(v["b"], 7);
        assert_eq!(v["c"], 0);
        assert_eq!(v["d"], 0);
    }

    #[test]
    fn 字符串与浮点不受影响() {
        // 资源路径里的前导零必须原样保留，否则资源引用会被改坏
        let v = parse(
            r#"{ "anim": "Dervish/buildings/0tbelmage.def", "s": "083", "f": 0.083, "g": 083.5 }"#,
        );
        assert_eq!(v["anim"], "Dervish/buildings/0tbelmage.def");
        assert_eq!(v["s"], "083");
        assert_eq!(v["f"], 0.083);
        assert_eq!(v["g"], 83.5);
    }

    #[test]
    fn 数组内前导零() {
        let v = parse(r#"[1, 02, 300]"#);
        assert_eq!(v[1], 2);
        assert_eq!(v[2], 300);
    }

    #[test]
    fn 与注释尾逗号共存() {
        // 同一文件里三种非法形态叠加（VCMI 手工配置常见）
        let v = parse(
            "{\n  // 注释\n  \"y\": 083,\n  \"x\": 07,\n}",
        );
        assert_eq!(v["y"], 83);
        assert_eq!(v["x"], 7);
    }

    #[test]
    fn 合法json仍走标准路径() {
        let v = parse(r#"{ "a": 1, "b": "0x0" }"#);
        assert_eq!(v["a"], 1);
        assert_eq!(v["b"], "0x0");
    }
}

/// 配置类型路径判定回归（2026-10-16：模组目录名污染实体分类的修复）。
///
/// 背景：`heroes` / `creatures` 分类里出现大量"无头像、无职业"的幽灵行，根因是
/// `<前缀>-<类型>` 的目录容错命中了**模组目录名**（`immortal-cultivation-heroes`、
/// `kurek-creatures`），导致该模组下**所有** JSON 都被采成英雄/生物。
#[cfg(test)]
mod type_path_tests {
    use super::*;

    #[test]
    fn 别的类型目录下的配置不得被采成本类型() {
        // 技能配置 —— 不得因为模组目录名 `-heroes` 被当成英雄
        let p = "d:/games/Mods/immortal-cultivation-heroes/Mods/L-skills/content/config/skills/skxx.json";
        assert!(!is_type_config_path(p, "heroes"), "skills/ 下的配置不是英雄");
        assert!(is_type_config_path(p, "skills"), "skills/ 下的是技能");

        // 宝物配置 —— 同上
        let a = "d:/games/Mods/immortal-cultivation-heroes/Mods/artifactsplus/content/config/artifacts/pendantofdeath.json";
        assert!(!is_type_config_path(a, "heroes"));
        assert!(is_type_config_path(a, "artifacts"));

        // 英雄目录下的文件 —— 不得被当成生物（模组目录名 `kurek-creatures`）
        let h = "d:/games/Mods/new-monsters-pack/Mods/kurek-creatures/Mods/wasps/content/config/wasps/heroes/bumblewasp.json";
        assert!(!is_type_config_path(h, "creatures"));
        assert!(is_type_config_path(h, "heroes"));
    }

    #[test]
    fn 认类型只看配置目录之后的路径() {
        // 2026-10-19 修正：**不再把模组目录名当类型目录**。
        //
        // 旧实现扫整条绝对路径，于是 `Mods/hota/Mods/artifacts/.../artiSpells/fakeHornSpell.json`
        // （mod 目录名恰好叫 artifacts，文件其实是**法术**）被采成宝物 —— 百科里多出一件
        // "召唤夺心魔"：无价格、无槽位。类型只能由 `config` 段之后的目录/文件名决定。
        assert!(!is_type_config_path(
            "d:/g/Mods/hota/Mods/artifacts/Content/config/artiSpells/fakeHornSpell.json",
            "artifacts"
        ));
        // 同理：mod 目录名含 "creatures" 不代表它的每个文件都是生物
        assert!(!is_type_config_path(
            "d:/games/Mods/new-monsters-pack/Mods/kurek-creatures/Mods/cherub/content/config/cherub/cherub.json",
            "creatures"
        ));
        assert!(!is_type_config_path(
            "d:/games/Mods/adventure-spells-pack/Mods/air-spells/content/config/mapspells/accuracy.json",
            "spells"
        ));
        // 聚合文件：文件名不是类型名 → 目录扫描不采（这类 mod 靠 mod.json 的
        // `"creatures": ["config/cherub/cherub.json"]` 清单装载，见 collect.rs 的
        // `mod_json_type_files`）
        assert!(!is_type_config_path(
            "d:/g/Mods/fused-artifacts/content/config/fusedartifacts.json",
            "artifacts"
        ));
        // 反向确认：`config` 之后的目录名仍然有效（含旧容错：分隔符后缀、单复数、大写）
        assert!(is_type_config_path(
            "d:/g/Mods/abode/content/config/abode-creatures/x.json",
            "creatures"
        ));
        assert!(is_type_config_path(
            "d:/games/Mods/new-monsters-pack/Mods/kurek-creatures/Mods/cherub/content/config/creatures/x.json",
            "creatures"
        ));
    }

    #[test]
    fn 既有容错不受影响() {
        // 分隔符后缀目录（家园 mod 写法）
        assert!(is_type_config_path(
            "d:/g/Mods/abode/content/config/abode-creatures/x.json",
            "creatures"
        ));
        // 聚合文件 `<mod>/heroes.json`
        assert!(is_type_config_path("d:/g/Mods/m/content/config/m/heroes.json", "heroes"));
        // 单复数 / 大写目录
        assert!(is_type_config_path("d:/g/Mods/m/content/config/m/artifact/a.json", "artifacts"));
        assert!(is_type_config_path("d:/g/Mods/m/content/config/m/Creatures/a.json", "creatures"));
        // 游戏根 `config/<sub>/<file>.json`（`read_game_root_configs` 走这条）
        assert!(is_type_config_path("d:/g/config/creatures/castle.json", "creatures"));
        // 魔法行会概率表：town 目录下不得被采成法术
        assert!(!is_type_config_path(
            "d:/g/Mods/m/content/config/m/town/spells.json",
            "spells"
        ));
    }
}
