use std::time::Duration;

use hive_protocol::{Control, DiffBase, Role};

use crate::common::Conn;
use crate::worktree::Repo;

/// Longer than the debounce and a re-list: whatever a change triggers has been sent by then.
const SETTLE: Duration = Duration::from_millis(800);

async fn watch(conn: &mut Conn, path: &str, base: DiffBase) {
    let path = path.to_owned();
    conn.send(0, Control::WatchWorktree { path, base }).await;
}

/// The next control messages, which must be `files` for `path` and then its `changes`.
async fn files(conn: &mut Conn, path: &str) -> Vec<String> {
    let files = match conn.control().await {
        (
            0,
            Control::Files {
                path: got,
                files,
                truncated: false,
            },
        ) if got == path => files,
        other => panic!("{other:?}"),
    };
    changed(conn, path).await;
    files
}

/// The next control message, which must be `changes` for `path`: the changed paths.
async fn changed(conn: &mut Conn, path: &str) -> Vec<String> {
    match conn.control().await {
        (
            0,
            Control::Changes {
                path: got,
                files,
                error: None,
                ..
            },
        ) if got == path => files.into_iter().map(|f| f.path).collect(),
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_watched_worktree_sends_its_files_after_every_change() {
    let repo = Repo::new();
    assert!(repo.hive(&["create", "fix"]).status.success());
    repo.write(".gitignore", "target/\n.claude/\n");
    repo.write("target/debug/build.o", "");
    repo.write("src/a.rs", "");
    let root = repo.root.display().to_string();
    let fix = repo.root.join(".claude/worktrees/fix");
    let fix_path = fix.display().to_string();

    let mut daemon = repo.env.daemon();
    let mut conn = repo.env.connect(Role::App).await;
    // Only worktrees of followed projects.
    watch(&mut conn, &root, DiffBase::Head).await;
    let refused = format!("{root} is not a worktree of a followed project");
    assert_eq!(
        conn.control().await,
        (0, Control::Error { message: refused })
    );
    conn.send(0, Control::AddProject { path: root.clone() })
        .await;
    assert!(matches!(
        conn.control().await.1,
        Control::ProjectAdded { .. }
    ));
    // Nor any other folder of it.
    let src = format!("{root}/src");
    watch(&mut conn, &src, DiffBase::Head).await;
    let refused = format!("{src} is not a worktree of a followed project");
    assert_eq!(
        conn.control().await,
        (0, Control::Error { message: refused })
    );

    watch(&mut conn, &root, DiffBase::Head).await;
    let listed = [".gitignore", "README", "src/a.rs"];
    assert_eq!(files(&mut conn, &root).await, listed);
    // Ignored trees are not watched: a change in one sends nothing.
    repo.write("target/debug/other.o", "");
    let quiet = tokio::time::timeout(SETTLE, conn.control()).await;
    assert!(quiet.is_err(), "{quiet:?}");

    repo.write("b.txt", "");
    let listed = [".gitignore", "README", "b.txt", "src/a.rs"];
    assert_eq!(files(&mut conn, &root).await, listed);
    std::fs::remove_file(repo.root.join("b.txt")).unwrap();
    assert_eq!(
        files(&mut conn, &root).await,
        [".gitignore", "README", "src/a.rs"]
    );
    std::fs::rename(repo.root.join("src/a.rs"), repo.root.join("src/c.rs")).unwrap();
    assert_eq!(
        files(&mut conn, &root).await,
        [".gitignore", "README", "src/c.rs"]
    );
    // A change that lists the same files sends only the changes.
    repo.write("src/c.rs", "edited");
    let untracked = [".gitignore", "src/c.rs"];
    assert_eq!(changed(&mut conn, &root).await, untracked);
    repo.write("d.txt", "");
    let listed = [".gitignore", "README", "d.txt", "src/c.rs"];
    assert_eq!(files(&mut conn, &root).await, listed);
    // The index counts: an ignored file added by force is listed.
    repo.git(&["add", "-f", "target/debug/build.o"]);
    let listed = [
        ".gitignore",
        "README",
        "d.txt",
        "src/c.rs",
        "target/debug/build.o",
    ];
    assert_eq!(files(&mut conn, &root).await, listed);

    // Watching another worktree replaces the first.
    watch(&mut conn, &fix_path, DiffBase::Head).await;
    assert_eq!(files(&mut conn, &fix_path).await, ["README"]);
    repo.write("e.txt", "");
    tokio::time::sleep(SETTLE).await;
    std::fs::write(fix.join("f.txt"), "").unwrap();
    assert_eq!(files(&mut conn, &fix_path).await, ["README", "f.txt"]);

    // Unwatched, nothing more arrives.
    conn.send(0, Control::UnwatchWorktree).await;
    tokio::time::sleep(SETTLE).await;
    std::fs::write(fix.join("g.txt"), "").unwrap();
    tokio::time::sleep(SETTLE).await;
    conn.send(0, Control::ListProjects).await;
    assert!(matches!(conn.control().await.1, Control::Projects { .. }));

    // Its changes are against the base asked: the merge-base with main keeps a commit of its
    // branch in view.
    repo.git_in(&fix, &["add", "g.txt"]);
    repo.git_in(&fix, &["commit", "-q", "-m", "g"]);
    watch(&mut conn, &fix_path, DiffBase::Branch).await;
    match conn.control().await {
        (0, Control::Files { files, .. }) => assert_eq!(files, ["README", "f.txt", "g.txt"]),
        other => panic!("{other:?}"),
    }
    assert_eq!(changed(&mut conn, &fix_path).await, ["f.txt", "g.txt"]);
    // A worktree that disappears is an error.
    std::fs::remove_dir_all(&fix).unwrap();
    match conn.control().await {
        (0, Control::Error { message }) => {
            assert!(message.starts_with("git ls-files"), "{message}")
        }
        other => panic!("{other:?}"),
    }
    // The app leaving ends the service, watch included.
    drop(conn);
    assert!(daemon.wait_exit().success());
}
