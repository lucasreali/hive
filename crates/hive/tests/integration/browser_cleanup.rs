//! An install that had the agent's browser, removed in 26.11, ends clean when the new service
//! starts, and what an older `claude` still asks for harms nothing.

use std::os::unix::fs::PermissionsExt as _;
use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};
use std::process::{Command, Stdio};

use hive_protocol::{AgentState, Control, Role};
use serde_json::{Value, json};

use crate::agents::hook;
use crate::common::{Env, stop, wait_until};
use crate::worktree::Repo;

#[tokio::test(flavor = "multi_thread")]
async fn the_first_start_leaves_nothing_of_the_agents_browser() {
    let env = Env::new();
    let config = env.path("data/hive/bin/hive-browser.json");
    std::fs::create_dir_all(config.parent().unwrap()).unwrap();
    std::fs::write(&config, r#"{"mcpServers":{"hive-browser":{}}}"#).unwrap();
    // The browser a killed service left, and its temporary profile.
    let runtime = env.path("run/hive");
    std::fs::create_dir(&runtime).unwrap();
    std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700)).unwrap();
    let profiles = runtime.join("browsers");
    let profile = profiles.join("1");
    std::fs::create_dir_all(&profile).unwrap();
    let mut browser = Command::new("sh")
        .args(["-c", "sleep 30; :", "browser"])
        .arg(format!("--user-data-dir={}", profile.display()))
        .process_group(0)
        .stdin(Stdio::null())
        .spawn()
        .unwrap();
    let group = browser.id().to_string();
    std::fs::write(profile.join("hive-browser-group"), group).unwrap();
    // The old setting, on with a path: not read, and no error.
    let settings = env.path("data/hive/settings.json");
    let old = json!({
        "agents": {"silence_secs": 7},
        "browser": {"enabled": true, "path": "/opt/chrome", "max": 3},
    });
    std::fs::write(&settings, old.to_string()).unwrap();
    // `sh` is running in place of the fork once its command line reads as such.
    let shows_flag = || {
        let out = Command::new("ps")
            .args(["-ww", "-o", "command=", "-p", &browser.id().to_string()])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).contains("--user-data-dir=")
    };
    wait_until(shows_flag);

    let daemon = env.daemon();

    // The socket is bound first; nothing is served before the cleanup is done.
    let mut app = env.handshake(Role::App).await;
    assert!(!config.exists());
    assert!(!profiles.exists());
    let mut status = None;
    wait_until(|| {
        status = browser.try_wait().unwrap();
        status.is_some()
    });
    assert_eq!(status.unwrap().signal(), Some(9));
    // The wrapper no longer knows of it.
    let wrapper = std::fs::read_to_string(env.path("data/hive/bin/claude")).unwrap();
    assert!(!wrapper.contains("mcp-config"), "{wrapper}");

    let Control::Settings { settings: read } = app.control().await.1 else {
        panic!("no settings first");
    };
    assert_eq!(read.agents.silence_secs, 7);
    // The app is told nothing about it, and the next save leaves it out of the file.
    let sent = serde_json::to_value(&read).unwrap();
    assert_eq!(sent.get("browser"), None);
    app.send(0, Control::SetSettings { settings: read }).await;
    assert!(matches!(app.control().await.1, Control::Settings { .. }));
    let file: Value = serde_json::from_slice(&std::fs::read(&settings).unwrap()).unwrap();
    assert_eq!(file["agents"]["silence_secs"], 7);
    assert_eq!(file.get("browser"), None);
    drop(app);
    stop(daemon);
}

#[tokio::test(flavor = "multi_thread")]
async fn an_older_claudes_ask_for_the_mcp_server_fails_and_harms_nothing() {
    let repo = Repo::new();
    let root = repo.root.display().to_string();
    let daemon = repo.env.daemon();
    let mut app = repo.env.connect(Role::App).await;
    app.send(0, Control::AddProject { path: root.clone() })
        .await;
    assert!(matches!(
        app.control().await,
        (0, Control::ProjectAdded { .. })
    ));
    app.open_terminal(1, &repo.root).await;
    let start = json!({"session_id": "s", "cwd": root});
    hook(&repo, &mut app, "1", "SessionStart", start).await;

    // What Claude Code runs from the config it got before the update.
    let out = repo.env.hive().arg("browser-mcp").output().unwrap();

    // An unknown command: nothing on the output, which the MCP client would read.
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(out.stdout, b"");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("browser-mcp"), "{stderr}");
    // The service still serves the session, and the old server's tools are tools like others.
    let call = json!({
        "session_id": "s",
        "cwd": root,
        "tool_name": "mcp__hive-browser__browser_click",
    });
    let seen = hook(&repo, &mut app, "1", "PreToolUse", call).await;
    let working = |control: &Control| {
        matches!(
            control,
            Control::AgentState {
                state: AgentState::Working,
                ..
            }
        )
    };
    assert!(seen.iter().any(|(_, c)| working(c)), "{seen:?}");
    drop(app);
    stop(daemon);
}
