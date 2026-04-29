//! OS native secret store wrapper (keyring crate).
//!
//! Windows Credential Manager / macOS Keychain / Linux Secret Service 사용.
//! user session 단위로 보호되며 master password 불필요.
//!
//! 저장하는 시크릿 (account 이름):
//! - `instance_token:<id>`: PSP instance 의 catalog/SSE bearer 토큰 (instance 별 격리)

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

// ----- Tauri commands (frontend 가 직접 호출) -----

/// instance 별로 keyring account 이름 분리. multi-instance 모델에서 각 instance 의 token 격리.
fn account_for_instance(instance_id: &str) -> String {
    format!("instance_token:{instance_id}")
}

/// 특정 instance 의 bearer 토큰을 keyring 에 저장. 빈 문자열이면 clear.
#[tauri::command]
pub fn instance_token_save(instance_id: String, token: String) -> Result<(), String> {
    let trimmed = token.trim();
    let account = account_for_instance(&instance_id);
    if trimmed.is_empty() {
        clear(&account)
    } else {
        save(&account, trimmed)
    }
}

/// 특정 instance 의 keyring 토큰 조회 (없으면 null).
#[tauri::command]
pub fn instance_token_load(instance_id: String) -> Result<Option<String>, String> {
    load(&account_for_instance(&instance_id))
}

/// 특정 instance 의 keyring 토큰 삭제 (logout, remove 등).
#[tauri::command]
pub fn instance_token_clear(instance_id: String) -> Result<(), String> {
    clear(&account_for_instance(&instance_id))
}

/// 단일 instance 모델 시절의 keyring entry ('instance_token') 를 새 instance_id 로 옮김.
/// PspLibrary 의 instance list 마이그레이션과 짝 — frontend 가 옛 'pengport.instance_url' 을
/// 새 instance entry 로 변환할 때 이 command 를 같이 호출.
/// 옛 entry 없으면 false. 있고 옮겨지면 true.
#[tauri::command]
pub fn instance_token_migrate_legacy(new_instance_id: String) -> Result<bool, String> {
    let legacy_account = "instance_token";
    match load(legacy_account)? {
        None => Ok(false),
        Some(token) => {
            let new_account = account_for_instance(&new_instance_id);
            save(&new_account, &token)?;
            clear(legacy_account)?;
            Ok(true)
        }
    }
}
