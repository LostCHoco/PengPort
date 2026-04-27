//! `submit_form` action — manifest 정의 폼 → POST.
//!
//! 명세: `docs/spec/psp-v1.md` 섹션 9.3.
//!
//! `fields: []` (빈 배열) 이면 폼 없이 즉시 POST 트리거 (단순 action).
//! 클라이언트가 폼 UI 렌더링은 src-tauri / React 측 담당.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::url_check::is_url_allowed;
use super::{ActionContext, ActionError};

#[derive(Debug, Clone, Deserialize)]
pub struct SubmitFormArgs {
    /// manifest base 의 path (`/pengport/...`) 또는 절대 URL.
    pub endpoint: String,

    #[serde(default)]
    pub fields: Vec<FormField>,

    #[serde(default)]
    pub submit_label: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FormField {
    pub name: String,
    pub label: String,

    #[serde(rename = "type")]
    pub kind: FormFieldType,

    #[serde(default)]
    pub required: bool,

    #[serde(default)]
    pub default: Option<Value>,

    #[serde(default)]
    pub options: Vec<FormFieldOption>,

    #[serde(default)]
    pub placeholder: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FormFieldType {
    String,
    Text,
    Number,
    Boolean,
    Select,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct FormFieldOption {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitFormIntent {
    /// 검증된 절대 URL — src-tauri 가 그대로 POST 호출.
    pub endpoint_url: String,
    pub fields: Vec<FormField>,
    pub submit_label: Option<String>,
}

pub fn validate_submit_form(
    raw: &Value,
    ctx: &ActionContext<'_>,
) -> Result<SubmitFormIntent, ActionError> {
    let args: SubmitFormArgs = serde_json::from_value(raw.clone())?;

    // endpoint: 절대 URL 또는 manifest base 의 path
    let endpoint_url = if args.endpoint.starts_with('/') {
        format!("{}{}", ctx.manifest_origin.trim_end_matches('/'), args.endpoint)
    } else {
        args.endpoint.clone()
    };
    is_url_allowed(
        &endpoint_url,
        ctx.manifest_origin,
        ctx.external_urls,
        ctx.allow_http,
    )?;

    // 각 field 검증
    for field in &args.fields {
        validate_field(field)?;
    }

    Ok(SubmitFormIntent {
        endpoint_url,
        fields: args.fields,
        submit_label: args.submit_label,
    })
}

fn validate_field(field: &FormField) -> Result<(), ActionError> {
    if field.name.is_empty() {
        return Err(ActionError::InvalidFormField("name 비어있음".into()));
    }
    if field.label.is_empty() {
        return Err(ActionError::InvalidFormField(format!(
            "label 비어있음: {}",
            field.name
        )));
    }
    if field.kind == FormFieldType::Select && field.options.is_empty() {
        return Err(ActionError::InvalidFormField(format!(
            "select 필드 '{}' 의 options 가 비어있음",
            field.name
        )));
    }
    Ok(())
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
    fn validates_relative_endpoint() {
        let args = json!({
            "endpoint": "/pengport/actions/quick_add",
            "fields": [
                {"name": "title", "label": "제목", "type": "string", "required": true}
            ]
        });
        let intent = validate_submit_form(&args, &ctx()).unwrap();
        assert_eq!(
            intent.endpoint_url,
            "https://service.example/pengport/actions/quick_add"
        );
        assert_eq!(intent.fields.len(), 1);
    }

    #[test]
    fn validates_absolute_endpoint_same_origin() {
        let args = json!({
            "endpoint": "https://service.example/api/x",
            "fields": []
        });
        let intent = validate_submit_form(&args, &ctx()).unwrap();
        assert_eq!(intent.endpoint_url, "https://service.example/api/x");
    }

    #[test]
    fn rejects_cross_origin_endpoint_without_external_urls() {
        let args = json!({"endpoint": "https://evil.example/api", "fields": []});
        let err = validate_submit_form(&args, &ctx()).unwrap_err();
        assert!(matches!(err, ActionError::UrlCheck(_)));
    }

    #[test]
    fn empty_fields_ok() {
        let args = json!({"endpoint": "/trigger", "fields": []});
        let intent = validate_submit_form(&args, &ctx()).unwrap();
        assert!(intent.fields.is_empty());
    }

    #[test]
    fn select_without_options_rejected() {
        let args = json!({
            "endpoint": "/api",
            "fields": [{"name": "x", "label": "X", "type": "select"}]
        });
        let err = validate_submit_form(&args, &ctx()).unwrap_err();
        assert!(matches!(err, ActionError::InvalidFormField(_)));
    }

    #[test]
    fn empty_field_name_rejected() {
        let args = json!({
            "endpoint": "/api",
            "fields": [{"name": "", "label": "X", "type": "string"}]
        });
        let err = validate_submit_form(&args, &ctx()).unwrap_err();
        assert!(matches!(err, ActionError::InvalidFormField(_)));
    }
}
