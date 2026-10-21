use hive_protocol::{Control, ProjectScripts, ProjectSettings, Role, Settings};

use crate::common::Conn;
use crate::worktree::Repo;

async fn request(conn: &mut Conn, message: Control) -> Control {
    conn.send(0, message).await;
    conn.control().await.1
}

async fn follow(conn: &mut Conn, root: &str) {
    let add = Control::AddProject { path: root.into() };
    let added = request(conn, add).await;
    assert!(matches!(added, Control::ProjectAdded { .. }), "{added:?}");
}

#[tokio::test]
async fn terminals_in_a_worktree_get_its_paths_and_a_stable_block_of_ports() {
    let repo = Repo::new();
    assert!(repo.hive(&["create", "a"]).status.success());
    assert!(repo.hive(&["create", "b"]).status.success());
    let root = repo.root.display().to_string();
    let wt = |name: &str| repo.root.join(".claude/worktrees").join(name);
    let mut daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;
    follow(&mut app, &root).await;
    let show = "echo \"port=$HIVE_PORT wt=$HIVE_WORKTREE_PATH root=$HIVE_ROOT_PATH.\"\r";

    app.open_terminal(1, &wt("a")).await;
    app.input(1, show).await;
    let a = wt("a").display().to_string();
    app.output_until(1, &format!("port=20000 wt={a} root={root}."))
        .await;
    // The main worktree gets the next block; a second terminal in `a` the same one.
    app.open_terminal(2, &repo.root).await;
    app.input(2, show).await;
    app.output_until(2, &format!("port=20010 wt={root} root={root}."))
        .await;
    app.open_terminal(3, &wt("a")).await;
    app.input(3, show).await;
    app.output_until(3, "port=20000 wt=").await;
    // Outside every followed worktree: nothing.
    app.open_terminal(4, &repo.env.path("home")).await;
    app.input(4, show).await;
    app.output_until(4, "port= wt= root=.").await;
    // No block can be saved: the paths without a port.
    let ports = repo.env.path("data/hive/ports.json");
    std::fs::remove_file(&ports).unwrap();
    std::fs::create_dir(&ports).unwrap();
    app.open_terminal(5, &wt("b")).await;
    app.input(5, show).await;
    let b = wt("b").display().to_string();
    app.output_until(5, &format!("port= wt={b} root={root}."))
        .await;
    drop(app);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn the_archive_script_runs_before_a_removal_and_can_cancel_it() {
    let repo = Repo::new();
    assert!(repo.hive(&["create", "a"]).status.success());
    assert!(repo.hive(&["create", "b"]).status.success());
    let root = repo.root.display().to_string();
    let wt = |name: &str| format!("{root}/.claude/worktrees/{name}");
    let mut daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;
    follow(&mut app, &root).await;

    let log = repo.env.path("home/archive.log");
    let archive = format!(
        "echo \"$HIVE_WORKTREE_PATH $HIVE_ROOT_PATH $HIVE_PORT\" >> {}\n\
         if [ -e stop ]; then echo refused; exit 4; fi",
        log.display()
    );
    let mut settings = Settings::default();
    let scripts = ProjectScripts {
        archive: Some(archive),
        ..Default::default()
    };
    settings
        .projects
        .insert(root.clone(), ProjectSettings { scripts });
    let set = Control::SetSettings {
        settings: settings.clone(),
    };
    assert_eq!(request(&mut app, set).await, Control::Settings { settings });

    let remove = |path: String, force: bool| Control::RemoveWorktree { path, force };
    // A failed script keeps the worktree, with its output.
    std::fs::write(format!("{}/stop", wt("a")), "").unwrap();
    let failed = Control::RemoveWorktreeFailed {
        path: wt("a"),
        message: "the archive script failed (exit status: 4):\nrefused".into(),
    };
    assert_eq!(request(&mut app, remove(wt("a"), false)).await, failed);
    assert!(std::path::Path::new(&wt("a")).exists());
    // Forced, its failure does not matter.
    let removed = request(&mut app, remove(wt("a"), true)).await;
    assert!(
        matches!(removed, Control::WorktreeRemoved { .. }),
        "{removed:?}"
    );
    let removed = request(&mut app, remove(wt("b"), false)).await;
    assert!(
        matches!(removed, Control::WorktreeRemoved { .. }),
        "{removed:?}"
    );
    // `a` is gone, so `b` got its block back.
    assert_eq!(
        std::fs::read_to_string(&log).unwrap(),
        format!(
            "{a} {root} 20000\n{a} {root} 20000\n{b} {root} 20000\n",
            a = wt("a"),
            b = wt("b")
        )
    );
    drop(app);
    assert!(daemon.wait_exit().success());
}

/// Writes an executable `sh` script `name` in `dir`.
fn script(dir: &std::path::Path, name: &str, body: &str) {
    use std::os::unix::fs::PermissionsExt;
    let file = dir.join(name);
    std::fs::write(&file, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[tokio::test]
async fn git_and_scripts_get_the_users_login_path_once_it_is_known() {
    let repo = Repo::new();
    assert!(repo.hive(&["create", "a"]).status.success());
    assert!(repo.hive(&["create", "b"]).status.success());
    let root = repo.root.display().to_string();
    let wt = |name: &str| format!("{root}/.claude/worktrees/{name}");
    // Only the login shell's `PATH` has a `git` that notes each run (then runs the real one)
    // and a tool the archive script calls. The shell is the test's own (`fish` on WSL,
    // `$SHELL` on macOS): the human's config never loads.
    let real_git = std::env::split_paths(&std::env::var_os("PATH").unwrap())
        .map(|dir| dir.join("git"))
        .find(|git| git.is_file())
        .unwrap();
    let (tools, login) = (repo.env.path("tools"), repo.env.path("login"));
    std::fs::create_dir_all(&tools).unwrap();
    std::fs::create_dir_all(&login).unwrap();
    let runs = repo.env.path("home/git.log");
    let noted = format!(
        "echo \"$*\" >> '{}'\nexec '{}' \"$@\"",
        runs.display(),
        real_git.display()
    );
    script(&login, "git", &noted);
    script(&login, "hive-login-tool", "echo login tool ran");
    let git_dir = real_git.parent().unwrap().to_owned();
    let service = [tools.clone(), git_dir, "/usr/bin".into(), "/bin".into()];
    let service = std::env::join_paths(service).unwrap();
    let print = format!("printf '%s\\n' '{}:'\"$PATH\"", login.display());
    script(&tools, "fish", &print);
    let shell = tools.join("fish");
    let hive = &mut repo.env.hive();
    let mut daemon = repo
        .env
        .daemon_with(hive.env("PATH", &service).env("SHELL", &shell));
    let mut app = repo.env.connect(Role::App).await;
    follow(&mut app, &root).await;

    let log = repo.env.path("home/archive.log");
    let archive = format!(
        "hive-login-tool >> '{}' || echo missing >> '{0}'",
        log.display()
    );
    let mut settings = Settings::default();
    let scripts = ProjectScripts {
        archive: Some(archive),
        ..Default::default()
    };
    settings
        .projects
        .insert(root.clone(), ProjectSettings { scripts });
    let set = Control::SetSettings {
        settings: settings.clone(),
    };
    assert_eq!(request(&mut app, set).await, Control::Settings { settings });
    let remove = |path: String| Control::RemoveWorktree { path, force: false };
    let removed = |reply: &Control| matches!(reply, Control::WorktreeRemoved { .. });
    let read = |file: &std::path::Path| std::fs::read_to_string(file).unwrap_or_default();

    // Before the user's `PATH` is known: the service's own, as always.
    let reply = request(&mut app, remove(wt("a"))).await;
    assert!(removed(&reply), "{reply:?}");
    assert_eq!(read(&log), "missing\n");
    assert_eq!(read(&runs), "");
    // Asked for by diagnostics; from then on git and the scripts are the user's.
    app.send(0, Control::GetDiagnostics).await;
    let (_, diagnostics) = app.control().await;
    assert!(
        matches!(diagnostics, Control::Diagnostics { .. }),
        "{diagnostics:?}"
    );
    let reply = request(&mut app, remove(wt("b"))).await;
    assert!(removed(&reply), "{reply:?}");
    assert_eq!(read(&log), "missing\nlogin tool ran\n");
    let git_runs = read(&runs);
    assert!(git_runs.contains("worktree remove"), "{git_runs}");
    drop(app);
    assert!(daemon.wait_exit().success());
}
