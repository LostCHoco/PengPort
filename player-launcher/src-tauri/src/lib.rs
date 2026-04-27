mod commands;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
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
            // updater
            commands::updater::get_update_token,
            commands::updater::set_update_token,
            commands::updater::update_token_source,
            commands::updater::validate_update_token,
            // secrets (OS keychain — instance_token)
            commands::secrets::instance_token_save,
            commands::secrets::instance_token_load,
            commands::secrets::instance_token_clear,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
