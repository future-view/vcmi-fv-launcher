//! 平台三元组与更新方式（ADR 0008 D1/D5/D7）。
//!
//! 与 `scripts/release.mjs` 的 `TARGETS` 描述符**镜像**：pack 侧决定构建命令与产物名，
//! 本模块决定客户端按哪个三元组拉清单、按哪种方式应用。**改一处必须同步另一处。**

use serde::{Deserialize, Serialize};

/// 更新方式（per-target 清单条目的 `updateMode` 字段）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UpdateMode {
    /// 就地替换：zip 解压后逐文件落地（Windows：PowerShell 脚本；Linux：POSIX shell）
    Inplace,
    /// AppImage 整包替换（pack 当前不发这类更新资产，保留枚举位）
    Appimage,
    /// macOS `.app.tar.gz` 整包替换 + 重签（待签名基建就绪后启用，见 ADR 0008 D7）
    Bundle,
    /// 不应用：只提示用户手动下载（macOS 无签名证书的过渡态）
    Manual,
}

impl UpdateMode {
    /// 清单字符串 → 枚举。缺省 / 无法识别 → `Inplace`（旧清单无该字段时行为不变）。
    pub fn from_manifest(s: Option<&str>) -> Self {
        match s.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
            Some("appimage") => Self::Appimage,
            Some("bundle") => Self::Bundle,
            Some("manual") => Self::Manual,
            _ => Self::Inplace,
        }
    }

    /// 序列化回清单里的字符串形式。
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Inplace => "inplace",
            Self::Appimage => "appimage",
            Self::Bundle => "bundle",
            Self::Manual => "manual",
        }
    }
}

/// 当前运行二进制的 target triple（`std::env::consts` **显式映射**）。
///
/// ⚠️ **macOS 的 OS 常量是 `macos` 但 triple 是 `darwin`**——字符串拼接必错；
/// ⚠️ 用**运行时**值而非编译期 `cfg!`：Rosetta 下 x86_64 构建必须报 `darwin-x86_64`
/// （更新要换同架构二进制，匹配的是二进制而不是宿主 CPU）。
///
/// 未知组合 → `None`（调用方按"无更新"静默处理，绝不猜）。
pub fn target_triple() -> Option<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("windows", "x86_64") => Some("windows-x86_64"),
        ("windows", "aarch64") => Some("windows-aarch64"),
        ("macos", "aarch64") => Some("darwin-aarch64"),
        ("macos", "x86_64") => Some("darwin-x86_64"),
        ("linux", "x86_64") => Some("linux-x86_64"),
        ("linux", "aarch64") => Some("linux-aarch64"),
        _ => None,
    }
}

/// 该平台 zip 更新包内的主程序名（与 `release.mjs::TARGETS[].bin` 镜像）。
/// 未知三元组 → Linux 通用名（`inplace` 白名单只对已知平台有意义）。
pub fn binary_name(triple: &str) -> &'static str {
    match triple {
        "windows-x86_64" | "windows-aarch64" => "vcmi-fv-launcher.exe",
        "darwin-aarch64" | "darwin-x86_64" => "VCMI-FV-Launcher.app",
        _ => "vcmi-fv-launcher",
    }
}

/// 该平台的**缺省**更新方式（与 `release.mjs::TARGETS[].updateMode` 镜像）。
/// 清单条目显式带了 `updateMode` 时以条目为准；这里只兜底旧条目。
///
/// ⚠️ darwin 当前是 `Manual`（无 Developer ID 证书的过渡态）；签名基建就绪后
/// 与描述符一起翻成 `Bundle`（ADR 0008 D7）。
pub fn default_update_mode(triple: &str) -> UpdateMode {
    match triple {
        "darwin-aarch64" | "darwin-x86_64" => UpdateMode::Manual,
        _ => UpdateMode::Inplace,
    }
}

/// 把清单 URL 模板里的 `{platform}` 占位符替换为当前三元组。
///
/// - 不含占位符 → 原样返回（设置页手填的绝对地址）；
/// - 含占位符但三元组解析不出 → `None`（调用方按"无更新"静默处理）。
pub fn resolve_manifest_url(template: &str) -> Option<String> {
    if !template.contains("{platform}") {
        return Some(template.to_string());
    }
    target_triple().map(|t| template.replace("{platform}", t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triple_is_known_or_none() {
        if let Some(t) = target_triple() {
            assert!(matches!(
                t,
                "windows-x86_64"
                    | "windows-aarch64"
                    | "darwin-aarch64"
                    | "darwin-x86_64"
                    | "linux-x86_64"
                    | "linux-aarch64"
            ));
        }
    }

    #[test]
    fn binary_name_mirrors_pack_descriptor() {
        assert_eq!(binary_name("windows-x86_64"), "vcmi-fv-launcher.exe");
        assert_eq!(binary_name("windows-aarch64"), "vcmi-fv-launcher.exe");
        assert_eq!(binary_name("darwin-aarch64"), "VCMI-FV-Launcher.app");
        assert_eq!(binary_name("darwin-x86_64"), "VCMI-FV-Launcher.app");
        assert_eq!(binary_name("linux-x86_64"), "vcmi-fv-launcher");
        // 未知三元组不 panic，走 Linux 通用名
        assert_eq!(binary_name("linux-riscv64"), "vcmi-fv-launcher");
    }

    #[test]
    fn update_mode_parsing() {
        assert_eq!(UpdateMode::from_manifest(None), UpdateMode::Inplace);
        assert_eq!(UpdateMode::from_manifest(Some("")), UpdateMode::Inplace);
        assert_eq!(
            UpdateMode::from_manifest(Some("manual")),
            UpdateMode::Manual
        );
        assert_eq!(
            UpdateMode::from_manifest(Some("BUNDLE")),
            UpdateMode::Bundle
        );
        assert_eq!(
            UpdateMode::from_manifest(Some("appimage")),
            UpdateMode::Appimage
        );
        assert_eq!(UpdateMode::from_manifest(Some("x")), UpdateMode::Inplace);
        // 过渡态：darwin 缺省 manual，其余 inplace
        assert_eq!(default_update_mode("darwin-aarch64"), UpdateMode::Manual);
        assert_eq!(default_update_mode("windows-x86_64"), UpdateMode::Inplace);
    }

    #[test]
    fn resolve_url_substitutes_platform() {
        let tpl = "https://example.com/update/{platform}.json";
        let resolved = resolve_manifest_url(tpl).unwrap();
        if let Some(t) = target_triple() {
            assert_eq!(resolved, format!("https://example.com/update/{t}.json"));
        }
        // 无占位符 → 原样（设置页手填兼容）
        assert_eq!(
            resolve_manifest_url("https://x/y.json").unwrap(),
            "https://x/y.json"
        );
    }
}
