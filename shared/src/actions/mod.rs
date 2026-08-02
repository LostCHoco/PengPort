//! [`crate::library::Recipe`] 검증 — `archives`+`files`(합쳐서 최소 1개, 각각 순서대로)
//! + `launch`(정확히 1개).
//!
//! ## 책임 분담
//!
//! `shared/actions/` 는 검증까지만. 실제 다운로드, 압축 해제, 화이트리스트 정리, 파일
//! 쓰기, 프로세스 spawn, third-party app 실행은 src-tauri 측이 처리한다 — shared 는
//! Tauri 무관여.
//!
//! ## 흐름
//!
//! ```text
//! Recipe (archives: Vec<ArchiveExtraction>, files: Vec<RecipeFile>, launch: LaunchAction)
//!     │
//!     ▼ actions::validate_recipe(recipe, ctx)
//! Ok(()) ←─ 통과하면 recipe 자체를 그대로 실행 계층에 넘김(별도 Intent 타입 없음)
//!     │
//!     ▼ src-tauri 가 순서대로 OS 호출
//! 실제 동작
//! ```

use thiserror::Error;

use crate::library::Recipe;

pub mod install;
pub mod launch;
pub mod relative_path;
pub mod third_party_app;
pub mod url_check;

pub use install::{validate_archive, validate_recipe_file};
pub use launch::validate_launch_action;
pub use relative_path::validate_relative_path;
pub use third_party_app::{
    build_launch_args, detect as detect_third_party_app, instance_dir as third_party_instance_dir,
    resolve_third_party_app, DataRootLookupContext, DownloadStrategy, ReadinessSignal,
    ResolvedThirdPartyApp, ThirdPartyAppDescriptor, ThirdPartyAppSource,
};

/// 검증 시점의 컨텍스트. `allow_http` 는 dev 모드에서 HTTP 허용 (production: false).
#[derive(Debug, Clone, Copy)]
pub struct ActionContext {
    pub allow_http: bool,
}

#[derive(Debug, Error)]
pub enum ActionError {
    #[error("레시피에 archives/files 가 하나도 없음 — 최소 1개 필요")]
    EmptyInstall,

    #[error("URL 검증 실패: {0}")]
    UrlCheck(#[from] url_check::UrlError),

    #[error("설정 값 오류: {0}")]
    InvalidConfig(String),
}

/// Recipe 전체를 검증 — `archives`+`files` 합쳐서 최소 1개(런타임 불변식) + 각 항목
/// + `optional_groups` 참조 정합성 + `launch`.
pub fn validate_recipe(recipe: &Recipe, ctx: &ActionContext) -> Result<(), ActionError> {
    if recipe.archives.is_empty() && recipe.files.is_empty() {
        return Err(ActionError::EmptyInstall);
    }
    for archive in &recipe.archives {
        install::validate_archive(archive, ctx)?;
    }
    for file in &recipe.files {
        install::validate_recipe_file(file)?;
    }
    validate_optional_groups(recipe)?;
    validate_archive_order(recipe)?;
    validate_folder_rules(recipe)?;
    launch::validate_launch_action(&recipe.launch)?;
    Ok(())
}

/// `FolderRule::path` 는 `Recipe.folder_rules` 안에서 유일해야 한다(같은 폴더에 정책이
/// 두 개면 어느 쪽을 적용할지 모호해짐) + `Filtered` 모드의 `patterns`/`disallow_patterns`
/// 는 전부 유효한 glob 문법이어야 한다(레시피 편집 시점에 오타를 바로 잡기 위함 —
/// 잘못된 패턴은 pruning 시점까지 미루지 않고 여기서 거부).
fn validate_folder_rules(recipe: &Recipe) -> Result<(), ActionError> {
    let mut seen = std::collections::HashSet::new();
    for rule in &recipe.folder_rules {
        if !seen.insert(rule.path.as_str()) {
            return Err(ActionError::InvalidConfig(format!(
                "folder_rules: path '{}' 중복 선언됨",
                rule.path
            )));
        }
        if let crate::library::FolderRuleMode::Filtered { patterns, disallow_patterns } = &rule.mode {
            for pattern in patterns.iter().chain(disallow_patterns.iter()) {
                if glob::Pattern::new(pattern).is_err() {
                    return Err(ActionError::InvalidConfig(format!(
                        "folder_rules: '{}' 의 패턴 '{pattern}' 이 유효한 glob 문법이 아님",
                        rule.path
                    )));
                }
            }
        }
    }
    Ok(())
}

/// `ArchiveExtraction::order` 는 `Recipe.archives` 전체에서 유일해야 한다 — 두 압축이
/// 같은 목적지에 겹치는 파일을 만들 때 "이 값이 더 큰 쪽이 이긴다"는 규칙이 값이
/// 겹치면 모호해지므로. 배열 순서가 아니라 이 필드가 실행 순서의 유일한 근거라는
/// 불변식을 스키마 단계에서 강제한다.
fn validate_archive_order(recipe: &Recipe) -> Result<(), ActionError> {
    let mut seen = std::collections::HashSet::new();
    for archive in &recipe.archives {
        if !seen.insert(archive.order) {
            return Err(ActionError::InvalidConfig(format!(
                "archives: order 값 '{}' 이 중복됨(archive '{}') — 모든 압축의 order 는 서로 달라야 함",
                archive.order, archive.url
            )));
        }
    }
    Ok(())
}

/// `optional_groups`의 id 가 중복 없어야 하고, `RecipeFile::optional_group`/
/// `ArchiveExtraction::optional_group`이 참조하는 id 는 전부 `optional_groups`에
/// 선언돼 있어야 함(오타로 인한 "존재 안 하는 그룹" 방지).
fn validate_optional_groups(recipe: &Recipe) -> Result<(), ActionError> {
    let mut seen = std::collections::HashSet::new();
    for group in &recipe.optional_groups {
        if !seen.insert(group.id.as_str()) {
            return Err(ActionError::InvalidConfig(format!(
                "optional_groups: id '{}' 중복 선언됨",
                group.id
            )));
        }
    }
    for archive in &recipe.archives {
        if let Some(group_id) = &archive.optional_group {
            if !seen.contains(group_id.as_str()) {
                return Err(ActionError::InvalidConfig(format!(
                    "archive '{}': optional_group '{group_id}' 이 optional_groups 에 선언 안 됨",
                    archive.url
                )));
            }
        }
    }
    for file in &recipe.files {
        if let Some(group_id) = &file.optional_group {
            if !seen.contains(group_id.as_str()) {
                return Err(ActionError::InvalidConfig(format!(
                    "file '{}': optional_group '{group_id}' 이 optional_groups 에 선언 안 됨",
                    file.path
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::library::{
        ArchiveExtraction, ArtifactVerification, FolderRule, FolderRuleMode, LaunchAction,
        RecipeInfo,
    };

    fn ctx() -> ActionContext {
        ActionContext { allow_http: false }
    }

    fn sample_archive(url: &str) -> ArchiveExtraction {
        sample_archive_with_order(url, 0)
    }

    fn sample_archive_with_order(url: &str, order: u32) -> ArchiveExtraction {
        ArchiveExtraction {
            url: url.to_string(),
            label: None,
            verification: ArtifactVerification::Sha256 {
                hash: "0".repeat(64),
            },
            order,
            extract_to: "".to_string(),
            optional_group: None,
            raw_filename: None,
            path_overrides: Vec::new(),
        }
    }

    fn sample_recipe(archives: Vec<ArchiveExtraction>, launch: LaunchAction) -> Recipe {
        Recipe {
            id: "sample".to_string(),
            name: "Sample".to_string(),
            recipe_info: RecipeInfo::default(),
            archives,
            files: vec![],
            optional_groups: vec![],
            folder_rules: vec![],
            launch,
        }
    }

    #[test]
    fn validate_recipe_ok() {
        let recipe = sample_recipe(
            vec![sample_archive("https://cdn.example.com/app.7z")],
            LaunchAction::SpawnProcess {
                entry_point: "app.exe".to_string(),
                entry_args: vec![],
            },
        );
        assert!(validate_recipe(&recipe, &ctx()).is_ok());
    }

    #[test]
    fn validate_recipe_rejects_empty_archives_and_files() {
        let recipe = sample_recipe(
            vec![],
            LaunchAction::SpawnProcess {
                entry_point: "app.exe".to_string(),
                entry_args: vec![],
            },
        );
        let err = validate_recipe(&recipe, &ctx()).unwrap_err();
        assert!(matches!(err, ActionError::EmptyInstall));
    }

    #[test]
    fn validate_recipe_propagates_archive_error() {
        let recipe = sample_recipe(
            vec![sample_archive("https://192.168.1.1/app.7z")],
            LaunchAction::SpawnProcess {
                entry_point: "app.exe".to_string(),
                entry_args: vec![],
            },
        );
        let err = validate_recipe(&recipe, &ctx()).unwrap_err();
        assert!(matches!(err, ActionError::UrlCheck(_)));
    }

    #[test]
    fn validate_recipe_propagates_launch_error() {
        let recipe = sample_recipe(
            vec![sample_archive("https://cdn.example.com/app.7z")],
            LaunchAction::SpawnProcess {
                entry_point: "".to_string(),
                entry_args: vec![],
            },
        );
        let err = validate_recipe(&recipe, &ctx()).unwrap_err();
        assert!(matches!(err, ActionError::InvalidConfig(_)));
    }

    #[test]
    fn validate_recipe_ok_with_declared_optional_group_reference() {
        let mut recipe = sample_recipe(
            vec![sample_archive("https://cdn.example.com/app.7z")],
            LaunchAction::SpawnProcess {
                entry_point: "app.exe".to_string(),
                entry_args: vec![],
            },
        );
        recipe.optional_groups.push(crate::library::OptionalGroup {
            id: "esong".to_string(),
            label: "테스트 그룹".to_string(),
            default_selected: true,
        });
        recipe.files.push(crate::library::RecipeFile {
            path: "ESong/song.bin".to_string(),
            override_content: None,
            optional_group: Some("esong".to_string()),
        });
        assert!(validate_recipe(&recipe, &ctx()).is_ok());
    }

    #[test]
    fn validate_recipe_rejects_undeclared_optional_group_reference() {
        let mut recipe = sample_recipe(
            vec![sample_archive("https://cdn.example.com/app.7z")],
            LaunchAction::SpawnProcess {
                entry_point: "app.exe".to_string(),
                entry_args: vec![],
            },
        );
        recipe.files.push(crate::library::RecipeFile {
            path: "ESong/song.bin".to_string(),
            override_content: None,
            optional_group: Some("esong".to_string()),
        });
        let err = validate_recipe(&recipe, &ctx()).unwrap_err();
        assert!(matches!(err, ActionError::InvalidConfig(_)));
    }

    #[test]
    fn validate_recipe_rejects_duplicate_optional_group_id() {
        let mut recipe = sample_recipe(
            vec![sample_archive("https://cdn.example.com/app.7z")],
            LaunchAction::SpawnProcess {
                entry_point: "app.exe".to_string(),
                entry_args: vec![],
            },
        );
        for _ in 0..2 {
            recipe.optional_groups.push(crate::library::OptionalGroup {
                id: "esong".to_string(),
                label: "테스트 그룹".to_string(),
                default_selected: false,
            });
        }
        let err = validate_recipe(&recipe, &ctx()).unwrap_err();
        assert!(matches!(err, ActionError::InvalidConfig(_)));
    }

    #[test]
    fn validate_recipe_rejects_duplicate_archive_order() {
        let recipe = sample_recipe(
            vec![
                sample_archive_with_order("https://cdn.example.com/a.7z", 1),
                sample_archive_with_order("https://cdn.example.com/b.7z", 1),
            ],
            LaunchAction::SpawnProcess {
                entry_point: "app.exe".to_string(),
                entry_args: vec![],
            },
        );
        let err = validate_recipe(&recipe, &ctx()).unwrap_err();
        assert!(matches!(err, ActionError::InvalidConfig(_)));
    }

    #[test]
    fn validate_recipe_ok_with_distinct_archive_order() {
        let recipe = sample_recipe(
            vec![
                sample_archive_with_order("https://cdn.example.com/a.7z", 1),
                sample_archive_with_order("https://cdn.example.com/b.7z", 2),
            ],
            LaunchAction::SpawnProcess {
                entry_point: "app.exe".to_string(),
                entry_args: vec![],
            },
        );
        assert!(validate_recipe(&recipe, &ctx()).is_ok());
    }

    fn sample_launch() -> LaunchAction {
        LaunchAction::SpawnProcess {
            entry_point: "app.exe".to_string(),
            entry_args: vec![],
        }
    }

    #[test]
    fn validate_recipe_ok_with_valid_folder_rule() {
        let mut recipe = sample_recipe(
            vec![sample_archive("https://cdn.example.com/app.7z")],
            sample_launch(),
        );
        recipe.folder_rules.push(FolderRule {
            path: "SampleApp/saves".to_string(),
            mode: FolderRuleMode::Filtered {
                patterns: BTreeSet::from(["*.sav".to_string()]),
                disallow_patterns: BTreeSet::new(),
            },
        });
        assert!(validate_recipe(&recipe, &ctx()).is_ok());
    }

    #[test]
    fn validate_recipe_rejects_duplicate_folder_rule_path() {
        let mut recipe = sample_recipe(
            vec![sample_archive("https://cdn.example.com/app.7z")],
            sample_launch(),
        );
        for _ in 0..2 {
            recipe.folder_rules.push(FolderRule {
                path: "SampleApp/saves".to_string(),
                mode: FolderRuleMode::Passthrough,
            });
        }
        let err = validate_recipe(&recipe, &ctx()).unwrap_err();
        assert!(matches!(err, ActionError::InvalidConfig(_)));
    }

    #[test]
    fn validate_recipe_rejects_invalid_glob_pattern() {
        let mut recipe = sample_recipe(
            vec![sample_archive("https://cdn.example.com/app.7z")],
            sample_launch(),
        );
        recipe.folder_rules.push(FolderRule {
            path: "SampleApp/saves".to_string(),
            mode: FolderRuleMode::Filtered {
                patterns: BTreeSet::from(["[".to_string()]),
                disallow_patterns: BTreeSet::new(),
            },
        });
        let err = validate_recipe(&recipe, &ctx()).unwrap_err();
        assert!(matches!(err, ActionError::InvalidConfig(_)));
    }

    #[test]
    fn validate_recipe_rejects_invalid_disallow_glob_pattern() {
        let mut recipe = sample_recipe(
            vec![sample_archive("https://cdn.example.com/app.7z")],
            sample_launch(),
        );
        recipe.folder_rules.push(FolderRule {
            path: "SampleApp/saves".to_string(),
            mode: FolderRuleMode::Filtered {
                patterns: BTreeSet::new(),
                disallow_patterns: BTreeSet::from(["[".to_string()]),
            },
        });
        let err = validate_recipe(&recipe, &ctx()).unwrap_err();
        assert!(matches!(err, ActionError::InvalidConfig(_)));
    }
}
