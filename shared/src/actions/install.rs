//! [`ArchiveExtraction`]/[`RecipeFile`] 검증 — 경로 안전성(zip-slip) + URL 안전성.
//!
//! 대상 루트(App 전용 폴더냐 third-party 앱 데이터 영역이냐)는 더 이상 여기서 검증할
//! 게 없다 — `Recipe.launch` 하나가 결정하므로 `launch::validate_launch_action`이 그
//! app_id 를 이미 검증한다(`recipe.rs` 모듈 설명 참고).
//!
//! 실제 다운로드·화이트리스트 정리·파일 쓰기는 src-tauri 가 담당(shared 는 Tauri
//! 무관여) — 이 모듈은 검증만 한다.

use super::relative_path::validate_relative_path;
use super::url_check::is_url_safe;
use super::{ActionContext, ActionError};
use crate::library::{is_valid_sha256_hex, ArchiveExtraction, ArtifactVerification, RecipeFile};

pub fn validate_archive(archive: &ArchiveExtraction, ctx: &ActionContext) -> Result<(), ActionError> {
    is_url_safe(&archive.url, ctx.allow_http)?;
    validate_relative_path(&archive.extract_to)
        .map_err(|e| ActionError::InvalidConfig(format!("archive: extract_to 오류: {e}")))?;
    match &archive.verification {
        ArtifactVerification::Sha256 { hash } if !is_valid_sha256_hex(hash) => {
            return Err(ActionError::InvalidConfig(
                "archive: verification 해시가 유효한 SHA256(64자리 hex)이 아님".to_string(),
            ));
        }
        ArtifactVerification::Sha256 { .. } => {}
    }
    if let Some(raw_filename) = &archive.raw_filename {
        if raw_filename.is_empty() {
            return Err(ActionError::InvalidConfig(
                "archive: raw_filename 은 비어있을 수 없음".to_string(),
            ));
        }
        validate_relative_path(raw_filename)
            .map_err(|e| ActionError::InvalidConfig(format!("archive: raw_filename 오류: {e}")))?;
    }
    Ok(())
}

pub fn validate_recipe_file(file: &RecipeFile) -> Result<(), ActionError> {
    validate_relative_path(&file.path)
        .map_err(|e| ActionError::InvalidConfig(format!("file: path 오류: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{ArtifactVerification, ConfigFileFormat, FileContent, OverrideContent};

    fn ctx() -> ActionContext {
        ActionContext { allow_http: false }
    }

    fn sha256_verification() -> ArtifactVerification {
        ArtifactVerification::Sha256 {
            hash: "0".repeat(64),
        }
    }

    fn archive(url: &str, extract_to: &str) -> ArchiveExtraction {
        ArchiveExtraction {
            url: url.to_string(),
            label: None,
            verification: sha256_verification(),
            order: 0,
            extract_to: extract_to.to_string(),
            optional_group: None,
            raw_filename: None,
            path_overrides: Vec::new(),
        }
    }

    #[test]
    fn validates_archive() {
        let a = archive("https://cdn.example.com/app.7z", "");
        assert!(validate_archive(&a, &ctx()).is_ok());
    }

    #[test]
    fn validates_archive_nested_extract_to() {
        let a = archive("https://cdn.example.com/pack.tar.gz", ".gamedata/.pack-src");
        assert!(validate_archive(&a, &ctx()).is_ok());
    }

    #[test]
    fn rejects_archive_unsafe_url() {
        let a = archive("https://192.168.1.1/app.7z", "");
        let err = validate_archive(&a, &ctx()).unwrap_err();
        assert!(matches!(err, ActionError::UrlCheck(_)));
    }

    #[test]
    fn rejects_archive_traversal_extract_to() {
        let a = archive("https://cdn.example.com/app.7z", "../../evil");
        let err = validate_archive(&a, &ctx()).unwrap_err();
        assert!(matches!(err, ActionError::InvalidConfig(_)));
    }

    #[test]
    fn validates_archive_with_raw_filename() {
        let mut a = archive("https://cdn.example.com/tool.jar", ".gamedata");
        a.raw_filename = Some("tool.jar".to_string());
        assert!(validate_archive(&a, &ctx()).is_ok());
    }

    #[test]
    fn rejects_archive_empty_raw_filename() {
        let mut a = archive("https://cdn.example.com/tool.jar", ".gamedata");
        a.raw_filename = Some("".to_string());
        let err = validate_archive(&a, &ctx()).unwrap_err();
        assert!(matches!(err, ActionError::InvalidConfig(_)));
    }

    #[test]
    fn rejects_archive_traversal_raw_filename() {
        let mut a = archive("https://cdn.example.com/tool.jar", ".gamedata");
        a.raw_filename = Some("../../evil.jar".to_string());
        let err = validate_archive(&a, &ctx()).unwrap_err();
        assert!(matches!(err, ActionError::InvalidConfig(_)));
    }

    #[test]
    fn validates_file_whitelist_entry_without_override() {
        let f = RecipeFile {
            path: "SampleApp/Launcher.exe".to_string(),
            override_content: None,
            optional_group: None,
        };
        assert!(validate_recipe_file(&f).is_ok());
    }

    #[test]
    fn validates_file_with_config_patch_override() {
        let f = RecipeFile {
            path: "SampleApp/option.ini".to_string(),
            override_content: Some(OverrideContent::ConfigPatch {
                format: ConfigFileFormat::Ini,
                patch: serde_json::json!({}),
            }),
            optional_group: None,
        };
        assert!(validate_recipe_file(&f).is_ok());
    }

    #[test]
    fn rejects_file_traversal_path() {
        let f = RecipeFile {
            path: "../outside.txt".to_string(),
            override_content: Some(OverrideContent::Literal {
                content: FileContent::Text {
                    content: "x".to_string(),
                },
            }),
            optional_group: None,
        };
        let err = validate_recipe_file(&f).unwrap_err();
        assert!(matches!(err, ActionError::InvalidConfig(_)));
    }
}
