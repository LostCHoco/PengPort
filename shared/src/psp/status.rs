//! PSP status response schema.
//!
//! 명세: `docs/spec/psp-v1.md` 섹션 6.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `manifest.endpoints.status` GET 응답.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StatusResponse {
    pub online: bool,

    #[serde(default)]
    pub metrics: Vec<Metric>,

    #[serde(default)]
    pub badges: Vec<Badge>,

    /// 현재 그 service 에 접속해 있는 사람 — service-native 신원.
    /// presence(모임 레이어)의 client-side 집계 소스. PengPort 는 호스팅하지 않고,
    /// 각 service 가 자기 status 로 보고한 것을 client 가 모아 roster 로 표시.
    #[serde(default)]
    pub present: Vec<Present>,

    /// RFC 3339 timestamp.
    #[serde(default)]
    pub last_updated: Option<String>,
}

/// presence 항목 — 현재 service 에 접속한 한 사람. service-native 신원.
///
/// forward-compatible: 미래 per-person 상세(접속시각·활동)는 선택 필드로 추가 가능
/// (serde default → 그 필드 없는 옛 어댑터 응답도 호환).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Present {
    /// service-native id (예: MC username). 식별·표시용.
    /// **untrusted** (어댑터가 채움) — fs/명령에 사용 금지, 표시만.
    pub id: String,

    /// 표시명 (없으면 client 가 `id` 사용).
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Metric {
    pub id: String,
    pub label: String,

    /// `type` 별 의미 다름. `players` 면 `{online, max, names}` 객체.
    pub value: Value,

    #[serde(rename = "type")]
    pub kind: MetricType,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MetricType {
    Number,
    Percentage,
    Bytes,
    Timestamp,
    String,
    Players,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Badge {
    pub id: String,
    pub label: String,
    pub level: BadgeLevel,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BadgeLevel {
    Info,
    Warning,
    Error,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserialize_minimal_status() {
        let json = r#"{"online": true}"#;
        let s: StatusResponse = serde_json::from_str(json).unwrap();
        assert!(s.online);
        assert!(s.metrics.is_empty());
        // present 누락 → 빈 배열 (forward/backward compatible: present 모르는 옛 어댑터 호환).
        assert!(s.present.is_empty());
    }

    #[test]
    fn deserialize_present() {
        let json = r#"
        {
          "online": true,
          "present": [
            {"id": "alphapeng", "label": "알파펭"},
            {"id": "betapeng"}
          ]
        }
        "#;
        let s: StatusResponse = serde_json::from_str(json).unwrap();
        assert_eq!(s.present.len(), 2);
        assert_eq!(s.present[0].id, "alphapeng");
        assert_eq!(s.present[0].label.as_deref(), Some("알파펭"));
        // label 누락 → None (client 가 id fallback).
        assert_eq!(s.present[1].label, None);

        // round-trip.
        let back: StatusResponse =
            serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.present.len(), 2);
    }

    #[test]
    fn deserialize_full_status() {
        let json = r#"
        {
          "online": true,
          "metrics": [
            {"id": "players", "label": "접속자", "type": "players",
             "value": {"online": 2, "max": 4, "names": ["alice", "bob"]}},
            {"id": "uptime", "label": "운영시간", "type": "timestamp", "value": "2026-04-27T15:00:00Z"}
          ],
          "badges": [
            {"id": "high-load", "label": "혼잡", "level": "warning"}
          ],
          "last_updated": "2026-04-27T18:00:00Z"
        }
        "#;
        let s: StatusResponse = serde_json::from_str(json).unwrap();
        assert_eq!(s.metrics.len(), 2);
        assert_eq!(s.metrics[0].kind, MetricType::Players);
        assert_eq!(s.badges[0].level, BadgeLevel::Warning);
    }
}
