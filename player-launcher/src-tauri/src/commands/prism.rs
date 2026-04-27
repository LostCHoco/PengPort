//! Prism 인스턴스 동기화 및 실행 관련 커맨드.
//!
//! Prism 위치 결정 우선순위:
//! 1. `PENGPORT_PRISM_ROOT` 환경변수 (dev override)
//! 2. exe 옆 `PrismLauncher/` 폴더 (구 번들 호환 + Phase 2 portable.flag 예정)
//! 3. 시스템 설치본:
//!    - `%LOCALAPPDATA%\Programs\PrismLauncher\prismlauncher.exe`
//!    - `C:\Program Files\PrismLauncher\prismlauncher.exe`
//!    - `PATH` 의 prismlauncher.exe
//!    - 데이터 폴더는 Prism default `%APPDATA%\PrismLauncher\`
//!
//! `packwiz-installer-bootstrap.jar` 은 Rust 바이너리에 `include_bytes!` 로 embed 되어
//! 첫 실행 시 `%APPDATA%/app.pengport/cache/` 에 풀린다.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Mutex, OnceLock};

/// 실행 중인 Prism 자식 process 의 PID 추적. server_id → PID.
/// stop_server 가 taskkill /T 로 prism + 자식 Minecraft 까지 종료.
fn running_pids() -> &'static Mutex<HashMap<String, u32>> {
    static MAP: OnceLock<Mutex<HashMap<String, u32>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

use pengport_shared::PrismPaths;
use serde::Serialize;
use tauri::{AppHandle, Manager};

/// 앱 바이너리에 포함된 packwiz-installer-bootstrap.jar.
/// 버전 갱신 시 `resources/packwiz-installer-bootstrap.jar` 파일만 교체하면 된다.
const BOOTSTRAP_JAR: &[u8] =
    include_bytes!("../../resources/packwiz-installer-bootstrap.jar");

/// 탐색된 Prism 의 위치 정보.
/// `data_dir` 는 instances/ 의 부모 (= Prism 데이터 루트).
#[derive(Debug, Clone, Serialize)]
pub struct PrismLocation {
    pub exe: PathBuf,
    pub data_dir: PathBuf,
    /// 어디서 찾았는지 (UI 표시 + 디버깅용).
    pub source: PrismSource,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PrismSource {
    /// 사용자가 Settings 에서 폴더를 직접 지정.
    User,
    /// `PENGPORT_PRISM_ROOT` 환경변수.
    Env,
    /// exe 옆 PrismLauncher/ (구 번들 / Phase 2 portable).
    Portable,
    /// PengPort 가 OOBE 에서 다운로드한 전용 Prism (`%LOCALAPPDATA%/app.pengport/prism/`).
    /// portable.txt 로 isolated 되어 시스템 Prism 데이터와 분리됨.
    Bundled,
    /// 시스템 설치본 (Programs/PrismLauncher 또는 Program Files).
    System,
    /// PATH 에서 발견.
    Path,
}

use super::paths;

/// bootstrap jar 을 캐시 폴더에 풀고 경로를 돌려준다. 같은 크기면 재기록 생략.
///
/// `pub(super)` — PSP commands 측 (`psp.rs`) 도 third_party prism-launcher 실행 시 활용.
pub(super) fn ensure_bootstrap_jar(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_cache_dir()
        .map_err(|e| format!("app_cache_dir: {e}"))?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("packwiz-installer-bootstrap.jar");
    let needs_write = match fs::metadata(&path) {
        Ok(m) => m.len() as usize != BOOTSTRAP_JAR.len(),
        Err(_) => true,
    };
    if needs_write {
        fs::write(&path, BOOTSTRAP_JAR).map_err(|e| e.to_string())?;
    }
    Ok(path)
}

/// `%APPDATA%\PrismLauncher\` — 시스템 설치본의 데이터 폴더.
/// Prism 이 처음 실행되면서 만들어주므로 우리가 미리 만들지는 않음.
fn appdata_prism_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join("PrismLauncher"))
}

/// prism_root 폴더의 데이터 디렉터리 결정.
/// portable.txt 가 옆에 있으면 portable 모드 → 같은 폴더가 데이터.
/// 없으면 시스템 설치본 → `%APPDATA%\PrismLauncher\`.
fn detect_data_dir(prism_root: &std::path::Path) -> Option<PathBuf> {
    if prism_root.join("portable.txt").is_file() {
        Some(prism_root.to_path_buf())
    } else {
        appdata_prism_dir()
    }
}

fn find_bundled_prism() -> Option<PrismLocation> {
    let root = paths::bundled_prism_root()?;
    let exe = root.join("prismlauncher.exe");
    if exe.is_file() {
        Some(PrismLocation {
            exe,
            data_dir: root,
            source: PrismSource::Bundled,
        })
    } else {
        None
    }
}

/// PATH 에서 prismlauncher.exe 를 찾는다. 발견되면 절대 경로 반환.
fn which_prismlauncher() -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join("prismlauncher.exe");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// Windows 레지스트리의 Uninstall InstallLocation 에서 Prism 위치 추출.
/// NSIS installer 가 사용자 정의 경로(D:\Tools\PrismLauncher\ 등)에 설치된 경우
/// 표준 경로 목록만으로는 못 찾기 때문에 이 단계가 필요하다.
#[cfg(windows)]
fn detect_prism_from_registry() -> Option<PrismLocation> {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ};
    use winreg::RegKey;

    // 후보 경로:
    // - HKLM: 모든 사용자용 설치 (기본 Program Files)
    // - HKCU: 현재 사용자만 설치 (LOCALAPPDATA\Programs)
    // - WOW6432Node: 32비트 빌드가 64비트 Windows 에 설치된 경우
    let candidates = [
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\PrismLauncher"),
        (HKEY_CURRENT_USER, r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\PrismLauncher"),
        (HKEY_LOCAL_MACHINE, r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall\PrismLauncher"),
    ];

    for (hive, subkey) in candidates {
        let root = RegKey::predef(hive);
        let Ok(key) = root.open_subkey_with_flags(subkey, KEY_READ) else { continue };
        let Ok(loc): Result<String, _> = key.get_value("InstallLocation") else { continue };
        let exe = PathBuf::from(loc.trim()).join("prismlauncher.exe");
        if exe.is_file() {
            return Some(PrismLocation {
                exe,
                data_dir: appdata_prism_dir()?,
                source: PrismSource::System,
            });
        }
    }
    None
}

#[cfg(not(windows))]
fn detect_prism_from_registry() -> Option<PrismLocation> {
    None
}

/// 시스템 설치본 탐지: 레지스트리 → 표준 경로 → PATH.
/// 레지스트리가 가장 신뢰할 수 있어 우선이다 (사용자 정의 설치 위치도 잡음).
fn detect_system_prism() -> Option<PrismLocation> {
    if let Some(loc) = detect_prism_from_registry() {
        return Some(loc);
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        candidates.push(PathBuf::from(local).join("Programs").join("PrismLauncher"));
    }
    if let Some(pf) = std::env::var_os("ProgramFiles") {
        candidates.push(PathBuf::from(pf).join("PrismLauncher"));
    } else {
        candidates.push(PathBuf::from(r"C:\Program Files\PrismLauncher"));
    }

    for dir in candidates {
        let exe = dir.join("prismlauncher.exe");
        if exe.is_file() {
            return Some(PrismLocation {
                exe,
                data_dir: appdata_prism_dir()?,
                source: PrismSource::System,
            });
        }
    }

    if let Some(exe) = which_prismlauncher() {
        return Some(PrismLocation {
            exe,
            data_dir: appdata_prism_dir()?,
            source: PrismSource::Path,
        });
    }

    None
}

/// Settings 에서 사용자가 지정한 override 경로.
/// 일반: `%APPDATA%/app.pengport/prism_settings.toml`
/// portable: `<exe>/data/prism_settings.toml`
fn settings_path() -> Option<PathBuf> {
    paths::app_data_root().map(|d| d.join("prism_settings.toml"))
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct PrismSettings {
    /// 사용자가 명시적으로 지정한 prismlauncher.exe 가 있는 폴더.
    /// 비어있으면 자동 탐색.
    override_root: Option<PathBuf>,
}

fn load_prism_settings() -> PrismSettings {
    let Some(path) = settings_path() else { return Default::default() };
    let Ok(text) = std::fs::read_to_string(path) else { return Default::default() };
    toml::from_str(&text).unwrap_or_default()
}

fn save_prism_settings(s: &PrismSettings) -> Result<(), String> {
    let path = settings_path().ok_or_else(|| "%APPDATA% 미정".to_string())?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("디렉터리 생성 실패: {e}"))?;
    }
    let text = toml::to_string_pretty(s).map_err(|e| e.to_string())?;
    std::fs::write(path, text).map_err(|e| format!("파일 쓰기 실패: {e}"))
}

fn find_user_override() -> Option<PrismLocation> {
    let s = load_prism_settings();
    let root = s.override_root?;
    let exe = root.join("prismlauncher.exe");
    if exe.is_file() {
        Some(PrismLocation {
            exe,
            data_dir: detect_data_dir(&root)?,
            source: PrismSource::User,
        })
    } else {
        None
    }
}

/// Prism 위치 결정 (모듈 doc-comment 의 우선순위 트리).
fn find_prism() -> Option<PrismLocation> {
    // 0. 사용자가 Settings 에서 명시적으로 지정한 폴더 (가장 우선)
    if let Some(loc) = find_user_override() {
        return Some(loc);
    }

    // 1. dev override
    if let Some(v) = std::env::var_os("PENGPORT_PRISM_ROOT") {
        let root = PathBuf::from(v);
        let exe = root.join("prismlauncher.exe");
        if exe.is_file() {
            return Some(PrismLocation {
                exe,
                data_dir: detect_data_dir(&root)?,
                source: PrismSource::Env,
            });
        }
    }

    // 2. exe 옆 PrismLauncher/ (구 번들 / Phase 2 portable)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let portable_dir = parent.join("PrismLauncher");
            let portable_exe = portable_dir.join("prismlauncher.exe");
            if portable_exe.is_file() {
                return Some(PrismLocation {
                    exe: portable_exe,
                    data_dir: portable_dir,
                    source: PrismSource::Portable,
                });
            }
        }
    }

    // 3. PengPort 가 OOBE 에서 받은 전용 Prism
    //    (사용자가 명시적으로 다운로드한 것이라 시스템 Prism 보다 우선.
    //    다른 Prism 으로 바꾸고 싶으면 Settings 에서 이 폴더를 비우면 됨.)
    if let Some(loc) = find_bundled_prism() {
        return Some(loc);
    }

    // 4. 시스템 설치
    detect_system_prism()
}

/// 사용자가 직접 폴더를 골라 override 등록. 빈 문자열이면 override 해제 → 자동 탐색 복귀.
/// 폴더 안에 prismlauncher.exe 가 없으면 거부.
#[tauri::command]
pub fn set_prism_override(root: String) -> Result<Option<PrismLocation>, String> {
    let trimmed = root.trim();
    let mut s = load_prism_settings();
    if trimmed.is_empty() {
        s.override_root = None;
    } else {
        let path = PathBuf::from(trimmed);
        if !path.join("prismlauncher.exe").is_file() {
            return Err(format!(
                "선택한 폴더에 prismlauncher.exe 가 없습니다: {}",
                path.display()
            ));
        }
        s.override_root = Some(path);
    }
    save_prism_settings(&s)?;
    Ok(find_prism())
}

/// 현재 다운로드된 전용 Prism (Bundled) 을 삭제. 사용자가 다른 Prism 으로 갈아탈 때 사용.
/// 삭제 후 새로 detect 한 결과 반환.
#[tauri::command]
pub fn remove_bundled_prism() -> Result<Option<PrismLocation>, String> {
    if let Some(root) = paths::bundled_prism_root() {
        if root.exists() {
            std::fs::remove_dir_all(&root)
                .map_err(|e| format!("Bundled Prism 삭제 실패: {e}"))?;
        }
    }
    Ok(find_prism())
}

/// `pub(super)` — PSP 측에서도 동일한 Prism 위치 결정 활용.
pub(super) fn prism_paths() -> Result<(PrismLocation, PrismPaths), String> {
    let loc = find_prism().ok_or_else(|| {
        "PrismLauncher 를 찾을 수 없습니다.\n\
         다음 중 하나가 필요합니다:\n\
         1. PrismLauncher 를 설치하세요 (https://prismlauncher.org).\n\
         2. (개발) PENGPORT_PRISM_ROOT 환경변수로 폴더 지정.\n\
         3. PengPort.exe 옆에 PrismLauncher/ 폴더 배치.".to_string()
    })?;
    // 시스템 설치본의 경우 데이터 폴더(=%APPDATA%\PrismLauncher)는 Prism 첫 실행 시 만들어짐.
    // instances/ 만 미리 만들면 Prism 이 자동 인식.
    let instances = loc.data_dir.join("instances");
    fs::create_dir_all(&instances)
        .map_err(|e| format!("instances 폴더 생성 실패 ({}): {e}", instances.display()))?;
    Ok((loc, PrismPaths::new(instances)))
}

/// 프론트엔드용: 현재 탐지된 Prism 정보 (UI 에 표시 / OOBE 분기 결정).
/// 못 찾으면 None 반환 (호출자가 OOBE 띄우기).
#[tauri::command]
pub fn detect_prism() -> Option<PrismLocation> {
    find_prism()
}

// --- OOBE 자동 다운로드 ---------------------------------------------------

const PRISM_RELEASES_API: &str =
    "https://api.github.com/repos/PrismLauncher/PrismLauncher/releases/latest";
/// Windows portable zip asset 의 이름 패턴 (대소문자 구분).
const PRISM_ASSET_PATTERN: &str = "Windows-MSVC-Portable";

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

/// 진행 단계 — 프론트엔드가 표시할 메시지 결정에 사용.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct PrismDownloadResult {
    pub version: String,
    pub install_dir: PathBuf,
}

/// PrismLauncher Windows Portable 최신 release 를 다운로드해서
/// `%LOCALAPPDATA%\app.pengport\prism\` 에 푼다.
/// portable.txt 를 함께 만들어 시스템 Prism 데이터(`%APPDATA%\PrismLauncher\`) 와 격리.
///
/// 동작:
/// 1. GitHub API 로 latest release 조회
/// 2. assets 중 `Windows-MSVC-Portable*.zip` 하나 선택
/// 3. 다운로드 (수십 MB)
/// 4. 기존 폴더 삭제 후 새로 풀기 (재실행 시 재다운로드 = 깨끗한 상태 보장)
/// 5. portable.txt 생성
///
/// 네트워크/IO 작업이므로 `spawn_blocking` 으로 실행. Tauri command 자체는 async.
#[tauri::command]
pub async fn download_prism() -> Result<PrismDownloadResult, String> {
    let dest = paths::bundled_prism_root()
        .ok_or_else(|| "캐시 루트를 결정할 수 없음 (%LOCALAPPDATA% 미정?)".to_string())?;

    let result = tauri::async_runtime::spawn_blocking(move || -> Result<PrismDownloadResult, String> {
        let release: GhRelease = ureq::get(PRISM_RELEASES_API)
            .header("User-Agent", "PengPort")
            .header("Accept", "application/vnd.github+json")
            .call()
            .map_err(|e| format!("GitHub API 호출 실패: {e}"))?
            .body_mut()
            .read_json()
            .map_err(|e| format!("release JSON 파싱 실패: {e}"))?;

        let asset = release.assets.iter()
            .find(|a| a.name.contains(PRISM_ASSET_PATTERN) && a.name.ends_with(".zip"))
            .ok_or_else(|| format!(
                "{} release 에 '{}*.zip' asset 이 없습니다",
                release.tag_name, PRISM_ASSET_PATTERN
            ))?;

        // zip 통째로 메모리에 받기 (대략 20~30 MB → 메모리 부담 없음).
        let mut buf = Vec::with_capacity(40 * 1024 * 1024);
        ureq::get(&asset.browser_download_url)
            .header("User-Agent", "PengPort")
            .call()
            .map_err(|e| format!("다운로드 실패: {e}"))?
            .body_mut()
            .as_reader()
            .read_to_end(&mut buf)
            .map_err(|e| format!("read_to_end: {e}"))?;

        // 깨끗한 폴더로 시작 (재시도 시 stale 잔재 제거).
        if dest.exists() {
            std::fs::remove_dir_all(&dest)
                .map_err(|e| format!("기존 폴더 정리 실패: {e}"))?;
        }
        std::fs::create_dir_all(&dest)
            .map_err(|e| format!("대상 폴더 생성 실패: {e}"))?;

        let cursor = std::io::Cursor::new(&buf);
        let mut archive = zip::ZipArchive::new(cursor)
            .map_err(|e| format!("zip 열기 실패: {e}"))?;
        archive.extract(&dest)
            .map_err(|e| format!("zip 풀기 실패: {e}"))?;

        // Prism 의 portable 모드 트리거: 옆에 portable.txt 가 있으면 데이터를
        // 자기 폴더에 둠 (시스템 Prism 의 %APPDATA%\PrismLauncher\ 와 분리).
        std::fs::write(dest.join("portable.txt"), b"")
            .map_err(|e| format!("portable.txt 생성 실패: {e}"))?;

        if !dest.join("prismlauncher.exe").is_file() {
            return Err(format!(
                "다운로드 완료했으나 prismlauncher.exe 가 보이지 않습니다 ({})",
                dest.display()
            ));
        }

        Ok(PrismDownloadResult {
            version: release.tag_name,
            install_dir: dest,
        })
    })
    .await
    .map_err(|e| format!("blocking task 실패: {e}"))??;

    Ok(result)
}

/// Prism 인스턴스를 띄우고 자식 process 수명을 추적한다 (sync 없음).
///
/// `pub(super)` — PSP `third_party.prism-launcher` 분기 (`commands::psp::invoke_third_party`)
/// 가 자체 sync (`upsert_prism_instance`) 후 이 함수만 호출.
///
/// 자식 종료 시 `server:stopped` event emit. spawn 직후 `server:started` 도 emit.
pub(super) fn spawn_prism_instance(
    app: &AppHandle,
    instance_id: &str,
) -> Result<(), String> {
    use tauri::Emitter;

    let (loc, _) = prism_paths()?;
    let mut child = Command::new(&loc.exe)
        .args(["--launch", instance_id])
        .spawn()
        .map_err(|e| format!("Prism 실행 실패: {e}"))?;

    let pid = child.id();
    running_pids()
        .lock()
        .unwrap()
        .insert(instance_id.to_string(), pid);

    let _ = app.emit(
        "server:started",
        serde_json::json!({ "serverId": instance_id }),
    );

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

/// 실행 중인 서버 (Prism + child Minecraft) 를 강제 종료.
/// Windows 의 `taskkill /T /F /PID` 로 process tree 전체 종료.
/// 종료 후 wait task 가 server:stopped event emit.
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
