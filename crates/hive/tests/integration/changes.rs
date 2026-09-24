use std::os::unix::ffi::OsStrExt;

use hive_protocol::{ChangedFile, Control, FileStatus, Role};

use crate::common::{Conn, stop};
use crate::worktree::Repo;

async fn changes(conn: &mut Conn, path: &str) -> Control {
    let path = path.to_owned();
    conn.send(0, Control::ListChanges { path }).await;
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
    let odd = std::ffi::OsStr::from_bytes(b"odd\xff");
    std::fs::write(repo.root.join(odd), "z\n").unwrap();
    let root = repo.root.display().to_string();

    let daemon = repo.env.daemon();
    let mut conn = repo.env.connect(Role::App).await;
    // Only a worktree of a followed project.
    let refused = changes(&mut conn, &root).await;
    let error = format!("{root} is not a worktree of a followed project");
    assert_eq!(
        refused,
        Control::Changes {
            path: root.clone(),
            files: vec![],
            added: 0,
            removed: 0,
            error: Some(error),
        }
    );
    follow(&mut conn, &root).await;

    let mut renamed = file("new name.txt", FileStatus::Renamed, Some(0), Some(0));
    renamed.old_path = Some("old name.txt".into());
    assert_eq!(
        changes(&mut conn, &root).await,
        Control::Changes {
            path: root.clone(),
            files: vec![
                file("a.txt", FileStatus::Modified, Some(2), Some(1)),
                file("c.txt", FileStatus::Deleted, Some(0), Some(1)),
                file("img.bin", FileStatus::Modified, None, None),
                file("n.txt", FileStatus::Added, Some(1), Some(0)),
                renamed,
                file("odd\u{fffd}", FileStatus::Untracked, Some(1), Some(0)),
                file("u dir/u.txt", FileStatus::Untracked, Some(2), Some(0)),
            ],
            added: 6,
            removed: 2,
            error: None,
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
            path: fresh.clone(),
            files: vec![
                file("loose.txt", FileStatus::Untracked, Some(1), Some(0)),
                file("staged.txt", FileStatus::Added, Some(1), Some(0)),
            ],
            added: 2,
            removed: 0,
            error: None,
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
