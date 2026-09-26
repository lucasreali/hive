use std::time::{Duration, Instant};

use hive_protocol::{ChatAnswer, ChatMode, Control, Role};

use crate::common::{Env, wait_until};

/// First `pid=<digits>` in the output (the echoed command line shows `pid=$...`, not digits).
fn pid_in(output: &str) -> Option<i32> {
    output.match_indices("pid=").find_map(|(at, _)| {
        let digits: String = output[at + 4..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        digits.parse().ok()
    })
}

async fn printed_pid(app: &mut crate::common::Conn, channel: u32) -> i32 {
    let mut seen = String::new();
    loop {
        seen.push_str(&app.output_until(channel, "\n").await);
        if let Some(pid) = pid_in(&seen) {
            return pid;
        }
    }
}

fn gone(pid: i32) -> bool {
    !hive::procs::list(hive::procs::Source::System)
        .iter()
        .any(|p| p.pid == pid)
}

#[tokio::test]
async fn terminal_runs_fish_with_hive_bin_first_on_path_and_its_id() {
    let env = Env::new();
    let mut daemon = env.daemon();
    let mut app = env.connect(Role::App).await;
    app.open_terminal(5, &env.path("home")).await;
    app.input(5, "echo \"id=$HIVE_TERMINAL_ID first=$PATH[1] cwd=$PWD\"\r")
        .await;
    let expected = format!(
        "id=5 first={} cwd={}",
        env.path("data/hive/bin").display(),
        // fish resolves its folder (on macOS the temp dir is under a symlink).
        env.path("home").canonicalize().unwrap().display()
    );
    app.output_until(5, &expected).await;
    drop(app);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn resize_changes_the_pty_size() {
    let env = Env::new();
    let mut daemon = env.daemon();
    let mut app = env.connect(Role::App).await;
    app.open_terminal(1, &env.path("home")).await;
    app.send(
        1,
        Control::Resize {
            cols: 101,
            rows: 33,
        },
    )
    .await;
    app.input(1, "stty size\r").await;
    app.output_until(1, "33 101").await;
    drop(app);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn shell_exit_is_reported_with_its_code() {
    let env = Env::new();
    let mut daemon = env.daemon();
    let mut app = env.connect(Role::App).await;
    app.open_terminal(2, &env.path("home")).await;
    app.input(2, "exit 3\r").await;
    assert_eq!(
        app.control().await,
        (2, Control::TerminalExited { code: Some(3) })
    );
    // The channel is free again.
    app.open_terminal(2, &env.path("home")).await;
    drop(app);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn closing_a_terminal_ends_its_shell() {
    let env = Env::new();
    let mut daemon = env.daemon();
    let mut app = env.connect(Role::App).await;
    app.open_terminal(3, &env.path("home")).await;
    app.send(3, Control::CloseTerminal).await;
    // The code depends on whether fish was killed by SIGHUP or handled it and exited.
    let (channel, message) = app.control().await;
    assert_eq!(channel, 3);
    assert!(
        matches!(message, Control::TerminalExited { .. }),
        "{message:?}"
    );
    // Closing or typing into a terminal that is gone is ignored.
    app.send(3, Control::CloseTerminal).await;
    app.input(3, "ignored\r").await;
    app.open_terminal(4, &env.path("home")).await;
    drop(app);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn invalid_open_requests_are_answered_with_errors() {
    let env = Env::new();
    let mut daemon = env.daemon();
    let mut app = env.connect(Role::App).await;
    let open = |cwd: &str| Control::OpenTerminal {
        cwd: cwd.into(),
        cols: 80,
        rows: 24,
    };
    let home = env.path("home").to_string_lossy().into_owned();

    app.send(0, open(&home)).await;
    assert_eq!(
        app.control().await,
        (
            0,
            Control::Error {
                message: "terminal channels start at 1".into()
            }
        )
    );

    app.open_terminal(7, &env.path("home")).await;
    app.send(7, open(&home)).await;
    assert_eq!(
        app.control().await,
        (
            7,
            Control::Error {
                message: "terminal 7 is already open".into()
            }
        )
    );

    app.send(8, open("/nonexistent/dir")).await;
    let (channel, Control::Error { message }) = app.control().await else {
        panic!("expected an error")
    };
    assert_eq!(channel, 8);
    assert!(
        message.starts_with("cannot start a terminal in /nonexistent/dir: "),
        "{message}"
    );

    app.send(
        9,
        Control::Welcome {
            version: "x".into(),
            distro: None,
        },
    )
    .await;
    assert_eq!(
        app.control().await,
        (
            9,
            Control::Error {
                message: "unexpected message from the app".into()
            }
        )
    );

    let chat = [
        Control::OpenChat {
            cwd: "/r".into(),
            resume: None,
            mode: None,
        },
        Control::ChatSend {
            chat: 10,
            text: "hi".into(),
            images: vec![],
        },
        Control::ChatAnswer {
            chat: 10,
            request: "r".into(),
            answer: ChatAnswer::Allow,
        },
        Control::ChatInterrupt { chat: 10 },
        Control::ChatSetMode {
            chat: 10,
            mode: ChatMode::Plan,
        },
        Control::CloseChat { chat: 10 },
        Control::ConfirmChatFolder {
            chat: 10,
            cwd: "/r".into(),
            accepted: Some(true),
        },
    ];
    for message in chat {
        app.send(10, message).await;
        let unavailable = Control::Error {
            message: "chat not available yet".into(),
        };
        assert_eq!(app.control().await, (10, unavailable));
    }
    drop(app);
    assert!(daemon.wait_exit().success());
}

#[tokio::test]
async fn app_disconnect_ends_every_process_group_of_the_terminal_promptly() {
    let env = Env::new();
    let mut daemon = env.daemon();
    let mut app = env.connect(Role::App).await;
    app.open_terminal(1, &env.path("home")).await;
    // A background job has its own process group; disown detaches it from fish.
    app.input(1, "sleep 30 &; disown; echo \"pid=$last_pid\"\r")
        .await;
    let pid = printed_pid(&mut app, 1).await;
    assert!(!gone(pid));
    drop(app);
    let start = Instant::now();
    assert!(daemon.wait_exit().success());
    // Everything exits on SIGHUP: no waiting for the grace period.
    assert!(
        start.elapsed() < Duration::from_millis(1500),
        "{:?}",
        start.elapsed()
    );
    wait_until(|| gone(pid));
}

#[tokio::test]
async fn processes_get_time_to_handle_sighup() {
    let env = Env::new();
    let mut daemon = env.daemon();
    let mut app = env.connect(Role::App).await;
    app.open_terminal(1, &env.path("home")).await;
    let done = env.path("home/cleaned-up");
    let script = format!(
        "trap \"sleep 0.3; touch {}; exit 0\" HUP; echo pid=$$; while :; do sleep 0.05; done",
        done.display()
    );
    app.input(1, &format!("sh -c '{script}'\r")).await;
    printed_pid(&mut app, 1).await;
    drop(app);
    assert!(daemon.wait_exit().success());
    assert!(
        done.exists(),
        "the HUP handler was killed before it finished"
    );
}

#[tokio::test]
async fn processes_ignoring_sighup_are_killed_after_the_grace_period() {
    let env = Env::new();
    let mut daemon = env.daemon();
    let mut app = env.connect(Role::App).await;
    app.open_terminal(1, &env.path("home")).await;
    app.input(1, "sh -c 'trap \"\" HUP; echo pid=$$; exec sleep 30'\r")
        .await;
    let pid = printed_pid(&mut app, 1).await;
    drop(app);
    assert!(daemon.wait_exit().success());
    wait_until(|| gone(pid));
}

/// On macOS the terminal runs `$SHELL` as a login shell: the user's startup files run and
/// Hive's bin dir still comes first on `PATH`.
#[cfg(target_os = "macos")]
#[tokio::test]
async fn login_shells_run_the_user_files_and_keep_hive_bin_first() {
    for (shell, file) in [("/bin/zsh", ".zshrc"), ("/bin/bash", ".bash_profile")] {
        let env = Env::new();
        let rc = "export PATH=/user/first:$PATH\nexport HIVE_TEST_RC=read\n";
        std::fs::write(env.path("home").join(file), rc).unwrap();
        let child = env
            .hive()
            .env("SHELL", shell)
            .arg("daemon")
            .stdin(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let mut daemon = crate::common::Daemon(child);
        wait_until(|| std::os::unix::net::UnixStream::connect(env.socket()).is_ok());
        let mut app = env.connect(Role::App).await;
        app.open_terminal(1, &env.path("home")).await;
        app.input(1, "echo \"rc=$HIVE_TEST_RC first=${PATH%%:*}\"\r")
            .await;
        let bin = env.path("data/hive/bin");
        app.output_until(1, &format!("rc=read first={}", bin.display()))
            .await;
        drop(app);
        assert!(daemon.wait_exit().success(), "{shell}");
    }
}
