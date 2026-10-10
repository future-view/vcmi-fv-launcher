//! 系统空闲检测：供后台缓存构建按「用户是否在用电脑」动态调整并发档位。
//!
//! 背景：ECO 档的本意是**不打扰正在使用电脑的用户**，但用户长时间无键鼠输入时机器是空闲的，
//! 此时仍维持低位并发只是白白拉长首次构建。本模块给出"距最后一次输入多少秒"，
//! 由 `wiki::batch::wiki_threads` 据此决定档位。
//!
//! 实现：只用 Win32 `GetLastInputInfo`（**不引入新依赖**，与 `commands/app.rs`
//! 的 PE 版本读取同款 `extern "system"` FFI 做法）。非 Windows 或调用失败时返回 `None`，
//! 调用方按"用户在用"处理（保持 ECO，绝不打扰）。
//!
//! 依赖方向：本层属 `infra`，不依赖 `vcmi` / `wiki` / `tauri`。

#[cfg(windows)]
mod win {
    #[repr(C)]
    struct LastInputInfo {
        cb_size: u32,
        dw_time: u32,
    }

    #[link(name = "user32")]
    extern "system" {
        /// `LASTINPUTINFO` 的输出参数版本（`GetLastInputInfo(LASTINPUTINFO*)`）
        fn GetLastInputInfo(plii: *mut LastInputInfo) -> i32;
        /// 自系统启动以来的毫秒数（64 位，无 49.7 天回绕问题）
        fn GetTickCount64() -> u64;
    }

    pub(super) fn idle_seconds() -> Option<u64> {
        let mut lii = LastInputInfo {
            cb_size: std::mem::size_of::<LastInputInfo>() as u32,
            dw_time: 0,
        };
        // SAFETY: 传入本函数栈上的合法结构体指针；cbSize 已按 API 要求填好；
        // 返回值 0 表示失败，此时不读取 dw_time。
        let ok = unsafe { GetLastInputInfo(&mut lii) };
        if ok == 0 {
            return None;
        }
        let now = unsafe { GetTickCount64() };
        Some(now.saturating_sub(lii.dw_time as u64) / 1000)
    }
}

/// 距最后一次键鼠输入的秒数；无法判定时返回 `None`（调用方按"用户在用"处理）。
#[cfg(windows)]
pub fn idle_seconds() -> Option<u64> {
    win::idle_seconds()
}

/// 非 Windows：无法判定，恒为 `None`（保持 ECO）。
#[cfg(not(windows))]
pub fn idle_seconds() -> Option<u64> {
    None
}
