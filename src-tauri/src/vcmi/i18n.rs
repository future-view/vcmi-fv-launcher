//! 模组 i18n 文案表读取：`Content/**/translation*/**/*.<lang>.json` → `key -> { lang -> text }`。
//!
//! 语言 key 直接采用 VCMI 原生标识（chinese / english / tchinese / russian / ...），
//! 由文件名推断（见 [`read_i18n_map`]），不映射为 zh-CN / en。
//!
//! **翻译是跨模组的**：VCMI 把全部激活 mod 的 translation 文件合并进一个全局语言池，
//! 键自带作用域（`faction.<modScope>.<factionId>.name`）。因此子模组的翻译常放在**父模组**
//! 里（实测：`crossroads-of-worlds/mods/Fen` 的中文名"泥沼"写在
//! `crossroads-of-worlds/content.zip::config/translation/crossroads-of-worlds/chinese.json`，
//! `hota/mods/bulwark` 的"坚垒"写在 `hota/content.zip`）。构建子模组时必须沿
//! `<parent>/mods/<child>` 链把祖先模组的翻译一并读入，否则城镇/生物/英雄名回退英文。
//!
//! 依赖方向：只依赖 `infra`（见 `docs/architecture/backend.zh-CN.md` §4.1）。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::Value;

/// 翻译字典：`key -> { lang -> text }`，key 形如 `faction.<scope>.<id>.name`（无语言后缀）。
pub(crate) type I18nMap = HashMap<String, HashMap<String, String>>;

/// 单 mod 翻译表缓存（键 = 目录指纹）。
///
/// 父模组的翻译会被其**全部子模组**复用（hota 30+、crossroads-of-worlds 12+、
/// new-chinese-mods 20+），缓存后每个目录只解析一次（zip 中央目录遍历 + 翻译条目读取）。
static MOD_I18N_CACHE: OnceLock<Mutex<Vec<(String, Arc<I18nMap>)>>> = OnceLock::new();
const MOD_I18N_CACHE_CAP: usize = 128;

/// 读取 mod 的翻译字典（含**祖先链**，见模块文档注释）。
///
/// 合并顺序：最远祖先 → … → 父 → 自身；后读优先，
/// 即 mod 自身的同名键覆盖父模组，父模组覆盖祖父模组。
pub(crate) fn read_i18n_map(mod_path: &Path) -> I18nMap {
    let mut out = I18nMap::new();
    // 祖先链：沿 `<parent>/mods/<child>` 逐级上溯（多层如 new-chinese-mods.h2-towns.xxx）。
    let mut chain: Vec<PathBuf> = Vec::new();
    let mut cur = mod_path.to_path_buf();
    loop {
        let Some(parent) = cur.parent() else { break };
        let is_mods_dir = parent
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.eq_ignore_ascii_case("mods"))
            .unwrap_or(false);
        if !is_mods_dir {
            break;
        }
        let Some(gp) = parent.parent() else { break };
        // 只把"像 mod 的目录"纳入（防把游戏根/盘符根当父 mod 反复读取）。
        let looks_like_mod = gp.join("mod.json").is_file()
            || gp.join("Content").is_dir()
            || gp.join("content.zip").is_file()
            || gp.join("mods").is_dir();
        if !looks_like_mod {
            break;
        }
        chain.push(gp.to_path_buf());
        cur = gp.to_path_buf();
    }
    for dir in chain.iter().rev() {
        let m = cached_mod_i18n(dir);
        merge_into(&mut out, &m);
    }
    let own = cached_mod_i18n(mod_path);
    merge_into(&mut out, &own);
    out
}

fn merge_into(dst: &mut I18nMap, src: &I18nMap) {
    for (k, langs) in src {
        let e = dst.entry(k.clone()).or_default();
        for (lang, text) in langs {
            e.insert(lang.clone(), text.clone());
        }
    }
}

fn cached_mod_i18n(dir: &Path) -> Arc<I18nMap> {
    // 指纹含 content.zip 自身：目录 mtime 不随"同名文件内容替换"更新，
    // 加入 zip 的 (mtime, len) 才能让翻译更新在同一进程内也失效。
    let mut fp_paths = vec![dir.to_path_buf()];
    let zip = dir.join("content.zip");
    if zip.is_file() {
        fp_paths.push(zip);
    }
    let key = crate::infra::cache::fingerprint_of(&fp_paths);
    let list = MOD_I18N_CACHE.get_or_init(|| Mutex::new(Vec::new()));
    {
        let guard = list.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((_, v)) = guard.iter().find(|(k, _)| *k == key) {
            return v.clone();
        }
    }
    let mut m = I18nMap::new();
    merge_mod_i18n(dir, &mut m);
    // **键统一小写**：VCMI 的翻译查找大小写不敏感（内部按小写存储/查询），
    // 而翻译键里的作用域/实体 id 大小写与 modId / 配置 id 常有出入
    // （如键 `artifact.crossroads-of-worlds.fen.fenCrossbow.description` 对
    // modId `crossroads-of-worlds.Fen` + id `fenCrossbow`）。
    // 在缓存层归一，保证「远祖先 → 父 → 自身」的覆盖顺序确定；查询端
    // `wiki::i18n::lookup` 亦按小写回退，两端一致才能命中。
    let arc = Arc::new(
        m.into_iter()
            .map(|(k, v)| (k.to_lowercase(), v))
            .collect::<I18nMap>(),
    );
    let mut guard = list.lock().unwrap_or_else(|e| e.into_inner());
    guard.push((key, arc.clone()));
    if guard.len() > MOD_I18N_CACHE_CAP {
        guard.remove(0);
    }
    arc
}

/// 根据文件名/路径推断 VCMI 语言 key（chinese/english/tchinese/russian/...）。
/// 未知语言直接以其文件名 base 作为 key（如 polish.json -> "polish"），保证可扩展。
fn classify_lang(base: &str) -> String {
    let b = base.to_lowercase();
    if b.contains("tchinese") {
        "tchinese".to_string()
    } else if b.contains("chinese") {
        "chinese".to_string()
    } else if b.contains("english") || b == "en" {
        "english".to_string()
    } else if b.contains("russian") || b == "ru" {
        "russian".to_string()
    } else if b.contains("polish") || b == "pl" {
        "polish".to_string()
    } else if b.contains("german") || b == "de" {
        "german".to_string()
    } else if b.contains("french") || b == "fr" {
        "french".to_string()
    } else {
        // 未知语言：保留文件名 base 作为语言 key，避免丢失
        base.to_string()
    }
}

/// 读取单个 mod 目录的全部翻译文件（磁盘 + `content.zip`）并入 `map`。
fn merge_mod_i18n(mod_dir: &Path, map: &mut I18nMap) {
    let merge_file = |path: &Path, map: &mut I18nMap| {
        if let Ok(text) = fs::read_to_string(path) {
            let v = crate::infra::json::parse_vcmi_json(&text);
            if let Value::Object(m) = v {
                let lang = classify_lang(&path.file_stem().unwrap_or_default().to_string_lossy());
                for (k, val) in m {
                    if let Some(s) = val.as_str() {
                        map.entry(k).or_default().insert(lang.clone(), s.to_string());
                    }
                }
            }
        }
    };

    // 1) `Content/**/translation*/**/*.json`：覆盖 `translation/`、`translations/`、
    //    `translationPavilion/`（new-pavilion）等全部变体——子模组/第三方 mod 的命名不统一，
    //    只认固定目录名会整体漏读（实测 new-pavilion 的中文名在 `config/translationPavilion/`）。
    let mut files: Vec<PathBuf> = Vec::new();
    collect_translation_dirs(&mod_dir.join("Content"), &mut files, 0);
    for f in files {
        merge_file(&f, map);
    }

    // 2) `Content/config/<lang>.json`：VCMI 核心模组（Mods/vcmi）的翻译平铺在这里，
    //    含 `core.bonus.<TYPE>.description` 等能力文本与原版实体名称。
    const LANGS: &[&str] = &[
        "chinese", "tchinese", "english", "russian", "polish", "german", "french", "spanish",
        "czech", "hungarian", "dutch", "italian", "portuguese", "swedish", "turkish",
        "japanese", "korean", "ukrainian", "belarusian", "bulgarian", "greek", "finnish",
        "filipino", "norwegian", "danish", "romanian", "serbian", "croatian", "slovak",
        "slovenian", "estonian", "latvian", "lithuanian", "vietnamese", "indonesian", "thai",
    ];
    let config_dir = mod_dir.join("Content").join("config");
    if config_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&config_dir) {
            for e in entries.flatten() {
                let p = e.path();
                if !p.is_file() || p.extension().map(|x| x != "json").unwrap_or(true) {
                    continue;
                }
                let stem = p.file_stem().unwrap_or_default().to_string_lossy().to_lowercase();
                if LANGS.contains(&stem.as_str()) {
                    merge_file(&p, map);
                }
            }
        }
    }

    // 3) `content.zip` 内任一路径段以 `translation` 开头的 `.json`
    //    （translation/chinese.json、config/translation/<mod>/chinese.json、
    //     config/translationPavilion/chinese.json 等形态都覆盖）。
    let zip_path = mod_dir.join("content.zip");
    if let Ok(file) = fs::File::open(&zip_path) {
        if let Ok(mut archive) = zip::ZipArchive::new(file) {
            for i in 0..archive.len() {
                if let Ok(mut entry) = archive.by_index(i) {
                    let name = entry.name().to_string();
                    let nl = name.replace('\\', "/");
                    let is_tr = nl.ends_with(".json")
                        && nl
                            .split('/')
                            .any(|seg| seg.to_lowercase().starts_with("translation"));
                    if !is_tr {
                        continue;
                    }
                    let base = name.rsplit('/').next().unwrap_or("").trim_end_matches(".json");
                    let lang = classify_lang(base);
                    let mut buf = Vec::new();
                    if std::io::Read::read_to_end(&mut entry, &mut buf).is_ok() {
                        let text = String::from_utf8_lossy(&buf);
                        let v = crate::infra::json::parse_vcmi_json(&text);
                        if let Value::Object(m) = v {
                            for (k, val) in m {
                                if let Some(s) = val.as_str() {
                                    map.entry(k).or_default().insert(lang.clone(), s.to_string());
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// 递归查找**名字以 `translation` 开头**的目录，收集其中的全部 `.json`。
///
/// 只 stat 目录、不读非翻译文件；`depth` 限制防止在异常结构上失控。
fn collect_translation_dirs(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 4 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if !p.is_dir() {
            continue;
        }
        let name = p
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_lowercase();
        if name.starts_with("translation") {
            let mut files = Vec::new();
            crate::infra::json::collect_all_json_files(&p, &mut files);
            out.extend(files);
        } else {
            collect_translation_dirs(&p, out, depth + 1);
        }
    }
}
