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

    /// RFC 3339 timestamp.
    #[serde(default)]
    pub last_updated: Option<String>,
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
