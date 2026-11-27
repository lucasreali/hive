//! The agents detected in Hive's terminals (9.20): detected from their hooks, followed
//! through their events and their terminal's output, named from their session logs.

use std::collections::HashMap;
use std::io;
use std::sync::atomic::Ordering;
use std::sync::{Arc, PoisonError};
use std::time::Instant;

use hive_protocol::{AgentEvent, Control, EventKind, OpenSession};

use super::State;
use crate::power;
use crate::projects;
use crate::sessions;
use crate::snapshot;
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

    /// Holds the power assertion while an agent works and the setting is on (17.17), and sends
    /// `subagent_worktrees` when the set changed. Called with the agents lock held, so the
    /// changes go out in order.
    pub(super) async fn owned_changed(&self, agents: &HashMap<String, Agent>) {
        self.power_changed(agents);
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

    /// Holds the power assertion while an agent works and the setting is on, else releases it.
    fn power_changed(&self, agents: &HashMap<String, Agent>) {
        let wanted = power::wanted(self.settings.keep_awake(), agents.values());
        let mut awake = self.awake.lock().unwrap_or_else(PoisonError::into_inner);
        awake.set(wanted);
    }

    /// Tracks agents: a `SessionStart` from one of our terminals marks its `claude` as hooked
    /// and detects the agent; any other event of a detected agent (or of its subagents)
    /// updates its state; its own `SessionEnd` removes it. The `SessionStart` that follows a
    /// compaction leaves a known agent as it is (state, subagents, tokens).
    pub(super) async fn saw(self: &Arc<Self>, event: &AgentEvent, sent_ns: u64) {
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
        let now = Instant::now();
        agent.clock(now, crate::hook::now_ms());
        if let Some(state) = agent.apply(id, event, sent_ns, now, &place) {
            self.to_app(channel, &state).await;
        }
        // After the state that lists them as ended (28.8).
        for finished in agent.finished(id) {
            self.to_app(channel, &finished).await;
        }
        if event.subagent.is_none() {
            self.turn_snapshot(id, agent.worktree.as_deref(), &event.kind);
        }
        // The transcript got a message: its usage is read on the next tick (at most once a
        // second).
        agent.usage.due |= matches!(
            event.kind,
            EventKind::ToolFinished { .. }
                | EventKind::TurnFinished
                | EventKind::SubagentStopped
                | EventKind::TeammateIdled
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

    /// The agent `id`'s own `kind` of event for its turn snapshots (27.2): a prompt takes one of
    /// `worktree` (a repository's, not a folder project's), an edit before it is done makes it
    /// late, its end drops its snapshots. Git runs on a blocking thread, never on the hook's path;
    /// the watched worktree's changes are sent again once it is done.
    fn turn_snapshot(self: &Arc<Self>, id: &str, worktree: Option<&str>, kind: &EventKind) {
        let (state, snapshots, id) = (self.clone(), self.snapshots.clone(), id.to_owned());
        match (kind, worktree) {
            (EventKind::PromptSubmitted, Some(worktree)) => {
                snapshots.started(&id);
                let (worktree, started) = (worktree.to_owned(), crate::hook::now_ms());
                tokio::spawn(async move {
                    let blocking = state.clone();
                    let taken = tokio::task::spawn_blocking(move || {
                        // A folder project's folder (23.1) is in no repository: git refuses it.
                        let dir = blocking.projects.worktree(&worktree)?;
                        snapshots
                            .take(&dir, &worktree, &id, started)
                            .map(|_| worktree)
                    });
                    let taken = taken.await.map_err(io::Error::other);
                    match taken.and_then(std::convert::identity) {
                        Ok(worktree) => state.rewatch(Some(&worktree)).await,
                        Err(err) => eprintln!("hive: warning: no snapshot of this turn: {err}"),
                    }
                });
            }
            (EventKind::ToolFinished { tool: Some(tool) }, _)
                if snapshot::EDITS.contains(&tool.as_str()) =>
            {
                snapshots.edited(&id);
            }
            (EventKind::SessionEnded { .. }, _) => {
                tokio::spawn(async move {
                    let _ = tokio::task::spawn_blocking(move || snapshots.end(&id)).await;
                    state.rewatch(None).await;
                });
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
        agent.project = project;
        agent.worktree = worktree;
        agent.cwd = cwd;
        agent.transcript = transcript::transcript_path(&event.raw);
        // Read on the next tick: what the transcript already holds is not news (interrupts).
        agent.usage.due = true;
        agent.claude_dir = claude_dir;
        let state = agent.message(&id);
        self.to_app(channel, &agent.detected(&id)).await;
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

    /// Sent to a newly connected app right after `Welcome`, and to a reloaded UI (`Resync`):
    /// every live agent and its state.
    pub(super) async fn snapshot(&self) {
        let agents = self.agents.lock().await;
        for (id, agent) in agents.iter() {
            self.to_app(agent.channel, &agent.detected(id)).await;
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
    /// transcripts due. Then holds or releases the power assertion: a state changed, or the
    /// setting.
    pub(super) async fn tick_agents(&self, last_output: &HashMap<u32, Instant>, now: Instant) {
        let silence = self.settings.silence();
        let mut agents = self.agents.lock().await;
        // A `now` taken before this is shown as this moment (`Agent::clock`).
        let clock = (Instant::now(), crate::hook::now_ms());
        for (id, agent) in agents.iter_mut() {
            let output = last_output.get(&agent.channel);
            agent.watched = self.watches(agent.channel);
            agent.clock(clock.0, clock.1);
            if let Some(message) =
                output.and_then(|&output| agent.reconcile(id, silence, output, now))
            {
                self.to_app(agent.channel, &message).await;
            }
            if let Some(message) = agent.expire(id, now) {
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
        self.power_changed(&agents);
    }
}

/// The agent an event belongs to: its session id; `None` for subagent events.
fn agent_id(event: &AgentEvent) -> Option<String> {
    match event.subagent {
        None => event.session_id.clone(),
        Some(_) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::power::tests::Calls;
    use hive_protocol::Settings;

    #[tokio::test(flavor = "multi_thread")]
    async fn an_edit_before_its_snapshot_makes_the_turn_late_and_titles_name_the_turns() {
        let dir = tempfile::tempdir().unwrap();
        let state = super::super::test_state(dir.path());
        let repo = dir.path().join("repo");
        std::fs::create_dir(&repo).unwrap();
        crate::health::tests::run(&repo, &["init", "-q"]);
        crate::health::tests::commit(&repo, "a");
        let id = repo.display().to_string();
        let tool = |tool: &str| EventKind::ToolFinished {
            tool: Some(tool.into()),
        };
        // A read is no edit; an edit is, only for its own session.
        for (session, event) in [("read", tool("Read")), ("edit", tool("Write"))] {
            state.snapshots.started(session);
            state.turn_snapshot(session, None, &event);
            let turn = state.snapshots.take(&repo, &id, session, 1).unwrap();
            assert_eq!(turn.late, session == "edit", "{session}");
        }
        // An agent's title names its turns; one without a title is not named.
        let mut agents = state.agents.lock().await;
        let mut titled = Agent::new(1, Instant::now(), 0);
        titled.title = Some("claude-3".into());
        agents.insert("s1".into(), titled);
        agents.insert("s2".into(), Agent::new(2, Instant::now(), 0));
        drop(agents);
        let titles = HashMap::from([("s1".to_owned(), "claude-3".to_owned())]);
        assert_eq!(state.titles().await, titles);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_power_assertion_is_held_while_an_agent_works_and_the_setting_is_on() {
        let dir = tempfile::tempdir().unwrap();
        let calls = Calls::default();
        let state = super::super::test_state_with(dir.path(), calls.power());
        let now = Instant::now();
        let quiet = HashMap::new();
        state.tick_agents(&quiet, now).await;
        assert!(calls.taken().is_empty());
        let mut agents = state.agents.lock().await;
        agents.insert("idle".into(), Agent::new(1, now, 0));
        state.owned_changed(&agents).await;
        assert!(calls.taken().is_empty());
        agents.insert("busy".into(), Agent::working(2, now));
        state.owned_changed(&agents).await;
        assert_eq!(calls.taken(), vec![true]);
        drop(agents);
        // The setting turned off: released on the next tick.
        let mut off = Settings::default();
        off.agents.keep_awake = false;
        state.settings.set(off).unwrap();
        state.tick_agents(&quiet, now).await;
        assert_eq!(calls.taken(), vec![true, false]);
        state.settings.set(Settings::default()).unwrap();
        state.tick_agents(&quiet, now).await;
        assert_eq!(calls.taken(), vec![true, false, true]);
        state.agents.lock().await.remove("busy");
        state.tick_agents(&quiet, now).await;
        assert_eq!(calls.taken(), vec![true, false, true, false]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn an_ended_background_command_leaves_on_the_tick_ten_seconds_later() {
        use crate::adapter::{Adapter, ClaudeCode};
        let dir = tempfile::tempdir().unwrap();
        let state = super::super::test_state(dir.path());
        let start = Instant::now();
        let event = |name: &str, extra: serde_json::Value| {
            let mut payload = serde_json::json!({"session_id": "s"});
            for (k, v) in extra.as_object().unwrap() {
                payload[k] = v.clone();
            }
            ClaudeCode.translate(name, Some("1".into()), payload)
        };
        let launch = event(
            "PostToolUse",
            serde_json::json!({"tool_name": "Bash", "tool_response": {"backgroundTaskId": "b1"}}),
        );
        let stop = event("Stop", serde_json::json!({"background_tasks": []}));
        let mut agent = Agent::new(1, start, 0);
        agent.apply("s", &launch, 1, start, &|_| None);
        agent.apply("s", &stop, 2, start, &|_| None);
        let listed = |agent: &Agent| {
            let message = agent.message("s");
            matches!(message, Control::AgentState { ref background, .. } if background.len() == 1)
        };
        assert!(listed(&agent));
        state.agents.lock().await.insert("s".into(), agent);
        let quiet = HashMap::new();
        state.tick_agents(&quiet, start).await;
        assert!(listed(&state.agents.lock().await["s"]));
        state
            .tick_agents(&quiet, start + crate::background::SHOWN_ENDED)
            .await;
        let left = state.agents.lock().await["s"].message("s");
        assert!(
            matches!(left, Control::AgentState { ref background, .. } if background.is_empty())
        );
    }
}
