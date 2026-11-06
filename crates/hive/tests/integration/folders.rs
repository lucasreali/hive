//! A plain folder followed as a folder project (23.1): its files panel, its own terminals and
//! agents, and `git init` making it a project.

use hive_protocol::{Control, DiffBase, Project, ProjectError, Role};
use serde_json::json;

use crate::agents::hook;
use crate::common::{Conn, Env};
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
    // A file written there sends its files again, the same list: the app reads it again.
    std::fs::write(dir.join("todo.md"), "- two\n").unwrap();
    let (0, Control::Files { path, files, .. }) = app.control().await else {
        panic!("expected files");
    };
    assert_eq!((path, files), (f.clone(), vec!["todo.md".to_owned()]));

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

#[tokio::test]
async fn a_new_project_whose_git_init_fails_says_so_and_its_folder_stays() {
    let env = Env::new();
    let home = env.path("home").canonicalize().unwrap();
    // A `git` that fails a plain `git init`, and runs the real one for anything else.
    let real = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|dir| dir.join("git"))
        .find(|git| git.is_file())
        .expect("git on PATH");
    let bin = env.path("bin");
    std::fs::create_dir(&bin).unwrap();
    let script = format!(
        "#!/bin/sh\ncase \"$*\" in *' init -q') echo nope >&2; exit 1;; esac\nexec '{}' \"$@\"\n",
        real.display()
    );
    std::fs::write(bin.join("git"), script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::Permissions::from_mode(0o755);
    std::fs::set_permissions(bin.join("git"), mode).unwrap();
    let path = std::env::join_paths(
        std::iter::once(bin).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let mut daemon = env.daemon_with(env.hive().env("PATH", path));
    let mut app = env.connect(Role::App).await;

    let create = Control::CreateProject {
        parent: home.display().to_string(),
        name: "shop".into(),
        git: true,
    };
    app.send(0, create).await;
    let (error, message) = refusal(&mut app).await;
    assert_eq!(error, ProjectError::CreateFailed);
    let shop = home.join("shop");
    let said = format!("{} was created, but git init failed: ", shop.display());
    assert!(message.starts_with(&said), "{message}");
    assert!(message.contains("nope"), "{message}");
    assert!(shop.is_dir());

    drop(app);
    assert!(daemon.wait_exit().success());
}

/// The next `add_project_failed`'s error and message, skipping the lists between.
async fn refusal(app: &mut Conn) -> (ProjectError, String) {
    loop {
        match app.control().await {
            (0, Control::AddProjectFailed { error, message, .. }) => break (error, message),
            (0, Control::Projects { .. }) => {}
            other => panic!("expected add_project_failed, got {other:?}"),
        }
    }
}

/// The next `project_created`, skipping the list before it (a folder project is a group).
async fn created(app: &mut Conn) -> (Project, String) {
    loop {
        match app.control().await {
            (0, Control::ProjectCreated { project, path }) => break (project, path),
            (0, Control::Projects { .. }) => {}
            other => panic!("expected project_created, got {other:?}"),
        }
    }
}

#[tokio::test]
async fn a_new_project_is_created_with_or_without_git_and_never_over_a_folder() {
    let env = Env::new();
    let home = env.path("home").canonicalize().unwrap();
    let parent = home.display().to_string();
    let mut daemon = env.daemon();
    let mut app = env.connect(Role::App).await;

    // "New folder" in the browser: made in the folder, once.
    let dir = Control::CreateDir {
        parent: parent.clone(),
        name: "work".into(),
    };
    app.send(0, dir.clone()).await;
    let made = Control::DirCreated {
        parent: parent.clone(),
        name: "work".into(),
    };
    assert_eq!(app.control().await, (0, made));
    assert!(home.join("work").is_dir());
    app.send(0, dir).await;
    let taken = Control::CreateDirFailed {
        parent,
        name: "work".into(),
        message: format!("{} already exists", home.join("work").display()),
    };
    assert_eq!(app.control().await, (0, taken));

    // With git: a repository, followed as a project; its terminal opens in it.
    let work = home.join("work").display().to_string();
    let create = |name: &str, git| Control::CreateProject {
        parent: work.clone(),
        name: name.into(),
        git,
    };
    app.send(0, create("shop", true)).await;
    let (project, path) = created(&mut app).await;
    let shop = home.join("work/shop");
    assert_eq!(path, shop.display().to_string());
    assert_eq!(
        (project.id.as_str(), project.folder),
        (path.as_str(), false)
    );
    assert!(shop.join(".git").is_dir());
    assert_eq!(app.open_terminal(1, &shop).await, Some(path.clone()));
    app.send(1, Control::CloseTerminal).await;
    while !matches!(app.control().await, (1, Control::TerminalExited { .. })) {}

    // Without git: a folder project.
    app.send(0, create("notes", false)).await;
    let (project, path) = created(&mut app).await;
    assert_eq!(path, home.join("work/notes").display().to_string());
    assert_eq!((project.id, project.folder), (path, true));
    assert!(!home.join("work/notes/.git").exists());

    // Refused: a taken name (nothing in it changes), a bad name, a missing parent, a folder
    // of Hive's.
    std::fs::write(shop.join("README"), "kept").unwrap();
    app.send(0, create("shop", false)).await;
    let (error, message) = refusal(&mut app).await;
    assert_eq!(error, ProjectError::NameTaken);
    assert_eq!(message, format!("{} already exists", shop.display()));
    assert_eq!(
        std::fs::read_to_string(shop.join("README")).unwrap(),
        "kept"
    );
    app.send(0, create("../up", true)).await;
    assert_eq!(refusal(&mut app).await.0, ProjectError::InvalidName);
    let missing = Control::CreateProject {
        parent: home.join("gone").display().to_string(),
        name: "x".into(),
        git: true,
    };
    app.send(0, missing).await;
    assert_eq!(refusal(&mut app).await.0, ProjectError::NotFound);
    assert!(!home.join("gone").exists());
    let data = env.path("data/hive");
    std::fs::create_dir_all(&data).unwrap();
    let hive = Control::CreateProject {
        parent: data.display().to_string(),
        name: "x".into(),
        git: false,
    };
    app.send(0, hive).await;
    assert_eq!(refusal(&mut app).await.0, ProjectError::ProtectedFolder);
    assert!(!data.join("x").exists());

    drop(app);
    assert!(daemon.wait_exit().success());
}
