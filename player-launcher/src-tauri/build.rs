fn main() {
    // PENGPORT_UPDATES_TOKEN 이 바뀌면 자동 재빌드 (option_env! 결과 갱신).
    println!("cargo:rerun-if-env-changed=PENGPORT_UPDATES_TOKEN");
    tauri_build::build()
}
