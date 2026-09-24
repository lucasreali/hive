//! Adapters translate raw provider hook payloads into the internal event model.

use hive_protocol::{AgentEvent, EventKind, Notification, Subagent};
use serde_json::Value;

/// One adapter per agent provider (Claude Code today).
pub trait Adapter {
    /// Translates one hook call. The raw payload is kept in the event.
    fn translate(&self, event: &str, terminal_id: Option<String>, payload: Value) -> AgentEvent;
}

pub struct ClaudeCode;

impl Adapter for ClaudeCode {
    fn translate(&self, event: &str, terminal_id: Option<String>, payload: Value) -> AgentEvent {
        let field = |name: &str| payload.get(name).and_then(Value::as_str).map(str::to_owned);
        let tool = field("tool_name");
        let kind = match event {
            "SessionStart" => EventKind::SessionStarted,
            "UserPromptSubmit" => EventKind::PromptSubmitted,
            "PreToolUse" => EventKind::ToolStarted { tool },
            "PostToolUse" => EventKind::ToolFinished { tool },
            "PostToolUseFailure" => EventKind::ToolFailed { tool },
            "PermissionRequest" => EventKind::PermissionRequested { tool },
            "Notification" => EventKind::Notification {
                notification: notification(field("notification_type").unwrap_or_default()),
            },
            "Stop" => EventKind::TurnFinished,
            "StopFailure" => EventKind::TurnFailed {
                error: field("error"),
            },
            "SubagentStart" => EventKind::SubagentStarted,
            "SubagentStop" => EventKind::SubagentStopped,
            "SessionEnd" => EventKind::SessionEnded {
                reason: field("reason"),
            },
            // Forwarded by `hive worktree hook-create`/`hook-remove` after their work; create
            // adds the path it printed as `worktree_path`.
            "WorktreeCreate" => EventKind::WorktreeCreated {
                name: field("name"),
                path: field("worktree_path"),
            },
            "WorktreeRemove" => EventKind::WorktreeRemoved {
                path: field("worktree_path"),
            },
            other => EventKind::Other {
                event: other.to_owned(),
            },
        };
        AgentEvent {
            provider: "claude-code".to_owned(),
            terminal_id,
            session_id: field("session_id"),
            subagent: field("agent_id").map(|id| Subagent {
                id,
                agent_type: field("agent_type"),
            }),
            cwd: field("cwd"),
            kind,
            raw: payload,
        }
    }
}

fn notification(kind: String) -> Notification {
    match kind.as_str() {
        "permission_prompt" => Notification::PermissionPrompt,
        "elicitation_dialog" => Notification::ElicitationDialog,
        "idle_prompt" => Notification::IdlePrompt,
        "agent_needs_input" => Notification::AgentNeedsInput,
        _ => Notification::Other(kind),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn kind(event: &str, payload: Value) -> EventKind {
        ClaudeCode.translate(event, None, payload).kind
    }

    #[test]
    fn common_fields_and_raw_payload_are_kept() {
        let payload = json!({
            "session_id": "abc", "cwd": "/repo/.claude/worktrees/x",
            "hook_event_name": "SessionStart", "source": "startup"
        });
        let event = ClaudeCode.translate("SessionStart", Some("7".into()), payload.clone());
        assert_eq!(
            event,
            AgentEvent {
                provider: "claude-code".into(),
                terminal_id: Some("7".into()),
                session_id: Some("abc".into()),
                subagent: None,
                cwd: Some("/repo/.claude/worktrees/x".into()),
                kind: EventKind::SessionStarted,
                raw: payload,
            }
        );
    }

    #[test]
    fn subagent_comes_from_agent_id_and_type() {
        let event = ClaudeCode.translate(
            "SubagentStart",
            None,
            json!({"agent_id": "a1", "agent_type": "Explore"}),
        );
        assert_eq!(event.kind, EventKind::SubagentStarted);
        assert_eq!(
            event.subagent,
            Some(Subagent {
                id: "a1".into(),
                agent_type: Some("Explore".into())
            })
        );
    }

    #[test]
    fn agent_type_without_agent_id_is_not_a_subagent() {
        let event = ClaudeCode.translate("SessionStart", None, json!({"agent_type": "reviewer"}));
        assert_eq!(event.subagent, None);
    }

    #[test]
    fn tool_events_carry_the_tool_name() {
        let bash = || Some("Bash".to_owned());
        let p = json!({"tool_name": "Bash"});
        assert_eq!(
            kind("PreToolUse", p.clone()),
            EventKind::ToolStarted { tool: bash() }
        );
        assert_eq!(
            kind("PostToolUse", p.clone()),
            EventKind::ToolFinished { tool: bash() }
        );
        assert_eq!(
            kind("PostToolUseFailure", p.clone()),
            EventKind::ToolFailed { tool: bash() }
        );
        assert_eq!(
            kind("PermissionRequest", p),
            EventKind::PermissionRequested { tool: bash() }
        );
    }

    #[test]
    fn simple_events_map_one_to_one() {
        assert_eq!(
            kind("UserPromptSubmit", json!({})),
            EventKind::PromptSubmitted
        );
        assert_eq!(kind("Stop", json!({})), EventKind::TurnFinished);
        assert_eq!(kind("SubagentStop", json!({})), EventKind::SubagentStopped);
    }

    #[test]
    fn failure_and_end_carry_their_reason() {
        assert_eq!(
            kind("StopFailure", json!({"error": "rate_limit"})),
            EventKind::TurnFailed {
                error: Some("rate_limit".into())
            }
        );
        assert_eq!(
            kind("SessionEnd", json!({"reason": "prompt_input_exit"})),
            EventKind::SessionEnded {
                reason: Some("prompt_input_exit".into())
            }
        );
    }

    #[test]
    fn worktree_events_carry_name_and_path() {
        let event = ClaudeCode.translate(
            "WorktreeCreate",
            None,
            json!({"name": "n", "worktree_path": "/r/.claude/worktrees/n", "agent_id": "a1"}),
        );
        assert_eq!(
            event.kind,
            EventKind::WorktreeCreated {
                name: Some("n".into()),
                path: Some("/r/.claude/worktrees/n".into())
            }
        );
        assert_eq!(event.subagent.map(|s| s.id), Some("a1".into()));
        assert_eq!(
            kind("WorktreeRemove", json!({"worktree_path": "/w"})),
            EventKind::WorktreeRemoved {
                path: Some("/w".into())
            }
        );
    }

    #[test]
    fn notifications_map_by_type() {
        let n = |t: &str, notification| {
            assert_eq!(
                kind("Notification", json!({"notification_type": t})),
                EventKind::Notification { notification }
            );
        };
        n("permission_prompt", Notification::PermissionPrompt);
        n("elicitation_dialog", Notification::ElicitationDialog);
        n("idle_prompt", Notification::IdlePrompt);
        n("agent_needs_input", Notification::AgentNeedsInput);
        n("auth_success", Notification::Other("auth_success".into()));
    }

    #[test]
    fn notification_without_type_is_other() {
        assert_eq!(
            kind("Notification", json!({})),
            EventKind::Notification {
                notification: Notification::Other(String::new())
            }
        );
    }

    #[test]
    fn unknown_events_and_non_object_payloads_are_kept_as_other() {
        let event = ClaudeCode.translate("CwdChanged", None, json!("not an object"));
        assert_eq!(
            event.kind,
            EventKind::Other {
                event: "CwdChanged".into()
            }
        );
        assert_eq!(event.session_id, None);
        assert_eq!(event.raw, json!("not an object"));
    }
}
