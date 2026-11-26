use hive_protocol::{Control, DiffBase, ReviewComment, Role};

use crate::common::{Conn, stop};
use crate::worktree::Repo;

async fn ask(conn: &mut Conn, request: Control) -> Control {
    conn.send(0, request).await;
    conn.control().await.1
}

/// The comments of a `comments` answer for `path`.
fn comments(reply: Control, path: &str) -> Vec<ReviewComment> {
    match reply {
        Control::Comments {
            path: got,
            comments,
        } if got == path => comments,
        other => panic!("expected comments: {other:?}"),
    }
}

/// Each comment's id, place and whether it is outdated.
fn places(comments: &[ReviewComment]) -> Vec<(u64, u32, u32, bool)> {
    let place = |c: &ReviewComment| (c.id, c.from, c.to, c.outdated);
    comments.iter().map(place).collect()
}

fn add(path: &str, file: &str, from: u32, to: u32, body: &str) -> Control {
    Control::AddComment {
        path: path.to_owned(),
        file: file.to_owned(),
        from,
        to,
        body: body.to_owned(),
    }
}

#[tokio::test]
async fn comments_follow_their_code_are_kept_and_go_with_the_worktree() {
    let repo = Repo::new();
    assert!(repo.hive(&["create", "a"]).status.success());
    let root = repo.root.display().to_string();
    let wt = format!("{root}/.claude/worktrees/a");
    let code = "fn main() {\n    let total = compute();\n    println!(\"{total}\");\n}\n";
    std::fs::write(format!("{wt}/main.rs"), code).unwrap();
    let mut daemon = repo.env.daemon();
    let mut conn = repo.env.connect(Role::App).await;

    // Only a worktree of a followed project.
    let refused = ask(&mut conn, Control::ListComments { path: wt.clone() }).await;
    let error =
        format!("Cannot list the review comments: {wt} is not a worktree of a followed project");
    assert_eq!(refused, Control::Error { message: error });
    let path = root.clone();
    assert!(matches!(
        ask(&mut conn, Control::AddProject { path }).await,
        Control::ProjectAdded { .. }
    ));
    let none = ask(&mut conn, Control::ListComments { path: wt.clone() }).await;
    assert_eq!(comments(none, &wt), []);
    // A path out of the worktree is refused.
    let out = ask(&mut conn, add(&wt, "../../../README", 1, 1, "x")).await;
    let error = "Cannot add the comment: not a relative path inside the worktree";
    assert_eq!(
        out,
        Control::Error {
            message: error.into()
        }
    );

    let added = comments(
        ask(&mut conn, add(&wt, "main.rs", 2, 3, "Name it?")).await,
        &wt,
    );
    assert_eq!(places(&added), [(1, 2, 3, false)]);
    assert_eq!(added[0].code[0], "    let total = compute();");
    assert_eq!(added[0].body, "Name it?");
    assert!(!added[0].author.is_empty());
    assert!(added[0].ms > 0);

    // Lines added above: it moves with its code.
    std::fs::write(format!("{wt}/main.rs"), format!("use std::io;\n\n{code}")).unwrap();
    let list = ask(&mut conn, Control::ListComments { path: wt.clone() }).await;
    assert_eq!(places(&comments(list, &wt)), [(1, 4, 5, false)]);
    // Resolved, then kept across a restart in Hive's folder.
    let resolve = Control::ResolveComment {
        path: wt.clone(),
        id: 1,
        resolved: true,
    };
    assert!(comments(ask(&mut conn, resolve).await, &wt)[0].resolved);
    let file = repo.env.path("data/hive/comments.json");
    assert!(std::fs::read_to_string(&file).unwrap().contains(&wt));
    drop(conn);
    stop(daemon);
    daemon = repo.env.daemon();
    let mut conn = repo.env.connect(Role::App).await;
    let list = comments(
        ask(&mut conn, Control::ListComments { path: wt.clone() }).await,
        &wt,
    );
    assert_eq!(places(&list), [(1, 4, 5, false)]);
    assert!(list[0].resolved);

    // The watched worktree's comments come after each change, before its `changes`.
    let watch = Control::WatchWorktree {
        path: wt.clone(),
        base: DiffBase::Head,
    };
    conn.send(0, watch).await;
    assert!(matches!(conn.control().await.1, Control::Files { .. }));
    let first = comments(conn.control().await.1, &wt);
    assert_eq!(places(&first), [(1, 4, 5, false)]);
    assert!(matches!(conn.control().await.1, Control::Changes { .. }));
    std::fs::write(format!("{wt}/main.rs"), "fn main() {}\n").unwrap();
    let outdated = loop {
        match conn.control().await.1 {
            Control::Comments { comments, .. } => break comments,
            Control::Files { .. } | Control::WorktreeStatus { .. } => {}
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(places(&outdated), [(1, 4, 5, true)]);
    conn.send(0, Control::UnwatchWorktree).await;

    // Removed, the worktree takes its comments along.
    let remove = Control::RemoveWorktree {
        path: wt.clone(),
        force: true,
    };
    conn.send(0, remove).await;
    let removed = loop {
        match conn.control().await.1 {
            removed @ Control::WorktreeRemoved { .. } => break removed,
            Control::Changes { .. } | Control::WorktreeStatus { .. } => {}
            other => panic!("{other:?}"),
        }
    };
    assert!(matches!(removed, Control::WorktreeRemoved { .. }));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "{}");
    drop(conn);
    stop(daemon);
}
