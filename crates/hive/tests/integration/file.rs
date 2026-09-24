use hive::file::{TEXT_LIMIT, version};
use hive_protocol::{Control, Role};

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
    std::os::unix::fs::symlink("/etc/hostname", repo.root.join("out")).unwrap();
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
