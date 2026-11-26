//! The diff since an agent's last turn and since a chosen commit (27.2), through the real
//! service: hooks fired by `hive hook` as Claude Code's wrapper fires them, from a terminal
//! running a stand-in `claude` (never the real one).

use std::io::Write;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use hive_protocol::{ChangedFile, Control, DiffBase, FileStatus, Role};
use serde_json::json;

use crate::common::Conn;
use crate::worktree::Repo;

/// Runs `hive hook <event>` from terminal 1 with `payload` on stdin, as the wrapper does.
fn fire(repo: &Repo, event: &str, payload: serde_json::Value) {
    let mut child = repo
        .env
        .hive()
        .args(["hook", event])
        .env("HIVE_TERMINAL_ID", "1")
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(payload.to_string().as_bytes()).unwrap();
    drop(stdin);
    assert!(child.wait().unwrap().success());
}

/// Follows the project at `path`, once the service has added it.
async fn follow(app: &mut Conn, path: String) {
    app.send(0, Control::AddProject { path }).await;
    while !matches!(app.control().await, (0, Control::ProjectAdded { .. })) {}
}

/// The next `changes` of `path` against `base`, other messages skipped.
async fn changes(app: &mut Conn, path: &str, base: &DiffBase) -> Control {
    let (path, base) = (path.to_owned(), base.clone());
    app.send(0, Control::ListChanges { path, base }).await;
    loop {
        if let (0, changes @ Control::Changes { .. }) = app.control().await {
            return changes;
        }
    }
}

/// The `changes` of `path` against `base` once `ready` holds for them (the snapshot runs in the
/// background), asked at most 200 times.
async fn changes_once(
    app: &mut Conn,
    path: &str,
    base: &DiffBase,
    ready: impl Fn(&Control) -> bool,
) -> Control {
    for _ in 0..200 {
        let listed = changes(app, path, base).await;
        if ready(&listed) {
            return listed;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("the changes never became ready");
}

fn turns(changes: &Control) -> Vec<String> {
    match changes {
        Control::Changes { turns, .. } => turns.iter().map(|t| t.session.clone()).collect(),
        _ => vec![],
    }
}

/// Every file under `dir` with its bytes; objects' bytes only (git may refresh their times).
fn files(dir: &Path) -> Vec<(std::path::PathBuf, Vec<u8>)> {
    let mut found = Vec::new();
    let mut folders = vec![dir.to_owned()];
    while let Some(folder) = folders.pop() {
        for entry in std::fs::read_dir(&folder).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                folders.push(path);
            } else {
                found.push((path.clone(), std::fs::read(&path).unwrap()));
            }
        }
    }
    found.sort();
    found
}

#[tokio::test]
async fn the_diff_since_an_agents_last_turn_shows_only_what_changed_since() {
    let repo = Repo::new();
    let root = repo.root.display().to_string();
    repo.commit("a.txt", "a\n");
    let fake = repo.env.fake_claude("home/claude");
    let mut daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;
    let path = root.clone();
    follow(&mut app, path).await;
    app.open_terminal(1, &repo.root).await;
    app.input(1, &format!("'{}'\r", fake.display())).await;
    // Before the turn: the user's own edit, staged, and an untracked file.
    repo.write("a.txt", "a\nuser\n");
    repo.git(&["add", "a.txt"]);
    repo.write("notes.txt", "n\n");
    let git_before = files(&repo.root.join(".git"));
    fire(
        &repo,
        "SessionStart",
        json!({"session_id": "s1", "cwd": root}),
    );
    fire(
        &repo,
        "UserPromptSubmit",
        json!({"session_id": "s1", "cwd": root}),
    );
    let turn = DiffBase::Turn {
        session: "s1".into(),
    };
    let ready = |c: &Control| turns(c) == ["s1"];
    changes_once(&mut app, &root, &turn, ready).await;
    // The turn: an edit, a new file, and the untracked file changed.
    repo.write("a.txt", "a\nuser\nagent\n");
    repo.write("b.txt", "b\n");
    repo.write("notes.txt", "n\nmore\n");
    let edit = json!({"session_id": "s1", "cwd": root, "tool_name": "Edit"});
    fire(&repo, "PostToolUse", edit);
    let file = |path: &str, status| ChangedFile {
        path: path.into(),
        status,
        old_path: None,
        added: Some(1),
        removed: Some(0),
    };
    let Control::Changes {
        base,
        base_error,
        files: listed,
        turns: listed_turns,
        ..
    } = changes(&mut app, &root, &turn).await
    else {
        unreachable!()
    };
    assert_eq!((base, base_error), (turn.clone(), None));
    assert_eq!(
        listed,
        [
            file("a.txt", FileStatus::Modified),
            file("b.txt", FileStatus::Added),
            file("notes.txt", FileStatus::Modified),
        ]
    );
    assert!(!listed_turns[0].late);
    // The open file's base is its text at the turn's start.
    let open = Control::OpenFile {
        worktree: root.clone(),
        path: "a.txt".into(),
        base: turn.clone(),
    };
    app.send(0, open).await;
    let file = loop {
        if let (0, file @ Control::File { .. }) = app.control().await {
            break file;
        }
    };
    let Control::File { content, base, .. } = file else {
        unreachable!()
    };
    assert_eq!(content.as_deref(), Some("a\nuser\nagent\n"));
    assert_eq!(base.as_deref(), Some("a\nuser\n"));
    // Nothing of the user's `.git` changed: not the index, no file added.
    let git_after = files(&repo.root.join(".git"));
    assert_eq!(git_after, git_before);

    // A commit picked: the commits listed, then the diff from the one before the last.
    app.send(0, Control::ListCommits { path: root.clone() })
        .await;
    let commits = loop {
        if let (0, Control::Commits { commits, .. }) = app.control().await {
            break commits;
        }
    };
    let subjects: Vec<&str> = commits.iter().map(|c| c.subject.as_str()).collect();
    assert_eq!(subjects, ["a.txt", "README"]);
    let picked = DiffBase::Commit {
        id: commits[1].id.clone(),
    };
    let Control::Changes {
        base,
        files: listed,
        ..
    } = changes(&mut app, &root, &picked).await
    else {
        unreachable!()
    };
    assert_eq!(base, picked);
    let names: Vec<&str> = listed.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(names, ["a.txt", "b.txt", "notes.txt"]);

    // The session ends: its snapshots go, and the turn base falls back to HEAD.
    fire(
        &repo,
        "SessionEnd",
        json!({"session_id": "s1", "cwd": root}),
    );
    let gone = changes_once(&mut app, &root, &turn, |c| turns(c).is_empty()).await;
    let Control::Changes {
        base, base_error, ..
    } = gone
    else {
        unreachable!()
    };
    assert_eq!(base, DiffBase::Head);
    assert_eq!(
        base_error.as_deref(),
        Some("No snapshot of this agent's last turn in this worktree")
    );
    drop(app);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn a_snapshot_taken_sends_the_watched_worktrees_changes_again() {
    let repo = Repo::new();
    let root = repo.root.display().to_string();
    let mut daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;
    let path = root.clone();
    follow(&mut app, path).await;
    app.open_terminal(1, &repo.root).await;
    let turn = DiffBase::Turn {
        session: "s1".into(),
    };
    let watch = Control::WatchWorktree {
        path: root.clone(),
        base: turn.clone(),
    };
    app.send(0, watch).await;
    // A session id that fits no ref gets no snapshot (the service says why in its log).
    fire(
        &repo,
        "SessionStart",
        json!({"session_id": "bad id", "cwd": root}),
    );
    fire(
        &repo,
        "UserPromptSubmit",
        json!({"session_id": "bad id", "cwd": root}),
    );
    fire(
        &repo,
        "SessionStart",
        json!({"session_id": "s1", "cwd": root}),
    );
    fire(
        &repo,
        "UserPromptSubmit",
        json!({"session_id": "s1", "cwd": root}),
    );
    // Unasked: the watched worktree's changes once the snapshot is there.
    loop {
        let (_, message) = app.control().await;
        if let Control::Changes { turns, base, .. } = message
            && !turns.is_empty()
        {
            assert_eq!(turns.len(), 1);
            assert_eq!((turns[0].session.as_str(), base), ("s1", turn));
            break;
        }
    }
    drop(app);
    assert!(daemon.wait_exit().success());
}
