use hive::file::{TEXT_LIMIT, version};
use hive_protocol::{Control, Role, SaveError};

use crate::common::{Conn, stop};
use crate::worktree::Repo;

async fn open(conn: &mut Conn, worktree: &str, path: &str) -> Control {
    let (worktree, path) = (worktree.to_owned(), path.to_owned());
    conn.send(0, Control::OpenFile { worktree, path }).await;
    conn.control().await.1
}

async fn follow(conn: &mut Conn, path: &str) {
    let path = path.to_owned();
    conn.send(0, Control::AddProject { path }).await;
    assert!(matches!(
        conn.control().await.1,
        Control::ProjectAdded { .. }
    ));
}

/// `(content, base, too_large, error)` of a `file` answer, checking its version.
async fn sides(
    conn: &mut Conn,
    worktree: &str,
    path: &str,
) -> (Option<String>, Option<String>, bool, Option<String>) {
    match open(conn, worktree, path).await {
        Control::File {
            worktree: w,
            path: p,
            content,
            base,
            version: v,
            binary,
            too_large,
            error,
        } => {
            assert_eq!((w.as_str(), p.as_str(), binary), (worktree, path, false));
            let expected = content.as_deref().map(|c| version(c.as_bytes()));
            assert!(too_large || v == expected, "{path}: {v:?}");
            (content, base, too_large, error)
        }
        other => panic!("expected a file, got {other:?}"),
    }
}

fn text(s: &str) -> Option<String> {
    Some(s.to_owned())
}

#[tokio::test]
async fn a_file_is_read_on_disk_and_at_head() {
    let repo = Repo::new();
    repo.commit("a.txt", "1\n2\n");
    repo.commit("c.txt", "gone\n");
    repo.commit("old.txt", "r\n");
    let limit = TEXT_LIMIT as usize;
    repo.commit("big", &"x".repeat(limit));
    repo.commit("bigger", &"x".repeat(limit + 1));
    repo.write("a.txt", "1\nTWO\n");
    std::fs::remove_file(repo.root.join("c.txt")).unwrap();
    repo.git(&["mv", "old.txt", "m.txt"]);
    repo.write("n.txt", "n\n");
    repo.git(&["add", "n.txt"]);
    repo.write("u dir/u.txt", "u\n");
    std::os::unix::fs::symlink("/etc/hosts", repo.root.join("out")).unwrap();
    let root = repo.root.display().to_string();

    let daemon = repo.env.daemon();
    let mut conn = repo.env.connect(Role::App).await;
    let error = format!("{root} is not a worktree of a followed project");
    assert_eq!(
        sides(&mut conn, &root, "a.txt").await,
        (None, None, false, Some(error))
    );
    follow(&mut conn, &root).await;

    let changed = (text("1\nTWO\n"), text("1\n2\n"), false, None);
    assert_eq!(sides(&mut conn, &root, "a.txt").await, changed);
    let deleted = (None, text("gone\n"), false, None);
    assert_eq!(sides(&mut conn, &root, "c.txt").await, deleted);
    let renamed = (text("r\n"), text("r\n"), false, None);
    assert_eq!(sides(&mut conn, &root, "m.txt").await, renamed);
    // Listed after a rename, a new file still has no base.
    let added = (text("n\n"), None, false, None);
    assert_eq!(sides(&mut conn, &root, "n.txt").await, added);
    let untracked = (text("u\n"), None, false, None);
    assert_eq!(sides(&mut conn, &root, "u dir/u.txt").await, untracked);
    let at_limit = text(&"x".repeat(limit));
    let big = (at_limit.clone(), at_limit, false, None);
    assert_eq!(sides(&mut conn, &root, "big").await, big);
    assert_eq!(
        sides(&mut conn, &root, "bigger").await,
        (None, None, true, None)
    );
    let error = |e: &str| (None, None, false, Some(e.to_owned()));
    assert_eq!(
        sides(&mut conn, &root, "nope").await,
        error("nope does not exist")
    );
    assert_eq!(
        sides(&mut conn, &root, "../x").await,
        error("not a relative path inside the worktree")
    );
    assert_eq!(
        sides(&mut conn, &root, "out").await,
        error("the file resolves outside the worktree")
    );

    // Before the first commit nothing has a base.
    let fresh = repo.env.path("home/fresh");
    std::fs::create_dir(&fresh).unwrap();
    let fresh = fresh.canonicalize().unwrap();
    repo.git_in(&fresh, &["init", "-q", "-b", "main"]);
    std::fs::write(fresh.join("staged.txt"), "s\n").unwrap();
    repo.git_in(&fresh, &["add", "staged.txt"]);
    let fresh = fresh.display().to_string();
    follow(&mut conn, &fresh).await;
    let staged = (text("s\n"), None, false, None);
    assert_eq!(sides(&mut conn, &fresh, "staged.txt").await, staged);
    drop(conn);
    stop(daemon);
}

async fn save(conn: &mut Conn, worktree: &str, content: &str, version: Option<String>) -> Control {
    let save = Control::SaveFile {
        worktree: worktree.to_owned(),
        path: "a.txt".to_owned(),
        content: content.to_owned(),
        version,
    };
    conn.send(0, save).await;
    conn.control().await.1
}

#[tokio::test]
async fn a_save_writes_only_over_the_version_the_app_read() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n");
    // A file the system would run instead of opening in an editor.
    let (program, system) = if cfg!(target_os = "macos") {
        ("run.command", "macOS")
    } else {
        ("run.cmd", "Windows")
    };
    std::fs::write(repo.root.join(program), "").unwrap();
    let root = repo.root.display().to_string();
    let daemon = repo.env.daemon();
    let mut conn = repo.env.connect(Role::App).await;
    let refused = save(&mut conn, &root, "x", None).await;
    assert!(matches!(
        refused,
        Control::SaveFailed {
            error: SaveError::InvalidPath,
            ..
        }
    ));
    follow(&mut conn, &root).await;

    let two = version(b"two\n");
    assert_eq!(
        save(&mut conn, &root, "two\n", Some(version(b"one\n"))).await,
        Control::FileSaved {
            worktree: root.clone(),
            path: "a.txt".to_owned(),
            version: two.clone(),
        }
    );
    assert_eq!(sides(&mut conn, &root, "a.txt").await.0, text("two\n"));

    // An agent wrote meanwhile: the app's version is stale and nothing is written.
    repo.write("a.txt", "agent\n");
    assert_eq!(
        save(&mut conn, &root, "mine\n", Some(two)).await,
        Control::SaveFailed {
            worktree: root.clone(),
            path: "a.txt".to_owned(),
            error: SaveError::Conflict,
            message: "a.txt changed on disk".to_owned(),
        }
    );
    assert_eq!(sides(&mut conn, &root, "a.txt").await.0, text("agent\n"));

    // The system would run it, so it is not handed to the app.
    let open = Control::OpenInEditor {
        worktree: root.clone(),
        path: program.to_owned(),
    };
    conn.send(0, open).await;
    assert_eq!(
        conn.control().await.1,
        Control::EditorTarget {
            worktree: root.clone(),
            path: program.to_owned(),
            windows_path: None,
            error: Some(format!(
                "{system} would run a .{} file instead of opening it in an editor",
                program.rsplit('.').next().unwrap()
            )),
        }
    );
    drop(conn);
    stop(daemon);
}

#[tokio::test]
async fn files_are_created_and_renamed_inside_a_followed_worktree() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n");
    let root = repo.root.display().to_string();
    let daemon = repo.env.daemon();
    let mut conn = repo.env.connect(Role::App).await;
    let create = |name: &str| Control::CreateFile {
        worktree: root.clone(),
        folder: String::new(),
        name: name.to_owned(),
    };
    let rename = |path: &str, name: &str| Control::RenameFile {
        worktree: root.clone(),
        path: path.to_owned(),
        name: name.to_owned(),
    };
    let failed = |message: &str| Control::FileOpFailed {
        worktree: root.clone(),
        message: message.to_owned(),
    };
    // Not a followed worktree yet.
    conn.send(0, create("b.txt")).await;
    assert!(matches!(
        conn.control().await.1,
        Control::FileOpFailed { .. }
    ));
    conn.send(0, rename("a.txt", "c.txt")).await;
    assert!(matches!(
        conn.control().await.1,
        Control::FileOpFailed { .. }
    ));
    follow(&mut conn, &root).await;

    conn.send(0, create("b.txt")).await;
    let created = Control::FileCreated {
        worktree: root.clone(),
        path: "b.txt".to_owned(),
    };
    assert_eq!(conn.control().await.1, created);
    assert_eq!(std::fs::read(repo.root.join("b.txt")).unwrap(), b"");
    conn.send(0, create("a.txt")).await;
    assert_eq!(conn.control().await.1, failed("a.txt already exists"));

    conn.send(0, rename("a.txt", "c.txt")).await;
    let renamed = Control::FileRenamed {
        worktree: root.clone(),
        path: "a.txt".to_owned(),
        to: "c.txt".to_owned(),
    };
    assert_eq!(conn.control().await.1, renamed);
    assert_eq!(sides(&mut conn, &root, "c.txt").await.0, text("one\n"));
    conn.send(0, rename("c.txt", "b.txt")).await;
    assert_eq!(conn.control().await.1, failed("b.txt already exists"));
    drop(conn);
    stop(daemon);
}
