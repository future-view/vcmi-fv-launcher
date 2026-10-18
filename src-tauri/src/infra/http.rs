//! 统一 HTTP 工具：跟随重定向的 blocking 客户端 + 带 ETag 的 GET + L2 镜像规则表 / L1 代理自动回退。
//!
//! 自 `repository/mod.rs` 抽出，供 `repository`（领域仓库 / 模组仓库）与 `updater`（启动器更新）共用。
//! 本层属 infra：除进程级网络出口配置外，无业务语义（`MirrorRule` 是纯网络概念，
//! `vcmi/config.rs` 的 `network.mirror_rules` 直接复用本模块类型）。
//!
//! 关键实测约束（勿违背）：
//! - Gitee raw / 附件都是 **302 跳转**，客户端必须跟随重定向；
//! - Gitee 返回的 `Content-Type` 是 `text/plain`，**不能靠它判定成功**，只能"2xx + 内容可解析"。
//!
//! 网络出口设计（`docs/features/network-egress.zh-CN.md` §3.1/§3.2，P3）：
//! - **默认直连**（代理与镜像规则均为空/off）；
//! - 请求遇**连接类失败**（connect/timeout/request）时按序自动回退：
//!   **直连 → 逐条镜像规则（声明序=优先级）→ L1 网络代理**；
//! - 镜像返回 403/429/5xx 视为镜像站坏 → 推进下一条；404 视为资源真不存在 → 停止（D2）；
//! - 代理仅支持 `http://` / `https://`（reqwest 0.12/0.13 已移除内置 SOCKS，`socks` feature 为空操作），
//!   且**只对 GitHub 主机参与**（N5）；镜像规则的 hosts 可填任意主机（D5）；
//! - 每次成功返回 [`Egress`]（直连 / 镜像@域名 / 代理），供日志与 N12 告警指名具体出口。

use crate::infra::berror::be_err;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use std::time::Duration;

use reqwest::blocking::{Client, RequestBuilder, Response};
use reqwest::{Proxy, Url};
use serde::{Deserialize, Serialize};

/// 进程级网络代理（http/https）。`None` = 直连。
/// 启动时由 `setup` 从 `launcher.json` 的 `network.proxy` 注入，`set_network_proxy` 改动时即时更新。
static PROXY: RwLock<Option<String>> = RwLock::new(None);

/// 进程级镜像规则表（L2）。空 = 不用镜像（**默认不内置**，镜像域名会腐化，由用户自填）。
/// 启动注入自 `launcher.json` 的 `network.mirror_rules`，`set_network_mirror_rules` 即时更新。
static MIRROR_RULES: RwLock<Vec<MirrorRule>> = RwLock::new(Vec::new());

/// 出口粘性（T2 定案）：一轮检查/下载里某条镜像或代理已确认可用后，后续 URL 直接先走
/// 粘性出口——避免每条都白等直连超时（140+ 个 mod.json × 12s 灾难）。
/// 粘性出口失败/配置变更/直连恢复时清除；每轮全新「检查更新」入口调 [`reset_sticky`] 重探直连。
#[derive(Clone, Debug, PartialEq)]
enum Sticky {
    /// 命中的镜像规则在规则表中的下标
    Rule(usize),
    Proxy,
}
static STICKY: RwLock<Option<Sticky>> = RwLock::new(None);

fn sticky() -> Option<Sticky> {
    STICKY.read().ok().and_then(|g| g.clone())
}

fn set_sticky(s: Option<Sticky>) {
    if let Ok(mut w) = STICKY.write() {
        *w = s;
    }
}

/// 清除出口粘性（新一轮检查开始时重探直连；镜像/代理配置变更时内部也会调）。
pub fn reset_sticky() {
    set_sticky(None);
}

/// A3 直连冷却时长：GitHub 直连连接类失败后一段时间内，跳过直连直接走镜像/代理——
/// 弱网（尤其被墙）用户不必每轮检查都白等一次 12s 连接超时。
const DIRECT_COOLDOWN_SECS: u64 = 30 * 60;
/// 直连冷却截止时刻（epoch 秒；0 = 无冷却）。进程内存态，不落盘（重启重探一次可接受）。
static DIRECT_COOLDOWN_UNTIL: AtomicU64 = AtomicU64::new(0);

/// 直连是否处于冷却期。
fn direct_cooling() -> bool {
    crate::infra::time::now_secs() < DIRECT_COOLDOWN_UNTIL.load(Ordering::SeqCst)
}

/// 记录一次直连连接类失败（开启冷却窗口）。
fn mark_direct_cooldown() {
    DIRECT_COOLDOWN_UNTIL.store(
        crate::infra::time::now_secs().saturating_add(DIRECT_COOLDOWN_SECS),
        Ordering::SeqCst,
    );
}

/// 直连成功：清除冷却（网络已恢复，不再跳过直连）。
fn clear_direct_cooldown() {
    DIRECT_COOLDOWN_UNTIL.store(0, Ordering::SeqCst);
}

/// 取 URL 的小写 host（解析失败返回空串）。Egress/日志展示与 hosts 匹配共用。
fn url_host(u: &str) -> String {
    Url::parse(u)
        .ok()
        .and_then(|x| x.host_str().map(|h| h.to_ascii_lowercase()))
        .unwrap_or_default()
}

/// 单条镜像规则（持久化在 `launcher.json` 的 `network.mirror_rules`，字段设计见方案 §3.2.2）。
/// 磁盘/IPC 字段名单词，无大小写转换问题；三层格式冻结不受影响。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MirrorRule {
    /// 临时停用而不删规则（默认启用）。
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// 可选备忘（镜像站名），仅用于展示与日志。
    #[serde(default)]
    pub note: Option<String>,
    /// 生效主机：精确域名或子域匹配（`host == e || host.ends_with(".e")`）。允许非 GitHub 主机（D5）。
    #[serde(default)]
    pub hosts: Vec<String>,
    /// 改写模式：`"prefix"`（拼接原 URL）或 `"template"`（占位符换形，见 §3.2.3）。
    #[serde(default = "default_mode")]
    pub mode: String,
    /// mode=prefix：镜像前缀（http(s)，归一后保证尾部 `/`）。
    #[serde(default)]
    pub prefix: Option<String>,
    /// mode=template：模板串，占位符 `{owner} {repo} {ref} {path} {url} {host}`，需 http(s) 且至少含一个占位符。
    #[serde(default)]
    pub template: Option<String>,
}

fn default_enabled() -> bool {
    true
}
fn default_mode() -> String {
    "prefix".to_string()
}

/// 成功出口标识（结果可追溯，§3.2.1 原则 4）。
#[derive(Debug, Clone)]
pub enum Egress {
    Direct,
    Mirror { host: String },
    Proxy,
}

impl Egress {
    /// 台账/日志短标签。
    pub fn label(&self) -> String {
        match self {
            Egress::Direct => "direct".to_string(),
            Egress::Mirror { host } => format!("mirror:{host}"),
            Egress::Proxy => "proxy".to_string(),
        }
    }
}

/// 判断主机是否 GitHub（精确边界，避免 `notgithub.com` 误判）。
fn host_is_github(host: &str) -> bool {
    let h = host.to_ascii_lowercase();
    h == "github.com"
        || h.ends_with(".github.com")
        || h == "githubusercontent.com"
        || h.ends_with(".githubusercontent.com")
}

/// URL 是否指向 GitHub 主机（解析失败按非 GitHub 处理）。L1 代理的参与条件（N5）。
pub fn is_github_url(url: &str) -> bool {
    Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(host_is_github))
        .unwrap_or(false)
}

/// 直连可达性探测结果。
#[derive(Debug, Clone)]
pub struct UrlProbe {
    /// 是否收到**任何** HTTP 响应（404 也算通：那只是地址不对，网络没问题）。
    pub ok: bool,
    pub status: Option<u16>,
    pub error: Option<String>,
}

/// 短超时**直连**探测：供首启向导判断「更新清单地址是否可达」。
///
/// 与常规请求的三点差异（**刻意不同**）：
/// - **不走镜像 / 代理回退链**：探测的目的正是判断"要不要开镜像"，走回退链会自我循环；
/// - **短超时**（连接 5s / 总 8s）：向导阶段不能让用户干等；且探测失败只是"给建议"，代价低；
/// - 收到任何 HTTP 状态都算 `ok`（只有连接类失败才算不通）。
pub fn probe_url(url: &str, connect_timeout: Duration, total_timeout: Duration) -> UrlProbe {
    let client = match Client::builder()
        .redirect(reqwest::redirect::Policy::limited(5))
        .timeout(total_timeout)
        .connect_timeout(connect_timeout)
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return UrlProbe {
                ok: false,
                status: None,
                error: Some(e.to_string()),
            }
        }
    };
    match client.get(url).send() {
        Ok(resp) => UrlProbe {
            ok: true,
            status: Some(resp.status().as_u16()),
            error: None,
        },
        Err(e) => UrlProbe {
            ok: false,
            status: None,
            error: Some(e.to_string()),
        },
    }
}

fn proxy_value() -> Option<String> {
    PROXY.read().ok().and_then(|g| g.clone())
}

/// 归一代理地址：`None` / 空 / `"off"` = 直连（`None`）；仅接受 http(s)，否则丢弃。
fn normalize_scheme(raw: Option<String>) -> Option<String> {
    raw.map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("off"))
        .filter(|s| s.starts_with("http://") || s.starts_with("https://"))
}

/// 配置进程级代理（同上归一）。幂等，供启动注入与 `set_network_proxy` 共用。
pub fn configure_proxy(proxy: Option<String>) {
    let norm = normalize_scheme(proxy);
    if let Ok(mut w) = PROXY.write() {
        *w = norm;
    }
    reset_sticky(); // 代理配置变了，粘性出口作废（下条请求重探）
}

/// 归一单条镜像规则：hosts 小写去空去重；prefix 保证尾部 `/`；template 需含占位符；
/// 关键字段缺失/非 http(s) → `None`（无效规则）。`set_network_mirror_rules` 以此做权威校验。
pub fn normalize_rule(r: &MirrorRule) -> Option<MirrorRule> {
    let mut hosts: Vec<String> = r
        .hosts
        .iter()
        .map(|h| h.trim().to_ascii_lowercase())
        .filter(|h| !h.is_empty())
        .collect();
    hosts.sort();
    hosts.dedup();
    if hosts.is_empty() {
        return None;
    }
    match r.mode.as_str() {
        "prefix" => {
            let p = normalize_scheme(r.prefix.clone())?;
            Some(MirrorRule {
                enabled: r.enabled,
                note: r.note.clone(),
                hosts,
                mode: "prefix".to_string(),
                prefix: Some(if p.ends_with('/') { p } else { format!("{p}/") }),
                template: None,
            })
        }
        "template" => {
            let t = normalize_scheme(r.template.clone())?;
            if !t.contains('{') {
                return None;
            }
            Some(MirrorRule {
                enabled: r.enabled,
                note: r.note.clone(),
                hosts,
                mode: "template".to_string(),
                prefix: None,
                template: Some(t),
            })
        }
        _ => None,
    }
}

/// 配置进程级镜像规则表（启动注入与 `set_network_mirror_rules` 共用；无效规则静默丢弃，
/// 保证手改坏 launcher.json 也不炸启动链路）。
pub fn configure_mirror_rules(rules: Vec<MirrorRule>) {
    let norm: Vec<MirrorRule> = rules.iter().filter_map(normalize_rule).collect();
    if let Ok(mut w) = MIRROR_RULES.write() {
        *w = norm;
    }
    reset_sticky(); // 规则表变了，粘性下标不再可信
}

fn rules_value() -> Vec<MirrorRule> {
    MIRROR_RULES.read().map(|g| g.clone()).unwrap_or_default()
}

/// hosts 匹配：精确域名或子域（`.suffix`）。
fn hosts_match(hosts: &[String], host: &str) -> bool {
    hosts
        .iter()
        .any(|e| e == host || host.ends_with(&format!(".{e}")))
}

/// 按规则改写 URL。占位符缺值/无法解析 → `None`（跳过该规则并记日志，不阻断回退链，§3.2.3）。
fn rewrite_url(url: &str, rule: &MirrorRule) -> Option<String> {
    let parsed = Url::parse(url).ok()?;
    let host = parsed.host_str()?.to_ascii_lowercase();
    let segs: Vec<&str> = parsed
        .path_segments()
        .map(|p| p.filter(|s| !s.is_empty()).collect())
        .unwrap_or_default();
    match rule.mode.as_str() {
        "prefix" => Some(format!("{}{url}", rule.prefix.as_deref()?)),
        "template" => {
            let t = rule.template.as_deref()?;
            // {url}/{host} 恒可用；先替换避免后续值中的花括号被二次解析
            let mut out = t.replace("{url}", url).replace("{host}", &host);
            for (token, idx) in [("{owner}", 0usize), ("{repo}", 1), ("{ref}", 2)] {
                if out.contains(token) {
                    out = out.replace(token, *segs.get(idx)?);
                }
            }
            if out.contains("{path}") {
                // {path} = 段 3.. 拼接；恰好 3 段时为空串（合法），不足 3 段 → 缺值
                if segs.len() < 3 {
                    return None;
                }
                out = out.replace("{path}", &segs[3..].join("/"));
            }
            Some(out)
        }
        _ => None,
    }
}

/// 镜像站常见坏法（推进下一条规则）：403 限流/拦截、429、5xx。
/// 404 不推进——视为资源真不存在，避免拿坏镜像掩盖真错（D2）。
fn retryable_status(status: u16) -> bool {
    matches!(status, 403 | 429 | 500 | 502 | 503 | 504)
}

/// 是否为**连接类**失败（可安全换出口重试）；4xx/5xx 属业务响应不在此列。
fn is_transient(e: &reqwest::Error) -> bool {
    e.is_connect() || e.is_timeout() || e.is_request()
}

/// 构建客户端。`with_proxy=true` 时挂代理（该客户端仅在 GitHub 回退末端使用，故全主机走代理即可）。
fn build_client(with_proxy: bool) -> Result<Client, String> {
    let mut b = Client::builder()
        .redirect(reqwest::redirect::Policy::limited(5))
        .timeout(Duration::from_secs(120))
        // 直连探测阶段：连接超时应远小于总超时，避免被墙时首试长时间挂起
        .connect_timeout(Duration::from_secs(12));
    if with_proxy {
        if let Some(p) = proxy_value() {
            b = b.proxy(
                Proxy::all(&p).map_err(|e| be_err("infra_proxy_invalid", &format!("{}", e)))?,
            );
        }
    }
    b.build()
        .map_err(|e| be_err("infra_http_client_create_failed", &format!("{}", e)))
}

/// 执行一个 GET 的**连接层自动回退**：（粘性出口→）直连 → 逐条镜像规则 → 代理（仅 GitHub）。
/// `build(client, url)` 用给定客户端与（被镜像改写的）URL 构造请求；成功时返回响应与出口标识，
/// 并记住可用出口（粘性，T2），后续 URL 不再逐条白等直连超时。
fn send_with_fallback<F>(url: &str, mut build: F) -> Result<(Response, Egress), String>
where
    F: FnMut(&Client, &str) -> RequestBuilder,
{
    let direct = build_client(false)?;
    let host = url_host(url);
    let hits: Vec<(usize, MirrorRule)> = rules_value()
        .into_iter()
        .enumerate()
        .filter(|(_, r)| r.enabled && hosts_match(&r.hosts, &host))
        .collect();

    // 0) 粘性出口快路：本轮已确认可用的镜像/代理先试；失败/规则变更则清除并回落完整链
    if let Some(st) = sticky() {
        match st {
            Sticky::Rule(i) => {
                let rewritten = hits
                    .iter()
                    .find(|(k, _)| *k == i)
                    .and_then(|(_, r)| rewrite_url(url, r));
                match rewritten {
                    Some(mu) => match build(&direct, &mu).send() {
                        Ok(resp) if !retryable_status(resp.status().as_u16()) => {
                            return Ok((
                                resp,
                                Egress::Mirror {
                                    host: url_host(&mu),
                                },
                            ));
                        }
                        // 粘性镜像坏了（连接失败或坏状态码）：清除后走完整链重选
                        Ok(_) | Err(_) => set_sticky(None),
                    },
                    None => set_sticky(None), // 规则被删/停用
                }
            }
            Sticky::Proxy => {
                if proxy_value().is_some() && is_github_url(url) {
                    match build_client(true) {
                        Ok(pc) => match build(&pc, url).send() {
                            Ok(resp) => return Ok((resp, Egress::Proxy)),
                            Err(_) => set_sticky(None),
                        },
                        Err(_) => set_sticky(None),
                    }
                } else {
                    set_sticky(None); // 代理已关闭/非 GitHub 域
                }
            }
        }
    }

    // 1) 直连原 URL（本轮首探，或粘性出口坏了之后的重探）；A3：GitHub 直连近期失败过
    //    （冷却期内）则跳过直连、直接从镜像/代理开始，弱网每轮省一次 12s 连接超时
    let mut last_err: Option<String> = None;
    if is_github_url(url) && direct_cooling() {
        tracing::debug!(target: "http", "直连冷却期内，跳过直连直接尝试镜像/代理");
    } else {
        match build(&direct, url).send() {
            Ok(resp) => {
                set_sticky(None); // 直连恢复：不再粘
                clear_direct_cooldown();
                return Ok((resp, Egress::Direct));
            }
            Err(e) => {
                if !is_transient(&e) {
                    return Err(be_err("infra_http_request_failed", &format!("{}", e)));
                }
                mark_direct_cooldown(); // A3：连接类失败 → 开启冷却
                last_err = Some(format!("直连失败: {e}"));
            }
        }
    }
    // 2) 镜像规则表：声明序 = 尝试序；镜像请求**永不叠加 L1 代理**（§3.2.4）；成功即设粘性
    let mut mirror_fail: Option<(Response, String)> = None;
    for (idx, r) in hits {
        let Some(mu) = rewrite_url(url, &r) else {
            tracing::warn!(
                target: "http",
                "镜像规则跳过（占位符缺值或字段无效）：{}",
                r.note.as_deref().unwrap_or(&r.mode)
            );
            continue;
        };
        let mhost = url_host(&mu);
        match build(&direct, &mu).send() {
            Ok(resp) => {
                if retryable_status(resp.status().as_u16()) {
                    tracing::warn!(
                        target: "http",
                        "镜像 {mhost} 返回 {}，推进下一条规则",
                        resp.status().as_u16()
                    );
                    mirror_fail = Some((resp, mhost));
                    continue;
                }
                set_sticky(Some(Sticky::Rule(idx)));
                return Ok((resp, Egress::Mirror { host: mhost }));
            }
            Err(e) => {
                if !is_transient(&e) {
                    return Err(be_err("infra_mirror_request_failed", &format!("{}", e)));
                }
                tracing::warn!(target: "http", "镜像 {mhost} 连接失败，推进下一条规则");
                last_err = Some(format!("镜像 {mhost} 连接失败: {e}"));
            }
        }
    }
    // 3) L1 网络代理（仅 GitHub 参与，N5 不变）；成功即设粘性
    if proxy_value().is_some() && is_github_url(url) {
        let pc = build_client(true)?;
        return build(&pc, url)
            .send()
            .map(|r| {
                set_sticky(Some(Sticky::Proxy));
                (r, Egress::Proxy)
            })
            .map_err(|e| be_err("infra_proxy_request_failed", &format!("{}", e)));
    }
    // 镜像全部返回坏状态码且无代理：透传最后一个镜像响应（让调用方按真实状态码报错）
    if let Some((resp, mh)) = mirror_fail {
        return Ok((resp, Egress::Mirror { host: mh }));
    }
    Err(format!(
        "网络请求失败: {}",
        last_err.unwrap_or_else(|| "直连冷却中（近期直连失败），镜像/代理均不可用".to_string())
    ))
}

/// 发起 GET（流式响应，供下载侧边读边写盘 + 进度 + 取消）。
/// 内置直连→镜像→代理回退；返回响应与**实际出口**（供台账/N12 告警指名镜像域名）。
pub fn get_send(url: &str) -> Result<(Response, Egress), String> {
    send_with_fallback(url, |c, u| c.get(u))
}

/// GET：返回 `(状态码, ETag, 响应体)`。304 时响应体为空。已内置直连→镜像→代理回退。
pub fn http_get(url: &str, etag: Option<&str>) -> Result<(u16, Option<String>, Vec<u8>), String> {
    let (resp, _egress) = send_with_fallback(url, |c, u| {
        let mut r = c.get(u);
        if let Some(e) = etag.filter(|e| !e.is_empty()) {
            r = r.header(reqwest::header::IF_NONE_MATCH, e);
        }
        r
    })?;
    let status = resp.status().as_u16();
    let new_etag = resp
        .headers()
        .get(reqwest::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    if status == 304 {
        return Ok((
            304,
            new_etag.or_else(|| etag.map(|s| s.to_string())),
            Vec::new(),
        ));
    }
    if !(200..300).contains(&status) {
        return Err(be_err("infra_server_returned", &format!("{}", status)));
    }
    let bytes = resp
        .bytes()
        .map_err(|e| be_err("infra_response_read_failed", &format!("{}", e)))?
        .to_vec();
    Ok((status, new_etag, bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule_prefix(p: &str, hosts: &[&str]) -> MirrorRule {
        MirrorRule {
            enabled: true,
            note: None,
            hosts: hosts.iter().map(|s| s.to_string()).collect(),
            mode: "prefix".into(),
            prefix: Some(p.into()),
            template: None,
        }
    }
    fn rule_template(t: &str) -> MirrorRule {
        MirrorRule {
            enabled: true,
            note: None,
            hosts: vec!["raw.githubusercontent.com".into()],
            mode: "template".into(),
            prefix: None,
            template: Some(t.into()),
        }
    }

    #[test]
    fn github_host_detection() {
        assert!(host_is_github("github.com"));
        assert!(host_is_github("raw.githubusercontent.com"));
        assert!(host_is_github("codeload.github.com"));
        assert!(host_is_github("objects.githubusercontent.com"));
        // 边界：不能把相似域名误判为 GitHub
        assert!(!host_is_github("notgithub.com"));
        assert!(!host_is_github("gitee.com"));
        // URL 解析（大小写不敏感、忽略路径）
        assert!(is_github_url(
            "https://GitHub.com/a/b/releases/download/v1/x.zip"
        ));
        assert!(is_github_url(
            "https://raw.githubusercontent.com/vcmi/x/develop/vcmi-1.7.json"
        ));
        assert!(!is_github_url(
            "https://gitee.com/futureview/x/raw/master/index.json"
        ));
    }

    #[test]
    fn rule_normalization() {
        // 补尾斜杠 / hosts 小写、去空、去重（sort 后 github.com 在前）
        let r = normalize_rule(&rule_prefix(
            "https://v4.gh-proxy.org",
            &["GitHub.COM", "github.com", " raw.githubusercontent.com "],
        ))
        .unwrap();
        assert_eq!(r.prefix.as_deref(), Some("https://v4.gh-proxy.org/"));
        assert_eq!(r.hosts, vec!["github.com", "raw.githubusercontent.com"]);
        // 空 hosts / 非 http(s)（含 socks）→ 无效
        assert!(normalize_rule(&rule_prefix("https://x.org/", &[])).is_none());
        assert!(normalize_rule(&rule_prefix("socks5://x.org/", &["github.com"])).is_none());
        assert!(normalize_rule(&rule_prefix("", &["github.com"])).is_none());
        // template 必须含占位符
        assert!(normalize_rule(&rule_template("https://cdn.example.com/fixed")).is_none());
    }

    #[test]
    fn template_rewrite_examples() {
        // 方案 §3.2.3 例 1：raw → jsdelivr
        let raw =
            "https://raw.githubusercontent.com/vcmi/vcmi-mods-repository/develop/vcmi-1.7.json";
        let r = rule_template("https://cdn.jsdelivr.net/gh/{owner}/{repo}@{ref}/{path}");
        assert_eq!(
            rewrite_url(raw, &r).as_deref(),
            Some("https://cdn.jsdelivr.net/gh/vcmi/vcmi-mods-repository@develop/vcmi-1.7.json")
        );
        // 深层路径：{path} 收余下全部段，不拆散
        let deep = "https://raw.githubusercontent.com/o/r/develop/config/creatures/x.json";
        assert_eq!(
            rewrite_url(deep, &r).as_deref(),
            Some("https://cdn.jsdelivr.net/gh/o/r@develop/config/creatures/x.json")
        );
        // 例 5：段数不足（单段）→ 缺值 skip（None），不阻断回退链
        assert_eq!(rewrite_url("https://github.com/vcmi", &r), None);
        // 恰好 3 段：{path} 为空串（合法）
        let three = "https://raw.githubusercontent.com/o/r/develop";
        assert_eq!(
            rewrite_url(three, &r).as_deref(),
            Some("https://cdn.jsdelivr.net/gh/o/r@develop/")
        );
    }

    #[test]
    fn prefix_rewrite_and_url_template() {
        // 例 2：release 大包只能走 prefix（gh-proxy 系）
        let zip = "https://github.com/herosix/wog/releases/download/v1.1.1/WOG.zip";
        let p = rule_prefix("https://v4.gh-proxy.org/", &["github.com"]);
        assert_eq!(
            rewrite_url(zip, &p).as_deref(),
            Some("https://v4.gh-proxy.org/https://github.com/herosix/wog/releases/download/v1.1.1/WOG.zip")
        );
        // 例 4：`{url}` 模板 ≡ prefix 模式（prefix 本质是模板的糖）
        let t = MirrorRule {
            hosts: p.hosts.clone(),
            ..rule_template("https://v4.gh-proxy.org/{url}")
        };
        assert_eq!(rewrite_url(zip, &t), rewrite_url(zip, &p));
    }

    #[test]
    fn host_scope_matching() {
        // D5：任意主机可配；子域匹配；近似域不误命中
        assert!(hosts_match(&["github.com".into()], "github.com"));
        assert!(hosts_match(
            &["githubusercontent.com".into()],
            "raw.githubusercontent.com"
        ));
        assert!(!hosts_match(&["github.com".into()], "notgithub.com"));
        assert!(hosts_match(&["gitee.com".into()], "gitee.com"));
        // 例 6：Gitee 不命中纯 GitHub 规则 → 恒直连
        assert!(!hosts_match(
            &["github.com".into(), "raw.githubusercontent.com".into()],
            "gitee.com"
        ));
    }

    #[test]
    fn mirror_status_advance() {
        // D2：403/429/5xx 推进下一条；404/3xx/2xx 停止回退（按真实响应处理）
        assert!(retryable_status(403));
        assert!(retryable_status(429));
        assert!(retryable_status(500));
        assert!(retryable_status(504));
        assert!(!retryable_status(404));
        assert!(!retryable_status(200));
        assert!(!retryable_status(301));
    }
}
