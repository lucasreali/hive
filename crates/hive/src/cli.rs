//! Command-line entry point.

use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

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
        Command::Worktree { command } => run_worktree(command),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("hive: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run_worktree(command: WorktreeCommand) -> io::Result<()> {
    let cwd = std::env::current_dir()?;
    match command {
        WorktreeCommand::Create { name, base } => {
            print_path(&worktree::create(&cwd, &name, base.as_deref())?)
        }
        WorktreeCommand::List => worktree::list(&cwd)?
            .iter()
            .try_for_each(|wt| writeln!(io::stdout(), "{wt}")),
        WorktreeCommand::Remove { name } => worktree::remove(&cwd, &name),
        WorktreeCommand::HookCreate => print_path(&worktree::hook_create(&mut io::stdin())?),
        WorktreeCommand::HookRemove => worktree::hook_remove(&mut io::stdin()),
    }
}

/// Prints a path byte for byte (a hook's stdout must be exactly the path).
fn print_path(path: &Path) -> io::Result<()> {
    let mut line = path.as_os_str().as_bytes().to_vec();
    line.push(b'\n');
    io::stdout().write_all(&line)
}

/// Runs `task` to completion, then drops the runtime without waiting for blocking
/// work (a pending stdin read would otherwise keep the process alive).
fn block_on(task: impl Future<Output = std::io::Result<()>>) -> std::io::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let result = runtime.block_on(task);
    runtime.shutdown_background();
    result
}
