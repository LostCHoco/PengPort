//! PSP events stream schema (SSE).
//!
//! 명세: `docs/spec/psp-v1.md` 섹션 7.
//!
//! 두 가지 컨텍스트:
//! - `ServiceEvent`: service 직접 events endpoint 의 event payload
//! - `InstanceEvent`: gateway 가 multiplexing 한 instance-level 이벤트

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::status::StatusResponse;

/// service 의 `manifest.endpoints.events` SSE 스트림에서 받는 이벤트.
///
/// SSE 의 `event:` 필드를 tag 로, `data:` JSON 을 content 로 매핑.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "event", content = "data", rename_all = "snake_case")]
pub enum ServiceEvent {
    StatusChanged(StatusResponse),
    Notification(NotificationEvent),
    Custom(CustomEvent),
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NotificationEvent {
    pub level: NotificationLevel,
    pub title: String,

    #[serde(default)]
    pub body: Option<String>,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum NotificationLevel {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CustomEvent {
    pub event_type: String,
    pub payload: Value,
}

/// instance 의 `endpoints.events` (gateway) SSE 스트림에서 받는 이벤트.
///
/// gateway 가 인스턴스 안 모든 service event 를 wrapping 해서 push 한다.
/// service 별 event 와 달리 `service_id` 가 항상 포함.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum InstanceEvent {
    ServiceStatusChanged {
        service_id: String,
        status: StatusResponse,
    },
    ServiceNotification {
        service_id: String,
        notification: NotificationEvent,
    },
    ServiceCustom {
        service_id: String,
        event_type: String,
        payload: Value,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn service_event_status_changed_round_trip() {
        let ev = ServiceEvent::StatusChanged(StatusResponse {
            online: true,
            metrics: vec![],
            badges: vec![],
            last_updated: None,
        });
        let json = serde_json::to_string(&ev).unwrap();
        let back: ServiceEvent = serde_json::from_str(&json).unwrap();
        match back {
            ServiceEvent::StatusChanged(s) => assert!(s.online),
            _ => panic!("expected StatusChanged"),
        }
    }

    #[test]
    fn service_event_notification_round_trip() {
        let ev = ServiceEvent::Notification(NotificationEvent {
            level: NotificationLevel::Info,
            title: "테스트".to_string(),
            body: Some("body".to_string()),
        });
        let json = serde_json::to_string(&ev).unwrap();
        let back: ServiceEvent = serde_json::from_str(&json).unwrap();
        match back {
            ServiceEvent::Notification(n) => {
                assert_eq!(n.level, NotificationLevel::Info);
                assert_eq!(n.title, "테스트");
            }
            _ => panic!("expected Notification"),
        }
    }
}
