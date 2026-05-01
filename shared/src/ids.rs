//! 파일시스템 안전한 식별자 검증.
//!
//! PSP service id, prism instance dir name 등 외부 (운영자 controlled / attacker
//! controlled) 입력이 fs 작업의 path component 로 사용되는 경우 path traversal 차단.
//!
//! ## 정책
//!
//! - 1~64자
//! - `[A-Za-z0-9_-]` 만 허용 (영숫자 + dash + underscore)
//! - 공백, `.`, `/`, `\`, NUL, OS reserved name 등 모두 거부
//!
//! 이 정책은 의도적으로 보수적 — `.` 도 허용 안 함으로써 `..` 자동 차단. 실 사용 사례
//! (펭돌서버: `modded-mc`, `rlcraft-mc`) 는 모두 이 정책 만족.
//!
//! ## 침투 위협 모델
//!
//! 1. 악성 instance 가 catalog 에 `id: "../../Windows/System32"` 같은 service entry 를 둔다
//! 2. 사용자가 그 instance 에 가입 → service 카드 표시 → Play 누름
//! 3. 검증 없으면 `instances/<id>/` 가 instances 폴더 밖 임의 위치를 가리킴 → instance.cfg
//!    / servers.dat 등 임의 위치에 작성
//! 4. 또는 사용자가 그 카드의 [앱 제거] 누름 → `fs::remove_dir_all(<악성_path>)` 호출 →
//!    임의 폴더 통째 삭제
//!
//! 이 모듈의 `validate_service_id` 가 모든 진입점에서 거부하면 위 시나리오 모두 차단.

use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum IdError {
    #[error("id 가 비어있음")]
    Empty,
    #[error("id 길이 초과 (최대 64자, 받음 {0})")]
    TooLong(usize),
    #[error("id 에 허용되지 않은 문자 포함 (영숫자/`-`/`_` 만): {0:?}")]
    InvalidChar(char),
}

const MAX_LEN: usize = 64;

/// PSP service id 등 fs 작업에 쓰일 식별자 검증.
///
/// 통과 = `[A-Za-z0-9_-]{1,64}`.
pub fn validate_service_id(id: &str) -> Result<(), IdError> {
    if id.is_empty() {
        return Err(IdError::Empty);
    }
    if id.len() > MAX_LEN {
        return Err(IdError::TooLong(id.len()));
    }
    for c in id.chars() {
        if !(c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(IdError::InvalidChar(c));
        }
    }
    Ok(())
}

/// `validate_service_id` 의 boolean 버전.
pub fn is_valid_service_id(id: &str) -> bool {
    validate_service_id(id).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn valid_simple_ids_pass() {
        for id in ["modded-mc", "rlcraft-mc", "alpha_peng", "Service123", "a", "X"] {
            assert!(is_valid_service_id(id), "expected valid: {id:?}");
        }
    }

    #[test]
    fn empty_rejected() {
        assert_eq!(validate_service_id(""), Err(IdError::Empty));
    }

    #[test]
    fn too_long_rejected() {
        let long = "a".repeat(65);
        assert!(matches!(
            validate_service_id(&long),
            Err(IdError::TooLong(65))
        ));
    }

    #[test]
    fn boundary_64_passes() {
        let boundary = "a".repeat(64);
        assert!(is_valid_service_id(&boundary));
    }

    #[test]
    fn path_traversal_rejected() {
        for evil in [
            "../etc",
            "..",
            ".",
            "../../Windows",
            "foo/bar",
            "foo\\bar",
            "modded-mc/../evil",
        ] {
            assert!(
                matches!(validate_service_id(evil), Err(IdError::InvalidChar(_))),
                "expected reject: {evil:?}"
            );
        }
    }

    #[test]
    fn windows_reserved_chars_rejected() {
        for evil in ["foo:bar", "foo*bar", "foo?bar", "foo<bar", "foo>bar", "foo|bar"] {
            assert!(
                matches!(validate_service_id(evil), Err(IdError::InvalidChar(_))),
                "expected reject: {evil:?}"
            );
        }
    }

    #[test]
    fn whitespace_rejected() {
        for evil in [" ", "foo bar", "foo\tbar", "foo\nbar"] {
            assert!(
                matches!(validate_service_id(evil), Err(IdError::InvalidChar(_))),
                "expected reject: {evil:?}"
            );
        }
    }

    #[test]
    fn null_byte_rejected() {
        assert!(matches!(
            validate_service_id("foo\0bar"),
            Err(IdError::InvalidChar(_))
        ));
    }

    #[test]
    fn unicode_rejected() {
        // 정책상 ASCII 만 — 유니코드 normalization 우회 차단.
        for evil in ["펭돌", "café", "naïve", "𓀀"] {
            assert!(
                matches!(validate_service_id(evil), Err(IdError::InvalidChar(_))),
                "expected reject: {evil:?}"
            );
        }
    }
}
