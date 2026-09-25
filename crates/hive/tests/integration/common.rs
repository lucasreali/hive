//! Test harness: runs the real `hive` binary in a throwaway HOME and XDG layout.

use std::path::PathBuf;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use futures_util::{SinkExt, StreamExt};
use hive_protocol::{Control, Frame, FrameCodec, Role, WorktreeStatus};
use tokio::net::UnixStream;
use tokio::net::unix::{OwnedReadHalf, OwnedWriteHalf};
use tokio_util::codec::{FramedRead, FramedWrite};

/// `WSL_DISTRO_NAME` of every spawned `hive`, reported back in `Welcome`.
pub const DISTRO: &str = "hive-test";

pub const TIMEOUT: Duration = Duration::from_secs(10);

pub struct Env {
    pub dir: tempfile::TempDir,
}

/// Kills every process still running with this environment (daemons started by a
/// bridge, shells and their children), so a failing test never leaks processes.
/// Processes get a moment to exit on their own first, so coverage data is written.
impl Drop for Env {
    fn drop(&mut self) {
        let start = Instant::now();
        while !self.processes().is_empty() && start.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(20));
        }
        for proc in self.processes() {
            let pid = nix::unistd::Pid::from_raw(proc.pid);
            let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGKILL);
        }
    }
}

impl Env {
    /// Processes running with this environment.
    pub fn processes(&self) -> Vec<hive::procs::Proc> {
        let marker = format!("XDG_RUNTIME_DIR={}", self.path("run").display());
        let pids = with_var(&marker);
        hive::procs::list(hive::procs::Source::System)
            .into_iter()
            .filter(|proc| pids.contains(&proc.pid))
            .collect()
    }
}

/// Processes whose environment holds `var` (`NAME=value`).
#[cfg(target_os = "linux")]
fn with_var(var: &str) -> Vec<i32> {
    hive::procs::list(hive::procs::Source::System)
        .into_iter()
        .map(|proc| proc.pid)
        .filter(|pid| {
            let environ = std::fs::read(format!("/proc/{pid}/environ")).unwrap_or_default();
            environ.split(|b| *b == 0).any(|v| v == var.as_bytes())
        })
        .collect()
}

/// Processes whose environment holds `var` (`NAME=value`, no spaces): `ps -E` prints the
/// command line and then the environment, space-separated.
#[cfg(target_os = "macos")]
fn with_var(var: &str) -> Vec<i32> {
    let out = Command::new("ps")
        .args(["-ax", "-E", "-ww", "-o", "pid=,command="])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|line| line.split(' ').any(|word| word == var))
        .filter_map(|line| line.split_whitespace().next()?.parse().ok())
        .collect()
}

pub struct Conn {
    pub reader: FramedRead<OwnedReadHalf, FrameCodec>,
    pub writer: FramedWrite<OwnedWriteHalf, FrameCodec>,
}

impl Env {
    pub fn new() -> Self {
        let env = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        for sub in ["home", "run", "data", "config"] {
            std::fs::create_dir(env.path(sub)).unwrap();
        }
        env
    }

    pub fn path(&self, sub: &str) -> PathBuf {
        self.dir.path().join(sub)
    }

    pub fn socket(&self) -> PathBuf {
        self.path("run/hive/hive.sock")
    }

    pub fn hive(&self) -> Command {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_hive"));
        cmd.env("HOME", self.path("home"))
            .env("XDG_RUNTIME_DIR", self.path("run"))
            .env("XDG_DATA_HOME", self.path("data"))
            .env("XDG_CONFIG_HOME", self.path("config"))
            .env("WSL_DISTRO_NAME", DISTRO)
            // git run by the service sees neither the real config nor an enclosing repository.
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CEILING_DIRECTORIES", self.dir.path())
            .env_remove("HIVE_TERMINAL_ID")
            // Claude's session logs are read (and deleted) under the throwaway HOME only.
            .env_remove("CLAUDE_CONFIG_DIR");
        // On macOS terminals run `$SHELL`; the tests' shell commands are fish's, as on WSL.
        #[cfg(target_os = "macos")]
        cmd.env("SHELL", "fish");
        cmd
    }

    /// Starts `hive daemon` and waits until its socket accepts connections.
    pub fn daemon(&self) -> Daemon {
        let child = self
            .hive()
            .arg("daemon")
            .stdin(Stdio::null())
            .spawn()
            .unwrap();
        let daemon = Daemon(child);
        wait_until(|| std::os::unix::net::UnixStream::connect(self.socket()).is_ok());
        daemon
    }

    pub async fn raw(&self) -> Conn {
        let (read, write) = UnixStream::connect(self.socket())
            .await
            .unwrap()
            .into_split();
        Conn {
            reader: FramedRead::new(read, FrameCodec),
            writer: FramedWrite::new(write, FrameCodec),
        }
    }

    /// Connects and completes the handshake.
    /// Connects the app and waits until the service serves it. `Welcome` goes out before the
    /// service registers the app, so an event sent right after it could miss the app.
    pub async fn app(&self) -> Conn {
        let mut app = self.connect(Role::App).await;
        app.send(0, Control::ListProjects).await;
        let projects = Control::Projects { projects: vec![] };
        assert_eq!(app.control().await, (0, projects));
        app
    }

    pub async fn connect(&self, role: Role) -> Conn {
        let mut conn = self.raw().await;
        conn.send(0, Control::hello(role, hive::VERSION)).await;
        assert_eq!(
            conn.control().await,
            (
                0,
                Control::Welcome {
                    version: hive::VERSION.into(),
                    distro: Some(DISTRO.into()),
                }
            )
        );
        conn
    }
}

impl Conn {
    pub async fn send(&mut self, channel: u32, message: Control) {
        self.writer
            .send(Frame::control(channel, &message))
            .await
            .unwrap();
    }

    pub async fn next(&mut self) -> Option<Frame> {
        self.next_in_time().await.expect("timed out")
    }

    /// The next frame (`None` once the connection closes), or `None` after [`TIMEOUT`].
    async fn next_in_time(&mut self) -> Option<Option<Frame>> {
        let frame = tokio::time::timeout(TIMEOUT, self.reader.next())
            .await
            .ok()?;
        let Some(frame) = frame.map(Result::unwrap) else {
            return Some(None);
        };
        // fish 4 asks the terminal for its primary device attributes and waits for the
        // answer before reading input: answer as xterm.js does.
        let query = b"\x1b[0c";
        if frame.kind == hive_protocol::FrameType::Terminal
            && frame.payload.windows(query.len()).any(|w| w == query)
        {
            self.input(frame.channel, "\x1b[?1;2c").await;
        }
        Some(Some(frame))
    }

    pub async fn input(&mut self, channel: u32, text: &str) {
        self.writer
            .send(Frame::terminal(channel, text.to_owned()))
            .await
            .unwrap();
    }

    /// Opens a terminal in `cwd` and waits until it is running.
    pub async fn open_terminal(&mut self, channel: u32, cwd: &std::path::Path) {
        let cwd = cwd.to_string_lossy().into_owned();
        self.send(
            channel,
            Control::OpenTerminal {
                cwd,
                cols: 80,
                rows: 24,
            },
        )
        .await;
        assert_eq!(self.control().await, (channel, Control::TerminalOpened));
    }

    /// Collects the terminal's output until it contains `needle`, skipping control frames.
    pub async fn output_until(&mut self, channel: u32, needle: &str) -> String {
        let mut seen = String::new();
        while !seen.contains(needle) {
            let Some(frame) = self.next_in_time().await else {
                panic!("no {needle:?} in the output: {seen:?}");
            };
            let frame = frame.expect("connection closed");
            if frame.kind == hive_protocol::FrameType::Terminal && frame.channel == channel {
                seen.push_str(&String::from_utf8_lossy(&frame.payload));
            }
        }
        seen
    }

    /// Next control frame, skipping terminal output and `worktree_status` (sent whenever a
    /// status changes, see [`Conn::worktree_status`]).
    pub async fn control(&mut self) -> (u32, Control) {
        loop {
            let frame = self.next().await.expect("connection closed");
            match frame.to_control() {
                Ok(Control::WorktreeStatus { .. }) | Err(_) => {}
                Ok(message) => return (frame.channel, message),
            }
        }
    }

    /// The next `worktree_status` for `path`, skipping every other frame.
    pub async fn worktree_status(&mut self, path: &str) -> Option<WorktreeStatus> {
        loop {
            let frame = self.next().await.expect("connection closed");
            if let Ok(Control::WorktreeStatus { path: got, status }) = frame.to_control()
                && got == path
            {
                return status;
            }
        }
    }
}

/// A running daemon; killed on drop so a failing test never leaks it.
pub struct Daemon(pub Child);

impl Daemon {
    pub fn wait_exit(&mut self) -> ExitStatus {
        wait_exit(&mut self.0)
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Stops a daemon with SIGTERM so it exits normally (and writes its coverage data).
pub fn stop(mut daemon: Daemon) {
    let pid = nix::unistd::Pid::from_raw(daemon.0.id() as i32);
    nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM).unwrap();
    assert!(daemon.wait_exit().success());
}

pub fn wait_until(mut ready: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ready() {
        assert!(start.elapsed() < TIMEOUT, "timed out waiting");
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub fn wait_exit(child: &mut Child) -> ExitStatus {
    let mut status = None;
    wait_until(|| {
        status = child.try_wait().unwrap();
        status.is_some()
    });
    status.unwrap()
}
