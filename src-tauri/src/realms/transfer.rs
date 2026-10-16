//! 领域导出 / 导入（zip）与包冲突处理。
//!
//! 导入的错误串格式（`{code,msg?,realmId?,realmName?}`，含 `conflict:` 前缀分支）是
//! **前端契约**（见设计文档 §7.1），保持逐字不变。

use super::base::{ensure_realm_dirs, now_date, read_config, REALM_PRESET_ID};
use super::maps::sync_realm_maps_meta;
use super::meta::{get_realm, realm_id_exists, remove_realm_dir_safely, write_realm_meta};
use super::presets::write_realm_preset;
use crate::infra::berror::be_err;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, State};
use zip::write::FileOptions;
use zip::ZipArchive;
use zip::ZipWriter;

use crate::infra::links::is_link;
use crate::state::AppPaths;
use crate::vcmi::paths::VcmiPaths;
use crate::vcmi::scan::scan_user_mods_dir;

/// 递归收集目录内文件，写入 `out: (相对包内路径, 绝对路径)`。
/// 跳过目录联接（借用模组 / 地图阀门不随包分发，设计 §7.1「借用模组不打包」）。
pub(crate) fn collect_dir(
    dir: &Path,
    prefix: &str,
    out: &mut Vec<(String, PathBuf)>,
) -> Result<(), String> {
    for entry in
        fs::read_dir(dir).map_err(|e| be_err("realm_dir_read_failed", &format!("{}", e)))?
    {
        let entry = entry.map_err(|e| be_err("realm_dir_read_failed", &format!("{}", e)))?;
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        if is_link(&path) {
            continue; // 跳过联接，不随包分发
        }
        let rel = format!("{prefix}/{name}");
        if path.is_dir() {
            collect_dir(&path, &rel, out)?;
        } else {
            out.push((rel, path));
        }
    }
    Ok(())
}

/// 收集领域快照需要打包的文件：仅 `vcmi/{Mods,Maps,config}` 与 `assets/`；
/// 排除 `cache`/`logs`/`Saves`、`.introPlayedAt`、`.bgmMuted`、`.prevActivePreset`、`lock` 等用户态点文件。
pub(crate) fn collect_realm_files(
    root: &Path,
    out: &mut Vec<(String, PathBuf)>,
) -> Result<(), String> {
    let allowed_vcmi = ["Mods", "Maps", "config"];
    for entry in
        fs::read_dir(root).map_err(|e| be_err("realm_realm_dir_read_failed", &format!("{}", e)))?
    {
        let entry = entry.map_err(|e| be_err("realm_realm_dir_read_failed", &format!("{}", e)))?;
        let name = entry.file_name().to_string_lossy().to_string();
        let path = entry.path();
        if name == "vcmi" && path.is_dir() {
            for sub in fs::read_dir(&path)
                .map_err(|e| be_err("realm_realm_dir_read_failed", &format!("{}", e)))?
            {
                let sub =
                    sub.map_err(|e| be_err("realm_realm_dir_read_failed", &format!("{}", e)))?;
                let sname = sub.file_name().to_string_lossy().to_string();
                if !allowed_vcmi.contains(&sname.as_str()) {
                    continue;
                }
                if is_link(&sub.path()) {
                    continue;
                }
                collect_dir(&sub.path(), &format!("vcmi/{sname}"), out)?;
            }
        } else if name == "assets" && path.is_dir() && !is_link(&path) {
            collect_dir(&path, "assets", out)?;
        }
        // 其它顶层项（cache/logs/Saves/lock/.introPlayedAt 等）不打包
    }
    Ok(())
}

/// 导出期临时文件守护：无论成功、失败还是取消都删除（配合 [`sanitize_lang_packs_in_presets`]）。
struct TmpFileGuard(Option<PathBuf>);
impl Drop for TmpFileGuard {
    fn drop(&mut self) {
        if let Some(p) = &self.0 {
            let _ = fs::remove_file(p);
        }
    }
}

/// 剔除导出包内 `vcmi/config/modSettings.json` 预设里「**不在领域台账**的语言包 id」。
///
/// 语言层（`realms::mod_manage::ensure_realm_lang_visible`）由启动器在运行期注入，属于**导出方本机**
/// 的显示语言选择，分发出去只会让接收方的预设里多出本机没有的模组引用。
///
/// 判据**复用既有台账、不引入新概念**：
/// - 维护者显式引用（`borrowed` / `bundled` / `pinned`）的语言包 → 属领域内容 → **保留**；
/// - 不在台账、仅出现在预设里 → 启动器注入或本机勾选 → **剔除**。
///
/// "是否语言包"按顶层 `rawType == "Translation"` 判定，数据源为领域 Mods + 游戏 Mods + 全局 user Mods。
/// 任何一步读不到 / 解析失败都**原样返回**（导出不该因此失败，更不该冒改写风险）。
///
/// 返回替换后的临时文件路径（未发生剔除时 `None`），由调用方在打包结束后删除。
fn sanitize_lang_packs_in_presets(
    id: &str,
    root: &Path,
    realm: &Value,
    files: &mut Vec<(String, PathBuf)>,
) -> Result<Option<PathBuf>, String> {
    const REL: &str = "vcmi/config/modSettings.json";
    let Some(idx) = files.iter().position(|(r, _)| r == REL) else {
        return Ok(None);
    };
    let src = files[idx].1.clone();
    let Ok(text) = fs::read_to_string(&src) else {
        return Ok(None);
    };
    let mut settings: Value = match crate::infra::json::parse_json_lenient(&text) {
        Ok(v) => v,
        Err(_) => return Ok(None),
    };

    // 语言包 id 集合（小写）：顶层 Translation 类
    let mut lang_ids: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut scan_dir = |dir: &Path| {
        for m in scan_user_mods_dir(dir) {
            if m.get("rawType").and_then(|v| v.as_str()) == Some("Translation") {
                if let Some(mid) = m.get("id").and_then(|v| v.as_str()) {
                    lang_ids.insert(mid.to_ascii_lowercase());
                }
            }
        }
    };
    scan_dir(&root.join("vcmi").join("Mods"));
    if let Some(cfg) = VcmiPaths::read_launcher_config() {
        let cfg = cfg.resolve_relative_to(&VcmiPaths::exe_dir());
        scan_dir(&cfg.paths.user_mods_dir);
        let gm = cfg
            .paths
            .game_mods_dir
            .clone()
            .unwrap_or_else(|| cfg.paths.game_dir.join("Mods"));
        scan_dir(&gm);
    }

    // 台账声明集合（小写）：维护者显式引用 → 保留
    let mods_val = realm.get("mods").cloned().unwrap_or(Value::Null);
    let mut declared: std::collections::HashSet<String> = std::collections::HashSet::new();
    for key in ["borrowed", "bundled"] {
        if let Some(arr) = mods_val.get(key).and_then(|v| v.as_array()) {
            for x in arr.iter().filter_map(|v| v.as_str()) {
                declared.insert(x.to_ascii_lowercase());
            }
        }
    }
    if let Some(arr) = mods_val.get("pinned").and_then(|v| v.as_array()) {
        for x in arr {
            let s = match x {
                Value::String(s) => s.clone(),
                obj => obj
                    .get("id")
                    .and_then(|i| i.as_str())
                    .unwrap_or_default()
                    .to_string(),
            };
            if !s.is_empty() {
                declared.insert(s.to_ascii_lowercase());
            }
        }
    }

    let mut changed = false;
    if let Some(presets) = settings.get_mut("presets").and_then(|v| v.as_object_mut()) {
        for (_name, entry) in presets.iter_mut() {
            if let Some(list) = entry.get_mut("mods").and_then(|v| v.as_array_mut()) {
                let before = list.len();
                list.retain(|v| {
                    let Some(mid) = v.as_str() else { return true };
                    let key = mid.to_ascii_lowercase();
                    !(lang_ids.contains(&key) && !declared.contains(&key))
                });
                if list.len() != before {
                    changed = true;
                }
            }
        }
    }
    if !changed {
        return Ok(None);
    }

    let tmp = std::env::temp_dir().join(format!("fv-realm-export-{id}.modSettings.json"));
    let out = serde_json::to_string_pretty(&settings)
        .map_err(|e| be_err("realm_json_serialize_failed", &format!("{}", e)))?;
    fs::write(&tmp, out)
        .map_err(|e| be_err("realm_export_file_create_failed", &format!("{}", e)))?;
    files[idx].1 = tmp.clone();
    Ok(Some(tmp))
}

/// 原版伪模组 id（`core` / `core.<faction>`）：其素材由接收方本机游戏数据生成，不随包分发。
/// 判定口径与 `modcache::version::parse_core_id` 的约定同源（大小写不敏感）。
fn is_core_mod_id(id: &str) -> bool {
    let l = id.to_ascii_lowercase();
    l == "core" || l.starts_with("core.")
}

/* ----------------------------- 导出：取消标记与进度 ----------------------------- */

/// 被取消的导出任务（领域 id 列表）—— 与 `repository::CANCELLED` 同款哨兵模式。
static EXPORT_CANCELLED: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn export_is_cancelled(id: &str) -> bool {
    EXPORT_CANCELLED
        .lock()
        .map(|v| v.iter().any(|x| x == id))
        .unwrap_or(false)
}

fn export_clear_cancelled(id: &str) {
    if let Ok(mut v) = EXPORT_CANCELLED.lock() {
        v.retain(|x| x != id);
    }
}

/// 取消正在进行的领域导出（按领域 id）。
/// 实际中止点在打包循环：逐文件查标记，命中即停并删除半成品。
#[tauri::command]
pub fn cancel_realm_export(realm_id: String) -> Result<(), String> {
    let mut v = EXPORT_CANCELLED
        .lock()
        .map_err(|_| be_err("realm_cancel_lock_corrupt", ""))?;
    if !v.contains(&realm_id) {
        v.push(realm_id);
    }
    Ok(())
}

/// 已压缩内容的扩展名：ZIP 里再 Deflate 只是白烧 CPU（webp / mp4 / lod 本身就是压缩容器）。
/// 这些条目改用 `Stored`，打包耗时降三到五成；**不影响导入校验** ——
/// `checksum` 基于条目原始内容与相对路径，与压缩方式无关。
const STORED_EXTS: &[&str] = &[
    "webp", "png", "jpg", "jpeg", "gif", "bmp", "tga", "mp4", "webm", "mkv", "ogv", "avi", "mp3",
    "ogg", "wav", "flac", "zip", "7z", "gz", "lod", "hd7", "pak", "h3m", "h3c",
];

fn zip_options(rel: &str) -> FileOptions<'static, ()> {
    let method = if is_precompressed(rel) {
        zip::CompressionMethod::Stored
    } else {
        zip::CompressionMethod::Deflated
    };
    FileOptions::default().compression_method(method)
}

/// 按扩展名判定条目是否已是压缩容器（`FileOptions` 没有 getter，单独拆出来才可单测）。
fn is_precompressed(rel: &str) -> bool {
    let ext = Path::new(rel)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    STORED_EXTS.contains(&ext.as_str())
}

/// 打包结果：完成（附条目校验和）或被取消。
enum PackOutcome {
    Done { checksum: String },
    Cancelled,
}

/// 把所有条目「读 64 KiB → 喂哈希 → 写 zip」**单遍流式**落进压缩包。
///
/// - 不再“先全量哈希再全量打包”：省一遍 IO，也不把整文件驻内存（大包峰值内存从数百 MB 降到常数）；
/// - checksum 与导入侧逐字对齐：对 `(相对路径, 原始内容)` 按路径排序后求 SHA-256，
///   `realm.json`（含该 checksum）由调用方在**所有条目之后**写入，因此一次遍历即可成立；
/// - 已压缩扩展名走 `Stored`（见 [`zip_options`]），不影响校验和（它只看原始内容）。
///
/// 抽成独立函数是为了可单测：Stored/Deflated 混排 + 排序 + 校验和顺序，
/// 是“导出的包能不能被导入侧校验通过”的关键，回归时不该依赖真机点一次导出。
fn write_zip_entries(
    zip: &mut ZipWriter<fs::File>,
    realm_id: &str,
    files: &[(String, PathBuf)],
    mut on_progress: impl FnMut(usize, usize, u64, &str),
) -> Result<PackOutcome, String> {
    let mut hasher = Sha256::new();
    let mut bytes_done = 0u64;
    let mut since_cancel_check = 0u64;
    let mut buf = vec![0u8; 64 * 1024];
    for (i, (rel, abs)) in files.iter().enumerate() {
        if export_is_cancelled(realm_id) {
            return Ok(PackOutcome::Cancelled);
        }
        hasher.update(rel.as_bytes());
        let mut src =
            fs::File::open(abs).map_err(|e| be_err("infra_file_read_failed", &format!("{}", e)))?;
        zip.start_file(rel.as_str(), zip_options(rel))
            .map_err(|e| be_err("realm_archive_write_failed", &format!("{}", e)))?;
        loop {
            let n = src
                .read(&mut buf)
                .map_err(|e| be_err("infra_file_read_failed", &format!("{}", e)))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            zip.write_all(&buf[..n])
                .map_err(|e| be_err("realm_archive_write_failed", &format!("{}", e)))?;
            bytes_done += n as u64;
            // 只在条目边界查取消，会让“单个超大文件”（几百 MB 过场视频）迟迟不响应；
            // 故每累计 ~4 MiB 再查一次（一次互斥锁，开销可忽略）。
            since_cancel_check += n as u64;
            if since_cancel_check >= 4 * 1024 * 1024 {
                since_cancel_check = 0;
                if export_is_cancelled(realm_id) {
                    return Ok(PackOutcome::Cancelled);
                }
            }
        }
        on_progress(i + 1, files.len(), bytes_done, rel);
    }
    Ok(PackOutcome::Done {
        checksum: crate::infra::hash::to_hex(&hasher.finalize()),
    })
}

/// 导出进度推送器：200 ms 节流（与 `mod-download-progress` 同口径），末帧（done>=total）强制发。
struct ExportReporter {
    app: AppHandle,
    realm_id: String,
    last: Instant,
}

impl ExportReporter {
    fn new(app: AppHandle, realm_id: String) -> Self {
        Self {
            app,
            realm_id,
            // 预置一个已过期的时间戳：首帧立即发出，前端不会长时间停在 0
            last: Instant::now() - Duration::from_secs(1),
        }
    }

    fn emit(
        &mut self,
        phase: &str,
        done: usize,
        total: usize,
        bytes_done: u64,
        bytes_total: u64,
        current: &str,
    ) {
        let finished = total > 0 && done >= total;
        if !finished && self.last.elapsed() < Duration::from_millis(200) {
            return;
        }
        self.last = Instant::now();
        let _ = self.app.emit(
            "realm-export-progress",
            json!({
                "realmId": self.realm_id,
                "phase": phase,
                "done": done,
                "total": total,
                "bytesDone": bytes_done,
                "bytesTotal": bytes_total,
                "current": current,
            }),
        );
    }
}

/// 导出领域快照（§7.1）：打包为 `.vcmirealm`（ZIP）。
/// - 包根 `realm.json`：领域元数据（含 `checksum`，作为完整性校验和）。
/// - `vcmi/{Mods,Maps,config}` 与 `assets/`：领域数据本体。
/// - 借用模组（目录联接）不打包；ZIP 注释写入 `vcmirealm` 作为包格式魔数，便于快速识别。
/// - 完整性校验：`checksum` = 对其余条目按路径排序后的 SHA-256。
///
/// 导出是重 IO（大包可达数百 MB），同步命令会冻结窗口 —— 命令壳只接参数，
/// 主体移交阻塞线程池（P8 同款），进度经 `realm-export-progress` 事件推送。
#[tauri::command]
pub async fn export_realm(
    app: AppHandle,
    realm_id: String,
    dest: String,
    // 附带「阵营巡礼」素材缓存（**默认 false**）：包内 `showcase-cache/<modId>/{cache.json,images/}`，
    // 接收方导入后无需重新采集即可播放（体积随之增大，由用户自行权衡）。
    include_showcase_cache: Option<bool>,
    // 生成用于远程仓库的索引（**默认 true**）：默认写 `<stem>.index.json` 片段；
    // 给了 `repo_dir` 则改为直写该仓库工作树的 `index.json`。
    include_index: Option<bool>,
    // 要打包的缓存 mod 列表（由前端按"本领域所需素材"算出，见 `lib/modcache/realmAssets.ts`）。
    // ⚠️ 必须由前端给：领域启用集合来自 modSettings，后端不重复实现该口径。
    cache_mods: Option<Vec<String>>,
    // 本地领域仓库工作树根（含 `index.json` 的那级）：给出即走「直写仓库」发布流程（§7.3）。
    repo_dir: Option<String>,
    // `download` / `cover` 的 URL 前缀；留空则由既有索引条目推导。
    url_base: Option<String>,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        export_realm_impl(
            app,
            realm_id,
            dest,
            include_showcase_cache,
            include_index,
            cache_mods,
            repo_dir,
            url_base,
        )
    })
    .await
    .map_err(|e| be_err("realm_export_task_failed", &format!("{}", e)))?
}

/// 导出主体（阻塞线程池执行）。路径全部经 `VcmiPaths` 的 exe 相对静态方法求取，无需 State。
pub fn export_realm_impl(
    app: AppHandle,
    realm_id: String,
    dest: String,
    include_showcase_cache: Option<bool>,
    include_index: Option<bool>,
    cache_mods: Option<Vec<String>>,
    repo_dir: Option<String>,
    url_base: Option<String>,
) -> Result<Value, String> {
    let id = VcmiPaths::sanitize_realm_id(&realm_id);
    export_clear_cancelled(&id); // 上次残留的取消标记不应影响本次
    let mut report = ExportReporter::new(app, id.clone());
    report.emit("prepare", 0, 0, 0, 0, "");
    // 收尾兜底：无论成功、失败还是取消，都清掉该领域的取消标记，避免下次导出被"秒取消"。
    struct CancelGuard(String);
    impl Drop for CancelGuard {
        fn drop(&mut self) {
            export_clear_cancelled(&self.0);
        }
    }
    let _guard = CancelGuard(id.clone());
    // 领域元数据来自目录内 realm.json（§3.1）；包内副本再加 checksum
    let realm = get_realm(&id)?;

    // 降级发布阻断（§7.3）：必须在**打包之前**判定——发布分支的 `dest` 就是仓库里的
    // 包路径，若等合并索引时才拦，包文件已被覆盖而索引未更新（半发布状态且无法安全回滚）。
    // 前端预览已按 `versionRel == "down"` 置灰按钮，这里兜住脚本调用与预览竞态。
    if let Some(dir) = repo_dir.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        let new_ver = realm
            .get("version")
            .and_then(|v| v.as_str())
            .unwrap_or("1.0.0");
        check_version_downgrade(Path::new(dir), &id, new_ver)?;
    }

    let root = realm_data_root(&id)?;

    let mut files: Vec<(String, PathBuf)> = Vec::new();
    collect_realm_files(&root, &mut files)?;

    // 语言层剔除：预设里"由启动器运行期注入"的语言包 id 不得随包分发（那是导出方本机的语言选择）。
    // 必须在 checksum 之前完成替换——替换只改路径、不改相对路径，排序与校验和口径不受影响；
    // 临时文件由守护对象在成功/失败/取消任一路径下删除。
    let _tmp_modsettings = TmpFileGuard(sanitize_lang_packs_in_presets(
        &id, &root, &realm, &mut files,
    )?);

    // 素材缓存：**必须在 checksum 之前加入**（导出/导入两侧都对全部条目求和，
    // 先算 checksum 再塞文件会导致导入校验失败）
    if include_showcase_cache.unwrap_or(false) {
        if let (Some(mods), Some(cache_root)) = (cache_mods.as_deref(), VcmiPaths::mods_cache_dir())
        {
            for mod_id in mods {
                // 原版伪模组不打包：打包它们会白增约 58 MB（撑爆 100 MB 下载上限），
                // 而缓存键含 `sourceHash` / `h3DataKey` / `h3CoreHash` / `schemaHash` ——
                // 跨机器、跨启动器版本必然失配，只会让接收方看到导出方的旧图。
                // 前端口径（`realmAssetTargets({ excludeCore: true })`）已过滤，这里是兜底。
                if is_core_mod_id(mod_id) {
                    continue;
                }
                let sid = crate::modcache::sanitize_id(mod_id);
                let dir = cache_root.join(&sid);
                if !dir.is_dir() {
                    continue;
                }
                let cache_json = dir.join("cache.json");
                if cache_json.is_file() {
                    files.push((format!("showcase-cache/{sid}/cache.json"), cache_json));
                }
                let images = dir.join("images");
                if images.is_dir() {
                    collect_dir(&images, &format!("showcase-cache/{sid}/images"), &mut files)?;
                }
            }
        }
    }

    // 与导入侧同序（按相对路径）：checksum 才可复现
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let bytes_total: u64 = files
        .iter()
        .map(|(_, p)| fs::metadata(p).map(|m| m.len()).unwrap_or(0))
        .sum();
    report.emit("collect", files.len(), files.len(), 0, bytes_total, "");

    let dest_path = Path::new(&dest);
    if let Some(parent) = dest_path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)
                .map_err(|e| be_err("realm_export_dir_create_failed", &format!("{}", e)))?;
        }
    }
    let file = fs::File::create(dest_path)
        .map_err(|e| be_err("realm_export_file_create_failed", &format!("{}", e)))?;
    let mut zip = ZipWriter::new(file);

    // **单遍流式**打包（见 [`write_zip_entries`]）；取消则删掉半成品回到可重试态。
    let checksum = match write_zip_entries(&mut zip, &id, &files, |done, total, bytes_done, rel| {
        report.emit("pack", done, total, bytes_done, bytes_total, rel);
    }) {
        Ok(PackOutcome::Done { checksum }) => checksum,
        Ok(PackOutcome::Cancelled) => {
            drop(zip);
            let _ = fs::remove_file(dest_path);
            return Ok(json!({ "ok": false, "cancelled": true, "path": dest }));
        }
        Err(e) => {
            drop(zip);
            let _ = fs::remove_file(dest_path);
            return Err(e);
        }
    };

    // 构建包根 realm.json（追加完整性校验和 + 剥离 `origin`；不含任何结构版本字段）
    let realm_bytes = build_package_realm_json(&realm, &checksum)?;
    zip.start_file("realm.json", zip_options("realm.json"))
        .map_err(|e| be_err("realm_archive_write_failed", &format!("{}", e)))?;
    zip.write_all(&realm_bytes)
        .map_err(|e| be_err("realm_archive_write_failed", &format!("{}", e)))?;

    // ZIP 注释：包格式魔数，便于快速识别合法包（§7.1）
    zip.set_comment("vcmirealm");
    zip.finish()
        .map_err(|e| be_err("realm_archive_finalize_failed", &format!("{}", e)))?;

    // 索引条目的 `sha256` = **整个 .vcmirealm 文件的 SHA-256**（与 `install_remote_realm` 的下载校验同源）；
    // 包内 `realm.json.checksum` = 条目内容校验和（与导入校验同源）。
    // ⚠️ 两者语义不同、值也不同，不可混用：曾把 checksum 当文件哈希写进索引，
    // 导致按该索引安装的领域必然「sha256 不匹配」（见仓库索引规范 §9.1）。
    let pkg_size = fs::metadata(&dest)
        .map_err(|e| be_err("realm_export_size_read_failed", &format!("{}", e)))?
        .len();
    let pkg_sha256 = crate::infra::hash::sha256_file(Path::new(&dest))?;
    let borrowed = realm
        .get("mods")
        .and_then(|m| m.get("borrowed"))
        .and_then(|b| b.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    // 标签：「键存在即声明」（含空数组 = 显式清空）——realm.json 无 tags 键时不写键，
    // 合并层据此保留索引既有值（键缺失 = 未声明；见仓库索引规范 §9.2）。
    let tags: Option<Vec<String>> = realm.get("tags").and_then(|t| t.as_array()).map(|a| {
        a.iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect::<Vec<_>>()
    });
    let version = realm
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("1.0.0")
        .to_string();
    let default_lang = realm
        .get("defaultLang")
        .and_then(|v| v.as_str())
        .unwrap_or("chinese")
        .to_string();
    // 多语言块：仅保留索引会用到的 name / description / tags，剥离 assets 等无关字段
    let i18n = realm
        .get("i18n")
        .and_then(|v| v.as_object())
        .map(|obj| {
            let mut out = serde_json::Map::new();
            for (lang, block) in obj {
                if let Some(b) = block.as_object() {
                    let mut nb = serde_json::Map::new();
                    if let Some(n) = b.get("name") {
                        nb.insert("name".to_string(), n.clone());
                    }
                    if let Some(d) = b.get("description") {
                        nb.insert("description".to_string(), d.clone());
                    }
                    // 标签仅接受数组（含空数组 = 显式清空）；null / 非数组不写出（保留索引既有值）
                    if let Some(t) = b.get("tags").filter(|v| v.is_array()) {
                        nb.insert("tags".to_string(), t.clone());
                    }
                    if !nb.is_empty() {
                        out.insert(lang.clone(), Value::Object(nb));
                    }
                }
            }
            if out.is_empty() {
                Value::Null
            } else {
                Value::Object(out)
            }
        })
        .unwrap_or(Value::Null);
    let mut entry = json!({
        "version": version,
        "author": realm.get("author").cloned().unwrap_or(Value::Null),
        "defaultLang": default_lang,
        "name": resolve_realm_i18n_field(&realm, "name")
            .map(Value::String)
            .unwrap_or(Value::Null),
        "description": resolve_realm_i18n_field(&realm, "description")
            .map(Value::String)
            .unwrap_or(Value::Null),
        "tags": tags.map_or(Value::Null, |t| json!(t)),
        "requiresMods": borrowed,
        "sha256": pkg_sha256,
        "size": pkg_size,
    });
    // realm.json 无 tags 键（未声明）→ 不写出本键：合并层保留索引既有值；空数组则照常写出（显式清空）。
    if entry["tags"].is_null() {
        entry.as_object_mut().expect("entry 是对象").remove("tags");
    }
    if i18n != Value::Null {
        entry["i18n"] = i18n;
    }
    if let Some(ge) = realm.get("gameEdition") {
        entry["gameEdition"] = ge.clone();
    }

    // 仓库产物（§7.3）：前端按分支传值——发布分支必写仓库，分享分支看「输出索引片段」勾选。
    // 参数缺省仍按启用处理（脚本 / 老调用方不传时保持旧行为）。
    // - 给了 `repo_dir` → 直写该仓库工作树（包副本 + 封面 + 合并 index.json），不再产出片段；
    // - 否则只写「仅含本领域的索引片段」`<stem>.index.json`，由发布者自行并入。
    let mut index_path: Option<PathBuf> = None;
    let mut repo_info: Option<Value> = None;
    let mut warnings: Vec<Value> = Vec::new();
    if include_index.unwrap_or(true) {
        let repo_arg = repo_dir.unwrap_or_default();
        if repo_arg.trim().is_empty() {
            let p = Path::new(&dest);
            let stem = p
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "index".to_string());
            let mut frag = p.parent().unwrap_or_else(|| Path::new("")).to_path_buf();
            frag.push(format!("{stem}.index.json"));
            let idx = json!({ "realms": { id.clone(): entry.clone() } });
            fs::write(
                &frag,
                serde_json::to_string_pretty(&idx).map_err(|e| {
                    be_err("realm_index_fragment_serialize_failed", &format!("{}", e))
                })?,
            )
            .map_err(|e| be_err("realm_index_fragment_write_failed", &format!("{}", e)))?;
            index_path = Some(frag);
        } else {
            report.emit("publish", 0, 0, 0, 0, "");
            let (info, warns) = publish_realm_to_repo(
                &realm,
                &root,
                &id,
                Path::new(&dest),
                Path::new(repo_arg.trim()),
                url_base.as_deref(),
                &mut entry,
                pkg_size,
            )?;
            repo_info = Some(info);
            warnings = warns;
        }
    }

    Ok(json!({
        "ok": true,
        "cancelled": false,
        "path": dest,
        "indexPath": index_path.map(|p| p.to_string_lossy().into_owned()),
        "repo": repo_info,
        "warnings": warnings,
        "fileCount": files.len(),
        "checksum": checksum,
        "sha256": pkg_sha256,
        "size": pkg_size,
        "bytesTotal": bytes_total,
        "showcaseCache": include_showcase_cache.unwrap_or(false),
        // 分片只在发布分支发生（分享分支恒为 0）；片数来自发布产物
        "partCount": repo_info
            .as_ref()
            .and_then(|r| r.get("partCount"))
            .and_then(|v| v.as_u64())
            .unwrap_or(0),
    }))
}

/// 告警条目：`code` 供前端映射多语言文案，`arg` 是已格式化的补充信息（体积 / 文件名等）。
fn warn(code: &str, arg: &str) -> Value {
    json!({ "code": code, "arg": arg })
}

/// 版本变更自检（降级已在 `downgrade_error` 上游阻断，不在此列）：
/// - 版本没变但包变了 → `version-unchanged`（接收方按版本判「已安装」，不会触发更新）；
/// - 版本变了但包一模一样 → `pkg-unchanged`（本次发布不带来任何内容变化）。
/// 纯函数，便于单测。
fn version_change_warnings(prev: Option<&Value>, entry: &Value) -> Vec<Value> {
    let Some(p) = prev else { return Vec::new() };
    let new_ver = entry
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let same_version =
        p.get("version").and_then(|v| v.as_str()) == entry.get("version").and_then(|v| v.as_str());
    let diff_sha =
        p.get("sha256").and_then(|v| v.as_str()) != entry.get("sha256").and_then(|v| v.as_str());
    if same_version && diff_sha {
        vec![warn("version-unchanged", &new_ver)]
    } else if !same_version && !diff_sha {
        vec![warn("pkg-unchanged", &new_ver)]
    } else {
        Vec::new()
    }
}

/// 领域包下载硬上限（启动器下载端会直接放弃）。
const PUBLISH_DOWNLOAD_MAX: u64 = 100 * 1024 * 1024;
/// Gitee 免费版 git 单文件上限：超过应改走「发行版附件」。
const PUBLISH_GIT_MAX: u64 = 50 * 1024 * 1024;

fn fmt_mb(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0)
}

/* ----------------------------- 分片（Sharding） ----------------------------- */

/// 单个分片的最大字节数 = **45 MiB**（47,185,920 B）。
///
/// 依据：GitHub 对 >50 MB 的单文件告警、>100 MB 拒绝 push；无论把 50 MB 按 MiB
/// （52,428,800）还是十进制 MB（50,000,000）理解，45 MiB 都留有余量（Gitee 免费版 git 同为 50 MB 线）。
/// 规格见 `docs/features/realms-sharded-package.zh-CN.md` §4。
pub(crate) const REALM_PART_MAX_BYTES: u64 = 45 * 1024 * 1024;

/// 单个分片规格（落库与索引条目 `parts[]` 共用）。
pub(crate) struct RealmPart {
    /// 分片文件名（`<stem>.part001`），相对 `realms/<id>/`
    pub(crate) name: String,
    pub(crate) size: u64,
    pub(crate) sha256: String,
}

/// 分片文件名：`<stem>.part001`（3 位零填充 → 999 片上限，约 43.9 GiB）。
pub(crate) fn part_name(stem: &str, index: usize) -> String {
    format!("{stem}.part{index:03}")
}

/// 分片文件名前缀（用于清理陈旧分片）：`<id>.part`。
fn part_prefix(id: &str) -> String {
    format!("{id}.part")
}

/// 把整包按**字节区间**切成固定大小的分片，产物写在源文件同目录。
///
/// 为什么是「成品切分」而不是改包结构：分片是托管站单文件上限的**传输层适配**，
/// 与 ZIP 内部结构无关——拼回 = 按序拼接，与原文逐字节一致。于是包格式、
/// `realm.json`、条目 checksum、导入校验全部零改（设计 §3 方案 A）。
///
/// 单遍 IO：逐片边读边写边算 SHA-256，不把整文件驻内存。任一步失败直接返回错误，
/// **半成品由调用方清理**（发布流程据此保证「单包已删、分片残缺」不会出现）。
pub(crate) fn split_realm_package(src: &Path, part_size: u64) -> Result<Vec<RealmPart>, String> {
    let total = fs::metadata(src)
        .map_err(|e| be_err("realm_export_size_read_failed", &format!("{}", e)))?
        .len();
    let dir = src.parent().unwrap_or_else(|| Path::new(""));
    let stem = src
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "realm".to_string());
    let count = ((total + part_size - 1) / part_size).max(1) as usize;
    let mut out: Vec<RealmPart> = Vec::with_capacity(count);
    let mut reader =
        fs::File::open(src).map_err(|e| be_err("infra_file_read_failed", &format!("{}", e)))?;
    let mut buf = vec![0u8; 1024 * 1024];
    for i in 0..count {
        let name = part_name(&stem, i + 1);
        let dest = dir.join(&name);
        let mut writer = fs::File::create(&dest)
            .map_err(|e| be_err("realm_export_file_create_failed", &format!("{}", e)))?;
        let mut hasher = Sha256::new();
        let mut written = 0u64;
        while written < part_size {
            let n = reader
                .read(&mut buf)
                .map_err(|e| be_err("infra_file_read_failed", &format!("{}", e)))?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            writer
                .write_all(&buf[..n])
                .map_err(|e| be_err("realm_file_write_failed", &format!("{}", e)))?;
            written += n as u64;
        }
        drop(writer);
        out.push(RealmPart {
            name,
            size: written,
            sha256: crate::infra::hash::to_hex(&hasher.finalize()),
        });
    }
    Ok(out)
}

/// 删除领域目录下的陈旧分片：只保留前 `keep` 片，其余（含不同 stem 的历史分片）一并清理。
///
/// 为什么必须主动清理：`index.json` 的 `parts` 只列当前片，仓库里多出来的旧片会被
/// 永久留在 git 历史里占体积，且与索引不一致（维护者看到的文件比索引多）。
fn prune_stale_parts(dir: &Path, id: &str, keep: usize) {
    let prefix = part_prefix(id);
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut names: Vec<String> = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with(&prefix) && e.path().is_file() {
            names.push(name);
        }
    }
    // 前 keep 片按名升序保留（`part001`…），其余删除 —— 名序即片序（3 位零填充）
    names.sort();
    for name in names.iter().skip(keep) {
        let _ = fs::remove_file(dir.join(name));
    }
}

/// 从既有索引条目推导仓库的 raw URL 前缀（形如 `https://host/owner/repo/raw/master/`）：
/// 取出现次数最多的 `download` 按 `/raw/<branch>/` 切。推不出返回 None（由调用方回退/告警）。
fn derive_url_base(existing: &Value) -> Option<String> {
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    let obj = existing.get("realms").and_then(|v| v.as_object())?;
    for e in obj.values() {
        let Some(d) = e.get("download").and_then(|v| v.as_str()) else {
            continue;
        };
        let Some(idx) = d.find("/raw/") else { continue };
        let rest = &d[idx + 5..];
        let Some(seg_end) = rest.find('/') else {
            continue;
        };
        *counts
            .entry(format!("{}/", &d[..idx + 5 + seg_end]))
            .or_default() += 1;
    }
    counts
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(base, _)| base)
}

/* ----------------------------- 下载地址前缀推导（S1 > S2 > S3） ----------------------------- */

/// 前缀来源（随发布结果回传，前端据此标「从哪推导的」）。
pub(crate) const URL_SOURCE_MANUAL: &str = "manual";
pub(crate) const URL_SOURCE_DECLARED: &str = "declared";
pub(crate) const URL_SOURCE_GIT: &str = "git";
pub(crate) const URL_SOURCE_ENTRIES: &str = "entries";
pub(crate) const URL_SOURCE_NONE: &str = "none";

/// 从工作树的 git 元数据推导 raw 前缀：`remote.origin.url` + 当前分支
/// → `https://<host>/<owner>/<repo>/raw/<branch>`（不带末尾 `/`）。
///
/// 为什么自己读文件而不是 `git ls-remote`：启动器不依赖 PATH 里有 git（便携包场景），
/// 而且这里只需两个只读文本。任一环取不到就返回 None 交给下一级兜底，
/// **绝不因 `.git` 形态特殊（worktree / submodule / detached）而报错阻断发布**。
fn derive_from_git(repo: &Path) -> Option<String> {
    let gitdir = resolve_gitdir(repo)?;
    let config = fs::read_to_string(gitdir.join("config")).ok()?;
    let remote = parse_git_remote_url(&config)?;
    let branch = parse_git_branch(&gitdir, &config);
    Some(format!("{remote}/raw/{branch}"))
}

/// `.git` 可能是目录（常规仓库），也可能是文本文件 `gitdir: <路径>`（worktree / submodule）。
fn resolve_gitdir(repo: &Path) -> Option<PathBuf> {
    let dot = repo.join(".git");
    if dot.is_dir() {
        return Some(dot);
    }
    let text = fs::read_to_string(&dot).ok()?;
    let path = text.strip_prefix("gitdir:")?.trim();
    if path.is_empty() {
        return None;
    }
    let p = PathBuf::from(path);
    Some(if p.is_absolute() { p } else { repo.join(p) })
}

/// 取 `[remote "origin"]` 段的 `url`（必须限段：多 remote 时全局扫第一个 `url =` 会拿错源）。
fn parse_git_remote_url(config: &str) -> Option<String> {
    let head = "[remote \"origin\"]";
    let start = config.find(head)?;
    let body = &config[start + head.len()..];
    let end = body.find('[').unwrap_or(body.len());
    normalize_remote_url(git_config_value(&body[..end], "url")?.as_str())
}

/// 远程地址 → `https://<host>/<owner>/<repo>`；支持 scp 形态与带 scheme 形态，认不出返回 None。
///
/// 宁可返回 None 也不给出不确定的地址：带非标准端口的 remote（自建 Bitbucket 的 7999 等）
/// 拼出的 raw 前缀极可能是错的，不如退给下一级（条目推导 / 用户手填）。
fn normalize_remote_url(url: &str) -> Option<String> {
    let u = url.trim().trim_end_matches('/').trim_end_matches(".git");
    if u.is_empty() {
        return None;
    }
    match u.split_once("://") {
        // 带 scheme：`https://h/o/r` / `git://h/o/r` / `ssh://git@h/o/r`
        Some((_, rest)) => {
            let cut = rest.find('/')?;
            let authority = &rest[..cut];
            // 去 userinfo（`ssh://git@host/…`）；主机段带非 443 端口则直接放弃
            let host_and_port = authority.rsplit('@').next().unwrap_or(authority);
            if let Some((_, port)) = host_and_port.split_once(':') {
                if port != "443" {
                    return None;
                }
            }
            let host = host_and_port.split(':').next().unwrap_or("");
            let path = rest[cut + 1..].trim_matches('/');
            if host.is_empty() || path.is_empty() {
                return None;
            }
            Some(format!("https://{host}/{path}"))
        }
        // scp 形态 `git@host:owner/repo`（无 `://`，且冒号后不接斜杠）
        None => {
            let at = u.find('@')?;
            let rest = &u[at + 1..];
            let colon = rest.find(':')?;
            if rest[colon + 1..].starts_with('/') {
                return None;
            }
            let host = &rest[..colon];
            let path = rest[colon + 1..].trim_matches('/');
            if !host.contains('.') || path.is_empty() {
                return None;
            }
            Some(format!("https://{host}/{path}"))
        }
    }
}

/// 从 git config 文本取单值键（忽略 section 与键名大小写，取首个命中）。
/// 只服务 `init.defaultBranch` 这类单值键，不必实现完整 ini 解析。
fn git_config_value(text: &str, key: &str) -> Option<String> {
    for line in text.lines() {
        let Some((k, v)) = line.trim().split_once('=') else {
            continue;
        };
        if k.trim().eq_ignore_ascii_case(key) {
            let v = v.trim().trim_matches('"');
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

/// 当前分支：`HEAD` 为 `ref: refs/heads/<branch>` 时取之（分支名可含 `/`，故只削前缀不拆后缀）；
/// detached（裸 SHA）时退 `init.defaultBranch`，再退 `master`（与各托管站默认一致）。
fn parse_git_branch(gitdir: &Path, config: &str) -> String {
    if let Ok(text) = fs::read_to_string(gitdir.join("HEAD")) {
        let t = text.trim();
        if let Some(name) = t.strip_prefix("ref: refs/heads/") {
            let name = name.trim();
            if !name.is_empty() {
                return name.to_string();
            }
        }
        if let Some(name) = t.strip_prefix("ref: ") {
            let name = name.trim();
            if !name.is_empty() {
                return name.to_string();
            }
        }
    }
    git_config_value(config, "defaultBranch").unwrap_or_else(|| "master".to_string())
}

/// 按 **S1 仓库声明 > S2 git 元数据 > S3 既有条目多数派** 推导前缀，返回 `(前缀无末尾斜杠, 来源)`。
/// 手工覆盖不进这条链（优先级最高，由 `plan_publish` 前置判断）。
fn derive_repo_url_base(repo: &Path, existing: &Value) -> Option<(String, &'static str)> {
    // S1：仓库自己声明——最准，且支持 Pages / 自建 CDN 等非 `/raw/` 形态
    if let Some(u) = existing
        .get("urlBase")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty())
    {
        return Some((u, URL_SOURCE_DECLARED));
    }
    // S2：git 元数据（空仓库 / 首次发布也能用，Gitee 与 GitHub 形态一致）
    if let Some(u) = derive_from_git(repo) {
        return Some((u, URL_SOURCE_GIT));
    }
    // S3：既有条目兜底（非 git 工作树、或自定义域名挂了 remote 也对不上时）
    derive_url_base(existing).map(|u| (u.trim_end_matches('/').to_string(), URL_SOURCE_ENTRIES))
}

/// 发布的「形状」：包与封面落地路径 + 前缀及其来源。
/// **预览与实际发布共用**同一套推导，避免弹窗上说的和真正写的不一致。
pub(crate) struct PublishPlan {
    pub(crate) pkg_path: PathBuf,
    pub(crate) cover_name: Option<String>,
    pub(crate) url_base: Option<String>,
    pub(crate) url_source: &'static str,
}

/// 算出发布形状。`pkg_name` 由包文件名决定（B 分支固定 `<id>.vcmirealm`）；
/// `url_base_override` 非空即为用户手填覆盖（只存本机，不写回仓库）。
/// `realm_root` 由调用方传入（发布侧用自参数、预览侧经 `VcmiPaths` 求），
/// 本函数**不得**自己抛路径，否则封接口径会与 `collect_realm_files` 不一致。
fn plan_publish(
    repo: &Path,
    realm_root: &Path,
    id: &str,
    pkg_name: &str,
    realm: &Value,
    url_base_override: Option<&str>,
    existing: &Value,
) -> PublishPlan {
    let manual = url_base_override
        .map(|s| s.trim().trim_end_matches('/').to_string())
        .filter(|s| !s.is_empty());
    let (url_base, url_source) = match manual {
        Some(b) => (Some(b), URL_SOURCE_MANUAL),
        None => match derive_repo_url_base(repo, existing) {
            Some((b, src)) => (Some(b), src),
            None => (None, URL_SOURCE_NONE),
        },
    };
    // 封面名：默认语言 `assets.cover`（回退 `wallpaper`）；实体文件不在就不承诺写封面
    let cover_name = realm
        .get("assets")
        .and_then(|a| a.get("cover").or_else(|| a.get("wallpaper")))
        .and_then(|v| v.as_str())
        .map(|rel| {
            let ext = Path::new(rel)
                .extension()
                .and_then(|e| e.to_str())
                .map(|e| e.to_ascii_lowercase())
                .unwrap_or_else(|| "png".to_string());
            (rel.to_string(), format!("cover.{ext}"))
        })
        .filter(|(rel, _)| {
            realm_root
                .join(rel.replace('/', std::path::MAIN_SEPARATOR_STR))
                .is_file()
        })
        .map(|(_, name)| name);
    PublishPlan {
        pkg_path: repo.join("realms").join(id).join(pkg_name),
        cover_name,
        url_base,
        url_source,
    }
}

/// 在索引顶层插入 `urlBase`，并保持「元信息在前、`realms` 在后」的可读顺序
/// （仓库文件是人手工维护的，位置即文档）。已有该键则不动。
fn insert_url_base(index: &mut Value, url_base: &str) {
    let Some(map) = index.as_object_mut() else {
        return;
    };
    if map.contains_key("urlBase") {
        return;
    }
    let entries: Vec<(String, Value)> = std::mem::take(map).into_iter().collect();
    let mut rebuilt = serde_json::Map::new();
    let mut inserted = false;
    for (k, v) in entries {
        if !inserted && k == "realms" {
            rebuilt.insert("urlBase".to_string(), json!(url_base));
            inserted = true;
        }
        rebuilt.insert(k, v);
    }
    if !inserted {
        rebuilt.insert("urlBase".to_string(), json!(url_base));
    }
    *map = rebuilt;
}

/// 领域数据根目录（`<launcher-data>/realms/<id>/vcmi` 的上一级）。
/// 导出与预览共用，避免两处各自拼路径导致口径不一致。
fn realm_data_root(id: &str) -> Result<PathBuf, String> {
    Ok(VcmiPaths::realm_vcmi_dir(id)
        .ok_or_else(|| be_err("common_data_dir_unknown", ""))?
        .parent()
        .ok_or_else(|| be_err("common_data_dir_unknown", ""))?
        .to_path_buf())
}

/// 降级发布阻断（§7.3）：新版本号低于仓库既有条目时拒绝发布。
/// 返回结构化错误 JSON（`{code, from, to}`），前端按 `code` 映射多语言文案。
///
/// 只读 `index.json` 一个文本；读不动 / 解析失败不在此拦——
/// 发布主流程读到坏索引会**中止**（绝不覆盖坏数据），两步各司其职，
/// 避免把「读失败」误报成「降级」。
fn check_version_downgrade(repo: &Path, id: &str, new_version: &str) -> Result<(), String> {
    let index_file = repo.join("index.json");
    if !index_file.is_file() {
        return Ok(());
    }
    let Ok(text) = fs::read_to_string(&index_file) else {
        return Ok(());
    };
    let Ok(existing) = crate::infra::json::parse_json_lenient(&text) else {
        return Ok(());
    };
    match downgrade_error(&existing, id, new_version) {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// 纯函数：既有索引 vs 新版本 → 降级错误（含 from/to），便于单测。
/// 既有条目缺 version（手工条目）时放行：没有比较基准就不拦截（宁缺勿错）。
fn downgrade_error(existing: &Value, id: &str, new_version: &str) -> Option<String> {
    let prev = existing
        .get("realms")
        .and_then(|r| r.get(id))
        .and_then(|e| e.get("version"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    if crate::infra::version::compare(new_version, prev) == std::cmp::Ordering::Less {
        return Some(
            json!({ "code": "version_downgrade", "from": prev, "to": new_version }).to_string(),
        );
    }
    None
}

/// 预览「发布到领域仓库」会写什么（弹窗在点导出**之前**就要把地址与落地路径显示给用户确认）。
/// 只读本地小文件（`index.json` + `.git` 两个文本），无网络、无写入，故保持同步命令。
#[tauri::command]
pub fn preview_realm_publish(
    repo_dir: String,
    realm_id: String,
    url_base: Option<String>,
) -> Result<Value, String> {
    let repo = PathBuf::from(repo_dir.trim());
    if repo_dir.trim().is_empty() {
        return Err(be_err("realm_publish_no_repo", ""));
    }
    if !repo.is_dir() {
        return Err(be_err(
            "realm_repo_dir_missing",
            &format!("{}", repo.display()),
        ));
    }
    let id = VcmiPaths::sanitize_realm_id(&realm_id);
    let realm = get_realm(&id)?;
    let index_file = repo.join("index.json");
    let mut index_exists = false;
    let mut index_parse_ok = true;
    let mut existing = Value::Null;
    if index_file.is_file() {
        index_exists = true;
        match fs::read_to_string(&index_file) {
            Ok(text) => {
                existing = crate::infra::json::parse_json_lenient(&text).unwrap_or(Value::Null);
                if existing.is_null() {
                    index_parse_ok = false;
                }
            }
            Err(_) => index_parse_ok = false,
        }
    }
    // 版本变化（弹窗据此禁用降级发布）：新版本取 realm.json（与 entry 构造同口径），
    // 既有条目版本来自索引；包是否变化不在此判定（预览时新包尚未打包，sha 不可知）。
    let next_version = realm
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("1.0.0")
        .to_string();
    let prev_version = existing
        .get("realms")
        .and_then(|r| r.get(&id))
        .and_then(|e| e.get("version"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let version_rel = match &prev_version {
        None => "none",
        Some(pv) => match crate::infra::version::compare(&next_version, pv) {
            std::cmp::Ordering::Less => "down",
            std::cmp::Ordering::Greater => "up",
            std::cmp::Ordering::Equal => "same",
        },
    };
    let plan = plan_publish(
        &repo,
        &realm_data_root(&id)?,
        &id,
        &format!("{id}.vcmirealm"),
        &realm,
        url_base.as_deref(),
        &existing,
    );
    let download = plan.url_base.as_ref().map(|b| {
        format!(
            "{b}/realms/{id}/{}",
            plan.pkg_path
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
        )
    });
    // 分片预估（设计 §7.3）：只做 metadata 遍历求字节数（**不读文件内容**），仍是快操作。
    // 0 = 预计不分包。实际片数以打包后为准（ZIP 压缩会改变最终体积），故弹窗仅作提示。
    let estimated_bytes = estimate_realm_bytes(&id);
    let estimated_part_count = if estimated_bytes > REALM_PART_MAX_BYTES {
        ((estimated_bytes + REALM_PART_MAX_BYTES - 1) / REALM_PART_MAX_BYTES) as usize
    } else {
        0
    };
    Ok(json!({
        "repoDir": repo.to_string_lossy(),
        "indexExists": index_exists,
        "indexParseOk": index_parse_ok,
        "pkgPath": plan.pkg_path.to_string_lossy(),
        "coverName": plan.cover_name,
        "urlBase": plan.url_base.map(|b| format!("{b}/")),
        "urlSource": plan.url_source,
        "downloadUrl": download,
        "prevVersion": prev_version,
        "nextVersion": next_version,
        "versionRel": version_rel,
        "entryExists": existing.get("realms").and_then(|r| r.get(&id)).is_some(),
        "estimatedPartCount": estimated_part_count,
        "partMaxBytes": REALM_PART_MAX_BYTES,
    }))
}

/// 领域待打包内容的字节数（用于分片预估）：与导出同源地收集文件（**只读 metadata**），
/// 保证「预览说的」与「打包做的」是同一份清单。任何一步失败都返回 0（预估可缺，不可错）。
fn estimate_realm_bytes(id: &str) -> u64 {
    let Ok(root) = realm_data_root(id) else {
        return 0;
    };
    let mut files: Vec<(String, PathBuf)> = Vec::new();
    if collect_realm_files(&root, &mut files).is_err() {
        return 0;
    }
    files
        .iter()
        .map(|(_, p)| fs::metadata(p).map(|m| m.len()).unwrap_or(0))
        .sum()
}

/// 索引条目里「可从包与产物推导」的字段；其余（如 `gameEdition`）属发布者手工字段，合入时保留。
/// 注意：可推导 ≠ 无条件覆盖——空值（`null` / 空数组 / 空串）视为「未声明」，不动既有值。
pub(crate) const DERIVED_INDEX_FIELDS: &[&str] = &[
    "version",
    "author",
    "defaultLang",
    "name",
    "description",
    "tags",
    "i18n",
    "requiresMods",
    "sha256",
    "size",
    "download",
    "cover",
    "parts",
];

/// 把导出条目合入仓库 `index.json`：**只覆盖可推导字段**，保留手工字段与既有条目顺序。
/// `entry` 里缺失的键（如本次没封面 → 无 `cover`）与「空值」（构造层把“没算出来”物化成的
/// `null` / 空数组 / 空串）一律不动既有值；`i18n` 按「语言级 + 块内字段级」两级合并。
/// **`tags` 例外**：键存在即声明——数组（含空数组 = 显式清空）照常覆盖，缺键才保留既有值
/// （录入功能上线后 tags 的唯一来源是 realm.json，作者删光标签必须能在索引侧生效）。
/// 返回 `(新索引, 发生变化的字段名)`；纯函数，便于单测。
pub(crate) fn merge_repo_index(
    existing: &Value,
    id: &str,
    entry: &Value,
    updated_at: &str,
) -> (Value, Vec<String>) {
    let mut realms = existing
        .get("realms")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let mut target = realms
        .get(id)
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();
    let mut changed: Vec<String> = Vec::new();
    for key in DERIVED_INDEX_FIELDS {
        let Some(new) = entry.get(*key) else { continue };
        // 「没算出来」≠「清空仓库」：空值不动既有值也不计入 changed——否则 realm.json 未配
        // author / description 时，会把索引里手工维护的值抹掉（仓库索引规范 §9.2）。
        // `tags` 例外：键存在即声明——数组（含空）照常覆盖；构造层对「缺键」已不写出，
        // 故空数组只可能来自编辑器里作者删光标签（显式清空）。
        let explicit_tags = *key == "tags" && new.is_array();
        if is_blank(new) && !explicit_tags {
            continue;
        }
        if *key == "i18n" {
            // i18n 两级合并：语言级（其余语言 / 手工加译保留）+ 块内字段级（name / description /
            // tags 逐键，「没声明」的键保留旧值；块内 tags 数组含空即声明）。
            let Some(new_map) = new.as_object() else {
                continue;
            };
            let Some(old_map) = target.get("i18n").and_then(|v| v.as_object()).cloned() else {
                target.insert((*key).to_string(), new.clone());
                continue;
            };
            let mut next = old_map.clone();
            for (lang, new_block) in new_map {
                let merged_block = match new_block.as_object() {
                    Some(nb) => {
                        let mut mb = old_map
                            .get(lang)
                            .and_then(|v| v.as_object())
                            .cloned()
                            .unwrap_or_default();
                        for (k, v) in nb {
                            // 块内 `tags` 与顶层同口径：数组（含空）即声明；其余键维持空值保护
                            if !is_blank(v) || (k == "tags" && v.is_array()) {
                                mb.insert(k.clone(), v.clone());
                            }
                        }
                        if mb.is_empty() {
                            continue; // 整块都是未声明值 → 不新增该语言
                        }
                        Value::Object(mb)
                    }
                    None => {
                        if is_blank(new_block) {
                            continue;
                        }
                        new_block.clone()
                    }
                };
                next.insert(lang.clone(), merged_block);
            }
            if next != old_map {
                changed.push((*key).to_string());
                target.insert((*key).to_string(), Value::Object(next));
            }
            continue;
        }
        let old = target.get(*key);
        if old == Some(new) {
            continue;
        }
        if old.is_some() {
            changed.push((*key).to_string());
        }
        target.insert((*key).to_string(), new.clone());
    }
    // `parts` / `download` 是**互斥且完全派生**的一对：由本次发布产物决定，不存在手工维护值。
    // 新条目没有的那一侧必须从既有条目删除，否则「单包 → 分片 → 单包」来回切换后索引会
    // 残留矛盾字段——残留 `download` 会让新启动器走单包分支（文件已删，必然 404）；
    // 残留 `parts` 会让旧启动器报「请升级」而实际早已回退成单包（设计 §7.2）。
    let sharded = entry
        .get("parts")
        .and_then(|v| v.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false);
    if sharded {
        if target.remove("download").is_some() {
            push_changed(&mut changed, "download");
        }
    } else if target.remove("parts").is_some() {
        push_changed(&mut changed, "parts");
    }
    realms.insert(id.to_string(), Value::Object(target));
    let mut out = existing.as_object().cloned().unwrap_or_default();
    if !out.contains_key("schemaVersion") {
        out.insert("schemaVersion".to_string(), json!(1));
    }
    out.insert("updatedAt".to_string(), json!(updated_at));
    out.insert("realms".to_string(), Value::Object(realms));
    (Value::Object(out), changed)
}

/// 记录变更字段（去重）：删除也算子段变化，`entryState` 的三态判定依赖它。
fn push_changed(changed: &mut Vec<String>, key: &str) {
    if !changed.iter().any(|c| c == key) {
        changed.push(key.to_string());
    }
}

/// 空值判定：`null` / 空数组 / 空串（含纯空白）/ 空对象 → 一律视为「未声明」。
/// 「未声明」不清空既有值——仓库索引里的手工字段与自定义扩展字段不应被无配置的发布抹掉。
fn is_blank(v: &Value) -> bool {
    match v {
        Value::Null => true,
        Value::String(s) => s.trim().is_empty(),
        Value::Array(a) => a.is_empty(),
        Value::Object(o) => o.is_empty(),
        _ => false,
    }
}

/// 构建包内 `realm.json` 字节：追加完整性校验和，并剥离启动器私有字段。
/// `origin` 是本机安装来源记录（远程安装时由 `repository::download` 写入 realm.json），
/// **不属于领域定义**——打进包会让接收方（含其他启动器）看到指向导出方安装源的无效块。
fn build_package_realm_json(realm: &Value, checksum: &str) -> Result<Vec<u8>, String> {
    let mut realm_json = realm.clone();
    if let Some(obj) = realm_json.as_object_mut() {
        obj.remove("origin");
        obj.insert("checksum".to_string(), json!(checksum));
    }
    serde_json::to_vec_pretty(&realm_json)
        .map_err(|e| be_err("realm_json_serialize_failed", &format!("{}", e)))
}

/// 直写本地领域仓库工作树（§7.3）：包副本 → 封面 → 合并 `index.json`。
///
/// 约定与 `vcmi-fv-launcher-realms` 现状一致：包与封面进 `realms/<id>/`，`download` / `cover`
/// 走仓库 raw 地址。返回 `(发布信息, 告警)`；告警不阻断（红/黄级由前端按 code 定色）。
fn publish_realm_to_repo(
    realm: &Value,
    realm_root: &Path,
    id: &str,
    dest: &Path,
    repo: &Path,
    url_base: Option<&str>,
    entry: &mut Value,
    pkg_size: u64,
) -> Result<(Value, Vec<Value>), String> {
    let mut warnings: Vec<Value> = Vec::new();
    if !repo.is_dir() {
        return Err(be_err(
            "realm_repo_dir_missing",
            &format!("{}", repo.display()),
        ));
    }
    let index_file = repo.join("index.json");
    // 既有索引：存在但解析失败 → **中止**（绝不把坏数据覆盖掉）；不存在 → 新建
    let existing = if index_file.is_file() {
        let text = fs::read_to_string(&index_file)
            .map_err(|e| be_err("realm_repo_index_read_failed", &format!("{}", e)))?;
        crate::infra::json::parse_json_lenient(&text)
            .map_err(|_| be_err("realm_repo_index_parse_abort", ""))?
    } else {
        Value::Null
    };
    // 降级阻断（双保险：`export_realm_impl` 已在打包前拦过一次——发布分支的 dest 就是
    // 本仓库包路径，若等合并索引才发现，包已被覆盖而索引未更新）。脚本调用若绕过前者，在此兜住。
    if let Some(e) = downgrade_error(
        &existing,
        id,
        entry.get("version").and_then(|v| v.as_str()).unwrap_or(""),
    ) {
        return Err(e);
    }

    // 1) 发布形状（与 `preview_realm_publish` 同源，保证弹窗预览与实际写入一致）
    let dir = repo.join("realms").join(id);
    fs::create_dir_all(&dir)
        .map_err(|e| be_err("realm_repo_dir_create_failed", &format!("{}", e)))?;
    let pkg_name = dest
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| format!("{id}.vcmirealm"));
    let plan = plan_publish(repo, realm_root, id, &pkg_name, realm, url_base, &existing);
    let pkg_path = plan.pkg_path.clone();
    // 导出路径本就在仓库内时就是它本身（零额外复制）；不在则复制一份并告警
    if !same_path(dest, &pkg_path) {
        fs::copy(dest, &pkg_path)
            .map_err(|e| be_err("realm_repo_pack_copy_failed", &format!("{}", e)))?;
        warnings.push(warn("copied-into-repo", &pkg_path.display().to_string()));
    }

    // 1.5) 分片：包超过单文件容忍线（45 MiB）时切成固定大小的分片落库。
    //
    // 放在**这里**切而不是「打包完立刻切」的唯一理由：分片 URL 需要 `urlBase`，
    // 而它要到 `plan_publish` 才定下来——给不出地址就不能切，否则仓库会留下
    // 「单包已删、索引无 parts」的死状态（设计 §7.1）。
    let needs_shard = pkg_size > REALM_PART_MAX_BYTES;
    let mut parts: Vec<RealmPart> = Vec::new();
    if needs_shard {
        match &plan.url_base {
            Some(b) => match split_realm_package(&pkg_path, REALM_PART_MAX_BYTES) {
                Ok(p) => {
                    parts = p;
                    // 整包与分片二选一落库：分片已就位，单包即成陈旧文件
                    let _ = fs::remove_file(&pkg_path);
                    prune_stale_parts(&dir, id, parts.len());
                    entry["parts"] = json!(parts
                        .iter()
                        .map(|p| json!({
                            "url": format!("{b}/realms/{id}/{}", p.name),
                            "size": p.size,
                            "sha256": p.sha256,
                        }))
                        .collect::<Vec<_>>());
                    // 分片条目不带 `download`（两者互斥，见 `merge_repo_index`）
                    if let Some(o) = entry.as_object_mut() {
                        o.remove("download");
                    }
                }
                Err(_) => {
                    // 切分失败：单包原样留在仓库，索引走单包路径。
                    // 宁可「不分片」，也绝不留「单包已删、分片残缺」。
                    prune_stale_parts(&dir, id, 0);
                    warnings.push(warn("shard-failed", ""));
                }
            },
            None => {
                // 推不出 URL 前缀 ⇒ 给不出分片地址，只能保留单包（超限告警照常触发）
                prune_stale_parts(&dir, id, 0);
                warnings.push(warn("shard-needs-url-base", &fmt_mb(pkg_size)));
            }
        }
    } else {
        // 未超阈值：仓库里若残留上次的分片必须清掉（索引只会列单包）
        prune_stale_parts(&dir, id, 0);
    }
    let sharded = !parts.is_empty();

    // 2) 下载地址：分片条目不写 `download`（`parts` 是其唯一来源）；单包沿用既有推导链
    //    前缀由 plan 给出（手工覆盖 > 声明 > git > 条目）；全推不出则沿用既有条目并告警
    if !sharded {
        match &plan.url_base {
            Some(b) => {
                entry["download"] = json!(format!("{b}/realms/{id}/{pkg_name}"));
            }
            None => {
                warnings.push(warn("no-url-base", ""));
                if let Some(prev) = existing
                    .get("realms")
                    .and_then(|r| r.get(id))
                    .and_then(|e| e.get("download"))
                {
                    entry["download"] = prev.clone(); // 沿用既有条目，不致把地址写空
                }
            }
        }
    }

    // 3) 封面：默认语言的 `assets.cover`（回退 `wallpaper`）落 `realms/<id>/cover.<ext>`。
    //    没配封面就不写 `cover` 字段 —— 合入时保留既有条目的封面值。
    let mut cover_path: Option<PathBuf> = None;
    if let Some(cover_name) = &plan.cover_name {
        let rel = realm
            .get("assets")
            .and_then(|a| a.get("cover").or_else(|| a.get("wallpaper")))
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let src = realm_root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        if src.is_file() {
            let dst = dir.join(cover_name);
            fs::copy(&src, &dst)
                .map_err(|e| be_err("realm_cover_copy_failed", &format!("{}", e)))?;
            if let Some(b) = &plan.url_base {
                entry["cover"] = json!(format!("{b}/realms/{id}/{cover_name}"));
            }
            cover_path = Some(dst);
        }
    }

    // 4) 发布前自检（与 `scripts/build-realms-index.mjs` 的告警清单同源）
    if sharded {
        // 分片后不再受单文件上限约束，`over-*` 两条体积告警语义失效：
        // 只提示「已分片」与（超 100 MB 时的）仓库体积增长——分片全部进 git 历史。
        warnings.push(warn(
            "sharded",
            &format!("{} × ≤{}", parts.len(), fmt_mb(REALM_PART_MAX_BYTES)),
        ));
        if pkg_size > PUBLISH_DOWNLOAD_MAX {
            warnings.push(warn("over-total-hint", &fmt_mb(pkg_size)));
        }
    } else {
        if pkg_size > PUBLISH_DOWNLOAD_MAX {
            warnings.push(warn("over-download-max", &fmt_mb(pkg_size)));
        } else if pkg_size > PUBLISH_GIT_MAX {
            warnings.push(warn("over-git-max", &fmt_mb(pkg_size)));
        }
    }
    let bundled: Vec<String> = realm
        .get("mods")
        .and_then(|m| m.get("bundled"))
        .and_then(|b| b.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    if !bundled.is_empty() {
        warnings.push(warn("bundled", &bundled.join("、")));
    }
    if id.is_empty()
        || !id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        warnings.push(warn("bad-id", id));
    }
    let prev = existing.get("realms").and_then(|r| r.get(id));
    // 版本变更自检（降级已在 `downgrade_error` 上游阻断，不在此列）
    warnings.extend(version_change_warnings(prev, entry));

    // 5) 合并写盘：2 空格缩进 + 末尾换行（与 `scripts/build-realms-index.mjs` 同口径：都是 pretty + 末尾换行，
    //    且只覆盖不重排已有字段）
    let (mut merged, changed) = merge_repo_index(&existing, id, entry, &now_date());
    // 条目动作三态必须在合并后定：完全重复的发布（changed 为空）报「已更新」会与结果态底部
    // 「字段无变化」自相矛盾，故单列为 unchanged。
    let entry_state = if prev.is_none() {
        "created"
    } else if changed.is_empty() {
        "unchanged"
    } else {
        "updated"
    };
    // 自举仅在「本次靠推导拿到前缀」且「仓库原本没声明 urlBase」时发生
    let bootstrapped = matches!(plan.url_source, URL_SOURCE_GIT | URL_SOURCE_ENTRIES)
        && plan.url_base.is_some()
        && existing
            .get("urlBase")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .trim()
            .is_empty();

    // 6) S4 自举：由 git / 条目推导出的前缀回写进 `index.json` 顶层 `urlBase`。
    //    之后本仓库的发布与协作者都无需再猜，而这行地址进 git diff 可审计。
    //    用户手填的覆盖值只存本机 `launcher.json`，**不写回仓库**（那是个人环境，不是仓库事实）。
    if bootstrapped {
        if let Some(b) = &plan.url_base {
            insert_url_base(&mut merged, &format!("{b}/"));
        }
    }
    fs::write(
        &index_file,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&merged)
                .map_err(|e| be_err("realm_repo_index_serialize_failed", &format!("{}", e)))?
        ),
    )
    .map_err(|e| be_err("realm_repo_index_write_failed", &format!("{}", e)))?;

    Ok((
        json!({
            "repoDir": repo.to_string_lossy(),
            "pkgPath": pkg_path.to_string_lossy(),
            "coverPath": cover_path.map(|p| p.to_string_lossy().into_owned()),
            "indexPath": index_file.to_string_lossy(),
            "entryState": entry_state,
            "changedFields": changed,
            "urlSource": plan.url_source,
            "urlBootstrapped": bootstrapped,
            // 分片产物（0 = 未分片）。路径由后端给，前端不自行拼文件名
            // （延续「不抄仓库布局约定」，见 `willWriteDir` 的由来）
            "partCount": parts.len(),
            "parts": parts
                .iter()
                .map(|p| json!({
                    "path": dir.join(&p.name).to_string_lossy(),
                    "size": p.size,
                    "sha256": p.sha256,
                }))
                .collect::<Vec<_>>(),
        }),
        warnings,
    ))
}

/// 路径等价判定（Windows 大小写不敏感）：用于决定是否需要往仓库复制一份包。
fn same_path(a: &Path, b: &Path) -> bool {
    let ka = fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
    let kb = fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    let sa = ka.to_string_lossy().to_ascii_lowercase().replace('\\', "/");
    let sb = kb.to_string_lossy().to_ascii_lowercase().replace('\\', "/");
    sa == sb
}

/// 构造结构化错误：前端 `catch` 后 `JSON.parse` 可得到 `{ code, msg }`。
pub(crate) fn import_err(code: &str, msg: &str) -> String {
    json!({ "code": code, "msg": msg }).to_string()
}

/// id 冲突错误：携带冲突领域的 id / name，供前端弹窗选择覆盖 / 重命名 / 取消。
pub(crate) fn conflict_err(id: &str, name: &str) -> String {
    json!({ "code": "id_conflict", "realmId": id, "realmName": name }).to_string()
}

/// 归一化 ZIP 内条目名：拒绝绝对路径 / `..` 穿越；仅允许解包到 `vcmi/` 或 `assets/`。
/// 返回包内相对路径（正斜杠），不安全返回 None（跳过）。
pub(crate) fn normalize_zip_path(name: &str) -> Option<String> {
    let rel = name.trim_start_matches('/');
    if rel.is_empty() || rel.starts_with("..") || rel.contains("../") || rel.contains('\\') {
        return None;
    }
    if rel == "realm.json" {
        return None; // 元数据单独解析，不解包
    }
    // `showcase-cache/`：领域快照附带的**素材缓存**（可选），解包目标不是领域目录而是全局缓存根，
    // 由导入循环分流处理（见 `import_realm` 第二遍）。
    if rel.starts_with("vcmi/") || rel.starts_with("assets/") || rel.starts_with("showcase-cache/")
    {
        Some(rel.to_string())
    } else {
        None // 其它顶层项（cache/logs/Saves 等）不导入
    }
}

/// 在 id 冲突时生成不重复的新 id（追加 `-2`、`-3`…），判重只看领域目录是否存在。
pub(crate) fn unique_realm_id(base: &str) -> String {
    let mut n = 2;
    loop {
        let candidate = format!("{base}-{n}");
        if !realm_id_exists(&candidate) {
            return candidate;
        }
        n += 1;
    }
}

/// 导入领域快照（§7.2）：读取 `.vcmirealm`（ZIP），校验版本 / 完整性，解包到领域目录，
/// 合并进 `launcher.json`，并直接写入 `__realm__` 预设；校验借用模组可用性。
/// 解析领域多语言字段（name / description 等）。
///
/// 回退链与前端 `resolveRealm` 保持一致：根级字符串优先；否则按
/// `i18n[ui_lang]` → `i18n[fallbackLang]`（tchinese→chinese，其余→english）
/// → `i18n.english` → 任意有该字段的语言 取值。
/// 这样即使 `realm.json` 只在 `i18n` 块里写了 name（无根级 name），
/// 导入校验与本地索引构建也能取到正确值。
pub(crate) fn resolve_realm_i18n_field(realm: &Value, field: &str) -> Option<String> {
    if let Some(s) = realm.get(field).and_then(|v| v.as_str()) {
        if !s.trim().is_empty() {
            return Some(s.to_string());
        }
    }
    let i18n = realm.get("i18n").and_then(|v| v.as_object())?;
    let ui_lang_val = read_config().and_then(|c| c.ui.lang);
    let ui_lang = ui_lang_val.as_deref().unwrap_or("english");
    let fallback = if ui_lang == "tchinese" {
        "chinese"
    } else {
        "english"
    };
    let mut order: Vec<&str> = vec![ui_lang, fallback, "english"];
    for (l, _) in i18n {
        if l.as_str() != ui_lang && l.as_str() != fallback {
            order.push(l.as_str());
        }
    }
    for l in order {
        if let Some(block) = i18n.get(l).and_then(|v| v.as_object()) {
            if let Some(s) = block.get(field).and_then(|v| v.as_str()) {
                if !s.trim().is_empty() {
                    return Some(s.to_string());
                }
            }
        }
    }
    None
}

/// `conflict` ∈ {"error"(默认，冲突则回传 id_conflict), "overwrite", "rename"}。
///
/// P8：解包大领域包是长阻塞 IO（Tauri 同步命令跑在主线程会冻结窗口）——
/// 命令壳只在主线程读路径（快 IO），主体移交 `spawn_blocking`。
#[tauri::command]
pub async fn import_realm(
    state: State<'_, AppPaths>,
    path: String,
    conflict: String,
) -> Result<Value, String> {
    let paths = state.0.lock().expect("paths lock poisoned").clone();
    tauri::async_runtime::spawn_blocking(move || import_realm_impl(paths, path, conflict))
        .await
        .map_err(|e| be_err("realm_import_task_failed", &format!("{}", e)))?
}

/// 导入主体（阻塞线程池执行）：paths 由命令壳预先 clone 交出，不再碰 State。
pub fn import_realm_impl(
    paths: VcmiPaths,
    path: String,
    conflict: String,
) -> Result<Value, String> {
    let file =
        fs::File::open(&path).map_err(|e| be_err("realm_file_open_failed", &format!("{}", e)))?;
    let mut archive =
        ZipArchive::new(file).map_err(|_| import_err("corrupt", "领域文件不是有效的压缩包"))?;

    // 包格式识别：ZIP 注释须为 `vcmirealm`（导出时写入的包格式魔数），用于快速识别合法包
    let comment = String::from_utf8_lossy(archive.comment());
    if comment.trim() != "vcmirealm" {
        return Err(import_err(
            "corrupt",
            "领域文件已损坏（不是有效的 .vcmirealm 包）",
        ));
    }

    // 第一遍：读取 realm.json + 收集非 realm.json 的文件条目（index, rel）
    let mut realm_bytes: Option<Vec<u8>> = None;
    let mut files: Vec<(usize, String)> = Vec::new();
    for i in 0..archive.len() {
        let mut f = archive
            .by_index(i)
            .map_err(|e| be_err("realm_archive_read_failed", &format!("{}", e)))?;
        if f.name() == "realm.json" {
            let mut buf = Vec::new();
            f.read_to_end(&mut buf)
                .map_err(|e| be_err("realm_json_read_failed", &format!("{}", e)))?;
            realm_bytes = Some(buf);
        } else if f.is_file() {
            if let Some(rel) = normalize_zip_path(f.name()) {
                files.push((i, rel));
            }
        }
    }

    let realm_bytes =
        realm_bytes.ok_or_else(|| import_err("corrupt", "领域文件已损坏（缺少 realm.json）"))?;
    let mut realm: Value = serde_json::from_slice(&realm_bytes)
        .map_err(|_| import_err("corrupt", "领域文件已损坏（realm.json 解析失败）"))?;
    let id = realm
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| import_err("corrupt", "领域文件已损坏（缺少 id）"))?;
    // 名称允许只写在 `i18n` 块里（无根级 name）：回退链与前端 resolveRealm 一致
    // （根级 → i18n[ui_lang] → fallback → english → 任意有值语言）。
    let name = resolve_realm_i18n_field(&realm, "name")
        .ok_or_else(|| import_err("corrupt", "领域文件已损坏（缺少 name）"))?;
    let checksum_expected = realm
        .get("checksum")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let id = VcmiPaths::sanitize_realm_id(id);
    let name_display = name.to_string();

    // id 冲突检查：领域目录是否已存在（§3.1）
    let resolved_id = if realm_id_exists(&id) {
        match conflict.as_str() {
            "overwrite" => id.clone(),
            "rename" => unique_realm_id(&id),
            _ => return Err(conflict_err(&id, &name_display)),
        }
    } else {
        id.clone()
    };

    let root = VcmiPaths::realm_vcmi_dir(&resolved_id)
        .ok_or_else(|| be_err("common_data_dir_unknown", ""))?
        .parent()
        .ok_or_else(|| be_err("common_data_dir_unknown", ""))?
        .to_path_buf();

    // overwrite 时先清理旧目录（安全删除：先摘借用联接，避免误删玩家真实模组）
    if conflict == "overwrite" && root.is_dir() {
        remove_realm_dir_safely(&resolved_id)?;
    }
    fs::create_dir_all(&root).map_err(|e| be_err("realm_dir_create_failed", &format!("{}", e)))?;

    // 第二遍：按相对路径排序后逐条读出、写盘、并流式求 SHA-256
    files.sort_by(|a, b| a.1.cmp(&b.1));
    let mut hasher = Sha256::new();
    for (i, rel) in &files {
        let mut f = archive
            .by_index(*i)
            .map_err(|e| be_err("realm_archive_read_failed", &format!("{}", e)))?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)
            .map_err(|e| be_err("realm_archive_read_failed", &format!("{}", e)))?;
        hasher.update(rel.as_bytes());
        hasher.update(&buf);
        // 分流：素材缓存解到**全局缓存根**（而非领域目录），且**只垫底不覆盖**
        // ——保护用户本地已构建（可能更新）的缓存；失败不阻断导入（缓存只是加速项）。
        if let Some(rest) = rel.strip_prefix("showcase-cache/") {
            if let Some(cache_root) = VcmiPaths::mods_cache_dir() {
                let dest = cache_root.join(rest.replace('/', std::path::MAIN_SEPARATOR_STR));
                if !dest.exists() {
                    if let Some(parent) = dest.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    let _ = fs::write(&dest, &buf);
                }
            }
            continue;
        }
        let dest = root.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| be_err("cmd_mkdir_failed", &format!("{}", e)))?;
        }
        fs::write(&dest, &buf).map_err(|e| be_err("realm_file_write_failed", &format!("{}", e)))?;
    }
    let checksum = crate::infra::hash::to_hex(&hasher.finalize());
    if !checksum_expected.is_empty() && checksum != checksum_expected {
        let _ = remove_realm_dir_safely(&resolved_id); // 回滚
        return Err(import_err("corrupt", "领域文件已损坏（校验和不匹配）"));
    }

    // 确保骨架目录（cache/logs/Saves/Mods）存在
    ensure_realm_dirs(&resolved_id)?;

    // 写入领域元数据到 `{realm-dir}/realm.json`（§3.1：目录即领域）。
    // 移除包专用字段、更新 id。导入领域**不写 lock** → 只读（§6.2）。
    if let Some(obj) = realm.as_object_mut() {
        obj.remove("checksum");
        obj.insert("id".to_string(), json!(resolved_id));
        if !obj.contains_key("createdAt") {
            obj.insert("createdAt".to_string(), json!(now_date()));
        }
        obj.insert("updateAt".to_string(), json!(now_date()));
        if !obj.contains_key("presetId") {
            obj.insert("presetId".to_string(), json!(REALM_PRESET_ID));
        }
    }
    write_realm_meta(&resolved_id, &realm)?;
    // 地图清单以解包后的目录为准，同步进 realm.json（§6）
    sync_realm_maps_meta(&resolved_id);

    // 直接生成 __realm__ 预设（§7.2：领域拥有独立配置目录，预设由启动器写入）
    write_realm_preset(&resolved_id)?;

    // 校验借用模组可用性（bundled 已随包解包，必然可用）
    let mut missing_mods: Vec<String> = Vec::new();
    let borrowed = realm
        .get("mods")
        .and_then(|m| m.get("borrowed"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if !borrowed.is_empty() {
        let mut installed: std::collections::HashSet<String> = std::collections::HashSet::new();
        for m in scan_user_mods_dir(&paths.user_mods_dir())
            .into_iter()
            .chain(scan_user_mods_dir(&paths.game_mods_dir()))
        {
            if let Some(s) = m.get("id").and_then(|v| v.as_str()) {
                installed.insert(s.to_string());
            }
        }
        for b in borrowed.iter() {
            if let Some(s) = b.as_str() {
                if !installed.contains(s) {
                    missing_mods.push(s.to_string());
                }
            }
        }
    }

    // 整合模组（bundled）已随包解包到 `{realm-dir}/vcmi/Mods/<modId>`：
    // 它们**不在全局已安装模组列表**里，`.vcmi-fv-launcher/mods` 百科缓存也没有其资料
    // ——领域信息页会因此拿不到模组描述、整合的城镇也没有预览图。
    // 这里把 modId + 目录一并返回，由前端触发批量构建，把资料分析进百科缓存。
    let mut bundled_mods: Vec<Value> = Vec::new();
    if let Some(bundled) = realm
        .get("mods")
        .and_then(|m| m.get("bundled"))
        .and_then(|v| v.as_array())
    {
        for b in bundled.iter() {
            if let Some(mid) = b.as_str() {
                let p = root.join("vcmi").join("Mods").join(mid);
                if p.is_dir() {
                    bundled_mods.push(json!({ "modId": mid, "modPath": p.to_string_lossy() }));
                }
            }
        }
    }

    // 同名校验（§9.3.3）：百科缓存按 `modId` 键控，整合模组若与本机已安装模组**重名**，
    // 两者会共用同一份缓存（谁后构建谁覆盖），资料会张冠李戴。导入不阻断，仅回传供前端提示。
    let mut duplicate_mods: Vec<String> = Vec::new();
    if !bundled_mods.is_empty() {
        let mut installed: std::collections::HashSet<String> = std::collections::HashSet::new();
        for m in scan_user_mods_dir(&paths.user_mods_dir())
            .into_iter()
            .chain(scan_user_mods_dir(&paths.game_mods_dir()))
        {
            if let Some(s) = m.get("id").and_then(|v| v.as_str()) {
                installed.insert(s.to_string());
            }
        }
        for b in bundled_mods.iter() {
            if let Some(mid) = b.get("modId").and_then(|v| v.as_str()) {
                if installed.contains(mid) && !duplicate_mods.iter().any(|x| x == mid) {
                    duplicate_mods.push(mid.to_string());
                }
            }
        }
    }

    Ok(json!({
        "ok": true,
        "realmId": resolved_id,
        "realmName": name_display,
        "needsMods": !missing_mods.is_empty(),
        "missingMods": missing_mods,
        "bundledMods": bundled_mods,
        "duplicateMods": duplicate_mods,
    }))
}

#[cfg(test)]
mod tests {
    use super::{
        build_package_realm_json, derive_repo_url_base, derive_url_base, downgrade_error,
        insert_url_base, is_core_mod_id, is_precompressed, merge_repo_index, normalize_remote_url,
        parse_git_branch, parse_git_remote_url, part_name, prune_stale_parts,
        publish_realm_to_repo, resolve_gitdir, split_realm_package, version_change_warnings,
        write_zip_entries, PackOutcome, DERIVED_INDEX_FIELDS, REALM_PART_MAX_BYTES,
        URL_SOURCE_DECLARED, URL_SOURCE_ENTRIES, URL_SOURCE_GIT,
    };
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};
    use zip::ZipWriter;

    /// 每个测试独占一个临时目录（名字带测试名，避免并行互踩）。
    fn sandbox(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("vcmi-fv-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn core_pseudo_mods_are_detected_case_insensitively() {
        assert!(is_core_mod_id("core"));
        assert!(is_core_mod_id("Core.Castle"));
        assert!(is_core_mod_id("core.conflux"));
        // 普通第三方 mod（即便名字里带 core）不算原版
        assert!(!is_core_mod_id("abyss-town"));
        assert!(!is_core_mod_id("my-core-overhaul"));
    }

    #[test]
    fn merge_keeps_manual_fields_and_reports_changed_keys() {
        let existing = json!({
            "schemaVersion": 1,
            "updatedAt": "2020-01-01",
            "realms": {
                "demo": { "version": "1.0.0", "sha256": "old", "size": 1, "gameEdition": "enhanced" },
                "other": { "version": "9.9.9" }
            }
        });
        let entry = json!({
            "version": "1.1.0",
            "sha256": "new",
            "size": 2,
            "download": "https://x/raw/master/realms/demo/demo.vcmirealm",
            "requiresMods": ["m1"],
        });
        let (merged, changed) = merge_repo_index(&existing, "demo", &entry, "2026-09-28");
        let demo = &merged["realms"]["demo"];
        assert_eq!(demo["version"], json!("1.1.0"));
        assert_eq!(demo["sha256"], json!("new"));
        // 手工字段（gameEdition）与未参与推导的条目必须原样保留
        assert_eq!(demo["gameEdition"], json!("enhanced"));
        assert_eq!(merged["realms"]["other"]["version"], json!("9.9.9"));
        assert_eq!(merged["updatedAt"], json!("2026-09-28"));
        // 单包发布：既有条目若残留分片必须被清掉（否则新启动器会走分片分支而文件已不在）
        assert!(
            demo.get("parts").is_none(),
            "单包发布应清除残留 parts：{demo}"
        );
        for k in ["version", "sha256", "size"] {
            assert!(
                changed.iter().any(|c| c == k),
                "changed 应含 {k}，实际 {changed:?}"
            );
        }
        assert!(!changed.iter().any(|c| c == "gameEdition"));
    }

    #[test]
    fn part_names_are_zero_padded_so_lexical_order_matches_part_order() {
        assert_eq!(part_name("demo", 1), "demo.part001");
        assert_eq!(part_name("demo", 12), "demo.part012");
        // 清理陈旧分片按文件名排序取前 N 个，故字典序必须等于数值序
        let mut names = vec![part_name("d", 10), part_name("d", 2), part_name("d", 1)];
        names.sort();
        assert_eq!(names, vec!["d.part001", "d.part002", "d.part010"]);
    }

    /// 切分 → 拼回闭环：拼回文件必须与原文**逐字节一致**——导入侧的整包 sha256 校验依赖这一点。
    #[test]
    fn split_then_concat_reproduces_the_original_package() {
        use std::fs;
        let dir = sandbox("split-roundtrip");
        let src = dir.join("demo.vcmirealm");
        let part = REALM_PART_MAX_BYTES as usize;
        // 2 整片 + 1 个不足整片的尾巴
        let blob: Vec<u8> = (0..(part * 2 + 1234)).map(|i| (i % 251) as u8).collect();
        fs::write(&src, &blob).unwrap();
        let want_sha = crate::infra::hash::sha256_file(&src).unwrap();

        let parts = split_realm_package(&src, REALM_PART_MAX_BYTES).unwrap();
        assert_eq!(
            parts.len(),
            3,
            "2 整片 + 1 尾片：{:?}",
            parts.iter().map(|p| p.size).collect::<Vec<_>>()
        );
        assert_eq!(parts[0].size as usize, part);
        assert_eq!(parts[2].size as usize, 1234);
        assert_eq!(parts.iter().map(|p| p.size).sum::<u64>(), blob.len() as u64);
        for (i, p) in parts.iter().enumerate() {
            assert_eq!(p.name, part_name("demo", i + 1));
            assert_eq!(fs::metadata(dir.join(&p.name)).unwrap().len(), p.size);
            assert_eq!(
                crate::infra::hash::sha256_file(&dir.join(&p.name)).unwrap(),
                p.sha256,
                "逐片 sha256 必须与产物一致"
            );
        }

        // 按数组序拼回（下载端同口径）
        let joined = dir.join("joined.vcmirealm");
        let mut out = fs::File::create(&joined).unwrap();
        for p in &parts {
            let mut f = fs::File::open(dir.join(&p.name)).unwrap();
            std::io::copy(&mut f, &mut out).unwrap();
        }
        drop(out);
        assert_eq!(fs::read(&joined).unwrap(), blob, "拼回必须逐字节一致");
        assert_eq!(crate::infra::hash::sha256_file(&joined).unwrap(), want_sha);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn split_boundary_sizes_do_not_produce_trailing_empty_parts() {
        use std::fs;
        let dir = sandbox("split-boundary");
        // 恰好一整片 → 只出 1 片，不产生空尾巴
        let src = dir.join("exact.vcmirealm");
        fs::write(&src, vec![7u8; REALM_PART_MAX_BYTES as usize]).unwrap();
        let parts = split_realm_package(&src, REALM_PART_MAX_BYTES).unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].size, REALM_PART_MAX_BYTES);

        // 比一片多 1 字节 → 2 片，尾片 1 字节
        let src2 = dir.join("one-over.vcmirealm");
        fs::write(&src2, vec![7u8; REALM_PART_MAX_BYTES as usize + 1]).unwrap();
        let parts2 = split_realm_package(&src2, REALM_PART_MAX_BYTES).unwrap();
        assert_eq!(parts2.len(), 2);
        assert_eq!(parts2[1].size, 1);
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn stale_parts_are_pruned_by_name_order() {
        use std::fs;
        let dir = sandbox("prune-parts");
        for n in [1, 2, 3, 4] {
            fs::write(dir.join(part_name("demo", n)), b"x").unwrap();
        }
        fs::write(dir.join(part_name("other", 1)), b"y").unwrap();
        fs::write(dir.join("demo.vcmirealm"), b"pkg").unwrap();
        fs::write(dir.join("cover.webp"), b"img").unwrap();
        prune_stale_parts(&dir, "demo", 2);
        assert!(dir.join("demo.part001").is_file());
        assert!(dir.join("demo.part002").is_file());
        assert!(!dir.join("demo.part003").is_file(), "多余分片应删除");
        assert!(!dir.join("demo.part004").is_file());
        // 其它领域的分片与非分片文件一概不动
        assert!(dir.join("other.part001").is_file());
        assert!(dir.join("demo.vcmirealm").is_file());
        assert!(dir.join("cover.webp").is_file());
        let _ = fs::remove_dir_all(&dir);
    }

    /// `parts` / `download` 互斥且完全派生：单包 ↔ 分片互转时旧的一侧必须被删除（设计 §7.2）。
    #[test]
    fn merge_switches_between_sharded_and_single_package() {
        let existing = json!({
            "realms": { "demo": {
                "version": "1.0.0",
                "download": "https://x/raw/master/realms/demo/demo.vcmirealm",
                "sha256": "old",
                "size": 1
            }}
        });
        // 单包 → 分片：`download` 必须消失（否则新启动器走单包分支，而文件已删）
        let sharded = json!({
            "version": "1.1.0", "sha256": "new", "size": 2,
            "parts": [{ "url": "https://x/raw/master/realms/demo/demo.part001", "size": 1, "sha256": "p1" }]
        });
        let (m1, c1) = merge_repo_index(&existing, "demo", &sharded, "2026-10-13");
        assert_eq!(m1["realms"]["demo"]["parts"], sharded["parts"]);
        assert!(
            m1["realms"]["demo"].get("download").is_none(),
            "分片条目不得残留 download：{}",
            m1["realms"]["demo"]
        );
        // `download` 是被删除的一侧 → 计入变更；`parts` 属「新增字段」，按既有语义
        // （`merge_is_idempotent_and_absent_keys_do_not_erase_existing`）新增不算变化
        assert!(c1.iter().any(|c| c == "download"), "删除应计入变更：{c1:?}");
        assert!(
            !c1.iter().any(|c| c == "parts"),
            "新增 parts 不计变化：{c1:?}"
        );

        // 分片 → 单包：`parts` 必须消失（否则旧启动器会报「请升级」而实际早已回退）
        let single = json!({
            "version": "1.2.0", "sha256": "new2", "size": 3,
            "download": "https://x/raw/master/realms/demo/demo.vcmirealm"
        });
        let (m2, c2) = merge_repo_index(&m1, "demo", &single, "2026-10-13");
        assert_eq!(m2["realms"]["demo"]["download"], single["download"]);
        assert!(
            m2["realms"]["demo"].get("parts").is_none(),
            "单包条目不得残留 parts：{}",
            m2["realms"]["demo"]
        );
        assert!(c2.iter().any(|c| c == "parts"), "删除应计入变更：{c2:?}");
        // 二次合并幂等且不再报变化
        let (m3, c3) = merge_repo_index(&m2, "demo", &single, "2026-10-13");
        assert_eq!(m2, m3);
        assert!(c3.is_empty(), "二次合并不应再报变化：{c3:?}");
    }

    #[test]
    fn merge_is_idempotent_and_absent_keys_do_not_erase_existing() {
        let existing = json!({
            "realms": { "demo": { "version": "1.0.0", "cover": "https://x/cover.webp" } }
        });
        let entry = json!({ "version": "1.0.0", "sha256": "a" });
        let (once, changed1) = merge_repo_index(&existing, "demo", &entry, "2026-09-28");
        let (twice, changed2) = merge_repo_index(&once, "demo", &entry, "2026-09-28");
        assert_eq!(once, twice, "二次合并必须幂等");
        assert!(changed1.is_empty(), "仅新增字段不算变化：{changed1:?}");
        assert!(changed2.is_empty());
        // 本次没算出 cover → 既有条目的封面不能被抹掉
        assert_eq!(
            once["realms"]["demo"]["cover"],
            json!("https://x/cover.webp")
        );
    }

    #[test]
    fn merge_blank_values_never_erase_manual_or_custom_fields() {
        // 构造层对「没算出来」的字段：`tags` 等不写出键、`author` 等写成 null；合并必须把它们
        // 当「未声明」，否则仓库索引里手工维护的标签、备注等会被无配置的发布抹掉。
        let existing = json!({
            "realms": { "demo": {
                "version": "1.0.0", "sha256": "old", "size": 1,
                "author": "原作者", "description": "手工描述",
                "tags": ["剧情", "高清"], "requiresMods": ["m1"],
                "customNote": "维护者自定义备注"
            }}
        });
        let entry = json!({
            "version": "1.1.0", "sha256": "new", "size": 2,
            "author": null, "name": null, "description": "",
            "requiresMods": []
        });
        let (merged, changed) = merge_repo_index(&existing, "demo", &entry, "2026-09-30");
        let demo = &merged["realms"]["demo"];
        assert_eq!(
            demo["tags"],
            json!(["剧情", "高清"]),
            "缺键的 tags 不得清掉既有标签"
        );
        assert_eq!(demo["author"], json!("原作者"));
        assert_eq!(demo["description"], json!("手工描述"));
        assert_eq!(demo["requiresMods"], json!(["m1"]));
        assert_eq!(
            demo["customNote"],
            json!("维护者自定义备注"),
            "自定义字段必须原样保留"
        );
        for k in ["tags", "author", "description", "requiresMods"] {
            assert!(!changed.iter().any(|c| c == k), "未声明不算变化：{k}");
        }
        assert!(changed.iter().any(|c| c == "version"));
    }

    #[test]
    fn merge_i18n_keeps_manually_added_languages() {
        let existing = json!({
            "realms": { "demo": {
                "version": "1.0.0",
                "i18n": { "en": { "name": "Old EN" }, "ja": { "name": "手訳" } }
            }}
        });
        let entry = json!({
            "version": "1.1.0",
            "i18n": { "en": { "name": "New EN" }, "chinese": { "name": "新中文" } }
        });
        let (merged, changed) = merge_repo_index(&existing, "demo", &entry, "2026-09-30");
        let i18n = &merged["realms"]["demo"]["i18n"];
        assert_eq!(i18n["en"]["name"], json!("New EN"), "块内字段照常覆盖");
        assert_eq!(i18n["ja"]["name"], json!("手訳"), "手工加译的其它语言保留");
        assert_eq!(i18n["chinese"]["name"], json!("新中文"));
        assert!(changed.iter().any(|c| c == "i18n"));
        // 二次合并幂等且不再报变化
        let (again, changed2) = merge_repo_index(&merged, "demo", &entry, "2026-09-30");
        assert_eq!(again, merged);
        assert!(changed2.is_empty(), "二次合并应无变化：{changed2:?}");
    }

    #[test]
    fn merge_i18n_keeps_manual_fields_inside_same_language_block() {
        // 同语言块按字段级合并：realm.json 的 i18n 块**没有 tags 键**（未声明）时，不得把索引
        // 块里手工维护的标签挤掉（整块替换的旧缺陷）；块内其它空值同样算「未声明」，不动旧值。
        // （块内显式声明（含空数组=清空）的语义见 merge_tags_array_is_explicit_statement_even_when_empty）
        let existing = json!({
            "realms": { "demo": {
                "version": "1.0.0",
                "i18n": { "english": {
                    "name": "VCMI", "description": "desc", "tags": ["IntroText"]
                }}
            }}
        });
        let entry = json!({
            "version": "1.1.0",
            "i18n": { "english": { "name": "VCMI2", "description": "" } }
        });
        let (merged, changed) = merge_repo_index(&existing, "demo", &entry, "2026-09-30");
        let en = &merged["realms"]["demo"]["i18n"]["english"];
        assert_eq!(en["name"], json!("VCMI2"), "块内声明的字段照常覆盖");
        assert_eq!(en["description"], json!("desc"), "块内空值不动旧值");
        assert_eq!(
            en["tags"],
            json!(["IntroText"]),
            "块内缺键（未声明）的 tags 必须保留"
        );
        assert!(changed.iter().any(|c| c == "i18n"));
        // 二次合并幂等
        let (again, changed2) = merge_repo_index(&merged, "demo", &entry, "2026-09-30");
        assert_eq!(again, merged);
        assert!(changed2.is_empty(), "二次合并应无变化：{changed2:?}");
    }

    #[test]
    fn merge_tags_array_is_explicit_statement_even_when_empty() {
        // `tags` 是「键存在即声明」：空数组 = 显式清空（录入功能上线后 tags 的唯一来源是
        // realm.json，作者删光标签必须能在索引侧生效）；缺键 = 未声明（保留既有值）。
        let existing = json!({
            "realms": { "demo": {
                "version": "1.0.0",
                "tags": ["剧情", "高清"],
                "i18n": { "english": { "name": "VCMI", "tags": ["IntroText"] } }
            }}
        });
        let entry = json!({
            "version": "1.1.0",
            "tags": [],
            "i18n": { "english": { "name": "VCMI2", "tags": [] } }
        });
        let (merged, changed) = merge_repo_index(&existing, "demo", &entry, "2026-09-30");
        let demo = &merged["realms"]["demo"];
        assert_eq!(demo["tags"], json!([]), "顶层空数组 = 显式清空");
        assert_eq!(
            demo["i18n"]["english"]["tags"],
            json!([]),
            "块内空数组 = 显式清空"
        );
        assert!(
            changed.iter().any(|c| c == "tags"),
            "显式清空要计入变更：{changed:?}"
        );
        assert!(changed.iter().any(|c| c == "i18n"));
        // 二次合并幂等且不再报变化
        let (again, changed2) = merge_repo_index(&merged, "demo", &entry, "2026-09-30");
        assert_eq!(again, merged);
        assert!(changed2.is_empty(), "二次合并应无变化：{changed2:?}");
    }

    #[test]
    fn package_realm_json_strips_origin_and_appends_checksum() {
        // `origin` 是本机安装来源记录（launcher 私有元数据），导出打包时必须剥离：
        // 其他启动器导入后会看到指向导出方安装源的无效块。
        let realm = json!({
            "id": "demo",
            "version": "1.0.0",
            "origin": {
                "repositoryUrl": "https://gitee.com/o/r/raw/master/index.json",
                "version": "0.9.0",
                "sha256": "old",
                "installedAt": "2026-09-11"
            }
        });
        let bytes = build_package_realm_json(&realm, "sum123").unwrap();
        let parsed: Value = serde_json::from_slice(&bytes).unwrap();
        assert!(parsed.get("origin").is_none(), "origin 必须剥离：{parsed}");
        assert_eq!(parsed["checksum"], json!("sum123"));
        assert_eq!(parsed["id"], json!("demo"));
        // 纯函数：入参不被改动
        assert!(realm.get("origin").is_some());
    }

    #[test]
    fn derived_fields_cover_the_publishable_set() {
        // 防手抖：新增可推导字段时必须同步这张表，否则合并会漏覆盖
        for k in [
            "sha256",
            "size",
            "download",
            "cover",
            "i18n",
            "requiresMods",
        ] {
            assert!(DERIVED_INDEX_FIELDS.contains(&k), "缺少 {k}");
        }
    }

    #[test]
    fn url_base_is_derived_from_majority_of_raw_downloads() {
        let existing = json!({
            "realms": {
                "a": { "download": "https://gitee.com/o/r/raw/master/realms/a/a.vcmirealm" },
                "b": { "download": "https://gitee.com/o/r/raw/master/realms/b/b.vcmirealm" },
                "c": { "download": "https://example.com/other/branch/c.vcmirealm" }
            }
        });
        assert_eq!(
            derive_url_base(&existing).as_deref(),
            Some("https://gitee.com/o/r/raw/master/")
        );
        // 完全认不出形态（无 /raw/）→ None，由调用方回退手工值并告警
        assert_eq!(
            derive_url_base(&json!({ "realms": { "a": { "download": "https://x/a.zip" } } })),
            None
        );
        assert_eq!(derive_url_base(&json!({})), None);
    }

    #[test]
    fn already_compressed_entries_skip_deflate() {
        // 图片 / 视频 / 地图与游戏数据本体：再 Deflate 只是白烧 CPU
        assert!(is_precompressed("assets/cover.webp"));
        assert!(is_precompressed("assets/intro.mp4"));
        assert!(is_precompressed("vcmi/Maps/demo.h3m"));
        assert!(is_precompressed("data/HEROES3.LOD")); // 扩展名大小写不敏感
                                                       // 文本类仍走 Deflate
        assert!(!is_precompressed("vcmi/Mods/demo/mod.json"));
        assert!(!is_precompressed("realm.json"));
    }

    /// 导出打包 → 导入校验的闭环回归：Stored/Deflated 混排、按路径排序、跨 64 KiB 分块读，
    /// 都必须让导入侧按 `(相对路径, 解压内容)` 复现出同一个 checksum。
    #[test]
    fn packed_entries_reproduce_import_side_checksum() {
        use std::fs;
        use std::io::Read;
        use std::path::PathBuf;
        use zip::ZipArchive;

        let dir = std::env::temp_dir().join("vcmi-fv-export-pack-roundtrip");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let small = b"{\"modID\":\"demo\"}".to_vec();
        // 超过一个读块（64 KiB）的二进制，确保走多次 read/write；扩展名 .webp → Stored
        let blob: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        fs::write(dir.join("mod.json"), &small).unwrap();
        fs::create_dir_all(dir.join("assets")).unwrap();
        fs::write(dir.join("assets/cover.webp"), &blob).unwrap();

        let mut files: Vec<(String, PathBuf)> = vec![
            (
                "assets/cover.webp".to_string(),
                dir.join("assets/cover.webp"),
            ),
            ("mod.json".to_string(), dir.join("mod.json")),
        ];
        files.sort_by(|a, b| a.0.cmp(&b.0));

        let zip_path = dir.join("out.vcmirealm");
        let mut zw = ZipWriter::new(fs::File::create(&zip_path).unwrap());
        let checksum = match write_zip_entries(&mut zw, "round-trip-test", &files, |_, _, _, _| {})
        {
            Ok(PackOutcome::Done { checksum }) => checksum,
            Ok(PackOutcome::Cancelled) => panic!("不该被取消"),
            Err(e) => panic!("打包失败：{e}"),
        };
        zw.set_comment("vcmirealm");
        zw.finish().unwrap();

        let mut ar = ZipArchive::new(fs::File::open(&zip_path).unwrap()).unwrap();
        assert_eq!(String::from_utf8_lossy(ar.comment()).trim(), "vcmirealm");
        let mut names: Vec<String> = (0..ar.len())
            .map(|i| ar.by_index(i).unwrap().name().to_string())
            .collect();
        names.sort();
        assert_eq!(names, vec!["assets/cover.webp", "mod.json"]);

        // 导入侧口径逐字复现（同时验证 Stored 条目内容不被截断/膨胀）
        let mut hasher = Sha256::new();
        for rel in &names {
            let mut f = ar.by_name(rel).unwrap();
            let mut buf = Vec::new();
            f.read_to_end(&mut buf).unwrap();
            let want = if rel.ends_with(".webp") {
                &blob
            } else {
                &small
            };
            assert_eq!(&buf, want, "{rel} 内容应与源文件一致");
            hasher.update(rel.as_bytes());
            hasher.update(&buf);
        }
        assert_eq!(
            crate::infra::hash::to_hex(&hasher.finalize()),
            checksum,
            "导入侧复算的校验和必须与导出侧一致"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// 降级发布阻断（§7.3）：结构化错误带 from/to；同版 / 升版 / 新条目 / 无版本基准一律放行。
    #[test]
    fn downgrade_is_blocked_with_structured_error() {
        let existing = json!({ "realms": { "demo": { "version": "1.2.0" } } });
        let err = downgrade_error(&existing, "demo", "1.0.0").expect("应阻断降级");
        let v: Value = serde_json::from_str(&err).unwrap();
        assert_eq!(v["code"], json!("version_downgrade"));
        assert_eq!(v["from"], json!("1.2.0"));
        assert_eq!(v["to"], json!("1.0.0"));
        assert!(downgrade_error(&existing, "demo", "1.2.0").is_none());
        assert!(downgrade_error(&existing, "demo", "1.3.0").is_none());
        assert!(downgrade_error(&existing, "newbie", "0.1.0").is_none());
        // 既有条目没有 version（手工条目）→ 无比较基准，放行（宁缺勿错）
        let no_ver = json!({ "realms": { "demo": { "download": "https://x/y" } } });
        assert!(downgrade_error(&no_ver, "demo", "1.0.0").is_none());
    }

    /// 版本变更三态自检：同版包变 / 升版包同各一条告警；正常升级与完全重复静默。
    #[test]
    fn version_change_warnings_cover_the_three_states() {
        let prev = json!({ "version": "1.0.0", "sha256": "aa" });
        let w =
            version_change_warnings(Some(&prev), &json!({ "version": "1.0.0", "sha256": "bb" }));
        assert_eq!(w.len(), 1);
        assert_eq!(w[0]["code"], json!("version-unchanged"));
        let w =
            version_change_warnings(Some(&prev), &json!({ "version": "1.1.0", "sha256": "aa" }));
        assert_eq!(w.len(), 1);
        assert_eq!(w[0]["code"], json!("pkg-unchanged"));
        // 正常升级 / 完全重复 / 新条目 → 静默
        assert!(version_change_warnings(
            Some(&prev),
            &json!({ "version": "1.1.0", "sha256": "bb" })
        )
        .is_empty());
        assert!(version_change_warnings(
            Some(&prev),
            &json!({ "version": "1.0.0", "sha256": "aa" })
        )
        .is_empty());
        assert!(
            version_change_warnings(None, &json!({ "version": "1.0.0", "sha256": "aa" }))
                .is_empty()
        );
    }

    /// 直写仓库工作树的端到端回归（临时目录，不碰真仓库）：
    /// 包与封面落地 / URL 从既有条目推导 / 手工字段保留 / 二次发布幂等。
    #[test]
    fn publish_lands_package_cover_and_merges_index() {
        use std::fs;

        let dir = std::env::temp_dir().join("vcmi-fv-publish-test");
        let _ = fs::remove_dir_all(&dir);
        let repo = dir.join("repo");
        let realm_root = dir.join("realm");
        fs::create_dir_all(repo.join("realms/demo")).unwrap();
        fs::create_dir_all(realm_root.join("assets")).unwrap();
        fs::write(
            repo.join("index.json"),
            concat!(
                "{\n  \"realms\": {\n",
                "    \"other\": { \"download\": \"https://gitee.com/o/r/raw/master/realms/other/other.vcmirealm\" },\n",
                "    \"demo\": { \"version\": \"1.0.0\", \"gameEdition\": \"enhanced\", \"tags\": [\"manual\"] }\n",
                "  }\n}\n",
            ),
        )
        .unwrap();
        // 导出产物在仓库外 → 应复制一份进 realms/<id>/ 并出 copied-into-repo 告警
        let outside = dir.join("demo.vcmirealm");
        fs::write(&outside, b"fake-package-bytes").unwrap();
        fs::write(realm_root.join("assets/cover.webp"), b"fake-webp").unwrap();

        let realm = json!({
            "id": "demo",
            "version": "1.1.0",
            "assets": { "cover": "assets/cover.webp" },
            "mods": { "bundled": ["big-town"] },
        });
        let run = || -> (Value, Vec<Value>) {
            // 构造层对「realm.json 无 tags 键」不写出 tags（未声明）——缺键不得清掉索引手工值；
            // 「空数组 = 显式清空」的语义见 merge_tags_array_is_explicit_statement_even_when_empty
            let mut entry = json!({ "version": "1.1.0", "sha256": "abc123", "size": 16 });
            publish_realm_to_repo(
                &realm,
                &realm_root,
                "demo",
                &outside,
                &repo,
                None,
                &mut entry,
                16,
            )
            .unwrap()
        };

        let (info, warnings) = run();
        assert!(
            repo.join("realms/demo/demo.vcmirealm").is_file(),
            "包应落进仓库"
        );
        assert!(
            repo.join("realms/demo/cover.webp").is_file(),
            "封面应落进仓库"
        );
        assert_eq!(info["entryState"], json!("updated"));
        assert!(
            warnings
                .iter()
                .any(|w| w["code"] == json!("copied-into-repo")),
            "仓库外导出应告警：{warnings:?}"
        );
        assert!(
            warnings.iter().any(|w| w["code"] == json!("bundled")),
            "整合模组应告警：{warnings:?}"
        );

        let written = fs::read_to_string(repo.join("index.json")).unwrap();
        assert!(
            written.ends_with("\n"),
            "写盘应带末尾换行（与发布脚本同构）"
        );
        let idx: Value = serde_json::from_str(&written).unwrap();
        let demo = &idx["realms"]["demo"];
        assert_eq!(
            demo["download"],
            json!("https://gitee.com/o/r/raw/master/realms/demo/demo.vcmirealm")
        );
        assert_eq!(
            demo["cover"],
            json!("https://gitee.com/o/r/raw/master/realms/demo/cover.webp")
        );
        assert_eq!(demo["version"], json!("1.1.0"));
        assert_eq!(demo["sha256"], json!("abc123"));
        // 手工字段与别的条目都不能被抹掉
        assert_eq!(demo["gameEdition"], json!("enhanced"));
        assert_eq!(
            demo["tags"],
            json!(["manual"]),
            "缺键的 tags 不得清掉索引里手工维护的标签"
        );
        assert_eq!(
            idx["realms"]["other"]["download"]
                .as_str()
                .unwrap()
                .contains("/raw/master/"),
            true
        );
        assert_eq!(idx["schemaVersion"], json!(1));
        assert!(idx["updatedAt"].as_str().unwrap().len() >= 10);

        // 二次发布：内容未变 → 不应再报任何字段变化（幂等），条目动作应为 unchanged
        let (again, _) = run();
        assert_eq!(again["changedFields"], json!([]), "重复发布应幂等：{again}");
        assert_eq!(
            again["entryState"],
            json!("unchanged"),
            "重复发布应识别「无变化」：{again}"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn remote_forms_normalize_or_reject() {
        for (input, want) in [
            ("https://github.com/o/r.git", "https://github.com/o/r"),
            ("https://gitee.com/o/r/", "https://gitee.com/o/r"),
            ("git@github.com:o/r.git", "https://github.com/o/r"),
            (
                "ssh://git@gitlab.example.com/g/o/r.git",
                "https://gitlab.example.com/g/o/r",
            ),
            ("git://github.com/o/r", "https://github.com/o/r"),
            ("https://github.com:443/o/r", "https://github.com/o/r"),
        ] {
            assert_eq!(
                normalize_remote_url(input).as_deref(),
                Some(want),
                "输入 {input}"
            );
        }
        // 认不出就别硬猜（退给下一级兜底）：本地镜像、非标准端口、file 协议均不接受
        for bad in [
            "",
            "C:/local/mirror.git",
            "ssh://git@host:7999/o/r.git",
            "file:///srv/git/r.git",
        ] {
            assert!(normalize_remote_url(bad).is_none(), "不该接受 {bad}");
        }
    }

    #[test]
    fn origin_url_is_read_from_its_own_section() {
        let config = concat!(
            "[remote \"upstream\"]\n\turl = https://github.com/other/upstream.git\n",
            "\tfetch = +refs/heads/*:refs/remotes/upstream/*\n",
            "[remote \"origin\"]\n\turl = git@gitee.com:me/mine.git\n",
            "[branch \"master\"]\n\tremote = origin\n",
        );
        assert_eq!(
            parse_git_remote_url(config).as_deref(),
            Some("https://gitee.com/me/mine")
        );
        // 没有 origin 段 → None（拿别的 remote 凑一凑，地址必然错）
        assert!(parse_git_remote_url("[remote \"upstream\"]\n\turl = https://x/y/z.git").is_none());
    }

    #[test]
    fn branch_prefers_head_ref_and_falls_back_to_default_branch() {
        use std::fs;
        let dir = sandbox("branch");
        // 分支名本身含 `/` 时只能削前缀，不能拆后缀
        fs::write(dir.join("HEAD"), "ref: refs/heads/feature/season-2\n").unwrap();
        assert_eq!(parse_git_branch(&dir, ""), "feature/season-2");
        // detached HEAD（裸 SHA）→ 退 init.defaultBranch，再退 master
        fs::write(
            dir.join("HEAD"),
            "8b1a3f0c9d2e4a5b6c7d8e9f0a1b2c3d4e5f6a7b\n",
        )
        .unwrap();
        assert_eq!(
            parse_git_branch(&dir, "[init]\n\tdefaultBranch = main\n"),
            "main"
        );
        assert_eq!(parse_git_branch(&dir, ""), "master");
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn gitdir_indirection_is_followed() {
        use std::fs;
        let dir = sandbox("gitdir");
        let real = dir.join("storage");
        fs::create_dir_all(&real).unwrap();
        fs::write(dir.join(".git"), format!("gitdir: {}\n", real.display())).unwrap();
        assert_eq!(resolve_gitdir(&dir).unwrap(), real);
        // worktree / submodule 的相对形式也要能接住
        fs::write(dir.join(".git"), "gitdir: .git/worktrees/x\n").unwrap();
        assert_eq!(resolve_gitdir(&dir).unwrap(), dir.join(".git/worktrees/x"));
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&real);
        assert!(resolve_gitdir(&dir).is_none());
    }

    #[test]
    fn url_base_priority_is_declaration_then_git_then_entries() {
        use std::fs;
        let dir = sandbox("url-priority");
        let git = dir.join(".git");
        fs::create_dir_all(&git).unwrap();
        fs::write(
            git.join("config"),
            "[remote \"origin\"]\n\turl = git@github.com:o/r.git\n",
        )
        .unwrap();
        fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        let entries = || json!({ "realms": { "x": { "download": "https://gitee.com/o/r/raw/master/realms/x/x.vcmirealm" } } });

        // S1 声明最优先（即使 git 与条目都推得出别的值）
        let declared = json!({ "urlBase": "https://cdn.example.com/realms/", "realms": {} });
        let (b, src) = derive_repo_url_base(&dir, &declared).unwrap();
        assert_eq!(
            (b.as_str(), src),
            ("https://cdn.example.com/realms", URL_SOURCE_DECLARED)
        );

        // 无声明 → git 优先于条目（新仓库首次发布也该拿到正确地址，而不是跟着旧条目的值走）
        let (b, src) = derive_repo_url_base(&dir, &entries()).unwrap();
        assert_eq!(
            (b.as_str(), src),
            ("https://github.com/o/r/raw/main", URL_SOURCE_GIT)
        );

        // 无 .git → 退条目；三者皆无 → None（由调用方告警）
        let plain = sandbox("url-priority-plain");
        let (b, src) = derive_repo_url_base(&plain, &entries()).unwrap();
        assert_eq!(
            (b.as_str(), src),
            ("https://gitee.com/o/r/raw/master", URL_SOURCE_ENTRIES)
        );
        assert!(derive_repo_url_base(&plain, &json!({ "realms": {} })).is_none());
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&plain);
    }

    #[test]
    fn publish_bootstraps_declared_url_base_only_once() {
        use std::fs;
        let dir = sandbox("bootstrap");
        let repo = dir.join("repo");
        let realm_root = dir.join("realm");
        fs::create_dir_all(repo.join(".git")).unwrap();
        fs::write(
            repo.join(".git/config"),
            "[remote \"origin\"]\n\turl = https://github.com/o/r.git\n",
        )
        .unwrap();
        fs::write(repo.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        fs::write(
            repo.join("index.json"),
            "{\n  \"schemaVersion\": 1,\n  \"realms\": {}\n}\n",
        )
        .unwrap();
        let pkg = dir.join("demo.vcmirealm");
        fs::write(&pkg, b"bytes").unwrap();
        let realm = json!({ "id": "demo", "version": "1.0.0" });

        let run = |override_base: Option<&str>| {
            let mut entry = json!({ "version": "1.0.0", "sha256": "aa", "size": 5 });
            let (info, _) = publish_realm_to_repo(
                &realm,
                &realm_root,
                "demo",
                &pkg,
                &repo,
                override_base,
                &mut entry,
                5,
            )
            .unwrap();
            (info, entry)
        };

        // 首次：靠 git 推出并自举回写，且排在 realms 之前（人读的文件，位置即文档）
        let (first, _) = run(None);
        assert_eq!(first["urlSource"], json!("git"));
        assert_eq!(first["urlBootstrapped"], json!(true));
        let idx: Value =
            serde_json::from_str(&fs::read_to_string(repo.join("index.json")).unwrap()).unwrap();
        assert_eq!(idx["urlBase"], json!("https://github.com/o/r/raw/main/"));
        let keys: Vec<&str> = idx
            .as_object()
            .unwrap()
            .keys()
            .map(|k| k.as_str())
            .collect();
        // `updatedAt` 由 merge 写入，排在末尾；关键是 `urlBase` 得夹在元信息区、`realms` 之前
        assert_eq!(keys, ["schemaVersion", "urlBase", "realms", "updatedAt"]);

        // 二次：改走声明、不再自举、字段幂等
        let (second, _) = run(None);
        assert_eq!(second["urlSource"], json!("declared"));
        assert_eq!(second["urlBootstrapped"], json!(false));
        assert_eq!(second["changedFields"], json!([]));

        // 手填覆盖只影响本次下载地址，不污染仓库
        let (_, entry) = run(Some("https://mirror.example.com/r/"));
        assert_eq!(
            entry["download"],
            json!("https://mirror.example.com/r/realms/demo/demo.vcmirealm")
        );
        let idx2: Value =
            serde_json::from_str(&fs::read_to_string(repo.join("index.json")).unwrap()).unwrap();
        assert_eq!(idx2["urlBase"], json!("https://github.com/o/r/raw/main/"));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn url_base_insertion_keeps_order_and_existing_value() {
        let mut idx = json!({ "schemaVersion": 1, "realms": { "a": {} }, "updatedAt": "x" });
        insert_url_base(&mut idx, "https://h/o/r/raw/main/");
        let keys: Vec<&str> = idx
            .as_object()
            .unwrap()
            .keys()
            .map(|k| k.as_str())
            .collect();
        assert_eq!(keys, ["schemaVersion", "urlBase", "realms", "updatedAt"]);
        // 已有则不覆盖（仓库作者的声明优先于启动器的推导）
        insert_url_base(&mut idx, "https://other/");
        assert_eq!(idx["urlBase"], json!("https://h/o/r/raw/main/"));
        // 没有 realms 键时追加末尾
        let mut bare = json!({ "schemaVersion": 1 });
        insert_url_base(&mut bare, "https://h/");
        assert_eq!(bare["urlBase"], json!("https://h/"));
    }
}
