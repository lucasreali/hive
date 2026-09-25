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
