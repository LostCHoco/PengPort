//! 프론트엔드(React)가 호출하는 Tauri command 모음.
//!
//! 각 서브모듈은 관련 기능 단위로 그룹화한다.
//! 명명 규칙: `{동사}_{명사}` (snake_case) — 예: `fetch_meta`, `sync_instance`.

pub mod browser_download;
pub mod file_import;
pub mod library;
pub mod maintenance;
pub mod paths;
pub mod self_update;
pub mod third_party_runtime;

// `secrets`(instance_token_* — 인스턴스별 keyring 토큰)는 0.2.0에서 인스턴스 개념이
// 없어지며 호출부가 사라짐. 파일은 남겨둠(컴파일됨, 커맨드 등록만 안 함) — 다른
// 레시피별 시크릿 필요가 생기면 재사용 가능. `trust.rs`(shared 크레이트)도 동일 취급.
pub mod secrets;

/// detached `.bat` 스크립트(`self_update.rs`/`maintenance.rs`가 재시작/정리용으로
/// 생성)를 UTF-8 BOM과 함께 쓴다 — BOM 없이 그냥 UTF-8 바이트로 쓰면 `cmd.exe`가
/// 시스템 ANSI 코드페이지(한국어 Windows는 CP949)로 해석해서, 스크립트 안에 담긴
/// 한글 경로(OneDrive 폴더명 등)가 깨져 "파일을 찾을 수 없습니다" 에러로 이어진다
/// (2026-08 실사용자가 실제로 겪음 — `OneDrive\바탕 화면\펭포트\...` 경로에서
/// 재현). BOM을 붙이면 Windows 10/11의 `cmd.exe`가 파일을 UTF-8로 인식해 정상
/// 해석한다.
pub(super) fn write_windows_batch_script(path: &std::path::Path, content: &str) -> std::io::Result<()> {
    const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];
    let mut bytes = Vec::with_capacity(UTF8_BOM.len() + content.len());
    bytes.extend_from_slice(&UTF8_BOM);
    bytes.extend_from_slice(content.as_bytes());
    std::fs::write(path, bytes)
}
