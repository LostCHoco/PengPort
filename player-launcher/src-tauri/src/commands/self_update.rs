//! 자체 업데이트 — portable exe 를 rename-to-delete 패턴으로 교체한다.
//!
//! `tauri-plugin-updater`는 Windows 에서 NSIS/MSI 설치형 배포만 공식 지원한다 —
//! `Update::install()`이 다운로드한 바이트를 인스톨러 실행 파일로 간주하고 spawn 하기
//! 때문에, portable exe(그냥 실행 파일 자체)를 넘기면 맞지 않는다. 하지만 그 크레이트의
//! `Update::download()`는 `install()`과 분리된 public API로, 엔드포인트 조회 +
//! minisign 서명 검증(`tauri.conf.json`의 기존 `pubkey`/`endpoints` 그대로 재사용)까지
//! 끝낸 raw 바이트만 돌려준다(2026-08 확인, `tauri-apps/plugins-workspace` 소스 직접
//! 확인 — `updater.rs`의 `Update::download`/`install`이 별개 메서드). 그래서 `check`/
//! `download`은 이 크레이트에 그대로 맡기고, "설치" 단계만 직접 구현한다:
//!
//! 1. 검증된 새 exe 바이트를 `<exe_dir>/PengPort.new.exe` 로 씀
//! 2. 지금 실행 중인 exe 를 `PengPort.old.exe` 로 rename — Windows 는 실행 중인 파일도
//!    rename 은 허용한다(덮어쓰기만 금지). `rustup`/`bun`/`deno` 등 단일 바이너리 CLI
//!    도구가 쓰는 표준 self-update 패턴("rename-to-delete").
//! 3. `PengPort.new.exe` 를 원래 이름으로 rename
//! 4. 새 exe 를 곧바로 spawn 하지 않고, 짧은 지연 후 실행하는 detached 스크립트를
//!    예약한 뒤 이 프로세스는 즉시 종료 — 이 프로세스가 `tauri_plugin_single_instance`
//!    잠금을 놓기 전에 새 exe 가 뜨면 "이미 실행 중"으로 오인돼 조용히 다시 종료될 수
//!    있어서, 확실히 종료된 뒤 새 exe 가 뜨도록 시간차를 둔다.
//!
//! 실패 시(주로 3번 rename)에는 원래 이름으로 롤백을 시도한다 — 절반만 진행된 채로
//! exe 이름을 잃어버리면 사용자가 직접 파일을 못 찾게 된다.

use tauri::AppHandle;
use tauri_plugin_updater::UpdaterExt;

#[derive(serde::Serialize)]
pub struct SelfUpdateInfo {
    version: String,
    current_version: String,
    body: Option<String>,
}

/// 새 버전이 있는지 확인만 한다(다운로드/설치 안 함). 없으면 `None`.
#[tauri::command]
pub async fn check_self_update(app: AppHandle) -> Result<Option<SelfUpdateInfo>, String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    let update = updater.check().await.map_err(|e| e.to_string())?;
    Ok(update.map(|u| SelfUpdateInfo {
        version: u.version,
        current_version: u.current_version,
        body: u.body,
    }))
}

/// 새 버전을 다운로드(서명 검증까지 완료)하고 rename-to-delete 로 교체 후 재시작을
/// 예약한다. 성공하면 이 프로세스가 곧 종료되므로 호출자 입장에서 반환 자체가 안
/// 보일 수 있다(정상).
#[tauri::command]
pub async fn install_self_update(app: AppHandle) -> Result<(), String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    let update = updater
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "적용 가능한 업데이트가 없습니다".to_string())?;

    let bytes = update
        .download(|_, _| {}, || {})
        .await
        .map_err(|e| e.to_string())?;

    #[cfg(windows)]
    {
        replace_running_exe_windows(&bytes)?;
        app.exit(0);
        Ok(())
    }

    #[cfg(not(windows))]
    {
        let _ = (app, bytes);
        Err("Windows 외 OS 미지원".to_string())
    }
}

/// rename-to-delete 의 핵심(1~3단계) — `exe`가 실제로 지금 실행 중인 프로세스일
/// 필요는 없다(경로 하나만 다룸). `current_exe()` 해석을 분리해둔 이유는 순수하게
/// 이 부분만 임시 디렉터리로 단위 테스트하기 위함(진짜 실행 파일을 건드리지 않고).
#[cfg(windows)]
fn stage_and_swap(exe: &std::path::Path, new_exe_bytes: &[u8]) -> Result<(), String> {
    let dir = exe
        .parent()
        .ok_or_else(|| "exe 폴더를 확인할 수 없습니다".to_string())?;
    let staged = dir.join("PengPort.new.exe");
    let old = dir.join("PengPort.old.exe");

    std::fs::write(&staged, new_exe_bytes).map_err(|e| format!("새 exe 기록 실패: {e}"))?;
    // 지난 업데이트가 못 지운 `.old.exe` 잔재가 있으면 먼저 정리 시도 — 실패해도 무시
    // (아래 rename 대상 이름만 안 겹치면 되므로 치명적이지 않다).
    let _ = std::fs::remove_file(&old);

    std::fs::rename(exe, &old).map_err(|e| format!("현재 exe rename 실패: {e}"))?;
    if let Err(e) = std::fs::rename(&staged, exe) {
        // 원래 이름을 잃은 채로 끝나면 사용자가 파일을 못 찾으니 반드시 롤백 시도.
        let _ = std::fs::rename(&old, exe);
        return Err(format!("새 exe 배치 실패, 롤백함: {e}"));
    }
    Ok(())
}

#[cfg(windows)]
fn replace_running_exe_windows(new_exe_bytes: &[u8]) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    const DETACHED_PROCESS: u32 = 0x0000_0008;

    let exe = std::env::current_exe().map_err(|e| format!("exe 경로 확인 실패: {e}"))?;
    stage_and_swap(&exe, new_exe_bytes)?;

    let script = format!(
        "@echo off\r\ntimeout /t 2 /nobreak > nul\r\nstart \"\" \"{p}\"\r\ndel \"%~f0\"\r\n",
        p = exe.display(),
    );
    let script_path =
        std::env::temp_dir().join(format!("pengport-relaunch-{}.bat", std::process::id()));
    super::write_windows_batch_script(&script_path, &script)
        .map_err(|e| format!("재시작 스크립트 생성 실패: {e}"))?;
    std::process::Command::new("cmd")
        .args(["/c", &script_path.to_string_lossy()])
        .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
        .spawn()
        .map_err(|e| format!("재시작 스크립트 실행 실패: {e}"))?;

    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn temp_test_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pengport-self-update-test-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn stage_and_swap_replaces_exe_content_and_keeps_old_copy() {
        let dir = temp_test_dir("swap-ok");
        let exe = dir.join("PengPort.exe");
        std::fs::write(&exe, b"old exe bytes").unwrap();

        stage_and_swap(&exe, b"new exe bytes").unwrap();

        assert_eq!(std::fs::read(&exe).unwrap(), b"new exe bytes");
        assert_eq!(std::fs::read(dir.join("PengPort.old.exe")).unwrap(), b"old exe bytes");
        // 스테이징 파일은 원래 이름으로 rename 돼 사라져야 한다.
        assert!(!dir.join("PengPort.new.exe").exists());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stage_and_swap_cleans_up_stale_old_exe_before_swapping() {
        let dir = temp_test_dir("stale-old");
        let exe = dir.join("PengPort.exe");
        std::fs::write(&exe, b"current exe").unwrap();
        // 지난 업데이트가 못 지운 잔재를 미리 만들어둔다.
        std::fs::write(dir.join("PengPort.old.exe"), b"stale leftover").unwrap();

        stage_and_swap(&exe, b"fresh exe").unwrap();

        assert_eq!(std::fs::read(&exe).unwrap(), b"fresh exe");
        // 잔재가 아니라 방금 교체된 이전 exe 내용으로 덮였어야 한다.
        assert_eq!(std::fs::read(dir.join("PengPort.old.exe")).unwrap(), b"current exe");

        let _ = std::fs::remove_dir_all(&dir);
    }

    // "새 exe 를 원래 이름으로 rename" 실패 시 롤백하는 분기는 의도적으로 단위
    // 테스트하지 않는다 — 이 시점엔 이미 원본이 `.old.exe`로 옮겨져 목적지 경로가
    // 비어있어서, 그 rename 하나만 콕 집어 실패시키려면 Windows 파일 핸들 공유
    // 모드 조작이 필요한데, 그러면 앞선 rename까지 같이 막혀 버려 원하는 분기를
    // 못 골라낸다. 코드 자체는 (1) 성공 경로, (2) 잔재 정리 두 테스트로 검증된
    // 같은 rename 호출의 단순 에러 처리라 별도 커버리지 없이도 위험은 낮다.
}
