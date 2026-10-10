// 控制台窗口策略（Windows）：
//   - dev（`debug_assertions` 开） → 保留
//   - release                     → 隐藏（`windows_subsystem = "windows"`）
//   - 任意 profile 想强制保留     → `tauri build --features console`
//     （见 package.json 的 build:console / build:console:release）
//
// ⚠️ 刻意不只用 `debug_assertions` 判定：那样一旦为了提速关掉断言（或改 profile），
//    控制台会跟着消失且极难排查。显式 feature 才能在任何档位下稳定要/不要控制台。
#![cfg_attr(
    all(not(debug_assertions), not(feature = "console")),
    windows_subsystem = "windows"
)]

fn main() {
    vcmi_mod_launcher_lib::run();
}
