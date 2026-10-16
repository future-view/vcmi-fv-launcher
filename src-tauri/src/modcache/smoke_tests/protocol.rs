//! 提取协议的**覆盖率**校验（CI 阻断）。
//!
//! 对应 `scripts/mod-cache-audit.mjs --ci` 的 Rust 版：内置 VCMI schema（`resources/vcmi-schemas`）
//! 定义的文本字段必须 100% 被 `resources/mod-cache-protocol.json` 登记（或列入 `outOfScope`）。
//! 这样"新增字段没写进协议"会在 `cargo test` 阶段直接失败，无需额外脚本/CI 配置。

#[test]
fn protocol_covers_schema_text_fields() {
    match crate::modcache::schema::check_protocol_coverage() {
        Ok(n) => {
            println!("协议覆盖的文本字段数: {}", n);
            assert!(
                n > 0,
                "协议表未覆盖任何 schema 文本字段（协议表或 schema 目录有问题）"
            );
        }
        Err(violations) => panic!(
            "协议未登记 {} 个文本字段（请补进 mod-cache-protocol.json 的 textFields/ignoredPaths，\
             或加入 outOfScope 声明）:\n{}",
            violations.len(),
            violations.join("\n")
        ),
    }
}

#[test]
fn builtin_schemas_available() {
    let hash = crate::modcache::schema::schema_content_hash();
    assert!(
        !hash.is_empty(),
        "未找到内置 schema 目录或 manifest.json 缺少 contentHash（请跑 scripts/sync-vcmi-schemas.mjs）"
    );
}
