use std::os::unix::fs::PermissionsExt;

use hive_protocol::{Control, Project, ProjectError, Role};

use crate::common::{Conn, Env};
use crate::worktree::Repo;

async fn add(conn: &mut Conn, path: &str) -> Control {
    let path = path.to_owned();
    conn.send(0, Control::AddProject { path }).await;
    conn.control().await.1
}

async fn added(conn: &mut Conn, path: &str) -> Project {
    match add(conn, path).await {
        Control::ProjectAdded { project } => project,
        other => panic!("{other:?}"),
    }
}

async fn list(conn: &mut Conn) -> Vec<Project> {
    conn.send(0, Control::ListProjects).await;
    match conn.control().await {
        (0, Control::Projects { projects }) => projects,
        other => panic!("{other:?}"),
    }
}

async fn refused(conn: &mut Conn, path: &str) -> (ProjectError, String) {
    match add(conn, path).await {
        Control::AddProjectFailed {
            path: got,
            error,
            message,
        } => {
            assert_eq!(got, path);
            (error, message)
        }
        other => panic!("{other:?}"),
    }
}

fn summary(project: &Project) -> Vec<(String, bool, bool, Option<String>)> {
    project
        .worktrees
        .iter()
        .map(|w| (w.name.clone(), w.main, w.claude, w.branch.clone()))
        .collect()
}

#[tokio::test]
async fn a_project_lists_its_worktrees_and_survives_a_restart() {
    let repo = Repo::new();
    assert!(repo.hive(&["create", "fix-a"]).status.success());
    let external = repo.env.path("home/external");
    repo.git(&[
        "worktree",
        "add",
        "-q",
        "-b",
        "ext",
        external.to_str().unwrap(),
    ]);
    std::fs::create_dir(repo.root.join("sub")).unwrap();
    let root = repo.root.display().to_string();

    let mut daemon = repo.env.daemon();
    let mut conn = repo.env.connect(Role::App).await;
    assert_eq!(list(&mut conn).await, vec![]);
    // A folder inside the repository adds the repository.
    let project = added(&mut conn, &format!("{root}/sub")).await;
    assert_eq!(
        (project.id.as_str(), project.path.as_str()),
        (&*root, &*root)
    );
    assert_eq!(project.name, "repo");
    assert_eq!(project.error, None);
    let mut worktrees = summary(&project);
    worktrees[1..].sort();
    let expected = [
        ("main", true, false, Some("main")),
        ("ext", false, false, Some("ext")),
        ("fix-a", false, true, Some("worktree-fix-a")),
    ]
    .map(|(n, m, c, b)| (n.to_owned(), m, c, b.map(str::to_owned)));
    assert_eq!(worktrees, expected);
    let fix = project
        .worktrees
        .iter()
        .find(|w| w.name == "fix-a")
        .unwrap();
    assert_eq!(fix.path, format!("{root}/.claude/worktrees/fix-a"));
    assert_eq!(fix.id, fix.path);

    // Adding it again, even from a linked worktree, changes nothing.
    let again = added(&mut conn, &format!("{root}/.claude/worktrees/fix-a")).await;
    assert_eq!(again, project);
    assert_eq!(list(&mut conn).await, vec![project.clone()]);

    let file = repo.env.path("data/hive/projects.json");
    let mode = std::fs::metadata(&file).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    let saved: Vec<String> = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(saved, [root]);

    drop(conn);
    assert!(daemon.wait_exit().success());
    let mut daemon = repo.env.daemon();
    let mut conn = repo.env.connect(Role::App).await;
    assert_eq!(list(&mut conn).await, vec![project]);
    drop(conn);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn folders_that_are_not_projects_are_refused() {
    let env = Env::new();
    let file = env.path("home/file");
    std::fs::write(&file, "").unwrap();
    let mut daemon = env.daemon();
    let mut conn = env.connect(Role::App).await;
    let home = env.path("home").display().to_string();
    let cases = [
        ("  ".to_owned(), ProjectError::EmptyPath),
        ("home".to_owned(), ProjectError::NotAbsolute),
        (format!("{home}/missing"), ProjectError::NotFound),
        (file.display().to_string(), ProjectError::NotADirectory),
        (home.clone(), ProjectError::NotAGitRepository),
    ];
    for (path, error) in cases {
        assert_eq!(refused(&mut conn, &path).await.0, error, "{path}");
    }
    let (_, message) = refused(&mut conn, &home).await;
    assert!(
        message.starts_with(&format!("{home} is not in a git repository")),
        "{message}"
    );
    assert_eq!(list(&mut conn).await, vec![]);
    drop(conn);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn a_list_that_cannot_be_saved_is_an_error() {
    let repo = Repo::new();
    let mut daemon = repo.env.daemon();
    let mut conn = repo.env.connect(Role::App).await;
    let data = repo.env.path("data/hive");
    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o500)).unwrap();
    let (error, message) = refused(&mut conn, &repo.root.display().to_string()).await;
    std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(error, ProjectError::Storage);
    assert!(message.starts_with("cannot save "), "{message}");
    assert_eq!(list(&mut conn).await, vec![]);
    drop(conn);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn a_moved_project_reports_why_it_has_no_worktrees() {
    let repo = Repo::new();
    let mut daemon = repo.env.daemon();
    let mut conn = repo.env.connect(Role::App).await;
    let project = added(&mut conn, &repo.root.display().to_string()).await;
    std::fs::rename(&repo.root, repo.env.path("home/moved")).unwrap();
    let [moved] = &list(&mut conn).await[..] else {
        panic!("one project expected")
    };
    assert_eq!(moved.id, project.id);
    assert!(moved.worktrees.is_empty());
    assert!(moved.error.as_deref().unwrap().contains("cannot change to"));
    drop(conn);
    assert!(daemon.wait_exit().success());
}

async fn request(conn: &mut Conn, message: Control) -> Control {
    conn.send(0, message).await;
    conn.control().await.1
}

#[tokio::test]
async fn the_dialog_lists_branches_validates_names_and_creates_worktrees() {
    let repo = Repo::new();
    let origin = repo.env.path("home/origin.git");
    repo.git(&["init", "-q", "--bare", origin.to_str().unwrap()]);
    repo.git(&["remote", "add", "origin", origin.to_str().unwrap()]);
    repo.git(&["push", "-q", "origin", "main"]);
    repo.git(&["remote", "set-head", "origin", "main"]);
    repo.git(&["branch", "develop"]);
    repo.commit(".gitignore", ".env\n");
    repo.commit(".worktreeinclude", ".env\n");
    repo.write(".env", "SECRET=1");
    repo.write(
        ".claude/settings.local.json",
        r#"{"hooks":{"WorktreeCreate":[{"hooks":[{"type":"command","command":"x"}]}]}}"#,
    );
    let root = repo.root.display().to_string();
    let mut daemon = repo.env.daemon();
    let mut conn = repo.env.connect(Role::App).await;

    // Requests for a project that is not followed are refused.
    let refused = format!("{root} is not a followed project");
    let branches = request(
        &mut conn,
        Control::ListBranches {
            project: root.clone(),
        },
    )
    .await;
    assert_eq!(
        branches,
        Control::Branches {
            project: root.clone(),
            local: vec![],
            remote: vec![],
            current: None,
            error: Some(refused.clone()),
        }
    );
    added(&mut conn, &root).await;

    let branches = request(
        &mut conn,
        Control::ListBranches {
            project: root.clone(),
        },
    )
    .await;
    assert_eq!(
        branches,
        Control::Branches {
            project: root.clone(),
            local: vec!["develop".into(), "main".into()],
            remote: vec!["origin/main".into()],
            current: Some("main".into()),
            error: None,
        }
    );

    let validate = |name: &str| Control::ValidateWorktreeName {
        project: root.clone(),
        name: name.into(),
    };
    let checked = request(&mut conn, validate("fix-a")).await;
    assert_eq!(
        checked,
        Control::WorktreeNameValidated {
            project: root.clone(),
            name: "fix-a".into(),
            folder: ".claude/worktrees/fix-a/".into(),
            branch: "worktree-fix-a".into(),
            error: None,
        }
    );
    let Control::WorktreeNameValidated { error, .. } = request(&mut conn, validate("Fix")).await
    else {
        panic!("expected a validation")
    };
    assert!(error.unwrap().starts_with("invalid worktree name \"Fix\""));

    let create = |name: &str, base: Option<&str>| Control::CreateWorktree {
        project: root.clone(),
        name: name.into(),
        base: base.map(Into::into),
    };
    let Control::WorktreeCreated {
        project,
        path,
        notes,
    } = request(&mut conn, create("fix-a", Some("origin/main"))).await
    else {
        panic!("expected a new worktree")
    };
    assert_eq!(path, format!("{root}/.claude/worktrees/fix-a"));
    assert!(project.worktrees.iter().any(|w| w.path == path && w.claude));
    assert_eq!(
        notes,
        [
            "warning: .claude/settings.local.json defines its own WorktreeCreate hook; it will compete with Hive's",
            "copied 1 file listed in .worktreeinclude",
        ]
    );
    assert_eq!(
        repo.git(&["rev-parse", "worktree-fix-a"]),
        repo.git(&["rev-parse", "origin/main"])
    );

    // The existing folder is now refused by name, as the CLI does.
    let Control::WorktreeNameValidated { error, .. } = request(&mut conn, validate("fix-a")).await
    else {
        panic!("expected a validation")
    };
    assert!(
        error
            .unwrap()
            .starts_with("worktree \"fix-a\" already exists at ")
    );
    let failed = request(&mut conn, create("fix-a", None)).await;
    let Control::CreateWorktreeFailed {
        project,
        name,
        message,
    } = failed
    else {
        panic!("expected a failure: {failed:?}")
    };
    assert_eq!((project.as_str(), name.as_str()), (&*root, "fix-a"));
    assert!(message.starts_with("worktree \"fix-a\" already exists"));
    drop(conn);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn the_menu_removes_and_renames_worktrees() {
    let repo = Repo::new();
    let root = repo.root.display().to_string();
    let wt = |name: &str| format!("{root}/.claude/worktrees/{name}");
    let mut daemon = repo.env.daemon();
    let mut conn = repo.env.connect(Role::App).await;
    added(&mut conn, &root).await;
    for name in ["a", "b", "c"] {
        let create = Control::CreateWorktree {
            project: root.clone(),
            name: name.into(),
            base: None,
        };
        let created = request(&mut conn, create).await;
        assert!(
            matches!(created, Control::WorktreeCreated { .. }),
            "{created:?}"
        );
    }
    let outside = repo.env.path("home/outside");
    repo.git(&[
        "worktree",
        "add",
        "-q",
        "-b",
        "out",
        outside.to_str().unwrap(),
    ]);
    let outside = outside.display().to_string();

    let remove = |path: &str, force: bool| Control::RemoveWorktree {
        path: path.into(),
        force,
    };
    let rename = |path: &str, name: &str| Control::RenameWorktree {
        path: path.into(),
        name: name.into(),
    };
    let remove_failure = |answer: Control| match answer {
        Control::RemoveWorktreeFailed { message, .. } => message,
        other => panic!("expected a failure: {other:?}"),
    };
    let rename_failure = |answer: Control| match answer {
        Control::RenameWorktreeFailed { message, .. } => message,
        other => panic!("expected a failure: {other:?}"),
    };

    // Only linked worktrees of followed projects.
    let main = remove_failure(request(&mut conn, remove(&root, true)).await);
    assert_eq!(main, format!("{root} is the project's main worktree"));
    let stray = rename_failure(request(&mut conn, rename("/nope", "x")).await);
    assert_eq!(stray, "/nope is not a worktree of a followed project");
    let foreign = rename_failure(request(&mut conn, rename(&outside, "x")).await);
    assert_eq!(
        foreign,
        "only worktrees under .claude/worktrees can be renamed"
    );

    // Changes keep a worktree unless forced; its branch stays.
    std::fs::write(format!("{}/new.txt", wt("a")), "x").unwrap();
    let dirty = remove_failure(request(&mut conn, remove(&wt("a"), false)).await);
    assert!(dirty.contains("use --force"), "{dirty}");
    let Control::WorktreeRemoved { project, path } =
        request(&mut conn, remove(&wt("a"), true)).await
    else {
        panic!("expected a removal")
    };
    assert_eq!(path, wt("a"));
    assert!(project.worktrees.iter().all(|w| w.path != path));
    assert_eq!(repo.git(&["branch", "--list", "worktree-a"]), "worktree-a");

    // A terminal working in a worktree keeps it from being renamed or removed.
    conn.open_terminal(1, std::path::Path::new(&wt("b"))).await;
    let busy = rename_failure(request(&mut conn, rename(&wt("b"), "d")).await);
    assert!(busy.starts_with("in use by "), "{busy}");
    assert!(busy.ends_with(": close its terminals first"), "{busy}");
    let busy = remove_failure(request(&mut conn, remove(&wt("b"), false)).await);
    assert!(busy.starts_with("in use by "), "{busy}");
    conn.send(1, Control::CloseTerminal).await;
    assert!(matches!(
        conn.control().await,
        (1, Control::TerminalExited { .. })
    ));

    // Folder and branch follow the new name.
    let Control::WorktreeRenamed {
        project,
        from,
        path,
    } = request(&mut conn, rename(&wt("b"), "d")).await
    else {
        panic!("expected a rename")
    };
    assert_eq!((from, path.clone()), (wt("b"), wt("d")));
    let renamed = project.worktrees.iter().find(|w| w.path == path).unwrap();
    assert_eq!(
        (renamed.name.as_str(), renamed.branch.as_deref()),
        ("d", Some("worktree-d"))
    );
    assert_eq!(repo.git(&["branch", "--list", "worktree-b"]), "");

    // A taken name is refused; a taken branch moves the folder back.
    let taken = rename_failure(request(&mut conn, rename(&wt("d"), "c")).await);
    assert!(
        taken.starts_with("worktree \"c\" already exists"),
        "{taken}"
    );
    repo.git(&["branch", "worktree-e"]);
    let taken = rename_failure(request(&mut conn, rename(&wt("d"), "e")).await);
    assert!(taken.contains("worktree-e"), "{taken}");
    assert!(std::path::Path::new(&wt("d")).is_dir());
    assert!(!std::path::Path::new(&wt("e")).exists());
    assert_eq!(
        repo.git_in(
            std::path::Path::new(&wt("d")),
            &["branch", "--show-current"]
        ),
        "worktree-d"
    );

    // A worktree on another branch keeps that branch.
    repo.git_in(
        std::path::Path::new(&wt("c")),
        &["switch", "-q", "-c", "other"],
    );
    let renamed = request(&mut conn, rename(&wt("c"), "f")).await;
    assert!(
        matches!(renamed, Control::WorktreeRenamed { .. }),
        "{renamed:?}"
    );
    assert_eq!(
        repo.git_in(
            std::path::Path::new(&wt("f")),
            &["branch", "--show-current"]
        ),
        "other"
    );
    assert_eq!(repo.git(&["branch", "--list", "worktree-c"]), "worktree-c");
    drop(conn);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn contents_are_searched_in_tracked_and_untracked_files() {
    let repo = Repo::new();
    repo.commit(".gitignore", "ignored.txt\n");
    repo.commit("src/a.ts", "const Needle = 1;\nno\nneedle again\n");
    repo.write("new.md", "a NEEDLE here\n");
    repo.write("ignored.txt", "needle\n");
    repo.write("bin.dat", "needle\0binary");
    let root = repo.root.display().to_string();
    let mut daemon = repo.env.daemon();
    let mut conn = repo.env.connect(Role::App).await;
    let search = |worktree: &str, query: &str| Control::SearchFiles {
        worktree: worktree.into(),
        query: query.into(),
    };

    // Only followed worktrees.
    let Control::SearchResults { error, .. } = request(&mut conn, search(&root, "needle")).await
    else {
        panic!("expected results")
    };
    assert_eq!(
        error,
        Some(format!("{root} is not a worktree of a followed project"))
    );
    added(&mut conn, &root).await;

    let found = request(&mut conn, search(&root, "needle")).await;
    let Control::SearchResults {
        worktree,
        query,
        matches,
        truncated,
        error,
    } = found
    else {
        panic!("expected results: {found:?}")
    };
    assert_eq!(
        (worktree, query, truncated, error),
        (root.clone(), "needle".into(), false, None)
    );
    let lines: Vec<(String, u64, String)> = matches
        .into_iter()
        .map(|m| (m.path, m.line, m.text))
        .collect();
    assert_eq!(
        lines,
        [
            ("new.md".into(), 1, "a NEEDLE here".into()),
            ("src/a.ts".into(), 1, "const Needle = 1;".into()),
            ("src/a.ts".into(), 3, "needle again".into()),
        ]
    );
    let Control::SearchResults { matches, error, .. } =
        request(&mut conn, search(&root, "absent")).await
    else {
        panic!("expected results")
    };
    assert_eq!((matches, error), (vec![], None));
    drop(conn);
    assert!(daemon.wait_exit().success());
}
