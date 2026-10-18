/**
 * GitHub 镜像规则的公共常量与「一键开关」预置规则（纯逻辑，无 React）。
 *
 * 落在 `lib` 而不是 `features/settings`：首启向导的一键开关与设置页「镜像规则」卡
 * 共用同一份口径 —— 否则"向导写的规则"与"设置页预填的域名"会各写一套而漂移。
 *
 * 与后端 `infra::http::MirrorRule` 对齐：hosts 匹配语法为「精确域名或子域」，
 * prefix 模式 = `前缀 + 原 URL`（`rewrite_url`），归一化由后端 `normalize_rule` 权威完成。
 */
import type { MirrorRule } from "@/lib/tauri";

/** 参与镜像的 GitHub 全家桶域名（release / raw / codeload / objects / gist） */
export const GITHUB_MIRROR_HOSTS: string[] = [
  "github.com",
  "raw.githubusercontent.com",
  "codeload.github.com",
  "objects.githubusercontent.com",
  "gist.githubusercontent.com",
];

/**
 * 一键开关使用的默认镜像前缀（**社区公共服务，不保证始终可用**）。
 *
 * 失效不影响正确性：`infra::http` 的回退链是「直连 → 逐条镜像 → 代理」，
 * 镜像坏掉（403/429/5xx）会自动推进到下一个出口，不会比不开更糟。
 */
export const DEFAULT_GITHUB_MIRROR_PREFIX = "https://v4.gh-proxy.org/";

/**
 * 构造一条「GitHub 镜像」预置规则（prefix 模式，默认启用）。
 *
 * `note` 由调用方给本地化文案，本模块不内置任何界面语言字符串。
 */
export function githubMirrorRule(
  prefix: string = DEFAULT_GITHUB_MIRROR_PREFIX,
  note?: string,
): MirrorRule {
  return {
    enabled: true,
    note: note?.trim() ? note : null,
    hosts: [...GITHUB_MIRROR_HOSTS],
    mode: "prefix",
    prefix,
    template: null,
  };
}
