//! 자동 업데이트 토큰 관리.
//!
//! Caddy 의 `/updates/*` 엔드포인트는 Bearer 토큰을 요구한다.
//! 토큰 우선순위:
//!   1. 사용자가 Settings 에서 저장한 값 (`%APPDATA%/app.pengport/updater.toml`)
//!   2. 빌드 타임에 임베드된 기본값 (`PENGPORT_UPDATES_TOKEN` env var)
//!
//! 회전 시: 서버 토큰 변경 → 친구가 Settings 에 새 토큰 입력 → 끝.
//! 재설치 불필요. AppData 는 NSIS update 가 건드리지 않으므로 토큰도 보존됨.

use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::paths;

const EMBEDDED_TOKEN: Option<&str> = option_env!("PENGPORT_UPDATES_TOKEN");

#[derive(Debug, Default, Serialize, Deserialize)]
struct UpdaterSettings {
    token: Option<String>,
}

fn settings_path() -> Option<PathBuf> {
    paths::app_data_root().map(|d| d.join("updater.toml"))
}

fn load() -> UpdaterSettings {
    let Some(path) = settings_path() else {
        return Default::default();
    };
    let Ok(text) = fs::read_to_string(path) else {
        return Default::default();
    };
    toml::from_str(&text).unwrap_or_default()
}

fn save(settings: &UpdaterSettings) -> Result<(), String> {
    let path = settings_path().ok_or_else(|| "app_data_root 결정 실패".to_string())?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("디렉터리 생성 실패: {e}"))?;
    }
    let text = toml::to_string_pretty(settings).map_err(|e| e.to_string())?;
    fs::write(path, text).map_err(|e| format!("파일 쓰기 실패: {e}"))
}

/// 현재 활성 토큰을 반환한다 (Tauri command 외 다른 모듈에서도 사용).
/// 사용자 저장값 우선, 없으면 임베드된 기본값, 그것도 없으면 빈 문자열.
pub fn current_token() -> String {
    if let Some(t) = load().token {
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
    let mut s = load();
    let trimmed = token.trim();
    s.token = if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    };
    save(&s)
}

/// 토큰 출처를 알려준다 (UI 표시용). "saved" / "embedded" / "none".
#[tauri::command]
pub fn update_token_source() -> &'static str {
    if load().token.is_some() {
        "saved"
    } else if EMBEDDED_TOKEN.is_some_and(|t| !t.is_empty()) {
        "embedded"
    } else {
        "none"
    }
}

/// 사용자 입력 토큰을 저장 전에 ping 으로 검증.
/// `/meta/servers.toml` 으로 200 응답이면 OK, 401 이면 Err.
/// OOBE/Settings 에서 잘못된 토큰 저장을 막기 위해 사용.
#[tauri::command]
pub async fn validate_update_token(token: String) -> Result<(), String> {
    let url = "https://pengdoll.duckdns.org/meta/servers.toml";
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
