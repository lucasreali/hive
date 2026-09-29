//! The service on native Windows (12.5.2, 12.5.3): started by the bridge, over its named
//! pipe, with terminals on a pseudoconsole, ending with the app connection. The Unix tests
//! stay Unix-only (their shells, paths and signals).

use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures_util::{SinkExt, StreamExt};
use hive::paths::Paths;
use hive_protocol::{
    AgentState, Control, Frame, FrameCodec, FrameType, Role, SessionWindow, Settings, TerminalShell,
};
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio_util::codec::{FramedRead, FramedWrite};

/// How long anything may take (PowerShell starts slowly on a busy runner).
const TIMEOUT: Duration = Duration::from_secs(60);

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

/// The app's end of the bridge, with each terminal's output so far.
struct App {
    reader: FramedRead<ChildStdout, FrameCodec>,
    writer: FramedWrite<ChildStdin, FrameCodec>,
    output: HashMap<u32, String>,
}

impl App {
    async fn send(&mut self, channel: u32, message: Control) {
        let frame = Frame::control(channel, &message);
        self.writer.send(frame).await.unwrap();
    }

    /// Types `line` and Enter into terminal `channel`.
    async fn type_line(&mut self, channel: u32, line: &str) {
        let frame = Frame::terminal(channel, format!("{line}\r"));
        self.writer.send(frame).await.unwrap();
    }

    /// Reads frames until `done` holds for a control message on `channel` or for the
    /// terminal's output; returns that message (none for output).
    async fn until(
        &mut self,
        channel: u32,
        what: &str,
        mut done: impl FnMut(Option<&Control>, &str) -> bool,
    ) -> Option<Control> {
        let wait = async {
            loop {
                let frame = self.reader.next().await.unwrap().unwrap();
                let output = self.output.entry(frame.channel).or_default();
                let message = match frame.kind {
                    FrameType::Terminal => {
                        output.push_str(&String::from_utf8_lossy(&frame.payload));
                        None
                    }
                    FrameType::Control => frame.to_control().ok(),
                };
                if frame.channel == channel && done(message.as_ref(), output) {
                    return message;
                }
            }
        };
        let waited = tokio::time::timeout(TIMEOUT, wait).await;
        let output = self.output.get(&channel);
        waited.unwrap_or_else(|_| panic!("no {what} on {channel}; its output: {output:?}"))
    }

    /// Skips messages until `want` comes on `channel`.
    async fn wait_for(&mut self, channel: u32, want: Control) {
        let what = format!("{want:?}");
        self.until(channel, &what, |message, _| message == Some(&want))
            .await;
    }

    /// Waits until terminal `channel` shows `text`.
    async fn shows(&mut self, channel: u32, text: &str) {
        self.until(channel, text, |_, output| output.contains(text))
            .await;
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
        output: HashMap::new(),
    };
    (child, app)
}

/// Opens terminal `channel` in `cwd`, outside every project.
async fn open(app: &mut App, channel: u32, cwd: &str) {
    let open = Control::OpenTerminal {
        cwd: cwd.into(),
        cols: 80,
        rows: 24,
        account: None,
    };
    app.send(channel, open).await;
    app.wait_for(channel, Control::TerminalOpened { worktree: None })
        .await;
}

/// Closes terminal `channel`; its shell exits.
async fn close(app: &mut App, channel: u32) {
    app.send(channel, Control::CloseTerminal).await;
    let exited = |message: Option<&Control>, _: &str| {
        matches!(message, Some(Control::TerminalExited { .. }))
    };
    app.until(channel, "its exit", exited).await;
}

/// The agent `id`'s state.
fn state_of(id: &str, state: AgentState) -> impl FnMut(Option<&Control>, &str) -> bool {
    move |message, _| {
        matches!(message, Some(Control::AgentState { id: agent, state: now, .. })
            if agent == id && *now == state)
    }
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

    // A terminal: PowerShell (7 on the runner) by default, typed into and resized.
    let home = env.path("home");
    std::fs::create_dir_all(&home).unwrap();
    let cwd = home.to_string_lossy().into_owned();
    open(&mut app, 1, &cwd).await;
    app.type_line(1, r#""ed=" + $PSVersionTable.PSEdition"#)
        .await;
    app.shows(1, "ed=Core").await;
    app.send(
        1,
        Control::Resize {
            cols: 132,
            rows: 40,
        },
    )
    .await;
    let window = "$Host.UI.RawUI.WindowSize";
    let size = format!(r#""size=" + {window}.Width + "x" + {window}.Height"#);
    app.type_line(1, &size).await;
    app.shows(1, "size=132x40").await;
    // Hook events from inside the terminal reach the app as its agent's states.
    let hive = env!("CARGO_BIN_EXE_hive");
    let hook = |event: &str| format!(r#"'{{"session_id":"s1"}}' | & '{hive}' hook {event}"#);
    app.type_line(1, &hook("SessionStart")).await;
    let detected = |message: Option<&Control>, _: &str| matches!(message, Some(Control::AgentDetected { id, .. }) if id == "s1");
    app.until(1, "the agent", detected).await;
    app.until(1, "idle", state_of("s1", AgentState::Idle)).await;
    app.type_line(1, &hook("UserPromptSubmit")).await;
    app.until(1, "working", state_of("s1", AgentState::Working))
        .await;
    close(&mut app, 1).await;

    // The shell setting: new terminals run the chosen one.
    let chosen = [
        (TerminalShell::Cmd, "echo cmd=%OS%", "cmd=Windows_NT"),
        (TerminalShell::GitBash, "echo sh=$((6*7))", "sh=42"),
    ];
    for (channel, (shell, line, shown)) in (2..).zip(chosen) {
        let mut settings = Settings::default();
        settings.terminal.shell = shell;
        let set = Control::SetSettings {
            settings: settings.clone(),
        };
        app.send(0, set).await;
        app.wait_for(0, Control::Settings { settings }).await;
        open(&mut app, channel, &cwd).await;
        app.type_line(channel, line).await;
        app.shows(channel, shown).await;
        close(&mut app, channel).await;
    }

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
