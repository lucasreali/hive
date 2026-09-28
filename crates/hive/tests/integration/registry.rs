//! The app's worktrees follow git's worktree registry, however a worktree is added or
//! removed (9.36).

use std::time::Duration;

use hive_protocol::{Control, Role, SpaceEnv};

use crate::common::Conn;
use crate::worktree::Repo;

/// Longer than a change of the registry takes to reach the app.
const SETTLED: Duration = Duration::from_millis(1500);

/// The worktree names of the only project in the next `projects`, the main one first.
async fn names(app: &mut Conn) -> Vec<String> {
    let (0, Control::Projects { projects }) = app.control().await else {
        panic!("expected projects");
    };
    assert_eq!(projects.len(), 1);
    let mut names: Vec<String> = projects[0]
        .worktrees
        .iter()
        .map(|w| w.name.clone())
        .collect();
    names[1..].sort();
    names
}

/// No `projects` (nor anything else) comes once a change would have reached the app: the
/// next message answers a request sent now.
async fn nothing_more(app: &mut Conn) {
    tokio::time::sleep(SETTLED).await;
    app.send(0, Control::GetSettings).await;
    let next = app.any_control().await;
    assert!(matches!(next, (0, Control::Settings { .. })), "{next:?}");
}

/// A repository followed by a running service, with an app connected.
async fn followed(repo: &Repo) -> (crate::common::Daemon, Conn) {
    let daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;
    let path = repo.root.display().to_string();
    app.send(0, Control::AddProject { path }).await;
    assert!(matches!(
        app.control().await,
        (0, Control::ProjectAdded { .. })
    ));
    (daemon, app)
}

#[tokio::test]
async fn worktrees_added_or_removed_outside_hive_reach_the_app() {
    let repo = Repo::new();
    let (mut daemon, mut app) = followed(&repo).await;
    let outside = repo.env.path("home/outside");
    let outside = outside.to_str().unwrap();

    // The repository's first linked worktree: git makes its registry then.
    repo.git(&["worktree", "add", "-q", "-b", "outside", outside]);
    assert_eq!(names(&mut app).await, ["main", "outside"]);
    repo.git(&["worktree", "remove", outside]);
    assert_eq!(names(&mut app).await, ["main"]);

    // `hive worktree` from a terminal.
    assert!(repo.hive(&["create", "a"]).status.success());
    assert_eq!(names(&mut app).await, ["main", "a"]);
    assert!(repo.hive(&["remove", "a"]).status.success());
    assert_eq!(names(&mut app).await, ["main"]);

    // A folder deleted by hand and then pruned.
    assert!(repo.hive(&["create", "b"]).status.success());
    assert_eq!(names(&mut app).await, ["main", "b"]);
    std::fs::remove_dir_all(repo.root.join(".claude/worktrees/b")).unwrap();
    repo.git(&["worktree", "prune"]);
    assert_eq!(names(&mut app).await, ["main"]);

    // The app's own requests are answered once: the change is already in the answer.
    let create = Control::CreateWorktree {
        project: repo.root.display().to_string(),
        name: "c".into(),
        base: None,
    };
    app.send(0, create).await;
    assert!(matches!(
        app.control().await,
        (0, Control::WorktreeCreated { .. })
    ));
    nothing_more(&mut app).await;
    drop(app);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn a_burst_of_worktrees_is_one_message() {
    let repo = Repo::new();
    let (mut daemon, mut app) = followed(&repo).await;
    assert!(repo.hive(&["create", "first"]).status.success());
    assert_eq!(names(&mut app).await, ["main", "first"]);

    let names_added = ["w1", "w2", "w3", "w4", "w5"];
    for name in names_added {
        let path = repo.env.path(&format!("home/{name}"));
        repo.git(&["worktree", "add", "-q", "-b", name, path.to_str().unwrap()]);
    }
    let expected = [&["main", "first"][..], &names_added].concat();
    assert_eq!(names(&mut app).await, expected);
    nothing_more(&mut app).await;
    drop(app);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn only_the_current_spaces_projects_are_watched() {
    let repo = Repo::new();
    let (mut daemon, mut app) = followed(&repo).await;
    let add = |name: &str| {
        let path = repo.env.path(&format!("home/{name}"));
        repo.git(&["worktree", "add", "-q", "-b", name, path.to_str().unwrap()]);
    };
    let ask = async |app: &mut Conn, message: Control| {
        app.send(0, message).await;
        let next = app.any_control().await;
        assert!(matches!(next, (0, Control::Spaces { .. })), "{next:?}");
    };

    // Another space: the project is not in it.
    let work = Control::CreateSpace {
        name: "Work".into(),
        env: SpaceEnv::default(),
    };
    ask(&mut app, work).await;
    // Also lets the watch follow the switch.
    nothing_more(&mut app).await;
    add("away");
    nothing_more(&mut app).await;
    // Back in its space, it is watched again, and what changed meanwhile is sent.
    let back = Control::SelectSpace {
        id: "default".into(),
    };
    ask(&mut app, back).await;
    assert_eq!(names(&mut app).await, ["main", "away"]);
    add("back");
    assert_eq!(names(&mut app).await, ["main", "away", "back"]);

    // A removed project is no longer watched.
    let id = repo.root.display().to_string();
    app.send(0, Control::RemoveProject { id }).await;
    loop {
        if let (0, Control::ProjectRemoved { .. }) = app.control().await {
            break;
        }
    }
    add("gone");
    nothing_more(&mut app).await;
    drop(app);
    assert!(daemon.wait_exit().success());
}
