use std::io::Write;
use std::process::Stdio;

use hive_protocol::{Control, Role};
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
    assert_eq!(seen, vec![(1, detected("s1", Some((&root, &fix_a)), &cwd))]);

    let home = repo.env.path("home").display().to_string();
    let outside = json!({"session_id": "s2", "cwd": home});
    let seen = hook(&repo, &mut app, "2", "SessionStart", outside).await;
    assert_eq!(seen, vec![(2, detected("s2", None, &home))]);

    // Not detected: a subagent, an unknown terminal, no session id, a terminal id that is not ours.
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
        ("1", "Stop", json!({"session_id": "s1"})),
    ];
    for (terminal, event, payload) in ignored {
        let seen = hook(&repo, &mut app, terminal, event, payload.clone()).await;
        assert_eq!(seen, vec![], "{event} {payload}");
    }

    let end = json!({"session_id": "s1", "reason": "prompt_input_exit"});
    let seen = hook(&repo, &mut app, "1", "SessionEnd", end).await;
    assert_eq!(seen, vec![(1, Control::AgentRemoved { id: "s1".into() })]);

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
