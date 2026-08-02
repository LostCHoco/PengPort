//! 아티팩트 검증 — 다운로드된 설치 아티팩트가 [`super::ArtifactVerification`] 선언과
//! 일치하는지 확인. 불일치 시 실행 자체를 차단한다.
//!
//! 옛 `manifest_check.rs`의 "manifest 거짓말 차단"(actions[].kind ⊆ permissions)이
//! 하던 역할을 계승 — 레시피에는 별도 permissions 선언이 없으므로(레시피 필드 자체가
//! 선언이자 실행 사양), 대신 "다운로드물이 레시피가 약속한 것과 실제로 같은가"를 검증한다.
//!
//! 이 모듈은 순수 검증 함수만 제공 — 실제 다운로드(HTTP GET)는 호출자(Tauri 커맨드
//! 등) 책임. `verify_artifact` 는 이미 메모리에 있는 bytes 를 검증한다.

use sha2::{Digest, Sha256};
use thiserror::Error;

use super::recipe::ArtifactVerification;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum VerifyError {
    #[error("SHA256 불일치 (기대: {expected}, 실제: {actual})")]
    Sha256Mismatch { expected: String, actual: String },

    #[error("SHA256 해시 형식 오류 (64자 hex 필요): {0}")]
    InvalidHashFormat(String),
}

/// 다운로드된 바이트가 레시피의 [`ArtifactVerification`] 선언과 일치하는지 검증.
///
/// 통과 못 하면 호출자는 그 아티팩트를 절대 실행/설치해선 안 된다 — 이게
/// "다운로드 페이지 폴백 없이 항상 원클릭"을 지키면서도 변조된 설치물을 막는
/// 유일한 게이트다.
///
/// 이미 메모리에 바이트 전체가 있을 때 쓰는 얇은 래퍼 — 스트리밍 다운로드처럼 바이트가
/// 청크 단위로만 존재할 때는 [`Sha256Verifier`]를 직접 써서 청크마다 `update()`한다.
pub fn verify_artifact(bytes: &[u8], verification: &ArtifactVerification) -> Result<(), VerifyError> {
    let mut v = Sha256Verifier::new();
    v.update(bytes);
    v.finish(verification)
}

/// 청크 단위로 들어오는 바이트(스트리밍 다운로드 등)를 다 받지 않고도 증분 해시할 수
/// 있는 검증기 — [`verify_artifact`]가 요구하는 "바이트 전체가 이미 메모리에 있음"
/// 전제를 스트리밍 호출자에게 강요하지 않기 위함.
pub struct Sha256Verifier {
    hasher: Sha256,
}

impl Sha256Verifier {
    pub fn new() -> Self {
        Self { hasher: Sha256::new() }
    }

    pub fn update(&mut self, chunk: &[u8]) {
        self.hasher.update(chunk);
    }

    /// 비교 없이 지금까지 받은 청크 전체의 해시값만 반환 — 레시피 편집 화면에서
    /// 로컬 파일 하나를 골라 해시를 알아낼 때처럼, 대조할 기대값이 아직 없는(오히려
    /// 이 값 자체가 기대값이 될) 경우에 쓴다.
    pub fn finalize_hex(self) -> String {
        hex_encode(&self.hasher.finalize())
    }

    /// 지금까지 받은 청크 전체의 해시를 [`ArtifactVerification`] 선언과 대조.
    pub fn finish(self, verification: &ArtifactVerification) -> Result<(), VerifyError> {
        match verification {
            ArtifactVerification::Sha256 { hash } => {
                if !is_valid_sha256_hex(hash) {
                    return Err(VerifyError::InvalidHashFormat(hash.to_string()));
                }
                let expected = hash.trim().to_ascii_lowercase();
                let actual = hex_encode(&self.hasher.finalize());
                if actual == expected {
                    Ok(())
                } else {
                    Err(VerifyError::Sha256Mismatch { expected, actual })
                }
            }
        }
    }
}

/// `hash`가 유효한 SHA256 hex 문자열(대소문자 무관 64자리)인지 — 레시피 저장/임포트
/// 시점(`actions::install::validate_archive`)과 설치 시점([`Sha256Verifier::finish`])
/// 양쪽에서 같은 기준으로 쓰는 순수 함수.
pub fn is_valid_sha256_hex(hash: &str) -> bool {
    let trimmed = hash.trim();
    trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit())
}

impl Default for Sha256Verifier {
    fn default() -> Self {
        Self::new()
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        write!(s, "{b:02x}").expect("write! to String never fails");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 테스트 전용 — 실제 구현(`Sha256Verifier`)과 별도로 참조 해시를 계산해
    /// 하드코딩된 상수 없이도 "match passes" 계열 테스트가 자기완결적이게 한다.
    fn reference_sha256_hex(bytes: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        hex_encode(&hasher.finalize())
    }

    #[test]
    fn sha256_match_passes() {
        let bytes = b"hello world";
        let hash = reference_sha256_hex(bytes);
        let v = ArtifactVerification::Sha256 { hash };
        assert!(verify_artifact(bytes, &v).is_ok());
    }

    #[test]
    fn sha256_mismatch_rejected() {
        let bytes = b"hello world";
        let wrong_hash = "0".repeat(64);
        let v = ArtifactVerification::Sha256 { hash: wrong_hash };
        let err = verify_artifact(bytes, &v).unwrap_err();
        assert!(matches!(err, VerifyError::Sha256Mismatch { .. }));
    }

    #[test]
    fn sha256_tampered_bytes_rejected() {
        // 레시피가 원본("hello world")의 해시를 선언했는데, 실제로 변조된
        // 아티팩트("hello world!")를 받으면 검증에서 반드시 걸려야 함 — 핵심 위협 시나리오.
        let hash = reference_sha256_hex(b"hello world");
        let v = ArtifactVerification::Sha256 { hash };
        let tampered = b"hello world!";
        let err = verify_artifact(tampered, &v).unwrap_err();
        assert!(matches!(err, VerifyError::Sha256Mismatch { .. }));
    }

    #[test]
    fn finalize_hex_matches_reference_and_is_chunk_order_independent() {
        let mut v = Sha256Verifier::new();
        v.update(b"hello ");
        v.update(b"world");
        assert_eq!(v.finalize_hex(), reference_sha256_hex(b"hello world"));
    }

    #[test]
    fn sha256_case_insensitive_hash() {
        let bytes = b"hello world";
        let hash_upper = reference_sha256_hex(bytes).to_ascii_uppercase();
        let v = ArtifactVerification::Sha256 { hash: hash_upper };
        assert!(verify_artifact(bytes, &v).is_ok());
    }

    #[test]
    fn sha256_empty_bytes() {
        // 0바이트 다운로드도 정상적으로 검증 가능해야 함(경계값).
        let hash = reference_sha256_hex(b"");
        let v = ArtifactVerification::Sha256 { hash };
        assert!(verify_artifact(b"", &v).is_ok());
    }

    #[test]
    fn sha256_invalid_hash_format_rejected() {
        let v = ArtifactVerification::Sha256 { hash: "not-a-hash".to_string() };
        let err = verify_artifact(b"x", &v).unwrap_err();
        assert!(matches!(err, VerifyError::InvalidHashFormat(_)));
    }

    #[test]
    fn sha256_wrong_length_rejected() {
        let v = ArtifactVerification::Sha256 { hash: "deadbeef".to_string() };
        let err = verify_artifact(b"x", &v).unwrap_err();
        assert!(matches!(err, VerifyError::InvalidHashFormat(_)));
    }

    #[test]
    fn sha256_verifier_chunked_update_matches_single_shot() {
        // 스트리밍 다운로드처럼 여러 청크로 나눠 update() 해도, 한 번에 verify_artifact()
        // 한 것과 같은 결과여야 한다 — 청크 경계가 해시 결과에 영향을 주면 안 됨.
        let full = b"hello streaming world";
        let hash = reference_sha256_hex(full);
        let v = ArtifactVerification::Sha256 { hash };

        let mut verifier = Sha256Verifier::new();
        for chunk in full.chunks(5) {
            verifier.update(chunk);
        }
        assert!(verifier.finish(&v).is_ok());
    }

    #[test]
    fn sha256_verifier_chunked_tampered_rejected() {
        let hash = reference_sha256_hex(b"hello streaming world");
        let v = ArtifactVerification::Sha256 { hash };

        let mut verifier = Sha256Verifier::new();
        for chunk in b"hello streaming WORLD".chunks(5) {
            verifier.update(chunk);
        }
        let err = verifier.finish(&v).unwrap_err();
        assert!(matches!(err, VerifyError::Sha256Mismatch { .. }));
    }
}
