use crate::common;

use std::os::unix::fs::PermissionsExt;

use common::{Env, stop};
use hive_protocol::{AgentEvent, Control, EventKind, PROTOCOL_VERSION, Role, Settings};
use serde_json::json;

fn mode(path: std::path::PathBuf) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[tokio::test]
async fn socket_and_lockfile_are_private() {
    let env = Env::new();
    let daemon = env.daemon();
    assert_eq!(mode(env.path("run/hive")), 0o700);
    assert_eq!(mode(env.socket()), 0o600);
    assert_eq!(mode(env.path("run/hive/hive.lock")), 0o600);
    stop(daemon);
}

#[tokio::test]
async fn only_one_daemon_runs_at_a_time() {
    let env = Env::new();
    let daemon = env.daemon();
    let second = env.hive().arg("daemon").output().unwrap();
    assert!(!second.status.success());
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(
        stderr.ends_with("; is another hive daemon running?\n"),
        "{stderr}"
    );
    stop(daemon);
}

#[tokio::test]
async fn stale_socket_file_is_replaced() {
    let env = Env::new();
    std::fs::DirBuilder::new()
        .recursive(true)
        .create(env.path("run/hive"))
        .unwrap();
    std::fs::set_permissions(env.path("run/hive"), std::fs::Permissions::from_mode(0o700)).unwrap();
    std::fs::write(env.socket(), "stale").unwrap();
    let mut daemon = env.daemon();
    drop(env.connect(Role::App).await);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn insecure_runtime_dir_is_refused() {
    let env = Env::new();
    std::fs::create_dir(env.path("run/hive")).unwrap();
    std::fs::set_permissions(env.path("run/hive"), std::fs::Permissions::from_mode(0o777)).unwrap();
    let out = env.hive().arg("daemon").output().unwrap();
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("insecure runtime directory"));
}

#[tokio::test]
async fn binary_version_mismatch_is_refused() {
    let env = Env::new();
    let mut daemon = env.daemon();
    let mut conn = env.raw().await;
    conn.send(0, Control::hello(Role::App, "0.0.0-other")).await;
    let expected = Control::VersionMismatch {
        protocol: PROTOCOL_VERSION,
        version: hive::VERSION.into(),
    };
    assert_eq!(conn.control().await, (0, expected));
    assert_eq!(conn.next().await, None);
    // A refused client is not the app: the daemon keeps waiting for one.
    drop(env.connect(Role::App).await);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn protocol_version_mismatch_is_refused() {
    let env = Env::new();
    let daemon = env.daemon();
    let mut conn = env.raw().await;
    let hello = Control::Hello {
        protocol: PROTOCOL_VERSION + 1,
        version: hive::VERSION.into(),
        role: Role::App,
    };
    conn.send(0, hello).await;
    assert!(matches!(
        conn.control().await,
        (0, Control::VersionMismatch { .. })
    ));
    assert_eq!(conn.next().await, None);
    stop(daemon);
}

#[tokio::test]
async fn first_message_must_be_hello() {
    let env = Env::new();
    let daemon = env.daemon();
    let mut conn = env.raw().await;
    conn.send(0, Control::CloseTerminal).await;
    assert_eq!(
        conn.control().await,
        (
            0,
            Control::Error {
                message: "expected a hello message".into()
            }
        )
    );
    assert_eq!(conn.next().await, None);
    stop(daemon);
}

#[tokio::test]
async fn hook_events_are_translated_and_sent_to_the_app() {
    let env = Env::new();
    let mut daemon = env.daemon();
    let mut app = env.app().await;
    let mut hook = env.connect(Role::Hook).await;
    let payload = json!({"session_id": "s1", "cwd": "/w"});
    hook.send(
        0,
        Control::Hook {
            event: "Stop".into(),
            terminal_id: Some("4".into()),
            payload: payload.clone(),
        },
    )
    .await;
    let expected = AgentEvent {
        provider: "claude-code".into(),
        terminal_id: Some("4".into()),
        session_id: Some("s1".into()),
        subagent: None,
        cwd: Some("/w".into()),
        kind: EventKind::TurnFinished,
        activity: None,
        raw: payload,
    };
    assert_eq!(app.control().await, (0, Control::Agent(expected)));
    drop(app);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn a_hook_connection_is_closed_after_one_message() {
    let env = Env::new();
    let mut daemon = env.daemon();
    let mut app = env.app().await;
    // Not a hook event: nothing is forwarded, the connection is closed.
    let mut hook = env.connect(Role::Hook).await;
    hook.send(0, Control::CloseTerminal).await;
    assert_eq!(hook.next().await, None);
    // A hook event: forwarded, then the connection is closed.
    let mut hook = env.connect(Role::Hook).await;
    let end = Control::Hook {
        event: "SessionEnd".into(),
        terminal_id: None,
        payload: json!({}),
    };
    hook.send(0, end).await;
    assert_eq!(hook.next().await, None);
    let (_, message) = app.control().await;
    assert!(
        matches!(&message, Control::Agent(event) if event.kind == EventKind::SessionEnded { reason: None }),
        "{message:?}"
    );
    drop(app);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn a_second_app_is_rejected() {
    let env = Env::new();
    let mut daemon = env.daemon();
    // `Welcome` ends the handshake before the daemon takes the app's place, so wait for an
    // answer from the app connection itself: then the second one is surely the second.
    let app = env.app().await;
    let mut second = env.handshake(Role::App).await;
    assert_eq!(
        second.control().await,
        (
            0,
            Control::Error {
                message: "another app is already connected".into()
            }
        )
    );
    assert_eq!(second.next().await, None);
    assert_eq!(
        daemon.0.try_wait().unwrap(),
        None,
        "rejecting a second app must not stop the daemon"
    );
    drop(app);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn daemon_exits_and_removes_its_socket_when_the_app_disconnects() {
    let env = Env::new();
    let mut daemon = env.daemon();
    drop(env.connect(Role::App).await);
    assert!(daemon.wait_exit().success());
    assert!(!env.socket().exists());
}

#[tokio::test]
async fn daemon_exits_cleanly_on_sigterm() {
    let env = Env::new();
    let daemon = env.daemon();
    stop(daemon);
    assert!(!env.socket().exists());
}

#[tokio::test]
async fn daemon_installs_the_claude_wrapper_and_hooks_settings() {
    let env = Env::new();
    let daemon = env.daemon();
    let wrapper = env.path("data/hive/bin/claude");
    assert_eq!(mode(wrapper.clone()), 0o755);
    let script = std::fs::read_to_string(wrapper).unwrap();
    let settings_path = env.path("data/hive/hive-hooks.json");
    assert!(
        script.contains(&format!("hive_settings='{}'", settings_path.display())),
        "{script}"
    );
    let settings: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(settings_path).unwrap()).unwrap();
    let hive = std::fs::canonicalize(env!("CARGO_BIN_EXE_hive")).unwrap();
    assert_eq!(
        settings["hooks"]["Stop"][0]["hooks"][0]["command"],
        json!(hive.to_str().unwrap())
    );
    stop(daemon);
}

#[tokio::test]
async fn daemon_fails_when_the_wrapper_cannot_be_installed() {
    let env = Env::new();
    // A file where the data directory should be.
    std::fs::write(env.path("data/hive"), "").unwrap();
    let out = env.hive().arg("daemon").output().unwrap();
    assert!(!out.status.success());
    assert!(!env.socket().exists());
}

#[tokio::test]
async fn settings_are_read_checked_and_saved_by_the_service() {
    let env = Env::new();
    std::fs::create_dir_all(env.path("config/hive")).unwrap();
    let file = env.path("config/hive/settings.json");
    std::fs::write(&file, "{").unwrap();
    let mut daemon = env.daemon();
    // An invalid file: the defaults, then why, and the file is left alone.
    let mut app = env.handshake(Role::App).await;
    let defaults = Control::Settings {
        settings: Settings::default(),
    };
    assert_eq!(app.control().await, (0, defaults));
    let (_, warning) = app.control().await;
    assert!(
        matches!(&warning, Control::SettingsFailed { message } if message.starts_with("Ignoring ")),
        "{warning:?}"
    );
    let mut settings = Settings::default();
    settings.agents.silence_secs = 61;
    let set = |settings| Control::SetSettings { settings };
    app.send(0, set(settings.clone())).await;
    let refused = Control::SettingsFailed {
        message: "agents.silence_secs must be between 2 and 60 (got 61)".into(),
    };
    assert_eq!(app.control().await, (0, refused));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "{");
    // Valid settings are saved, privately, and answered.
    settings.agents.silence_secs = 2;
    app.send(0, set(settings.clone())).await;
    let saved = Control::Settings { settings };
    assert_eq!(app.control().await, (0, saved.clone()));
    assert_eq!(mode(file), 0o600);
    app.send(0, Control::GetSettings).await;
    assert_eq!(app.control().await, (0, saved));
    drop(app);
    assert!(daemon.wait_exit().success());
}
