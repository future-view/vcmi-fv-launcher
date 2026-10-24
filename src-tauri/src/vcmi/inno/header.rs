//! Inno Setup **5.2.x** 设置头解析与解包：应用信息 + 文件表 + 数据块表 + 解压落盘。
//!
//! 字段顺序严格对齐 innoextract（zlib 许可，`src/setup/*.cpp`）在 5.2.x 版本门限下的读取序列：
//! - `header::load`（`setup/header.cpp:146`）：28 个字符串 → lead_bytes(32) → 各表计数 →
//!   winver(10) → 颜色/口令盐/磁盘 → 若干枚举 → 压缩方式 → 架构标志 → 签名信息 → 选项位域(46 位 = 6 字节)
//! - 条目顺序（`setup/info.cpp:148`）：languages → messages → permissions → types → components →
//!   tasks → directories → files（我们到 files 为止）
//! - 数据块表在**第二个块**里（`info.cpp:233` 重新取 block_reader）
//!
//! 只支持 5.2.x：其它版本的门限不同（5.2.5 起 license/info 移位、5.3.x 增加字段），
//! 硬套会静默读错，因此直接返回明确错误而不是猜测。

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use super::crc32::crc32;
use super::{lzma, md5};

/// 文件表中的一项（已与数据块表关联）
#[derive(Debug, Clone)]
pub struct InnoFileEntry {
    /// 目标路径（Inno 的 `DestName`，相对 `{app}`，含子目录）
    pub destination: String,
    /// 源路径（`Source`，多为 `app\...` 形式）
    #[allow(dead_code)] // 诊断打印用，解包本身只需要 destination
    pub source: String,
    /// 解压后字节数
    pub size: u64,
    /// 压缩后字节数
    pub compressed_size: u64,
    /// 数据区内的绝对偏移（`data_start + chunk.offset`）
    pub offset: u64,
    /// `stored` / `zlib` / `bzip2` / `lzma1`
    pub compression: String,
    /// 数据块 MD5（小写十六进制，Inno 5.x 用于校验）
    pub md5: String,
    /// 需要 Inno 的 exe filter 才能还原原始字节（PE 调用指令优化）
    pub filtered: bool,
    /// 数据块被 ARC4 加密（带口令的安装器）
    pub encrypted: bool,
}

/// 解析结果
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct InnoListing {
    pub version: String,
    pub app_name: String,
    pub publisher: String,
    /// 头部的整体压缩方式（`stored` / `zlib` / `bzip2` / `lzma1`）
    pub compression: String,
    pub files: Vec<InnoFileEntry>,
}

/// 读取 `name` 指向的安装器并列出文件表。**只读**。
pub fn list(path: &Path) -> Result<InnoListing, String> {
    let probe = super::probe(path)?;
    let (major, minor) = parse_version_pair(&probe.version)?;
    if major != 5 || minor != 2 {
        return Err(format!(
            "暂只支持 Inno Setup 5.2.x 设置头，当前为 {}",
            probe.version
        ));
    }

    let header_start = probe.header_offset;
    let len = probe.file_size.saturating_sub(header_start + 64);
    let mut f = File::open(path).map_err(|e| format!("打开安装器失败：{e}"))?;
    f.seek(SeekFrom::Start(header_start + 64))
        .map_err(|e| format!("定位设置头失败：{e}"))?;
    let mut region = vec![0u8; len as usize];
    f.read_exact(&mut region)
        .map_err(|e| format!("读取设置头失败：{e}"))?;

    let mut pos = 0usize;
    let main_block = read_block(&region, &mut pos)?;
    let mut c = Cur::new(&main_block, probe.unicode);
    let header = parse_header(&mut c)?;
    let files = parse_entries_until_files(&mut c, &header)?;

    let data_block = read_block(&region, &mut pos)?;
    let mut dc = Cur::new(&data_block, probe.unicode);
    let data_entries = parse_data_entries(&mut dc, header.data_entry_count)?;

    let compression = compression_name(header.compression);
    let mut out = Vec::with_capacity(files.len());
    for file in files {
        let Some(data) = data_entries.get(file.location as usize) else {
            continue; // 数据表缺项（异常包）——跳过而不是伪造大小
        };
        out.push(InnoFileEntry {
            destination: file.destination,
            source: file.source,
            size: data.file_size,
            compressed_size: data.chunk_size,
            offset: probe.data_start + data.offset as u64,
            compression: if data.compressed {
                compression.to_string()
            } else {
                "stored".to_string()
            },
            md5: data.md5.clone(),
            filtered: data.filtered,
            encrypted: data.encrypted,
        });
    }

    Ok(InnoListing {
        version: probe.version,
        app_name: header.app_name,
        publisher: header.publisher,
        compression: compression.to_string(),
        files: out,
    })
}

/// 解包选项
#[derive(Debug, Clone)]
pub struct ExtractOptions {
    /// 跳过 `{tmp}` 下的安装器临时资源（EULA/BMP/PNG 等）
    pub skip_tmp: bool,
    /// 只解目标路径包含该子串的条目（大小写不敏感）
    pub only_containing: Option<String>,
    /// 最多解多少个条目
    pub limit: Option<usize>,
    /// 是否解出标记了 PE 过滤的条目（未实现 exe filter，字节与原文件不一致）
    pub include_filtered: bool,
}

impl Default for ExtractOptions {
    fn default() -> Self {
        Self {
            skip_tmp: true,
            only_containing: None,
            limit: None,
            include_filtered: false,
        }
    }
}

/// 解包结果
#[derive(Debug, Default, Clone)]
pub struct ExtractReport {
    /// 成功写盘的文件数
    pub written: usize,
    /// 写盘字节数（解压后）
    pub bytes: u64,
    /// 按选项/路径规则跳过的条目数
    pub skipped: usize,
    /// 失败条目（目标路径 + 原因）
    pub failed: Vec<(String, String)>,
    /// MD5 与原安装器记录不一致的条目
    pub md5_mismatch: Vec<String>,
}

/// 零依赖解包：按数据块表逐块 LZMA1 解码并落盘，最后用 Inno 记录的 MD5 自校验。
///
/// 路径规则：去掉 `{app}` / `{tmp}` 这类根前缀，其余按 `\` 拆成目录；含 `..` / 盘符的条目直接拒绝。
pub fn extract(
    installer: &Path,
    out_dir: &Path,
    opts: &ExtractOptions,
) -> Result<ExtractReport, String> {
    let listing = list(installer)?;
    let mut f = File::open(installer).map_err(|e| format!("打开安装器失败：{e}"))?;
    let mut report = ExtractReport::default();
    let pattern = opts.only_containing.as_ref().map(|s| s.to_lowercase());

    for entry in &listing.files {
        if let Some(pat) = &pattern {
            if !entry.destination.to_lowercase().contains(pat) {
                report.skipped += 1;
                continue;
            }
        }
        if let Some(limit) = opts.limit {
            if report.written >= limit {
                break;
            }
        }
        if opts.skip_tmp && entry.destination.to_lowercase().starts_with("{tmp}\\") {
            report.skipped += 1;
            continue;
        }
        if entry.encrypted {
            report.failed.push((
                entry.destination.clone(),
                "数据块被加密（ARC4），暂不支持".into(),
            ));
            continue;
        }
        if entry.filtered && !opts.include_filtered {
            report.skipped += 1; // 需要 exe filter 才能还原； Chronicles 所需数据文件不受影响
            continue;
        }
        let Some(rel) = safe_relative(&entry.destination) else {
            report
                .failed
                .push((entry.destination.clone(), "目标路径不安全或无法解析".into()));
            continue;
        };

        match read_chunk(&mut f, entry) {
            Ok(data) => {
                if data.len() as u64 != entry.size {
                    report.failed.push((
                        entry.destination.clone(),
                        format!("解压长度不符：期望 {} 实际 {}", entry.size, data.len()),
                    ));
                    continue;
                }
                let actual = md5::hex(&md5::digest(&data));
                if !entry.md5.is_empty() && actual != entry.md5 {
                    report.md5_mismatch.push(entry.destination.clone());
                }
                let target = out_dir.join(&rel);
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("创建目录失败 {}：{e}", parent.display()))?;
                }
                std::fs::write(&target, &data)
                    .map_err(|e| format!("写入失败 {}：{e}", target.display()))?;
                report.written += 1;
                report.bytes += data.len() as u64;
            }
            Err(e) => report.failed.push((entry.destination.clone(), e)),
        }
    }

    Ok(report)
}

/// 读一个数据块并解压（块首 4 字节为 `zlb\x1a` 魔数）。
fn read_chunk(f: &mut File, entry: &InnoFileEntry) -> Result<Vec<u8>, String> {
    let mut buf = vec![0u8; entry.compressed_size as usize + 4];
    f.seek(SeekFrom::Start(entry.offset))
        .map_err(|e| format!("定位数据块失败：{e}"))?;
    f.read_exact(&mut buf)
        .map_err(|e| format!("读取数据块失败：{e}"))?;
    if &buf[..4] != super::CHUNK_MAGIC {
        return Err("数据块魔数不符（zlb）".to_string());
    }
    match entry.compression.as_str() {
        "lzma1" => {
            lzma::decompress(&buf[4..], Some(entry.size as usize)).map_err(|e| e.to_string())
        }
        "stored" => {
            let n = entry.size as usize;
            buf.get(4..4 + n)
                .map(|s| s.to_vec())
                .ok_or_else(|| "stored 块长度不足".to_string())
        }
        other => Err(format!("不支持的块压缩方式：{other}")),
    }
}

/// 把 Inno 的目标路径（`{app}\a\b.txt`）转成安全相对路径。
fn safe_relative(destination: &str) -> Option<std::path::PathBuf> {
    let (root, rest) = destination.split_once('\\')?;
    if !root.starts_with('{') {
        return None;
    }
    let mut path = std::path::PathBuf::new();
    for seg in rest.split('\\') {
        if seg.is_empty() || seg == "." || seg == ".." || seg.contains(':') || seg.contains('/') {
            return None;
        }
        path.push(seg);
    }
    if path.as_os_str().is_empty() {
        None
    } else {
        Some(path)
    }
}

fn parse_version_pair(v: &str) -> Result<(u32, u32), String> {
    let mut it = v.split('.');
    let major = it
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("无法解析版本号：{v}"))?;
    let minor = it
        .next()
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| format!("无法解析版本号：{v}"))?;
    Ok((major, minor))
}

/// 读一个设置头块：12 字节块头 + 每 4KiB 带 CRC32 的载荷，按需 LZMA1 解压。
fn read_block(region: &[u8], pos: &mut usize) -> Result<Vec<u8>, String> {
    let hdr = region
        .get(*pos..*pos + 9)
        .ok_or_else(|| "设置头块越界".to_string())?;
    let want_crc = u32::from_le_bytes([hdr[0], hdr[1], hdr[2], hdr[3]]);
    let stored = u32::from_le_bytes([hdr[4], hdr[5], hdr[6], hdr[7]]) as usize;
    let compressed = hdr[8];
    if crc32(&hdr[4..9]) != want_crc {
        return Err("设置头块头 CRC32 校验失败".to_string());
    }
    let raw = region
        .get(*pos + 9..*pos + 9 + stored)
        .ok_or_else(|| "设置头块数据越界".to_string())?;
    *pos += 9 + stored;

    if compressed == 0 {
        return Ok(raw.to_vec());
    }

    // 剥掉每 4KiB 前哨的 CRC32
    let mut data = Vec::with_capacity(raw.len());
    let mut i = 0usize;
    while i + 4 <= raw.len() {
        let n = (raw.len() - i - 4).min(4096);
        let chunk = &raw[i + 4..i + 4 + n];
        let want = u32::from_le_bytes([raw[i], raw[i + 1], raw[i + 2], raw[i + 3]]);
        if crc32(chunk) != want {
            return Err("设置头分块 CRC32 校验失败".to_string());
        }
        data.extend_from_slice(chunk);
        i += 4 + n;
    }

    match compressed {
        1 => lzma::decompress(&data, None).map_err(|e| e.to_string()),
        other => Err(format!("不支持的设置头压缩方式：{other}")),
    }
}

fn compression_name(method: u8) -> &'static str {
    match method {
        0 => "stored",
        1 => "zlib",
        2 => "bzip2",
        3 => "lzma1",
        _ => "unknown",
    }
}

/// 小端只读游标
struct Cur<'a> {
    b: &'a [u8],
    pos: usize,
    unicode: bool,
}

impl<'a> Cur<'a> {
    fn new(b: &'a [u8], unicode: bool) -> Self {
        Self { b, pos: 0, unicode }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.pos.checked_add(n).ok_or("偏移溢出")?;
        let slice = self
            .b
            .get(self.pos..end)
            .ok_or_else(|| format!("设置头数据不足（偏移 {} 处需要 {} 字节）", self.pos, n))?;
        self.pos = end;
        Ok(slice)
    }

    fn skip(&mut self, n: usize) -> Result<(), String> {
        self.take(n).map(|_| ())
    }

    fn u8(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> Result<u32, String> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn u64(&mut self) -> Result<u64, String> {
        let b = self.take(8)?;
        let mut a = [0u8; 8];
        a.copy_from_slice(b);
        Ok(u64::from_le_bytes(a))
    }

    fn i64(&mut self) -> Result<i64, String> {
        Ok(self.u64()? as i64)
    }

    fn bytes(&mut self) -> Result<Vec<u8>, String> {
        let n = self.u32()? as usize;
        Ok(self.take(n)?.to_vec())
    }

    /// 长度前缀字符串（ANSI 版按 cp1252，Unicode 版按 UTF-16LE）
    fn text(&mut self) -> Result<String, String> {
        let raw = self.bytes()?;
        if self.unicode {
            let (s, _, _) = encoding_rs::UTF_16LE.decode(&raw);
            Ok(s.into_owned())
        } else {
            let (s, _, _) = encoding_rs::WINDOWS_1252.decode(&raw);
            Ok(s.into_owned())
        }
    }

    /// 位域：每 8 位 1 字节，低位在前；恰好 3 字节时补 1 字节（32 位安装器）
    fn flags(&mut self, bits: usize) -> Result<u64, String> {
        let bytes = bits.div_ceil(8);
        let mut out = 0u64;
        for i in 0..bytes {
            let b = self.u8()? as u64;
            if i < 8 {
                out |= b << (i * 8);
            }
        }
        if bytes == 3 {
            self.skip(1)?;
        }
        Ok(out)
    }
}

struct HeaderInfo {
    app_name: String,
    publisher: String,
    compression: u8,
    language_count: usize,
    message_count: usize,
    permission_count: usize,
    type_count: usize,
    component_count: usize,
    task_count: usize,
    directory_count: usize,
    file_count: usize,
    data_entry_count: usize,
}

fn parse_header(c: &mut Cur<'_>) -> Result<HeaderInfo, String> {
    let app_name = c.text()?;
    let _app_versioned_name = c.text()?;
    let _app_id = c.text()?;
    let _app_copyright = c.text()?;
    let publisher = c.text()?;
    let _app_publisher_url = c.text()?;
    let _app_support_phone = c.text()?;
    let _app_support_url = c.text()?;
    let _app_updates_url = c.text()?;
    let _app_version = c.text()?;
    let _default_dir_name = c.text()?;
    let _default_group_name = c.text()?;
    let _base_filename = c.text()?;
    // license_text / info_before / info_after（< 5.2.5 在本段）
    let _license = c.text()?;
    let _info_before = c.text()?;
    let _info_after = c.text()?;
    let _uninstall_files_dir = c.text()?;
    let _uninstall_name = c.text()?;
    let _uninstall_icon = c.text()?;
    let _app_mutex = c.text()?;
    let _default_user_name = c.text()?;
    let _default_user_org = c.text()?;
    let _default_serial = c.text()?;
    let _compiled_code = c.text()?; // 4.0.0 ≤ v < 5.2.5
    let _app_readme = c.text()?;
    let _app_contact = c.text()?;
    let _app_comments = c.text()?;
    let _app_modify_path = c.text()?;
    let _uninstaller_signature = c.text()?; // 5.2.1 ≤ v < 5.3.10

    c.skip(32)?; // lead_bytes 字符集（256 位）

    let language_count = c.u32()? as usize;
    let message_count = c.u32()? as usize;
    let permission_count = c.u32()? as usize;
    let type_count = c.u32()? as usize;
    let component_count = c.u32()? as usize;
    let task_count = c.u32()? as usize;
    let directory_count = c.u32()? as usize;
    let file_count = c.u32()? as usize;
    let data_entry_count = c.u32()? as usize;
    let _icon_count = c.u32()? as usize;
    let _ini_count = c.u32()? as usize;
    let _registry_count = c.u32()? as usize;
    let _delete_count = c.u32()? as usize;
    let _unins_delete_count = c.u32()? as usize;
    let _run_count = c.u32()? as usize;
    let _unins_run_count = c.u32()? as usize;

    c.skip(20)?; // winver 区间：begin + end，各 10 字节
    let _back_color = c.u32()?;
    let _back_color2 = c.u32()?;
    let _image_back_color = c.u32()?;
    c.skip(16)?; // 口令 MD5
    c.skip(8)?; // 口令盐
    let _extra_disk_space = c.i64()?;
    let _slices_per_disk = c.u32()?;
    let _uninstall_log_mode = c.u8()?;
    let _dir_exists_warning = c.u8()?;
    let _privileges_required = c.u8()?;
    let _show_language_dialog = c.u8()?;
    let _language_detection = c.u8()?;
    let compression = c.u8()?;
    let _arch_allowed = c.flags(4)?;
    let _arch_installed = c.flags(4)?;
    let _signed_uninstaller_size = c.u32()?;
    let _signed_uninstaller_crc = c.u32()?;
    let _options = c.flags(46)?;

    Ok(HeaderInfo {
        app_name,
        publisher,
        compression,
        language_count,
        message_count,
        permission_count,
        type_count,
        component_count,
        task_count,
        directory_count,
        file_count,
        data_entry_count,
    })
}

struct FileStub {
    source: String,
    destination: String,
    location: u32,
}

/// 依次解析到 `files` 为止（后续的 icons/ini/registry/... 本阶段不需要）。
fn parse_entries_until_files(c: &mut Cur<'_>, h: &HeaderInfo) -> Result<Vec<FileStub>, String> {
    for _ in 0..h.language_count {
        skip_language(c)?;
    }
    for _ in 0..h.message_count {
        let _name = c.text()?;
        let _value = c.bytes()?;
        c.skip(4)?; // language 索引
    }
    for _ in 0..h.permission_count {
        let _ = c.bytes()?;
    }
    for _ in 0..h.type_count {
        for _ in 0..4 {
            let _ = c.text()?;
        }
        // winver 区间(20) + 标志(1) + 枚举(1) + size(u64)
        c.skip(20 + 1 + 1 + 8)?;
    }
    for _ in 0..h.component_count {
        for _ in 0..5 {
            let _ = c.text()?;
        }
        // 额外磁盘占用(u64) + level(i32) + used(u8) + winver 区间(20) + 标志(1) + size(u64)
        c.skip(8 + 4 + 1 + 20 + 1 + 8)?;
    }
    for _ in 0..h.task_count {
        for _ in 0..6 {
            let _ = c.text()?;
        }
        // level(i32) + used(u8) + winver 区间(20) + 标志(1)
        c.skip(4 + 1 + 20 + 1)?;
    }
    for _ in 0..h.directory_count {
        let _name = c.text()?;
        for _ in 0..6 {
            let _ = c.text()?;
        }
        c.skip(4 + 20 + 2 + 1)?;
    }

    let mut files = Vec::with_capacity(h.file_count);
    for _ in 0..h.file_count {
        let source = c.text()?;
        let destination = c.text()?;
        let _install_font_name = c.text()?;
        for _ in 0..6 {
            let _ = c.text()?;
        }
        c.skip(20)?; // winver 区间
        let location = c.u32()?;
        let _attributes = c.u32()?;
        let _external_size = c.u64()?;
        c.skip(2)?; // permission
        let _options = c.flags(32)?;
        let _file_type = c.u8()?;
        files.push(FileStub {
            source,
            destination,
            location,
        });
    }

    Ok(files)
}

fn skip_language(c: &mut Cur<'_>) -> Result<(), String> {
    for _ in 0..10 {
        let _ = c.text()?;
    }
    c.skip(4)?; // language_id
    c.skip(4)?; // codepage
    c.skip(4 * 4)?; // 四个字号
    c.skip(1)?; // right_to_left
    Ok(())
}

struct DataStub {
    offset: u32,
    file_size: u64,
    chunk_size: u64,
    md5: String,
    compressed: bool,
    /// CallInstructionOptimized
    filtered: bool,
    /// ChunkEncrypted
    encrypted: bool,
}

fn parse_data_entries(c: &mut Cur<'_>, count: usize) -> Result<Vec<DataStub>, String> {
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let _first_slice = c.u32()?;
        let _last_slice = c.u32()?;
        let offset = c.u32()?;
        let _file_offset = c.u64()?;
        let file_size = c.u64()?;
        let chunk_size = c.u64()?;
        let md5_raw = c.take(16)?;
        let md5 = md5_raw
            .iter()
            .map(|b| format!("{:02x}", b))
            .collect::<String>();
        c.skip(8)?; // FILETIME
        c.skip(4)?; // file_version_ms
        c.skip(4)?; // file_version_ls
        let options = c.flags(9)?;
        out.push(DataStub {
            offset,
            file_size,
            chunk_size,
            md5,
            compressed: options & (1 << 7) != 0, // ChunkCompressed
            filtered: options & (1 << 4) != 0,   // CallInstructionOptimized
            encrypted: options & (1 << 6) != 0,  // ChunkEncrypted
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_relative_strips_root_and_rejects_escape() {
        assert_eq!(
            safe_relative("{app}\\Data\\bitmap.lod").unwrap(),
            std::path::Path::new("Data/bitmap.lod")
        );
        assert!(safe_relative("{app}\\..\\evil.txt").is_none());
        assert!(safe_relative("{app}\\C:\\evil.txt").is_none());
        assert!(safe_relative("no-root-prefix.txt").is_none());
    }

    #[test]
    fn flag_reader_packs_low_bits_first() {
        let data = [0b0000_1010u8, 0b0000_0001];
        let mut c = Cur::new(&data, false);
        assert_eq!(c.flags(9).unwrap(), 0b1_0000_1010);
    }

    #[test]
    fn flag_reader_pads_three_byte_sets() {
        let data = [0xFFu8, 0xFF, 0xFF, 0x00];
        let mut c = Cur::new(&data, false);
        assert_eq!(c.flags(17).unwrap(), 0xFFFF_FF);
        assert_eq!(c.pos, 4, "3 字节位域应额外占用 1 字节填充");
    }
}
