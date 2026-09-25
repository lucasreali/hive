//! Agent states from hook events ("Mapeamento de estados" in `docs/hive.md`). Pure logic: the
//! daemon feeds events and the clock in, and sends the resulting `agent_state` messages.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use hive_protocol::{AgentEvent, AgentState, Control, EventKind, Notification, SubagentState};
use serde_json::Value;

/// Subagents kept per agent; later ones are ignored, so a message always fits in a frame.
const MAX_SUBAGENTS: usize = 32;

/// Longest subagent id or type kept (real ones are short); longer ones are ignored.
const MAX_ID: usize = 256;

/// Longest worktree path or subagent cwd used to link a subagent to its own worktree.
const MAX_PATH: usize = 4096;

/// Background launches remembered per subagent; the oldest is forgotten first.
const MAX_LAUNCHED: usize = 32;

/// A detected agent: its terminal and the state of it and its live subagents.
#[derive(Debug)]
pub struct Agent {
    /// Terminal channel (`HIVE_TERMINAL_ID`) the agent runs in.
    pub channel: u32,
    /// The worktree the agent itself was placed in (#19). Subagents are linked to other
    /// worktrees only when it is known.
    pub worktree: Option<String>,
    /// The folder the agent runs in, from its `SessionStart`: where to resume it.
    pub cwd: Option<String>,
    /// The session's name from its log (the user's, else Claude's), once known.
    pub title: Option<String>,
    state: AgentState,
    /// In start order.
    subagents: Vec<SubagentState>,
    /// The last cwd placed per live subagent, so `place` runs once per cwd.
    placed: HashMap<String, String>,
    /// Background task ids each live subagent launched (Bash `run_in_background`, `Monitor`,
    /// a background `Agent`), from its `PostToolUse` `tool_response`.
    launched: HashMap<String, Vec<String>>,
    /// Subagents that ended their turn (`SubagentStop`) while one of their launches still
    /// runs: kept listed, working, until woken or until none of those runs any more.
    waiting: HashSet<String>,
    /// Last hook event for the agent or a subagent; silence is counted from here at the latest.
    last_event: Instant,
    /// Whether its terminal is the one in view in the focused app window (the app's `view`);
    /// the daemon keeps it current.
    pub watched: bool,
    /// It finished while watched: already seen, so not pending until its state changes.
    seen: bool,
    /// What the agent itself is doing, from its last tool call; cleared when its turn ends.
    activity: Option<String>,
    /// Wall clock (ms since the epoch) when the displayed state began.
    since_ms: u64,
    /// A moment and its wall clock time: wall times are counted from here with the monotonic
    /// clock, so the caller's clocks are the only ones used.
    origin: (Instant, u64),
}

impl Agent {
    /// An agent that just sent `SessionStart`, `now` being `wall_ms` on the wall clock (ms
    /// since the epoch).
    pub fn new(channel: u32, now: Instant, wall_ms: u64) -> Self {
        Self {
            channel,
            worktree: None,
            cwd: None,
            title: None,
            state: AgentState::Idle,
            subagents: Vec::new(),
            placed: HashMap::new(),
            launched: HashMap::new(),
            waiting: HashSet::new(),
            last_event: now,
            watched: false,
            seen: false,
            activity: None,
            since_ms: wall_ms,
            origin: (now, wall_ms),
        }
    }

    /// `now` on the wall clock.
    fn wall(&self, now: Instant) -> u64 {
        let (at, ms) = self.origin;
        ms + now.saturating_duration_since(at).as_millis() as u64
    }

    /// Applies one hook event of this agent (or of one of its subagents). `place` answers the
    /// worktree containing a cwd; it is slow, so it is asked once per new subagent cwd.
    /// Returns the new message when it changed.
    ///
    /// A subagent owns a worktree (#22) when a `WorktreeCreate` carries its `agent_id`, or
    /// else when its own events come from a worktree other than its agent's (the `cwd`
    /// follows Claude into the worktree). It loses it on that worktree's `WorktreeRemove`, or
    /// by leaving. To confirm by spike 1.12: whether `WorktreeCreate` carries `agent_id`, and
    /// that a subagent's events carry its own `cwd`.
    ///
    /// A subagent leaves on its `SubagentStop`, unless a background task it launched is still
    /// in that payload's `background_tasks` (Claude Code lists the session's running ones on
    /// every `Stop`/`SubagentStop`): it waits to be woken by it. A waiting subagent leaves
    /// when a later `background_tasks` no longer has any of its launches, or when its own
    /// worktree is removed.
    pub fn apply(
        &mut self,
        id: &str,
        event: &AgentEvent,
        now: Instant,
        place: &dyn Fn(&str) -> Option<String>,
    ) -> Option<Control> {
        self.changed(id, now, |agent| {
            agent.last_event = now;
            if let EventKind::WorktreeRemoved { path: Some(path) } = &event.kind {
                let gone = agent
                    .subagents
                    .iter()
                    .filter(|s| agent.waiting.contains(&s.id) && s.worktree.as_ref() == Some(path));
                for id in gone.map(|s| s.id.clone()).collect::<Vec<_>>() {
                    agent.leave(&id);
                }
                for sub in &mut agent.subagents {
                    if sub.worktree.as_ref() == Some(path) {
                        sub.worktree = None;
                    }
                }
            }
            let live = event.raw.get("background_tasks").and_then(Value::as_array);
            let stopped = match (&event.subagent, &event.kind) {
                (Some(sub), EventKind::SubagentStopped) => Some(&sub.id),
                _ => None,
            };
            if let Some(id) = stopped {
                agent.waiting.insert(id.clone());
            }
            if stopped.is_some() || live.is_some() {
                agent.prune(live.map_or(&[], Vec::as_slice));
            }
            let state = state_of(&event.kind);
            let Some(sub) = &event.subagent else {
                if let Some(state) = state {
                    agent.state = state;
                }
                return follow(&mut agent.activity, event);
            };
            // `SubagentStop` has no state: a kept subagent is left as `prune` set it below.
            if let EventKind::SessionEnded { .. } = event.kind {
                return agent.leave(&sub.id);
            }
            let known = agent.subagents.iter().position(|s| s.id == sub.id);
            if let Some(i) = known {
                follow(&mut agent.subagents[i].activity, event);
            }
            if let EventKind::WorktreeCreated {
                path: Some(path), ..
            } = &event.kind
            {
                if let Some(i) = known
                    && path.len() <= MAX_PATH
                {
                    agent.subagents[i].worktree = Some(path.clone());
                }
                return;
            }
            let Some(state) = state else { return };
            agent.waiting.remove(&sub.id);
            let i = match known {
                Some(i) => i,
                None if agent.subagents.len() < MAX_SUBAGENTS
                    && sub.id.len() <= MAX_ID
                    && sub.agent_type.as_ref().is_none_or(|t| t.len() <= MAX_ID) =>
                {
                    agent.subagents.push(SubagentState {
                        id: sub.id.clone(),
                        agent_type: sub.agent_type.clone(),
                        state,
                        worktree: None,
                        activity: event.activity.clone(),
                        since_ms: 0,
                    });
                    agent.subagents.len() - 1
                }
                None => return,
            };
            let known = &mut agent.subagents[i];
            known.state = state;
            if let Some(task) = launch(&event.raw) {
                let ids = agent.launched.entry(sub.id.clone()).or_default();
                if ids.len() == MAX_LAUNCHED {
                    ids.remove(0);
                }
                ids.push(task);
            }
            // Without a hook naming it, a subagent's worktree is where its events come from.
            let (Some(cwd), Some(own), None) = (&event.cwd, &agent.worktree, &known.worktree)
            else {
                return;
            };
            if cwd.len() > MAX_PATH || agent.placed.get(&sub.id) == Some(cwd) {
                return;
            }
            agent.placed.insert(sub.id.clone(), cwd.clone());
            known.worktree = place(cwd).filter(|w| w != own);
        })
    }

    /// Keeps each waiting subagent's launches that are still in `live` (`background_tasks`),
    /// working; one with none left leaves.
    fn prune(&mut self, live: &[Value]) {
        let live: HashSet<&str> = live
            .iter()
            .filter_map(|t| t.get("id").and_then(Value::as_str))
            .collect();
        for id in self.waiting.clone() {
            let ids = self.launched.entry(id.clone()).or_default();
            ids.retain(|t| live.contains(t.as_str()));
            if ids.is_empty() {
                self.leave(&id);
            } else if let Some(sub) = self.subagents.iter_mut().find(|s| s.id == id) {
                sub.state = AgentState::Working;
            }
        }
    }

    /// Forgets a subagent.
    fn leave(&mut self, id: &str) {
        self.subagents.retain(|s| s.id != id);
        self.placed.remove(id);
        self.launched.remove(id);
        self.waiting.remove(id);
    }

    /// Rule 2, checked periodically: after `silence` (the `agents.silence_secs` setting)
    /// without terminal output (nor hook events), working and waiting-for-permission become
    /// waiting for you: the agent was interrupted (Esc/Ctrl+C fire no `Stop`). Output alone
    /// never moves a state back; only hook events do. A subagent waiting on a background task
    /// is silent by design and keeps working.
    pub fn reconcile(
        &mut self,
        id: &str,
        silence: Duration,
        last_output: Instant,
        now: Instant,
    ) -> Option<Control> {
        if now.saturating_duration_since(last_output.max(self.last_event)) < silence {
            return None;
        }
        self.changed(id, now, |agent| {
            let waiting = &agent.waiting;
            let subagents = agent.subagents.iter_mut();
            let subagents = subagents
                .filter(|s| !waiting.contains(&s.id))
                .map(|s| (&mut s.state, &mut s.activity));
            let own = (&mut agent.state, &mut agent.activity);
            for (state, activity) in std::iter::once(own).chain(subagents) {
                if matches!(state, AgentState::Working | AgentState::WaitingPermission) {
                    // Interrupted: the tool call it was on is over.
                    *state = AgentState::WaitingYou;
                    *activity = None;
                }
            }
        })
    }

    /// The `agent_state` message. Rule 1: the most urgent of the agent, its subagents and
    /// "with subagents" (when any is live) is shown. A seen agent is not pending.
    pub fn message(&self, id: &str) -> Control {
        let state = self.displayed();
        Control::AgentState {
            id: id.to_owned(),
            state,
            urgency: state.urgency(),
            pending: state.pending() && !self.seen,
            subagents: self.subagents.clone(),
            activity: self.activity.clone(),
            since_ms: self.since_ms,
        }
    }

    fn displayed(&self) -> AgentState {
        let with = (!self.subagents.is_empty()).then_some(AgentState::WithSubagents);
        self.subagents
            .iter()
            .map(|s| s.state)
            .chain(with)
            .fold(self.state, Ord::max)
    }

    /// Applies `update`; when the displayed state changed, the agent is seen only if it just
    /// finished (working or with subagents → waiting for you) while watched (hive.md item 5).
    /// A state that changed (the displayed one, or a subagent's) begins at `now`.
    fn changed(
        &mut self,
        id: &str,
        now: Instant,
        update: impl FnOnce(&mut Self),
    ) -> Option<Control> {
        let before = self.message(id);
        let was = self.displayed();
        let subagents: HashMap<String, AgentState> = self
            .subagents
            .iter()
            .map(|s| (s.id.clone(), s.state))
            .collect();
        update(self);
        let shown = self.displayed();
        let wall = self.wall(now);
        if shown != was {
            let busy = matches!(was, AgentState::Working | AgentState::WithSubagents);
            self.seen = self.watched && busy && shown == AgentState::WaitingYou;
            self.since_ms = wall;
        }
        for sub in &mut self.subagents {
            if subagents.get(&sub.id) != Some(&sub.state) {
                sub.since_ms = wall;
            }
        }
        let after = self.message(id);
        (after != before).then_some(after)
    }
}

/// Keeps the activity an event brings; the end of a turn, a session or a subagent clears it.
fn follow(activity: &mut Option<String>, event: &AgentEvent) {
    match event.kind {
        EventKind::TurnFinished
        | EventKind::TurnFailed { .. }
        | EventKind::SessionEnded { .. }
        | EventKind::SubagentStopped => *activity = None,
        _ => {
            if let Some(doing) = &event.activity {
                *activity = Some(doing.clone());
            }
        }
    }
}

/// The background task a `PostToolUse` started: `backgroundTaskId` (Bash
/// `run_in_background`), `taskId` (`Monitor`) or `agentId` (an `Agent`) in its `tool_response`.
fn launch(raw: &Value) -> Option<String> {
    let response = raw.get("tool_response")?;
    ["backgroundTaskId", "taskId", "agentId"]
        .iter()
        .find_map(|k| response.get(k).and_then(Value::as_str))
        .filter(|id| id.len() <= MAX_ID)
        .map(str::to_owned)
}

/// The table: which state an event leads to; `None` leaves the state as it is.
/// `PostToolUseFailure` (`ToolFailed`) is routine and means working, never error.
fn state_of(kind: &EventKind) -> Option<AgentState> {
    use AgentState::*;
    Some(match kind {
        EventKind::SessionStarted => Idle,
        EventKind::PromptSubmitted
        | EventKind::ToolStarted { .. }
        | EventKind::ToolFinished { .. }
        | EventKind::ToolFailed { .. }
        | EventKind::SubagentStarted => Working,
        EventKind::PermissionRequested { .. }
        | EventKind::Notification {
            notification: Notification::PermissionPrompt | Notification::ElicitationDialog,
        } => WaitingPermission,
        EventKind::TurnFinished
        | EventKind::Notification {
            notification: Notification::IdlePrompt | Notification::AgentNeedsInput,
        } => WaitingYou,
        EventKind::TurnFailed { .. } => Error,
        EventKind::SessionEnded { .. } => Ended,
        EventKind::SubagentStopped
        | EventKind::Notification {
            notification: Notification::Other(_),
        }
        | EventKind::WorktreeCreated { .. }
        | EventKind::WorktreeRemoved { .. }
        | EventKind::Other { .. } => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{Adapter, ClaudeCode};
    use AgentState::*;
    use serde_json::json;

    /// The `agents.silence_secs` default.
    const SILENCE: Duration = Duration::from_secs(5);

    /// A hook call as Claude Code sends it; `agent` makes it a subagent's.
    fn hook(name: &str, agent: Option<&str>, extra: serde_json::Value) -> AgentEvent {
        let mut payload = json!({"session_id": "s", "agent_type": "Explore"});
        if let Some(agent) = agent {
            payload["agent_id"] = agent.into();
        }
        for (k, v) in extra.as_object().unwrap() {
            payload[k] = v.clone();
        }
        ClaudeCode.translate(name, Some("1".into()), payload)
    }

    impl Agent {
        /// `apply` for events whose cwd is in no worktree.
        fn feed(&mut self, id: &str, event: &AgentEvent, now: Instant) -> Option<Control> {
            self.apply(id, event, now, &|_| None)
        }
    }

    fn notification(kind: &str) -> AgentEvent {
        hook("Notification", None, json!({"notification_type": kind}))
    }

    fn shown(agent: &Agent) -> (AgentState, Vec<(String, AgentState)>) {
        let subs = agent.subagents.iter().map(|s| (s.id.clone(), s.state));
        (agent.displayed(), subs.collect())
    }

    fn after(event: &AgentEvent) -> AgentState {
        let now = Instant::now();
        let mut agent = Agent::new(1, now, 0);
        agent.feed("s", event, now);
        shown(&agent).0
    }

    #[test]
    fn a_new_agent_is_idle_on_its_channel() {
        let agent = Agent::new(3, Instant::now(), 0);
        assert_eq!(agent.channel, 3);
        assert_eq!(shown(&agent), (Idle, vec![]));
    }

    #[test]
    fn events_map_to_states_per_the_table() {
        let tool = json!({"tool_name": "Bash"});
        let table = [
            ("UserPromptSubmit", Working),
            ("PreToolUse", Working),
            ("PostToolUse", Working),
            ("PostToolUseFailure", Working),
            ("PermissionRequest", WaitingPermission),
            ("Stop", WaitingYou),
            ("StopFailure", Error),
            ("SessionEnd", Ended),
        ];
        for (name, state) in table {
            assert_eq!(after(&hook(name, None, tool.clone())), state, "{name}");
        }
        let notifications = [
            ("permission_prompt", WaitingPermission),
            ("elicitation_dialog", WaitingPermission),
            ("idle_prompt", WaitingYou),
            ("agent_needs_input", WaitingYou),
        ];
        for (kind, state) in notifications {
            assert_eq!(after(&notification(kind)), state, "{kind}");
        }
    }

    #[test]
    fn session_start_makes_an_agent_idle_again() {
        let now = Instant::now();
        let mut agent = Agent::new(1, now, 0);
        agent.feed("s", &hook("Stop", None, json!({})), now);
        let idle = agent.feed("s", &hook("SessionStart", None, json!({})), now);
        assert_eq!(idle, Some(agent.message("s")));
        assert_eq!(shown(&agent).0, Idle);
    }

    #[test]
    fn events_without_a_state_change_nothing_and_send_nothing() {
        let now = Instant::now();
        let mut agent = Agent::new(1, now, 0);
        agent.feed("s", &hook("PreToolUse", None, json!({})), now);
        for event in [
            notification("auth_success"),
            hook("CwdChanged", None, json!({})),
            hook("SubagentStop", None, json!({})),
            hook("PostToolUse", None, json!({})),
        ] {
            assert_eq!(agent.feed("s", &event, now), None, "{:?}", event.kind);
            assert_eq!(shown(&agent).0, Working);
        }
    }

    #[test]
    fn a_change_returns_the_new_message() {
        let now = Instant::now();
        let mut agent = Agent::new(1, now, 1_000);
        let later = now + Duration::from_millis(5);
        let sent = agent.feed("s", &hook("UserPromptSubmit", None, json!({})), later);
        assert_eq!(
            sent,
            Some(Control::AgentState {
                id: "s".into(),
                state: Working,
                urgency: 2,
                pending: false,
                subagents: vec![],
                activity: None,
                since_ms: 1_005,
            })
        );
    }

    fn tool(name: &str, agent: Option<&str>, description: &str) -> AgentEvent {
        let input = json!({"tool_name": "Bash", "tool_input": {"description": description}});
        hook(name, agent, input)
    }

    fn activities(agent: &Agent) -> (Option<&str>, Vec<Option<&str>>) {
        let subs = agent.subagents.iter().map(|s| s.activity.as_deref());
        (agent.activity.as_deref(), subs.collect())
    }

    #[test]
    fn the_agent_keeps_its_last_activity_until_its_turn_ends() {
        let now = Instant::now();
        for end in ["Stop", "StopFailure", "SessionEnd"] {
            let mut agent = Agent::new(1, now, 0);
            let sent = agent.feed("s", &tool("PreToolUse", None, "Run tests"), now);
            let doing = Some("Run tests".to_owned());
            assert!(
                matches!(sent, Some(Control::AgentState { activity, .. }) if activity == doing)
            );
            // A tool that finishes, or an event without one, keeps it; a new one replaces it.
            agent.feed("s", &tool("PostToolUse", None, "x"), now);
            agent.feed("s", &notification("idle_prompt"), now);
            assert_eq!(activities(&agent), (Some("Run tests"), vec![]));
            let asking = agent.feed("s", &tool("PermissionRequest", None, "Deploy"), now);
            assert!(asking.is_some());
            assert_eq!(activities(&agent), (Some("Deploy"), vec![]));
            agent.feed("s", &hook(end, None, json!({})), now);
            assert_eq!(activities(&agent), (None, vec![]), "{end}");
        }
    }

    #[test]
    fn a_subagent_keeps_its_own_activity_until_it_stops() {
        let now = Instant::now();
        let mut agent = Agent::new(1, now, 0);
        agent.feed("s", &tool("PreToolUse", None, "Main"), now);
        agent.feed("s", &tool("PreToolUse", Some("a"), "First"), now);
        agent.feed("s", &hook("SubagentStart", Some("b"), json!({})), now);
        agent.feed("s", &tool("PreToolUse", Some("b"), "Second"), now);
        agent.feed("s", &tool("PermissionRequest", Some("a"), "Third"), now);
        let all = (Some("Main"), vec![Some("Third"), Some("Second")]);
        assert_eq!(activities(&agent), all);
        agent.feed("s", &hook("Stop", Some("b"), json!({})), now);
        assert_eq!(
            activities(&agent),
            (Some("Main"), vec![Some("Third"), None])
        );
        // Kept listed while its background task runs, its own turn is over.
        agent.feed("s", &launched("a", "taskId", "t1"), now);
        agent.feed("s", &stop_with("SubagentStop", Some("a"), &["t1"]), now);
        assert_eq!(activities(&agent), (Some("Main"), vec![None, None]));
    }

    #[test]
    fn silence_clears_the_activity_of_what_it_interrupts() {
        let start = Instant::now();
        let mut agent = Agent::new(1, start, 0);
        agent.feed("s", &tool("PreToolUse", None, "Main"), start);
        agent.feed("s", &tool("PreToolUse", Some("a"), "Sub"), start);
        agent.feed("s", &tool("PreToolUse", Some("b"), "Busy"), start);
        agent.feed("s", &launched("b", "taskId", "t1"), start);
        agent.feed("s", &stop_with("SubagentStop", Some("b"), &["t1"]), start);
        agent.feed("s", &hook("SubagentStart", Some("c"), json!({})), start);
        agent.feed("s", &tool("PreToolUse", Some("c"), "Done"), start);
        agent.feed("s", &hook("SessionStart", Some("c"), json!({})), start);
        assert_eq!(
            activities(&agent),
            (Some("Main"), vec![Some("Sub"), None, Some("Done")])
        );
        assert!(agent.reconcile("s", start, start + SILENCE).is_some());
        // An idle subagent is not interrupted, so it keeps what it last did.
        assert_eq!(activities(&agent), (None, vec![None, None, Some("Done")]));
    }

    fn since(agent: &Agent) -> (u64, Vec<u64>) {
        let subs = agent.subagents.iter().map(|s| s.since_ms);
        (agent.since_ms, subs.collect())
    }

    #[test]
    fn each_state_begins_on_the_wall_clock_when_it_changes() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut agent = Agent::new(1, start, 50_000);
        assert_eq!(since(&agent), (50_000, vec![]));
        agent.feed("s", &hook("UserPromptSubmit", None, json!({})), at(10));
        // Still working: the state goes on.
        agent.feed("s", &hook("PreToolUse", None, json!({})), at(20));
        assert_eq!(since(&agent), (50_010, vec![]));
        agent.feed("s", &hook("SubagentStart", Some("a"), json!({})), at(30));
        agent.feed("s", &hook("PreToolUse", Some("a"), json!({})), at(40));
        agent.feed("s", &hook("SubagentStart", Some("b"), json!({})), at(50));
        assert_eq!(since(&agent), (50_030, vec![50_030, 50_050]));
        agent.feed(
            "s",
            &hook("PermissionRequest", Some("b"), json!({})),
            at(60),
        );
        assert_eq!(since(&agent), (50_060, vec![50_030, 50_060]));
        // A clock going back never makes a time before the agent began.
        agent.feed(
            "s",
            &hook("PostToolUse", Some("b"), json!({})),
            start - Duration::from_millis(1),
        );
        assert_eq!(since(&agent), (50_000, vec![50_030, 50_000]));
        assert!(agent.reconcile("s", at(60), at(60) + SILENCE).is_some());
        assert_eq!(since(&agent), (55_060, vec![55_060, 55_060]));
    }

    #[test]
    fn subagents_are_listed_until_they_stop() {
        let now = Instant::now();
        let mut agent = Agent::new(1, now, 0);
        agent.feed("s", &hook("PreToolUse", None, json!({})), now);
        let sent = agent.feed("s", &hook("SubagentStart", Some("a"), json!({})), now);
        assert_eq!(
            sent,
            Some(Control::AgentState {
                id: "s".into(),
                state: WithSubagents,
                urgency: 3,
                pending: false,
                subagents: vec![SubagentState {
                    id: "a".into(),
                    agent_type: Some("Explore".into()),
                    state: Working,
                    worktree: None,
                    activity: None,
                    since_ms: 0,
                }],
                activity: None,
                since_ms: 0,
            })
        );
        agent.feed("s", &hook("SubagentStart", Some("b"), json!({})), now);
        agent.feed("s", &hook("Stop", Some("b"), json!({})), now);
        let both = vec![("a".into(), Working), ("b".into(), WaitingYou)];
        assert_eq!(shown(&agent), (WaitingYou, both));
        agent.feed("s", &hook("SubagentStop", Some("b"), json!({})), now);
        assert_eq!(shown(&agent), (WithSubagents, vec![("a".into(), Working)]));
        agent.feed("s", &hook("SessionEnd", Some("a"), json!({})), now);
        assert_eq!(shown(&agent), (Working, vec![]));
    }

    #[test]
    fn the_most_urgent_state_wins() {
        let now = Instant::now();
        let mut agent = Agent::new(1, now, 0);
        agent.feed("s", &hook("SubagentStart", Some("a"), json!({})), now);
        agent.feed("s", &hook("SubagentStart", Some("b"), json!({})), now);
        agent.feed("s", &hook("PermissionRequest", Some("b"), json!({})), now);
        assert_eq!(shown(&agent).0, WaitingPermission);
        agent.feed("s", &hook("StopFailure", None, json!({})), now);
        assert_eq!(shown(&agent).0, WaitingPermission);
        agent.feed("s", &hook("PostToolUse", Some("b"), json!({})), now);
        assert_eq!(shown(&agent).0, Error);
        agent.feed("s", &hook("Stop", None, json!({})), now);
        assert_eq!(shown(&agent).0, WaitingYou);
        agent.feed("s", &hook("SessionStart", None, json!({})), now);
        assert_eq!(shown(&agent).0, WithSubagents);
    }

    #[test]
    fn subagent_events_without_a_state_keep_it() {
        let now = Instant::now();
        let mut agent = Agent::new(1, now, 0);
        agent.feed("s", &hook("PermissionRequest", Some("a"), json!({})), now);
        let other = hook("Notification", Some("a"), json!({"notification_type": "x"}));
        assert_eq!(agent.feed("s", &other, now), None);
        // An unknown subagent without a state is not added.
        let unknown = hook("Notification", Some("z"), json!({"notification_type": "x"}));
        assert_eq!(agent.feed("s", &unknown, now), None);
        assert_eq!(
            shown(&agent),
            (WaitingPermission, vec![("a".into(), WaitingPermission)])
        );
    }

    #[test]
    fn subagents_are_limited_in_number_and_id_length() {
        let now = Instant::now();
        let mut agent = Agent::new(1, now, 0);
        let long = "x".repeat(MAX_ID + 1);
        let at_limit = "y".repeat(MAX_ID);
        let start =
            |id: &str, kind: &str| hook("SubagentStart", Some(id), json!({"agent_type": kind}));
        assert_eq!(agent.feed("s", &start(&long, "t"), now), None);
        assert_eq!(agent.feed("s", &start("a", &long), now), None);
        assert!(agent.feed("s", &start(&at_limit, &at_limit), now).is_some());
        for n in 1..MAX_SUBAGENTS {
            assert!(agent.feed("s", &start(&n.to_string(), "t"), now).is_some());
        }
        assert_eq!(agent.feed("s", &start("last", "t"), now), None);
        assert_eq!(shown(&agent).1.len(), MAX_SUBAGENTS);
        // A subagent without a type is kept.
        agent.feed("s", &hook("SubagentStop", Some("1"), json!({})), now);
        let untyped = ClaudeCode.translate(
            "SubagentStart",
            None,
            json!({"session_id": "s", "agent_id": "u"}),
        );
        assert!(agent.feed("s", &untyped, now).is_some());
    }

    #[test]
    fn silence_turns_working_and_waiting_permission_into_waiting_you() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut agent = Agent::new(1, start, 0);
        agent.feed("s", &hook("PreToolUse", None, json!({})), at(1_000));
        agent.feed("s", &hook("SubagentStart", Some("a"), json!({})), at(1_000));
        agent.feed(
            "s",
            &hook("PermissionRequest", Some("b"), json!({})),
            at(1_000),
        );
        agent.feed("s", &hook("Stop", Some("c"), json!({})), at(1_000));
        // Counted from the later of the last output and the last hook event, for as long as
        // the setting says.
        let longer = Duration::from_secs(10);
        assert_eq!(agent.reconcile("s", longer, at(2_000), at(7_000)), None);
        assert_eq!(agent.reconcile("s", SILENCE, at(2_000), at(6_999)), None);
        assert_eq!(agent.reconcile("s", SILENCE, start, at(5_999)), None);
        let sent = agent.reconcile("s", SILENCE, at(2_000), at(7_000));
        assert_eq!(sent, Some(agent.message("s")));
        let all_waiting = ["a", "b", "c"]
            .map(|id| (id.to_owned(), WaitingYou))
            .to_vec();
        assert_eq!(shown(&agent), (WaitingYou, all_waiting));
        // Nothing left to change; output alone does not move it back.
        assert_eq!(agent.reconcile("s", SILENCE, at(7_000), at(20_000)), None);
        assert_eq!(shown(&agent).0, WaitingYou);
    }

    #[test]
    fn silence_leaves_other_states_alone() {
        let start = Instant::now();
        let late = start + SILENCE * 2;
        for (name, state) in [
            ("SessionStart", Idle),
            ("StopFailure", Error),
            ("SessionEnd", Ended),
        ] {
            let mut agent = Agent::new(1, start, 0);
            agent.feed("s", &hook(name, None, json!({})), start);
            assert_eq!(agent.reconcile("s", SILENCE, start, late), None, "{name}");
            assert_eq!(shown(&agent).0, state);
        }
        // Output newer than `now` (clock races) counts as no silence.
        let mut agent = Agent::new(1, start, 0);
        agent.feed("s", &hook("PreToolUse", None, json!({})), start);
        assert_eq!(agent.reconcile("s", SILENCE, late, start), None);
    }

    fn pending(agent: &Agent) -> bool {
        matches!(
            agent.message("s"),
            Control::AgentState { pending: true, .. }
        )
    }

    #[test]
    fn finishing_while_watched_is_seen_until_the_state_changes() {
        let now = Instant::now();
        let mut agent = Agent::new(1, now, 0);
        agent.watched = true;
        agent.feed("s", &hook("PreToolUse", None, json!({})), now);
        let sent = agent.feed("s", &hook("Stop", None, json!({})), now);
        assert_eq!(sent, Some(agent.message("s")));
        assert_eq!((shown(&agent).0, pending(&agent)), (WaitingYou, false));
        // Looking away later keeps it seen, and so do events that change nothing shown.
        agent.watched = false;
        agent.feed("s", &hook("Stop", None, json!({})), now);
        assert!(!pending(&agent));
        // Its next change makes it pending again as usual.
        agent.feed("s", &hook("StopFailure", None, json!({})), now);
        assert_eq!((shown(&agent).0, pending(&agent)), (Error, true));
        agent.feed("s", &hook("PreToolUse", None, json!({})), now);
        agent.feed("s", &hook("Stop", None, json!({})), now);
        assert_eq!((shown(&agent).0, pending(&agent)), (WaitingYou, true));
    }

    #[test]
    fn only_finishing_while_watched_is_seen() {
        let now = Instant::now();
        let watched = |events: &[AgentEvent]| {
            let mut agent = Agent::new(1, now, 0);
            agent.watched = true;
            for event in events {
                agent.feed("s", event, now);
            }
            (shown(&agent).0, pending(&agent))
        };
        let stop = hook("Stop", None, json!({}));
        let with = hook("SubagentStart", Some("a"), json!({}));
        let sub_stop = hook("Stop", Some("a"), json!({}));
        assert_eq!(watched(&[with, sub_stop]), (WaitingYou, false));
        // From idle it did not finish anything; asking permission is not finishing.
        assert_eq!(watched(&[notification("idle_prompt")]), (WaitingYou, true));
        let ask = hook("PermissionRequest", None, json!({}));
        let tool = hook("PreToolUse", None, json!({}));
        assert_eq!(watched(&[tool, ask, stop]), (WaitingYou, true));
        // Interrupted (silence) while watched is finishing too.
        let mut agent = Agent::new(1, now, 0);
        agent.watched = true;
        agent.feed("s", &hook("PreToolUse", None, json!({})), now);
        assert!(agent.reconcile("s", SILENCE, now, now + SILENCE).is_some());
        assert_eq!((shown(&agent).0, pending(&agent)), (WaitingYou, false));
    }

    fn worktrees(agent: &Agent) -> Vec<(&str, Option<&str>)> {
        let subs = agent.subagents.iter();
        subs.map(|s| (s.id.as_str(), s.worktree.as_deref()))
            .collect()
    }

    #[test]
    fn a_worktree_hook_naming_a_subagent_gives_it_that_worktree_until_removed() {
        let now = Instant::now();
        let mut agent = Agent::new(1, now, 0);
        let create = |sub: &str, path: &str| {
            hook("WorktreeCreate", Some(sub), json!({"worktree_path": path}))
        };
        // Unknown subagents are not added by it.
        assert_eq!(agent.feed("s", &create("a", "/r/w"), now), None);
        agent.feed("s", &hook("SubagentStart", Some("a"), json!({})), now);
        agent.feed("s", &hook("SubagentStart", Some("b"), json!({})), now);
        let sent = agent.feed("s", &create("a", "/r/w"), now);
        assert_eq!(sent, Some(agent.message("s")));
        assert_eq!(worktrees(&agent), [("a", Some("/r/w")), ("b", None)]);
        // Other events keep it; a path without its worktree or too long is ignored.
        agent.feed("s", &hook("PreToolUse", Some("a"), json!({})), now);
        let bare = hook("WorktreeCreate", Some("b"), json!({}));
        assert_eq!(agent.feed("s", &bare, now), None);
        let long = format!("/{}", "x".repeat(MAX_PATH));
        assert_eq!(agent.feed("s", &create("b", &long), now), None);
        let at_limit = &long[1..];
        assert!(agent.feed("s", &create("b", at_limit), now).is_some());
        assert_eq!(
            worktrees(&agent),
            [("a", Some("/r/w")), ("b", Some(at_limit))]
        );
        // Its removal, from the agent or a subagent, unlinks only that worktree.
        let remove = |path: &str| hook("WorktreeRemove", None, json!({"worktree_path": path}));
        assert_eq!(agent.feed("s", &remove("/r/other"), now), None);
        assert!(agent.feed("s", &remove("/r/w"), now).is_some());
        assert_eq!(worktrees(&agent), [("a", None), ("b", Some(at_limit))]);
        let by_sub = hook(
            "WorktreeRemove",
            Some("b"),
            json!({"worktree_path": at_limit}),
        );
        assert!(agent.feed("s", &by_sub, now).is_some());
        assert_eq!(worktrees(&agent), [("a", None), ("b", None)]);
    }

    #[test]
    fn a_subagent_working_in_another_worktree_owns_it() {
        let now = Instant::now();
        let mut agent = Agent::new(1, now, 0);
        let at = |cwd: &str| hook("PreToolUse", Some("a"), json!({"cwd": cwd}));
        let asked = std::cell::Cell::new(0);
        let place = |cwd: &str| {
            asked.set(asked.get() + 1);
            let own = cwd.starts_with("/r/w");
            Some(if own { "/r/w" } else { "/r" }.to_owned())
        };
        // Without the agent's own worktree nothing is placed.
        assert!(agent.apply("s", &at("/r/w"), now, &place).is_some());
        assert_eq!((asked.get(), worktrees(&agent)), (0, vec![("a", None)]));
        agent.worktree = Some("/r".into());
        // In the agent's worktree: not its own; the same cwd is not placed twice.
        assert_eq!(agent.apply("s", &at("/r/src"), now, &place), None);
        assert_eq!(agent.apply("s", &at("/r/src"), now, &place), None);
        assert_eq!((asked.get(), worktrees(&agent)), (1, vec![("a", None)]));
        let sent = agent.apply("s", &at("/r/w/src"), now, &place);
        assert_eq!(sent, Some(agent.message("s")));
        assert_eq!(worktrees(&agent), [("a", Some("/r/w"))]);
        // Once linked, it keeps it wherever it goes.
        assert_eq!(agent.apply("s", &at("/r"), now, &place), None);
        assert_eq!(asked.get(), 2);
        // Leaving forgets it all: back with the same cwd, it is placed again.
        agent.feed("s", &hook("SubagentStop", Some("a"), json!({})), now);
        agent.apply("s", &at("/r/w/src"), now, &place);
        assert_eq!(
            (asked.get(), worktrees(&agent)),
            (3, vec![("a", Some("/r/w"))])
        );
        // A subagent without a cwd, or with one too long, is not placed.
        let b = |extra| hook("PreToolUse", Some("b"), extra);
        agent.apply("s", &b(json!({})), now, &place);
        let long = format!("/r/w/{}", "x".repeat(MAX_PATH));
        agent.apply("s", &b(json!({"cwd": long})), now, &place);
        assert_eq!(asked.get(), 3);
        let at_limit = &long[..MAX_PATH];
        agent.apply("s", &b(json!({"cwd": at_limit})), now, &place);
        assert_eq!(asked.get(), 4);
        assert_eq!(
            worktrees(&agent),
            [("a", Some("/r/w")), ("b", Some("/r/w"))]
        );
    }

    /// `PostToolUse` of a subagent's tool that started background task `task` under `key`.
    fn launched(sub: &str, key: &str, task: &str) -> AgentEvent {
        hook(
            "PostToolUse",
            Some(sub),
            json!({"tool_response": {key: task}}),
        )
    }

    /// A stop carrying Claude Code's list of the session's running background tasks.
    fn stop_with(name: &str, sub: Option<&str>, live: &[&str]) -> AgentEvent {
        let tasks: Vec<_> = live
            .iter()
            .map(|id| json!({"id": id, "status": "running"}))
            .collect();
        hook(name, sub, json!({"background_tasks": tasks}))
    }

    #[test]
    fn a_subagent_waiting_on_its_background_task_stays_listed_until_it_is_done() {
        let start = Instant::now();
        let mut agent = Agent::new(1, start, 0);
        agent.feed("s", &hook("SubagentStart", Some("a"), json!({})), start);
        let bash = json!({"tool_name": "Bash", "tool_input": {"run_in_background": true}});
        agent.feed("s", &hook("PreToolUse", Some("a"), bash), start);
        agent.feed("s", &launched("a", "backgroundTaskId", "b1"), start);
        // Silence before it stops makes it waiting for you; stopping to wait makes it working.
        agent.reconcile("s", SILENCE, start, start + SILENCE);
        let late = start + SILENCE;
        let stop = stop_with("SubagentStop", Some("a"), &["a", "b1"]);
        assert!(agent.feed("s", &stop, late).is_some());
        assert_eq!(shown(&agent), (WithSubagents, vec![("a".into(), Working)]));
        // It stays working through silence, the agent's own events and tasks listed again.
        assert_eq!(
            agent.reconcile("s", SILENCE, late, late + SILENCE * 2),
            None
        );
        agent.feed("s", &hook("PreToolUse", None, json!({})), late);
        agent.feed("s", &stop_with("Stop", None, &["b1"]), late);
        assert_eq!(shown(&agent).1, [("a".into(), Working)]);
        // Woken by the notice it starts again, waits on a monitor, then stops with none left.
        agent.feed("s", &hook("SubagentStart", Some("a"), json!({})), late);
        agent.feed("s", &launched("a", "taskId", "m1"), late);
        agent.feed("s", &stop_with("SubagentStop", Some("a"), &["m1"]), late);
        assert_eq!(shown(&agent).1, [("a".into(), Working)]);
        agent.feed("s", &hook("SubagentStart", Some("a"), json!({})), late);
        agent.feed("s", &stop_with("SubagentStop", Some("a"), &["b1"]), late);
        assert_eq!(shown(&agent), (WaitingYou, vec![]));
        // Another subagent's running tasks do not keep one; neither does a stop without a list.
        agent.feed("s", &hook("SubagentStart", Some("b"), json!({})), late);
        agent.feed("s", &launched("b", "agentId", "x"), late);
        agent.feed(
            "s",
            &stop_with("SubagentStop", Some("b"), &["b1", "m1"]),
            late,
        );
        agent.feed("s", &hook("SubagentStart", Some("c"), json!({})), late);
        agent.feed("s", &launched("c", "backgroundTaskId", "b2"), late);
        agent.feed("s", &hook("SubagentStop", Some("c"), json!({})), late);
        assert_eq!(shown(&agent), (WaitingYou, vec![]));
    }

    #[test]
    fn a_waiting_subagent_leaves_when_its_tasks_end_its_worktree_goes_or_it_ends() {
        let now = Instant::now();
        let mut agent = Agent::new(1, now, 0);
        for sub in ["a", "b", "c", "d"] {
            agent.feed("s", &hook("SubagentStart", Some(sub), json!({})), now);
            let task = format!("t{sub}");
            agent.feed("s", &launched(sub, "backgroundTaskId", &task), now);
        }
        let create = |sub| {
            hook(
                "WorktreeCreate",
                Some(sub),
                json!({"worktree_path": "/r/w"}),
            )
        };
        agent.feed("s", &create("b"), now);
        agent.feed("s", &create("c"), now);
        let all = ["ta", "tb", "tc", "td"];
        for sub in ["a", "b", "d"] {
            agent.feed("s", &stop_with("SubagentStop", Some(sub), &all), now);
        }
        assert_eq!(shown(&agent).1.len(), 4);
        // Its worktree removed: the waiting one leaves, the working one only loses it.
        let remove = hook("WorktreeRemove", None, json!({"worktree_path": "/r/w"}));
        agent.feed("s", &remove, now);
        assert_eq!(worktrees(&agent), [("a", None), ("c", None), ("d", None)]);
        // A later list without its tasks (here from the agent's `Stop`).
        agent.feed("s", &stop_with("Stop", None, &["tc", "td"]), now);
        assert_eq!(worktrees(&agent), [("c", None), ("d", None)]);
        agent.feed("s", &hook("SessionEnd", Some("d"), json!({})), now);
        assert_eq!(worktrees(&agent), [("c", None)]);
    }

    #[test]
    fn background_launches_are_limited_in_number_and_id_length() {
        let now = Instant::now();
        let kept = |launches: usize, live: &str| {
            let mut agent = Agent::new(1, now, 0);
            agent.feed("s", &hook("SubagentStart", Some("a"), json!({})), now);
            for n in 0..launches {
                let task = format!("t{n}");
                agent.feed("s", &launched("a", "backgroundTaskId", &task), now);
            }
            agent.feed("s", &stop_with("SubagentStop", Some("a"), &[live]), now);
            !shown(&agent).1.is_empty()
        };
        assert!(kept(MAX_LAUNCHED, "t0"));
        assert!(!kept(MAX_LAUNCHED + 1, "t0"));
        assert!(kept(MAX_LAUNCHED + 1, &format!("t{MAX_LAUNCHED}")));
        let mut agent = Agent::new(1, now, 0);
        agent.feed("s", &hook("SubagentStart", Some("a"), json!({})), now);
        let long = "x".repeat(MAX_ID + 1);
        agent.feed("s", &launched("a", "taskId", &long), now);
        agent.feed("s", &stop_with("SubagentStop", Some("a"), &[&long]), now);
        assert!(shown(&agent).1.is_empty());
        agent.feed("s", &hook("SubagentStart", Some("a"), json!({})), now);
        agent.feed("s", &launched("a", "taskId", &long[1..]), now);
        agent.feed(
            "s",
            &stop_with("SubagentStop", Some("a"), &[&long[1..]]),
            now,
        );
        assert_eq!(shown(&agent).1.len(), 1);
    }
}
