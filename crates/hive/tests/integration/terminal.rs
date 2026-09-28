use std::time::{Duration, Instant};

use hive_protocol::{Control, Role};

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

pub async fn printed_pid(app: &mut crate::common::Conn, channel: u32) -> i32 {
    let mut seen = String::new();
    loop {
        seen.push_str(&app.output_until(channel, "\n").await);
        if let Some(pid) = pid_in(&seen) {
            return pid;
        }
    }
}

/// PTYs the process `pid` has open.
#[cfg(target_os = "linux")]
fn ptys_open(pid: u32) -> usize {
    std::fs::read_dir(format!("/proc/{pid}/fd"))
        .unwrap()
        .filter_map(|fd| std::fs::read_link(fd.ok()?.path()).ok())
        .filter(|target| target.ends_with("ptmx"))
        .count()
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
    #[cfg(target_os = "linux")]
    assert_eq!(ptys_open(daemon.0.id()), 1);
    app.input(2, "exit 3\r").await;
    assert_eq!(
        app.control().await,
        (2, Control::TerminalExited { code: Some(3) })
    );
    // Its PTY is closed: nothing is left to write to it.
    #[cfg(target_os = "linux")]
    wait_until(|| ptys_open(daemon.0.id()) == 0);
    // The channel is free again.
    app.open_terminal(2, &env.path("home")).await;
    drop(app);
    assert!(daemon.wait_exit().success());
}

/// Collects `channel`'s output until its `terminal_exited`; also returns how long after the
/// first `pid=<digits>` in the output the exit came.
async fn output_until_exit(app: &mut crate::common::Conn, channel: u32) -> (String, Duration) {
    let mut seen = String::new();
    let mut pid_at = None;
    loop {
        let frame = app.next().await.expect("connection closed");
        if frame.channel != channel {
            continue;
        }
        if frame.kind == hive_protocol::FrameType::Terminal {
            seen.push_str(&String::from_utf8_lossy(&frame.payload));
            if pid_at.is_none() && pid_in(&seen).is_some() {
                pid_at = Some(Instant::now());
            }
        } else if let Ok(Control::TerminalExited { .. }) = frame.to_control() {
            let pid_at = pid_at.expect("no pid in the output");
            return (seen, pid_at.elapsed());
        }
    }
}

#[tokio::test]
async fn shell_exit_is_reported_while_a_disowned_job_holds_the_pty() {
    let env = Env::new();
    let mut daemon = env.daemon();
    let mut app = env.connect(Role::App).await;
    app.open_terminal(1, &env.path("home")).await;
    app.input(1, "sleep 30 &; disown; echo \"pid=$last_pid\"; exit\r")
        .await;
    let (output, after) = output_until_exit(&mut app, 1).await;
    assert!(after < Duration::from_secs(1), "{after:?}");
    // The output printed right before `exit` was delivered, and the job ended with the tab.
    let pid = pid_in(&output).unwrap();
    wait_until(|| gone(pid));
    // Nothing of it is left: the channel is free again.
    app.open_terminal(1, &env.path("home")).await;
    drop(app);
    assert!(daemon.wait_exit().success());
}

/// A process in its own session is not ended with the tab, yet the exit is still reported.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn shell_exit_is_reported_while_another_session_holds_the_pty() {
    let env = Env::new();
    let mut daemon = env.daemon();
    let mut app = env.connect(Role::App).await;
    app.open_terminal(1, &env.path("home")).await;
    app.input(
        1,
        "setsid sh -c 'echo pid=$$; exec sleep 30' &; sleep 0.2; exit\r",
    )
    .await;
    let (_, after) = output_until_exit(&mut app, 1).await;
    // `sleep 0.2` runs after the pid is printed, then the drain.
    assert!(after < Duration::from_secs(1), "{after:?}");
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

/// Bytes of terminal `channel`'s output read until at least `enough` came, or none for 1 s.
async fn read_output(app: &mut crate::common::Conn, channel: u32, enough: usize) -> usize {
    let mut bytes = 0;
    while bytes < enough {
        let quiet = Duration::from_secs(1);
        let Ok(frame) = tokio::time::timeout(quiet, app.next()).await else {
            break;
        };
        let frame = frame.expect("connection closed");
        if frame.kind == hive_protocol::FrameType::Terminal && frame.channel == channel {
            bytes += frame.payload.len();
        }
    }
    bytes
}

/// Resident memory of the process `pid`, in KiB.
fn rss_kib(pid: u32) -> usize {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
}

#[tokio::test]
async fn a_flooded_terminal_waits_for_the_app_to_acknowledge_its_output() {
    use hive::terminal::HIGH_WATER;
    let env = Env::new();
    let mut daemon = env.daemon();
    let mut app = env.connect(Role::App).await;
    app.open_terminal(1, &env.path("home")).await;
    app.input(1, "yes\r").await;
    let mut unacked = read_output(&mut app, 1, 64 * 1024).await;
    // The app stops reading and acknowledging: the service does not keep what `yes` prints.
    let before = rss_kib(daemon.0.id());
    tokio::time::sleep(Duration::from_secs(2)).await;
    let grown = rss_kib(daemon.0.id()).saturating_sub(before);
    assert!(grown < 16 * 1024, "the service grew by {grown} KiB");
    // It sent no more than the high water mark and one read.
    let most = HIGH_WATER + 64 * 1024;
    unacked += read_output(&mut app, 1, usize::MAX).await;
    assert!((HIGH_WATER + 1..=most).contains(&unacked), "{unacked}");
    // Acknowledged, the output flows again, until the app is behind again.
    let bytes = u32::try_from(unacked).unwrap();
    app.send(1, Control::Ack { bytes }).await;
    let more = read_output(&mut app, 1, usize::MAX).await;
    assert!((HIGH_WATER + 1..=most).contains(&more), "{more}");
    drop(app);
    assert!(daemon.wait_exit().success());
}

/// The next frame, read as the app does: terminal 1's output is acknowledged once written.
async fn acked(app: &mut crate::common::Conn) -> hive_protocol::Frame {
    let frame = app.next().await.expect("connection closed");
    if frame.kind == hive_protocol::FrameType::Terminal && frame.channel == 1 {
        let bytes = u32::try_from(frame.payload.len()).unwrap();
        app.send(1, Control::Ack { bytes }).await;
    }
    frame
}

#[tokio::test]
async fn a_terminal_echoes_within_the_load_test_bounds_while_another_floods() {
    let env = Env::new();
    let mut daemon = env.daemon();
    let mut app = env.connect(Role::App).await;
    app.open_terminal(1, &env.path("home")).await;
    app.open_terminal(2, &env.path("home")).await;
    app.input(2, "echo ok-(math 1 + 1)\r").await;
    app.output_until(2, "ok-2").await;
    app.input(1, "yes\r").await;
    let mut flooded = 0;
    while flooded < 1024 * 1024 {
        let frame = acked(&mut app).await;
        if frame.channel == 1 {
            flooded += frame.payload.len();
        }
    }
    // Typed at 10 keys/s as in the load test (1.11): each key's echo is timed.
    let mut latencies = Vec::new();
    for key in "abcdefghijklmnopqrstuvwxyzabcdefghijklmnopqrstuvwxyz".chars() {
        let typed = Instant::now();
        app.input(2, &key.to_string()).await;
        loop {
            let frame = acked(&mut app).await;
            if frame.kind == hive_protocol::FrameType::Terminal && frame.channel == 2 {
                break;
            }
        }
        latencies.push(typed.elapsed());
        // The rest of fish's redraw, until the next key.
        let pause = typed + Duration::from_millis(100);
        while tokio::time::timeout_at(pause.into(), acked(&mut app))
            .await
            .is_ok()
        {}
    }
    latencies.sort();
    let at = |percent: usize| latencies[latencies.len() * percent / 100];
    assert!(at(95) < Duration::from_millis(50), "{latencies:?}");
    assert!(at(99) < Duration::from_millis(100), "{latencies:?}");
    drop(app);
    assert!(daemon.wait_exit().success());
}
