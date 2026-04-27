//! PSP instance metadata schema.
//!
//! 명세: `docs/spec/psp-v1.md` 섹션 3.
//!
//! `/.well-known/pengport-instance` GET 응답.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InstanceMetadata {
    pub schema_version: u32,
    pub name: String,

    #[serde(default)]
    pub description: Option<String>,

    pub operator: OperatorInfo,
    pub endpoints: InstanceEndpoints,
    pub auth: InstanceAuth,

    #[serde(default)]
    pub icon_url: Option<String>,

    /// 클라이언트 최소 버전 (semver).
    #[serde(default)]
    pub pengport_min_version: Option<String>,

    /// (Phase 3+) 인스턴스 fingerprint pinning.
    #[serde(default)]
    pub public_key_fingerprint: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OperatorInfo {
    pub name: String,

    #[serde(default)]
    pub contact: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InstanceEndpoints {
    /// services.toml 또는 합쳐진 services.d/ URL.
    pub catalog: String,

    /// gateway SSE URL (선택). 없으면 클라이언트가 service event 직접 구독.
    #[serde(default)]
    pub events: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct InstanceAuth {
    #[serde(rename = "type")]
    pub kind: InstanceAuthType,

    /// `type=token` 시 사용자에게 보여줄 안내 문구.
    #[serde(default)]
    pub token_hint: Option<String>,

    /// `type=oauth2` 시 OAuth endpoints (Phase 2+).
    #[serde(default)]
    pub oauth2: Option<OAuth2Endpoints>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InstanceAuthType {
    None,
    Token,
    Oauth2,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OAuth2Endpoints {
    pub authorization_url: String,
    pub token_url: String,

    #[serde(default)]
    pub scopes: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_token_instance() {
        let json = r#"
        {
          "schema_version": 1,
          "name": "펭돌서버",
          "operator": {"name": "LostCHoco"},
          "endpoints": {
            "catalog": "https://x.example/services.toml",
            "events":  "https://x.example/events"
          },
          "auth": {
            "type": "token",
            "token_hint": "Settings 에서 입력"
          }
        }
        "#;
        let m: InstanceMetadata = serde_json::from_str(json).unwrap();
        assert_eq!(m.name, "펭돌서버");
        assert_eq!(m.auth.kind, InstanceAuthType::Token);
        assert_eq!(m.endpoints.events.as_deref(), Some("https://x.example/events"));
    }

    #[test]
    fn deserialize_none_auth() {
        let json = r#"
        {
          "schema_version": 1,
          "name": "공개 인스턴스",
          "operator": {"name": "Public"},
          "endpoints": {"catalog": "https://x.example/services.toml"},
          "auth": {"type": "none"}
        }
        "#;
        let m: InstanceMetadata = serde_json::from_str(json).unwrap();
        assert_eq!(m.auth.kind, InstanceAuthType::None);
    }
}
