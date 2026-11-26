//! Staging, committing and pushing from the Changes panel (27.6) through the service, against a
//! bare repository beside the project standing for GitHub (never a real remote).

use hive_protocol::{Control, GitAction, Role, SpaceEnv};

use crate::common::{Conn, stop};
use crate::worktree::Repo;

/// The next answer to a git request, skipping what the watches send meanwhile.
async fn answer(app: &mut Conn, request: Control) -> Control {
    app.send(0, request).await;
    loop {
        let (_, message) = app.control().await;
        if matches!(
            message,
            Control::GitDone { .. } | Control::GitFailed { .. } | Control::PushTarget { .. }
        ) {
            return message;
        }
    }
}

#[tokio::test]
async fn a_change_is_committed_as_the_space_and_pushed_never_forced() {
    let repo = Repo::new();
    let bare = repo.env.path("github.git");
    let bare = bare.to_str().unwrap();
    repo.git(&["init", "-q", "--bare", bare]);
    let daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;
    let root = repo.root.display().to_string();
    app.send(0, Control::AddProject { path: root.clone() })
        .await;
    assert!(matches!(
        app.control().await.1,
        Control::ProjectAdded { .. }
    ));
    // The space's git identity (21.5) signs the commit.
    let identity = SpaceEnv {
        git_name: Some("Space Me".into()),
        git_email: Some("space@me".into()),
        ..SpaceEnv::default()
    };
    let update = Control::UpdateSpace {
        id: "default".into(),
        name: "Default".into(),
        env: identity,
    };
    app.send(0, update).await;
    while !matches!(app.any_control().await.1, Control::Spaces { .. }) {}

    repo.write("a.txt", "a\n");
    repo.write("b.txt", "b\n");
    let stage = Control::StageFiles {
        worktree: root.clone(),
        paths: vec!["a.txt".into()],
        staged: true,
    };
    let staged = Control::GitDone {
        worktree: root.clone(),
        action: GitAction::Stage,
        message: "Staged a.txt".into(),
    };
    assert_eq!(answer(&mut app, stage.clone()).await, staged);
    // Unstaged and staged again.
    let unstage = Control::StageFiles {
        worktree: root.clone(),
        paths: vec!["a.txt".into()],
        staged: false,
    };
    let Control::GitDone {
        action: GitAction::Unstage,
        ..
    } = answer(&mut app, unstage).await
    else {
        panic!("expected the unstage")
    };
    assert_eq!(answer(&mut app, stage).await, staged);
    let commit = Control::Commit {
        worktree: root.clone(),
        message: "Add a".into(),
    };
    let Control::GitDone {
        action: GitAction::Commit,
        message,
        ..
    } = answer(&mut app, commit).await
    else {
        panic!("expected the commit")
    };
    let short = repo.git(&["log", "-1", "--format=%h"]);
    assert_eq!(message, format!("Committed {short}: Add a"));
    assert_eq!(
        repo.git(&["log", "-1", "--format=%an <%ae>|%s"]),
        "Space Me <space@me>|Add a"
    );
    assert_eq!(repo.git(&["status", "--porcelain"]), "?? b.txt");

    // No upstream and no origin: refused, nothing run.
    let ask = Control::GetPushTarget {
        worktree: root.clone(),
    };
    let none = "main has no upstream and this repository has no remote origin";
    assert_eq!(
        answer(&mut app, ask.clone()).await,
        Control::PushTarget {
            worktree: root.clone(),
            branch: None,
            target: None,
            set_upstream: false,
            ahead: None,
            error: Some(none.into()),
        }
    );
    repo.git(&["remote", "add", "origin", bare]);
    assert_eq!(
        answer(&mut app, ask.clone()).await,
        Control::PushTarget {
            worktree: root.clone(),
            branch: Some("main".into()),
            target: Some("origin/main".into()),
            set_upstream: true,
            ahead: None,
            error: None,
        }
    );
    let push = Control::Push {
        worktree: root.clone(),
        branch: "main".into(),
        target: "origin/main".into(),
    };
    let pushed = Control::GitDone {
        worktree: root.clone(),
        action: GitAction::Push,
        message: "Pushed main to origin/main, now its upstream".into(),
    };
    assert_eq!(answer(&mut app, push.clone()).await, pushed);
    assert_eq!(
        repo.git_in(
            repo.env.path("github.git").as_path(),
            &["rev-parse", "main"]
        ),
        repo.git(&["rev-parse", "HEAD"])
    );

    // Someone else pushes first: git's refusal, and their commit stays.
    let other = repo.env.path("other");
    let other_path = other.to_str().unwrap();
    repo.git(&["clone", "-q", "-b", "main", bare, other_path]);
    repo.git_in(&other, &["commit", "-q", "--allow-empty", "-m", "theirs"]);
    repo.git_in(&other, &["push", "-q", "origin", "main"]);
    let theirs = repo.git_in(&other, &["rev-parse", "HEAD"]);
    repo.git(&["commit", "-q", "--allow-empty", "-m", "mine"]);
    let Control::PushTarget { ahead, .. } = answer(&mut app, ask).await else {
        panic!("expected the target")
    };
    assert_eq!(ahead, Some(1));
    let Control::GitFailed {
        action: GitAction::Push,
        message,
        ..
    } = answer(&mut app, push.clone()).await
    else {
        panic!("expected the refusal")
    };
    assert!(message.contains("[rejected]"), "{message}");
    assert_eq!(
        repo.git_in(
            repo.env.path("github.git").as_path(),
            &["rev-parse", "main"]
        ),
        theirs
    );

    // A detached HEAD has nothing to push.
    repo.git(&["switch", "-q", "--detach"]);
    let Control::GitFailed { message, .. } = answer(&mut app, push).await else {
        panic!("expected the refusal")
    };
    assert_eq!(
        message,
        "This worktree is on no branch (a detached HEAD): nothing to push"
    );
    // Only a worktree of a followed project.
    let elsewhere = Control::Commit {
        worktree: "/elsewhere".into(),
        message: "m".into(),
    };
    let refused = Control::GitFailed {
        worktree: "/elsewhere".into(),
        action: GitAction::Commit,
        message: "/elsewhere is not a worktree of a followed project".into(),
    };
    assert_eq!(answer(&mut app, elsewhere).await, refused);
    // The workflows run by hand are read off the frame loop: none in this repository.
    let list = Control::ListWorkflows {
        project: root.clone(),
        worktree: root.clone(),
    };
    app.send(0, list).await;
    loop {
        if let Control::Workflows {
            workflows, error, ..
        } = app.control().await.1
        {
            assert_eq!((workflows, error), (vec![], None));
            break;
        }
    }
    drop(app);
    stop(daemon);
}
