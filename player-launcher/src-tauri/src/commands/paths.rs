//! 앱 데이터 경로 결정. portable.flag 가 exe 옆에 있으면 모든 사용자 데이터를
//! exe 옆 `./data/` 로 격리한다 (USB 시나리오: PC방, 공용 PC).
//!
//! - 일반 모드: OS 표준 위치 (`%APPDATA%/PengPort/`, `%LOCALAPPDATA%/PengPort/`)
//! - Portable 모드: `<exe_dir>/data/`, `<exe_dir>/data/cache/`
//!
//! Tauri 의 `app_data_dir()` / `app_cache_dir()` 는 identifier 만 따라 OS 표준 위치를
//! 반환하므로 portable 모드 분기에는 사용하지 않는다.

use std::path::PathBuf;

/// Tauri identifier — `tauri.conf.json` 의 `identifier` 와 동기 유지.
/// 0.1.3 에서 `app.pengport` → `PengPort` 변경. 옛 폴더는 lib.rs 의 migration helper 가 rename.
pub const APP_IDENTIFIER: &str = "PengPort";

/// 0.1.3 이전 (0.1.2 이하) 시절의 식별자. migration 시 옛 폴더 path 결정용.
pub const LEGACY_APP_IDENTIFIER: &str = "app.pengport";

/// PengPort 의 0.1.x 이력에서 사용된 모든 fs 식별자 (현재 + 옛).
/// 데이터 초기화 / 완전 삭제 시 모두 삭제 대상.
/// - `PengPort`          : 0.1.3 부터 (current)
/// - `app.pengport`      : 0.1.2 까지
/// - `PengdollPark`      : 0.1.0 시절의 productName 기반 (옛 빌드의 webview userData 일 수 있음)
/// - `app.pengdollpark`  : 0.1.0 시절 identifier 기반
pub const ALL_FS_IDENTIFIERS: &[&str] = &[
    "PengPort",
    "app.pengport",
    "PengdollPark",
    "app.pengdollpark",
];

/// NSIS Uninstall registry 에 등록되었을 모든 productName 들. 완전 삭제 시 각각의 uninstaller 시도.
pub const ALL_PRODUCT_NAMES: &[&str] = &["PengPort", "PengdollPark"];

fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(|p| p.to_path_buf())
}

/// exe 옆에 portable.flag 파일이 있는지.
/// 있으면 모든 데이터를 exe 옆 폴더에 둔다.
pub fn is_portable() -> bool {
    exe_dir()
        .map(|d| d.join("portable.flag").is_file())
        .unwrap_or(false)
}

/// 사용자 설정 루트 (작은 설정 파일들).
/// 일반: `%APPDATA%/PengPort/`
/// portable: `<exe_dir>/data/`
pub fn app_data_root() -> Option<PathBuf> {
    if is_portable() {
        Some(exe_dir()?.join("data"))
    } else {
        std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join(APP_IDENTIFIER))
    }
}

/// 캐시 / 큰 파일 루트 (PengPort 가 받은 Prism 등).
/// 일반: `%LOCALAPPDATA%/PengPort/`
/// portable: `<exe_dir>/data/cache/`
pub fn app_cache_root() -> Option<PathBuf> {
    if is_portable() {
        Some(exe_dir()?.join("data").join("cache"))
    } else {
        std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join(APP_IDENTIFIER))
    }
}

/// 0.1.3 이전의 옛 식별자 (`app.pengport`) 가 만든 데이터 폴더가 있으면 새 폴더로 rename 한다.
/// 앱 본체가 시작되기 직전에 (Tauri webview 가 user data dir 에 lock 잡기 전) 한 번만 호출.
///
/// 새 폴더가 이미 존재하면 (사용자가 0.1.3 클린 설치) 옛 폴더 그대로 두고 skip — 데이터 충돌 회피.
/// 옛 폴더가 없으면 (0.1.3 이 첫 설치) 그대로 skip.
///
/// keyring 의 service name (`app.pengport`) 은 그대로 유지 — 사용자 fs 와 무관, 마이그레이션 불필요.
pub fn migrate_legacy_app_dirs() {
    if is_portable() {
        return;
    }
    for var in ["APPDATA", "LOCALAPPDATA"] {
        let Some(root) = std::env::var_os(var) else {
            continue;
        };
        let old_dir = PathBuf::from(&root).join(LEGACY_APP_IDENTIFIER);
        let new_dir = PathBuf::from(&root).join(APP_IDENTIFIER);
        if old_dir.exists() && !new_dir.exists() {
            if let Err(e) = std::fs::rename(&old_dir, &new_dir) {
                eprintln!(
                    "legacy app dir 이동 실패 ({} → {}): {e}",
                    old_dir.display(),
                    new_dir.display()
                );
            }
        }
    }
}

/// OOBE 에서 다운로드한 전용 Prism 위치.
pub fn bundled_prism_root() -> Option<PathBuf> {
    Some(app_cache_root()?.join("prism"))
}
