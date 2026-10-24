//! Inno Setup 安装器的**只读**格式探测：版本 / 头部位置 / 数据区起点 / 压缩算法。
//!
//! 用途：历代记（Chronicles）离线包要"内建解包"还是"外部 CLI"，取决于压缩算法——
//! zlib 可直接用现有 `flate2`（零新依赖），LZMA 则需引入解码器。本模块只读探测、不写盘。
//!
//! # 依据（为什么这样判定）
//!
//! 官方启动器把 innoextract 作为库编入（`references/vcmi/launcher/lib/innoextract`，
//! 本仓库子模块为空，且上游依赖 Boost 1.74+ —— 不可直接移植）。因此不去猜 Inno 头部的
//! 内部结构，只用**可从上游源码核对的固定事实**：
//!
//! 1. **版本串**：Inno 安装器在文件末尾附近写入 `Inno Setup Setup Data (x.y.z)`。
//!    实测 `setup_heroes_chronicles.exe` 命中于偏移 680,503,955（文件 681,359,670 字节）。
//! 2. **数据区起点**：Inno 把压缩数据紧跟在 PE 映像之后（文件尾是设置头）。
//!    取 PE 各节 `PointerToRawData + SizeOfRawData` 的最大值即可，实测该处首字节正是块魔数。
//! 3. **压缩算法**：Inno 5.2+ 的每个数据块都以 4 字节魔数 `7A 6C 62 1A`（`zlb\0x1A`）开头
//!    （见 innoextract `src/stream/chunk.cpp`：`chunk_id = { 'z', 'l', 'b', 0x1a }`），
//!    魔数之后才是算法自带的流头，**不能**直接按首字节判类型：
//!    - 魔数后 `5D xx xx xx xx` ⇒ LZMA1：props 字节 + 4 字节字典大小（`stream/lzma.cpp`
//!      读 5 字节头，`properties = header[0]`，`dict_size = LE(header+1)`）
//!    - 魔数后 `78 01/9C/DA` 且 `(CMF*256+FLG)%31==0` ⇒ zlib（RFC1950）
//!    - 魔数后 `42 5A 68`("BZh") ⇒ bzip2
//!    没有魔数时退化为按算法流头直接判断（兼容未加壳的样本）。
//!
//! ⚠️ 本模块**只读**：不修改安装器、不写任何文件。

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

pub mod crc32;
/// 5.2.x 设置头解析（文件表 + 数据块表）。
pub mod header;
/// 纯 Rust 的 raw LZMA1 解码器。
pub mod lzma;
/// MD5（校验数据块表里记录的文件摘要）。
pub mod md5;

/// Inno 5.2+ 数据块魔数（`zlb` + `0x1A`）。
const CHUNK_MAGIC: &[u8] = b"zlb\x1a";

/// 探测结果（供日志/诊断输出；不参与落盘决策）
///
/// 其中 `header_size` / `payload_size` / `compression` / `lzma_dict_size` /
/// `first_chunk_hex` 只服务于人工核对与单测断言：真正解包走 `header.rs`，
/// 自己从设置头取版本与表结构，不依赖这些派生字段。放宽 dead_code 警告，
/// 避免为"消除警告"而删掉排障时最需要的信息。
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct InnoProbe {
    /// 形如 `5.2.3`（解析失败为空串）
    pub version: String,
    /// 是否为 Unicode 安装器（版本串带 `(u)` 后缀）
    pub unicode: bool,
    /// 版本串所在偏移（= 设置头起点）
    pub header_offset: u64,
    /// 安装器总字节数
    pub file_size: u64,
    /// 设置头跨度（`file_size - header_offset`）
    pub header_size: u64,
    /// PE 映像结束位置（= 追加数据区起点）
    pub data_start: u64,
    /// 追加数据区字节数（不含尾部设置头：`header_offset - data_start`）
    pub payload_size: u64,
    /// `zlib` / `bzip2` / `lzma1` / `lzma2` / `unknown`
    pub compression: String,
    /// LZMA 字典大小（仅 LZMA1 可从块头直接读出）
    pub lzma_dict_size: Option<u32>,
    /// 数据区首块的前 16 字节（十六进制，便于人工核对）
    pub first_chunk_hex: String,
}

const VERSION_NEEDLE: &[u8] = b"Inno Setup Setup Data (";
/// 尾部扫描窗口：版本串实测距文件尾 ~856 KB，取 4 MB 留余量
const TAIL_WINDOW: u64 = 4 * 1024 * 1024;

/// 探测一个 Inno Setup 安装器。**只读**，不会创建/修改任何文件。
pub fn probe(path: &Path) -> Result<InnoProbe, String> {
    let mut f = File::open(path).map_err(|e| format!("打开安装器失败：{e}"))?;
    let file_size = f
        .metadata()
        .map_err(|e| format!("读取文件大小失败：{e}"))?
        .len();
    if file_size < 0x1000 {
        return Err("文件过小，不像是安装器".to_string());
    }

    // ① 尾部找版本串
    let window = TAIL_WINDOW.min(file_size);
    let win_start = file_size - window;
    let mut buf = vec![0u8; window as usize];
    f.seek(SeekFrom::Start(win_start))
        .map_err(|e| format!("定位失败：{e}"))?;
    f.read_exact(&mut buf)
        .map_err(|e| format!("读取尾部失败：{e}"))?;
    let pos = buf
        .windows(VERSION_NEEDLE.len())
        .position(|w| w == VERSION_NEEDLE)
        .ok_or_else(|| "未找到 Inno Setup 版本串（可能不是 Inno 安装器）".to_string())?;
    let header_offset = win_start + pos as u64;
    // 版本字段是定长 64 字节：`Inno Setup Setup Data (5.2.3)`，Unicode 版追加 ` (u)`
    let field = &buf[pos..(pos + 64).min(buf.len())];
    let text = match field.iter().position(|b| *b == 0) {
        Some(n) => &field[..n],
        None => field,
    };
    let text = String::from_utf8_lossy(text).to_string();
    let version = text
        .split_once('(')
        .and_then(|(_, rest)| rest.split_once(')'))
        .map(|(v, _)| v.trim().to_string())
        .unwrap_or_default();
    let unicode = text.contains("(u)");
    drop(buf);

    // ② PE 映像结束位置 ⇒ 追加数据区起点
    let data_start = pe_image_end(&mut f).unwrap_or(0);

    // ③ 数据区首块特征字节（16 字节足够覆盖块魔数 + 各算法流头）
    let mut head = [0u8; 16];
    let read_ok = f
        .seek(SeekFrom::Start(data_start))
        .and_then(|_| f.read_exact(&mut head))
        .is_ok();
    let first_chunk_hex = if read_ok {
        head.iter()
            .map(|b| format!("{:02x}", b))
            .collect::<Vec<_>>()
            .join(" ")
    } else {
        String::new()
    };
    let (compression, lzma_dict_size) = classify(&head);

    Ok(InnoProbe {
        version,
        unicode,
        header_offset,
        file_size,
        header_size: file_size - header_offset,
        data_start,
        payload_size: header_offset.saturating_sub(data_start),
        compression,
        lzma_dict_size,
        first_chunk_hex,
    })
}

/// 按数据区首块判定压缩算法，返回 `(算法名, LZMA 字典大小)`。
fn classify(first: &[u8]) -> (String, Option<u32>) {
    // Inno 5.2+：先剥掉 4 字节块魔数，再按算法流头判断
    let payload = if first.len() >= 4 && &first[..4] == CHUNK_MAGIC {
        &first[4..]
    } else {
        first
    };

    // LZMA1：1 字节 props + 4 字节字典大小
    if payload.len() >= 5 {
        let props = payload[0];
        let dict = u32::from_le_bytes([payload[1], payload[2], payload[3], payload[4]]);
        if props <= 9 * 5 * 5 && dict >= 4096 && dict.is_power_of_two() {
            return ("lzma1".to_string(), Some(dict));
        }
    }
    // bzip2
    if payload.len() >= 3 && &payload[..3] == b"BZh" {
        return ("bzip2".to_string(), None);
    }
    // zlib：CMF 低 4 位 = 8（deflate），且 (CMF*256+FLG) % 31 == 0
    if payload.len() >= 2 && (payload[0] & 0x0F) == 8 {
        let check = (payload[0] as u16) * 256 + payload[1] as u16;
        if check % 31 == 0 {
            return ("zlib".to_string(), None);
        }
    }
    // LZMA2：单字节 props（字典大小由公式派生，无显式字段），放在最后避免吃掉 zlib 的 CMF
    if !payload.is_empty() && payload[0] <= 40 {
        return ("lzma2".to_string(), None);
    }
    ("unknown".to_string(), None)
}

/// PE 映像结束位置 = 各节 `PointerToRawData + SizeOfRawData` 的最大值。
fn pe_image_end(f: &mut File) -> Option<u64> {
    f.seek(SeekFrom::Start(0)).ok()?;
    let mut dos = [0u8; 0x40];
    f.read_exact(&mut dos).ok()?;
    if &dos[..2] != b"MZ" {
        return None;
    }
    let e_lfanew = u32::from_le_bytes(dos[0x3C..0x40].try_into().ok()?) as u64;
    f.seek(SeekFrom::Start(e_lfanew)).ok()?;
    let mut pe = [0u8; 24];
    f.read_exact(&mut pe).ok()?;
    if &pe[..4] != b"PE\0\0" {
        return None;
    }
    let num_sections = u16::from_le_bytes(pe[4 + 2..4 + 4].try_into().ok()?) as usize;
    let opt_size = u16::from_le_bytes(pe[4 + 16..4 + 18].try_into().ok()?) as u64;
    let section_table = e_lfanew + 4 + 20 + opt_size;
    let mut end: u64 = 0;
    for i in 0..num_sections {
        f.seek(SeekFrom::Start(section_table + (i as u64) * 40))
            .ok()?;
        let mut sec = [0u8; 40];
        f.read_exact(&mut sec).ok()?;
        let size_raw = u32::from_le_bytes(sec[16..20].try_into().ok()?) as u64;
        let ptr_raw = u32::from_le_bytes(sec[20..24].try_into().ok()?) as u64;
        end = end.max(ptr_raw + size_raw);
    }
    if end == 0 {
        None
    } else {
        Some(end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// 造一个"最小 Inno 安装器"：MZ + 1 个节 + 追加数据 + 尾部版本串。
    /// 追加数据区的首块就是真实安装器里那种 `zlb\x1a` + 算法流头。
    fn write_fake(path: &Path, chunk: &[u8]) {
        let mut f = File::create(path).unwrap();
        // DOS 头
        let mut dos = vec![0u8; 0x40];
        dos[..2].copy_from_slice(b"MZ");
        dos[0x3C..0x40].copy_from_slice(&(0x40u32).to_le_bytes());
        f.write_all(&dos).unwrap();
        // PE 头
        let mut pe = vec![0u8; 4 + 20];
        pe[..4].copy_from_slice(b"PE\0\0");
        pe[4 + 2..4 + 4].copy_from_slice(&1u16.to_le_bytes()); // 1 节
        pe[4 + 16..4 + 18].copy_from_slice(&0u16.to_le_bytes()); // 可选头长度 0
        f.write_all(&pe).unwrap();
        // 节表（1 项）：PointerToRawData=0x80, SizeOfRawData=0x100
        let mut sec = vec![0u8; 40];
        sec[16..20].copy_from_slice(&0x100u32.to_le_bytes());
        sec[20..24].copy_from_slice(&0x80u32.to_le_bytes());
        f.write_all(&sec).unwrap();
        // 补到节起始 0x80，再铺满节原始数据（0x80..0x180）
        let cur = f.stream_position().unwrap();
        f.write_all(&vec![0u8; (0x80 - cur) as usize]).unwrap();
        f.write_all(&vec![0u8; 0x100]).unwrap();
        // 追加数据区紧跟 PE 映像之后：首块写在 0x180（= data_start）
        f.write_all(chunk).unwrap();
        // 尾部：填充 + 版本串（定长 64 字节字段）
        f.write_all(&vec![0u8; 4096]).unwrap();
        let mut ver = [0u8; 64];
        let s = b"Inno Setup Setup Data (5.2.3)";
        ver[..s.len()].copy_from_slice(s);
        f.write_all(&ver).unwrap();
    }

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("vcmi-fv-inno-{}", name));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn probe_reads_version_and_zlib_chunk() {
        let d = tmp("zlib");
        let p = d.join("setup.exe");
        let mut chunk = Vec::new();
        chunk.extend_from_slice(CHUNK_MAGIC);
        chunk.extend_from_slice(&[0x78, 0x9C, 0x03, 0x00]);
        chunk.resize(16, 0);
        write_fake(&p, &chunk);

        let r = probe(&p).expect("probe");
        assert_eq!(r.version, "5.2.3");
        assert_eq!(r.compression, "zlib");
        assert_eq!(
            r.data_start, 0x180,
            "数据区起点 = 节 PointerToRawData+SizeOfRawData"
        );
        assert!(r.header_offset > r.data_start);
        assert_eq!(r.header_size, r.file_size - r.header_offset);
        assert_eq!(r.payload_size, r.header_offset - r.data_start);
        assert!(
            r.first_chunk_hex.starts_with("7a 6c 62 1a"),
            "首块应以 zlb 魔数开头"
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn probe_detects_lzma1_and_bzip2() {
        let d = tmp("other");
        // LZMA1：魔数 + props(0x5D) + 字典 32 MiB
        let mut lz = Vec::new();
        lz.extend_from_slice(CHUNK_MAGIC);
        lz.push(0x5D);
        lz.extend_from_slice(&0x0200_0000u32.to_le_bytes());
        lz.resize(16, 0);
        let p1 = d.join("lzma.exe");
        write_fake(&p1, &lz);
        let r1 = probe(&p1).unwrap();
        assert_eq!(r1.compression, "lzma1");
        assert_eq!(r1.lzma_dict_size, Some(0x0200_0000));

        let mut bz = Vec::new();
        bz.extend_from_slice(CHUNK_MAGIC);
        bz.extend_from_slice(b"BZh9");
        bz.resize(16, 0);
        let p2 = d.join("bz.exe");
        write_fake(&p2, &bz);
        assert_eq!(probe(&p2).unwrap().compression, "bzip2");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 没有块魔数时退化为按算法流头判断（兼容非 Inno 5.2+ 布局）。
    #[test]
    fn probe_classifies_bare_payload() {
        let d = tmp("bare");
        let mut chunk = vec![0x78, 0x9C];
        chunk.resize(16, 0);
        let p = d.join("bare.exe");
        write_fake(&p, &chunk);
        assert_eq!(probe(&p).unwrap().compression, "zlib");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn probe_rejects_non_inno() {
        let d = tmp("noninno");
        let p = d.join("plain.exe");
        std::fs::write(&p, vec![0u8; 8192]).unwrap();
        assert!(probe(&p).is_err(), "无版本串应报错");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// 真实安装器探测（无该文件时跳过）。
    /// `INNO_SAMPLE=<安装器路径> cargo test --lib vcmi::inno -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn probe_real_installer() {
        let Ok(p) = std::env::var("INNO_SAMPLE") else {
            eprintln!("跳过：未设置 INNO_SAMPLE");
            return;
        };
        let r = probe(Path::new(&p)).expect("probe real");
        println!(
            "version={} unicode={} file={} header={} (+{}) data_start={} payload={} compression={} dict={:?} first={}",
            r.version,
            r.unicode,
            r.file_size,
            r.header_offset,
            r.header_size,
            r.data_start,
            r.payload_size,
            r.compression,
            r.lzma_dict_size,
            r.first_chunk_hex
        );
    }

    /// 真实安装器的**小样本解包**（无该文件时跳过）：验证 LZMA1 块解码 + MD5 自校验。
    /// `INNO_SAMPLE=<安装器路径> cargo test --lib vcmi::inno -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn extracts_real_installer_sample() {
        let Ok(p) = std::env::var("INNO_SAMPLE") else {
            eprintln!("跳过：未设置 INNO_SAMPLE");
            return;
        };
        let out = std::env::temp_dir().join("vcmi-fv-inno-extract");
        let _ = std::fs::remove_dir_all(&out);
        let opts = header::ExtractOptions {
            skip_tmp: false,
            only_containing: Some("gog_png".to_string()),
            ..Default::default()
        };
        let rep = header::extract(Path::new(&p), &out, &opts).expect("extract");
        println!(
            "written={} bytes={} skipped={} failed={:?} md5_mismatch={:?}",
            rep.written, rep.bytes, rep.skipped, rep.failed, rep.md5_mismatch
        );
        assert!(rep.written >= 3, "应解出至少 3 个 gog_png*.png");
        assert!(rep.failed.is_empty(), "不应有失败条目");
        assert!(rep.md5_mismatch.is_empty(), "MD5 必须与安装器记录一致");
        let png = out.join("gog_png1.png");
        assert_eq!(std::fs::metadata(&png).unwrap().len(), 70);
        let _ = std::fs::remove_dir_all(&out);
    }

    /// 真实安装器**全量解包**（需要显式给输出目录，避免单测默认写 700MB）：
    /// `INNO_SAMPLE=<安装器> INNO_EXTRACT_ALL_DIR=<目录> cargo test --lib vcmi::inno -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn extracts_real_installer_all() {
        let Ok(p) = std::env::var("INNO_SAMPLE") else {
            eprintln!("跳过：未设置 INNO_SAMPLE");
            return;
        };
        let Ok(dir) = std::env::var("INNO_EXTRACT_ALL_DIR") else {
            eprintln!("跳过：未设置 INNO_EXTRACT_ALL_DIR");
            return;
        };
        let out = std::path::PathBuf::from(dir);
        let start = std::time::Instant::now();
        let rep = header::extract(Path::new(&p), &out, &header::ExtractOptions::default())
            .expect("extract all");
        let secs = start.elapsed().as_secs_f64();
        println!(
            "written={} bytes={} ({:.0} MiB) skipped={} failed={} md5_mismatch={} elapsed={:.1}s",
            rep.written,
            rep.bytes,
            rep.bytes as f64 / 1048576.0,
            rep.skipped,
            rep.failed.len(),
            rep.md5_mismatch.len(),
            secs
        );
        for (name, why) in rep.failed.iter().take(10) {
            println!("  FAIL {name}: {why}");
        }
        for name in rep.md5_mismatch.iter().take(10) {
            println!("  MD5  {name}");
        }
        assert!(
            rep.failed.is_empty(),
            "不应有失败条目：{:?}",
            rep.failed.len()
        );
        assert!(rep.md5_mismatch.is_empty(), "MD5 全部应一致");
        // 未解出的条目只应是需要 exe filter 的 PE 文件（按默认选项跳过）
        let listing = header::list(Path::new(&p)).expect("list");
        assert_eq!(
            rep.written + rep.skipped,
            listing.files.len(),
            "写出 + 跳过 应等于文件表条目数"
        );
        for must in [
            "Data/bitmap.lod",
            "Data/sprite.lod",
            "Warlords of the Wasteland/Data/xBitmap.lod",
            "The Sword of Frost/data/xBitmap.lod",
        ] {
            assert!(out.join(must).is_file(), "缺少 {must}");
        }
        assert!(
            std::fs::read_dir(out.join("MP3"))
                .map(|d| d.count())
                .unwrap_or(0)
                > 0,
            "MP3 目录不应为空"
        );
    }

    /// 真实安装器的**文件表**解析（无该文件时跳过）。阶段 1 的验收点：
    /// 应用名正确 + 文件条目数与解压总大小可读。
    /// `INNO_SAMPLE=<安装器路径> cargo test --lib vcmi::inno -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn lists_real_installer_files() {
        let Ok(p) = std::env::var("INNO_SAMPLE") else {
            eprintln!("跳过：未设置 INNO_SAMPLE");
            return;
        };
        let listing = header::list(Path::new(&p)).expect("list real");
        let total: u64 = listing.files.iter().map(|f| f.size).sum();
        println!(
            "version={} app={:?} publisher={:?} header_compression={} files={} total_unpacked={} MiB",
            listing.version,
            listing.app_name,
            listing.publisher,
            listing.compression,
            listing.files.len(),
            total / (1024 * 1024)
        );

        // 按 `{app}\<篇名>` 归组：官方启动器就是靠 app/<chronicle-name> 识别装了哪几篇
        let mut groups: std::collections::BTreeMap<String, (usize, u64)> = Default::default();
        for f in &listing.files {
            let tail = f.destination.split_once('\\').map(|(_, t)| t).unwrap_or("");
            let key = tail.split('\\').next().unwrap_or("").to_string();
            let e = groups.entry(key).or_insert((0, 0));
            e.0 += 1;
            e.1 += f.size;
        }
        for (name, (n, bytes)) in &groups {
            println!(
                "  {:<32} {:>4} 文件 {:>8} MiB",
                name,
                n,
                bytes / (1024 * 1024)
            );
        }
        for f in listing
            .files
            .iter()
            .filter(|f| f.destination.contains(".lod"))
        {
            println!("  LOD {:>10} {}", f.size, f.destination);
        }

        assert_eq!(listing.app_name, "Heroes Chronicles");
        assert_eq!(listing.publisher, "GOG.com");
        assert!(!listing.files.is_empty(), "文件表不应为空");

        // 8 篇各占一个目录（与官方 chroniclesextractor 的 app/<name> 识别口径一致），
        // 且每篇都有自己的 Data/*.lod
        const CHAPTERS: [&str; 8] = [
            "Warlords of the Wasteland",
            "Conquest of the Underworld",
            "Masters of the Elements",
            "Clash of the Dragons",
            "The World Tree",
            "The Fiery Moon",
            "Revolt of the Beastmasters",
            "The Sword of Frost",
        ];
        for chapter in CHAPTERS {
            assert!(groups.contains_key(chapter), "缺少篇目目录：{chapter}");
            let prefix = format!("{{app}}\\{chapter}\\data\\");
            assert!(
                listing.files.iter().any(|f| f
                    .destination
                    .to_lowercase()
                    .starts_with(&prefix.to_lowercase())
                    && f.destination.to_lowercase().ends_with(".lod")),
                "篇目 {chapter} 缺少 Data/*.lod"
            );
        }
        assert!(
            listing.files.iter().any(|f| f
                .destination
                .eq_ignore_ascii_case("{app}\\Data\\bitmap.lod")),
            "缺少基础 bitmap.lod"
        );
    }
}
