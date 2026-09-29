//! Hive's terminals: their PTYs, opened, fed, copied to the app and closed (9.20).

use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};

use hive_protocol::{Control, Frame};
use pty_process::OwnedReadPty;
use tokio::io::AsyncReadExt;
use tokio::process::Child;
use tokio::sync::mpsc;

use super::State;
use crate::terminal::{self, Input};
use crate::{procs, projects, watch};

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

    pub(super) async fn open(
        self: &Arc<Self>,
        channel: u32,
        cwd: &str,
        cols: u16,
        rows: u16,
        frames: mpsc::UnboundedSender<Frame>,
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

/// Warns the app about terminals running a `claude` that sends no hook events, and applies
/// the silence rule to agents whose terminal went quiet; sends the session usage when it changed
/// (12.1: its reset, a space switch).
pub(super) async fn watch_terminals(state: Arc<State>) {
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
    mut pty: OwnedReadPty,
    mut child: Child,
    output: terminal::Output,
    last_output: terminal::LastOutput,
) {
    let copy = copy(&mut pty, &output, &last_output);
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
/// behind, the PTY is not read, so the program in it waits (9.19).
async fn copy(
    pty: &mut OwnedReadPty,
    output: &terminal::Output,
    last_output: &terminal::LastOutput,
) {
    let mut buf = vec![0; 64 * 1024];
    while let Ok(n @ 1..) = pty.read(&mut buf).await {
        last_output.touch();
        if !output.send(&buf[..n]).await {
            break;
        }
    }
}
