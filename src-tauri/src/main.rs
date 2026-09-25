// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use hive_lib::commands;
use tauri::path::BaseDirectory;
use tauri::Manager;

// Wiring only (excluded from coverage, see COVERAGE_EXCLUSIONS.md): builder, plugins, state and
// command registration. Commands and any logic live in lib.rs, which is tested.
fn main() {
    let result = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let bundled = app.path().resolve("hive", BaseDirectory::Resource).ok();
            let macos = cfg!(target_os = "macos");
            let (program, args) =
                hive_lib::bridge_command(macos, &|key| std::env::var_os(key), bundled);
            // Restarting runs the exit events, so the connection ends first (`on_run_event`).
            let handle = app.handle().clone();
            let hive = hive_lib::Hive::new(program, args)
                .with_restart(move || handle.request_restart())
                .with_install(|update, bytes| update.install(bytes).map_err(|e| e.to_string()));
            app.manage(hive);
            #[cfg(windows)]
            disable_browser_keys(app);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::connect,
            commands::check_update,
            commands::install_update,
            commands::open_terminal,
            commands::write_terminal,
            commands::resize_terminal,
            commands::close_terminal,
            commands::list_projects,
            commands::add_project,
            commands::list_branches,
            commands::validate_worktree_name,
            commands::create_worktree,
            commands::remove_worktree,
            commands::rename_worktree,
            commands::watch_worktree,
            commands::unwatch_worktree,
            commands::set_view,
            commands::list_changes,
            commands::open_file,
            commands::search_files,
            commands::list_dirs,
            commands::list_sessions,
            commands::locate_session,
            commands::delete_session,
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

/// Turns off WebView2's browser keys (Ctrl+P print, F5/Ctrl+R reload, Ctrl+F find, F12, Alt+←/→…).
/// Tauri does not expose wry's `with_browser_accelerator_keys`, so this sets it on the WebView2
/// settings directly. Editing keys and the page's own key handlers keep working.
#[cfg(windows)]
fn disable_browser_keys(app: &tauri::App) {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings3;
    use windows_core::Interface;

    let Some(window) = app.get_webview_window("main") else {
        eprintln!("hive-app: no main window to turn browser keys off");
        return;
    };
    let result = window.with_webview(|webview| {
        // SAFETY: COM calls on the live controller, on the webview's own thread.
        let result = unsafe {
            webview
                .controller()
                .CoreWebView2()
                .and_then(|core| core.Settings())
                .and_then(|settings| settings.cast::<ICoreWebView2Settings3>())
                .and_then(|settings| settings.SetAreBrowserAcceleratorKeysEnabled(false))
        };
        if let Err(error) = result {
            eprintln!("hive-app: could not turn browser keys off: {error}");
        }
    });
    if let Err(error) = result {
        eprintln!("hive-app: could not turn browser keys off: {error}");
    }
}
