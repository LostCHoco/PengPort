//! 앱 데이터 경로 결정. PengPort 는 완전 portable — 모든 사용자 데이터를 exe 옆
//! `./data/`(설정) · `./data/cache/`(캐시/큰 파일) 에 둔다. 설치 절차·레지스트리
//! 흔적이 없다: exe 하나 + 옆의 data 폴더가 전부다(2026-08 portable 전환,
//! `docs/track/portable-transition.md`). 옛 OS 표준 위치(`%APPDATA%/PengPort/`,
//! `%LOCALAPPDATA%/PengPort/`)에 데이터가 남아있으면 `migrate_os_standard_data_to_portable`
//! 이 최초 실행 시 1 회 이전한다.
//!
//! Tauri 의 `app_data_dir()` / `app_cache_dir()` 는 identifier 만 따라 OS 표준 위치를
//! 반환하므로 쓰지 않는다.

use std::path::{Path, PathBuf};

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

/// exe 가 있는 폴더 — portable 모델의 유일한 기준점(설치 경로 개념 자체가 없음).
pub fn exe_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()?
        .parent()
        .map(|p| p.to_path_buf())
}

/// 사용자 설정 루트 (작은 설정 파일들) — 항상 `<exe_dir>/data/`.
pub fn app_data_root() -> Option<PathBuf> {
    Some(exe_dir()?.join("data"))
}

/// 캐시 / 큰 파일 루트 (PengPort 가 받은 Prism 등) — 항상 `<exe_dir>/data/cache/`.
pub fn app_cache_root() -> Option<PathBuf> {
    Some(exe_dir()?.join("data").join("cache"))
}

/// 0.1.3 이전의 옛 식별자 (`app.pengport`) 가 OS 표준 위치에 만든 데이터 폴더가
/// 있으면 새 이름으로 rename 한다. `migrate_os_standard_data_to_portable`(같은 파일)
/// 이 OS 표준 위치 → portable 위치 이전을 하기 *전에* 반드시 먼저 호출 — 그래야
/// 옛 식별자 폴더까지 하나의 정식 이름(`PengPort`)으로 합쳐진 뒤 이전된다.
///
/// 새 폴더가 이미 존재하면 옛 폴더 그대로 두고 skip — 데이터 충돌 회피.
/// 옛 폴더가 없으면 그대로 skip.
///
/// keyring 의 service name (`app.pengport`) 은 그대로 유지 — 사용자 fs 와 무관, 마이그레이션 불필요.
pub fn migrate_legacy_app_dirs() {
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

const MIGRATION_MARKER_FILE: &str = ".migration-complete";

/// 옛 OS 표준 위치(`%APPDATA%/PengPort/`, `%LOCALAPPDATA%/PengPort/`)에 남은 데이터를
/// portable 위치(`<exe_dir>/data/`, `<exe_dir>/data/cache/`)로 1 회 이전한다. 앱 시작
/// 직후 `migrate_legacy_app_dirs` 바로 다음에 한 번만 호출(그 함수가 옛 identifier를
/// 먼저 `PengPort`로 합쳐놔야 여기서 한 위치만 보면 됨).
///
/// **멱등·재시도 안전**: 완료 마커(`<exe_dir>/data/.migration-complete`)가 있으면 즉시
/// skip. 마커는 두 원본 위치의 모든 항목이 전부 이전(또는 원래 없음)됐을 때만 쓴다 —
/// 일부 항목이 잠겨있어 실패하면 마커를 안 써서 다음 실행이 남은 항목만 재시도한다
/// (이미 옮겨진 항목은 원본에 더는 없으니 재시도 대상이 아니다 — 부분 실패가 데이터를
/// 두 위치에 걸쳐 영구히 흩어놓지 않는다).
///
/// **이동 방식**: 항목마다 우선 `rename`(원자적, 같은 드라이브면 항상 성공) 시도, 다른
/// 드라이브(예: exe 는 USB 인데 옛 데이터는 `C:`)라 실패하면 재귀 복사 후 원본 삭제로
/// 폴백한다.
pub fn migrate_os_standard_data_to_portable() {
    let Some(data_root) = app_data_root() else {
        return;
    };
    if data_root.join(MIGRATION_MARKER_FILE).exists() {
        return;
    }
    let Some(cache_root) = app_cache_root() else {
        return;
    };

    let mut all_ok = true;
    if let Some(appdata) = std::env::var_os("APPDATA") {
        let old = PathBuf::from(appdata).join(APP_IDENTIFIER);
        all_ok &= migrate_dir_contents(&old, &data_root);
    }
    if let Some(localappdata) = std::env::var_os("LOCALAPPDATA") {
        let old = PathBuf::from(localappdata).join(APP_IDENTIFIER);
        all_ok &= migrate_dir_contents(&old, &cache_root);
    }

    if !all_ok {
        eprintln!("portable 데이터 이전이 일부 실패 — 다음 실행 시 남은 항목 재시도");
        return;
    }
    if let Err(e) = std::fs::create_dir_all(&data_root) {
        eprintln!("portable data 폴더 생성 실패: {e}");
        return;
    }
    if let Err(e) = std::fs::write(data_root.join(MIGRATION_MARKER_FILE), "") {
        eprintln!("마이그레이션 완료 마커 기록 실패: {e}");
    }
}

/// `src`의 모든 하위 항목을 `dst`로 옮긴다(`src` 자체가 없으면 옮길 게 없다는 뜻이라
/// 성공으로 취급). 항목 하나가 실패해도 나머지는 계속 시도 — 전부 성공해야 true.
fn migrate_dir_contents(src: &Path, dst: &Path) -> bool {
    let Ok(entries) = std::fs::read_dir(src) else {
        return true;
    };
    if let Err(e) = std::fs::create_dir_all(dst) {
        eprintln!("portable 대상 폴더 생성 실패 ({}): {e}", dst.display());
        return false;
    }
    let mut ok = true;
    for entry in entries.flatten() {
        let from = entry.path();
        let Some(name) = from.file_name() else {
            continue;
        };
        let to = dst.join(name);
        if let Err(e) = move_path(&from, &to) {
            eprintln!(
                "데이터 이전 실패 ({} → {}): {e}",
                from.display(),
                to.display()
            );
            ok = false;
        }
    }
    // 전부 옮겨졌으면 이제 빈 폴더가 된 src 자체도 정리 — 실패해도(뭔가 남았으면) 조용히 무시.
    if ok {
        let _ = std::fs::remove_dir(src);
    }
    ok
}

/// 파일/폴더 하나를 이동한다. 같은 드라이브면 `rename`(원자적)으로 끝나고, 드라이브가
/// 달라 실패하면 재귀 복사 후 원본 삭제로 폴백한다.
fn move_path(from: &Path, to: &Path) -> std::io::Result<()> {
    match std::fs::rename(from, to) {
        Ok(()) => Ok(()),
        Err(_) if from.is_dir() => {
            copy_dir_recursive(from, to)?;
            std::fs::remove_dir_all(from)
        }
        Err(_) if from.is_file() => {
            std::fs::copy(from, to)?;
            std::fs::remove_file(from)
        }
        Err(e) => Err(e),
    }
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else {
            std::fs::copy(&from, &to)?;
        }
    }
    Ok(())
}

/// PengPort 가 OOBE 에서 자동 다운로드한 third-party app 전용 위치. `app_id` 별로
/// 격리 — 어떤 third-party app 이든 같은 규칙(범용).
pub fn bundled_third_party_root(app_id: &str) -> Option<PathBuf> {
    Some(app_cache_root()?.join(app_id))
}

/// 레시피 전용 앱 루트 — `InstallStep::DownloadArchive{target: App}`/`WriteFile`/
/// `WriteConfig`/`LaunchAction::SpawnProcess`의 대상. 레시피별로 격리(Prism instance
/// 폴더와 같은 패턴). 설치 완료 마커(`.pengport-markers/`)도 이 폴더 아래 둔다 —
/// third-party app 데이터 영역에 쓰는 스텝이라도 idempotency 기록은 항상 여기.
pub fn app_root(recipe_id: &str) -> Option<PathBuf> {
    Some(app_cache_root()?.join("apps").join(recipe_id))
}

/// `.pengz`(레시피 번들 파일) 확장자를 이 실행 파일로 연결 — 설치 프로그램이 아니라
/// 이 프로세스가 런타임에 직접 등록한다: NSIS installer는 설치 시점에만 동작해
/// `cargo run`/`tauri dev`에서는 검증할 방법이 없다. `HKEY_CURRENT_USER` 아래만 쓰므로
/// 관리자 권한 불필요, 재실행해도 같은 값 덮어쓰기라 idempotent.
#[cfg(windows)]
pub fn register_pengz_file_association() {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let exe = exe.to_string_lossy();
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);

    let register = || -> std::io::Result<()> {
        let (ext_key, _) = hkcu.create_subkey(format!(
            "Software\\Classes\\.{}",
            pengport_shared::library::FILE_EXTENSION
        ))?;
        ext_key.set_value("", &"PengPort.RecipeFile")?;

        let (progid_key, _) = hkcu.create_subkey("Software\\Classes\\PengPort.RecipeFile")?;
        progid_key.set_value("", &"PengPort 레시피 파일")?;

        let (cmd_key, _) =
            hkcu.create_subkey("Software\\Classes\\PengPort.RecipeFile\\shell\\open\\command")?;
        cmd_key.set_value("", &format!("\"{exe}\" \"%1\""))?;
        Ok(())
    };

    if let Err(e) = register() {
        eprintln!(".pengz 파일 연결 등록 실패: {e}");
    }
}

#[cfg(not(windows))]
pub fn register_pengz_file_association() {}
