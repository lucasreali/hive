//! `hive daemon`: the service. Lives exactly as long as the app connection.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::fs::{File, Permissions};
use std::io;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;

use futures_util::{SinkExt, StreamExt};
use hive_protocol::{
    AgentEvent, Control, EventKind, Frame, FrameCodec, FrameType, PROTOCOL_VERSION, Role,
};
use pty_process::OwnedReadPty;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::net::{UnixListener, UnixStream};
use tokio::process::Child;
use tokio::signal::unix::{Signal, SignalKind, signal};
use tokio::sync::{Mutex, mpsc};
use tokio_util::codec::{FramedRead, FramedWrite};

use crate::VERSION;
use crate::adapter::{Adapter, ClaudeCode};
use crate::paths::Paths;
use crate::projects::{self, Projects};
use crate::terminal::{self, Input, Terminal};
use crate::{procs, watch, worktree, wrapper};

/// Terminal output waiting to be written to the app; bounded so a slow app slows the PTYs down.
const TERMINAL_QUEUE: usize = 256;

pub async fn run(paths: &Paths) -> io::Result<()> {
    paths.prepare_runtime()?;
    let _lock = lock(paths)?;
    wrapper::install(paths, &std::env::current_exe()?)?;
    // Handle SIGTERM before anyone can connect, so an early one still cleans up.
    let terminate = signal(SignalKind::terminate())?;
    let socket = paths.socket();
    // A socket left by a crashed daemon; the lock proves nobody is serving it.
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket)?;
    std::fs::set_permissions(&socket, Permissions::from_mode(0o600))?;
    let projects = Projects::load(paths.projects());
    let result = serve(listener, terminate, paths.bin_dir(), projects).await;
    let _ = std::fs::remove_file(&socket);
    result
}

/// Single-instance guard: an exclusive lock held for the daemon's whole life.
fn lock(paths: &Paths) -> io::Result<File> {
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(paths.lock())?;
    file.try_lock().map_err(|err| {
        io::Error::other(format!(
            "cannot lock {}: {err}; is another hive daemon running?",
            paths.lock().display()
        ))
    })?;
    Ok(file)
}

async fn serve(
    listener: UnixListener,
    mut terminate: Signal,
    bin_dir: PathBuf,
    projects: Projects,
) -> io::Result<()> {
    let state = Arc::new(State {
        app: Mutex::new(None),
        terminals: Mutex::new(HashMap::new()),
        agents: Mutex::new(HashMap::new()),
        bin_dir,
        projects,
    });
    let (app_gone, mut app_gone_rx) = mpsc::channel::<()>(1);
    let watcher = tokio::spawn(watch_terminals(state.clone()));
    loop {
        tokio::select! {
            accepted = listener.accept() => {
                let (stream, _) = accepted?;
                tokio::spawn(connection(stream, state.clone(), app_gone.clone()));
            }
            _ = app_gone_rx.recv() => break,
            _ = terminate.recv() => break,
        }
    }
    watcher.abort();
    let sessions: Vec<i32> = state
        .terminals
        .lock()
        .await
        .values()
        .map(|t| t.session)
        .collect();
    terminal::end_sessions(&sessions).await;
    Ok(())
}

struct State {
    app: Mutex<Option<Outbox>>,
    /// Open terminals by channel. The channel number is also the `HIVE_TERMINAL_ID`.
    terminals: Mutex<HashMap<u32, Terminal>>,
    /// Detected agents (Stage 1: presence only): session id → terminal channel. Locked after
    /// `terminals` when both are needed.
    agents: Mutex<HashMap<String, u32>>,
    bin_dir: PathBuf,
    projects: Projects,
}

/// Queue to the app connection's writer.
struct Outbox {
    control: mpsc::UnboundedSender<Frame>,
}

impl State {
    /// Sends a control message to the app, if one is connected.
    async fn to_app(&self, channel: u32, message: &Control) {
        if let Some(app) = &*self.app.lock().await {
            let _ = app.control.send(Frame::control(channel, message));
        }
    }

    /// Tracks agents: a `SessionStart` from one of our terminals marks its `claude` as hooked
    /// and detects the agent; a `SessionEnd` removes it. Subagent events are ignored (Stage 4).
    async fn saw(&self, event: &AgentEvent) {
        match event.kind {
            EventKind::SessionStarted => {
                if let Some(channel) = event.terminal_id.as_deref().and_then(|t| t.parse().ok()) {
                    self.detect(channel, event).await;
                }
            }
            EventKind::SessionEnded { .. } => {
                let Some(id) = agent_id(event) else { return };
                let mut agents = self.agents.lock().await;
                if let Some(channel) = agents.remove(&id) {
                    self.to_app(channel, &Control::AgentRemoved { id }).await;
                }
            }
            _ => {}
        }
    }

    /// Places and announces the agent while holding the agents lock, so a `SessionEnd` or the
    /// terminal's exit arriving while git runs waits and removes it afterwards.
    async fn detect(&self, channel: u32, event: &AgentEvent) {
        let mut terminals = self.terminals.lock().await;
        let Some(terminal) = terminals.get_mut(&channel) else {
            return;
        };
        terminal.watch.hooked();
        let Some(id) = agent_id(event) else { return };
        let mut agents = self.agents.lock().await;
        // Other terminals keep working meanwhile.
        drop(terminals);
        let cwd = event.cwd.clone();
        let place = tokio::task::block_in_place(|| {
            let cwd = cwd.as_deref()?;
            projects::place(&self.projects.list(), cwd)
        });
        let (project, worktree) = place.unzip();
        agents.insert(id.clone(), channel);
        let detected = Control::AgentDetected {
            id,
            project,
            worktree,
            cwd,
        };
        self.to_app(channel, &detected).await;
    }

    async fn input(&self, channel: u32, input: Input) {
        if let Some(terminal) = self.terminals.lock().await.get(&channel) {
            terminal.send(input);
        }
    }

    async fn open(
        self: &Arc<Self>,
        channel: u32,
        cwd: &str,
        cols: u16,
        rows: u16,
        output: mpsc::Sender<Frame>,
    ) {
        let opened = {
            let mut terminals = self.terminals.lock().await;
            match terminals.entry(channel) {
                _ if channel == 0 => Err("terminal channels start at 1".to_owned()),
                Entry::Occupied(_) => Err(format!("terminal {channel} is already open")),
                Entry::Vacant(slot) => terminal::spawn(channel, cwd, cols, rows, &self.bin_dir)
                    .map(|(terminal, pty, child)| {
                        slot.insert(terminal);
                        tokio::spawn(pump(self.clone(), channel, pty, child, output));
                    }),
            }
        };
        let reply = match opened {
            Ok(()) => Control::TerminalOpened,
            Err(message) => Control::Error { message },
        };
        self.to_app(channel, &reply).await;
    }

    /// Answers a project request off the frame loop, since git can take a while.
    fn projects(self: &Arc<Self>, request: impl FnOnce(&Projects) -> Control + Send + 'static) {
        let state = self.clone();
        tokio::spawn(async move {
            // The daemon's runtime is multi-threaded, so other tasks keep running meanwhile.
            let reply = tokio::task::block_in_place(|| request(&state.projects));
            state.to_app(0, &reply).await;
        });
    }

    /// Ends the terminal's processes; its exit is reported by [`pump`].
    async fn close(&self, channel: u32) {
        if let Some(terminal) = self.terminals.lock().await.get(&channel) {
            let sessions = [terminal.session];
            tokio::spawn(async move { terminal::end_sessions(&sessions).await });
        }
    }
}

/// The agent an event belongs to: its session id; `None` for subagent events.
fn agent_id(event: &AgentEvent) -> Option<String> {
    match event.subagent {
        None => event.session_id.clone(),
        Some(_) => None,
    }
}

/// Warns the app about terminals running a `claude` that sends no hook events.
async fn watch_terminals(state: Arc<State>) {
    let mut ticks = tokio::time::interval(watch::INTERVAL);
    loop {
        ticks.tick().await;
        let running = watch::claude_sessions(&procs::list(Path::new("/proc")));
        let now = Instant::now();
        let unhooked: Vec<u32> = state
            .terminals
            .lock()
            .await
            .iter_mut()
            .filter_map(|(channel, t)| {
                let session = t.session;
                t.watch
                    .tick(running.contains(&session), now)
                    .then_some(*channel)
            })
            .collect();
        for channel in unhooked {
            state.to_app(channel, &Control::UnhookedAgent).await;
        }
    }
}

/// Copies PTY output to the app until the PTY closes, then reports the exit.
async fn pump(
    state: Arc<State>,
    channel: u32,
    mut pty: OwnedReadPty,
    mut child: Child,
    output: mpsc::Sender<Frame>,
) {
    let mut buf = vec![0; 64 * 1024];
    while let Ok(n @ 1..) = pty.read(&mut buf).await {
        let frame = Frame::terminal(channel, Bytes::copy_from_slice(&buf[..n]));
        if output.send(frame).await.is_err() {
            break;
        }
    }
    let code = child.wait().await.ok().and_then(|status| status.code());
    {
        let mut terminals = state.terminals.lock().await;
        terminals.remove(&channel);
        let mut agents = state.agents.lock().await;
        for (id, _) in agents.extract_if(|_, t| *t == channel) {
            state.to_app(channel, &Control::AgentRemoved { id }).await;
        }
    }
    state
        .to_app(channel, &Control::TerminalExited { code })
        .await;
}

async fn connection(stream: UnixStream, state: Arc<State>, app_gone: mpsc::Sender<()>) {
    let (read, write) = stream.into_split();
    let mut reader = FramedRead::new(read, FrameCodec);
    let mut writer = FramedWrite::new(write, FrameCodec);
    let Some(role) = handshake(&mut reader, &mut writer).await else {
        return;
    };
    match role {
        Role::Hook => hook_connection(reader, &state).await,
        Role::App => {
            if app_connection(reader, writer, &state).await {
                let _ = app_gone.send(()).await;
            }
        }
    }
}

/// Reads the client's `Hello`; answers `Welcome`, or `VersionMismatch` and gives up.
async fn handshake<R, W>(
    reader: &mut FramedRead<R, FrameCodec>,
    writer: &mut FramedWrite<W, FrameCodec>,
) -> Option<Role>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let hello = reader.next().await?.ok()?.to_control();
    let (reply, role) = match hello {
        Ok(Control::Hello {
            protocol,
            version,
            role,
        }) if protocol == PROTOCOL_VERSION && version == VERSION => (
            Control::Welcome {
                version: VERSION.to_owned(),
                distro: std::env::var("WSL_DISTRO_NAME").ok(),
            },
            Some(role),
        ),
        Ok(Control::Hello { .. }) => (
            Control::VersionMismatch {
                protocol: PROTOCOL_VERSION,
                version: VERSION.to_owned(),
            },
            None,
        ),
        _ => (
            Control::Error {
                message: "expected a hello message".to_owned(),
            },
            None,
        ),
    };
    // A hook client may already be gone after sending its event; that is fine.
    let _ = writer.send(Frame::control(0, &reply)).await;
    role
}

/// A hook connection carries exactly one event; the connection is closed after it.
async fn hook_connection<R: AsyncRead + Unpin>(
    mut reader: FramedRead<R, FrameCodec>,
    state: &State,
) {
    if let Some(Ok(frame)) = reader.next().await
        && let Ok(Control::Hook {
            event,
            terminal_id,
            payload,
        }) = frame.to_control()
    {
        let event = ClaudeCode.translate(&event, terminal_id, payload);
        state.saw(&event).await;
        state.to_app(0, &Control::Agent(event)).await;
    }
}

/// Serves the app until it disconnects. Returns false if another app was already connected.
async fn app_connection<R, W>(
    mut reader: FramedRead<R, FrameCodec>,
    mut writer: FramedWrite<W, FrameCodec>,
    state: &Arc<State>,
) -> bool
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let (control_tx, control_rx) = mpsc::unbounded_channel();
    let (terminal_tx, terminal_rx) = mpsc::channel(TERMINAL_QUEUE);
    {
        let mut app = state.app.lock().await;
        if app.is_some() {
            let message = Control::Error {
                message: "another app is already connected".to_owned(),
            };
            let _ = writer.send(Frame::control(0, &message)).await;
            return false;
        }
        *app = Some(Outbox {
            control: control_tx,
        });
    }
    let writer = tokio::spawn(write_prioritized(writer, control_rx, terminal_rx));
    while let Some(Ok(frame)) = reader.next().await {
        app_frame(state, frame, &terminal_tx).await;
    }
    writer.abort();
    *state.app.lock().await = None;
    true
}

async fn app_frame(state: &Arc<State>, frame: Frame, output: &mpsc::Sender<Frame>) {
    let channel = frame.channel;
    let message = match frame.kind {
        FrameType::Terminal => return state.input(channel, Input::Data(frame.payload)).await,
        FrameType::Control => frame.to_control(),
    };
    match message {
        Ok(Control::OpenTerminal { cwd, cols, rows }) => {
            state.open(channel, &cwd, cols, rows, output.clone()).await;
        }
        Ok(Control::Resize { cols, rows }) => {
            state.input(channel, Input::Resize { cols, rows }).await
        }
        Ok(Control::CloseTerminal) => state.close(channel).await,
        Ok(Control::ListProjects) => state.projects(|projects| Control::Projects {
            projects: projects.list(),
        }),
        Ok(Control::AddProject { path }) => {
            state.projects(move |projects| match projects.add(&path) {
                Ok(project) => Control::ProjectAdded { project },
                Err((error, message)) => Control::AddProjectFailed {
                    path,
                    error,
                    message,
                },
            })
        }
        Ok(Control::ListBranches { project }) => state.projects(move |projects| {
            let (branches, error) = match projects.branches(&project) {
                Ok(branches) => (branches, None),
                Err(err) => (Default::default(), Some(err.to_string())),
            };
            Control::Branches {
                project,
                local: branches.local,
                remote: branches.remote,
                current: branches.current,
                error,
            }
        }),
        Ok(Control::ValidateWorktreeName { project, name }) => state.projects(move |projects| {
            let error = projects.validate_worktree_name(&project, &name).err();
            let (folder, branch) = worktree::planned(&name);
            Control::WorktreeNameValidated {
                project,
                name,
                folder,
                branch,
                error: error.map(|err| err.to_string()),
            }
        }),
        Ok(Control::CreateWorktree {
            project,
            name,
            base,
        }) => state.projects(move |projects| {
            match projects.create_worktree(&project, &name, base.as_deref()) {
                Ok((project, created)) => Control::WorktreeCreated {
                    project,
                    path: created.path.to_string_lossy().into_owned(),
                    notes: created.notes,
                },
                Err(err) => Control::CreateWorktreeFailed {
                    project,
                    name,
                    message: err.to_string(),
                },
            }
        }),
        _ => {
            let message = "unexpected message from the app".to_owned();
            state.to_app(channel, &Control::Error { message }).await;
        }
    }
}

/// Writes queued frames, always draining control frames before terminal frames.
async fn write_prioritized<W: AsyncWrite + Unpin>(
    mut writer: FramedWrite<W, FrameCodec>,
    mut control: mpsc::UnboundedReceiver<Frame>,
    mut terminal: mpsc::Receiver<Frame>,
) {
    loop {
        let frame = tokio::select! {
            biased;
            Some(frame) = control.recv() => frame,
            Some(frame) = terminal.recv() => frame,
            else => return,
        };
        if writer.send(frame).await.is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn control_frames_are_written_before_queued_terminal_frames() {
        let (control_tx, control_rx) = mpsc::unbounded_channel();
        let (terminal_tx, terminal_rx) = mpsc::channel(8);
        terminal_tx.send(Frame::terminal(1, "out")).await.unwrap();
        terminal_tx.send(Frame::terminal(1, "more")).await.unwrap();
        control_tx
            .send(Frame::control(0, &Control::CloseTerminal))
            .unwrap();
        drop((control_tx, terminal_tx));

        let (client, server) = tokio::io::duplex(1024);
        write_prioritized(
            FramedWrite::new(server, FrameCodec),
            control_rx,
            terminal_rx,
        )
        .await;
        let frames: Vec<Frame> = FramedRead::new(client, FrameCodec)
            .map(Result::unwrap)
            .collect()
            .await;
        assert_eq!(
            frames,
            vec![
                Frame::control(0, &Control::CloseTerminal),
                Frame::terminal(1, "out"),
                Frame::terminal(1, "more"),
            ]
        );
    }

    #[tokio::test]
    async fn writer_stops_when_the_peer_is_gone() {
        let (control_tx, control_rx) = mpsc::unbounded_channel();
        let (_terminal_tx, terminal_rx) = mpsc::channel(1);
        control_tx.send(Frame::terminal(1, "x")).unwrap();
        let (client, server) = tokio::io::duplex(64);
        drop(client);
        // Returns instead of looping forever even though the queues stay open.
        let writer = write_prioritized(
            FramedWrite::new(server, FrameCodec),
            control_rx,
            terminal_rx,
        );
        let finished = tokio::time::timeout(std::time::Duration::from_secs(5), writer).await;
        assert!(finished.is_ok());
    }
}
