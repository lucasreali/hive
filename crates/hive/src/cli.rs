//! Command-line entry point.

use std::io::{self, Write};
use std::process::ExitCode;

use clap::{Parser, Subcommand};
#[cfg(unix)]
use tokio::signal::unix::{SignalKind, signal};

use crate::paths::Paths;
use crate::worktree;

#[derive(Debug, Parser)]
#[command(
    name = "hive",
    version,
    about = "Companion service and CLI for the Hive desktop app"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Run the service (started by `hive bridge`; lives as long as the app connection).
    Daemon,
    /// Connect stdio to the service, starting it if needed (run by the app through `wsl.exe`).
    Bridge,
    /// Forward one Claude Code hook call (JSON on stdin) to the service. Always exits 0.
    Hook {
        /// Hook event name, e.g. `SessionStart`.
        event: String,
        /// Also append the raw call to this JSONL file (timestamp, event, terminal, payload).
        #[arg(long, value_name = "FILE")]
        record: Option<std::path::PathBuf>,
    },
    /// Show a short label on this terminal's tab and agent row in the app (run inside a Hive
    /// terminal). At most 40 characters are kept.
    Badge {
        /// The label; several words are joined with spaces.
        #[arg(required_unless_present = "clear", conflicts_with = "clear")]
        text: Vec<String>,
        /// Remove the label.
        #[arg(long)]
        clear: bool,
    },
    /// Claude Code's statusline in Hive's terminals (JSON on stdin): reports the 5-hour usage
    /// to the service, then runs the user's own statusline and prints what it prints.
    Statusline,
    /// Manage git worktrees in `.claude/worktrees/` (Claude Code's convention).
    Worktree {
        #[command(subcommand)]
        command: WorktreeCommand,
    },
}

#[derive(Debug, Subcommand)]
enum WorktreeCommand {
    /// Create `.claude/worktrees/<name>` on a new branch `worktree-<name>` and print its path.
    Create {
        name: String,
        /// Local or remote branch to start from (default: HEAD).
        #[arg(long)]
        base: Option<String>,
    },
    /// List the repository's worktrees: path and branch.
    List,
    /// Remove a worktree without changes; its branch is kept.
    Remove { name: String },
    /// `WorktreeCreate` hook: reads Claude Code's JSON on stdin, prints only the new path.
    HookCreate,
    /// `WorktreeRemove` hook: reads Claude Code's JSON on stdin.
    HookRemove,
}

pub fn run() -> ExitCode {
    let cli = Cli::parse();
    let paths = Paths::from_env();
    // `hive statusline` exits with the user's statusline's code.
    #[cfg(unix)]
    let mut code = 0;
    #[cfg(windows)]
    let code = 0;
    let result = match cli.command {
        Command::Daemon => block_on(crate::daemon::run(&paths)),
        Command::Bridge => {
            std::env::current_exe().and_then(|hive| block_on(crate::bridge::run(&paths, &hive)))
        }
        Command::Hook { event, record } => {
            // Whatever happens, the agent must not see a failing hook.
            let stdin = tokio::io::stdin();
            let _ = block_on(async {
                crate::hook::run(&event, record.as_deref(), &paths, stdin).await;
                Ok(())
            });
            Ok(())
        }
        Command::Badge { text, .. } => {
            let Some(terminal) = terminal_id(std::env::var("HIVE_TERMINAL_ID").ok()) else {
                eprintln!("hive: not in a Hive terminal (HIVE_TERMINAL_ID is missing or invalid)");
                return ExitCode::from(2);
            };
            // `--clear` leaves `text` empty, which clears the badge.
            block_on(crate::hook::badge(&paths, terminal, text.join(" ")))
        }
        #[cfg(unix)]
        Command::Statusline => block_on(statusline(&paths)).and_then(|(out, status)| {
            code = status;
            io::stdout().write_all(&out)
        }),
        // It runs the user's statusline with `sh` (12.5.5).
        #[cfg(windows)]
        Command::Statusline => Err(crate::windows::unsupported("the statusline")),
        Command::Worktree { command } => run_worktree(command, &paths),
    };
    match result {
        Ok(()) => ExitCode::from(code),
        Err(err) => {
            eprintln!("hive: {err}");
            ExitCode::FAILURE
        }
    }
}

/// A Hive terminal's id: its channel number, from 1.
fn terminal_id(value: Option<String>) -> Option<u32> {
    value?.parse().ok().filter(|&id| id != 0)
}

fn run_worktree(command: WorktreeCommand, paths: &Paths) -> io::Result<()> {
    let cwd = std::env::current_dir()?;
    match command {
        WorktreeCommand::Create { name, base } => {
            report(worktree::create(&cwd, &name, base.as_deref())?)
        }
        WorktreeCommand::List => worktree::list(&cwd)?
            .iter()
            .try_for_each(|wt| writeln!(io::stdout(), "{wt}")),
        WorktreeCommand::Remove { name } => worktree::remove(&cwd, &name),
        WorktreeCommand::HookCreate => {
            let mut payload = worktree::read_payload(&mut io::stdin())?;
            let created = worktree::hook_create(&payload)?;
            let path = created.path.to_string_lossy().into_owned();
            report(created)?;
            if let Some(fields) = payload.as_object_mut() {
                fields.insert("worktree_path".to_owned(), path.into());
            }
            tell_service(paths, "WorktreeCreate", payload);
            Ok(())
        }
        WorktreeCommand::HookRemove => {
            let payload = worktree::read_payload(&mut io::stdin())?;
            worktree::hook_remove(&payload)?;
            tell_service(paths, "WorktreeRemove", payload);
            Ok(())
        }
    }
}

/// Reports a worktree hook's work to the service like `hive hook` does, so the app's
/// worktrees follow. Never changes the hook's output or exit code.
fn tell_service(paths: &Paths, event: &str, payload: serde_json::Value) {
    let terminal_id = std::env::var("HIVE_TERMINAL_ID").ok();
    let _ = block_on(async {
        let sent_ns = crate::hook::monotonic_ns();
        crate::hook::forward(paths, event, terminal_id, payload, sent_ns).await;
        Ok(())
    });
}

/// Prints the notes on stderr and the path byte for byte on stdout (a hook's stdout must be
/// exactly the path).
fn report(created: worktree::Created) -> io::Result<()> {
    for note in &created.notes {
        eprintln!("hive: {note}");
    }
    let mut line = created.path.as_os_str().as_encoded_bytes().to_vec();
    line.push(b'\n');
    io::stdout().write_all(&line)
}

/// `hive statusline` (see [`crate::statusline::run`]), cancelled by a SIGTERM: Claude Code
/// cancels a run when a newer one starts.
#[cfg(unix)]
async fn statusline(paths: &Paths) -> io::Result<crate::statusline::Output> {
    let mut terminate = signal(SignalKind::terminate())?;
    let cancel = async move {
        terminate.recv().await;
    };
    let (env, cwd) = (crate::statusline::env, std::env::current_dir().ok());
    let stdin = tokio::io::stdin();
    Ok(crate::statusline::run(paths, stdin, env, cwd, cancel).await)
}

/// Runs `task` to completion, then drops the runtime without waiting for blocking
/// work (a pending stdin read would otherwise keep the process alive).
fn block_on<T>(task: impl Future<Output = std::io::Result<T>>) -> std::io::Result<T> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(task);
    runtime.shutdown_background();
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_ids_are_positive_numbers() {
        assert_eq!(terminal_id(Some("7".into())), Some(7));
        for bad in ["0", "-1", "x", ""] {
            assert_eq!(terminal_id(Some(bad.into())), None, "{bad}");
        }
        assert_eq!(terminal_id(None), None);
    }
}
