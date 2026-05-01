//! PSP (PengPort Service Protocol) Tauri commands — Phase 1.
//!
//! 명세: `docs/spec/psp-v1.md`. 보안 모델: `docs/spec/05-psp.md` 섹션 12.
//!
//! 기존 commands (servers / prism / updater) 와 병행. PSP UI 가 정착하면
//! 점진 마이그레이션.
//!
//! ## 구조
//!
//! - **fetch**: `psp_load_instance`, `psp_load_manifest`
//! - **validate**: `psp_validate_manifest`
//! - **invoke**: `psp_invoke_action` (open_url/open_protocol/submit_form/third_party),
//!   `psp_submit_form_with_data`
//! - **trust** (TOFU): `psp_trust`, `psp_revoke_trust`, `psp_list_trusts`
//!
//! ## third_party 흐름 (3-tier 신뢰 모델)
//!
//! ```text
//! frontend: psp_invoke_action(kind=third_party, args, instance_id=...)
//!   ↓
//! backend: dispatch → ThirdPartyAppIntent
//!   ↓
//! backend: trust check (kind="third_party.{app_id}", id="host:port")
//!   ├── trusted (+ packwiz_url 동일) → upsert_prism_instance + spawn_prism → ActionOutcome::Launched
//!   └── untrusted → ActionOutcome::NeedsConfirm { ... }
//!         ↓
//!       frontend: 사용자 dialog → 동의 시 psp_trust(...) → 다시 invoke
//! ```

use std::path::PathBuf;
use std::time::Duration;

use pengport_shared::actions::manifest_check::{
    validate_manifest, ManifestCheckContext, ManifestValidationError,
};
use pengport_shared::actions::third_party::prism_launcher::PrismLauncherConfig;
use pengport_shared::actions::{dispatch, ActionContext, ActionIntent, ThirdPartyAppIntent};
use pengport_shared::psp::catalog::ServicesCatalog;
use pengport_shared::psp::fetch::{
    fetch_instance_metadata, fetch_service_manifest, fetch_services_catalog,
};
use pengport_shared::psp::manifest::NativeActionKind;
use pengport_shared::psp::{InstanceMetadata, ServiceManifest};
use pengport_shared::trust::{TrustEntry, TrustStore};
use serde::{Deserialize, Serialize};
use tauri_plugin_opener::OpenerExt;

const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

/// dev/release 별 HTTP 허용 (CSP 와 별도, URL allowlist 검증용).
fn allow_http() -> bool {
    cfg!(debug_assertions)
}

/// `%APPDATA%/app.pengport/trust.json` (또는 portable 모드의 `<exe>/data/trust.json`).
fn trust_store_path() -> Result<PathBuf, String> {
    super::paths::app_data_root()
        .map(|d| d.join("trust.json"))
        .ok_or_else(|| "app_data_root 미정 (%APPDATA% 환경변수 없음)".to_string())
}

async fn load_trust_store() -> Result<TrustStore, String> {
    let path = trust_store_path()?;
    tauri::async_runtime::spawn_blocking(move || TrustStore::load(path).map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("blocking task panic: {e}"))?
}

async fn save_trust_store(store: TrustStore) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || store.save().map_err(|e| e.to_string()))
        .await
        .map_err(|e| format!("blocking task panic: {e}"))?
}

// ===== Fetch =====

#[tauri::command]
pub async fn psp_load_instance(instance_url: String) -> Result<InstanceMetadata, String> {
    tauri::async_runtime::spawn_blocking(move || {
        fetch_instance_metadata(&instance_url, HTTP_TIMEOUT).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("blocking task panic: {e}"))?
}

#[tauri::command]
pub async fn psp_load_manifest(
    service_url: String,
    bearer_token: Option<String>,
) -> Result<ServiceManifest, String> {
    tauri::async_runtime::spawn_blocking(move || {
        fetch_service_manifest(&service_url, bearer_token.as_deref(), HTTP_TIMEOUT)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("blocking task panic: {e}"))?
}

#[tauri::command]
pub async fn psp_load_catalog(
    catalog_url: String,
    bearer_token: Option<String>,
) -> Result<ServicesCatalog, String> {
    let mut catalog = tauri::async_runtime::spawn_blocking(move || {
        fetch_services_catalog(&catalog_url, bearer_token.as_deref(), HTTP_TIMEOUT)
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("blocking task panic: {e}"))??;

    // 보안: catalog 의 service.id 가 fs-safe 형식이 아니면 entry 자체를 제거.
    // service id 는 Prism instance dir name + maintenance 의 path component 로 쓰이므로
    // path traversal 차단을 위해 이 단계에서 sanitize. invalid id 는 사용자에게 표시도 안 함.
    catalog.services.retain(|s| {
        if pengport_shared::is_valid_service_id(&s.id) {
            true
        } else {
            eprintln!(
                "PSP catalog: service id {:?} 가 fs-safe 형식이 아니라 거부 (path traversal 방지)",
                s.id
            );
            false
        }
    });

    Ok(catalog)
}

// ===== Validate =====

/// manifest 일관성 검증. catalog id 가 있으면 manifest.id 와 매칭 검사.
#[tauri::command]
pub fn psp_validate_manifest(
    manifest: ServiceManifest,
    base_url: String,
    catalog_id: Option<String>,
) -> Result<(), String> {
    let ctx = ManifestCheckContext {
        base_url: &base_url,
        catalog_id: catalog_id.as_deref(),
        allow_http: allow_http(),
    };
    validate_manifest(&manifest, &ctx).map_err(|e: ManifestValidationError| e.to_string())
}

// ===== Dispatch + Invoke =====

#[derive(Debug, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ActionOutcome {
    /// open_url / open_protocol — OS 가 URL 열기 끝.
    Done,
    /// submit_form — POST 응답.
    Submitted { status: u16 },
    /// third_party — 신뢰된 대상이라 즉시 실행함.
    Launched { instance_id: String },
    /// 사용자 동의 필요 (Tier 2/3). frontend 가 dialog 띄우고 사용자 확인 후
    /// `psp_trust` 호출 → 동일 invoke 재시도.
    NeedsConfirm {
        /// trust kind (예: `"third_party.prism-launcher"`).
        trust_kind: String,
        /// trust subject id (예: `"play.example.com:25565"`).
        subject_id: String,
        /// 사용자 표시용 이름.
        display: String,
        /// 사용자에게 보여줄 컨텍스트 (host, port, packwiz_url, version, install_hint 등).
        details: serde_json::Value,
    },
    /// third_party app 이 시스템에 없음. frontend 가 inline 동의 dialog → 자동 다운로드 →
    /// 동일 invoke 재시도. 사용자가 의도적으로 설치하는 흐름이라 페이지 이동 없이 처리.
    ThirdPartyMissing {
        /// 어느 third-party app 이 필요한지 (예: `"prism-launcher"`).
        app_id: String,
        /// manifest 가 명시한 install hint (`{name, homepage?}` 또는 null).
        /// frontend 가 dialog 의 안내 문구에 활용.
        install_hint: serde_json::Value,
    },
}

/// Action dispatch + OS 호출.
///
/// `instance_id` 는 third_party 분기에서만 의미. 다른 kind 에서는 무시.
#[tauri::command]
pub async fn psp_invoke_action(
    kind: String,
    args: serde_json::Value,
    manifest_origin: String,
    external_urls: Vec<String>,
    instance_id: Option<String>,
    app: tauri::AppHandle,
) -> Result<ActionOutcome, String> {
    let kind_enum: NativeActionKind =
        serde_json::from_value(serde_json::Value::String(kind.clone()))
            .map_err(|_| format!("unknown native action kind: {kind}"))?;

    let intent = {
        let ctx = ActionContext {
            manifest_origin: &manifest_origin,
            external_urls: &external_urls,
            allow_http: allow_http(),
        };
        dispatch(kind_enum, &args, &ctx).map_err(|e| e.to_string())?
    };

    match intent {
        ActionIntent::OpenUrl(i) => {
            app.opener()
                .open_url(&i.url, None::<&str>)
                .map_err(|e| format!("open_url 실패: {e}"))?;
            Ok(ActionOutcome::Done)
        }
        ActionIntent::OpenProtocol(i) => {
            app.opener()
                .open_url(&i.full_url, None::<&str>)
                .map_err(|e| format!("open_protocol 실패: {e}"))?;
            Ok(ActionOutcome::Done)
        }
        ActionIntent::SubmitForm(i) => {
            // 단순 POST 트리거 (fields 비어있는 경우). 폼 데이터 동봉은
            // `psp_submit_form_with_data` 별도 command.
            let endpoint = i.endpoint_url.clone();
            let status = tauri::async_runtime::spawn_blocking(move || {
                let agent = ureq::Agent::new_with_defaults();
                agent
                    .post(&endpoint)
                    .send_empty()
                    .map(|r| r.status().as_u16())
                    .map_err(|e| format!("submit_form POST 실패: {e}"))
            })
            .await
            .map_err(|e| format!("blocking task panic: {e}"))??;
            Ok(ActionOutcome::Submitted { status })
        }
        ActionIntent::ThirdPartyApp(i) => {
            let instance_id = instance_id.ok_or_else(|| {
                "third_party action 호출 시 instance_id 인자 필수".to_string()
            })?;
            invoke_third_party(i, instance_id, &app).await
        }
    }
}

/// third_party 실행 + trust check. `psp_invoke_action` 의 ThirdPartyApp 분기 본체.
async fn invoke_third_party(
    intent: ThirdPartyAppIntent,
    instance_id: String,
    app: &tauri::AppHandle,
) -> Result<ActionOutcome, String> {
    // 보안 1차 방어선: instance_id (= PSP service id) 가 fs-safe 형식인지.
    // catalog 가 attacker-controlled 이므로 service.id 도 attacker-controlled. 이 id 가
    // upsert_prism_instance / spawn_prism_instance 의 path component 로 쓰이므로 traversal 차단.
    pengport_shared::validate_service_id(&instance_id)
        .map_err(|e| format!("service id 형식 오류 ({instance_id:?}): {e}"))?;

    // Phase 1 카탈로그: prism-launcher 만.
    if intent.app_id != "prism-launcher" {
        return Err(format!(
            "미지원 third_party app: '{}' (Phase 1: prism-launcher 만)",
            intent.app_id
        ));
    }

    let config: PrismLauncherConfig = serde_json::from_value(intent.config.clone())
        .map_err(|e| format!("PrismLauncherConfig 파싱 실패: {e}"))?;

    let trust_kind = format!("third_party.{}", intent.app_id);
    let subject_id = format!("{}:{}", config.host, config.port);

    // Trust check — host:port 일치 + packwiz_url 동일 시에만 trusted.
    // packwiz_url 변경 = 코드 실행 출처 변경 → 재confirm (TOFU + identity-key 패턴).
    let store = load_trust_store().await?;
    let trusted = match store.find(&trust_kind, &subject_id) {
        Some(entry) => {
            let stored_packwiz = entry
                .metadata
                .get("packwiz_url")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let current_packwiz = config.packwiz_url.clone();
            stored_packwiz == current_packwiz
        }
        None => false,
    };

    if !trusted {
        let display = config
            .display_name
            .clone()
            .unwrap_or_else(|| format!("{}:{}", config.host, config.port));
        let details = serde_json::json!({
            "app_id": intent.app_id,
            "host": config.host,
            "port": config.port,
            "version": config.version,
            "loader": config.loader,
            "loader_version": config.loader_version,
            "packwiz_url": config.packwiz_url,
            "install_hint": intent.install_hint,
        });
        return Ok(ActionOutcome::NeedsConfirm {
            trust_kind,
            subject_id,
            display,
            details,
        });
    }

    // trusted → 인스턴스 sync + Prism spawn.
    // prism 미설치 시 별도 outcome 으로 빠짐 (frontend 가 inline 설치 dialog).
    let prism_paths = match super::prism::prism_paths() {
        Ok((_, p)) => p,
        Err(_) => {
            return Ok(ActionOutcome::ThirdPartyMissing {
                app_id: intent.app_id.clone(),
                install_hint: serde_json::to_value(&intent.install_hint)
                    .unwrap_or(serde_json::Value::Null),
            });
        }
    };
    let jar = super::prism::ensure_bootstrap_jar(app)?;

    let instance_id_for_sync = instance_id.clone();
    let config_for_sync = config.clone();
    tauri::async_runtime::spawn_blocking(move || {
        pengport_shared::prism::upsert_prism_instance(
            &prism_paths,
            &instance_id_for_sync,
            &config_for_sync,
            &jar,
        )
    })
    .await
    .map_err(|e| format!("blocking task panic: {e}"))?
    .map_err(|e| e.to_string())?;

    super::prism::spawn_prism_instance(app, &instance_id)?;

    Ok(ActionOutcome::Launched { instance_id })
}

/// Submit form with user-filled data (fields 가 있는 경우).
///
/// frontend 가 폼 UI 동적 생성 후 사용자 입력 받아서 호출. shared 의 dispatch
/// 로 endpoint 검증 후 POST 전송.
#[tauri::command]
pub async fn psp_submit_form_with_data(
    args: serde_json::Value,        // SubmitFormArgs (endpoint + fields + ...)
    field_values: serde_json::Value, // {field_name: user_value, ...}
    manifest_origin: String,
    external_urls: Vec<String>,
    bearer_token: Option<String>,
) -> Result<ActionOutcome, String> {
    use pengport_shared::actions::validate_submit_form;

    let intent = {
        let ctx = ActionContext {
            manifest_origin: &manifest_origin,
            external_urls: &external_urls,
            allow_http: allow_http(),
        };
        validate_submit_form(&args, &ctx).map_err(|e| e.to_string())?
    };

    let endpoint = intent.endpoint_url.clone();
    let body = serde_json::json!({"fields": field_values});

    let status = tauri::async_runtime::spawn_blocking(move || {
        let agent = ureq::Agent::new_with_defaults();
        let mut req = agent.post(&endpoint);
        if let Some(token) = bearer_token {
            req = req.header("Authorization", format!("Bearer {token}"));
        }
        req.send_json(body)
            .map(|r| r.status().as_u16())
            .map_err(|e| format!("submit_form POST 실패: {e}"))
    })
    .await
    .map_err(|e| format!("blocking task panic: {e}"))??;

    Ok(ActionOutcome::Submitted { status })
}

// ===== Trust =====

/// 사용자가 명시적으로 `NeedsConfirm` 에 대해 동의했을 때 frontend 가 호출.
///
/// 같은 (trust_kind, subject_id) 가 이미 있으면 metadata/display 갱신 (TOFU 의 갱신 케이스).
#[tauri::command]
pub async fn psp_trust(
    trust_kind: String,
    subject_id: String,
    display: String,
    metadata: serde_json::Value,
) -> Result<(), String> {
    let mut store = load_trust_store().await?;
    store.upsert(TrustEntry::new(trust_kind, subject_id, display, metadata));
    save_trust_store(store).await
}

/// 신뢰 철회. 다음 invoke 시 `NeedsConfirm` 으로 돌아감.
#[tauri::command]
pub async fn psp_revoke_trust(trust_kind: String, subject_id: String) -> Result<bool, String> {
    let mut store = load_trust_store().await?;
    let removed = store.revoke(&trust_kind, &subject_id);
    save_trust_store(store).await?;
    Ok(removed)
}

/// 신뢰 목록 (Settings UI 의 "신뢰 관리" 페이지에서 사용).
///
/// `kind_filter` 가 None 이면 전체. `Some("third_party.prism-launcher")` 같은 식으로 필터.
#[tauri::command]
pub async fn psp_list_trusts(kind_filter: Option<String>) -> Result<Vec<TrustEntryDto>, String> {
    let store = load_trust_store().await?;
    let entries: Vec<TrustEntryDto> = store
        .list(kind_filter.as_deref())
        .into_iter()
        .map(TrustEntryDto::from)
        .collect();
    Ok(entries)
}

/// frontend 직렬화 전용 view (TrustEntry 와 동일 필드, Serialize 만 필요).
#[derive(Debug, Serialize, Deserialize)]
pub struct TrustEntryDto {
    pub subject_kind: String,
    pub subject_id: String,
    pub display: String,
    pub metadata: serde_json::Value,
    pub trusted_at: i64,
}

impl From<&TrustEntry> for TrustEntryDto {
    fn from(e: &TrustEntry) -> Self {
        Self {
            subject_kind: e.subject_kind.clone(),
            subject_id: e.subject_id.clone(),
            display: e.display.clone(),
            metadata: e.metadata.clone(),
            trusted_at: e.trusted_at,
        }
    }
}
