//! The agent's browser (24.6, D22.3): an installed Chrome, Chromium, Edge or Brave run headless
//! with a temporary profile, its DevTools (CDP) port on `127.0.0.1`; the agent drives it through
//! the `hive-browser` MCP server (Playwright MCP, `hive browser-mcp`), and the app watches it
//! through a CDP screencast (view only: nothing is ever sent to the page).

use std::ffi::{OsStr, OsString};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use hive_protocol::{Control, Frame, FrameCodec, Role};
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tokio_util::codec::Framed;

use crate::VERSION;
use crate::paths::Paths;

/// The macOS apps looked for, in this order, in `/Applications` and `~/Applications`.
const APPS: [&str; 4] = [
    "Google Chrome.app/Contents/MacOS/Google Chrome",
    "Chromium.app/Contents/MacOS/Chromium",
    "Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    "Brave Browser.app/Contents/MacOS/Brave Browser",
];

/// The programs looked for on `PATH` on Linux (and in WSL), in this order.
const PROGRAMS: [&str; 6] = [
    "google-chrome",
    "google-chrome-stable",
    "chromium",
    "chromium-browser",
    "microsoft-edge",
    "brave-browser",
];

/// The image the panel gets, fixed (the human, 24.5): JPEG quality and width in pixels.
const QUALITY: u32 = 60;
const WIDTH: u32 = 1280;

/// How long a starting browser gets to write its `DevToolsActivePort`.
pub const START_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a browser gets to end on SIGTERM before it is killed.
const STOP_GRACE: Duration = Duration::from_secs(2);
/// The largest CDP message read; a bigger one ends the connection.
const MAX_MESSAGE: usize = 4 << 20;
/// The largest frame forwarded (base64): its control message stays under `MAX_PAYLOAD`.
pub const MAX_FRAME: usize = 3 << 20;
/// The longest page URL shown, in characters.
const MAX_URL: usize = 2048;
/// The most pages followed; older ones are forgotten.
const MAX_PAGES: usize = 100;
/// The most `DevToolsActivePort` may hold.
const MAX_PORT_FILE: u64 = 1024;
/// How long `hive browser-mcp` waits for the service to start the browser.
const ENDPOINT_TIMEOUT: Duration = Duration::from_secs(30);

/// The browser to run: `setting` (`browser.path`) when set, else the first installed one of
/// `candidates`. A snap Chromium (under `snap`) is skipped: its confinement cannot use Hive's
/// temporary profile.
pub fn find(
    setting: Option<&str>,
    candidates: impl IntoIterator<Item = PathBuf>,
    snap: &Path,
) -> Result<PathBuf, String> {
    if let Some(path) = setting {
        let path = PathBuf::from(path);
        return if program(&path) {
            Ok(path)
        } else {
            Err(format!(
                "browser.path {} is not a program Hive can run",
                path.display()
            ))
        };
    }
    let mut snapped = None;
    for candidate in candidates.into_iter().filter(|c| program(c)) {
        if is_snap(&candidate, snap) {
            snapped.get_or_insert(candidate);
        } else {
            return Ok(candidate);
        }
    }
    Err(match snapped {
        Some(path) => format!(
            "Only a snap browser was found ({}), which cannot run with Hive's temporary profile: set browser.path to another one",
            path.display()
        ),
        None => {
            "No Chrome, Chromium, Edge or Brave found: install one or set browser.path".to_owned()
        }
    })
}

/// Where this platform's browsers install (see [`find`]).
pub fn candidates(home: Option<&Path>, path: Option<&OsStr>) -> Vec<PathBuf> {
    if cfg!(target_os = "macos") {
        let roots = [
            Some(PathBuf::from("/Applications")),
            home.map(|h| h.join("Applications")),
        ];
        app_candidates(roots.into_iter().flatten())
    } else if cfg!(target_os = "linux") {
        path_candidates(path)
    } else {
        // Native Windows gets no browser (D17.0): the service in WSL finds one there.
        Vec::new()
    }
}

/// Each app of [`APPS`] in each of `roots`, folder by folder.
fn app_candidates(roots: impl Iterator<Item = PathBuf>) -> Vec<PathBuf> {
    roots
        .flat_map(|root| APPS.iter().map(move |app| root.join(app)))
        .collect()
}

/// Each program of [`PROGRAMS`] in each absolute folder of `path`, program by program.
fn path_candidates(path: Option<&OsStr>) -> Vec<PathBuf> {
    let dirs: Vec<PathBuf> = path
        .map(|p| {
            std::env::split_paths(p)
                .filter(|d| d.is_absolute())
                .collect()
        })
        .unwrap_or_default();
    PROGRAMS
        .iter()
        .flat_map(|name| dirs.iter().map(move |dir| dir.join(name)))
        .collect()
}

/// An executable regular file (links followed).
fn program(path: &Path) -> bool {
    path.is_absolute()
        && path
            .metadata()
            .is_ok_and(|m| m.is_file() && (cfg!(windows) || crate::mode::executable(&m)))
}

/// Whether `path` is a snap's: it resolves under `snap`, or it is a small script that runs one
/// (Ubuntu's `chromium-browser`).
fn is_snap(path: &Path, snap: &Path) -> bool {
    let root = crate::paths::canonical(snap).unwrap_or_else(|_| snap.to_owned());
    if crate::paths::canonical(path).is_ok_and(|p| p.starts_with(&root)) {
        return true;
    }
    let mut head = Vec::new();
    let read = std::fs::File::open(path).and_then(|f| f.take(64 * 1024).read_to_end(&mut head));
    let needle = format!("{}/", snap.display());
    read.is_ok()
        && head.starts_with(b"#!")
        && head.windows(needle.len()).any(|w| w == needle.as_bytes())
}

/// The browser's arguments, each separate: headless, the temporary `profile`, DevTools on a
/// random port of `127.0.0.1`, nothing asked of the system keychain, one blank page.
pub fn args(profile: &Path) -> Vec<OsString> {
    let mut dir = OsString::from("--user-data-dir=");
    dir.push(profile);
    let flags = [
        "--headless=new",
        "--remote-debugging-port=0",
        "--remote-debugging-address=127.0.0.1",
        "--no-first-run",
        "--no-default-browser-check",
        "--use-mock-keychain",
        "--password-store=basic",
        "--window-size=1280,800",
    ];
    let mut out = vec![dir];
    out.extend(flags.iter().map(OsString::from));
    out.push("about:blank".into());
    out
}

/// The port and browser path a browser wrote to `<profile>/DevToolsActivePort`, once it has:
/// a port, then `/devtools/browser/<id>` (letters, digits and `-` only).
pub fn active_port(text: &str) -> Option<(u16, String)> {
    let mut lines = text.lines();
    let port: u16 = lines.next()?.parse().ok().filter(|&p| p != 0)?;
    let path = lines.next()?;
    let id = path.strip_prefix("/devtools/browser/")?;
    let safe = !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    safe.then(|| (port, path.to_owned()))
}

/// A running browser: its process (its own process group) and its temporary profile.
pub struct Process {
    child: tokio::process::Child,
    profile: PathBuf,
}

/// Starts `program` with a new temporary `profile` and connects to its DevTools: the process,
/// the CDP connection and the port. Anything failing on the way stops the process.
pub async fn start(
    program: &Path,
    profile: PathBuf,
) -> Result<(Process, WebSocketStream<TcpStream>, u16), String> {
    let _ = std::fs::remove_dir_all(&profile);
    crate::mode::folder(std::fs::DirBuilder::new().recursive(true), 0o700)
        .create(&profile)
        .map_err(|err| {
            format!(
                "Cannot make the browser's profile {}: {err}",
                profile.display()
            )
        })?;
    let mut command = tokio::process::Command::new(program);
    command
        .args(args(&profile))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    // Its helpers in its group, so stopping it ends them too.
    #[cfg(unix)]
    command.process_group(0);
    let child = command.spawn();
    let child = match child {
        Ok(child) => child,
        Err(err) => {
            let _ = std::fs::remove_dir_all(&profile);
            return Err(format!("Cannot start {}: {err}", program.display()));
        }
    };
    let mut process = Process { child, profile };
    match connect(&mut process).await {
        Ok((ws, port)) => Ok((process, ws, port)),
        Err(err) => {
            process.stop().await;
            Err(err)
        }
    }
}

/// Waits for the browser's `DevToolsActivePort`, then opens its CDP connection.
async fn connect(process: &mut Process) -> Result<(WebSocketStream<TcpStream>, u16), String> {
    let file = process.profile.join("DevToolsActivePort");
    let wait = async {
        loop {
            if let Ok(Some(status)) = process.child.try_wait() {
                return Err(format!("The browser exited as it started ({status})"));
            }
            let mut text = String::new();
            let read = std::fs::File::open(&file)
                .and_then(|f| f.take(MAX_PORT_FILE).read_to_string(&mut text));
            if let Some(found) = read.ok().and_then(|_| active_port(&text)) {
                return Ok(found);
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    let (port, path) = tokio::time::timeout(START_TIMEOUT, wait)
        .await
        .map_err(|_| "The browser did not open its DevTools port in time".to_owned())??;
    let lost = |err: &dyn std::fmt::Display| format!("Cannot connect to the browser: {err}");
    // Only to the loopback address: the URL never picks the host.
    let stream = TcpStream::connect(("127.0.0.1", port))
        .await
        .map_err(|e| lost(&e))?;
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE));
    let url = format!("ws://127.0.0.1:{port}{path}");
    let (ws, _) = tokio_tungstenite::client_async_with_config(url, stream, Some(config))
        .await
        .map_err(|e| lost(&e))?;
    Ok((ws, port))
}

impl Process {
    /// Ends the browser (SIGTERM to its group, SIGKILL after [`STOP_GRACE`]) and deletes its
    /// profile.
    pub async fn stop(mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.child.id().and_then(|id| i32::try_from(id).ok()) {
            use nix::sys::signal::{Signal, killpg};
            use nix::unistd::Pid;
            let _ = killpg(Pid::from_raw(pid), Signal::SIGTERM);
            if tokio::time::timeout(STOP_GRACE, self.child.wait())
                .await
                .is_err()
            {
                let _ = killpg(Pid::from_raw(pid), Signal::SIGKILL);
            }
        }
        let _ = self.child.kill().await;
        let _ = std::fs::remove_dir_all(&self.profile);
    }
}

/// What the service asks of a browser's screencast.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    /// The panel shows this browser (`true`, restarting the screencast) or no longer does.
    Watch(bool),
    /// The app drew the last frame.
    Drawn,
}

/// What a browser's screencast tells the service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The page shown is now at this URL (`None`: no page).
    Url(Option<String>),
    /// A frame for the app: a JPEG in base64.
    Frame(String),
}

/// The screencast of one browser, as a state machine over CDP messages: it follows the pages
/// (the one last opened or navigated is shown), attaches to the shown one while watched, and
/// lets one frame at a time reach the app, acknowledging each to the browser once drawn (the
/// browser sends no more than two unacknowledged; the newest waits here).
#[derive(Debug, Default)]
pub struct Screen {
    /// The last CDP command id used.
    id: u64,
    /// The pages, `(target id, url)`; the last one is shown.
    pages: Vec<(String, String)>,
    watched: bool,
    /// The target attached to (or being attached to).
    target: Option<String>,
    /// The id of the `Target.attachToTarget` awaited.
    attaching: Option<u64>,
    /// The attached target's CDP session.
    session: Option<String>,
    /// The frame the app has, not yet drawn (its screencast frame number).
    pending: Option<u64>,
    /// The newest frame not yet sent to the app.
    queued: Option<(u64, String)>,
    /// The URL last reported.
    url: Option<String>,
}

/// Messages to the browser (`Value`s) and events for the service.
#[derive(Debug, Default, PartialEq)]
pub struct Out {
    pub cdp: Vec<Value>,
    pub events: Vec<Event>,
}

impl Screen {
    /// The first message: report every target, now and as they come.
    pub fn start(&mut self) -> Out {
        let mut out = Out::default();
        self.send(
            &mut out,
            None,
            "Target.setDiscoverTargets",
            json!({ "discover": true }),
        );
        out
    }

    /// Applies one of the service's commands.
    pub fn command(&mut self, command: Command) -> Out {
        let mut out = Out::default();
        match command {
            Command::Watch(on) => {
                // Restarted when already watched: a reloaded app has no frame pending.
                self.watched = false;
                self.follow(&mut out);
                self.watched = on;
                self.follow(&mut out);
            }
            Command::Drawn => {
                if let (Some(frame), Some(session)) = (self.pending.take(), self.session.clone()) {
                    let ack = json!({ "sessionId": frame });
                    self.send(&mut out, Some(&session), "Page.screencastFrameAck", ack);
                }
                if let Some((frame, data)) = self.queued.take() {
                    self.pending = Some(frame);
                    out.events.push(Event::Frame(data));
                }
            }
        }
        out
    }

    /// Applies one message from the browser; anything unexpected is ignored.
    pub fn message(&mut self, message: &Value) -> Out {
        let mut out = Out::default();
        let params = &message["params"];
        let info = &params["targetInfo"];
        match message["method"].as_str() {
            Some("Target.targetCreated" | "Target.targetInfoChanged") if info["type"] == "page" => {
                if let (Some(id), Some(url)) = (info["targetId"].as_str(), info["url"].as_str()) {
                    self.page(id, url);
                }
            }
            Some("Target.targetDestroyed") => {
                let id = params["targetId"].as_str();
                self.pages.retain(|(target, _)| Some(target.as_str()) != id);
                if self.target.as_deref() == id {
                    self.detached();
                }
            }
            Some("Target.detachedFromTarget") => {
                if self.session.is_some() && params["sessionId"].as_str() == self.session.as_deref()
                {
                    self.detached();
                }
            }
            Some("Page.screencastFrame") => self.frame(message, &mut out),
            Some(_) => {}
            None => self.answer(message, &mut out),
        }
        self.follow(&mut out);
        out
    }

    /// A page created or changed: shown when it is new or its URL changed (navigated).
    fn page(&mut self, id: &str, url: &str) {
        let url = crate::adapter::clip(url, MAX_URL);
        if self.pages.iter().any(|(t, u)| t == id && *u == url) {
            return;
        }
        self.pages.retain(|(t, _)| t != id);
        self.pages.push((id.to_owned(), url));
        if self.pages.len() > MAX_PAGES {
            self.pages.remove(0);
        }
    }

    /// The attached target or its session is gone.
    fn detached(&mut self) {
        self.target = None;
        self.attaching = None;
        self.session = None;
        self.pending = None;
        self.queued = None;
    }

    /// An answer to one of our commands: only `Target.attachToTarget`'s matters.
    fn answer(&mut self, message: &Value, out: &mut Out) {
        let session = message["result"]["sessionId"].as_str();
        if self.attaching.is_some() && message["id"].as_u64() == self.attaching {
            self.attaching = None;
            let Some(session) = session else {
                // Refused (the page went away): not tried again until the pages change.
                return;
            };
            self.session = Some(session.to_owned());
            let params = json!({
                "format": "jpeg",
                "quality": QUALITY,
                "maxWidth": WIDTH,
                "maxHeight": WIDTH,
                "everyNthFrame": 1,
            });
            self.send(out, Some(session), "Page.startScreencast", params);
        } else if let Some(session) = session {
            // A late attach to a page no longer wanted.
            let params = json!({ "sessionId": session });
            self.send(out, None, "Target.detachFromTarget", params);
        }
    }

    /// A screencast frame of the attached page: to the app, or queued while it draws one.
    fn frame(&mut self, message: &Value, out: &mut Out) {
        let ours =
            self.session.is_some() && message["sessionId"].as_str() == self.session.as_deref();
        let params = &message["params"];
        let (Some(frame), Some(data)) = (params["sessionId"].as_u64(), params["data"].as_str())
        else {
            return;
        };
        if !ours || !base64(data) {
            return;
        }
        if self.pending.is_none() {
            self.pending = Some(frame);
            out.events.push(Event::Frame(data.to_owned()));
        } else if let Some((older, _)) = self.queued.replace((frame, data.to_owned()))
            && let Some(session) = self.session.clone()
        {
            // Replaced unseen: the browser may send the next.
            let ack = json!({ "sessionId": older });
            self.send(out, Some(&session), "Page.screencastFrameAck", ack);
        }
    }

    /// Attaches to the page to show while watched (detaching from another first), and reports
    /// the shown page's URL when it changed.
    fn follow(&mut self, out: &mut Out) {
        let shown = self.pages.last().map(|(target, _)| target.clone());
        let wanted = shown.clone().filter(|_| self.watched);
        if self.target != wanted {
            if let Some(session) = self.session.clone() {
                self.send(out, Some(&session), "Page.stopScreencast", json!({}));
                let params = json!({ "sessionId": session });
                self.send(out, None, "Target.detachFromTarget", params);
            }
            self.detached();
            if let Some(target) = wanted {
                let params = json!({ "targetId": target, "flatten": true });
                self.attaching = Some(self.send(out, None, "Target.attachToTarget", params));
                self.target = Some(target);
            }
        }
        let url = self.pages.last().map(|(_, url)| url.clone());
        if url != self.url {
            self.url.clone_from(&url);
            out.events.push(Event::Url(url));
        }
    }

    /// Queues one CDP command; its id.
    fn send(&mut self, out: &mut Out, session: Option<&str>, method: &str, params: Value) -> u64 {
        self.id += 1;
        let mut message = json!({ "id": self.id, "method": method, "params": params });
        if let Some(session) = session {
            message["sessionId"] = json!(session);
        }
        out.cdp.push(message);
        self.id
    }
}

/// Only base64's characters, at most [`MAX_FRAME`]: safe in a `data:` URL.
fn base64(data: &str) -> bool {
    !data.is_empty()
        && data.len() <= MAX_FRAME
        && data
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/' | b'='))
}

/// Runs one browser's screencast over its CDP connection until it closes: the service's
/// `commands` in, `events` out. Returns when the browser is gone (or stopped).
pub async fn screencast<S>(
    mut ws: WebSocketStream<S>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    mut events: impl FnMut(Event),
) where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    let mut screen = Screen::default();
    let mut out = screen.start();
    loop {
        for message in out.cdp.drain(..) {
            if ws.send(Message::text(message.to_string())).await.is_err() {
                return;
            }
        }
        out.events.drain(..).for_each(&mut events);
        out = tokio::select! {
            message = ws.next() => match message {
                Some(Ok(Message::Text(text))) => match serde_json::from_str::<Value>(&text) {
                    Ok(value) => screen.message(&value),
                    Err(_) => Out::default(),
                },
                Some(Ok(_)) => Out::default(),
                Some(Err(_)) | None => return,
            },
            command = commands.recv() => match command {
                Some(command) => screen.command(command),
                None => return,
            },
        };
    }
}

/// `hive browser-mcp`'s question to the service: the CDP endpoint of the browser of the
/// worktree holding `cwd`, started first if needed.
pub async fn endpoint(paths: &Paths, cwd: &str) -> Result<String, String> {
    let asked = async {
        let stream = paths
            .connect()
            .await
            .map_err(|err| format!("cannot reach the Hive service: {err}"))?;
        let mut framed = Framed::new(stream, FrameCodec);
        let hello = Control::hello(Role::Hook, VERSION);
        let ask = Control::BrowserEndpoint {
            cwd: cwd.to_owned(),
        };
        for message in [hello, ask] {
            framed
                .send(Frame::control(0, &message))
                .await
                .map_err(|err| err.to_string())?;
        }
        while let Some(frame) = framed.next().await {
            match frame.map_err(|err| err.to_string())?.to_control() {
                Ok(Control::BrowserReady { endpoint }) => return Ok(endpoint),
                Ok(Control::Error { message }) => return Err(message),
                Ok(Control::VersionMismatch { version, .. }) => {
                    return Err(format!(
                        "the Hive service is version {version}, not {VERSION}"
                    ));
                }
                _ => {}
            }
        }
        Err("the Hive service closed the connection".to_owned())
    };
    tokio::time::timeout(ENDPOINT_TIMEOUT, asked)
        .await
        .unwrap_or_else(|_| Err("the Hive service did not start the browser in time".to_owned()))
}

/// The Playwright MCP's command for `endpoint`: `bunx`, else `npx`, from the first absolute
/// folder of `path` holding it; each argument separate.
pub fn mcp_command(path: Option<&OsStr>, endpoint: &str) -> Option<(PathBuf, Vec<String>)> {
    let dirs: Vec<PathBuf> = path
        .map(|p| {
            std::env::split_paths(p)
                .filter(|d| d.is_absolute())
                .collect()
        })
        .unwrap_or_default();
    let package = "@playwright/mcp@latest";
    let runners: [(&str, &[&str]); 2] = [("bunx", &[package]), ("npx", &["-y", package])];
    runners.iter().find_map(|(name, args)| {
        let program = dirs.iter().map(|dir| dir.join(name)).find(|p| program(p))?;
        let mut args: Vec<String> = args.iter().map(|&a| a.to_owned()).collect();
        args.extend(["--cdp-endpoint".to_owned(), endpoint.to_owned()]);
        Some((program, args))
    })
}

/// `hive browser-mcp`: runs the Playwright MCP on the agent's browser of this worktree, its
/// stdio the agent's. On Unix it becomes that program; it returns only on a failure.
pub async fn mcp(paths: &Paths) -> io::Result<()> {
    let cwd = std::env::current_dir()?;
    let cwd = cwd.to_string_lossy();
    let endpoint = endpoint(paths, &cwd).await.map_err(io::Error::other)?;
    let path = std::env::var_os("PATH");
    let (program, args) = mcp_command(path.as_deref(), &endpoint)
        .ok_or_else(|| io::Error::other("neither bunx nor npx is on PATH"))?;
    let mut command = std::process::Command::new(&program);
    command.args(&args);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let err = command.exec();
        Err(io::Error::other(format!(
            "cannot run {}: {err}",
            program.display()
        )))
    }
    #[cfg(windows)]
    {
        let status = command.status()?;
        status
            .success()
            .then_some(())
            .ok_or_else(|| io::Error::other(format!("{} ended with {status}", program.display())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn exe(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::Permissions::from_mode(0o755);
            std::fs::set_permissions(path, mode).unwrap();
        }
    }

    #[test]
    fn the_setting_wins_else_the_first_installed_candidate() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (tmp.path().join("a/chrome"), tmp.path().join("b/chromium"));
        exe(&b, "");
        let snap = tmp.path().join("snap");
        let both = || [a.clone(), b.clone()];
        assert_eq!(find(None, both(), &snap), Ok(b.clone()));
        exe(&a, "");
        assert_eq!(find(None, both(), &snap), Ok(a.clone()));
        let set = b.to_str().unwrap();
        assert_eq!(find(Some(set), both(), &snap), Ok(b.clone()));
        let missing = tmp.path().join("none");
        let err = find(Some(missing.to_str().unwrap()), both(), &snap).unwrap_err();
        assert_eq!(
            err,
            format!(
                "browser.path {} is not a program Hive can run",
                missing.display()
            )
        );
        assert!(find(Some("chrome"), both(), &snap).is_err());
        let none = find(None, [missing], &snap).unwrap_err();
        assert_eq!(
            none,
            "No Chrome, Chromium, Edge or Brave found: install one or set browser.path"
        );
    }

    #[cfg(unix)]
    #[test]
    fn only_executable_files_count() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let plain = tmp.path().join("plain");
        std::fs::write(&plain, "").unwrap();
        std::fs::set_permissions(&plain, std::fs::Permissions::from_mode(0o644)).unwrap();
        let folder = tmp.path().join("folder");
        std::fs::create_dir(&folder).unwrap();
        assert!(!program(&plain));
        assert!(!program(&folder));
        assert!(!program(Path::new("relative")));
    }

    #[cfg(unix)]
    #[test]
    fn a_snap_browser_is_skipped_with_a_message() {
        let tmp = tempfile::tempdir().unwrap();
        let snap = tmp.path().join("snap");
        let real = snap.join("bin/chromium");
        exe(&real, "");
        let link = tmp.path().join("bin/chromium");
        std::fs::create_dir_all(link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        // Ubuntu's transitional script.
        let script = tmp.path().join("usr/chromium-browser");
        exe(
            &script,
            &format!("#!/bin/sh\nexec {} \"$@\"\n", real.display()),
        );
        let other = tmp.path().join("opt/brave-browser");
        let found = |list: &[&PathBuf]| find(None, list.iter().map(|p| (*p).clone()), &snap);
        let err = found(&[&link, &script]).unwrap_err();
        assert!(
            err.starts_with(&format!(
                "Only a snap browser was found ({})",
                link.display()
            )),
            "{err}"
        );
        assert!(err.ends_with("set browser.path to another one"), "{err}");
        exe(&other, "#!/bin/sh\necho /snapshot\n");
        assert_eq!(found(&[&link, &script, &other]), Ok(other.clone()));
    }

    #[test]
    fn candidates_are_the_platforms_install_places() {
        let home = Path::new("/h");
        let path = std::env::join_paths(["/usr/bin", "rel", "/opt/b"]).unwrap();
        let found = candidates(Some(home), Some(&path));
        if cfg!(target_os = "macos") {
            assert_eq!(found.len(), 8);
            assert_eq!(
                found[0],
                Path::new("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
            );
            assert_eq!(
                found[4],
                Path::new("/h/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")
            );
            assert_eq!(
                found[7],
                Path::new("/h/Applications/Brave Browser.app/Contents/MacOS/Brave Browser")
            );
        } else if cfg!(target_os = "linux") {
            assert_eq!(found.len(), 12);
            assert_eq!(found[0], Path::new("/usr/bin/google-chrome"));
            assert_eq!(found[1], Path::new("/opt/b/google-chrome"));
            assert_eq!(found[11], Path::new("/opt/b/brave-browser"));
        } else {
            assert!(found.is_empty());
        }
        // Both forms, whatever the platform.
        assert_eq!(app_candidates([PathBuf::from("/A")].into_iter()).len(), 4);
        assert_eq!(path_candidates(None), Vec::<PathBuf>::new());
        assert_eq!(path_candidates(Some(&path)).len(), 12);
    }

    #[test]
    fn the_command_line_is_separate_arguments() {
        let args = args(Path::new("/p/it's a profile"));
        let expected = [
            "--user-data-dir=/p/it's a profile",
            "--headless=new",
            "--remote-debugging-port=0",
            "--remote-debugging-address=127.0.0.1",
            "--no-first-run",
            "--no-default-browser-check",
            "--use-mock-keychain",
            "--password-store=basic",
            "--window-size=1280,800",
            "about:blank",
        ];
        assert_eq!(args, expected.map(OsString::from));
    }

    #[test]
    fn the_active_port_file_is_checked() {
        let good = "9222\n/devtools/browser/ab-12\n";
        assert_eq!(
            active_port(good),
            Some((9222, "/devtools/browser/ab-12".into()))
        );
        for bad in [
            "",
            "9222\n",
            "0\n/devtools/browser/a",
            "x\n/devtools/browser/a",
            "70000\n/devtools/browser/a",
            "9222\n/devtools/page/a",
            "9222\n/devtools/browser/",
            "9222\n/devtools/browser/a/../b",
            "9222\n/devtools/browser/a?b",
        ] {
            assert_eq!(active_port(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn frames_are_only_base64() {
        assert!(base64("/9j/4AAQ+x=="));
        assert!(!base64(""));
        assert!(!base64("ab\"c"));
        assert!(!base64("ab c"));
        assert!(base64(&"A".repeat(MAX_FRAME)));
        assert!(!base64(&"A".repeat(MAX_FRAME + 1)));
    }

    fn page(method: &str, id: &str, url: &str) -> Value {
        json!({ "method": method, "params": { "targetInfo": { "targetId": id, "type": "page", "url": url } } })
    }

    fn methods(out: &Out) -> Vec<String> {
        out.cdp
            .iter()
            .map(|m| m["method"].as_str().unwrap().to_owned())
            .collect()
    }

    fn frame(session: &str, n: u64, data: &str) -> Value {
        json!({ "method": "Page.screencastFrame", "sessionId": session, "params": { "data": data, "sessionId": n, "metadata": {} } })
    }

    /// A screen watching page `p1`, attached as session `s1`.
    fn attached() -> Screen {
        let mut screen = Screen::default();
        assert_eq!(methods(&screen.start()), ["Target.setDiscoverTargets"]);
        let out = screen.message(&page("Target.targetCreated", "p1", "about:blank"));
        assert_eq!(out.cdp, Vec::<Value>::new());
        assert_eq!(out.events, [Event::Url(Some("about:blank".into()))]);
        let out = screen.command(Command::Watch(true));
        assert_eq!(
            out.cdp,
            [
                json!({ "id": 2, "method": "Target.attachToTarget", "params": { "targetId": "p1", "flatten": true } })
            ]
        );
        let out = screen.message(&json!({ "id": 2, "result": { "sessionId": "s1" } }));
        let start = json!({
            "id": 3, "method": "Page.startScreencast", "sessionId": "s1",
            "params": { "format": "jpeg", "quality": 60, "maxWidth": 1280, "maxHeight": 1280, "everyNthFrame": 1 },
        });
        assert_eq!(out.cdp, [start]);
        screen
    }

    #[test]
    fn one_frame_at_a_time_reaches_the_app_and_is_acked_once_drawn() {
        let mut screen = attached();
        let out = screen.message(&frame("s1", 7, "AAAA"));
        assert_eq!(
            (out.cdp.len(), out.events),
            (0, vec![Event::Frame("AAAA".into())])
        );
        // Two more while the app draws: the newest waits, the one it replaced is acked.
        assert_eq!(screen.message(&frame("s1", 8, "BBBB")), Out::default());
        let out = screen.message(&frame("s1", 9, "CCCC"));
        let ack = |id: u64, n: u64| json!({ "id": id, "method": "Page.screencastFrameAck", "sessionId": "s1", "params": { "sessionId": n } });
        assert_eq!(out.cdp, [ack(4, 8)]);
        assert!(out.events.is_empty());
        let out = screen.command(Command::Drawn);
        assert_eq!(
            (out.cdp, out.events),
            (vec![ack(5, 7)], vec![Event::Frame("CCCC".into())])
        );
        let out = screen.command(Command::Drawn);
        assert_eq!((out.cdp, out.events), (vec![ack(6, 9)], vec![]));
        // Nothing pending: nothing to ack.
        assert_eq!(screen.command(Command::Drawn), Out::default());
        // Another session's frame, a bad one, one without its number: ignored.
        for ignored in [
            frame("s2", 10, "DDDD"),
            frame("s1", 10, "not base64!"),
            json!({ "method": "Page.screencastFrame", "sessionId": "s1", "params": { "data": "AA" } }),
            json!({ "method": "Page.screencastFrame", "sessionId": "s1", "params": { "sessionId": 3 } }),
        ] {
            assert_eq!(screen.message(&ignored), Out::default(), "{ignored}");
        }
    }

    #[test]
    fn the_page_last_opened_or_navigated_is_shown() {
        let mut screen = attached();
        let out = screen.message(&page("Target.targetCreated", "p2", "https://a.test/"));
        let switch = [
            "Page.stopScreencast",
            "Target.detachFromTarget",
            "Target.attachToTarget",
        ];
        assert_eq!(methods(&out), switch);
        assert_eq!(out.cdp[1]["params"], json!({ "sessionId": "s1" }));
        assert_eq!(out.cdp[2]["params"]["targetId"], "p2");
        assert_eq!(out.events, [Event::Url(Some("https://a.test/".into()))]);
        let attach = out.cdp[2]["id"].as_u64().unwrap();
        screen.message(&json!({ "id": attach, "result": { "sessionId": "s2" } }));
        // p1 navigates: shown again.
        let out = screen.message(&page("Target.targetInfoChanged", "p1", "https://b.test/"));
        assert_eq!(methods(&out), switch);
        assert_eq!(out.events, [Event::Url(Some("https://b.test/".into()))]);
        // The same URL again (a title change): nothing.
        let attach = out.cdp[2]["id"].as_u64().unwrap();
        let stale = attach;
        let out = screen.message(&page("Target.targetInfoChanged", "p1", "https://b.test/"));
        assert_eq!(out, Out::default());
        // Not a page: nothing.
        let worker = json!({ "method": "Target.targetCreated", "params": { "targetInfo": { "targetId": "w", "type": "service_worker", "url": "x" } } });
        assert_eq!(screen.message(&worker), Out::default());
        // p1 closes before its attach is answered: p2 is shown, the late answer detached.
        let gone = json!({ "method": "Target.targetDestroyed", "params": { "targetId": "p1" } });
        let out = screen.message(&gone);
        assert_eq!(methods(&out), ["Target.attachToTarget"]);
        assert_eq!(out.events, [Event::Url(Some("https://a.test/".into()))]);
        let out = screen.message(&json!({ "id": stale, "result": { "sessionId": "late" } }));
        assert_eq!(
            out.cdp,
            [
                json!({ "id": 12, "method": "Target.detachFromTarget", "params": { "sessionId": "late" } })
            ]
        );
        // The last page closes: no page, nothing attached.
        let attach = 11;
        screen.message(&json!({ "id": attach, "result": { "sessionId": "s3" } }));
        let gone = json!({ "method": "Target.targetDestroyed", "params": { "targetId": "p2" } });
        let out = screen.message(&gone);
        assert_eq!(out.cdp, Vec::<Value>::new());
        assert_eq!(out.events, [Event::Url(None)]);
    }

    #[test]
    fn unwatched_nothing_is_attached_and_a_lost_session_is_attached_again() {
        let mut screen = attached();
        screen.message(&frame("s1", 1, "AAAA"));
        let out = screen.command(Command::Watch(false));
        assert_eq!(
            methods(&out),
            ["Page.stopScreencast", "Target.detachFromTarget"]
        );
        assert_eq!(screen.command(Command::Drawn), Out::default());
        // Watched again: attached again.
        let out = screen.command(Command::Watch(true));
        assert_eq!(methods(&out), ["Target.attachToTarget"]);
        let id = out.cdp[0]["id"].as_u64().unwrap();
        // Refused: not tried again by itself.
        assert_eq!(
            screen.message(&json!({ "id": id, "error": { "message": "no" } })),
            Out::default()
        );
        // Watching again restarts it.
        let out = screen.command(Command::Watch(true));
        let id = out.cdp[0]["id"].as_u64().unwrap();
        screen.message(&json!({ "id": id, "result": { "sessionId": "s9" } }));
        // Another session's detach: nothing; ours: attached again.
        let other =
            json!({ "method": "Target.detachedFromTarget", "params": { "sessionId": "zz" } });
        assert_eq!(screen.message(&other), Out::default());
        let ours =
            json!({ "method": "Target.detachedFromTarget", "params": { "sessionId": "s9" } });
        assert_eq!(methods(&screen.message(&ours)), ["Target.attachToTarget"]);
        // Unknown methods and answers: nothing.
        assert_eq!(
            screen.message(&json!({ "method": "Page.loadEventFired" })),
            Out::default()
        );
        assert_eq!(
            screen.message(&json!({ "id": 1, "result": {} })),
            Out::default()
        );
    }

    #[test]
    fn urls_are_cut_and_pages_bounded() {
        let mut screen = Screen::default();
        let long = format!("https://a.test/{}", "x".repeat(3000));
        let out = screen.message(&page("Target.targetCreated", "p", &long));
        let Event::Url(Some(url)) = &out.events[0] else {
            panic!()
        };
        assert_eq!(url.chars().count(), MAX_URL);
        for i in 0..=MAX_PAGES {
            screen.message(&page(
                "Target.targetCreated",
                &format!("t{i}"),
                "about:blank",
            ));
        }
        assert_eq!(screen.pages.len(), MAX_PAGES);
        assert_eq!(screen.pages[0].0, "t1");
    }

    #[test]
    fn the_mcp_runs_with_bunx_else_npx() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (tmp.path().join("a"), tmp.path().join("b"));
        let path = std::env::join_paths([Path::new("rel"), &a, &b]).unwrap();
        let end = "http://127.0.0.1:1";
        assert_eq!(mcp_command(Some(&path), end), None);
        assert_eq!(mcp_command(None, end), None);
        exe(&b.join("npx"), "");
        let npx = (
            b.join("npx"),
            ["-y", "@playwright/mcp@latest", "--cdp-endpoint", end]
                .map(String::from)
                .to_vec(),
        );
        assert_eq!(mcp_command(Some(&path), end), Some(npx));
        exe(&b.join("bunx"), "");
        let bunx = (
            b.join("bunx"),
            ["@playwright/mcp@latest", "--cdp-endpoint", end]
                .map(String::from)
                .to_vec(),
        );
        assert_eq!(mcp_command(Some(&path), end), Some(bunx));
    }

    #[tokio::test]
    async fn the_screencast_runs_over_a_websocket_until_it_closes() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            let next = |m: Option<Result<Message, _>>| -> Value {
                serde_json::from_str(m.unwrap().unwrap().to_text().unwrap()).unwrap()
            };
            assert_eq!(next(ws.next().await)["method"], "Target.setDiscoverTargets");
            let created = page("Target.targetCreated", "p", "about:blank").to_string();
            for text in ["not json".to_owned(), created] {
                ws.send(Message::text(text)).await.unwrap();
            }
            ws.send(Message::binary(vec![1u8])).await.unwrap();
            assert_eq!(next(ws.next().await)["method"], "Target.attachToTarget");
            ws.send(Message::text(
                json!({ "id": 2, "result": { "sessionId": "s" } }).to_string(),
            ))
            .await
            .unwrap();
            assert_eq!(next(ws.next().await)["method"], "Page.startScreencast");
            ws.send(Message::text(frame("s", 1, "AAAA").to_string()))
                .await
                .unwrap();
            assert_eq!(next(ws.next().await)["method"], "Page.screencastFrameAck");
        });
        let (commands, rx) = mpsc::unbounded_channel();
        let url = format!("ws://{addr}/devtools/browser/x");
        let (ws, _) = tokio_tungstenite::connect_async(url).await.unwrap();
        let (events_tx, mut events) = mpsc::unbounded_channel();
        let cast = tokio::spawn(screencast(ws, rx, move |e| {
            let _ = events_tx.send(e);
        }));
        assert_eq!(
            events.recv().await,
            Some(Event::Url(Some("about:blank".into())))
        );
        commands.send(Command::Watch(true)).unwrap();
        assert_eq!(events.recv().await, Some(Event::Frame("AAAA".into())));
        commands.send(Command::Drawn).unwrap();
        server.await.unwrap();
        // The browser closed the connection: the screencast ends.
        cast.await.unwrap();
        // And without commands (the service dropped it).
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            while ws.next().await.is_some_and(|m| m.is_ok()) {}
        });
        let (ws, _) = tokio_tungstenite::connect_async(format!("ws://{addr}/"))
            .await
            .unwrap();
        let (commands, rx) = mpsc::unbounded_channel::<Command>();
        drop(commands);
        screencast(ws, rx, |_| {}).await;
        server.await.unwrap();
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_browser_that_fails_to_start_is_reported_and_its_profile_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let profile = tmp.path().join("browsers/1");
        // Exits at once.
        let quits = tmp.path().join("quits");
        exe(&quits, "#!/bin/sh\nexit 3\n");
        let err = start(&quits, profile.clone()).await.err().unwrap();
        assert!(err.starts_with("The browser exited as it started"), "{err}");
        assert!(!profile.exists());
        let err = start(&tmp.path().join("missing"), profile.clone())
            .await
            .err()
            .unwrap();
        assert!(err.starts_with("Cannot start "), "{err}");
        assert!(!profile.exists());
        // Writes a port nobody listens on.
        let port = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            l.local_addr().unwrap().port()
        };
        let fake = tmp.path().join("fake");
        let script = format!(
            "#!/bin/sh\nfor a; do case $a in --user-data-dir=*) d=${{a#--user-data-dir=}};; esac; done\nprintf '{port}\\n/devtools/browser/x\\n' > \"$d/DevToolsActivePort\"\nexec sleep 30\n"
        );
        exe(&fake, &script);
        let err = start(&fake, profile.clone()).await.err().unwrap();
        assert!(err.starts_with("Cannot connect to the browser"), "{err}");
        assert!(!profile.exists());
        // The profile cannot be made.
        std::fs::write(tmp.path().join("file"), "").unwrap();
        let err = start(&fake, tmp.path().join("file/p")).await.err().unwrap();
        assert!(
            err.starts_with("Cannot make the browser's profile"),
            "{err}"
        );
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn a_browser_ignoring_sigterm_is_killed() {
        let tmp = tempfile::tempdir().unwrap();
        let profile = tmp.path().join("p");
        std::fs::create_dir(&profile).unwrap();
        let child = tokio::process::Command::new("sh")
            .args(["-c", "trap '' TERM; while :; do sleep 0.05; done"])
            .process_group(0)
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let pid = child.id().unwrap();
        // The trap is in place before the signal.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let started = std::time::Instant::now();
        Process {
            child,
            profile: profile.clone(),
        }
        .stop()
        .await;
        assert!(started.elapsed() >= STOP_GRACE);
        assert!(!profile.exists());
        let alive = nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None);
        assert!(alive.is_err(), "{pid} still runs");
    }
}
