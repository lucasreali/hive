use std::io::Write;

use futures_util::SinkExt;
use hive::file::{PDF_LIMIT, SERVE_LIMIT, TEXT_LIMIT, version};
use hive_protocol::{Control, DiffBase, Frame, FrameType, Media, READ_LIMIT, Role, SaveError};

use crate::common::{Conn, stop};
use crate::worktree::Repo;

async fn open(conn: &mut Conn, worktree: &str, path: &str) -> Control {
    let (worktree, path) = (worktree.to_owned(), path.to_owned());
    let base = DiffBase::Head;
    conn.send(
        0,
        Control::OpenFile {
            worktree,
            path,
            base,
        },
    )
    .await;
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
            media,
        } => {
            assert_eq!((w.as_str(), p.as_str(), binary), (worktree, path, false));
            assert_eq!(media, None, "{path}");
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

/// The `table` answer for `path` of `worktree`: its sheets' `(rows, total, partial)`, or its
/// error.
async fn table(
    conn: &mut Conn,
    worktree: &str,
    path: &str,
) -> Result<Vec<(Vec<Vec<String>>, u64, bool)>, String> {
    let (w, p) = (worktree.to_owned(), path.to_owned());
    let open = Control::OpenTable {
        worktree: w,
        path: p,
    };
    conn.send(0, open).await;
    match conn.control().await.1 {
        Control::Table {
            worktree: w,
            path: p,
            sheets,
            error,
        } => {
            assert_eq!((w.as_str(), p.as_str()), (worktree, path));
            match error {
                Some(error) => Err(error),
                None => Ok(sheets
                    .into_iter()
                    .map(|s| (s.rows, s.total, s.partial))
                    .collect()),
            }
        }
        other => panic!("expected a table, got {other:?}"),
    }
}

/// The `media` of the `file` answer for `path` of `worktree`.
async fn media(conn: &mut Conn, worktree: &str, path: &str) -> Option<Media> {
    match open(conn, worktree, path).await {
        Control::File { media, .. } => media,
        other => panic!("expected a file, got {other:?}"),
    }
}

/// The `file_range` answer to a `read_file` of `path` in the worktree `token` names, and the
/// bytes of the data frame after it, if any.
async fn read(conn: &mut Conn, token: &str, path: &str, range: (u64, u32)) -> (Control, Vec<u8>) {
    let (token, path, (offset, len)) = (token.to_owned(), path.to_owned(), range);
    let id = 9;
    let read = Control::ReadFile {
        id,
        token,
        path,
        offset,
        len,
    };
    conn.send(0, read).await;
    let (_, range) = conn.control().await;
    if !matches!(range, Control::FileRange { error: None, .. }) {
        return (range, Vec::new());
    }
    loop {
        let frame = conn.next().await.unwrap();
        if frame.kind == FrameType::Data {
            assert_eq!(frame.channel, id);
            return (range, frame.payload.to_vec());
        }
    }
}

/// A file starting with `head`, made `len` bytes long without writing them (sparse).
fn sized(path: std::path::PathBuf, head: &[u8], len: u64) {
    let mut file = std::fs::File::create(path).unwrap();
    file.write_all(head).unwrap();
    file.set_len(len).unwrap();
}

#[tokio::test]
async fn images_and_pdfs_are_read_in_ranges_through_their_worktree_s_token() {
    let repo = Repo::new();
    let png = b"\x89PNG\r\n\x1a\nrest".to_vec();
    std::fs::write(repo.root.join("a.png"), &png).unwrap();
    repo.write(
        "b.svg",
        "<?xml version=\"1.0\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\"/>\n",
    );
    repo.write("c.txt", "text\n");
    sized(repo.root.join("edge.gif"), b"GIF89a", SERVE_LIMIT);
    sized(repo.root.join("big.gif"), b"GIF89a", SERVE_LIMIT + 1);
    sized(repo.root.join("doc.pdf"), b"%PDF-1.4\n", SERVE_LIMIT + 1);
    sized(repo.root.join("huge.pdf"), b"%PDF-1.4\n", PDF_LIMIT + 1);
    let outside = repo.env.path("home/out.png");
    std::fs::write(&outside, &png).unwrap();
    std::os::unix::fs::symlink(&outside, repo.root.join("out.png")).unwrap();
    let root = repo.root.display().to_string();

    let daemon = repo.env.daemon();
    let mut conn = repo.env.connect(Role::App).await;
    // Not followed: no media, no token.
    assert_eq!(media(&mut conn, &root, "a.png").await, None);
    follow(&mut conn, &root).await;

    let Some(Media {
        mime,
        token,
        version,
    }) = media(&mut conn, &root, "a.png").await
    else {
        panic!("a.png is an image");
    };
    assert_eq!(mime, "image/png");
    assert!(version.starts_with("12-"), "{version}");
    let svg = media(&mut conn, &root, "b.svg").await.unwrap();
    assert_eq!((svg.mime.as_str(), &svg.token), ("image/svg+xml", &token));
    let pdf = media(&mut conn, &root, "doc.pdf").await.unwrap();
    assert_eq!(pdf.mime, "application/pdf");
    let edge = media(&mut conn, &root, "edge.gif").await.unwrap();
    assert_eq!(edge.mime, "image/gif");
    // Text, over its cap, outside the worktree: none.
    for path in ["c.txt", "big.gif", "huge.pdf", "out.png", "gone.png"] {
        assert_eq!(media(&mut conn, &root, path).await, None, "{path}");
    }

    let range = |size, mime: &str, version: &str| Control::FileRange {
        id: 9,
        size,
        mime: mime.into(),
        version: version.into(),
        error: None,
    };
    let whole = read(&mut conn, &token, "a.png", (0, READ_LIMIT)).await;
    assert_eq!(whole, (range(12, "image/png", &version), png.clone()));
    let part = read(&mut conn, &token, "a.png", (4, 3)).await;
    assert_eq!(part.1, png[4..7]);
    let (text, bytes) = read(&mut conn, &token, "c.txt", (0, READ_LIMIT)).await;
    assert!(
        matches!(text, Control::FileRange { size: 5, ref mime, .. } if mime == "application/octet-stream")
    );
    assert_eq!(bytes, b"text\n");
    // At most `READ_LIMIT` bytes a read.
    let (_, bytes) = read(&mut conn, &token, "doc.pdf", (0, u32::MAX)).await;
    assert_eq!(bytes.len(), READ_LIMIT as usize);

    let failed = |error: &str| Control::FileRange {
        id: 9,
        size: 0,
        mime: String::new(),
        version: String::new(),
        error: Some(error.into()),
    };
    for (token, path, error) in [
        (&*token, "big.gif", "big.gif is larger than 20 MiB"),
        (&*token, "huge.pdf", "huge.pdf is larger than 100 MiB"),
        (&*token, "../x", "not a relative path inside the worktree"),
        (&*token, "out.png", "the file resolves outside the worktree"),
        (&*token, "gone.png", "gone.png does not exist"),
        (&*root, "a.png", "no worktree has this token"),
    ] {
        let answer = read(&mut conn, token, path, (0, READ_LIMIT)).await;
        assert_eq!(answer, (failed(error), Vec::new()), "{path}");
    }

    // Unfollowed, its token reads nothing.
    let id = root.clone();
    conn.send(0, Control::RemoveProject { id }).await;
    while !matches!(conn.control().await.1, Control::ProjectRemoved { .. }) {}
    let gone = format!("{root} is not a worktree of a followed project");
    let answer = read(&mut conn, &token, "a.png", (0, READ_LIMIT)).await;
    assert_eq!(answer, (failed(&gone), Vec::new()));

    // A data frame is no request.
    conn.writer.send(Frame::data(3, &b"x"[..])).await.unwrap();
    let message = "unexpected message from the app".to_owned();
    assert_eq!(conn.control().await, (3, Control::Error { message }));
    drop(conn);
    stop(daemon);
}

#[tokio::test]
async fn a_table_file_is_read_on_disk_whatever_its_size() {
    let repo = Repo::new();
    repo.write("a.csv", "id,\"name, full\"\n1,Ann\n");
    // Past `TEXT_LIMIT`, and not UTF-8: its text is not shown, its first rows are.
    let rows: String = (0..20_000).map(|i| format!("{i},caf\u{e9}\n")).collect();
    let mut big = rows.into_bytes();
    big.extend(b"last,caf\xe9\n".repeat(TEXT_LIMIT as usize / 10));
    std::fs::write(repo.root.join("big.TSV"), &big).unwrap();
    let root = repo.root.display().to_string();

    let daemon = repo.env.daemon();
    let mut conn = repo.env.connect(Role::App).await;
    let error = format!("{root} is not a worktree of a followed project");
    assert_eq!(table(&mut conn, &root, "a.csv").await, Err(error));
    follow(&mut conn, &root).await;

    let cells = |row: &[&str]| row.iter().map(|&c| c.to_owned()).collect::<Vec<_>>();
    let small = vec![cells(&["id", "name, full"]), cells(&["1", "Ann"])];
    assert_eq!(
        table(&mut conn, &root, "a.csv").await,
        Ok(vec![(small, 2, false)])
    );
    let sheets = table(&mut conn, &root, "big.TSV").await.unwrap();
    let [(rows, total, partial)] = &sheets[..] else {
        panic!("one sheet: {sheets:?}");
    };
    assert_eq!(rows.len(), hive::table::TABLE_ROWS);
    assert_eq!(rows[1], cells(&["1,caf\u{e9}"]));
    let expected = 20_000 + TEXT_LIMIT / 10;
    assert_eq!((*total, *partial), (expected, false));
    assert_eq!(
        table(&mut conn, &root, "a.txt").await,
        Err("not a table file".into())
    );
    assert_eq!(
        table(&mut conn, &root, "gone.csv").await,
        Err("gone.csv does not exist".into())
    );
    drop(conn);
    stop(daemon);
}

#[tokio::test]
async fn a_workbook_is_read_by_a_child_and_its_failures_only_end_that() {
    let repo = Repo::new();
    let fixtures =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workbooks");
    for name in ["book.xlsx", "book.xls", "book.ods", "huge.xls", "big.xls"] {
        std::fs::copy(fixtures.join(name), repo.root.join(name)).unwrap();
    }
    repo.write("broken.ODS", "not a zip");
    let root = repo.root.display().to_string();

    let daemon = repo.env.daemon();
    let mut conn = repo.env.connect(Role::App).await;
    follow(&mut conn, &root).await;

    let cells = |row: &[&str]| row.iter().map(|&c| c.to_owned()).collect::<Vec<_>>();
    let orders = vec![
        cells(&["Item", "Date", "Amount", "Note"]),
        cells(&["Pen", "2024-03-15", "", "blue"]),
        cells(&["Ink", "2024-03-16 14:30:00", "7", "TRUE"]),
        cells(&["Total", "", "14"]),
    ];
    let notes = vec![cells(&["Hello"]), cells(&["12:00:00"])];
    for name in ["book.xlsx", "book.xls", "book.ods"] {
        assert_eq!(
            table(&mut conn, &root, name).await,
            Ok(vec![(orders.clone(), 4, false), (notes.clone(), 2, false)]),
            "{name}"
        );
    }
    let broken = table(&mut conn, &root, "broken.ODS").await.unwrap_err();
    assert!(broken.starts_with("not a readable workbook: "), "{broken}");
    // Its dimensions claim 2^32 rows of 2^16 columns: the child's reservation fails and ends
    // it, the service answers and goes on.
    assert_eq!(
        table(&mut conn, &root, "huge.xls").await,
        Err("the workbook could not be read".into())
    );
    assert_eq!(table(&mut conn, &root, "book.ods").await.unwrap().len(), 2);
    // Its dimensions make the child reserve about 2.7 GB it never touches: past Linux's
    // `RLIMIT_AS` of 1 GiB, so it ends there; elsewhere the reservation stays virtual and
    // the footprint small, so it is read.
    let big = table(&mut conn, &root, "big.xls").await;
    if cfg!(target_os = "linux") {
        assert_eq!(big, Err("the workbook could not be read".into()));
    } else {
        assert_eq!(big, Ok(vec![(vec![cells(&["a"])], 1, false)]));
    }
    drop(conn);
    stop(daemon);
}

#[tokio::test]
async fn file_requests_list_the_worktrees_once() {
    let repo = Repo::new();
    // A git that logs its arguments first on the service's `PATH` (9.14).
    let (bin, log) = (repo.env.path("bin"), repo.env.path("git.log"));
    std::fs::create_dir(&bin).unwrap();
    let path = std::env::var_os("PATH").unwrap();
    let dirs = || std::env::split_paths(&path);
    let real = dirs().map(|d| d.join("git")).find(|git| git.is_file());
    let script = format!(
        "#!/bin/sh\necho \"$*\" >> '{}'\nexec '{}' \"$@\"\n",
        log.display(),
        real.unwrap().display()
    );
    std::fs::write(bin.join("git"), script).unwrap();
    let executable = std::os::unix::fs::PermissionsExt::from_mode(0o755);
    std::fs::set_permissions(bin.join("git"), executable).unwrap();
    let on_path = std::env::join_paths(std::iter::once(bin).chain(dirs())).unwrap();
    let daemon = repo.env.daemon_with(repo.env.hive().env("PATH", on_path));
    let lists = || {
        let logged = std::fs::read_to_string(&log).unwrap_or_default();
        logged.matches("worktree list").count()
    };
    let mut conn = repo.env.connect(Role::App).await;
    let root = repo.root.display().to_string();
    follow(&mut conn, &root).await;
    // Its main worktree found, listed when added, and again once its registry is watched.
    crate::common::wait_until(|| lists() >= 3);
    let listed = lists();
    let hello = (text("hello"), text("hello"), false, None);
    for _ in 0..5 {
        assert_eq!(sides(&mut conn, &root, "README").await, hello);
    }
    assert_eq!(lists(), listed, "listed again");

    // A worktree the app creates is followed at once.
    let create = Control::CreateWorktree {
        project: root.clone(),
        name: "w".into(),
        base: None,
    };
    conn.send(0, create).await;
    let Control::WorktreeCreated { path, .. } = conn.control().await.1 else {
        panic!("expected a new worktree")
    };
    assert_eq!(sides(&mut conn, &path, "README").await, hello);
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
    let open = |reveal| Control::OpenInEditor {
        worktree: root.clone(),
        path: program.to_owned(),
        reveal,
    };
    conn.send(0, open(false)).await;
    assert_eq!(
        conn.control().await.1,
        Control::EditorTarget {
            worktree: root.clone(),
            path: program.to_owned(),
            reveal: false,
            windows_path: None,
            error: Some(format!(
                "{system} might run or install {program}: Hive only reveals it"
            )),
        }
    );
    // Revealed, it is only shown in the file manager.
    conn.send(0, open(true)).await;
    let (_, revealed) = conn.control().await;
    #[cfg(not(target_os = "linux"))]
    {
        let real = std::fs::canonicalize(repo.root.join(program)).unwrap();
        let shown = Control::EditorTarget {
            worktree: root.clone(),
            path: program.to_owned(),
            reveal: true,
            windows_path: Some(real.display().to_string()),
            error: None,
        };
        assert_eq!(revealed, shown);
    }
    // No `wslpath` on this machine: only running it fails.
    #[cfg(target_os = "linux")]
    assert_eq!(
        revealed,
        Control::EditorTarget {
            worktree: root.clone(),
            path: program.to_owned(),
            reveal: true,
            windows_path: None,
            error: Some("No such file or directory (os error 2)".to_owned()),
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

#[tokio::test]
async fn files_are_moved_and_folders_created_inside_a_followed_worktree() {
    let repo = Repo::new();
    repo.commit("a.txt", "one\n");
    let root = repo.root.display().to_string();
    let daemon = repo.env.daemon();
    let mut conn = repo.env.connect(Role::App).await;
    let mkdir = |name: &str| Control::CreateFolder {
        worktree: root.clone(),
        folder: String::new(),
        name: name.to_owned(),
    };
    let moving = |path: &str, folder: &str| Control::MoveFile {
        worktree: root.clone(),
        path: path.to_owned(),
        folder: folder.to_owned(),
    };
    let failed = |message: &str| Control::FileOpFailed {
        worktree: root.clone(),
        message: message.to_owned(),
    };
    // Not a followed worktree yet.
    conn.send(0, mkdir("d")).await;
    assert!(matches!(
        conn.control().await.1,
        Control::FileOpFailed { .. }
    ));
    conn.send(0, moving("a.txt", "")).await;
    assert!(matches!(
        conn.control().await.1,
        Control::FileOpFailed { .. }
    ));
    follow(&mut conn, &root).await;

    conn.send(0, mkdir("d")).await;
    let created = Control::FolderCreated {
        worktree: root.clone(),
        path: "d".to_owned(),
    };
    assert_eq!(conn.control().await.1, created);
    assert!(repo.root.join("d").is_dir());
    conn.send(0, mkdir("d")).await;
    assert_eq!(conn.control().await.1, failed("d already exists"));

    conn.send(0, moving("a.txt", "d")).await;
    let moved = Control::FileRenamed {
        worktree: root.clone(),
        path: "a.txt".to_owned(),
        to: "d/a.txt".to_owned(),
    };
    assert_eq!(conn.control().await.1, moved);
    assert_eq!(sides(&mut conn, &root, "d/a.txt").await.0, text("one\n"));
    conn.send(0, moving("d/a.txt", "..")).await;
    assert_eq!(
        conn.control().await.1,
        failed("not a relative path inside the worktree")
    );

    // A folder moves with what it holds, unless a process works inside it.
    conn.send(0, mkdir("e")).await;
    conn.control().await;
    conn.send(0, moving("d", "e")).await;
    let moved = Control::FileRenamed {
        worktree: root.clone(),
        path: "d".to_owned(),
        to: "e/d".to_owned(),
    };
    assert_eq!(conn.control().await.1, moved);
    assert_eq!(sides(&mut conn, &root, "e/d/a.txt").await.0, text("one\n"));
    let mut sleep = std::process::Command::new("sleep")
        .arg("30")
        .current_dir(repo.root.join("e/d"))
        .spawn()
        .unwrap();
    conn.send(0, mkdir("g")).await;
    conn.control().await;
    conn.send(0, moving("e", "g")).await;
    let busy = format!(
        "in use by sleep ({}): close its terminals first",
        sleep.id()
    );
    assert_eq!(conn.control().await.1, failed(&busy));
    sleep.kill().unwrap();
    sleep.wait().unwrap();
    assert!(repo.root.join("e/d/a.txt").exists());
    drop(conn);
    stop(daemon);
}

#[tokio::test]
async fn files_and_folders_are_deleted_inside_a_followed_worktree() {
    // On macOS they go to the Trash: the repository is on a disk image of its own, whose Trash
    // is never the user's.
    #[cfg(target_os = "macos")]
    let (volume, repo) = {
        let env = crate::common::Env::new();
        let volume = crate::common::Volume::new(&env.path("trash.dmg"), &env.path("home/vol"));
        (volume, Repo::in_env(env, "home/vol/repo"))
    };
    #[cfg(not(target_os = "macos"))]
    let repo = Repo::new();
    repo.commit("a.txt", "one\n");
    std::fs::create_dir_all(repo.root.join("d/e")).unwrap();
    std::fs::write(repo.root.join("d/e/b.txt"), "b").unwrap();
    let root = repo.root.display().to_string();
    let daemon = repo.env.daemon();
    let mut conn = repo.env.connect(Role::App).await;
    let delete = |path: &str| Control::DeleteFile {
        worktree: root.clone(),
        path: path.to_owned(),
    };
    let deleted = |path: &str| Control::FileDeleted {
        worktree: root.clone(),
        path: path.to_owned(),
    };
    let failed = |message: &str| Control::FileOpFailed {
        worktree: root.clone(),
        message: message.to_owned(),
    };
    // Not a followed worktree yet.
    conn.send(0, delete("a.txt")).await;
    assert!(matches!(
        conn.control().await.1,
        Control::FileOpFailed { .. }
    ));
    assert!(repo.root.join("a.txt").exists());
    follow(&mut conn, &root).await;

    conn.send(0, delete("a.txt")).await;
    assert_eq!(conn.control().await.1, deleted("a.txt"));
    assert!(!repo.root.join("a.txt").exists());
    conn.send(0, delete(".git")).await;
    assert_eq!(
        conn.control().await.1,
        failed("Hive does not change what is inside .git")
    );

    // A folder goes with what it holds, unless a process works inside it.
    let mut sleep = std::process::Command::new("sleep")
        .arg("30")
        .current_dir(repo.root.join("d/e"))
        .spawn()
        .unwrap();
    conn.send(0, delete("d")).await;
    let busy = format!(
        "in use by sleep ({}): close its terminals first",
        sleep.id()
    );
    assert_eq!(conn.control().await.1, failed(&busy));
    sleep.kill().unwrap();
    sleep.wait().unwrap();
    assert!(repo.root.join("d/e/b.txt").exists());
    conn.send(0, delete("d")).await;
    assert_eq!(conn.control().await.1, deleted("d"));
    assert!(!repo.root.join("d").exists());
    // A symlink goes itself, never what it points to (outside the worktree; on macOS still on
    // the test's disk image, so even a link followed never reaches the user's Trash).
    let outside = repo.root.parent().unwrap().join("outside");
    std::fs::create_dir_all(outside.join("f")).unwrap();
    std::fs::write(outside.join("s"), "s").unwrap();
    std::os::unix::fs::symlink(outside.join("s"), repo.root.join("ls")).unwrap();
    std::os::unix::fs::symlink(outside.join("f"), repo.root.join("lf")).unwrap();
    for link in ["ls", "lf"] {
        conn.send(0, delete(link)).await;
        assert_eq!(conn.control().await.1, deleted(link));
        assert!(repo.root.join(link).symlink_metadata().is_err());
    }
    assert!(outside.join("s").is_file() && outside.join("f").is_dir());
    #[cfg(target_os = "macos")]
    {
        for link in ["ls", "lf"] {
            let trashed = volume.trashed(link).symlink_metadata().unwrap();
            assert!(trashed.file_type().is_symlink());
        }
        let a = std::fs::read_to_string(volume.trashed("a.txt")).unwrap();
        let b = std::fs::read_to_string(volume.trashed("d/e/b.txt")).unwrap();
        assert_eq!((a.as_str(), b.as_str()), ("one\n", "b"));
    }
    drop(conn);
    stop(daemon);
}
