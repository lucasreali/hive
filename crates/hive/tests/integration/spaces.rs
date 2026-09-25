use hive_protocol::{Control, ProjectError, Role, Space, SpaceEnv};
use serde_json::json;

use crate::agents::hook;
use crate::common::Conn;
use crate::worktree::Repo;

/// Sends `message` and returns the next control message, `spaces` included.
async fn ask(conn: &mut Conn, message: Control) -> Control {
    conn.send(0, message).await;
    conn.any_control().await.1
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
    let claude = repo.env.path("home/work-claude");
    let logs = claude.join("projects").join(
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
    let claude = claude.display().to_string();
    let mut daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;

    // Before any change: one default space, sent before the projects.
    let mut default = Space {
        id: "default".into(),
        name: "Default".into(),
        projects: vec![],
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
        claude_config_dir: Some(claude.clone()),
        git_name: Some("Work Me".into()),
        git_email: Some("me@work".into()),
        gh_config_dir: Some(claude.clone()),
    };
    let mut work = Space {
        id: "space-1".into(),
        name: "Work".into(),
        projects: vec![],
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

    // Its terminals get the space's identity, and its agents' sessions are in its folder.
    app.open_terminal(1, &repo.root).await;
    let echo = "echo \"$CLAUDE_CONFIG_DIR|$GIT_AUTHOR_NAME|$GIT_COMMITTER_NAME|$GIT_AUTHOR_EMAIL|$GIT_COMMITTER_EMAIL|$GH_CONFIG_DIR\"\r";
    app.input(1, echo).await;
    let expected = format!("{claude}|Work Me|Work Me|me@work|me@work|{claude}");
    app.output_until(1, &expected).await;
    let start = json!({"session_id": "s", "cwd": root});
    let seen = hook(&repo, &mut app, "1", "SessionStart", start).await;
    let title = Control::AgentTitle {
        id: "s".into(),
        title: "Work task".into(),
    };
    assert!(seen.contains(&(1, title)), "{seen:?}");
    let Control::Sessions { sessions, error } = ask(&mut app, Control::ListSessions).await else {
        panic!("expected sessions")
    };
    let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
    assert_eq!((ids, error), (vec!["s"], None));

    // Another space shows none of them, and cannot take its project.
    let select = |id: &str| Control::SelectSpace { id: id.into() };
    assert_eq!(
        ask(&mut app, select("default")).await,
        spaces(&[&default, &work], "default")
    );
    let listed = ask(&mut app, Control::ListSessions).await;
    let empty = Control::Sessions {
        sessions: vec![],
        error: None,
    };
    assert_eq!(listed, empty);
    let refused = ask(&mut app, add(&root)).await;
    let other = Control::AddProjectFailed {
        path: root.clone(),
        error: ProjectError::InOtherSpace,
        message: format!("{root} is already in the space Work"),
    };
    assert_eq!(refused, other);
    // Adding it again to its own space changes nothing.
    ask(&mut app, select("space-1")).await;
    assert_eq!(
        ask(&mut app, add(&root)).await,
        spaces(&[&default, &work], "space-1")
    );
    let again = app.control().await.1;
    assert!(matches!(again, Control::ProjectAdded { .. }), "{again:?}");

    // Only an empty space can be deleted.
    let delete = |id: &str| Control::DeleteSpace { id: id.into() };
    let kept = ask(&mut app, delete("space-1")).await;
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
        ask(&mut app, rename).await,
        spaces(&[&default, &work], "space-1")
    );
    assert_eq!(
        ask(&mut app, delete("default")).await,
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
