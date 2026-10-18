//! H3 的**非 LOD 归档**：`.vid`（视频）与 `.snd`（音频）条目表解析。
//!
//! 格式对齐官方 `CArchiveLoader`（`references/vcmi/lib/filesystem/CArchiveLoader.cpp:103-169`）：
//! - `.vid`：文件头 `u32 条目数`；每条目 `char name[40]` + `i32 offset`。
//!   条目**没有长度字段**——官方把全部 offset 收集成有序集合、`it++` 取下一位作为结束位置，
//!   最后一项以文件总长收尾。本模块沿用同一算法。
//! - `.snd`：文件头 `u32 条目数`；每条目 `char name[40]` + `i32 offset` + `i32 fullSize`。
//!   波兰版文件名没有扩展名，官方统一补 `.wav`；这里同样"取首个 NUL 之前 + .wav"。
//!
//! 用途：历代记（Chronicles）导入时按官方 `chroniclesextractor.cpp` 把 `xVideo.vid` /
//! `xSound.snd` 的条目解到 mod 的 `Video/` / `Sounds/` 目录。

use std::fs;
use std::io::Read;
use std::path::Path;

/// 归档条目：`(名称, 偏移, 字节数)`
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveEntry {
    pub name: String,
    pub offset: u64,
    pub size: u64,
}

/// 读 `.vid` 条目表（名字取首个 NUL 之前，长度由相邻 offset 推导）。
pub fn vid_entries(path: &Path) -> Option<Vec<ArchiveEntry>> {
    let mut f = fs::File::open(path).ok()?;
    let total = f.metadata().ok()?.len();
    let mut head = [0u8; 4];
    f.read_exact(&mut head).ok()?;
    let count = u32::from_le_bytes(head) as usize;
    if count == 0 || count > (1 << 20) {
        return None;
    }
    let mut raw: Vec<(String, u64)> = Vec::with_capacity(count);
    let mut buf = vec![0u8; count * 44];
    f.read_exact(&mut buf).ok()?;
    for i in 0..count {
        let base = i * 44;
        let name = cstr_name(&buf[base..base + 40]);
        let off = u32::from_le_bytes(buf[base + 40..base + 44].try_into().ok()?) as u64;
        raw.push((name, off));
    }
    // 官方口径：offset 有序集合 + 文件总长 ⇒ 区间即长度
    let mut offsets: Vec<u64> = raw.iter().map(|(_, o)| *o).collect();
    offsets.sort_unstable();
    offsets.push(total);
    let mut out: Vec<ArchiveEntry> = Vec::with_capacity(count);
    for (name, off) in raw {
        let idx = offsets.iter().position(|o| *o == off)?;
        let end = offsets.get(idx + 1).copied()?;
        out.push(ArchiveEntry {
            name,
            offset: off,
            size: end.saturating_sub(off),
        });
    }
    Some(out)
}

/// 读 `.snd` 条目表（名字补 `.wav`，长度字段直接给出）。
pub fn snd_entries(path: &Path) -> Option<Vec<ArchiveEntry>> {
    let mut f = fs::File::open(path).ok()?;
    let mut head = [0u8; 4];
    f.read_exact(&mut head).ok()?;
    let count = u32::from_le_bytes(head) as usize;
    if count == 0 || count > (1 << 20) {
        return None;
    }
    let mut buf = vec![0u8; count * 48];
    f.read_exact(&mut buf).ok()?;
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let base = i * 48;
        let stem = cstr_name(&buf[base..base + 40]);
        let off = u32::from_le_bytes(buf[base + 40..base + 44].try_into().ok()?) as u64;
        let size = u32::from_le_bytes(buf[base + 44..base + 48].try_into().ok()?) as u64;
        out.push(ArchiveEntry {
            name: format!("{}.wav", stem.to_lowercase()),
            offset: off,
            size,
        });
    }
    Some(out)
}

/// 定长 char 数组 → 首个 NUL 之前的字符串。
fn cstr_name(buf: &[u8]) -> String {
    let end = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    String::from_utf8_lossy(&buf[..end]).trim().to_string()
}

/// 按偏移读取一段字节（vid / snd 通用）。
pub fn read_entry(path: &Path, e: &ArchiveEntry) -> Option<Vec<u8>> {
    use std::io::Seek;
    let mut f = fs::File::open(path).ok()?;
    f.seek(std::io::SeekFrom::Start(e.offset)).ok()?;
    let mut buf = vec![0u8; e.size as usize];
    f.read_exact(&mut buf).ok()?;
    Some(buf)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("vcmi-fv-archive-{}", name));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn vid_sizes_derived_from_offsets() {
        let d = tmp("vid");
        let p = d.join("x.vid");
        let mut f = fs::File::create(&p).unwrap();
        // 头：2 条
        f.write_all(&2u32.to_le_bytes()).unwrap();
        let mut n1 = [0u8; 40];
        n1[..3].copy_from_slice(b"A.B");
        f.write_all(&n1).unwrap();
        f.write_all(&100u32.to_le_bytes()).unwrap(); // offset 100
        let mut n2 = [0u8; 40];
        n2[..3].copy_from_slice(b"C.D");
        f.write_all(&n2).unwrap();
        f.write_all(&130u32.to_le_bytes()).unwrap(); // offset 130
        // 数据区：头 4 + 条目 44×2 = 92；补到总长 160 ⇒ 末项长度 30
        f.write_all(&vec![0u8; 160 - 92]).unwrap();
        drop(f);

        let entries = vid_entries(&p).expect("vid 解析");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "A.B");
        assert_eq!(entries[0].offset, 100);
        assert_eq!(entries[0].size, 30, "长度 = 下一个 offset - 本 offset");
        assert_eq!(entries[1].size, 30, "最后一项长度 = 文件总长 - offset");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn snd_names_get_wav_and_size_from_field() {
        let d = tmp("snd");
        let p = d.join("x.snd");
        let mut f = fs::File::create(&p).unwrap();
        f.write_all(&1u32.to_le_bytes()).unwrap();
        let mut n = [0u8; 40];
        n[..4].copy_from_slice(b"HORN");
        f.write_all(&n).unwrap();
        f.write_all(&48u32.to_le_bytes()).unwrap();
        f.write_all(&7u32.to_le_bytes()).unwrap();
        f.write_all(b"xxxxxxx").unwrap();
        drop(f);

        let entries = snd_entries(&p).expect("snd 解析");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "horn.wav", "名字补 .wav 并小写（官方同款）");
        assert_eq!(entries[0].size, 7);
        let _ = fs::remove_dir_all(&d);
    }
}
