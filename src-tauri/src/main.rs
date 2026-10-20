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
            let resource = |name: &str| app.path().resolve(name, BaseDirectory::Resource).ok();
            let var = |key: &str| std::env::var_os(key);
            let macos = cfg!(target_os = "macos");
            let (program, args, bundled) = hive_lib::bridge_command(macos, &var, resource("hive"));
            // Restarting runs the exit events, so the connection ends first (`on_run_event`).
            let handle = app.handle().clone();
            let exit = app.handle().clone();
            let hive = hive_lib::Hive::new(program, args)
                .with_bundled(bundled)
                .with_restart(move || handle.request_restart())
                .with_exit(move || exit.exit(0))
                .with_install(|update, bytes| update.install(bytes).map_err(|e| e.to_string()))
                .with_open(
                    |path: &str| tauri_plugin_opener::open_path(path, None::<&str>),
                    |path: &str| tauri_plugin_opener::reveal_item_in_dir(path),
                )
                .with_notify(notifier(app));
            // WSL or Windows (12.5.4), only on Windows.
            #[cfg(windows)]
            let hive = hive.with_modes(hive_lib::Modes::new(
                app.path().app_config_dir()?.join("mode"),
                list_wsl,
                hive_lib::native_bridge(&var, resource("hive.exe")),
            ));
            app.manage(hive);
            #[cfg(target_os = "macos")]
            ask_before_terminating(app.handle().clone());
            // The window (`"create": false` in the config) with its guards (open point #15): the
            // webview never leaves the app and never opens another window.
            let window = app.config().app.windows.first().cloned();
            let window = window.ok_or("hive-app: no window in tauri.conf.json")?;
            let dev = tauri::is_dev()
                .then(|| app.config().build.dev_url.clone())
                .flatten();
            tauri::WebviewWindowBuilder::from_config(app.handle(), &window)?
                .on_navigation(move |url| hive_lib::app_url(url, macos, dev.as_ref()))
                .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
                .build()?;
            #[cfg(windows)]
            disable_browser_keys(app);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::connect,
            commands::set_mode,
            commands::check_update,
            commands::install_update,
            commands::open_terminal,
            commands::write_terminal,
            commands::resize_terminal,
            commands::close_terminal,
            commands::ack_terminal,
            commands::list_projects,
            commands::add_project,
            commands::list_branches,
            commands::validate_worktree_name,
            commands::create_worktree,
            commands::remove_worktree,
            commands::rename_worktree,
            commands::watch_worktree,
            commands::unwatch_worktree,
            commands::expand_ignored,
            commands::set_view,
            commands::list_changes,
            commands::open_file,
            commands::search_files,
            commands::list_dirs,
            commands::list_sessions,
            commands::locate_session,
            commands::delete_session,
            commands::save_file,
            commands::create_file,
            commands::rename_file,
            commands::move_file,
            commands::delete_file,
            commands::create_folder,
            commands::open_in_editor,
            commands::open_path,
            commands::get_settings,
            commands::set_settings,
            commands::create_space,
            commands::update_space,
            commands::delete_space,
            commands::remove_project,
            commands::select_space,
            commands::list_gh_accounts,
            commands::switch_gh_account,
            commands::list_pulls,
            commands::open_pull,
            commands::act_on_pull,
            commands::create_pull,
            commands::list_runs,
            commands::open_run,
            commands::open_job_log,
            commands::act_on_run,
            commands::open_settings_file,
            commands::get_diagnostics,
            commands::show_notification,
        ])
        .build(tauri::generate_context!());
    match result {
        Ok(app) => app.run(|app, event| {
            // A click on the Dock icon with the window minimized or hidden brings it back.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen {
                has_visible_windows: false,
                ..
            } = event
            {
                hive_lib::show_window(app);
            }
            hive_lib::on_run_event(app, event)
        }),
        Err(error) => {
            eprintln!("hive-app: {error}");
            std::process::exit(1);
        }
    }
}

/// The app, for `should_terminate`, which AppKit calls without one.
#[cfg(target_os = "macos")]
static APP: std::sync::OnceLock<tauri::AppHandle> = std::sync::OnceLock::new();

/// Makes ⌘Q, the Dock's "Quit" and logging out ask `Hive::should_quit` first (D17.3): they
/// all reach `[NSApp terminate:]`, which quits at once unless the app delegate answers
/// `applicationShouldTerminate:`. tao's delegate (`TaoAppDelegateParent`) has no such method,
/// and Tauri sends no `ExitRequested` for them, so the method is added to its class here.
#[cfg(target_os = "macos")]
fn ask_before_terminating(app: tauri::AppHandle) {
    use std::ffi::{c_char, c_void};

    // The Objective-C runtime, which every macOS app links.
    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> *mut c_void;
        fn sel_registerName(name: *const c_char) -> *mut c_void;
        fn class_addMethod(
            class: *mut c_void,
            name: *mut c_void,
            imp: *const c_void,
            types: *const c_char,
        ) -> i8;
    }

    /// `NSApplicationTerminateReply`: `NSTerminateCancel` (0) keeps the app, which then asks
    /// the user; `NSTerminateNow` (1) quits. A logout the app cancels stops, as macOS does for
    /// any app that asks first.
    extern "C" fn should_terminate(_: *mut c_void, _: *mut c_void, _: *mut c_void) -> usize {
        let quit = APP
            .get()
            .is_none_or(|app| app.state::<hive_lib::Hive>().should_quit(app));
        usize::from(quit)
    }

    let _ = APP.set(app);
    // SAFETY: C strings with their NUL; the method's types ("Q@:@": NSUInteger from self, _cmd
    // and the sender) match `should_terminate`'s signature.
    let added = unsafe {
        let class = objc_getClass(c"TaoAppDelegateParent".as_ptr());
        !class.is_null()
            && class_addMethod(
                class,
                sel_registerName(c"applicationShouldTerminate:".as_ptr()),
                should_terminate as *const c_void,
                c"Q@:@".as_ptr(),
            ) != 0
    };
    if !added {
        eprintln!("hive-app: ⌘Q cannot ask before quitting: no app delegate to extend");
    }
}

/// `wsl.exe -l -q`: the WSL distributions, if any (`hive_lib::Modes` runs it, time-limited),
/// without a console window.
#[cfg(windows)]
fn list_wsl() -> std::process::Command {
    use std::os::windows::process::CommandExt;
    let mut command = std::process::Command::new("wsl.exe");
    command.args(["-l", "-q"]).creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    command
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

/// Windows toasts as the installed app (13.5); a click runs `click` on the toast's thread. Silent:
/// Hive plays its own tone.
#[cfg(windows)]
fn notifier(
    app: &tauri::App,
) -> impl Fn(&str, &str, hive_lib::Click) -> Result<(), String> + Send + Sync + 'static {
    use tauri_winrt_notification::Toast;
    let identifier = &app.config().identifier;
    let id = hive_lib::notifier(tauri::is_dev(), identifier, Toast::POWERSHELL_APP_ID).to_owned();
    move |title: &str, body: &str, click: hive_lib::Click| {
        Toast::new(&id)
            .title(title)
            .text1(body)
            .sound(None)
            .on_activated(move |_| {
                click();
                Ok(())
            })
            .show()
            .map_err(|error| error.to_string())
    }
}

/// macOS notifications as the app's bundle (13.5), silent (Hive plays its own tone); `send`
/// blocks until the click or the dismissal, so `waiting_notifier` runs each on a thread.
#[cfg(target_os = "macos")]
fn notifier(
    app: &tauri::App,
) -> impl Fn(&str, &str, hive_lib::Click) -> Result<(), String> + Send + Sync + 'static {
    use mac_notification_sys::{Notification, NotificationResponse};
    let identifier = &app.config().identifier;
    let id = hive_lib::notifier(tauri::is_dev(), identifier, "com.apple.Terminal");
    if let Err(error) = mac_notification_sys::set_application(id) {
        eprintln!("hive-app: notifications cannot show as {id}: {error}");
    }
    hive_lib::waiting_notifier(|title: &str, body: &str, wait: bool| {
        let sent = Notification::new()
            .title(title)
            .message(body)
            .wait_for_click(wait)
            .send();
        sent.map(|response| response == NotificationResponse::Click)
            .map_err(|error| error.to_string())
    })
}

/// Linux (development only): the notification plugin's, which reports no click.
#[cfg(not(any(windows, target_os = "macos")))]
fn notifier(
    app: &tauri::App,
) -> impl Fn(&str, &str, hive_lib::Click) -> Result<(), String> + Send + Sync + 'static {
    use tauri_plugin_notification::NotificationExt;
    let app = app.handle().clone();
    move |title: &str, body: &str, _: hive_lib::Click| {
        let builder = app.notification().builder().title(title).body(body);
        builder.show().map_err(|error| error.to_string())
    }
}
