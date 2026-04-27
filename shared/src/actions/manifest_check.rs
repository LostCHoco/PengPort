//! Manifest 전체의 일관성 검증.
//!
//! 명세: `docs/spec/psp-v1.md` 섹션 5.3.
//!
//! 이 검증이 PSP 의 핵심 보안 layer — manifest 가 거짓말할 수 없게 만든다.
//!
//! 검증 항목:
//! 1. `schema_version == 1`, `psp_version == 1`
//! 2. (선택) `manifest.id == catalog 의 service id`
//! 3. **`actions[].kind ⊆ permissions.native_actions`** — manifest 가 선언한
//!    권한과 실제 사용하는 kind 일관성. 거짓말 차단.
//! 4. 모든 URL (endpoints, icon_url, action args 안의 외부 URL) 이 manifest
//!    origin 또는 `permissions.external_urls` 매칭. URL allowlist.
//!
//! 검증 통과 못 한 manifest 는 카드 표시 안 함, 사용자에게 사유 표시.

use serde_json::Value;
use thiserror::Error;

use crate::psp::manifest::{NativeActionKind, ServiceManifest};

use super::url_check::{is_url_allowed, UrlError};

#[derive(Debug, Error)]
pub enum ManifestValidationError {
    #[error("schema_version 비호환 (필요 1, 받음 {0})")]
    SchemaVersionMismatch(u32),

    #[error("psp_version 비호환 (필요 1, 받음 {0})")]
    PspVersionMismatch(u32),

    #[error("manifest.id 가 catalog id 와 불일치 (catalog: '{expected}', manifest: '{actual}')")]
    IdMismatch { expected: String, actual: String },

    #[error("action '{action_id}' 의 kind '{kind}' 가 permissions.native_actions 에 선언 안 됨")]
    ActionNotPermitted {
        action_id: String,
        kind: &'static str,
    },

    #[error("URL '{url}' 검증 실패 (위치: {location}): {source}")]
    UrlNotAllowed {
        location: String,
        url: String,
        #[source]
        source: UrlError,
    },
}

pub struct ManifestCheckContext<'a> {
    /// service base URL (manifest 가 호스팅된 origin).
    pub base_url: &'a str,

    /// catalog 의 service id — manifest.id 와 매칭 검사 (선택).
    pub catalog_id: Option<&'a str>,

    /// dev 모드에서 HTTP 허용 여부.
    pub allow_http: bool,
}

pub fn validate_manifest(
    m: &ServiceManifest,
    ctx: &ManifestCheckContext<'_>,
) -> Result<(), ManifestValidationError> {
    // 1. version
    if m.schema_version != 1 {
        return Err(ManifestValidationError::SchemaVersionMismatch(m.schema_version));
    }
    if m.psp_version != 1 {
        return Err(ManifestValidationError::PspVersionMismatch(m.psp_version));
    }

    // 2. catalog id 매칭 (선택)
    if let Some(expected) = ctx.catalog_id {
        if m.id != expected {
            return Err(ManifestValidationError::IdMismatch {
                expected: expected.to_string(),
                actual: m.id.clone(),
            });
        }
    }

    // 3. actions[].kind ⊆ permissions.native_actions (거짓말 차단)
    for action in &m.actions {
        if !m.permissions.native_actions.contains(&action.kind) {
            return Err(ManifestValidationError::ActionNotPermitted {
                action_id: action.id.clone(),
                kind: action.kind.as_str(),
            });
        }
    }

    // 4. URL allowlist
    for (location, url) in collect_urls(m) {
        is_url_allowed(
            &url,
            ctx.base_url,
            &m.permissions.external_urls,
            ctx.allow_http,
        )
        .map_err(|source| ManifestValidationError::UrlNotAllowed {
            location,
            url,
            source,
        })?;
    }

    Ok(())
}

/// manifest 의 모든 외부 URL 을 수집 (위치 라벨 포함).
fn collect_urls(m: &ServiceManifest) -> Vec<(String, String)> {
    let mut urls = Vec::new();

    urls.push(("endpoints.status".into(), m.endpoints.status.clone()));
    if let Some(events) = &m.endpoints.events {
        urls.push(("endpoints.events".into(), events.clone()));
    }
    if let Some(icon) = &m.icon_url {
        urls.push(("icon_url".into(), icon.clone()));
    }

    for action in &m.actions {
        match action.kind {
            NativeActionKind::OpenUrl => {
                if let Some(url) = action.args.get("url").and_then(Value::as_str) {
                    urls.push((
                        format!("actions['{}'].args.url", action.id),
                        url.to_string(),
                    ));
                }
            }
            NativeActionKind::SubmitForm => {
                // 절대 URL 만 manifest 검증 단계에서 봄 — relative 는 base_url 과 결합됨
                if let Some(ep) = action.args.get("endpoint").and_then(Value::as_str) {
                    if ep.starts_with("http://") || ep.starts_with("https://") {
                        urls.push((
                            format!("actions['{}'].args.endpoint", action.id),
                            ep.to_string(),
                        ));
                    }
                }
            }
            NativeActionKind::ThirdPartyApp => {
                // install_hint.homepage 도 사용자에게 노출 + 클릭 가능 → 검증.
                if let Some(homepage) = action
                    .args
                    .get("install_hint")
                    .and_then(|h| h.get("homepage"))
                    .and_then(Value::as_str)
                {
                    urls.push((
                        format!("actions['{}'].args.install_hint.homepage", action.id),
                        homepage.to_string(),
                    ));
                }
                // config 안의 URL (예: prism-launcher 의 packwiz_url) 은
                // third_party entry 가 자체 검증 (별도 라운드).
            }
            NativeActionKind::OpenProtocol => {
                // OS protocol scheme — URL allowlist 영역 아님 (scheme allowlist 별도).
            }
        }
    }

    urls
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_manifest(json: &str) -> ServiceManifest {
        serde_json::from_str(json).expect("manifest 파싱 실패")
    }

    fn ctx(base_url: &'static str) -> ManifestCheckContext<'static> {
        ManifestCheckContext {
            base_url,
            catalog_id: None,
            allow_http: false,
        }
    }

    fn ctx_with_id(base_url: &'static str, catalog_id: &'static str) -> ManifestCheckContext<'static> {
        ManifestCheckContext {
            base_url,
            catalog_id: Some(catalog_id),
            allow_http: false,
        }
    }

    const VALID_MANIFEST: &str = r#"
    {
      "schema_version": 1,
      "id": "test",
      "name": "Test",
      "icon_url": "https://service.example/icon.png",
      "endpoints": {
        "status": "https://service.example/status",
        "events": "https://service.example/events"
      },
      "actions": [
        {
          "id": "open",
          "label": "열기",
          "kind": "open_url",
          "args": {"url": "https://service.example"}
        }
      ],
      "permissions": {
        "native_actions": ["open_url"],
        "external_urls": ["https://service.example/*"],
        "events": ["status_changed"]
      },
      "psp_version": 1
    }
    "#;

    #[test]
    fn valid_manifest_passes() {
        let m = parse_manifest(VALID_MANIFEST);
        let r = validate_manifest(&m, &ctx("https://service.example"));
        assert!(r.is_ok(), "got: {:?}", r);
    }

    #[test]
    fn schema_version_mismatch_rejected() {
        let m = parse_manifest(&VALID_MANIFEST.replace("\"schema_version\": 1", "\"schema_version\": 99"));
        let r = validate_manifest(&m, &ctx("https://service.example"));
        assert!(matches!(r, Err(ManifestValidationError::SchemaVersionMismatch(99))));
    }

    #[test]
    fn psp_version_mismatch_rejected() {
        let m = parse_manifest(&VALID_MANIFEST.replace("\"psp_version\": 1", "\"psp_version\": 99"));
        let r = validate_manifest(&m, &ctx("https://service.example"));
        assert!(matches!(r, Err(ManifestValidationError::PspVersionMismatch(99))));
    }

    #[test]
    fn id_mismatch_rejected() {
        let m = parse_manifest(VALID_MANIFEST);
        let r = validate_manifest(&m, &ctx_with_id("https://service.example", "different-id"));
        assert!(matches!(r, Err(ManifestValidationError::IdMismatch { .. })));
    }

    #[test]
    fn id_match_passes() {
        let m = parse_manifest(VALID_MANIFEST);
        let r = validate_manifest(&m, &ctx_with_id("https://service.example", "test"));
        assert!(r.is_ok());
    }

    /// 거짓말 manifest 차단 핵심 테스트 — actions 가 permissions 에 없는 kind 호출.
    #[test]
    fn lying_manifest_rejected_action_not_permitted() {
        // permissions 에 open_url 만 있는데 actions 에 native_third_party_app 사용
        let json = r#"
        {
          "schema_version": 1,
          "id": "test",
          "name": "Test",
          "endpoints": {"status": "https://service.example/status"},
          "actions": [
            {
              "id": "evil",
              "label": "Evil",
              "kind": "native_third_party_app",
              "args": {"app": "fake"}
            }
          ],
          "permissions": {
            "native_actions": ["open_url"],
            "external_urls": [],
            "events": []
          },
          "psp_version": 1
        }
        "#;
        let m = parse_manifest(json);
        let r = validate_manifest(&m, &ctx("https://service.example"));
        match r {
            Err(ManifestValidationError::ActionNotPermitted { action_id, kind }) => {
                assert_eq!(action_id, "evil");
                assert_eq!(kind, "native_third_party_app");
            }
            other => panic!("expected ActionNotPermitted, got {:?}", other),
        }
    }

    #[test]
    fn icon_url_file_scheme_rejected() {
        let m = parse_manifest(&VALID_MANIFEST.replace(
            "\"icon_url\": \"https://service.example/icon.png\"",
            "\"icon_url\": \"file:///etc/passwd\"",
        ));
        let r = validate_manifest(&m, &ctx("https://service.example"));
        assert!(matches!(r, Err(ManifestValidationError::UrlNotAllowed { .. })));
    }

    #[test]
    fn status_endpoint_localhost_rejected() {
        let m = parse_manifest(&VALID_MANIFEST.replace(
            "\"status\": \"https://service.example/status\"",
            "\"status\": \"https://localhost:8080/admin\"",
        ));
        let r = validate_manifest(&m, &ctx("https://service.example"));
        assert!(matches!(r, Err(ManifestValidationError::UrlNotAllowed { .. })));
    }

    #[test]
    fn open_url_args_cross_origin_rejected() {
        // open_url 의 url 이 manifest origin 도 external_urls 도 아님
        let json = r#"
        {
          "schema_version": 1,
          "id": "test",
          "name": "Test",
          "endpoints": {"status": "https://service.example/status"},
          "actions": [
            {"id": "evil", "label": "x", "kind": "open_url",
             "args": {"url": "https://evil.example/x"}}
          ],
          "permissions": {
            "native_actions": ["open_url"],
            "external_urls": [],
            "events": []
          },
          "psp_version": 1
        }
        "#;
        let m = parse_manifest(json);
        let r = validate_manifest(&m, &ctx("https://service.example"));
        match r {
            Err(ManifestValidationError::UrlNotAllowed { location, url, .. }) => {
                assert!(location.contains("evil"), "location: {}", location);
                assert!(location.contains(".args.url"), "location: {}", location);
                assert!(url.contains("evil.example"));
            }
            other => panic!("expected UrlNotAllowed, got {:?}", other),
        }
    }

    #[test]
    fn open_url_args_external_urls_match_passes() {
        let json = r#"
        {
          "schema_version": 1,
          "id": "test",
          "name": "Test",
          "endpoints": {"status": "https://service.example/status"},
          "actions": [
            {"id": "open", "label": "x", "kind": "open_url",
             "args": {"url": "https://cdn.example.com/x"}}
          ],
          "permissions": {
            "native_actions": ["open_url"],
            "external_urls": ["https://cdn.example.com/*"],
            "events": []
          },
          "psp_version": 1
        }
        "#;
        let m = parse_manifest(json);
        let r = validate_manifest(&m, &ctx("https://service.example"));
        assert!(r.is_ok(), "got: {:?}", r);
    }

    #[test]
    fn submit_form_relative_endpoint_skipped() {
        // relative endpoint 는 manifest 검증 단계에서 검사 안 함 (action::dispatch 에서 정규화)
        let json = r#"
        {
          "schema_version": 1,
          "id": "test",
          "name": "Test",
          "endpoints": {"status": "https://service.example/status"},
          "actions": [
            {"id": "submit", "label": "x", "kind": "submit_form",
             "args": {"endpoint": "/api/x", "fields": []}}
          ],
          "permissions": {
            "native_actions": ["submit_form"],
            "external_urls": [],
            "events": []
          },
          "psp_version": 1
        }
        "#;
        let m = parse_manifest(json);
        let r = validate_manifest(&m, &ctx("https://service.example"));
        assert!(r.is_ok(), "got: {:?}", r);
    }

    #[test]
    fn submit_form_absolute_endpoint_cross_origin_rejected() {
        let json = r#"
        {
          "schema_version": 1,
          "id": "test",
          "name": "Test",
          "endpoints": {"status": "https://service.example/status"},
          "actions": [
            {"id": "submit", "label": "x", "kind": "submit_form",
             "args": {"endpoint": "https://evil.example/api", "fields": []}}
          ],
          "permissions": {
            "native_actions": ["submit_form"],
            "external_urls": [],
            "events": []
          },
          "psp_version": 1
        }
        "#;
        let m = parse_manifest(json);
        let r = validate_manifest(&m, &ctx("https://service.example"));
        assert!(matches!(r, Err(ManifestValidationError::UrlNotAllowed { .. })));
    }

    #[test]
    fn third_party_install_hint_homepage_validated() {
        // install_hint.homepage 가 cross-origin → 거부
        let json = r#"
        {
          "schema_version": 1,
          "id": "test",
          "name": "Test",
          "endpoints": {"status": "https://service.example/status"},
          "actions": [
            {"id": "play", "label": "Play", "kind": "native_third_party_app",
             "args": {
               "app": "prism-launcher",
               "config": {},
               "install_hint": {"name": "Prism", "homepage": "https://evil.example/"}
             }}
          ],
          "permissions": {
            "native_actions": ["native_third_party_app"],
            "external_urls": [],
            "events": []
          },
          "psp_version": 1
        }
        "#;
        let m = parse_manifest(json);
        let r = validate_manifest(&m, &ctx("https://service.example"));
        match r {
            Err(ManifestValidationError::UrlNotAllowed { location, .. }) => {
                assert!(location.contains("install_hint"), "{}", location);
            }
            other => panic!("expected UrlNotAllowed, got {:?}", other),
        }
    }

    #[test]
    fn third_party_install_hint_in_external_urls_passes() {
        let json = r#"
        {
          "schema_version": 1,
          "id": "test",
          "name": "Test",
          "endpoints": {"status": "https://service.example/status"},
          "actions": [
            {"id": "play", "label": "Play", "kind": "native_third_party_app",
             "args": {
               "app": "prism-launcher",
               "config": {},
               "install_hint": {"name": "Prism", "homepage": "https://prismlauncher.org/"}
             }}
          ],
          "permissions": {
            "native_actions": ["native_third_party_app"],
            "external_urls": ["https://prismlauncher.org/*"],
            "events": []
          },
          "psp_version": 1
        }
        "#;
        let m = parse_manifest(json);
        let r = validate_manifest(&m, &ctx("https://service.example"));
        assert!(r.is_ok(), "got: {:?}", r);
    }
}
