//! VCMI 官方启动器（vcmiqt）+ 游戏引擎的用户配置播种：首次运行向导状态。
//!
//! 依赖方向（见 `docs/architecture/backend.zh-CN.md` §4.1）：
//! 只依赖 `infra`；**不得**出现 `tauri` 类型。
//!
//! 背景：官方启动器是否弹「首次运行向导」取决于用户配置目录里
//! `settings.json` 的 `launcher.setupCompleted`（`references/vcmi`，1.8-dev）：
//! - `launcher/mainwindow_moc.cpp:83,129-134`：仅当
//!   `h3DataFound && settings["launcher"]["setupCompleted"]` 才直接进主界面，否则 `enterSetup()`；
//! - `config/schemas/settings.json:756`：`setupCompleted` 默认 **false**。
//!
//! 于是我们启动器接管的每个**新**用户目录（重定向/便携的 `userdata`、领域隔离配置目录）
//! 都会被官方启动器当成全新安装而弹向导——即便游戏数据就躺在游戏目录里、向导毫无作用。
//! 本模块负责在目录建好后把该标记补上。

use crate::infra::berror::be_err;
use std::fs;
use std::path::Path;

use serde_json::Value;

/// 播种 `settings.json` 的 `launcher.setupCompleted=true`，让官方启动器跳过首次运行向导。
///
/// 作用于任意配置目录（全局用户配置目录 / 领域隔离配置目录）：
/// 领域激活后 VCMI 的活动配置目录是领域目录，因此需要同样播种。
///
/// 安全约束：
/// - **尊重显式值**：只要 `launcher.setupCompleted` 这个键存在（哪怕是 false）就不动它
///   —— 官方「重新运行初始化」（`startGame/StartGameTab.cpp:305-314`）会写 false 并清空
///   `userDataPath` 下的 Data/Maps/Mp3/Video；若我们写回 true，用户将永远无法重跑向导。
/// - 文件存在但**解析失败直接报错**，绝不降级为空对象覆写（否则清空用户全部设置）。
/// - `general.language` 仅在缺失时按 `launcher.json` 的 `ui.lang` 补齐
///   （与 `set_vcmi_language` 同语义：语言不一致会让 VCMI 把翻译模组判为 BROKEN）。
pub fn seed_setup_completed(config_dir: &Path) -> Result<(), String> {
    let p = config_dir.join("settings.json");
    fs::create_dir_all(config_dir).map_err(|e| be_err("realm_config_dir_create_failed", &format!("{}", e)))?;

    let mut root: Value = if p.is_file() {
        let text = fs::read_to_string(&p).map_err(|e| be_err("cmd_settings_read_failed", &format!("{}", e)))?;
        crate::infra::json::parse_json_lenient(&text)
            .map_err(|e| be_err("cmd_settings_parse_abort", &format!("{}", e)))?
    } else {
        serde_json::json!({})
    };
    if !root.is_object() {
        return Err(be_err("vcmi_settings_struct", ""));
    }

    // 已显式写入过该标记（true 或 false）→ 原样保留
    if root
        .get("launcher")
        .and_then(|l| l.get("setupCompleted"))
        .is_some()
    {
        return Ok(());
    }

    let root_obj = root.as_object_mut().expect("已校验为对象");
    let launcher = root_obj
        .entry("launcher".to_string())
        .or_insert_with(|| serde_json::json!({}));
    if !launcher.is_object() {
        *launcher = serde_json::json!({});
    }
    launcher["setupCompleted"] = serde_json::json!(true);

    let ui_lang = crate::vcmi::paths::VcmiPaths::read_launcher_config()
        .and_then(|c| c.ui.lang)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let has_lang = root
        .get("general")
        .and_then(|g| g.get("language"))
        .and_then(|v| v.as_str())
        .map(|s| !s.trim().is_empty())
        .unwrap_or(false);
    if let (Some(lang), false) = (ui_lang, has_lang) {
        if !root.get("general").map(|v| v.is_object()).unwrap_or(false) {
            root["general"] = serde_json::json!({});
        }
        root["general"]["language"] = serde_json::json!(lang);
    }

    let text = serde_json::to_string_pretty(&root)
        .map_err(|e| be_err("vcmi_settings_serialize_failed", &format!("{}", e)))?;
    fs::write(&p, text).map_err(|e| be_err("cmd_settings_write_failed", &format!("{}", e)))?;
    tracing::info!(
        target: "paths",
        "已播种 settings.json 的 launcher.setupCompleted=true（跳过官方启动器首次运行向导）"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::seed_setup_completed;
    use std::fs;
    use std::path::PathBuf;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("vcmi-fv-setup-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        d
    }

    fn read(dir: &PathBuf) -> serde_json::Value {
        let text = fs::read_to_string(dir.join("settings.json")).expect("settings.json 应已生成");
        serde_json::from_str(&text).expect("应为合法 JSON")
    }

    /// 空目录：生成文件并写入 setupCompleted=true
    #[test]
    fn seeds_new_config_dir() {
        let dir = tmp("new");
        seed_setup_completed(&dir).expect("播种应成功");
        let v = read(&dir);
        assert_eq!(v["launcher"]["setupCompleted"], serde_json::json!(true));
        let _ = fs::remove_dir_all(&dir);
    }

    /// 已有配置但无 launcher 段：补标记，且**保留**既有内容
    #[test]
    fn merges_into_existing_settings() {
        let dir = tmp("merge");
        fs::create_dir_all(&dir).unwrap();
        fs::write(
            dir.join("settings.json"),
            r#"{"general":{"language":"chinese"},"video":{"targetfps":60}}"#,
        )
        .unwrap();
        seed_setup_completed(&dir).expect("播种应成功");
        let v = read(&dir);
        assert_eq!(v["launcher"]["setupCompleted"], serde_json::json!(true));
        assert_eq!(v["general"]["language"], serde_json::json!("chinese"));
        assert_eq!(v["video"]["targetfps"], serde_json::json!(60));
        let _ = fs::remove_dir_all(&dir);
    }

    /// 显式 false（官方「重新运行初始化」写入）：必须原样保留，不得改写为 true
    #[test]
    fn respects_explicit_false() {
        let dir = tmp("false");
        fs::create_dir_all(&dir).unwrap();
        let before = r#"{"launcher":{"setupCompleted":false}}"#;
        fs::write(dir.join("settings.json"), before).unwrap();
        seed_setup_completed(&dir).expect("应视为已完成（无需改动）");
        assert_eq!(
            fs::read_to_string(dir.join("settings.json")).unwrap(),
            before,
            "显式 false 不应被改写"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// 文件损坏：必须报错而不是覆写
    #[test]
    fn refuses_corrupt_file() {
        let dir = tmp("corrupt");
        fs::create_dir_all(&dir).unwrap();
        let before = "{ not json";
        fs::write(dir.join("settings.json"), before).unwrap();
        assert!(seed_setup_completed(&dir).is_err());
        assert_eq!(
            fs::read_to_string(dir.join("settings.json")).unwrap(),
            before,
            "损坏文件不应被覆写"
        );
        let _ = fs::remove_dir_all(&dir);
    }
}
