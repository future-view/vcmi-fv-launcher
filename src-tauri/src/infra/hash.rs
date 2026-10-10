//! 哈希工具：SHA-256 摘要与字节 → 十六进制。
//!
//! 汇总原先 `repository::sha256_file` 与 `realms::to_hex` 两份实现
//! （见 `docs/architecture/backend.zh-CN.md` §4.4）。

use crate::infra::berror::be_err;
use std::fs;
use std::io::Read;
use std::path::Path;

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
    let mut f = fs::File::open(path).map_err(|e| be_err("infra_file_read_failed", &format!("{}", e)))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = f.read(&mut buf).map_err(|e| be_err("infra_file_read_failed", &format!("{}", e)))?;
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
/// - `content.zip`：`大小 + 首尾各 64 KiB 采样`（避免每次读整个压缩包，可能数十 MB）。
///
/// 局限：捕捉不到"仅中间字节变化且大小不变"的极端情形（属可接受折中）；
/// 需要绝对精确时可 `--fresh` / `VCMI_WIKI_REBUILD_FRESH=1` 强制重建。
///
/// 哈希用自实现的 FNV-1a（而非 `DefaultHasher`）：实现稳定、不随工具链版本变化，
/// 避免升级 Rust 后整库缓存意外全量失效。
pub(crate) fn compute_source_hash(mod_path: &Path) -> String {
    const SAMPLE: usize = 64 * 1024;
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;
    fn fnv1a(bytes: &[u8], h: &mut u64) {
        for b in bytes {
            *h ^= *b as u64;
            *h = h.wrapping_mul(FNV_PRIME);
        }
    }
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
    format!("{h:016x}")
}
