//! Instance metadata + service manifest HTTP fetch.
//!
//! 명세: `docs/spec/psp-v1.md` 섹션 3, 5.
//!
//! `/.well-known/pengport-instance` (instance metadata) +
//! `/.well-known/pengport-service` (service manifest).
//!
//! 본 모듈은 검증·동의 흐름의 1차 fetch 만 담당. 일관성 검증은 호출자가
//! `actions::manifest_check::validate_manifest` 별도 호출.

use std::io::Read;
use std::time::Duration;

use thiserror::Error;

use super::{InstanceMetadata, ServiceManifest};
use super::catalog::ServicesCatalog;

#[derive(Debug, Error)]
pub enum FetchError {
    #[error("URL 형식 오류 (http/https 필요): {0}")]
    InvalidUrl(String),

    #[error("HTTP 요청 실패: {0}")]
    Http(String),

    #[error("HTTP {0}")]
    HttpStatus(u16),

    #[error("인증 실패 (401) — 토큰 갱신 필요")]
    Unauthorized,

    #[error("응답 본문 읽기 실패: {0}")]
    ReadBody(String),

    #[error("JSON 파싱 실패: {0}")]
    Json(String),
}

/// Instance metadata fetch.
///
/// `instance_base_url` 예: `https://pengdoll.duckdns.org`.
pub fn fetch_instance_metadata(
    instance_base_url: &str,
    timeout: Duration,
) -> Result<InstanceMetadata, FetchError> {
    let url = build_well_known_url(instance_base_url, "pengport-instance")?;
    let body = http_get(&url, timeout, None)?;
    serde_json::from_str(&body).map_err(|e| FetchError::Json(e.to_string()))
}

/// Service manifest fetch. `bearer_token` 은 인스턴스 auth.type=token 시 필요.
pub fn fetch_service_manifest(
    service_base_url: &str,
    bearer_token: Option<&str>,
    timeout: Duration,
) -> Result<ServiceManifest, FetchError> {
    let url = build_well_known_url(service_base_url, "pengport-service")?;
    let body = http_get(&url, timeout, bearer_token)?;
    serde_json::from_str(&body).map_err(|e| FetchError::Json(e.to_string()))
}

/// Services catalog fetch. URL 은 instance metadata 의 `endpoints.catalog`.
/// JSON 또는 TOML 응답 자동 감지: Content-Type 무시, 본문 첫 글자 `{` 면 JSON, 아니면 TOML.
pub fn fetch_services_catalog(
    catalog_url: &str,
    bearer_token: Option<&str>,
    timeout: Duration,
) -> Result<ServicesCatalog, FetchError> {
    if !catalog_url.starts_with("http://") && !catalog_url.starts_with("https://") {
        return Err(FetchError::InvalidUrl(catalog_url.to_string()));
    }
    let body = http_get(catalog_url, timeout, bearer_token)?;
    let trimmed = body.trim_start();
    if trimmed.starts_with('{') {
        serde_json::from_str(&body).map_err(|e| FetchError::Json(e.to_string()))
    } else {
        toml::from_str(&body).map_err(|e| FetchError::Json(e.to_string()))
    }
}

/// 초대 코드 redeem — `POST <base>/invite/redeem` `{"code": ...}` → 현재 EVENTS_TOKEN.
///
/// invite B: 안정적 `INVITE_CODE` 를 *현재* 토큰으로 교환. 토큰은 URL/링크에 일절
/// 나타나지 않고 이 POST 응답 본문으로만 전달된다. 코드 불일치 → `Unauthorized`,
/// redeem 비활성 인스턴스(INVITE_CODE 미설정) → `HttpStatus(404)`.
pub fn redeem_invite(
    instance_base_url: &str,
    code: &str,
    timeout: Duration,
) -> Result<String, FetchError> {
    if !instance_base_url.starts_with("http://") && !instance_base_url.starts_with("https://") {
        return Err(FetchError::InvalidUrl(instance_base_url.to_string()));
    }
    let url = format!(
        "{}/invite/redeem",
        instance_base_url.trim_end_matches('/')
    );

    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build()
        .new_agent();

    let body = serde_json::json!({ "code": code });
    // ureq 의 http_status_as_error 기본값 차이를 양쪽 다 견고하게 처리:
    // 4xx 가 Err(StatusCode) 로 오든, Ok 응답의 status 로 오든 동일하게 매핑.
    let mut response = match agent.post(&url).send_json(&body) {
        Ok(r) => r,
        Err(ureq::Error::StatusCode(code)) => return Err(status_to_err(code)),
        Err(e) => return Err(FetchError::Http(e.to_string())),
    };
    let status = response.status();
    if !status.is_success() {
        return Err(status_to_err(status.as_u16()));
    }

    let parsed: RedeemResponse = response
        .body_mut()
        .read_json()
        .map_err(|e| FetchError::Json(e.to_string()))?;
    Ok(parsed.token)
}

fn status_to_err(code: u16) -> FetchError {
    match code {
        401 => FetchError::Unauthorized,
        other => FetchError::HttpStatus(other),
    }
}

#[derive(serde::Deserialize)]
struct RedeemResponse {
    token: String,
}

fn build_well_known_url(base: &str, name: &str) -> Result<String, FetchError> {
    if !base.starts_with("http://") && !base.starts_with("https://") {
        return Err(FetchError::InvalidUrl(base.to_string()));
    }
    let trimmed = base.trim_end_matches('/');
    Ok(format!("{trimmed}/.well-known/{name}"))
}

fn http_get(url: &str, timeout: Duration, bearer: Option<&str>) -> Result<String, FetchError> {
    let agent = ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        .build()
        .new_agent();

    let mut req = agent.get(url);
    if let Some(token) = bearer {
        req = req.header("Authorization", format!("Bearer {token}"));
    }

    let mut response = req.call().map_err(|e| FetchError::Http(e.to_string()))?;
    let status = response.status();
    if status == 401 {
        return Err(FetchError::Unauthorized);
    }
    if !status.is_success() {
        return Err(FetchError::HttpStatus(status.as_u16()));
    }
    let mut body = String::new();
    response
        .body_mut()
        .as_reader()
        .read_to_string(&mut body)
        .map_err(|e| FetchError::ReadBody(e.to_string()))?;
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn well_known_url_https() {
        let url = build_well_known_url("https://x.example", "pengport-instance").unwrap();
        assert_eq!(url, "https://x.example/.well-known/pengport-instance");
    }

    #[test]
    fn well_known_url_trailing_slash() {
        let url = build_well_known_url("https://x.example/", "pengport-service").unwrap();
        assert_eq!(url, "https://x.example/.well-known/pengport-service");
    }

    #[test]
    fn well_known_url_with_path_preserved() {
        // base 에 path 있어도 그대로 유지 (인스턴스가 sub-path 호스팅 가능)
        let url = build_well_known_url("https://x.example/instance1", "pengport-instance").unwrap();
        assert_eq!(url, "https://x.example/instance1/.well-known/pengport-instance");
    }

    #[test]
    fn well_known_url_invalid_scheme() {
        let r = build_well_known_url("ftp://x.example", "pengport-instance");
        assert!(matches!(r, Err(FetchError::InvalidUrl(_))));
    }

    #[test]
    fn well_known_url_no_scheme() {
        let r = build_well_known_url("example.com", "pengport-instance");
        assert!(matches!(r, Err(FetchError::InvalidUrl(_))));
    }
}
