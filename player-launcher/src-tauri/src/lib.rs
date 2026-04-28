mod commands;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Updater plugin 의 Authorization 헤더 — Caddy /updates/* Bearer 보호.
    // current_token() 은 keyring 우선, 없으면 빌드 임베드 (PENGPORT_UPDATES_TOKEN).
    // 빈 문자열이면 헤더 추가 없이 빌드 (build).
    let updater_token = commands::updater::current_token();
    let mut updater_builder = tauri_plugin_updater::Builder::new();
    if !updater_token.is_empty() {
        updater_builder = updater_builder
            .header(
                "Authorization",
                format!("Bearer {updater_token}"),
            )
            .expect("invalid Authorization header value");
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(updater_builder.build())
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
