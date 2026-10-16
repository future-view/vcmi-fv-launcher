//! h3m/h3c 解析器批量验证工具（独立 example，不依赖 lib crate 的私有模块可见性）。
//!
//! 复用 `src/infra/h3m.rs`（只依赖 `crate::infra::berror`，见下方 shim），
//! 对照 `homm3-map-translator` 语料库的目录语义做三重验证：
//! 1. **解析成功率**：所有 `.h3m` / `.h3c` 必须成功产出 MapMeta；
//! 2. **语言识别**：目录语言标签（`cs-CZ` 等 BCP-47）→ VCMI identifier，
//!    与解析器自检结果比对；
//! 3. **战役完整性**：`h3c` 的 `scenario_count` ≥ 1，且场景名不含 UTF-8 替换符
//!    （U+FFFD 乱码回归护栏：regionName 必须与战役名同走编码判决管线）。
//!
//! 用法（在 `src-tauri/` 或仓库根目录执行均可，默认语料路径自动探测）：
//! ```text
//! cargo run --release --example h3m_check -- [语料目录] [--sample N] [--lang-strict] [--verbose]
//! ```
//! - `--sample N`：每个「版本×类型×语言」组合最多抽 N 个文件（默认 0 = 全量）
//! - `--lang-strict`：语言不一致也计入失败（默认仅报告）
//! - 退出码：0 全部通过；1 存在解析失败（或 strict 下语言不一致）
// `h3m.rs` 用 `crate::infra::berror::be_err` 返回结构化错误码；本例用 `#[path]` 直接引入源文件，
// 此时 crate 根即本文件，故需按**同一模块路径**提供该依赖（引用同一份实现，不是复制）。
mod infra {
    // 内联模块里的 `#[path]` 基准目录是 `examples/infra/`，故需上溯两级到 crate 根
    #[path = "../../src/infra/berror.rs"]
    pub mod berror;
}
#[path = "../src/infra/h3m.rs"]
mod h3m;

use h3m::{parse_file, to_vcmi_lang};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

fn main() -> ExitCode {
    let mut corpus: Option<String> = None;
    let mut sample: usize = 0;
    let mut lang_strict = false;
    let mut verbose = false;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--sample" => sample = args.next().and_then(|v| v.parse().ok()).unwrap_or(0),
            "--lang-strict" => lang_strict = true,
            "--verbose" => verbose = true,
            other => corpus = Some(other.into()),
        }
    }

    let corpus = match corpus.map(PathBuf::from) {
        Some(p) if p.is_dir() => p,
        _ => match default_corpus() {
            Some(p) => p,
            None => {
                eprintln!("未找到语料目录。用法: h3m_check <目录> [--sample N] [--lang-strict] [--verbose]");
                return ExitCode::from(2);
            }
        },
    };

    // 收集文件：按「版本/类型/语言」分组后排序，保证抽样确定性。
    // 语料布局：<ROE|AB|SOD|HOTA|WOG>/{map,campaign}/<BCP-47>/…/*.h3m|h3c
    let mut groups: BTreeMap<(String, String, String), Vec<PathBuf>> = BTreeMap::new();
    let mut seen = 0usize;
    collect(&corpus, 0, "", &mut |path: &PathBuf, rel| {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())?
            .to_ascii_lowercase();
        if !matches!(ext.as_str(), "h3m" | "h3c") {
            return None;
        }
        let mut parts = Path::new(rel).components();
        let version = parts.next()?.as_os_str().to_string_lossy().into_owned();
        let kind_dir = parts.next()?.as_os_str().to_string_lossy().into_owned();
        let kind = if kind_dir == "campaign" {
            "campaign"
        } else {
            "map"
        };
        let lang_dir = parts.next()?.as_os_str().to_string_lossy().into_owned();
        groups
            .entry((version, kind.to_string(), lang_dir))
            .or_default()
            .push(path.clone());
        seen += 1;
        if seen % 500 == 0 {
            eprintln!("…扫描 {seen} 个文件");
        }
        Some(())
    });
    if groups.is_empty() {
        eprintln!("目录 {} 下未发现 h3m/h3c 文件", corpus.display());
        return ExitCode::from(2);
    }

    let started = Instant::now();
    let mut total = 0usize;
    let mut fail = 0usize;
    let mut lang_miss = 0usize;
    let mut hard_failures: Vec<String> = Vec::new();
    let mut lang_issues: Vec<String> = Vec::new();
    let mut name_miss_total = 0usize;
    let mut garble_total = 0usize;
    println!(
        "{:<5} {:<9} {:<6} {:>6} {:>6} {:>6} {:>8} {:>8} {:>8}",
        "ver", "kind", "lang", "files", "ok", "fail", "langMiss", "nameMiss", "garble"
    );
    for ((version, kind, lang_dir), files) in &mut groups {
        files.sort();
        let picked: &[PathBuf] = if sample > 0 && files.len() > sample {
            let step = files.len() / sample;
            &(0..sample)
                .map(|i| files[i * step].clone())
                .collect::<Vec<_>>()
        } else {
            &files
        };
        let (mut ok, mut bad, mut miss, mut name_miss) = (0usize, 0usize, 0usize, 0usize);
        let mut garble = 0usize;
        let expected = to_vcmi_lang(&lang_dir);
        for path in picked {
            total += 1;
            match parse_file(path) {
                Ok(meta) => {
                    ok += 1;
                    let mut line_bad = false;
                    if meta.name.trim().is_empty() {
                        name_miss += 1;
                        line_bad = true;
                        hard_failures.push(format!("{}: 地图名为空", path.display()));
                    }
                    if meta.lang != expected {
                        miss += 1;
                        lang_miss += 1;
                        line_bad = true;
                        lang_issues.push(format!(
                            "{}: lang={} 期望 {}",
                            path.display(),
                            meta.lang,
                            expected
                        ));
                    }
                    if kind == "campaign" {
                        if meta.scenario_count < 1 {
                            bad += 1;
                            fail += 1;
                            hard_failures.push(format!(
                                "{}: 战役场景数为 {}",
                                path.display(),
                                meta.scenario_count
                            ));
                        }
                        // 场景名乱码（UTF-8 替换符 U+FFFD）：编码判决 / 解码管线回归护栏
                        let garbled = meta
                            .scenarios
                            .iter()
                            .filter(|s| s.name.contains('\u{FFFD}'))
                            .count();
                        if garbled > 0 {
                            bad += 1;
                            fail += 1;
                            garble += 1;
                            hard_failures.push(format!(
                                "{}: {} 个场景名乱码（U+FFFD）",
                                path.display(),
                                garbled
                            ));
                        }
                    }
                    if verbose {
                        let name: String = meta
                            .name
                            .chars()
                            .take(30)
                            .collect::<String>()
                            .replace(['\r', '\n'], " ");
                        println!(
                            "  {version} | {kind} | {lang_dir} | lang={} enc={} | {} | {}",
                            meta.lang,
                            meta.encoding,
                            name,
                            path.display()
                        );
                    } else if line_bad && hard_failures.len() + lang_issues.len() > 200 {
                        break;
                    }
                }
                Err(e) => {
                    bad += 1;
                    fail += 1;
                    hard_failures.push(format!("{}: {}", path.display(), e));
                }
            }
        }
        name_miss_total += name_miss;
        garble_total += garble;
        println!(
            "{version:<5} {kind:<9} {lang_dir:<6} {:>6} {ok:>6} {bad:>6} {miss:>8} {name_miss:>8} {garble:>8}",
            picked.len()
        );
    }

    println!(
        "\n合计: {} 文件, {} 失败, {} 语言不一致, {} 空名, {} 乱码场景 — 耗时 {:.1}s",
        total,
        fail,
        lang_miss,
        name_miss_total,
        garble_total,
        started.elapsed().as_secs_f32()
    );
    if !hard_failures.is_empty() || !lang_issues.is_empty() {
        println!("\n硬错误（解析失败/空名）{} 条:", hard_failures.len());
        for f in &hard_failures {
            println!("  - {f}");
        }
        let shown = lang_issues.len().min(30);
        println!("\n语言不一致（前 {shown}/{} 条）:", lang_issues.len());
        for f in lang_issues.iter().take(30) {
            println!("  - {f}");
        }
    }
    if fail > 0 || (lang_strict && lang_miss > 0) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

/// 从常见工作目录探测默认语料位置（仓库根 / src-tauri）。
fn default_corpus() -> Option<PathBuf> {
    for rel in [
        "../homm3-map-translator/Maps",
        "../../homm3-map-translator/Maps",
    ] {
        let p = PathBuf::from(rel);
        if p.is_dir() {
            return Some(p);
        }
    }
    None
}

/// 递归遍历（限深度，跳过不可读目录）；`rel` 为相对语料根的完整路径。
fn collect(
    dir: &Path,
    depth: usize,
    prefix: &str,
    out: &mut impl FnMut(&PathBuf, &str) -> Option<()>,
) {
    if depth > 12 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<(PathBuf, bool)> = entries
        .flatten()
        .map(|e| {
            let p = e.path();
            let is_dir = p.is_dir();
            (p, is_dir)
        })
        .collect();
    paths.sort();
    for (path, is_dir) in paths {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if is_dir {
            collect(&path, depth + 1, &rel, out);
        } else {
            let _ = out(&path, &rel);
        }
    }
}
