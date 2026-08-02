//! `.pengz` 파일 임포트/내보내기 — 레시피 1개 또는 여러 개를 스냅샷으로 인코딩.
//!
//! **스냅샷이지 구독이 아니다**: 임포트는 1회성 — 원본이 나중에 바뀌어도 이미
//! 임포트한 라이브러리 항목엔 반영 안 됨. 호스팅도 필요 없다 — 파일 자체가 데이터를
//! 통째로 담는다(gzip 압축 JSON). 사용자는 이 파일을 디스코드/카톡 등 어디로든
//! 그냥 보내면 된다.
//!
//! 2026-08 이전엔 base64url 딥링크(`pengport://import?data=...`)도 있었으나, 다건/
//! 대용량 번들에서 Windows `CreateProcess`의 ~32KB 명령줄 길이 제한에 걸리는 문제가
//! 있어(URL 전체가 프로토콜 핸들러 실행 인자로 실림) 파일 방식으로 완전히 대체하고
//! 제거했다(`docs/track/portable-transition.md` 인접 세션 결정 — 파일 크기엔 사실상
//! 제한이 없고, 관리해야 할 임포트 경로도 하나로 줄어듦).

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::recipe::Recipe;
use crate::actions::ThirdPartyAppDescriptor;
use crate::ids::validate_service_id;

/// v8 레시피 스키마(archives/files 항목의 `target` 필드 제거) 도입으로 2→3, third-party
/// app descriptor 를 번들에 포함하며 3→4, `ArchiveExtraction::order`(필수 필드) 추가로
/// 4→5. 스키마 버전이 안 맞으면 명시적 거부(구버전 번들이 조용히 일부만 디코딩되는 것
/// 방지 — `order` 없는 구버전 압축은 필드 자체가 없어 파싱 단계에서 걸러야 함).
pub const SCHEMA_VERSION: u32 = 5;

/// 번들 하나에 담을 수 있는 최대 레시피 수. 무한정 커지는 걸 방지(악의적으로 거대한
/// 번들을 만들어 파싱 부담을 주는 것 방지).
pub const MAX_BUNDLE_RECIPES: usize = 50;

/// 번들 하나에 담을 수 있는 최대 third-party app descriptor 수. 레시피 하나가 보통
/// 참조하는 서드파티 앱은 0~1개라 레시피보다 훨씬 작은 한도로 충분.
pub const MAX_BUNDLE_THIRD_PARTY_APPS: usize = 10;

/// 파일로 내보낼 때 쓰는 확장자(점 없이) — `.pengz`. gzip 압축된 [`BundleEnvelope`]
/// JSON을 그대로 담는다 — 파일이라 URL-safe 텍스트일 필요가 없고, 바이너리 그대로
/// 저장/전송 가능.
pub const FILE_EXTENSION: &str = "pengz";

#[derive(Debug, Error, PartialEq, Eq)]
pub enum BundleError {
    #[error("번들이 비어있음 — 레시피 또는 third-party app 이 1개 이상 필요")]
    Empty,

    #[error("번들의 레시피 수가 한도 초과 (최대 {MAX_BUNDLE_RECIPES}, 받음 {0})")]
    TooManyRecipes(usize),

    #[error("번들의 third-party app 수가 한도 초과 (최대 {MAX_BUNDLE_THIRD_PARTY_APPS}, 받음 {0})")]
    TooManyThirdPartyApps(usize),

    #[error("미지원 bundle schema version: {0} (현재 {SCHEMA_VERSION})")]
    UnsupportedVersion(u32),

    #[error("JSON 파싱 실패: {0}")]
    Json(String),

    #[error("gzip 압축/해제 실패: {0}")]
    Gzip(String),

    #[error("레시피 id '{id}' 가 유효하지 않음: {source}")]
    InvalidRecipeId {
        id: String,
        #[source]
        source: crate::ids::IdError,
    },

    #[error("third-party app id '{id}' 가 유효하지 않음: {source}")]
    InvalidThirdPartyAppId {
        id: String,
        #[source]
        source: crate::ids::IdError,
    },
}

/// [`decode_bundle_file`]의 결과 — 레시피와 third-party app descriptor 둘 다 같은
/// 1회성 confirm 흐름 안에서 함께 온다(`import.rs` 참고).
#[derive(Debug, Clone, Default)]
pub struct BundleContents {
    pub recipes: Vec<Recipe>,
    pub third_party_apps: Vec<ThirdPartyAppDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct BundleEnvelope {
    schema_version: u32,
    recipes: Vec<Recipe>,
    #[serde(default)]
    third_party_apps: Vec<ThirdPartyAppDescriptor>,
}

/// 레시피/third-party app 개수 한도를 확인하고 [`BundleEnvelope`]로 감싼다.
fn build_envelope(
    recipes: &[Recipe],
    third_party_apps: &[ThirdPartyAppDescriptor],
) -> Result<BundleEnvelope, BundleError> {
    if recipes.is_empty() && third_party_apps.is_empty() {
        return Err(BundleError::Empty);
    }
    if recipes.len() > MAX_BUNDLE_RECIPES {
        return Err(BundleError::TooManyRecipes(recipes.len()));
    }
    if third_party_apps.len() > MAX_BUNDLE_THIRD_PARTY_APPS {
        return Err(BundleError::TooManyThirdPartyApps(third_party_apps.len()));
    }
    Ok(BundleEnvelope {
        schema_version: SCHEMA_VERSION,
        recipes: recipes.to_vec(),
        third_party_apps: third_party_apps.to_vec(),
    })
}

/// 역직렬화된 [`BundleEnvelope`]의 구조 검증(스키마 버전, 개수 한도, id 형식).
/// **untrusted 입력**(파일은 누구든 만들어 보낼 수 있음) — 여기서 하는 검증은
/// "구조가 유효한가"까지다. 아티팩트 신뢰(서명/해시)는 실제 설치 시점에
/// `verify::verify_artifact` 가 별도로 검사한다 — 이 함수 통과가 "설치해도 안전하다"는
/// 뜻은 아니다.
fn validate_envelope(envelope: BundleEnvelope) -> Result<BundleContents, BundleError> {
    if envelope.schema_version != SCHEMA_VERSION {
        return Err(BundleError::UnsupportedVersion(envelope.schema_version));
    }
    if envelope.recipes.is_empty() && envelope.third_party_apps.is_empty() {
        return Err(BundleError::Empty);
    }
    if envelope.recipes.len() > MAX_BUNDLE_RECIPES {
        return Err(BundleError::TooManyRecipes(envelope.recipes.len()));
    }
    if envelope.third_party_apps.len() > MAX_BUNDLE_THIRD_PARTY_APPS {
        return Err(BundleError::TooManyThirdPartyApps(envelope.third_party_apps.len()));
    }
    for r in &envelope.recipes {
        validate_service_id(&r.id).map_err(|source| BundleError::InvalidRecipeId {
            id: r.id.clone(),
            source,
        })?;
    }
    for d in &envelope.third_party_apps {
        validate_service_id(&d.id).map_err(|source| BundleError::InvalidThirdPartyAppId {
            id: d.id.clone(),
            source,
        })?;
    }

    Ok(BundleContents {
        recipes: envelope.recipes,
        third_party_apps: envelope.third_party_apps,
    })
}

/// 레시피 + third-party app descriptor 를 `.pengz` 파일에 쓸 바이트로 인코딩 — gzip
/// 압축된 JSON, base64 없음(파일이라 바이너리 그대로 저장 가능).
pub fn encode_bundle_file(
    recipes: &[Recipe],
    third_party_apps: &[ThirdPartyAppDescriptor],
) -> Result<Vec<u8>, BundleError> {
    use std::io::Write;

    let envelope = build_envelope(recipes, third_party_apps)?;
    let json = serde_json::to_vec(&envelope).map_err(|e| BundleError::Json(e.to_string()))?;
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    encoder
        .write_all(&json)
        .map_err(|e| BundleError::Gzip(e.to_string()))?;
    encoder.finish().map_err(|e| BundleError::Gzip(e.to_string()))
}

/// `encode_bundle_file` 의 결과(`.pengz` 파일 바이트)를 디코딩 + 검증.
pub fn decode_bundle_file(bytes: &[u8]) -> Result<BundleContents, BundleError> {
    use std::io::Read;

    let mut decoder = flate2::read::GzDecoder::new(bytes);
    let mut json = Vec::new();
    decoder
        .read_to_end(&mut json)
        .map_err(|e| BundleError::Gzip(e.to_string()))?;
    let envelope: BundleEnvelope =
        serde_json::from_slice(&json).map_err(|e| BundleError::Json(e.to_string()))?;
    validate_envelope(envelope)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions::ThirdPartyAppDescriptor;
    use crate::library::recipe::{LaunchAction};

    fn sample_recipe(id: &str) -> Recipe {
        Recipe {
            id: id.to_string(),
            name: format!("App {id}"),
            recipe_info: Default::default(),
            archives: vec![],
            files: vec![],
            optional_groups: vec![],
            folder_rules: vec![],
            launch: LaunchAction::SpawnProcess {
                entry_point: "x.exe".to_string(),
                entry_args: vec![],
            },
        }
    }

    fn sample_descriptor(id: &str) -> ThirdPartyAppDescriptor {
        ThirdPartyAppDescriptor {
            id: id.to_string(),
            label: None,
            exe_filename: "test_app.exe".to_string(),
            download_strategy: None,
            instances_subfolder: None,
            system_appdata_folder_name: None,
            readiness_signal: None,
            launch_args_template: vec![],
            post_download_marker_files: vec![],
        }
    }

    #[test]
    fn round_trip_bundle_file() {
        let recipes = vec![sample_recipe("sample-service"), sample_recipe("sample-service-2")];
        let apps = vec![sample_descriptor("test_app")];
        let bytes = encode_bundle_file(&recipes, &apps).unwrap();
        // gzip 매직 바이트(0x1f 0x8b) — base64 인코딩을 안 거친다는 것도 같이 확인.
        assert_eq!(&bytes[..2], &[0x1f, 0x8b]);
        let decoded = decode_bundle_file(&bytes).unwrap();
        assert_eq!(decoded.recipes.len(), 2);
        assert_eq!(decoded.third_party_apps.len(), 1);
    }

    #[test]
    fn decode_bundle_file_garbage_rejected() {
        let err = decode_bundle_file(b"not gzip data").unwrap_err();
        assert!(matches!(err, BundleError::Gzip(_)));
    }

    #[test]
    fn decode_bundle_file_rejects_path_traversal_id() {
        let recipe = sample_recipe("../../evil");
        let bytes = encode_bundle_file(&[recipe], &[]).unwrap();
        let err = decode_bundle_file(&bytes).unwrap_err();
        assert!(matches!(err, BundleError::InvalidRecipeId { .. }));
    }

    #[test]
    fn decode_bundle_file_rejects_path_traversal_third_party_app_id() {
        let descriptor = sample_descriptor("../../evil");
        let bytes = encode_bundle_file(&[], &[descriptor]).unwrap();
        let err = decode_bundle_file(&bytes).unwrap_err();
        assert!(matches!(err, BundleError::InvalidThirdPartyAppId { .. }));
    }

    #[test]
    fn decode_bundle_file_unsupported_version_rejected() {
        let envelope = BundleEnvelope {
            schema_version: 999,
            recipes: vec![sample_recipe("x")],
            third_party_apps: vec![],
        };
        let json = serde_json::to_vec(&envelope).unwrap();
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        std::io::Write::write_all(&mut encoder, &json).unwrap();
        let bytes = encoder.finish().unwrap();
        let err = decode_bundle_file(&bytes).unwrap_err();
        assert!(matches!(err, BundleError::UnsupportedVersion(999)));
    }

    #[test]
    fn decode_bundle_file_empty_rejected() {
        let envelope = BundleEnvelope {
            schema_version: SCHEMA_VERSION,
            recipes: vec![],
            third_party_apps: vec![],
        };
        let json = serde_json::to_vec(&envelope).unwrap();
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
        std::io::Write::write_all(&mut encoder, &json).unwrap();
        let bytes = encoder.finish().unwrap();
        assert_eq!(decode_bundle_file(&bytes).unwrap_err(), BundleError::Empty);
    }

    #[test]
    fn encode_empty_rejected() {
        assert_eq!(encode_bundle_file(&[], &[]).unwrap_err(), BundleError::Empty);
    }

    #[test]
    fn encode_too_many_recipes_rejected() {
        let recipes: Vec<Recipe> = (0..MAX_BUNDLE_RECIPES + 1)
            .map(|i| sample_recipe(&format!("app{i}")))
            .collect();
        let err = encode_bundle_file(&recipes, &[]).unwrap_err();
        assert!(matches!(err, BundleError::TooManyRecipes(_)));
    }

    #[test]
    fn encode_too_many_third_party_apps_rejected() {
        let apps: Vec<ThirdPartyAppDescriptor> = (0..MAX_BUNDLE_THIRD_PARTY_APPS + 1)
            .map(|i| sample_descriptor(&format!("app{i}")))
            .collect();
        let err = encode_bundle_file(&[], &apps).unwrap_err();
        assert!(matches!(err, BundleError::TooManyThirdPartyApps(_)));
    }
}
