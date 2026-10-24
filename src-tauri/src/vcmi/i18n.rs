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

/// 释放单 mod 翻译表缓存（构建全部结束后调用；见 [`crate::infra::cache_gc`]）。
///
/// 容量 128 条 × 每个 mod 的整张翻译表：大发行版里这是构建期最大的常驻块之一。
pub fn release_caches() {
    if let Some(c) = MOD_I18N_CACHE.get() {
        if let Ok(mut g) = c.lock() {
            g.clear();
            g.shrink_to_fit();
        }
    }
}

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
/// VCMI 已知语言名（`CGeneralTextHandler::getPreferredLanguage` 的取值域）。
///
/// 提升到模块级是因为两处都要用：① `Content/config/<lang>.json` 平铺扫描；
/// ② `lang_of_path` 的"文件名 / 父目录名是不是语言"判定。
const KNOWN_LANGS: &[&str] = &[
    "chinese",
    "tchinese",
    "english",
    "russian",
    "polish",
    "german",
    "french",
    "spanish",
    "czech",
    "hungarian",
    "dutch",
    "italian",
    "portuguese",
    "swedish",
    "turkish",
    "japanese",
    "korean",
    "ukrainian",
    "belarusian",
    "bulgarian",
    "greek",
    "finnish",
    "filipino",
    "norwegian",
    "danish",
    "romanian",
    "serbian",
    "croatian",
    "slovak",
    "slovenian",
    "estonian",
    "latvian",
    "lithuanian",
    "vietnamese",
    "indonesian",
    "thai",
];

/// 是否是已知语言名（大小写不敏感）。
fn known_lang(s: &str) -> Option<&'static str> {
    let l = s.to_lowercase();
    KNOWN_LANGS.iter().copied().find(|k| *k == l)
}

/// 由**文件路径**推断语言（VCMI 的翻译布局有两种，见 `declared_translations`）：
///
/// 1. 文件名即语言：`translation/chinese.json` / `translations/english.json`；
/// 2. **按语言分组的子目录**：`translation/hota/chinese/game.json` —— HotA 就是这么放的
///    （`Mods/hota/content/config/translation/hota/<lang>/*.json`，文件名是
///    `game.json` / `campaigns.json` / `maps.json`）。此前只认第 1 种，这些文件全部落进
///    伪语言键 `game` / `campaigns` / `maps`，中文/德文/波兰文… 一个都取不到
///    （实测 east-realm 发行版 cove 城镇"全是英文"的根因）。
///
/// 两者都不命中返回 `None` —— 调用方**丢弃**该文件：VCMI 只装载 mod.json 声明的清单，
/// 未声明又认不出语言的文件它同样不会读；而修复类模组塞在翻译目录里的非语言文件
/// （如 `translation/hota/hungarian.json` 之外的杂项）进了缓存也只是无意义数据。
fn lang_of_path(path: &Path) -> Option<String> {
    let rel = path
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join("/");
    lang_of_rel_path(&rel)
}

/// `lang_of_path` 的字符串版：入参是 `/` 分隔的相对路径（磁盘路径或 zip 条目名都用它）。
/// 只看**最后两段**：文件名 stem，以及父目录名。
fn lang_of_rel_path(rel: &str) -> Option<String> {
    let segs: Vec<&str> = rel.split('/').filter(|s| !s.is_empty()).collect();
    let file = segs.last()?;
    let stem = file.trim_end_matches(".json").to_lowercase();
    if let Some(l) = known_lang(&stem) {
        return Some(l.to_string());
    }
    let parent = segs.len().checked_sub(2).map(|i| segs[i])?.to_lowercase();
    known_lang(&parent).map(|l| l.to_string())
}

/// `mod.json` 声明的翻译清单 → `(语言, 文件绝对路径)`。
///
/// **这是 VCMI 的权威来源**（`CModHandler::loadTranslation`，
/// lib/modding/CModHandler.cpp:237-271）：
///   - 顶层 `translations: [...]` → 该 mod 的**基础语言**
///     （`ModDescription::getBaseLanguage()`：mod.json 的 `language` 字段，缺省 english）；
///   - 顶层 `<lang>: { "translations": [...] }` → 该语言；
///   - 路径相对 mod 的 `Content/` 解析（VCMI 的 mod 文件系统以 content 为根）。
///
/// 目录扫描只在我们这边的兜底逻辑里用；两者重复的文件以声明为准（见 `merge_mod_i18n`）。
fn declared_translations(mod_dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    let mut mod_json = None;
    for name in ["mod.json", "Mod.json", "MOD.json"] {
        let p = mod_dir.join(name);
        if let Ok(text) = fs::read_to_string(&p) {
            if let Value::Object(m) = crate::infra::json::parse_vcmi_json(&text) {
                mod_json = Some(m);
                break;
            }
        }
    }
    let Some(mj) = mod_json else {
        return out;
    };
    // 基础语言：`language` 字段，缺省 english（对齐 ModDescription::getBaseLanguage）
    let base_lang = mj
        .get("language")
        .and_then(|v| v.as_str())
        .and_then(known_lang)
        .unwrap_or("english")
        .to_string();
    let resolve = |rel: &str| -> Option<PathBuf> {
        let rel = rel.trim().trim_start_matches('/').replace('\\', "/");
        for root in ["Content", "content"] {
            let f = mod_dir.join(root).join(&rel);
            if f.is_file() {
                return Some(f);
            }
        }
        None
    };
    // 顶层平铺清单 → 基础语言
    if let Some(arr) = mj.get("translations").and_then(|v| v.as_array()) {
        for x in arr {
            if let Some(rel) = x.as_str() {
                if let Some(p) = resolve(rel) {
                    out.push((base_lang.clone(), p));
                }
            }
        }
    }
    // 按语言分组的清单
    for (k, v) in &mj {
        let Some(lang) = known_lang(k) else { continue };
        let Some(arr) = v.get("translations").and_then(|x| x.as_array()) else {
            continue;
        };
        for x in arr {
            if let Some(rel) = x.as_str() {
                if let Some(p) = resolve(rel) {
                    out.push((lang.to_string(), p));
                }
            }
        }
    }
    out
}

/// 读取单个 mod 目录的全部翻译文件（mod.json 声明 + 目录扫描 + `content.zip`）并入 `map`。
fn merge_mod_i18n(mod_dir: &Path, map: &mut I18nMap) {
    let merge_text = |path: &Path, lang: &str, map: &mut I18nMap| {
        if let Ok(text) = fs::read_to_string(path) {
            if let Value::Object(m) = crate::infra::json::parse_vcmi_json(&text) {
                for (k, val) in m {
                    if let Some(s) = val.as_str() {
                        map.entry(k)
                            .or_default()
                            .insert(lang.to_string(), s.to_string());
                    }
                }
            }
        }
    };

    // 0) `mod.json` 声明的清单（**权威语言来源**，VCMI 同款）。
    //    先记下已装载的文件，目录扫描时跳过，避免同一文件被两种语言各存一份。
    let mut loaded: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for (lang, path) in declared_translations(mod_dir) {
        merge_text(&path, &lang, map);
        loaded.insert(path);
    }

    // 1) `Content/**/translation*/**/*.json`：覆盖 `translation/`、`translations/`、
    //    `translationPavilion/`（new-pavilion）等全部变体——子模组/第三方 mod 的命名不统一，
    //    只认固定目录名会整体漏读（实测 new-pavilion 的中文名在 `config/translationPavilion/`）。
    //    语言按 `lang_of_path` 判定（文件名 → 父目录名），判不出的**丢弃**（见该函数注释）。
    let mut files: Vec<PathBuf> = Vec::new();
    collect_translation_dirs(&mod_dir.join("Content"), &mut files, 0);
    for f in files {
        if loaded.contains(&f) {
            continue;
        }
        if let Some(lang) = lang_of_path(&f) {
            merge_text(&f, &lang, map);
            loaded.insert(f);
        }
    }

    // 2) `Content/config/<lang>.json`：VCMI 核心模组（Mods/vcmi）的翻译平铺在这里，
    //    含 `core.bonus.<TYPE>.description` 等能力文本与原版实体名称。
    let config_dir = mod_dir.join("Content").join("config");
    if config_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(&config_dir) {
            for e in entries.flatten() {
                let p = e.path();
                if !p.is_file() || p.extension().map(|x| x != "json").unwrap_or(true) {
                    continue;
                }
                let stem = p
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_lowercase();
                if known_lang(&stem).is_some() {
                    merge_text(&p, &stem, map);
                }
            }
        }
    }

    // 3) `content.zip` 内任一路径段以 `translation` 开头的 `.json`
    //    （translation/chinese.json、config/translation/<mod>/chinese.json、
    //     config/translationPavilion/chinese.json 等形态都覆盖）。
    //    语言判定与磁盘版一致：先看文件名，再看父目录名；判不出的一律跳过。
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
                    let Some(lang) = lang_of_rel_path(&nl) else {
                        continue;
                    };
                    let mut buf = Vec::new();
                    if std::io::Read::read_to_end(&mut entry, &mut buf).is_ok() {
                        let text = String::from_utf8_lossy(&buf);
                        let v = crate::infra::json::parse_vcmi_json(&text);
                        if let Value::Object(m) = v {
                            for (k, val) in m {
                                if let Some(s) = val.as_str() {
                                    map.entry(k)
                                        .or_default()
                                        .insert(lang.clone(), s.to_string());
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
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
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

#[cfg(test)]
mod lang_path_tests {
    use super::{known_lang, lang_of_rel_path};

    /// 平铺写法：文件名即语言（绝大多数第三方 mod）
    #[test]
    fn 平铺文件按文件名取语言() {
        assert_eq!(
            lang_of_rel_path("config/translations/chinese.json").as_deref(),
            Some("chinese")
        );
        assert_eq!(
            lang_of_rel_path("Content/config/translation/english.json").as_deref(),
            Some("english")
        );
        // 旧的大小写/别名写法（`zh`/`ru` 不在 known_lang 里，但 `tchinese` 在）
        assert_eq!(
            lang_of_rel_path("translation/tchinese.json").as_deref(),
            Some("tchinese")
        );
    }

    /// 按语言分组的子目录：HotA 的 `translation/<mod>/<lang>/*.json`
    /// （实测 east-realm 发行版；修复前这些文件全部落进伪语言键 `game`）
    #[test]
    fn 嵌套目录按父目录取语言() {
        assert_eq!(
            lang_of_rel_path("config/translation/hota/chinese/game.json").as_deref(),
            Some("chinese")
        );
        assert_eq!(
            lang_of_rel_path("config/translation/hota/german/maps.json").as_deref(),
            Some("german")
        );
        assert_eq!(
            lang_of_rel_path("Content/translation/polish/campaigns.json").as_deref(),
            Some("polish")
        );
    }

    /// 判不出语言 → None（调用方丢弃）：文件名与父目录都不是已知语言。
    /// 这类文件在修复类模组里只是噪音，VCMI 也只装载 mod.json 声明过的清单。
    #[test]
    fn 认不出语言时返回无() {
        assert_eq!(lang_of_rel_path("config/translation/hota/game.json"), None);
        assert_eq!(lang_of_rel_path("config/translations/strings.json"), None);
        assert_eq!(lang_of_rel_path("game.json"), None);
    }

    #[test]
    fn 已知语言表大小写不敏感() {
        assert_eq!(known_lang("Chinese"), Some("chinese"));
        assert_eq!(known_lang("POLISH"), Some("polish"));
        assert_eq!(known_lang("klingon"), None);
    }
}
