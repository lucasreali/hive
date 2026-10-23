//! Testable code of the Tauri app: the link to the service through `hive bridge` (#14, #24)
//! and the commands the UI calls. `main.rs` only wires it.
//!
//! Control messages go to the UI on one `Channel` given by `connect`, as the service's JSON
//! plus a `channel` field. Terminal output goes, as raw bytes, to the `Channel` given for that
//! terminal by `open_terminal`. No Tauri events are used.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::OsString;
use std::future::Future;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use hive_protocol::{
    AccountDir, AgentState, Control, DiffBase, Frame, FrameCodec, FrameError, FrameType, GhAccount,
    PullAction, Role, RunAction, SessionTarget, Settings, SpaceEnv, MAX_PAYLOAD, PROTOCOL_VERSION,
};
use serde_json::{json, Value};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Manager, RunEvent, Runtime};
use tauri_plugin_updater::{Update, Updater};
use tauri_plugin_window_state::StateFlags;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::codec::{FramedRead, FramedWrite};

/// The app's descriptor limit (17.6).
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod fds;

/// App version, compared with the `hive` binary's in the handshake (#29).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Runs inside WSL through `wsl.exe --exec`, so no login or interactive shell (and no fish
/// config) runs first; on macOS through `/bin/sh` directly. `$1` is the `HIVE_BRIDGE` override
/// and `$2` the path of the `hive` the installer bundles (a Windows path under WSL, a POSIX one
/// on macOS), each passed as an argument (maybe empty). The bundled one is copied into Hive's
/// bin dir when it differs from the copy there, through a temporary file and a rename, so a
/// running `hive` keeps its file, and hooks keep a stable path even when macOS runs the app
/// from a translocated one. `xattr` (macOS) drops the copy's quarantine; where it is missing the
/// failure is ignored. Without either, `cargo install`'s (development).
const BRIDGE_SCRIPT: &str = r#"[ -n "$1" ] && exec "$1" bridge
hive=$HOME/.cargo/bin/hive
if [ -n "$2" ]; then
  hive=${XDG_DATA_HOME:-$HOME/.local/share}/hive/bin/hive
  case "$2" in
    /*) src=$2 ;;
    *) src=$(wslpath -u "${2#'\\?\'}") || exit 1 ;;
  esac
  if ! cmp -s "$src" "$hive"; then
    mkdir -p "${hive%/*}" && cp "$src" "$hive.new" && chmod 755 "$hive.new" || exit 1
    xattr -d com.apple.quarantine "$hive.new" 2>/dev/null
    mv -f "$hive.new" "$hive" || exit 1
  fi
fi
exec "$hive" bridge"#;

/// How much of the bridge's stderr is kept as the disconnect reason.
const STDERR_LIMIT: u64 = 16_384;
/// How long to wait for the bridge to exit and flush stderr once its stdout closed.
const EXIT_WAIT: Duration = Duration::from_secs(2);

const NOT_CONNECTED: &str = "not connected to the hive service";
/// How many paths the service sent (`editor_target`, `session_located`) wait for `open_path`.
const APPROVED_LIMIT: usize = 16;
/// How long `wsl.exe -l -q` may take (12.5.4): it runs before the window shows, and a wedged WSL
/// service would otherwise keep the app from starting.
const LIST_WAIT: Duration = Duration::from_secs(3);
/// How often the listing is checked for its end.
const LIST_POLL: Duration = Duration::from_millis(10);
/// The most bytes read from the `mode` file; a longer file holds no mode.
const MODE_LIMIT: u64 = 16;

/// Program and arguments that start `hive bridge`, and whether that runs the `hive` the installer
/// bundles (an installed app) rather than a development one, for the connection dialog (12.4).
pub type Bridge = (OsString, Vec<OsString>, bool);

/// Program and arguments that start `hive bridge`: from Windows through WSL (#14, 4.18), or
/// natively when `macos` (5.2). `HIVE_WSL_DISTRO` picks the WSL distribution (Windows only)
/// and `HIVE_BRIDGE` the `hive` binary (an absolute path, for development). `bundled` is where
/// the installer put `hive`, used when that file exists. Nothing is ever spliced into the script.
/// The flag says whether the bridge runs that bundled `hive` (an installed app) rather than
/// `HIVE_BRIDGE` or `cargo install`'s (development), for the connection dialog (12.4).
pub fn bridge_command(
    macos: bool,
    var: &dyn Fn(&str) -> Option<OsString>,
    bundled: Option<PathBuf>,
) -> Bridge {
    let var = |key| var(key).filter(|value| !value.is_empty());
    let mut args = Vec::new();
    if !macos {
        if let Some(distro) = var("HIVE_WSL_DISTRO") {
            args.extend([OsString::from("-d"), distro]);
        }
        args.extend(["--exec", "/bin/sh"].map(OsString::from));
    }
    args.extend(["-c", BRIDGE_SCRIPT, "sh"].map(OsString::from));
    let over = var("HIVE_BRIDGE");
    let bundled = bundled.filter(|path| path.is_file());
    let runs_bundled = over.is_none() && bundled.is_some();
    args.push(over.unwrap_or_default());
    args.push(bundled.unwrap_or_default().into());
    let program = if macos { "/bin/sh" } else { "wsl.exe" };
    (program.into(), args, runs_bundled)
}

/// `hive.exe bridge` run directly, for the service on Windows itself (12.5.4): the `hive.exe`
/// the installer bundles next to the app when that file exists, else `HIVE_BRIDGE` (a Windows
/// path, development), else the one on `PATH` (`cargo install`'s).
pub fn native_bridge(var: &dyn Fn(&str) -> Option<OsString>, bundled: Option<PathBuf>) -> Bridge {
    let bundled = bundled.filter(|path| path.is_file());
    let runs_bundled = bundled.is_some();
    let program = bundled
        .map(PathBuf::into_os_string)
        .or_else(|| var("HIVE_BRIDGE").filter(|value| !value.is_empty()))
        .unwrap_or_else(|| "hive.exe".into());
    (program, vec!["bridge".into()], runs_bundled)
}

/// Where the service runs on Windows (12.5.4): inside WSL, or on Windows itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Wsl,
    Native,
}

impl Mode {
    /// Its name in the mode file and to the UI.
    fn name(self) -> &'static str {
        match self {
            Self::Wsl => "wsl",
            Self::Native => "native",
        }
    }

    fn parse(name: &str) -> Option<Self> {
        [Self::Wsl, Self::Native]
            .into_iter()
            .find(|mode| mode.name() == name)
    }
}

/// The choice between the service in WSL and on Windows itself (12.5.4). The app keeps it, since
/// it is needed before any service starts. Each mode has a service of its own, so its own
/// projects, spaces and settings.
pub struct Modes {
    /// The file the chosen mode is kept in (`mode` in the app's config folder).
    file: PathBuf,
    /// Whether WSL has a distribution. Without one the service runs on Windows, never asking.
    wsl: bool,
    /// The bridge on Windows itself (`native_bridge`).
    native: Bridge,
}

impl Modes {
    /// The choice on Windows, kept in `file`. `list` gives the `wsl.exe -l -q` command, which
    /// tells whether WSL is there (without it the service runs on Windows, never asking); it is
    /// not run with Windows saved: WSL was there when it was chosen, and a wedged WSL must not
    /// hold up a start that does not need it. The listing gets [`LIST_WAIT`].
    pub fn new(
        file: PathBuf,
        list: impl FnOnce() -> std::process::Command,
        native: Bridge,
    ) -> Self {
        let native_saved = saved(&file) == Some(Mode::Native);
        Self {
            wsl: native_saved || has_wsl(output_within(&mut list(), LIST_WAIT, reader)),
            file,
            native,
        }
    }

    /// The mode to start in: the saved one, or `None` to ask first; without WSL always Windows.
    fn start(&self) -> Option<Mode> {
        if !self.wsl {
            return Some(Mode::Native);
        }
        saved(&self.file)
    }

    fn save(&self, mode: Mode) -> std::io::Result<()> {
        // Always a folder (the app's config folder): `create_dir_all("")` does nothing.
        let folder = self.file.parent().unwrap_or(std::path::Path::new(""));
        std::fs::create_dir_all(folder)?;
        std::fs::write(&self.file, mode.name())
    }
}

/// The mode saved in `file`, if any. At most [`MODE_LIMIT`] bytes are read.
fn saved(file: &std::path::Path) -> Option<Mode> {
    let file = std::fs::File::open(file).ok()?;
    let mut text = String::new();
    file.take(MODE_LIMIT + 1).read_to_string(&mut text).ok()?;
    let fits = text.len() as u64 <= MODE_LIMIT;
    fits.then(|| Mode::parse(text.trim())).flatten()
}

/// Runs `command` for its stdout, killing it after about `limit` (then `TimedOut`): both its exit
/// and the end of its output must come by then, so a program it left holding the output open
/// cannot hold up the start either. Counted in polls rather than read off a clock, so nothing
/// can make the wait endless. `start` runs the output's reader on a thread ([`reader`]).
fn output_within(
    command: &mut std::process::Command,
    limit: Duration,
    start: Start,
) -> std::io::Result<Output> {
    // The output is read on a thread of its own, started first so that nothing needs undoing
    // when it cannot start. It ends once every holder of the output closed it (ponytail: left
    // blocked until then after a time-out, one idle thread).
    let (give, pipe) = std::sync::mpsc::channel::<Option<std::process::ChildStdout>>();
    let (send, read) = std::sync::mpsc::channel();
    start(Box::new(move || {
        let mut out = Vec::new();
        if let Ok(Some(mut stdout)) = pipe.recv() {
            let _ = stdout.read_to_end(&mut out);
        }
        let _ = send.send(out);
    }))?;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let _ = give.send(child.stdout.take());
    let mut stdout = None;
    for _ in 0..limit.as_millis() / LIST_POLL.as_millis() {
        stdout = stdout.or_else(|| read.try_recv().ok());
        if let (Some(stdout), Some(status)) = (&stdout, child.try_wait()?) {
            let stdout = stdout.clone();
            return Ok(Output {
                status,
                stdout,
                stderr: Vec::new(),
            });
        }
        std::thread::sleep(LIST_POLL);
    }
    let _ = child.kill();
    let _ = child.wait();
    Err(std::io::ErrorKind::TimedOut.into())
}

/// Runs work on a thread of its own ([`reader`]; a failing one in the tests).
type Start = fn(Box<dyn FnOnce() + Send>) -> std::io::Result<()>;

/// Starts `output_within`'s reader on a new thread.
fn reader(work: Box<dyn FnOnce() + Send>) -> std::io::Result<()> {
    let thread = std::thread::Builder::new().name("wsl-list".into());
    thread.spawn(work).map(drop)
}

/// Whether `wsl.exe -l -q` listed a distribution. It prints their names in UTF-16 (UTF-8 with
/// `WSL_UTF8=1`), one per line, and fails or prints nothing when WSL or every distribution is
/// missing. A name always holds a letter or digit, an ASCII byte in either encoding. A listing
/// killed at its time limit leaves WSL unknown, counted as there: the saved mode starts, else
/// the first run asks, so a slow WSL never moves a WSL user to Windows.
fn has_wsl(list: std::io::Result<Output>) -> bool {
    match list {
        Ok(out) => out.status.success() && out.stdout.iter().any(u8::is_ascii_alphanumeric),
        Err(error) => error.kind() == std::io::ErrorKind::TimedOut,
    }
}

/// Whether the webview may load `url` (open point #15): only the app itself, from the one origin
/// Tauri serves it from (`tauri://localhost` when `macos`, `http://tauri.localhost` on Windows)
/// or, in a development build, from the dev server `dev`. Links open outside, through the
/// opener plugin.
pub fn app_url(url: &tauri::Url, macos: bool, dev: Option<&tauri::Url>) -> bool {
    let (scheme, host) = if macos {
        ("tauri", "localhost")
    } else {
        ("http", "tauri.localhost")
    };
    let bundled = url.scheme() == scheme && url.host_str() == Some(host) && url.port().is_none();
    bundled || dev.is_some_and(|dev| dev.origin() == url.origin())
}

/// Whether the app at `exe` runs translocated (17.10): macOS runs a quarantined app opened
/// where it was downloaded from a random read-only copy under `…/AppTranslocation/…`, where
/// the updater cannot replace it. A path check is enough: that folder exists only for this.
/// Only on macOS (`main.rs` passes `cfg!(target_os = "macos")`); an unknown `exe` is not.
pub fn translocated(macos: bool, exe: Option<std::path::PathBuf>) -> bool {
    macos
        && exe.is_some_and(|exe| {
            exe.components()
                .any(|part| part.as_os_str() == "AppTranslocation")
        })
}

/// Why an update does not install while translocated; the UI explains the fix (17.10).
pub const TRANSLOCATED: &str =
    "Hive runs from a temporary read-only copy; move it to Applications to update";

/// Tauri state: the bridge command and the live link to the service.
pub struct Hive {
    /// `hive bridge` through WSL, or natively on macOS (`bridge_command`).
    bridge: Bridge,
    /// The choice of WSL or Windows (12.5.4); none on macOS.
    modes: Option<Modes>,
    link: Arc<Mutex<Link>>,
    /// Held through a whole `reconnect`, so two mode switches never overlap: one would start its
    /// bridge while the other still ends the old one, whose end then cuts the new connection.
    reconnecting: tokio::sync::Mutex<()>,
    /// Restarts the app once an update is installed; given by `main.rs` (`with_restart`).
    restart: Option<Box<dyn Fn() + Send + Sync>>,
    /// Ends the app (Tauri's `exit`); given by `main.rs` (`with_exit`).
    exit: Option<Box<dyn Fn() + Send + Sync>>,
    /// Runs a downloaded update's installer; given by `main.rs` (`with_install`).
    install: Option<Box<Installer>>,
    /// Opens a path, or shows it in the file manager; given by `main.rs` (`with_open`).
    open: Option<Box<Opener>>,
    /// Shows an OS notification; given by `main.rs` (`with_notify`).
    notify: Option<Box<Notifier>>,
    /// Whether the OS lets Hive show notifications (17.19); given by `main.rs` on macOS
    /// (`with_permission`).
    permission: Option<Arc<Permissions>>,
    /// Plays the alert tone (17.19); given by `main.rs` on macOS (`with_tone`).
    tone: Option<Tone>,
    /// Opens a URL with the system (`with_url`).
    url: Option<Box<UrlOpener>>,
    /// `defaults` and arguments before its own, which read the title bar's double-click
    /// setting (`with_defaults`).
    defaults: (OsString, Vec<OsString>),
    /// The app runs translocated (`translocated`); given by `main.rs` (`with_translocated`).
    translocated: bool,
    /// Shows the menu bar icon; given by `main.rs` (`with_tray`, 17.22).
    tray: Option<Box<tray::Apply>>,
    /// Reads the files copied to the clipboard; given by `main.rs` on macOS (`with_clipboard`,
    /// 18.3).
    clipboard: Option<Box<dyn Clipboard>>,
}

/// The clipboard's copied files (18.3), which WebKit shows a page only by name: read by the app
/// side (`macos::Pasteboard` on macOS), behind this trait so [`Hive::pasted_files`] is tested
/// without AppKit.
pub trait Clipboard: Send + Sync {
    /// The paths of the files on the clipboard (copied in Finder), in order; none when it holds
    /// no files.
    fn file_paths(&self) -> Vec<String>;
}

/// Runs the installer of a downloaded update.
type Installer = dyn Fn(&Update, &[u8]) -> Result<(), String> + Send + Sync;
/// Opens `path` with the system's default app, or shows it in the file manager when `reveal`.
type Opener = dyn Fn(&str, bool) -> Result<(), String> + Send + Sync;
/// Opens a URL with the system's handler for it.
type UrlOpener = dyn Fn(&str) -> Result<(), String> + Send + Sync;
/// Asks the OS whether Hive may show notifications; it may wait a moment for the answer.
type Permissions = dyn Fn() -> Permission + Send + Sync;
/// Plays a sound file at a volume from 0 to 1.
type Player = dyn Fn(&Path, f32) -> Result<(), String> + Send + Sync;
/// What a click on an OS notification does (13.5).
pub type Click = Box<dyn Fn() + Send + Sync>;
/// Shows an OS notification with a title and a body; a click on it runs the [`Click`].
type Notifier = dyn Fn(&str, &str, Click) -> Result<(), String> + Send + Sync;

/// The app the OS shows a notification as (13.5): the installed app's `identifier`, which the
/// Windows installer gives its shortcut as the AppUserModelID and macOS reads from the bundle.
/// A development build has none registered, so it borrows `stand_in` (PowerShell's, Terminal's).
pub fn notifier<'a>(dev: bool, identifier: &'a str, stand_in: &'a str) -> &'a str {
    if dev {
        stand_in
    } else {
        identifier
    }
}

/// The identifier the app runs under (17.18): the installed app's `identifier`, or, for a
/// development build, that with `.dev` after it. The single-instance plugin keys the running
/// instance on it (a socket in `/tmp` on macOS, a D-Bus name on Linux, a mutex on Windows) and
/// the window-state plugin keeps its file in the app config folder named after it, so a
/// development run neither focuses the installed app (nor is focused by it) nor moves its window.
pub fn instance_identifier(dev: bool, identifier: &str) -> String {
    if dev {
        format!("{identifier}.dev")
    } else {
        identifier.to_owned()
    }
}

/// What the window-state plugin saves and restores (17.18): size, position, maximised and full
/// screen. Not its decorations (each platform's config sets them) nor whether it is visible (the
/// app always opens its window).
pub const WINDOW_STATE: StateFlags = StateFlags::SIZE
    .union(StateFlags::POSITION)
    .union(StateFlags::MAXIMIZED)
    .union(StateFlags::FULLSCREEN);

/// Brings the main window up, minimised or hidden, restored and focused: a click on a
/// notification (13.5), a second launch of the app (17.18), a Dock click with no window shown
/// ([`reopen`]), a quit that asks first (D17.3) or an agent chosen in the menu bar icon's menu
/// (17.22). Answers whether there was a window to bring up.
pub fn bring_up<R: Runtime>(app: &AppHandle<R>) -> bool {
    let Some(window) = app.get_webview_window("main") else {
        return false;
    };
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
    true
}

/// How many notifications wait for their click at once (`waiting_notifier`); later ones show
/// without click handling until one is answered. On macOS each wait holds a thread and a timer
/// on the main run loop until the notification leaves the Notification Center.
pub const CLICK_WAITS: usize = 8;

/// A notifier for a platform whose `send(title, body, wait)` blocks (macOS: until the click or
/// the dismissal, when `wait`) and answers whether the user clicked it: each one is sent on a
/// thread of its own, at most [`CLICK_WAITS`] of them waiting for their click.
pub fn waiting_notifier(
    send: impl Fn(&str, &str, bool) -> Result<bool, String> + Send + Sync + 'static,
) -> impl Fn(&str, &str, Click) -> Result<(), String> + Send + Sync + 'static {
    waits(send, notification_thread)
}

/// [`waiting_notifier`], its threads started by `start`.
fn waits(
    send: impl Fn(&str, &str, bool) -> Result<bool, String> + Send + Sync + 'static,
    start: Start,
) -> impl Fn(&str, &str, Click) -> Result<(), String> + Send + Sync + 'static {
    let send = Arc::new(send);
    // How many wait for their click.
    let waiting = Arc::new(Mutex::new(0_usize));
    move |title: &str, body: &str, click: Click| {
        let wait = {
            let mut waiting = waiting.lock().unwrap_or_else(PoisonError::into_inner);
            let wait = *waiting < CLICK_WAITS;
            *waiting += usize::from(wait);
            wait
        };
        let done = Arc::clone(&waiting);
        let release = move || {
            if wait {
                *done.lock().unwrap_or_else(PoisonError::into_inner) -= 1;
            }
        };
        let (send, title, body) = (Arc::clone(&send), title.to_owned(), body.to_owned());
        let release_here = release.clone();
        let started = start(Box::new(move || {
            let outcome = send(&title, &body, wait);
            release();
            answered(outcome, &click);
        }));
        started.map_err(|error| {
            release_here();
            format!("cannot show the notification: {error}")
        })
    }
}

/// Runs `click` when the platform says the user clicked the notification; a failure to show it
/// is logged.
pub fn answered(outcome: Result<bool, String>, click: &Click) {
    match outcome {
        Ok(true) => click(),
        Ok(false) => {}
        Err(error) => eprintln!("hive-app: notification failed: {error}"),
    }
}

/// Starts a [`waiting_notifier`]'s notification on a new thread.
fn notification_thread(work: Box<dyn FnOnce() + Send>) -> std::io::Result<()> {
    let thread = std::thread::Builder::new().name("notification".into());
    thread.spawn(work).map(drop)
}

/// The text without control characters, which a Windows toast's XML refuses.
fn printable(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// Whether the OS lets Hive show notifications (17.19), as `notification_permission` tells
/// the UI: `unknown` where Hive cannot ask (a development build, a platform without a check) or
/// before the user answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permission {
    Granted,
    Denied,
    Unknown,
}

impl Permission {
    /// Its name in `notification_permission`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Granted => "granted",
            Self::Denied => "denied",
            Self::Unknown => "unknown",
        }
    }
}

/// macOS's System Settings on its Notifications pane (17.19).
pub const NOTIFICATION_SETTINGS: &str =
    "x-apple.systempreferences:com.apple.Notifications-Settings.extension";

/// Whether `exe` runs from an app bundle (`Hive.app/Contents/MacOS/hive-app`): macOS's
/// notification center serves only a bundled app and raises an exception for any other process
/// (17.19), so a development build never asks it.
pub fn in_app_bundle(exe: &Path) -> bool {
    let mut up = exe.ancestors().skip(1).map(Path::file_name);
    let (macos, contents, app) = (
        up.next().flatten(),
        up.next().flatten(),
        up.next().flatten(),
    );
    let bundle = app.and_then(|name| Path::new(name).extension());
    macos == Some("MacOS".as_ref())
        && contents == Some("Contents".as_ref())
        && bundle == Some("app".as_ref())
}

/// The alert tone's sample rate, length and frequency, as `src/notify.ts` synthesizes it.
const TONE_RATE: u32 = 44_100;
const TONE_SECS: f64 = 0.2;
const TONE_HZ: f64 = 880.0;
/// Its gain at full volume, falling exponentially to `TONE_END` (Web Audio's ramp).
const TONE_GAIN: f64 = 0.15;
const TONE_END: f64 = 0.001;
/// The file the tone is written to, in the folder given to `with_tone`.
const TONE_FILE: &str = "alert-tone.wav";

/// The alert tone (17.19) as a 16-bit mono WAV file: the beep `src/notify.ts` synthesizes with
/// Web Audio, at full volume (the player scales it).
pub fn tone_wav() -> Vec<u8> {
    let samples = (f64::from(TONE_RATE) * TONE_SECS) as u32;
    let data = samples * 2;
    let mut wav = Vec::with_capacity(44 + data as usize);
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&(36 + data).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16_u32.to_le_bytes()); // the format chunk's size
    wav.extend_from_slice(&1_u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&1_u16.to_le_bytes()); // mono
    wav.extend_from_slice(&TONE_RATE.to_le_bytes());
    wav.extend_from_slice(&(TONE_RATE * 2).to_le_bytes()); // bytes per second
    wav.extend_from_slice(&2_u16.to_le_bytes()); // bytes per sample
    wav.extend_from_slice(&16_u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data.to_le_bytes());
    for i in 0..samples {
        let t = f64::from(i) / f64::from(TONE_RATE);
        let gain = TONE_GAIN * (TONE_END / TONE_GAIN).powf(t / TONE_SECS);
        let sample = gain * (std::f64::consts::TAU * TONE_HZ * t).sin();
        wav.extend_from_slice(&((sample * f64::from(i16::MAX)) as i16).to_le_bytes());
    }
    wav
}

/// Plays a sound file with `program` (macOS's `afplay`) as `program -v <volume> <file>`, each a
/// separate argument; a thread of its own waits for it to end.
pub fn afplay(program: PathBuf) -> impl Fn(&Path, f32) -> Result<(), String> + Send + Sync {
    move |file: &Path, volume: f32| {
        let mut child = std::process::Command::new(&program)
            .arg("-v")
            .arg(volume.to_string())
            .arg(file)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(tone_failed)?;
        let thread = std::thread::Builder::new().name("tone".into());
        let waiting = thread.spawn(move || child.wait().map(drop));
        waiting.map(drop).map_err(tone_failed)
    }
}

fn tone_failed(error: std::io::Error) -> String {
    format!("cannot play the tone: {error}")
}

/// The alert tone played by the app (17.19): its folder, whether its file was written in this
/// run, and the player.
struct Tone {
    dir: PathBuf,
    written: Mutex<bool>,
    play: Box<Player>,
}

impl Tone {
    /// Writes the tone's file once a run (through a temporary file renamed over it, so a tone
    /// still playing keeps its file), then plays it at `volume` percent.
    fn play(&self, volume: u8) -> Result<(), String> {
        let file = self.dir.join(TONE_FILE);
        let mut written = self.written.lock().unwrap_or_else(PoisonError::into_inner);
        if !*written {
            let temporary = self.dir.join(format!("{TONE_FILE}.tmp"));
            std::fs::create_dir_all(&self.dir)
                .and_then(|()| std::fs::write(&temporary, tone_wav()))
                .and_then(|()| std::fs::rename(&temporary, &file))
                .map_err(|error| format!("cannot write the tone: {error}"))?;
            *written = true;
        }
        drop(written);
        (self.play)(&file, f32::from(volume) / 100.0)
    }
}

/// macOS' `defaults`, which reads the user's preferences.
const DEFAULTS: &str = "/usr/bin/defaults";
/// How long reading a preference may take: it runs on a double click.
const DEFAULTS_WAIT: Duration = Duration::from_secs(2);

/// What a double click on the title bar does, from macOS' "Double-click a window's title bar
/// to" (Desktop & Dock), the global `AppleActionOnDoubleClick`: "Minimize" minimizes, "None"
/// does nothing; "Maximize" (Zoom), "Fill" and an unset or unknown value zoom, as Tauri's own
/// drag region does (its maximize is the window's zoom, which fills the screen).
pub fn double_click_action(setting: Option<&str>) -> &'static str {
    match setting.map(str::trim) {
        Some("Minimize") => "minimize",
        Some("None") => "none",
        _ => "zoom",
    }
}

/// An open terminal: where its output goes, and where it runs, for a UI reloaded after a
/// WebContent crash to show its tab again (`terminal_kept`).
struct Term {
    output: Channel<InvokeResponseBody>,
    cwd: String,
    /// Its `terminal_opened`'s `worktree`; null until then.
    worktree: Value,
}

#[derive(Default)]
struct Link {
    /// Control messages to the UI.
    ui: Option<Channel<Value>>,
    /// Frames to the service; `None` when there is no connection.
    frames: Option<mpsc::UnboundedSender<Frame>>,
    /// Replayed to a reloaded UI.
    welcome: Option<Value>,
    /// Every open terminal, by frame channel.
    terminals: HashMap<u32, Term>,
    /// The WebContent process died (18.21, D18.5): the next `connect` is its reloaded UI,
    /// which keeps the terminals.
    crashed: bool,
    last_channel: u32,
    /// Reads the service until the connection ends; it owns the bridge process.
    reader: Option<JoinHandle<()>>,
    /// The newer release `check_update` found and downloaded, for `install_update` or the
    /// app's exit (4.19).
    update: Option<(Update, Vec<u8>)>,
    /// The paths the service sent for the app to open, oldest first, each for one `open_path`.
    approved: VecDeque<String>,
    /// Whether the UI was already told to move a translocated app (`connect`): the notice
    /// opens once per run, not again on a reload.
    translocation_told: bool,
    /// Whether the bridge runs the installer's `hive` (`Bridge`); sent with
    /// `version_mismatch` and `disconnected`, so the UI shows only fixes an installed app's
    /// user can apply (12.4).
    bundled: bool,
    /// The mode chosen (`Modes`), `None` until the user chooses one.
    mode: Option<Mode>,
    /// The agents whose last state makes quitting ask first (`at_risk`), by id.
    at_risk: HashSet<String>,
    /// The `confirm_close` setting the service last sent; `None` before any (its default: on).
    confirm_close: Option<bool>,
    /// When the UI was last asked to confirm a quit (`quit_requested`).
    asked: Option<Instant>,
}

impl Link {
    /// Lets `open_path` open `path` once; the oldest waiting path goes past [`APPROVED_LIMIT`].
    fn approve(&mut self, path: String) {
        if self.approved.len() == APPROVED_LIMIT {
            self.approved.pop_front();
        }
        self.approved.push_back(path);
    }

    /// Follows what quitting asks about (D17.3) in a service message: an agent's state, an
    /// agent gone, the `confirm_close` setting.
    fn follow(&mut self, message: &Value) {
        let id = message["id"].as_str().map(str::to_owned);
        match (message["type"].as_str(), id) {
            (Some("agent_state"), Some(id)) => {
                let state = serde_json::from_value(message["state"].clone()).ok();
                if state.is_some_and(at_risk) {
                    self.at_risk.insert(id);
                } else {
                    self.at_risk.remove(&id);
                }
            }
            (Some("agent_removed"), Some(id)) => drop(self.at_risk.remove(&id)),
            (Some("settings"), _) => {
                self.confirm_close = message["settings"]["agents"]["confirm_close"].as_bool();
            }
            _ => {}
        }
    }

    /// The exit decision for what this link knows at `now`.
    fn quit(&self, now: Instant) -> Quit {
        let confirm = self.confirm_close.unwrap_or(true);
        let asked = self.asked.map(|asked| now.saturating_duration_since(asked));
        quit_decision(self.ui.is_some(), confirm, self.at_risk.len(), asked)
    }

    /// Sends `message` to the UI; false when there is none or the send failed.
    fn to_ui(&self, message: Value) -> bool {
        (self.ui.as_ref()).is_some_and(|ui| ui.send(message).is_ok())
    }

    fn frame(&self, frame: Frame) -> Result<(), String> {
        // The writer could not encode it and would end the connection.
        if frame.payload.len() > MAX_PAYLOAD {
            return Err(FrameError::Oversized(frame.payload.len()).to_string());
        }
        self.frames
            .as_ref()
            .and_then(|frames| frames.send(frame).ok())
            .ok_or_else(|| NOT_CONNECTED.to_owned())
    }

    fn send(&self, channel: u32, message: &Control) -> Result<(), String> {
        self.frame(Frame::control(channel, message))
    }

    /// Sends `message` on a new channel and returns it.
    fn open(&mut self, message: &Control) -> Result<u32, String> {
        let id = self
            .last_channel
            .checked_add(1)
            .ok_or("no terminal channel left")?;
        self.send(id, message)?;
        self.last_channel = id;
        Ok(id)
    }
}

fn lock(link: &Mutex<Link>) -> MutexGuard<'_, Link> {
    link.lock().unwrap_or_else(PoisonError::into_inner)
}

/// How a connection ended.
enum End {
    /// The service refused the handshake; the UI already has `version_mismatch`.
    Refused,
    /// The bridge closed its stdout.
    Closed,
    /// The stream broke the protocol.
    Broken(String),
}

impl Hive {
    pub fn new(program: OsString, args: Vec<OsString>) -> Self {
        Self {
            bridge: (program, args, false),
            modes: None,
            link: Arc::default(),
            reconnecting: tokio::sync::Mutex::default(),
            restart: None,
            exit: None,
            install: None,
            open: None,
            notify: None,
            permission: None,
            tone: None,
            url: None,
            defaults: (DEFAULTS.into(), Vec::new()),
            translocated: false,
            tray: None,
            clipboard: None,
        }
    }

    /// Reads macOS preferences with `program` and its first `args` instead of
    /// `/usr/bin/defaults` (tests).
    pub fn with_defaults(mut self, program: &str, args: &[&str]) -> Self {
        self.defaults = (program.into(), args.iter().map(OsString::from).collect());
        self
    }

    /// What a double click on the empty title bar does on macOS ([`double_click_action`]): the
    /// setting is read on every click, so a change in System Settings counts at once. A
    /// `defaults` that fails (the key never set) or takes too long counts as unset.
    pub fn title_bar_action(&self) -> &'static str {
        let (program, args) = &self.defaults;
        let mut read = std::process::Command::new(program);
        read.args(args)
            .args(["read", "-g", "AppleActionOnDoubleClick"]);
        let out = output_within(&mut read, DEFAULTS_WAIT, reader).ok();
        let out = out.filter(|out| out.status.success());
        let setting = out.map(|out| String::from_utf8_lossy(&out.stdout).into_owned());
        double_click_action(setting.as_deref())
    }

    /// Tells the UI the app runs translocated (17.10): `connect` sends `translocated`, and an
    /// update is not installed.
    pub fn with_translocated(mut self, translocated: bool) -> Self {
        self.translocated = translocated;
        self
    }

    /// Shows OS notifications with `notify`, the platform's own (13.5).
    pub fn with_notify(
        mut self,
        notify: impl Fn(&str, &str, Click) -> Result<(), String> + Send + Sync + 'static,
    ) -> Self {
        self.notify = Some(Box::new(notify));
        self
    }

    /// Shows an OS notification about `agent` (13.5), its text without control characters. A
    /// click on it brings the window to the front, restored and focused, and sends `notification_clicked {agent}` to the UI, which
    /// goes to the agent if it is still there (`goToAgent`).
    pub fn show_notification<R: Runtime>(
        &self,
        app: &AppHandle<R>,
        title: &str,
        body: &str,
        agent: String,
    ) -> Result<(), String> {
        let notify = self
            .notify
            .as_ref()
            .ok_or("this app cannot show notifications")?;
        let (app, link) = (app.clone(), Arc::clone(&self.link));
        let click = move || {
            bring_up(&app);
            lock(&link).to_ui(json!({"type": "notification_clicked", "agent": agent}));
        };
        notify(&printable(title), &printable(body), Box::new(click))
    }

    /// Asks the OS with `check` whether Hive may show notifications (17.19).
    pub fn with_permission(
        mut self,
        check: impl Fn() -> Permission + Send + Sync + 'static,
    ) -> Self {
        self.permission = Some(Arc::new(check));
        self
    }

    /// Asks the OS whether Hive may show notifications, on Tauri's blocking threads (the answer
    /// may take a moment); the answer goes to the UI as `notification_permission {permission}`,
    /// `unknown` where the app cannot ask.
    pub fn check_notifications(&self) {
        let check = self.permission.clone();
        let link = Arc::clone(&self.link);
        let answer = move || {
            let permission = check.map_or(Permission::Unknown, |check| check()).name();
            let message = json!({"type": "notification_permission", "permission": permission});
            lock(&link).to_ui(message);
        };
        tauri::async_runtime::spawn_blocking(answer);
    }

    /// Opens URLs with `open` (the system's handler).
    pub fn with_url(
        mut self,
        open: impl Fn(&str) -> Result<(), String> + Send + Sync + 'static,
    ) -> Self {
        self.url = Some(Box::new(open));
        self
    }

    /// Opens macOS's notification settings (17.19), where the user lets Hive notify again.
    pub fn open_notification_settings(&self) -> Result<(), String> {
        let open = self.url.as_ref().ok_or("this app cannot open URLs")?;
        open(NOTIFICATION_SETTINGS)
    }

    /// Plays the alert tone from the app (17.19), its file written in `dir`, with `play`.
    pub fn with_tone(
        mut self,
        dir: PathBuf,
        play: impl Fn(&Path, f32) -> Result<(), String> + Send + Sync + 'static,
    ) -> Self {
        let (written, play) = (Mutex::new(false), Box::new(play));
        self.tone = Some(Tone { dir, written, play });
        self
    }

    /// Plays the alert tone at `volume` percent (at most 100; 0 plays none).
    pub fn play_tone(&self, volume: u8) -> Result<(), String> {
        let tone = self.tone.as_ref().ok_or("this app plays no tone")?;
        match volume.min(100) {
            0 => Ok(()),
            volume => tone.play(volume),
        }
    }

    /// Opens paths with `open` (the system's default app), or `reveal` (the file manager).
    pub fn with_open<E: std::fmt::Display>(
        mut self,
        open: impl Fn(&str) -> Result<(), E> + Send + Sync + 'static,
        reveal: impl Fn(&str) -> Result<(), E> + Send + Sync + 'static,
    ) -> Self {
        self.open = Some(Box::new(move |path, show| {
            let opened = if show { reveal(path) } else { open(path) };
            opened.map_err(|e| e.to_string())
        }));
        self
    }

    /// Opens `path` with the system's default app, or shows it in the file manager when
    /// `reveal` (open point #15): only a path the service just sent in `editor_target` or
    /// `session_located` (checked there: `hive::file::windows_path`, 9.8; a session's log), and
    /// each at most once, even when opening it fails. The webview itself has no permission to
    /// open a path, so a script in it can open nothing else.
    pub fn open_path(&self, path: String, reveal: bool) -> Result<(), String> {
        let mut link = self.link();
        let Some(at) = link.approved.iter().position(|approved| *approved == path) else {
            return Err(format!("Hive opens only a path the service sent: {path}"));
        };
        link.approved.remove(at);
        drop(link);
        let open = self.open.as_ref().ok_or("this app cannot open paths")?;
        open(&path, reveal)
    }

    /// Reads the files copied to the clipboard from `clipboard` (18.3).
    pub fn with_clipboard(mut self, clipboard: impl Clipboard + 'static) -> Self {
        self.clipboard = Some(Box::new(clipboard));
        self
    }

    /// What pasting the files copied in Finder types (18.3): their paths, quoted as a drop's
    /// ([`quote::typed`], which leaves out a path with a control character: an escape could end
    /// the bracketed paste early and type the rest as keys); empty when the clipboard holds no
    /// usable files or the app cannot read it, and the UI then pastes the clipboard's text.
    pub fn pasted_files(&self) -> String {
        let paths = self
            .clipboard
            .as_ref()
            .map_or_else(Vec::new, |clipboard| clipboard.file_paths());
        quote::typed(&paths)
    }

    /// Tells the UI the bridge runs the installer's `hive` (see `bridge_command`).
    pub fn with_bundled(mut self, bundled: bool) -> Self {
        self.bridge.2 = bundled;
        self
    }

    /// Offers the choice of WSL or Windows (12.5.4), starting in the mode saved.
    pub fn with_modes(mut self, modes: Modes) -> Self {
        self.link().mode = modes.start();
        self.modes = Some(modes);
        self
    }

    /// Keeps the mode the user chose, the first run's or the settings' (12.5.4); `reconnect`
    /// then starts its service.
    pub fn choose_mode(&self, mode: &str) -> Result<(), String> {
        let modes = self
            .modes
            .as_ref()
            .ok_or("Hive offers no choice of service here")?;
        let mode = Mode::parse(mode).ok_or_else(|| format!("unknown service mode: {mode}"))?;
        // Without WSL no mode is saved: installing WSL later asks.
        if mode == Mode::Wsl && !modes.wsl {
            return Err("WSL has no distribution: Hive runs on Windows".into());
        }
        let saved = modes.save(mode);
        saved.map_err(|error| format!("cannot save the service mode: {error}"))?;
        self.link().mode = Some(mode);
        Ok(())
    }

    /// Ends the connection, if any, then connects the same UI again, to the service of the mode
    /// chosen now. Must run inside the Tokio runtime.
    pub async fn reconnect(&self) {
        let _one_at_a_time = self.reconnecting.lock().await;
        self.shutdown(EXIT_WAIT).await;
        let ui = self.link().ui.clone();
        if let Some(ui) = ui {
            self.connect(ui);
        }
    }

    pub fn with_restart(mut self, restart: impl Fn() + Send + Sync + 'static) -> Self {
        self.restart = Some(Box::new(restart));
        self
    }

    /// How `request_quit` ends the app.
    pub fn with_exit(mut self, exit: impl Fn() + Send + Sync + 'static) -> Self {
        self.exit = Some(Box::new(exit));
        self
    }

    /// Whether the app may quit now (D17.3, #47): ⌘Q, the Dock's "Quit" and logging out ask
    /// this first on macOS (`macos::ask_before_terminating`), and `request_quit` too. When
    /// agents would end with the app and the `confirm_close` setting is on, the answer is no:
    /// the window comes up and the UI gets `quit_requested`, which shows the same close dialog
    /// as the window's Close button; confirming it closes the window, which quits. A UI that
    /// cannot get the message (gone, stale) is forgotten and the app quits; so does a second
    /// request within [`ASK_AGAIN`] of the last one, so a hung UI never keeps the app.
    pub fn should_quit<R: Runtime>(&self, app: &AppHandle<R>) -> bool {
        if self.link().quit(Instant::now()) == Quit::Now {
            return true;
        }
        bring_up(app);
        let mut link = self.link();
        if !link.to_ui(json!({"type": "quit_requested"})) {
            link.ui = None;
            return true;
        }
        link.asked = Some(Instant::now());
        false
    }

    /// Quits the app from inside it (the app menu's Quit, 17.13): at once, or after the UI's
    /// confirmation (`should_quit`).
    pub fn request_quit<R: Runtime>(&self, app: &AppHandle<R>) {
        if self.should_quit(app) {
            if let Some(exit) = &self.exit {
                exit();
            }
        }
    }

    pub fn with_install(
        mut self,
        install: impl Fn(&Update, &[u8]) -> Result<(), String> + Send + Sync + 'static,
    ) -> Self {
        self.install = Some(Box::new(install));
        self
    }

    fn link(&self) -> MutexGuard<'_, Link> {
        lock(&self.link)
    }

    /// Files dropped from Finder on the window (18.1), at (`x`, `y`) in points from its top
    /// left: the UI gets `files_dropped` with their paths as one shell-quoted text
    /// ([`quote::typed`]: paths that may not be typed are left out), which it types into the
    /// terminal whose pane is there, if any. False when there is nothing to type or no UI to
    /// send it to.
    pub fn files_dropped(&self, paths: &[String], x: f64, y: f64) -> bool {
        let text = quote::typed(paths);
        let message = json!({"type": "files_dropped", "text": text, "x": x, "y": y});
        !text.is_empty() && self.link().to_ui(message)
    }

    /// Sends every service message to `ui`, starting the bridge unless a connection is up.
    /// A reloaded UI calls this again: its old terminals are closed and `welcome` is replayed.
    /// After a WebContent crash ([`Hive::web_content_terminated`]) they stay instead, each
    /// told to the UI as `terminal_kept` for it to reattach (`attach_terminal`, 18.21).
    /// With the choice of WSL or Windows, the UI first gets `app_mode`, and nothing starts
    /// until a mode is chosen. Must run inside the Tokio runtime.
    pub fn connect(&self, ui: Channel<Value>) {
        let mut link = self.link();
        link.ui = Some(ui);
        if self.translocated {
            // Every UI learns it (the update button explains it); only the first opens the notice.
            let notice = !std::mem::replace(&mut link.translocation_told, true);
            link.to_ui(json!({"type": "translocated", "notice": notice}));
        }
        // After a WebContent crash the agents in them live on: the UI reattaches (18.21).
        if !std::mem::take(&mut link.crashed) {
            for id in std::mem::take(&mut link.terminals).into_keys() {
                let _ = link.send(id, &Control::CloseTerminal);
            }
        }
        let bridge = match &self.modes {
            None => &self.bridge,
            Some(modes) => {
                let mode = link.mode.map(Mode::name);
                link.to_ui(json!({"type": "app_mode", "mode": mode, "wsl": modes.wsl}));
                match link.mode {
                    None => return,
                    Some(Mode::Wsl) => &self.bridge,
                    Some(Mode::Native) => &modes.native,
                }
            }
        };
        if link.frames.is_some() {
            if let Some(welcome) = link.welcome.clone() {
                link.to_ui(welcome);
                let mut kept: Vec<_> = link.terminals.iter().collect();
                kept.sort_by_key(|(id, _)| **id);
                for (id, term) in kept {
                    let (cwd, worktree) = (&term.cwd, &term.worktree);
                    let message = json!({"type": "terminal_kept", "channel": id, "cwd": cwd, "worktree": worktree});
                    link.to_ui(message);
                }
                let _ = link.send(0, &Control::GetSettings);
                let _ = link.send(0, &Control::ListProjects);
                let _ = link.send(0, &Control::Resync);
            }
            return;
        }
        link.bundled = bridge.2;
        drop(link);
        if let Err(error) = self.spawn(bridge) {
            disconnected(
                &mut self.link(),
                format!("cannot start the hive bridge: {error}"),
            );
        }
    }

    fn spawn(&self, (program, args, _): &Bridge) -> std::io::Result<()> {
        let mut command = tokio::process::Command::new(program);
        command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(windows)]
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW: no console window for wsl.exe
        let mut child = command.spawn()?;
        let stdin = pipe(child.stdin.take())?;
        let stdout = pipe(child.stdout.take())?;
        let stderr = pipe(child.stderr.take())?;
        // Owns the child until the connection ends (it is killed on drop), then explains the end.
        let exit = async move {
            let mut text = Vec::new();
            let finish = async {
                let _ = stderr.take(STDERR_LIMIT).read_to_end(&mut text).await;
                let _ = child.wait().await;
            };
            let _ = tokio::time::timeout(EXIT_WAIT, finish).await;
            let text = String::from_utf8_lossy(&text).trim().to_owned();
            if text.is_empty() {
                "the hive bridge exited".to_owned()
            } else {
                text
            }
        };
        self.attach(stdout, stdin, exit);
        Ok(())
    }

    /// Speaks the protocol over `reader`/`writer`, starting with the handshake.
    /// `exit` gives the disconnect reason once the reader closes.
    fn attach(
        &self,
        reader: impl AsyncRead + Unpin + Send + 'static,
        writer: impl AsyncWrite + Unpin + Send + 'static,
        exit: impl Future<Output = String> + Send + 'static,
    ) {
        let (frames, mut queue) = mpsc::unbounded_channel();
        let _ = frames.send(Frame::control(0, &Control::hello(Role::App, VERSION)));
        self.link().frames = Some(frames);
        tokio::spawn(async move {
            let mut writer = FramedWrite::new(writer, FrameCodec);
            while let Some(frame) = queue.recv().await {
                if writer.send(frame).await.is_err() {
                    break;
                }
            }
        });
        let link = self.link.clone();
        let task = tokio::spawn(async move {
            let reason = match pump(&link, FramedRead::new(reader, FrameCodec)).await {
                End::Refused => return,
                End::Closed => exit.await,
                End::Broken(error) => error,
            };
            disconnected(&mut lock(&link), reason);
        });
        self.link().reader = Some(task);
    }

    /// Ends the connection as the app exits (#18): the bridge's stdin closes, so the bridge
    /// exits and the service ends every terminal and agent with the app connection (#14).
    /// A bridge still running after `wait` is killed, so no `wsl.exe` outlives the app.
    pub async fn shutdown(&self, wait: Duration) {
        let reader = {
            let mut link = self.link();
            link.frames = None;
            link.reader.take()
        };
        let Some(mut reader) = reader else { return };
        if tokio::time::timeout(wait, &mut reader).await.is_err() {
            // Dropping the task drops the bridge process, which kills it.
            reader.abort();
            let _ = reader.await;
        }
    }

    /// Opens a terminal on a new channel; its output goes to `output`. Returns the channel.
    /// A terminal in `cwd` with the Claude `account` asked for (12.2; `None`: the current one).
    pub fn open_terminal(
        &self,
        cwd: String,
        cols: u16,
        rows: u16,
        account: Option<AccountDir>,
        output: Channel<InvokeResponseBody>,
    ) -> Result<u32, String> {
        let mut link = self.link();
        let open = Control::OpenTerminal {
            cwd: cwd.clone(),
            cols,
            rows,
            account,
        };
        let id = link.open(&open)?;
        let worktree = Value::Null;
        link.terminals.insert(
            id,
            Term {
                output,
                cwd,
                worktree,
            },
        );
        Ok(id)
    }

    /// The WebContent process died (18.21, D18.5); its page reloads, and that UI's `connect`
    /// keeps the terminals.
    pub fn web_content_terminated(&self) {
        self.link().crashed = true;
    }

    /// A UI reloaded after a WebContent crash takes terminal `id`'s output (`terminal_kept`).
    /// What the dead page never acknowledged is acknowledged, so the terminal's output flows.
    pub fn attach_terminal(
        &self,
        id: u32,
        output: Channel<InvokeResponseBody>,
    ) -> Result<(), String> {
        let mut link = self.link();
        let term = link.terminals.get_mut(&id).ok_or("no such terminal")?;
        term.output = output;
        link.send(id, &Control::Ack { bytes: u32::MAX })
    }

    /// Keystrokes or pasted text, split to fit the frame size limit.
    pub fn write_terminal(&self, id: u32, data: &str) -> Result<(), String> {
        let link = self.link();
        data.as_bytes()
            .chunks(MAX_PAYLOAD)
            .try_for_each(|chunk| link.frame(Frame::terminal(id, chunk.to_vec())))
    }

    pub fn resize_terminal(&self, id: u32, cols: u16, rows: u16) -> Result<(), String> {
        self.link().send(id, &Control::Resize { cols, rows })
    }

    pub fn close_terminal(&self, id: u32) -> Result<(), String> {
        self.link().send(id, &Control::CloseTerminal)
    }

    /// `bytes` more of terminal `id`'s output were written to its screen (9.19).
    pub fn ack_terminal(&self, id: u32, bytes: u32) -> Result<(), String> {
        self.link().send(id, &Control::Ack { bytes })
    }

    /// Asks for every project with its worktrees; they arrive as `projects`.
    pub fn list_projects(&self) -> Result<(), String> {
        self.link().send(0, &Control::ListProjects)
    }

    /// The answer arrives as `project_added` or `add_project_failed`.
    pub fn add_project(&self, path: String) -> Result<(), String> {
        self.link().send(0, &Control::AddProject { path })
    }

    /// The answer arrives as `project_removed` or `remove_project_failed`.
    pub fn remove_project(&self, id: String) -> Result<(), String> {
        self.link().send(0, &Control::RemoveProject { id })
    }

    /// The answer arrives as `branches`.
    pub fn list_branches(&self, project: String) -> Result<(), String> {
        self.link().send(0, &Control::ListBranches { project })
    }

    /// The answer arrives as `worktree_name_validated`.
    pub fn validate_worktree_name(&self, project: String, name: String) -> Result<(), String> {
        self.link()
            .send(0, &Control::ValidateWorktreeName { project, name })
    }

    /// The answer arrives as `worktree_created` or `create_worktree_failed`.
    pub fn create_worktree(
        &self,
        project: String,
        name: String,
        base: Option<String>,
    ) -> Result<(), String> {
        let create = Control::CreateWorktree {
            project,
            name,
            base,
        };
        self.link().send(0, &create)
    }

    /// The answer arrives as `worktree_created` or `create_worktree_failed`.
    pub fn create_issue_worktree(
        &self,
        project: String,
        number: u64,
        base: Option<String>,
    ) -> Result<(), String> {
        let create = Control::CreateIssueWorktree {
            project,
            number,
            base,
        };
        self.link().send(0, &create)
    }

    /// The answer arrives as `worktree_removed` or `remove_worktree_failed`.
    pub fn remove_worktree(&self, path: String, force: bool) -> Result<(), String> {
        self.link()
            .send(0, &Control::RemoveWorktree { path, force })
    }

    /// The answer arrives as `worktree_renamed` or `rename_worktree_failed`.
    pub fn rename_worktree(&self, path: String, name: String) -> Result<(), String> {
        self.link().send(0, &Control::RenameWorktree { path, name })
    }

    /// Answered by `files` now and after every change in the worktree.
    pub fn watch_worktree(&self, path: String, base: DiffBase) -> Result<(), String> {
        self.link().send(0, &Control::WatchWorktree { path, base })
    }

    pub fn unwatch_worktree(&self) -> Result<(), String> {
        self.link().send(0, &Control::UnwatchWorktree)
    }

    /// The ignored folders open in the watched worktree's tree; answered by `files`.
    pub fn expand_ignored(&self, path: String, folders: Vec<String>) -> Result<(), String> {
        self.link()
            .send(0, &Control::ExpandIgnored { path, folders })
    }

    /// The terminal shown and whether the window has the focus.
    pub fn set_view(&self, terminal: Option<u32>, focused: bool) -> Result<(), String> {
        self.link().send(0, &Control::View { terminal, focused })
    }

    /// The answer arrives as `changes`.
    pub fn list_changes(&self, path: String, base: DiffBase) -> Result<(), String> {
        self.link().send(0, &Control::ListChanges { path, base })
    }

    /// The answer arrives as `sessions`.
    pub fn list_sessions(&self) -> Result<(), String> {
        self.link().send(0, &Control::ListSessions)
    }

    /// The answer arrives as `session_located`.
    pub fn locate_session(&self, id: String, target: SessionTarget) -> Result<(), String> {
        self.link().send(0, &Control::LocateSession { id, target })
    }

    /// The answer arrives as `session_deleted` or `delete_session_failed`.
    pub fn delete_session(&self, id: String) -> Result<(), String> {
        self.link().send(0, &Control::DeleteSession { id })
    }

    /// The answer arrives as `dirs`.
    pub fn list_dirs(&self, path: String, windows: bool) -> Result<(), String> {
        self.link().send(0, &Control::ListDirs { path, windows })
    }

    /// The answer arrives as `search_results`.
    pub fn search_files(&self, worktree: String, query: String) -> Result<(), String> {
        self.link()
            .send(0, &Control::SearchFiles { worktree, query })
    }

    /// The answer arrives as `file`.
    pub fn open_file(&self, worktree: String, path: String, base: DiffBase) -> Result<(), String> {
        let open = Control::OpenFile {
            worktree,
            path,
            base,
        };
        self.link().send(0, &open)
    }

    /// The answer arrives as `file_saved` or `save_failed`.
    pub fn save_file(
        &self,
        worktree: String,
        path: String,
        content: String,
        version: Option<String>,
    ) -> Result<(), String> {
        let save = Control::SaveFile {
            worktree,
            path,
            content,
            version,
        };
        self.link().send(0, &save)
    }

    /// The answer arrives as `file_created` or `file_op_failed`.
    pub fn create_file(
        &self,
        worktree: String,
        folder: String,
        name: String,
    ) -> Result<(), String> {
        let create = Control::CreateFile {
            worktree,
            folder,
            name,
        };
        self.link().send(0, &create)
    }

    /// The answer arrives as `file_renamed` or `file_op_failed`.
    pub fn rename_file(&self, worktree: String, path: String, name: String) -> Result<(), String> {
        let rename = Control::RenameFile {
            worktree,
            path,
            name,
        };
        self.link().send(0, &rename)
    }

    /// The answer arrives as `file_renamed` or `file_op_failed`.
    pub fn move_file(&self, worktree: String, path: String, folder: String) -> Result<(), String> {
        let moving = Control::MoveFile {
            worktree,
            path,
            folder,
        };
        self.link().send(0, &moving)
    }

    /// The answer arrives as `file_deleted` or `file_op_failed`.
    pub fn delete_file(&self, worktree: String, path: String) -> Result<(), String> {
        self.link().send(0, &Control::DeleteFile { worktree, path })
    }

    /// The answer arrives as `folder_created` or `file_op_failed`.
    pub fn create_folder(
        &self,
        worktree: String,
        folder: String,
        name: String,
    ) -> Result<(), String> {
        let create = Control::CreateFolder {
            worktree,
            folder,
            name,
        };
        self.link().send(0, &create)
    }

    /// The answer arrives as `editor_target`.
    pub fn open_in_editor(&self, worktree: String, path: String) -> Result<(), String> {
        self.link()
            .send(0, &Control::OpenInEditor { worktree, path })
    }

    /// The answer arrives as `settings`.
    pub fn get_settings(&self) -> Result<(), String> {
        self.link().send(0, &Control::GetSettings)
    }

    /// The answer arrives as `settings` or `settings_failed`.
    pub fn set_settings(&self, settings: Settings) -> Result<(), String> {
        self.link().send(0, &Control::SetSettings { settings })
    }

    /// The answer to each space request arrives as `spaces` or `space_failed`.
    pub fn create_space(&self, name: String, env: SpaceEnv) -> Result<(), String> {
        self.link().send(0, &Control::CreateSpace { name, env })
    }

    pub fn update_space(&self, id: String, name: String, env: SpaceEnv) -> Result<(), String> {
        self.link().send(0, &Control::UpdateSpace { id, name, env })
    }

    pub fn delete_space(&self, id: String) -> Result<(), String> {
        self.link().send(0, &Control::DeleteSpace { id })
    }

    pub fn select_space(&self, id: String) -> Result<(), String> {
        self.link().send(0, &Control::SelectSpace { id })
    }

    /// The answer to both arrives as `gh_accounts` (9.30).
    pub fn list_gh_accounts(&self, gh_config_dir: Option<String>) -> Result<(), String> {
        self.link()
            .send(0, &Control::ListGhAccounts { gh_config_dir })
    }

    pub fn switch_gh_account(
        &self,
        gh_config_dir: Option<String>,
        account: GhAccount,
    ) -> Result<(), String> {
        let switch = Control::SwitchGhAccount {
            gh_config_dir,
            account,
        };
        self.link().send(0, &switch)
    }

    /// The answer arrives as `pulls` (9.31).
    pub fn list_pulls(&self, project: String, force: bool) -> Result<(), String> {
        self.link().send(0, &Control::ListPulls { project, force })
    }

    /// The answer arrives as `pull`.
    pub fn open_pull(&self, project: String, number: u64) -> Result<(), String> {
        self.link().send(0, &Control::OpenPull { project, number })
    }

    /// The answer arrives as `pull_done` or `pull_failed` (a checkout: `worktree_created`).
    pub fn act_on_pull(
        &self,
        project: String,
        number: u64,
        action: PullAction,
    ) -> Result<(), String> {
        let act = Control::ActOnPull {
            project,
            number,
            action,
        };
        self.link().send(0, &act)
    }

    /// The answer arrives as `pull_done` or `pull_failed`.
    pub fn create_pull(
        &self,
        worktree: String,
        title: String,
        body: String,
        base: String,
        draft: bool,
    ) -> Result<(), String> {
        let create = Control::CreatePull {
            worktree,
            title,
            body,
            base,
            draft,
        };
        self.link().send(0, &create)
    }

    /// The answer arrives as `runs` (9.32).
    pub fn list_runs(
        &self,
        project: String,
        branch: Option<String>,
        force: bool,
    ) -> Result<(), String> {
        let list = Control::ListRuns {
            project,
            branch,
            force,
        };
        self.link().send(0, &list)
    }

    /// The answer arrives as `run`.
    pub fn open_run(&self, project: String, run: u64) -> Result<(), String> {
        self.link().send(0, &Control::OpenRun { project, run })
    }

    /// The answer arrives as `job_log`.
    pub fn open_job_log(&self, project: String, job: u64) -> Result<(), String> {
        self.link().send(0, &Control::OpenJobLog { project, job })
    }

    /// The answer arrives as `run_done` or `run_failed`.
    pub fn act_on_run(
        &self,
        project: String,
        run: u64,
        action: RunAction,
        branch: Option<String>,
    ) -> Result<(), String> {
        let act = Control::ActOnRun {
            project,
            run,
            action,
            branch,
        };
        self.link().send(0, &act)
    }

    /// The answer arrives as `editor_target` with an empty `worktree`.
    pub fn open_settings_file(&self) -> Result<(), String> {
        self.link().send(0, &Control::OpenSettingsFile)
    }

    /// The answer arrives as `diagnostics`.
    pub fn get_diagnostics(&self) -> Result<(), String> {
        self.link().send(0, &Control::GetDiagnostics)
    }
}

impl Hive {
    /// Asks the release endpoint for a newer version (4.19) and downloads it at once. No newer
    /// version, or a failed check (offline, GitHub down), sends nothing: it is not worth a notice.
    /// A `manual` check (the menu's "Check for Updates…", 17.13) answers those too
    /// (`update_checked`), and offers again an update already downloaded.
    pub async fn check_update(&self, updater: tauri_plugin_updater::Result<Updater>, manual: bool) {
        let ready = self
            .link()
            .update
            .as_ref()
            .map(|(update, _)| update.version.clone());
        if let Some(version) = ready.filter(|_| manual) {
            self.link()
                .to_ui(json!({"type": "update_ready", "version": version}));
            return;
        }
        let update = match async { updater?.check().await }.await {
            Ok(Some(update)) => update,
            checked => return self.checked(manual, checked.err().map(|e| e.to_string())),
        };
        let bytes = update.download(|_, _| {}, || {}).await;
        let bytes = bytes.map_err(|error| error.to_string());
        if let Err(error) = &bytes {
            self.checked(manual, Some(error.clone()));
        }
        self.downloaded(update, bytes);
    }

    /// A check that found nothing to offer: `update_checked {error}` (null: up to date) when the
    /// user asked for it (`manual`); nothing for the check at start, which the next start repeats.
    fn checked(&self, manual: bool, error: Option<String>) {
        if manual {
            self.link()
                .to_ui(json!({"type": "update_checked", "error": error}));
        }
    }

    /// A downloaded update, its signature checked, waits for `install_update` or the app's exit
    /// and goes to the UI as `update_ready {version}`. A failed download sends nothing, like a
    /// failed check: the next start tries again.
    fn downloaded(&self, update: Update, bytes: Result<Vec<u8>, String>) {
        let Ok(bytes) = bytes else { return };
        let mut link = self.link();
        link.to_ui(json!({"type": "update_ready", "version": update.version}));
        link.update = Some((update, bytes));
    }

    /// Runs the installer of the downloaded update, then restarts. On Windows the installer
    /// ends the app itself.
    pub async fn install_update(&self) {
        let result = self
            .install_pending()
            .unwrap_or_else(|| Err("no update to install".to_owned()));
        self.installed(result);
    }

    /// Installs the downloaded update, if any, so the next start runs it. Taken once: the exit
    /// that follows `install_update`'s restart does not install it again. A translocated app
    /// cannot replace itself, so it says why instead of running the installer (17.10).
    pub fn install_pending(&self) -> Option<Result<(), String>> {
        let (update, bytes) = self.link().update.take()?;
        if self.translocated {
            return Some(Err(TRANSLOCATED.to_owned()));
        }
        let install = self.install.as_ref()?;
        Some(install(&update, &bytes))
    }

    /// Restarts once installed; a failure goes to the UI as `update_failed {error}`.
    fn installed(&self, result: Result<(), String>) {
        match result {
            Ok(()) => self.restart.iter().for_each(|restart| restart()),
            Err(error) => {
                self.link()
                    .to_ui(json!({"type": "update_failed", "error": error}));
            }
        }
    }
}

/// A piped stdio handle of the bridge; always there, since every one is requested.
fn pipe<T>(pipe: Option<T>) -> std::io::Result<T> {
    pipe.ok_or(std::io::ErrorKind::BrokenPipe.into())
}

/// Routes service frames until the connection ends.
async fn pump<R: AsyncRead + Unpin>(
    link: &Mutex<Link>,
    mut frames: FramedRead<R, FrameCodec>,
) -> End {
    while let Some(frame) = frames.next().await {
        let frame = match frame {
            Ok(frame) => frame,
            Err(error) => return End::Broken(error.to_string()),
        };
        let mut link = lock(link);
        if frame.kind == FrameType::Terminal {
            if let Some(term) = link.terminals.get(&frame.channel) {
                let _ = term
                    .output
                    .send(InvokeResponseBody::Raw(frame.payload.to_vec()));
            }
            continue;
        }
        // Forwarded as the service's JSON: only the type is read here. One message this side
        // cannot read is skipped; the connection and its terminals stay up.
        let value = serde_json::from_slice::<Value>(&frame.payload).ok();
        let Some(mut value) = value.filter(|value| value["type"].is_string()) else {
            eprintln!("hive-app: skipped a control message it cannot read");
            continue;
        };
        // Messages for a terminal this UI did not open (e.g. one closed by a reload) are
        // dropped.
        if frame.channel != 0 && !link.terminals.contains_key(&frame.channel) {
            continue;
        }
        value["channel"] = frame.channel.into();
        match value["type"].as_str().unwrap_or_default() {
            "welcome" => {
                link.welcome = Some(value.clone());
                // The UI always gets the projects after the handshake.
                let _ = link.send(0, &Control::ListProjects);
            }
            "version_mismatch" => {
                // The UI shows both sides, so it gets the app's own versions too.
                value["app_version"] = VERSION.into();
                value["app_protocol"] = PROTOCOL_VERSION.into();
                value["bundled"] = link.bundled.into();
                link.to_ui(value);
                link.frames = None;
                return End::Refused;
            }
            "terminal_exited" => drop(link.terminals.remove(&frame.channel)),
            "terminal_opened" => {
                if let Some(term) = link.terminals.get_mut(&frame.channel) {
                    term.worktree = value["worktree"].clone();
                }
            }
            // A path the app may open once (`open_path`).
            "editor_target" | "session_located" => {
                if let Some(path) = value["windows_path"].as_str() {
                    link.approve(path.to_owned());
                }
            }
            _ => link.follow(&value),
        }
        link.to_ui(value);
    }
    End::Closed
}

/// Ends the connection: every open terminal exits, then the UI gets `disconnected`.
fn disconnected(link: &mut Link, reason: String) {
    link.frames = None;
    link.welcome = None;
    // The agents end with the connection; a new one sends the live ones' states again.
    link.at_risk.clear();
    // A path from an ended connection never opens.
    link.approved.clear();
    for id in std::mem::take(&mut link.terminals).into_keys() {
        link.to_ui(json!({"type": "terminal_exited", "channel": id, "code": null}));
    }
    link.to_ui(json!({"type": "disconnected", "reason": reason, "bundled": link.bundled}));
}

/// What a request to quit does (D17.3): quit now, or ask the UI first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Quit {
    Now,
    Ask,
}

/// How long after asking the UI another request to quit quits at once (17.7's escape hatch:
/// ⌘Q twice quits even with a hung UI).
const ASK_AGAIN: Duration = Duration::from_secs(5);

/// The exit decision (D17.3, #47): ask first only when a UI is there to ask, the
/// `confirm_close` setting is on and `at_risk` agents would end with the app, unless the UI
/// was `asked` less than [`ASK_AGAIN`] ago.
fn quit_decision(ui: bool, confirm_close: bool, at_risk: usize, asked: Option<Duration>) -> Quit {
    let again = asked.is_some_and(|ago| ago < ASK_AGAIN);
    if ui && confirm_close && at_risk > 0 && !again {
        Quit::Ask
    } else {
        Quit::Now
    }
}

/// The states that make quitting ask first (#18), as the UI's `agentsAtRisk`: working, with
/// subagents, waiting for permission, a plan or an answer, or waiting for you.
fn at_risk(state: AgentState) -> bool {
    state.writes() || state == AgentState::WaitingYou
}

/// [`Hive::should_quit`] for the app's `Hive`; an app without one quits.
pub fn should_quit<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.try_state::<Hive>()
        .is_none_or(|hive| hive.should_quit(app))
}

/// Files dropped from another app (Finder) on the window, at (`x`, `y`) in points from the
/// window's top left (18.1): [`Hive::files_dropped`] for the app's `Hive`. False without one.
pub fn files_dropped<R: Runtime>(app: &AppHandle<R>, paths: &[String], x: f64, y: f64) -> bool {
    app.try_state::<Hive>()
        .is_some_and(|hive| hive.files_dropped(paths, x, y))
}

/// The webview's WebContent process died (18.21, macOS: wry's
/// `on_web_content_process_terminate`; WebKit kills one past its memory limit): the page
/// reloads, and its `connect` keeps the terminals ([`Hive::web_content_terminated`]).
pub fn web_content_terminated<R: Runtime>(webview: &tauri::Webview<R>) {
    if let Some(hive) = webview.try_state::<Hive>() {
        hive.web_content_terminated();
    }
    let _ = webview.reload();
}

/// A click on the Dock icon (`RunEvent::Reopen`): with no visible window (minimized, hidden)
/// the window comes back. True when it did.
pub fn reopen<R: Runtime>(app: &AppHandle<R>, has_visible_windows: bool) -> bool {
    !has_visible_windows && bring_up(app)
}

/// Handles the app's run events: on exit the connection ends before the process does.
/// Closing the window goes through the UI first, which confirms when agents are running.
pub fn on_run_event<R: Runtime>(app: &AppHandle<R>, event: RunEvent) {
    if let RunEvent::Exit = event {
        let hive = app.state::<Hive>();
        tauri::async_runtime::block_on(hive.shutdown(EXIT_WAIT));
        // An update downloaded but not applied is installed now: the next start runs it.
        let _ = hive.install_pending();
    }
}

/// The commands the UI calls (in a module: Tauri cannot export `pub` commands from the crate root).
pub mod commands {
    use super::*;
    use tauri::State;

    /// Sync on purpose: an `async` command's expansion in `main.rs` shows up as an uncovered
    /// line here. Sync commands run outside Tokio, which the bridge's tasks need, so enter it.
    #[tauri::command]
    pub fn connect(hive: State<'_, Hive>, on_message: Channel<Value>) {
        let runtime = tauri::async_runtime::handle();
        let _context = runtime.inner().enter();
        hive.connect(on_message);
    }

    /// Keeps the mode chosen (12.5.4), then reconnects on Tauri's runtime, like `check_update`.
    #[tauri::command]
    pub fn set_mode<R: Runtime>(app: AppHandle<R>, mode: String) -> Result<(), String> {
        app.state::<Hive>().choose_mode(&mode)?;
        tauri::async_runtime::spawn(async move {
            app.state::<Hive>().reconnect().await;
        });
        Ok(())
    }

    /// Sync like `connect`: the check runs on Tauri's runtime and answers on the UI channel.
    /// `manual`: the user asked, so "up to date" and a failure are answered too.
    #[tauri::command]
    pub fn check_update<R: Runtime>(app: AppHandle<R>, manual: Option<bool>) {
        use tauri_plugin_updater::UpdaterExt;
        let manual = manual == Some(true);
        tauri::async_runtime::spawn(async move {
            // Only a newer release, downloaded and signed, is offered (`update_ready`).
            app.state::<Hive>()
                .check_update(app.updater(), manual)
                .await;
        });
    }

    #[tauri::command]
    pub fn install_update<R: Runtime>(app: AppHandle<R>) {
        tauri::async_runtime::spawn(async move {
            app.state::<Hive>().install_update().await;
        });
    }

    #[tauri::command]
    pub fn open_terminal(
        hive: State<'_, Hive>,
        cwd: String,
        cols: u16,
        rows: u16,
        account: Option<AccountDir>,
        on_data: Channel<InvokeResponseBody>,
    ) -> Result<u32, String> {
        hive.open_terminal(cwd, cols, rows, account, on_data)
    }

    #[tauri::command]
    pub fn attach_terminal(
        hive: State<'_, Hive>,
        id: u32,
        on_data: Channel<InvokeResponseBody>,
    ) -> Result<(), String> {
        hive.attach_terminal(id, on_data)
    }

    #[tauri::command]
    pub fn write_terminal(hive: State<'_, Hive>, id: u32, data: String) -> Result<(), String> {
        hive.write_terminal(id, &data)
    }

    #[tauri::command]
    pub fn resize_terminal(
        hive: State<'_, Hive>,
        id: u32,
        cols: u16,
        rows: u16,
    ) -> Result<(), String> {
        hive.resize_terminal(id, cols, rows)
    }

    #[tauri::command]
    pub fn close_terminal(hive: State<'_, Hive>, id: u32) -> Result<(), String> {
        hive.close_terminal(id)
    }

    #[tauri::command]
    pub fn ack_terminal(hive: State<'_, Hive>, id: u32, bytes: u32) -> Result<(), String> {
        hive.ack_terminal(id, bytes)
    }

    #[tauri::command]
    pub fn list_projects(hive: State<'_, Hive>) -> Result<(), String> {
        hive.list_projects()
    }

    #[tauri::command]
    pub fn add_project(hive: State<'_, Hive>, path: String) -> Result<(), String> {
        hive.add_project(path)
    }

    #[tauri::command]
    pub fn remove_project(hive: State<'_, Hive>, id: String) -> Result<(), String> {
        hive.remove_project(id)
    }

    #[tauri::command]
    pub fn list_branches(hive: State<'_, Hive>, project: String) -> Result<(), String> {
        hive.list_branches(project)
    }

    #[tauri::command]
    pub fn validate_worktree_name(
        hive: State<'_, Hive>,
        project: String,
        name: String,
    ) -> Result<(), String> {
        hive.validate_worktree_name(project, name)
    }

    #[tauri::command]
    pub fn create_worktree(
        hive: State<'_, Hive>,
        project: String,
        name: String,
        base: Option<String>,
    ) -> Result<(), String> {
        hive.create_worktree(project, name, base)
    }

    #[tauri::command]
    pub fn create_issue_worktree(
        hive: State<'_, Hive>,
        project: String,
        number: u64,
        base: Option<String>,
    ) -> Result<(), String> {
        hive.create_issue_worktree(project, number, base)
    }

    #[tauri::command]
    pub fn remove_worktree(hive: State<'_, Hive>, path: String, force: bool) -> Result<(), String> {
        hive.remove_worktree(path, force)
    }

    #[tauri::command]
    pub fn rename_worktree(
        hive: State<'_, Hive>,
        path: String,
        name: String,
    ) -> Result<(), String> {
        hive.rename_worktree(path, name)
    }

    #[tauri::command]
    pub fn list_sessions(hive: State<'_, Hive>) -> Result<(), String> {
        hive.list_sessions()
    }

    #[tauri::command]
    pub fn locate_session(
        hive: State<'_, Hive>,
        id: String,
        target: SessionTarget,
    ) -> Result<(), String> {
        hive.locate_session(id, target)
    }

    #[tauri::command]
    pub fn delete_session(hive: State<'_, Hive>, id: String) -> Result<(), String> {
        hive.delete_session(id)
    }

    #[tauri::command]
    pub fn list_dirs(hive: State<'_, Hive>, path: String, windows: bool) -> Result<(), String> {
        hive.list_dirs(path, windows)
    }

    #[tauri::command]
    pub fn search_files(
        hive: State<'_, Hive>,
        worktree: String,
        query: String,
    ) -> Result<(), String> {
        hive.search_files(worktree, query)
    }

    #[tauri::command]
    pub fn watch_worktree(
        hive: State<'_, Hive>,
        path: String,
        base: DiffBase,
    ) -> Result<(), String> {
        hive.watch_worktree(path, base)
    }

    #[tauri::command]
    pub fn unwatch_worktree(hive: State<'_, Hive>) -> Result<(), String> {
        hive.unwatch_worktree()
    }

    #[tauri::command]
    pub fn expand_ignored(
        hive: State<'_, Hive>,
        path: String,
        folders: Vec<String>,
    ) -> Result<(), String> {
        hive.expand_ignored(path, folders)
    }

    #[tauri::command]
    pub fn set_view(
        hive: State<'_, Hive>,
        terminal: Option<u32>,
        focused: bool,
    ) -> Result<(), String> {
        hive.set_view(terminal, focused)
    }

    #[tauri::command]
    pub fn list_changes(hive: State<'_, Hive>, path: String, base: DiffBase) -> Result<(), String> {
        hive.list_changes(path, base)
    }

    #[tauri::command]
    pub fn open_file(
        hive: State<'_, Hive>,
        worktree: String,
        path: String,
        base: DiffBase,
    ) -> Result<(), String> {
        hive.open_file(worktree, path, base)
    }

    #[tauri::command]
    pub fn save_file(
        hive: State<'_, Hive>,
        worktree: String,
        path: String,
        content: String,
        version: Option<String>,
    ) -> Result<(), String> {
        hive.save_file(worktree, path, content, version)
    }

    #[tauri::command]
    pub fn create_file(
        hive: State<'_, Hive>,
        worktree: String,
        folder: String,
        name: String,
    ) -> Result<(), String> {
        hive.create_file(worktree, folder, name)
    }

    #[tauri::command]
    pub fn rename_file(
        hive: State<'_, Hive>,
        worktree: String,
        path: String,
        name: String,
    ) -> Result<(), String> {
        hive.rename_file(worktree, path, name)
    }

    #[tauri::command]
    pub fn move_file(
        hive: State<'_, Hive>,
        worktree: String,
        path: String,
        folder: String,
    ) -> Result<(), String> {
        hive.move_file(worktree, path, folder)
    }

    #[tauri::command]
    pub fn delete_file(
        hive: State<'_, Hive>,
        worktree: String,
        path: String,
    ) -> Result<(), String> {
        hive.delete_file(worktree, path)
    }

    #[tauri::command]
    pub fn create_folder(
        hive: State<'_, Hive>,
        worktree: String,
        folder: String,
        name: String,
    ) -> Result<(), String> {
        hive.create_folder(worktree, folder, name)
    }

    #[tauri::command]
    pub fn open_in_editor(
        hive: State<'_, Hive>,
        worktree: String,
        path: String,
    ) -> Result<(), String> {
        hive.open_in_editor(worktree, path)
    }

    // ponytail: sync like every command here (an `async` command left a macro line uncovered);
    // opening a `\\wsl.localhost` path may briefly wait for WSL on the main thread.
    #[tauri::command]
    pub fn open_path(hive: State<'_, Hive>, path: String, reveal: bool) -> Result<(), String> {
        hive.open_path(path, reveal)
    }

    #[tauri::command]
    pub fn get_settings(hive: State<'_, Hive>) -> Result<(), String> {
        hive.get_settings()
    }

    #[tauri::command]
    pub fn set_settings(hive: State<'_, Hive>, settings: Settings) -> Result<(), String> {
        hive.set_settings(settings)
    }

    #[tauri::command]
    pub fn create_space(hive: State<'_, Hive>, name: String, env: SpaceEnv) -> Result<(), String> {
        hive.create_space(name, env)
    }

    #[tauri::command]
    pub fn update_space(
        hive: State<'_, Hive>,
        id: String,
        name: String,
        env: SpaceEnv,
    ) -> Result<(), String> {
        hive.update_space(id, name, env)
    }

    #[tauri::command]
    pub fn delete_space(hive: State<'_, Hive>, id: String) -> Result<(), String> {
        hive.delete_space(id)
    }

    #[tauri::command]
    pub fn select_space(hive: State<'_, Hive>, id: String) -> Result<(), String> {
        hive.select_space(id)
    }

    #[tauri::command]
    pub fn list_gh_accounts(
        hive: State<'_, Hive>,
        gh_config_dir: Option<String>,
    ) -> Result<(), String> {
        hive.list_gh_accounts(gh_config_dir)
    }

    #[tauri::command]
    pub fn switch_gh_account(
        hive: State<'_, Hive>,
        gh_config_dir: Option<String>,
        account: GhAccount,
    ) -> Result<(), String> {
        hive.switch_gh_account(gh_config_dir, account)
    }

    #[tauri::command]
    pub fn list_pulls(hive: State<'_, Hive>, project: String, force: bool) -> Result<(), String> {
        hive.list_pulls(project, force)
    }

    #[tauri::command]
    pub fn open_pull(hive: State<'_, Hive>, project: String, number: u64) -> Result<(), String> {
        hive.open_pull(project, number)
    }

    #[tauri::command]
    pub fn act_on_pull(
        hive: State<'_, Hive>,
        project: String,
        number: u64,
        action: PullAction,
    ) -> Result<(), String> {
        hive.act_on_pull(project, number, action)
    }

    #[tauri::command]
    pub fn create_pull(
        hive: State<'_, Hive>,
        worktree: String,
        title: String,
        body: String,
        base: String,
        draft: bool,
    ) -> Result<(), String> {
        hive.create_pull(worktree, title, body, base, draft)
    }

    #[tauri::command]
    pub fn list_runs(
        hive: State<'_, Hive>,
        project: String,
        branch: Option<String>,
        force: bool,
    ) -> Result<(), String> {
        hive.list_runs(project, branch, force)
    }

    #[tauri::command]
    pub fn open_run(hive: State<'_, Hive>, project: String, run: u64) -> Result<(), String> {
        hive.open_run(project, run)
    }

    #[tauri::command]
    pub fn open_job_log(hive: State<'_, Hive>, project: String, job: u64) -> Result<(), String> {
        hive.open_job_log(project, job)
    }

    #[tauri::command]
    pub fn act_on_run(
        hive: State<'_, Hive>,
        project: String,
        run: u64,
        action: RunAction,
        branch: Option<String>,
    ) -> Result<(), String> {
        hive.act_on_run(project, run, action, branch)
    }

    #[tauri::command]
    pub fn open_settings_file(hive: State<'_, Hive>) -> Result<(), String> {
        hive.open_settings_file()
    }

    /// What the UI does on a double click on the empty title bar (macOS): `zoom`, `minimize`
    /// or `none`.
    #[tauri::command]
    pub fn title_bar_action(hive: State<'_, Hive>) -> &'static str {
        hive.title_bar_action()
    }

    #[tauri::command]
    pub fn get_diagnostics(hive: State<'_, Hive>) -> Result<(), String> {
        hive.get_diagnostics()
    }

    #[tauri::command]
    pub fn show_notification<R: Runtime>(
        app: AppHandle<R>,
        hive: State<'_, Hive>,
        title: String,
        body: String,
        agent: String,
    ) -> Result<(), String> {
        hive.show_notification(&app, &title, &body, agent)
    }

    #[tauri::command]
    pub fn check_notifications(hive: State<'_, Hive>) {
        hive.check_notifications();
    }

    #[tauri::command]
    pub fn open_notification_settings(hive: State<'_, Hive>) -> Result<(), String> {
        hive.open_notification_settings()
    }

    #[tauri::command]
    pub fn play_tone(hive: State<'_, Hive>, volume: u8) -> Result<(), String> {
        hive.play_tone(volume)
    }

    /// The files copied in Finder as pasting them types them (18.3): their quoted paths, empty
    /// for none. Sync, so it runs on the main thread, where AppKit is used.
    #[tauri::command]
    pub fn pasted_files(hive: State<'_, Hive>) -> String {
        hive.pasted_files()
    }

    /// The menu bar icon (17.22) for what the UI sent: `shown` (the setting), the most urgent
    /// state (and whether it is pending), the pending agents and the usage ring's lines.
    #[tauri::command]
    pub fn set_tray(
        hive: State<'_, Hive>,
        shown: bool,
        urgent: Option<(AgentState, bool)>,
        agents: Vec<tray::Agent>,
        usage: Vec<String>,
    ) -> Result<(), String> {
        hive.set_tray(shown, urgent, &agents, &usage)
    }
}

#[cfg(target_os = "macos")]
pub mod macos;
pub mod menu;
pub mod quote;
pub mod tray;

#[cfg(test)]
mod tests;
