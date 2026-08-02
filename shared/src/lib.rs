//! PengPort 플랫폼 공유 크레이트.
//!
//! ## 모듈 구성
//!
//! - **`library`** — 0.2.0 앱 라이브러리 핵심(v7 스키마): [`library::Recipe`] + 아티팩트
//!   검증([`library::verify`]) + 로컬 저장소([`library::LibraryStore`]) + 링크 임포트/
//!   내보내기([`library::bundle`], [`library::import`]).
//! - **`actions`** — `Recipe.archives`/`Recipe.files`/`Recipe.launch` 검증
//!   ([`actions::validate_recipe`]) + third-party app 탐지([`actions::third_party_app`]).
//! - **`servers_dat`** — Minecraft `.minecraft/servers.dat` (NBT). 런타임 병합 경로는
//!   v5 재설계로 불필요해졌으나(third-party app 데이터는 정적 콘텐츠로 레시피가 직접
//!   들고 있음), 레시피 작성 시점 도구로 재활용할지는 열린 질문 — 결정 전까지 존치.

pub mod actions;
pub mod ids;
pub mod library;
pub mod servers_dat;

pub use ids::{is_valid_service_id, validate_service_id, IdError};
pub use servers_dat::{upsert_server as upsert_servers_dat, ServersDat, ServersDatError};
