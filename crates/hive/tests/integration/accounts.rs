//! Claude accounts (12.2): kept in the settings, given to new terminals, their sessions
//! listed together, and made of the Claude config folders spaces had.

use std::path::Path;

use hive_protocol::{Account, AccountDir, Control, Role, Settings};
use serde_json::json;

use crate::agents::{hook, next_usage};
use crate::common::Conn;
use crate::worktree::Repo;

async fn request(conn: &mut Conn, message: Control) -> Control {
    conn.send(0, message).await;
    conn.control().await.1
}

/// Writes a session `id` of `cwd` into the Claude config folder `claude`, and returns its log.
fn session(claude: &Path, cwd: &str, id: &str) -> std::path::PathBuf {
    let folder: String = cwd
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let dir = claude.join("projects").join(folder);
    std::fs::create_dir_all(&dir).unwrap();
    let log = dir.join(format!("{id}.jsonl"));
    let usage = json!({"input_tokens": 7, "output_tokens": 2});
    let lines = [
        json!({"type": "user", "cwd": cwd, "message": {"content": id}}),
        json!({"type": "assistant", "message": {"id": "m", "usage": usage}}),
    ];
    std::fs::write(&log, format!("{}\n{}\n", lines[0], lines[1])).unwrap();
    log
}

#[tokio::test]
async fn new_terminals_get_the_current_account_and_every_accounts_sessions_are_listed() {
    let repo = Repo::new();
    let root = repo.root.display().to_string();
    let work = repo.env.path("home/.claude-work");
    session(&repo.env.path("home/.claude"), &root, "a");
    let work_log = session(&work, &root, "b");
    let work = work.display().to_string();
    let mut daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;
    let added = request(&mut app, Control::AddProject { path: root.clone() }).await;
    assert!(matches!(added, Control::ProjectAdded { .. }), "{added:?}");

    // The default account: no `CLAUDE_CONFIG_DIR`.
    let echo = "echo \"[$CLAUDE_CONFIG_DIR]\"\r";
    app.open_terminal(1, &repo.root).await;
    app.input(1, echo).await;
    app.output_until(1, "[]").await;

    // Another account made current: new terminals get it, open ones keep theirs.
    let mut settings = Settings::default();
    settings.claude.accounts.push(Account {
        name: "Work".into(),
        config_dir: work.clone(),
    });
    settings.claude.account = Some(work.clone());
    let set = Control::SetSettings {
        settings: settings.clone(),
    };
    assert_eq!(request(&mut app, set).await, Control::Settings { settings });
    app.open_terminal(2, &repo.root).await;
    app.input(2, echo).await;
    app.output_until(2, &format!("[{work}]")).await;
    app.input(1, echo).await;
    app.output_until(1, "[]").await;

    // A terminal asked for with an account (to log in, or to resume one of its sessions) gets
    // that one, the default account too; a folder no account has is refused.
    let open = |account: Option<&str>| Control::OpenTerminal {
        cwd: root.clone(),
        cols: 80,
        rows: 24,
        account: Some(AccountDir {
            config_dir: account.map(Into::into),
        }),
    };
    app.send(3, open(None)).await;
    let opened = app.control().await;
    assert!(
        matches!(opened, (3, Control::TerminalOpened { .. })),
        "{opened:?}"
    );
    app.input(3, echo).await;
    app.output_until(3, "[]").await;
    app.send(4, open(Some("/nope"))).await;
    let refused = Control::Error {
        message: "No Claude account has the folder /nope".into(),
    };
    assert_eq!(app.control().await, (4, refused));

    // The sessions of every account, whichever is current, each with its account's folder.
    let Control::Sessions { sessions, .. } = request(&mut app, Control::ListSessions).await else {
        panic!("expected sessions")
    };
    let mut listed: Vec<_> = (sessions.iter())
        .map(|s| (s.id.as_str(), s.config_dir.as_deref()))
        .collect();
    listed.sort_unstable();
    assert_eq!(listed, [("a", None), ("b", Some(work.as_str()))]);
    // Each with the line resuming it from any shell of this system, as its account.
    let mut lines: Vec<_> = (sessions.iter())
        .map(|s| s.resume_command.clone().unwrap())
        .collect();
    lines.sort_unstable();
    assert_eq!(
        lines,
        [
            format!("cd '{root}' && CLAUDE_CONFIG_DIR='{work}' claude --resume b"),
            format!("cd '{root}' && claude --resume a"),
        ]
    );

    // An agent's transcript in any account's folder is read, whatever its terminal's account.
    let start = json!({"session_id": "b", "cwd": root, "transcript_path": work_log});
    let seen = hook(&repo, &mut app, "1", "SessionStart", start).await;
    let usage = Control::AgentUsage {
        id: "b".into(),
        context_tokens: 7,
        context_limit: 200_000,
        output_tokens: 2,
    };
    assert_eq!(next_usage(&mut app, seen).await, usage);
    drop(app);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn the_spaces_claude_folders_become_accounts_once() {
    let repo = Repo::new();
    let work = repo.env.path("home/.claude-work").display().to_string();
    let file = repo.env.path("data/hive/spaces.json");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let env = json!({"claude_config_dir": work, "git_name": "Me"});
    let spaces = json!({"current": "default", "spaces": [
        {"id": "default", "name": "Default", "env": {"claude_config_dir": null}},
        {"id": "space-1", "name": "Work", "env": env},
        {"id": "space-2", "name": "Work too", "env": {"claude_config_dir": work}},
    ]});
    std::fs::write(&file, spaces.to_string()).unwrap();
    let mut expected = Settings::default();
    expected.claude.accounts.push(Account {
        name: "Work".into(),
        config_dir: work.clone(),
    });
    // Twice: the second start finds them made already.
    for _ in 0..2 {
        let mut daemon = repo.env.daemon();
        let mut app = repo.env.handshake(Role::App).await;
        let settings = Control::Settings {
            settings: expected.clone(),
        };
        assert_eq!(app.control().await, (0, settings));
        let saved = std::fs::read_to_string(&file).unwrap();
        assert!(!saved.contains("claude_config_dir"), "{saved}");
        assert!(saved.contains("\"git_name\": \"Me\""), "{saved}");
        drop(app);
        assert!(daemon.wait_exit().success());
    }
}
