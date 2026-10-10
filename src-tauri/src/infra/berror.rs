//! 后端错误 → 结构化错误码。
//!
//! 所有「会反馈给前端用户」的 `Err(...)` 不再硬编码中文串，而是返回
//! `{"code": "<模块>_<语义>", "arg": "<可选附加上下文>"}` 的 JSON 串；
//! 前端经 `backendErrorText` 按 `code` 查 i18n 键本地化，当前界面语言缺译时回退 `zhCN` 词典，
//! 完全解析失败时（非本结构）原样显示旧式串，保证向后兼容。
//!
//! 用法：
//! - 无附加上下文：`return Err(be_err("repo_url_empty"));`
//! - 带一个动态参数（词典模板用 `{arg}` 占位）：
//!   `return Err(be_err("settings_gamedir_no_exe"));` （动态内容如需外显，放进 `arg`）
//!
//! 中文原文统一维护在 `src/i18n/*` 的 `beErr` 段（zh-CN 为唯一真源），不要在此硬编码中文。
use serde_json::json;

/// 构造结构化错误。`arg` 为动态附加上下文（无则 `""`）。
pub(crate) fn be_err(code: &str, arg: &str) -> String {
    json!({ "code": code, "arg": arg }).to_string()
}
