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
    AgentState, Control, Frame, FrameCodec, FrameType, ProjectScripts, ProjectSettings, Role,
    SessionWindow, Settings, TerminalShell,
};
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio_util::codec::{FramedRead, FramedWrite};

/// How long anything may take: short, so that a mutant that breaks the service fails well within
/// cargo-mutants' 120 s instead of timing out.
const TIMEOUT: Duration = Duration::from_secs(20);

/// A throwaway profile: app data, settings, home and Claude folder in a temporary folder, and
/// a fake `claude.exe` first on `PATH` (`tests/fixtures/fake_claude.rs`: never the real one).
struct Env {
    dir: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("fake");
        std::fs::create_dir(&fake).unwrap();
        let source = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/fake_claude.rs");
        let built = std::process::Command::new("rustc")
            .args(["--edition", "2021", "-o"])
            .arg(fake.join("claude.exe"))
            .arg(source)
            .status();
        assert!(built.unwrap().success());
        Self { dir }
    }

    fn path(&self, sub: &str) -> PathBuf {
        self.dir.path().join(sub)
    }

    fn vars(&self) -> Vec<(&'static str, PathBuf)> {
        let path = std::env::var_os("PATH").unwrap();
        let path = std::env::split_paths(&path);
        let path = std::env::join_paths([self.path("fake")].into_iter().chain(path));
        vec![
            ("LOCALAPPDATA", self.path("local")),
            ("APPDATA", self.path("roaming")),
            ("USERPROFILE", self.path("home")),
            ("HOME", self.path("home")),
            ("CLAUDE_CONFIG_DIR", self.path("claude")),
            ("PATH", path.unwrap().into()),
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

    // The shell setting: new terminals run the chosen one. Git Bash's login files put its own
    // folders first on `PATH` (the user's may too): Hive's bin folder goes before them after.
    std::fs::write(home.join(".bash_profile"), "export PATH=/usr/bin:$PATH\n").unwrap();
    let bin = hive::terminal::conpty::msys_path(&paths.bin_dir());
    let first = format!("sh=42 first={bin}.");
    let chosen = [
        (TerminalShell::Cmd, "echo cmd=%OS%", "cmd=Windows_NT"),
        (
            TerminalShell::GitBash,
            r#"echo "sh=$((6*7)) first=${PATH%%:*}.""#,
            first.as_str(),
        ),
        (TerminalShell::Default, "echo back", "back"),
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

    // The service and the hooks run a copy in the bin folder, beside the `claude` wrapper.
    let (hive_copy, wrapper) = (
        paths.bin_dir().join("hive.exe"),
        paths.bin_dir().join("claude.exe"),
    );
    assert!(hive_copy.is_file() && wrapper.is_file());
    let hooks = std::fs::read_to_string(paths.hooks_settings()).unwrap();
    let hooks: serde_json::Value = serde_json::from_str(&hooks).unwrap();
    let command = &hooks["hooks"]["SessionStart"][0]["hooks"][0]["command"];
    assert_eq!(command.as_str(), hive_copy.to_str());

    // `claude` in a terminal is the wrapper: the fake `claude` gets Hive's hooks, and its
    // hooks reach the app (an agent) and make a worktree.
    let repo = home.join("repo");
    let git = |args: &[&str]| {
        let mut git = std::process::Command::new("git");
        git.args(["-c", "user.name=t", "-c", "user.email=t@t", "-C"])
            .arg(&repo);
        assert!(git.args(args).status().unwrap().success());
    };
    std::fs::create_dir(&repo).unwrap();
    git(&["init", "-q"]);
    git(&["commit", "-q", "--allow-empty", "-m", "init"]);
    let repo_cwd = repo.to_string_lossy().into_owned();
    open(&mut app, 5, &repo_cwd).await;
    // The user's PowerShell profile (from the real Documents folder) runs in the terminal: only
    // when `claude` is the wrapper and the next one on `PATH` the fake is it typed, never the
    // real one.
    let fake = env.path("fake").join("claude.exe");
    let (wrapper, fake) = (wrapper.display(), fake.display());
    let first = format!(
        "$c = @(Get-Command claude -All); \
         if ($c[0].Source -eq '{wrapper}' -and $c[1].Source -eq '{fake}') {{ 'claude=' + 'fake' }}"
    );
    app.type_line(5, &first).await;
    app.shows(5, "claude=fake").await;
    app.type_line(5, "claude SessionStart s2 WorktreeCreate w1")
        .await;
    let detected = |message: Option<&Control>, _: &str| matches!(message, Some(Control::AgentDetected { id, .. }) if id == "s2");
    app.until(5, "the wrapped agent", detected).await;
    app.shows(5, "WorktreeCreate=0:w1").await;
    let worktree = repo.join(".claude").join("worktrees").join("w1");
    assert!(worktree.is_dir());
    let output = &app.output[&5];
    assert!(output.contains("settings"), "{output:?}");
    assert!(output.contains("SessionStart=0:"), "{output:?}");
    let remove = format!("claude WorktreeRemove {}", worktree.display());
    app.type_line(5, &remove).await;
    app.shows(5, "WorktreeRemove=0:").await;
    assert!(!worktree.exists());
    close(&mut app, 5).await;

    // The statusline command parses in both shells Claude Code may run it with: it reports the
    // 5-hour window, through the pipe, and prints the user's own statusline.
    let claude_dir = env.path("claude");
    std::fs::create_dir_all(&claude_dir).unwrap();
    let user = r#"{"statusLine": {"type": "command", "command": "echo mine"}}"#;
    std::fs::write(claude_dir.join("settings.json"), user).unwrap();
    let statusline = hooks["statusLine"]["command"].as_str().unwrap();
    let resets_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        + 3600;
    let usage = SessionWindow {
        used_percentage: 42,
        resets_at,
    };
    let window = serde_json::json!({ "rate_limits": { "five_hour": usage } });
    let path = std::env::var_os("PATH").unwrap();
    let bash = hive::terminal::conpty::git_bash(&path).unwrap();
    let powershell = ["-NoProfile", "-NonInteractive", "-Command", statusline];
    let shells: [(PathBuf, Vec<&str>, String); 2] = [
        ("powershell".into(), powershell.to_vec(), window.to_string()),
        (bash, vec!["-c", statusline], "{}".into()),
    ];
    for (shell, args, input) in shells {
        let mut call = Command::new(&shell);
        call.args(&args)
            .envs(env.vars())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut call = call.spawn().unwrap();
        let mut stdin = call.stdin.take().unwrap();
        stdin.write_all(input.as_bytes()).await.unwrap();
        drop(stdin);
        let out = tokio::time::timeout(TIMEOUT, call.wait_with_output()).await;
        let out = out.unwrap().unwrap();
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert_eq!(stdout.trim(), "mine", "{shell:?}: {out:?}");
        assert!(out.status.success(), "{shell:?}: {out:?}");
    }
    let usage = Some(usage);
    app.wait_for(0, Control::SessionUsage { usage }).await;
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

#[tokio::test]
async fn a_held_worktree_is_kept_and_the_archive_script_runs_in_the_terminals_shell() {
    let env = Env::new();
    let root = env.path("repo");
    std::fs::create_dir_all(&root).unwrap();
    let git = |args: &[&str]| {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(args)
            .output()
            .unwrap();
        assert!(out.status.success(), "{out:?}");
    };
    git(&["init", "-q", "-b", "main"]);
    let who = ["-c", "user.name=a", "-c", "user.email=a@b"];
    git(&[&who[..], &["commit", "-q", "--allow-empty", "-m", "m"]].concat());
    let (mut bridge, mut app) = bridge(&env);
    app.send(0, Control::hello(Role::App, hive::VERSION)).await;
    let path = root.to_string_lossy().into_owned();
    app.send(0, Control::AddProject { path }).await;
    let added =
        |message: Option<&Control>, _: &str| matches!(message, Some(Control::ProjectAdded { .. }));
    let Some(Control::ProjectAdded { project }) = app.until(0, "the project", added).await else {
        unreachable!()
    };
    // The archive script, in Command Prompt: it notes the worktree it ran in.
    let archived = root.join("archived");
    let script = format!(r#"echo %HIVE_WORKTREE_PATH%> "{}""#, archived.display());
    let mut settings = Settings::default();
    settings.terminal.shell = TerminalShell::Cmd;
    let scripts = ProjectScripts {
        archive: Some(script),
        ..Default::default()
    };
    let project_settings = ProjectSettings { scripts };
    settings
        .projects
        .insert(project.id.clone(), project_settings);
    let set = Control::SetSettings {
        settings: settings.clone(),
    };
    app.send(0, set).await;
    app.wait_for(0, Control::Settings { settings }).await;
    let create = Control::CreateWorktree {
        project: project.id,
        name: "a".into(),
        base: None,
    };
    app.send(0, create).await;
    let created = |message: Option<&Control>, _: &str| {
        matches!(message, Some(Control::WorktreeCreated { .. }))
    };
    let Some(Control::WorktreeCreated { path, .. }) = app.until(0, "the worktree", created).await
    else {
        unreachable!()
    };

    // A process working in it (not in a terminal of Hive) keeps it, before its script runs.
    let mut ping = std::process::Command::new("ping")
        .args(["-n", "30", "127.0.0.1"])
        .current_dir(&path)
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let remove = Control::RemoveWorktree {
        path: path.clone(),
        force: false,
    };
    app.send(0, remove.clone()).await;
    let answered = |message: Option<&Control>, _: &str| {
        matches!(
            message,
            Some(Control::RemoveWorktreeFailed { .. } | Control::WorktreeRemoved { .. })
        )
    };
    let failed = app.until(0, "the refusal", answered).await;
    let Some(Control::RemoveWorktreeFailed { message, .. }) = failed else {
        panic!("{failed:?}")
    };
    let held = format!("in use by ping ({}): close its terminals first", ping.id());
    assert_eq!(message.to_lowercase(), held);
    assert!(std::path::Path::new(&path).is_dir());
    assert!(!archived.exists());

    // Once it is gone: the script runs in the worktree, then the worktree goes.
    ping.kill().unwrap();
    ping.wait().unwrap();
    app.send(0, remove).await;
    let removed = app.until(0, "the removal", answered).await;
    assert!(
        matches!(removed, Some(Control::WorktreeRemoved { .. })),
        "{removed:?}"
    );
    assert!(!std::path::Path::new(&path).exists());
    let noted = std::fs::read_to_string(&archived).unwrap();
    assert_eq!(noted.trim_end(), path);

    drop(app);
    let ended = tokio::time::timeout(TIMEOUT, bridge.wait()).await;
    assert!(ended.unwrap().unwrap().success());
}
