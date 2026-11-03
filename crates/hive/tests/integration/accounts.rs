//! Claude accounts (12.2): kept in the settings, given to new terminals, their sessions
//! listed together, and made of the Claude config folders spaces had.

use std::path::Path;

use hive_protocol::{Account, AccountDir, ClaudeLogin, Control, Role, Settings};
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

#[tokio::test]
async fn every_claude_in_a_terminal_gets_the_shared_plugin_which_settings_list_and_open() {
    use std::os::unix::fs::PermissionsExt;
    let env = crate::common::Env::new();
    // A `wslpath`, and a `claude` that prints its arguments (never the real one), of the
    // test's own.
    let tools = env.path("tools");
    let fake = env.path("fake");
    for dir in [&tools, &fake] {
        std::fs::create_dir_all(dir).unwrap();
    }
    let args = "#!/bin/sh\nprintf '<%s>' \"$@\"; echo '<end>'\n";
    for (file, script) in [
        (tools.join("wslpath"), "#!/bin/sh\necho \"W:$2\"\n"),
        (fake.join("claude"), args),
    ] {
        std::fs::write(&file, script).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    // First on the service's own `PATH`, which still finds the terminals' shell.
    let path = std::env::var_os("PATH").unwrap_or_default();
    let path =
        std::env::join_paths(std::iter::once(tools.clone()).chain(std::env::split_paths(&path)));
    let child = env
        .hive()
        .env("PATH", path.unwrap())
        .arg("daemon")
        .stdin(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let mut daemon = crate::common::Daemon(child);
    crate::common::wait_until(|| std::os::unix::net::UnixStream::connect(env.socket()).is_ok());
    let mut app = env.connect(Role::App).await;
    // Made at the service's start, as a plugin.
    let shared = env.path("data/hive/shared");
    let manifest = std::fs::read_to_string(shared.join(".claude-plugin/plugin.json")).unwrap();
    assert!(manifest.contains("\"name\": \"hive-shared\""), "{manifest}");

    // The wrapper of a terminal, with only the fake `claude` to find.
    let settings = env.path("data/hive/hive-hooks.json");
    let wrapper = env.path("data/hive/bin/claude");
    let run = format!(
        "env PATH='{}' '{}' one\r",
        fake.display(),
        wrapper.display()
    );
    app.open_terminal(1, &env.path("home")).await;
    app.input(1, &run).await;
    let with = format!(
        "<--settings><{}><--plugin-dir><{}><one><end>",
        settings.display(),
        shared.display()
    );
    app.output_until(1, &with).await;

    // Listed by the settings, by name.
    std::fs::create_dir_all(shared.join("skills/review")).unwrap();
    std::fs::write(shared.join("skills/review/SKILL.md"), "x").unwrap();
    std::fs::write(shared.join("agents/tester.md"), "x").unwrap();
    std::fs::write(shared.join(".mcp.json"), r#"{"mcpServers": {"db": {}}}"#).unwrap();
    let listed = Control::Shared {
        folder: shared.display().to_string(),
        plugin: "hive-shared".into(),
        skills: vec!["review".into()],
        agents: vec!["tester".into()],
        commands: vec![],
        mcp_servers: vec!["db".into()],
        error: None,
    };
    assert_eq!(request(&mut app, Control::GetShared).await, listed);

    // Without the folder, the hooks settings only.
    std::fs::remove_dir_all(&shared).unwrap();
    app.input(1, &run).await;
    let without = format!("<--settings><{}><one><end>", settings.display());
    app.output_until(1, &without).await;

    // Opening it makes it again.
    #[cfg(target_os = "linux")]
    let windows_path = format!("W:{}", shared.display());
    #[cfg(target_os = "macos")]
    let windows_path = shared.display().to_string();
    let target = Control::EditorTarget {
        worktree: String::new(),
        path: "shared".into(),
        reveal: false,
        windows_path: Some(windows_path),
        error: None,
    };
    assert_eq!(request(&mut app, Control::OpenShared).await, target);
    assert!(shared.join(".claude-plugin/plugin.json").is_file());
    // One that cannot be made: why.
    std::fs::remove_dir_all(&shared).unwrap();
    std::fs::write(&shared, "").unwrap();
    let Control::EditorTarget {
        windows_path,
        error,
        ..
    } = request(&mut app, Control::OpenShared).await
    else {
        panic!("expected editor_target")
    };
    assert_eq!(windows_path, None);
    assert!(error.is_some());
    drop(app);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn each_account_says_whether_it_logged_in() {
    let repo = Repo::new();
    let home = repo.env.path("home");
    let work = repo.env.path("home/.claude-work");
    std::fs::create_dir_all(&work).unwrap();
    // The default account's file is beside `~/.claude`, a named account's in its folder.
    std::fs::write(home.join(".claude.json"), r#"{"oauthAccount":{"a":1}}"#).unwrap();
    std::fs::write(work.join(".claude.json"), r#"{"numStartups":1}"#).unwrap();
    let work = work.display().to_string();
    let mut daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;
    let mut settings = Settings::default();
    settings.claude.accounts.push(Account {
        name: "Work".into(),
        config_dir: work.clone(),
    });
    let set = Control::SetSettings {
        settings: settings.clone(),
    };
    assert_eq!(request(&mut app, set).await, Control::Settings { settings });
    let login = |config_dir: Option<&str>, logged_in| ClaudeLogin {
        config_dir: config_dir.map(String::from),
        logged_in,
    };
    let logins = vec![login(None, true), login(Some(&work), false)];
    let answer = request(&mut app, Control::ListClaudeLogins).await;
    assert_eq!(answer, Control::ClaudeLogins { logins });
    // Its `/login` finished: asked again, it is logged in.
    let done = r#"{"numStartups":1,"oauthAccount":{}}"#;
    std::fs::write(Path::new(&work).join(".claude.json"), done).unwrap();
    let logins = vec![login(None, true), login(Some(&work), true)];
    let answer = request(&mut app, Control::ListClaudeLogins).await;
    assert_eq!(answer, Control::ClaudeLogins { logins });
    drop(app);
    assert!(daemon.wait_exit().success());
}
