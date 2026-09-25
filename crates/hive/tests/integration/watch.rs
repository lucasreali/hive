use std::io::Write;
use std::process::Stdio;
use std::time::Duration;

use hive_protocol::{Control, Role};

use crate::common::{Env, TIMEOUT};

/// Command line running a stand-in named `claude` (a copy of `dash` that sleeps),
/// so no real Claude Code runs.
fn fake_claude(env: &Env) -> String {
    let path = env.path("home/claude");
    std::fs::copy("/bin/dash", &path).unwrap();
    format!("{} -c 'sleep 30'\r", path.display())
}

#[tokio::test]
async fn claude_without_hook_events_is_reported() {
    let env = Env::new();
    let mut daemon = env.daemon();
    let mut app = env.connect(Role::App).await;
    app.open_terminal(4, &env.path("home")).await;
    app.input(4, &fake_claude(&env)).await;
    assert_eq!(app.control().await, (4, Control::UnhookedAgent));
    drop(app);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn claude_that_sent_session_start_is_not_reported() {
    let env = Env::new();
    let mut daemon = env.daemon();
    let mut app = env.connect(Role::App).await;
    app.open_terminal(6, &env.path("home")).await;
    app.input(6, &fake_claude(&env)).await;
    // Reading the output answers the shell's terminal queries (`Conn::next`).
    let start = std::time::Instant::now();
    while !env.processes().iter().any(|p| p.comm == "claude") {
        assert!(start.elapsed() < TIMEOUT, "claude did not start");
        let _ = tokio::time::timeout(Duration::from_millis(50), app.next()).await;
    }
    let mut hook = env
        .hive()
        .args(["hook", "SessionStart"])
        .env("HIVE_TERMINAL_ID", "6")
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    hook.stdin.take().unwrap().write_all(b"{}").unwrap();
    assert!(hook.wait().unwrap().success());
    let (_, Control::Agent(_)) = app.control().await else {
        panic!("expected the SessionStart event")
    };
    // Longer than the detection delay plus one check interval.
    let quiet = tokio::time::timeout(Duration::from_secs(7), app.control()).await;
    assert!(quiet.is_err(), "unexpected message: {quiet:?}");
    drop(app);
    assert!(daemon.wait_exit().success());
}
