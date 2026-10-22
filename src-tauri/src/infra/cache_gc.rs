//! 构建期进程级缓存的**统一释放入口**。
//!
//! 背景（2026-10-16）：缓存构建从 103 分钟压到 3 分钟后，暴露出下一个问题 ——
//! **构建结束后启动器内存占用不回落**。原因是构建期加的所有加速缓存都是进程级
//! `OnceLock`：**容量上界只约束"构建中的峰值"，不约束"构建完的残留"**。
//! 上界越大（按机器内存分档，最大 512 MB + 2×192 MB），构建完仍挂在进程上的就越多。
//!
//! 本模块把这些散落各处的缓存收敛成一个入口 [`release_build_caches`]，由**整轮构建结束时**
//! 调用一次（见 `commands::cache::release_wiki_caches`）。
//!
//! ## 为什么不在每批结束时调用
//!
//! 一轮构建是「阶段一（全部 mod，仅数据）→ 阶段二（城镇批 / 其余批，补素材）→ 阶段三（高清）」
//! 共 3~5 批。阶段之间刻意共用缓存：阶段二的 town 批与 rest 批都还会解析配置、解析资源，
//! 批间清空会让下一批多付一次全量扫描（正是当初加缓存要消灭的开销）。
//! ⇒ 只在**整轮结束**清一次；单批结束时仍只做既有的 AI session / HD sheet 回收
//!   （`batch.rs` 收尾），那两项是"完成即无用、且重建代价极低"的纯产物缓存。
//!
//! ## 安全性
//!
//! 列出的缓存**全部是可重建的派生数据**（目录索引 / 解析结果 / 编码产物 / 去重集合），
//! 键都带指纹且可自失效；清空只影响性能，不影响任何产物正确性。

/// 释放全部构建期缓存。
///
/// 幂等、可重复调用；未初始化的缓存（`OnceLock::get()` 为 `None`）直接跳过。
pub fn release_build_caches() {
    // ── infra ──
    // 目录清单（`Content/` 递归结果）：572 个目录下全量清单，条目最多的常驻块之一
    crate::infra::dirscan::release_caches();
    // lod 目录表（H3sprite/H3bitmap 等，最多 4 份）
    crate::infra::lod::release_caches();

    // ── vcmi ──
    // 资源解析：zip 索引 / 文件字节 / 跨模组正负结果 / 载体探测
    // （**保留目录索引**，理由见 `resource::release_caches`：兜底/跨模组路径仍依赖它，
    //   清掉后完全未命中的资源首次要重建 870 个索引 ≈ 7.9s）
    crate::vcmi::resource::release_caches();
    // 单 mod 翻译表（容量 128，每份是一张完整 I18nMap）
    crate::vcmi::i18n::release_caches();
    // bonus type 图标表（`bonuses.json` 的 graphics 段，跨全部 mod 合并）
    crate::vcmi::bonus::release_caches();

    // ── modcache ──
    // 配置收集 / 原始配置快照（值为整份 mod 配置的 Value 树，最大的一块）
    crate::modcache::collect::release_caches();
    // webp 编码缓存 / DEF 帧缓存 / 已建目录集合
    crate::modcache::assets::release_caches();
    // 核心 i18n 合并表 + 注入去重 + raw.json 落盘去重
    crate::modcache::build::release_caches();
    // 构建成本估计（LPT 排序用）
    crate::modcache::batch::release_caches();
    // 核心翻译表（`Mods/vcmi` 合并结果）
    crate::modcache::i18n::release_caches();
    // AI 超分 session 池（模型权重 + ORT arena 高水位，单批收尾已清，这里兜底）
    crate::infra::ai::clear_sessions();
    // HD 图集解码缓存
    crate::vcmi::hd_edition::trim_source_cache();

    tracing::debug!(target: "launcher", "已释放构建期缓存（回收整轮构建的常驻内存）");
}
