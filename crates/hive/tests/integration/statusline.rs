use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

use hive_protocol::{Account, Control, SessionWindow, Settings, UsageWindow};
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
    let shown = Control::SessionUsage {
        usage,
        week: None,
        limit_at: None,
        week_limit_at: None,
    };
    (0, shown)
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
                week: None,
                limit_at: None,
                week_limit_at: None,
            }
        )
    );

    // Another account's window is kept, not shown, until it is the current space's account
    // (22.5).
    let work = env.path("work");
    user_statusline(&work, "printf work");
    let out = statusline(&env, &hive, Some(&work), &input(7.0, now + 3600));
    assert_eq!(out.stdout, b"work");
    let config_dir = work.to_string_lossy().into_owned();
    let mut settings = Settings::default();
    settings.claude.accounts.push(Account {
        name: "Work".into(),
        config_dir: config_dir.clone(),
        hive_folder: false,
    });
    let add = Control::SetSettings {
        settings: settings.clone(),
    };
    app.send(0, add).await;
    let answer = (
        0,
        Control::Settings {
            settings: settings.clone(),
            problems: Vec::new(),
        },
    );
    assert_eq!(app.control().await, answer);
    let select = Control::UpdateSpace {
        id: "default".into(),
        name: "Default".into(),
        env: hive_protocol::SpaceEnv {
            claude_account: Some(config_dir),
            ..Default::default()
        },
    };
    app.send(0, select).await;
    assert_eq!(app.control().await, usage(7, now + 3600));
    // Kept in a file (21.8): a new service shows it right away, while it is before its reset.
    drop(app);
    assert!(daemon.wait_exit().success());
    // The space's account is kept in the spaces.
    let mut daemon = env.daemon();
    let mut app = env.handshake(hive_protocol::Role::App).await;
    assert_eq!(
        app.control().await,
        (
            0,
            Control::Settings {
                settings,
                problems: Vec::new()
            }
        )
    );
    assert_eq!(app.control().await, usage(7, now + 3600));
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
        limit_at: None,
        week_limit_at: None,
    };
    assert_eq!(app.control().await, (0, week));
    // 80% of a window alerts once (17.23), after its usage; 85% this late in the window lasts
    // until its reset: no pace.
    let out = statusline(&env, &hive, Some(&work), &input(85.0, now + 600));
    assert!(out.status.success(), "{out:?}");
    assert_eq!(app.control().await, usage(85, now + 600));
    let alert = Control::UsageAlert {
        window: UsageWindow::Session,
        threshold: 80,
        resets_at: now + 600,
    };
    assert_eq!(app.control().await, (0, alert));
    let out = statusline(&env, &hive, Some(&work), &input(86.0, now + 600));
    assert!(out.status.success(), "{out:?}");
    assert_eq!(app.control().await, usage(86, now + 600));
    drop(app);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn either_window_alone_reaches_the_app() {
    let env = Env::new();
    let mut daemon = env.daemon();
    let mut app = env.app().await;
    let hive = hive_statusline(&env);
    let now = hive::hook::now_ms() / 1000;
    let window = |used_percentage, resets_at| SessionWindow {
        used_percentage,
        resets_at,
    };
    // The 7-day window without a 5-hour one (28.1): shown alone, and it alerts.
    let week = json!({ "rate_limits": {
        "seven_day": { "used_percentage": 81, "resets_at": now + 86400 },
    } });
    let out = statusline(&env, &hive, None, &week.to_string());
    assert!(out.status.success(), "{out:?}");
    let shown = Control::SessionUsage {
        usage: None,
        week: Some(window(81, now + 86400)),
        limit_at: None,
        week_limit_at: None,
    };
    assert_eq!(app.control().await, (0, shown));
    let alert = Control::UsageAlert {
        window: UsageWindow::Week,
        threshold: 80,
        resets_at: now + 86400,
    };
    assert_eq!(app.control().await, (0, alert));
    // The 5-hour one alone: the week disappears.
    let out = statusline(&env, &hive, None, &input(12.0, now + 3600));
    assert!(out.status.success(), "{out:?}");
    assert_eq!(app.control().await, usage(12, now + 3600));
    drop(app);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn a_window_that_cannot_be_kept_is_still_shown() {
    let env = Env::new();
    // Where the windows are kept is a folder.
    std::fs::create_dir_all(env.path("data/hive/usage.json")).unwrap();
    let mut daemon = env.daemon();
    let mut app = env.app().await;
    let hive = hive_statusline(&env);
    let now = hive::hook::now_ms() / 1000;
    let out = statusline(&env, &hive, None, &input(12.0, now + 3600));
    assert!(out.status.success(), "{out:?}");
    assert_eq!(app.control().await, usage(12, now + 3600));
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

#[tokio::test]
async fn a_service_started_inside_a_hooked_claude_gives_its_terminals_none_of_its_guards() {
    let env = Env::new();
    // Started from a `claude` the wrapper ran (`HIVE_WRAPPED`) and from the user's statusline
    // `hive statusline` ran (`HIVE_STATUSLINE`): its terminals' `claude` would skip Hive's
    // settings, and their `hive statusline` would report nothing.
    let mut hive = env.hive();
    hive.env("HIVE_WRAPPED", "1").env("HIVE_STATUSLINE", "1");
    let mut daemon = env.daemon_with(&mut hive);
    let mut app = env.app().await;
    app.open_terminal(1, &env.path("home")).await;
    app.input(1, "echo \"[$HIVE_WRAPPED|$HIVE_STATUSLINE]\"\r")
        .await;
    app.output_until(1, "[|]").await;
    drop(app);
    assert!(daemon.wait_exit().success());
}

/// Claude Code's statusline input as its docs show it today (2.1.x: `rate_limits` with
/// `five_hour`, `seven_day` and a gateway's `spend_limit`, among many other fields).
fn current_input(now: u64) -> Value {
    json!({
        "cwd": "/current/working/directory",
        "session_id": "abc123",
        "session_name": "my-session",
        "transcript_path": "/path/to/transcript.jsonl",
        "model": { "id": "claude-opus-5-5", "display_name": "Opus" },
        "workspace": {
            "current_dir": "/current/working/directory",
            "project_dir": "/original/project/directory",
            "added_dirs": [],
            "git_worktree": "feature-xyz",
        },
        "version": "2.1.332",
        "output_style": { "name": "default" },
        "cost": { "total_cost_usd": 0.01234, "total_duration_ms": 45000 },
        "context_window": {
            "total_input_tokens": 15500,
            "context_window_size": 200000,
            "used_percentage": 8,
            "current_usage": { "input_tokens": 8500, "output_tokens": 1200 },
        },
        "exceeds_200k_tokens": false,
        "prompt_cache": { "warm": true, "ttl": "1h", "expires_at": now + 600, "hit_ratio": 0.91 },
        "fast_mode": false,
        "effort": { "level": "high" },
        "thinking": { "enabled": true },
        "rate_limits": {
            "five_hour": { "used_percentage": 23.5, "resets_at": now + 3600 },
            "seven_day": { "used_percentage": 41.2, "resets_at": now + 86400 },
            "spend_limit": {
                "used_percentage": 62.8, "resets_at": now + 7 * 86400,
                "used_usd": 314.12, "limit_usd": 500, "period": "monthly",
            },
        },
        "vim": { "mode": "NORMAL" },
        "worktree": { "name": "my-feature", "path": "/p", "branch": "b" },
    })
}

#[tokio::test]
async fn a_terminal_of_another_accounts_space_shows_that_accounts_usage() {
    use std::os::unix::fs::PermissionsExt;
    let repo = crate::worktree::Repo::new();
    let env = &repo.env;
    let mut daemon = env.daemon();
    let mut app = env.app().await;
    let add = Control::AddProject {
        path: repo.root.display().to_string(),
    };
    app.send(0, add).await;
    let added = app.control().await.1;
    assert!(matches!(added, Control::ProjectAdded { .. }), "{added:?}");
    let now = hive::hook::now_ms() / 1000;
    // A Claude Code stand-in (never the real one): it runs the `statusLine` of the settings
    // its `--settings` names, which outrank the user's (Claude Code's settings precedence),
    // through `sh -c` with the current input on stdin, as Claude Code does.
    let fake = env.path("fake");
    std::fs::create_dir_all(&fake).unwrap();
    let input = env.path("input.json");
    std::fs::write(&input, current_input(now).to_string()).unwrap();
    let script = format!(
        "#!/bin/sh\nwhile [ $# -gt 0 ]; do\n  [ \"$1\" = --settings ] && settings=$2\n  shift\n\
         done\ncommand=$(sed -n 's/^ *\"command\": \"\\(.* statusline\\)\",*$/\\1/p' \"$settings\")\n\
         sh -c \"$command\" < '{}'\necho \"<status $?>\"\n",
        input.display()
    );
    std::fs::write(fake.join("claude"), script).unwrap();
    std::fs::set_permissions(fake.join("claude"), std::fs::Permissions::from_mode(0o755)).unwrap();

    // An account with its own statusline, the current space's (22.5).
    let work = env.path("data/hive/accounts/work");
    user_statusline(&work, "printf mine");
    let config_dir = work.to_string_lossy().into_owned();
    let mut settings = Settings::default();
    settings.claude.accounts.push(Account {
        name: "Work".into(),
        config_dir: config_dir.clone(),
        // In Hive's folder, as an account Hive made (22.2).
        hive_folder: true,
    });
    let set = Control::SetSettings {
        settings: settings.clone(),
    };
    app.send(0, set).await;
    assert_eq!(
        app.control().await,
        (
            0,
            Control::Settings {
                settings,
                problems: Vec::new()
            }
        )
    );
    let select = Control::UpdateSpace {
        id: "default".into(),
        name: "Default".into(),
        env: hive_protocol::SpaceEnv {
            claude_account: Some(config_dir),
            ..Default::default()
        },
    };
    app.send(0, select).await;

    // `claude` in a terminal of that space's project: the wrapper, then the stand-in.
    app.open_terminal(1, &repo.root).await;
    let wrapper = env.path("data/hive/bin/claude");
    let run = format!(
        "env PATH='{}:/usr/bin:/bin' '{}'\r",
        fake.display(),
        wrapper.display()
    );
    app.input(1, &run).await;
    let window = |used_percentage, resets_at| SessionWindow {
        used_percentage,
        resets_at,
    };
    let shown = Control::SessionUsage {
        usage: Some(window(24, now + 3600)),
        week: Some(window(41, now + 86400)),
        limit_at: None,
        week_limit_at: None,
    };
    // What the user's statusline printed reaches the terminal, and the account's windows the
    // app, in either order.
    let (mut output, mut usage) = (String::new(), None);
    while !output.contains("mine<status 0>") || usage.is_none() {
        let frame = app.next().await.expect("connection closed");
        match frame.to_control() {
            Ok(message @ Control::SessionUsage { usage: Some(_), .. }) => usage = Some(message),
            Ok(_) => {}
            Err(_) => output.push_str(&String::from_utf8_lossy(&frame.payload)),
        }
    }
    assert_eq!(usage, Some(shown));
    drop(app);
    assert!(daemon.wait_exit().success());
}
