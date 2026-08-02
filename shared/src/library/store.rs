//! 로컬 라이브러리 저장소 — [`LibraryEntry`] 리스트의 유일한 영속 데이터.
//!
//! `trust.rs`의 `TrustStore`와 같은 패턴(atomic JSON 파일, `.tmp` → rename)을 그대로
//! 따른다. 카탈로그 fetch·인스턴스 개념 없음 — 이 파일이 사용자의 "인스턴스"다.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::recipe::Recipe;

/// v8 레시피 스키마(archives/files 항목의 `target` 필드 제거, 루트는 `launch` 하나로
/// 결정) 도입으로 3→4. `target`이 `sub_path`를 갖고 있던 third-party 앱 하위 경로
/// 항목들은 그 `sub_path`를 `extract_to`/`path`에 직접 접어 넣어야 의미가 보존되므로
/// (단순히 필드를 무시하면 잘못된 위치를 가리키게 됨), 버전을 올려 명시적으로
/// 거부한다(재마이그레이션 필요). `ArchiveExtraction::order`(필수, 압축 실행 순서를
/// 배열 순서 대신 명시적으로 지정) 추가로 4→5 — 이 필드 없는 구버전 레시피는 파싱
/// 자체가 실패해야 하므로 버전을 올려 명시적으로 거부.
pub const SCHEMA_VERSION: u32 = 5;

#[derive(Debug, Error)]
pub enum LibraryError {
    #[error("I/O 실패: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON 파싱 실패: {0}")]
    Parse(#[from] serde_json::Error),

    #[error("미지원 library schema version: {0} (현재 {SCHEMA_VERSION})")]
    UnsupportedVersion(u32),
}

/// [`Recipe`]를 감싸는 로컬 레이어. export/import(`bundle.rs`) 시 `recipe`만 대상 —
/// `local_root_override`는 그 컴퓨터에서만 의미가 있어 절대 링크에 실리지 않는다.
///
/// (예: 포터블 앱을 이미 다른 경로에 설치해뒀다면 그 폴더를 루트로 지정할 수 있어야
/// 하는데, 절대경로는 공유 데이터인 레시피에 넣으면 안 되므로 로컬 오버라이드로 분리.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryEntry {
    pub recipe: Recipe,
    #[serde(default)]
    pub local_root_override: Option<PathBuf>,
    /// 사용자가 실제로 선택한 [`super::recipe::OptionalGroup`] id 들 — 로컬 전용(레시피
    /// 공유 데이터 아님, `local_root_override`와 같은 축). `None`이면 "아직 한 번도
    /// 확인 안 함" — `recipe.optional_groups`가 비어있지 않으면 설치 전에 확인
    /// 다이얼로그를 띄워야 한다는 신호. `Some(set)`은 이미 확인됨(빈 집합도 유효한
    /// 선택 — "필수 파일만" 의미).
    #[serde(default)]
    pub selected_optional_groups: Option<HashSet<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LibraryData {
    version: u32,
    #[serde(default)]
    entries: Vec<LibraryEntry>,
}

impl Default for LibraryData {
    fn default() -> Self {
        Self {
            version: SCHEMA_VERSION,
            entries: Vec::new(),
        }
    }
}

/// 디스크 파일과 in-memory 데이터를 묶은 store. 명시적 `save()` 호출 시 디스크 동기화.
#[derive(Debug)]
pub struct LibraryStore {
    path: PathBuf,
    data: LibraryData,
}

impl LibraryStore {
    /// 파일 로드. 파일 없으면 빈 store. 파싱 실패 / version 불일치는 에러
    /// (자동 덮어쓰기 금지 — `trust.rs`와 동일 정책).
    pub fn load(path: impl Into<PathBuf>) -> Result<Self, LibraryError> {
        let path = path.into();
        if !path.exists() {
            return Ok(Self {
                path,
                data: LibraryData::default(),
            });
        }
        let bytes = fs::read(&path)?;
        let data: LibraryData = serde_json::from_slice(&bytes)?;
        if data.version != SCHEMA_VERSION {
            return Err(LibraryError::UnsupportedVersion(data.version));
        }
        Ok(Self { path, data })
    }

    /// atomic 쓰기 (`.tmp` → rename).
    pub fn save(&self) -> Result<(), LibraryError> {
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

    pub fn get(&self, id: &str) -> Option<&LibraryEntry> {
        self.data.entries.iter().find(|e| e.recipe.id == id)
    }

    pub fn contains(&self, id: &str) -> bool {
        self.get(id).is_some()
    }

    /// 레시피를 추가 또는 갱신 — 같은 id 면 덮어쓰기. 기존 `local_root_override`는
    /// 보존한다(레시피 갱신은 공유 데이터만 바뀌는 것이지, 사용자의 로컬 설정을
    /// 초기화할 이유가 없다). 이미 있던 항목이면 배열 안 위치도 그대로 둔다(제자리
    /// 교체) — 라이브러리 목록 순서가 곧 이 배열 순서라, 지우고 맨 뒤에 다시 넣으면
    /// 편집할 때마다 카드가 맨 뒤로 밀려나는 문제가 있었다. 새 항목만 맨 뒤에 추가.
    /// 링크 임포트 confirm 시점에 이미 "이 항목들을 추가/갱신합니다"를 사용자에게
    /// 보여준 뒤 호출되므로, 여기서 별도 중복 거부는 하지 않는다(호출자가 정책 결정 —
    /// `import.rs` 참고).
    pub fn upsert(&mut self, recipe: Recipe) {
        if let Some(pos) = self.data.entries.iter().position(|e| e.recipe.id == recipe.id) {
            let existing_override = self.data.entries[pos].local_root_override.clone();
            let existing_selection = self.data.entries[pos].selected_optional_groups.clone();
            self.data.entries[pos] = LibraryEntry {
                recipe,
                local_root_override: existing_override,
                selected_optional_groups: existing_selection,
            };
        } else {
            self.data.entries.push(LibraryEntry {
                recipe,
                local_root_override: None,
                selected_optional_groups: None,
            });
        }
    }

    /// 라이브러리 카드 순서(사용자가 드래그로 정한 순서)를 `ids`대로 재배치. `ids`에
    /// 없는 기존 항목은 유실 방지를 위해 뒤에 그대로 이어붙인다(프론트가 보낸 목록이
    /// 어떤 이유로 일부 누락돼도 데이터는 안 사라짐) — `ids`에만 있고 실제 스토어엔
    /// 없는 id 는 조용히 무시.
    pub fn reorder(&mut self, ids: &[String]) {
        let mut remaining = std::mem::take(&mut self.data.entries);
        let mut reordered = Vec::with_capacity(remaining.len());
        for id in ids {
            if let Some(pos) = remaining.iter().position(|e| &e.recipe.id == id) {
                reordered.push(remaining.remove(pos));
            }
        }
        reordered.extend(remaining);
        self.data.entries = reordered;
    }

    /// 로컬 루트 오버라이드 설정/해제(`None`이면 해제). 대상 id 가 없으면 false.
    pub fn set_local_root_override(&mut self, id: &str, root: Option<PathBuf>) -> bool {
        match self.data.entries.iter_mut().find(|e| e.recipe.id == id) {
            Some(e) => {
                e.local_root_override = root;
                true
            }
            None => false,
        }
    }

    /// 선택 그룹 확정/변경(`None`이면 "아직 확인 안 함" 상태로 되돌림 — 전체 삭제 후
    /// 다음 설치 때 다시 물어보게 하려는 용도). 대상 id 가 없으면 false.
    pub fn set_selected_optional_groups(&mut self, id: &str, groups: Option<HashSet<String>>) -> bool {
        match self.data.entries.iter_mut().find(|e| e.recipe.id == id) {
            Some(e) => {
                e.selected_optional_groups = groups;
                true
            }
            None => false,
        }
    }

    /// 삭제. 존재했으면 true.
    pub fn remove(&mut self, id: &str) -> bool {
        let before = self.data.entries.len();
        self.data.entries.retain(|e| e.recipe.id != id);
        self.data.entries.len() != before
    }

    pub fn list(&self) -> &[LibraryEntry] {
        &self.data.entries
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::recipe::{ConfigFileFormat, LaunchAction, OverrideContent, RecipeFile};

    fn temp_path(name: &str) -> PathBuf {
        let p = std::env::temp_dir()
            .join(format!("pengport-library-{name}-{}.json", std::process::id()));
        let _ = fs::remove_file(&p);
        p
    }

    fn sample_recipe(id: &str) -> Recipe {
        Recipe {
            id: id.to_string(),
            name: format!("App {id}"),
            recipe_info: Default::default(),
            archives: vec![],
            files: vec![RecipeFile {
                path: "x.ini".to_string(),
                override_content: Some(OverrideContent::ConfigPatch {
                    format: ConfigFileFormat::Ini,
                    patch: serde_json::json!({}),
                }),
                optional_group: None,
            }],
            optional_groups: vec![],
            folder_rules: vec![],
            launch: LaunchAction::SpawnProcess {
                entry_point: "x.exe".to_string(),
                entry_args: vec![],
            },
        }
    }

    #[test]
    fn load_creates_empty_when_missing() {
        let path = temp_path("missing");
        let store = LibraryStore::load(&path).unwrap();
        assert!(store.list().is_empty());
    }

    #[test]
    fn upsert_then_save_then_load() {
        let path = temp_path("save-load");
        let mut store = LibraryStore::load(&path).unwrap();
        store.upsert(sample_recipe("sample-service"));
        store.save().unwrap();

        let loaded = LibraryStore::load(&path).unwrap();
        assert_eq!(loaded.list().len(), 1);
        assert!(loaded.contains("sample-service"));
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn upsert_replaces_same_id() {
        let path = temp_path("replace");
        let mut store = LibraryStore::load(&path).unwrap();
        store.upsert(sample_recipe("sample-service"));
        let mut updated = sample_recipe("sample-service");
        updated.name = "새 이름".to_string();
        store.upsert(updated);

        assert_eq!(store.list().len(), 1);
        assert_eq!(store.list()[0].recipe.name, "새 이름");
    }

    /// 편집(기존 id 갱신)이 라이브러리 목록 순서를 안 바꾸는지 — 예전엔 지우고 맨
    /// 뒤에 다시 넣어서 편집할 때마다 카드가 맨 뒤로 밀려나는 버그가 있었다.
    #[test]
    fn upsert_keeps_position_of_existing_entry() {
        let path = temp_path("keep-position");
        let mut store = LibraryStore::load(&path).unwrap();
        store.upsert(sample_recipe("a"));
        store.upsert(sample_recipe("b"));
        store.upsert(sample_recipe("c"));

        let mut updated_b = sample_recipe("b");
        updated_b.name = "B (수정됨)".to_string();
        store.upsert(updated_b);

        let ids: Vec<&str> = store.list().iter().map(|e| e.recipe.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"]);
        assert_eq!(store.get("b").unwrap().recipe.name, "B (수정됨)");
    }

    #[test]
    fn reorder_applies_given_order() {
        let path = temp_path("reorder-basic");
        let mut store = LibraryStore::load(&path).unwrap();
        store.upsert(sample_recipe("a"));
        store.upsert(sample_recipe("b"));
        store.upsert(sample_recipe("c"));

        store.reorder(&["c".to_string(), "a".to_string(), "b".to_string()]);

        let ids: Vec<&str> = store.list().iter().map(|e| e.recipe.id.as_str()).collect();
        assert_eq!(ids, vec!["c", "a", "b"]);
    }

    #[test]
    fn reorder_appends_missing_entries_and_ignores_unknown_ids() {
        let path = temp_path("reorder-partial");
        let mut store = LibraryStore::load(&path).unwrap();
        store.upsert(sample_recipe("a"));
        store.upsert(sample_recipe("b"));
        store.upsert(sample_recipe("c"));

        // "b"는 목록에서 빠뜨리고, 존재하지 않는 "z"를 섞어서 호출 — 데이터 유실도,
        // 오류도 없어야 함.
        store.reorder(&["c".to_string(), "z".to_string(), "a".to_string()]);

        let ids: Vec<&str> = store.list().iter().map(|e| e.recipe.id.as_str()).collect();
        assert_eq!(ids, vec!["c", "a", "b"]);
    }

    #[test]
    fn upsert_preserves_local_root_override() {
        let path = temp_path("preserve-override");
        let mut store = LibraryStore::load(&path).unwrap();
        store.upsert(sample_recipe("sample-app"));
        assert!(store.set_local_root_override("sample-app", Some(PathBuf::from("D:/Games/SampleApp"))));

        // 레시피 갱신(예: 재임포트) — override 는 그대로 남아야 함.
        let mut updated = sample_recipe("sample-app");
        updated.name = "SampleApp (갱신)".to_string();
        store.upsert(updated);

        assert_eq!(
            store.get("sample-app").unwrap().local_root_override,
            Some(PathBuf::from("D:/Games/SampleApp"))
        );
    }

    #[test]
    fn selected_optional_groups_defaults_to_none() {
        let path = temp_path("groups-default");
        let mut store = LibraryStore::load(&path).unwrap();
        store.upsert(sample_recipe("sample-app"));
        assert_eq!(store.get("sample-app").unwrap().selected_optional_groups, None);
    }

    #[test]
    fn set_selected_optional_groups_updates_and_upsert_preserves() {
        let path = temp_path("groups-set");
        let mut store = LibraryStore::load(&path).unwrap();
        store.upsert(sample_recipe("sample-app"));
        let mut groups = std::collections::HashSet::new();
        groups.insert("esong".to_string());
        assert!(store.set_selected_optional_groups("sample-app", Some(groups.clone())));
        assert_eq!(store.get("sample-app").unwrap().selected_optional_groups, Some(groups.clone()));

        // 레시피 갱신(재임포트 등) — 선택은 그대로 남아야 함(local_root_override와 동일 정책).
        let mut updated = sample_recipe("sample-app");
        updated.name = "SampleApp (갱신)".to_string();
        store.upsert(updated);
        assert_eq!(store.get("sample-app").unwrap().selected_optional_groups, Some(groups));
    }

    #[test]
    fn set_selected_optional_groups_none_resets_to_unconfirmed() {
        let path = temp_path("groups-reset");
        let mut store = LibraryStore::load(&path).unwrap();
        store.upsert(sample_recipe("sample-app"));
        let mut groups = std::collections::HashSet::new();
        groups.insert("esong".to_string());
        store.set_selected_optional_groups("sample-app", Some(groups));
        store.set_selected_optional_groups("sample-app", None);
        assert_eq!(store.get("sample-app").unwrap().selected_optional_groups, None);
    }

    #[test]
    fn remove_removes() {
        let path = temp_path("remove");
        let mut store = LibraryStore::load(&path).unwrap();
        store.upsert(sample_recipe("sample-service"));
        assert!(store.remove("sample-service"));
        assert!(!store.contains("sample-service"));
        assert!(!store.remove("sample-service"));
    }

    #[test]
    fn rejects_unsupported_version() {
        let path = temp_path("unsupported-version");
        let bad = serde_json::to_vec_pretty(&serde_json::json!({
            "version": 999,
            "entries": []
        }))
        .unwrap();
        fs::write(&path, bad).unwrap();
        match LibraryStore::load(&path) {
            Err(LibraryError::UnsupportedVersion(999)) => {}
            other => panic!("expected UnsupportedVersion, got {other:?}"),
        }
        let _ = fs::remove_file(&path);
    }

    #[test]
    fn atomic_save_does_not_leave_tmp_file() {
        let path = temp_path("atomic");
        let mut store = LibraryStore::load(&path).unwrap();
        store.upsert(sample_recipe("x"));
        store.save().unwrap();

        let tmp = path.with_extension("json.tmp");
        assert!(!tmp.exists());
        let _ = fs::remove_file(&path);
    }
}
