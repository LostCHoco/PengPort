//! 환경변수 기반 설정.
//!
//! ## 필수
//! - `INSTANCE_NAME`         — instance metadata 의 name (예: "펭돌서버")
//! - `INSTANCE_OPERATOR`     — operator name (예: "LostCHoco")
//!
//! ## 선택
//! - `BIND`                  — HTTP 리슨 (default `0.0.0.0:8080`)
//! - `INSTANCE_DESCRIPTION`  — instance description
//! - `INSTANCE_OPERATOR_CONTACT` — operator contact (이메일/URL)
//! - `INSTANCE_ICON_URL`     — instance icon
//! - `INSTANCE_AUTH_TYPE`    — `none` | `token` (default `none`)
//! - `INSTANCE_TOKEN_HINT`   — `auth.type=token` 시 사용자에게 보여줄 안내
//! - `INSTANCE_PUBLIC_BASE_URL` — instance metadata 의 endpoints URL prefix
//!                                  (gateway 의 외부 publish 도메인)
//! - `SERVICES_DIR`          — services.d/ 디렉토리 경로 (default `./services.d`)
//! - `EVENTS_TOKEN`          — catalog 보호용 token. 없으면 인증 없음

use std::env;
use std::path::PathBuf;

use anyhow::{Context, Result};

#[derive(Clone)]
pub struct SecretString(String);

impl SecretString {
    pub fn new(s: String) -> Self {
        Self(s)
    }
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for SecretString {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("\"***\"")
    }
}

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub bind: String,
    pub services_dir: PathBuf,
    pub events_token: Option<SecretString>,

    // instance metadata 필드
    pub instance_name: String,
    pub instance_description: Option<String>,
    pub operator_name: String,
    pub operator_contact: Option<String>,
    pub instance_icon_url: Option<String>,
    pub instance_auth_type: String,
    pub instance_token_hint: Option<String>,
    pub instance_public_base_url: Option<String>,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        let bind = env::var("BIND").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
        let services_dir = env::var("SERVICES_DIR")
            .unwrap_or_else(|_| "./services.d".to_string())
            .into();
        let events_token = env::var("EVENTS_TOKEN").ok().map(SecretString::new);

        let instance_name =
            env::var("INSTANCE_NAME").context("INSTANCE_NAME 환경변수 누락")?;
        let instance_description = env::var("INSTANCE_DESCRIPTION").ok();
        let operator_name =
            env::var("INSTANCE_OPERATOR").context("INSTANCE_OPERATOR 누락")?;
        let operator_contact = env::var("INSTANCE_OPERATOR_CONTACT").ok();
        let instance_icon_url = env::var("INSTANCE_ICON_URL").ok();
        let instance_auth_type =
            env::var("INSTANCE_AUTH_TYPE").unwrap_or_else(|_| "none".to_string());
        let instance_token_hint = env::var("INSTANCE_TOKEN_HINT").ok();
        let instance_public_base_url = env::var("INSTANCE_PUBLIC_BASE_URL").ok();

        Ok(Self {
            bind,
            services_dir,
            events_token,
            instance_name,
            instance_description,
            operator_name,
            operator_contact,
            instance_icon_url,
            instance_auth_type,
            instance_token_hint,
            instance_public_base_url,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_debug_masks() {
        let s = SecretString::new("hidden".into());
        assert_eq!(format!("{:?}", s), "\"***\"");
    }
}
