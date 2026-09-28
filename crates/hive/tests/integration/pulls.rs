use hive_protocol::{Control, PullAction, Role};

use crate::worktree::Repo;

const SEARCH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/fixtures/gh/pr-search.json"
);

#[tokio::test]
async fn the_pull_requests_view_runs_gh_off_the_frame_loop() {
    let repo = Repo::new();
    repo.git(&["remote", "add", "origin", "https://github.com/o/r.git"]);
    // A fake gh first on the user's `PATH` (their temporary shell config), never the real one:
    // the recorded list, and a checkout onto the branch `feature`.
    let fake = repo.env.path("fake-gh");
    std::fs::create_dir(&fake).unwrap();
    let log = repo.env.path("gh.log");
    let script = format!(
        r#"#!/bin/sh
echo "$1 $2 $3" >> '{}'
case "$1 $2" in
  "api graphql") cat '{SEARCH}' ;;
  "pr checkout") git checkout -q -b feature ;;
esac
"#,
        log.display()
    );
    std::fs::write(fake.join("gh"), script).unwrap();
    let mode = std::os::unix::fs::PermissionsExt::from_mode(0o755);
    std::fs::set_permissions(fake.join("gh"), mode).unwrap();
    let fish = repo.env.path("config/fish");
    std::fs::create_dir_all(&fish).unwrap();
    let config = format!("set -gx PATH '{}' $PATH\n", fake.display());
    std::fs::write(fish.join("config.fish"), config).unwrap();
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
