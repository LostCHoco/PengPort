//! 데이터 정리/삭제/언인스톨 — 사용자가 명시적으로 호출하는 위험 작업.
//!
//! 세 가지 명령:
//! - `wipe_all_data`        — PengPort 가 만든 모든 state 초기화 (프로그램 자체는 유지)
//! - `remove_prism_instance` — Prism 의 인스턴스 폴더 1개 삭제
//! - `uninstall_self`       — Windows NSIS uninstaller 호출 후 자체 종료
//!
//! 호출 직전에 frontend 에서 confirm dialog 로 사용자 동의 필수. Rust 측은 검증 없음.

use std::path::PathBuf;

use serde::Serialize;
use tauri::AppHandle;

use super::{paths, prism};

// ---------------------------------------------------------------------------
// remove_prism_instance — Prism 의 instances/<id>/ 폴더 1개 삭제
// ---------------------------------------------------------------------------

/// PengPort 가 spawn 한 Prism 인스턴스의 폴더를 통째로 삭제한다.
///
/// `instance_id` 는 PSP service id (= Prism instance dir name). 이 폴더 안에는 Minecraft
/// 의 saves/ 등 사용자 데이터도 있으므로 호출자가 frontend 에서 명시적 confirm 후 호출할 것.
///
/// 보안: instance_id 는 외부 (catalog) controlled 이므로 validate_service_id 통과 강제.
/// 미통과 시 즉시 거부 — path traversal 로 임의 폴더 삭제 차단.
///
/// Prism 본체나 다른 instance 폴더는 건드리지 않음. Bundled vs system Prism 구분 없이
/// 현재 active prism_paths 의 instances/ 아래에서 동작.
#[tauri::command]
pub async fn remove_prism_instance(instance_id: String) -> Result<(), String> {
    pengport_shared::validate_service_id(&instance_id)
        .map_err(|e| format!("instance_id 형식 오류 ({instance_id:?}): {e}"))?;

    let (_, prism_paths) = prism::prism_paths()?;
    let dir = prism_paths.instance_dir(&instance_id);
    if !dir.exists() {
        return Ok(());
    }
    tauri::async_runtime::spawn_blocking(move || {
        std::fs::remove_dir_all(&dir).map_err(|e| {
            format!(
                "Prism 인스턴스 폴더 삭제 실패 ({}): {e}",
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

/// `wipe_all_data` 의 입력 — frontend 가 가진 정보 (instance ids, prism instance ids).
/// keyring 은 enumerate API 가 없어 frontend 가 instance id 목록을 넘겨줘야 한다.
/// localStorage 도 frontend 가 따로 비워야 한다 (이 함수는 native state 만 담당).
#[derive(Debug, serde::Deserialize)]
pub struct WipeRequest {
    /// keyring 의 `instance_token:<id>` entry 들을 정리할 대상.
    pub instance_ids: Vec<String>,
    /// Prism 의 instances/<id>/ 폴더들을 삭제할 대상 (PengPort 가 만든 것만).
    pub prism_instance_ids: Vec<String>,
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
/// - `%APPDATA%/app.pengport/prism_settings.toml`
/// - `%LOCALAPPDATA%/app.pengport/prism/` (bundled PrismLauncher)
/// - `%LOCALAPPDATA%/app.pengport/packwiz-installer-bootstrap.jar`
/// - 호출자가 지정한 Prism instance 폴더들 (PengPort 가 만든 것만)
///
/// 영향 범위 **밖**:
/// - 앱 자체 (실행 파일 + tauri.conf 의 OS 표준 위치 잔재)
/// - 시스템 Prism 의 다른 인스턴스 (PengPort 가 안 만든 것)
/// - localStorage / IndexedDB (frontend 가 별도 정리)
#[tauri::command]
pub async fn wipe_all_data(req: WipeRequest) -> Result<WipeReport, String> {
    let mut report = WipeReport::default();

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

    // 2) Prism 인스턴스 폴더들 (PengPort 가 만든 것).
    //    Prism 자체가 시스템에 없으면 지울 인스턴스 폴더도 없으므로 silent skip — failures 에
    //    추가하면 사용자에게 "실패" 로 보여 혼란.
    //
    //    보안: 각 id 는 path traversal 차단 위해 validate_service_id 통과 강제.
    //    미통과 id 는 failures 에 기록하고 skip.
    if !req.prism_instance_ids.is_empty() {
        if let Ok((_, prism_paths)) = prism::prism_paths() {
            for id in &req.prism_instance_ids {
                if let Err(e) = pengport_shared::validate_service_id(id) {
                    report
                        .failures
                        .push(format!("prism instance '{id}' (id 형식 오류): {e}"));
                    continue;
                }
                let dir = prism_paths.instance_dir(id);
                if dir.exists() {
                    if let Err(e) = std::fs::remove_dir_all(&dir) {
                        report
                            .failures
                            .push(format!("prism instance '{id}': {e}"));
                    } else {
                        report.paths_removed.push(dir);
                    }
                }
            }
        }
    }

    // 3) PengPort 의 모든 옛/현 식별자가 만든 사용자 폴더 통째로 삭제.
    //    %APPDATA%/<id>/ 와 %LOCALAPPDATA%/<id>/ 두 곳 × ALL_FS_IDENTIFIERS.
    //    포함되는 데이터: trust.json, prism_settings.toml, WebView2 EBWebView (localStorage
    //    /IndexedDB/Cookies), bundled prism, bootstrap jar 등 PengPort 가 만든 것 전부.
    let appdata = std::env::var_os("APPDATA");
    let localappdata = std::env::var_os("LOCALAPPDATA");
    for id in paths::ALL_FS_IDENTIFIERS {
        for root_var in [&appdata, &localappdata] {
            let Some(root) = root_var else { continue };
            let dir = PathBuf::from(root).join(id);
            if dir.exists() {
                match std::fs::remove_dir_all(&dir) {
                    Ok(()) => report.paths_removed.push(dir),
                    Err(e) => report
                        .failures
                        .push(format!("{}: {e}", dir.display())),
                }
            }
        }
    }

    Ok(report)
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
// uninstall_self — Windows NSIS uninstaller 실행 + 자체 종료
// ---------------------------------------------------------------------------

/// Windows: 레지스트리의 `Uninstall\<ProductName>` 에서 `UninstallString` 을 찾아 실행하고
/// 자체 종료한다. NSIS uninstaller 가 PengPort.exe 를 lock 한 상태이면 실패하므로 우리는
/// detached 로 spawn 후 즉시 종료.
///
/// 옛 productName ("PengdollPark") 의 uninstaller 도 등록되어 있으면 같이 spawn — 사용자가
/// "완전 삭제" 의도로 호출했으니 모든 흔적 정리.
///
/// 추가로: NSIS 는 install path (Program Files\PengPort) 만 정리하고 user data
/// (%LOCALAPPDATA%\PengPort\EBWebView 등) 는 보존하므로, background batch 로 ALL_FS_IDENTIFIERS
/// 의 user data 폴더들을 PengPort 종료 직후 자동 정리.
///
/// `silent`: true 면 NSIS uninstaller 의 `/S` flag (silent uninstall) 사용 — 사용자에게 추가
/// confirm dialog 없이 진행. ephemeral 모드 종료 시 자동 cleanup 흐름에 사용.
///
/// Windows 외 플랫폼에서는 미지원.
#[tauri::command]
pub async fn uninstall_self(app: AppHandle, silent: Option<bool>) -> Result<(), String> {
    let silent = silent.unwrap_or(false);
    #[cfg(windows)]
    {
        let mut spawned_any = false;
        for product in paths::ALL_PRODUCT_NAMES {
            if let Some(unins) = locate_uninstaller_windows(product) {
                if let Err(e) = spawn_uninstaller_windows(&unins, silent) {
                    eprintln!("uninstaller spawn 실패 ({}): {e}", product);
                } else {
                    spawned_any = true;
                }
            }
        }
        if !spawned_any {
            if silent {
                // silent 호출 (ephemeral 모드 종료 cleanup 등) — uninstaller 없어도 wipe + exit
                // 흐름 그대로 진행. dev 빌드 (NSIS 안 거침) 또는 portable 빌드에서도 정상 동작.
                eprintln!(
                    "uninstaller 위치 못 찾음 — silent 모드라 wipe/exit 만 진행 (NSIS 설치본 아닌 빌드)"
                );
            } else {
                return Err("uninstaller 위치를 찾을 수 없습니다 (이 빌드가 NSIS 설치본이 아닐 수 있음)".to_string());
            }
        }
        // user data 폴더들을 5초 후 background 정리 — PengPort 종료 + webview2 자식 process 의
        // lock 풀릴 시간 확보. NSIS 의 Program Files 정리와 별개.
        schedule_userdata_cleanup_windows();
        // 짧은 grace period 후 자체 종료.
        let app_for_exit = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            std::thread::sleep(std::time::Duration::from_millis(500));
            app_for_exit.exit(0);
        });
        Ok(())
    }

    #[cfg(not(windows))]
    {
        let _ = (app, silent);
        Err("Windows 외 OS 미지원".to_string())
    }
}

/// PengPort 종료 후 5초 뒤 user data 폴더들을 정리하는 background cmd batch.
/// detached + no-window 로 spawn 후 PengPort exit. webview2 자식 process 가 종료될 시간을
/// 주기 위해 timeout. 각 rd 는 best-effort (lock 잡힌 항목 있으면 skip).
#[cfg(windows)]
fn schedule_userdata_cleanup_windows() {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const DETACHED_PROCESS: u32 = 0x0000_0008;

    let appdata = std::env::var_os("APPDATA").unwrap_or_default();
    let localappdata = std::env::var_os("LOCALAPPDATA").unwrap_or_default();

    let mut parts: Vec<String> = vec!["timeout /t 5 /nobreak > nul".to_string()];
    for id in paths::ALL_FS_IDENTIFIERS {
        let p1 = PathBuf::from(&appdata).join(id);
        let p2 = PathBuf::from(&localappdata).join(id);
        parts.push(format!("rd /s /q \"{}\" 2>nul", p1.display()));
        parts.push(format!("rd /s /q \"{}\" 2>nul", p2.display()));
    }
    let cmd = parts.join(" & ");

    let _ = std::process::Command::new("cmd")
        .args(["/c", &cmd])
        .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
        .spawn();
}

#[cfg(windows)]
fn locate_uninstaller_windows(product: &str) -> Option<PathBuf> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
    use winreg::RegKey;

    let candidates = [
        (
            HKEY_CURRENT_USER,
            format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\{product}"),
        ),
        (
            HKEY_LOCAL_MACHINE,
            format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\{product}"),
        ),
        (
            HKEY_LOCAL_MACHINE,
            format!(r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\{product}"),
        ),
    ];

    for (hive, subkey) in candidates {
        let root = RegKey::predef(hive);
        let Ok(key) = root.open_subkey_with_flags(&subkey, KEY_READ) else {
            continue;
        };
        let Ok(s): Result<String, _> = key.get_value("UninstallString") else {
            continue;
        };
        // UninstallString 은 보통 따옴표 포함된 절대 경로. trim + 따옴표 제거.
        let trimmed = s.trim().trim_matches('"');
        let path = PathBuf::from(trimmed);
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

#[cfg(windows)]
fn spawn_uninstaller_windows(unins: &std::path::Path, silent: bool) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    // CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS — 부모(우리)가 죽어도 NSIS 가 살아있게.
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    let mut cmd = std::process::Command::new(unins);
    if silent {
        // NSIS 의 silent uninstall flag — 사용자 confirm dialog 없이 진행. ephemeral 모드 cleanup 용.
        cmd.arg("/S");
    }
    cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("uninstaller spawn 실패 ({}): {e}", unins.display()))
}
