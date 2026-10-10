//! Windows 就地替换：暂存文件 → **静默 PowerShell 脚本** → detached spawn。
//!
//! 核心事实（勿违背）：
//! - Windows **不能覆盖运行中的 exe，但可重命名** → 脚本先等旧进程退出，再
//!   `Move-Item 旧→.old` + `Move-Item 新→原位`，禁用覆盖式拷贝；
//! - **必须原位替换**（数据目录 = exe 同级 `.vcmi-fv-launcher`，换目录 = 配置/领域/缓存全丢）；
//! - **绝不要让用户看到控制台窗口**：替换脚本由 `powershell.exe` 承载，spawn 时带
//!   `CREATE_NO_WINDOW` + `-WindowStyle Hidden`。
//!   ⚠️ 反面教材：曾用 `cmd /C` + 批处理实现，既会闪出黑框，又同时踩 cmd 的三重坑
//!   （`/C` 引号剥离、`%~dp1` 字符串拼接、批处理体按 ANSI 代码页解码）——
//!   实测在 `start` 一行报「文件名、目录名或卷标语法不正确。」；
//! - 路径**只经命令行参数传递**（Rust `.arg()` 负责引号），PS 脚本体保持纯 ASCII、
//!   不含任何路径 → 不受代码页影响，非 ASCII 安装目录同样安全。

use crate::infra::berror::be_err;
use std::fs;
use std::path::Path;
use std::process::Command;

use super::download;
use super::{data_dir, exe_path, pending_apply, phase, set_pending_apply, set_phase, target, UpdatePhase};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// 不给子进程分配可见控制台（替换脚本必须静默）。
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
#[cfg(windows)]
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

/// 应用更新：状态校验 → 可写性探测 → 写脚本 → detached spawn。
///
/// `relaunch`：替换完成后是否**重新拉起新版本**。
/// - `true`：用于「重启并应用」——脚本等本进程退出后原地替换并拉起新版本；
/// - `false`：用于「关闭时自动应用」——只替换二进制，**不拉起**，用户下次手动打开即为新版。
///
/// 成功后由前端「重启并应用」路径延迟调用 `close_window`（本函数绝不 `app.exit`，否则 IPC 响应送不达）；
/// 脚本等本进程退出后完成替换，并按 `relaunch` 决定是否拉起。
pub fn apply_update(relaunch: bool) -> Result<(), String> {
    if cfg!(debug_assertions) {
        return Err(be_err("updater_apply_debug", ""));
    }
    if phase() != UpdatePhase::Downloaded {
        return Err(be_err("updater_apply_not_downloaded", ""));
    }
    let entry = { target().clone().ok_or_else(|| be_err("updater_no_checked_update", ""))? };
    let exe = exe_path()?;
    let dir = exe
        .parent()
        .ok_or_else(|| be_err("common_exe_dir_unknown", ""))?
        .to_path_buf();

    // 可写性探测：不可写（如 Program Files）→ 走「手动替换」降级
    let probe = dir.join(".vcmi-fv-write-probe");
    match fs::write(&probe, b"ok") {
        Ok(()) => {
            let _ = fs::remove_file(&probe);
        }
        Err(e) => {
            return Err(be_err("upd_launcher_dir_not_writable", &format!("{}", e)))
        }
    }

    // zip 形态：脚本按「暂存目录 + 相对路径」逐文件移动，%Staged 必须是**已解压的暂存目录**；
    // 单 exe 形态：脚本把 Staged 直接移动到原位，%Staged 必须是**暂存的 .exe 文件**。
    let staged = if entry.is_zip() {
        download::staged_dir(&entry.version)?
    } else {
        download::staged_path(&entry)?
    };
    if !staged.exists() {
        return Err(be_err("updater_apply_staged_missing", ""));
    }

    let data = data_dir()?;
    let ps1 = data.join("update-apply.ps1");
    let log = data.join("update-apply.log");
    let _ = fs::remove_file(&log);
    fs::write(&ps1, APPLY_PS1).map_err(|e| be_err("upd_script_write_failed", &format!("{}", e)))?;

    spawn_replacer(&ps1, &staged, &exe, &log, relaunch)?;
    set_phase(UpdatePhase::Idle);
    // 记录「已应用目标版本」（注意：写的是 entry.version 而非 CURRENT——此刻 exe 尚未被替换，
    // CURRENT 仍是旧版）。用于识别「应用后又手工降级」：回退后
    // appliedUpdateVersion != CURRENT_VERSION → 7 天节流重新生效（见 manifest.rs::check_update）。
    // 写在这里可同时覆盖「手动重启并应用」与「关闭时自动应用」两条路径。
    super::mark_applied_version(&entry.version, &entry.sha256);
    // 标记已清除：脚本在父进程退出后才完成替换，提前清掉可避免
    // （a）下次启动（若脚本异常未替换）误报旧版本已就绪；
    // （b）关闭时自动应用路径（close_window）与手动路径共用本函数，统一在此清。
    set_pending_apply(None);
    tracing::info!(
        target: "updater",
        "替换脚本已启动（powershell 静默），等待进程退出后原位应用新版本 {}（日志：{}）",
        entry.version,
        log.display()
    );
    Ok(())
}

/// 替换脚本本体。**纯 ASCII，不含任何路径**（路径全部由命令行参数传入）。
///
/// 参数顺序（`-File` 之后的位置参数）：`TargetPid` / `Staged` / `InstalledExe` / `LogPath`。
///
/// - `Staged` 是**目录**（zip 多文件包）→ 递归逐文件移动到安装目录，主程序放最后；
/// - `Staged` 是**文件**（单 exe 包）→ 直接移动到安装目录的 exe 位置；
/// - **只移动、绝不删除目录**：目标已是目录（可能是借用联接）→ 跳过并记日志（见 rules 04）；
/// - 全程写日志到 `LogPath`，失败可回溯（无窗口，日志是唯一现场）。
const APPLY_PS1: &str = r##"param(
  [string]$TargetPid,
  [string]$Staged,
  [string]$InstalledExe,
  [string]$LogPath,
  [string]$Relaunch
)

$ErrorActionPreference = 'SilentlyContinue'
$ProgressPreference = 'SilentlyContinue'

function Write-Log([string]$Message) {
  try {
    $line = '{0} {1}' -f (Get-Date -Format 'yyyy-MM-dd HH:mm:ss'), $Message
    Add-Content -LiteralPath $LogPath -Value $line -Encoding UTF8
  } catch { }
}

Write-Log ('start; target pid={0}; staged={1}; exe={2}; relaunch={3}' -f $TargetPid, $Staged, $InstalledExe, $Relaunch)

# IMPORTANT: use .NET Path APIs for parent dir / file name.
# PowerShell 5.1 rejects `Split-Path -LiteralPath X -Leaf/-Parent` (AmbiguousParameterSet)
# and returns an empty string; that silently breaks every Move-Item below.
$installDir = [System.IO.Path]::GetDirectoryName($InstalledExe)
$exeName = [System.IO.Path]::GetFileName($InstalledExe)
if (-not $installDir -or -not $exeName) {
  Write-Log 'ERROR: invalid InstalledExe argument; abort'
  exit 1
}
if (-not (Test-Path -LiteralPath $Staged)) {
  Write-Log 'ERROR: staged payload missing; abort'
  exit 1
}
if (-not (Test-Path -LiteralPath $installDir -PathType Container)) {
  Write-Log 'ERROR: install directory missing; abort'
  exit 1
}

$targetId = 0
[void][int]::TryParse($TargetPid, [ref]$targetId)

if ($targetId -gt 0) {
  $proc = Get-Process -Id $targetId -ErrorAction SilentlyContinue
  if ($proc) {
    Write-Log 'waiting for old launcher to exit'
    try { [void]$proc.WaitForExit(30000) } catch { }
    if (-not $proc.HasExited) {
      Write-Log 'old launcher still alive after 30s; force killing'
      Stop-Process -Id $targetId -Force -ErrorAction SilentlyContinue
      Start-Sleep -Milliseconds 500
    }
  }
}

$items = @()
if (Test-Path -LiteralPath $Staged -PathType Container) {
  $root = (Get-Item -LiteralPath $Staged).FullName
  $rootLen = $root.Length
  $items = @(Get-ChildItem -LiteralPath $root -Recurse -File -Force | ForEach-Object {
    [pscustomobject]@{ Src = $_.FullName; Rel = $_.FullName.Substring($rootLen).TrimStart([char]92) }
  })
} else {
  $items = @([pscustomobject]@{ Src = $Staged; Rel = $exeName })
}
Write-Log ('files to place: {0}' -f $items.Count)

$failed = 0
$main = $null
foreach ($it in $items) {
  if ($it.Rel -ieq $exeName) { $main = $it; continue }
  $dst = Join-Path $installDir $it.Rel
  $parent = [System.IO.Path]::GetDirectoryName($dst)
  if ($parent -and -not (Test-Path -LiteralPath $parent -PathType Container)) {
    [void](New-Item -ItemType Directory -Force -Path $parent)
  }
  # Never touch an existing directory at the destination (may be a junction, see rules 04).
  if (Test-Path -LiteralPath $dst -PathType Container) { Write-Log ('skip directory: ' + $it.Rel); continue }
  if (Test-Path -LiteralPath $dst) { Move-Item -Force -LiteralPath $dst -Destination ($dst + '.old') }
  Move-Item -Force -LiteralPath $it.Src -Destination $dst
  if (Test-Path -LiteralPath $dst) {
    Write-Log ('placed: ' + $it.Rel)
  } else {
    Write-Log ('FAILED: ' + $it.Rel)
    $failed++
  }
}

$mainOk = $false
if ($main) {
  $dst = Join-Path $installDir $main.Rel
  $bak = $dst + '.old'
  if (Test-Path -LiteralPath $dst) { Move-Item -Force -LiteralPath $dst -Destination $bak }
  Move-Item -Force -LiteralPath $main.Src -Destination $dst
  if (Test-Path -LiteralPath $dst) {
    $mainOk = $true
    Write-Log 'main program replaced'
  } else {
    Write-Log 'ERROR: main program replace failed; rolling back'
    if (Test-Path -LiteralPath $bak) { Move-Item -Force -LiteralPath $bak -Destination $dst }
    $failed++
  }
} else {
  Write-Log 'ERROR: main program not found in staged payload'
  $failed++
}

if ($mainOk) {
  if ($Relaunch -eq '1') {
    Start-Process -FilePath (Join-Path $installDir $exeName) -WorkingDirectory $installDir
    Write-Log 'new launcher started'
  } else {
    Write-Log ('relaunch suppressed (relaunch={0}); launcher binary updated only, no auto start' -f $Relaunch)
  }
}

# Keep the staged payload when anything failed, so the user can retry.
if (Test-Path -LiteralPath $Staged -PathType Container) {
  if ($failed -eq 0) {
    Remove-Item -Recurse -Force -LiteralPath $Staged -ErrorAction SilentlyContinue
  } else {
    Write-Log 'staged payload kept for retry (partial failure)'
  }
}
Remove-Item -Force -LiteralPath $PSCommandPath -ErrorAction SilentlyContinue
Write-Log ('done; failed={0}' -f $failed)
"##;

/// 关闭启动器时是否应**自动完成**待应用的（含 exe 的）更新。
///
/// 前置：用户此前已下载该更新（`pendingApplyVersion` 已记录 = 升级意图明确），
/// 但没点「重启并应用」就关闭。满足全部条件才自动替换：
/// - 处于 `Downloaded` 阶段且 `target` 已解析；
/// - 待应用版本 == target 版本（「跳过此版本」会清空 pending，自然不满足）；
/// - **含 exe**（纯配置包在启动期已就地应用，不在此处理）；
/// - 暂存内容就绪（解压目录 / 单 exe 文件存在）。
///
/// 返回 false 时 `close_window` 走普通关闭；pending 原样保留，下次启动仍提示。
/// 该分支仅在 Windows 生效（就地替换逻辑是 Windows 专属）。
pub fn should_auto_apply_on_close() -> bool {
    #[cfg(not(windows))]
    {
        false
    }
    #[cfg(windows)]
    {
        if cfg!(debug_assertions) {
            return false;
        }
        if phase() != UpdatePhase::Downloaded {
            return false;
        }
        let entry = match target().clone() {
            Some(e) => e,
            None => return false,
        };
        if pending_apply().as_deref() != Some(entry.version.as_str()) {
            return false;
        }
        // 纯配置包（无 exe）在启动期已就地应用，这里只处理需要替换 exe 的更新
        if entry.is_zip() && !download::staged_has_exe(&entry.version) {
            return false;
        }
        if !download::staged_ready(&entry) {
            return false;
        }
        true
    }
}

/// **进程内直接落地**：把已解压的更新内容逐文件写到目标位置（**无需退出 / 重启**）。
///
/// 用于**纯配置更新包**（zip 内不含 exe）：exe 没变，就不必走"退出后替换"的脚本流程。
/// 除 exe 自身与可能被 AI 加载的 `runtime` dll 外（realm 数据 / 缓存 / 配置补丁）都能在运行中写入。
///
/// ## 铁律（勿违背）
/// - **只写文件，绝不删除目录**：领域目录内可能存在**借用联接（junction）**，
///   `remove_dir_all` 会连带删掉联接指向的真实文件（见 rules 04）。
/// - 目标已是目录（含联接）→ **跳过并记日志**，绝不尝试替换。
///
/// 返回**未能落地**的相对路径（空 = 全部成功；非空时前端应提示"关闭启动器后重试"）。
pub fn apply_inplace(version: &str) -> Result<Vec<String>, String> {
    let exe = exe_path()?;
    let dir = exe
        .parent()
        .ok_or_else(|| be_err("common_exe_dir_unknown", ""))?
        .to_path_buf();
    let src = download::staged_dir(version)?;
    if !src.is_dir() {
        return Err(be_err("updater_apply_inplace_missing", ""));
    }
    let mut files = Vec::new();
    collect_rel_files(&src, &src, &mut files)?;

    let mut failed = Vec::new();
    for rel in files {
        let target = dir.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        if target.is_dir() {
            // ⚠️ 同名目录（可能是借用联接）绝不删除 —— 跳过
            tracing::warn!(target: "updater", "跳过：目标为目录（可能是借用联接）：{rel}");
            failed.push(rel);
            continue;
        }
        if let Some(p) = target.parent() {
            let _ = fs::create_dir_all(p);
        }
        let from = src.join(rel.replace('/', std::path::MAIN_SEPARATOR_STR));
        // `copy` 直接覆盖已存在文件（Windows CopyFile 语义），不删任何东西
        match fs::copy(&from, &target) {
            Ok(_) => {}
            Err(e) => {
                tracing::warn!(target: "updater", "落地失败（可能被占用）：{rel} —— {e}");
                failed.push(rel);
            }
        }
    }
    Ok(failed)
}

/// 递归收集 `root` 下的相对路径（`/` 分隔）
fn collect_rel_files(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), String> {
    for e in fs::read_dir(dir).map_err(|e| be_err("upd_extract_dir_read_failed", &format!("{}", e)))? {
        let e = e.map_err(|e| be_err("upd_extract_dir_read_failed", &format!("{}", e)))?;
        let p = e.path();
        if p.is_dir() {
            collect_rel_files(root, &p, out)?;
        } else {
            let rel = p
                .strip_prefix(root)
                .map_err(|_| be_err("upd_extract_dir_path_invalid", ""))?
                .to_string_lossy()
                .replace('\\', "/");
            out.push(rel);
        }
    }
    Ok(())
}

/// 静默拉起替换脚本（`powershell -File`，无窗口，独立进程组 → 不随本进程退出被回收）。
/// `relaunch` 作为第 5 个位置参数传入脚本，决定是否替换后重新拉起。
#[cfg(windows)]
fn spawn_replacer(ps1: &Path, staged: &Path, exe: &Path, log: &Path, relaunch: bool) -> Result<(), String> {
    let pid = std::process::id();
    Command::new("powershell.exe")
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-WindowStyle")
        .arg("Hidden")
        .arg("-File")
        .arg(ps1)
        .arg(pid.to_string())
        .arg(staged)
        .arg(exe)
        .arg(log)
        .arg(if relaunch { "1" } else { "0" })
        .creation_flags(CREATE_NO_WINDOW | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .map_err(|e| be_err("upd_script_spawn_failed", &format!("{}", e)))?;
    Ok(())
}

#[cfg(not(windows))]
fn spawn_replacer(_ps1: &Path, _staged: &Path, _exe: &Path, _log: &Path, _relaunch: bool) -> Result<(), String> {
    Err(be_err("upd_windows_only", ""))
}

#[cfg(test)]
mod tests {
    /// 替换脚本会被写成**无 BOM** 文件：PowerShell 5.1 按 ANSI 代码页解码它
    /// （中文 Windows = GBK）—— 任何非 ASCII 字符都会变乱码，甚至吞掉下一行代码。
    /// 所以脚本体必须永远保持纯 ASCII（路径一律走命令行参数）。
    #[test]
    fn apply_ps1_must_be_ascii() {
        assert!(
            super::APPLY_PS1.is_ascii(),
            "APPLY_PS1 含非 ASCII 字符，会在 PowerShell 5.1 下乱码"
        );
    }
}
