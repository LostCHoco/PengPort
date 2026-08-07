mod commands;

use tauri::{Emitter, Manager};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 자체 업데이트 직후 재시작된 프로세스 — 옛 프로세스(같은 exe, 다른 pid)가
    // `tauri_plugin_single_instance` 잠금을 놓을 때까지 기다린 뒤에야 정상 초기화를
    // 계속한다(안 그러면 이 새 프로세스가 "이미 실행 중"으로 오인돼 옛 프로세스로
    // focus 만 넘기고 조용히 종료해버림). `commands::self_update` 모듈 설명 참고.
    let startup_args: Vec<String> = std::env::args().collect();
    if let Some(old_pid) = commands::self_update::parse_wait_for_exit_pid(&startup_args) {
        commands::self_update::wait_for_process_exit(old_pid, std::time::Duration::from_secs(10));
    }

    // 0.1.3 부터 사용자 데이터 폴더가 `app.pengport` → `PengPort` 로 변경됨.
    // Tauri webview 가 user data dir 에 lock 잡기 전에 옛 폴더를 새 이름으로 rename.
    commands::paths::migrate_legacy_app_dirs();
    // 2026-08 portable 전환: 옛 OS 표준 위치(%APPDATA%/%LOCALAPPDATA%)에 데이터가 남아있으면
    // exe 옆 portable 위치로 1 회 이전. 위 legacy rename 바로 다음 — 순서 중요.
    commands::paths::migrate_os_standard_data_to_portable();

    // 콜드 스타트(이 process 자체가 `.pengz` 경로를 argv 로 받으며 새로 뜬 경우) —
    // 프론트엔드가 mount 되기 전에 이벤트를 emit하면 놓칠 수 있어 상태로 잡아둔다.
    // frontend 가 mount 직후 `take_pending_pengz_file` 로 1회 조회.
    let initial_pengz = commands::file_import::find_pengz_arg(
        &std::env::args().collect::<Vec<_>>(),
    );

    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default();

    // single_instance: 두 번째 PengPort process 가 뜨면(예: `.pengz` 파일 더블클릭) 즉시
    // 종료되고 첫 번째 process 의 webview 가 focus 된다. `.pengz` 파일은 콜백에서 argv 를
    // 직접 검사해 이벤트로 emit — 이 시점엔 첫 인스턴스의 frontend 가 이미 떠 있으니
    // 레이스 없음(콜드 스타트와 다른 점, 위 `initial_pengz` 참고).
    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
            }
            if let Some(path) = commands::file_import::find_pengz_arg(&argv) {
                let _ = app.emit("pengz-file-opened", path);
            }
        }));
    }

    builder
        .plugin(tauri_plugin_opener::init())
        // updater 플러그인 자체(`app.updater()` 로 접근)만 쓴다 — Windows 에서 install()
        // 이 NSIS/MSI 전용이라 우리는 `commands::self_update`에서 download() 만 재사용하고
        // 설치는 직접 구현. process 플러그인은 그 커스텀 구현이 std::process 로 직접 자기
        // 자신을 재시작하므로 쓰지 않음(제거 완료, 2026-08 portable 전환).
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .manage(commands::file_import::PendingPengzFile(std::sync::Mutex::new(
            initial_pengz,
        )))
        .setup(|_app| {
            // `.pengz` 파일 확장자 연결을 런타임에 직접 등록(설치 프로그램이 아니라 이
            // 프로세스가 직접) — dev 환경에서도 동작하게.
            #[cfg(windows)]
            commands::paths::register_pengz_file_association();
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // third-party app 실행/모니터링/자동 다운로드 — app_id 하나로 등록된 모든
            // 앱을 다루는 범용 커맨드(옛 Prism 전용 commands::prism 은 삭제됨)
            commands::third_party_runtime::download_third_party_app,
            commands::third_party_runtime::stop_server,
            commands::third_party_runtime::is_service_running,
            // 앱 라이브러리 (0.2.0 — 레시피 목록 + `.pengz` 파일 임포트/내보내기 + 실행)
            commands::library::library_list,
            commands::library::library_get,
            commands::library::library_upsert,
            commands::library::library_get_local_root_override,
            commands::library::library_set_local_root_override,
            commands::library::library_get_selected_optional_groups,
            commands::library::library_set_selected_optional_groups,
            commands::library::scan_folder_relative_paths,
            commands::library::compute_file_sha256,
            commands::library::read_file_base64,
            commands::library::library_stage_manual_archive_file,
            commands::library::library_remove,
            commands::library::library_reorder,
            commands::file_import::library_export_file,
            commands::file_import::library_preview_import_file,
            commands::file_import::library_confirm_import_file,
            commands::file_import::take_pending_pengz_file,
            commands::library::library_install,
            commands::library::library_resolve_override_conflicts,
            commands::library::library_resolve_archive_conflicts,
            commands::library::library_cancel_install,
            commands::library::library_install_status,
            commands::library::library_install_diagnostics,
            commands::library::library_launch,
            commands::library::library_open_folder,
            commands::library::library_delete_installed_data,
            // third-party app 탐지/설정 — app_id 하나로 등록된 모든 앱을 다루는 범용 커맨드
            commands::library::list_third_party_app_ids,
            commands::library::list_third_party_apps,
            commands::library::list_third_party_app_descriptors,
            commands::library::detect_third_party_app,
            commands::library::configure_third_party_app_override,
            commands::library::remove_bundled_third_party_app,
            commands::library::third_party_app_upsert,
            commands::library::third_party_app_remove,
            // 데이터 정리/언인스톨
            commands::maintenance::wipe_all_data,
            commands::maintenance::remove_third_party_app_instance,
            commands::maintenance::uninstall_self,
            // 자체 업데이트 (portable rename-to-delete)
            commands::self_update::check_self_update,
            commands::self_update::install_self_update,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
