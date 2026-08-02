//! `.pengz` 파일 임포트 워크플로 — [`super::bundle`](디코딩) + [`super::store`](영속화) 를 묶는다.
//!
//! 트러스트 모델의 "임포트 시 1회 confirm" 이 이 모듈의 두 단계로 나뉜다:
//! 1. [`preview_file`] — 번들에 뭐가 들어있는지 보여주기만 함(스토어 변경 없음).
//! 2. [`commit_file`] — 사용자가 확인한 뒤 실제로 스토어에 반영.
//!
//! Tauri 커맨드가 이 두 함수를 그대로 노출하고, 그 사이에 실제 confirm UI가 낀다.
//! **1회성 confirm** — 항목 개수와 무관하게 번들 전체에 대해 딱 한 번만 물어본다
//! (항목별 반복 confirm 없음), [[app_library_essence]] 참고.

use serde::Serialize;

use super::bundle::{decode_bundle_file, BundleContents, BundleError};
use super::store::LibraryStore;
use super::third_party_store::ThirdPartyAppStore;

#[derive(Debug, Clone, Serialize)]
pub struct ImportPreviewItem {
    pub id: String,
    pub name: String,
    pub icon_url: Option<String>,
    /// 이미 라이브러리에 같은 id 가 있으면 true — confirm UI 가 "덮어씀" 표시용.
    pub already_in_library: bool,
}

/// [`ImportPreviewItem`]과 같은 모양이지만 third-party app 은 `icon_url`이 없다 —
/// descriptor 자체에 그 필드가 없다(third-party app 은 레시피가 아니라 실행 위치 데이터).
#[derive(Debug, Clone, Serialize)]
pub struct ImportPreviewThirdPartyApp {
    pub id: String,
    pub label: String,
    /// 이미 로컬 third-party app store 에 같은 id 가 있으면 true.
    pub already_registered: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ImportPreview {
    pub items: Vec<ImportPreviewItem>,
    #[serde(default)]
    pub third_party_apps: Vec<ImportPreviewThirdPartyApp>,
}

/// `.pengz` 파일 번들 내용을 미리보기 — 스토어를 변경하지 않는다.
pub fn preview_file(
    bytes: &[u8],
    store: &LibraryStore,
    third_party_store: &ThirdPartyAppStore,
) -> Result<ImportPreview, BundleError> {
    let contents = decode_bundle_file(bytes)?;
    Ok(contents_to_preview(contents, store, third_party_store))
}

fn contents_to_preview(
    contents: BundleContents,
    store: &LibraryStore,
    third_party_store: &ThirdPartyAppStore,
) -> ImportPreview {
    let items = contents
        .recipes
        .into_iter()
        .map(|r| ImportPreviewItem {
            already_in_library: store.contains(&r.id),
            id: r.id,
            name: r.name,
            icon_url: r.recipe_info.icon_url,
        })
        .collect();
    let third_party_apps = contents
        .third_party_apps
        .into_iter()
        .map(|d| ImportPreviewThirdPartyApp {
            already_registered: third_party_store.contains(&d.id),
            label: d.label.clone().unwrap_or_else(|| d.id.clone()),
            id: d.id,
        })
        .collect();
    ImportPreview {
        items,
        third_party_apps,
    }
}

/// `.pengz` 파일 번들을 실제로 두 스토어에 반영. 호출자가 `store.save()`/
/// `third_party_store.save()`는 별도로 호출해야 한다(Tauri 커맨드 레이어가
/// load→commit→save 를 한 트랜잭션처럼 묶는다).
///
/// 반환값 = 임포트된 레시피 id 목록(순서 보존). third-party app 은 확인용 정보가
/// 아니라 레시피를 돌리기 위한 부수 데이터라 별도 반환하지 않는다(호출자가 필요하면
/// `third_party_store.list()`로 조회).
pub fn commit_file(
    bytes: &[u8],
    store: &mut LibraryStore,
    third_party_store: &mut ThirdPartyAppStore,
) -> Result<Vec<String>, BundleError> {
    let contents = decode_bundle_file(bytes)?;
    let ids: Vec<String> = contents.recipes.iter().map(|r| r.id.clone()).collect();
    for recipe in contents.recipes {
        store.upsert(recipe);
    }
    for descriptor in contents.third_party_apps {
        third_party_store.upsert(descriptor);
    }
    Ok(ids)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::ThirdPartyAppDescriptor;
    use crate::library::bundle::encode_bundle_file;
    use crate::library::recipe::{LaunchAction, Recipe, RecipeInfo};
    use std::fs;
    use std::path::PathBuf;

    fn temp_path(name: &str) -> PathBuf {
        let p = std::env::temp_dir()
            .join(format!("pengport-import-{name}-{}.json", std::process::id()));
        let _ = fs::remove_file(&p);
        p
    }

    fn sample_recipe(id: &str) -> Recipe {
        Recipe {
            id: id.to_string(),
            name: format!("App {id}"),
            recipe_info: RecipeInfo {
                icon_url: Some("https://example.com/icon.png".to_string()),
                background_url: None,
            },
            archives: vec![],
            files: vec![],
            optional_groups: vec![],
            folder_rules: vec![],
            launch: LaunchAction::SpawnProcess {
                entry_point: "x.exe".to_string(),
                entry_args: vec![],
            },
        }
    }

    fn sample_descriptor(id: &str) -> ThirdPartyAppDescriptor {
        ThirdPartyAppDescriptor {
            id: id.to_string(),
            label: Some(format!("App {id}")),
            exe_filename: "test_app.exe".to_string(),
            download_strategy: None,
            instances_subfolder: None,
            system_appdata_folder_name: None,
            readiness_signal: None,
            launch_args_template: vec![],
            post_download_marker_files: vec![],
        }
    }

    fn empty_third_party_store(name: &str) -> ThirdPartyAppStore {
        ThirdPartyAppStore::load(temp_path(name)).unwrap()
    }

    #[test]
    fn preview_file_shows_new_items_as_not_in_library() {
        let store = LibraryStore::load(temp_path("preview-file-new")).unwrap();
        let tp_store = empty_third_party_store("preview-file-new-tp");
        let bytes = encode_bundle_file(&[sample_recipe("sample-service")], &[]).unwrap();
        let preview = preview_file(&bytes, &store, &tp_store).unwrap();
        assert_eq!(preview.items.len(), 1);
        assert!(!preview.items[0].already_in_library);
    }

    #[test]
    fn preview_file_flags_existing_items() {
        let mut store = LibraryStore::load(temp_path("preview-file-existing")).unwrap();
        store.upsert(sample_recipe("sample-service"));
        let tp_store = empty_third_party_store("preview-file-existing-tp");
        let bytes = encode_bundle_file(&[sample_recipe("sample-service")], &[]).unwrap();
        let preview = preview_file(&bytes, &store, &tp_store).unwrap();
        assert!(preview.items[0].already_in_library);
    }

    #[test]
    fn preview_file_shows_third_party_apps_and_flags_existing() {
        let store = LibraryStore::load(temp_path("preview-file-tp-new")).unwrap();
        let mut tp_store = empty_third_party_store("preview-file-tp-new-tp");
        tp_store.upsert(sample_descriptor("already_here"));
        let bytes = encode_bundle_file(
            &[],
            &[sample_descriptor("test_app"), sample_descriptor("already_here")],
        )
        .unwrap();
        let preview = preview_file(&bytes, &store, &tp_store).unwrap();
        assert_eq!(preview.third_party_apps.len(), 2);
        assert!(!preview.third_party_apps[0].already_registered);
        assert!(preview.third_party_apps[1].already_registered);
    }

    #[test]
    fn preview_file_does_not_mutate_store() {
        let store = LibraryStore::load(temp_path("preview-file-immutable")).unwrap();
        let tp_store = empty_third_party_store("preview-file-immutable-tp");
        let bytes = encode_bundle_file(&[sample_recipe("x")], &[]).unwrap();
        preview_file(&bytes, &store, &tp_store).unwrap();
        assert!(store.list().is_empty());
    }

    #[test]
    fn commit_file_adds_all_recipes_in_bundle() {
        let mut store = LibraryStore::load(temp_path("commit-file-multi")).unwrap();
        let mut tp_store = empty_third_party_store("commit-file-multi-tp");
        let bytes =
            encode_bundle_file(&[sample_recipe("sample-service"), sample_recipe("sample-service-2")], &[]).unwrap();
        let imported = commit_file(&bytes, &mut store, &mut tp_store).unwrap();
        assert_eq!(imported, vec!["sample-service".to_string(), "sample-service-2".to_string()]);
        assert_eq!(store.list().len(), 2);
    }

    #[test]
    fn commit_file_adds_third_party_apps_in_bundle() {
        let mut store = LibraryStore::load(temp_path("commit-file-tp")).unwrap();
        let mut tp_store = empty_third_party_store("commit-file-tp-tp");
        let bytes = encode_bundle_file(&[sample_recipe("sample-service")], &[sample_descriptor("test_app")]).unwrap();
        commit_file(&bytes, &mut store, &mut tp_store).unwrap();
        assert!(tp_store.contains("test_app"));
    }

    #[test]
    fn commit_file_invalid_bundle_leaves_store_untouched() {
        let mut store = LibraryStore::load(temp_path("commit-file-invalid")).unwrap();
        let mut tp_store = empty_third_party_store("commit-file-invalid-tp");
        store.upsert(sample_recipe("existing"));
        let err = commit_file(b"not gzip data", &mut store, &mut tp_store).unwrap_err();
        assert!(matches!(err, BundleError::Gzip(_)));
        assert_eq!(store.list().len(), 1);
    }
}
