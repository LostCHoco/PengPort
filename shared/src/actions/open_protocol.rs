//! `open_protocol` action — OS protocol handler 호출 (예: `steam://run/123`).
//!
//! 명세: `docs/spec/psp-v1.md` 섹션 9.2.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{ActionContext, ActionError};

/// PSP v1 의 등록된 protocol scheme 화이트리스트. 새 scheme 추가는 명세 갱신 필수.
pub const ALLOWED_SCHEMES: &[&str] = &["steam", "obsidian", "vscode", "mailto", "tel"];

#[derive(Debug, Clone, Deserialize)]
pub struct OpenProtocolArgs {
    pub scheme: String,
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenProtocolIntent {
    pub scheme: String,
    pub target: String,
    /// 클라이언트가 OS 에 dispatch 할 완성 URL (`{scheme}://{target}`).
    pub full_url: String,
}

pub fn validate_open_protocol(
    raw: &Value,
    _ctx: &ActionContext<'_>,
) -> Result<OpenProtocolIntent, ActionError> {
    let args: OpenProtocolArgs = serde_json::from_value(raw.clone())?;

    let scheme = args.scheme.to_ascii_lowercase();
    if !ALLOWED_SCHEMES.contains(&scheme.as_str()) {
        return Err(ActionError::UnsupportedScheme(scheme));
    }

    // target 은 protocol 별 의미라 자유. 단 control character 차단.
    if args.target.chars().any(|c| c.is_control()) {
        return Err(ActionError::UnsupportedScheme(format!(
            "control char in target: {}",
            args.scheme
        )));
    }

    let full_url = format!("{scheme}://{}", args.target);
    Ok(OpenProtocolIntent {
        scheme,
        target: args.target,
        full_url,
    })
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
    fn validates_steam_scheme() {
        let args = json!({"scheme": "steam", "target": "run/123"});
        let intent = validate_open_protocol(&args, &ctx()).unwrap();
        assert_eq!(intent.scheme, "steam");
        assert_eq!(intent.full_url, "steam://run/123");
    }

    #[test]
    fn case_insensitive_scheme() {
        let args = json!({"scheme": "STEAM", "target": "run/123"});
        let intent = validate_open_protocol(&args, &ctx()).unwrap();
        assert_eq!(intent.scheme, "steam");
    }

    #[test]
    fn rejects_unknown_scheme() {
        let args = json!({"scheme": "evil", "target": "x"});
        let err = validate_open_protocol(&args, &ctx()).unwrap_err();
        assert!(matches!(err, ActionError::UnsupportedScheme(_)));
    }

    #[test]
    fn rejects_control_char_in_target() {
        let args = json!({"scheme": "steam", "target": "run\n/123"});
        let err = validate_open_protocol(&args, &ctx()).unwrap_err();
        assert!(matches!(err, ActionError::UnsupportedScheme(_)));
    }

    #[test]
    fn missing_field_errors() {
        let args = json!({"scheme": "steam"});
        let err = validate_open_protocol(&args, &ctx()).unwrap_err();
        assert!(matches!(err, ActionError::ArgsParse(_)));
    }

    #[test]
    fn allowed_schemes_constant() {
        // PSP v1 명세와 일치 검증
        for s in ["steam", "obsidian", "vscode", "mailto", "tel"] {
            assert!(ALLOWED_SCHEMES.contains(&s));
        }
    }
}
