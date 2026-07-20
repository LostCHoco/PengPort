//! Prism Launcher entry — Minecraft 클라이언트 + packwiz 통합.
//!
//! 명세: `docs/spec/psp-v1.md` 섹션 9.4 의 `prism-launcher` 카탈로그 entry.
//!
//! 검증 layer 만 — 실 Prism 실행·packwiz sync 는 src-tauri 측에서
//! `ThirdPartyAppIntent` 받아 `shared::prism::sync_all()` 등 호출.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::actions::url_check::{is_private_or_loopback_host, is_url_allowed};
use crate::actions::{ActionContext, ActionError};

use super::{ThirdPartyAppIntegration, ThirdPartyAppIntent};

pub struct PrismLauncher;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PrismLauncherConfig {
    /// 도메인 또는 public IP. private/loopback 은 차단 (kiosk 빌드 예외 추후).
    pub host: String,
    pub port: u16,
    pub version: String,
    pub loader: PrismLoader,

    #[serde(default)]
    pub loader_version: Option<String>,

    /// 인증 팩 번들(tar.gz) URL. 런처가 EVENTS_TOKEN 으로 1회 인증 GET → 인스턴스
    /// `.minecraft/.packwiz-src/` 에 추출 → 로컬 packwiz-installer 실행. overrides(제작자
    /// 저작물)는 인증 채널로만 배포(공개 재배포 금지), mod jar 는 CF 공개 CDN.
    #[serde(default)]
    pub pack_bundle_url: Option<String>,

    #[serde(default)]
    pub java_major: Option<u32>,

    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PrismLoader {
    Vanilla,
    Fabric,
    Forge,
    Neoforge,
    Quilt,
}

impl ThirdPartyAppIntegration for PrismLauncher {
    fn id(&self) -> &'static str {
        "prism-launcher"
    }

    fn display_name(&self) -> &'static str {
        "Prism Launcher"
    }

    fn homepage(&self) -> &'static str {
        "https://prismlauncher.org/"
    }

    fn validate_config(
        &self,
        config: &Value,
        ctx: &ActionContext<'_>,
    ) -> Result<ThirdPartyAppIntent, ActionError> {
        let cfg: PrismLauncherConfig = serde_json::from_value(config.clone())?;

        // port
        if cfg.port == 0 {
            return Err(ActionError::InvalidFormField(
                "prism-launcher: port 0 unsupported".into(),
            ));
        }

        // host private/loopback 차단
        if is_private_or_loopback_host(&cfg.host) {
            return Err(ActionError::InvalidFormField(format!(
                "prism-launcher: host '{}' is private/loopback (kiosk dev only)",
                cfg.host
            )));
        }

        // loader_version 필수 (vanilla 외)
        if cfg.loader != PrismLoader::Vanilla && cfg.loader_version.is_none() {
            return Err(ActionError::InvalidFormField(format!(
                "prism-launcher: loader '{:?}' requires loader_version",
                cfg.loader
            )));
        }

        // pack_bundle_url 검증 (same-origin — 매니페스트 origin 또는 external_urls 허용)
        if let Some(pack_bundle_url) = &cfg.pack_bundle_url {
            is_url_allowed(
                pack_bundle_url,
                ctx.manifest_origin,
                ctx.external_urls,
                ctx.allow_http,
            )?;
        }

        // version 형식 검증 (단순 — 1.x.y 또는 비슷)
        if cfg.version.is_empty() {
            return Err(ActionError::InvalidFormField(
                "prism-launcher: version empty".into(),
            ));
        }

        let config_payload = serde_json::to_value(&cfg).map_err(ActionError::ArgsParse)?;
        Ok(ThirdPartyAppIntent {
            app_id: self.id().to_string(),
            config: config_payload,
            install_hint: None, // dispatch 가 채움
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn ctx_no_external() -> ActionContext<'static> {
        ActionContext {
            manifest_origin: "https://service.example",
            external_urls: &[],
            allow_http: false,
        }
    }

    #[test]
    fn validates_minimal_fabric() {
        let cfg = json!({
            "host": "play.example.com",
            "port": 25565,
            "version": "1.21.1",
            "loader": "fabric",
            "loader_version": "0.18.4"
        });
        let intent = PrismLauncher.validate_config(&cfg, &ctx_no_external()).unwrap();
        assert_eq!(intent.app_id, "prism-launcher");
    }

    #[test]
    fn validates_vanilla_without_loader_version() {
        let cfg = json!({
            "host": "play.example.com",
            "port": 25565,
            "version": "1.21.1",
            "loader": "vanilla"
        });
        let intent = PrismLauncher.validate_config(&cfg, &ctx_no_external()).unwrap();
        assert_eq!(intent.app_id, "prism-launcher");
    }

    #[test]
    fn rejects_fabric_without_loader_version() {
        let cfg = json!({
            "host": "play.example.com",
            "port": 25565,
            "version": "1.21.1",
            "loader": "fabric"
        });
        let err = PrismLauncher.validate_config(&cfg, &ctx_no_external()).unwrap_err();
        assert!(matches!(err, ActionError::InvalidFormField(_)));
    }

    #[test]
    fn rejects_localhost_host() {
        let cfg = json!({
            "host": "localhost",
            "port": 25565,
            "version": "1.21.1",
            "loader": "vanilla"
        });
        let err = PrismLauncher.validate_config(&cfg, &ctx_no_external()).unwrap_err();
        assert!(matches!(err, ActionError::InvalidFormField(_)));
    }

    #[test]
    fn rejects_private_network_host() {
        let cfg = json!({
            "host": "192.168.1.10",
            "port": 25565,
            "version": "1.21.1",
            "loader": "vanilla"
        });
        let err = PrismLauncher.validate_config(&cfg, &ctx_no_external()).unwrap_err();
        assert!(matches!(err, ActionError::InvalidFormField(_)));
    }

    #[test]
    fn rejects_port_zero() {
        let cfg = json!({
            "host": "play.example.com",
            "port": 0,
            "version": "1.21.1",
            "loader": "vanilla"
        });
        let err = PrismLauncher.validate_config(&cfg, &ctx_no_external()).unwrap_err();
        assert!(matches!(err, ActionError::InvalidFormField(_)));
    }

    #[test]
    fn rejects_pack_bundle_url_cross_origin() {
        let cfg = json!({
            "host": "play.example.com",
            "port": 25565,
            "version": "1.21.1",
            "loader": "fabric",
            "loader_version": "0.18.4",
            "pack_bundle_url": "https://evil.example/pack.tar.gz"
        });
        let err = PrismLauncher.validate_config(&cfg, &ctx_no_external()).unwrap_err();
        assert!(matches!(err, ActionError::UrlCheck(_)));
    }

    #[test]
    fn validates_pack_bundle_url_in_external_urls() {
        let cfg = json!({
            "host": "play.example.com",
            "port": 25565,
            "version": "1.21.1",
            "loader": "fabric",
            "loader_version": "0.18.4",
            "pack_bundle_url": "https://cdn.example.com/pack/modded.tar.gz"
        });
        let urls = vec!["https://cdn.example.com/*".to_string()];
        let ctx = ActionContext {
            manifest_origin: "https://service.example",
            external_urls: &urls,
            allow_http: false,
        };
        PrismLauncher.validate_config(&cfg, &ctx).unwrap();
    }

    #[test]
    fn rejects_empty_version() {
        let cfg = json!({
            "host": "play.example.com",
            "port": 25565,
            "version": "",
            "loader": "vanilla"
        });
        let err = PrismLauncher.validate_config(&cfg, &ctx_no_external()).unwrap_err();
        assert!(matches!(err, ActionError::InvalidFormField(_)));
    }

    #[test]
    fn rejects_missing_required_field() {
        let cfg = json!({
            "host": "play.example.com",
            "port": 25565,
            "loader": "vanilla"
            // version 누락
        });
        let err = PrismLauncher.validate_config(&cfg, &ctx_no_external()).unwrap_err();
        assert!(matches!(err, ActionError::ArgsParse(_)));
    }
}
