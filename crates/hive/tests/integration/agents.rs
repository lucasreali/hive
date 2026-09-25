use std::io::Write;
use std::process::Stdio;

use hive_protocol::AgentState::{self, *};
use hive_protocol::{Control, OpenSession, Role, SessionTarget, SubagentState};
use serde_json::{Value, json};

use crate::common::Conn;
use crate::worktree::Repo;

/// Runs `hive hook <event>` from terminal `terminal`, like Claude Code does, and returns the
/// app's messages up to the forwarded event (so the service has handled the call).
pub(crate) async fn hook(
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
        activity: None,
        since_ms: 0,
    }
}

fn sub(id: &str, state: AgentState) -> SubagentState {
    SubagentState {
        id: id.into(),
        agent_type: Some("Explore".into()),
        state,
        worktree: None,
        activity: None,
        since_ms: 0,
    }
}

/// Runs `hive worktree <hook>` from terminal 1 and returns the app's messages up to the
/// forwarded event, leaving out the `projects` it triggers.
async fn worktree_hook(
    repo: &Repo,
    app: &mut Conn,
    hook: &str,
    payload: Value,
) -> Vec<(u32, Control)> {
    let mut child = repo
        .hive_cmd(&repo.root, &[hook])
        .env("HIVE_TERMINAL_ID", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(payload.to_string().as_bytes()).unwrap();
    drop(stdin);
    assert!(child.wait().unwrap().success());
    let (mut seen, mut forwarded, mut listed) = (Vec::new(), false, false);
    while !(forwarded && listed) {
        match app.control().await {
            (0, Control::Agent(_)) => forwarded = true,
            (0, Control::Projects { .. }) => listed = true,
            other => seen.push(other),
        }
    }
    seen
}

#[tokio::test]
async fn a_subagents_own_worktree_is_sent_with_it() {
    let repo = Repo::new();
    let root = repo.root.display().to_string();
    let worktree = |name: &str| format!("{root}/.claude/worktrees/{name}");
    let (sub_a, sub_b) = (worktree("sub-a"), worktree("sub-b"));
    let mut daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;
    app.send(0, Control::AddProject { path: root.clone() })
        .await;
    assert!(matches!(
        app.control().await,
        (0, Control::ProjectAdded { .. })
    ));
    app.open_terminal(1, &repo.root).await;
    let start = json!({"session_id": "s", "cwd": root});
    hook(&repo, &mut app, "1", "SessionStart", start).await;
    let subagent = |id: &str, extra| {
        let ids = json!({"session_id": "s", "agent_id": id, "agent_type": "Explore", "cwd": root});
        merged(ids, extra)
    };
    let owning = |id: &str, path: &str| SubagentState {
        worktree: Some(path.into()),
        ..sub(id, Working)
    };
    let with = |subagents| vec![(1, state("s", WithSubagents, subagents))];

    // In its agent's worktree a subagent has none of its own.
    let seen = hook(
        &repo,
        &mut app,
        "1",
        "SubagentStart",
        subagent("a", json!({})),
    )
    .await;
    assert_eq!(seen, with(vec![sub("a", Working)]));
    // A `WorktreeCreate` naming the subagent gives it the new worktree.
    let create = subagent("a", json!({"name": "sub-a"}));
    let seen = worktree_hook(&repo, &mut app, "hook-create", create).await;
    assert_eq!(seen, with(vec![owning("a", &sub_a)]));
    // One naming nobody: the subagent whose events come from inside it owns it.
    let create = json!({"session_id": "s", "cwd": root, "name": "sub-b"});
    assert_eq!(
        worktree_hook(&repo, &mut app, "hook-create", create).await,
        vec![]
    );
    let inside = subagent("b", json!({"cwd": format!("{sub_b}/src")}));
    let seen = hook(&repo, &mut app, "1", "SubagentStart", inside).await;
    assert_eq!(seen, with(vec![owning("a", &sub_a), owning("b", &sub_b)]));
    // Removing the worktree unlinks it; the subagent leaving takes its own along.
    let remove = json!({"session_id": "s", "cwd": sub_a, "worktree_path": sub_a});
    let seen = worktree_hook(&repo, &mut app, "hook-remove", remove).await;
    assert_eq!(seen, with(vec![sub("a", Working), owning("b", &sub_b)]));
    let seen = hook(
        &repo,
        &mut app,
        "1",
        "SubagentStop",
        subagent("b", json!({})),
    )
    .await;
    assert_eq!(seen, with(vec![sub("a", Working)]));
    drop(app);
    assert!(daemon.wait_exit().success());
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
            // What the subagent asks to do is its activity.
            subagent(json!({"tool_name": "Bash", "tool_input": {"command": "make\nx"}})),
            vec![state(
                "s",
                WaitingPermission,
                vec![SubagentState {
                    activity: Some("make".into()),
                    ..sub("a", WaitingPermission)
                }],
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

#[tokio::test]
async fn an_agent_finishing_in_view_of_the_focused_window_is_not_pending() {
    let repo = Repo::new();
    let mut daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;
    app.open_terminal(1, &repo.root).await;
    let cwd = repo.root.display().to_string();
    let start = json!({"session_id": "s", "cwd": cwd});
    hook(&repo, &mut app, "1", "SessionStart", start).await;
    let s = || json!({"session_id": "s"});
    let turn = async |app: &mut Conn, terminal, focused| {
        app.send(0, Control::View { terminal, focused }).await;
        // Answered after the view is applied: frames are handled in order.
        app.send(0, Control::ListProjects).await;
        assert!(matches!(app.control().await, (0, Control::Projects { .. })));
        hook(&repo, app, "1", "UserPromptSubmit", s()).await;
        hook(&repo, app, "1", "Stop", s()).await
    };
    let finished = |pending| {
        let message = Control::AgentState {
            id: "s".into(),
            state: WaitingYou,
            urgency: WaitingYou.urgency(),
            pending,
            subagents: vec![],
            activity: None,
            since_ms: 0,
        };
        vec![(1, message)]
    };
    // Shown but the window is not focused, or focused on another terminal: pending as usual.
    assert_eq!(turn(&mut app, Some(1), false).await, finished(true));
    assert_eq!(turn(&mut app, Some(2), true).await, finished(true));
    assert_eq!(turn(&mut app, None, true).await, finished(true));
    assert_eq!(turn(&mut app, Some(1), true).await, finished(false));
    drop(app);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn sessions_of_followed_projects_are_listed_located_and_deleted() {
    let repo = Repo::new();
    let root = repo.root.display().to_string();
    let logs = repo.env.path("home/.claude/projects").join(
        root.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect::<String>(),
    );
    std::fs::create_dir_all(&logs).unwrap();
    for id in ["s", "old"] {
        let line =
            json!({"type": "user", "cwd": root, "message": {"content": format!("task {id}")}});
        std::fs::write(logs.join(format!("{id}.jsonl")), line.to_string()).unwrap();
    }
    let mut daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;
    let mut ask = async |message: Control| {
        app.send(0, message).await;
        app.control().await.1
    };
    // Only the followed projects' sessions.
    assert_eq!(
        ask(Control::ListSessions).await,
        Control::Sessions {
            sessions: vec![],
            error: None
        }
    );
    let added = ask(Control::AddProject { path: root.clone() }).await;
    assert!(matches!(added, Control::ProjectAdded { .. }), "{added:?}");
    let Control::Sessions { sessions, error } = ask(Control::ListSessions).await else {
        panic!("expected sessions")
    };
    let mut ids: Vec<_> = sessions.iter().map(|s| s.id.as_str()).collect();
    ids.sort();
    assert_eq!((ids, error), (vec!["old", "s"], None));

    for target in [SessionTarget::Log, SessionTarget::Folder] {
        let located = ask(Control::LocateSession {
            id: "s".into(),
            target,
        })
        .await;
        let Control::SessionLocated { id, target: t, .. } = located else {
            panic!("expected a location: {located:?}")
        };
        assert_eq!((id.as_str(), t), ("s", target));
    }
    let Control::SessionLocated { error, .. } = ask(Control::LocateSession {
        id: "nope".into(),
        target: SessionTarget::Log,
    })
    .await
    else {
        panic!("expected a location")
    };
    assert_eq!(
        error.as_deref(),
        Some("no session nope in the followed projects")
    );

    // A running session is not deleted.
    app.open_terminal(1, &repo.root).await;
    hook(
        &repo,
        &mut app,
        "1",
        "SessionStart",
        json!({"session_id": "s", "cwd": root}),
    )
    .await;
    let mut ask = async |message: Control| {
        app.send(0, message).await;
        loop {
            if let (
                0,
                answer @ (Control::SessionDeleted { .. } | Control::DeleteSessionFailed { .. }),
            ) = app.control().await
            {
                return answer;
            }
        }
    };
    assert_eq!(
        ask(Control::DeleteSession { id: "s".into() }).await,
        Control::DeleteSessionFailed {
            id: "s".into(),
            message: "the session is running: end it first".into()
        }
    );
    assert_eq!(
        ask(Control::DeleteSession { id: "old".into() }).await,
        Control::SessionDeleted { id: "old".into() }
    );
    assert!(!logs.join("old.jsonl").exists());
    drop(app);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn unreadable_session_logs_are_reported() {
    let repo = Repo::new();
    // Claude's `projects` is a file here: its logs cannot be listed.
    std::fs::create_dir_all(repo.env.path("home/.claude")).unwrap();
    std::fs::write(repo.env.path("home/.claude/projects"), "").unwrap();
    let mut daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;
    app.send(0, Control::ListSessions).await;
    let Control::Sessions { sessions, error } = app.control().await.1 else {
        panic!("expected sessions")
    };
    assert!(sessions.is_empty());
    assert!(error.is_some_and(|e| e.contains("Not a directory")));
    drop(app);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn sessions_running_when_the_app_closes_are_sent_to_the_next_app() {
    let repo = Repo::new();
    let root = repo.root.display().to_string();
    let mut daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;
    app.open_terminal(1, &repo.root).await;
    app.open_terminal(2, &repo.root).await;
    hook(
        &repo,
        &mut app,
        "2",
        "SessionStart",
        json!({"session_id": "b", "cwd": root}),
    )
    .await;
    hook(
        &repo,
        &mut app,
        "1",
        "SessionStart",
        json!({"session_id": "a", "cwd": root}),
    )
    .await;
    // One without a cwd cannot be resumed anywhere.
    app.open_terminal(3, &repo.root).await;
    hook(
        &repo,
        &mut app,
        "3",
        "SessionStart",
        json!({"session_id": "c"}),
    )
    .await;
    drop(app);
    assert!(daemon.wait_exit().success());
    let kept = repo.env.path("data/hive/open-sessions.json");
    assert!(kept.exists());

    let mut daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;
    let open = |id: &str| OpenSession {
        id: id.into(),
        cwd: root.clone(),
    };
    assert_eq!(
        app.control().await,
        (
            0,
            Control::RestoreSessions {
                sessions: vec![open("a"), open("b")]
            }
        )
    );
    assert!(!kept.exists());
    drop(app);
    assert!(daemon.wait_exit().success());
    // Nothing ran this time: the next app resumes nothing.
    assert!(!kept.exists());
}

#[tokio::test]
async fn the_app_closes_even_when_the_open_sessions_cannot_be_kept() {
    let repo = Repo::new();
    let root = repo.root.display().to_string();
    // A folder where the list goes: it cannot be written.
    let kept = repo.env.path("data/hive/open-sessions.json");
    std::fs::create_dir_all(&kept).unwrap();
    let mut daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;
    app.open_terminal(1, &repo.root).await;
    hook(
        &repo,
        &mut app,
        "1",
        "SessionStart",
        json!({"session_id": "a", "cwd": root}),
    )
    .await;
    drop(app);
    assert!(daemon.wait_exit().success());
    assert!(kept.is_dir());
}

#[tokio::test]
async fn an_agent_gets_its_session_name_and_its_renames() {
    let repo = Repo::new();
    let root = repo.root.display().to_string();
    let logs = repo.env.path("home/.claude/projects").join(
        root.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect::<String>(),
    );
    std::fs::create_dir_all(&logs).unwrap();
    let log = logs.join("s.jsonl");
    std::fs::write(&log, "{\"type\":\"ai-title\",\"aiTitle\":\"Fix it\"}\n").unwrap();
    let mut daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;
    app.open_terminal(1, &repo.root).await;
    let title = |title: &str| {
        (
            1,
            Control::AgentTitle {
                id: "s".into(),
                title: title.into(),
            },
        )
    };
    let seen = hook(
        &repo,
        &mut app,
        "1",
        "SessionStart",
        json!({"session_id": "s", "cwd": root}),
    )
    .await;
    assert_eq!(seen.last(), Some(&title("Fix it")));
    // The same name is not sent again; a rename shows at the end of the turn.
    let turn = json!({"session_id": "s", "cwd": root});
    let seen = hook(&repo, &mut app, "1", "Stop", turn.clone()).await;
    assert!(!seen.contains(&title("Fix it")), "{seen:?}");
    let renamed = "{\"type\":\"ai-title\",\"aiTitle\":\"Fix it\"}\n{\"type\":\"custom-title\",\"customTitle\":\"Mine\"}\n";
    std::fs::write(&log, renamed).unwrap();
    // A subagent's end of turn does not read the log again.
    let sub = json!({"session_id": "s", "agent_id": "a", "agent_type": "Explore", "cwd": root});
    let seen = hook(&repo, &mut app, "1", "Stop", sub).await;
    assert!(!seen.contains(&title("Mine")), "{seen:?}");
    let seen = hook(&repo, &mut app, "1", "Stop", turn).await;
    assert!(seen.contains(&title("Mine")), "{seen:?}");
    // An agent without a folder has no log to name it.
    app.open_terminal(2, &repo.root).await;
    let seen = hook(
        &repo,
        &mut app,
        "2",
        "SessionStart",
        json!({"session_id": "t"}),
    )
    .await;
    assert!(
        !seen
            .iter()
            .any(|(_, m)| matches!(m, Control::AgentTitle { .. })),
        "{seen:?}"
    );
    drop(app);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn a_subagents_transcript_is_sent_and_followed_while_watched() {
    use hive_protocol::{TranscriptEntry, TranscriptRole};
    let repo = Repo::new();
    let root = repo.root.display().to_string();
    let parent = repo.env.path("home/.claude/projects/-repo/s.jsonl");
    let log = parent.with_extension("").join("subagents/agent-a.jsonl");
    std::fs::create_dir_all(log.parent().unwrap()).unwrap();
    let said = |text: &str| {
        format!(
            "{}\n",
            json!({"type": "user", "message": {"content": text}})
        )
    };
    std::fs::write(&log, said("first")).unwrap();
    let append = |text: &str| {
        let mut file = std::fs::File::options().append(true).open(&log).unwrap();
        file.write_all(said(text).as_bytes()).unwrap();
    };
    let user = |text: &str| TranscriptEntry {
        role: TranscriptRole::User,
        text: text.into(),
        tool: None,
    };
    let mut daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;
    app.open_terminal(1, &repo.root).await;
    let start = json!({"session_id": "s", "cwd": root, "transcript_path": parent});
    hook(&repo, &mut app, "1", "SessionStart", start).await;
    let watch = |agent: &str, subagent: &str| Control::WatchTranscript {
        agent: agent.into(),
        subagent: subagent.into(),
    };
    let unwatch = |agent: &str, subagent: &str| Control::UnwatchTranscript {
        agent: agent.into(),
        subagent: subagent.into(),
    };

    app.send(0, watch("s", "a")).await;
    let first = Control::Transcript {
        agent: "s".into(),
        subagent: "a".into(),
        entries: vec![user("first")],
        truncated: false,
    };
    assert_eq!(app.control().await, (0, first));
    // Unwatching another subagent leaves this one followed.
    app.send(0, unwatch("s", "b")).await;
    app.send(0, unwatch("t", "a")).await;
    append("second");
    let appended = Control::TranscriptAppended {
        agent: "s".into(),
        subagent: "a".into(),
        entries: vec![user("second")],
    };
    assert_eq!(app.control().await, (0, appended));

    // Once unwatched, nothing more is sent.
    app.send(0, unwatch("s", "a")).await;
    append("third");
    tokio::time::sleep(std::time::Duration::from_millis(2500)).await;
    // Neither an unknown agent nor a bad subagent id has a transcript.
    let unknown = Control::Error {
        message: "no transcript is known for this subagent".into(),
    };
    for (agent, subagent) in [("nope", "a"), ("s", "../x")] {
        app.send(0, watch(agent, subagent)).await;
        assert_eq!(app.control().await, (0, unknown.clone()));
    }
    drop(app);
    assert!(daemon.wait_exit().success());
}
