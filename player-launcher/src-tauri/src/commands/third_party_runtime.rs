//! third-party app(Prism 등) 실행/모니터링/자동 다운로드 — 전부 descriptor 데이터를
//! 소비할 뿐 어떤 앱인지 몰라도 되는 범용 코드. 옛 `commands::prism`이 Prism 이름으로
//! 하드코딩했던 마지막 부분(spawn/stop/download)이 여기로 옮겨오며 완전히 제네릭화됐다
//! — `commands::prism` 자체는 삭제됨. `docs/design/THIRD_PARTY_PLATFORM_MODEL.md` 참고.
//!
//! 위치 해석(override→bundled→시스템 탐지)과 준비 완료 감시(`ReadinessSignal`)는
//! `commands::library`(`resolve_known_third_party_app`/`watch_third_party_app_readiness`)
//! 가 이미 범용으로 갖고 있어 그대로 재사용한다 — 이 모듈이 새로 갖는 건 "실행 프로세스
//! 수명 추적"과 "다운로드 방법(`DownloadStrategy`) 해석" 둘뿐.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

use pengport_shared::actions::DownloadStrategy;
use serde::Serialize;
use tauri::{AppHandle, Emitter};

/// 실행 중인 third-party app 자식 process 의 PID 추적. recipe_id(=instance_id) → PID.
/// `stop_server` 가 taskkill /T 로 그 프로세스 + 자식(예: Prism 이 띄운 Minecraft) 까지 종료.
fn running_pids() -> &'static Mutex<HashMap<String, u32>> {
    static MAP: OnceLock<Mutex<HashMap<String, u32>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 해당 recipe id 의 third-party app 인스턴스가 현재 실행 중인지. `running_pids` 의
/// hashmap 조회. `ServiceCard`/`AppCard` 가 border 색 / Play→종료 버튼 토글에 사용.
#[tauri::command]
pub fn is_service_running(service_id: String) -> bool {
    running_pids()
        .lock()
        .map(|m| m.contains_key(&service_id))
        .unwrap_or(false)
}

/// 추적 중인 실행 프로세스 전부 강제 종료 — `commands::maintenance`의 "초기화"/
/// "PengPort 삭제"가 파일 삭제 전에 호출한다. 프리즘/그 자식(마인크래프트 등)이
/// 데이터 폴더의 파일을 잠그고 있으면 그 폴더 삭제가 실패하기 때문 — `stop_server`
/// (단일 id)와 같은 taskkill 메커니즘을 추적 중인 전부에 적용. blocking(각 프로세스마다
/// `taskkill` 서브프로세스를 동기 실행) — 호출자가 `spawn_blocking` 안에서 부를 것.
#[cfg(windows)]
pub(crate) fn kill_all_running_blocking() {
    let pids: Vec<u32> = running_pids()
        .lock()
        .map(|m| m.values().copied().collect())
        .unwrap_or_default();
    for pid in pids {
        let _ = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .status();
    }
}

#[cfg(not(windows))]
pub(crate) fn kill_all_running_blocking() {}

/// 실행 중인 third-party app 인스턴스를 강제 종료. Windows 의 `taskkill /T /F /PID` 로
/// process tree 전체 종료(예: Prism + 그 자식 Minecraft). 종료 후 `spawn_third_party_app`
/// 의 wait task 가 `server:stopped` event emit.
#[tauri::command]
pub async fn stop_server(server_id: String) -> Result<(), String> {
    let pid = running_pids().lock().unwrap().get(&server_id).copied();
    let Some(pid) = pid else {
        return Err(format!("'{server_id}' 가 실행 중이 아닙니다."));
    };

    tauri::async_runtime::spawn_blocking(move || -> Result<(), String> {
        #[cfg(windows)]
        {
            let status = Command::new("taskkill")
                .args(["/T", "/F", "/PID", &pid.to_string()])
                .status()
                .map_err(|e| format!("taskkill 실행 실패: {e}"))?;
            if !status.success() {
                return Err(format!("taskkill 종료 코드 {status}"));
            }
        }
        #[cfg(unix)]
        {
            // 추후 Linux/Mac 지원 시 SIGTERM → SIGKILL 패턴.
            let _ = pid;
            return Err("Windows 외 OS 미지원".to_string());
        }
        Ok(())
    })
    .await
    .map_err(|e| format!("blocking join: {e}"))?
}

/// third-party app 인스턴스를 띄우고 자식 process 수명을 추적한다.
///
/// `commands::library::library_launch` 가 `ThirdPartyAppLaunch{app_id}` 실행 시
/// install steps(레시피의 정적 콘텐츠 기록)를 마친 뒤 호출. 실행 인자는
/// descriptor 의 `launch_args_template`(예: Prism 은 `["--launch", "{instance_id}"]`)을
/// [`pengport_shared::actions::build_launch_args`]로 치환해서 만든다 — 이 함수는 그
/// 문자열이 어떤 의미인지 몰라도 된다.
///
/// Event 흐름:
/// - `server:started`             — spawn 완료 (= "준비 중" 시작)
/// - `third_party_app:child_ready` — descriptor 의 `readiness_signal`이 판별되면 emit
///   (`commands::library::watch_third_party_app_readiness`, 범용).
/// - `server:stopped`             — process 종료
pub(super) fn spawn_third_party_app(
    app: &AppHandle,
    app_id: &str,
    instance_id: &str,
) -> Result<(), String> {
    let descriptor = super::library::find_third_party_descriptor(app_id)?
        .ok_or_else(|| format!("알 수 없는 third-party app: {app_id}"))?;
    let resolved = super::library::resolve_known_third_party_app(&descriptor)?;

    if let Some(subfolder) = &descriptor.instances_subfolder {
        let instances = resolved.data_root.join(subfolder);
        std::fs::create_dir_all(&instances)
            .map_err(|e| format!("instances 폴더 생성 실패 ({}): {e}", instances.display()))?;
    }

    let args = pengport_shared::actions::build_launch_args(&descriptor.launch_args_template, instance_id);
    let mut child = Command::new(&resolved.exe)
        .args(&args)
        .spawn()
        .map_err(|e| format!("{app_id} 실행 실패: {e}"))?;

    let pid = child.id();
    running_pids()
        .lock()
        .unwrap()
        .insert(instance_id.to_string(), pid);

    let _ = app.emit(
        "server:started",
        serde_json::json!({ "serverId": instance_id }),
    );

    if let Some(signal) = descriptor.readiness_signal {
        let id_for_watch = instance_id.to_string();
        let still_running = move || {
            running_pids()
                .lock()
                .map(|m| m.contains_key(&id_for_watch))
                .unwrap_or(false)
        };
        super::library::watch_third_party_app_readiness(
            pid,
            instance_id.to_string(),
            signal,
            still_running,
            app.clone(),
        );
    }

    let app_for_wait = app.clone();
    let id_for_wait = instance_id.to_string();
    tauri::async_runtime::spawn_blocking(move || {
        let _ = child.wait();
        running_pids().lock().unwrap().remove(&id_for_wait);
        let _ = app_for_wait.emit(
            "server:stopped",
            serde_json::json!({ "serverId": id_for_wait }),
        );
    });

    Ok(())
}

// ---------------------------------------------------------------------------
// 자동 다운로드(OOBE) — descriptor 의 `download_strategy` 를 해석. `DownloadStrategy`
// 자체가 이미 앱 무관 데이터라, 여기도 앱별 분기가 없다(옛 `download_prism`이
// PrismLauncher/PrismLauncher repo + `Windows-MSVC-Portable` 패턴을 하드코딩했던 것과
// 달리, 그 값들은 이제 descriptor 데이터로 들어온다).
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct GhRelease {
    tag_name: String,
    assets: Vec<GhAsset>,
}

#[derive(serde::Deserialize)]
struct GhAsset {
    name: String,
    browser_download_url: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ThirdPartyAppDownloadResult {
    /// GitHub release 태그(예: `"9.6"`) — `StaticUrl` 전략은 release 개념이 없어 `None`.
    pub version: Option<String>,
    pub install_dir: PathBuf,
}

/// descriptor 의 `download_strategy` 로 전용 사본(bundled root, `%LOCALAPPDATA%\PengPort\
/// <app_id>\`)을 받아 설치한다. 기존 폴더는 삭제 후 새로 풀기(재시도 시 재다운로드 =
/// 깨끗한 상태 보장, 옛 `download_prism`과 동일 정책).
#[tauri::command]
pub async fn download_third_party_app(app_id: String) -> Result<ThirdPartyAppDownloadResult, String> {
    let descriptor = super::library::find_third_party_descriptor(&app_id)?
        .ok_or_else(|| format!("알 수 없는 third-party app: {app_id}"))?;
    let strategy = descriptor
        .download_strategy
        .clone()
        .ok_or_else(|| format!("{app_id}: 자동 다운로드 미지원"))?;
    let dest = super::paths::bundled_third_party_root(&app_id)
        .ok_or_else(|| "캐시 루트를 결정할 수 없음 (%LOCALAPPDATA% 미정?)".to_string())?;
    let exe_filename = descriptor.exe_filename.clone();
    let marker_files = descriptor.post_download_marker_files.clone();

    tauri::async_runtime::spawn_blocking(
        move || -> Result<ThirdPartyAppDownloadResult, String> {
            let version = match strategy {
                DownloadStrategy::GithubLatestRelease {
                    repo,
                    asset_name_pattern,
                } => {
                    let (tag, bytes) =
                        fetch_github_latest_release_zip(&repo, &asset_name_pattern)?;
                    place_downloaded_zip(
                        std::io::Cursor::new(&bytes),
                        &dest,
                        &exe_filename,
                        &marker_files,
                    )?;
                    Some(tag)
                }
                DownloadStrategy::StaticUrl { url, verification } => {
                    let tmp_dir = super::paths::app_cache_root()
                        .map(|d| d.join(format!("{app_id}.pengport-download-tmp")))
                        .ok_or_else(|| "캐시 루트를 결정할 수 없음 (%LOCALAPPDATA% 미정?)".to_string())?;
                    // 이전 다운로드 시도가 중간에 죽었을 때 남을 수 있는 잔재 정리 — 이
                    // 폴더는 이 앱 전용 스크래치 공간이라 통째로 지워도 안전(레시피
                    // 아카이브 다운로드의 `archive_tmp_dir` 정리 패턴과 동일).
                    let _ = std::fs::remove_dir_all(&tmp_dir);
                    // 레시피 압축과 달리 여기엔 "브라우저로 열어서 폴백" 개념이 없다(사람이
                    // 지켜보고 있다가 다운로드 폴더에서 파일을 찾아줄 앱 카드 UI 흐름이 아님)
                    // — 직접 다운로드가 깨끗하게 성공하지 않으면(에러 상태든 인터랙티브
                    // 페이지든) 그냥 명확한 에러로 알린다.
                    let tmp_path = match super::library::download_and_verify_to_file(
                        &url,
                        &verification,
                        &app_id,
                        std::time::Duration::from_secs(300),
                        &tmp_dir,
                        None, // third-party app 자동 다운로드는 취소 개념이 없음(레시피 설치 전용).
                        |_, _| {},
                    )? {
                        super::library::DownloadOutcome::Downloaded(path) => path,
                        super::library::DownloadOutcome::InteractivePage => {
                            return Err(format!(
                                "{app_id} 다운로드 실패: 직접 받아지지 않는 링크입니다(에러 응답이거나 사람이 눌러야 하는 페이지) — 다운로드 URL을 확인해주세요."
                            ));
                        }
                    };
                    let _guard = super::library::TempFileGuard(tmp_path.clone());
                    let file = std::fs::File::open(&tmp_path)
                        .map_err(|e| format!("임시 파일 열기 실패 ({}): {e}", tmp_path.display()))?;
                    place_downloaded_zip(file, &dest, &exe_filename, &marker_files)?;
                    None
                }
            };

            Ok(ThirdPartyAppDownloadResult {
                version,
                install_dir: dest,
            })
        },
    )
    .await
    .map_err(|e| format!("blocking task 실패: {e}"))?
}

/// GitHub 저장소(`"owner/repo"`)의 최신 release 에서 `.zip` + `asset_name_pattern`
/// 부분 문자열을 만족하는 자산 하나를 통째로 메모리에 받는다. 반환값 = (release 태그,
/// 압축 바이트). 런처류 자산은 실측 20~30MB 안팎이라 메모리 버퍼링 부담이 없음(옛
/// `download_prism`이 이미 이렇게 동작했음, 회귀 아님).
fn fetch_github_latest_release_zip(
    repo: &str,
    asset_name_pattern: &str,
) -> Result<(String, Vec<u8>), String> {
    let api_url = format!("https://api.github.com/repos/{repo}/releases/latest");
    let release: GhRelease = ureq::get(&api_url)
        .header("User-Agent", "PengPort")
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("GitHub API 호출 실패 ({repo}): {e}"))?
        .body_mut()
        .read_json()
        .map_err(|e| format!("release JSON 파싱 실패: {e}"))?;

    let asset = release
        .assets
        .iter()
        .find(|a| a.name.contains(asset_name_pattern) && a.name.ends_with(".zip"))
        .ok_or_else(|| {
            format!(
                "{} release 에 '{asset_name_pattern}*.zip' asset 이 없습니다",
                release.tag_name
            )
        })?;

    let mut buf = Vec::with_capacity(40 * 1024 * 1024);
    ureq::get(&asset.browser_download_url)
        .header("User-Agent", "PengPort")
        .call()
        .map_err(|e| format!("다운로드 실패: {e}"))?
        .body_mut()
        .as_reader()
        .read_to_end(&mut buf)
        .map_err(|e| format!("read_to_end: {e}"))?;

    Ok((release.tag_name, buf))
}

/// zip reader 를 `dest` 에 통째로 풀고(기존 내용은 삭제 후 재생성), `marker_files`를 빈
/// 내용으로 만든 뒤 `exe_filename`이 실제로 나왔는지 확인한다. 레시피 아카이브
/// (`extract_archive_file`)와 달리 화이트리스트 정리를 하지 않는다 — third-party app
/// 자체를 통째로 신뢰하는 영역이라 압축 안의 모든 파일이 유효하다고 본다.
fn place_downloaded_zip<R: std::io::Read + std::io::Seek>(
    reader: R,
    dest: &Path,
    exe_filename: &str,
    marker_files: &[String],
) -> Result<(), String> {
    if dest.exists() {
        std::fs::remove_dir_all(dest).map_err(|e| format!("기존 폴더 정리 실패: {e}"))?;
    }
    std::fs::create_dir_all(dest).map_err(|e| format!("대상 폴더 생성 실패: {e}"))?;

    let mut archive = zip::ZipArchive::new(reader).map_err(|e| format!("zip 열기 실패: {e}"))?;
    archive
        .extract(dest)
        .map_err(|e| format!("zip 풀기 실패: {e}"))?;

    for marker in marker_files {
        std::fs::write(dest.join(marker), b"")
            .map_err(|e| format!("{marker} 생성 실패: {e}"))?;
    }

    if !dest.join(exe_filename).is_file() {
        return Err(format!(
            "다운로드 완료했으나 {exe_filename} 가 보이지 않습니다 ({})",
            dest.display()
        ));
    }

    Ok(())
}
