//! Agent states from hook events ("Mapeamento de estados" in `docs/hive.md`). Pure logic: the
//! daemon feeds events and the clock in, and sends the resulting `agent_state` messages.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use hive_protocol::{AgentEvent, AgentState, Control, EventKind, Notification, SubagentState};

/// Rule 2: an agent working or waiting for permission whose terminal prints nothing for this
/// long was interrupted (Esc/Ctrl+C fire no `Stop`), so it waits for you.
pub const SILENCE: Duration = Duration::from_secs(5);

/// Subagents kept per agent; later ones are ignored, so a message always fits in a frame.
const MAX_SUBAGENTS: usize = 32;

/// Longest subagent id or type kept (real ones are short); longer ones are ignored.
const MAX_ID: usize = 256;

/// Longest worktree path or subagent cwd used to link a subagent to its own worktree.
const MAX_PATH: usize = 4096;

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
    /// Last hook event for the agent or a subagent; silence is counted from here at the latest.
    last_event: Instant,
}

impl Agent {
    /// An agent that just sent `SessionStart`.
    pub fn new(channel: u32, now: Instant) -> Self {
        Self {
            channel,
            worktree: None,
            cwd: None,
            title: None,
            state: AgentState::Idle,
            subagents: Vec::new(),
            placed: HashMap::new(),
            last_event: now,
        }
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
    pub fn apply(
        &mut self,
        id: &str,
        event: &AgentEvent,
        now: Instant,
        place: &dyn Fn(&str) -> Option<String>,
    ) -> Option<Control> {
        self.changed(id, |agent| {
            agent.last_event = now;
            if let EventKind::WorktreeRemoved { path: Some(path) } = &event.kind {
                for sub in &mut agent.subagents {
                    if sub.worktree.as_ref() == Some(path) {
                        sub.worktree = None;
                    }
                }
            }
            let state = state_of(&event.kind);
            let Some(sub) = &event.subagent else {
                if let Some(state) = state {
                    agent.state = state;
                }
                return;
            };
            if matches!(
                event.kind,
                EventKind::SubagentStopped | EventKind::SessionEnded { .. }
            ) {
                agent.subagents.retain(|s| s.id != sub.id);
                agent.placed.remove(&sub.id);
                return;
            }
            let known = agent.subagents.iter().position(|s| s.id == sub.id);
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
                    });
                    agent.subagents.len() - 1
                }
                None => return,
            };
            let known = &mut agent.subagents[i];
            known.state = state;
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

    /// Rule 2, checked periodically: after [`SILENCE`] without terminal output (nor hook
    /// events), working and waiting-for-permission become waiting for you. Output alone never
    /// moves a state back; only hook events do.
    pub fn reconcile(&mut self, id: &str, last_output: Instant, now: Instant) -> Option<Control> {
        if now.saturating_duration_since(last_output.max(self.last_event)) < SILENCE {
            return None;
        }
        self.changed(id, |agent| {
            let subagents = agent.subagents.iter_mut().map(|s| &mut s.state);
            for state in std::iter::once(&mut agent.state).chain(subagents) {
                if matches!(state, AgentState::Working | AgentState::WaitingPermission) {
                    *state = AgentState::WaitingYou;
                }
            }
        })
    }

    /// The `agent_state` message. Rule 1: the most urgent of the agent, its subagents and
    /// "with subagents" (when any is live) is shown.
    pub fn message(&self, id: &str) -> Control {
        let state = self.displayed();
        Control::AgentState {
            id: id.to_owned(),
            state,
            urgency: state.urgency(),
            pending: state.pending(),
            subagents: self.subagents.clone(),
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

    fn changed(&mut self, id: &str, update: impl FnOnce(&mut Self)) -> Option<Control> {
        let before = self.message(id);
        update(self);
        let after = self.message(id);
        (after != before).then_some(after)
    }
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
        let mut agent = Agent::new(1, now);
        agent.feed("s", event, now);
        shown(&agent).0
    }

    #[test]
    fn a_new_agent_is_idle_on_its_channel() {
        let agent = Agent::new(3, Instant::now());
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
        let mut agent = Agent::new(1, now);
        agent.feed("s", &hook("Stop", None, json!({})), now);
        let idle = agent.feed("s", &hook("SessionStart", None, json!({})), now);
        assert_eq!(idle, Some(agent.message("s")));
        assert_eq!(shown(&agent).0, Idle);
    }

    #[test]
    fn events_without_a_state_change_nothing_and_send_nothing() {
        let now = Instant::now();
        let mut agent = Agent::new(1, now);
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
        let mut agent = Agent::new(1, now);
        let sent = agent.feed("s", &hook("UserPromptSubmit", None, json!({})), now);
        assert_eq!(
            sent,
            Some(Control::AgentState {
                id: "s".into(),
                state: Working,
                urgency: 2,
                pending: false,
                subagents: vec![],
            })
        );
    }

    #[test]
    fn subagents_are_listed_until_they_stop() {
        let now = Instant::now();
        let mut agent = Agent::new(1, now);
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
                }],
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
        let mut agent = Agent::new(1, now);
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
        let mut agent = Agent::new(1, now);
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
        let mut agent = Agent::new(1, now);
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
        let mut agent = Agent::new(1, start);
        agent.feed("s", &hook("PreToolUse", None, json!({})), at(1_000));
        agent.feed("s", &hook("SubagentStart", Some("a"), json!({})), at(1_000));
        agent.feed(
            "s",
            &hook("PermissionRequest", Some("b"), json!({})),
            at(1_000),
        );
        agent.feed("s", &hook("Stop", Some("c"), json!({})), at(1_000));
        // Counted from the later of the last output and the last hook event.
        assert_eq!(agent.reconcile("s", at(2_000), at(6_999)), None);
        assert_eq!(agent.reconcile("s", start, at(5_999)), None);
        let sent = agent.reconcile("s", at(2_000), at(7_000));
        assert_eq!(sent, Some(agent.message("s")));
        let all_waiting = ["a", "b", "c"]
            .map(|id| (id.to_owned(), WaitingYou))
            .to_vec();
        assert_eq!(shown(&agent), (WaitingYou, all_waiting));
        // Nothing left to change; output alone does not move it back.
        assert_eq!(agent.reconcile("s", at(7_000), at(20_000)), None);
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
            let mut agent = Agent::new(1, start);
            agent.feed("s", &hook(name, None, json!({})), start);
            assert_eq!(agent.reconcile("s", start, late), None, "{name}");
            assert_eq!(shown(&agent).0, state);
        }
        // Output newer than `now` (clock races) counts as no silence.
        let mut agent = Agent::new(1, start);
        agent.feed("s", &hook("PreToolUse", None, json!({})), start);
        assert_eq!(agent.reconcile("s", late, start), None);
    }

    fn worktrees(agent: &Agent) -> Vec<(&str, Option<&str>)> {
        let subs = agent.subagents.iter();
        subs.map(|s| (s.id.as_str(), s.worktree.as_deref()))
            .collect()
    }

    #[test]
    fn a_worktree_hook_naming_a_subagent_gives_it_that_worktree_until_removed() {
        let now = Instant::now();
        let mut agent = Agent::new(1, now);
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
        let mut agent = Agent::new(1, now);
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
}
