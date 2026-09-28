//! `hive daemon`: the service. Lives exactly as long as the app connection.

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fs::{File, Permissions, TryLockError};
use std::io;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};

use bytes::Bytes;

use futures_util::{SinkExt, StreamExt};
use hive_protocol::{
    AgentEvent, Control, DiffBase, EventKind, Frame, FrameCodec, FrameError, FrameType, GhAccount,
    OpenSession, PROTOCOL_VERSION, Project, Role, SaveError, SessionTarget,
};
use pty_process::OwnedReadPty;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite};
use tokio::net::{UnixListener, UnixStream};
use tokio::process::Child;
use tokio::signal::unix::{Signal, SignalKind, signal};
use tokio::sync::{Mutex, mpsc};
use tokio_util::codec::{FramedRead, FramedWrite};

use crate::VERSION;
use crate::adapter::{self, Adapter, ClaudeCode};
use crate::files::{Listing, Watcher};
use crate::paths::Paths;
use crate::projects::{self, Projects};
use crate::scripts::{self, Ports};
use crate::sessions::{self, Sessions};
use crate::settings;
use crate::spaces::Spaces;
use crate::states::{self, Agent};
use crate::terminal::{self, Input, Terminal};
use crate::{
    bridge, changes, dirs, file, health, procs, search, transcript, watch, worktree, wrapper,
};

/// Terminal output waiting to be written to the app; bounded so a slow app slows the PTYs down.
const TERMINAL_QUEUE: usize = 256;

/// How often a service waiting for the lock tries it again.
const LOCK_RETRY: Duration = Duration::from_millis(20);

/// Longest `hive badge` label, in characters.
const MAX_BADGE: usize = 40;

pub async fn run(paths: &Paths) -> io::Result<()> {
    // Started by `hive bridge`: leave its session, so the service outlives nothing but the
    // app connection. Fails harmlessly for a group leader (e.g. started from a shell).
    let _ = nix::unistd::setsid();
    paths.prepare_runtime()?;
    let _lock = lock(paths).await?;
    wrapper::install(paths, &std::env::current_exe()?)?;
    // Handle SIGTERM before anyone can connect, so an early one still cleans up.
    let terminate = signal(SignalKind::terminate())?;
    let socket = paths.socket();
    // A socket left by a crashed daemon; the lock proves nobody is serving it.
    let _ = std::fs::remove_file(&socket);
    let listener = UnixListener::bind(&socket)?;
    std::fs::set_permissions(&socket, Permissions::from_mode(0o600))?;
    let projects = Projects::load(paths.spaces(), &paths.projects());
    let sessions = Sessions::new(sessions::root(|key| std::env::var_os(key)));
    let settings = settings::Store::load(paths.settings());
    let ports = Ports::new(paths.ports());
    let restore = Restore {
        file: paths.open_sessions(),
        pending: Mutex::new(sessions::take_open(&paths.open_sessions())),
    };
    let state = State::new(
        paths.bin_dir(),
        projects,
        sessions,
        settings,
        ports,
        restore,
    );
    let state = Arc::new(state);
    state.ask_user_path();
    let result = serve(listener, &socket, terminate, state).await;
    let _ = std::fs::remove_file(&socket);
    result
}

/// Single-instance guard: an exclusive lock held for the daemon's whole life. A service that
/// is still ending holds it for up to its grace period, so the lock is tried again for as long
/// as the bridge waits for a new service.
async fn lock(paths: &Paths) -> io::Result<File> {
    let file = File::options()
        .create(true)
        .truncate(false)
        .write(true)
        .mode(0o600)
        .open(paths.lock())?;
    let retry = async {
        loop {
            match file.try_lock() {
                Err(TryLockError::WouldBlock) => tokio::time::sleep(LOCK_RETRY).await,
                locked => return locked,
            }
        }
    };
    let locked = tokio::time::timeout(bridge::START_TIMEOUT, retry)
        .await
        .unwrap_or_else(|_| file.try_lock());
    locked.map_err(|err| {
        io::Error::other(format!(
            "cannot lock {}: {err}; is another hive daemon running?",
            paths.lock().display()
        ))
    })?;
    Ok(file)
}

async fn serve(
    listener: UnixListener,
    socket: &Path,
    mut terminate: Signal,
    state: Arc<State>,
) -> io::Result<()> {
    let (app_gone, mut app_gone_rx) = mpsc::channel::<()>(1);
    let watcher = tokio::spawn(watch_terminals(state.clone()));
    let health = tokio::spawn(watch_health(state.clone(), health::INTERVAL));
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
    // Released before the terminals end: an app started again meanwhile gets a new service
    // (waiting for the lock) instead of a listen queue nobody accepts.
    drop(listener);
    let _ = std::fs::remove_file(socket);
    watcher.abort();
    health.abort();
    // Before the terminals end (and their sessions with them): what to resume next time.
    state.save_open().await;
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

/// The service's state. Lock order, when one task holds several: `terminals` → `agents` →
/// `app`. `agents` is held across slow work (placing an agent runs git, reading a session
/// log or a transcript), so a terminal's input and output never take an async lock (9.13):
/// they go through `inputs` and [`terminal::LastOutput`]. The std locks (`inputs`, `sent`)
/// are never held across an await.
struct State {
    /// Control frames to the app connection's writer, while an app is connected.
    app: Mutex<Option<mpsc::UnboundedSender<Frame>>>,
    /// Open terminals by channel. The channel number is also the `HIVE_TERMINAL_ID`.
    terminals: Mutex<HashMap<u32, Terminal>>,
    /// Each open terminal's input queue, by channel.
    inputs: std::sync::Mutex<HashMap<u32, mpsc::UnboundedSender<Input>>>,
    /// The worktree each open terminal opened in, by channel, when it is one: a subagent's
    /// own worktree with a terminal of the human's is not only the subagent's (9.35).
    terminal_worktrees: std::sync::Mutex<HashMap<u32, String>>,
    /// Detected agents by session id, with their terminal and state.
    agents: Mutex<HashMap<String, Agent>>,
    /// The task watching the worktree of the app's files panel.
    watching: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// The terminal in view in the focused app window (the app's `view`); 0 when none, since
    /// terminal channels start at 1.
    watched: AtomicU32,
    bin_dir: PathBuf,
    projects: Projects,
    /// Claude Code's session logs of the followed projects.
    sessions: Sessions,
    settings: settings::Store,
    ports: Ports,
    restore: Restore,
    /// The worktree statuses the app has, so only changes are sent.
    sent: std::sync::Mutex<health::Sent>,
    /// The sessions list the app has (`Sessions`), so an unchanged one is not sent again.
    listed: std::sync::Mutex<Option<Control>>,
    /// The last `subagent_worktrees` sent, so only changes are sent.
    owned: std::sync::Mutex<Vec<String>>,
    /// The user's `PATH` ([`wrapper::user_path`]), asked for at start and again, in the
    /// background, while no `claude` is on it: the user's shell never holds up the frames.
    user_path: tokio::sync::watch::Sender<Option<OsString>>,
    /// Whether the user's `PATH` is being asked for.
    asking_path: std::sync::atomic::AtomicBool,
    /// The pull requests lists fetched (9.31).
    pulls: std::sync::Mutex<crate::pulls::Cache>,
}

/// The sessions running in Hive's terminals when the app last closed.
struct Restore {
    /// Where they are kept between runs.
    file: PathBuf,
    /// Read at start, sent to the first app that connects.
    pending: Mutex<Vec<OpenSession>>,
}

impl State {
    fn new(
        bin_dir: PathBuf,
        projects: Projects,
        sessions: Sessions,
        settings: settings::Store,
        ports: Ports,
        restore: Restore,
    ) -> Self {
        Self {
            app: Mutex::new(None),
            terminals: Mutex::new(HashMap::new()),
            inputs: Default::default(),
            terminal_worktrees: Default::default(),
            agents: Mutex::new(HashMap::new()),
            watching: Mutex::new(None),
            watched: AtomicU32::new(0),
            bin_dir,
            projects,
            sessions,
            settings,
            ports,
            restore,
            sent: Default::default(),
            listed: Default::default(),
            owned: Default::default(),
            user_path: tokio::sync::watch::Sender::new(None),
            asking_path: Default::default(),
            pulls: Default::default(),
        }
    }

    /// Asks for the user's `PATH` in the background, unless it is already being asked for.
    fn ask_user_path(self: &Arc<Self>) {
        if self.asking_path.swap(true, Ordering::SeqCst) {
            return;
        }
        let state = self.clone();
        tokio::spawn(async move {
            let var = std::env::var_os;
            let (shell, timeout) = (wrapper::path_shell(), wrapper::SHELL_TIMEOUT);
            let path = wrapper::user_path(shell, var("PATH"), var("HOME"), timeout).await;
            // Done before the answer wakes anyone waiting for it.
            state.asking_path.store(false, Ordering::SeqCst);
            state.user_path.send_replace(Some(path));
        });
    }

    /// The real `claude` the user's terminals would run (waits only for the user's first
    /// `PATH`, at start). None found: the `PATH` is asked for again, for the next time.
    async fn user_claude(self: &Arc<Self>) -> Option<PathBuf> {
        let mut known = self.user_path.subscribe();
        let path = known.wait_for(Option::is_some).await.ok();
        let path = path.and_then(|path| path.clone()).unwrap_or_default();
        let claude = wrapper::real_claude(Some(&path), &self.bin_dir);
        if claude.is_none() {
            self.ask_user_path();
        }
        claude
    }

    fn sent(&self) -> std::sync::MutexGuard<'_, health::Sent> {
        self.sent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn inputs(&self) -> std::sync::MutexGuard<'_, HashMap<u32, mpsc::UnboundedSender<Input>>> {
        self.inputs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Gives the worktrees of the projects in a reply their status (git, so on a blocking
    /// thread), remembered as sent.
    fn with_health(&self, reply: &mut Control) {
        let projects = match reply {
            Control::Projects { projects } => projects.as_mut_slice(),
            Control::ProjectAdded { project }
            | Control::WorktreeCreated { project, .. }
            | Control::WorktreeRemoved { project, .. }
            | Control::WorktreeRenamed { project, .. } => std::slice::from_mut(project),
            _ => return,
        };
        for project in projects {
            health::fill(project);
            let mut sent = self.sent();
            for w in &project.worktrees {
                sent.changed(&w.path, &w.status);
            }
        }
    }

    /// Sends `worktree_status` for every followed worktree (only the one at `only`, when
    /// given) whose status is not the one the app has.
    async fn refresh_health(&self, only: Option<&str>) {
        let changed =
            tokio::task::block_in_place(|| self.health_changed(&self.projects.list(), only));
        for message in changed {
            self.to_app(0, &message).await;
        }
    }

    /// Sends `subagent_worktrees` when the set changed. Called with the agents lock held, so
    /// the changes go out in order.
    async fn owned_changed(&self, agents: &HashMap<String, Agent>) {
        let worktrees = {
            let terminals = self
                .terminal_worktrees
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            states::subagent_worktrees(agents.values(), terminals.values())
        };
        {
            let mut sent = self.owned.lock().unwrap_or_else(PoisonError::into_inner);
            if *sent == worktrees {
                return;
            }
            sent.clone_from(&worktrees);
        }
        self.to_app(0, &Control::SubagentWorktrees { worktrees })
            .await;
    }

    /// The `worktree_status` of every worktree of `projects` (only the one at `only`, when
    /// given) whose status is not the one the app has.
    fn health_changed(&self, projects: &[Project], only: Option<&str>) -> Vec<Control> {
        let mut changed = Vec::new();
        for project in projects {
            let chosen = project.worktrees.iter();
            for w in chosen.filter(|w| only.is_none_or(|path| path == w.path)) {
                let status = health::of(project, w);
                if self.sent().changed(&w.path, &status) {
                    let path = w.path.clone();
                    changed.push(Control::WorktreeStatus { path, status });
                }
            }
        }
        changed
    }

    /// Sends a control message to the app, if one is connected.
    async fn to_app(&self, channel: u32, message: &Control) {
        if let Some(app) = &*self.app.lock().await {
            let _ = app.send(Frame::control(channel, message));
        }
    }

    /// Whether the terminal `channel` is in view in the focused app window.
    fn watches(&self, channel: u32) -> bool {
        self.watched.load(Ordering::Relaxed) == channel
    }

    /// Tracks agents: a `SessionStart` from one of our terminals marks its `claude` as hooked
    /// and detects the agent; any other event of a detected agent (or of its subagents)
    /// updates its state; its own `SessionEnd` removes it. The `SessionStart` that follows a
    /// compaction leaves a known agent as it is (state, subagents, tokens).
    async fn saw(&self, event: &AgentEvent, sent_ns: u64) {
        if event.kind == EventKind::SessionStarted {
            let compacted =
                event.raw.get("source").and_then(serde_json::Value::as_str) == Some("compact");
            if compacted
                && let Some(id) = agent_id(event)
                && self.agents.lock().await.contains_key(&id)
            {
                return;
            }
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
        agent.watched = self.watches(channel);
        if let Some(state) = agent.apply(id, event, sent_ns, Instant::now(), &place) {
            self.to_app(channel, &state).await;
        }
        // The transcript got a message: its usage is read on the next tick (at most once a
        // second).
        agent.usage.due |= matches!(
            event.kind,
            EventKind::ToolFinished { .. } | EventKind::TurnFinished | EventKind::SubagentStopped
        );
        // Claude names a session after its first turn; a rename shows at the end of a turn.
        let turn = matches!(event.kind, EventKind::TurnFinished);
        if event.subagent.is_none() && (agent.title.is_none() || turn) {
            self.retitle(id, agent).await;
        }
        if event.subagent.is_none() && matches!(event.kind, EventKind::SessionEnded { .. }) {
            agents.remove(id);
            let id = id.clone();
            self.to_app(channel, &Control::AgentRemoved { id }).await;
        }
        self.owned_changed(&agents).await;
    }

    /// Places and announces the agent while holding the agents lock, so a `SessionEnd` or the
    /// terminal's exit arriving while git runs waits and removes it afterwards.
    async fn detect(&self, channel: u32, event: &AgentEvent) {
        let mut terminals = self.terminals.lock().await;
        let Some(terminal) = terminals.get_mut(&channel) else {
            return;
        };
        terminal.watch.hooked();
        let claude_dir = terminal.claude_dir.clone();
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
        let mut agent = Agent::new(channel, Instant::now(), crate::hook::now_ms());
        agent.worktree = worktree.clone();
        agent.cwd = cwd.clone();
        agent.transcript = transcript::transcript_path(&event.raw);
        // Read on the next tick: what the transcript already holds is not news (interrupts).
        agent.usage.due = true;
        agent.claude_dir = claude_dir;
        let state = agent.message(&id);
        let detected = Control::AgentDetected {
            id: id.clone(),
            project,
            worktree,
            cwd,
        };
        self.to_app(channel, &detected).await;
        self.to_app(channel, &state).await;
        // A resumed session already has its name.
        self.retitle(&id, &mut agent).await;
        agents.insert(id, agent);
        self.owned_changed(&agents).await;
    }

    /// Reads the agent's session name from its log and sends it when it changed.
    async fn retitle(&self, id: &str, agent: &mut Agent) {
        let Some(cwd) = agent.cwd.clone() else { return };
        let sessions = self.sessions.at(agent.claude_dir.as_deref());
        let title = tokio::task::block_in_place(|| sessions.title(id, &cwd));
        let Some(title) = title.filter(|t| agent.title.as_ref() != Some(t)) else {
            return;
        };
        agent.title = Some(title.clone());
        let message = Control::AgentTitle {
            id: id.to_owned(),
            title,
        };
        self.to_app(agent.channel, &message).await;
    }

    /// Keeps the sessions running in Hive's terminals, in channel order, to resume them when
    /// the app opens again.
    async fn save_open(&self) {
        let agents = self.agents.lock().await;
        let mut open: Vec<(u32, OpenSession)> = agents
            .iter()
            .filter_map(|(id, agent)| {
                let cwd = agent.cwd.clone()?;
                let id = id.clone();
                Some((agent.channel, OpenSession { id, cwd }))
            })
            .collect();
        open.sort_by_key(|(channel, _)| *channel);
        let open: Vec<OpenSession> = open.into_iter().map(|(_, s)| s).collect();
        if let Err(err) = sessions::save_open(&self.restore.file, &open) {
            eprintln!("hive: warning: cannot keep the open sessions: {err}");
        }
    }

    /// The settings, then why the settings file was ignored, if it was.
    async fn send_settings(&self) {
        let (settings, warning) = self.settings.get();
        self.to_app(0, &Control::Settings { settings }).await;
        if let Some(message) = warning {
            self.to_app(0, &Control::SettingsFailed { message }).await;
        }
    }

    /// Sent to a newly connected app right after `Welcome`: the state of every live agent.
    async fn snapshot(&self) {
        let agents = self.agents.lock().await;
        for (id, agent) in agents.iter() {
            self.to_app(agent.channel, &agent.message(id)).await;
            if let Some(title) = agent.title.clone() {
                let id = id.clone();
                self.to_app(agent.channel, &Control::AgentTitle { id, title })
                    .await;
            }
            if let Some(usage) = agent.usage.message(id) {
                self.to_app(agent.channel, &usage).await;
            }
        }
        // The new app has none yet: sent unless still none.
        self.owned
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        self.owned_changed(&agents).await;
    }

    /// Queues input or a resize; ignored once the terminal is gone.
    fn input(&self, channel: u32, input: Input) {
        if let Some(terminal) = self.inputs().get(&channel) {
            let _ = terminal.send(input);
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
        // Its space's environment (6.14) with its GitHub account's token (9.30), and its
        // worktree's `HIVE_*` (6.8), placed before the lock since placing lists worktrees.
        let space = tokio::task::block_in_place(|| self.projects.space_env(cwd));
        let mut env = crate::spaces::vars(&space);
        if space.gh_account.is_some() {
            let gh = self.gh().await;
            match tokio::task::block_in_place(|| gh.vars(&space)) {
                Ok(vars) => env.extend(vars),
                // It still opens, and the human sees it would not act as the space's account.
                Err(message) => {
                    eprintln!("hive: warning: {message}");
                    self.to_app(0, &Control::Notice { message }).await;
                }
            }
        }
        let place = tokio::task::block_in_place(|| projects::place(&self.projects.list(), cwd));
        let worktree = place.as_ref().map(|(_, worktree)| worktree.clone());
        env.extend(tokio::task::block_in_place(|| self.hive_env(place)));
        let claude_dir = space.claude_config_dir;
        let opened = {
            let mut terminals = self.terminals.lock().await;
            match terminals.entry(channel) {
                _ if channel == 0 => Err("terminal channels start at 1".to_owned()),
                Entry::Occupied(_) => Err(format!("terminal {channel} is already open")),
                Entry::Vacant(slot) => {
                    terminal::spawn(channel, cwd, cols, rows, &self.bin_dir, &env).map(
                        |(mut terminal, input, pty, child)| {
                            terminal.claude_dir = claude_dir;
                            let last = terminal.last_output.clone();
                            let session = terminal.session;
                            slot.insert(terminal);
                            self.inputs().insert(channel, input);
                            let state = self.clone();
                            tokio::spawn(pump(state, channel, session, pty, child, output, last));
                        },
                    )
                }
            }
        };
        let reply = match opened {
            Ok(()) => {
                if let Some(worktree) = worktree.clone() {
                    let mut worktrees = self
                        .terminal_worktrees
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner);
                    worktrees.insert(channel, worktree);
                }
                Control::TerminalOpened { worktree }
            }
            Err(message) => Control::Error { message },
        };
        self.to_app(channel, &reply).await;
        // A subagent's worktree the human opened a terminal in shows again.
        let agents = self.agents.lock().await;
        self.owned_changed(&agents).await;
    }

    /// The `HIVE_*` environment (6.8) of a process in the `place` (`projects::place`) of its
    /// cwd: none outside the followed worktrees, and no `HIVE_PORT` when its block cannot be
    /// given.
    fn hive_env(&self, place: Option<(String, String)>) -> Vec<(&'static str, String)> {
        let Some((root, worktree)) = place else {
            return Vec::new();
        };
        let port = self.ports.port(&worktree).inspect_err(|err| {
            eprintln!("hive: warning: no ports for {worktree}: {err}");
        });
        scripts::env(&root, &worktree, port.ok())
    }

    /// Runs the archive script of the project `root`, if it has one, in `worktree` (6.8).
    fn archive(&self, root: &str, worktree: &str) -> io::Result<()> {
        let Some(script) = self.settings.scripts(root).archive else {
            return Ok(());
        };
        let env = self.hive_env(projects::place(&self.projects.list(), worktree));
        scripts::run(&script, Path::new(worktree), &env, scripts::ARCHIVE_TIME)
    }

    /// Answers a project request off the frame loop, since git can take a while.
    fn projects(self: &Arc<Self>, request: impl FnOnce(&Projects) -> Control + Send + 'static) {
        let state = self.clone();
        tokio::spawn(async move {
            // The daemon's runtime is multi-threaded, so other tasks keep running meanwhile.
            let reply = tokio::task::block_in_place(|| {
                let mut reply = request(&state.projects);
                state.with_health(&mut reply);
                reply
            });
            if state.new_to_app(&reply) {
                state.to_app(0, &reply).await;
            }
        });
    }

    /// Whether the app lacks `reply`: always, except a sessions list equal to the last one
    /// sent, which is remembered.
    fn new_to_app(&self, reply: &Control) -> bool {
        !matches!(reply, Control::Sessions { .. })
            || self.listed().replace(reply.clone()).as_ref() != Some(reply)
    }

    /// The sessions list the app has; `None` for a new app, or a reloaded UI (which asks for
    /// the projects again).
    fn listed(&self) -> std::sync::MutexGuard<'_, Option<Control>> {
        self.listed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Answers a request on the current space's projects and their Claude sessions (in the
    /// space's Claude folder) off the frame loop, since reading logs can take a while too.
    /// The request also gets the sessions of the detected agents, read off the frame loop
    /// too: typing never waits for the agents lock (9.13).
    fn sessions(
        self: &Arc<Self>,
        request: impl FnOnce(&[Project], &Sessions, HashSet<String>) -> Control + Send + 'static,
    ) {
        let state = self.clone();
        tokio::spawn(async move {
            let agents = state.agents.lock().await.keys().cloned().collect();
            let asked = state.clone();
            state.projects(move |projects| {
                let (current, claude_dir) = projects.current();
                request(&current, &asked.sessions.at(claude_dir.as_deref()), agents)
            });
        });
    }

    /// Applies a space request (6.14): answers the spaces, or why nothing changed.
    async fn change_spaces(&self, change: impl FnOnce(&mut Spaces) -> Result<(), String>) {
        let changed = tokio::task::block_in_place(|| self.projects.change_spaces(change));
        let reply = match changed {
            Ok(()) => self.projects.spaces_message(),
            Err(message) => Control::SpaceFailed { message },
        };
        self.to_app(0, &reply).await;
    }

    /// `gh` on the user's `PATH` (waits only for the first answer, at start).
    async fn gh(&self) -> crate::gh::Gh {
        let mut known = self.user_path.subscribe();
        let path = known.wait_for(Option::is_some).await.ok();
        crate::gh::Gh::on(path.and_then(|path| path.clone()).unwrap_or_default())
    }

    /// Answers `gh_accounts` for `gh_config_dir` off the frame loop (`gh` asks GitHub), after
    /// making `switch` `gh`'s active account when given (9.30).
    fn gh_accounts(self: &Arc<Self>, gh_config_dir: Option<String>, switch: Option<GhAccount>) {
        let state = self.clone();
        tokio::spawn(async move {
            let gh = state.gh().await;
            let (accounts, problem) = tokio::task::block_in_place(|| {
                match crate::spaces::gh_config_dir(gh_config_dir.clone(), true) {
                    Ok(dir) => gh.answer(dir.as_deref(), switch.as_ref()),
                    Err(problem) => (Vec::new(), Some(problem)),
                }
            });
            let reply = Control::GhAccounts {
                gh_config_dir,
                accounts,
                problem,
            };
            state.to_app(0, &reply).await;
        });
    }

    /// Answers a request of the pull requests view (9.31) off the frame loop: `gh` asks
    /// GitHub.
    fn pulls(self: &Arc<Self>, request: Control) {
        let state = self.clone();
        tokio::spawn(async move {
            let gh = state.gh().await;
            let replies = tokio::task::block_in_place(|| {
                let mut replies = crate::pulls::answer(&gh, &state.projects, &state.pulls, request);
                replies
                    .iter_mut()
                    .for_each(|reply| state.with_health(reply));
                replies
            });
            for reply in replies {
                state.to_app(0, &reply).await;
            }
        });
    }

    /// Stops following the project `id` (9.28), unless a process of Hive's terminals (their
    /// `sessions`) works in it; its settings and its worktrees' port blocks go with it.
    async fn remove_project(&self, id: String, sessions: &HashSet<i32>) {
        let removed = tokio::task::block_in_place(|| {
            self.projects.remove(&id, procs::Source::System, sessions)
        });
        let worktrees = match removed {
            Ok(worktrees) => worktrees,
            Err(err) => {
                let message = err.to_string();
                return self
                    .to_app(0, &Control::RemoveProjectFailed { id, message })
                    .await;
            }
        };
        self.to_app(0, &self.projects.spaces_message()).await;
        let reply = match tokio::task::block_in_place(|| self.settings.forget(&id)) {
            Ok(settings) => settings.map(|settings| Control::Settings { settings }),
            Err(message) => Some(Control::SettingsFailed { message }),
        };
        if let Some(reply) = reply {
            self.to_app(0, &reply).await;
        }
        if let Err(err) = tokio::task::block_in_place(|| self.ports.forget(&worktrees)) {
            eprintln!("hive: warning: cannot free the ports of {id}: {err}");
        }
        self.to_app(0, &Control::ProjectRemoved { id }).await;
    }

    /// Watches `path` for the files panel instead of the worktree watched until now, if any;
    /// its changes against `base`.
    async fn watch_worktree(self: &Arc<Self>, path: Option<(String, DiffBase)>) {
        let mut watching = self.watching.lock().await;
        if let Some(task) = watching.take() {
            task.abort();
        }
        *watching = path.map(|(path, base)| tokio::spawn(watch_files(self.clone(), path, base)));
    }

    /// The one place a change in the watched worktree `path` is reported to the app, after
    /// the debounce: `files` when the listing changed (`None` when it did not), then its
    /// `changes` against `base` every time, since an edit changes the diff but not the list,
    /// then its status if it changed.
    async fn worktree_changed(&self, path: &str, base: DiffBase, listing: Option<&Listing>) {
        if let Some(listing) = listing {
            let files = Control::Files {
                path: path.to_owned(),
                files: listing.files.clone(),
                truncated: listing.truncated,
            };
            self.to_app(0, &files).await;
        }
        // One listing of the projects gives both the changes' base and the status.
        let (changes, statuses) = tokio::task::block_in_place(|| {
            let projects = self.projects.list();
            let changes = changes::answer(&projects, path.to_owned(), base);
            (changes, self.health_changed(&projects, Some(path)))
        });
        self.to_app(0, &changes).await;
        for message in statuses {
            self.to_app(0, &message).await;
        }
    }

    /// Ends the terminal's processes, off the frame loop; its exit is reported by [`pump`].
    fn close(self: &Arc<Self>, channel: u32) {
        let state = self.clone();
        tokio::spawn(async move {
            let terminal = state
                .terminals
                .lock()
                .await
                .get(&channel)
                .map(|t| t.session);
            if let Some(session) = terminal {
                terminal::end_sessions(&[session]).await;
            }
        });
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
        // `/proc` is read on a blocking thread, and not at all while no terminal is open.
        let running = if state.terminals.lock().await.is_empty() {
            HashSet::new()
        } else {
            let list = || watch::claude_sessions(&procs::list(procs::Source::System));
            tokio::task::block_in_place(list)
        };
        let now = Instant::now();
        let mut last_output = HashMap::new();
        let unhooked: Vec<u32> = state
            .terminals
            .lock()
            .await
            .iter_mut()
            .filter_map(|(channel, t)| {
                last_output.insert(*channel, t.last_output.get());
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
        let silence = state.settings.silence();
        for (id, agent) in state.agents.lock().await.iter_mut() {
            let output = last_output.get(&agent.channel);
            agent.watched = state.watches(agent.channel);
            if let Some(message) =
                output.and_then(|&output| agent.reconcile(id, silence, output, now))
            {
                state.to_app(agent.channel, &message).await;
            }
            // While it may be interrupted, its transcript is read every tick for the interrupt.
            if agent.busy() {
                agent.usage.due = true;
            }
            // Its space's Claude projects folder, which its transcript must stay inside.
            let sessions = state.sessions.at(agent.claude_dir.as_deref());
            let (Some(path), Some(root), true) =
                (&agent.transcript, sessions.root(), agent.usage.due)
            else {
                continue;
            };
            // A bounded read (see `transcript::Usage`), off the other tasks' threads.
            let usage = &mut agent.usage;
            let read = || usage.read(id, root, path);
            if let Some(message) = tokio::task::block_in_place(read) {
                state.to_app(agent.channel, &message).await;
            }
            if agent.usage.interrupted()
                && let Some(message) = agent.interrupt(id, now)
            {
                state.to_app(agent.channel, &message).await;
            }
        }
    }
}

/// Sends the worktree statuses that changed, every `interval` (first right away).
async fn watch_health(state: Arc<State>, interval: std::time::Duration) {
    let mut ticks = tokio::time::interval(interval);
    loop {
        ticks.tick().await;
        state.refresh_health(None).await;
    }
}

/// Lists the worktree `path` now and after every change, until aborted or the watch fails.
/// Git and inotify run on a blocking thread, off the frame loop.
async fn watch_files(state: Arc<State>, path: String, base: DiffBase) {
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
                    .worktree_changed(&path, base, changed.then_some(&listing))
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

/// Time the output still gets once the shell exited while another process keeps the PTY
/// open (9.15).
const DRAIN: Duration = Duration::from_millis(200);

/// Copies PTY output to the app until the shell exits, then reports the exit. The copy takes
/// no lock (9.13); only the exit does. A disowned job or a `setsid` child can keep the PTY
/// open after the shell exits (risk 9): the output gets [`DRAIN`], then the terminal's other
/// process groups end as when its tab closes (#18).
async fn pump(
    state: Arc<State>,
    channel: u32,
    session: i32,
    mut pty: OwnedReadPty,
    mut child: Child,
    output: mpsc::Sender<Frame>,
    last_output: terminal::LastOutput,
) {
    let copy = copy(&mut pty, channel, &output, &last_output);
    tokio::pin!(copy);
    let status = tokio::select! {
        () = &mut copy => child.wait().await,
        status = child.wait() => {
            let _ = tokio::time::timeout(DRAIN, &mut copy).await;
            status
        }
    };
    // Whether or not they still hold the PTY (macOS revokes it when the shell exits).
    terminal::end_sessions(&[session]).await;
    let code = status.ok().and_then(|status| status.code());
    {
        let mut terminals = state.terminals.lock().await;
        terminals.remove(&channel);
        state.inputs().remove(&channel);
        state
            .terminal_worktrees
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&channel);
        let mut agents = state.agents.lock().await;
        for (id, _) in agents.extract_if(|_, a| a.channel == channel) {
            state.to_app(channel, &Control::AgentRemoved { id }).await;
        }
        state.owned_changed(&agents).await;
    }
    state
        .to_app(channel, &Control::TerminalExited { code })
        .await;
}

/// Copies PTY output to the app until the PTY closes or the app is gone.
async fn copy(
    pty: &mut OwnedReadPty,
    channel: u32,
    output: &mpsc::Sender<Frame>,
    last_output: &terminal::LastOutput,
) {
    let mut buf = vec![0; 64 * 1024];
    while let Ok(n @ 1..) = pty.read(&mut buf).await {
        last_output.touch();
        let frame = Frame::terminal(channel, Bytes::copy_from_slice(&buf[..n]));
        if output.send(frame).await.is_err() {
            break;
        }
    }
}

/// [`projects::held`] for a folder about to be renamed or moved, with this machine's processes.
fn held(projects: &Projects) -> impl Fn(&Path) -> io::Result<()> {
    move |folder| projects::held(&projects.list(), folder, procs::Source::System)
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

/// A hook connection carries exactly one event (or one `hive badge`); the connection is
/// closed after it.
async fn hook_connection<R: AsyncRead + Unpin>(
    mut reader: FramedRead<R, FrameCodec>,
    state: &Arc<State>,
) {
    let Some(Ok(frame)) = reader.next().await else {
        return;
    };
    match frame.to_control() {
        Ok(Control::Badge { text }) => {
            let channel = frame.channel;
            // Held while sending, so a badge cannot follow the terminal's `terminal_exited`.
            let terminals = state.terminals.lock().await;
            if terminals.contains_key(&channel) {
                let text = adapter::clip(&text, MAX_BADGE);
                state.to_app(channel, &Control::Badge { text }).await;
            }
        }
        Ok(Control::Hook {
            event,
            terminal_id,
            payload,
            sent_ns,
        }) => {
            // The payload stays here: the app gets only what the service makes of it.
            let event = ClaudeCode.translate(&event, terminal_id, payload);
            state.saw(&event, sent_ns).await;
            if let EventKind::WorktreeCreated { .. } | EventKind::WorktreeRemoved { .. } =
                event.kind
            {
                // The app's worktrees follow a `claude -w` or a subagent's worktree; listed
                // after the agent states it changed.
                state.projects(|projects| Control::Projects {
                    projects: projects.list(),
                });
            }
        }
        _ => {}
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
    *state.listed() = None;
    state.send_settings().await;
    state.snapshot().await;
    // Only the first app after a restart resumes the sessions the last one left.
    let restore = std::mem::take(&mut *state.restore.pending.lock().await);
    if !restore.is_empty() {
        let sessions = restore;
        state
            .to_app(0, &Control::RestoreSessions { sessions })
            .await;
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
        FrameType::Terminal => return state.input(channel, Input::Data(frame.payload)),
        FrameType::Control => frame.to_control(),
    };
    match message {
        Ok(Control::OpenTerminal { cwd, cols, rows }) => {
            state.open(channel, &cwd, cols, rows, output.clone()).await;
        }
        Ok(Control::Resize { cols, rows }) => state.input(channel, Input::Resize { cols, rows }),
        Ok(Control::CloseTerminal) => state.close(channel),
        Ok(Control::WatchWorktree { path, base }) => state.watch_worktree(Some((path, base))).await,
        Ok(Control::UnwatchWorktree) => state.watch_worktree(None).await,
        Ok(Control::View { terminal, focused }) => {
            let watched = terminal.filter(|_| focused).unwrap_or(0);
            state.watched.store(watched, Ordering::Relaxed);
        }
        Ok(Control::GetSettings) => state.send_settings().await,
        Ok(Control::SetSettings { settings }) => {
            let reply = match tokio::task::block_in_place(|| state.settings.set(settings)) {
                Ok(settings) => Control::Settings { settings },
                Err(message) => Control::SettingsFailed { message },
            };
            state.to_app(0, &reply).await;
        }
        Ok(Control::OpenSettingsFile) => {
            let located = tokio::task::block_in_place(|| {
                file::windows(state.settings.ensure_file()?, OsStr::new("wslpath"))
            });
            let target = Control::EditorTarget {
                worktree: String::new(),
                path: String::new(),
                error: located.as_ref().err().map(ToString::to_string),
                windows_path: located.ok(),
            };
            state.to_app(0, &target).await;
        }
        Ok(Control::GetDiagnostics) => {
            let claude = state.user_claude().await;
            let diagnostics = Control::Diagnostics {
                settings_file: state.settings.file().display().to_string(),
                wrapper: state.bin_dir.join("claude").display().to_string(),
                claude: claude.map(|c| c.display().to_string()),
            };
            state.to_app(0, &diagnostics).await;
        }
        Ok(Control::ListProjects) => {
            *state.listed() = None;
            state.to_app(0, &state.projects.spaces_message()).await;
            state.projects(|projects| Control::Projects {
                projects: projects.list(),
            })
        }
        Ok(Control::AddProject { path }) => {
            let state = state.clone();
            tokio::spawn(async move {
                let added = tokio::task::block_in_place(|| {
                    let project = state.projects.add(&path)?;
                    let mut reply = Control::ProjectAdded { project };
                    state.with_health(&mut reply);
                    Ok(reply)
                });
                let reply = match added {
                    Ok(reply) => {
                        // It joined the current space.
                        state.to_app(0, &state.projects.spaces_message()).await;
                        reply
                    }
                    Err((error, message)) => Control::AddProjectFailed {
                        path,
                        error,
                        message,
                    },
                };
                state.to_app(0, &reply).await;
            });
        }
        Ok(Control::RemoveProject { id }) => {
            let state = state.clone();
            tokio::spawn(async move {
                let terminals = state.terminals.lock().await;
                let sessions: HashSet<i32> = terminals.values().map(|t| t.session).collect();
                drop(terminals);
                state.remove_project(id, &sessions).await
            });
        }
        Ok(Control::CreateSpace { name, env }) => {
            state.change_spaces(|s| s.create(&name, env)).await
        }
        Ok(Control::UpdateSpace { id, name, env }) => {
            state.change_spaces(|s| s.update(&id, &name, env)).await
        }
        Ok(Control::DeleteSpace { id }) => state.change_spaces(|s| s.delete(&id)).await,
        Ok(Control::SelectSpace { id }) => state.change_spaces(|s| s.select(&id)).await,
        Ok(Control::ListGhAccounts { gh_config_dir }) => state.gh_accounts(gh_config_dir, None),
        Ok(Control::SwitchGhAccount {
            gh_config_dir,
            account,
        }) => state.gh_accounts(gh_config_dir, Some(account)),
        Ok(
            request @ (Control::ListPulls { .. }
            | Control::OpenPull { .. }
            | Control::ActOnPull { .. }
            | Control::CreatePull { .. }),
        ) => state.pulls(request),
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
        Ok(Control::RemoveWorktree { path, force }) => {
            let archiving = state.clone();
            state.projects(move |projects| {
                let archive = |root: &str| archiving.archive(root, &path);
                match projects.remove_worktree(&path, force, procs::Source::System, archive) {
                    Ok(project) => Control::WorktreeRemoved { project, path },
                    Err(err) => Control::RemoveWorktreeFailed {
                        path,
                        message: err.to_string(),
                    },
                }
            })
        }
        Ok(Control::RenameWorktree { path, name }) => {
            state.projects(move |projects| {
                match projects.rename_worktree(&path, &name, procs::Source::System) {
                    Ok((project, to)) => Control::WorktreeRenamed {
                        project,
                        from: path,
                        path: to,
                    },
                    Err(err) => Control::RenameWorktreeFailed {
                        path,
                        name,
                        message: err.to_string(),
                    },
                }
            })
        }
        Ok(Control::ListChanges { path, base }) => {
            state.projects(move |projects| changes::answer(&projects.list(), path, base))
        }
        Ok(Control::ListSessions) => {
            // Hive's terminals: their hooks name their sessions.
            state.sessions(move |projects, sessions, mut running| {
                // Claude keeps a record of each running `claude` beside its projects folder.
                let records = sessions
                    .root()
                    .and_then(Path::parent)
                    .map(|d| d.join("sessions"));
                running.extend(procs::claude_sessions(
                    procs::Source::System,
                    records.as_deref(),
                ));
                let ((sessions, truncated), error) = match sessions.list(projects, &running) {
                    Ok(listed) => (listed, None),
                    Err(err) => ((Vec::new(), false), Some(err.to_string())),
                };
                Control::Sessions {
                    sessions,
                    error,
                    truncated,
                }
            })
        }
        Ok(Control::LocateSession { id, target }) => {
            state.sessions(move |projects, sessions, _| {
                let located = sessions.find(projects, &id).and_then(|session| {
                    let path = match target {
                        SessionTarget::Log => session.log,
                        SessionTarget::Folder => session.cwd,
                    };
                    file::windows(Path::new(&path), OsStr::new("wslpath"))
                });
                Control::SessionLocated {
                    id,
                    target,
                    error: located.as_ref().err().map(ToString::to_string),
                    windows_path: located.ok(),
                }
            })
        }
        Ok(Control::DeleteSession { id }) => {
            state.sessions(move |projects, sessions, running| {
                // A running session keeps writing its log.
                let deleted = if running.contains(&id) {
                    Err(io::Error::other("the session is running: end it first"))
                } else {
                    sessions.delete(projects, &id)
                };
                match deleted {
                    Ok(()) => Control::SessionDeleted { id },
                    Err(err) => Control::DeleteSessionFailed {
                        id,
                        message: err.to_string(),
                    },
                }
            })
        }
        Ok(Control::ListDirs { path, windows }) => state.projects(move |_| {
            let home = std::env::var_os("HOME").map(PathBuf::from);
            dirs::answer(path, windows, home.as_deref(), &dirs::WINDOWS)
        }),
        Ok(Control::SearchFiles { worktree, query }) => state.projects(move |projects| {
            let found = projects
                .worktree(&worktree)
                .and_then(|dir| search::search(&dir, &query));
            let (matches, truncated, error) = match found {
                Ok((matches, truncated)) => (matches, truncated, None),
                Err(err) => (Vec::new(), false, Some(err.to_string())),
            };
            Control::SearchResults {
                worktree,
                query,
                matches,
                truncated,
                error,
            }
        }),
        Ok(Control::OpenFile {
            worktree,
            path,
            base,
        }) => state.projects(move |projects| {
            let read = changes::against(&projects.list(), &worktree, base)
                .and_then(|against| file::read(&against.dir, &path, against.commit.as_deref()));
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
        Ok(Control::CreateFile {
            worktree,
            folder,
            name,
        }) => state.projects(move |projects| {
            let created = projects
                .worktree(&worktree)
                .and_then(|dir| file::create(&dir, &folder, &name));
            match created {
                Ok(path) => Control::FileCreated { worktree, path },
                Err(err) => Control::FileOpFailed {
                    worktree,
                    message: err.to_string(),
                },
            }
        }),
        Ok(Control::RenameFile {
            worktree,
            path,
            name,
        }) => state.projects(move |projects| {
            let renamed = projects
                .worktree(&worktree)
                .and_then(|dir| file::rename(&dir, &path, &name, &held(projects)));
            match renamed {
                Ok(to) => Control::FileRenamed { worktree, path, to },
                Err(err) => Control::FileOpFailed {
                    worktree,
                    message: err.to_string(),
                },
            }
        }),
        Ok(Control::MoveFile {
            worktree,
            path,
            folder,
        }) => state.projects(move |projects| {
            let moved = projects
                .worktree(&worktree)
                .and_then(|dir| file::move_to(&dir, &path, &folder, &held(projects)));
            match moved {
                Ok(to) => Control::FileRenamed { worktree, path, to },
                Err(err) => Control::FileOpFailed {
                    worktree,
                    message: err.to_string(),
                },
            }
        }),
        Ok(Control::DeleteFile { worktree, path }) => state.projects(move |projects| {
            let deleted = projects
                .worktree(&worktree)
                .and_then(|dir| file::delete(&dir, &path, &held(projects)));
            match deleted {
                Ok(()) => Control::FileDeleted { worktree, path },
                Err(err) => Control::FileOpFailed {
                    worktree,
                    message: err.to_string(),
                },
            }
        }),
        Ok(Control::CreateFolder {
            worktree,
            folder,
            name,
        }) => state.projects(move |projects| {
            let created = projects
                .worktree(&worktree)
                .and_then(|dir| file::create_folder(&dir, &folder, &name));
            match created {
                Ok(path) => Control::FolderCreated { worktree, path },
                Err(err) => Control::FileOpFailed {
                    worktree,
                    message: err.to_string(),
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

    fn test_state(dir: &Path) -> Arc<State> {
        let restore = Restore {
            file: dir.join("open-sessions.json"),
            pending: Mutex::new(Vec::new()),
        };
        let projects = Projects::load(dir.join("spaces.json"), &dir.join("projects.json"));
        Arc::new(State::new(
            dir.into(),
            projects,
            Sessions::new(None),
            settings::Store::load(dir.join("settings.json")),
            Ports::new(dir.join("ports.json")),
            restore,
        ))
    }

    #[test]
    fn an_unchanged_sessions_list_is_not_sent_again() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(dir.path());
        let list = |error: Option<&str>| Control::Sessions {
            sessions: vec![],
            error: error.map(Into::into),
            truncated: false,
        };
        assert!(state.new_to_app(&list(None)));
        assert!(!state.new_to_app(&list(None)));
        assert!(state.new_to_app(&list(Some("x"))));
        // Anything else always goes.
        assert!(state.new_to_app(&Control::ListSessions));
        assert!(state.new_to_app(&Control::ListSessions));
        assert!(!state.new_to_app(&list(Some("x"))));
        // A new app, or a reloaded UI, has none.
        *state.listed() = None;
        assert!(state.new_to_app(&list(Some("x"))));
    }

    #[test]
    fn a_path_ask_already_running_is_not_started_again() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(dir.path());
        state.asking_path.store(true, Ordering::SeqCst);
        // Without a runtime, starting another ask would panic.
        state.ask_user_path();
        assert!(state.asking_path.load(Ordering::SeqCst));
        assert_eq!(*state.user_path.borrow(), None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn changed_worktree_statuses_are_sent_on_every_tick() {
        // Through a real daemon this would take the 30 s interval.
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap().join("r");
        std::fs::create_dir(&root).unwrap();
        let git = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(&root)
                .args(["-c", "user.name=t", "-c", "user.email=t@t"])
                .args(args)
                .env("GIT_CONFIG_GLOBAL", "/dev/null")
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?}");
        };
        git(&["init", "-q", "-b", "main"]);
        git(&["commit", "-q", "--allow-empty", "-m", "a"]);
        let state = test_state(dir.path());
        let path = root.display().to_string();
        state.projects.add(&path).unwrap();
        let (app, mut sent) = mpsc::unbounded_channel();
        *state.app.lock().await = Some(app);
        let ticking = tokio::spawn(watch_health(
            state.clone(),
            std::time::Duration::from_millis(50),
        ));
        let next = async |sent: &mut mpsc::UnboundedReceiver<Frame>| {
            let frame = tokio::time::timeout(std::time::Duration::from_secs(10), sent.recv());
            let control = frame
                .await
                .expect("no status")
                .unwrap()
                .to_control()
                .unwrap();
            let json = serde_json::to_value(control).unwrap();
            assert_eq!(
                (&json["type"], &json["path"]),
                (&"worktree_status".into(), &path.as_str().into())
            );
            json["status"]["changes"].as_u64().unwrap()
        };
        // Sent when first seen, then only when it changed.
        assert_eq!(next(&mut sent).await, 0);
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert!(sent.try_recv().is_err(), "sent again unchanged");
        std::fs::write(root.join("new"), "").unwrap();
        assert_eq!(next(&mut sent).await, 1);
        std::fs::remove_file(root.join("new")).unwrap();
        assert_eq!(next(&mut sent).await, 0);
        ticking.abort();
    }

    #[tokio::test]
    async fn a_connecting_app_gets_the_state_of_every_live_agent() {
        // Agents outlive an app connection only in principle (the service exits with the
        // app), so the snapshot is checked here rather than through a real daemon.
        let dir = tempfile::tempdir().unwrap();
        let mut named = Agent::new(4, Instant::now(), 0);
        named.title = Some("Named".into());
        let log = dir.path().join("s.jsonl");
        let turn =
            r#"{"type":"assistant","message":{"usage":{"input_tokens":7,"output_tokens":2}}}"#;
        std::fs::write(&log, format!("{turn}\n")).unwrap();
        let usage = Control::AgentUsage {
            id: "s".into(),
            context_tokens: 7,
            context_limit: 200_000,
            output_tokens: 2,
        };
        let read = named.usage.read("s", dir.path(), &log);
        assert_eq!(read, Some(usage.clone()));
        // A subagent working in a worktree of its own.
        let mut owning = Agent::new(5, Instant::now(), 0);
        owning.worktree = Some("/r".into());
        let start = AgentEvent {
            provider: "claude-code".into(),
            terminal_id: Some("5".into()),
            session_id: Some("u".into()),
            subagent: Some(hive_protocol::Subagent {
                id: "a".into(),
                agent_type: None,
            }),
            cwd: Some("/r/w".into()),
            kind: EventKind::SubagentStarted,
            activity: None,
            raw: serde_json::Value::Null,
        };
        owning.apply("u", &start, 0, Instant::now(), &|cwd| Some(cwd.to_owned()));
        let owning_state = owning.message("u");
        let state = test_state(dir.path());
        *state.agents.lock().await =
            HashMap::from([("s".to_owned(), named), ("u".to_owned(), owning)]);
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
        let mut got = Vec::new();
        for _ in 0..6 {
            let next = tokio::time::timeout(std::time::Duration::from_secs(5), frames.next());
            let frame = next.await.expect("no snapshot").unwrap().unwrap();
            got.push((frame.channel, frame.to_control().unwrap()));
        }
        let idle = |id: &str| Control::AgentState {
            id: id.into(),
            state: hive_protocol::AgentState::Idle,
            urgency: 1,
            pending: false,
            interrupted: false,
            alert: None,
            writing: false,
            subagents: vec![],
            activity: None,
            since_ms: 0,
        };
        // Each agent's state; the named one's name too, and only after its state.
        let named = Control::AgentTitle {
            id: "s".into(),
            title: "Named".into(),
        };
        let at = |message: &Control| got.iter().position(|(_, m)| m == message);
        // The settings come first.
        let settings = Control::Settings {
            settings: Default::default(),
        };
        assert_eq!(got[0], (0, settings));
        assert!(got.contains(&(4, idle("s"))), "{got:?}");
        assert!(got.contains(&(5, owning_state)), "{got:?}");
        // Then the worktrees subagents own.
        let owned = Control::SubagentWorktrees {
            worktrees: vec!["/r/w".into()],
        };
        assert_eq!(got.last(), Some(&(0, owned)));
        assert!(at(&named) > at(&idle("s")), "{got:?}");
        assert_eq!(got[at(&named).unwrap()].0, 4);
        // Its usage too, once known.
        assert!(at(&usage) > at(&idle("s")), "{got:?}");
        assert_eq!(got[at(&usage).unwrap()].0, 4);
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
