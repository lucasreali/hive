//! `hive daemon`: the service. Lives exactly as long as the app connection.

use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::ffi::OsStr;
use std::fs::{File, Permissions};
use std::io;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;

use futures_util::{SinkExt, StreamExt};
use hive_protocol::{
    AgentEvent, Control, EventKind, Frame, FrameCodec, FrameError, FrameType, PROTOCOL_VERSION,
    Role, SaveError,
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
use crate::files::{Listing, Watcher};
use crate::paths::Paths;
use crate::projects::{self, Projects};
use crate::states::Agent;
use crate::terminal::{self, Input, Terminal};
use crate::{changes, file, procs, watch, worktree, wrapper};

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
        watching: Mutex::new(None),
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
    /// Control frames to the app connection's writer, while an app is connected.
    app: Mutex<Option<mpsc::UnboundedSender<Frame>>>,
    /// Open terminals by channel. The channel number is also the `HIVE_TERMINAL_ID`.
    terminals: Mutex<HashMap<u32, Terminal>>,
    /// Detected agents by session id, with their terminal and state. Locked after `terminals`
    /// when both are needed.
    agents: Mutex<HashMap<String, Agent>>,
    /// The task watching the worktree of the app's files panel.
    watching: Mutex<Option<tokio::task::JoinHandle<()>>>,
    bin_dir: PathBuf,
    projects: Projects,
}

impl State {
    /// Sends a control message to the app, if one is connected.
    async fn to_app(&self, channel: u32, message: &Control) {
        if let Some(app) = &*self.app.lock().await {
            let _ = app.send(Frame::control(channel, message));
        }
    }

    /// Tracks agents: a `SessionStart` from one of our terminals marks its `claude` as hooked
    /// and detects the agent; any other event of a detected agent (or of its subagents)
    /// updates its state; its own `SessionEnd` removes it.
    async fn saw(&self, event: &AgentEvent) {
        if event.kind == EventKind::SessionStarted {
            if let Some(channel) = event.terminal_id.as_deref().and_then(|t| t.parse().ok()) {
                self.detect(channel, event).await;
            }
            return;
        }
        let Some(id) = &event.session_id else { return };
        let mut agents = self.agents.lock().await;
        let Some(agent) = agents.get_mut(id) else {
            return;
        };
        let channel = agent.channel;
        let place = |cwd: &str| {
            let place = tokio::task::block_in_place(|| projects::place(&self.projects.list(), cwd));
            place.map(|(_, worktree)| worktree)
        };
        if let Some(state) = agent.apply(id, event, Instant::now(), &place) {
            self.to_app(channel, &state).await;
        }
        if event.subagent.is_none() && matches!(event.kind, EventKind::SessionEnded { .. }) {
            agents.remove(id);
            let id = id.clone();
            self.to_app(channel, &Control::AgentRemoved { id }).await;
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
        let mut agent = Agent::new(channel, Instant::now());
        agent.worktree = worktree.clone();
        let state = agent.message(&id);
        agents.insert(id.clone(), agent);
        let detected = Control::AgentDetected {
            id,
            project,
            worktree,
            cwd,
        };
        self.to_app(channel, &detected).await;
        self.to_app(channel, &state).await;
    }

    /// Sent to a newly connected app right after `Welcome`: the state of every live agent.
    async fn snapshot(&self) {
        for (id, agent) in self.agents.lock().await.iter() {
            self.to_app(agent.channel, &agent.message(id)).await;
        }
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

    /// Watches `path` for the files panel instead of the worktree watched until now, if any.
    async fn watch_worktree(self: &Arc<Self>, path: Option<String>) {
        let mut watching = self.watching.lock().await;
        if let Some(task) = watching.take() {
            task.abort();
        }
        *watching = path.map(|path| tokio::spawn(watch_files(self.clone(), path)));
    }

    /// The one place a change in the watched worktree `path` is reported to the app, after
    /// the debounce: `files` when the listing changed (`None` when it did not), then its
    /// `changes` every time, since an edit changes the diff but not the list.
    async fn worktree_changed(&self, path: &str, listing: Option<&Listing>) {
        if let Some(listing) = listing {
            let files = Control::Files {
                path: path.to_owned(),
                files: listing.files.clone(),
                truncated: listing.truncated,
            };
            self.to_app(0, &files).await;
        }
        let listed = tokio::task::block_in_place(|| changes::list(Path::new(path)));
        self.to_app(0, &changes::message(path.to_owned(), listed))
            .await;
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

/// Warns the app about terminals running a `claude` that sends no hook events, and applies
/// the silence rule to agents whose terminal went quiet.
async fn watch_terminals(state: Arc<State>) {
    let mut ticks = tokio::time::interval(watch::INTERVAL);
    loop {
        ticks.tick().await;
        let running = watch::claude_sessions(&procs::list(Path::new("/proc")));
        let now = Instant::now();
        let mut last_output = HashMap::new();
        let unhooked: Vec<u32> = state
            .terminals
            .lock()
            .await
            .iter_mut()
            .filter_map(|(channel, t)| {
                last_output.insert(*channel, t.last_output);
                let session = t.session;
                t.watch
                    .tick(running.contains(&session), now)
                    .then_some(*channel)
            })
            .collect();
        for channel in unhooked {
            state.to_app(channel, &Control::UnhookedAgent).await;
        }
        // Not under the terminals lock: placing a new agent holds the agents lock while git runs.
        for (id, agent) in state.agents.lock().await.iter_mut() {
            let output = last_output.get(&agent.channel);
            if let Some(message) = output.and_then(|&output| agent.reconcile(id, output, now)) {
                state.to_app(agent.channel, &message).await;
            }
        }
    }
}

/// Lists the worktree `path` now and after every change, until aborted or the watch fails.
/// Git and inotify run on a blocking thread, off the frame loop.
async fn watch_files(state: Arc<State>, path: String) {
    let started = tokio::task::block_in_place(|| {
        let root = state.projects.worktree(&path)?;
        Watcher::new(&root)
    });
    let mut watcher = match started {
        Ok(watcher) => watcher,
        Err(err) => return state.to_app(0, &error(err)).await,
    };
    let mut last = None;
    let mut watching = Ok(());
    while watching.is_ok() {
        match tokio::task::block_in_place(|| watcher.list()) {
            Ok(listing) => {
                let changed = last.as_ref() != Some(&listing);
                state
                    .worktree_changed(&path, changed.then_some(&listing))
                    .await;
                last = Some(listing);
            }
            // E.g. the worktree was removed; it is listed again on the next change.
            Err(err) => state.to_app(0, &error(err)).await,
        }
        watching = watcher.changed().await;
    }
}

fn error(err: io::Error) -> Control {
    Control::Error {
        message: err.to_string(),
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
        let mut terminals = state.terminals.lock().await;
        terminals
            .entry(channel)
            .and_modify(|t| t.last_output = Instant::now());
        drop(terminals);
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
        for (id, _) in agents.extract_if(|_, a| a.channel == channel) {
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
    state: &Arc<State>,
) {
    if let Some(Ok(frame)) = reader.next().await
        && let Ok(Control::Hook {
            event,
            terminal_id,
            payload,
        }) = frame.to_control()
    {
        let event = ClaudeCode.translate(&event, terminal_id, payload);
        if let EventKind::WorktreeCreated { .. } | EventKind::WorktreeRemoved { .. } = event.kind {
            // The app's worktrees follow a `claude -w` or a subagent's worktree.
            state.projects(|projects| Control::Projects {
                projects: projects.list(),
            });
        }
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
        *app = Some(control_tx);
    }
    state.snapshot().await;
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
        Ok(Control::WatchWorktree { path }) => state.watch_worktree(Some(path)).await,
        Ok(Control::UnwatchWorktree) => state.watch_worktree(None).await,
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
        Ok(Control::ListChanges { path }) => state.projects(move |projects| {
            let listed = projects.worktree(&path).and_then(|dir| changes::list(&dir));
            changes::message(path, listed)
        }),
        Ok(Control::OpenFile { worktree, path }) => state.projects(move |projects| {
            let read = projects
                .worktree(&worktree)
                .and_then(|dir| file::read(&dir, &path));
            file::message(worktree, path, read)
        }),
        Ok(Control::SaveFile {
            worktree,
            path,
            content,
            version,
        }) => state.projects(move |projects| {
            let saved = match projects.worktree(&worktree) {
                Ok(dir) => file::save(&dir, &path, &content, version.as_deref()),
                Err(err) => Err((SaveError::InvalidPath, err.to_string())),
            };
            match saved {
                Ok(version) => Control::FileSaved {
                    worktree,
                    path,
                    version,
                },
                Err((error, message)) => Control::SaveFailed {
                    worktree,
                    path,
                    error,
                    message,
                },
            }
        }),
        Ok(Control::OpenInEditor { worktree, path }) => state.projects(move |projects| {
            let located = projects
                .worktree(&worktree)
                .and_then(|dir| file::windows_path(&dir, &path, OsStr::new("wslpath")));
            Control::EditorTarget {
                worktree,
                path,
                error: located.as_ref().err().map(ToString::to_string),
                windows_path: located.ok(),
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
        match writer.send(frame).await {
            // Nothing was written: the app misses this message, not every later one.
            Err(FrameError::Oversized(len)) => {
                eprintln!("hive: warning: dropped a {len}-byte message to the app");
            }
            Err(_) => return,
            Ok(()) => {}
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
    async fn a_connecting_app_gets_the_state_of_every_live_agent() {
        // Agents outlive an app connection only in principle (the service exits with the
        // app), so the snapshot is checked here rather than through a real daemon.
        let dir = tempfile::tempdir().unwrap();
        let state = Arc::new(State {
            app: Mutex::new(None),
            terminals: Mutex::new(HashMap::new()),
            agents: Mutex::new(HashMap::from([(
                "s".to_owned(),
                Agent::new(4, Instant::now()),
            )])),
            watching: Mutex::new(None),
            bin_dir: dir.path().into(),
            projects: Projects::load(dir.path().join("projects.json")),
        });
        // The same stream types as the daemon, so no second instantiation skews line coverage.
        let (client, server) = UnixStream::pair().unwrap();
        let (read, write) = server.into_split();
        let serving = tokio::spawn({
            let state = state.clone();
            async move {
                let reader = FramedRead::new(read, FrameCodec);
                app_connection(reader, FramedWrite::new(write, FrameCodec), &state).await
            }
        });
        let mut frames = FramedRead::new(client, FrameCodec);
        let first = tokio::time::timeout(std::time::Duration::from_secs(5), frames.next());
        let frame = first.await.expect("no snapshot").unwrap().unwrap();
        assert_eq!(frame.channel, 4);
        assert_eq!(
            frame.to_control().unwrap(),
            Control::AgentState {
                id: "s".into(),
                state: hive_protocol::AgentState::Idle,
                urgency: 1,
                pending: false,
                subagents: vec![],
            }
        );
        drop(frames);
        assert!(serving.await.unwrap());
    }

    #[tokio::test]
    async fn a_frame_too_big_to_write_is_dropped_and_writing_goes_on() {
        let (control_tx, control_rx) = mpsc::unbounded_channel();
        let (terminal_tx, terminal_rx) = mpsc::channel(1);
        let huge = Frame {
            kind: FrameType::Control,
            channel: 0,
            payload: Bytes::from(vec![b' '; hive_protocol::MAX_PAYLOAD + 1]),
        };
        control_tx.send(huge).unwrap();
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
        assert_eq!(frames, vec![Frame::control(0, &Control::CloseTerminal)]);
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
