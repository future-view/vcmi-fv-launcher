/**
 * Tauri IPC 契约层（前端唯一出口）。
 *
 * 契约三方一致性由 `npm run check:invoke-parity` 校验：
 * 前端 `invoke("x")` 字面量 ↔ 后端 `#[tauri::command] fn x` ↔ `lib.rs` 注册表。
 *
 * 实现按域拆在 `./ipc/*`（每个文件只放同域命令），本文件仅做 barrel 再导出，
 * 外部一律 `import ... from "@/lib/tauri"` —— 保持不变。
 * 新增命令：在对应域文件里导出函数；跨域能力不足时先建新域文件，不要再堆回单体。
 */
export * from "./ipc/core";
export * from "./ipc/window";
export * from "./ipc/config";
export * from "./ipc/prefs";
export * from "./ipc/dialog";
export * from "./ipc/wiki";
export * from "./ipc/settings-json";
export * from "./ipc/launch";
export * from "./ipc/wiki-read";
export * from "./ipc/vcmi-settings";
export * from "./ipc/realm";
export * from "./ipc/realm-export";
export * from "./ipc/realm-maps";
export * from "./ipc/realm-repo";
export * from "./ipc/mods";
export * from "./ipc/runtime-info";
export * from "./ipc/mod-install";
export * from "./ipc/game-import";
export * from "./ipc/def";
export * from "./ipc/updater";
export * from "./ipc/runtime";
