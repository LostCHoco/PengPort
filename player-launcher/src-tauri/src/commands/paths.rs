//! 앱 데이터 경로 결정. portable.flag 가 exe 옆에 있으면 모든 사용자 데이터를
//! exe 옆 `./data/` 로 격리한다 (USB 시나리오: PC방, 공용 PC).
//!
//! - 일반 모드: OS 표준 위치 (`%APPDATA%/app.pengport/`, `%LOCALAPPDATA%/app.pengport/`)
//! - Portable 모드: `<exe_dir>/data/`, `<exe_dir>/data/cache/`
//!
//! Tauri 의 `app_data_dir()` / `app_cache_dir()` 는 identifier 만 따라 OS 표준 위치를
//! 반환하므로 portable 모드 분기에는 사용하지 않는다.

use std::path::PathBuf;

/// Tauri identifier — `tauri.conf.json` 의 `identifier` 와 동기 유지.
pub const APP_IDENTIFIER: &str = "app.pengport";

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
/// 일반: `%APPDATA%/app.pengport/`
/// portable: `<exe_dir>/data/`
pub fn app_data_root() -> Option<PathBuf> {
    if is_portable() {
        Some(exe_dir()?.join("data"))
    } else {
        std::env::var_os("APPDATA").map(|p| PathBuf::from(p).join(APP_IDENTIFIER))
    }
}

/// 캐시 / 큰 파일 루트 (PengPort 가 받은 Prism 등).
/// 일반: `%LOCALAPPDATA%/app.pengport/`
/// portable: `<exe_dir>/data/cache/`
pub fn app_cache_root() -> Option<PathBuf> {
    if is_portable() {
        Some(exe_dir()?.join("data").join("cache"))
    } else {
        std::env::var_os("LOCALAPPDATA").map(|p| PathBuf::from(p).join(APP_IDENTIFIER))
    }
}

/// OOBE 에서 다운로드한 전용 Prism 위치.
pub fn bundled_prism_root() -> Option<PathBuf> {
    Some(app_cache_root()?.join("prism"))
}
