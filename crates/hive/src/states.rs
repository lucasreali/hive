//! Agent states from hook events ("Mapeamento de estados" in `docs/hive.md`). Pure logic: the
//! daemon feeds events and the clock in, and sends the resulting `agent_state` messages.

use std::time::{Duration, Instant};

use hive_protocol::{AgentEvent, AgentState, Control, EventKind, Notification, SubagentState};

/// Rule 2: an agent working or waiting for permission whose terminal prints nothing for this
/// long was interrupted (Esc/Ctrl+C fire no `Stop`), so it waits for you.
pub const SILENCE: Duration = Duration::from_secs(5);

/// Subagents kept per agent; later ones are ignored, so a message always fits in a frame.
const MAX_SUBAGENTS: usize = 32;

/// Longest subagent id or type kept (real ones are short); longer ones are ignored.
const MAX_ID: usize = 256;

/// A detected agent: its terminal and the state of it and its live subagents.
#[derive(Debug)]
pub struct Agent {
    /// Terminal channel (`HIVE_TERMINAL_ID`) the agent runs in.
    pub channel: u32,
    state: AgentState,
    /// In start order.
    subagents: Vec<SubagentState>,
    /// Last hook event for the agent or a subagent; silence is counted from here at the latest.
    last_event: Instant,
}

impl Agent {
    /// An agent that just sent `SessionStart`.
    pub fn new(channel: u32, now: Instant) -> Self {
        Self {
            channel,
            state: AgentState::Idle,
            subagents: Vec::new(),
            last_event: now,
        }
    }

    /// Applies one hook event of this agent (or of one of its subagents). Returns the new
    /// message when it changed.
    pub fn apply(&mut self, id: &str, event: &AgentEvent, now: Instant) -> Option<Control> {
        self.changed(id, |agent| {
            agent.last_event = now;
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
                return;
            }
            let Some(state) = state else { return };
            if let Some(known) = agent.subagents.iter_mut().find(|s| s.id == sub.id) {
                known.state = state;
            } else if agent.subagents.len() < MAX_SUBAGENTS
                && sub.id.len() <= MAX_ID
                && sub.agent_type.as_ref().is_none_or(|t| t.len() <= MAX_ID)
            {
                agent.subagents.push(SubagentState {
                    id: sub.id.clone(),
                    agent_type: sub.agent_type.clone(),
                    state,
                });
            }
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
        agent.apply("s", event, now);
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
        agent.apply("s", &hook("Stop", None, json!({})), now);
        let idle = agent.apply("s", &hook("SessionStart", None, json!({})), now);
        assert_eq!(idle, Some(agent.message("s")));
        assert_eq!(shown(&agent).0, Idle);
    }

    #[test]
    fn events_without_a_state_change_nothing_and_send_nothing() {
        let now = Instant::now();
        let mut agent = Agent::new(1, now);
        agent.apply("s", &hook("PreToolUse", None, json!({})), now);
        for event in [
            notification("auth_success"),
            hook("CwdChanged", None, json!({})),
            hook("SubagentStop", None, json!({})),
            hook("PostToolUse", None, json!({})),
        ] {
            assert_eq!(agent.apply("s", &event, now), None, "{:?}", event.kind);
            assert_eq!(shown(&agent).0, Working);
        }
    }

    #[test]
    fn a_change_returns_the_new_message() {
        let now = Instant::now();
        let mut agent = Agent::new(1, now);
        let sent = agent.apply("s", &hook("UserPromptSubmit", None, json!({})), now);
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
        agent.apply("s", &hook("PreToolUse", None, json!({})), now);
        let sent = agent.apply("s", &hook("SubagentStart", Some("a"), json!({})), now);
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
                }],
            })
        );
        agent.apply("s", &hook("SubagentStart", Some("b"), json!({})), now);
        agent.apply("s", &hook("Stop", Some("b"), json!({})), now);
        let both = vec![("a".into(), Working), ("b".into(), WaitingYou)];
        assert_eq!(shown(&agent), (WaitingYou, both));
        agent.apply("s", &hook("SubagentStop", Some("b"), json!({})), now);
        assert_eq!(shown(&agent), (WithSubagents, vec![("a".into(), Working)]));
        agent.apply("s", &hook("SessionEnd", Some("a"), json!({})), now);
        assert_eq!(shown(&agent), (Working, vec![]));
    }

    #[test]
    fn the_most_urgent_state_wins() {
        let now = Instant::now();
        let mut agent = Agent::new(1, now);
        agent.apply("s", &hook("SubagentStart", Some("a"), json!({})), now);
        agent.apply("s", &hook("SubagentStart", Some("b"), json!({})), now);
        agent.apply("s", &hook("PermissionRequest", Some("b"), json!({})), now);
        assert_eq!(shown(&agent).0, WaitingPermission);
        agent.apply("s", &hook("StopFailure", None, json!({})), now);
        assert_eq!(shown(&agent).0, WaitingPermission);
        agent.apply("s", &hook("PostToolUse", Some("b"), json!({})), now);
        assert_eq!(shown(&agent).0, Error);
        agent.apply("s", &hook("Stop", None, json!({})), now);
        assert_eq!(shown(&agent).0, WaitingYou);
        agent.apply("s", &hook("SessionStart", None, json!({})), now);
        assert_eq!(shown(&agent).0, WithSubagents);
    }

    #[test]
    fn subagent_events_without_a_state_keep_it() {
        let now = Instant::now();
        let mut agent = Agent::new(1, now);
        agent.apply("s", &hook("PermissionRequest", Some("a"), json!({})), now);
        let other = hook("Notification", Some("a"), json!({"notification_type": "x"}));
        assert_eq!(agent.apply("s", &other, now), None);
        // An unknown subagent without a state is not added.
        let unknown = hook("Notification", Some("z"), json!({"notification_type": "x"}));
        assert_eq!(agent.apply("s", &unknown, now), None);
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
        assert_eq!(agent.apply("s", &start(&long, "t"), now), None);
        assert_eq!(agent.apply("s", &start("a", &long), now), None);
        assert!(
            agent
                .apply("s", &start(&at_limit, &at_limit), now)
                .is_some()
        );
        for n in 1..MAX_SUBAGENTS {
            assert!(agent.apply("s", &start(&n.to_string(), "t"), now).is_some());
        }
        assert_eq!(agent.apply("s", &start("last", "t"), now), None);
        assert_eq!(shown(&agent).1.len(), MAX_SUBAGENTS);
        // A subagent without a type is kept.
        agent.apply("s", &hook("SubagentStop", Some("1"), json!({})), now);
        let untyped = ClaudeCode.translate(
            "SubagentStart",
            None,
            json!({"session_id": "s", "agent_id": "u"}),
        );
        assert!(agent.apply("s", &untyped, now).is_some());
    }

    #[test]
    fn silence_turns_working_and_waiting_permission_into_waiting_you() {
        let start = Instant::now();
        let at = |ms| start + Duration::from_millis(ms);
        let mut agent = Agent::new(1, start);
        agent.apply("s", &hook("PreToolUse", None, json!({})), at(1_000));
        agent.apply("s", &hook("SubagentStart", Some("a"), json!({})), at(1_000));
        agent.apply(
            "s",
            &hook("PermissionRequest", Some("b"), json!({})),
            at(1_000),
        );
        agent.apply("s", &hook("Stop", Some("c"), json!({})), at(1_000));
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
            agent.apply("s", &hook(name, None, json!({})), start);
            assert_eq!(agent.reconcile("s", start, late), None, "{name}");
            assert_eq!(shown(&agent).0, state);
        }
        // Output newer than `now` (clock races) counts as no silence.
        let mut agent = Agent::new(1, start);
        agent.apply("s", &hook("PreToolUse", None, json!({})), start);
        assert_eq!(agent.reconcile("s", late, start), None);
    }
}
