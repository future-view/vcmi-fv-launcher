//! 能力图标（`bonusToGraphics`）回归（2026-10-16）。
//!
//! VCMI 百科生物页的能力图标是**两级**来源（`client/windows/wiki/WikiCreatureContent.cpp:307-309`）：
//!
//! ```cpp
//! const ImagePath icon = !b->customIconPath.empty() ? b->customIconPath   // ① 生物配置 ability["icon"]
//!                                                 : LIBRARY->bth->bonusToGraphics(b); // ② bonuses.json
//! ```
//!
//! ② 的数据在真实发行版里由 `vcmi-extras` 的 **Bonus Icons** 子模组提供
//! （`Mods/vcmi-extras/Mods/bonusIcons/mods/Bonus Icons/Content/config/bonusIcons/bonuses.json`，
//! 为每个 `core:<TYPE>` 定义 `graphics.icon`，资源是 `Content/data/zvs/Lib1.res/<NAME>.png`）。
//!
//! 用户报"所有技能图片均为空图"时的实测：19,376 条能力里只有 1,042 条有图标（5.4%），
//! 且全是有 ① 的；② 那条链路没产出任何图标。本组用例锁住：表能读到、图标能落到磁盘。
//!
//! 运行：`VCMI_GAME_DIR=<游戏目录> cargo test --lib ability_icons -- --ignored --nocapture`

use super::*;

use crate::vcmi::bonus::bonus_graphics_map;
use crate::vcmi::resource::{set_wiki_mod_paths, set_wiki_user_data_dir};
use crate::vcmi::scan::cached_mod_paths;
use crate::vcmi::spell_icons::{spell_immune_icon, spell_immune_icons};

/// 注入真实安装的模组集合（跨模组资源解析与 bonus 表扫描都依赖它）。
fn setup_env(game_dir: &Path) {
    let paths = VcmiPaths::detect();
    set_wiki_user_data_dir(&user_data_dir(game_dir));
    set_wiki_mod_paths(&cached_mod_paths(&paths));
}

fn game_mod(mod_rel: &str) -> Option<PathBuf> {
    let p = game_dir().join("Mods").join(mod_rel);
    p.is_dir().then_some(p)
}

/// ① bonus 类型 → 图标资源表：必须包含 `Bonus Icons` 子模组提供的条目。
#[test]
#[ignore]
fn bonus_graphics_table_reads_bonus_icons_submod() {
    let game_dir = game_dir();
    setup_env(&game_dir);
    let table = bonus_graphics_map(&game_dir);
    println!("bonus 图标表：{} 个类型", table.len());
    for t in [
        "ADDITIONAL_ATTACK",
        "DEATH_STARE",
        "TWO_HEX_ATTACK_BREATH",
        "JOUSTING",
    ] {
        println!("  {t} = {:?}", table.get(t));
    }
    assert!(
        table.len() > 1,
        "bonus 图标表为空：没扫到任何 mod 的 bonuses.json（真实发行版的图标来自 vcmi-extras 的 Bonus Icons 子模组）"
    );
    let aa = table
        .get("ADDITIONAL_ATTACK")
        .and_then(|g| g.icon.clone())
        .expect("ADDITIONAL_ATTACK 应有 graphics.icon（Bonus Icons 子模组提供）");
    assert!(
        !aa.is_empty(),
        "ADDITIONAL_ATTACK 的图标资源不应为空字符串"
    );
}

/// ② 端到端：完整构建一个带 `bonusToGraphics` 图标的 mod，能力图标必须落到磁盘。
#[test]
#[ignore]
fn ability_icons_are_extracted_for_bonus_graphics() {
    let game_dir = game_dir();
    setup_env(&game_dir);
    // `1-Knight-town` 的 6 级兵带 DEATH_STARE / JOUSTING（都没有 ① 的 ability["icon"]）
    let Some(mod_dir) = game_mod("1-Knight-town") else {
        eprintln!("跳过：未安装 1-Knight-town");
        return;
    };
    let mod_id = "1-knight-town";
    let cache_root = std::env::temp_dir().join("vcmi_fv_ability_icon_test");
    let mod_cache = cache_root.join(crate::modcache::sanitize_id(mod_id));
    let _ = fs::remove_dir_all(&mod_cache);
    let _ = fs::create_dir_all(&cache_root);
    let r = build_mod_wiki_phase(&mod_dir.to_string_lossy(), mod_id, &cache_root, &game_dir, 2);
    assert_eq!(
        r.get("ok").and_then(|x| x.as_bool()),
        Some(true),
        "构建失败：{r}"
    );
    let text = fs::read_to_string(mod_cache.join("cache.json")).expect("cache.json 未落盘");
    let data: Value = serde_json::from_str(&text).expect("cache.json 解析失败");

    let mut total = 0usize;
    let mut with_icon = 0usize;
    let mut missing_file = Vec::new();
    for c in data.get("creatures").and_then(|x| x.as_array()).unwrap_or(&vec![]) {
        for a in c.get("abilities").and_then(|x| x.as_array()).unwrap_or(&vec![]) {
            total += 1;
            match a.get("icon").and_then(|x| x.as_str()) {
                Some(rel) if !rel.is_empty() => {
                    with_icon += 1;
                    if !cache_root.join(rel).is_file() {
                        missing_file.push(format!(
                            "{}.{} → {rel}",
                            c.get("id").and_then(|x| x.as_str()).unwrap_or("?"),
                            a.get("type").and_then(|x| x.as_str()).unwrap_or("?")
                        ));
                    }
                }
                _ => {}
            }
        }
    }
    println!("能力 {total} 条，有图标 {with_icon} 条，文件缺失 {} 条", missing_file.len());
    for m in missing_file.iter().take(5) {
        println!("  缺文件：{m}");
    }
    assert!(total > 0, "该 mod 应至少有若干能力");
    assert!(
        with_icon > 0,
        "bonusToGraphics 一路没产出任何能力图标（表读到了但资源提取失败？）"
    );
    assert!(missing_file.is_empty(), "图标声明了但文件没落盘：{missing_file:?}");
    // 覆盖率护栏：全发行版预估约 82%，这个 mod 实测 35/42。若回归到"几乎全空"（如旧实现 0 条）
    // 必须失败 —— 这正是用户反复报的"所有技能图片均为空图"。
    assert!(
        with_icon * 2 >= total,
        "能力图标覆盖率过低（{with_icon}/{total}）：`<file>/<sub>`（zvs/Lib1.res/E_X）资源解析回归？"
    );
}

/// ③ `SPELL_IMMUNITY` 特例：这类能力的图标取自**被免疫法术自己**的 `graphics.iconImmune`
/// （`CBonusTypeHandler.cpp:109-113`），与 `bonuses.json` 无关 —— Bonus Icons 的
/// `core:SPELL_IMMUNITY` 图形段本来就是空的（注释 "Uses icon from spell config"）。
///
/// 数据来自 `vcmi-extras` 的 **Immunity Icons** 子模组，且它的两个文件**不在**
/// `Content/config/` 下：放在 `Content/` 根，由 `mod.json` 的 `"spells"` 清单声明
/// （`Mods/vcmi-extras/Mods/bonusIcons/mods/Immunity Icons/Content/spellImmunityIcons.json`，
/// 键形如 `core:blind`）。本用例锁住「`Content/config/` 之外声明的法术文件也要扫到」——
/// 只按 `config/**/spells/**` 收文件时这张表必为空、`SPELL_IMMUNITY` 能力随之恒无图。
#[test]
#[ignore]
fn spell_immunity_icons_come_from_declared_spell_files() {
    let game_dir = game_dir();
    setup_env(&game_dir);
    let table = spell_immune_icons(&game_dir);
    println!("法术 iconImmune 表：{} 条", table.len());
    for k in ["blind", "antiMagic", "armageddon", "stoneGaze", "poison"] {
        println!("  {k} = {:?}", table.get(k));
    }
    if table.is_empty() {
        eprintln!("跳过：该发行版没有法术 iconImmune 数据（未装 vcmi-extras 的 Immunity Icons 子模组）");
        return;
    }
    // 实测 `config/creatures/dungeon.json` 的 Troglodyte 写的就是 `subtype: "spell.blind"`
    assert!(
        table.contains_key("blind"),
        "Immunity Icons 子模组给 `core:blind` 定义了 iconImmune，表里却查不到"
    );
    assert_eq!(
        spell_immune_icon(&game_dir, "spell.blind"),
        table.get("blind").cloned(),
        "`spell.blind` 形态的 subtype 应归一到末段 `blind`"
    );
}

/// ④ 端到端：`SPELL_IMMUNITY` 能力的图标必须落到磁盘。
///
/// `astral-town` 的 5 级兵写 `SPELL_IMMUNITY: spell.armageddon` 等、且不带 ① `ability["icon"]`，
/// 图标只可能来自 Immunity Icons 子模组给 `core:<spellId>` 补的 `graphics.iconImmune`。
#[test]
#[ignore]
fn spell_immunity_icons_are_extracted() {
    let game_dir = game_dir();
    setup_env(&game_dir);
    let Some(mod_dir) = game_mod("astral-town") else {
        eprintln!("跳过：未安装 astral-town");
        return;
    };
    let mod_id = "astral-town";
    let cache_root = std::env::temp_dir().join("vcmi_fv_spell_immunity_icon_test");
    let mod_cache = cache_root.join(crate::modcache::sanitize_id(mod_id));
    let _ = fs::remove_dir_all(&mod_cache);
    let _ = fs::create_dir_all(&cache_root);
    let r = build_mod_wiki_phase(&mod_dir.to_string_lossy(), mod_id, &cache_root, &game_dir, 2);
    assert_eq!(r.get("ok").and_then(|x| x.as_bool()), Some(true), "构建失败：{r}");
    let text = fs::read_to_string(mod_cache.join("cache.json")).expect("cache.json 未落盘");
    let data: Value = serde_json::from_str(&text).expect("cache.json 解析失败");

    let mut total = 0usize;
    let mut with_icon = 0usize;
    let mut missing = Vec::new();
    for c in data.get("creatures").and_then(|x| x.as_array()).unwrap_or(&vec![]) {
        for a in c.get("abilities").and_then(|x| x.as_array()).unwrap_or(&vec![]) {
            if !a
                .get("type")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .eq_ignore_ascii_case("SPELL_IMMUNITY")
            {
                continue;
            }
            total += 1;
            if let Some(rel) = a.get("icon").and_then(|x| x.as_str()).filter(|s| !s.is_empty()) {
                with_icon += 1;
                if !cache_root.join(rel).is_file() {
                    missing.push(format!(
                        "{}.{} → {rel}",
                        c.get("id").and_then(|x| x.as_str()).unwrap_or("?"),
                        a.get("subtype").and_then(|x| x.as_str()).unwrap_or("?")
                    ));
                }
            }
        }
    }
    println!("SPELL_IMMUNITY 能力 {total} 条，有图标 {with_icon} 条");
    for m in missing.iter().take(5) {
        println!("  缺文件：{m}");
    }
    assert!(total > 0, "该 mod 应有 SPELL_IMMUNITY 能力");
    assert!(
        with_icon > 0,
        "特例链没产出任何 SPELL_IMMUNITY 图标（表里查到了，但 `spellimmunity/*.png` 资源解析失败？）"
    );
    assert!(missing.is_empty(), "图标声明了但文件没落盘：{missing:?}");
}

/// ⑤ 生物能力表的**垃圾行**回归：完全没有文案的能力必须整行不显示。
///
/// VCMI `WikiCreatureContent.cpp:298-307`：行文本 = 内嵌 `description`（非空时），否则
/// `bonusToString(b, creature)`；**两者都空 → `if(text.empty()) continue;`**。配置里的能力标识
/// （`debuff1` / `noCastByDefault` / `castearthquake2` …）从来不是 VCMI 的文案来源 —— 早期把它
/// 当兜底名称，于是这些"作者只为多触发一条 bonus 随手起名"的条目变成没图、没描述、
/// 名字是 camelCase 的垃圾行（用户实测 `land-of-light.ultraman` 的生物一次多出十余行）。
///
/// 顺带锁住 VCMI 同款去重（`:306-307`，`(type, subtype, text)` 相同只留一行）：该 mod 把同一条
/// `SPELL_DAMAGE_REDUCTION / spellSchool.earth` 写了两遍，此前列表里出现两行完全相同的"魔法抗性"。
#[test]
#[ignore]
fn creature_abilities_skip_textless_entries() {
    let game_dir = game_dir();
    setup_env(&game_dir);
    let Some(mod_dir) = game_mod("land-of-light/mods/ultraman") else {
        eprintln!("跳过：未安装 land-of-light 的 ultraman 子模组");
        return;
    };
    let mod_id = "land-of-light.ultraman";
    let cache_root = std::env::temp_dir().join("vcmi_fv_ability_text_test");
    let mod_cache = cache_root.join(crate::modcache::sanitize_id(mod_id));
    let _ = fs::remove_dir_all(&mod_cache);
    let _ = fs::create_dir_all(&cache_root);
    let r = build_mod_wiki_phase(&mod_dir.to_string_lossy(), mod_id, &cache_root, &game_dir, 2);
    assert_eq!(r.get("ok").and_then(|x| x.as_bool()), Some(true), "构建失败：{r}");
    let text = fs::read_to_string(mod_cache.join("cache.json")).expect("cache.json 未落盘");
    let data: Value = serde_json::from_str(&text).expect("cache.json 解析失败");

    // 该 mod 配置里这些能力标识（`abilities` 的 key）都没有任何文案 → VCMI 不显示
    let junk = [
        "debuff1",
        "debuff2",
        "debuff3",
        "debuff4",
        "debuff5",
        "debuff6",
        "buff1",
        "buff2",
        "buff3",
        "buff4",
        "buff5",
        "buff6",
        "casts",
        "noCastByDefault",
        "castearthquake2",
    ];
    let mut rows = 0usize;
    let mut with_icon = 0usize;
    let mut bad: Vec<String> = Vec::new();
    for c in data.get("creatures").and_then(|x| x.as_array()).unwrap_or(&vec![]) {
        for a in c.get("abilities").and_then(|x| x.as_array()).unwrap_or(&vec![]) {
            rows += 1;
            if a.get("icon").and_then(|x| x.as_str()).is_some() {
                with_icon += 1;
            }
            for name in [a.pointer("/name/chinese"), a.pointer("/name/english")] {
                if let Some(s) = name.and_then(|x| x.as_str()) {
                    if junk.iter().any(|k| k.eq_ignore_ascii_case(s)) {
                        bad.push(format!(
                            "{}.{} → {s}",
                            c.get("id").and_then(|x| x.as_str()).unwrap_or("?"),
                            a.get("type").and_then(|x| x.as_str()).unwrap_or("?")
                        ));
                    }
                }
            }
        }
    }
    println!("能力行 {rows} 条，有图 {with_icon} 条");
    assert!(rows > 0, "该 mod 的生物应有能力行");
    assert!(
        with_icon * 3 >= rows,
        "有图比例过低（{with_icon}/{rows}）：能力图标链回归？"
    );
    assert!(
        bad.is_empty(),
        "出现了以配置能力标识当名称的垃圾行（VCMI 此时整行不显示）：{bad:?}"
    );
}

/// ⑥ 能力文案里的 `${subtype.spell}` 必须替换成**法术名**。
///
/// 配置写 `"subtype": "spell.blind"`（VCMI 的 `type.name` 实体写法），实体名查询键必须是末段
/// `blind` —— 早期直接拿 `spell.blind` 去拼 `spell.core.spell.blind.name`，永远不命中，于是
/// `core.bonus.SPELL_IMMUNITY.description`（`{特定魔法免疫}\n免疫${subtype.spell}魔法`）
/// 显示成**"免疫魔法"**（法术名被替换成空串）。
///
/// ⚠️ 同一字段仍要用 **dotted 原文**查图标表（`subtype_icons` 的键就写成 `spellSchool.earth`），
/// 两者不能混用。
#[test]
#[ignore]
fn ability_text_substitutes_spell_subtype_name() {
    let game_dir = game_dir();
    setup_env(&game_dir);
    // `12-stlkundertaker-town` 的 studlv5 写 `SPELL_IMMUNITY: spell.blind`
    let Some(mod_dir) = game_mod("12-stlkundertaker-town") else {
        eprintln!("跳过：未安装 12-stlkundertaker-town");
        return;
    };
    let mod_id = "12-stlkundertaker-town";
    let cache_root = std::env::temp_dir().join("vcmi_fv_ability_subtype_test");
    let mod_cache = cache_root.join(crate::modcache::sanitize_id(mod_id));
    let _ = fs::remove_dir_all(&mod_cache);
    let _ = fs::create_dir_all(&cache_root);
    let r = build_mod_wiki_phase(&mod_dir.to_string_lossy(), mod_id, &cache_root, &game_dir, 2);
    assert_eq!(r.get("ok").and_then(|x| x.as_bool()), Some(true), "构建失败：{r}");
    let text = fs::read_to_string(mod_cache.join("cache.json")).expect("cache.json 未落盘");
    let data: Value = serde_json::from_str(&text).expect("cache.json 解析失败");

    let mut rows = Vec::new();
    for c in data.get("creatures").and_then(|x| x.as_array()).unwrap_or(&vec![]) {
        for a in c.get("abilities").and_then(|x| x.as_array()).unwrap_or(&vec![]) {
            let sub = a.get("subtype").and_then(|x| x.as_str()).unwrap_or("");
            if !sub.starts_with("spell.") {
                continue;
            }
            let desc = a
                .pointer("/description/chinese")
                .or_else(|| a.pointer("/description/english"))
                .and_then(|x| x.as_str())
                .unwrap_or("");
            rows.push((sub.to_string(), desc.to_string()));
        }
    }
    println!("形如 `spell.<id>` 的子类型能力 {} 条", rows.len());
    for (s, d) in rows.iter().take(6) {
        println!("  {s} → {d}");
    }
    assert!(!rows.is_empty(), "该 mod 应有 `spell.<id>` 子类型的能力");
    assert!(
        rows.iter()
            .any(|(_, d)| d.contains("连珠火球") || d.contains("Fireball")),
        "没有任何一条把法术名替换进文案：`${{subtype.spell}}` 仍是空串（子类型查询键取了 dotted 原文？）\n{}",
        rows.iter()
            .map(|(s, d)| format!("  {s} → {d}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    // 名称不在翻译池里的 mod 法术（如 `spell.paraFungus`）替换后仍是空串 —— 那是**数据缺口**，
    // 不是本用例要锁的代码缺陷，故只提示不判失败。
    for (s, d) in rows.iter().filter(|(_, d)| {
        d.contains("免疫魔法") || d.contains("cannot be affected by .")
    }) {
        println!("  （提示）{s} 的法术名不在翻译池里 → 文案留空：{d}");
    }
}
