//! 자체 업데이트 — portable exe 를 `self-replace` 크레이트로 교체한다.
//!
//! `tauri-plugin-updater`는 Windows 에서 NSIS/MSI 설치형 배포만 공식 지원한다 —
//! `Update::install()`이 다운로드한 바이트를 인스톨러 실행 파일로 간주하고 spawn 하기
//! 때문에, portable exe(그냥 실행 파일 자체)를 넘기면 맞지 않는다. 하지만 그 크레이트의
//! `Update::download()`는 `install()`과 분리된 public API로, 엔드포인트 조회 +
//! minisign 서명 검증(`tauri.conf.json`의 기존 `pubkey`/`endpoints` 그대로 재사용)까지
//! 끝낸 raw 바이트만 돌려준다(2026-08 확인, `tauri-apps/plugins-workspace` 소스 직접
//! 확인 — `updater.rs`의 `Update::download`/`install`이 별개 메서드). 그래서 `check`/
//! `download`은 이 크레이트에 그대로 맡기고, "설치" 단계만 직접 구현한다.
//!
//! **버전 이력**: 처음엔 rename-to-delete + `cmd.exe`/`.bat`(`start "" "path"`)로 직접
//! 구현했으나, 실사용자가 한글 경로(`OneDrive\바탕 화면\...`)에서 재시작이 안 되는 걸
//! 실제로 겪음 — 배치 파일에 UTF-8 BOM을 붙여도(`cmd.exe`가 스크립트 "읽기"는 UTF-8로
//! 정상 해석) 여전히 실패했다. `start` 빌트인 자체가 내부적으로 어딘가 ANSI 코드페이지를
//! 한 번 더 거치는 것으로 보임(정확한 원인은 끝내 못 찾음). PowerShell 직접 실행으로도
//! 바꿔봤지만 같은 종류의 "외부 셸에 경로를 실어 보낸다"는 구조적 리스크가 남는 데다,
//! 검증 자체가 이 세션에서 신뢰할 수 없었다(반복 테스트 타이밍에 따라 성공/실패가
//! 오락가락함).
//!
//! 그래서 **rustup 등이 실제로 쓰는 `self-replace` 크레이트**(<https://github.com/mitsuhiko/self-replace>)
//! 로 교체했다 — 실제 소스를 직접 읽어 확인한 핵심: 경로를 텍스트로 셸에 실어 보내는
//! 지점이 어디에도 없다(전부 `std::process::Command`/WinAPI 인자로 직접 전달 — Rust가
//! 이미 UTF-16 API를 쓰므로 유니코드 안전). 옛 exe 삭제는 `FILE_FLAG_DELETE_ON_CLOSE`로
//! OS에 위임(`del` 명령에 기대지 않음). 유일한 `cmd.exe` 사용은 인자 없는
//! `cmd.exe /c exit` 하나뿐(경로 전혀 안 실림). 롤백용 `.old.exe` 백업은 유지 안 함
//! (2026-08 사용자 확인 — 필요 없다고 판단, 크레이트도 그런 기능 자체가 없음).
//!
//! 재시작(크레이트 범위 밖 — `self-replace`는 "교체"까지만 책임짐)은 같은 원리를
//! 재사용: 교체 직후 새 exe(이미 원래 경로에 자리잡음)를 `--pengport-wait-for-exit
//! <옛 pid>` 인자와 함께 그냥 spawn(순수 Rust `Command`, 셸 없음). 새로 뜬 프로세스는
//! `lib.rs::run()` 맨 앞에서 그 인자를 보고, 옛 프로세스가 실제로 종료될 때까지 기다린
//! 뒤(`sysinfo`로 폴링 — 고정 시간 추측 아님, `tauri_plugin_single_instance` 잠금을
//! 옛 프로세스가 아직 쥐고 있을 때 뜨면 "이미 실행 중"으로 오인되는 걸 막기 위함) 정상
//! 초기화를 계속한다.

use tauri::AppHandle;
use tauri_plugin_updater::UpdaterExt;

/// 재시작된 새 프로세스가 기다려야 할 옛 프로세스 pid를 넘기는 내부 신호 — 사용자가
/// 직접 쓸 일 없음. `lib.rs::run()`이 시작하자마자 이 인자를 확인한다.
pub const WAIT_FOR_EXIT_ARG: &str = "--pengport-wait-for-exit";

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

/// 새 버전을 다운로드(서명 검증까지 완료)하고 `self-replace`로 교체 후 재시작을
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
        replace_and_relaunch_windows(&bytes)?;
        app.exit(0);
        Ok(())
    }

    #[cfg(not(windows))]
    {
        let _ = (app, bytes);
        Err("Windows 외 OS 미지원".to_string())
    }
}

#[cfg(windows)]
fn replace_and_relaunch_windows(new_exe_bytes: &[u8]) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| format!("exe 경로 확인 실패: {e}"))?;
    let dir = exe
        .parent()
        .ok_or_else(|| "exe 폴더를 확인할 수 없습니다".to_string())?;

    // self_replace 는 "이걸로 지금 exe 를 교체해라"만 받는다 — 대상 파일이
    // 실제로 있어야 하므로 새 바이트를 임시 파일로 먼저 써둔다.
    let staged = dir.join("PengPort.new.exe");
    std::fs::write(&staged, new_exe_bytes).map_err(|e| format!("새 exe 기록 실패: {e}"))?;

    self_replace::self_replace(&staged).map_err(|e| format!("자체 교체 실패: {e}"))?;
    // 크레이트 문서 권장대로 스테이징 파일 정리 — 실패해도 치명적이지 않음(다음
    // 업데이트 때 새로 덮어써짐, `self_replace` 자체는 이미 완료된 뒤).
    let _ = std::fs::remove_file(&staged);

    spawn_relaunch(&exe, std::process::id())
}

/// 새 exe(교체가 끝나 이미 `exe` 자리에 있음)를 `--pengport-wait-for-exit <옛 pid>`
/// 인자와 함께 spawn — 순수 `Command`(셸 없음)라 경로에 무엇이 들어있든 안전하다.
/// `exe`/`old_pid`를 인자로 받는 이유는 `stage_and_swap`과 같음: 실제 실행 중인
/// 프로세스 없이 테스트하기 위함.
#[cfg(windows)]
fn spawn_relaunch(exe: &std::path::Path, old_pid: u32) -> Result<(), String> {
    std::process::Command::new(exe)
        .arg(WAIT_FOR_EXIT_ARG)
        .arg(old_pid.to_string())
        .spawn()
        .map_err(|e| format!("재시작 실패: {e}"))?;
    Ok(())
}

/// `args`(보통 `std::env::args().collect()`)에서 [`WAIT_FOR_EXIT_ARG`] 뒤의 pid를
/// 파싱 — 있으면 이 프로세스가 자체 업데이트로 막 재시작됐다는 뜻.
pub fn parse_wait_for_exit_pid(args: &[String]) -> Option<u32> {
    let idx = args.iter().position(|a| a == WAIT_FOR_EXIT_ARG)?;
    args.get(idx + 1)?.parse().ok()
}

/// `pid`가 종료될 때까지 기다린다(최대 `timeout` — 옛 프로세스가 어떤 이유로든 안
/// 죽어도 새 프로세스가 영원히 멈춰있진 않게 하는 안전장치). 옛 `timeout /t 2`(고정
/// 시간 추측) 대신 실제 종료를 확인 — 옛 프로세스가 더 오래 걸려도 안전하고, 빨리
/// 끝나면 그만큼 빨리 이어서 진행한다.
pub fn wait_for_process_exit(pid: u32, timeout: std::time::Duration) {
    use sysinfo::{Pid, ProcessesToUpdate, System};
    let mut sys = System::new();
    let target = Pid::from_u32(pid);
    let start = std::time::Instant::now();
    loop {
        sys.refresh_processes(ProcessesToUpdate::All);
        if sys.process(target).is_none() {
            return;
        }
        if start.elapsed() >= timeout {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_wait_for_exit_pid_finds_value_after_flag() {
        let args: Vec<String> = ["PengPort.exe", "--pengport-wait-for-exit", "1234"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(parse_wait_for_exit_pid(&args), Some(1234));
    }

    #[test]
    fn parse_wait_for_exit_pid_none_when_flag_absent() {
        let args: Vec<String> = ["PengPort.exe"].iter().map(|s| s.to_string()).collect();
        assert_eq!(parse_wait_for_exit_pid(&args), None);
    }

    #[test]
    fn parse_wait_for_exit_pid_none_when_value_missing_or_invalid() {
        let missing: Vec<String> = ["PengPort.exe", "--pengport-wait-for-exit"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(parse_wait_for_exit_pid(&missing), None);

        let invalid: Vec<String> = ["PengPort.exe", "--pengport-wait-for-exit", "not-a-number"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        assert_eq!(parse_wait_for_exit_pid(&invalid), None);
    }

    /// 존재하지 않는 pid는 이미 "종료된" 것과 같으므로 즉시 반환해야 한다(대기 없이) —
    /// 프로세스를 직접 띄우는 테스트는 이 세션에서 신뢰할 수 없었다고 확인돼
    /// (2026-08) 의도적으로 안 함, 순수 로직만 검증.
    #[test]
    fn wait_for_process_exit_returns_immediately_for_nonexistent_pid() {
        let start = std::time::Instant::now();
        // u32::MAX 는 실제 pid로 쓰일 일이 사실상 없다.
        wait_for_process_exit(u32::MAX, std::time::Duration::from_secs(5));
        assert!(
            start.elapsed() < std::time::Duration::from_secs(1),
            "존재하지 않는 pid인데도 타임아웃까지 기다림 — 폴링 로직 확인 필요"
        );
    }
}
