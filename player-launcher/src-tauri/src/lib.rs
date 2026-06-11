mod commands;

use tauri::Manager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 0.1.3 부터 사용자 데이터 폴더가 `app.pengport` → `PengPort` 로 변경됨.
    // Tauri webview 가 user data dir 에 lock 잡기 전에 옛 폴더를 새 이름으로 rename.
    commands::paths::migrate_legacy_app_dirs();

    #[allow(unused_mut)]
    let mut builder = tauri::Builder::default();

    // single_instance + deep_link 통합: deep link 클릭 시 OS 가 새 PengPort process 를 spawn
    // 하면서 `pengport://join?...` 를 argv 로 넘김. 두 번째 process 는 즉시 종료되고, 첫 번째
    // process 의 webview 가 focus 됨. URL forward 자체는 single_instance 의 `deep-link`
    // feature 가 자동 처리 — frontend 의 onOpenUrl listener 가 cold/hot 양쪽 다 받음.
    #[cfg(desktop)]
    {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // 두 번째 launch 가 떨어지면 첫 인스턴스의 메인 윈도우 포커스 — UX 보강 (URL 처리는
            // deep-link feature 가 자동).
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_focus();
            }
        }));
    }

    builder
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .setup(|app| {
            // Windows 의 NSIS installer 는 deep link scheme 을 OS 에 자동 등록 안 함.
            // 런타임에 register_all() 호출 — dev 환경 + production 모두 같은 흐름으로 작동.
            // 두 번째 launch 부터는 idempotent (같은 scheme 등록은 no-op).
            #[cfg(any(windows, target_os = "linux"))]
            {
                use tauri_plugin_deep_link::DeepLinkExt;
                if let Err(e) = app.deep_link().register_all() {
                    eprintln!("deep_link register_all 실패: {e}");
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Prism 탐지/설치 (PSP 의 third_party.prism-launcher 가 사용)
            commands::prism::detect_prism,
            commands::prism::download_prism,
            commands::prism::set_prism_override,
            commands::prism::remove_bundled_prism,
            commands::prism::stop_server,
            commands::prism::is_prism_instance_installed,
            commands::prism::is_service_running,
            // secrets (OS keychain — instance_token, multi-instance 별 격리)
            commands::secrets::instance_token_save,
            commands::secrets::instance_token_load,
            commands::secrets::instance_token_clear,
            commands::secrets::instance_token_migrate_legacy,
            // PSP
            commands::psp::psp_load_instance,
            commands::psp::psp_load_manifest,
            commands::psp::psp_load_catalog,
            commands::psp::invite_redeem,
            commands::psp::psp_validate_manifest,
            commands::psp::psp_invoke_action,
            commands::psp::psp_submit_form_with_data,
            commands::psp::psp_trust,
            commands::psp::psp_revoke_trust,
            commands::psp::psp_list_trusts,
            // 데이터 정리/언인스톨
            commands::maintenance::wipe_all_data,
            commands::maintenance::remove_prism_instance,
            commands::maintenance::uninstall_self,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
