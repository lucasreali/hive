use std::io::Write;
use std::process::Stdio;

use hive_protocol::AgentState::{self, *};
use hive_protocol::{Control, Role, SubagentState};
use serde_json::{Value, json};

use crate::common::Conn;
use crate::worktree::Repo;

/// Runs `hive hook <event>` from terminal `terminal`, like Claude Code does, and returns the
/// app's messages up to the forwarded event (so the service has handled the call).
async fn hook(
    repo: &Repo,
    app: &mut Conn,
    terminal: &str,
    event: &str,
    payload: Value,
) -> Vec<(u32, Control)> {
    let mut child = repo
        .env
        .hive()
        .args(["hook", event])
        .env("HIVE_TERMINAL_ID", terminal)
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(payload.to_string().as_bytes()).unwrap();
    drop(stdin);
    assert!(child.wait().unwrap().success());
    let mut seen = Vec::new();
    loop {
        match app.control().await {
            (0, Control::Agent(_)) => return seen,
            other => seen.push(other),
        }
    }
}

fn merged(mut base: Value, extra: Value) -> Value {
    for (key, value) in extra.as_object().unwrap() {
        base[key] = value.clone();
    }
    base
}

fn state(id: &str, state: AgentState, subagents: Vec<SubagentState>) -> Control {
    Control::AgentState {
        id: id.into(),
        state,
        urgency: state.urgency(),
        pending: state.pending(),
        subagents,
    }
}

fn sub(id: &str, state: AgentState) -> SubagentState {
    SubagentState {
        id: id.into(),
        agent_type: Some("Explore".into()),
        state,
    }
}

fn detected(id: &str, place: Option<(&str, &str)>, cwd: &str) -> Control {
    Control::AgentDetected {
        id: id.into(),
        project: place.map(|p| p.0.into()),
        worktree: place.map(|p| p.1.into()),
        cwd: Some(cwd.into()),
    }
}

#[tokio::test]
async fn agents_are_placed_by_their_cwd_and_removed_when_they_end() {
    let repo = Repo::new();
    assert!(repo.hive(&["create", "fix-a"]).status.success());
    let root = repo.root.display().to_string();
    let fix_a = format!("{root}/.claude/worktrees/fix-a");
    let mut daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;
    app.send(0, Control::AddProject { path: root.clone() })
        .await;
    assert!(matches!(
        app.control().await,
        (0, Control::ProjectAdded { .. })
    ));
    // Both terminals start in the main worktree; only the payload's cwd places the agent.
    app.open_terminal(1, &repo.root).await;
    app.open_terminal(2, &repo.root).await;

    let cwd = format!("{fix_a}/src");
    let start = json!({"session_id": "s1", "cwd": cwd, "source": "startup"});
    let seen = hook(&repo, &mut app, "1", "SessionStart", start).await;
    let placed = detected("s1", Some((&root, &fix_a)), &cwd);
    assert_eq!(seen, vec![(1, placed), (1, state("s1", Idle, vec![]))]);

    let home = repo.env.path("home").display().to_string();
    let outside = json!({"session_id": "s2", "cwd": home});
    let seen = hook(&repo, &mut app, "2", "SessionStart", outside).await;
    let placed = detected("s2", None, &home);
    assert_eq!(seen, vec![(2, placed), (2, state("s2", Idle, vec![]))]);

    // Not detected: a subagent, an unknown terminal, no session id, a terminal id that is not
    // ours. No state change: unknown or missing sessions, an unknown subagent, a notification
    // without a state.
    let ignored = [
        (
            "1",
            "SessionStart",
            json!({"session_id": "s1", "agent_id": "a", "cwd": root}),
        ),
        (
            "7",
            "SessionStart",
            json!({"session_id": "s3", "cwd": root}),
        ),
        ("1", "SessionStart", json!({"cwd": root})),
        (
            "x",
            "SessionStart",
            json!({"session_id": "s4", "cwd": root}),
        ),
        ("1", "SessionEnd", json!({"session_id": "unknown"})),
        (
            "1",
            "SessionEnd",
            json!({"session_id": "s1", "agent_id": "a"}),
        ),
        ("1", "Stop", json!({})),
        ("1", "Stop", json!({"session_id": "unknown"})),
        (
            "1",
            "Notification",
            json!({"session_id": "s1", "notification_type": "auth_success"}),
        ),
    ];
    for (terminal, event, payload) in ignored {
        let seen = hook(&repo, &mut app, terminal, event, payload.clone()).await;
        assert_eq!(seen, vec![], "{event} {payload}");
    }

    let end = json!({"session_id": "s1", "reason": "prompt_input_exit"});
    let seen = hook(&repo, &mut app, "1", "SessionEnd", end).await;
    let removed = Control::AgentRemoved { id: "s1".into() };
    assert_eq!(seen, vec![(1, state("s1", Ended, vec![])), (1, removed)]);

    // The agent's terminal exits: the agent goes first, then the terminal.
    app.input(2, "exit\r").await;
    assert_eq!(
        app.control().await,
        (2, Control::AgentRemoved { id: "s2".into() })
    );
    assert_eq!(
        app.control().await,
        (2, Control::TerminalExited { code: Some(0) })
    );
    drop(app);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn agent_states_follow_hook_events_and_terminal_silence() {
    let repo = Repo::new();
    let mut daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;
    app.open_terminal(1, &repo.root).await;
    let cwd = repo.root.display().to_string();
    let start = json!({"session_id": "s", "cwd": cwd});
    hook(&repo, &mut app, "1", "SessionStart", start).await;

    let main = |extra| merged(json!({"session_id": "s"}), extra);
    let subagent = |extra| {
        let ids = json!({"session_id": "s", "agent_id": "a", "agent_type": "Explore"});
        merged(ids, extra)
    };
    let steps: [(&str, Value, Vec<Control>); 6] = [
        (
            "UserPromptSubmit",
            main(json!({})),
            vec![state("s", Working, vec![])],
        ),
        (
            "SubagentStart",
            subagent(json!({})),
            vec![state("s", WithSubagents, vec![sub("a", Working)])],
        ),
        (
            "PermissionRequest",
            subagent(json!({"tool_name": "Bash"})),
            vec![state(
                "s",
                WaitingPermission,
                vec![sub("a", WaitingPermission)],
            )],
        ),
        // A failed tool is routine: still working, never an error.
        (
            "PostToolUseFailure",
            main(json!({"tool_name": "Bash"})),
            vec![],
        ),
        (
            "SubagentStop",
            subagent(json!({})),
            vec![state("s", Working, vec![])],
        ),
        (
            "Notification",
            main(json!({"notification_type": "permission_prompt"})),
            vec![state("s", WaitingPermission, vec![])],
        ),
    ];
    for (event, payload, expected) in steps {
        let seen = hook(&repo, &mut app, "1", event, payload).await;
        let expected: Vec<_> = expected.into_iter().map(|m| (1, m)).collect();
        assert_eq!(seen, expected, "{event}");
    }

    // Rule 2: the terminal prints nothing for 5 s (an interrupt fires no Stop).
    let waited = std::time::Instant::now();
    assert_eq!(app.control().await, (1, state("s", WaitingYou, vec![])));
    assert!(waited.elapsed() >= std::time::Duration::from_secs(4));

    let seen = hook(
        &repo,
        &mut app,
        "1",
        "StopFailure",
        main(json!({"error": "x"})),
    )
    .await;
    assert_eq!(seen, vec![(1, state("s", Error, vec![]))]);
    drop(app);
    assert!(daemon.wait_exit().success());
}
