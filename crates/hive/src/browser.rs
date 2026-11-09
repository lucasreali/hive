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

/// The programs looked for on `PATH` on Linux (and in WSL), in this order.
#[cfg(not(target_os = "macos"))]
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
#[cfg(unix)]
const STOP_GRACE: Duration = Duration::from_secs(2);
/// The largest CDP message read; a bigger one ends the connection.
const MAX_MESSAGE: usize = 4 << 20;
/// The largest frame forwarded (base64): its control message stays under `MAX_PAYLOAD`.
pub const MAX_FRAME: usize = 3 << 20;
/// The longest page URL shown, in characters.
const MAX_URL: usize = 2048;
/// The longest page title shown, in characters.
const MAX_TITLE: usize = 256;
/// The longest text of a tool's input quoted in a caption, in characters.
pub(crate) const MAX_QUOTE: usize = 120;
/// The longest caption of an action, in characters.
const MAX_CAPTION: usize = 300;
/// What the names of the `hive-browser` MCP server's tools start with, in hook payloads.
const TOOL_PREFIX: &str = "mcp__hive-browser__";
/// The most pages followed; older ones are forgotten.
const MAX_PAGES: usize = 100;
/// The most `DevToolsActivePort` may hold.
const MAX_PORT_FILE: u64 = 1024;
/// How long `hive browser-mcp` waits for the service to start the browser.
const ENDPOINT_TIMEOUT: Duration = Duration::from_secs(30);

/// The `hive-browser` MCP server's config, in the bin folder while the agent's browser is on:
/// the `claude` wrapper passes it with `--mcp-config` while it is there.
pub const MCP_CONFIG: &str = "hive-browser.json";

/// Writes [`MCP_CONFIG`] into `bin` when `enabled` (its server runs `hive browser-mcp`), else
/// removes it.
pub fn configure(bin: &Path, hive: &Path, enabled: bool) -> io::Result<()> {
    let file = bin.join(MCP_CONFIG);
    if !enabled {
        return match std::fs::remove_file(&file) {
            Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
            _ => Ok(()),
        };
    }
    let hive = hive
        .to_str()
        .ok_or_else(|| io::Error::other(format!("hive path is not UTF-8: {}", hive.display())))?;
    let server = json!({ "type": "stdio", "command": hive, "args": ["browser-mcp"] });
    let config = json!({ "mcpServers": { "hive-browser": server } });
    crate::wrapper::write_atomic(&file, format!("{config:#}\n").as_bytes(), 0o644)
}

/// In a browser's profile: its process group, as decimal digits.
const GROUP_FILE: &str = "hive-browser-group";

/// Ends the browsers an earlier service left running (killed before it could stop them: their
/// DevTools port has no password) and removes `dir`, their profiles' folder. A profile's
/// [`GROUP_FILE`] names the group killed, and only while its leader still runs with that
/// profile: a number the system gave another process since is left alone. `kill` ends a group
/// ([`kill_group`]).
#[cfg(unix)]
pub fn reap(dir: &Path, source: crate::procs::Source, kill: &mut dyn FnMut(i32)) {
    for profile in std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
    {
        let mut text = String::new();
        let read = std::fs::File::open(profile.join(GROUP_FILE))
            .and_then(|f| f.take(32).read_to_string(&mut text));
        let group = read.ok().and_then(|_| text.trim().parse::<i32>().ok());
        let mut flag = OsString::from("--user-data-dir=");
        flag.push(&profile);
        let flag = flag.to_string_lossy().into_owned();
        // Never 0 (the service's own group), 1 (`killpg(1)` is `kill(-1)`: every process of
        // the user) or a negative number, whatever their arguments.
        let leader = group
            .filter(|&g| g > 1)
            .filter(|&g| crate::procs::args(source, g).is_some_and(|args| args.contains(&flag)));
        leader.into_iter().for_each(&mut *kill);
    }
    let _ = std::fs::remove_dir_all(dir);
}

/// Ends the process group `group` at once (SIGKILL).
#[cfg(unix)]
pub fn kill_group(group: i32) {
    use nix::sys::signal::{Signal, killpg};
    let _ = killpg(nix::unistd::Pid::from_raw(group), Signal::SIGKILL);
}

/// The browser to run: `setting` (`browser.path`) when set, else the first installed one of
/// `candidates`. A snap Chromium (under `snap`) is skipped: its confinement cannot use Hive's
/// temporary profile.
pub fn find(
    setting: Option<&str>,
    candidates: Vec<PathBuf>,
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

#[cfg(target_os = "macos")]
pub use crate::macos::browser_candidates as candidates;

/// Where browsers install elsewhere: each program of [`PROGRAMS`] in each absolute folder of
/// `path`, program by program. Native Windows finds none (no `.exe` is looked for, D17.0): the
/// service in WSL finds one there.
#[cfg(not(target_os = "macos"))]
pub fn candidates(_home: Option<&Path>, path: Option<&OsStr>) -> Vec<PathBuf> {
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

/// Starts `program` with a new temporary `profile` and connects to its DevTools, waiting at most
/// `wait` for its port: the process, the CDP connection and the port. Anything failing on the
/// way stops the process.
pub async fn start(
    program: &Path,
    profile: PathBuf,
    wait: Duration,
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
    // Its process group (its own: `process_group(0)`), for a later service to end it should this
    // one be killed before it could ([`reap`]).
    let group = child.id().unwrap_or(0).to_string();
    let _ = std::fs::write(profile.join(GROUP_FILE), group);
    let mut process = Process { child, profile };
    match connect(&mut process, wait).await {
        Ok((ws, port)) => Ok((process, ws, port)),
        Err(err) => {
            process.stop().await;
            Err(err)
        }
    }
}

/// Waits for the browser's `DevToolsActivePort`, then opens its CDP connection, all within
/// `wait`: a port that takes the connection and never answers counts as no port.
async fn connect(
    process: &mut Process,
    wait: Duration,
) -> Result<(WebSocketStream<TcpStream>, u16), String> {
    let file = process.profile.join("DevToolsActivePort");
    let connecting = async {
        let (port, path) = loop {
            if let Ok(Some(status)) = process.child.try_wait() {
                return Err(format!("The browser exited as it started ({status})"));
            }
            let mut text = String::new();
            let read = std::fs::File::open(&file)
                .and_then(|f| f.take(MAX_PORT_FILE).read_to_string(&mut text));
            if let Some(found) = read.ok().and_then(|_| active_port(&text)) {
                break found;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        };
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
    };
    tokio::time::timeout(wait, connecting)
        .await
        .map_err(|_| "The browser did not open its DevTools port in time".to_owned())?
}

impl Process {
    /// A process for a test of what happens around it.
    #[cfg(test)]
    pub(crate) fn of(child: tokio::process::Child, profile: PathBuf) -> Self {
        Self { child, profile }
    }

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
    /// The browser is about to stop: one picture of the page shown, so its result can still
    /// be seen (25.2), as [`Event::Last`].
    Capture,
}

/// What a browser's screencast tells the service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    /// The page shown is now at this URL (`None`: no page).
    Url(Option<String>),
    /// The page shown now has this title (`None`: none, or no page).
    Title(Option<String>),
    /// A frame for the app: a JPEG in base64.
    Frame(String),
    /// [`Command::Capture`]'s answer: a JPEG in base64, `None` when there is no picture.
    Last(Option<String>),
}

/// The screencast of one browser, as a state machine over CDP messages: it follows the pages
/// (the one last opened or navigated is shown), attaches to the shown one while watched, and
/// lets one frame at a time reach the app, acknowledging each to the browser once drawn (the
/// browser sends no more than two unacknowledged; the newest waits here).
#[derive(Debug, Default)]
pub struct Screen {
    /// The last CDP command id used.
    id: u64,
    /// The pages; the last one is shown.
    pages: Vec<Page>,
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
    /// The title last reported.
    title: Option<String>,
    /// The id of the `Target.attachToTarget` awaited for [`Command::Capture`].
    capture_attaching: Option<u64>,
    /// The id of the `Page.captureScreenshot` awaited for [`Command::Capture`].
    capturing: Option<u64>,
}

/// A page the browser reported.
#[derive(Debug)]
struct Page {
    target: String,
    url: String,
    /// Empty: none.
    title: String,
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
            Command::Capture => match self.pages.last().map(|p| p.target.clone()) {
                Some(target) => {
                    let params = json!({ "targetId": target, "flatten": true });
                    let id = self.send(&mut out, None, "Target.attachToTarget", params);
                    self.capture_attaching = Some(id);
                }
                None => out.events.push(Event::Last(None)),
            },
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
                    self.page(id, url, info["title"].as_str().unwrap_or_default());
                }
            }
            Some("Target.targetDestroyed") => {
                let id = params["targetId"].as_str();
                self.pages.retain(|page| Some(page.target.as_str()) != id);
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
    fn page(&mut self, id: &str, url: &str, title: &str) {
        let url = crate::adapter::clip(url, MAX_URL);
        let title = crate::adapter::clip(title, MAX_TITLE);
        let same = |p: &&mut Page| p.target == id && p.url == url;
        if let Some(page) = self.pages.iter_mut().find(same) {
            page.title = title;
            return;
        }
        self.pages.retain(|p| p.target != id);
        self.pages.push(Page {
            target: id.to_owned(),
            url,
            title,
        });
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

    /// An answer to one of our commands: only `Target.attachToTarget`'s and
    /// `Page.captureScreenshot`'s matter.
    fn answer(&mut self, message: &Value, out: &mut Out) {
        let session = message["result"]["sessionId"].as_str();
        let id = message["id"].as_u64();
        if self.capture_attaching.is_some() && id == self.capture_attaching {
            self.capture_attaching = None;
            let Some(session) = session else {
                return out.events.push(Event::Last(None));
            };
            let params = json!({ "format": "jpeg", "quality": QUALITY });
            let shot = self.send(out, Some(session), "Page.captureScreenshot", params);
            self.capturing = Some(shot);
        } else if self.capturing.is_some() && id == self.capturing {
            self.capturing = None;
            let data = message["result"]["data"].as_str().filter(|d| base64(d));
            out.events.push(Event::Last(data.map(str::to_owned)));
        } else if self.attaching.is_some() && message["id"].as_u64() == self.attaching {
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
        let shown = self.pages.last().map(|page| page.target.clone());
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
        let url = self.pages.last().map(|page| page.url.clone());
        if url != self.url {
            self.url.clone_from(&url);
            out.events.push(Event::Url(url));
        }
        let title = self.pages.last().map(|page| page.title.clone());
        let title = title.filter(|title| !title.is_empty());
        if title != self.title {
            self.title.clone_from(&title);
            out.events.push(Event::Title(title));
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
pub async fn screencast(
    mut ws: WebSocketStream<TcpStream>,
    mut commands: mpsc::UnboundedReceiver<Command>,
    events: mpsc::UnboundedSender<Event>,
) {
    let mut screen = Screen::default();
    let mut out = screen.start();
    loop {
        for message in out.cdp.drain(..) {
            // A connection that fails to write ends at the next read.
            let _ = ws.send(Message::text(message.to_string())).await;
        }
        for event in out.events.drain(..) {
            let _ = events.send(event);
        }
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

/// Whether `tool`, as hook payloads name it, is one of the agent's browser's.
pub fn is_tool(tool: &str) -> bool {
    tool.starts_with(TOOL_PREFIX)
}

/// What an agent's call of a browser tool does (25.2), for the app's caption: plain text from its
/// untrusted `input`, each part and the whole size-capped; `None` for any other tool. A form's
/// values are left out: they are the likeliest to be secret.
pub fn caption(tool: &str, input: &Value) -> Option<String> {
    let name = tool.strip_prefix(TOOL_PREFIX)?;
    let flag = |field: &str| input.get(field).and_then(Value::as_bool) == Some(true);
    let text = |field: &str| {
        let value = input.get(field)?.as_str()?;
        Some(crate::adapter::clip(value, MAX_QUOTE)).filter(|v| !v.is_empty())
    };
    let quoted = |field: &str, none: &str| {
        text(field).map_or_else(|| none.to_owned(), |value| format!("'{value}'"))
    };
    let on = |field: &str| quoted(field, "an element");
    let count = |field: &str| {
        input
            .get(field)
            .and_then(Value::as_array)
            .map_or(0, Vec::len)
    };
    let many = |n: usize, what: &str| format!("{n} {what}{}", if n == 1 { "" } else { "s" });
    let caption = match name {
        "browser_navigate" => match text("url") {
            Some(url) => format!("opened {url}"),
            None => "opened a page".to_owned(),
        },
        "browser_navigate_back" => "went back".to_owned(),
        "browser_click" if flag("doubleClick") => format!("double-clicked {}", on("element")),
        "browser_click" => format!("clicked {}", on("element")),
        "browser_type" if flag("submit") => {
            format!("typed {} and pressed Enter", quoted("text", "some text"))
        }
        "browser_type" => format!("typed {}", quoted("text", "some text")),
        "browser_fill_form" => format!("filled {}", many(count("fields"), "field")),
        "browser_press_key" => format!("pressed {}", quoted("key", "a key")),
        "browser_hover" => format!("hovered over {}", on("element")),
        "browser_select_option" => format!("chose an option of {}", on("element")),
        "browser_drag" => format!("dragged {} to {}", on("startElement"), on("endElement")),
        "browser_drop" => format!("dropped onto {}", on("element")),
        "browser_file_upload" => match count("paths") {
            0 => "cancelled a file upload".to_owned(),
            n => format!("uploaded {}", many(n, "file")),
        },
        "browser_take_screenshot" => "took a screenshot".to_owned(),
        "browser_snapshot" => "read the page".to_owned(),
        "browser_find" => format!("searched the page for {}", quoted("text", "a pattern")),
        "browser_evaluate" | "browser_run_code" | "browser_run_code_unsafe" => {
            "ran a script on the page".to_owned()
        }
        "browser_wait_for" => match (text("text"), text("textGone")) {
            (Some(shown), _) => format!("waited for '{shown}'"),
            (None, Some(gone)) => format!("waited for '{gone}' to go"),
            (None, None) => "waited".to_owned(),
        },
        "browser_tabs" => match input.get("action").and_then(Value::as_str) {
            Some("new") => "opened a new tab".to_owned(),
            Some("close") => "closed a tab".to_owned(),
            Some("select") => "switched tabs".to_owned(),
            _ => "listed the tabs".to_owned(),
        },
        "browser_close" => "closed the page".to_owned(),
        "browser_resize" => "resized the window".to_owned(),
        "browser_handle_dialog" if flag("accept") => "accepted a dialog".to_owned(),
        "browser_handle_dialog" => "dismissed a dialog".to_owned(),
        "browser_console_messages" => "read the console".to_owned(),
        "browser_network_request" | "browser_network_requests" => {
            "read the network requests".to_owned()
        }
        "browser_emulate_media" => "changed how the page is shown".to_owned(),
        other => format!("used {other}"),
    };
    Some(crate::adapter::clip(&caption, MAX_CAPTION))
}

/// `hive browser-mcp`'s question to the service: the CDP endpoint of the browser of the
/// worktree holding `cwd`, started first if needed, within `wait`.
pub async fn endpoint(paths: &Paths, cwd: &str, wait: Duration) -> Result<String, String> {
    let asked = async {
        let mut framed = Framed::new(paths.connect().await?, FrameCodec);
        let hello = Control::hello(Role::Hook, VERSION);
        let ask = Control::BrowserEndpoint {
            cwd: cwd.to_owned(),
        };
        for message in [hello, ask] {
            let frame = Frame::control(0, &message);
            framed.send(frame).await.map_err(io::Error::other)?;
        }
        while let Some(frame) = framed.next().await {
            match frame.map_err(io::Error::other)?.to_control() {
                Ok(Control::BrowserReady { endpoint }) => return Ok(Ok(endpoint)),
                Ok(Control::Error { message }) => return Ok(Err(message)),
                Ok(Control::VersionMismatch { version, .. }) => {
                    let why = format!("the Hive service is version {version}, not {VERSION}");
                    return Ok(Err(why));
                }
                _ => {}
            }
        }
        Ok::<_, io::Error>(Err("the Hive service closed the connection".to_owned()))
    };
    match tokio::time::timeout(wait, asked).await {
        Ok(Ok(answer)) => answer,
        Ok(Err(err)) => Err(format!("cannot reach the Hive service: {err}")),
        Err(_) => Err("the Hive service did not start the browser in time".to_owned()),
    }
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
/// stdio the agent's; its exit code.
pub async fn mcp(paths: &Paths) -> io::Result<u8> {
    let cwd = std::env::current_dir()?;
    let endpoint = endpoint(paths, &cwd.to_string_lossy(), ENDPOINT_TIMEOUT)
        .await
        .map_err(io::Error::other)?;
    let path = std::env::var_os("PATH");
    let (program, args) = mcp_command(path.as_deref(), &endpoint)
        .ok_or_else(|| io::Error::other("neither bunx nor npx is on PATH"))?;
    // shortcut: a child, not `exec` (on Unix too, so a test's run is measured); one ended by a
    // signal leaves the MCP to end on its closed stdin.
    let mut command = tokio::process::Command::new(&program);
    let status = command.args(&args).status().await?;
    Ok(crate::wrapper::exit_byte(status.code().unwrap_or(1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The caption of a call of browser tool `name`.
    fn said(name: &str, input: Value) -> String {
        caption(&format!("mcp__hive-browser__{name}"), &input).unwrap()
    }

    #[test]
    fn only_the_browsers_tools_are_its_own() {
        assert!(is_tool("mcp__hive-browser__browser_click"));
        assert!(!is_tool("Read"));
        assert!(!is_tool("mcp__other__browser_click"));
        assert_eq!(caption("Read", &json!({"element": "x"})), None);
    }

    #[test]
    fn each_tool_call_has_its_caption_from_its_input() {
        let cases = [
            (
                "browser_navigate",
                json!({"url": "https://a.test/"}),
                "opened https://a.test/",
            ),
            ("browser_navigate", json!({}), "opened a page"),
            ("browser_navigate", json!({"url": 3}), "opened a page"),
            ("browser_navigate", json!({"url": "  "}), "opened a page"),
            ("browser_navigate_back", json!({}), "went back"),
            (
                "browser_click",
                json!({"element": "Learn more", "target": "e1"}),
                "clicked 'Learn more'",
            ),
            (
                "browser_click",
                json!({"target": "e1"}),
                "clicked an element",
            ),
            (
                "browser_click",
                json!({"element": "Row", "doubleClick": true}),
                "double-clicked 'Row'",
            ),
            (
                "browser_click",
                json!({"element": "Row", "doubleClick": false}),
                "clicked 'Row'",
            ),
            (
                "browser_type",
                json!({"text": "Honey bee", "submit": true}),
                "typed 'Honey bee' and pressed Enter",
            ),
            (
                "browser_type",
                json!({"text": "Honey bee", "submit": false}),
                "typed 'Honey bee'",
            ),
            ("browser_type", json!({}), "typed some text"),
            (
                "browser_type",
                json!({"submit": true}),
                "typed some text and pressed Enter",
            ),
            (
                "browser_fill_form",
                json!({"fields": [{"value": "hunter2"}]}),
                "filled 1 field",
            ),
            (
                "browser_fill_form",
                json!({"fields": [{}, {}]}),
                "filled 2 fields",
            ),
            ("browser_fill_form", json!({}), "filled 0 fields"),
            (
                "browser_press_key",
                json!({"key": "Enter"}),
                "pressed 'Enter'",
            ),
            ("browser_press_key", json!({}), "pressed a key"),
            (
                "browser_hover",
                json!({"element": "Menu"}),
                "hovered over 'Menu'",
            ),
            (
                "browser_select_option",
                json!({"element": "Size"}),
                "chose an option of 'Size'",
            ),
            (
                "browser_drag",
                json!({"startElement": "A", "endElement": "B"}),
                "dragged 'A' to 'B'",
            ),
            (
                "browser_drag",
                json!({}),
                "dragged an element to an element",
            ),
            (
                "browser_drop",
                json!({"element": "Box"}),
                "dropped onto 'Box'",
            ),
            ("browser_file_upload", json!({}), "cancelled a file upload"),
            (
                "browser_file_upload",
                json!({"paths": ["/a"]}),
                "uploaded 1 file",
            ),
            (
                "browser_file_upload",
                json!({"paths": ["/a", "/b"]}),
                "uploaded 2 files",
            ),
            ("browser_take_screenshot", json!({}), "took a screenshot"),
            ("browser_snapshot", json!({}), "read the page"),
            (
                "browser_find",
                json!({"text": "Price"}),
                "searched the page for 'Price'",
            ),
            (
                "browser_find",
                json!({"regex": "x"}),
                "searched the page for a pattern",
            ),
            (
                "browser_evaluate",
                json!({"function": "() => 1"}),
                "ran a script on the page",
            ),
            ("browser_run_code", json!({}), "ran a script on the page"),
            (
                "browser_run_code_unsafe",
                json!({}),
                "ran a script on the page",
            ),
            (
                "browser_wait_for",
                json!({"text": "Done"}),
                "waited for 'Done'",
            ),
            (
                "browser_wait_for",
                json!({"textGone": "Busy"}),
                "waited for 'Busy' to go",
            ),
            ("browser_wait_for", json!({"time": 2}), "waited"),
            ("browser_tabs", json!({"action": "new"}), "opened a new tab"),
            ("browser_tabs", json!({"action": "close"}), "closed a tab"),
            ("browser_tabs", json!({"action": "select"}), "switched tabs"),
            ("browser_tabs", json!({"action": "list"}), "listed the tabs"),
            ("browser_close", json!({}), "closed the page"),
            (
                "browser_resize",
                json!({"width": 1, "height": 2}),
                "resized the window",
            ),
            (
                "browser_handle_dialog",
                json!({"accept": true}),
                "accepted a dialog",
            ),
            (
                "browser_handle_dialog",
                json!({"accept": false}),
                "dismissed a dialog",
            ),
            ("browser_console_messages", json!({}), "read the console"),
            (
                "browser_network_request",
                json!({"index": 1}),
                "read the network requests",
            ),
            (
                "browser_network_requests",
                json!({}),
                "read the network requests",
            ),
            (
                "browser_emulate_media",
                json!({}),
                "changed how the page is shown",
            ),
            ("browser_new_thing", json!({}), "used browser_new_thing"),
        ];
        for (name, input, expected) in cases {
            assert_eq!(said(name, input.clone()), expected, "{name} {input}");
        }
    }

    #[test]
    fn a_captions_untrusted_text_is_plain_and_capped() {
        // Control and invisible characters go.
        assert_eq!(
            said("browser_click", json!({"element": "a\u{202E}b\nc\u{7}"})),
            "clicked 'abc'"
        );
        // Each quote is capped, then the whole.
        let long = "x".repeat(100_000);
        let one = said("browser_click", json!({"element": long}));
        assert_eq!(one.chars().count(), "clicked ''".len() + MAX_QUOTE);
        assert!(one.ends_with("…'"));
        let tool = format!("mcp__hive-browser__{long}");
        let whole = caption(&tool, &json!({})).unwrap();
        assert_eq!(whole.chars().count(), MAX_CAPTION);
    }

    /// A page message of the browser.
    fn titled(method: &str, id: &str, url: &str, title: Option<&str>) -> Value {
        let mut info = json!({ "targetId": id, "type": "page", "url": url });
        if let Some(title) = title {
            info["title"] = json!(title);
        }
        json!({ "method": method, "params": { "targetInfo": info } })
    }

    #[test]
    fn the_shown_pages_title_is_reported_and_a_title_alone_moves_no_page() {
        let mut screen = Screen::default();
        let created = titled("Target.targetCreated", "a", "https://a.test/", None);
        let out = screen.message(&created);
        assert_eq!(out.events, [Event::Url(Some("https://a.test/".into()))]);
        let changed = "Target.targetInfoChanged";
        let out = screen.message(&titled(
            changed,
            "a",
            "https://a.test/",
            Some(" Shop\u{7} "),
        ));
        assert_eq!(out.events, [Event::Title(Some("Shop".into()))]);
        // The same again: nothing new.
        let out = screen.message(&titled(changed, "a", "https://a.test/", Some("Shop")));
        assert_eq!(out.events, []);
        // Another page shows, with its title.
        let out = screen.message(&titled(changed, "b", "https://b.test/", Some("Other")));
        assert_eq!(
            out.events,
            [
                Event::Url(Some("https://b.test/".into())),
                Event::Title(Some("Other".into()))
            ]
        );
        // The first page's title changing does not bring it back to the front.
        let out = screen.message(&titled(changed, "a", "https://a.test/", Some("Shop 2")));
        assert_eq!(out.events, []);
        assert_eq!(screen.pages[0].title, "Shop 2");
        // Blank fillers are no title, so the tab falls back to the address.
        let blank = titled(changed, "b", "https://b.test/", Some("\u{3164}\u{3164}"));
        assert_eq!(screen.message(&blank).events, [Event::Title(None)]);
        screen.message(&titled(changed, "b", "https://b.test/", Some("Other")));
        // An empty title is none; a long one is capped.
        let out = screen.message(&titled(changed, "b", "https://b.test/", Some("")));
        assert_eq!(out.events, [Event::Title(None)]);
        let long = "t".repeat(1000);
        let out = screen.message(&titled(changed, "b", "https://b.test/", Some(&long)));
        let capped = "t".repeat(MAX_TITLE - 1) + "…";
        assert_eq!(out.events, [Event::Title(Some(capped))]);
    }

    #[test]
    fn a_stopping_browser_gives_one_picture_of_the_shown_page() {
        let mut screen = Screen::default();
        // No page: nothing to take.
        let out = screen.command(Command::Capture);
        assert_eq!((out.cdp, out.events), (vec![], vec![Event::Last(None)]));
        screen.message(&titled(
            "Target.targetCreated",
            "p",
            "https://a.test/",
            None,
        ));
        // A message with no id, before any picture is asked for, is no answer.
        assert_eq!(screen.message(&json!({})).events, []);
        // Of two pages, the one shown: the last opened.
        screen.message(&titled(
            "Target.targetCreated",
            "q",
            "https://b.test/",
            None,
        ));
        let attach = screen.command(Command::Capture).cdp.remove(0);
        assert_eq!(attach["method"], "Target.attachToTarget");
        // Another answer while the attach is awaited is not it, and it is still awaited.
        let other = json!({ "id": 9999, "error": { "message": "no" } });
        let out = screen.message(&other);
        assert_eq!((out.cdp, out.events), (vec![], vec![]));
        assert_eq!(
            attach["params"],
            json!({ "targetId": "q", "flatten": true })
        );
        let attached = json!({ "id": attach["id"], "result": { "sessionId": "s" } });
        let shot = screen.message(&attached).cdp.remove(0);
        assert_eq!(shot["method"], "Page.captureScreenshot");
        assert_eq!(shot["sessionId"], "s");
        assert_eq!(
            shot["params"],
            json!({ "format": "jpeg", "quality": QUALITY })
        );
        // Another answer is not it.
        let other = json!({ "id": 9999, "result": { "data": "BBBB" } });
        assert_eq!(screen.message(&other).events, []);
        let answer = json!({ "id": shot["id"], "result": { "data": "AAAA" } });
        assert_eq!(
            screen.message(&answer).events,
            [Event::Last(Some("AAAA".into()))]
        );
        // Answered once: the same again is no picture.
        assert_eq!(screen.message(&answer).events, []);
    }

    #[test]
    fn a_picture_that_cannot_be_taken_is_said_so() {
        let mut screen = Screen::default();
        screen.message(&titled(
            "Target.targetCreated",
            "p",
            "https://a.test/",
            None,
        ));
        // The page refuses the attach.
        let attach = screen.command(Command::Capture).cdp.remove(0);
        let refused = json!({ "id": attach["id"], "error": { "message": "gone" } });
        let out = screen.message(&refused);
        assert_eq!((out.cdp, out.events), (vec![], vec![Event::Last(None)]));
        // A picture that is no base64 is none.
        let attach = screen.command(Command::Capture).cdp.remove(0);
        let attached = json!({ "id": attach["id"], "result": { "sessionId": "s" } });
        let shot = screen.message(&attached).cdp.remove(0);
        let bad = json!({ "id": shot["id"], "result": { "data": "<script>" } });
        assert_eq!(screen.message(&bad).events, [Event::Last(None)]);
    }

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
    fn the_mcp_config_is_written_while_on_and_removed_when_off() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join(MCP_CONFIG);
        configure(tmp.path(), Path::new("/opt/it's hive"), true).unwrap();
        let config: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        let server =
            json!({ "type": "stdio", "command": "/opt/it's hive", "args": ["browser-mcp"] });
        assert_eq!(config, json!({ "mcpServers": { "hive-browser": server } }));
        configure(tmp.path(), Path::new("/h"), false).unwrap();
        assert!(!file.exists());
        // Already gone: fine; a folder in its place cannot be removed as a file.
        configure(tmp.path(), Path::new("/h"), false).unwrap();
        std::fs::create_dir(&file).unwrap();
        assert!(configure(tmp.path(), Path::new("/h"), false).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let bad = Path::new(OsStr::from_bytes(b"/bad\xff"));
            let err = configure(tmp.path(), bad, true).unwrap_err();
            assert!(err.to_string().contains("not UTF-8"), "{err}");
        }
    }

    #[test]
    fn the_setting_wins_else_the_first_installed_candidate() {
        let tmp = tempfile::tempdir().unwrap();
        let (a, b) = (tmp.path().join("a/chrome"), tmp.path().join("b/chromium"));
        exe(&b, "");
        let snap = tmp.path().join("snap");
        let both = || vec![a.clone(), b.clone()];
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
        let none = find(None, vec![missing], &snap).unwrap_err();
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
        // Past its first KiB: the first 64 KiB are read.
        let pad = "#".repeat(4096);
        let text = format!("#!/bin/sh\n{pad}\nexec {} \"$@\"\n", real.display());
        exe(&script, &text);
        let other = tmp.path().join("opt/brave-browser");
        let found =
            |list: &[&PathBuf]| find(None, list.iter().map(|p| (*p).clone()).collect(), &snap);
        let err = found(&[&link, &script]).unwrap_err();
        assert!(
            err.starts_with(&format!(
                "Only a snap browser was found ({})",
                link.display()
            )),
            "{err}"
        );
        assert!(err.ends_with("set browser.path to another one"), "{err}");
        exe(&other, &format!("#!/bin/sh\n{pad}\necho /snapshot\n"));
        assert_eq!(found(&[&link, &script, &other]), Ok(other.clone()));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn candidates_are_the_programs_on_path() {
        let home = Path::new("/h");
        let path = std::env::join_paths(["/usr/bin", "rel", "/opt/b"]).unwrap();
        let found = candidates(Some(home), Some(&path));
        assert_eq!(found.len(), 12);
        assert_eq!(found[0], Path::new("/usr/bin/google-chrome"));
        assert_eq!(found[1], Path::new("/opt/b/google-chrome"));
        assert_eq!(found[11], Path::new("/opt/b/brave-browser"));
        assert_eq!(candidates(None, None), Vec::<PathBuf>::new());
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
        let url = crate::adapter::clip(&long, MAX_URL);
        assert_eq!(url.chars().count(), MAX_URL);
        assert_eq!(out.events, [Event::Url(Some(url))]);
        for i in 0..=MAX_PAGES {
            screen.message(&page(
                "Target.targetCreated",
                &format!("t{i}"),
                "about:blank",
            ));
        }
        assert_eq!(screen.pages.len(), MAX_PAGES);
        assert_eq!(screen.pages[0].target, "t1");
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

    /// A service at `paths`' socket that answers each connection with `reply` (none: closes).
    #[cfg(unix)]
    fn service(paths: &Paths, reply: Option<Control>) -> tokio::task::JoinHandle<()> {
        let listener = tokio::net::UnixListener::bind(paths.socket()).unwrap();
        tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut framed = Framed::new(stream, FrameCodec);
            let hello = framed.next().await.unwrap().unwrap().to_control().unwrap();
            assert!(matches!(hello, Control::Hello { .. }));
            let ask = framed.next().await.unwrap().unwrap().to_control().unwrap();
            assert_eq!(ask, Control::BrowserEndpoint { cwd: "/w".into() });
            if let Some(reply) = reply {
                framed
                    .send(Frame::control(0, &Control::Resync))
                    .await
                    .unwrap();
                framed.send(Frame::control(0, &reply)).await.unwrap();
                // Holds the connection until the client gives up.
                let _ = framed.next().await;
            }
        })
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn the_endpoint_ask_reports_each_answer_of_the_service() {
        let tmp = tempfile::tempdir().unwrap();
        let run = tmp.path().join("run");
        crate::mode::folder(std::fs::DirBuilder::new().recursive(true), 0o700)
            .create(&run)
            .unwrap();
        let paths = Paths {
            logs: run.clone(),
            runtime: run,
            data: tmp.path().join("data"),
            config: tmp.path().join("config"),
        };
        let wait = Duration::from_secs(10);
        let err = endpoint(&paths, "/w", wait).await.unwrap_err();
        assert!(err.starts_with("cannot reach the Hive service: "), "{err}");
        let answers = [
            (
                Some(Control::BrowserReady {
                    endpoint: "http://127.0.0.1:9".into(),
                }),
                Ok("http://127.0.0.1:9".to_owned()),
            ),
            (
                Some(Control::Error {
                    message: "off".into(),
                }),
                Err("off".to_owned()),
            ),
            (
                Some(Control::VersionMismatch {
                    protocol: 0,
                    version: "0.0.0".into(),
                }),
                Err(format!("the Hive service is version 0.0.0, not {VERSION}")),
            ),
            (
                None,
                Err("the Hive service closed the connection".to_owned()),
            ),
        ];
        for (reply, expected) in answers {
            let server = service(&paths, reply);
            assert_eq!(endpoint(&paths, "/w", wait).await, expected);
            server.await.unwrap();
            std::fs::remove_file(paths.socket()).unwrap();
        }
        // A service that never answers.
        let listener = tokio::net::UnixListener::bind(paths.socket()).unwrap();
        let held = tokio::spawn(async move { listener.accept().await.map(|(stream, _)| stream) });
        let slow = endpoint(&paths, "/w", Duration::from_millis(100)).await;
        assert_eq!(
            slow,
            Err("the Hive service did not start the browser in time".to_owned())
        );
        drop(held);
    }

    /// A WebSocket to `url`, over a plain TCP stream as the service opens one.
    async fn client(url: &str) -> WebSocketStream<TcpStream> {
        let host = url.trim_start_matches("ws://").split('/').next().unwrap();
        let stream = TcpStream::connect(host).await.unwrap();
        tokio_tungstenite::client_async(url, stream)
            .await
            .unwrap()
            .0
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
        let ws = client(&url).await;
        let (events_tx, mut events) = mpsc::unbounded_channel();
        let cast = tokio::spawn(screencast(ws, rx, events_tx));
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
        let ws = client(&format!("ws://{addr}/")).await;
        let (commands, rx) = mpsc::unbounded_channel::<Command>();
        drop(commands);
        let (events, _) = mpsc::unbounded_channel();
        screencast(ws, rx, events).await;
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
        // Run once first: macOS checks a new program on its first run, seconds when loaded.
        let first = std::process::Command::new(&quits).status().unwrap();
        assert_eq!(first.code(), Some(3));
        let err = start(&quits, profile.clone(), START_TIMEOUT)
            .await
            .err()
            .unwrap();
        assert!(err.starts_with("The browser exited as it started"), "{err}");
        assert!(!profile.exists());
        let err = start(&tmp.path().join("missing"), profile.clone(), START_TIMEOUT)
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
        let port_file = tmp.path().join("port");
        std::fs::write(&port_file, port.to_string()).unwrap();
        let script = format!(
            "#!/bin/sh\nfor a; do case $a in --user-data-dir=*) d=${{a#--user-data-dir=}};; esac; done\n[ -n \"$d\" ] || exit 0\nprintf '%s\\n/devtools/browser/x\\n' \"$(cat '{}')\" > \"$d/DevToolsActivePort\"\nexec sleep 30\n",
            port_file.display()
        );
        exe(&fake, &script);
        // Run once first: macOS checks a new program on its first run, seconds when loaded.
        assert!(
            std::process::Command::new(&fake)
                .status()
                .unwrap()
                .success()
        );
        let err = start(&fake, profile.clone(), START_TIMEOUT)
            .await
            .err()
            .unwrap();
        assert!(err.starts_with("Cannot connect to the browser"), "{err}");
        assert!(!profile.exists());
        // A browser that answers: started, its profile kept until it stops.
        let answering = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = answering.local_addr().unwrap().port();
        std::fs::write(&port_file, port.to_string()).unwrap();
        let cdp = tokio::spawn(async move {
            let (stream, _) = answering.accept().await.unwrap();
            tokio_tungstenite::accept_async(stream).await.unwrap()
        });
        let (process, _ws, started) = start(&fake, profile.clone(), START_TIMEOUT).await.unwrap();
        assert_eq!(started, port);
        assert!(profile.join("DevToolsActivePort").is_file());
        process.stop().await;
        assert!(!profile.exists());
        drop(cdp.await.unwrap());
        // A port that takes the connection, then closes it: no WebSocket.
        let refusing = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        std::fs::write(
            &port_file,
            refusing.local_addr().unwrap().port().to_string(),
        )
        .unwrap();
        let closer = std::thread::spawn(move || drop(refusing.accept()));
        let err = start(&fake, profile.clone(), START_TIMEOUT)
            .await
            .err()
            .unwrap();
        assert!(err.starts_with("Cannot connect to the browser"), "{err}");
        closer.join().unwrap();
        // A port that takes the connection and never answers: the whole start is bounded.
        let mute = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        std::fs::write(&port_file, mute.local_addr().unwrap().port().to_string()).unwrap();
        let held = tokio::spawn(async move { mute.accept().await.map(|(stream, _)| stream) });
        let wait = Duration::from_millis(500);
        let err = start(&fake, profile.clone(), wait).await.err().unwrap();
        assert_eq!(err, "The browser did not open its DevTools port in time");
        assert!(!profile.exists());
        drop(held);
        // A browser that never writes its port.
        let silent = tmp.path().join("silent");
        exe(
            &silent,
            "#!/bin/sh\n[ -n \"$1\" ] || exit 0\nexec sleep 30\n",
        );
        assert!(
            std::process::Command::new(&silent)
                .status()
                .unwrap()
                .success()
        );
        let wait = Duration::from_millis(300);
        let err = start(&silent, profile.clone(), wait).await.err().unwrap();
        assert_eq!(err, "The browser did not open its DevTools port in time");
        assert!(!profile.exists());
        // The profile cannot be made.
        std::fs::write(tmp.path().join("file"), "").unwrap();
        let err = start(&fake, tmp.path().join("file/p"), START_TIMEOUT)
            .await
            .err()
            .unwrap();
        assert!(
            err.starts_with("Cannot make the browser's profile"),
            "{err}"
        );
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread")]
    async fn browsers_an_earlier_service_left_are_killed_and_their_profiles_removed() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("browsers");
        let alive =
            |pid: u32| nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid as i32), None).is_ok();
        // One started as the service starts them, then left: the service was killed. It waits
        // in a loop, so its arguments stay its own.
        let fake = tmp.path().join("left");
        let port_file = tmp.path().join("port");
        let script = format!(
            "#!/bin/sh\nfor a; do case $a in --user-data-dir=*) d=${{a#--user-data-dir=}};; esac; done\n[ -n \"$d\" ] || exit 0\nprintf '%s\\n/devtools/browser/x\\n' \"$(cat '{}')\" > \"$d/DevToolsActivePort\"\nwhile :; do sleep 1; done\n",
            port_file.display()
        );
        exe(&fake, &script);
        // Run once first: macOS checks a new program on its first run, seconds when loaded.
        assert!(
            std::process::Command::new(&fake)
                .status()
                .unwrap()
                .success()
        );
        let answering = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        std::fs::write(
            &port_file,
            answering.local_addr().unwrap().port().to_string(),
        )
        .unwrap();
        let cdp = tokio::spawn(async move {
            let (stream, _) = answering.accept().await.unwrap();
            tokio_tungstenite::accept_async(stream).await.unwrap()
        });
        let profile = dir.join("1");
        let (process, _ws, _) = start(&fake, profile.clone(), START_TIMEOUT).await.unwrap();
        let _cdp = cdp.await.unwrap();
        let group = process.child.id().unwrap();
        // Never stopped.
        std::mem::forget(process);
        // Another process now has the number another profile names: not ours, left alone.
        let other = tokio::process::Command::new("sleep")
            .arg("30")
            .process_group(0)
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let stranger = other.id().unwrap();
        std::fs::create_dir_all(dir.join("2")).unwrap();
        std::fs::write(dir.join("2").join(GROUP_FILE), stranger.to_string()).unwrap();
        // Unreadable numbers and a profile without one: nothing to kill.
        for (name, text) in [("3", "x"), ("4", "")] {
            std::fs::create_dir_all(dir.join(name)).unwrap();
            std::fs::write(dir.join(name).join(GROUP_FILE), text).unwrap();
        }
        std::fs::create_dir_all(dir.join("6")).unwrap();
        let source = crate::procs::Source::System;
        reap(&dir, source, &mut kill_group);
        assert!(!dir.exists());
        let pid = nix::unistd::Pid::from_raw(group as i32);
        // A thread of its own: a test runtime would wait for it on a failure.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || tx.send(nix::sys::wait::waitpid(pid, None)));
        let ended = rx.recv_timeout(Duration::from_secs(10));
        // Not left running when the test fails.
        let _ = nix::sys::signal::killpg(pid, nix::sys::signal::SIGKILL);
        let killed = nix::sys::wait::WaitStatus::Signaled(pid, nix::sys::signal::SIGKILL, false);
        assert_eq!(ended, Ok(Ok(killed)));
        assert!(alive(stranger), "{stranger} was killed");
        drop(other);
        // No folder: nothing to do.
        reap(&dir, source, &mut kill_group);
    }

    #[cfg(unix)]
    #[test]
    fn groups_0_1_and_negative_are_never_killed_even_with_a_browsers_arguments() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("browsers");
        // A process table where each of them runs with its profile's arguments.
        let procs = tmp.path().join("proc");
        for (name, group) in [("a", 1), ("b", 0), ("c", -7), ("d", 4242)] {
            let profile = dir.join(name);
            std::fs::create_dir_all(&profile).unwrap();
            std::fs::write(profile.join(GROUP_FILE), group.to_string()).unwrap();
            let entry = procs.join(group.to_string());
            std::fs::create_dir_all(&entry).unwrap();
            let cmdline = format!("chrome\0--user-data-dir={}\0", profile.display());
            std::fs::write(entry.join("cmdline"), cmdline).unwrap();
        }
        let mut killed = Vec::new();
        reap(&dir, crate::procs::Source::Dir(&procs), &mut |g| {
            killed.push(g)
        });
        assert_eq!(killed, [4242]);
        assert!(!dir.exists());
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
