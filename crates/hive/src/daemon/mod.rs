//! `hive daemon`: the service. Lives exactly as long as the app connection.
//!
//! Split by job (9.20): this module holds the service's `State`, its start and end, and
//! the projects, health and files-panel work; `app` serves the connections (the app's
//! frames and hook calls) and writes to the app; `terminals` runs the PTYs; `agents`
//! follows the agents detected in them.

mod agents;
mod app;
mod terminals;
mod usage;

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs::{File, TryLockError};
use std::io;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use hive_protocol::{Control, DiffBase, Frame, GhAccount, OpenSession, Project, Worktree};
#[cfg(unix)]
use tokio::net::{UnixListener as Listener, UnixStream};
#[cfg(unix)]
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::{Mutex, mpsc};

#[cfg(windows)]
use crate::windows::{Listener, accept};

use crate::files::{Listing, Watcher};
use crate::paths::Paths;
use crate::projects::{self, Projects};
use crate::registry::Registry;
use crate::scripts::{self, Ports};
use crate::sessions::{self, Sessions};
use crate::settings;
use crate::spaces::Spaces;
use crate::states::Agent;
use crate::terminal::{self, Input, Terminal};
use crate::{bridge, changes, health, hook, procs, wrapper};
use app::connection;
use terminals::watch_terminals;

/// How often a service waiting for the lock tries it again.
const LOCK_RETRY: Duration = Duration::from_millis(20);

pub async fn run(paths: &Paths) -> io::Result<()> {
    // Started by `hive bridge`: leave its session, so the service outlives nothing but the
    // app connection. Fails harmlessly for a group leader (e.g. started from a shell).
    // (On Windows the bridge starts it detached.)
    #[cfg(unix)]
    let _ = nix::unistd::setsid();
    paths.prepare_runtime()?;
    let _lock = lock(paths).await?;
    wrapper::install(paths, &std::env::current_exe()?)?;
    // Handle SIGTERM before anyone can connect, so an early one still cleans up.
    #[cfg(unix)]
    let mut terminate = signal(SignalKind::terminate())?;
    #[cfg(unix)]
    let terminated = async move {
        terminate.recv().await;
    };
    // Nothing signals a service started detached: it ends with the app connection.
    #[cfg(windows)]
    let terminated = std::future::pending();
    // On Windows this file names the service's pipe: removed too, once the service ends.
    let socket = paths.socket();
    #[cfg(unix)]
    let listener = {
        // A socket left by a crashed daemon; the lock proves nobody is serving it.
        let _ = std::fs::remove_file(&socket);
        let listener = Listener::bind(&socket)?;
        std::fs::set_permissions(&socket, std::fs::Permissions::from_mode(0o600))?;
        listener
    };
    // A new pipe, named in the runtime folder.
    #[cfg(windows)]
    let listener = Listener::bind(paths)?;
    let projects = Projects::load(paths.spaces(), &paths.projects());
    let sessions = Sessions::new(sessions::root(|key| std::env::var_os(key)));
    let settings = settings::Store::load(paths.settings());
    projects.migrate_accounts(&mut |legacy| settings.migrate(legacy));
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
    let result = serve(listener, &socket, terminated, state).await;
    let _ = std::fs::remove_file(&socket);
    result
}

/// Single-instance guard: an exclusive lock held for the daemon's whole life. A service that
/// is still ending holds it for up to its grace period, so the lock is tried again for as long
/// as the bridge waits for a new service.
async fn lock(paths: &Paths) -> io::Result<File> {
    let file = crate::mode::private(File::options().create(true).truncate(false).write(true))
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

/// The next connection to the service.
#[cfg(unix)]
async fn accept(listener: &mut Listener) -> io::Result<UnixStream> {
    listener.accept().await.map(|(stream, _)| stream)
}

async fn serve(
    mut listener: Listener,
    socket: &Path,
    terminated: impl Future<Output = ()>,
    state: Arc<State>,
) -> io::Result<()> {
    let (app_gone, mut app_gone_rx) = mpsc::channel::<()>(1);
    let watcher = tokio::spawn(watch_terminals(state.clone()));
    let health = tokio::spawn(watch_health(state.clone(), health::INTERVAL));
    let registry = tokio::spawn(watch_registry(state.clone(), Registry::new()));
    tokio::pin!(terminated);
    loop {
        tokio::select! {
            accepted = accept(&mut listener) => {
                tokio::spawn(connection(accepted?, state.clone(), app_gone.clone()));
            }
            _ = app_gone_rx.recv() => break,
            () = &mut terminated => break,
        }
    }
    // Released before the terminals end: an app started again meanwhile gets a new service
    // (waiting for the lock) instead of a listen queue nobody accepts.
    drop(listener);
    let _ = std::fs::remove_file(socket);
    let files = state.watching.lock().await.take().map(|files| files.task);
    for task in [watcher, health, registry].into_iter().chain(files) {
        stop(task).await;
    }
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

/// Stops a background task and waits until it has. One caught in blocking work
/// (`block_in_place`) finishes it and polls its next timer while the runtime still runs:
/// polling a timer once the runtime shuts down panics.
async fn stop(task: tokio::task::JoinHandle<()>) {
    task.abort();
    let _ = task.await;
}

/// The worktree of the app's files panel: the task watching it, and the ignored folders open
/// in its tree (14.2), which that task follows.
struct Watching {
    path: String,
    task: tokio::task::JoinHandle<()>,
    open: tokio::sync::watch::Sender<Vec<String>>,
}

/// The service's state. Lock order, when one task holds several: `terminals` → `agents` →
/// `app`, and `usage` → `app`. `agents` is held across slow work (placing an agent runs git, reading a session
/// log or a transcript), so a terminal's input and output never take an async lock (9.13):
/// they go through `inputs` and [`terminal::LastOutput`]. The std locks (`inputs`, `sent`)
/// are never held across an await.
struct State {
    /// Control frames to the app connection's writer, while an app is connected.
    app: Mutex<Option<mpsc::UnboundedSender<Frame>>>,
    /// Open terminals by channel. The channel number is also the `HIVE_TERMINAL_ID`.
    terminals: Mutex<HashMap<u32, Terminal>>,
    /// Each open terminal's input queue and output (for the app's acknowledgements), by channel.
    inputs: std::sync::Mutex<HashMap<u32, (mpsc::UnboundedSender<Input>, terminal::Output)>>,
    /// The worktree each open terminal opened in, by channel, when it is one: a subagent's
    /// own worktree with a terminal of the human's is not only the subagent's (9.35).
    terminal_worktrees: std::sync::Mutex<HashMap<u32, String>>,
    /// Detected agents by session id, with their terminal and state; followed by [`agents`].
    agents: Mutex<HashMap<String, Agent>>,
    /// The worktree of the app's files panel, while one is watched.
    watching: Mutex<Option<Watching>>,
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
    /// Each project's worktrees as last sent to the app (without status), so a change of
    /// git's registry the app already has (its own request, a hook) is not sent again.
    worktrees_sent: std::sync::Mutex<HashMap<String, Vec<Worktree>>>,
    /// Woken when the current space's projects change, so their registries are watched.
    refollow: tokio::sync::Notify,
    /// Held while [`State::worktrees_changed`] lists and sends.
    changing: Mutex<()>,
    /// The user's `PATH` ([`wrapper::user_path`]), asked for when first needed (diagnostics,
    /// `gh`), never at start (9.20), and again, in the background, while no `claude` is on it.
    user_path: tokio::sync::watch::Sender<Option<OsString>>,
    /// Whether the user's `PATH` is being asked for.
    asking_path: std::sync::atomic::AtomicBool,
    /// The pull requests lists fetched (9.31).
    pulls: std::sync::Mutex<crate::pulls::Cache>,
    /// The Actions runs lists fetched (9.32).
    runs: std::sync::Mutex<crate::pulls::Cache>,
    /// The 5-hour usage windows `hive statusline` reported (12.1), and the one the app has.
    usage: Mutex<usage::Usage>,
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
            worktrees_sent: Default::default(),
            refollow: Default::default(),
            changing: Mutex::new(()),
            user_path: tokio::sync::watch::Sender::new(None),
            asking_path: Default::default(),
            pulls: Default::default(),
            runs: Default::default(),
            usage: Default::default(),
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
            #[cfg(unix)]
            let shell = Some(wrapper::path_shell());
            // No shell config changes it on Windows: the service's is the user's.
            #[cfg(windows)]
            let shell = None;
            let (home, timeout) = (var(crate::dirs::HOME), wrapper::SHELL_TIMEOUT);
            let path = wrapper::user_path(shell, var("PATH"), home, timeout).await;
            // Done before the answer wakes anyone waiting for it.
            state.asking_path.store(false, Ordering::SeqCst);
            state.user_path.send_replace(Some(path));
        });
    }

    /// The user's `PATH`: asked for the first time it is needed, then the last answer.
    async fn users_path(self: &Arc<Self>) -> OsString {
        let mut known = self.user_path.subscribe();
        let unknown = known.borrow().is_none();
        if unknown {
            self.ask_user_path();
        }
        let path = known.wait_for(Option::is_some).await.ok();
        path.and_then(|path| path.clone()).unwrap_or_default()
    }

    /// The real `claude` the user's terminals would run (waits only for the user's first
    /// `PATH`). None found: the `PATH` is asked for again, for the next time.
    async fn user_claude(self: &Arc<Self>) -> Option<PathBuf> {
        let path = self.users_path().await;
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

    fn inputs(
        &self,
    ) -> std::sync::MutexGuard<'_, HashMap<u32, (mpsc::UnboundedSender<Input>, terminal::Output)>>
    {
        self.inputs
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn worktrees_sent(&self) -> std::sync::MutexGuard<'_, HashMap<String, Vec<Worktree>>> {
        self.worktrees_sent
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Whether the app has `projects`, no others (a group's repository may leave, 14.1), with
    /// these worktrees.
    fn known(&self, projects: &[Project]) -> bool {
        let sent = self.worktrees_sent();
        sent.len() == projects.len()
            && (projects.iter()).all(|p| sent.get(&p.id) == Some(&p.worktrees))
    }

    /// The one place the followed projects' worktrees are known to have changed outside
    /// Hive's own requests: a worktree hook, or git's registry (9.36). The app gets the
    /// projects again, with health, unless they are what it has (both saw the same change).
    async fn worktrees_changed(&self) {
        // One change at a time, so the second of two at once sees what the first sent.
        let _turn = self.changing.lock().await;
        let reply = tokio::task::block_in_place(|| {
            self.projects.forget();
            let projects = self.projects.list();
            if self.known(&projects) {
                return None;
            }
            let mut reply = Control::Projects { projects };
            self.with_health(&mut reply);
            Some(reply)
        });
        if let Some(reply) = reply {
            self.to_app(0, &reply).await;
        }
    }

    /// Gives the worktrees of the projects in a reply their status (git, so on a blocking
    /// thread), remembered as sent.
    fn with_health(&self, reply: &mut Control) {
        let projects = match reply {
            Control::Projects { projects } => {
                // Every project the app has.
                self.worktrees_sent().clear();
                projects.as_mut_slice()
            }
            Control::ProjectAdded { project }
            | Control::WorktreeCreated { project, .. }
            | Control::WorktreeRemoved { project, .. }
            | Control::WorktreeRenamed { project, .. } => std::slice::from_mut(project),
            _ => return,
        };
        for project in projects {
            self.worktrees_sent()
                .insert(project.id.clone(), project.worktrees.clone());
            health::fill(project);
            let mut sent = self.sent();
            for w in &project.worktrees {
                sent.changed(&w.path, &w.status);
            }
        }
    }

    /// Sends `worktree_status` for every followed worktree whose status is not the one the
    /// app has.
    async fn refresh_health(&self) {
        let changed =
            tokio::task::block_in_place(|| self.health_changed(&self.projects.list(), None));
        for message in changed {
            self.to_app(0, &message).await;
        }
    }

    /// The `worktree_status` of every worktree of `projects` (only the one at `only`, when
    /// given, with its changed files when counted) whose status is not the one the app has.
    fn health_changed(
        &self,
        projects: &[Project],
        only: Option<(&str, Option<changes::Totals>)>,
    ) -> Vec<Control> {
        let mut changed = Vec::new();
        for project in projects {
            let chosen = project.worktrees.iter();
            for w in chosen.filter(|w| only.is_none_or(|(path, _)| path == w.path)) {
                let status = health::of(project, w, only.and_then(|(_, totals)| totals));
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

    /// The settings, then why the settings file was ignored, if it was.
    async fn send_settings(&self) {
        let (settings, warning) = self.settings.get();
        self.to_app(0, &Control::Settings { settings }).await;
        if let Some(message) = warning {
            self.to_app(0, &Control::SettingsFailed { message }).await;
        }
    }

    /// The `HIVE_*` environment (6.8) of a process in the `place` (`projects::place`) of its
    /// cwd: none outside the followed worktrees, nor in a group's folder (its scripts and ports
    /// are not asked for, 14.1), and no `HIVE_PORT` when its block cannot be given.
    fn hive_env(&self, place: Option<(String, String)>) -> Vec<(&'static str, String)> {
        let Some((root, worktree)) = place.filter(|(root, _)| !self.projects.is_group(root)) else {
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
        let shell = self.settings.get().0.terminal.shell;
        let (dir, time) = (Path::new(worktree), scripts::ARCHIVE_TIME);
        scripts::run(&script, dir, &env, time, shell)
    }

    /// Answers a project request off the frame loop, since git can take a while.
    fn projects(self: &Arc<Self>, request: impl FnOnce(&Projects) -> Control + Send + 'static) {
        let state = self.clone();
        tokio::spawn(async move { state.answer(request).await });
    }

    /// [`State::projects`] for a request that changes worktrees, or lists them afresh: in turn
    /// with [`State::worktrees_changed`], which then finds the change already sent (and never
    /// sends a list older than this answer after it).
    fn change_worktrees(
        self: &Arc<Self>,
        request: impl FnOnce(&Projects) -> Control + Send + 'static,
    ) {
        let state = self.clone();
        tokio::spawn(async move {
            let _turn = state.changing.lock().await;
            state.answer(request).await;
        });
    }

    async fn answer(&self, request: impl FnOnce(&Projects) -> Control) {
        // The daemon's runtime is multi-threaded, so other tasks keep running meanwhile.
        let reply = tokio::task::block_in_place(|| {
            let mut reply = request(&self.projects);
            self.with_health(&mut reply);
            reply
        });
        if self.new_to_app(&reply) {
            self.to_app(0, &reply).await;
        }
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

    /// Answers a request on the current space's projects and their Claude sessions (in every
    /// account's Claude folder, 12.2) off the frame loop, since reading logs can take a while too.
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
                request(&projects.current(), &asked.accounts_sessions(None), agents)
            });
        });
    }

    /// The sessions of `first`'s Claude config folder (a terminal's; `None`: the default
    /// account's), then of the default account and every other account (12.2).
    fn accounts_sessions(&self, first: Option<&str>) -> Sessions {
        let dirs = self.settings.account_dirs();
        let others = dirs.iter().map(|dir| Some(dir.as_str()));
        let dirs: Vec<Option<&str>> = [first, None].into_iter().chain(others).collect();
        self.sessions.at(&dirs)
    }

    /// Applies a space request (6.14): answers the spaces, or why nothing changed.
    async fn change_spaces(&self, change: impl FnOnce(&mut Spaces) -> Result<(), String>) {
        let changed = tokio::task::block_in_place(|| self.projects.change_spaces(change));
        let reply = match changed {
            Ok(()) => {
                // A space switch changes the projects followed.
                self.refollow.notify_one();
                self.projects.spaces_message()
            }
            Err(message) => Control::SpaceFailed { message },
        };
        self.to_app(0, &reply).await;
    }

    /// The Claude config folder whose usage the status bar shows (12.1): the selected
    /// account's (12.2), else the one the service's terminals get (the default account).
    fn account(&self) -> Option<PathBuf> {
        let selected = self.settings.account().map(OsString::from);
        sessions::claude_dir(|key| match key {
            "CLAUDE_CONFIG_DIR" => selected.clone().or_else(|| std::env::var_os(key)),
            _ => std::env::var_os(key),
        })
    }

    /// Sends the current account's 5-hour window when it is not the one the app has: after a
    /// report, and every second for its reset and an account switch.
    async fn send_usage(&self) {
        let account = self.account();
        let mut usage = self.usage.lock().await;
        let now = hook::now_ms() / 1000;
        if let Some(message) = usage.changed(account.as_deref(), now) {
            self.to_app(0, &message).await;
        }
    }

    /// `gh` on the user's `PATH` (waits only for the first answer).
    async fn gh(self: &Arc<Self>) -> crate::gh::Gh {
        crate::gh::Gh::on(self.users_path().await)
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

    /// Answers a request of the pull requests (9.31) or Actions (9.32) view off the frame
    /// loop: `gh` asks GitHub.
    fn github(self: &Arc<Self>, request: Control) {
        let state = self.clone();
        tokio::spawn(async move {
            // A checkout makes a worktree: in turn with the registry watch, as
            // `change_worktrees`.
            let _turn = match request {
                Control::ActOnPull { .. } => Some(state.changing.lock().await),
                _ => None,
            };
            let gh = state.gh().await;
            let replies = tokio::task::block_in_place(|| {
                let projects = &state.projects;
                let mut replies = match request {
                    Control::ListRuns { .. }
                    | Control::OpenRun { .. }
                    | Control::OpenJobLog { .. }
                    | Control::ActOnRun { .. } => {
                        crate::actions::answer(&gh, projects, &state.runs, request)
                    }
                    _ => crate::pulls::answer(&gh, projects, &state.pulls, request),
                };
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

    /// Stops following the project `id` (9.28), a group with its repositories (14.1), unless a
    /// process of Hive's terminals (their `sessions`) works in it; their settings and their
    /// worktrees' port blocks go with them. A group's repositories are removed before it.
    async fn remove_project(&self, id: String, sessions: &HashSet<i32>) {
        let removed = tokio::task::block_in_place(|| {
            self.projects.remove(&id, procs::Source::System, sessions)
        });
        let (worktrees, inside) = match removed {
            Ok(removed) => removed,
            Err(err) => {
                let message = err.to_string();
                return self
                    .to_app(0, &Control::RemoveProjectFailed { id, message })
                    .await;
            }
        };
        self.refollow.notify_one();
        self.to_app(0, &self.projects.spaces_message()).await;
        if let Err(err) = tokio::task::block_in_place(|| self.ports.forget(&worktrees)) {
            eprintln!("hive: warning: cannot free the ports of {id}: {err}");
        }
        for id in inside.into_iter().chain([id]) {
            let reply = match tokio::task::block_in_place(|| self.settings.forget(&id)) {
                Ok(settings) => settings.map(|settings| Control::Settings { settings }),
                Err(message) => Some(Control::SettingsFailed { message }),
            };
            if let Some(reply) = reply {
                self.to_app(0, &reply).await;
            }
            self.worktrees_sent().remove(&id);
            self.to_app(0, &Control::ProjectRemoved { id }).await;
        }
    }

    /// Watches `path` for the files panel instead of the worktree watched until now, if any;
    /// its changes against `base`.
    async fn watch_worktree(self: &Arc<Self>, path: Option<(String, DiffBase)>) {
        let mut watching = self.watching.lock().await;
        if let Some(old) = watching.take() {
            old.task.abort();
        }
        *watching = path.map(|(path, base)| {
            let (open, folders) = tokio::sync::watch::channel(Vec::new());
            let task = tokio::spawn(watch_files(self.clone(), path.clone(), base, folders));
            Watching { path, task, open }
        });
    }

    /// The ignored folders open in the files tree of the watched worktree `path`; nothing when
    /// another one is watched (the app asked before it switched). Refused, as `watch_worktree`
    /// is, for a path that is no worktree of a followed project (a group's folder, 14.1).
    async fn expand_ignored(&self, path: &str, folders: Vec<String>) {
        if let Err(err) = tokio::task::block_in_place(|| self.projects.worktree(path)) {
            return self.to_app(0, &error(err)).await;
        }
        if let Some(watching) = self.watching.lock().await.as_ref()
            && watching.path == path
        {
            watching.open.send_replace(folders);
        }
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
                ignored: listing.ignored.clone(),
                truncated: listing.truncated,
            };
            self.to_app(0, &files).await;
        }
        // One listing of the projects gives both the changes' base and the status, and one
        // `git status` both the changes and the status's count (9.14).
        let (changes, statuses) = tokio::task::block_in_place(|| {
            let projects = self.projects.list();
            let (changes, totals) = changes::answer(&projects, path.to_owned(), base);
            (
                changes,
                self.health_changed(&projects, Some((path, totals))),
            )
        });
        self.to_app(0, &changes).await;
        for message in statuses {
            self.to_app(0, &message).await;
        }
    }
}

/// Sends the worktree statuses that changed, every `interval` (first right away).
async fn watch_health(state: Arc<State>, interval: std::time::Duration) {
    let mut ticks = tokio::time::interval(interval);
    loop {
        ticks.tick().await;
        // Worktrees changed by hand (e.g. a branch switched) show within a tick.
        state.projects.forget();
        state.refresh_health().await;
    }
}

/// Lists the worktree `path` now and after every change, or of the ignored folders `open` in
/// its tree, until aborted or the watch fails. Git and inotify run on a blocking thread, off
/// the frame loop.
async fn watch_files(
    state: Arc<State>,
    path: String,
    base: DiffBase,
    mut open: tokio::sync::watch::Receiver<Vec<String>>,
) {
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
        watcher.open(open.borrow_and_update().iter().cloned());
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
        // A folder opened or closed mid-burst ends the wait too: the re-list covers both, and
        // the changes go with it as after any change.
        watching = tokio::select! {
            changed = watcher.changed() => changed,
            // Never an error: the sender outlives this task.
            Ok(()) = open.changed() => Ok(()),
        };
    }
}

/// Watches git's worktree registry of the current space's projects (9.36), and its groups'
/// folders (14.1): however a worktree, or a group's repository, is added or removed, the app
/// gets the projects again, once per burst.
async fn watch_registry(state: Arc<State>, registry: io::Result<Registry>) {
    let mut registry = match registry {
        Ok(registry) => registry,
        // E.g. no inotify instance left: the worktrees still follow hooks and the app.
        Err(err) => return eprintln!("hive: warning: cannot watch git's worktrees: {err}"),
    };
    let mut started = false;
    loop {
        let (roots, groups) = (state.projects.roots(), state.projects.groups());
        // Before listing, so a change made meanwhile is seen next time.
        tokio::task::block_in_place(|| registry.follow(&roots, &groups));
        // After a change, and after the projects followed changed: what happened in a
        // registry while it was not watched yet (e.g. right after a space switch) is sent too.
        if started {
            state.worktrees_changed().await;
        }
        started = true;
        tokio::select! {
            () = state.refollow.notified() => {}
            () = registry.changed() => {}
        }
    }
}

fn error(err: io::Error) -> Control {
    Control::Error {
        message: err.to_string(),
    }
}

#[cfg(test)]
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_stopped_task_is_out_of_its_blocking_work_before_the_runtime_shuts_down() {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        let blocked = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ms = std::time::Duration::from_millis;
        let work = {
            let blocked = blocked.clone();
            move || {
                tokio::task::block_in_place(|| std::thread::sleep(ms(300)));
                blocked.store(true, Ordering::SeqCst);
                ms(10_000)
            }
        };
        // Blocking work, then a timer polled in the same step (one line: the task never
        // gets past the timer, it is stopped there).
        let task = runtime.spawn(async move { tokio::time::sleep(work()).await });
        runtime.block_on(async {
            tokio::time::sleep(ms(50)).await;
            stop(task).await;
        });
        assert!(blocked.load(Ordering::SeqCst));
        runtime.shutdown_background();
    }

    #[test]
    fn the_registry_watch_ends_only_without_a_watcher() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(dir.path());
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .unwrap();
        // It ends instead of waiting for changes forever.
        let failed = Err(io::Error::other("no inotify"));
        runtime.block_on(watch_registry(state.clone(), failed));
        // With a watcher it follows the projects (again on a change of them) until stopped.
        state.refollow.notify_one();
        let watching = runtime.spawn(watch_registry(state, Registry::new()));
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert!(!watching.is_finished());
        // Not waited for: a watch that never yields (a broken `Registry::changed`) cannot
        // hold the test up.
        runtime.shutdown_background();
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
        let root = crate::paths::canonical(dir.path()).unwrap().join("r");
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
        // A worktree made behind Hive's back is listed again on a tick (9.14).
        let added = crate::paths::canonical(dir.path()).unwrap().join("w");
        git(&["worktree", "add", "-q", "-b", "w", added.to_str().unwrap()]);
        let frame = tokio::time::timeout(std::time::Duration::from_secs(10), sent.recv());
        let control = frame.await.expect("no status").unwrap().to_control();
        let json = serde_json::to_value(control.unwrap()).unwrap();
        assert_eq!(json["path"], added.display().to_string());
        ticking.abort();
    }

    /// Placing it would open it, which sends the user's credentials to its host.
    #[cfg(windows)]
    #[tokio::test]
    async fn on_windows_no_terminal_opens_in_a_network_folder() {
        let dir = tempfile::tempdir().unwrap();
        let state = test_state(dir.path());
        let (app, mut sent) = mpsc::unbounded_channel();
        *state.app.lock().await = Some(app);
        let (frames, _) = mpsc::unbounded_channel();
        state.open(1, r"\\host\share", (80, 24), None, frames).await;
        let frame = sent.try_recv().unwrap();
        let message = r"\\host\share: network and device paths are not supported: use a drive path such as C:\…";
        let error = Control::Error {
            message: message.into(),
        };
        assert_eq!((frame.channel, frame.to_control().unwrap()), (1, error));
        assert!(state.terminals.lock().await.is_empty());
    }
}
