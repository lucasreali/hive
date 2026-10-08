use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use hive_protocol::{Account, Control, SessionWindow, Settings};
use serde_json::{Value, json};

use crate::common::{Env, wait_until};
use crate::hook::run;

/// Makes `command` the user's statusline in the Claude config folder `claude`.
fn user_statusline(claude: &Path, command: &str) {
    std::fs::create_dir_all(claude).unwrap();
    let settings = json!({ "statusLine": { "type": "command", "command": command } });
    std::fs::write(claude.join("settings.json"), settings.to_string()).unwrap();
}

/// Hive's own statusline command, from the settings the `claude` wrapper injects.
fn hive_statusline(env: &Env) -> String {
    let text = std::fs::read_to_string(env.path("data/hive/hive-hooks.json")).unwrap();
    let settings: Value = serde_json::from_str(&text).unwrap();
    settings["statusLine"]["command"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// Runs `command` through `sh -c`, as Claude Code runs a statusline, in `hive`'s test
/// environment (plus `CLAUDE_CONFIG_DIR` when given).
fn statusline(env: &Env, command: &str, claude: Option<&Path>, input: &str) -> Output {
    let hive = env.hive();
    let mut sh = Command::new("sh");
    sh.arg("-c").arg(command).current_dir(env.dir.path());
    for (key, value) in hive.get_envs() {
        match value {
            Some(value) => sh.env(key, value),
            None => sh.env_remove(key),
        };
    }
    if let Some(claude) = claude {
        sh.env("CLAUDE_CONFIG_DIR", claude);
    }
    run(sh, input.as_bytes())
}

fn input(used: f64, resets_at: u64) -> String {
    json!({ "rate_limits": { "five_hour": { "used_percentage": used, "resets_at": resets_at } } })
        .to_string()
}

fn usage(used_percentage: u8, resets_at: u64) -> (u32, Control) {
    let usage = Some(SessionWindow {
        used_percentage,
        resets_at,
    });
    (0, Control::SessionUsage { usage, week: None })
}

#[tokio::test]
async fn the_session_window_reaches_the_app_and_the_users_statusline_prints_unchanged() {
    let env = Env::new();
    let mut daemon = env.daemon();
    let mut app = env.app().await;
    let hive = hive_statusline(&env);
    user_statusline(&env.path("home/.claude"), "printf 'mine:'; cat; exit 4");
    let now = hive::hook::now_ms() / 1000;

    let soon = input(41.6, now + 3);
    let out = statusline(&env, &hive, None, &soon);
    assert_eq!(out.status.code(), Some(4), "{out:?}");
    assert_eq!(String::from_utf8_lossy(&out.stdout), format!("mine:{soon}"));
    assert!(out.stderr.is_empty(), "{out:?}");
    assert_eq!(app.control().await, usage(42, now + 3));
    // Past its reset it is not shown (the service checks every second).
    assert_eq!(
        app.control().await,
        (
            0,
            Control::SessionUsage {
                usage: None,
                week: None
            }
        )
    );

    // Another account's window is kept, not shown, until it is the selected account (12.2).
    let work = env.path("work");
    user_statusline(&work, "printf work");
    let out = statusline(&env, &hive, Some(&work), &input(7.0, now + 3600));
    assert_eq!(out.stdout, b"work");
    let config_dir = work.to_string_lossy().into_owned();
    let mut settings = Settings::default();
    settings.claude.accounts.push(Account {
        name: "Work".into(),
        config_dir: config_dir.clone(),
    });
    settings.claude.account = Some(config_dir);
    let select = Control::SetSettings {
        settings: settings.clone(),
    };
    app.send(0, select).await;
    let answer = (
        0,
        Control::Settings {
            settings: settings.clone(),
        },
    );
    assert_eq!(app.control().await, answer);
    assert_eq!(app.control().await, usage(7, now + 3600));
    // Kept in memory only: a new service has none until a statusline runs.
    drop(app);
    assert!(daemon.wait_exit().success());
    // The selected account is kept in the settings.
    let mut daemon = env.daemon();
    let mut app = env.handshake(hive_protocol::Role::App).await;
    assert_eq!(app.control().await, (0, Control::Settings { settings }));
    app.send(0, Control::ListProjects).await;
    let projects = Control::Projects { projects: vec![] };
    assert_eq!(app.control().await, (0, projects));
    let out = statusline(&env, &hive, Some(&work), &input(9.0, now + 3600));
    assert!(out.status.success(), "{out:?}");
    assert_eq!(app.control().await, usage(9, now + 3600));
    // The 7-day window goes with it (15.3).
    let window = |used_percentage, resets_at| SessionWindow {
        used_percentage,
        resets_at,
    };
    let both = json!({ "rate_limits": {
        "five_hour": { "used_percentage": 9, "resets_at": now + 3600 },
        "seven_day": { "used_percentage": 40.6, "resets_at": now + 86400 },
    } });
    let out = statusline(&env, &hive, Some(&work), &both.to_string());
    assert!(out.status.success(), "{out:?}");
    let week = Control::SessionUsage {
        usage: Some(window(9, now + 3600)),
        week: Some(window(41, now + 86400)),
    };
    assert_eq!(app.control().await, (0, week));
    drop(app);
    assert!(daemon.wait_exit().success());
}

#[test]
fn hives_own_statusline_as_the_users_prints_nothing_without_a_service() {
    let env = Env::new();
    let hive = format!("'{}' statusline", env!("CARGO_BIN_EXE_hive"));
    user_statusline(&env.path("home/.claude"), &hive);
    let start = Instant::now();
    let out = statusline(&env, &hive, None, &input(1.0, 1));
    assert!(out.status.success(), "{out:?}");
    assert!(out.stdout.is_empty() && out.stderr.is_empty(), "{out:?}");
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );
}

#[test]
fn a_sigterm_ends_the_users_statusline_and_prints_nothing() {
    let env = Env::new();
    user_statusline(&env.path("home/.claude"), "sleep 5; echo late");
    let child = env
        .hive()
        .arg("statusline")
        .current_dir(env.dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Claude Code cancels a run once the user's statusline is running.
    wait_until(|| env.processes().iter().any(|p| p.comm == "sleep"));
    let start = Instant::now();
    let pid = nix::unistd::Pid::from_raw(child.id() as i32);
    nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "{out:?}");
    assert!(out.stdout.is_empty() && out.stderr.is_empty(), "{out:?}");
    assert!(
        start.elapsed() < Duration::from_secs(2),
        "{:?}",
        start.elapsed()
    );
    // Its statusline ended with it.
    wait_until(|| env.processes().is_empty());
}
