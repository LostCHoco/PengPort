//! 자동 업데이트 토큰 관리.
//!
//! Caddy 의 `/updates/*` 엔드포인트는 Bearer 토큰을 요구한다.
//! 토큰 우선순위:
//!   1. OS keychain (`secrets::load_updater_token`) — 사용자가 Settings 에서 저장한 값
//!   2. 빌드 타임에 임베드된 기본값 (`PENGPORT_UPDATES_TOKEN` env var)
//!
//! 회전 시: 서버 토큰 변경 → 친구가 Settings 에 새 토큰 입력 → 끝.
//! 재설치 불필요. keyring 은 NSIS update 가 건드리지 않으므로 토큰도 보존됨.
//!
//! ## 마이그레이션 (옛 `updater.toml` → keyring)
//!
//! 이전 버전은 `%APPDATA%/app.pengport/updater.toml` 에 평문 저장했음.
//! 첫 호출 시 자동으로 keyring 으로 옮기고 옛 파일 삭제 (best-effort, 실패 무시).

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::{paths, secrets};

const EMBEDDED_TOKEN: Option<&str> = option_env!("PENGPORT_UPDATES_TOKEN");

#[derive(Debug, Default, Serialize, Deserialize)]
struct LegacyUpdaterToml {
    token: Option<String>,
}

fn legacy_settings_path() -> Option<PathBuf> {
    paths::app_data_root().map(|d| d.join("updater.toml"))
}

/// 옛 `updater.toml` 이 있으면 keyring 으로 옮기고 파일 삭제.
/// 한 번만 작동 (이후엔 파일 없음). 모든 단계에서 best-effort — 실패 시 silent.
fn migrate_legacy_toml() {
    let Some(path) = legacy_settings_path() else {
        return;
    };
    if !path.exists() {
        return;
    }
    let Ok(text) = fs::read_to_string(&path) else {
        return;
    };
    let Ok(legacy): Result<LegacyUpdaterToml, _> = toml::from_str(&text) else {
        // 파싱 실패해도 파일은 정리 (옛 손상된 파일 잔재)
        let _ = fs::remove_file(&path);
        return;
    };
    if let Some(token) = legacy.token.as_deref().filter(|t| !t.is_empty()) {
        // keyring 저장 성공 시에만 파일 삭제 (실패하면 파일 보존하여 다음 시도)
        if secrets::save_updater_token(token).is_ok() {
            let _ = fs::remove_file(&path);
        }
    } else {
        // 빈 토큰이면 그냥 정리
        let _ = fs::remove_file(&path);
    }
}

/// 현재 활성 토큰을 반환한다 (Tauri command 외 다른 모듈에서도 사용).
/// 첫 호출 시 옛 TOML 자동 마이그레이션. 저장값 우선, 없으면 임베드 기본값, 그것도 없으면 빈 문자열.
pub fn current_token() -> String {
    migrate_legacy_toml();
    if let Ok(Some(t)) = secrets::load_updater_token() {
        return t;
    }
    EMBEDDED_TOKEN.unwrap_or("").to_string()
}

#[tauri::command]
pub fn get_update_token() -> String {
    current_token()
}

/// 사용자 저장값 갱신. 빈 문자열을 넘기면 저장값을 지움 (= 임베드 기본값으로 fallback).
#[tauri::command]
pub fn set_update_token(token: String) -> Result<(), String> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        secrets::clear_updater_token()
    } else {
        secrets::save_updater_token(trimmed)
    }
}

/// 토큰 출처를 알려준다 (UI 표시용). "saved" / "embedded" / "none".
#[tauri::command]
pub fn update_token_source() -> &'static str {
    migrate_legacy_toml();
    if matches!(secrets::load_updater_token(), Ok(Some(_))) {
        "saved"
    } else if EMBEDDED_TOKEN.is_some_and(|t| !t.is_empty()) {
        "embedded"
    } else {
        "none"
    }
}

/// 사용자 입력 토큰을 저장 전에 ping 으로 검증.
/// `/updates/latest.json` 으로 200 응답이면 OK, 401 이면 Err.
/// OOBE/Settings 에서 잘못된 토큰 저장을 막기 위해 사용.
#[tauri::command]
pub async fn validate_update_token(token: String) -> Result<(), String> {
    let url = "https://pengdoll.duckdns.org/updates/latest.json";
    let token = token.trim().to_string();
    if token.is_empty() {
        return Err("토큰이 비어있습니다".to_string());
    }
    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        let agent = ureq::Agent::config_builder()
            .timeout_global(Some(std::time::Duration::from_secs(8)))
            .build()
            .new_agent();
        let resp = agent
            .get(url)
            .header("Authorization", format!("Bearer {token}"))
            .call()
            .map_err(|e| format!("네트워크 오류: {e}"))?;
        let status = resp.status();
        if status == 401 {
            return Err("토큰이 인증되지 않았습니다 (401). 관리자에게 받은 값을 확인하세요.".to_string());
        }
        if !status.is_success() {
            return Err(format!("서버 응답 이상 (HTTP {status})"));
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("blocking join: {e}"))?
}
