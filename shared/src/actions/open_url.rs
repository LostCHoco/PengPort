//! `open_url` action — https URL 시스템 브라우저 열기.
//!
//! 명세: `docs/spec/psp-v1.md` 섹션 9.1.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::url_check::is_url_allowed;
use super::{ActionContext, ActionError};

#[derive(Debug, Clone, Deserialize)]
pub struct OpenUrlArgs {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenUrlIntent {
    pub url: String,
}

pub fn validate_open_url(
    raw: &Value,
    ctx: &ActionContext<'_>,
) -> Result<OpenUrlIntent, ActionError> {
    let args: OpenUrlArgs = serde_json::from_value(raw.clone())?;
    is_url_allowed(&args.url, ctx.manifest_origin, ctx.external_urls, ctx.allow_http)?;
    Ok(OpenUrlIntent { url: args.url })
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
    fn validates_same_origin() {
        let args = json!({"url": "https://service.example/path"});
        let intent = validate_open_url(&args, &ctx()).unwrap();
        assert_eq!(intent.url, "https://service.example/path");
    }

    #[test]
    fn rejects_file_scheme() {
        let args = json!({"url": "file:///etc/passwd"});
        let err = validate_open_url(&args, &ctx()).unwrap_err();
        assert!(matches!(err, ActionError::UrlCheck(_)));
    }

    #[test]
    fn rejects_localhost() {
        let args = json!({"url": "https://localhost:8080/x"});
        let err = validate_open_url(&args, &ctx()).unwrap_err();
        assert!(matches!(err, ActionError::UrlCheck(_)));
    }

    #[test]
    fn rejects_unmatched_origin_without_external_urls() {
        let args = json!({"url": "https://other.example/x"});
        let err = validate_open_url(&args, &ctx()).unwrap_err();
        assert!(matches!(err, ActionError::UrlCheck(_)));
    }

    #[test]
    fn missing_url_field_errors() {
        let args = json!({});
        let err = validate_open_url(&args, &ctx()).unwrap_err();
        assert!(matches!(err, ActionError::ArgsParse(_)));
    }
}
