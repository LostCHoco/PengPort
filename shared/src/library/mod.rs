//! 앱 라이브러리 — 0.2.0 본질(설치→인증→실행 원클릭 + flat 라이브러리)의 핵심 데이터 모델.
//!
//! [`Recipe`](v8 스키마, `recipe.rs` 모듈 설명 참고)가 라이브러리의 유일한 단위,
//! [`LibraryStore`]가 유일한 영속 데이터([`LibraryEntry`]로 감싸 로컬 전용 필드 분리).
//!
//! 신뢰 모델은 두 층:
//! - **아티팩트 검증**(다운로드물이 레시피가 선언한 해시와 일치하는가) — [`verify`].
//! - **`.pengz` 파일 임포트 시 1회 confirm**(번들에 뭐가 들어있는지 보여주기) — [`import`].

pub mod bundle;
pub mod import;
pub mod recipe;
pub mod store;
pub mod third_party_store;
pub mod verify;

pub use bundle::{decode_bundle_file, encode_bundle_file, BundleError, FILE_EXTENSION};
pub use import::{ImportPreview, ImportPreviewItem};
pub use recipe::{
    ArchiveExtraction, ArtifactVerification, FileContent, FolderRule, FolderRuleMode,
    LaunchAction, OptionalGroup, OverrideContent, PathOverride, Recipe, RecipeFile, RecipeInfo,
};
pub use store::{LibraryEntry, LibraryError, LibraryStore};
pub use third_party_store::{ThirdPartyAppStore, ThirdPartyStoreError};
pub use verify::{is_valid_sha256_hex, verify_artifact, Sha256Verifier, VerifyError};
