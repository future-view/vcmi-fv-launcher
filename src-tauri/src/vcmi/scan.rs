//! 模组扫描与分类：遍历 `Mods/`、解析 `mod.json`、按 `modType` 归类、归并 submod。
//!
//! 用户数据目录优先、游戏目录补齐（`scan_mods` 命令的合并规则见 `commands.rs`）；
//! 子模组用 `parent.child` 形式 ID 归并（`norm_mod_id`）。
//!
//! 依赖方向：只依赖 `infra`（见 `docs/architecture/backend.zh-CN.md` §4.1）。

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::infra::cache::cfg_cache_get;
use crate::infra::json;
use crate::vcmi::paths::VcmiPaths;
use crate::vcmi::resource::content_zip_is_encrypted;

pub(crate) const RESERVED_SCOPES: &[&str] = &["core", "vcmi"];

/// 归一化 mod id：**磁盘目录名小写**，子 mod 用 `parent.child`（父级已小写，故只需处理叶子）。
///
/// 大小写不是随意的：VCMI 的 modId **恒为目录名的小写**
/// （`lib/modding/ModManager.cpp:54` 的 `boost::algorithm::to_lower_copy`），
/// 而 modId 是全局主键 —— `modSettings.json` 的 presets、`depends` / `conflicts`、
/// 跨模组引用 `<modId>:<entityId>` 用的都是这个形态。
/// 实测该发行版 572 个模组的 presets 里**含大写字母的 modId 为 0 个**，反证 VCMI 侧确实小写。
///
/// 此前保留磁盘原名（`1-Knight-town`），后果：启动器内部主键与 VCMI 主键**字符串不等**，
/// 各处只能加大写容错（前端 `modIdKey`、`factionId.endsWith`、`resolvePresetIds` 的
/// diskName 回退）；且在 Linux/macOS（大小写敏感）上缓存目录名与 Windows 不一致
/// （跨平台发行见 ADR 0008）。
///
/// ⚠️ 原始目录名完整保留在 [`scan_mod_dir`] 输出的 `diskName` 字段里，磁盘定位请用它或
/// 索引里的 `modPath`——**不要用 modId 拼磁盘路径**（小写后在 Linux 上必然找不到目录）。
pub(crate) fn norm_mod_id(parent: Option<&str>, disk_name: &str) -> String {
    let leaf = disk_name.trim().to_lowercase();
    match parent {
        Some(p) => format!("{}.{}", p, leaf),
        None => leaf,
    }
}

/// 从 mod.json 提取分类：严格优先 modType；缺失时才安全推测。
/// 翻译类关键词（ID / 名称 / 描述）优先级高于 factions，
/// 避免含 factions 配置的翻译 mod 被误判为 Town。
pub(crate) fn classify_mod(
    id: &str,
    mod_type: Option<&str>,
    name: &str,
    description: &str,
    factions: &[String],
) -> String {
    // 1. 显式 modType 最优先
    if let Some(t) = mod_type {
        let tl = t.trim();
        if !tl.is_empty() {
            return tl.to_string();
        }
    }

    let id_l = id.to_lowercase();
    let name_l = name.to_lowercase();
    let desc_l = description.to_lowercase();

    if id == "core" || id == "vcmi" {
        return "Core".into();
    }
    if id == "h3e-main-menu" {
        return "Interface".into();
    }

    // 翻译类关键词（覆盖中文常见写法）
    let is_translation = id_l.contains("translation")
        || id_l.contains("chinese")
        || id_l.contains("language")
        || name_l.contains("translation")
        || name_l.contains("chinese")
        || name_l.contains("language")
        || name_l.contains("简体")
        || name_l.contains("繁体")
        || name_l.contains("中文")
        || name_l.contains("汉化")
        || desc_l.contains("translation")
        || desc_l.contains("translate")
        || desc_l.contains("简体")
        || desc_l.contains("繁体")
        || desc_l.contains("中文")
        || desc_l.contains("汉化");
    if is_translation {
        return "Translation".into();
    }

    // 真正含 factions 配置的才识别为城镇
    if !factions.is_empty() {
        return "Town".into();
    }
    if id_l.contains("town") || id_l.contains("faction") {
        return "Town".into();
    }
    if id_l.contains("hero") {
        return "Heroes".into();
    }
    if id_l.contains("spell") {
        return "Spells".into();
    }
    if id_l.contains("artifact") {
        return "Artifacts".into();
    }
    if id_l.contains("creature") {
        return "Creatures".into();
    }
    if id_l.contains("skill") {
        return "Skills".into();
    }
    if id_l.contains("object") {
        return "Objects".into();
    }
    if id_l.contains("mechanic") {
        return "Mechanics".into();
    }
    if id_l.contains("ai") || id_l.contains("mmai") {
        return "AI".into();
    }
    if id_l.contains("interface") || id_l.contains("menu") {
        return "Interface".into();
    }
    if id_l.contains("graphic") || id_l.contains("gfx") || id_l.contains("hd") {
        return "Graphical".into();
    }
    if id_l.contains("music") {
        return "Music".into();
    }
    if id_l.contains("sound") {
        return "Sounds".into();
    }
    if id_l.contains("resource") {
        return "Resources".into();
    }
    if id_l.contains("map") {
        return "Maps".into();
    }
    if id_l.contains("template") {
        return "Templates".into();
    }
    if id_l.contains("campaign") {
        return "Campaigns".into();
    }
    if id_l.contains("compat") {
        return "Compatibility".into();
    }
    if id_l.contains("test") {
        return "Test".into();
    }
    if id_l.contains("demo") {
        return "Demo".into();
    }
    "Other".into()
}

/// 探测城镇 faction id：优先读取 Content/config/factions/*.json 文件名，
/// 兼容大小写 variations（VCMI mod 目录 Content 可能大写 C）。
pub(crate) fn detect_factions(dir: &Path) -> Vec<String> {
    let mut list = Vec::new();
    for base in [
        "Content/config/factions",
        "content/config/factions",
        "config/factions",
    ] {
        let p = dir.join(base);
        if !p.is_dir() {
            continue;
        }
        if let Ok(entries) = fs::read_dir(&p) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file()
                    && path
                        .extension()
                        .map(|s| s.eq_ignore_ascii_case("json"))
                        .unwrap_or(false)
                {
                    if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                        list.push(stem.to_string());
                    }
                }
            }
        }
    }
    list.sort();
    list.dedup();
    list
}

/// 是否为基础游戏语言包：提供 VCMI 基础翻译作用域文件
/// `content/translation/{game,campaigns,maps,chronicles}.json`
/// （汉化包翻译的是内容模组，文件按模组命名，不会有这些固定文件名）。
/// 同时覆盖磁盘散文件与 `content.zip` 两种形态。仅对声明 `modType == "Translation"` 的模组调用，
/// 避免对全部模组做代价较高的目录/zip 探测。
fn has_base_translation(dir: &Path) -> bool {
    const BASE_FILES: &[&str] = &[
        "game.json",
        "campaigns.json",
        "maps.json",
        "chronicles.json",
    ];
    // 1) 磁盘 Content/translation*/<base>.json（VCMI 标准大写 Content，并兼容小写 content）
    for content_root in [dir.join("Content"), dir.join("content")] {
        if !content_root.is_dir() {
            continue;
        }
        for tr in ["translation", "translations"] {
            let tr_dir = content_root.join(tr);
            if tr_dir.is_dir() {
                for f in BASE_FILES {
                    if tr_dir.join(f).is_file() {
                        return true;
                    }
                }
            }
        }
    }
    // 2) content.zip 内任一路径段以 `translation` 开头、且末段为 base 文件
    if let Ok(file) = fs::File::open(dir.join("content.zip")) {
        if let Ok(mut archive) = zip::ZipArchive::new(file) {
            for i in 0..archive.len() {
                if let Ok(entry) = archive.by_index(i) {
                    let nl = entry.name().replace('\\', "/").to_lowercase();
                    if !nl.ends_with(".json") {
                        continue;
                    }
                    let segs: Vec<&str> = nl.split('/').collect();
                    let fname = segs.last().copied().unwrap_or("");
                    let in_tr = segs.iter().any(|s| s.starts_with("translation"));
                    if in_tr && BASE_FILES.iter().any(|b| *b == fname) {
                        return true;
                    }
                }
            }
        }
    }
    false
}

/// 扫描单个 mod 目录（含子 mod），返回 JSON 节点
pub(crate) fn scan_mod_dir(dir: &Path, parent_id: Option<&str>) -> Value {
    let disk_name = dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let mod_json = dir.join("mod.json");
    let id = norm_mod_id(parent_id, &disk_name);
    let mut name = disk_name.clone();
    let mut desc = String::new();
    let mut mod_type: Option<String> = None;
    let mut depends: Vec<String> = vec![];
    let mut soft_depends: Vec<String> = vec![];
    let mut conflicts: Vec<String> = vec![];
    let mut version: Option<String> = None;
    let mut author: Option<String> = None;
    let mut factions: Vec<String> = vec![];
    let mut keep_disabled: bool = false;
    let mut language: Option<String> = None;
    let mut compatibility: Option<Value> = None;
    let mut m = Value::Null;

    if mod_json.exists() {
        m = json::read_vcmi_json(&mod_json);
        if let Some(obj) = m.as_object() {
            if let Some(n) = obj.get("name").and_then(|v| v.as_str()) {
                name = n.to_string();
            }
            if let Some(d) = obj.get("description").and_then(|v| v.as_str()) {
                desc = d.to_string();
            }
            mod_type = obj
                .get("modType")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            depends = str_arr(obj.get("depends"));
            soft_depends = str_arr(obj.get("softDepends"));
            conflicts = str_arr(obj.get("conflicts"));
            version = obj
                .get("version")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            author = obj
                .get("author")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            factions = str_arr(obj.get("factions"));
            // keepDisabled：true=安装后不自动启用；缺失按 VCMI 语义视为 false（默认启用）。
            keep_disabled = obj
                .get("keepDisabled")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            // language：翻译类模组的基准语言；VCMI 用它判定「与 UI 语言不符」→ BROKEN
            // （`lib/modding/ModManager.cpp` 的 ModDependenciesResolver::tryAddMods 规则之一）。
            language = obj
                .get("language")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            // compatibility：`{ min, max }`（引擎支持的版本区间）。判定语义见
            // `lib/modding/ModDescription.cpp::isCompatible` 与 `CModVersion::compatible`：
            // major 必须相等；min/max 只写到某一段（如 "1.8"）时该段以下不再比较。
            // 这里**只透传原始字符串**，比较在前端做（避免后端持有多份引擎版本源）。
            if let Some(c) = obj.get("compatibility").and_then(|v| v.as_object()) {
                let min = c.get("min").and_then(|v| v.as_str()).unwrap_or("");
                let max = c.get("max").and_then(|v| v.as_str()).unwrap_or("");
                if !min.is_empty() || !max.is_empty() {
                    compatibility = Some(json!({ "min": min, "max": max }));
                }
            }
        }
    }

    // 若 mod.json 未显式声明 factions，自动从城镇配置目录探测
    if factions.is_empty() {
        factions = detect_factions(dir);
    }

    let has_content_zip = dir.join("content.zip").exists();

    // 基础游戏语言包标记：仅对声明 Translation 的模组探测（避免全量代价），
    // 供前端区分"基础语言包"与"内容汉化包"（两者都标 modType=Translation）。
    let base_translation = mod_type.as_deref() == Some("Translation") && has_base_translation(dir);

    // 子 mod
    let mut subs: Vec<Value> = vec![];
    for sub_dir_name in ["Mods", "mods"] {
        let sub_dir = dir.join(sub_dir_name);
        if sub_dir.is_dir() {
            if let Ok(entries) = fs::read_dir(&sub_dir) {
                for entry in entries.flatten() {
                    if entry.path().is_dir() {
                        let sub = scan_mod_dir(&entry.path(), Some(&id));
                        subs.push(sub);
                    }
                }
            }
            break;
        }
    }

    // 通用多语言块：收集 mod.json 中所有对象型语言块（chinese/english/tchinese/...）
    let i18n = collect_i18n(&m);

    let type_str = classify_mod(&id, mod_type.as_deref(), &name, &desc, &factions);
    json!({
        "id": id,
        "diskName": disk_name,
        "parentId": parent_id.unwrap_or(&id).to_string(),
        "name": name,
        "localizedName": name,
        "desc": desc,
        "i18n": i18n,
        "type": type_str,
        // mod.json 的 modType **原文**（未做关键词推断）：`type` 可能是猜的，
        // 而"翻译模组语言与游戏语言不符"这类判定必须用真值（VCMI 只看 modType）。
        "rawType": mod_type,
        "factions": factions,
        "version": version,
        "author": author,
        "language": language,
        // 是否为基础游戏语言包（提供 content/translation/{game,campaigns,maps,chronicles}.json）。
        // 区分"基础语言包"与"内容汉化包"（两者 modType 都是 Translation）。
        "baseTranslation": base_translation,
        "compatibility": compatibility,
        "depends": depends,
        "softDepends": soft_depends,
        "conflicts": conflicts,
        "modPath": dir.to_string_lossy(),
        "hasContentZip": has_content_zip,
        "contentZipEncrypted": if has_content_zip { content_zip_is_encrypted(dir) } else { false },
        "isSubmod": parent_id.is_some(),
        "keepDisabled": keep_disabled,
        "subs": subs,
    })
}

/// 不属于 VCMI 多语言块的已知字段
pub(crate) const NON_I18N_KEYS: &[&str] = &[
    "name",
    "description",
    "modType",
    "version",
    "author",
    "depends",
    "softDepends",
    "conflicts",
    "language",
    "factions",
    "compatibility",
    "keepDisabled",
];

pub(crate) fn collect_i18n(m: &Value) -> Value {
    let mut map = serde_json::Map::new();
    if let Some(obj) = m.as_object() {
        for (k, v) in obj {
            if NON_I18N_KEYS.contains(&k.as_str()) {
                continue;
            }
            if let Some(block) = v.as_object() {
                let has_name = block.get("name").map_or(false, |x| x.is_string());
                let has_desc = block.get("description").map_or(false, |x| x.is_string());
                if has_name || has_desc {
                    map.insert(k.clone(), v.clone());
                }
            }
        }
    }
    Value::Object(map)
}

pub(crate) fn str_arr(v: Option<&Value>) -> Vec<String> {
    v.and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|x| x.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default()
}

/// 扫描单个 Mods 目录的顶层 mod（供扫描 fallback 合并用）
/// 单目录模组扫描结果缓存：`key = 目录路径|mtime`。
///
/// 567 个 mod 时一次实扫要 `read_dir` + 逐个解析 `mod.json`（数百毫秒，HDD 更久），
/// 而 `scan_mods` / `list_installed_mods` / `collect_mod_paths` 都会调用它。
static USER_MODS_SCAN_CACHE: std::sync::OnceLock<Mutex<Vec<(String, std::sync::Arc<Vec<Value>>)>>> =
    std::sync::OnceLock::new();

/// 扫一个 Mods 目录（**带指纹缓存**，见 [`USER_MODS_SCAN_CACHE`]）。
pub(crate) fn scan_user_mods_dir(user_mods_dir: &Path) -> Vec<Value> {
    let key = format!(
        "{}|{}",
        user_mods_dir.display(),
        dir_mtime_key(user_mods_dir)
    );
    let arc = cfg_cache_get(&USER_MODS_SCAN_CACHE, key, || {
        scan_user_mods_dir_uncached(user_mods_dir)
    });
    arc.as_ref().clone()
}

/// [`scan_user_mods_dir`] 的无缓存实现（真正读盘 + 解析 `mod.json`）。
fn scan_user_mods_dir_uncached(user_mods_dir: &Path) -> Vec<Value> {
    let mut root_mods: Vec<Value> = vec![];
    if user_mods_dir.is_dir() {
        if let Ok(entries) = fs::read_dir(user_mods_dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() {
                    let disk = p
                        .file_name()
                        .map(|s| s.to_string_lossy().to_string())
                        .unwrap_or_default();
                    if RESERVED_SCOPES.contains(&disk.as_str()) || disk.starts_with('.') {
                        continue;
                    }
                    if p.join("mod.json").exists() {
                        root_mods.push(scan_mod_dir(&p, None));
                    }
                }
            }
        }
    }
    root_mods
}

/// 把单个模组（含其 `subs` 子模组）递归展开为领域选择所需的精简条目：
/// `{id, name, desc, i18n, version, author, source, type, parentId, isSubmod, factions}`。
/// 复用 `scan_mod_dir` 已识别的 modType / parentId / isSubmod / factions，
/// 不重新解析 mod.json，避免重复开销与分类不一致（§9.6.1 / §3.4）。
/// 展示元数据（desc / i18n / version / author）**必须透传**：领域「可引用」页的
/// 行名称与抽屉简介依赖 i18n 选词，缺了它们只能显示英文原文 / 简介恒空。
pub(crate) fn push_installed_mod(
    m: &Value,
    source: &str,
    seen: &mut std::collections::HashSet<String>,
    out: &mut Vec<Value>,
) {
    let id = m
        .get("id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if id.is_empty() || !seen.insert(id.clone()) {
        return;
    }
    let name = match m.get("name") {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Object(o)) => o
            .get("chinese")
            .and_then(|v| v.as_str())
            .or_else(|| o.get("english").and_then(|v| v.as_str()))
            .unwrap_or(&id)
            .to_string(),
        _ => id.clone(),
    };
    let type_str = m
        .get("type")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    // modType 原文（未推断）：语言不符判定必须用它（见 `scan_mod_dir` 的同名字段说明）
    let raw_type = m.get("rawType").cloned().unwrap_or(Value::Null);
    let parent_id = m
        .get("parentId")
        .and_then(|v| v.as_str())
        .unwrap_or(&id)
        .to_string();
    let is_submod = m.get("isSubmod").and_then(|v| v.as_bool()).unwrap_or(false);
    let factions = m.get("factions").cloned().unwrap_or(Value::Null);
    out.push(json!({
        "id": id,
        // 磁盘目录原名（保留大小写、可能含空格）+ 绝对路径。
        //
        // ⚠️ 必须透传：`id` 是**小写归一**后的 VCMI modId，**不能当目录名用**
        // （大小写敏感的文件系统上找不到）。此前 `InstalledMod` 不带这两个字段，
        // 消费方（领域编辑器、ModBrowser 兜底条目）只能拿小写 `id` 冒充 `diskName`，
        // 而领域侧 `realms/*` 又用它 `join` 出磁盘路径 —— 跨平台会静默失效。
        "diskName": m.get("diskName").cloned().unwrap_or(json!(id)),
        "modPath": m.get("modPath").cloned().unwrap_or(json!("")),
        "name": name,
        // 展示元数据（scan_mod_dir 同源透传）：多语言名/描述选词、抽屉简介、版本列回退都依赖。
        "desc": m.get("desc").cloned().unwrap_or(json!("")),
        "i18n": m.get("i18n").cloned().unwrap_or(json!({})),
        "version": m.get("version").cloned().unwrap_or(Value::Null),
        "author": m.get("author").cloned().unwrap_or(Value::Null),
        "source": source,
        "type": type_str,
        "parentId": parent_id,
        "isSubmod": is_submod,
        "keepDisabled": m.get("keepDisabled").and_then(|v| v.as_bool()).unwrap_or(false),
        "factions": factions,
        // 依赖信息（与 ModInfo 对齐）：禁用某模组时前端据此做「必需依赖守卫」（§3.3 / 领域模组页）。
        "depends": m.get("depends").cloned().unwrap_or(json!([])),
        "softDepends": m.get("softDepends").cloned().unwrap_or(json!([])),
        "conflicts": m.get("conflicts").cloned().unwrap_or(json!([])),
        // 引擎兼容区间（`compatibility.min/max`）：领域模组列表也要做兼容提示。
        "compatibility": m.get("compatibility").cloned().unwrap_or(Value::Null),
        "language": m.get("language").cloned().unwrap_or(Value::Null),
        "rawType": raw_type,
        // 基础游戏语言包标记：领域列表也要做「基础语言包 vs 内容汉化」区分
        // （领域自带的语言包识别依赖它，见 `realms::mod_manage::ensure_realm_lang_mod`）。
        "baseTranslation": m.get("baseTranslation").cloned().unwrap_or(json!(false)),
    }));
    if let Some(subs) = m.get("subs").and_then(|v| v.as_array()) {
        for s in subs {
            push_installed_mod(s, source, seen, out);
        }
    }
}

/// 模组条目：**`(磁盘绝对路径, 规范 modId)`**。
///
/// 带上 `modId` 是为了让资源解析能直接答出「这份素材属于哪个模组」，供百科缓存把产物
/// 归属到**素材真正的来源方**而非提取调用方（游戏级素材 → `core`；跨模组引用 → 被引用方）。
/// `.0` = 磁盘路径（大小写敏感文件系统上定位用），`.1` = modId（缓存归属用）。
pub(crate) type ModEntry = (PathBuf, String);

/// 遍历所有 mod（含子 mod）收集磁盘路径 + modId 列表，供跨 mod 资源解析。
/// 接受已解出的 `&VcmiPaths`，便于在 spawn_blocking 闭包内使用（无需持有 State）。
pub(crate) fn collect_mod_paths(paths: &VcmiPaths) -> Vec<ModEntry> {
    let game = paths.game_mods_dir();
    let user = paths.user_mods_dir();
    let mut list: Vec<ModEntry> = vec![];
    let mut seen = std::collections::HashSet::new();
    fn walk(v: &Value, list: &mut Vec<ModEntry>, seen: &mut std::collections::HashSet<String>) {
        if let Some(obj) = v.as_object() {
            if let Some(mp) = obj.get("modPath").and_then(|x| x.as_str()) {
                if seen.insert(mp.to_string()) {
                    let mid = obj
                        .get("id")
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string();
                    list.push((PathBuf::from(mp), mid));
                }
            }
            if let Some(subs) = obj.get("subs").and_then(|x| x.as_array()) {
                for s in subs {
                    walk(s, list, seen);
                }
            }
        } else if let Some(arr) = v.as_array() {
            for it in arr {
                walk(it, list, seen);
            }
        }
    }
    let game_mods = scan_user_mods_dir(&game);
    let user_mods = scan_user_mods_dir(&user);
    walk(&json!(game_mods), &mut list, &mut seen);
    walk(&json!(user_mods), &mut list, &mut seen);
    //
    // ⚠️ 排序是**必需**而非美化：跨模组检索是「遍历全部模组，首个命中即返回」，
    // 命中结果直接决定素材归属（进而决定产物落盘目录）。顺序若沿用文件系统枚举顺序，
    // 归属就会跨机器 / 跨文件系统不可复现 → 产物路径变化 → 缓存键抖动 → 反复重建。
    // 按 modId 字典序固定，任意机器上同一模组集合得到同一顺序。
    list.sort_by(|a, b| a.1.cmp(&b.1));
    list
}

/// `.def` 资源定位用的模组路径列表：按 Mods 目录指纹缓存，
/// 避免每次 `read_def_bytes` 都重新扫描全部模组（600+ 目录）造成卡顿。
pub(crate) static MOD_PATHS_CACHE: std::sync::OnceLock<
    Mutex<Vec<(String, std::sync::Arc<Vec<ModEntry>>)>>,
> = std::sync::OnceLock::new();

pub(crate) fn cached_mod_paths(paths: &VcmiPaths) -> std::sync::Arc<Vec<ModEntry>> {
    // ⚠️ key **必须**带两个 Mods 目录的 mtime 指纹，不能只有路径：
    // 否则启动器运行期间新装 / 删除的模组永远不生效（官方启动器把 mod 装到
    // `userDataPath/Mods`，用户也可能手工放进 `游戏目录/Mods`），
    // 连带 `CROSSMOD_MISS`（"这个资源哪都没有"）的负结论也不会失效。
    let key = format!(
        "{}|{}|{}|{}",
        paths.game_mods_dir().display(),
        paths.user_mods_dir().display(),
        dir_mtime_key(&paths.game_mods_dir()),
        dir_mtime_key(&paths.user_mods_dir()),
    );
    cfg_cache_get(&MOD_PATHS_CACHE, key, || collect_mod_paths(paths))
}

/// 目录 mtime 指纹（纳秒字符串；目录不存在 → `"none"`）。
///
/// 只取 mtime、**不数子项**是刻意的：新增/删除子目录必然改变父目录 mtime（NTFS 可靠），
/// 而 `read_dir().count()` 在 567 个 mod 时每次要几毫秒，本函数却会被详情页的每个资源
/// 请求调用两次（`read_def_bytes` + `anim_json::locate`）。
pub(crate) fn dir_mtime_key(dir: &Path) -> String {
    fs::metadata(dir)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos().to_string())
        .unwrap_or_else(|| "none".to_string())
}

/// 强制作废模组相关缓存（安装 / 卸载模组、手动重扫、切换领域后调用）。
///
/// 平时靠 [`cached_mod_paths`] / [`scan_user_mods_dir`] 的目录 mtime 指纹自动失效；
/// 本函数用于 mtime 不可靠（网络盘 / 某些虚拟文件系统）或调用方明确知道集合变了、
/// 要**立刻**生效的场合。
pub(crate) fn invalidate_mod_paths() {
    crate::infra::cache::cfg_cache_clear(&MOD_PATHS_CACHE);
    crate::infra::cache::cfg_cache_clear(&USER_MODS_SCAN_CACHE);
    crate::vcmi::resource::clear_crossmod_caches();
}
