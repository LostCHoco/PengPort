//! PSP service manifest schema.
//!
//! 명세: `docs/spec/psp-v1.md` 섹션 5.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `/.well-known/pengport-service` GET 응답.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServiceManifest {
    pub schema_version: u32,
    pub id: String,
    pub name: String,

    #[serde(default)]
    pub description: Option<String>,

    #[serde(default)]
    pub icon_url: Option<String>,

    #[serde(default)]
    pub category_hint: Option<CategoryHint>,

    pub endpoints: ManifestEndpoints,
    pub actions: Vec<ServiceAction>,
    pub permissions: Permissions,
    pub psp_version: u32,
}

/// Service 가 자기 분류를 hint 로 알림. 클라이언트가 그룹핑/정렬에 사용 가능.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CategoryHint {
    Game,
    Media,
    Files,
    Communication,
    Dev,
    Infra,
    Productivity,
    Other,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ManifestEndpoints {
    /// Status polling URL.
    pub status: String,

    /// SSE events URL (선택).
    #[serde(default)]
    pub events: Option<String>,
}

/// 사용자 액션 — kind 별로 args 의 schema 가 다르다.
///
/// `args` 는 raw `serde_json::Value` 로 보관. 핸들러 호출 시점에 kind 별
/// 구체 타입으로 deserialize 한다 (트랙 10 의 native action handlers).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServiceAction {
    pub id: String,
    pub label: String,

    #[serde(default)]
    pub primary: bool,

    pub kind: NativeActionKind,

    #[serde(default = "Value::default")]
    pub args: Value,
}

/// PSP v1 의 표준 native action kinds.
///
/// 위험한 kind (`native_command`, `native_file_mount` 등) 는 명세 차원에서
/// 정의 안 함 — 추가 시 PSP 명세 갱신 필수.
///
/// `ThirdPartyApp` 은 본가 카탈로그의 외부 앱 통합 (예: Prism Launcher).
/// 새 외부 앱 추가는 본가 PR (`shared/src/actions/third_party/<app>.rs`).
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum NativeActionKind {
    OpenUrl,
    OpenProtocol,
    SubmitForm,
    #[serde(rename = "native_third_party_app")]
    ThirdPartyApp,
}

impl NativeActionKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OpenUrl => "open_url",
            Self::OpenProtocol => "open_protocol",
            Self::SubmitForm => "submit_form",
            Self::ThirdPartyApp => "native_third_party_app",
        }
    }
}

/// Service 가 요청하는 권한. Tier 2 동의 UI 의 입력. 클라이언트의 일관성
/// 검증 (manifest.actions[].kind ⊆ permissions.native_actions, 등) 필수.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Permissions {
    /// 사용 가능 native action kind 목록.
    #[serde(default)]
    pub native_actions: Vec<NativeActionKind>,

    /// 외부 URL glob 패턴 (예: `https://example.com/*`).
    #[serde(default)]
    pub external_urls: Vec<String>,

    /// 발행할 event type 목록.
    #[serde(default)]
    pub events: Vec<EventType>,
}

/// PSP 표준 event types.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    StatusChanged,
    Notification,
    Custom,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_minimal_manifest() {
        let json = r#"
        {
          "schema_version": 1,
          "id": "test-service",
          "name": "Test",
          "endpoints": { "status": "https://x.example/status" },
          "actions": [],
          "permissions": {},
          "psp_version": 1
        }
        "#;
        let m: ServiceManifest = serde_json::from_str(json).unwrap();
        assert_eq!(m.id, "test-service");
        assert_eq!(m.permissions.native_actions.len(), 0);
    }

    #[test]
    fn deserialize_full_manifest() {
        let json = r#"
        {
          "schema_version": 1,
          "id": "modded-mc",
          "name": "알파펭",
          "description": "Modded MC",
          "icon_url": "https://x.example/icon.png",
          "category_hint": "game",
          "endpoints": {
            "status": "https://x.example/status",
            "events": "https://x.example/events"
          },
          "actions": [
            {
              "id": "play",
              "label": "Play",
              "primary": true,
              "kind": "native_third_party_app",
              "args": {
                "app": "prism-launcher",
                "config": {"host": "x.example", "port": 25566, "version": "1.21.1", "loader": "fabric"}
              }
            }
          ],
          "permissions": {
            "native_actions": ["native_third_party_app"],
            "external_urls": ["https://x.example/*"],
            "events": ["status_changed"]
          },
          "psp_version": 1
        }
        "#;
        let m: ServiceManifest = serde_json::from_str(json).unwrap();
        assert_eq!(m.id, "modded-mc");
        assert_eq!(m.actions[0].kind, NativeActionKind::ThirdPartyApp);
        assert_eq!(m.actions[0].kind.as_str(), "native_third_party_app");
        assert_eq!(m.category_hint, Some(CategoryHint::Game));
    }

    #[test]
    fn deserialize_submit_form_action() {
        let json = r#"
        {
          "schema_version": 1,
          "id": "alice-todo",
          "name": "ToDo",
          "endpoints": {"status": "https://x.example/status"},
          "actions": [
            {
              "id": "quick_add",
              "label": "빠른 추가",
              "kind": "submit_form",
              "args": {
                "endpoint": "/pengport/actions/quick_add",
                "fields": [{"name": "title", "label": "제목", "type": "string", "required": true}]
              }
            }
          ],
          "permissions": {"native_actions": ["submit_form"]},
          "psp_version": 1
        }
        "#;
        let m: ServiceManifest = serde_json::from_str(json).unwrap();
        assert_eq!(m.actions[0].kind, NativeActionKind::SubmitForm);
        assert_eq!(m.actions[0].kind.as_str(), "submit_form");
    }

    #[test]
    fn native_action_kind_round_trip() {
        for k in [
            NativeActionKind::OpenUrl,
            NativeActionKind::OpenProtocol,
            NativeActionKind::SubmitForm,
            NativeActionKind::ThirdPartyApp,
        ] {
            let s = serde_json::to_string(&k).unwrap();
            let back: NativeActionKind = serde_json::from_str(&s).unwrap();
            assert_eq!(k, back);
        }
    }
}
