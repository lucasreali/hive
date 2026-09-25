//! Adapters translate raw provider hook payloads into the internal event model.

use hive_protocol::{AgentEvent, EventKind, Notification, Subagent};
use serde_json::Value;
use std::path::Path;

/// Longest activity kept, in characters.
const MAX_ACTIVITY: usize = 120;

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
        let activity = match &kind {
            EventKind::ToolStarted { tool: Some(tool) }
            | EventKind::PermissionRequested { tool: Some(tool) } => Some(activity(tool, &payload)),
            _ => None,
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
            activity,
            raw: payload,
        }
    }
}

/// What a tool call does, from its untrusted `tool_input`: a file relative to the payload's
/// `cwd` when inside it, a Bash call's description (else its command's first line), a search
/// pattern, a subagent's description, else the tool's name. Control characters are dropped
/// and the text is cut at [`MAX_ACTIVITY`] characters.
fn activity(tool: &str, payload: &Value) -> String {
    let input = |name: &str| {
        let value = payload.get("tool_input")?.get(name)?.as_str()?;
        Some(value).filter(|v| !v.trim().is_empty())
    };
    let text = match tool {
        "Edit" | "Write" | "MultiEdit" | "NotebookEdit" | "Read" => {
            let path = input("file_path").or_else(|| input("notebook_path"));
            let cwd = payload.get("cwd").and_then(Value::as_str);
            path.map(|path| {
                let short = cwd
                    .and_then(|cwd| Path::new(path).strip_prefix(cwd).ok())
                    .and_then(Path::to_str)
                    .filter(|p| !p.is_empty())
                    .unwrap_or(path);
                let verb = if tool == "Read" { "Reading" } else { "Editing" };
                format!("{verb} {short}")
            })
        }
        "Bash" => input("description")
            .or_else(|| input("command").and_then(|c| c.lines().find(|l| !l.trim().is_empty())))
            .map(str::to_owned),
        "Grep" | "Glob" => input("pattern").map(|p| format!("Searching {p}")),
        "Agent" | "Task" => input("description").map(str::to_owned),
        _ => None,
    };
    clip(&text.unwrap_or_else(|| tool.to_owned()), MAX_ACTIVITY)
}

/// Untrusted text made fit to show: control characters dropped, trimmed, and cut at `max`
/// characters (the last one becomes "…" when cut).
pub fn clip(text: &str, max: usize) -> String {
    let clean: String = text.chars().filter(|c| !c.is_control()).collect();
    let mut chars = clean.trim().chars();
    let mut out: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() {
        out.pop();
        out.push('…');
    }
    out
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
                activity: None,
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

    fn activity_of(event: &str, tool: &str, input: Value) -> Option<String> {
        let payload = json!({"tool_name": tool, "tool_input": input, "cwd": "/repo/w"});
        ClaudeCode.translate(event, None, payload).activity
    }

    fn doing(tool: &str, input: Value) -> String {
        activity_of("PreToolUse", tool, input).unwrap()
    }

    #[test]
    fn tool_calls_describe_their_files_relative_to_cwd() {
        assert_eq!(
            doing("Edit", json!({"file_path": "/repo/w/src/x.ts"})),
            "Editing src/x.ts"
        );
        for tool in ["Write", "MultiEdit"] {
            assert_eq!(doing(tool, json!({"file_path": "/repo/w/a"})), "Editing a");
        }
        assert_eq!(
            doing("NotebookEdit", json!({"notebook_path": "/repo/w/n.ipynb"})),
            "Editing n.ipynb"
        );
        assert_eq!(
            doing("Read", json!({"file_path": "/repo/wx/a"})),
            "Reading /repo/wx/a"
        );
        assert_eq!(
            doing("Read", json!({"file_path": "/repo/w"})),
            "Reading /repo/w"
        );
        let no_cwd = json!({"tool_name": "Read", "tool_input": {"file_path": "/repo/w/a"}});
        let event = ClaudeCode.translate("PreToolUse", None, no_cwd);
        assert_eq!(event.activity.as_deref(), Some("Reading /repo/w/a"));
    }

    #[test]
    fn tool_calls_describe_commands_searches_and_subagents() {
        let bash = |input| doing("Bash", input);
        assert_eq!(
            bash(json!({"command": "cargo test", "description": "Run tests"})),
            "Run tests"
        );
        assert_eq!(
            bash(json!({"command": "\n  cargo test\necho", "description": " "})),
            "cargo test"
        );
        assert_eq!(
            doing("Grep", json!({"pattern": "fn main"})),
            "Searching fn main"
        );
        assert_eq!(
            doing("Glob", json!({"pattern": "**/*.rs"})),
            "Searching **/*.rs"
        );
        assert_eq!(
            doing("Agent", json!({"description": "Find bugs"})),
            "Find bugs"
        );
        assert_eq!(doing("Task", json!({"description": "Plan"})), "Plan");
    }

    #[test]
    fn other_or_malformed_tool_calls_show_the_tool_name() {
        assert_eq!(doing("WebFetch", json!({"url": "https://x"})), "WebFetch");
        assert_eq!(doing("Bash", json!({"command": 3})), "Bash");
        assert_eq!(doing("Edit", json!("not an object")), "Edit");
        for tool in ["Read", "Grep", "Agent"] {
            assert_eq!(doing(tool, json!({})), tool);
        }
    }

    #[test]
    fn activity_is_cleaned_and_limited() {
        let long = "x".repeat(500);
        let text = doing("Bash", json!({ "description": long }));
        assert_eq!(text.chars().count(), 120);
        assert!(text.ends_with("x…"));
        let exact = "y".repeat(120);
        assert_eq!(
            doing("Bash", json!({ "description": exact.clone() })),
            exact
        );
        assert_eq!(
            doing("Bash", json!({"description": "a\u{1b}[31mb\tc\r"})),
            "a[31mbc"
        );
    }

    #[test]
    fn clip_trims_before_counting() {
        assert_eq!(clip("  \u{7}ab  ", 2), "ab");
        assert_eq!(clip(" abc ", 2), "a…");
        assert_eq!(clip("\t\n", 5), "");
    }

    #[test]
    fn only_tool_starts_and_permission_requests_have_an_activity() {
        let input = json!({"description": "Run"});
        assert_eq!(
            activity_of("PermissionRequest", "Bash", input.clone()).as_deref(),
            Some("Run")
        );
        for event in ["PostToolUse", "PostToolUseFailure", "Stop"] {
            assert_eq!(activity_of(event, "Bash", input.clone()), None, "{event}");
        }
        let nameless = ClaudeCode.translate("PreToolUse", None, json!({"tool_input": input}));
        assert_eq!(nameless.activity, None);
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
