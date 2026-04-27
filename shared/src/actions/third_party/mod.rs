//! Third-party app integration registry.
//!
//! 명세: `docs/spec/psp-v1.md` 섹션 9.4.
//!
//! 본가 클라이언트가 보유하는 외부 앱 카탈로그. 새 앱 추가는 본가 PR
//! (entry 파일 신설 + `lookup()` / `registered_ids()` 갱신).
//!
//! Phase 1 카탈로그: `prism-launcher`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{ActionContext, ActionError};

pub mod prism_launcher;

/// 외부 앱 통합 entry. 각 앱이 이 trait 구현.
pub trait ThirdPartyAppIntegration: Send + Sync {
    fn id(&self) -> &'static str;
    fn display_name(&self) -> &'static str;
    fn homepage(&self) -> &'static str;

    /// `args.config` 검증 + 정형화된 Intent 반환.
    /// `manifest.permissions.external_urls` 와 결합한 URL allowlist 도 entry 가 적용.
    fn validate_config(
        &self,
        config: &Value,
        ctx: &ActionContext<'_>,
    ) -> Result<ThirdPartyAppIntent, ActionError>;
}

/// 검증된 third-party action intent — src-tauri 가 실 OS 호출.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThirdPartyAppIntent {
    pub app_id: String,
    /// entry 가 검증한 config (재직렬화).
    pub config: Value,
    /// manifest 의 install_hint (검증된 형태). dispatch 단계에서 채움.
    #[serde(default)]
    pub install_hint: Option<InstallHint>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstallHint {
    pub name: String,
    #[serde(default)]
    pub homepage: Option<String>,
}

/// Phase 1 카탈로그 lookup. 새 앱 = 본가 PR.
pub fn lookup(app_id: &str) -> Option<Box<dyn ThirdPartyAppIntegration>> {
    match app_id {
        "prism-launcher" => Some(Box::new(prism_launcher::PrismLauncher)),
        _ => None,
    }
}

/// 등록된 모든 app id 목록 (UI 에서 "지원 앱" 표시 등에 활용).
pub const REGISTERED_IDS: &[&str] = &["prism-launcher"];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_prism_launcher() {
        let entry = lookup("prism-launcher").unwrap();
        assert_eq!(entry.id(), "prism-launcher");
        assert_eq!(entry.display_name(), "Prism Launcher");
    }

    #[test]
    fn lookup_unknown_returns_none() {
        assert!(lookup("steam").is_none());
        assert!(lookup("").is_none());
    }

    #[test]
    fn registered_ids_matches_lookup() {
        for id in REGISTERED_IDS {
            assert!(lookup(id).is_some(), "REGISTERED_IDS 의 '{id}' 가 lookup 에 없음");
        }
    }
}
