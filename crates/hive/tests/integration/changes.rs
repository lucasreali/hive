use std::os::unix::ffi::OsStrExt;

use hive_protocol::{ChangedFile, Control, DiffBase, FileStatus, Role, SeenMark};

use crate::common::{Conn, stop};
use crate::worktree::Repo;

async fn changes(conn: &mut Conn, path: &str) -> Control {
    let path = path.to_owned();
    let base = DiffBase::Branch;
    conn.send(0, Control::ListChanges { path, base }).await;
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

fn file(path: &str, status: FileStatus, added: Option<u64>, removed: Option<u64>) -> ChangedFile {
    ChangedFile {
        path: path.into(),
        status,
        old_path: None,
        added,
        removed,
        seen: None,
    }
}

#[tokio::test]
async fn a_worktree_lists_what_differs_from_head() {
    let repo = Repo::new();
    repo.commit(".gitignore", "*.log\n");
    repo.commit("a.txt", "1\n2\n3\n");
    repo.commit("c.txt", "gone\n");
    repo.commit("old name.txt", "r\n");
    repo.commit("img.bin", "\0\x01");
    repo.write("a.txt", "1\nTWO\n3\n4\n");
    repo.write("n.txt", "n\n");
    repo.git(&["add", "n.txt"]);
    std::fs::remove_file(repo.root.join("c.txt")).unwrap();
    repo.git(&["mv", "old name.txt", "new name.txt"]);
    repo.write("img.bin", "\0\x02");
    repo.write("u dir/u.txt", "u1\nu2");
    repo.write("x.log", "ignored\n");
    // A name that is not UTF-8 (macOS refuses to create one).
    let not_utf8 = cfg!(target_os = "linux");
    if not_utf8 {
        let odd = std::ffi::OsStr::from_bytes(b"odd\xff");
        std::fs::write(repo.root.join(odd), "z\n").unwrap();
    }
    let root = repo.root.display().to_string();

    let daemon = repo.env.daemon();
    let mut conn = repo.env.connect(Role::App).await;
    // Only a worktree of a followed project.
    let refused = changes(&mut conn, &root).await;
    let error = format!("{root} is not a worktree of a followed project");
    assert_eq!(
        refused,
        Control::Changes {
            base: DiffBase::Head,
            branch: None,
            base_error: None,
            path: root.clone(),
            files: vec![],
            added: 0,
            removed: 0,
            error: Some(error),
            turns: vec![],
        }
    );
    follow(&mut conn, &root).await;

    // The main worktree is compared with HEAD, whatever the base asked.
    let no_branch =
        Some("No branch to compare with: this is the main worktree, or it is detached".to_owned());
    let mut renamed = file("new name.txt", FileStatus::Renamed, Some(0), Some(0));
    renamed.old_path = Some("old name.txt".into());
    assert_eq!(
        changes(&mut conn, &root).await,
        Control::Changes {
            base: DiffBase::Head,
            branch: None,
            base_error: no_branch.clone(),
            path: root.clone(),
            files: vec![
                file("a.txt", FileStatus::Modified, Some(2), Some(1)),
                file("c.txt", FileStatus::Deleted, Some(0), Some(1)),
                file("img.bin", FileStatus::Modified, None, None),
                file("n.txt", FileStatus::Added, Some(1), Some(0)),
                renamed,
                file("odd\u{fffd}", FileStatus::Untracked, Some(1), Some(0)),
                file("u dir/u.txt", FileStatus::Untracked, Some(2), Some(0)),
            ]
            .into_iter()
            .filter(|f| not_utf8 || f.path != "odd\u{fffd}")
            .collect(),
            added: 5 + u64::from(not_utf8),
            removed: 2,
            error: None,
            turns: vec![],
        }
    );

    // Before the first commit everything is new.
    let fresh = repo.env.path("home/fresh");
    std::fs::create_dir(&fresh).unwrap();
    let fresh = fresh.canonicalize().unwrap();
    repo.git_in(&fresh, &["init", "-q", "-b", "main"]);
    std::fs::write(fresh.join("staged.txt"), "s\n").unwrap();
    repo.git_in(&fresh, &["add", "staged.txt"]);
    std::fs::write(fresh.join("loose.txt"), "l\n").unwrap();
    let fresh = fresh.display().to_string();
    follow(&mut conn, &fresh).await;
    assert_eq!(
        changes(&mut conn, &fresh).await,
        Control::Changes {
            base: DiffBase::Head,
            branch: None,
            base_error: no_branch,
            path: fresh.clone(),
            files: vec![
                file("loose.txt", FileStatus::Untracked, Some(1), Some(0)),
                file("staged.txt", FileStatus::Added, Some(1), Some(0)),
            ],
            added: 2,
            removed: 0,
            error: None,
            turns: vec![],
        }
    );

    // A worktree that vanished answers why.
    std::fs::remove_dir_all(repo.root.join(".git")).unwrap();
    let Control::Changes { files, error, .. } = changes(&mut conn, &root).await else {
        panic!("expected changes")
    };
    assert_eq!(files, vec![]);
    assert!(error.is_some());
    drop(conn);
    stop(daemon);
}

/// Each changed file's path and seen mark (27.1).
fn marks(reply: Control) -> Vec<(String, Option<SeenMark>)> {
    let Control::Changes { files, base, .. } = reply else {
        panic!("expected changes: {reply:?}")
    };
    assert_eq!(base, DiffBase::Branch);
    files.into_iter().map(|f| (f.path, f.seen)).collect()
}

#[tokio::test]
async fn seen_marks_are_kept_invalidated_and_dropped_with_the_worktree() {
    let repo = Repo::new();
    assert!(repo.hive(&["create", "a"]).status.success());
    let root = repo.root.display().to_string();
    let wt = format!("{root}/.claude/worktrees/a");
    std::fs::write(format!("{wt}/README"), "changed").unwrap();
    std::fs::write(format!("{wt}/new.txt"), "n\n").unwrap();
    let mut daemon = repo.env.daemon();
    let mut conn = repo.env.connect(Role::App).await;
    follow(&mut conn, &root).await;

    let mark = Control::MarkSeen {
        path: wt.clone(),
        base: DiffBase::Branch,
        files: vec!["README".into(), "nope".into()],
        seen: true,
    };
    conn.send(0, mark).await;
    let seen = Some(SeenMark::Seen);
    let expected = vec![("README".to_owned(), seen), ("new.txt".to_owned(), None)];
    assert_eq!(marks(conn.control().await.1), expected);
    // In Hive's folder, kept across a restart.
    let file = repo.env.path("data/hive/seen.json");
    assert!(std::fs::read_to_string(&file).unwrap().contains(&wt));
    drop(conn);
    stop(daemon);
    daemon = repo.env.daemon();
    let mut conn = repo.env.connect(Role::App).await;
    assert_eq!(marks(changes(&mut conn, &wt).await), expected);

    // A new change to it: changed since seen.
    std::fs::write(format!("{wt}/README"), "changed again").unwrap();
    let changed = vec![
        ("README".to_owned(), Some(SeenMark::Changed)),
        ("new.txt".to_owned(), None),
    ];
    assert_eq!(marks(changes(&mut conn, &wt).await), changed);

    // Removed, the worktree takes its marks along.
    let remove = Control::RemoveWorktree {
        path: wt.clone(),
        force: true,
    };
    conn.send(0, remove).await;
    let removed = conn.control().await.1;
    assert!(
        matches!(removed, Control::WorktreeRemoved { .. }),
        "{removed:?}"
    );
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "{}");
    drop(conn);
    stop(daemon);
}
