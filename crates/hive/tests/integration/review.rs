//! Review worktrees (27.4) through the app's requests.

use std::path::Path;

use hive_protocol::{Control, Role, SpaceEnv};

use crate::common::{Conn, stop};
use crate::worktree::Repo;

/// The answer to `message`, past the projects the registry watch sends meanwhile (9.36).
async fn request(conn: &mut Conn, message: Control) -> Control {
    conn.send(0, message).await;
    loop {
        match conn.control().await.1 {
            Control::Projects { .. } | Control::WorktreeStatus { .. } => {}
            answer => return answer,
        }
    }
}

#[tokio::test]
async fn a_review_is_made_brought_back_on_request_and_discarded() {
    let repo = Repo::new();
    let root = repo.root.display().to_string();
    let daemon = repo.env.daemon();
    let mut conn = repo.env.connect(Role::App).await;
    let add = Control::AddProject { path: root.clone() };
    assert!(matches!(
        request(&mut conn, add).await,
        Control::ProjectAdded { .. }
    ));
    // The merge commit gets the space's identity (21.5).
    let identity = Control::UpdateSpace {
        id: "default".into(),
        name: "Default".into(),
        env: SpaceEnv {
            git_name: Some("Space Me".into()),
            git_email: Some("space@example.com".into()),
            ..SpaceEnv::default()
        },
    };
    conn.send(0, identity).await;
    while !matches!(conn.any_control().await.1, Control::Spaces { .. }) {}

    // The main worktree's review: a new worktree, marked as its review.
    let create = Control::CreateReview { path: root.clone() };
    let Control::WorktreeCreated { project, path, .. } = request(&mut conn, create).await else {
        panic!("expected a review")
    };
    assert_eq!(path, format!("{root}/.claude/worktrees/review-main"));
    let review = project.worktrees.iter().find(|w| w.path == path).unwrap();
    assert_eq!(review.review_of.as_deref(), Some(root.as_str()));
    assert_eq!(review.branch.as_deref(), Some("review-main"));
    // Listed again, it is still marked.
    let Control::Projects { projects } = ({
        conn.send(0, Control::ListProjects).await;
        conn.control().await.1
    }) else {
        panic!("expected projects")
    };
    let listed = projects[0].worktrees.iter().find(|w| w.path == path);
    assert_eq!(listed.unwrap().review_of.as_deref(), Some(root.as_str()));

    let refused = request(&mut conn, Control::CreateReview { path: "/no".into() }).await;
    let Control::CreateWorktreeFailed {
        project,
        name,
        message,
    } = refused
    else {
        panic!("expected a failure: {refused:?}")
    };
    assert_eq!((project.as_str(), name.as_str()), ("", "/no"));
    assert_eq!(message, "/no is not a worktree of a followed project");

    // The plan lists the review's commits; nothing changes until asked.
    repo.git_in(
        Path::new(&path),
        &["commit", "-q", "--allow-empty", "-m", "fix"],
    );
    repo.git(&["commit", "-q", "--allow-empty", "-m", "meanwhile"]);
    let plan = Control::PlanBringBack { path: path.clone() };
    let Control::BringBackPlan {
        plan: Some(plan),
        error: None,
        ..
    } = request(&mut conn, plan).await
    else {
        panic!("expected a plan")
    };
    assert_eq!((plan.total, plan.fast_forward), (1, false));
    assert_eq!(plan.commits[0].summary, "fix");
    assert_eq!(plan.into, "main");
    let not_a_review = Control::PlanBringBack { path: root.clone() };
    let Control::BringBackPlan {
        plan: None,
        error: Some(error),
        ..
    } = request(&mut conn, not_a_review).await
    else {
        panic!("expected no plan")
    };
    assert_eq!(error, "main is not a review");

    // A tip the user did not see is refused; the one shown is merged.
    let bring = |tip: &str| Control::BringBack {
        path: path.clone(),
        into: plan.into.clone(),
        base: plan.base.clone(),
        tip: tip.into(),
    };
    let Control::BringBackFailed { message, .. } = request(&mut conn, bring("0")).await else {
        panic!("expected a refusal")
    };
    assert_eq!(
        message,
        "review-main changed since the commits were shown: look again"
    );
    let Control::BroughtBack {
        project,
        commits,
        fast_forward,
        ..
    } = request(&mut conn, bring(&plan.tip)).await
    else {
        panic!("expected the changes back")
    };
    assert_eq!((commits, fast_forward), (1, false));
    assert!(project.worktrees[0].status.is_some(), "with its health");
    assert_eq!(
        repo.git(&["log", "-1", "--format=%P|%s|%an <%ae>"]),
        format!(
            "{} {}|Merge review review-main into main|Space Me <space@example.com>",
            repo.git(&["rev-parse", "HEAD~"]),
            plan.tip
        )
    );

    // Discarded once brought back: the worktree and its branch go.
    let discard = |path: &str| Control::DiscardReview {
        path: path.into(),
        force: false,
    };
    let Control::RemoveWorktreeFailed { message, .. } = request(&mut conn, discard(&root)).await
    else {
        panic!("expected a refusal")
    };
    assert_eq!(message, "main is not a review");
    let Control::WorktreeRemoved {
        project,
        path: gone,
    } = request(&mut conn, discard(&path)).await
    else {
        panic!("expected a removal")
    };
    assert_eq!(gone, path);
    assert!(project.worktrees.iter().all(|w| w.path != path));
    assert_eq!(repo.git(&["branch", "--list", "review-main"]), "");
    let saved = std::fs::read_to_string(repo.env.path("data/hive/reviews.json")).unwrap();
    assert_eq!(saved, "[]");
    drop(conn);
    stop(daemon);
}
