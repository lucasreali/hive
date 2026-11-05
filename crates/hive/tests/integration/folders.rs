//! A plain folder followed as a folder project (23.1): its files panel, its own terminals and
//! agents, and `git init` making it a project.

use hive_protocol::{Control, DiffBase, Role};
use serde_json::json;

use crate::agents::hook;
use crate::worktree::Repo;

#[tokio::test]
async fn a_folder_project_holds_its_agents_and_git_init_makes_it_a_project() {
    let repo = Repo::new();
    std::fs::create_dir_all(repo.env.path("home/notes/docs")).unwrap();
    std::fs::write(repo.env.path("home/notes/todo.md"), "- one\n").unwrap();
    let dir = repo.env.path("home/notes").canonicalize().unwrap();
    let f = dir.display().to_string();
    let mut daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;

    // Added: the list, then the folder project, a group of no repository.
    app.send(0, Control::AddProject { path: f.clone() }).await;
    let (0, Control::Projects { projects }) = app.control().await else {
        panic!("expected projects");
    };
    assert_eq!(projects.len(), 1);
    assert_eq!((projects[0].group, projects[0].folder), (true, true));
    let (0, Control::ProjectAdded { project }) = app.control().await else {
        panic!("expected project_added");
    };
    assert_eq!(project, projects[0]);
    assert_eq!(project.worktrees[0].path, f);

    // Its files panel lists and reads its files, with no changes and no base.
    let watch = Control::WatchWorktree {
        path: f.clone(),
        base: DiffBase::Head,
    };
    app.send(0, watch).await;
    let (0, Control::Files { path, files, .. }) = app.control().await else {
        panic!("expected files");
    };
    assert_eq!((path, files), (f.clone(), vec!["todo.md".to_owned()]));
    let open = Control::OpenFile {
        worktree: f.clone(),
        path: "todo.md".into(),
        base: DiffBase::Head,
    };
    app.send(0, open).await;
    let (
        0,
        Control::File {
            content,
            base,
            error,
            ..
        },
    ) = app.control().await
    else {
        panic!("expected file");
    };
    assert_eq!(
        (content.as_deref(), base, error),
        (Some("- one\n"), None, None)
    );

    // A terminal in it runs there, without the worktrees' `HIVE_*`; its agent is under it.
    assert_eq!(app.open_terminal(1, &dir).await, Some(f.clone()));
    app.input(1, "echo \"root=$HIVE_ROOT_PATH.\"\n").await;
    app.output_until(1, "root=.").await;
    let docs = dir.join("docs").display().to_string();
    let start = json!({"session_id": "s1", "cwd": docs});
    let seen = hook(&repo, &mut app, "1", "SessionStart", start).await;
    let placed = Control::AgentDetected {
        id: "s1".into(),
        project: Some(f.clone()),
        worktree: Some(f.clone()),
        cwd: Some(docs.clone()),
    };
    assert_eq!(seen[0], (1, placed));

    // "Initialize git": the watch of its folder makes it a project, with no new add.
    app.send(0, Control::InitGit { id: f.clone() }).await;
    let projects = loop {
        match app.control().await {
            (0, Control::Projects { projects }) if !projects[0].group => break projects,
            (0, Control::Projects { .. } | Control::Files { .. } | Control::Error { .. }) => {}
            (1, _) => {}
            other => panic!("expected projects, got {other:?}"),
        }
    };
    assert_eq!(projects.len(), 1);
    assert_eq!(
        (projects[0].id.as_str(), projects[0].folder),
        (f.as_str(), false)
    );
    assert!(dir.join(".git").is_dir());
    // Kept so: no longer a group in the spaces file.
    let file = repo.env.path("data/hive/spaces.json");
    let saved: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(saved["spaces"][0]["projects"], json!([f]));
    assert_eq!(saved["spaces"][0].get("groups"), None);
    // A project now, not a folder project: asked again, refused.
    app.send(0, Control::InitGit { id: f.clone() }).await;
    let refused = Control::Error {
        message: format!("{f} is not a folder project"),
    };
    loop {
        match app.control().await {
            (0, message) if message == refused => break,
            (0, Control::Projects { .. } | Control::Files { .. } | Control::Changes { .. }) => {}
            (1, _) => {}
            other => panic!("expected the refusal, got {other:?}"),
        }
    }
    app.send(1, Control::CloseTerminal).await;
    while !matches!(app.control().await, (1, Control::TerminalExited { .. })) {}
    drop(app);
    assert!(daemon.wait_exit().success());
}
