use std::io::Write;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use hive_protocol::{Control, EventKind, Role};
use serde_json::{Value, json};

use crate::common::Env;

/// Runs `hive <args>` with `stdin` as input, like Claude Code runs a hook.
fn run(mut cmd: Command, stdin: &[u8]) -> Output {
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // The hook may stop reading early (oversized input); a broken pipe is fine.
    let _ = child.stdin.take().unwrap().write_all(stdin);
    child.wait_with_output().unwrap()
}

fn hook(env: &Env, args: &[&str], stdin: &[u8]) -> Output {
    let mut cmd = env.hive();
    cmd.arg("hook").args(args).env("HIVE_TERMINAL_ID", "9");
    run(cmd, stdin)
}

#[tokio::test]
async fn hook_call_reaches_the_app_and_prints_nothing() {
    let env = Env::new();
    let mut daemon = env.daemon();
    let mut app = env.app().await;
    let payload = json!({"session_id": "s", "cwd": "/w", "hook_event_name": "Stop"});
    let out = hook(&env, &["Stop"], payload.to_string().as_bytes());
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
    assert!(out.stderr.is_empty());
    let (_, Control::Agent(event)) = app.control().await else {
        panic!("expected an agent event")
    };
    assert_eq!(event.kind, EventKind::TurnFinished);
    assert_eq!(event.terminal_id.as_deref(), Some("9"));
    assert_eq!(event.raw, payload);
    drop(app);
    assert!(daemon.wait_exit().success());
}

#[test]
fn hook_without_a_service_exits_zero_quickly() {
    let env = Env::new();
    let start = Instant::now();
    let out = hook(&env, &["SessionStart"], b"{}");
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn hook_gives_up_on_a_service_that_does_not_read() {
    let env = Env::new();
    std::fs::create_dir(env.path("run/hive")).unwrap();
    // Accepts connections (backlog) but never reads: a large write blocks.
    let _listener = std::os::unix::net::UnixListener::bind(env.socket()).unwrap();
    let big = format!("\"{}\"", "x".repeat(hive::hook::MAX_INPUT - 2));
    let start = Instant::now();
    let out = hook(&env, &["PostToolUse"], big.as_bytes());
    assert!(out.status.success());
    let elapsed = start.elapsed();
    assert!(
        elapsed >= Duration::from_millis(200) && elapsed < Duration::from_secs(2),
        "{elapsed:?}"
    );
}

#[test]
fn record_appends_the_raw_call() {
    let env = Env::new();
    let file = env.path("home/hooks.jsonl");
    let file_arg = file.to_string_lossy().into_owned();
    let out = hook(
        &env,
        &["Notification", "--record", &file_arg],
        br#"{"notification_type": "idle_prompt"}"#,
    );
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
    let line: Value =
        serde_json::from_str(std::fs::read_to_string(&file).unwrap().trim_end()).unwrap();
    assert_eq!(line["event"], "Notification");
    assert_eq!(line["terminal_id"], "9");
    assert_eq!(line["payload"], json!({"notification_type": "idle_prompt"}));
}

#[test]
fn record_failure_is_reported_on_stderr_but_still_exits_zero() {
    let env = Env::new();
    let dir = env.path("home").to_string_lossy().into_owned();
    let out = hook(&env, &["Stop", "--record", &dir], b"{}");
    assert!(out.status.success());
    assert!(out.stdout.is_empty());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.starts_with(&format!("hive: cannot record the hook call in {dir}: ")),
        "{stderr}"
    );
}

fn badge(env: &Env, terminal: Option<&str>, args: &[&str]) -> Output {
    let mut cmd = env.hive();
    cmd.arg("badge").args(args);
    if let Some(id) = terminal {
        cmd.env("HIVE_TERMINAL_ID", id);
    }
    run(cmd, b"")
}

#[tokio::test]
async fn badge_reaches_the_app_on_its_terminal_cleaned_and_cut() {
    let env = Env::new();
    let mut daemon = env.daemon();
    let mut app = env.app().await;
    app.open_terminal(1, &env.path("home")).await;
    // A hook connection that closes after its hello, or sends garbage, forwards nothing.
    drop(env.connect(Role::Hook).await);
    let mut garbage = env.connect(Role::Hook).await;
    garbage.send(1, Control::TerminalOpened).await;
    drop(garbage);
    // Not an open terminal: dropped.
    assert!(badge(&env, Some("9"), &["ignored"]).status.success());
    let out = badge(&env, Some("1"), &["  fixing\u{1b}", "tests  "]);
    assert!(out.status.success(), "{out:?}");
    assert!(out.stdout.is_empty() && out.stderr.is_empty());
    let text = "fixing tests".to_owned();
    assert_eq!(app.control().await, (1, Control::Badge { text }));
    let long = "x".repeat(60);
    assert!(badge(&env, Some("1"), &[&long]).status.success());
    let text = format!("{}…", "x".repeat(39));
    assert_eq!(app.control().await, (1, Control::Badge { text }));
    assert!(badge(&env, Some("1"), &["--clear"]).status.success());
    let text = String::new();
    assert_eq!(app.control().await, (1, Control::Badge { text }));
    drop(app);
    assert!(daemon.wait_exit().success());
}

#[test]
fn badge_outside_a_hive_terminal_is_a_usage_error() {
    let env = Env::new();
    for terminal in [None, Some("0"), Some("x")] {
        let out = badge(&env, terminal, &["x"]);
        assert_eq!(out.status.code(), Some(2), "{terminal:?}");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(stderr.contains("HIVE_TERMINAL_ID"), "{stderr}");
    }
    assert_eq!(badge(&env, Some("1"), &[]).status.code(), Some(2));
    let both = badge(&env, Some("1"), &["x", "--clear"]);
    assert_eq!(both.status.code(), Some(2));
}

#[test]
fn badge_without_a_service_fails() {
    let env = Env::new();
    let out = badge(&env, Some("1"), &["x"]);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.starts_with("hive: cannot reach the Hive service: "),
        "{stderr}"
    );
}
