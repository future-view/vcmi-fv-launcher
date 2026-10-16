//! 机器内存探测：供构建期各类缓存**按内存分档**设定上界。
//!
//! 背景：缓存的收益与风险都取决于容量——容量太小命中率低（省不掉重复 IO/CPU），
//! 太大又会把内存吃满（尤其首次全量构建要同时处理数百个模组）。故统一按机器物理内存分档。
//!
//! 实现：Win32 `GlobalMemoryStatusEx`（与 [`crate::infra::idle`] 同款 `extern "system"` FFI，
//! **不引入新依赖**）。非 Windows / 调用失败返回 `None`，调用方回落到**最保守档**。

/// 缓存预算档位（字节）。
#[derive(Debug, Clone, Copy)]
pub struct MemBudget {
    /// 资源文件原始字节缓存（DEF / 位图 / LOD 内条目等）
    pub file_bytes: usize,
    /// 解析产物缓存（配置 JSON 的合并结果、原始配置快照）
    pub config_bytes: usize,
    /// 图片缓存（webp 编码结果、解码后的帧）
    pub image_bytes: usize,
}

/// 最保守档（无法探测内存时使用）：总量约 80 MB，任何机器都吃得下。
const TIER_MIN: MemBudget = MemBudget {
    file_bytes: 32 * 1024 * 1024,
    config_bytes: 24 * 1024 * 1024,
    image_bytes: 24 * 1024 * 1024,
};

/// 按物理内存分档。
///
/// | 物理内存 | 文件字节 | 解析产物 | 图片 |
/// |---|---|---|---|
/// | < 8 GB  | 32 MB | 24 MB | 24 MB |
/// | 8~16 GB | 96 MB | 48 MB | 48 MB |
/// | 16~32 GB| 256 MB| 96 MB | 96 MB |
/// | ≥ 32 GB | 512 MB| 192 MB| 192 MB|
pub fn cache_budget() -> MemBudget {
    match total_memory_bytes() {
        Some(b) if b >= 32 * 1024 * 1024 * 1024 => MemBudget {
            file_bytes: 512 * 1024 * 1024,
            config_bytes: 192 * 1024 * 1024,
            image_bytes: 192 * 1024 * 1024,
        },
        Some(b) if b >= 16 * 1024 * 1024 * 1024 => MemBudget {
            file_bytes: 256 * 1024 * 1024,
            config_bytes: 96 * 1024 * 1024,
            image_bytes: 96 * 1024 * 1024,
        },
        Some(b) if b >= 8 * 1024 * 1024 * 1024 => MemBudget {
            file_bytes: 96 * 1024 * 1024,
            config_bytes: 48 * 1024 * 1024,
            image_bytes: 48 * 1024 * 1024,
        },
        _ => TIER_MIN,
    }
}

/// 物理内存总量（字节）；无法探测时 `None`。
#[cfg(windows)]
pub fn total_memory_bytes() -> Option<u64> {
    #[repr(C)]
    struct MemoryStatusEx {
        dw_length: u32,
        dw_memory_load: u32,
        ull_total_phys: u64,
        ull_avail_phys: u64,
        ull_total_page_file: u64,
        ull_avail_page_file: u64,
        ull_total_virtual: u64,
        ull_avail_virtual: u64,
        ull_avail_extended_virtual: u64,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GlobalMemoryStatusEx(lp_buffer: *mut MemoryStatusEx) -> i32;
    }

    let mut st = MemoryStatusEx {
        dw_length: std::mem::size_of::<MemoryStatusEx>() as u32,
        dw_memory_load: 0,
        ull_total_phys: 0,
        ull_avail_phys: 0,
        ull_total_page_file: 0,
        ull_avail_page_file: 0,
        ull_total_virtual: 0,
        ull_avail_virtual: 0,
        ull_avail_extended_virtual: 0,
    };
    // SAFETY: 传入本函数栈上的合法结构体指针，dwLength 已按 API 要求填好；
    // 返回 0 表示失败，此时不读取字段。
    let ok = unsafe { GlobalMemoryStatusEx(&mut st) };
    if ok == 0 || st.ull_total_phys == 0 {
        None
    } else {
        Some(st.ull_total_phys)
    }
}

/// 非 Windows：暂不探测，回落保守档。
#[cfg(not(windows))]
pub fn total_memory_bytes() -> Option<u64> {
    None
}
