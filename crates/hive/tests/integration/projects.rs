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
