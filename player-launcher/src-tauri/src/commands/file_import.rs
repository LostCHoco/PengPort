//! `.pengz` 파일 기반 레시피 번들 내보내기/가져오기 — 딥링크(`library_export_link`/
//! `library_preview_import`/`library_confirm_import`, `library.rs`)의 파일 버전.
//!
//! 딥링크는 URL을 OS 프로토콜 핸들러가 `CreateProcess`의 커맨드라인 인자로 넘기는데,
//! Windows 의 커맨드라인 길이 한도(약 32KB)에 걸릴 수 있다(레시피를 여럿 묶은 번들일수록
//! 위험). 파일로 내보내면 그 한도 자체가 없어진다 — 커맨드라인엔 파일 경로(짧음)만
//! 실리고, 실제 데이터는 파일 안에 있다. 미리보기/확인/반영 로직(`import::preview_file`/
//! `commit_file`)은 딥링크와 완전히 같은 스토어 반영 함수를 공유한다.
//!
//! OS 통합(`.pengz` 확장자를 PengPort 로 연결)은 `commands::paths::
//! register_pengz_file_association`. 더블클릭 시 argv 로 파일 경로가 넘어오는 걸
//! 감지하는 부분([`find_pengz_arg`], cold/hot start 처리)은 `lib.rs`.

use std::sync::Mutex;

use pengport_shared::library::import;

use super::library::{load_library_store, load_third_party_app_store, known_third_party_apps};

/// 더블클릭으로 콜드 스타트(=이 process 자체가 argv 로 `.pengz` 경로를 받으며 새로
/// 뜬 경우) 됐을 때, 프론트엔드가 mount 되기 전에 이벤트를 emit 하면 못 받을 수 있는
/// 레이스를 피하기 위한 상태 — 프론트엔드가 mount 직후 [`take_pending_pengz_file`]로
/// 1회 조회한다(핫 스타트는 `lib.rs`의 single_instance 콜백에서 이벤트로 직접 emit —
/// 그 시점엔 프론트엔드가 이미 떠 있어 레이스가 없음).
pub struct PendingPengzFile(pub Mutex<Option<String>>);

/// argv 목록에서 `.pengz` 로 끝나는 경로를 찾는다(대소문자 무관 — Windows 는 확장자
/// 대소문자를 구분하지 않음). 첫 항목(exe 자기 경로)은 자연히 안 걸림.
pub fn find_pengz_arg(argv: &[String]) -> Option<String> {
    let suffix = format!(".{}", pengport_shared::library::FILE_EXTENSION);
    argv.iter()
        .find(|a| a.to_lowercase().ends_with(&suffix))
        .cloned()
}

/// 콜드 스타트 시 잡아둔 `.pengz` 경로를 프론트엔드가 mount 직후 1회 조회 — 있으면
/// 소비(다음 호출은 `None`).
#[tauri::command]
pub fn take_pending_pengz_file(state: tauri::State<PendingPengzFile>) -> Option<String> {
    state.0.lock().unwrap().take()
}

/// 라이브러리 항목들을 `.pengz` 파일로 내보내기 — `save_path`는 프론트엔드가
/// `@tauri-apps/plugin-dialog`의 저장 다이얼로그로 미리 받아온 경로(레시피
/// 편집기의 "폴더 불러오기" 등과 같은 컨벤션 — OS 다이얼로그는 프론트엔드,
/// 실제 파일 I/O 는 Rust).
#[tauri::command]
pub async fn library_export_file(ids: Vec<String>, save_path: String) -> Result<(), String> {
    if ids.is_empty() {
        return Err("내보낼 항목이 없음".to_string());
    }
    let store = load_library_store().await?;
    let recipes: Vec<pengport_shared::library::Recipe> = ids
        .iter()
        .filter_map(|id| store.get(id).map(|e| e.recipe.clone()))
        .collect();
    if recipes.len() != ids.len() {
        return Err("일부 id 가 라이브러리에 없음".to_string());
    }
    let third_party_apps = known_third_party_apps()?;
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = pengport_shared::library::encode_bundle_file(&recipes, &third_party_apps)
            .map_err(|e| e.to_string())?;
        std::fs::write(&save_path, bytes).map_err(|e| format!("파일 쓰기 실패: {e}"))
    })
    .await
    .map_err(|e| format!("blocking task panic: {e}"))?
}

/// `.pengz` 파일 경로를 미리보기 — 스토어를 바꾸지 않는다. `library_preview_import`
/// (딥링크 버전)의 파일 버전 — confirm 다이얼로그는 두 경로 다 같은 `ImportPreview`
/// 모양을 그대로 렌더링.
#[tauri::command]
pub async fn library_preview_import_file(
    path: String,
) -> Result<pengport_shared::library::ImportPreview, String> {
    let store = load_library_store().await?;
    let tp_store = load_third_party_app_store().await?;
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = std::fs::read(&path).map_err(|e| format!("파일 읽기 실패: {e}"))?;
        import::preview_file(&bytes, &store, &tp_store).map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| format!("blocking task panic: {e}"))?
}

/// 사용자가 confirm 다이얼로그에서 확인한 뒤 호출 — `library_confirm_import`의 파일 버전.
#[tauri::command]
pub async fn library_confirm_import_file(path: String) -> Result<Vec<String>, String> {
    let mut store = load_library_store().await?;
    let mut tp_store = load_third_party_app_store().await?;
    tauri::async_runtime::spawn_blocking(move || {
        let bytes = std::fs::read(&path).map_err(|e| format!("파일 읽기 실패: {e}"))?;
        let ids = import::commit_file(&bytes, &mut store, &mut tp_store).map_err(|e| e.to_string())?;
        store.save().map_err(|e| e.to_string())?;
        tp_store.save().map_err(|e| e.to_string())?;
        Ok(ids)
    })
    .await
    .map_err(|e| format!("blocking task panic: {e}"))?
}
