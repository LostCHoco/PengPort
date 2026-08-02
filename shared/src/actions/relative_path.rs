//! 상대경로 안전성 검증 — zip-slip 류 차단. `ArchiveExtraction`/`RecipeFile`/`LaunchAction`의
//! 모든 경로 필드(압축 해제 대상, 실행 파일 경로, 설정 파일 경로, third-party app 데이터
//! 파일 경로)가 공통으로 재사용한다.

use std::path::{Component, Path};

/// 앱 루트 폴더를 벗어나는 상대경로(zip-slip 류) 차단. `""`(빈 문자열)은 루트 자체를
/// 뜻해 허용. 절대경로·`..`·드라이브 접두사·빈 컴포넌트는 전부 거부.
pub fn validate_relative_path(rel: &str) -> Result<(), String> {
    if rel.is_empty() {
        return Ok(());
    }
    let path = Path::new(rel);
    if path.is_absolute() {
        return Err(format!("절대경로 금지: {rel}"));
    }
    for component in path.components() {
        match component {
            Component::Normal(_) => {}
            other => return Err(format!("허용되지 않는 경로 구성요소({other:?}): {rel}")),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_root_ok() {
        assert!(validate_relative_path("").is_ok());
    }

    #[test]
    fn normal_relative_path_ok() {
        assert!(validate_relative_path("SampleApp/Launcher.exe").is_ok());
    }

    #[test]
    fn rejects_parent_traversal() {
        assert!(validate_relative_path("../../evil").is_err());
    }

    #[test]
    fn rejects_absolute_path() {
        assert!(validate_relative_path("C:/Windows/System32/cmd.exe").is_err());
        assert!(validate_relative_path("/etc/passwd").is_err());
    }

    #[test]
    fn rejects_embedded_traversal_component() {
        assert!(validate_relative_path("SampleApp/../../../evil").is_err());
    }
}
