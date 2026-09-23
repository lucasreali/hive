// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// Wiring only (excluded from coverage, see COVERAGE_EXCLUSIONS.md): builder, plugins and
// command registration. Commands and any logic live in lib.rs, which is tested.
fn main() {
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .run(tauri::generate_context!());
    if let Err(error) = result {
        eprintln!("hive-app: {error}");
        std::process::exit(1);
    }
}
