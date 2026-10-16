//! H3 LOD 档案读取器（Data/H3sprite.lod、H3bitmap.lod 等）。
//! 格式依据 VCMI 源码 `CArchiveLoader::initLODArchive`（references/vcmi/lib/filesystem/CArchiveLoader.cpp）：
//! - offset 0x08: u32 文件总数
//! - offset 0x5C 起，每项 32 字节：name[16] + offset(u32) + fullSize(u32) + unused(u32) + compressedSize(u32)
//! - compressedSize == fullSize 时为未压缩数据；否则为 zlib 流
//!
//! 用途：H3 原版界面/图标 DEF（SMALRES、UN32/UN44、stackWindow/* 等）不在磁盘散文件中，
//! 而是打包在 lod 内，启动器需要自行解包后交给 DefReader 解码。

use flate2::read::ZlibDecoder;
use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// lod 目录表缓存（key = 路径 + (mtime, len) 指纹，容量上限见 `CFG_CACHE_CAP`）。
/// 游戏目录的 lod 候选只有 3 个（H3sprite / H3ab_spr / H3bitmap），容量 4 足够。
static LOD_INDEX_CACHE: OnceLock<Mutex<Vec<(String, Arc<HashMap<String, LodEntry>>)>>> =
    OnceLock::new();

#[derive(Clone, Copy)]
struct LodEntry {
    offset: u32,
    full_size: u32,
    compressed_size: u32,
}

/// 已打开的 lod 档案（仅持有路径与目录表，读取时按需 seek）。
pub struct LodArchive {
    path: PathBuf,
    /// 目录表：跨实例共享（缓存命中时零拷贝克隆 Arc）
    entries: Arc<HashMap<String, LodEntry>>,
}

/// 释放 lod 目录表缓存（构建结束调用；见 [`crate::infra::cache_gc`]）。
pub fn release_caches() {
    crate::infra::cache::cfg_cache_clear(&LOD_INDEX_CACHE);
}

impl LodArchive {
    /// 打开 lod 并解析目录表（失败返回 None，调用方回退下一个候选 lod）。
    ///
    /// 目录表走进程级缓存：**只有首次**真正读取并解析 lod，之后按指纹命中。
    ///
    /// ⚠️ 修改前本函数每次调用都 `fs::read` **整个 lod 文件**（数百 MB）并重建目录表，
    /// 全量构建中被调用数万次（实测 `lod.open` 累计 3,775s CPU ≈ 总量的两成），
    /// 是最大的单项开销。`lod.open` / `lod.open.bytes` 埋点用于验证缓存命中效果。
    pub fn open(path: &Path) -> Option<LodArchive> {
        crate::infra::metrics::scope("lod.open", || {
            let entries = Self::cached_entries(path);
            Some(LodArchive {
                path: path.to_path_buf(),
                entries,
            })
        })
    }

    /// 目录表缓存：键为路径指纹，值共享（Arc）。未命中时持锁解析一次（避免并发惊群）。
    fn cached_entries(path: &Path) -> Arc<HashMap<String, LodEntry>> {
        let key = crate::infra::cache::fingerprint_of(&[path.to_path_buf()]);
        crate::infra::cache::cfg_cache_get(&LOD_INDEX_CACHE, key, || {
            Self::parse_entries(path).unwrap_or_default()
        })
    }

    /// 实际解析：读取整个 lod 并重建目录表（每次调用都是一次全文件读取，故必须缓存）。
    fn parse_entries(path: &Path) -> Option<HashMap<String, LodEntry>> {
        let data = fs::read(path).ok()?;
        crate::infra::metrics::bytes("lod.open.bytes", data.len() as u64);
        if data.len() < 0x5c {
            return None;
        }
        let total = u32::from_le_bytes([data[8], data[9], data[10], data[11]]) as usize;
        let mut entries = HashMap::with_capacity(total);
        let mut pos = 0x5cusize;
        for _ in 0..total {
            if pos + 32 > data.len() {
                break;
            }
            let name_bytes = &data[pos..pos + 16];
            let name_end = name_bytes.iter().position(|&b| b == 0).unwrap_or(16);
            let name = String::from_utf8_lossy(&name_bytes[..name_end]).to_uppercase();
            let rd_u32 = |off: usize| {
                u32::from_le_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
            };
            let offset = rd_u32(pos + 16);
            let full_size = rd_u32(pos + 20);
            let compressed_size = rd_u32(pos + 28);
            if !name.is_empty() {
                entries.insert(
                    name,
                    LodEntry {
                        offset,
                        full_size,
                        compressed_size,
                    },
                );
            }
            pos += 32;
        }
        Some(entries)
    }

    /// 按名称（大小写不敏感）读取条目原始字节，自动处理 zlib 解压。
    pub fn read(&self, name: &str) -> Option<Vec<u8>> {
        crate::infra::metrics::scope("lod.read", || self.read_inner(name))
    }

    fn read_inner(&self, name: &str) -> Option<Vec<u8>> {
        let key = name.trim().to_uppercase();
        let e = *self.entries.get(&key)?;
        let mut f = fs::File::open(&self.path).ok()?;
        use std::io::{Seek, SeekFrom};
        f.seek(SeekFrom::Start(e.offset as u64)).ok()?;
        let raw_size = if e.compressed_size > 0 {
            e.compressed_size
        } else {
            e.full_size
        } as usize;
        let mut raw = vec![0u8; raw_size];
        f.read_exact(&mut raw).ok()?;
        if e.compressed_size == e.full_size || e.compressed_size == 0 {
            return Some(raw);
        }
        // zlib 流：解压到 full_size
        let mut out = Vec::with_capacity(e.full_size as usize);
        ZlibDecoder::new(&raw[..]).read_to_end(&mut out).ok()?;
        if out.len() >= e.full_size as usize {
            Some(out)
        } else {
            None
        }
    }
}

/// 只读取 lod 的**条目名集合**，不读整个文件、不建常规目录表。
///
/// `parse_entries` 需要 `fs::read` 整个 lod（`H3bitmap.lod` 可达数百 MB）才能建表；
/// 而目录表位于文件头部（`0x5c + count * 32`），**只问"有没有某个条目"时不必付这份代价**
/// —— 资源体检一类只关心存在性的场景用本函数（几百 KB 级读取）。
pub fn lod_entry_names(path: &Path) -> Option<Vec<String>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = fs::File::open(path).ok()?;
    let mut header = [0u8; 0x5c];
    f.read_exact(&mut header).ok()?;
    let total = u32::from_le_bytes([header[8], header[9], header[10], header[11]]) as usize;
    // 目录表：每项 32 字节，名字占前 16 字节
    let mut table = vec![0u8; total.checked_mul(32)?];
    f.seek(SeekFrom::Start(0x5c)).ok()?;
    f.read_exact(&mut table).ok()?;
    let mut out = Vec::with_capacity(total);
    for i in 0..total {
        let chunk = &table[i * 32..i * 32 + 16];
        let end = chunk.iter().position(|&b| b == 0).unwrap_or(16);
        if end == 0 {
            continue;
        }
        out.push(String::from_utf8_lossy(&chunk[..end]).to_uppercase());
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 写一个只含目录表的"最小合法 lod"（体检只读头部，不需要真实数据段）。
    fn write_fake_lod(path: &Path, names: &[&str]) {
        let mut buf = vec![0u8; 0x5c];
        buf[8..12].copy_from_slice(&(names.len() as u32).to_le_bytes());
        for n in names {
            let mut entry = [0u8; 32];
            let b = n.as_bytes();
            let len = b.len().min(16);
            entry[..len].copy_from_slice(&b[..len]);
            buf.extend_from_slice(&entry);
        }
        fs::write(path, buf).expect("write fake lod");
    }

    #[test]
    fn lod_entry_names_reads_directory_table() {
        let dir = std::env::temp_dir().join("vcmi-fv-lod-names-test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("mkdir");
        let p = dir.join("mini.lod");
        write_fake_lod(&p, &["help.txt", "Players.PAL", "default.def"]);
        let names = lod_entry_names(&p).expect("parse entries");
        assert!(
            names.contains(&"HELP.TXT".to_string()),
            "应大写归一：{names:?}"
        );
        assert!(names.contains(&"PLAYERS.PAL".to_string()));
        assert!(names.contains(&"DEFAULT.DEF".to_string()));
        assert_eq!(names.len(), 3);
        let _ = fs::remove_dir_all(&dir);
    }
}

/// 同 `read_game_resource_ext`，但返回**实际命中的扩展名**，且在精确名未命中时按 VCMI 资源
/// 解析习惯做**基名 + 多扩展名**回退。
///
/// 用途：原版城镇背景/兵种背景在配置里常写作 `.bmp` 甚至无扩展名（如 `castle.json` 的
/// `townBackground: "TBCSBACK.bmp"`、`creatureBackground: "CRBKGCAS"`），但实际资源是
/// H3bitmap.lod 内的 `TBCSBACK.PCX` / `CRBKGCAS.PCX`。VCMI 资源定位按基名 + 扩展名灵活
/// 匹配；此处复刻该行为，使 `resolve_resource_fallback` 能取到字节并带上正确类型（pcx），
/// 下游 `encode_webp` 已支持 pcx 解码。
pub fn read_game_resource_ext(game_dir: &Path, name: &str) -> Option<(Vec<u8>, String)> {
    let norm = name.trim().replace('\\', "/");
    let file_part = norm.rsplit('/').next().unwrap_or(&norm).to_string();
    let base_name = file_part
        .split('.')
        .next()
        .unwrap_or(&file_part)
        .to_uppercase();
    let orig_ext = file_part
        .rsplit('.')
        .next()
        .map(|s| s.to_uppercase())
        .filter(|e| !e.is_empty() && e.len() <= 4);
    // 候选扩展名：原始扩展名优先，再补 VCMI 常见位图/精灵类型
    let mut exts: Vec<String> = Vec::new();
    if let Some(e) = &orig_ext {
        exts.push(e.clone());
    }
    for e in ["PCX", "BMP", "PNG", "DEF"] {
        if !exts.iter().any(|x| x == e) {
            exts.push(e.to_string());
        }
    }
    // 1) 磁盘散文件：精确名 + 基名配各扩展名
    let sprite_roots = [
        game_dir
            .join("Mods")
            .join("vcmi")
            .join("Content")
            .join("Sprites"),
        game_dir
            .join("Mods")
            .join("vcmi")
            .join("Content")
            .join("Sprites2x"),
        game_dir.join("Sprites"),
        game_dir.to_path_buf(),
        game_dir.join("Data"),
    ];
    for root in &sprite_roots {
        let orig = root.join(&norm);
        if orig.is_file() {
            if let Ok(b) = fs::read(&orig) {
                let ext = orig_ext
                    .clone()
                    .unwrap_or_else(|| "bmp".to_string())
                    .to_lowercase();
                return Some((b, ext));
            }
        }
        for e in &exts {
            let cand = root.join(format!("{}.{}", base_name, e));
            if cand.is_file() {
                if let Ok(b) = fs::read(&cand) {
                    return Some((b, e.to_lowercase()));
                }
            }
        }
    }
    // 2) lod 包：基名配各扩展名（lod 内名称为扁平大写基名）
    let lod_candidates = [
        "H3sprite.lod",
        "H3ab_spr.lod",
        "H3bitmap.lod",
        "H3ab_bmp.lod",
    ];
    for lod_name in lod_candidates {
        let lod_path = game_dir.join("Data").join(lod_name);
        if !lod_path.is_file() {
            continue;
        }
        if let Some(arch) = LodArchive::open(&lod_path) {
            for e in &exts {
                let key = format!("{}.{}", base_name, e);
                if let Some(b) = arch.read(&key) {
                    return Some((b, e.to_lowercase()));
                }
            }
        }
    }
    None
}
