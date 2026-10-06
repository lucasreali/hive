use std::time::Duration;

use hive_protocol::{Control, DiffBase, Role};

use crate::common::{Conn, stop};
use crate::worktree::Repo;

/// Longer than the debounce and a re-list: whatever a change triggers has been sent by then.
const SETTLE: Duration = Duration::from_millis(800);

async fn watch(conn: &mut Conn, path: &str, base: DiffBase) {
    let path = path.to_owned();
    conn.send(0, Control::WatchWorktree { path, base }).await;
}

/// The next control messages, which must be `files` for `path` and then its `changes`.
async fn files(conn: &mut Conn, path: &str) -> Vec<String> {
    listing(conn, path).await.0
}

/// The next control messages, which must be `files` for `path` and then its `changes`: its
/// files and ignored entries.
async fn listing(conn: &mut Conn, path: &str) -> (Vec<String>, Vec<String>) {
    let listing = match conn.control().await {
        (
            0,
            Control::Files {
                path: got,
                files,
                ignored,
                truncated: false,
            },
        ) if got == path => (files, ignored),
        other => panic!("{other:?}"),
    };
    changed(conn, path).await;
    listing
}

async fn expand(conn: &mut Conn, path: &str, folders: &[&str]) {
    let path = path.to_owned();
    let folders = folders.iter().map(|f| (*f).to_owned()).collect();
    conn.send(0, Control::ExpandIgnored { path, folders }).await;
}

/// Nothing arrives for a while.
async fn quiet(conn: &mut Conn) {
    let quiet = tokio::time::timeout(SETTLE, conn.control()).await;
    assert!(quiet.is_err(), "{quiet:?}");
}

#[tokio::test]
async fn ignored_entries_show_and_an_open_ignored_folder_is_listed_and_watched() {
    let repo = Repo::new();
    repo.write(".gitignore", ".env\nnode_modules/\n");
    repo.write(".env", "KEY=1\n");
    repo.write("node_modules/pkg/index.js", "");
    let root = repo.root.display().to_string();
    let daemon = repo.env.daemon();
    let mut conn = repo.env.connect(Role::App).await;
    conn.send(0, Control::AddProject { path: root.clone() })
        .await;
    assert!(matches!(
        conn.control().await.1,
        Control::ProjectAdded { .. }
    ));
    watch(&mut conn, &root, DiffBase::Head).await;
    let (files, ignored) = listing(&mut conn, &root).await;
    assert_eq!(files, [".gitignore", "README"]);
    assert_eq!(ignored, [".env", "node_modules/"]);
    // Closed, the folder is not watched.
    repo.write("node_modules/top.js", "");
    quiet(&mut conn).await;
    // Another worktree's folders change nothing.
    expand(&mut conn, "/elsewhere", &["node_modules"]).await;
    quiet(&mut conn).await;

    // Opened, one level of it is listed, and watched.
    expand(&mut conn, &root, &["node_modules"]).await;
    let level = [
        ".env",
        "node_modules/",
        "node_modules/pkg/",
        "node_modules/top.js",
    ];
    assert_eq!(listing(&mut conn, &root).await.1, level);
    repo.write("node_modules/new.js", "");
    let (_, ignored) = listing(&mut conn, &root).await;
    assert!(
        ignored.contains(&"node_modules/new.js".to_owned()),
        "{ignored:?}"
    );
    // Closed again, its watch goes.
    expand(&mut conn, &root, &[]).await;
    assert_eq!(listing(&mut conn, &root).await.1, [".env", "node_modules/"]);
    repo.write("node_modules/later.js", "");
    quiet(&mut conn).await;

    // An ignored file opens and saves as any other.
    conn.send(0, Control::UnwatchWorktree).await;
    let open = Control::OpenFile {
        worktree: root.clone(),
        path: ".env".into(),
        base: DiffBase::Head,
    };
    conn.send(0, open).await;
    let version = match conn.control().await.1 {
        Control::File {
            content, version, ..
        } => {
            assert_eq!(content.as_deref(), Some("KEY=1\n"));
            version
        }
        other => panic!("{other:?}"),
    };
    let save = Control::SaveFile {
        worktree: root.clone(),
        path: ".env".into(),
        content: "KEY=2\n".into(),
        version,
    };
    conn.send(0, save).await;
    assert!(matches!(conn.control().await.1, Control::FileSaved { .. }));
    let saved = std::fs::read_to_string(repo.root.join(".env")).unwrap();
    assert_eq!(saved, "KEY=2\n");
    drop(conn);
    stop(daemon);
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
