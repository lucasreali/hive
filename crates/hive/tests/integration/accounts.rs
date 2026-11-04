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
async fn terminals_get_their_projects_space_account_and_every_accounts_sessions_are_listed() {
    let repo = Repo::new();
    let root = repo.root.display().to_string();
    let work = repo.env.path("home/.claude-work");
    std::fs::create_dir_all(&work).unwrap();
    session(&repo.env.path("home/.claude"), &root, "a");
    let work_log = session(&work, &root, "b");
    let work = work.display().to_string();
    // A second project, for a second space.
    let other = repo.root.with_file_name("other");
    std::fs::create_dir(&other).unwrap();
    repo.git_in(&other, &["init", "-q", "-b", "main"]);
    let mut daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;
    let added = request(&mut app, Control::AddProject { path: root.clone() }).await;
    assert!(matches!(added, Control::ProjectAdded { .. }), "{added:?}");

    // The default space's account is the default one: no `CLAUDE_CONFIG_DIR`.
    let echo = "echo \"[$CLAUDE_CONFIG_DIR]\"\r";
    app.open_terminal(1, &repo.root).await;
    app.input(1, echo).await;
    app.output_until(1, "[]").await;

    let mut settings = Settings::default();
    settings.claude.accounts.push(Account {
        name: "Work".into(),
        config_dir: work.clone(),
        hive_folder: false,
    });
    let set = Control::SetSettings {
        settings: settings.clone(),
    };
    let saved = Control::Settings {
        settings: settings.clone(),
    };
    assert_eq!(request(&mut app, set).await, saved);
    // A space's account must be one of the accounts (22.5).
    let space = |account: &str| Control::CreateSpace {
        name: "Work".into(),
        env: hive_protocol::SpaceEnv {
            claude_account: Some(account.into()),
            ..Default::default()
        },
    };
    app.send(0, space("/nope")).await;
    let unknown = Control::SpaceFailed {
        message: "No Claude account has the folder /nope".into(),
    };
    assert_eq!(app.any_control().await, (0, unknown));
    // A space with the other account, made current, with the second project.
    app.send(0, space(&work)).await;
    let made = app.any_control().await.1;
    assert!(matches!(made, Control::Spaces { .. }), "{made:?}");
    let add = Control::AddProject {
        path: other.display().to_string(),
    };
    let added = request(&mut app, add).await;
    assert!(matches!(added, Control::ProjectAdded { .. }), "{added:?}");

    // Two terminals at once, each with its project's space's account, whichever space is
    // current; one opened before keeps its own.
    app.open_terminal(2, &other).await;
    app.input(2, echo).await;
    app.output_until(2, &format!("[{work}]")).await;
    app.open_terminal(6, &repo.root).await;
    app.input(6, echo).await;
    app.output_until(6, "[]").await;
    app.input(1, echo).await;
    app.output_until(1, "[]").await;

    // An account a space uses cannot be removed, naming the space.
    let removed = Control::SetSettings {
        settings: Settings::default(),
    };
    let refused = Control::SettingsFailed {
        message: "Work is the Claude account of the space Work: choose another one for it first"
            .into(),
    };
    assert_eq!(request(&mut app, removed).await, refused);
    let select = Control::SelectSpace {
        id: "default".into(),
    };
    app.send(0, select).await;
    let selected = app.any_control().await.1;
    assert!(matches!(selected, Control::Spaces { .. }), "{selected:?}");

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
    // Each opened with its account's name (21.7).
    let named = |worktree: &str, account: &str| Control::TerminalOpened {
        worktree: Some(worktree.into()),
        account: account.into(),
    };
    app.send(3, open(None)).await;
    assert_eq!(app.control().await, (3, named(&root, "Default")));
    app.input(3, echo).await;
    app.output_until(3, "[]").await;
    app.send(5, open(Some(&work))).await;
    assert_eq!(app.control().await, (5, named(&root, "Work")));
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
        hive_folder: false,
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
async fn the_account_selected_before_spaces_had_one_becomes_every_spaces() {
    let repo = Repo::new();
    let work = repo.env.path("home/.claude-work").display().to_string();
    let spaces_file = repo.env.path("data/hive/spaces.json");
    let settings_file = repo.env.path("data/hive/settings.json");
    for file in [&spaces_file, &settings_file] {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    }
    // As 21.10 wrote them: no account in the spaces, the selected one in the settings.
    let spaces = json!({"current": "default", "spaces": [
        {"id": "default", "name": "Default", "env": {"git_name": "Me"}},
        {"id": "space-1", "name": "Work", "env": {}},
    ]});
    std::fs::write(&spaces_file, spaces.to_string()).unwrap();
    let account = json!({"name": "Work", "config_dir": work});
    let settings = json!({"claude": {"accounts": [account], "account": work}});
    std::fs::write(&settings_file, settings.to_string()).unwrap();
    let mut expected = Settings::default();
    expected.claude.accounts.push(Account {
        name: "Work".into(),
        config_dir: work.clone(),
        hive_folder: false,
    });
    // Twice: the second start finds it done.
    for _ in 0..2 {
        let mut daemon = repo.env.daemon();
        let mut app = repo.env.handshake(Role::App).await;
        let settings = Control::Settings {
            settings: expected.clone(),
        };
        assert_eq!(app.control().await, (0, settings));
        app.send(0, Control::ListProjects).await;
        let Control::Spaces { spaces, .. } = app.any_control().await.1 else {
            panic!("expected spaces")
        };
        let accounts: Vec<_> = spaces.iter().map(|s| &s.env.claude_account).collect();
        assert_eq!(accounts, [&Some(work.clone()), &Some(work.clone())]);
        assert_eq!(spaces[0].env.git_name.as_deref(), Some("Me"));
        let saved = std::fs::read_to_string(&settings_file).unwrap();
        assert!(!saved.contains("\"account\""), "{saved}");
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
        hive_folder: false,
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

#[tokio::test]
async fn a_session_moves_into_another_accounts_folder() {
    let repo = Repo::new();
    let root = repo.root.display().to_string();
    let work = repo.env.path("home/.claude-work");
    std::fs::create_dir_all(&work).unwrap();
    let log = session(&repo.env.path("home/.claude"), &root, "a");
    let work = work.display().to_string();
    let mut daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;
    let added = request(&mut app, Control::AddProject { path: root.clone() }).await;
    assert!(matches!(added, Control::ProjectAdded { .. }), "{added:?}");
    let mut settings = Settings::default();
    settings.claude.accounts.push(Account {
        name: "Work".into(),
        config_dir: work.clone(),
        hive_folder: false,
    });
    let set = Control::SetSettings {
        settings: settings.clone(),
    };
    assert_eq!(request(&mut app, set).await, Control::Settings { settings });
    let mut ask = async |id: &str, config_dir: Option<&str>| {
        let config_dir = config_dir.map(Into::into);
        let id = id.into();
        app.send(0, Control::MoveSession { id, config_dir }).await;
        loop {
            if let answer @ (Control::SessionMoved { .. } | Control::MoveSessionFailed { .. }) =
                app.control().await.1
            {
                return answer;
            }
        }
    };
    // Only into an account's folder.
    assert_eq!(
        ask("a", Some("/nope")).await,
        Control::MoveSessionFailed {
            id: "a".into(),
            message: "/nope is not a Claude account".into(),
        }
    );
    assert!(log.exists());
    assert_eq!(
        ask("a", Some(&work)).await,
        Control::SessionMoved {
            id: "a".into(),
            config_dir: Some(work.clone()),
        }
    );
    let moved = Path::new(&work)
        .join("projects")
        .join(log.parent().unwrap().file_name().unwrap())
        .join("a.jsonl");
    assert!(moved.exists() && !log.exists());
    let Control::MoveSessionFailed { message, .. } = ask("b", None).await else {
        panic!("expected a refusal")
    };
    assert_eq!(message, "no session b in the followed projects");
    // Listed once, in the account it now lives in.
    app.send(0, Control::ListSessions).await;
    let sessions = loop {
        if let (0, Control::Sessions { sessions, .. }) = app.control().await {
            break sessions;
        }
    };
    let listed: Vec<_> = (sessions.iter())
        .map(|s| (s.id.as_str(), s.config_dir.as_deref()))
        .collect();
    assert_eq!(listed, [("a", Some(work.as_str()))]);
    drop(app);
    assert!(daemon.wait_exit().success());
}

// Unix: the folders' mode.
#[cfg(unix)]
#[tokio::test]
async fn hive_makes_a_new_accounts_private_folder_and_deletes_only_its_own() {
    use std::os::unix::fs::PermissionsExt;
    let env = crate::common::Env::new();
    let mut daemon = env.daemon();
    // Hive's folder, named by `HIVE_HOME`, and its `accounts/` are private from the start.
    let mode = |dir: &Path| std::fs::metadata(dir).unwrap().permissions().mode() & 0o777;
    let accounts = env.path("data/hive/accounts");
    assert_eq!(mode(&env.path("data/hive")), 0o700);
    assert_eq!(mode(&accounts), 0o700);
    let mut app = env.connect(Role::App).await;
    let create = |name: &str| Control::CreateAccount { name: name.into() };
    let Control::Settings { settings } = request(&mut app, create("Work")).await else {
        panic!("not created");
    };
    let work = accounts.join("work");
    let made = Account {
        name: "Work".into(),
        config_dir: work.display().to_string(),
        hive_folder: true,
    };
    assert_eq!(settings.claude.accounts, std::slice::from_ref(&made));
    assert_eq!(mode(&work), 0o700);
    let failed = request(&mut app, create("..")).await;
    let message = "\"..\" cannot name a folder: give the account a letter or a digit".into();
    assert_eq!(failed, Control::SettingsFailed { message });
    // Removed with its folder; a folder Hive did not make is refused.
    let remove = |config_dir: &str, delete_folder| Control::RemoveAccount {
        config_dir: config_dir.into(),
        delete_folder,
    };
    let home = env.path("home").display().to_string();
    let message = format!("Hive did not make {home}: it is never deleted");
    let refused = request(&mut app, remove(&home, true)).await;
    assert_eq!(refused, Control::SettingsFailed { message });
    let Control::Settings { settings } = request(&mut app, remove(&made.config_dir, true)).await
    else {
        panic!("not removed");
    };
    assert!(settings.claude.accounts.is_empty());
    assert!(!work.exists());
    drop(app);
    assert!(daemon.wait_exit().success());
}
