//! PengPort 플랫폼 공유 크레이트.
//!
//! ## 모듈 구성
//!
//! - **`psp`** — PSP v1 schema (manifest, status, events, catalog, instance metadata, fetch)
//! - **`actions`** — PSP action 검증 + dispatch (open_url / open_protocol / submit_form / native_third_party_app)
//! - **`trust`** — TOFU 신뢰 저장소 (3-tier 신뢰 모델 영속화)
//! - **`prism`** — PSP `third_party.prism-launcher` entry 의 인스턴스 sync (Prism 측 instance.cfg / mmc-pack.json 렌더 + servers.dat 등록)
//! - **`servers_dat`** — Minecraft `.minecraft/servers.dat` (NBT) 자동 등록
//!
//! 옛 servers.toml 흐름 (`servers`, `cdn`, `status`) 은 PSP 단방향 마이그레이션과 함께 제거됨.

pub mod actions;
pub mod prism;
pub mod psp;
pub mod servers_dat;
pub mod trust;

pub use prism::{upsert_prism_instance, InstanceOutcome, PrismError, PrismPaths};
pub use servers_dat::{upsert_server as upsert_servers_dat, ServersDat, ServersDatError};
