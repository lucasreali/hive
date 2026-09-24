// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use hive_lib::commands;

// Wiring only (excluded from coverage, see COVERAGE_EXCLUSIONS.md): builder, plugins, state and
// command registration. Commands and any logic live in lib.rs, which is tested.
fn main() {
    let (program, args) = hive_lib::bridge_command(|key| std::env::var_os(key));
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .manage(hive_lib::Hive::new(program, args))
        .invoke_handler(tauri::generate_handler![
            commands::connect,
            commands::open_terminal,
            commands::write_terminal,
            commands::resize_terminal,
            commands::close_terminal,
            commands::list_projects,
            commands::add_project,
            commands::list_branches,
            commands::validate_worktree_name,
            commands::create_worktree,
            commands::watch_worktree,
            commands::unwatch_worktree,
            commands::list_changes,
            commands::open_file,
            commands::save_file,
            commands::open_in_editor,
        ])
        .build(tauri::generate_context!());
    match result {
        Ok(app) => app.run(hive_lib::on_run_event),
        Err(error) => {
            eprintln!("hive-app: {error}");
            std::process::exit(1);
        }
    }
}
