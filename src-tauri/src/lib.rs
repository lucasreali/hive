//! Testable code of the Tauri app: the link to the service through `hive bridge` (#14, #24)
//! and the commands the UI calls. `main.rs` only wires it.
//!
//! Control messages go to the UI on one `Channel` given by `connect`, as the service's JSON
//! plus a `channel` field. Terminal output goes, as raw bytes, to the `Channel` given for that
//! terminal by `open_terminal`. No Tauri events are used.

use std::collections::HashMap;
use std::ffi::OsString;
use std::future::Future;
use std::process::Stdio;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use hive_protocol::{Control, Frame, FrameCodec, FrameType, Role, MAX_PAYLOAD};
use serde_json::{json, Value};
use tauri::ipc::{Channel, InvokeResponseBody};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::sync::mpsc;
use tokio_util::codec::{FramedRead, FramedWrite};

/// App version, compared with the `hive` binary's in the handshake (#29).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Runs inside WSL through `wsl.exe --exec`, so no login or interactive shell (and no fish
/// config) runs first. `$1` is the optional `HIVE_BRIDGE` override, passed as an argument.
const BRIDGE_SCRIPT: &str = r#"exec "${1:-$HOME/.cargo/bin/hive}" bridge"#;

/// How much of the bridge's stderr is kept as the disconnect reason.
const STDERR_LIMIT: u64 = 16_384;
/// How long to wait for the bridge to exit and flush stderr once its stdout closed.
const EXIT_WAIT: Duration = Duration::from_secs(2);

const NOT_CONNECTED: &str = "not connected to the hive service";

/// Program and arguments that start `hive bridge` from Windows (#14, #29).
/// `HIVE_WSL_DISTRO` picks the WSL distribution and `HIVE_BRIDGE` the `hive` binary
/// (an absolute Linux path, for development). Neither is ever spliced into the script.
pub fn bridge_command(var: impl Fn(&str) -> Option<OsString>) -> (OsString, Vec<OsString>) {
    let var = |key| var(key).filter(|value| !value.is_empty());
    let mut args = Vec::new();
    if let Some(distro) = var("HIVE_WSL_DISTRO") {
        args.extend([OsString::from("-d"), distro]);
    }
    args.extend(["--exec", "/bin/sh", "-c", BRIDGE_SCRIPT, "sh"].map(OsString::from));
    args.extend(var("HIVE_BRIDGE"));
    ("wsl.exe".into(), args)
}

/// Tauri state: the bridge command and the live link to the service.
pub struct Hive {
    program: OsString,
    args: Vec<OsString>,
    link: Arc<Mutex<Link>>,
}

#[derive(Default)]
struct Link {
    /// Control messages to the UI.
    ui: Option<Channel<Value>>,
    /// Frames to the service; `None` when there is no connection.
    frames: Option<mpsc::UnboundedSender<Frame>>,
    /// Replayed to a reloaded UI.
    welcome: Option<Value>,
    /// Output channel of every open terminal, by frame channel.
    terminals: HashMap<u32, Channel<InvokeResponseBody>>,
    last_channel: u32,
}

impl Link {
    fn to_ui(&self, message: Value) {
        if let Some(ui) = &self.ui {
            let _ = ui.send(message);
        }
    }

    fn frame(&self, frame: Frame) -> Result<(), String> {
        self.frames
            .as_ref()
            .and_then(|frames| frames.send(frame).ok())
            .ok_or_else(|| NOT_CONNECTED.to_owned())
    }

    fn send(&self, channel: u32, message: &Control) -> Result<(), String> {
        self.frame(Frame::control(channel, message))
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
            program,
            args,
            link: Arc::default(),
        }
    }

    fn link(&self) -> MutexGuard<'_, Link> {
        lock(&self.link)
    }

    /// Sends every service message to `ui`, starting the bridge unless a connection is up.
    /// A reloaded UI calls this again: its old terminals are closed and `welcome` is replayed.
    /// Must run inside the Tokio runtime.
    pub fn connect(&self, ui: Channel<Value>) {
        let mut link = self.link();
        link.ui = Some(ui);
        for id in std::mem::take(&mut link.terminals).into_keys() {
            let _ = link.send(id, &Control::CloseTerminal);
        }
        if link.frames.is_some() {
            if let Some(welcome) = link.welcome.clone() {
                link.to_ui(welcome);
            }
            return;
        }
        drop(link);
        if let Err(error) = self.spawn() {
            disconnected(
                &mut self.link(),
                format!("cannot start the hive bridge: {error}"),
            );
        }
    }

    fn spawn(&self) -> std::io::Result<()> {
        let mut command = tokio::process::Command::new(&self.program);
        command
            .args(&self.args)
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
        tokio::spawn(async move {
            let reason = match pump(&link, FramedRead::new(reader, FrameCodec)).await {
                End::Refused => return,
                End::Closed => exit.await,
                End::Broken(error) => error,
            };
            disconnected(&mut lock(&link), reason);
        });
    }

    /// Opens a terminal on a new channel; its output goes to `output`. Returns the channel.
    pub fn open_terminal(
        &self,
        cwd: String,
        cols: u16,
        rows: u16,
        output: Channel<InvokeResponseBody>,
    ) -> Result<u32, String> {
        let mut link = self.link();
        let id = link
            .last_channel
            .checked_add(1)
            .ok_or("no terminal channel left")?;
        link.send(id, &Control::OpenTerminal { cwd, cols, rows })?;
        link.last_channel = id;
        link.terminals.insert(id, output);
        Ok(id)
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
            if let Some(output) = link.terminals.get(&frame.channel) {
                let _ = output.send(InvokeResponseBody::Raw(frame.payload.to_vec()));
            }
            continue;
        }
        let message = match frame.to_control() {
            Ok(message) => message,
            Err(error) => return End::Broken(error.to_string()),
        };
        // Messages for a terminal this UI did not open (e.g. one closed by a reload) are dropped.
        if frame.channel != 0 && !link.terminals.contains_key(&frame.channel) {
            continue;
        }
        let mut value = serde_json::to_value(&message).unwrap_or_default();
        value["channel"] = frame.channel.into();
        match message {
            Control::Welcome { .. } => link.welcome = Some(value.clone()),
            Control::VersionMismatch { .. } => {
                link.to_ui(value);
                link.frames = None;
                return End::Refused;
            }
            Control::TerminalExited { .. } => drop(link.terminals.remove(&frame.channel)),
            _ => {}
        }
        link.to_ui(value);
    }
    End::Closed
}

/// Ends the connection: every open terminal exits, then the UI gets `disconnected`.
fn disconnected(link: &mut Link, reason: String) {
    link.frames = None;
    link.welcome = None;
    for id in std::mem::take(&mut link.terminals).into_keys() {
        link.to_ui(json!({"type": "terminal_exited", "channel": id, "code": null}));
    }
    link.to_ui(json!({"type": "disconnected", "reason": reason}));
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

    #[tauri::command]
    pub fn open_terminal(
        hive: State<'_, Hive>,
        cwd: String,
        cols: u16,
        rows: u16,
        on_data: Channel<InvokeResponseBody>,
    ) -> Result<u32, String> {
        hive.open_terminal(cwd, cols, rows, on_data)
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
}

#[cfg(test)]
mod tests;
