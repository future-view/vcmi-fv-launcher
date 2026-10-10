//! H3 高清版（Heroes III HD Edition）`.pak` 容器读取器（**只读**）。
//!
//! 格式依据 VCMI 引擎 `client/render/hdEdition/PakLoader.cpp`（VCMI 自身也只有只读解析器，
//! 没有写入器——HD 素材由官方 Launcher 从用户自备的 HD 版安装原样拷贝而来）：
//!
//! - offset 0：`u32 magic`（恒为 4）+ `u32 headerOffset`
//! - headerOffset 处：`u32 entriesCount`，随后每条目：
//!   `name[20]`（大写、`\0` 结尾）+ metadataOffset + metadataSize + countSheets + compressedSize
//!   + fullSize + sheets[countSheets].compressedSize[] + sheets[countSheets].fullSize[]
//! - sheet 数据**紧跟 metadata 之后**；`compressedSize → fullSize` 为 zlib 流
//! - 每条目的 sheet 解压后是一张 **DDS 图集**（DXT1/DXT5）；帧的裁剪矩形由 metadata
//!   **文本表**描述（每行 12 或 18 个空格分隔的整数）
//!
//! ## 分层
//!
//! 本模块只做**容器级**工作：索引解析、帧表解析、sheet 原始字节读取（zlib 解压）。
//! DDS/DXT 解码在 `infra::image`（`decode_dds`），「HD 素材来源」的业务编排在 `vcmi::hd_edition`。
//!
//! ## 与 DEF/LOD 的关系（易混点）
//!
//! pak **不是** LOD 的另一种写法：它是"按 H3 原始资源名索引的高清图集"，帧名与 DEF 的
//! 13 字节帧名字段一致（`ThCsMag1.21f` → `THCSMAG1`，见 `infra::def::DefReader::frame_name`）。
//!
//! 依赖方向：仅标准库 + `flate2`；不得出现 `tauri` 类型，也不得依赖 `vcmi`（见 `infra/mod.rs`）。

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use flate2::read::ZlibDecoder;

/// 合法 magic：VCMI 侧 `assert(magic == 4)`。
const PAK_MAGIC: u32 = 4;

/// 条目名固定 20 字节（`\0` 结尾；H3 资源名最长 8~12 字符）。
const NAME_LEN: usize = 20;

/// 条目数上限：防损坏/伪造文件触发天量分配（实测 x2/x3 pak 各 571 / 2161 条）。
const MAX_ENTRIES: usize = 100_000;

/// 单条目 sheet 数上限（实测无超过 8 的条目）。
const MAX_SHEETS: usize = 4096;

/// 单条目 metadata 表上限（实测数 KB）。
const MAX_METADATA_BYTES: usize = 4 * 1024 * 1024;

/// 单 sheet 解压上限（图集 DDS，实测最大约 6 MB；给足余量但拒绝异常值）。
const MAX_SHEET_BYTES: usize = 256 * 1024 * 1024;

/// pak 内的一张图集切片（sheet 上的裁剪矩形，可含独立阴影矩形）。
#[derive(Clone, Debug)]
pub struct PakSheet {
    pub compressed_size: u32,
    pub full_size: u32,
}

/// 一个归档条目（= 一个动画/资源名，含若干 sheet 与若干帧）。
#[derive(Debug)]
pub struct PakEntry {
    pub name: String,
    pub metadata_offset: u32,
    pub metadata_size: u32,
    pub sheets: Vec<PakSheet>,
}

/// pak 内的一帧图像。
#[derive(Clone, Debug)]
pub struct PakFrame {
    /// 所属条目名（大写，如 `HALLCSTL`）
    pub entry: String,
    /// 帧名（大写，如 `THCSMAG1`）——即 VCMI `imagesByName` 的键
    pub name: String,
    pub sheet: u32,
    /// 图集裁剪矩形 `(x, y, w, h)`，单位是**该档像素**（x2/x3）
    pub rect: (u32, u32, u32, u32),
    /// 绘制偏移（VCMI `ImageEntry::spriteOffsetX/Y`，VCMI 绘制时**乘以显示倍率**）。
    ///
    /// ⚠️ 列序见 [`parse_metadata`]：`g(1)/g(3)`。⚠️ 数值**随档位写入**（实测船坞帧
    /// x2 = `(0,1)`、x3 = `(0,2)`），因此必须与**同一档**的图配对使用，勿跨档比较。
    pub sprite_offset: (i32, i32),
    /// 0/1（VCMI `assert(rotation == 0 || rotation == 1)`）
    pub rotation: u32,
    /// 阴影帧：`(sheet, (x, y, w, h))`
    pub shadow: Option<(u32, (u32, u32, u32, u32))>,
}

/// 已打开的 pak（持路径 + 索引；sheet 按需 seek 读取）。
pub struct PakArchive {
    path: PathBuf,
    entries: HashMap<String, PakEntry>,
    /// 条目名 → 已解析帧表（懒构建；同一 entry 的多帧查询只解析一次）
    frames: Mutex<HashMap<String, Arc<Vec<PakFrame>>>>,
    /// 帧名 → (条目名, 帧下标)；首次按名查询时全量扫描一次 metadata
    index: Mutex<Option<Arc<HashMap<String, (String, usize)>>>>,
}

impl PakArchive {
    /// 打开 pak 并解析条目索引（失败返回 `None`，调用方回落原版素材）。
    pub fn open(path: &Path) -> Option<Self> {
        let mut f = File::open(path).ok()?;
        let mut head = [0u8; 8];
        f.read_exact(&mut head).ok()?;
        let magic = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
        let header_offset = u32::from_le_bytes([head[4], head[5], head[6], head[7]]);
        if magic != PAK_MAGIC {
            return None;
        }
        f.seek(SeekFrom::Start(header_offset as u64)).ok()?;
        let mut buf4 = [0u8; 4];
        f.read_exact(&mut buf4).ok()?;
        let count = u32::from_le_bytes(buf4) as usize;
        if count == 0 || count > MAX_ENTRIES {
            return None;
        }

        let mut entries = HashMap::with_capacity(count);
        for _ in 0..count {
            let name = read_name(&mut f)?;
            let mut nums = [0u8; 20];
            f.read_exact(&mut nums).ok()?;
            let rd = |i: usize| u32::from_le_bytes([nums[i], nums[i + 1], nums[i + 2], nums[i + 3]]);
            let metadata_offset = rd(0);
            let metadata_size = rd(4);
            let count_sheets = rd(8) as usize;
            if count_sheets > MAX_SHEETS {
                return None;
            }
            // sheets 的两段 u32 数组（先压缩大小、后解压大小）
            let mut comp = vec![0u32; count_sheets];
            let mut full = vec![0u32; count_sheets];
            let mut tmp = [0u8; 4];
            for v in comp.iter_mut() {
                f.read_exact(&mut tmp).ok()?;
                *v = u32::from_le_bytes(tmp);
            }
            for v in full.iter_mut() {
                f.read_exact(&mut tmp).ok()?;
                *v = u32::from_le_bytes(tmp);
            }
            let sheets = comp
                .into_iter()
                .zip(full)
                .map(|(compressed_size, full_size)| PakSheet {
                    compressed_size,
                    full_size,
                })
                .collect();
            if !name.is_empty() {
                entries.insert(
                    name.clone(),
                    PakEntry {
                        name,
                        metadata_offset,
                        metadata_size,
                        sheets,
                    },
                );
            }
        }

        Some(Self {
            path: path.to_path_buf(),
            entries,
            frames: Mutex::new(HashMap::new()),
            index: Mutex::new(None),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// 条目数（测试/日志用）
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// 按名查帧。
    ///
    /// 匹配顺序（对齐 VCMI `HdImageLoader::find` 的 `imagesByName` 语义，另加宽容回退）：
    /// 1. **帧名**精确匹配（大小写不敏感）——正常路径（DEF 帧名换算而来）；
    /// 2. 名字恰好是某 **entry 名**时取该 entry 的**首帧**（调用方拿不到帧名时的兜底）。
    ///
    /// ⚠️ 不要按"帧下标"与 DEF 对齐：pak 只收录需要替换的帧（实测 `CPRSMALL` pak 内 124 帧
    /// vs DEF 151 帧），序号不对应，**只有名字可靠**。
    pub fn find(&self, name: &str) -> Option<PakFrame> {
        let key = name.trim().to_uppercase();
        if key.is_empty() {
            return None;
        }
        let (entry, idx) = {
            let ix = self.ensure_index()?;
            ix.get(&key).cloned()?
        };
        let frames = self.frames_of(&entry)?;
        frames.get(idx).cloned()
    }

    /// 读某条目某 sheet 的**已解压** DDS 字节（zlib 流；未压缩时原样返回）。
    pub fn read_sheet(&self, entry: &str, sheet: u32) -> Option<Vec<u8>> {
        let e = self.entries.get(&entry.trim().to_uppercase())?;
        let s = e.sheets.get(sheet as usize)?;
        // sheet 数据起点 = metadata 之后，再跳过前序 sheet
        let mut off = e.metadata_offset as u64 + e.metadata_size as u64;
        for prev in &e.sheets[..sheet as usize] {
            off += prev.compressed_size as u64;
        }
        let n = if s.compressed_size > 0 {
            s.compressed_size
        } else {
            s.full_size
        } as usize;
        if n == 0 || n > MAX_SHEET_BYTES {
            return None;
        }
        let mut f = File::open(&self.path).ok()?;
        f.seek(SeekFrom::Start(off)).ok()?;
        let mut raw = vec![0u8; n];
        f.read_exact(&mut raw).ok()?;
        if s.compressed_size == 0 || s.compressed_size == s.full_size {
            return Some(raw);
        }
        let mut out = Vec::with_capacity(s.full_size as usize);
        ZlibDecoder::new(&raw[..]).read_to_end(&mut out).ok()?;
        // 解压不足声明大小 → 视为损坏（宁可回落原版素材，不交给下游解出半张图）
        if out.len() < s.full_size as usize {
            return None;
        }
        Some(out)
    }

    /// 某条目的帧表（懒解析 + 复用）
    fn frames_of(&self, entry: &str) -> Option<Arc<Vec<PakFrame>>> {
        let key = entry.trim().to_uppercase();
        if let Ok(guard) = self.frames.lock() {
            if let Some(v) = guard.get(&key) {
                return Some(v.clone());
            }
        }
        let e = self.entries.get(&key)?;
        let meta = self.read_metadata(e)?;
        let parsed = Arc::new(parse_metadata(&key, &meta));
        let mut guard = self.frames.lock().ok()?;
        Some(guard.entry(key).or_insert(parsed).clone())
    }

    fn read_metadata(&self, e: &PakEntry) -> Option<Vec<u8>> {
        let n = e.metadata_size as usize;
        if n == 0 || n > MAX_METADATA_BYTES {
            return None;
        }
        let mut f = File::open(&self.path).ok()?;
        f.seek(SeekFrom::Start(e.metadata_offset as u64)).ok()?;
        let mut buf = vec![0u8; n];
        f.read_exact(&mut buf).ok()?;
        Some(buf)
    }

    /// 全量「帧名 → (entry, 下标)」索引（懒构建一次，进程内复用）。
    fn ensure_index(&self) -> Option<Arc<HashMap<String, (String, usize)>>> {
        let mut guard = self.index.lock().ok()?;
        if let Some(ix) = guard.as_ref() {
            return Some(ix.clone());
        }
        let mut ix: HashMap<String, (String, usize)> = HashMap::new();
        for name in self.entries.keys() {
            if let Some(frames) = self.frames_of(name) {
                for (i, fr) in frames.iter().enumerate() {
                    // 同名多帧时取首次出现（与 VCMI `try_emplace` 一致）
                    ix.entry(fr.name.clone()).or_insert_with(|| (name.clone(), i));
                }
            }
        }
        // entry 名兜底（取首帧）：仅用于"拿不到帧名"的调用方，不覆盖已有帧名键
        for name in self.entries.keys() {
            ix.entry(name.clone()).or_insert_with(|| (name.clone(), 0));
        }
        let arc = Arc::new(ix);
        *guard = Some(arc.clone());
        Some(arc)
    }
}

/// 读 20 字节名字（`\0` 截断 + 大写，对齐 VCMI `to_upper_copy`）。
fn read_name(f: &mut File) -> Option<String> {
    let mut buf = [0u8; NAME_LEN];
    f.read_exact(&mut buf).ok()?;
    let end = buf.iter().position(|&b| b == 0).unwrap_or(NAME_LEN);
    Some(String::from_utf8_lossy(&buf[..end]).to_uppercase())
}

/// 解析 metadata 文本表为帧列表。
///
/// 每行：`name sheetIndex spriteOffsetX ? spriteOffsetY ? sheetOffsetX sheetOffsetY width
/// height rotation hasShadow [阴影 6 字段]`（`?` 为 VCMI 未使用的两个字段）。
/// 行数与列数不符（截断/异形）时跳过该行——单帧坏数据不得毁掉整张表。
fn parse_metadata(entry: &str, data: &[u8]) -> Vec<PakFrame> {
    let end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    let text = String::from_utf8_lossy(&data[..end]);
    let mut out = Vec::new();
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let Some(raw_name) = it.next() else { continue };
        let nums: Vec<i64> = it.filter_map(|t| t.parse::<i64>().ok()).collect();
        if nums.len() != 11 && nums.len() != 17 {
            continue;
        }
        let g = |i: usize| nums.get(i).copied().unwrap_or(0);
        // sheetOffset/width/height 为像素级（x2/x3）；spriteOffset 为 1x 逻辑值（可负）
        let rect = (
            g(5).max(0) as u32,
            g(6).max(0) as u32,
            g(7).max(0) as u32,
            g(8).max(0) as u32,
        );
        if rect.2 == 0 || rect.3 == 0 {
            continue;
        }
        let has_shadow = g(10) != 0;
        let shadow = if has_shadow && nums.len() == 17 {
            Some((
                g(11).max(0) as u32,
                (
                    g(12).max(0) as u32,
                    g(13).max(0) as u32,
                    g(14).max(0) as u32,
                    g(15).max(0) as u32,
                ),
            ))
        } else {
            None
        };
        out.push(PakFrame {
            entry: entry.to_string(),
            name: raw_name.to_uppercase(),
            sheet: g(0).max(0) as u32,
            rect,
            // 列序以 VCMI 为准（`PakLoader.cpp` 的 `ImageEntry` 赋值）：
            // `sheetIndex spriteOffsetX unknown1 spriteOffsetY unknown2 sheetOffsetX …`
            // —— spriteOffset 在 g(1)/g(3)，**不是** g(2)/g(4)（那两列是 VCMI 未使用的 unknown）。
            sprite_offset: (g(1) as i32, g(3) as i32),
            rotation: if g(9) == 1 { 1 } else { 0 },
            shadow,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 探测不到的路径：静默返回 None（调用方回落原版素材）
    #[test]
    fn open_missing_returns_none() {
        assert!(PakArchive::open(Path::new("Z:/definitely/not/here.pak")).is_none());
    }

    /// 真机冒烟：需本机装有官方 HD 版素材 mod（`VCMI_HD_PAK_DIR` 可覆盖）。
    ///
    /// 期望值来自 2026-09-23 对 `...\userdata\Mods\hd-edition\mods\x2\content\data` 的实测
    /// （Python 独立解析同格式得到，见项目记忆 `reference_hd_edition_pak.md`）。
    #[test]
    #[ignore]
    fn real_hd_paks_smoke() {
        let dir = std::env::var("VCMI_HD_PAK_DIR")
            .ok()
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../references/Heroes3-VCMI/userdata/Mods/hd-edition/mods/x2/content/data")
            });
        let dir = PathBuf::from(dir);
        let sprite = PakArchive::open(&dir.join("sprite_DXT_com_x2.pak")).expect("sprite pak 应可打开");
        let bitmap = PakArchive::open(&dir.join("bitmap_DXT_com_x2.pak")).expect("bitmap pak 应可打开");
        assert_eq!(sprite.entry_count(), 2161, "sprite x2 条目数");
        assert_eq!(bitmap.entry_count(), 571, "bitmap x2 条目数");

        // 建筑图标：DEF 帧名 `ThCsMag1.21f` → `THCSMAG1`
        let f = sprite.find("THCSMAG1").expect("应命中 THCSMAG1");
        assert_eq!(f.rect, (0, 0, 300, 140), "x2 建筑图标尺寸");
        assert_eq!(f.entry, "HALLCSTL");
        // 生物小图
        assert!(sprite.find("CPRS097S").is_some(), "应命中生物小图 CPRS097S");
        // 城镇幕布（bitmap pak）：x2 = 1600×748
        let t = bitmap.find("TBCSBACK").expect("应命中 TBCSBACK");
        assert_eq!(t.rect, (0, 0, 1600, 748));
        // 英雄头像
        let h = bitmap.find("HPL000KN").expect("应命中头像 HPL000KN");
        assert_eq!(h.rect.2, 116);

        // `spriteOffset` 的**列序**：VCMI `ImageEntry` 的顺序是
        // `sheetIndex spriteOffsetX unknown1 spriteOffsetY unknown2 sheetOffsetX …`
        // —— 取 `g(1)/g(3)`。回归：曾取 `g(2)/g(4)`（VCMI 未使用的 unknown 列）恒得 `(0,0)`，
        // 城镇幕布按它定位时就会丢掉真正需要的偏移（船坞 `TBCSDKNN` 被上移 51px 压住战神像）。
        // ⚠️ 数值**随档位写入**（本机实测 x2 = `(0,1)`、x3 = `(0,2)`），故只断言"非零"锁列序；
        // 上游使用它时必须取**同一档 pak** 的值（`lookup_with_offset` 已保证）。
        let d = sprite.find("TBCSDKNN").expect("应命中城堡船坞帧 TBCSDKNN");
        assert_eq!(d.entry, "TBCSDOCK");
        assert_eq!(d.sprite_offset.0, 0);
        assert!(
            d.sprite_offset.1 > 0,
            "船坞帧 spriteOffsetY 应 > 0（列序回归：旧口径恒 (0,0)），实得 {:?}",
            d.sprite_offset
        );
        println!("TBCSDKNN: rect {:?} rot {} offset {:?}", d.rect, d.rotation, d.sprite_offset);

        // sheet 解压后是完整 DDS
        let dds = sprite.read_sheet("TBCSBACK", 0).or_else(|| bitmap.read_sheet("TBCSBACK", 0));
        let dds = dds.expect("应能读出 TBCSBACK 的 sheet");
        assert_eq!(&dds[..4], b"DDS ", "sheet 应为 DDS 图集");
    }
}
