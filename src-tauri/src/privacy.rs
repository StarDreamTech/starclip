//! Local-only classification. A successful scan is not a claim that all secrets are detectable.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    sync::OnceLock,
};
use url::Url;

pub type Result<T> = std::result::Result<T, String>;
pub const MAX_RULES: usize = 100;
const MAX_SCAN_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrivacyRule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub condition: String,
    pub value: String,
    pub tag: Option<String>,
    pub mark_private: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrivacyRules {
    pub revision: u32,
    pub rules: Vec<PrivacyRule>,
}
impl Default for PrivacyRules {
    fn default() -> Self {
        Self {
            revision: 1,
            rules: vec![],
        }
    }
}
impl PrivacyRules {
    pub fn validate(&self) -> Result<()> {
        if self.revision == 0 || self.rules.len() > MAX_RULES {
            return Err("规则版本无效或超过 100 条。".into());
        }
        let mut ids = HashSet::new();
        for rule in &self.rules {
            if rule.id.is_empty()
                || rule.id.len() > 64
                || !rule
                    .id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                || !ids.insert(&rule.id)
            {
                return Err("规则编号无效或重复。".into());
            }
            if rule.name.trim().is_empty()
                || rule.name.chars().count() > 60
                || rule.value.trim().is_empty()
                || rule.value.chars().count() > 200
            {
                return Err("规则名称需为 1–60 字，条件需为 1–200 字。".into());
            }
            if let Some(tag) = &rule.tag {
                validate_tags(std::slice::from_ref(tag))?;
            }
            if !rule.mark_private && rule.tag.is_none() {
                return Err("规则至少需要加标签或标为隐私。".into());
            }
            match rule.condition.as_str() {
                "keyword" => {}
                "domain" => {
                    normalize_rule_host(&rule.value)?;
                }
                _ => return Err("规则只支持关键词或域名条件。".into()),
            }
        }
        Ok(())
    }
}

pub fn validate_tags(tags: &[String]) -> Result<()> {
    if tags.len() > 32
        || tags.iter().any(|tag| {
            tag.trim().is_empty() || tag.chars().count() > 40 || tag.chars().any(char::is_control)
        })
    {
        return Err("每条最多 32 个标签，每个标签需为 1–40 字且不能含控制字符。".into());
    }
    Ok(())
}

pub fn union(a: &[String], b: &[String]) -> Vec<String> {
    a.iter()
        .chain(b)
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Persisted separately from ordinary labels. Reasons stay until an explicit local review.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Metadata {
    pub record_identity: String,
    pub privacy_manual: bool,
    pub privacy_reasons: Vec<String>,
    pub scan_version: u32,
    pub manual_tags: Vec<String>,
    pub rule_tags: Vec<String>,
    pub link_sources: Vec<String>,
    pub link_hosts: Vec<String>,
    /// 复制时前台 / 剪贴板所有者应用名，作为标签展示；重新扫描保留。
    pub app_tags: Vec<String>,
    /// 浏览器随复制提供的来源页面，只保留协议、域名与路径。
    pub origin_url: String,
    pub origin_sources: Vec<String>,
    pub origin_hosts: Vec<String>,
}

/// 采集时由系统剪贴板附带的上下文，与正文分开记录。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CaptureContext {
    /// 复制方通过系统约定标记了敏感 / 临时内容（如密码管理器）。
    pub concealed: bool,
    pub app: Option<String>,
    pub origin_url: Option<String>,
}
pub const CONCEALED_REASON: &str = "复制来源标记为敏感内容";
pub const MAX_APP_TAGS: usize = 8;

/// 规范来源应用名：去空白、去控制字符、截到 40 字，不能作为标签时返回 None。
pub fn normalize_app_name(name: &str) -> Option<String> {
    let name: String = name
        .chars()
        .filter(|c| !c.is_control())
        .collect::<String>()
        .trim()
        .chars()
        .take(40)
        .collect();
    let name = name.trim().to_owned();
    (!name.is_empty()).then_some(name)
}

/// 只接受 http(s) 页面地址，去掉账号、查询与片段，避免把会话参数写入历史。
pub fn sanitize_origin_url(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() || raw.len() > 2048 {
        return None;
    }
    let mut url = Url::parse(raw).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return None;
    }
    url.set_query(None);
    url.set_fragment(None);
    let _ = url.set_username("");
    let _ = url.set_password(None);
    let value = url.to_string();
    (value.len() <= 1024).then_some(value)
}

fn origin_facets(origin: &str) -> (Vec<String>, Vec<String>) {
    match Url::parse(origin)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
    {
        Some(host) => {
            let host = host.trim_end_matches('.').to_ascii_lowercase();
            let sources = source(&host)
                .map(|s| vec![s.to_owned()])
                .unwrap_or_default();
            (sources, vec![host])
        }
        None => (vec![], vec![]),
    }
}

impl Metadata {
    /// 合并本次复制的上下文。隐私标记只加不减，解除仍需本机复核。
    pub fn apply_capture(&mut self, context: &CaptureContext) {
        if context.concealed {
            self.privacy_reasons = union(&self.privacy_reasons, &[CONCEALED_REASON.into()]);
        }
        if let Some(app) = context.app.as_deref().and_then(normalize_app_name) {
            if !self.app_tags.contains(&app) && self.app_tags.len() < MAX_APP_TAGS {
                self.app_tags.push(app);
            }
        }
        if let Some(origin) = context.origin_url.as_deref().and_then(sanitize_origin_url) {
            let (sources, hosts) = origin_facets(&origin);
            self.origin_url = origin;
            self.origin_sources = sources;
            self.origin_hosts = hosts;
        }
    }
    pub fn normalize_image_policy(&mut self, kind: &str) {
        if kind == "image" {
            // This legacy marker described a missing capability, not a secret
            // match. Preserve manual protection and every real rule reason.
            self.privacy_reasons
                .retain(|reason| reason != "图片尚未进行文字识别");
            self.scan_version = 0;
        }
    }
    pub fn ensure_identity(&mut self) -> Result<()> {
        if self.record_identity.is_empty() {
            let mut bytes = [0u8; 32];
            getrandom::fill(&mut bytes).map_err(|_| "无法生成记录身份，未开放数据。")?;
            self.record_identity = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        }
        Ok(())
    }
    pub fn is_private(&self) -> bool {
        self.privacy_manual || !self.privacy_reasons.is_empty()
    }
    pub fn tags(&self) -> Vec<String> {
        union(&union(&self.manual_tags, &self.rule_tags), &self.app_tags)
    }
    pub fn scanned(
        &self,
        content: &str,
        title: &str,
        kind: &str,
        rules: &PrivacyRules,
        reviewed: bool,
    ) -> Self {
        let scan = scan_with_origin(content, title, kind, &self.origin_hosts, rules);
        let mut next = Self {
            record_identity: self.record_identity.clone(),
            privacy_manual: self.privacy_manual,
            privacy_reasons: {
                let mut reasons = if reviewed {
                    scan.reasons
                } else {
                    union(&self.privacy_reasons, &scan.reasons)
                };
                if reasons.len() > 128 {
                    reasons.truncate(127);
                    reasons.push("历史隐私原因已合并，需本机复核".into());
                }
                reasons
            },
            scan_version: if scan.complete { rules.revision } else { 0 },
            manual_tags: self.manual_tags.clone(),
            rule_tags: scan.tags,
            link_sources: scan.sources,
            link_hosts: scan.hosts,
            app_tags: self.app_tags.clone(),
            origin_url: self.origin_url.clone(),
            origin_sources: self.origin_sources.clone(),
            origin_hosts: self.origin_hosts.clone(),
        };
        next.normalize_image_policy(kind);
        next
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrivacyRescan {
    pub token: String,
    pub total: usize,
    pub private_count: usize,
    pub newly_private_count: usize,
    pub changed_count: usize,
}

#[derive(Debug, Default)]
pub struct Scan {
    pub reasons: Vec<String>,
    pub tags: Vec<String>,
    pub sources: Vec<String>,
    pub hosts: Vec<String>,
    pub complete: bool,
}

fn normalize_rule_host(value: &str) -> Result<String> {
    let value = value.trim();
    if value.contains(['/', '?', '#', '@', ':', '*']) {
        return Err("域名条件只填域名，例如 linux.do，不含路径、端口或通配符。".into());
    }
    let url = Url::parse(&format!("https://{value}")).map_err(|_| "域名条件无效。")?;
    let host = url
        .host_str()
        .ok_or("域名条件无效。")?
        .trim_end_matches('.')
        .to_owned();
    if host.is_empty() || !host.contains('.') {
        return Err("域名条件需为完整域名。".into());
    }
    Ok(host)
}
fn domain_matches(host: &str, domain: &str) -> bool {
    host == domain
        || host
            .strip_suffix(domain)
            .is_some_and(|prefix| prefix.ends_with('.'))
}
fn source(host: &str) -> Option<&'static str> {
    if host == "mp.weixin.qq.com" {
        Some("wechat")
    } else if domain_matches(host, "linux.do") {
        Some("linuxdo")
    } else if domain_matches(host, "x.com") || domain_matches(host, "twitter.com") {
        Some("x")
    } else if host == "t.co" {
        Some("x-short")
    } else if domain_matches(host, "bilibili.com") {
        Some("bilibili")
    } else if host == "b23.tv" {
        Some("bilibili-short")
    } else {
        None
    }
}
fn secret_key(key: &str) -> bool {
    let key: String = key
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    matches!(
        key.as_str(),
        "password"
            | "passwd"
            | "pwd"
            | "secret"
            | "clientsecret"
            | "token"
            | "accesstoken"
            | "refreshtoken"
            | "apikey"
            | "authorization"
            | "privatekey"
            | "mnemonic"
            | "seedphrase"
    )
}
fn url_credentials(url: &Url) -> bool {
    url.password().is_some()
        || !url.username().is_empty()
        || url
            .query_pairs()
            .any(|(k, v)| secret_key(&k) && !v.trim().is_empty())
        || url.fragment().is_some_and(|f| {
            url::form_urlencoded::parse(f.as_bytes())
                .any(|(k, v)| secret_key(&k) && !v.trim().is_empty())
        })
}

fn wordlists() -> &'static [HashMap<&'static str, u16>; 2] {
    static LISTS: OnceLock<[HashMap<&'static str, u16>; 2]> = OnceLock::new();
    LISTS.get_or_init(|| {
        [
            include_str!("../resources/bip39/english.txt"),
            include_str!("../resources/bip39/chinese_simplified.txt"),
        ]
        .map(|text| {
            text.lines()
                .enumerate()
                .map(|(i, word)| (word, i as u16))
                .collect()
        })
    })
}
fn valid_checksum(indices: &[u16]) -> bool {
    let total_bits = indices.len() * 11;
    let entropy_bits = total_bits * 32 / 33;
    let bit = |n: usize| ((indices[n / 11] >> (10 - n % 11)) & 1) as u8;
    let entropy: Vec<u8> = (0..entropy_bits / 8)
        .map(|i| (0..8).fold(0, |value, j| (value << 1) | bit(i * 8 + j)))
        .collect();
    let digest = Sha256::digest(entropy);
    (entropy_bits..total_bits)
        .all(|n| bit(n) == ((digest[(n - entropy_bits) / 8] >> (7 - (n - entropy_bits) % 8)) & 1))
}
fn mnemonic_reason(text: &str) -> Option<&'static str> {
    let lower = text.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphabetic())
        .filter(|w| !w.is_empty())
        .collect();
    let mut suspected = false;
    for list in wordlists() {
        let mut run = Vec::new();
        // Sentinel flushes the last consecutive run. This catches a phrase in a note,
        // without joining unrelated tokens across words that are not in the word list.
        for word in words.iter().copied().chain(std::iter::once("")) {
            if let Some(index) = list.get(word) {
                run.push(*index);
                continue;
            }
            for length in [12usize, 15, 18, 21, 24] {
                if run.windows(length).any(valid_checksum) {
                    return Some("助记词格式（词表与校验通过）");
                }
                if run.len() == length {
                    suspected = true;
                }
            }
            run.clear();
        }
    }
    if suspected {
        Some("疑似助记词（词表匹配，校验未通过）")
    } else {
        None
    }
}

pub fn scan(content: &str, title: &str, kind: &str, rules: &PrivacyRules) -> Scan {
    scan_with_origin(content, title, kind, &[], rules)
}

/// `origin_hosts` 来自浏览器提供的来源页面，只参与域名规则匹配，不计入正文链接。
pub fn scan_with_origin(
    content: &str,
    title: &str,
    kind: &str,
    origin_hosts: &[String],
    rules: &PrivacyRules,
) -> Scan {
    // Image content is an internal fingerprint, not recognized image text.
    let content = if kind == "image" { "" } else { content };
    let mut result = Scan {
        complete: true,
        ..Scan::default()
    };
    if content.len() > MAX_SCAN_BYTES || title.chars().count() > 200 {
        result.complete = false;
        result.reasons.push("内容超过扫描上限".into());
        return result;
    }
    if kind == "image" {
        // No OCR is performed here. An unreadable image is pending review,
        // not evidence of a secret; otherwise every screenshot would be
        // hidden before the user has a chance to inspect it.
        result.complete = false;
    }
    let text = format!("{content}\n{title}");
    let lower = text.to_lowercase();
    if text.contains("-----BEGIN ") && text.contains("PRIVATE KEY-----") {
        result.reasons.push("私钥标记".into());
    }
    for field in [content, title] {
        if let Some(reason) = mnemonic_reason(field) {
            result.reasons.push(reason.into());
        }
    }
    for token in text.split(|c: char| {
        c.is_whitespace() || matches!(c, '\'' | '"' | ',' | ';' | '(' | ')' | '{' | '}')
    }) {
        if [
            "sk-",
            "ghp_",
            "github_pat_",
            "gho_",
            "xoxb-",
            "xoxp-",
            "AKIA",
            "ASIA",
        ]
        .iter()
        .any(|prefix| token.starts_with(prefix) && token.len() >= prefix.len() + 16)
        {
            result.reasons.push("凭据格式".into());
        }
    }
    for line in text.lines() {
        for field in line.split([',', ';', '&']) {
            if let Some((key, value)) = field.split_once(['=', ':']) {
                let key = key.trim().trim_matches(['"', '\'', '{', '}']);
                let key = key
                    .split_whitespace()
                    .last()
                    .unwrap_or(key)
                    .trim_matches(['"', '\'']);
                if secret_key(key)
                    && !value
                        .trim_matches(|c: char| {
                            c.is_whitespace() || matches!(c, '\'' | '"' | '{' | '}')
                        })
                        .is_empty()
                {
                    result.reasons.push("凭据字段".into());
                }
            }
        }
    }
    let mut cursor = 0;
    let mut url_count = 0;
    while let Some(relative) = text[cursor..].char_indices().find_map(|(i, c)| {
        let tail = &text[cursor + i..];
        (matches!(c, 'h' | 'H')
            && (tail
                .get(..7)
                .is_some_and(|s| s.eq_ignore_ascii_case("http://"))
                || tail
                    .get(..8)
                    .is_some_and(|s| s.eq_ignore_ascii_case("https://"))))
        .then_some(i)
    }) {
        let offset = cursor + relative;
        url_count += 1;
        if url_count > 64 {
            result.complete = false;
            result.reasons.push("链接数量超过扫描上限".into());
            break;
        }
        let raw = text[offset..]
            .split(|c: char| {
                c.is_whitespace()
                    || matches!(
                        c,
                        '<' | '>'
                            | '"'
                            | '\''
                            | '`'
                            | '('
                            | ')'
                            | '['
                            | ']'
                            | '{'
                            | '}'
                            | '，'
                            | '。'
                            | '！'
                            | '？'
                    )
            })
            .next()
            .unwrap_or("");
        // Consume the complete candidate so URLs in its path/query/fragment are not sources.
        cursor = offset + raw.len();
        let candidate = raw.trim_end_matches(['.', ',', ';', '!', '?']);
        match Url::parse(candidate) {
            Ok(url) if matches!(url.scheme(), "http" | "https") && url.host_str().is_some() => {
                let host = url
                    .host_str()
                    .unwrap()
                    .trim_end_matches('.')
                    .to_ascii_lowercase();
                if let Some(source) = source(&host) {
                    result.sources.push(source.into());
                }
                result.hosts.push(host);
                if url_credentials(&url) {
                    result.reasons.push("链接包含凭据".into());
                }
            }
            _ => {
                result.complete = false;
                result.reasons.push("链接解析未完成".into());
            }
        }
    }
    for rule in rules.rules.iter().filter(|r| r.enabled) {
        let matched = match rule.condition.as_str() {
            "keyword" => lower.contains(&rule.value.trim().to_lowercase()),
            "domain" => normalize_rule_host(&rule.value).ok().is_some_and(|domain| {
                result
                    .hosts
                    .iter()
                    .chain(origin_hosts)
                    .any(|host| host == &domain)
            }),
            _ => false,
        };
        if matched {
            if rule.mark_private {
                result.reasons.push(format!("自定义规则：{}", rule.name));
            }
            if let Some(tag) = &rule.tag {
                result.tags.push(tag.trim().to_owned());
            }
        }
    }
    result.reasons = union(&result.reasons, &[]);
    result.tags = union(&result.tags, &[]);
    result.hosts = union(&result.hosts, &[]);
    result.sources = union(&result.sources, &[]);
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn capture_context_sanitizes_origin_and_app_names() {
        assert_eq!(
            sanitize_origin_url("https://a:b@Example.COM/path/页?token=1#x").as_deref(),
            Some("https://example.com/path/%E9%A1%B5")
        );
        assert_eq!(sanitize_origin_url("file:///Users/fixture"), None);
        assert_eq!(sanitize_origin_url("javascript:alert(1)"), None);
        assert_eq!(
            normalize_app_name(" \u{7}原创应用 ").as_deref(),
            Some("原创应用")
        );
        assert_eq!(normalize_app_name("   "), None);
        assert_eq!(
            normalize_app_name(&"长".repeat(60))
                .unwrap()
                .chars()
                .count(),
            40
        );
        let mut metadata = Metadata::default();
        for index in 0..20 {
            metadata.apply_capture(&CaptureContext {
                app: Some(format!("原创应用{index}")),
                ..CaptureContext::default()
            });
        }
        assert_eq!(metadata.app_tags.len(), MAX_APP_TAGS);
        let rescanned = metadata.scanned("原创正文", "", "text", &PrivacyRules::default(), true);
        assert_eq!(
            rescanned.app_tags, metadata.app_tags,
            "rescans keep app tags"
        );
    }
    #[test]
    fn domains_parse_multiple_urls_and_reject_lookalikes() {
        let scan = scan("HTTP://MP.WEIXIN.QQ.COM./s/a https://linux.do/t/1 https://x.com/u https://t.co/a https://www.bilibili.com/video/a https://b23.tv/b https://x.com.evil.test https://linux.do@evil.test/", "", "text", &PrivacyRules::default());
        assert_eq!(
            scan.sources,
            vec![
                "bilibili",
                "bilibili-short",
                "linuxdo",
                "wechat",
                "x",
                "x-short"
            ]
        );
        assert!(scan.hosts.contains(&"evil.test".into()));
        assert!(scan.reasons.contains(&"链接包含凭据".into()));
        assert!(scan.complete);
        for value in [
            "https://fake-linux.do/",
            "https://notx.com",
            "https://mp.weixin.qq.com.evil.test",
            "https://b23.tv.evil.test",
            "https://х.com",
        ] {
            assert!(scan_for(value).sources.is_empty());
        }
    }
    fn scan_for(text: &str) -> Scan {
        scan(text, "", "text", &PrivacyRules::default())
    }
    #[test]
    fn known_public_mnemonic_vectors_and_prose_boundary() {
        // Public BIP39 zero-entropy test vectors; never use them as a wallet.
        assert!(scan_for("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about").reasons.iter().any(|s| s.contains("校验通过")));
        assert!(scan_for("的 的 的 的 的 的 的 的 的 的 的 在")
            .reasons
            .iter()
            .any(|s| s.contains("校验通过")));
        assert!(scan_for("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon").reasons.iter().any(|s| s.contains("校验未通过")));
        assert!(scan_for(
            "There are twelve words in this ordinary sentence about a friendly spaceship."
        )
        .reasons
        .is_empty());
        assert!(scan_for("这是一段普通中文产品介绍，数据仅保存在本机。")
            .reasons
            .is_empty());
        assert!(scan_for("abandon about").reasons.is_empty());
    }
    #[test]
    fn embedded_urls_are_not_source_hosts_and_title_does_not_hide_mnemonics() {
        for value in [
            "https://evil.example/?next=https://x.com",
            "https://evil.example/#https://x.com",
            "https://evil.example/path/https://linux.do",
        ] {
            assert!(scan_for(value).sources.is_empty());
        }
        assert_eq!(
            scan_for("https://evil.example/?next=https://x.com https://linux.do/t/1").sources,
            vec!["linuxdo"]
        );
        let result = scan("abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon", "原创标题", "text", &PrivacyRules::default());
        assert!(result.reasons.iter().any(|s| s.contains("校验未通过")));
        assert!(!scan_for(&"a".repeat(MAX_SCAN_BYTES + 1)).complete);
    }
    #[test]
    fn credentials_limits_and_rules() {
        for text in [
            "https://example.test/?access_token=fixed-test",
            "https://example.test/#password=example",
            "https://user:pass@example.test/",
            "Authorization: Bearer fixed-test-only",
            "export API_KEY=fixture-only",
            "const password = 'fixture-only'",
            "{\"api_key\":\"fixture-only\"}",
            "-----BEGIN OPENSSH PRIVATE KEY-----",
        ] {
            assert!(!scan_for(text).reasons.is_empty(), "{text}");
        }
        assert!(scan_for("password safety guidelines").reasons.is_empty());
        assert!(!scan_for(&"https://example.test/ ".repeat(65)).complete);
        assert!(!scan_for("https://[bad").complete);
        let rules = PrivacyRules {
            revision: 2,
            rules: vec![PrivacyRule {
                id: "fixture".into(),
                name: "内部资料".into(),
                enabled: true,
                condition: "keyword".into(),
                value: "INTERNAL".into(),
                tag: Some("工作".into()),
                mark_private: true,
            }],
        };
        rules.validate().unwrap();
        let scan = scan("internal note", "", "text", &rules);
        assert_eq!(scan.tags, vec!["工作"]);
        assert!(!scan.reasons.is_empty());
    }
    #[test]
    fn images_are_pending_without_becoming_private_by_default() {
        let scan = scan("", "游戏截图", "image", &PrivacyRules::default());
        assert!(!scan.complete);
        assert!(scan.reasons.is_empty());
        let metadata =
            Metadata::default().scanned("", "游戏截图", "image", &PrivacyRules::default(), false);
        assert!(!metadata.is_private());
        assert_eq!(metadata.scan_version, 0);
    }
}
