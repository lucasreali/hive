use hive_protocol::{
    Control, GhAccount, GhLogin, ProjectError, Role, Space, SpaceEnv, SpaceGhAccount,
};
use serde_json::json;

use crate::agents::hook;
use crate::common::Conn;
use crate::worktree::Repo;

/// Sends `message` and returns the next control message, `spaces` included.
async fn ask(conn: &mut Conn, message: Control) -> Control {
    conn.send(0, message).await;
    conn.any_control().await.1
}

/// The next control message but a sessions list pushed meanwhile, `spaces` included: once the
/// app has a list, the service sends it again whenever a burst in Claude's folders makes it
/// change (D19.4), as a session watch followed again (each `list_sessions`) may report later
/// on a loaded machine. Such a list holds only the session `s`, or nothing.
async fn past_sessions(conn: &mut Conn) -> Control {
    loop {
        match conn.any_control().await.1 {
            Control::Sessions { sessions, .. } => {
                assert!(sessions.iter().all(|s| s.id == "s"), "{sessions:?}");
            }
            other => return other,
        }
    }
}

/// [`ask`] past a sessions list pushed meanwhile ([`past_sessions`]).
async fn ask_past_sessions(conn: &mut Conn, message: Control) -> Control {
    conn.send(0, message).await;
    past_sessions(conn).await
}

fn spaces(spaces: &[&Space], current: &str) -> Control {
    Control::Spaces {
        spaces: spaces.iter().map(|s| (*s).clone()).collect(),
        current: current.into(),
    }
}

fn failed(message: &str) -> Control {
    Control::SpaceFailed {
        message: message.into(),
    }
}

#[tokio::test]
async fn spaces_group_projects_and_give_their_terminals_an_identity() {
    let repo = Repo::new();
    let root = repo.root.display().to_string();
    // The default Claude account's sessions (the accounts are `accounts.rs`', 12.2).
    let logs = repo.env.path("home/.claude/projects").join(
        root.chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect::<String>(),
    );
    std::fs::create_dir_all(&logs).unwrap();
    let log = [
        json!({"type": "user", "cwd": root, "message": {"content": "task"}}),
        json!({"type": "ai-title", "aiTitle": "Work task"}),
    ];
    std::fs::write(logs.join("s.jsonl"), format!("{}\n{}\n", log[0], log[1])).unwrap();
    let mut daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;

    // Before any change: one default space, sent before the projects.
    let mut default = Space {
        id: "default".into(),
        name: "Default".into(),
        projects: vec![],
        groups: vec![],
        env: SpaceEnv::default(),
    };
    assert_eq!(
        ask(&mut app, Control::ListProjects).await,
        spaces(&[&default], "default")
    );
    let none = Control::Projects { projects: vec![] };
    assert_eq!(app.control().await, (0, none));

    // A refused request changes nothing.
    let blank = Control::CreateSpace {
        name: " ".into(),
        env: SpaceEnv::default(),
    };
    assert_eq!(
        ask(&mut app, blank).await,
        failed("Enter a name for the space")
    );

    let env = SpaceEnv {
        git_name: Some("Work Me".into()),
        git_email: Some("me@work".into()),
        gh_account: None,
        claude_account: None,
    };
    let mut work = Space {
        id: "space-1".into(),
        name: "Work".into(),
        projects: vec![],
        groups: vec![],
        env: env.clone(),
    };
    let create = Control::CreateSpace {
        name: "Work".into(),
        env,
    };
    assert_eq!(
        ask(&mut app, create).await,
        spaces(&[&default, &work], "space-1")
    );

    // A new project joins the current space.
    let add = |path: &str| Control::AddProject { path: path.into() };
    work.projects.push(root.clone());
    assert_eq!(
        ask(&mut app, add(&root)).await,
        spaces(&[&default, &work], "space-1")
    );
    let added = app.control().await.1;
    assert!(matches!(added, Control::ProjectAdded { .. }), "{added:?}");

    // Its terminals get the space's identity (the default Claude account: it names none).
    app.open_terminal(1, &repo.root).await;
    let echo = "echo \"[$CLAUDE_CONFIG_DIR]$GIT_AUTHOR_NAME|$GIT_COMMITTER_NAME|$GIT_AUTHOR_EMAIL|$GIT_COMMITTER_EMAIL.\"\r";
    app.input(1, echo).await;
    let expected = "[]Work Me|Work Me|me@work|me@work.";
    app.output_until(1, expected).await;
    let start = json!({"session_id": "s", "cwd": root});
    let seen = hook(&repo, &mut app, "1", "SessionStart", start).await;
    let title = Control::AgentTitle {
        id: "s".into(),
        title: "Work task".into(),
    };
    assert!(seen.contains(&(1, title)), "{seen:?}");
    let Control::Sessions {
        sessions, error, ..
    } = ask(&mut app, Control::ListSessions).await
    else {
        panic!("expected sessions")
    };
    let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
    assert_eq!((ids, error), (vec!["s"], None));

    // Another space shows none of them, and cannot take its project.
    let select = |id: &str| Control::SelectSpace { id: id.into() };
    assert_eq!(
        ask_past_sessions(&mut app, select("default")).await,
        spaces(&[&default, &work], "default")
    );
    let listed = ask(&mut app, Control::ListSessions).await;
    let empty = Control::Sessions {
        sessions: vec![],
        error: None,
        truncated: false,
    };
    assert_eq!(listed, empty);
    let refused = ask_past_sessions(&mut app, add(&root)).await;
    let other = Control::AddProjectFailed {
        path: root.clone(),
        error: ProjectError::InOtherSpace,
        message: format!("{root} is already in the space Work"),
    };
    assert_eq!(refused, other);
    // Adding it again to its own space changes nothing.
    ask_past_sessions(&mut app, select("space-1")).await;
    assert_eq!(
        ask_past_sessions(&mut app, add(&root)).await,
        spaces(&[&default, &work], "space-1")
    );
    let again = past_sessions(&mut app).await;
    assert!(matches!(again, Control::ProjectAdded { .. }), "{again:?}");

    // Only an empty space can be deleted.
    let delete = |id: &str| Control::DeleteSpace { id: id.into() };
    let kept = ask_past_sessions(&mut app, delete("space-1")).await;
    assert_eq!(
        kept,
        failed("Work has projects: only an empty space can be deleted")
    );
    let rename = Control::UpdateSpace {
        id: "default".into(),
        name: "Personal".into(),
        env: SpaceEnv::default(),
    };
    default.name = "Personal".into();
    assert_eq!(
        ask_past_sessions(&mut app, rename).await,
        spaces(&[&default, &work], "space-1")
    );
    assert_eq!(
        ask_past_sessions(&mut app, delete("default")).await,
        spaces(&[&work], "space-1")
    );
    drop(app);
    assert!(daemon.wait_exit().success());

    // They are kept for the next start.
    let file = repo.env.path("data/hive/spaces.json");
    let mode =
        std::os::unix::fs::PermissionsExt::mode(&std::fs::metadata(&file).unwrap().permissions());
    assert_eq!(mode & 0o777, 0o600);
    let mut daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;
    // The agent's session, to resume, comes first.
    let restore = app.control().await.1;
    assert!(
        matches!(restore, Control::RestoreSessions { .. }),
        "{restore:?}"
    );
    assert_eq!(
        ask(&mut app, Control::ListProjects).await,
        spaces(&[&work], "space-1")
    );
    drop(app);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn a_terminal_gets_the_space_of_the_project_its_folder_resolves_into() {
    let repo = Repo::new();
    let root = repo.root.display().to_string();
    // A second project, in the default space, with a link into the first.
    let other = repo.root.with_file_name("other");
    std::fs::create_dir(&other).unwrap();
    repo.git_in(&other, &["init", "-q", "-b", "main"]);
    std::os::unix::fs::symlink(&repo.root, other.join("link")).unwrap();
    let other = other.display().to_string();
    let mut daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;
    let add = |path: &str| Control::AddProject { path: path.into() };
    ask(&mut app, add(&other)).await;
    let added = app.control().await.1;
    assert!(matches!(added, Control::ProjectAdded { .. }), "{added:?}");
    let env = SpaceEnv {
        git_name: Some("Work Me".into()),
        ..SpaceEnv::default()
    };
    let create = Control::CreateSpace {
        name: "Work".into(),
        env,
    };
    let created = ask(&mut app, create).await;
    assert!(matches!(created, Control::Spaces { .. }), "{created:?}");
    ask(&mut app, add(&root)).await;
    let added = app.control().await.1;
    assert!(matches!(added, Control::ProjectAdded { .. }), "{added:?}");

    // Both folders lie in `other` by their names, but in the Work project once resolved.
    let echo = "echo \"name=$GIT_AUTHOR_NAME.\"\r";
    for (channel, cwd) in [
        (1, format!("{other}/link")),
        (2, format!("{other}/../repo")),
    ] {
        app.open_terminal(channel, std::path::Path::new(&cwd)).await;
        app.input(channel, echo).await;
        app.output_until(channel, "name=Work Me.").await;
    }
    drop(app);
    assert!(daemon.wait_exit().success());
}

/// A fake `gh` in `<env>/fake-gh`, never the real one: it logs each call's arguments and
/// `GH_TOKEN` to `<env>/gh.log`, lists two accounts and prints `tok-<login>` as a token.
fn fake_gh(repo: &Repo) -> std::path::PathBuf {
    let dir = repo.env.path("fake-gh");
    std::fs::create_dir(&dir).unwrap();
    let log = repo.env.path("gh.log");
    let script = format!(
        r#"#!/bin/sh
printf '%s|%s\n' "$*" "${{GH_TOKEN-}}" >> '{}'
case "$1 $2" in
  "auth status") printf '%s\n' github.com \
    '  ✓ Logged in to github.com account octo-personal (/x/hosts.yml)' \
    '  - Active account: true' \
    '  ✓ Logged in to github.com account octo-work (/x/hosts.yml)' \
    '  - Active account: false' ;;
  "auth token") [ "$6" != octo-gone ] || {{ echo 'no oauth token found for octo-gone' >&2; exit 1; }}; echo "tok-$6" ;;
  "api user") [ "$GH_TOKEN" = tok-octo-work ] || {{ echo 'HTTP 401: Bad credentials' >&2; exit 1; }}; echo '{{"login":"octo-work","id":42}}' ;;
esac
"#,
        log.display()
    );
    let gh = dir.join("gh");
    std::fs::write(&gh, script).unwrap();
    let mode = std::os::unix::fs::PermissionsExt::from_mode(0o755);
    std::fs::set_permissions(&gh, mode).unwrap();
    dir
}

fn gh_account(login: &str) -> GhAccount {
    GhAccount {
        host: "github.com".into(),
        login: login.into(),
    }
}

#[tokio::test]
async fn each_space_gives_its_terminals_its_own_github_account() {
    let repo = Repo::new();
    let root = repo.root.display().to_string();
    let other = repo.root.with_file_name("other");
    std::fs::create_dir(&other).unwrap();
    repo.git_in(&other, &["init", "-q", "-b", "main"]);
    // First on the user's `PATH` through their (temporary) shell config, as a real install
    // would be: on macOS the login shell puts the system folders, where CI has a real gh,
    // before the service's own `PATH`.
    let fake = fake_gh(&repo);
    let fish = repo.env.path("config/fish");
    std::fs::create_dir_all(&fish).unwrap();
    let config = format!("set -gx PATH '{}' $PATH\n", fake.display());
    std::fs::write(fish.join("config.fish"), config).unwrap();
    // A token in the service's own environment never reaches gh.
    let mut hive = repo.env.hive();
    hive.env("GH_TOKEN", "tok-leaked");
    let mut daemon = repo.env.daemon_with(&mut hive);
    let mut app = repo.env.connect(Role::App).await;

    // The accounts logged in to gh, logins only.
    let listed = ask(
        &mut app,
        Control::ListGhAccounts {
            gh_config_dir: None,
        },
    )
    .await;
    let login = |login: &str, active: bool| GhLogin {
        host: "github.com".into(),
        login: login.into(),
        active,
        logged_in: true,
    };
    let accounts = Control::GhAccounts {
        gh_config_dir: None,
        accounts: vec![login("octo-personal", true), login("octo-work", false)],
        problem: None,
    };
    assert_eq!(listed, accounts);

    // The default space uses the personal account, a Work space the work one, from a gh
    // config folder of its own (24.13).
    let mut replies = vec![listed];
    let folder = repo.env.path("gh-work").display().to_string();
    std::fs::create_dir(&folder).unwrap();
    let space = |login: &str| SpaceEnv {
        gh_account: Some(SpaceGhAccount {
            account: gh_account(login),
            config_dir: login.ends_with("work").then(|| folder.clone()),
        }),
        ..SpaceEnv::default()
    };
    let rename = Control::UpdateSpace {
        id: "default".into(),
        name: "Personal".into(),
        env: space("octo-personal"),
    };
    replies.push(ask(&mut app, rename).await);
    let add = |path: &std::path::Path| Control::AddProject {
        path: path.display().to_string(),
    };
    replies.push(ask(&mut app, add(&other)).await);
    replies.push(app.control().await.1);
    // Only a folder of the settings' (24.13).
    let unknown = Control::CreateSpace {
        name: "Work".into(),
        env: space("octo-work"),
    };
    let refused =
        format!("{folder} is not a gh config folder of Settings → Accounts: add it there first");
    let refused = Control::SpaceFailed { message: refused };
    assert_eq!(ask(&mut app, unknown).await, refused);
    // A blank folder is gh's own config.
    let mut own = space("octo-personal");
    if let Some(account) = &mut own.gh_account {
        account.config_dir = Some(" ".into());
    }
    let blank = Control::UpdateSpace {
        id: "default".into(),
        name: "Personal".into(),
        env: own,
    };
    let Control::Spaces { spaces, .. } = ask(&mut app, blank).await else {
        panic!("expected spaces")
    };
    assert_eq!(spaces[0].env, space("octo-personal"));
    let mut settings = hive_protocol::Settings::default();
    settings.gh.config_dirs.push(format!("{folder}/"));
    let saved = ask(&mut app, Control::SetSettings { settings }).await;
    assert!(matches!(saved, Control::Settings { .. }), "{saved:?}");
    let create = Control::CreateSpace {
        name: "Work".into(),
        env: space("octo-work"),
    };
    replies.push(ask(&mut app, create).await);
    replies.push(ask(&mut app, add(&repo.root)).await);
    replies.push(app.control().await.1);
    let Control::Spaces { spaces, .. } = &replies[5] else {
        panic!("expected spaces: {:?}", replies[5])
    };
    assert_eq!(spaces[1].env, space("octo-work"));
    assert_eq!(spaces[1].projects, [root]);
    let echo = "echo \"token=$GH_TOKEN|$GH_HOST|$GH_CONFIG_DIR.\"\r";
    for (channel, cwd, token, dir) in [
        (1, repo.root.clone(), "tok-octo-work", folder.as_str()),
        (2, other.clone(), "tok-octo-personal", ""),
    ] {
        app.open_terminal(channel, &cwd).await;
        app.input(channel, echo).await;
        let expected = format!("token={token}|github.com|{dir}.");
        app.output_until(channel, &expected).await;
    }
    // An account gh has no token for: the terminal opens anyway, and the human is told.
    let gone = Control::UpdateSpace {
        id: "space-1".into(),
        name: "Work".into(),
        env: space("octo-gone"),
    };
    replies.push(ask(&mut app, gone).await);
    let cwd = repo.root.display().to_string();
    let (cols, rows) = (80, 24);
    let account = None;
    let open = Control::OpenTerminal {
        cwd,
        cols,
        rows,
        account,
    };
    app.send(3, open).await;
    let notice = Control::Notice {
        message: "No GitHub token for octo-gone on github.com: this terminal uses gh's active account (gh auth token --hostname github.com --user octo-gone failed: no oauth token found for octo-gone)".into(),
    };
    assert_eq!(app.control().await, (0, notice));
    let worktree = Some(repo.root.display().to_string());
    assert_eq!(
        app.control().await,
        (
            3,
            Control::TerminalOpened {
                worktree,
                account: "Default".into()
            }
        )
    );

    // gh's own active account changes only when asked for.
    let switch = Control::SwitchGhAccount {
        gh_config_dir: None,
        account: gh_account("octo-work"),
    };
    replies.push(ask(&mut app, switch).await);
    assert_eq!(replies.last(), Some(&accounts));
    let bad = Control::SwitchGhAccount {
        gh_config_dir: Some("relative".into()),
        account: gh_account("octo-work"),
    };
    let refused = Control::GhAccounts {
        gh_config_dir: Some("relative".into()),
        accounts: vec![],
        problem: Some("The GitHub CLI config folder must be an absolute path".into()),
    };
    assert_eq!(ask(&mut app, bad).await, refused);

    // The app got logins, never a token; gh never got the service's token.
    for reply in &replies {
        let json = serde_json::to_string(reply).unwrap();
        assert!(!json.contains("tok-"), "{json}");
    }
    let log = std::fs::read_to_string(repo.env.path("gh.log")).unwrap();
    let calls: Vec<&str> = log.lines().collect();
    assert_eq!(
        calls,
        [
            "auth status|",
            "auth token --hostname github.com --user octo-work|",
            "auth token --hostname github.com --user octo-personal|",
            "auth token --hostname github.com --user octo-gone|",
            "auth switch --hostname github.com --user octo-work|",
            "auth status|",
        ]
    );
    drop(app);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn a_remote_base_is_fetched_as_the_space_and_never_prompts() {
    let repo = Repo::new();
    let root = repo.root.display().to_string();
    let fake = fake_gh(&repo);
    let fish = repo.env.path("config/fish");
    std::fs::create_dir_all(&fish).unwrap();
    let config = format!("set -gx PATH '{}' $PATH\n", fake.display());
    std::fs::write(fish.join("config.fish"), config).unwrap();
    // The remote answers through a fake ssh that logs what git gave it, then fails.
    let log = repo.env.path("ssh.log");
    let ssh = repo.env.path("ssh");
    let script = format!(
        "#!/bin/sh\nprintf '%s|%s|%s|%s\\n' \"$GH_TOKEN\" \"$GIT_TERMINAL_PROMPT\" \"$GCM_INTERACTIVE\" \"$WSLENV\" >> '{}'\necho 'ssh: no network here' >&2\nexit 255\n",
        log.display()
    );
    std::fs::write(&ssh, script).unwrap();
    let mode = std::os::unix::fs::PermissionsExt::from_mode(0o755);
    std::fs::set_permissions(&ssh, mode).unwrap();
    repo.git(&["remote", "add", "origin", "ssh://example.invalid/x.git"]);
    repo.git(&["config", "core.sshCommand", ssh.to_str().unwrap()]);
    repo.git(&["update-ref", "refs/remotes/origin/main", "HEAD"]);
    // Under WSL a Windows credential helper sees only what `WSLENV` lists.
    let mut hive = repo.env.hive();
    hive.env("WSLENV", "WT_SESSION");
    let mut daemon = repo.env.daemon_with(&mut hive);
    let mut app = repo.env.connect(Role::App).await;
    let work = Control::UpdateSpace {
        id: "default".into(),
        name: "Work".into(),
        env: SpaceEnv {
            gh_account: Some(SpaceGhAccount {
                account: gh_account("octo-work"),
                config_dir: None,
            }),
            ..SpaceEnv::default()
        },
    };
    ask(&mut app, work).await;
    let add = Control::AddProject { path: root.clone() };
    ask(&mut app, add).await;
    app.control().await;

    let create = Control::CreateWorktree {
        project: root.clone(),
        name: "fetched".into(),
        base: Some("origin/main".into()),
    };
    app.send(0, create).await;
    let Control::WorktreeCreated { path, notes, .. } = app.control().await.1 else {
        panic!("expected a new worktree")
    };
    assert_eq!(path, format!("{root}/.claude/worktrees/fetched"));
    assert_eq!(
        notes,
        ["could not fetch origin/main: ssh: no network here; created from the last fetched copy"]
    );
    let log = std::fs::read_to_string(log).unwrap();
    let quiet = "GIT_TERMINAL_PROMPT:GCM_INTERACTIVE";
    assert_eq!(log, format!("tok-octo-work|0|never|WT_SESSION:{quiet}\n"));
    drop(app);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn an_account_identity_is_its_login_and_noreply_email() {
    let repo = Repo::new();
    let fake = fake_gh(&repo);
    let fish = repo.env.path("config/fish");
    std::fs::create_dir_all(&fish).unwrap();
    let config = format!("set -gx PATH '{}' $PATH\n", fake.display());
    std::fs::write(fish.join("config.fish"), config).unwrap();
    let mut daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;
    let get = |dir: Option<&str>, login: &str| Control::GetGhIdentity {
        gh_config_dir: dir.map(str::to_owned),
        account: gh_account(login),
    };
    let found = Control::GhIdentity {
        account: gh_account("octo-work"),
        name: "octo-work".into(),
        email: "42+octo-work@users.noreply.github.com".into(),
    };
    assert_eq!(ask(&mut app, get(None, "octo-work")).await, found);
    let failed = |login: &str, message: &str| Control::GhIdentityFailed {
        account: gh_account(login),
        message: message.into(),
    };
    let refused = failed(
        "octo-personal",
        "Could not get the identity of octo-personal on github.com: gh api user failed: HTTP 401: Bad credentials",
    );
    assert_eq!(ask(&mut app, get(None, "octo-personal")).await, refused);
    let relative = failed(
        "octo-work",
        "The GitHub CLI config folder must be an absolute path",
    );
    assert_eq!(ask(&mut app, get(Some("rel"), "octo-work")).await, relative);
    drop(app);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn without_gh_the_accounts_say_so() {
    let repo = Repo::new();
    let empty = repo.env.path("no-programs");
    std::fs::create_dir(&empty).unwrap();
    let mut daemon = repo.env.daemon_on_path(&empty);
    let mut app = repo.env.connect(Role::App).await;
    let listed = ask(
        &mut app,
        Control::ListGhAccounts {
            gh_config_dir: None,
        },
    )
    .await;
    let missing = Control::GhAccounts {
        gh_config_dir: None,
        accounts: vec![],
        problem: Some("gh (the GitHub CLI) is not installed".into()),
    };
    assert_eq!(listed, missing);
    drop(app);
    assert!(daemon.wait_exit().success());
}
