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
/// Prism 본체나 다른 instance 폴더는 건드리지 않음. Bundled vs system Prism 구분 없이
/// 현재 active prism_paths 의 instances/ 아래에서 동작.
#[tauri::command]
pub async fn remove_prism_instance(instance_id: String) -> Result<(), String> {
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
    if !req.prism_instance_ids.is_empty() {
        if let Ok((_, prism_paths)) = prism::prism_paths() {
            for id in &req.prism_instance_ids {
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
/// Windows 외 플랫폼에서는 미지원.
#[tauri::command]
pub async fn uninstall_self(app: AppHandle) -> Result<(), String> {
    #[cfg(windows)]
    {
        let mut spawned_any = false;
        for product in paths::ALL_PRODUCT_NAMES {
            if let Some(unins) = locate_uninstaller_windows(product) {
                if let Err(e) = spawn_uninstaller_windows(&unins) {
                    eprintln!("uninstaller spawn 실패 ({}): {e}", product);
                } else {
                    spawned_any = true;
                }
            }
        }
        if !spawned_any {
            return Err("uninstaller 위치를 찾을 수 없습니다 (이 빌드가 NSIS 설치본이 아닐 수 있음)".to_string());
        }
        // 짧은 grace period 후 자체 종료. NSIS 가 우리를 끝나기를 기다리지 않고 lock 충돌이 날 수 있어
        // 명시적 exit. 호출자(frontend)는 이 함수가 사실상 반환하지 않는다고 가정.
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
fn spawn_uninstaller_windows(unins: &std::path::Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    // CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS — 부모(우리)가 죽어도 NSIS 가 살아있게.
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    std::process::Command::new(unins)
        .creation_flags(CREATE_NEW_PROCESS_GROUP | DETACHED_PROCESS)
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("uninstaller spawn 실패 ({}): {e}", unins.display()))
}
