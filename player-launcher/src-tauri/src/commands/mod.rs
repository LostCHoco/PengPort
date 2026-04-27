//! 프론트엔드(React)가 호출하는 Tauri command 모음.
//!
//! 각 서브모듈은 관련 기능 단위로 그룹화한다.
//! 명명 규칙: `{동사}_{명사}` (snake_case) — 예: `fetch_meta`, `sync_instance`.

pub mod paths;
pub mod prism;
pub mod psp;
pub mod updater;
