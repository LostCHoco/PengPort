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
use crate::library::{is_valid_sha256_hex, ArchiveExtraction, ArtifactVerification, OverrideContent, RecipeFile};

/// 리터럴 override(`OverrideContent::Literal`)가 겨냥할 수 없는 확장자 — 실행
/// 가능하거나(exe/dll/com/scr/msi/bat/cmd/ps1/vbs/js/jar/sh/app) 실행에 직접 관여하는
/// 파일이면, 이미 해시로 검증된 아카이브 콘텐츠를 검증 없는 리터럴로 조용히 갈아치울
/// 수 있다(entry_point 여부와 무관 — 같은 폴더의 비-entry_point 실행 파일도
/// DLL-hijack류 위험이 동일함).
const EXECUTABLE_ROLE_EXTENSIONS: &[&str] =
    &["exe", "dll", "com", "scr", "msi", "bat", "cmd", "ps1", "vbs", "js", "jar", "sh", "app"];

fn is_executable_role_path(path: &str) -> bool {
    std::path::Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| EXECUTABLE_ROLE_EXTENSIONS.iter().any(|blocked| ext.eq_ignore_ascii_case(blocked)))
}

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
    for path_override in &archive.path_overrides {
        validate_relative_path(&path_override.from)
            .map_err(|e| ActionError::InvalidConfig(format!("archive: path_overrides.from 오류: {e}")))?;
        validate_relative_path(&path_override.to)
            .map_err(|e| ActionError::InvalidConfig(format!("archive: path_overrides.to 오류: {e}")))?;
    }
    Ok(())
}

pub fn validate_recipe_file(file: &RecipeFile) -> Result<(), ActionError> {
    validate_relative_path(&file.path)
        .map_err(|e| ActionError::InvalidConfig(format!("file: path 오류: {e}")))?;
    if matches!(file.override_content, Some(OverrideContent::Literal { .. })) && is_executable_role_path(&file.path)
    {
        return Err(ActionError::InvalidConfig(format!(
            "file: '{}' 는 실행 가능한 확장자라 리터럴 override 대상이 될 수 없음 \
             (검증되지 않은 콘텐츠로 실행 파일을 갈아치우는 걸 방지)",
            file.path
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::{ArtifactVerification, FileContent, OverrideContent};

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
    fn validates_archive_with_path_override() {
        let mut a = archive("https://cdn.example.com/pack.zip", "");
        a.path_overrides = vec![crate::library::PathOverride {
            from: "raw_source.txt".to_string(),
            to: "moved/raw_source.txt".to_string(),
        }];
        assert!(validate_archive(&a, &ctx()).is_ok());
    }

    #[test]
    fn rejects_archive_path_override_traversal_to() {
        let mut a = archive("https://cdn.example.com/pack.zip", "");
        a.path_overrides = vec![crate::library::PathOverride {
            from: "raw_source.txt".to_string(),
            to: "../../../../Users/Public/Startup/evil.exe".to_string(),
        }];
        let err = validate_archive(&a, &ctx()).unwrap_err();
        assert!(matches!(err, ActionError::InvalidConfig(_)));
    }

    #[test]
    fn rejects_archive_path_override_absolute_to() {
        let mut a = archive("https://cdn.example.com/pack.zip", "");
        a.path_overrides = vec![crate::library::PathOverride {
            from: "raw_source.txt".to_string(),
            to: "C:/Windows/System32/evil.dll".to_string(),
        }];
        let err = validate_archive(&a, &ctx()).unwrap_err();
        assert!(matches!(err, ActionError::InvalidConfig(_)));
    }

    #[test]
    fn rejects_archive_path_override_traversal_from() {
        let mut a = archive("https://cdn.example.com/pack.zip", "");
        a.path_overrides = vec![crate::library::PathOverride {
            from: "../outside.txt".to_string(),
            to: "moved/outside.txt".to_string(),
        }];
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
    fn validates_file_with_literal_override() {
        let f = RecipeFile {
            path: "SampleApp/option.ini".to_string(),
            override_content: Some(OverrideContent::Literal {
                content: FileContent::Text { content: "[GRAPHICS]\n3D_Mode=0\n".to_string() },
            }),
            optional_group: None,
        };
        assert!(validate_recipe_file(&f).is_ok());
    }

    #[test]
    fn rejects_literal_override_on_executable_path() {
        let f = RecipeFile {
            path: "SampleApp/Other.exe".to_string(),
            override_content: Some(OverrideContent::Literal {
                content: FileContent::Text { content: "x".to_string() },
            }),
            optional_group: None,
        };
        let err = validate_recipe_file(&f).unwrap_err();
        assert!(matches!(err, ActionError::InvalidConfig(_)));
    }

    #[test]
    fn rejects_literal_override_on_script_path_case_insensitive() {
        let f = RecipeFile {
            path: "SampleApp/setup.BAT".to_string(),
            override_content: Some(OverrideContent::Literal {
                content: FileContent::Text { content: "start evil.exe".to_string() },
            }),
            optional_group: None,
        };
        let err = validate_recipe_file(&f).unwrap_err();
        assert!(matches!(err, ActionError::InvalidConfig(_)));
    }

    #[test]
    fn allows_executable_path_without_override() {
        // 압축이 exe를 그대로 풀어두는 것(override_content 없음)은 검증 대상이 아님 —
        // 그건 ArtifactVerification이 이미 지키는 영역.
        let f = RecipeFile {
            path: "SampleApp/Launcher.exe".to_string(),
            override_content: None,
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
