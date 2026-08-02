//! 데이터 정리/삭제 — 사용자가 명시적으로 호출하는 위험 작업.
//!
//! 세 가지 명령:
//! - `wipe_all_data`                  — PengPort 가 만든 모든 state 초기화 (프로그램 자체는 유지)
//! - `remove_third_party_app_instance` — third-party app(예: Prism) 의 인스턴스 폴더 1개 삭제
//! - `uninstall_self`                 — exe + data 폴더 자체 삭제 후 종료. Portable 모델이라
//!   인스톨러가 없다 — 설정 화면의 수동 UI는 없고, kiosk(ephemeral) 모드 종료 자동
//!   cleanup 흐름 전용
//!
//! 호출 직전에 frontend 에서 confirm dialog 로 사용자 동의 필수. Rust 측은 검증 없음.

use std::path::{Path, PathBuf};

use serde::Serialize;
use tauri::AppHandle;

use super::{library, paths};

// ---------------------------------------------------------------------------
// remove_third_party_app_instance — third-party app 의 instances/<id>/ 폴더 1개 삭제
// ---------------------------------------------------------------------------

/// PengPort 가 spawn 한 third-party app 인스턴스의 폴더를 통째로 삭제한다.
///
/// `instance_id` 는 레시피 id(= 인스턴스 dir name). 이 폴더 안에는 세이브 등 사용자
/// 데이터도 있으므로 호출자가 frontend 에서 명시적 confirm 후 호출할 것.
///
/// 보안: `app_id`/`instance_id` 둘 다 외부(링크 임포트) controlled 이므로
/// `third_party_app_instance_dir`(내부적으로 `validate_service_id` 강제 — third_party_app.rs
/// 참고)를 통해서만 경로를 만든다 — path traversal 로 임의 폴더 삭제 차단.
///
/// 해당 third-party app 본체나 다른 instance 폴더는 건드리지 않음. Bundled vs system
/// 구분 없이 현재 활성 위치의 instances/ 아래에서 동작.
#[tauri::command]
pub async fn remove_third_party_app_instance(app_id: String, instance_id: String) -> Result<(), String> {
    let dir = library::third_party_app_instance_dir(&app_id, &instance_id)?;
    if !dir.exists() {
        return Ok(());
    }
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::remove_dir_all(&dir).map_err(|e| {
            format!(
                "{app_id} 인스턴스 폴더 삭제 실패 ({}): {e}",
                dir.display()
            )
        })
    })
    .await
    .map_err(|e| format!("blocking task panic: {e}"))?
}

// ---------------------------------------------------------------------------
// wipe_all_data — PengPort 가 만든 state 전부 초기화 (앱 자체는 유지)
// ---------------------------------------------------------------------------

/// `third_party_app_instances` 항목 하나 — 특정 third-party app(`app_id`)의 인스턴스
/// 폴더들(`instance_ids`) 을 삭제할 대상.
#[derive(Debug, serde::Deserialize)]
pub struct ThirdPartyAppInstanceWipeTarget {
    pub app_id: String,
    pub instance_ids: Vec<String>,
}

/// `wipe_all_data` 의 입력 — frontend 가 가진 정보 (instance ids, third-party app 인스턴스
/// ids). keyring 은 enumerate API 가 없어 frontend 가 instance id 목록을 넘겨줘야 한다.
/// localStorage 도 frontend 가 따로 비워야 한다 (이 함수는 native state 만 담당).
#[derive(Debug, serde::Deserialize)]
pub struct WipeRequest {
    /// keyring 의 `instance_token:<id>` entry 들을 정리할 대상.
    pub instance_ids: Vec<String>,
    /// third-party app 별 instances/<id>/ 폴더들을 삭제할 대상(PengPort 가 만든 것만).
    /// app_id 로 그룹화된 이유: 서로 다른 third-party app 이 각자의 데이터 루트 아래
    /// 자기 instances/ 를 가지므로, 삭제 전에 app_id 별로 한 번씩만 위치를 해석하면 된다.
    #[serde(default)]
    pub third_party_app_instances: Vec<ThirdPartyAppInstanceWipeTarget>,
}

#[derive(Debug, Default, Serialize)]
pub struct WipeReport {
    /// 삭제된 keyring entry 수.
    pub keyring_cleared: usize,
    /// 삭제된 파일/디렉토리 경로.
    pub paths_removed: Vec<PathBuf>,
    /// 삭제 시도했지만 실패한 항목 (메시지). best-effort — 일부 실패해도 다른 항목은 진행.
    pub failures: Vec<String>,
}

/// PengPort 가 만든 모든 native state 를 초기화한다.
/// 대상:
/// - keyring `app.pengport` service 의 `instance_token:*` entry 들
/// - `%APPDATA%/app.pengport/trust.json`
/// - `%APPDATA%/app.pengport/third_party_app_overrides.json` (Prism 등 override 경로)
/// - `%LOCALAPPDATA%/app.pengport/<app_id>/` (bundled third-party app, 예: Prism)
/// - 호출자가 지정한 third-party app 인스턴스 폴더들 (PengPort 가 만든 것만)
///
/// 영향 범위 **밖**:
/// - 앱 자체 (실행 파일 + tauri.conf 의 OS 표준 위치 잔재)
/// - 시스템에 이미 설치된 third-party app 의 다른 인스턴스 (PengPort 가 안 만든 것)
/// - localStorage / IndexedDB (frontend 가 별도 정리)
/// - **이 프로세스(PengPort) 자신이 지금 쓰고 있는 webview 프로필**(`EBWebView`) — "초기화"는
///   PengPort 를 종료하지 않는 흐름이라 그 폴더는 항상 잠겨 있어 못 지운다("PengPort
///   삭제"는 실제로 종료한 뒤 지우므로 이 제약이 없다 — `uninstall_self` 참고).
#[tauri::command]
pub async fn wipe_all_data(req: WipeRequest) -> Result<WipeReport, String> {
    tauri::async_runtime::spawn_blocking(move || -> Result<WipeReport, String> {
        let mut report = WipeReport::default();

        // 0) 추적 중인 실행 프로세스(프리즘/그 자식 마인크래프트 등) 강제 종료 — 실사용
        //    중 발견된 버그: 이걸 안 하면 그 프로세스가 잠근 파일(예: prism_launcher\
        //    accounts.json, 로드된 DLL) 때문에 아래 3)의 삭제가 실패해서 서드파티 앱
        //    사본이 그대로 남았다.
        super::third_party_runtime::kill_all_running_blocking();

        // 1) keyring 정리.
        for id in &req.instance_ids {
            let account = format!("instance_token:{id}");
            match keyring_clear(&account) {
                Ok(true) => report.keyring_cleared += 1,
                Ok(false) => {} // 이미 없음
                Err(e) => report
                    .failures
                    .push(format!("keyring '{account}': {e}")),
            }
        }

        // 2) third-party app 인스턴스 폴더들 (PengPort 가 만든 것) — app_id 별로 위치 해석.
        //    해당 app 이 시스템에 없으면 지울 인스턴스 폴더도 없으므로 silent skip — failures 에
        //    추가하면 사용자에게 "실패" 로 보여 혼란.
        //
        //    보안: 각 id 는 path traversal 차단 위해 validate_service_id 통과 강제
        //    (`third_party_app_instance_dir` 내부에서도 재검증). 미통과 id 는 failures 에
        //    기록하고 skip.
        for target in &req.third_party_app_instances {
            for id in &target.instance_ids {
                if let Err(e) = pengport_shared::validate_service_id(id) {
                    report
                        .failures
                        .push(format!("{} instance '{id}' (id 형식 오류): {e}", target.app_id));
                    continue;
                }
                let Ok(dir) = library::third_party_app_instance_dir(&target.app_id, id) else {
                    continue;
                };
                match remove_path_with_retries(&dir, 5) {
                    Ok(()) => report.paths_removed.push(dir),
                    Err(e) => report
                        .failures
                        .push(format!("{} instance '{id}': {e}", target.app_id)),
                }
            }
        }

        // 3) PengPort 의 모든 옛/현 식별자가 만든 사용자 폴더 정리.
        //    %APPDATA%/<id>/ 와 %LOCALAPPDATA%/<id>/ 두 곳 × ALL_FS_IDENTIFIERS — 폴더
        //    자체를 통째로 `remove_dir_all` 하지 않고 **하위 항목을 하나씩** 지운다.
        //    이유(실사용 중 발견): 이 초기화는 PengPort 자신을 종료하지 않으므로, 지금
        //    이 프로세스의 webview 가 쓰고 있는 `EBWebView` 하위 폴더는 항상 잠겨 있어
        //    못 지운다. 통째로 지우려 하면 그 하나 때문에 전체가 실패해서, 진짜 지워야
        //    할 `prism_launcher` 같은 형제 폴더까지 같이 안 지워졌었다 — 개별 삭제로
        //    바꾸면 `EBWebView`만 실패로 남고 나머지는 정상 삭제된다.
        let appdata = std::env::var_os("APPDATA");
        let localappdata = std::env::var_os("LOCALAPPDATA");
        for id in paths::ALL_FS_IDENTIFIERS {
            for root_var in [&appdata, &localappdata] {
                let Some(root) = root_var else { continue };
                let dir = PathBuf::from(root).join(id);
                let Ok(entries) = std::fs::read_dir(&dir) else {
                    continue;
                };
                for entry in entries.flatten() {
                    let path = entry.path();
                    match remove_path_with_retries(&path, 5) {
                        Ok(()) => report.paths_removed.push(path),
                        Err(e) => report.failures.push(format!("{}: {e}", path.display())),
                    }
                }
                // 하위 항목이 전부 지워졌으면(EBWebView 처럼 잠긴 게 없었으면) 이제 빈
                // 폴더가 된 dir 자체도 정리 — 실패해도(아직 뭔가 남아있으면) 조용히 무시,
                // 위에서 이미 그 원인은 failures 에 기록됨.
                let _ = std::fs::remove_dir(&dir);
            }
        }

        Ok(report)
    })
    .await
    .map_err(|e| format!("blocking task panic: {e}"))?
}

/// 잠깐 잠겨 있을 수 있는 파일/폴더 삭제를 짧은 간격으로 재시도한다 — 방금 강제
/// 종료한 프로세스가 파일 핸들을 실제로 놓기까지 살짝 지연이 있을 수 있어서, 얼마나
/// 기다려야 할지 미리 알 수 없는 값을 고정 시간으로 추측하는 대신 "안 되면 곧 다시
/// 시도"가 더 정확하다(환경마다 필요한 시간이 다름 — 느린 디스크, 백신 스캔 등).
/// `max_attempts`로 무한 재시도는 방지 — 계속 잠겨있는 항목(예: 지금 실행 중인
/// PengPort 자신의 webview 프로필)은 그 한도만큼만 시도하고 실패로 보고한다.
fn remove_path_with_retries(path: &Path, max_attempts: u32) -> std::io::Result<()> {
    if !path.exists() {
        return Ok(());
    }
    let mut last_err = None;
    for attempt in 0..max_attempts.max(1) {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        let result = if path.is_dir() {
            std::fs::remove_dir_all(path)
        } else {
            std::fs::remove_file(path)
        };
        match result {
            Ok(()) => return Ok(()),
            Err(_) if !path.exists() => return Ok(()),
            Err(e) => last_err = Some(e),
        }
    }
    Err(last_err.expect("max_attempts.max(1) >= 1 이므로 최소 한 번은 시도해 last_err 가 채워짐"))
}

/// keyring entry 1개 정리. Ok(true)=삭제됨, Ok(false)=원래 없음.
fn keyring_clear(account: &str) -> Result<bool, String> {
    use keyring::Entry;
    let entry = Entry::new("app.pengport", account)
        .map_err(|e| format!("entry: {e}"))?;
    match entry.delete_credential() {
        Ok(()) => Ok(true),
        Err(keyring::Error::NoEntry) => Ok(false),
        Err(e) => Err(format!("delete: {e}")),
    }
}

// ---------------------------------------------------------------------------
// uninstall_self — exe + data 폴더 자체 삭제 후 종료 (portable, 인스톨러 없음)
// ---------------------------------------------------------------------------

/// exe 파일과 `data/` 폴더를 지우고 자체 종료한다. Portable 모델이라 "삭제"엔 인스톨러가
/// 관여하지 않는다 — PengPort 가 차지한 흔적은 이 둘뿐이다.
///
/// kiosk(ephemeral) 모드 종료 자동 cleanup 전용 호출부(`App.tsx`) — 설정 화면엔 대응하는
/// 수동 UI가 없다(사용자가 폴더를 직접 지우는 게 더 간단하고 확실해서 만들지 않음).
/// 호출 전에 `wipe_all_data`가 이미 override 경로까지 인식하는 정리를 최대한 마쳤다고
/// 가정 — 여기선 그게 못 지운 나머지(이 프로세스 자신이 지금 잠그고 있는 exe 파일과
/// `EBWebView` 등)를 프로세스 종료 후 마저 정리한다.
///
/// Windows 외 플랫폼에서는 미지원.
#[tauri::command]
pub async fn uninstall_self(app: AppHandle) -> Result<(), String> {
    #[cfg(windows)]
    {
        // 삭제 전에 실행 중인 프로세스(프리즘 등) 강제 종료 — `wipe_all_data`와 같은 이유
        // (그게 잠근 파일 때문에 아래 백그라운드 정리 스크립트의 삭제가 실패할 수 있음).
        tauri::async_runtime::spawn_blocking(super::third_party_runtime::kill_all_running_blocking)
            .await
            .map_err(|e| format!("blocking task panic: {e}"))?;

        schedule_self_removal_windows()?;

        // 짧은 grace period 후 자체 종료 — 그 순간부터 exe 파일 잠금이 풀리고,
        // 위에서 예약한 스크립트가 재시도 끝에 지운다.
        let app_for_exit = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            std::thread::sleep(std::time::Duration::from_millis(500));
            app_for_exit.exit(0);
        });
        Ok(())
    }

    #[cfg(not(windows))]
    {
        let _ = app;
        Err("Windows 외 OS 미지원".to_string())
    }
}

/// PengPort 종료 후 exe 파일과 `data/` 폴더를 지우는 background .bat 스크립트를 생성해
/// detached + no-window 로 실행한다.
///
/// **재시도 방식(고정 대기 아님)**: 이 프로세스와 webview2 자식 process 들이 종료 후
/// 실제로 파일 핸들을 놓기까지 걸리는 시간은 환경(디스크 속도, 백신 스캔 등)마다
/// 달라서, "몇 초 기다렸다가 딱 한 번 시도"는 부족하면 조용히 실패하고 충분하면 괜히
/// 느리다. 대신 경로마다 "삭제 시도 → 실패하면 1초 후 재시도"를 **최대 15번**(무한
/// 재시도 방지) 반복한다. 스크립트는 끝나면 자기 자신을 지운다.
#[cfg(windows)]
fn schedule_self_removal_windows() -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const DETACHED_PROCESS: u32 = 0x0000_0008;

    let exe = std::env::current_exe().map_err(|e| format!("exe 경로 확인 실패: {e}"))?;
    let data_dir =
        paths::app_data_root().ok_or_else(|| "data 경로를 확인할 수 없습니다".to_string())?;

    let mut script = String::from("@echo off\r\n");
    script.push_str(&retry_delete_block("del /f /q", &exe, "exe_done"));
    script.push_str(&retry_delete_block("rd /s /q", &data_dir, "data_done"));
    script.push_str("del \"%~f0\"\r\n");

    let script_path =
        std::env::temp_dir().join(format!("pengport-cleanup-{}.bat", std::process::id()));
    super::write_windows_batch_script(&script_path, &script)
        .map_err(|e| format!("정리 스크립트 생성 실패: {e}"))?;

    std::process::Command::new("cmd")
        .args(["/c", &script_path.to_string_lossy()])
        .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("정리 스크립트 실행 실패: {e}"))
}

/// `delete_cmd`(`del /f /q` 또는 `rd /s /q`)로 `path`를 최대 15 회 재시도 삭제하는
/// 배치 스크립트 블록 — 성공하면 `label`로 점프해 다음 블록으로 넘어간다.
#[cfg(windows)]
fn retry_delete_block(delete_cmd: &str, path: &Path, label: &str) -> String {
    format!(
        "for /l %%i in (1,1,15) do (\r\n  {delete_cmd} \"{p}\" 2>nul\r\n  if not exist \"{p}\" goto :{label}\r\n  timeout /t 1 /nobreak > nul\r\n)\r\n:{label}\r\n",
        p = path.display(),
    )
}
