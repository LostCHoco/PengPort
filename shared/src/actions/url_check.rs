//! URL allowlist 검증 — PSP 일관성 검증의 핵심 보안 layer.
//!
//! 명세: `docs/spec/psp-v1.md` 섹션 5.3 (manifest 일관성 검증), 섹션 9 (각 native
//! action 의 args 검증).
//!
//! 정책:
//! - HTTPS only (production). dev 모드 한정 HTTP 허용 (`allow_http`).
//! - `file://`, `javascript:`, `data:` scheme MUST NOT be allowed.
//! - loopback (localhost / 127.0.0.1 / ::1) 차단.
//! - RFC 1918 private network 차단 (10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16,
//!   169.254.0.0/16, fc00::/7).
//! - origin 일치 (manifest base URL 과 same-origin) 또는 `permissions.external_urls`
//!   glob 패턴 매칭.

use std::net::IpAddr;

use thiserror::Error;
use url::Url;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum UrlError {
    #[error("URL 파싱 실패: {0}")]
    Parse(String),

    #[error("허용되지 않는 scheme: {0}")]
    BlockedScheme(String),

    #[error("HTTP 는 dev 모드에서만 허용됩니다: {0}")]
    HttpInProduction(String),

    #[error("loopback / private network 호스트 차단: {0}")]
    PrivateHost(String),

    #[error("URL 이 manifest origin / external_urls 와 매칭 안 됨: {0}")]
    NotAllowed(String),
}

const BLOCKED_SCHEMES: &[&str] = &["file", "javascript", "data", "blob", "vbscript"];

/// URL 이 모든 정책을 통과하는지 검증.
///
/// `manifest_origin` 은 manifest 의 base URL (예: `https://service.example`).
/// `external_urls` 는 manifest `permissions.external_urls` 패턴 목록.
/// `allow_http` 는 dev 모드에서 true.
pub fn is_url_allowed(
    url: &str,
    manifest_origin: &str,
    external_urls: &[String],
    allow_http: bool,
) -> Result<(), UrlError> {
    let parsed = Url::parse(url).map_err(|e| UrlError::Parse(e.to_string()))?;
    let scheme = parsed.scheme();

    // 1. blocked schemes
    if BLOCKED_SCHEMES.contains(&scheme) {
        return Err(UrlError::BlockedScheme(scheme.to_string()));
    }

    // 2. HTTP 는 dev 만
    if scheme == "http" && !allow_http {
        return Err(UrlError::HttpInProduction(url.to_string()));
    }

    // 3. https / http 외 다른 scheme 은 별도 함수 (open_protocol) 처리
    if scheme != "https" && scheme != "http" {
        return Err(UrlError::BlockedScheme(scheme.to_string()));
    }

    // 4. host 검증 (private/loopback 차단)
    if let Some(host) = parsed.host_str() {
        if is_private_or_loopback_host(host) {
            return Err(UrlError::PrivateHost(host.to_string()));
        }
    } else {
        return Err(UrlError::Parse("host 누락".to_string()));
    }

    // 5. origin 매칭 또는 external_urls 패턴 매칭
    let manifest = Url::parse(manifest_origin)
        .map_err(|e| UrlError::Parse(format!("manifest_origin: {e}")))?;
    if same_origin(&parsed, &manifest) {
        return Ok(());
    }
    for pat in external_urls {
        if glob_match(&parsed, pat) {
            return Ok(());
        }
    }
    Err(UrlError::NotAllowed(url.to_string()))
}

/// host 가 loopback 또는 RFC 1918 private network 인지.
pub fn is_private_or_loopback_host(host: &str) -> bool {
    let lowered = host.to_ascii_lowercase();
    if lowered == "localhost" || lowered == "ip6-localhost" || lowered == "ip6-loopback" {
        return true;
    }
    let Ok(ip): Result<IpAddr, _> = host.parse() else {
        return false;
    };
    is_private_or_loopback_ip(ip)
}

fn is_private_or_loopback_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                // unique local (fc00::/7)
                || (v6.segments()[0] & 0xfe00) == 0xfc00
        }
    }
}

fn same_origin(a: &Url, b: &Url) -> bool {
    a.scheme() == b.scheme() && a.host_str() == b.host_str() && a.port_or_known_default() == b.port_or_known_default()
}

/// glob 패턴 매칭. 지원 형식:
/// - `https://example.com/*`
/// - `https://*.example.com/*`
/// - `https://cdn.example.com/static/*`
fn glob_match(url: &Url, pattern: &str) -> bool {
    let Ok(pat) = parse_pattern(pattern) else {
        return false;
    };

    // scheme
    if pat.scheme != "*" && pat.scheme != url.scheme() {
        return false;
    }

    // host
    let url_host = match url.host_str() {
        Some(h) => h,
        None => return false,
    };
    if !host_match(url_host, &pat.host) {
        return false;
    }

    // path
    let url_path = url.path();
    if pat.path.ends_with('*') {
        let prefix = &pat.path[..pat.path.len() - 1];
        url_path.starts_with(prefix)
    } else {
        url_path == pat.path
    }
}

#[derive(Debug)]
struct Pattern<'a> {
    scheme: &'a str,
    host: &'a str,
    path: &'a str,
}

fn parse_pattern(pat: &str) -> Result<Pattern<'_>, ()> {
    // 형식: scheme://host[/path]
    let (scheme, rest) = pat.split_once("://").ok_or(())?;
    let (host, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/*"),
    };
    Ok(Pattern { scheme, host, path })
}

fn host_match(actual: &str, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    if let Some(suffix) = pattern.strip_prefix("*.") {
        // *.example.com 은 example.com 자체는 매칭 안 함, sub.example.com 매칭
        return actual.ends_with(&format!(".{suffix}")) || actual == suffix;
    }
    actual == pattern
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_same_origin_ok() {
        let r = is_url_allowed(
            "https://service.example/path",
            "https://service.example",
            &[],
            false,
        );
        assert!(r.is_ok());
    }

    #[test]
    fn https_external_glob_ok() {
        let r = is_url_allowed(
            "https://cdn.example.com/static/x.png",
            "https://service.example",
            &["https://cdn.example.com/*".into()],
            false,
        );
        assert!(r.is_ok());
    }

    #[test]
    fn https_external_subdomain_glob_ok() {
        let r = is_url_allowed(
            "https://api.example.com/v1",
            "https://service.example",
            &["https://*.example.com/*".into()],
            false,
        );
        assert!(r.is_ok());
    }

    #[test]
    fn http_in_production_blocked() {
        let r = is_url_allowed(
            "http://service.example/path",
            "http://service.example",
            &[],
            false,
        );
        assert!(matches!(r, Err(UrlError::HttpInProduction(_))));
    }

    #[test]
    fn http_allowed_in_dev() {
        let r = is_url_allowed(
            "http://service.example/path",
            "http://service.example",
            &[],
            true,
        );
        assert!(r.is_ok());
    }

    #[test]
    fn file_scheme_blocked() {
        let r = is_url_allowed("file:///etc/passwd", "https://service.example", &[], false);
        assert!(matches!(r, Err(UrlError::BlockedScheme(_))));
    }

    #[test]
    fn javascript_scheme_blocked() {
        let r = is_url_allowed(
            "javascript:alert(1)",
            "https://service.example",
            &[],
            false,
        );
        assert!(matches!(r, Err(UrlError::BlockedScheme(_))));
    }

    #[test]
    fn localhost_blocked() {
        let r = is_url_allowed(
            "https://localhost:8080/admin",
            "https://service.example",
            &[],
            false,
        );
        assert!(matches!(r, Err(UrlError::PrivateHost(_))));
    }

    #[test]
    fn loopback_127_blocked() {
        let r = is_url_allowed(
            "https://127.0.0.1/admin",
            "https://service.example",
            &[],
            false,
        );
        assert!(matches!(r, Err(UrlError::PrivateHost(_))));
    }

    #[test]
    fn private_network_192_blocked() {
        let r = is_url_allowed(
            "https://192.168.1.1/page",
            "https://service.example",
            &[],
            false,
        );
        assert!(matches!(r, Err(UrlError::PrivateHost(_))));
    }

    #[test]
    fn private_network_10_blocked() {
        let r = is_url_allowed(
            "https://10.0.0.5/page",
            "https://service.example",
            &[],
            false,
        );
        assert!(matches!(r, Err(UrlError::PrivateHost(_))));
    }

    #[test]
    fn unmatched_origin_rejected() {
        let r = is_url_allowed(
            "https://other.example/page",
            "https://service.example",
            &["https://allowed.example/*".into()],
            false,
        );
        assert!(matches!(r, Err(UrlError::NotAllowed(_))));
    }

    #[test]
    fn invalid_url_rejected() {
        let r = is_url_allowed("not a url", "https://service.example", &[], false);
        assert!(matches!(r, Err(UrlError::Parse(_))));
    }

    #[test]
    fn host_match_subdomain_or_apex() {
        // *.example.com 은 example.com 자체 + sub.example.com 모두 매칭
        assert!(host_match("example.com", "*.example.com"));
        assert!(host_match("a.example.com", "*.example.com"));
        assert!(host_match("a.b.example.com", "*.example.com"));
        assert!(!host_match("evil.com", "*.example.com"));
        assert!(!host_match("notexample.com", "*.example.com"));
    }
}
