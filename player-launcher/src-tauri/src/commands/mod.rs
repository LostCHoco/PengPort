//! 프론트엔드(React)가 호출하는 Tauri command 모음.
//!
//! 각 서브모듈은 관련 기능 단위로 그룹화한다.
//! 명명 규칙: `{동사}_{명사}` (snake_case) — 예: `fetch_meta`, `sync_instance`.

pub mod browser_download;
pub mod config_patch;
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
