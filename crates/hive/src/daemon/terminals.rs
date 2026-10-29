//! Hive's terminals: their PTYs, opened, fed, copied to the app and closed (9.20).

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};

use hive_protocol::{AccountDir, Control, Frame};
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc;

use super::State;
use crate::terminal::{self, Child, Input};
use crate::{osc, procs, projects, watch};

impl State {
    /// Queues input or a resize; ignored once the terminal is gone.
    pub(super) fn input(&self, channel: u32, input: Input) {
        if let Some((terminal, _)) = self.inputs().get(&channel) {
            let _ = terminal.send(input);
        }
    }

    /// The app wrote `bytes` of the terminal's output to its screen (9.19); ignored once the
    /// terminal is gone.
    pub(super) fn ack(&self, channel: u32, bytes: u32) {
        if let Some((_, output)) = self.inputs().get(&channel) {
            output.ack(bytes);
        }
    }

    /// Opens a terminal of `(cols, rows)` in `cwd`, as the `account` asked for (12.2; the
    /// current one when `None`).
    pub(super) async fn open(
        self: &Arc<Self>,
        channel: u32,
        cwd: &str,
        (cols, rows): (u16, u16),
        account: Option<AccountDir>,
        frames: mpsc::UnboundedSender<Frame>,
    ) {
        // Never in a network or device folder: placing it would open it.
        #[cfg(windows)]
        if let Err(err) = crate::windows::local(std::path::Path::new(cwd)) {
            let message = err.to_string();
            return self.to_app(channel, &Control::Error { message }).await;
        }
        let claude_dir = match self.settings.claude_dir(account) {
            Ok(dir) => dir,
            Err(message) => return self.to_app(channel, &Control::Error { message }).await,
        };
        // Its Claude account, its space's environment (6.14) with its GitHub account's token
        // (9.30), and its worktree's `HIVE_*` (6.8), placed before the lock since placing
        // lists worktrees.
        let space = tokio::task::block_in_place(|| self.projects.space_env(cwd));
        let claude = claude_dir
            .iter()
            .map(|dir| ("CLAUDE_CONFIG_DIR", dir.clone()));
        let mut env: Vec<_> = claude.chain(crate::spaces::vars(&space)).collect();
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
        let shell = self.settings.get().0.terminal.shell;
        // The locale's variables (D18.2), read at start; still being read, waited for here,
        // never under the lock below.
        #[cfg(unix)]
        tokio::task::block_in_place(terminal::locale_env);
        let opened = {
            let mut terminals = self.terminals.lock().await;
            match terminals.entry(channel) {
                _ if channel == 0 => Err("terminal channels start at 1".to_owned()),
                Entry::Occupied(_) => Err(format!("terminal {channel} is already open")),
                Entry::Vacant(slot) => {
                    let size = (cols, rows);
                    terminal::spawn(channel, cwd, size, &self.bin_dir, &env, shell).map(
                        |(mut terminal, input, pty, child)| {
                            terminal.claude_dir = claude_dir;
                            let last = terminal.last_output.clone();
                            let session = terminal.session;
                            slot.insert(terminal);
                            let output = terminal::Output::new(channel, frames);
                            self.inputs().insert(channel, (input, output.clone()));
                            // Before the pump starts: a shell that exits at once removes it.
                            if let Some(worktree) = worktree.clone() {
                                let mut worktrees = self
                                    .terminal_worktrees
                                    .lock()
                                    .unwrap_or_else(PoisonError::into_inner);
                                worktrees.insert(channel, worktree);
                            }
                            let state = self.clone();
                            tokio::spawn(pump(state, channel, session, pty, child, output, last));
                        },
                    )
                }
            }
        };
        if opened.is_ok() {
            self.tick.notify_one();
        }
        let reply = match opened {
            Ok(()) => Control::TerminalOpened { worktree },
            Err(message) => Control::Error { message },
        };
        self.to_app(channel, &reply).await;
        // A subagent's worktree the human opened a terminal in shows again.
        let agents = self.agents.lock().await;
        self.owned_changed(&agents).await;
    }

    /// Ends the terminal's processes, off the frame loop; its exit is reported by [`pump`].
    pub(super) fn close(self: &Arc<Self>, channel: u32) {
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

impl State {
    /// Whether the terminals' tick has anything to do (19.3): a terminal is open (its agents'
    /// silence rule, its unhooked `claude`), or the app has a usage window (its reset, its
    /// pace). Neither starts without waking the tick.
    pub(super) async fn ticking(&self) -> bool {
        // One lock at a time.
        let open = !self.terminals.lock().await.is_empty();
        open || self.usage.lock().await.shown()
    }
}

/// Warns the app about terminals running a `claude` that sends no hook events, and applies
/// the silence rule to agents whose terminal went quiet; sends the session usage when it changed
/// (12.1: its reset, a space switch). Every `every` while [`State::ticking`], asleep otherwise.
/// Processes are read only when some terminal printed since the last tick (a `claude` that
/// starts or exits prints) or a watch is still pending.
pub(super) async fn watch_terminals(
    state: Arc<State>,
    mut scan: impl procs::Scan,
    every: Duration,
) {
    let mut ticks = tokio::time::interval(every);
    // Each terminal's last output as of the last tick.
    let mut printed = HashMap::new();
    loop {
        ticks.tick().await;
        if !state.ticking().await {
            state.tick.notified().await;
            // The next tick a whole interval from now, not a burst of the ones missed asleep.
            ticks.reset();
            continue;
        }
        let (sessions, last_output, pending) = {
            let terminals = state.terminals.lock().await;
            let sessions: HashSet<i32> = terminals.values().map(|t| t.session).collect();
            let last_output: HashMap<u32, Instant> = (terminals.iter())
                .map(|(channel, t)| (*channel, t.last_output.get()))
                .collect();
            let pending = terminals.values().any(|t| t.watch.pending());
            (sessions, last_output, pending)
        };
        let due = pending || last_output != printed;
        printed.clone_from(&last_output);
        let now = Instant::now();
        if due && !sessions.is_empty() {
            // Only the terminals' processes, read on a blocking thread.
            let list = || watch::claude_sessions(&scan.in_sessions(&sessions));
            let running = tokio::task::block_in_place(list);
            let unhooked: Vec<u32> = (state.terminals.lock().await.iter_mut())
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
        // Not under the terminals lock: placing a new agent holds the agents lock while git runs.
        state.tick_agents(&last_output, now).await;
        state.send_usage().await;
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
    mut pty: terminal::Pty,
    mut child: Child,
    output: terminal::Output,
    last_output: terminal::LastOutput,
) {
    let copy = copy(&state, channel, &mut pty, &output, &last_output);
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

/// Copies PTY output to the app until the PTY closes or the app is gone. While the app is
/// behind, the PTY is not read, so the program in it waits (9.19). The OSC sequences the
/// service acts on (17.25, [`osc`]) are read on the way, each after the output holding it.
async fn copy(
    state: &Arc<State>,
    channel: u32,
    pty: &mut terminal::Pty,
    output: &terminal::Output,
    last_output: &terminal::LastOutput,
) {
    let mut buf = vec![0; 64 * 1024];
    let (mut scanner, mut shell) = (osc::Scanner::default(), osc::Shell::default());
    while let Ok(n @ 1..) = pty.read(&mut buf).await {
        last_output.touch();
        let signals = scanner.feed(&buf[..n]);
        if !output.send(&buf[..n]).await {
            break;
        }
        let (progress, alerts) = shell.read(signals, Instant::now());
        if let Some(progress) = progress {
            output.control(&progress);
        }
        for alert in alerts {
            state.alert(channel, alert);
        }
    }
}

impl State {
    /// Sends what `alert` of terminal `channel` amounts to ([`osc::message`]), off the
    /// output's path: whether an agent runs there takes the agents lock (9.13).
    fn alert(self: &Arc<Self>, channel: u32, alert: osc::Alert) {
        let state = self.clone();
        tokio::spawn(async move {
            let agent = state
                .agents
                .lock()
                .await
                .values()
                .any(|a| a.channel == channel);
            let secs = state.settings.get().0.notifications.command_secs;
            if let Some(message) = osc::message(alert, secs, agent, state.watches(channel)) {
                state.to_app(channel, &message).await;
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const EVERY: Duration = Duration::from_millis(10);

    /// A process source that counts its listings; session 7 runs a `claude` when `claude`.
    #[derive(Clone, Default)]
    struct Counting {
        listings: Arc<AtomicUsize>,
        claude: bool,
    }

    impl procs::Scan for Counting {
        fn in_sessions(&mut self, sessions: &HashSet<i32>) -> Vec<procs::Proc> {
            self.listings.fetch_add(1, Ordering::SeqCst);
            let claude = procs::Proc {
                pid: 8,
                pgrp: 8,
                session: 7,
                comm: "claude".into(),
            };
            (self.claude && sessions.contains(&7))
                .then_some(claude)
                .into_iter()
                .collect()
        }
    }

    impl Counting {
        fn listings(&self) -> usize {
            self.listings.load(Ordering::SeqCst)
        }

        /// Waits, a hundred ticks at most, until there were `n` listings.
        async fn until(&self, n: usize) {
            for _ in 0..100 {
                if self.listings() >= n {
                    return;
                }
                tokio::time::sleep(EVERY).await;
            }
        }
    }

    /// Opens a terminal of session 7 as `open` does, without a shell; its output.
    async fn open(state: &State) -> terminal::LastOutput {
        let last_output = terminal::LastOutput::new();
        let terminal = terminal::Terminal {
            session: 7,
            watch: watch::Watch::default(),
            last_output: last_output.clone(),
            claude_dir: None,
        };
        state.terminals.lock().await.insert(1, terminal);
        state.tick.notify_one();
        last_output
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn processes_are_listed_only_after_output_or_while_a_claude_is_unanswered() {
        let dir = tempfile::tempdir().unwrap();
        let state = super::super::test_state(dir.path());
        let scan = Counting::default();
        let task = tokio::spawn(watch_terminals(state.clone(), scan.clone(), EVERY));
        // No terminal: no listing.
        tokio::time::sleep(EVERY * 10).await;
        assert_eq!(scan.listings(), 0);
        // A new terminal is listed once, then not while it stays quiet.
        let output = open(&state).await;
        scan.until(1).await;
        tokio::time::sleep(EVERY * 10).await;
        assert_eq!(scan.listings(), 1);
        // Output: listed once more.
        output.touch();
        scan.until(2).await;
        tokio::time::sleep(EVERY * 10).await;
        assert_eq!(scan.listings(), 2);
        task.abort();

        // A `claude` that sent no `SessionStart` yet is listed every tick, though quiet.
        let state = super::super::test_state(dir.path());
        let scan = Counting {
            claude: true,
            ..Counting::default()
        };
        let task = tokio::spawn(watch_terminals(state.clone(), scan.clone(), EVERY));
        open(&state).await;
        scan.until(5).await;
        assert!(scan.listings() >= 5);
        // Hooked: quiet again (a tick already under way may still list).
        state
            .terminals
            .lock()
            .await
            .get_mut(&1)
            .unwrap()
            .watch
            .hooked();
        let hooked = scan.listings();
        tokio::time::sleep(EVERY * 10).await;
        assert!(scan.listings() <= hooked + 1);
        task.abort();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_tick_runs_while_a_terminal_is_open_or_a_usage_window_is_shown() {
        let dir = tempfile::tempdir().unwrap();
        let state = super::super::test_state(dir.path());
        assert!(!state.ticking().await);
        open(&state).await;
        assert!(state.ticking().await);
        state.terminals.lock().await.clear();
        assert!(!state.ticking().await);
        // A window reported is shown once sent, then ticks for its reset and pace.
        let account = state.account().unwrap().to_string_lossy().into_owned();
        let window = hive_protocol::SessionWindow {
            used_percentage: 10,
            resets_at: crate::hook::now_ms() / 1000 + 3600,
        };
        state.usage.lock().await.report(account, window, None);
        assert!(!state.ticking().await);
        state.send_usage().await;
        assert!(state.ticking().await);
    }
}
