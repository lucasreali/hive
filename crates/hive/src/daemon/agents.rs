//! The agents detected in Hive's terminals (9.20): detected from their hooks, followed
//! through their events and their terminal's output, named from their session logs.

use std::collections::HashMap;
use std::sync::PoisonError;
use std::sync::atomic::Ordering;
use std::time::Instant;

use hive_protocol::{AgentEvent, Control, EventKind, OpenSession};

use super::State;
use crate::projects;
use crate::sessions;
use crate::states::{self, Agent};
use crate::transcript;

impl State {
    /// The app's `view`: the terminal `watched` (0 for none) is in view in the focused window.
    /// Its agent waiting for you is seen now (15.6), not at its next event.
    pub(super) async fn view(&self, watched: u32) {
        self.watched.store(watched, Ordering::Relaxed);
        for (id, agent) in self.agents.lock().await.iter_mut() {
            if let Some(message) = agent.watch(id, self.watches(agent.channel)) {
                self.to_app(agent.channel, &message).await;
            }
        }
    }

    /// Sends `subagent_worktrees` when the set changed. Called with the agents lock held, so
    /// the changes go out in order.
    pub(super) async fn owned_changed(&self, agents: &HashMap<String, Agent>) {
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

    /// Tracks agents: a `SessionStart` from one of our terminals marks its `claude` as hooked
    /// and detects the agent; any other event of a detected agent (or of its subagents)
    /// updates its state; its own `SessionEnd` removes it. The `SessionStart` that follows a
    /// compaction leaves a known agent as it is (state, subagents, tokens).
    pub(super) async fn saw(&self, event: &AgentEvent, sent_ns: u64) {
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
        let sessions = self.sessions.at(&[agent.claude_dir.as_deref()]);
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
    pub(super) async fn save_open(&self) {
        let agents = self.agents.lock().await;
        let mut open: Vec<(u32, OpenSession)> = agents
            .iter()
            .filter_map(|(id, agent)| {
                let cwd = agent.cwd.clone()?;
                let id = id.clone();
                let config_dir = agent.claude_dir.clone();
                Some((
                    agent.channel,
                    OpenSession {
                        id,
                        cwd,
                        config_dir,
                    },
                ))
            })
            .collect();
        open.sort_by_key(|(channel, _)| *channel);
        let open: Vec<OpenSession> = open.into_iter().map(|(_, s)| s).collect();
        if let Err(err) = sessions::save_open(&self.restore.file, &open) {
            eprintln!("hive: warning: cannot keep the open sessions: {err}");
        }
    }

    /// Sent to a newly connected app right after `Welcome`: the state of every live agent.
    pub(super) async fn snapshot(&self) {
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

    /// The agents' half of [`super::terminals::watch_terminals`]'s tick: applies the silence
    /// rule to agents whose terminal went quiet (`last_output`, by channel) and reads the
    /// transcripts due.
    pub(super) async fn tick_agents(&self, last_output: &HashMap<u32, Instant>, now: Instant) {
        let silence = self.settings.silence();
        for (id, agent) in self.agents.lock().await.iter_mut() {
            let output = last_output.get(&agent.channel);
            agent.watched = self.watches(agent.channel);
            if let Some(message) =
                output.and_then(|&output| agent.reconcile(id, silence, output, now))
            {
                self.to_app(agent.channel, &message).await;
            }
            // While it may be interrupted, its transcript is read every tick for the interrupt.
            if agent.busy() {
                agent.usage.due = true;
            }
            let (Some(path), true) = (&agent.transcript, agent.usage.due) else {
                continue;
            };
            // Claude's projects folders (its terminal's, then every account's, 12.2), which its
            // transcript must stay inside.
            let roots = self.accounts_sessions(agent.claude_dir.as_deref()).roots();
            // A bounded read (see `transcript::Usage`), off the other tasks' threads.
            let usage = &mut agent.usage;
            let read = || usage.read(id, &roots, path);
            if let Some(message) = tokio::task::block_in_place(read) {
                self.to_app(agent.channel, &message).await;
            }
            if agent.usage.interrupted()
                && let Some(message) = agent.interrupt(id, now)
            {
                self.to_app(agent.channel, &message).await;
            }
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
