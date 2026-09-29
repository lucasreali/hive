//! The service on native Windows (12.5.2): started by the bridge, over its named pipe, ending
//! with the app connection. Terminals come with 12.5.3, so the Unix tests stay Unix-only.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use hive::paths::Paths;
use hive_protocol::{Control, Frame, FrameCodec, Role, SessionWindow};
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio_util::codec::{FramedRead, FramedWrite};

const TIMEOUT: Duration = Duration::from_secs(20);

/// A throwaway profile: app data, settings, home and Claude folder in a temporary folder.
struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn path(&self, sub: &str) -> PathBuf {
        self.dir.path().join(sub)
    }

    fn vars(&self) -> Vec<(&'static str, PathBuf)> {
        vec![
            ("LOCALAPPDATA", self.path("local")),
            ("APPDATA", self.path("roaming")),
            ("USERPROFILE", self.path("home")),
            ("HOME", self.path("home")),
            ("CLAUDE_CONFIG_DIR", self.path("claude")),
        ]
    }

    /// The service's paths, as `hive` run by [`Env::hive`] finds them.
    fn paths(&self) -> Paths {
        let vars = self.vars();
        hive::windows::paths(|key| {
            let found = vars.iter().find(|(k, _)| *k == key);
            found.map(|(_, v)| v.clone().into_os_string())
        })
    }

    fn hive(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_hive"));
        cmd.envs(self.vars())
            .env_remove("WSL_DISTRO_NAME")
            .kill_on_drop(true);
        for (key, _) in std::env::vars_os() {
            if key.to_string_lossy().starts_with("HIVE_") {
                cmd.env_remove(key);
            }
        }
        cmd
    }
}

/// The app's end of the bridge.
struct App {
    reader: FramedRead<ChildStdout, FrameCodec>,
    writer: FramedWrite<ChildStdin, FrameCodec>,
}

impl App {
    async fn send(&mut self, channel: u32, message: Control) {
        let frame = Frame::control(channel, &message);
        self.writer.send(frame).await.unwrap();
    }

    /// Skips control messages until `want` comes on `channel`.
    async fn wait_for(&mut self, channel: u32, want: Control) {
        let wait = async {
            loop {
                let frame = self.reader.next().await.unwrap().unwrap();
                if frame.channel == channel && frame.to_control().ok() == Some(want.clone()) {
                    return;
                }
            }
        };
        let waited = tokio::time::timeout(TIMEOUT, wait).await;
        assert!(waited.is_ok(), "no {want:?}");
    }
}

fn bridge(env: &Env) -> (Child, App) {
    let mut child = env
        .hive()
        .arg("bridge")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let app = App {
        reader: FramedRead::new(child.stdout.take().unwrap(), FrameCodec),
        writer: FramedWrite::new(child.stdin.take().unwrap(), FrameCodec),
    };
    (child, app)
}

#[tokio::test]
async fn the_service_serves_the_app_over_its_pipe_and_ends_with_it() {
    let env = Env::new();
    let paths = env.paths();
    // No service yet: the bridge starts one, detached.
    let (mut bridge, mut app) = bridge(&env);
    app.send(0, Control::hello(Role::App, hive::VERSION)).await;
    let welcome = Control::Welcome {
        version: hive::VERSION.into(),
        distro: None,
    };
    app.wait_for(0, welcome).await;
    let settings = Control::Settings {
        settings: Default::default(),
    };
    app.wait_for(0, settings).await;
    app.send(0, Control::ListProjects).await;
    app.wait_for(0, Control::Projects { projects: vec![] })
        .await;
    // Terminals are not there yet, and say so.
    let cwd = env.path("home").to_string_lossy().into_owned();
    let open = Control::OpenTerminal {
        cwd: cwd.clone(),
        cols: 80,
        rows: 24,
    };
    app.send(1, open).await;
    let message =
        format!("cannot start a terminal in {cwd}: a terminal is not supported on Windows yet");
    app.wait_for(1, Control::Error { message }).await;

    // A hook connection of this user reaches the app through the pipe.
    let resets_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
    let usage = SessionWindow {
        used_percentage: 42,
        resets_at,
    };
    let mut hook = FramedWrite::new(paths.connect().await.unwrap(), FrameCodec);
    let hello = Control::hello(Role::Hook, hive::VERSION);
    hook.send(Frame::control(0, &hello)).await.unwrap();
    let claude_dir = env.path("claude").to_string_lossy().into_owned();
    let report = Control::StatuslineUsage { claude_dir, usage };
    hook.send(Frame::control(0, &report)).await.unwrap();
    let usage = Some(usage);
    app.wait_for(0, Control::SessionUsage { usage }).await;
    drop(hook);
    // `hive hook` as Claude Code runs it: quiet, and it always succeeds.
    let mut call = env.hive();
    let mut call = call
        .args(["hook", "Stop"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = call.stdin.take().unwrap();
    stdin.write_all(br#"{"session_id":"s"}"#).await.unwrap();
    drop(stdin);
    let out = call.wait_with_output().await.unwrap();
    assert!(out.status.success());
    assert!(out.stdout.is_empty());

    // The app leaves: the bridge ends, then the service.
    drop(app);
    let ended = tokio::time::timeout(TIMEOUT, bridge.wait()).await;
    assert!(ended.unwrap().unwrap().success());
    let started = Instant::now();
    loop {
        let gone = paths.connect().await.is_err();
        let lock = std::fs::File::options().write(true).open(paths.lock());
        if gone && lock.unwrap().try_lock().is_ok() {
            break;
        }
        assert!(started.elapsed() < TIMEOUT, "the service is still running");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
