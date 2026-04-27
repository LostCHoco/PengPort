//! Native action handlers — PSP v1 의 표준 native action kind 처리.
//!
//! 명세: `docs/spec/psp-v1.md` 섹션 9.
//!
//! ## 책임 분담
//!
//! `shared/actions/` 는 args 파싱 + 검증 + 의도 표현 (Intent enum) 까지.
//! 실제 OS 호출 (브라우저 열기, protocol handler dispatch, HTTP POST 전송 등)
//! 은 src-tauri 측이 Tauri API 사용해 처리한다 — shared 는 Tauri 무관여.
//!
//! ## 흐름
//!
//! ```text
//! manifest.actions[i] (raw)
//!     │
//!     ▼ actions::dispatch(action, ctx)
//! ActionIntent  ←─ 검증 + 정형화된 의도
//!     │
//!     ▼ src-tauri 가 받아서 OS 호출
//! 실제 동작
//! ```

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::psp::NativeActionKind;

pub mod manifest_check;
pub mod open_protocol;
pub mod open_url;
pub mod submit_form;
pub mod third_party;
pub mod url_check;

pub use manifest_check::{validate_manifest, ManifestCheckContext, ManifestValidationError};
pub use open_protocol::{validate_open_protocol, OpenProtocolArgs, OpenProtocolIntent};
pub use open_url::{validate_open_url, OpenUrlArgs, OpenUrlIntent};
pub use submit_form::{
    validate_submit_form, FormField, FormFieldType, FormFieldOption, SubmitFormArgs,
    SubmitFormIntent,
};
pub use third_party::{
    InstallHint, ThirdPartyAppIntegration, ThirdPartyAppIntent,
};

/// Action 검증·dispatch 시점의 컨텍스트.
///
/// `manifest_origin` 은 service URL (예: `https://my-todo.alice.example`).
/// `external_urls` 는 manifest 의 `permissions.external_urls` 패턴 목록.
/// `allow_http` 는 dev 모드에서 HTTP 허용 (production: false).
#[derive(Debug, Clone)]
pub struct ActionContext<'a> {
    pub manifest_origin: &'a str,
    pub external_urls: &'a [String],
    pub allow_http: bool,
}

/// 검증된 action 의 정형화된 의도. src-tauri 가 받아 실 OS 호출.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActionIntent {
    OpenUrl(OpenUrlIntent),
    OpenProtocol(OpenProtocolIntent),
    SubmitForm(SubmitFormIntent),
    ThirdPartyApp(ThirdPartyAppIntent),
}

#[derive(Debug, Error)]
pub enum ActionError {
    #[error("action 'kind' field 누락 또는 알 수 없음: {0}")]
    UnknownKind(String),

    #[error("args 파싱 실패: {0}")]
    ArgsParse(#[from] serde_json::Error),

    #[error("URL 검증 실패: {0}")]
    UrlCheck(#[from] url_check::UrlError),

    #[error("등록되지 않은 protocol scheme: {0}")]
    UnsupportedScheme(String),

    #[error("폼 필드 정의 오류: {0}")]
    InvalidFormField(String),

    #[error("third-party app '{0}' 가 본가 카탈로그에 없음")]
    UnknownApp(String),

    #[error("third-party action 의 'app' 필드 누락")]
    MissingAppField,

    #[error("아직 지원하지 않는 action kind: {0:?}")]
    Unsupported(NativeActionKind),
}

/// manifest 의 raw action 을 검증해서 ActionIntent 로 변환.
pub fn dispatch(
    kind: NativeActionKind,
    args: &serde_json::Value,
    ctx: &ActionContext<'_>,
) -> Result<ActionIntent, ActionError> {
    match kind {
        NativeActionKind::OpenUrl => {
            let intent = validate_open_url(args, ctx)?;
            Ok(ActionIntent::OpenUrl(intent))
        }
        NativeActionKind::OpenProtocol => {
            let intent = validate_open_protocol(args, ctx)?;
            Ok(ActionIntent::OpenProtocol(intent))
        }
        NativeActionKind::SubmitForm => {
            let intent = validate_submit_form(args, ctx)?;
            Ok(ActionIntent::SubmitForm(intent))
        }
        NativeActionKind::ThirdPartyApp => {
            let app_id = args
                .get("app")
                .and_then(serde_json::Value::as_str)
                .ok_or(ActionError::MissingAppField)?;
            let entry = third_party::lookup(app_id)
                .ok_or_else(|| ActionError::UnknownApp(app_id.to_string()))?;
            let config = args
                .get("config")
                .cloned()
                .unwrap_or(serde_json::Value::Null);
            let mut intent = entry.validate_config(&config, ctx)?;
            // install_hint 추출 (있으면, 무효한 형태면 무시 — 표시용)
            if let Some(hint) = args.get("install_hint") {
                if let Ok(parsed) = serde_json::from_value::<third_party::InstallHint>(hint.clone()) {
                    intent.install_hint = Some(parsed);
                }
            }
            Ok(ActionIntent::ThirdPartyApp(intent))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx() -> ActionContext<'static> {
        ActionContext {
            manifest_origin: "https://service.example",
            external_urls: &[],
            allow_http: false,
        }
    }

    #[test]
    fn dispatch_open_url_ok() {
        let args = json!({"url": "https://service.example/page"});
        let intent = dispatch(NativeActionKind::OpenUrl, &args, &ctx()).unwrap();
        match intent {
            ActionIntent::OpenUrl(i) => assert_eq!(i.url, "https://service.example/page"),
            _ => panic!(),
        }
    }

    #[test]
    fn dispatch_third_party_prism_launcher_ok() {
        let args = json!({
            "app": "prism-launcher",
            "config": {
                "host": "play.example.com",
                "port": 25565,
                "version": "1.21.1",
                "loader": "vanilla"
            },
            "install_hint": {"name": "Prism Launcher", "homepage": "https://prismlauncher.org/"}
        });
        let intent = dispatch(NativeActionKind::ThirdPartyApp, &args, &ctx()).unwrap();
        match intent {
            ActionIntent::ThirdPartyApp(i) => {
                assert_eq!(i.app_id, "prism-launcher");
                assert_eq!(i.install_hint.as_ref().unwrap().name, "Prism Launcher");
            }
            _ => panic!(),
        }
    }

    #[test]
    fn dispatch_third_party_unknown_app() {
        let args = json!({"app": "unknown-app", "config": {}});
        let err = dispatch(NativeActionKind::ThirdPartyApp, &args, &ctx()).unwrap_err();
        assert!(matches!(err, ActionError::UnknownApp(_)));
    }

    #[test]
    fn dispatch_third_party_missing_app() {
        let args = json!({"config": {}});
        let err = dispatch(NativeActionKind::ThirdPartyApp, &args, &ctx()).unwrap_err();
        assert!(matches!(err, ActionError::MissingAppField));
    }
}
