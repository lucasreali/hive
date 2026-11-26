//! Line provenance (27.5): `hive hook PostToolUse` from a Hive terminal records an agent's
//! edit, and `who_wrote` / `list_session_files` answer from it, also after the service starts
//! again.

use std::time::Duration;

use hive_protocol::{Control, DiffBase, LineOrigin, LineSide, Role};
use serde_json::json;

use crate::common::{Conn, stop};
use crate::worktree::Repo;

const NEW_LINE: &str = "let provenance = recorded_by_the_hook();";

/// Runs `hive hook PostToolUse` as Claude Code runs it in terminal 1, with an `Edit`'s payload
/// (SYNTHETIC: the shape follows Claude Code's hooks reference; the text is made up).
fn edit_hook(repo: &Repo, session: &str) {
    let file = repo.root.join("src/lib.rs");
    let original = "fn main() {}\n";
    let payload = json!({
        "session_id": session,
        "transcript_path": repo.env.path("home/.claude/projects/-repo/s.jsonl"),
        "cwd": repo.root,
        "hook_event_name": "PostToolUse",
        "tool_name": "Edit",
        "tool_use_id": "toolu_01Synthetic",
        "tool_input": {"file_path": file, "old_string": "fn main() {}",
            "new_string": format!("fn main() {{}}\n{NEW_LINE}"), "replace_all": false},
        "tool_response": {"filePath": file, "oldString": "fn main() {}",
            "newString": format!("fn main() {{}}\n{NEW_LINE}"), "originalFile": original,
            "structuredPatch": [{"oldStart": 1, "oldLines": 1, "newStart": 1, "newLines": 2,
                "lines": [" fn main() {}", format!("+{NEW_LINE}")]}],
            "userModified": false, "replaceAll": false},
    });
    let mut cmd = repo.env.hive();
    cmd.arg("hook")
        .arg("PostToolUse")
        .env("HIVE_TERMINAL_ID", "1");
    let out = crate::hook::run(cmd, payload.to_string().as_bytes());
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
}

async fn follow(app: &mut Conn, root: &str) {
    app.send(0, Control::AddProject { path: root.into() }).await;
    loop {
        if let (0, Control::ProjectAdded { .. }) = app.control().await {
            return;
        }
    }
}

async fn who_wrote(app: &mut Conn, root: &str, line: u32) -> LineOrigin {
    let asked = Control::WhoWrote {
        worktree: root.into(),
        path: "src/lib.rs".into(),
        line,
        side: LineSide::Worktree,
        base: DiffBase::Head,
    };
    app.send(0, asked).await;
    loop {
        if let (
            0,
            Control::LineAuthor {
                origin, line: l, ..
            },
        ) = app.control().await
        {
            assert_eq!(l, line);
            return origin;
        }
    }
}

#[tokio::test]
async fn an_agents_edit_is_found_from_its_line_after_a_restart() {
    let repo = Repo::new();
    let root = repo.root.display().to_string();
    repo.write("src/lib.rs", &format!("fn main() {{}}\n{NEW_LINE}\n"));
    let daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;
    follow(&mut app, &root).await;
    edit_hook(&repo, "0b5c9e7e-1111-4c2d-9f00-aaaaaaaaaaaa");
    // The hook's connection is handled on its own: asked until it is.
    let started = std::time::Instant::now();
    let (edit, commit) = loop {
        match who_wrote(&mut app, &root, 2).await {
            LineOrigin::Session { edit, commit } => break (edit, commit),
            other => assert!(started.elapsed() < Duration::from_secs(5), "{other:?}"),
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(commit, None);
    assert_eq!(edit.session, "0b5c9e7e-1111-4c2d-9f00-aaaaaaaaaaaa");
    assert_eq!((edit.tool.as_str(), edit.start, edit.len), ("Edit", 2, 1));
    assert_eq!(edit.tool_use_id.as_deref(), Some("toolu_01Synthetic"));
    // No log of it: it shows as deleted.
    assert!(edit.deleted);
    assert!(matches!(
        who_wrote(&mut app, &root, 1).await,
        LineOrigin::Unknown { .. }
    ));
    app.send(
        0,
        Control::ListSessionFiles {
            id: "0b5c9e7e-1111-4c2d-9f00-aaaaaaaaaaaa".into(),
        },
    )
    .await;
    let files = loop {
        if let (0, Control::SessionFiles { files, .. }) = app.control().await {
            break files;
        }
    };
    assert_eq!(files.len(), 1);
    assert_eq!(
        (
            files[0].path.as_str(),
            files[0].lines_now,
            files[0].first_line
        ),
        ("src/lib.rs", 1, Some(2))
    );
    // Written at shutdown, kept private, never inside the repository.
    drop(app);
    stop(daemon);
    let dir = repo.env.path("data/hive/provenance");
    let stored: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().collect();
    assert_eq!(stored.len(), 1, "{stored:?}");
    let text = std::fs::read_to_string(stored[0].path()).unwrap();
    assert!(!text.contains(NEW_LINE), "no file text is kept");
    assert!(!text.contains("fn main"), "no file text is kept");
    // A new service answers from the file.
    let _daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;
    let LineOrigin::Session { edit, .. } = who_wrote(&mut app, &root, 2).await else {
        panic!("not kept")
    };
    assert_eq!(edit.start, 2);
    // A commit later, the line is still the session's, live.
    repo.git(&["add", "-A"]);
    repo.git(&["commit", "-q", "-m", "agent work"]);
    tokio::time::sleep(Duration::from_millis(10)).await;
    assert!(matches!(
        who_wrote(&mut app, &root, 2).await,
        LineOrigin::Session { commit: None, .. }
    ));
}
