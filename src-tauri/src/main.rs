// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use hive_lib::{commands, tray};
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::path::BaseDirectory;
use tauri::tray::{TrayIcon, TrayIconBuilder};
use tauri::Manager;

// Wiring only (excluded from coverage, see COVERAGE_EXCLUSIONS.md): builder, plugins, state and
// command registration. Commands and any logic live in lib.rs, which is tested.
fn main() {
    // A panic goes to stderr, which `setup` sends to the app's log (D18.6).
    #[cfg(target_os = "macos")]
    std::panic::set_hook(hive_lib::log::hook(|text| eprint!("{text}")));
    // A GUI app starts with 256 descriptors on macOS (17.6).
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    hive_lib::fds::raise_at_start(&hive_lib::fds::Process, &mut std::io::stderr());
    let mut context = tauri::generate_context!();
    let identifier = hive_lib::instance_identifier(tauri::is_dev(), &context.config().identifier);
    context.config_mut().identifier = identifier;
    let builder = tauri::Builder::default()
        // First, so a second launch exits before anything else starts (17.18).
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            hive_lib::bring_up(app);
        }))
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(hive_lib::WINDOW_STATE)
                .build(),
        )
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_updater::Builder::new().build());
    // A WebContent crash reloads the page and keeps the terminals (18.21).
    #[cfg(target_os = "macos")]
    let builder = builder.on_web_content_process_terminate(hive_lib::web_content_terminated);
    let result = builder
        .on_menu_event(|app, event| hive_lib::menu::on_menu(app, event.id().as_ref()))
        .setup(|app| {
            // Here, past the single-instance check: a second launch never rotates the log.
            #[cfg(target_os = "macos")]
            if let Err(error) = hive_lib::log::start(std::env::var_os("HOME"), tauri::is_dev(), 2) {
                eprintln!("hive-app: no app log: {error}");
            }
            #[cfg(target_os = "macos")]
            app.set_menu(app_menu(app.handle())?)?;
            let resource = |name: &str| app.path().resolve(name, BaseDirectory::Resource).ok();
            let var = |key: &str| std::env::var_os(key);
            let macos = cfg!(target_os = "macos");
            let (program, args, bundled) = hive_lib::bridge_command(macos, &var, resource("hive"));
            // Restarting runs the exit events, so the connection ends first (`on_run_event`).
            let handle = app.handle().clone();
            let exit = app.handle().clone();
            let translocated = hive_lib::translocated(macos, std::env::current_exe().ok());
            let hive = hive_lib::Hive::new(program, args)
                .with_bundled(bundled)
                .with_translocated(translocated)
                .with_restart(move || handle.request_restart())
                .with_exit(move || exit.exit(0))
                .with_install(|update, bytes| update.install(bytes).map_err(|e| e.to_string()))
                .with_update_dir(app.path().app_cache_dir()?)
                .with_open(
                    |path: &str| tauri_plugin_opener::open_path(path, None::<&str>),
                    |path: &str| tauri_plugin_opener::reveal_item_in_dir(path),
                )
                .with_notify(notifier(app))
                .with_url(|url: &str| {
                    tauri_plugin_opener::open_url(url, None::<&str>).map_err(|e| e.to_string())
                })
                .with_tray(tray(app));
            // Asks for notification permission at launch; the tone plays from the app (17.19).
            #[cfg(target_os = "macos")]
            let hive = hive
                .with_permission(hive_lib::macos::permissions(
                    std::env::current_exe().ok().as_deref(),
                    hive_lib::macos::CENTER,
                ))
                .with_tone(
                    app.path().app_cache_dir()?,
                    hive_lib::afplay("/usr/bin/afplay".into()),
                )
                // ⌘V pastes the paths of the files copied in Finder (18.3).
                .with_clipboard(hive_lib::macos::Pasteboard::general());
            // WSL or Windows (12.5.4), only on Windows.
            #[cfg(windows)]
            let hive = hive.with_modes(hive_lib::Modes::new(
                app.path().app_config_dir()?.join("mode"),
                list_wsl,
                hive_lib::native_bridge(&var, resource("hive.exe")),
            ));
            app.manage(hive);
            #[cfg(target_os = "macos")]
            {
                let handle = app.handle().clone();
                let class = c"TaoAppDelegateParent";
                let ask = move || hive_lib::should_quit(&handle);
                if !hive_lib::macos::ask_before_terminating(class, ask) {
                    eprintln!("hive-app: ⌘Q cannot ask before quitting: no app delegate to extend");
                }
            }
            // The window (`"create": false` in the config) with its guards (open point #15): the
            // webview never leaves the app and never opens another window.
            let window = app.config().app.windows.first().cloned();
            let window = window.ok_or("hive-app: no window in tauri.conf.json")?;
            // The traffic lights centred in the title bar as this macOS lays them out (18.4);
            // tao and wry keep them there (each redraw, leaving full screen).
            #[cfg(target_os = "macos")]
            let window = tauri::utils::config::WindowConfig {
                traffic_light_position: hive_lib::macos::traffic_lights(c"NSWindow")
                    .or(window.traffic_light_position),
                ..window
            };
            let dev = tauri::is_dev()
                .then(|| app.config().build.dev_url.clone())
                .flatten();
            let _window = tauri::WebviewWindowBuilder::from_config(app.handle(), &window)?
                .on_navigation(move |url| hive_lib::app_url(url, macos, dev.as_ref()))
                .on_new_window(|_, _| tauri::webview::NewWindowResponse::Deny)
                .build()?;
            // Files dropped from Finder type their paths into the terminal under them (18.1).
            #[cfg(target_os = "macos")]
            {
                let handle = app.handle().clone();
                let drop = move |paths: Vec<String>, x, y| {
                    hive_lib::files_dropped(&handle, &paths, x, y);
                };
                _window.with_webview(move |webview| {
                    // SAFETY: the window's WKWebView, on the main thread (`with_webview`).
                    let taken = unsafe { hive_lib::macos::take_file_drops(webview.inner(), drop) };
                    if !taken {
                        eprintln!("hive-app: files dropped from Finder cannot reach a terminal");
                    }
                })?;
            }
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
            commands::attach_terminal,
            commands::write_terminal,
            commands::resize_terminal,
            commands::close_terminal,
            commands::ack_terminal,
            commands::list_projects,
            commands::add_project,
            commands::list_branches,
            commands::validate_worktree_name,
            commands::create_worktree,
            commands::create_issue_worktree,
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
            commands::title_bar_action,
            commands::show_notification,
            commands::check_notifications,
            commands::open_notification_settings,
            commands::play_tone,
            commands::set_tray,
            commands::pasted_files,
        ])
        .build(context);
    match result {
        Ok(app) => app.run(|app, event| {
            // Tauri's `RunEvent::Reopen` cannot be built outside it, so a test could not send it
            // to `on_run_event`: unpacked here, decided in the tested `reopen`.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen {
                has_visible_windows,
                ..
            } = event
            {
                hive_lib::reopen(app, has_visible_windows);
            }
            hive_lib::on_run_event(app, event)
        }),
        Err(error) => {
            eprintln!("hive-app: {error}");
            std::process::exit(1);
        }
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

/// macOS notifications (13.5, 19.10), silent (Hive plays its own tone): an installed app posts
/// them on `UNUserNotificationCenter` and gets their clicks through its delegate, set once here;
/// nothing waits for a click. A development build cannot use the center (it raises an exception
/// outside an app bundle) and shows the notification plugin's, without clicks.
#[cfg(target_os = "macos")]
fn notifier(app: &tauri::App) -> Box<hive_lib::Notifier> {
    let exe = std::env::current_exe().ok();
    if !exe.is_some_and(|exe| hive_lib::in_app_bundle(&exe)) {
        return Box::new(plugin_notifier(app));
    }
    let center = hive_lib::macos::CENTER;
    let (notify, clicked) = hive_lib::identified_notifier(move |id, title, body| {
        hive_lib::macos::post(center, id, title, body)
    });
    if !hive_lib::macos::receive_clicks(center, hive_lib::macos::DELEGATE_CLASS, clicked) {
        eprintln!("hive-app: notification clicks cannot reach Hive");
    }
    notify
}

/// Linux (development only): the notification plugin's, which reports no click.
#[cfg(not(any(windows, target_os = "macos")))]
fn notifier(
    app: &tauri::App,
) -> impl Fn(&str, &str, hive_lib::Click) -> Result<(), String> + Send + Sync + 'static {
    plugin_notifier(app)
}

/// The notification plugin's notifications, which report no click.
#[cfg(not(windows))]
fn plugin_notifier(
    app: &tauri::App,
) -> impl Fn(&str, &str, hive_lib::Click) -> Result<(), String> + Send + Sync + 'static {
    use tauri_plugin_notification::NotificationExt;
    let app = app.handle().clone();
    move |title: &str, body: &str, _: hive_lib::Click| {
        let builder = app.notification().builder().title(title).body(body);
        builder.show().map_err(|error| error.to_string())
    }
}

/// `hive_lib::menu::MENUS` as Tauri's menu (17.13): item for item, in order.
#[cfg(target_os = "macos")]
fn app_menu(app: &tauri::AppHandle) -> tauri::Result<tauri::menu::Menu<tauri::Wry>> {
    use hive_lib::menu::{Item, Native, Role, MENUS};
    use tauri::menu::{
        AboutMetadata, IsMenuItem, Menu, MenuItem, PredefinedMenuItem as P, Submenu,
    };
    let menu = Menu::new(app)?;
    for spec in MENUS {
        let mut items: Vec<Box<dyn IsMenuItem<tauri::Wry>>> = Vec::new();
        for item in spec.items {
            items.push(match *item {
                Item::Own { id, text, keys } => {
                    Box::new(MenuItem::with_id(app, id, text, true, keys)?)
                }
                Item::Separator => Box::new(P::separator(app)?),
                Item::Native(native) => Box::new(match native {
                    Native::About => P::about(app, None, Some(AboutMetadata::default()))?,
                    Native::Services => P::services(app, None)?,
                    Native::Hide => P::hide(app, None)?,
                    Native::HideOthers => P::hide_others(app, None)?,
                    Native::ShowAll => P::show_all(app, None)?,
                    Native::Undo => P::undo(app, None)?,
                    Native::Redo => P::redo(app, None)?,
                    Native::Cut => P::cut(app, None)?,
                    Native::Copy => P::copy(app, None)?,
                    Native::Paste => P::paste(app, None)?,
                    Native::SelectAll => P::select_all(app, None)?,
                    Native::Fullscreen => P::fullscreen(app, None)?,
                    Native::Minimize => P::minimize(app, None)?,
                    Native::Zoom => P::maximize(app, None)?,
                }),
            });
        }
        let refs: Vec<&dyn IsMenuItem<tauri::Wry>> = items.iter().map(|item| &**item).collect();
        let submenu = Submenu::with_items(app, spec.title, true, &refs)?;
        match spec.role {
            Role::Window => submenu.set_as_windows_menu_for_nsapp()?,
            Role::Help => submenu.set_as_help_menu_for_nsapp()?,
            Role::Plain => {}
        }
        menu.append(&submenu)?;
    }
    Ok(menu)
}

/// The menu bar icon (17.22): shows `hive_lib::tray`'s view, or removes it. Choosing an item
/// goes to `hive_lib::tray::chosen`.
fn tray(app: &tauri::App) -> impl Fn(tray::Tray) -> Result<(), String> + Send + Sync {
    let app = app.handle().clone();
    move |wanted| match wanted {
        tray::Tray::Remove => {
            app.remove_tray_by_id(tray::TRAY);
            Ok(())
        }
        tray::Tray::Show(view) => tray::place(
            app.tray_by_id(tray::TRAY),
            view,
            |shown, view| update_tray(&app, &shown, view).map_err(|e| e.to_string()),
            |view| create_tray(&app, view).map_err(|e| e.to_string()),
        ),
    }
}

/// The view's menu: one item per entry, as `hive_lib::tray` made it.
fn tray_menu(app: &tauri::AppHandle, view: &tray::View) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;
    for entry in &view.entries {
        match entry {
            tray::Entry::Item { id, text, enabled } => {
                menu.append(&MenuItem::with_id(app, id, text, *enabled, None::<&str>)?)?
            }
            tray::Entry::Separator => menu.append(&PredefinedMenuItem::separator(app)?)?,
        }
    }
    Ok(menu)
}

fn update_tray(app: &tauri::AppHandle, shown: &TrayIcon, view: tray::View) -> tauri::Result<()> {
    let menu = tray_menu(app, &view)?;
    // Only when the most urgent state changed (`hive_lib::tray::Shown`).
    if let Some(icon) = view.icon {
        shown.set_icon(Some(Image::new_owned(icon.rgba, tray::SIZE, tray::SIZE)))?;
        shown.set_icon_as_template(icon.template)?;
    }
    shown.set_tooltip(Some(view.tooltip))?;
    shown.set_menu(Some(menu))
}

fn create_tray(app: &tauri::AppHandle, view: tray::View) -> tauri::Result<()> {
    let menu = tray_menu(app, &view)?;
    let mut builder = TrayIconBuilder::with_id(tray::TRAY);
    // Always there: `hive_lib::tray::place` creates no tray without one.
    if let Some(icon) = view.icon {
        builder = builder
            .icon(Image::new_owned(icon.rgba, tray::SIZE, tray::SIZE))
            .icon_as_template(icon.template);
    }
    builder
        .tooltip(view.tooltip)
        .menu(&menu)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| {
            let _ = tray::chosen(app, &app.state::<hive_lib::Hive>(), event.id().as_ref());
        })
        .build(app)
        .map(drop)
}
