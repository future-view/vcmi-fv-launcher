//! 哈希工具：SHA-256 摘要与字节 → 十六进制。
//!
//! 汇总原先 `repository::sha256_file` 与 `realms::to_hex` 两份实现
//! （见 `docs/architecture/backend.zh-CN.md` §4.4）。

use crate::infra::berror::be_err;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// 字节序列 → 小写十六进制串。
pub fn to_hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// 文件的 SHA-256 十六进制摘要（流式读取，64 KiB 缓冲，避免整包入内存）。
pub fn sha256_file(path: &Path) -> Result<String, String> {
    let mut f =
        fs::File::open(path).map_err(|e| be_err("infra_file_read_failed", &format!("{}", e)))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f
            .read(&mut buf)
            .map_err(|e| be_err("infra_file_read_failed", &format!("{}", e)))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(to_hex(&hasher.finalize()))
}

/// 计算 mod 的 sourceHash（**基于文件内容**，不依赖 mtime）。用于缓存失效判断。
///
/// 为什么必须用内容而非 mtime：mtime 跨设备/拷贝/解压后必然变化
/// （ZIP 仅存 2 秒精度，纳秒级比对必然失配），会让"随包携带的缓存"全部失效——
/// 而失效路径还会清空 `images/` 并重建，使便携化彻底落空。
/// 改为内容哈希后，只要 mod 内容一致（换机器 / 换路径 / 换盘符），缓存即可命中。
///
/// 采样策略（兼顾正确性与启动开销——652 个 mod 每次启动都要算一次）：
/// - `mod.json` / `Content/mod.json`：**全文哈希**（体积很小，通常是权威的版本/元数据）；
/// - `content.zip`：`大小 + 首尾各 64 KiB 采样`（避免每次读整个压缩包，可能数十 MB）；
/// - `Content/config/**`：**(相对路径, 字节数) 有序列表**（见 [`add_config_tree`]）。
///   2026-10-15 补上——此前只覆盖 mod.json 与 zip，而配置散放在磁盘的发行版
///   （本仓库实测 10,329 个 JSON）改配置后 sourceHash 不变 → 缓存静默返回旧数据。
///
/// 局限：捕捉不到"仅中间字节变化且大小不变"的极端情形（属可接受折中）；
/// 需要绝对精确时可 `--fresh` / `VCMI_WIKI_REBUILD_FRESH=1` 强制重建。
///
/// 哈希用自实现的 FNV-1a（而非 `DefaultHasher`）：实现稳定、不随工具链版本变化，
/// 避免升级 Rust 后整库缓存意外全量失效。
/// FNV-1a 64 位：**实现稳定、不随工具链版本变化**，故缓存键可跨 Rust 升级保持稳定
/// （`DefaultHasher` 无此保证，升级工具链会让整库缓存意外全量失效）。
pub(crate) fn fnv1a(bytes: &[u8], h: &mut u64) {
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    for b in bytes {
        *h ^= *b as u64;
        *h = h.wrapping_mul(FNV_PRIME);
    }
}

/// 一次性 FNV-1a（内容寻址缓存用，见 `modcache::assets` 的 webp 编码缓存）。
pub(crate) fn fnv1a_of(bytes: &[u8]) -> u64 {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    let mut h = FNV_OFFSET;
    fnv1a(bytes, &mut h);
    h
}

pub(crate) fn compute_source_hash(mod_path: &Path) -> String {
    const SAMPLE: usize = 64 * 1024;
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    // 小文件：全文哈希
    fn add_full(p: &Path, h: &mut u64) {
        if let Ok(bytes) = fs::read(p) {
            fnv1a(&bytes, h);
        }
    }
    // 大文件：大小 + 首尾片段采样
    fn add_sampled(p: &Path, h: &mut u64) {
        use std::io::{Read, Seek, SeekFrom};
        let Ok(md) = fs::metadata(p) else { return };
        let len = md.len();
        fnv1a(&len.to_le_bytes(), h);
        let Ok(mut f) = fs::File::open(p) else { return };
        let mut head = vec![0u8; SAMPLE];
        if let Ok(n) = f.read(&mut head) {
            fnv1a(&head[..n], h);
        }
        if len > SAMPLE as u64 && f.seek(SeekFrom::Start(len - SAMPLE as u64)).is_ok() {
            let mut tail = vec![0u8; SAMPLE];
            if let Ok(n) = f.read(&mut tail) {
                fnv1a(&tail[..n], h);
            }
        }
    }
    let mut h = FNV_OFFSET;
    add_full(&mod_path.join("mod.json"), &mut h);
    add_full(&mod_path.join("Content/mod.json"), &mut h);
    add_sampled(&mod_path.join("content.zip"), &mut h);
    // 磁盘散放的配置树（绝大多数 mod 的真实形态，见 `config_tree_fingerprint`）。
    fnv1a(&config_tree_fingerprint(mod_path).to_le_bytes(), &mut h);
    format!("{h:016x}")
}

/// 定位 mod 的磁盘配置目录：`Content/config` 优先，不存在再试 `content/config`
/// （Windows 不区分大小写；Linux 区分，故两个都要试）。
fn config_dir_of(mod_path: &Path) -> Option<PathBuf> {
    let c = mod_path.join("Content").join("config");
    if c.is_dir() {
        return Some(c);
    }
    let c = mod_path.join("content").join("config");
    if c.is_dir() {
        return Some(c);
    }
    None
}

/// `config/` 目录树指纹：**(相对路径, 字节数)** 有序列表的 FNV-1a。
///
/// 必要性（2026-10-15 定位）：旧口径只哈希 `mod.json` + `content.zip` 采样，而本类发行版
/// 把城镇/生物/英雄配置**散放在磁盘** `Content/config/` 下（实测 10,329 个 JSON，
/// 全库仅 1 个 mod 用 content.zip）→ 用户升级/替换 mod 配置后 `sourceHash` 不变、
/// 缓存判为命中 → **静默返回旧数据**。
///
/// 只取元数据、不读文件内容：`read_dir` 在 Windows 上随目录枚举返回文件大小，
/// 代价约等于一次列目录（全库 1 万条目 ≈ 毫秒级），而每次启动都要对全部 mod 算一遍。
/// 覆盖：文件增删改名、以及"替换成不同大小的文件"；同大小改内容仍要靠 `--fresh`。
///
/// 返回 `0` 表示该 mod 没有磁盘散放的配置（纯 zip 形态）。
///
/// 另一用途：作为会话级解析缓存的失效键（见 `modcache::collect`）——改配置即失效，
/// 不会出现"改了配置但重建仍用旧解析结果"。
pub(crate) fn config_tree_fingerprint(mod_path: &Path) -> u64 {
    // 防御：异常巨大的目录不无限收集（正常 mod 的配置远低于此）
    const MAX_ENTRIES: usize = 50_000;
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    let Some(root) = config_dir_of(mod_path) else {
        return 0;
    };
    let mut items: Vec<(String, u64)> = Vec::new();
    collect_config_entries(&root, &root, &mut items, MAX_ENTRIES);
    // 排序：read_dir 顺序在不同文件系统/平台上不稳定，必须归一后才可作指纹
    items.sort();
    let mut h = FNV_OFFSET;
    for (rel, len) in items {
        fnv1a(rel.as_bytes(), &mut h);
        fnv1a(&len.to_le_bytes(), &mut h);
    }
    h
}

/// 递归收集 `root` 下条目的 (相对路径, 大小)。
fn collect_config_entries(root: &Path, dir: &Path, out: &mut Vec<(String, u64)>, max: usize) {
    if out.len() >= max {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        if out.len() >= max {
            return;
        }
        let p = e.path();
        let rel = p
            .strip_prefix(root)
            .unwrap_or(&p)
            .to_string_lossy()
            .replace('\\', "/");
        if p.is_dir() {
            collect_config_entries(root, &p, out, max);
        } else {
            let len = e.metadata().ok().map(|m| m.len()).unwrap_or(0);
            out.push((rel, len));
        }
    }
}
