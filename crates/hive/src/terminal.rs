//! PTY-backed terminals running fish. Pass-through only: no scrollback is kept here.

use std::path::Path;
use std::time::{Duration, Instant};

use bytes::Bytes;
use nix::sys::signal::{Signal, killpg};
use nix::unistd::Pid;
use pty_process::{OwnedReadPty, OwnedWritePty, Size};
use tokio::io::AsyncWriteExt;
use tokio::process::Child;
use tokio::sync::mpsc;

use crate::procs;
use crate::watch::Watch;

/// Time a terminal's processes get to exit after SIGHUP before SIGKILL.
const GRACE: Duration = Duration::from_secs(2);

/// A running terminal, as kept in the service registry.
pub struct Terminal {
    /// Session id of the shell (it is the session leader, so this is also its pid).
    pub session: i32,
    /// Unhooked-`claude` detector for this terminal.
    pub watch: Watch,
    /// When the PTY last printed something (the silence rule of agent states).
    pub last_output: Instant,
    input: mpsc::UnboundedSender<Input>,
}

pub enum Input {
    Data(Bytes),
    Resize { cols: u16, rows: u16 },
}

impl Terminal {
    /// Queues input or a resize; ignored once the terminal is gone.
    pub fn send(&self, input: Input) {
        let _ = self.input.send(input);
    }
}

/// Starts `fish` on a new PTY in `cwd`, with `bin_dir` first on `PATH` and
/// `HIVE_TERMINAL_ID` set. Returns the registry entry, the output side and the child.
pub fn spawn(
    id: u32,
    cwd: &str,
    cols: u16,
    rows: u16,
    bin_dir: &Path,
) -> Result<(Terminal, OwnedReadPty, Child), String> {
    let start = || -> pty_process::Result<_> {
        let (pty, pts) = pty_process::open()?;
        pty.resize(Size::new(rows, cols))?;
        let child = pty_process::Command::new("fish")
            .arg("-C")
            .arg(path_command(bin_dir))
            .env("HIVE_TERMINAL_ID", id.to_string())
            .env("TERM", "xterm-256color")
            .current_dir(cwd)
            .spawn(pts)?;
        Ok((pty, child))
    };
    let (pty, child) = start().map_err(|err| format!("cannot start a terminal in {cwd}: {err}"))?;
    // The shell leads its own session, so its pid is the session id. The pid is always
    // known right after spawning; -1 (no process has it) keeps a missing one harmless.
    let session = child.id().map_or(-1, |pid| pid as i32);
    let (output, writer) = pty.into_split();
    let (input, input_rx) = mpsc::unbounded_channel();
    tokio::spawn(feed(writer, input_rx));
    Ok((
        Terminal {
            session,
            watch: Watch::default(),
            last_output: Instant::now(),
            input,
        },
        output,
        child,
    ))
}

/// fish command run after the user's config: puts `bin_dir` first on `PATH` for this shell only.
/// Never `fish_add_path` without flags: it would persist through a universal variable.
fn path_command(bin_dir: &Path) -> String {
    let dir = bin_dir
        .to_string_lossy()
        .replace('\\', "\\\\")
        .replace('\'', "\\'");
    format!("set -gx PATH '{dir}' $PATH")
}

async fn feed(mut pty: OwnedWritePty, mut input: mpsc::UnboundedReceiver<Input>) {
    while let Some(input) = input.recv().await {
        // A dead PTY fails every write; the loop ends when the terminal is dropped.
        let _ = match input {
            Input::Data(bytes) => pty.write_all(&bytes).await.map_err(|_| ()),
            Input::Resize { cols, rows } => pty.resize(Size::new(rows, cols)).map_err(|_| ()),
        };
    }
}

/// Ends every process group in the given sessions: SIGHUP, then SIGKILL for
/// whatever is still alive after [`GRACE`].
pub async fn end_sessions(sessions: &[i32]) {
    signal_sessions(sessions, Signal::SIGHUP);
    let _ = tokio::time::timeout(GRACE, async {
        while any_alive(sessions) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
    signal_sessions(sessions, Signal::SIGKILL);
}

fn any_alive(sessions: &[i32]) -> bool {
    procs::list(procs::Source::System)
        .iter()
        .any(|p| sessions.contains(&p.session))
}

fn signal_sessions(sessions: &[i32], signal: Signal) {
    for proc in procs::list(procs::Source::System) {
        if sessions.contains(&proc.session) {
            let _ = killpg(Pid::from_raw(proc.pgrp), signal);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_command_quotes_the_directory_for_fish() {
        assert_eq!(
            path_command(Path::new("/d/hive/bin")),
            "set -gx PATH '/d/hive/bin' $PATH"
        );
        assert_eq!(
            path_command(Path::new("/it's a\\dir")),
            r"set -gx PATH '/it\'s a\\dir' $PATH"
        );
    }
}
