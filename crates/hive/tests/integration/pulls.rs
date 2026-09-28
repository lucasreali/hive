use std::path::PathBuf;

use hive_protocol::{Control, PullAction, Role, RunAction};

use crate::worktree::Repo;

const SEARCH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/gh/pr-search.json"
);
const RUNS: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/gh/run-list.json"
);

/// A GitHub remote, and a fake gh first on the user's `PATH` (their temporary shell config),
/// never the real one: it logs its first three arguments to the returned file, then runs
/// `answer` (a `case` on `"$1 $2"`).
fn fake_gh(repo: &Repo, answer: &str) -> PathBuf {
    repo.git(&["remote", "add", "origin", "https://github.com/o/r.git"]);
    let fake = repo.env.path("fake-gh");
    std::fs::create_dir(&fake).unwrap();
    let log = repo.env.path("gh.log");
    let script = format!(
        "#!/bin/sh\necho \"$1 $2 $3\" >> '{}'\ncase \"$1 $2\" in\n{answer}\nesac\n",
        log.display()
    );
    std::fs::write(fake.join("gh"), script).unwrap();
    let mode = std::os::unix::fs::PermissionsExt::from_mode(0o755);
    std::fs::set_permissions(fake.join("gh"), mode).unwrap();
    let fish = repo.env.path("config/fish");
    std::fs::create_dir_all(&fish).unwrap();
    let config = format!("set -gx PATH '{}' $PATH\n", fake.display());
    std::fs::write(fish.join("config.fish"), config).unwrap();
    log
}

#[tokio::test]
async fn the_pull_requests_view_runs_gh_off_the_frame_loop() {
    let repo = Repo::new();
    // The recorded list, and a checkout onto the branch `feature`.
    let log = fake_gh(
        &repo,
        &format!(
            "  \"api graphql\") cat '{SEARCH}' ;;\n  \"pr checkout\") git checkout -q -b feature ;;"
        ),
    );
    let mut daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;
    let root = repo.root.display().to_string();
    let add = Control::AddProject { path: root.clone() };
    app.send(0, add).await;
    assert!(matches!(
        app.control().await.1,
        Control::ProjectAdded { .. }
    ));

    let list = Control::ListPulls {
        project: root.clone(),
        force: false,
    };
    app.send(0, list).await;
    let Control::Pulls {
        project,
        repo: Some(listed),
        mine,
        error: None,
        ..
    } = app.control().await.1
    else {
        panic!("expected the list")
    };
    assert_eq!(
        (project, listed.name, mine.len()),
        (root.clone(), "cli/cli".into(), 3)
    );

    let checkout = Control::ActOnPull {
        project: root.clone(),
        number: 5,
        action: PullAction::Checkout,
    };
    app.send(0, checkout).await;
    let Control::WorktreeCreated { project, path, .. } = app.control().await.1 else {
        panic!("expected the new worktree")
    };
    let created = project.worktrees.iter().find(|w| w.path == path).unwrap();
    assert_eq!(
        path,
        repo.root
            .join(".claude/worktrees/pr-5")
            .display()
            .to_string()
    );
    // With its health, as every worktree the app gets.
    assert_eq!(created.branch.as_deref(), Some("feature"));
    assert!(created.status.is_some());
    // The list again, from GitHub: the new worktree may be a pull request's.
    assert!(matches!(
        app.control().await.1,
        Control::Pulls { error: None, .. }
    ));
    let calls = std::fs::read_to_string(&log).unwrap();
    assert_eq!(
        calls,
        "api graphql --hostname\npr checkout 5\napi graphql --hostname\n"
    );
    drop(app);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn the_actions_view_runs_gh_off_the_frame_loop() {
    let repo = Repo::new();
    // The recorded runs; `gh run view` has nothing to say.
    let log = fake_gh(&repo, &format!("  \"run list\") cat '{RUNS}' ;;"));
    let mut daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;
    let root = repo.root.display().to_string();
    app.send(0, Control::AddProject { path: root.clone() })
        .await;
    assert!(matches!(
        app.control().await.1,
        Control::ProjectAdded { .. }
    ));

    let list = Control::ListRuns {
        project: root.clone(),
        branch: None,
        force: false,
    };
    app.send(0, list).await;
    let Control::Runs {
        runs, error: None, ..
    } = app.control().await.1
    else {
        panic!("expected the runs")
    };
    assert_eq!(runs.len(), 5);

    let cancel = Control::ActOnRun {
        project: root.clone(),
        run: 7,
        action: RunAction::Cancel,
        branch: None,
    };
    app.send(0, cancel).await;
    let done = Control::RunDone {
        project: root.clone(),
        run: 7,
        message: "Cancelling the run".into(),
    };
    assert_eq!(app.control().await.1, done);
    assert!(matches!(
        app.control().await.1,
        Control::Run {
            run: 7,
            detail: None,
            ..
        }
    ));
    assert!(matches!(
        app.control().await.1,
        Control::Runs { error: None, .. }
    ));
    let calls = std::fs::read_to_string(&log).unwrap();
    assert_eq!(
        calls,
        "run list --repo\nrun cancel 7\nrun view 7\nrun list --repo\n"
    );
    drop(app);
    assert!(daemon.wait_exit().success());
}
