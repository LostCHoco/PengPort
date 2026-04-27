//! OS native secret store wrapper (keyring crate).
//!
//! Windows Credential Manager / macOS Keychain / Linux Secret Service 사용.
//! user session 단위로 보호되며 master password 불필요.
//!
//! 저장하는 시크릿 (account 이름):
//! - `instance_token`: PSP instance 의 catalog/SSE bearer 토큰 (사용자별 instance 접근)
//! - `updater_token`:  Caddy `/updates/*` Bearer 토큰 (auto-update endpoint 보호)

use keyring::Entry;

/// keyring service 이름. 모든 PengPort 시크릿이 이 service 아래로 묶임.
/// tauri.conf.json 의 identifier 와 일치.
const SERVICE: &str = "app.pengport";

fn entry(account: &str) -> Result<Entry, String> {
    Entry::new(SERVICE, account).map_err(|e| format!("keyring entry: {e}"))
}

fn save(account: &str, value: &str) -> Result<(), String> {
    let entry = entry(account)?;
    entry
        .set_password(value)
        .map_err(|e| format!("keyring save: {e}"))
}

fn load(account: &str) -> Result<Option<String>, String> {
    let entry = entry(account)?;
    match entry.get_password() {
        Ok(v) => Ok(Some(v)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("keyring load: {e}")),
    }
}

fn clear(account: &str) -> Result<(), String> {
    let entry = entry(account)?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("keyring clear: {e}")),
    }
}

// ----- 다른 module 에서 사용하는 typed API -----

pub fn load_updater_token() -> Result<Option<String>, String> {
    load("updater_token")
}

pub fn save_updater_token(token: &str) -> Result<(), String> {
    save("updater_token", token)
}

pub fn clear_updater_token() -> Result<(), String> {
    clear("updater_token")
}

// ----- Tauri commands (frontend 가 직접 호출) -----

/// PSP instance 의 bearer 토큰을 keyring 에 저장.
/// 빈 문자열이면 clear 동작.
#[tauri::command]
pub fn instance_token_save(token: String) -> Result<(), String> {
    let trimmed = token.trim();
    if trimmed.is_empty() {
        clear("instance_token")
    } else {
        save("instance_token", trimmed)
    }
}

/// keyring 에서 instance 토큰 조회 (없으면 null).
#[tauri::command]
pub fn instance_token_load() -> Result<Option<String>, String> {
    load("instance_token")
}

/// keyring 에서 instance 토큰 삭제 (logout 등).
#[tauri::command]
pub fn instance_token_clear() -> Result<(), String> {
    clear("instance_token")
}
