//! third-party app descriptor 로컬 저장소 — [`super::store::LibraryStore`]와 완전히 같은
//! 패턴(atomic JSON 파일, `.tmp` → rename, version 불일치 시 에러)을 그대로 따른다.
//!
//! PengPort 는 descriptor 0개로 시작한다 — 레시피가 로컬 파일 + 링크 임포트로 들어오듯,
//! third-party app descriptor 도 같은 경로로 들어온다: 사용자가 링크를 열면 그 안의
//! 레시피와 함께 descriptor 도 이 스토어에 반영된다(`bundle.rs`/`import.rs` 참고).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::actions::ThirdPartyAppDescriptor;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Error)]
pub enum ThirdPartyStoreError {
    #[error("I/O 실패: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON 파싱 실패: {0}")]
    Parse(#[from] serde_json::Error),

    #[error("미지원 third-party app store schema version: {0} (현재 {SCHEMA_VERSION})")]
    UnsupportedVersion(u32),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ThirdPartyStoreData {
    version: u32,
    #[serde(default)]
    descriptors: Vec<ThirdPartyAppDescriptor>,
}

impl Default for ThirdPartyStoreData {
    fn default() -> Self {
        Self {
            version: SCHEMA_VERSION,
            descriptors: Vec::new(),
        }
    }
}

/// 디스크 파일과 in-memory 데이터를 묶은 store. 명시적 `save()` 호출 시 디스크 동기화.
#[derive(Debug)]
pub struct ThirdPartyAppStore {
    path: PathBuf,
    data: ThirdPartyStoreData,
}

impl ThirdPartyAppStore {
    /// 파일 로드. 파일 없으면 빈 store(= PengPort 가 아는 third-party app 이 하나도
    /// 없는 정상 초기 상태). 파싱 실패 / version 불일치는 에러(자동 덮어쓰기 금지).
    pub fn load(path: impl Into<PathBuf>) -> Result<Self, ThirdPartyStoreError> {
        let path = path.into();
        if !path.exists() {
            return Ok(Self {
                path,
                data: ThirdPartyStoreData::default(),
            });
        }
        let bytes = fs::read(&path)?;
        let data: ThirdPartyStoreData = serde_json::from_slice(&bytes)?;
        if data.version != SCHEMA_VERSION {
            return Err(ThirdPartyStoreError::UnsupportedVersion(data.version));
        }
        Ok(Self { path, data })
    }

    /// atomic 쓰기 (`.tmp` → rename).
    pub fn save(&self) -> Result<(), ThirdPartyStoreError> {
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

    pub fn get(&self, id: &str) -> Option<&ThirdPartyAppDescriptor> {
        self.data.descriptors.iter().find(|d| d.id == id)
    }

    pub fn contains(&self, id: &str) -> bool {
        self.get(id).is_some()
    }

    /// descriptor 를 추가 또는 갱신 — 같은 id 면 덮어쓰기.
    pub fn upsert(&mut self, descriptor: ThirdPartyAppDescriptor) {
        self.data.descriptors.retain(|d| d.id != descriptor.id);
        self.data.descriptors.push(descriptor);
    }

    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.data.descriptors.len();
        self.data.descriptors.retain(|d| d.id != id);
        self.data.descriptors.len() != before
    }

    pub fn list(&self) -> &[ThirdPartyAppDescriptor] {
        &self.data.descriptors
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(name: &str) -> PathBuf {
        let p = std::env::temp_dir()
            .join(format!("pengport-thirdparty-store-{name}-{}.json", std::process::id()));
        let _ = fs::remove_file(&p);
        p
    }

    fn sample_descriptor(id: &str) -> ThirdPartyAppDescriptor {
        ThirdPartyAppDescriptor {
            id: id.to_string(),
            label: Some(format!("App {id}")),
            exe_filename: "test_app.exe".to_string(),
            download_strategy: None,
            instances_subfolder: Some("instances".to_string()),
            system_appdata_folder_name: None,
            readiness_signal: None,
            launch_args_template: vec![],
            post_download_marker_files: vec![],
        }
    }

    #[test]
    fn load_creates_empty_when_missing() {
        let path = temp_path("missing");
        let store = ThirdPartyAppStore::load(&path).unwrap();
        assert!(store.list().is_empty());
    }

    #[test]
    fn upsert_then_save_then_load() {
        let path = temp_path("save-load");
        let mut store = ThirdPartyAppStore::load(&path).unwrap();
        store.upsert(sample_descriptor("test_app"));
        store.save().unwrap();

        let loaded = ThirdPartyAppStore::load(&path).unwrap();
        assert_eq!(loaded.list().len(), 1);
        assert!(loaded.contains("test_app"));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn upsert_replaces_same_id() {
        let path = temp_path("replace");
        let mut store = ThirdPartyAppStore::load(&path).unwrap();
        store.upsert(sample_descriptor("test_app"));
        let mut updated = sample_descriptor("test_app");
        updated.label = Some("새 이름".to_string());
        store.upsert(updated);

        assert_eq!(store.list().len(), 1);
        assert_eq!(store.get("test_app").unwrap().label, Some("새 이름".to_string()));
    }

    #[test]
    fn remove_removes() {
        let path = temp_path("remove");
        let mut store = ThirdPartyAppStore::load(&path).unwrap();
        store.upsert(sample_descriptor("test_app"));
        assert!(store.remove("test_app"));
        assert!(!store.contains("test_app"));
        assert!(!store.remove("test_app"));
    }

    #[test]
    fn rejects_unsupported_version() {
        let path = temp_path("unsupported-version");
        let bad = serde_json::to_vec_pretty(&serde_json::json!({
            "version": 999,
            "descriptors": []
        }))
        .unwrap();
        fs::write(&path, bad).unwrap();
        match ThirdPartyAppStore::load(&path) {
            Err(ThirdPartyStoreError::UnsupportedVersion(999)) => {}
            other => panic!("expected UnsupportedVersion, got {other:?}"),
        }
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn atomic_save_does_not_leave_tmp_file() {
        let path = temp_path("atomic");
        let mut store = ThirdPartyAppStore::load(&path).unwrap();
        store.upsert(sample_descriptor("test_app"));
        store.save().unwrap();

        let tmp = path.with_extension("json.tmp");
        assert!(!tmp.exists());
        let _ = fs::remove_file(&path);
    }
}
