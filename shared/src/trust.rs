//! TOFU (Trust On First Use) 신뢰 저장소.
//!
//! 명세: `docs/spec/05-psp.md` 섹션 12 (3-tier 신뢰 모델).
//!
//! 사용자가 명시적으로 동의한 신뢰 대상을 영구 기록한다. 진짜 시크릿이 아닌
//! 메타데이터이므로 plain JSON in app_data 로 저장 (업계 패턴: SSH `known_hosts`,
//! VS Code Workspace Trust). OS keychain 은 토큰/패스워드 같은 진짜 시크릿 전용.
//!
//! ## 형식
//!
//! ```json
//! {
//!   "version": 1,
//!   "entries": [
//!     {
//!       "subject_kind": "third_party.prism-launcher",
//!       "subject_id": "play.example.com:25565",
//!       "display": "PengDoll Modded",
//!       "metadata": { "host": "...", "port": ..., "pack_bundle_url": "..." },
//!       "trusted_at": 1714200000
//!     }
//!   ]
//! }
//! ```
//!
//! ## subject_kind 규약
//!
//! - `third_party.{app_id}` — third-party app 별 신뢰 대상 (예: `third_party.prism-launcher`)
//! - 향후: `instance` (PSP 인스턴스), `service.{instance_origin_hash}` (service 별)
//!
//! ## subject_id 규약
//!
//! kind 별로 정함. 사람이 읽을 수 있는 형태 우선 (디버깅·UI 표시).
//! - prism-launcher: `host:port` (예: `play.example.com:25565`)
//!
//! ## metadata 규약
//!
//! kind 별 추가 비교 키. confirm 트리거 정책에서 사용.
//! - prism-launcher: `{ host, port, pack_bundle_url }` — pack_bundle_url 변경 시 재confirm

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// 현재 schema version. 형식이 깨지는 변경마다 증가시키고 마이그레이션 코드 추가.
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum TrustError {
    #[error("I/O 실패: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON 파싱 실패: {0}")]
    Parse(#[from] serde_json::Error),

    #[error("미지원 trust schema version: {0} (현재 {SCHEMA_VERSION})")]
    UnsupportedVersion(u32),
}

/// 한 신뢰 entry. subject_kind + subject_id 가 unique key.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustEntry {
    pub subject_kind: String,
    pub subject_id: String,
    pub display: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
    /// UNIX seconds. UI 가 사용자 locale 로 포맷.
    pub trusted_at: i64,
}

impl TrustEntry {
    /// 현재 시각으로 trusted_at 채워진 신규 entry.
    pub fn new(
        subject_kind: impl Into<String>,
        subject_id: impl Into<String>,
        display: impl Into<String>,
        metadata: serde_json::Value,
    ) -> Self {
        Self {
            subject_kind: subject_kind.into(),
            subject_id: subject_id.into(),
            display: display.into(),
            metadata,
            trusted_at: now_unix(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustData {
    pub version: u32,
    #[serde(default)]
    pub entries: Vec<TrustEntry>,
}

impl Default for TrustData {
    fn default() -> Self {
        Self {
            version: SCHEMA_VERSION,
            entries: Vec::new(),
        }
    }
}

/// 디스크 파일과 in-memory 데이터를 묶은 store. 명시적 `save()` 호출 시 디스크 동기화.
#[derive(Debug)]
pub struct TrustStore {
    path: PathBuf,
    data: TrustData,
}

impl TrustStore {
    /// 파일 로드. 파일 없으면 빈 store. 파싱 실패 / version 불일치 는 에러
    /// (자동 덮어쓰기 금지 — 사용자가 검토 + 백업 후 처리).
    pub fn load(path: impl Into<PathBuf>) -> Result<Self, TrustError> {
        let path = path.into();
        if !path.exists() {
            return Ok(Self {
                path,
                data: TrustData::default(),
            });
        }
        let bytes = fs::read(&path)?;
        let data: TrustData = serde_json::from_slice(&bytes)?;
        if data.version != SCHEMA_VERSION {
            return Err(TrustError::UnsupportedVersion(data.version));
        }
        Ok(Self { path, data })
    }

    /// atomic 쓰기 (`.tmp` → rename). 부분 쓰기로 인한 손상 방지.
    pub fn save(&self) -> Result<(), TrustError> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(&self.data)?;
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, &bytes)?;
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn is_trusted(&self, subject_kind: &str, subject_id: &str) -> bool {
        self.find(subject_kind, subject_id).is_some()
    }

    pub fn find(&self, subject_kind: &str, subject_id: &str) -> Option<&TrustEntry> {
        self.data
            .entries
            .iter()
            .find(|e| e.subject_kind == subject_kind && e.subject_id == subject_id)
    }

    /// 추가 또는 갱신. 같은 (kind, id) 면 덮어쓰기 (metadata/display/trusted_at 갱신).
    pub fn upsert(&mut self, entry: TrustEntry) {
        self.data.entries.retain(|e| {
            !(e.subject_kind == entry.subject_kind && e.subject_id == entry.subject_id)
        });
        self.data.entries.push(entry);
    }

    /// 삭제. 존재했으면 true.
    pub fn revoke(&mut self, subject_kind: &str, subject_id: &str) -> bool {
        let before = self.data.entries.len();
        self.data
            .entries
            .retain(|e| !(e.subject_kind == subject_kind && e.subject_id == subject_id));
        self.data.entries.len() != before
    }

    /// kind 필터 (None = 전체). UI 의 "신뢰 목록" 표시용.
    pub fn list(&self, subject_kind: Option<&str>) -> Vec<&TrustEntry> {
        self.data
            .entries
            .iter()
            .filter(|e| subject_kind.map_or(true, |k| e.subject_kind == k))
            .collect()
    }

    pub fn entries(&self) -> &[TrustEntry] {
        &self.data.entries
    }
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp_path(name: &str) -> PathBuf {
        let p = std::env::temp_dir()
            .join(format!("pengport-trust-{name}-{}.json", std::process::id()));
        let _ = fs::remove_file(&p);
        p
    }

    fn sample_entry(kind: &str, id: &str) -> TrustEntry {
        TrustEntry::new(kind, id, format!("display-{id}"), json!({"k": "v"}))
    }

    #[test]
    fn load_creates_empty_when_missing() {
        let path = temp_path("missing");
        let store = TrustStore::load(&path).unwrap();
        assert!(store.entries().is_empty());
    }

    #[test]
    fn upsert_then_save_then_load() {
        let path = temp_path("save-load");
        let mut store = TrustStore::load(&path).unwrap();
        store.upsert(sample_entry("third_party.prism-launcher", "host:25565"));
        store.save().unwrap();

        let loaded = TrustStore::load(&path).unwrap();
        assert_eq!(loaded.entries().len(), 1);
        assert!(loaded.is_trusted("third_party.prism-launcher", "host:25565"));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn upsert_replaces_same_key() {
        let path = temp_path("replace");
        let mut store = TrustStore::load(&path).unwrap();
        let mut e1 = sample_entry("third_party.prism-launcher", "host:25565");
        e1.display = "old".into();
        store.upsert(e1);
        let mut e2 = sample_entry("third_party.prism-launcher", "host:25565");
        e2.display = "new".into();
        store.upsert(e2);

        assert_eq!(store.entries().len(), 1);
        assert_eq!(store.entries()[0].display, "new");
    }

    #[test]
    fn revoke_removes() {
        let path = temp_path("revoke");
        let mut store = TrustStore::load(&path).unwrap();
        store.upsert(sample_entry("third_party.prism-launcher", "host:25565"));
        assert!(store.revoke("third_party.prism-launcher", "host:25565"));
        assert!(!store.is_trusted("third_party.prism-launcher", "host:25565"));
        assert!(!store.revoke("third_party.prism-launcher", "host:25565")); // 두 번째 제거: false
    }

    #[test]
    fn list_filters_by_kind() {
        let path = temp_path("list");
        let mut store = TrustStore::load(&path).unwrap();
        store.upsert(sample_entry("third_party.prism-launcher", "a:1"));
        store.upsert(sample_entry("third_party.prism-launcher", "b:2"));
        store.upsert(sample_entry("instance", "https://x"));

        assert_eq!(store.list(None).len(), 3);
        assert_eq!(store.list(Some("third_party.prism-launcher")).len(), 2);
        assert_eq!(store.list(Some("instance")).len(), 1);
        assert_eq!(store.list(Some("nonexistent")).len(), 0);
    }

    #[test]
    fn rejects_unsupported_version() {
        let path = temp_path("unsupported-version");
        // 미래 버전 데이터 시뮬레이션
        let bad = serde_json::to_vec_pretty(&serde_json::json!({
            "version": 999,
            "entries": []
        }))
        .unwrap();
        fs::write(&path, bad).unwrap();
        match TrustStore::load(&path) {
            Err(TrustError::UnsupportedVersion(999)) => {}
            other => panic!("expected UnsupportedVersion, got {other:?}"),
        }
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn atomic_save_does_not_corrupt_existing_on_panic() {
        // 기본적인 atomic 동작 검증: save 후 .tmp 가 남지 않음
        let path = temp_path("atomic");
        let mut store = TrustStore::load(&path).unwrap();
        store.upsert(sample_entry("k", "id"));
        store.save().unwrap();

        let tmp = path.with_extension("json.tmp");
        assert!(!tmp.exists(), ".tmp 파일이 남아 있으면 안 됨");
        let _ = fs::remove_file(&path);
    }
}
