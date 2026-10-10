//! 领域个性化：介绍页（intro）状态、BGM 开关、自定义素材导入（封面 / 主题曲 / 背景）。

use crate::infra::berror::be_err;
use super::base::realm_root;

use serde_json::{json, Value};
use std::fs;
use std::path::Path;


/// 返回领域目录绝对路径（`{realm-data}/realms/<id>`），供前端解析 `assets/` 素材。
#[tauri::command]
pub fn realm_dir(realm_id: String) -> Result<String, String> {
    Ok(realm_root(&realm_id)?.to_string_lossy().to_string())
}

/// 读取登场演出上次播放时间（§9.1 的 24 小时策略）。无记录返回 null。
#[tauri::command]
pub fn get_intro_state(realm_id: String) -> Value {
    let root = match realm_root(&realm_id) {
        Ok(p) => p,
        Err(_) => return json!({ "playedAt": Value::Null }),
    };
    let f = root.join(".introPlayedAt");
    let played: Option<u64> = if f.is_file() {
        fs::read_to_string(&f)
            .ok()
            .and_then(|t| t.trim().parse::<u64>().ok())
    } else {
        None
    };
    json!({ "playedAt": played })
}

/// 记录登场演出已播放（写入 unix 秒时间戳到 `{realm-dir}/.introPlayedAt`）。
#[tauri::command]
pub fn mark_intro_played(realm_id: String) -> Result<(), String> {
    let root = realm_root(&realm_id)?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    fs::create_dir_all(&root).map_err(|e| be_err("realm_dir_create_failed", &format!("{}", e)))?;
    fs::write(root.join(".introPlayedAt"), ts.to_string())
        .map_err(|e| be_err("realm_showcase_write_failed", &format!("{}", e)))
}

/// 读取当前启动器下该领域是否静音背景音乐（用户态开关，不随导出走）。
/// 无记录返回 false（默认播放）。
#[tauri::command]
pub fn get_bgm_muted(realm_id: String) -> bool {
    let root = match realm_root(&realm_id) {
        Ok(p) => p,
        Err(_) => return false,
    };
    let f = root.join(".bgmMuted");
    f.is_file() && fs::read_to_string(&f).map(|t| t.trim() == "true").unwrap_or(false)
}

/// 写入当前启动器下该领域的背景音乐静音开关（`{realm-dir}/.bgmMuted`）。
/// 该点文件属用户态，导出打包时由 `collect_realm_files` 排除，不随 realm 走。
#[tauri::command]
pub fn set_bgm_muted(realm_id: String, muted: bool) -> Result<(), String> {
    let root = realm_root(&realm_id)?;
    fs::create_dir_all(&root).map_err(|e| be_err("realm_dir_create_failed", &format!("{}", e)))?;
    fs::write(root.join(".bgmMuted"), if muted { "true" } else { "false" })
        .map_err(|e| be_err("realm_bgm_flag_write_failed", &format!("{}", e)))
}

/// 把玩家选择的文件复制为领域素材（§5.4 的 `{realm-dir}/assets/`）。
/// `kind` ∈ {"wallpaper", "cover", "video", "music", "introMusic"}；返回相对领域目录的路径
/// （wallpaper→`assets/wallpaper.<ext>`、cover→`assets/cover.<ext>`、video→`assets/intro.<ext>`）。
///
/// `sub` / `stem` 是**可选的命名覆盖**，为「阵营巡礼的自定义底图」而加：
/// 每个阵营各要一张底图，不能都落到同一个扁平 `assets/<kind stem>.<ext>`（会互相覆盖），
/// 故允许指定 `sub`（assets 下的子目录，如 `showcase`）与 `stem`（该素材的唯一名）。
/// 两者都做**路径安全净化**（只留 ASCII 字母数字、`-`、`_`），避免 `../` 之类的越界写入。
/// 省略时行为与之前完全一致。`stem` 单独给出时会清掉 `<sub>/<stem>.*` 的旧扩展名文件。
#[tauri::command]
pub fn import_realm_asset(
    realm_id: String,
    src: String,
    kind: String,
    sub: Option<String>,
    stem: Option<String>,
) -> Result<Value, String> {
    let root = realm_root(&realm_id)?;
    let src_path = Path::new(&src);
    if !src_path.is_file() {
        return Err(be_err("realm_import_src_missing", ""));
    }
    let ext = src_path
        .extension()
        .and_then(|e| e.to_str())
        .filter(|e| !e.is_empty() && e.len() <= 8 && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .map(|e| e.to_lowercase())
        .unwrap_or_else(|| "bin".to_string());

    /// 路径片段净化：只保留 ASCII 字母数字与 `-` `_`（`.` 也挡掉，防止 `..` 越界）。
    fn safe_seg(s: &str) -> String {
        s.chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .collect::<String>()
    }

    let stem_owned = match stem.as_deref().map(safe_seg) {
        Some(s) if !s.is_empty() => s,
        _ => match kind.as_str() {
            "video" => "intro",
            "wallpaper" => "wallpaper",
            "music" => "music",
            "introMusic" => "intro-music",
            // cover（封面）及其余未知 kind
            _ => "cover",
        }
        .to_string(),
    };
    let sub_owned = sub
        .as_deref()
        .map(safe_seg)
        .filter(|s| !s.is_empty())
        .unwrap_or_default();

    let assets = root.join("assets");
    let dir = if sub_owned.is_empty() {
        assets.clone()
    } else {
        assets.join(&sub_owned)
    };
    fs::create_dir_all(&dir).map_err(|e| be_err("realm_assets_dir_create_failed", &format!("{}", e)))?;
    // 清理同名不同扩展名的旧素材，避免反复上传后堆积
    if let Ok(entries) = fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file()
                && p.file_stem().and_then(|s| s.to_str()) == Some(stem_owned.as_str())
                && p.extension().and_then(|e| e.to_str()) != Some(ext.as_str())
            {
                let _ = fs::remove_file(&p);
            }
        }
    }
    let rel = if sub_owned.is_empty() {
        format!("assets/{stem_owned}.{ext}")
    } else {
        format!("assets/{sub_owned}/{stem_owned}.{ext}")
    };
    fs::copy(src_path, root.join(&rel)).map_err(|e| be_err("realm_assets_copy_failed", &format!("{}", e)))?;
    Ok(json!({ "path": rel }))
}
