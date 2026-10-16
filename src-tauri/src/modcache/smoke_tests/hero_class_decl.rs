//! 英雄职业声明回归（2026-10-16，`WIKI_CACHE_VERSION` 2）。
//!
//! 背景：不少 H3 时代 mod 把**职业定义**写在 `heroes/` 目录里，只在 `mod.json` 的
//! `heroClasses` 键下声明（实测 `moonspire` / `10-blazers--town`）。旧实现只按目录
//! 回退分类，于是这些文件被当英雄读，造成城镇页：
//!
//! 1. 4 行"幽灵英雄"（`moonPriest` / `groveKeeper` / `daoSage` / `kungFuMaster`：
//!    无头像、无职业、特长只有一个 ✦）；
//! 2. 真英雄的职业名回落成 id —— 分组标题显示 `MOONPRIEST // GROVEKEEPER`，
//!    且 `affinity` 为空使"力量 / 魔法"两个分组表塌成一张。
//!
//! 本组用真实 mod 目录验证修复后**产物**（cache.json）的两个不变量：
//! 职业条目不再以英雄身份出现；英雄的 `className` / `affinity` / `factionId` 能解析。
//!
//! 运行：`VCMI_GAME_DIR=<游戏目录> cargo test --lib hero_class_decl -- --ignored --nocapture`
//! （游戏目录缺省回落仓库 `references/Heroes3-VCMI` fixture，未装该 mod 时整组跳过）

use super::*;

/// 取 `<游戏目录>/Mods/<相对路径>`，不存在返回 None（用例跳过）。
fn game_mod(mod_rel: &str) -> Option<PathBuf> {
    let p = game_dir().join("Mods").join(mod_rel);
    p.is_dir().then_some(p)
}

/// 全新构建单个 mod 的**阶段一（仅数据）** 并读回 `cache.json`。
///
/// 只跑阶段一：本组断言全在数据层（英雄/职业路由），跑素材阶段会白等 HDD 上的
/// DEF/肖像提取（数百毫秒到数分钟），对结论没有贡献。
/// 每次先删掉本 mod 的缓存目录 —— 构建逻辑的改动不在缓存键里，残留缓存会让断言读到旧产物。
fn build_and_read(mod_dir: &Path, mod_id: &str, game_dir: &Path) -> Value {
    let cache_root = std::env::temp_dir().join("vcmi_fv_hero_class_test");
    let mod_cache = cache_root.join(crate::modcache::sanitize_id(mod_id));
    let _ = fs::remove_dir_all(&mod_cache);
    let _ = fs::create_dir_all(&cache_root);
    let r = build_mod_wiki_phase(
        &mod_dir.to_string_lossy(),
        mod_id,
        &cache_root,
        game_dir,
        0,
    );
    assert_eq!(
        r.get("ok").and_then(|x| x.as_bool()),
        Some(true),
        "构建失败 {mod_id}: {r}"
    );
    let text = fs::read_to_string(mod_cache.join("cache.json"))
        .unwrap_or_else(|e| panic!("{mod_id} cache.json 未落盘: {e}"));
    serde_json::from_str(&text).expect("cache.json 解析失败")
}

fn hero_ids(data: &Value) -> Vec<String> {
    data.get("heroes")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|h| h.get("id").and_then(|x| x.as_str()).map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn hero_of<'a>(data: &'a Value, id: &str) -> Option<&'a Value> {
    data.get("heroes")
        .and_then(|x| x.as_array())
        .and_then(|a| a.iter().find(|h| h.get("id").and_then(|x| x.as_str()) == Some(id)))
}

/// `moonspire`：2 个职业声明在 `heroClasses` 下但文件位于 `heroes/`；
/// 另有 2 个**未声明**的残留职业文件（`daoSage` / `kungFuMaster`）。
#[test]
#[ignore]
fn moonspire_class_files_are_not_heroes() {
    let Some(mod_dir) = game_mod("moonspire") else {
        eprintln!("跳过：未安装 moonspire");
        return;
    };
    let game_dir = game_dir();
    crate::vcmi::resource::set_wiki_user_data_dir(&user_data_dir(&game_dir));
    let data = build_and_read(&mod_dir, "moonspire", &game_dir);
    let ids = hero_ids(&data);

    // ① 4 个职业 id 都不得再作为英雄出现（截图里那 4 行空图标）
    for cid in ["moonPriest", "groveKeeper", "daoSage", "kungFuMaster"] {
        assert!(
            !ids.contains(&cid.to_string()),
            "职业 {cid} 不得以英雄身份出现：{ids:?}"
        );
    }
    // ② 同时不能把真英雄删掉（moonspire 的 17 个英雄）
    assert!(
        ids.len() >= 17,
        "英雄数量异常（应为 17 个）：{ids:?}"
    );

    // ③ 职业名 / 阵营 / 力量-魔法归属必须解析出来（否则分组标题回落 id、分组塌成一张表）
    let h = hero_of(&data, "moonspireHero01").expect("moonspireHero01 缺失");
    let class_name = h
        .get("className")
        .and_then(|c| c.get("chinese"))
        .and_then(|x| x.as_str())
        .unwrap_or("");
    assert!(
        !class_name.is_empty() && class_name != "moonPriest",
        "职业名必须解析成中文，而不是回落 id：{class_name:?}"
    );
    assert_eq!(
        h.get("affinity").and_then(|x| x.as_str()),
        Some("magic"),
        "affinity 必须来自 heroClass：{h:?}"
    );
    assert_eq!(
        h.get("factionId").and_then(|x| x.as_str()),
        Some("moonspire"),
        "factionId 必须来自 heroClass.faction"
    );
    // ④ 全部英雄都得有职业（幽灵行已清空）
    for hid in &ids {
        let hh = hero_of(&data, hid).unwrap();
        assert!(
            !hh.get("class").and_then(|x| x.as_str()).unwrap_or("").is_empty(),
            "英雄 {hid} 没有职业"
        );
    }
}

/// `10-blazers--town`：职业文件同样写在 `heroes/` 目录（`honorWarrior` / `brightWizard`）。
#[test]
#[ignore]
fn blazers_class_files_are_not_heroes() {
    let Some(mod_dir) = game_mod("10-blazers--town") else {
        eprintln!("跳过：未安装 10-blazers--town");
        return;
    };
    let game_dir = game_dir();
    crate::vcmi::resource::set_wiki_user_data_dir(&user_data_dir(&game_dir));
    let data = build_and_read(&mod_dir, "10-blazers--town", &game_dir);
    let ids = hero_ids(&data);
    for cid in ["honorWarrior", "brightWizard"] {
        assert!(
            !ids.contains(&cid.to_string()),
            "职业 {cid} 不得以英雄身份出现：{ids:?}"
        );
    }
    // 英雄 id 与声明文件名不同（`blazersHero01.json` 里的条目叫 `blaTimfur` 之类），
    // 故不写死 id：只要求存在"职业名已解析成中文"且 `affinity` 非空的英雄。
    let resolved = data
        .get("heroes")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter().any(|h| {
                let cid = h.get("class").and_then(|x| x.as_str()).unwrap_or("");
                let cname = h
                    .get("className")
                    .and_then(|c| c.get("chinese"))
                    .and_then(|x| x.as_str())
                    .unwrap_or("");
                let aff = h.get("affinity").and_then(|x| x.as_str()).unwrap_or("");
                !cid.is_empty() && cname != cid && !aff.is_empty()
            })
        })
        .unwrap_or(false);
    assert!(
        resolved,
        "必须有英雄同时解析出中文职业名与 affinity（否则分组标题回落 id、力量/魔法分组塌成一张）：{ids:?}"
    );
}

/// 四维主属性的右键说明必须能从 h3-core 的 `arraytxt` 解出（**纯数据用例**，不需要游戏目录）。
///
/// 对齐 VCMI `CHeroWindow.cpp:131-132`：主属性区域的右键文本 = `generaltexth->arraytxt[2 + v]`
/// （`0..3` 依次 = 攻击力 / 防御力 / 力量 / 知识）。`ARRAYTXT.TXT` 由
/// `scripts/gen-h3-core-texts.mjs` 烘成 `core.arraytxt.<行号>`（文本形如 `{名称}\n\n描述`）。
#[test]
fn primary_skill_texts_come_from_h3_core_arraytxt() {
    let Some(dir) = h3_core_data_dir() else {
        eprintln!("跳过：未找到 h3-core 数据目录（先跑 scripts/gen-h3-core-texts.mjs）");
        return;
    };
    let Some(i18n) = load_h3_core_i18n(&dir) else {
        eprintln!("跳过：h3-core 文案表为空");
        return;
    };
    let v = crate::modcache::build::primary_skill_texts(&i18n);
    println!("primarySkillTexts = {v}");
    for k in ["attack", "defense", "spellPower", "knowledge"] {
        let zh = v
            .pointer(&format!("/{k}/chinese"))
            .and_then(|x| x.as_str())
            .unwrap_or("");
        assert!(
            !zh.is_empty(),
            "`{k}` 缺中文说明（h3-core 未烘入 arraytxt？）：{v}"
        );
        assert!(
            !zh.contains('{') && !zh.contains('}'),
            "`{k}` 描述里残留了名称段（应已剥掉 `{{名称}}\n\n`）：{zh}"
        );
    }
}
