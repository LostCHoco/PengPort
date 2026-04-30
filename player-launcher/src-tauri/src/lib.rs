mod commands;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 0.1.3 부터 사용자 데이터 폴더가 `app.pengport` → `PengPort` 로 변경됨.
    // Tauri webview 가 user data dir 에 lock 잡기 전에 옛 폴더를 새 이름으로 rename.
    commands::paths::migrate_legacy_app_dirs();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_dialog::init())
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
